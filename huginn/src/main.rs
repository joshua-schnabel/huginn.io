use std::sync::Arc;

use anyhow::Context;
use chrono::Local;
use clap::Parser;
use colored::Colorize;
use huginn_core::config::{AppConfig, LogFormat};
use huginn_core::event::{EventHub, ProbeEvent};
use huginn_core::types::ProbeResult;
use huginn_influx::writer::{run_subscriber_batched, InfluxWriter};
use tokio::sync::broadcast;
use tracing::{error, info};
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

    /// Output format: pretty (default) or json
    #[arg(long, env = "HUGINN_LOG_FORMAT", default_value = "pretty")]
    output: String,
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    // Load config (applies ENV overrides internally)
    let cfg = AppConfig::load(&args.config)
        .with_context(|| format!("Failed to load config from '{}'", args.config))?;

    // Determine log format (CLI flag > ENV > config file)
    let use_json = args.output.to_lowercase() == "json" || cfg.log.format == LogFormat::Json;

    init_tracing(use_json, &cfg.log.level);

    info!("huginn starting — config: {}", args.config);

    // Shutdown channel — Ctrl+C sends the signal
    let (shutdown_tx, _): (Shutdown, _) = broadcast::channel(1);
    let shutdown_ctrl = shutdown_tx.clone();
    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.ok();
        info!("Shutdown signal received");
        let _ = shutdown_ctrl.send(());
    });

    run(Arc::new(cfg), use_json, shutdown_tx).await
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

    // Console output subscriber
    let console_hub = Arc::clone(&hub);
    tokio::spawn(async move {
        let mut rx = console_hub.subscribe();
        loop {
            match rx.recv().await {
                Ok(ProbeEvent::ProbeCompleted(result)) => print_result(&result, use_json),
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    error!("Console subscriber dropped {n} events");
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    // InfluxDB subscriber
    let writer =
        Arc::new(InfluxWriter::new(&cfg.influx).context("Failed to initialise InfluxDB writer")?);
    let influx_hub = Arc::clone(&hub);
    tokio::spawn(run_subscriber_batched(
        writer,
        influx_hub,
        cfg.influx.batch_size,
        cfg.influx.batch_timeout_ms,
    ));

    // Web UI subscriber (if enabled)
    if cfg.ui.enabled {
        let port = cfg.ui.port;
        let web_hub = Arc::clone(&hub);
        tokio::spawn(async move {
            if let Err(e) = huginn_web::server::run_server(port, web_hub).await {
                error!("Web UI error: {e}");
            }
        });
    }

    // Subscribe to shutdown BEFORE moving shutdown_tx into the scheduler
    let mut shutdown_rx = shutdown_tx.subscribe();

    // Start scheduler — publishes ProbeEvents to the hub
    scheduler::run(Arc::clone(&cfg), Arc::clone(&hub), shutdown_tx).await;

    // Block until a shutdown signal arrives; this keeps all spawned tasks alive.
    let _ = shutdown_rx.recv().await;

    // Drop hub — signals RecvError::Closed to all event subscribers
    drop(hub);

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
        AppConfig, InfluxConfig, LogConfig, ProbeConfig, ProbeType, UiConfig,
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

    /// Minimal config: 1 TCP probe → connection refused (instant fail), no UI.
    fn minimal_config(token_file: &tempfile::NamedTempFile) -> AppConfig {
        AppConfig {
            influx: InfluxConfig {
                url: "http://127.0.0.1:19999".into(),
                org: "o".into(),
                bucket: "b".into(),
                token_file: token_file.path().to_string_lossy().into_owned(),
                batch_size: 10,
                batch_timeout_ms: 1000,
            },
            probes: vec![ProbeConfig {
                name: "test-probe".into(),
                probe_type: ProbeType::Tcp,
                target: "127.0.0.1:1".into(),
                interval_secs: 1,
                timeout_secs: 1,
                expected_status: None,
                dns_query: None,
                dns_expected_ip: None,
            }],
            ui: UiConfig {
                enabled: false,
                port: 9900,
            },
            log: LogConfig::default(),
            event_hub_capacity: 256,
        }
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
