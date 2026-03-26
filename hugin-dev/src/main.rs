use std::sync::Arc;

use anyhow::Context;
use chrono::Local;
use clap::Parser;
use colored::Colorize;
use hugin_core::config::{AppConfig, LogFormat};
use hugin_core::event::{EventHub, ProbeEvent};
use hugin_core::types::ProbeResult;
use hugin_influx::writer::{run_subscriber_batched, InfluxWriter};
use hugin_probes::scheduler;
use tokio::sync::broadcast;
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

// ---------------------------------------------------------------------------
// CLI args
// ---------------------------------------------------------------------------

#[derive(Parser, Debug)]
#[command(
    name = "hugin-dev",
    version,
    about = "Uptime & response-time monitor — InfluxDB backend"
)]
struct Args {
    /// Path to the YAML config file
    #[arg(short, long, env = "HUGIN_CONFIG", default_value = "/etc/hugin/config.yaml")]
    config: String,

    /// Output format: pretty (default) or json
    #[arg(long, env = "HUGIN_LOG_FORMAT", default_value = "pretty")]
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
    let use_json = args.output.to_lowercase() == "json"
        || cfg.log.format == LogFormat::Json;

    // Initialise tracing
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(&cfg.log.level));

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

    info!("hugin-dev starting — config: {}", args.config);

    // Shutdown channel — Ctrl+C sends the signal
    let (shutdown_tx, _) = broadcast::channel::<()>(1);
    let shutdown_tx_ctrl = shutdown_tx.clone();
    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.ok();
        info!("Shutdown signal received");
        let _ = shutdown_tx_ctrl.send(());
    });

    run(Arc::new(cfg), use_json, shutdown_tx).await
}

// ---------------------------------------------------------------------------
// Core wiring — extracted for testability
// ---------------------------------------------------------------------------

pub(crate) async fn run(
    cfg: Arc<AppConfig>,
    use_json: bool,
    shutdown_tx: broadcast::Sender<()>,
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
    let writer = Arc::new(
        InfluxWriter::new(&cfg.influx).context("Failed to initialise InfluxDB writer")?,
    );
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
            if let Err(e) = hugin_web::server::run_server(port, web_hub).await {
                error!("Web UI error: {e}");
            }
        });
    }

    // Start scheduler — publishes ProbeEvents to the hub
    scheduler::run(Arc::clone(&cfg), Arc::clone(&hub), shutdown_tx).await;

    // Wait until the hub is dropped (scheduler exits on shutdown, hub closes)
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
    use hugin_core::config::{AppConfig, InfluxConfig, LogConfig, ProbeConfig, ProbeType, UiConfig};
    use hugin_core::types::ProbeResult;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::sync::broadcast;

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
            ui: UiConfig { enabled: false, port: 9900 },
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

    /// run() starts, fires one probe, then shuts down cleanly.
    #[tokio::test]
    async fn run_exits_cleanly_on_shutdown() {
        let tf = tempfile_with("mytoken");
        let cfg = Arc::new(minimal_config(&tf));
        let (shutdown_tx, _) = broadcast::channel::<()>(1);

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
        let (shutdown_tx, _) = broadcast::channel::<()>(1);

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

        let (shutdown_tx, _) = broadcast::channel::<()>(1);
        tokio::spawn(run(Arc::clone(&cfg), false, shutdown_tx.clone()));

        tokio::time::sleep(Duration::from_millis(150)).await;

        let resp = reqwest::get(format!("http://127.0.0.1:{port}/health"))
            .await
            .expect("health request failed");
        assert_eq!(resp.status().as_u16(), 200);

        shutdown_tx.send(()).ok();
    }

    /// run() returns an error when InfluxWriter::new() fails (missing token file).
    #[tokio::test]
    async fn run_returns_error_on_missing_token_file() {
        let mut cfg = minimal_config(&tempfile_with("x"));
        cfg.influx.token_file = "/nonexistent/path/to/token.file".into();
        cfg.ui.enabled = false;
        let cfg = Arc::new(cfg);
        let (shutdown_tx, _) = broadcast::channel::<()>(1);

        let result = run(cfg, false, shutdown_tx).await;
        assert!(result.is_err(), "expected error for missing token file");
    }
}
