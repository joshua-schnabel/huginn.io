use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use chrono::Local;
use clap::Parser;
use colored::Colorize;
use huginn_core::config::{AppConfig, LogFormat};
use huginn_core::event::{EventHub, ProbeEvent};
use huginn_core::types::ProbeResult;
use huginn_influx::queue::RetryQueue;
use huginn_influx::writer::{run_batcher, run_writer, InfluxWriter};
use tokio::sync::broadcast;
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;

mod scheduler;

// ---------------------------------------------------------------------------
// Type aliases
// ---------------------------------------------------------------------------

type Shutdown = broadcast::Sender<()>;

// ---------------------------------------------------------------------------
// CLI args
// ---------------------------------------------------------------------------

#[derive(Parser, Debug)]
#[command(
    name = "huginn",
    version,
    about = "Uptime & response-time monitor — InfluxDB backend"
)]
struct Args {
    /// Path to the YAML config file
    #[arg(
        short,
        long,
        env = "HUGINN_CONFIG",
        default_value = "/etc/huginn/config.yaml"
    )]
    config: String,

    /// Output format: pretty or json. Overrides `log.format` from the config file.
    ///
    /// Deliberately an Option with no default: with `default_value = "pretty"`
    /// this was always set, so "not given" and "explicitly pretty" were
    /// indistinguishable and the config could never be overridden back.
    #[arg(long, env = "HUGINN_LOG_FORMAT")]
    output: Option<String>,
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    // Load config (applies ENV overrides internally). Warnings are returned
    // rather than logged: tracing isn't up yet — its log level comes from this
    // very config — so anything logged here would be discarded.
    let (cfg, mut warnings) = AppConfig::load_with_warnings(&args.config)
        .with_context(|| format!("Failed to load config from '{}'", args.config))?;

    let use_json = resolve_output_format(args.output.as_deref(), &cfg.log.format, &mut warnings);

    init_tracing(use_json, &cfg.log.level);

    // Now that there is somewhere for them to go.
    for w in &warnings {
        warn!("{w}");
    }

    info!("huginn starting — config: {}", args.config);

    // Shutdown channel — an OS stop signal fires it.
    let (shutdown_tx, _): (Shutdown, _) = broadcast::channel(1);
    let shutdown_sig = shutdown_tx.clone();
    tokio::spawn(async move {
        wait_for_shutdown_signal().await;
        info!("Shutdown signal received");
        let _ = shutdown_sig.send(());
    });

    run(Arc::new(cfg), use_json, shutdown_tx).await
}

/// Resolve when the OS asks the process to stop.
///
/// Ctrl+C (SIGINT) on every platform, plus **SIGTERM** on Unix — the signal
/// systemd and `docker stop` actually send. Catching only SIGINT meant the whole
/// shutdown drain never ran under those supervisors, so every buffered-but-
/// unwritten InfluxDB result was lost on each restart/deploy.
async fn wait_for_shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        match signal(SignalKind::terminate()) {
            Ok(mut term) => {
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => {}
                    _ = term.recv() => {}
                }
            }
            Err(e) => {
                warn!("could not install SIGTERM handler ({e}) — Ctrl+C only");
                let _ = tokio::signal::ctrl_c().await;
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

// ---------------------------------------------------------------------------
// Output format resolution
// ---------------------------------------------------------------------------

/// Resolve the output format: `--output` (or `HUGINN_LOG_FORMAT`) wins over
/// `log.format` from the config file — in *both* directions.
///
/// This was `args.output == "json" || cfg.log.format == Json`. Being an OR, a
/// config file saying `format: json` could never be overridden back to pretty
/// from the CLI, which contradicts the documented CLI > ENV > YAML precedence.
fn resolve_output_format(
    cli: Option<&str>,
    cfg_format: &LogFormat,
    warnings: &mut Vec<String>,
) -> bool {
    match cli {
        Some(s) if s.eq_ignore_ascii_case("json") => true,
        Some(s) if s.eq_ignore_ascii_case("pretty") => false,
        Some(other) => {
            warnings.push(format!(
                "--output/HUGINN_LOG_FORMAT='{other}' is not a known format \
                 (expected json/pretty) — falling back to the config file"
            ));
            *cfg_format == LogFormat::Json
        }
        None => *cfg_format == LogFormat::Json,
    }
}

// ---------------------------------------------------------------------------
// Tracing initialisation
// ---------------------------------------------------------------------------

fn init_tracing(use_json: bool, log_level: &str) {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(log_level));

    if use_json {
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .json()
            .init();
    } else {
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .pretty()
            .init();
    }
}

// ---------------------------------------------------------------------------
// Core wiring — extracted for testability
// ---------------------------------------------------------------------------

pub(crate) async fn run(
    cfg: Arc<AppConfig>,
    use_json: bool,
    shutdown_tx: Shutdown,
) -> anyhow::Result<()> {
    // Central event hub — all components subscribe here
    let hub = Arc::new(EventHub::new(cfg.event_hub_capacity));

    // Console output subscriber. Subscribe *before* spawning: a broadcast
    // receiver only sees events sent after it subscribed, so subscribing inside
    // the task could miss the first probe tick if the task isn't polled in time.
    // A Receiver doesn't keep the hub's Sender alive, so it still gets Closed at
    // shutdown (when run() drops `hub`).
    let mut console_rx = hub.subscribe();
    tokio::spawn(async move {
        loop {
            match console_rx.recv().await {
                Ok(ProbeEvent::ProbeCompleted(result)) => print_result(&result, use_json),
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    error!("Console subscriber dropped {n} events");
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    // InfluxDB: two tasks with a bounded queue between them. The batcher never
    // awaits I/O, so a slow or dead InfluxDB can't stall the EventHub reader and
    // cause events to be dropped at the source.
    let writer =
        Arc::new(InfluxWriter::new(&cfg.influx).context("Failed to initialise InfluxDB writer")?);
    let queue = Arc::new(RetryQueue::new(cfg.influx.max_buffered_bytes));

    // Subscribe before spawning (see the console subscriber above) so no early
    // probe result is lost to InfluxDB at startup.
    let batcher_rx = hub.subscribe();
    tokio::spawn(run_batcher(
        batcher_rx,
        Arc::clone(&queue),
        cfg.influx.batch_size,
        cfg.influx.batch_timeout_ms,
    ));

    let writer_handle = tokio::spawn(run_writer(
        writer,
        Arc::clone(&queue),
        cfg.influx.retry_initial_backoff_ms,
        cfg.influx.retry_max_backoff_ms,
        shutdown_tx.subscribe(),
    ));

    // Web UI + Prometheus subscribers (if enabled). Both listeners feed from
    // one shared WebState so the hub gains a single subscriber either way.
    if cfg.ui.enabled || cfg.metrics.enabled {
        // Bind both sockets here, on the main task, before anything is spawned.
        // They used to be bound inside their own tasks, so a taken port produced
        // a single logged error while the process carried on without the service
        // that had been explicitly enabled. For the UI that is a dashboard
        // nobody can reach; for metrics it is worse, because Prometheus reports
        // a scrape target that never answers as the *monitored* host being down.
        // Enabled and absent is not a state worth having.
        let ui_listener = if cfg.ui.enabled {
            Some(
                huginn_web::server::bind_ui(&cfg.ui.bind, cfg.ui.port)
                    .await
                    .context("Failed to start the debug UI")?,
            )
        } else {
            None
        };
        // Read the key file up front too: a configured-but-broken key file must
        // stop startup, not fall back to serving unauthenticated (same
        // fail-closed rule as the InfluxDB token).
        let metrics_bound = if cfg.metrics.enabled {
            let api_key = cfg
                .metrics
                .read_api_key()
                .context("Failed to read the metrics API key file")?;
            Some((
                huginn_web::prometheus::bind_metrics(&cfg.metrics.bind, cfg.metrics.port)
                    .await
                    .context("Failed to start the Prometheus metrics listener")?,
                api_key,
            ))
        } else {
            None
        };

        let state = Arc::new(huginn_web::state::WebState::new());
        Arc::clone(&state).start_event_loop(Arc::clone(&hub));
        if let Some(listener) = ui_listener {
            let ui_state = Arc::clone(&state);
            tokio::spawn(async move {
                if let Err(e) = huginn_web::server::serve_ui(listener, ui_state).await {
                    error!("Web UI error: {e}");
                }
            });
        }
        if let Some((listener, api_key)) = metrics_bound {
            let metrics_state = Arc::clone(&state);
            tokio::spawn(async move {
                if let Err(e) =
                    huginn_web::prometheus::serve_metrics(listener, metrics_state, api_key).await
                {
                    error!("Prometheus metrics error: {e}");
                }
            });
        }
    }

    // Subscribe to shutdown BEFORE moving shutdown_tx into the scheduler
    let mut shutdown_rx = shutdown_tx.subscribe();

    // Start scheduler — publishes ProbeEvents to the hub
    scheduler::run(Arc::clone(&cfg), Arc::clone(&hub), shutdown_tx).await;

    // Block until a shutdown signal arrives; this keeps all spawned tasks alive.
    let _ = shutdown_rx.recv().await;

    // Drop hub — signals RecvError::Closed to all event subscribers. The batcher
    // takes that as its cue to queue whatever it still holds and close the queue.
    drop(hub);

    // Give the writer a bounded window to drain. Without this await, returning
    // here tears the runtime down mid-drain and the buffered results — the ones
    // the queue exists to protect — are lost at the last moment. The bound
    // matters just as much: retries are unbounded, so an InfluxDB that is down
    // at shutdown would otherwise keep the process alive forever.
    let drain = Duration::from_millis(cfg.influx.shutdown_drain_timeout_ms);
    match tokio::time::timeout(drain, writer_handle).await {
        Ok(Ok(())) => info!("InfluxDB writer drained cleanly"),
        Ok(Err(e)) => error!("InfluxDB writer task failed: {e}"),
        Err(_) => warn!(
            timeout_ms = cfg.influx.shutdown_drain_timeout_ms,
            "InfluxDB unreachable at shutdown — discarding buffered results"
        ),
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Console output
// ---------------------------------------------------------------------------

fn print_result(r: &ProbeResult, json: bool) {
    if json {
        println!("{}", serde_json::to_string(r).unwrap_or_default());
        return;
    }

    let ts = Local::now().format("%Y-%m-%d %H:%M:%S");
    let status = if r.up {
        "✅".green().bold()
    } else {
        "❌".red().bold()
    };
    let name = r.probe_name.cyan();
    let kind = r.probe_type.to_uppercase().yellow();
    let ms = format!("{:.1}ms", r.response_ms).white();

    let extra = match (r.status_code, &r.error) {
        (Some(code), _) => format!("  HTTP {code}").dimmed().to_string(),
        (_, Some(err)) => format!("  {err}").red().to_string(),
        _ => String::new(),
    };

    println!("[{ts}]  {status}  {name:<28} {kind:<6}  {ms}{extra}");
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use huginn_core::config::{
        AppConfig, InfluxConfig, LogConfig, MetricsConfig, ProbeConfig, ProbeType, UiConfig,
    };
    use huginn_core::types::ProbeResult;
    use std::sync::Arc;
    use std::time::Duration;

    // -----------------------------------------------------------------------
    // Fixtures
    // -----------------------------------------------------------------------

    fn success_result() -> ProbeResult {
        ProbeResult::success("web", "http", "https://example.com", 42.5, Some(200))
    }

    fn failure_result() -> ProbeResult {
        ProbeResult::failure("db", "tcp", "host:5432", 5000.0, "connection refused")
    }

    fn no_status_result() -> ProbeResult {
        ProbeResult::success("dns", "udp", "8.8.8.8:53", 1.5, None)
    }

    /// Write `content` to a temp file and return its path.
    fn tempfile_with(content: &str) -> tempfile::NamedTempFile {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        use std::io::Write as _;
        write!(f, "{content}").unwrap();
        f
    }

    /// Bind port 0 and return the OS-assigned free port number.
    fn free_port() -> u16 {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    }

    /// Minimal config: 1 TCP probe against a closed port, no UI, InfluxDB on a
    /// dead port.
    ///
    /// `shutdown_drain_timeout_ms` is deliberately tiny. InfluxDB here is
    /// unreachable, so on shutdown the writer attempts one doomed POST — and a
    /// connect to a closed local port takes ~2s to be refused on Windows, not
    /// microseconds. With the 5s default these tests spend their whole budget
    /// inside that drain. 200ms exercises the "InfluxDB unreachable at
    /// shutdown, give up" path, which is exactly the path this config models.
    /// A *successful* drain is covered by tests/influx_retry_test.rs.
    fn minimal_config(token_file: &tempfile::NamedTempFile) -> AppConfig {
        AppConfig {
            influx: InfluxConfig {
                url: "http://127.0.0.1:19999".into(),
                org: "o".into(),
                bucket: "b".into(),
                token_file: token_file.path().to_string_lossy().into_owned(),
                batch_size: 10,
                batch_timeout_ms: 1000,
                shutdown_drain_timeout_ms: 200,
                ..Default::default()
            },
            probes: vec![ProbeConfig {
                name: "test-probe".into(),
                probe_type: ProbeType::Tcp,
                target: "127.0.0.1:1".into(),
                interval_secs: 1,
                timeout_secs: 1,
                ..Default::default()
            }],
            ui: UiConfig {
                enabled: false,
                bind: "127.0.0.1".into(),
                port: 9900,
            },
            metrics: MetricsConfig::default(),
            log: LogConfig::default(),
            event_hub_capacity: 256,
        }
    }

    // -----------------------------------------------------------------------
    // resolve_output_format
    // -----------------------------------------------------------------------

    /// The regression this function exists for: the old OR meant a config saying
    /// json could never be overridden back to pretty from the CLI.
    #[test]
    fn cli_pretty_overrides_config_json() {
        let mut w = Vec::new();
        assert!(!resolve_output_format(
            Some("pretty"),
            &LogFormat::Json,
            &mut w
        ));
        assert!(w.is_empty());
    }

    #[test]
    fn cli_json_overrides_config_pretty() {
        let mut w = Vec::new();
        assert!(resolve_output_format(
            Some("json"),
            &LogFormat::Pretty,
            &mut w
        ));
    }

    #[test]
    fn config_decides_when_cli_absent() {
        let mut w = Vec::new();
        assert!(resolve_output_format(None, &LogFormat::Json, &mut w));
        assert!(!resolve_output_format(None, &LogFormat::Pretty, &mut w));
        assert!(w.is_empty());
    }

    #[test]
    fn cli_format_is_case_insensitive() {
        let mut w = Vec::new();
        assert!(resolve_output_format(
            Some("JSON"),
            &LogFormat::Pretty,
            &mut w
        ));
        assert!(!resolve_output_format(
            Some("Pretty"),
            &LogFormat::Json,
            &mut w
        ));
    }

    /// An unknown value must warn and defer, not silently pick a format.
    #[test]
    fn unknown_cli_format_warns_and_falls_back_to_config() {
        let mut w = Vec::new();
        assert!(resolve_output_format(Some("xml"), &LogFormat::Json, &mut w));
        assert_eq!(w.len(), 1);
        assert!(
            w[0].contains("xml"),
            "warning should name the bad value: {w:?}"
        );
    }

    // -----------------------------------------------------------------------
    // print_result unit tests (existing)
    // -----------------------------------------------------------------------

    #[test]
    fn print_result_json_does_not_panic_on_success() {
        print_result(&success_result(), true);
    }

    #[test]
    fn print_result_json_does_not_panic_on_failure() {
        print_result(&failure_result(), true);
    }

    #[test]
    fn print_result_pretty_does_not_panic_on_success() {
        print_result(&success_result(), false);
    }

    #[test]
    fn print_result_pretty_does_not_panic_on_failure() {
        print_result(&failure_result(), false);
    }

    #[test]
    fn print_result_pretty_no_status_code() {
        print_result(&no_status_result(), false);
    }

    // -----------------------------------------------------------------------
    // run() integration tests
    // -----------------------------------------------------------------------

    /// run() must NOT return while no shutdown signal has been sent.
    ///
    /// This is the guard for the keep-alive in run(). Deleting the
    /// `shutdown_rx.recv().await` makes run() fall through to Ok(()) instantly,
    /// main() returns, and the Tokio runtime cancels every probe before it
    /// fires — the daemon monitors nothing. That bug shipped on this branch and
    /// no test caught it: `run_exits_cleanly_on_shutdown` below asserts only
    /// that run() *does* return, which a broken run() does trivially.
    #[tokio::test]
    async fn run_stays_alive_without_shutdown_signal() {
        let tf = tempfile_with("mytoken");
        let cfg = Arc::new(minimal_config(&tf));
        let (shutdown_tx, _) = broadcast::channel(1);

        let handle = tokio::spawn(run(Arc::clone(&cfg), false, shutdown_tx.clone()));

        // Deliberately send nothing.
        tokio::time::sleep(Duration::from_millis(300)).await;

        assert!(
            !handle.is_finished(),
            "run() returned without a shutdown signal — probes are killed at startup"
        );

        shutdown_tx.send(()).ok();
        let _ = tokio::time::timeout(Duration::from_secs(3), handle).await;
    }

    /// run() starts, fires one probe, then shuts down cleanly.
    #[tokio::test]
    async fn run_exits_cleanly_on_shutdown() {
        let tf = tempfile_with("mytoken");
        let cfg = Arc::new(minimal_config(&tf));
        let (shutdown_tx, _) = broadcast::channel(1);

        let handle = tokio::spawn(run(Arc::clone(&cfg), false, shutdown_tx.clone()));

        tokio::time::sleep(Duration::from_millis(100)).await;
        shutdown_tx.send(()).ok();

        tokio::time::timeout(Duration::from_secs(3), handle)
            .await
            .expect("run() did not exit within 3 s")
            .expect("run() task panicked")
            .expect("run() returned an error");
    }

    /// run() with use_json=true reaches the json branch of print_result.
    #[tokio::test]
    async fn run_json_mode_exits_cleanly() {
        let tf = tempfile_with("mytoken");
        let cfg = Arc::new(minimal_config(&tf));
        let (shutdown_tx, _) = broadcast::channel(1);

        let handle = tokio::spawn(run(Arc::clone(&cfg), true, shutdown_tx.clone()));

        tokio::time::sleep(Duration::from_millis(100)).await;
        shutdown_tx.send(()).ok();

        tokio::time::timeout(Duration::from_secs(3), handle)
            .await
            .expect("run() did not exit within 3 s")
            .expect("run() task panicked")
            .expect("run() returned an error");
    }

    /// run() with ui.enabled=true spawns the web server — /health must respond.
    #[tokio::test]
    async fn run_with_ui_enabled_responds_to_health_check() {
        let tf = tempfile_with("mytoken");
        let port = free_port();
        let mut cfg = minimal_config(&tf);
        cfg.ui.enabled = true;
        cfg.ui.port = port;
        let cfg = Arc::new(cfg);

        let (shutdown_tx, _) = broadcast::channel(1);
        tokio::spawn(run(Arc::clone(&cfg), false, shutdown_tx.clone()));

        // Poll rather than sleep a fixed 150ms: under load (a parallel cargo
        // build, a busy CI runner) the listener isn't necessarily up yet, and a
        // single unretried request made this test flake.
        let url = format!("http://127.0.0.1:{port}/health");
        let ok = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if matches!(reqwest::get(&url).await, Ok(r) if r.status().as_u16() == 200) {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await;

        shutdown_tx.send(()).ok();
        assert!(ok.is_ok(), "/health did not return 200 within 10s");
    }

    /// An enabled listener whose port is taken must stop startup.
    ///
    /// It used to bind inside its own spawned task, so a clash produced one
    /// logged line and the daemon carried on without the service that had been
    /// explicitly turned on. For metrics that is the worse half: Prometheus
    /// reports a scrape target that never answers as the *monitored* host being
    /// down, so the failure is attributed to the wrong machine entirely.
    #[tokio::test]
    async fn run_fails_when_an_enabled_listener_port_is_taken() {
        for which in ["ui", "metrics"] {
            let tf = tempfile_with("mytoken");
            let port = free_port();
            let _squatter = std::net::TcpListener::bind(("127.0.0.1", port)).unwrap();

            let mut cfg = minimal_config(&tf);
            match which {
                "ui" => {
                    cfg.ui.enabled = true;
                    cfg.ui.port = port;
                }
                _ => {
                    cfg.metrics.enabled = true;
                    cfg.metrics.port = port;
                }
            }

            let (shutdown_tx, _) = broadcast::channel(1);
            let result = run(Arc::new(cfg), false, shutdown_tx).await;
            assert!(
                result.is_err(),
                "{which}: a listener that cannot bind must stop startup"
            );
        }
    }

    /// run() returns an error when InfluxWriter::new() fails (missing token file).
    #[tokio::test]
    async fn run_returns_error_on_missing_token_file() {
        let mut cfg = minimal_config(&tempfile_with("x"));
        cfg.influx.token_file = "/nonexistent/path/to/token.file".into();
        cfg.ui.enabled = false;
        let cfg = Arc::new(cfg);
        let (shutdown_tx, _) = broadcast::channel(1);

        let result = run(cfg, false, shutdown_tx).await;
        assert!(result.is_err(), "expected error for missing token file");
    }
}
