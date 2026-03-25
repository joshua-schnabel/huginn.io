use std::sync::Arc;

use anyhow::Context;
use axum::{extract::State, response::Html, routing::get, Json, Router};
use chrono::Local;
use clap::Parser;
use colored::Colorize;
use hugin_core::config::{AppConfig, LogFormat};
use hugin_core::types::ProbeResult;
use hugin_influx::writer::InfluxWriter;
use hugin_probes::scheduler;
use tokio::sync::{broadcast, RwLock};
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

// ---------------------------------------------------------------------------
// CLI args
// ---------------------------------------------------------------------------

#[derive(Parser, Debug)]
#[command(
    name = "hugin-dec",
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
// Shared state for debug UI
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct AppState {
    results: Arc<RwLock<Vec<ProbeResult>>>,
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

    info!("hugin-dec starting — config: {}", args.config);

    let cfg = Arc::new(cfg);
    let state = AppState {
        results: Arc::new(RwLock::new(Vec::new())),
    };

    // Start debug UI if enabled
    if cfg.ui.enabled {
        let port = cfg.ui.port;
        let ui_state = state.clone();
        tokio::spawn(async move {
            if let Err(e) = run_ui(port, ui_state).await {
                error!("Debug UI error: {e}");
            }
        });
        info!("Debug UI listening on http://0.0.0.0:{}", cfg.ui.port);
    }

    // Build InfluxDB writer
    let writer = Arc::new(
        InfluxWriter::new(&cfg.influx).context("Failed to initialise InfluxDB writer")?,
    );

    // Shutdown channel
    let (shutdown_tx, shutdown_rx) = broadcast::channel::<()>(1);
    tokio::spawn({
        let shutdown_tx = shutdown_tx.clone();
        async move {
            tokio::signal::ctrl_c().await.ok();
            info!("Shutdown signal received");
            let _ = shutdown_tx.send(());
        }
    });

    // Start scheduler — returns a broadcast sender of ProbeResults
    let result_tx = scheduler::run(Arc::clone(&cfg), shutdown_rx).await;
    let mut result_rx = result_tx.subscribe();

    // Process results: print to stdout + write to InfluxDB
    loop {
        match result_rx.recv().await {
            Ok(result) => {
                print_result(&result, use_json);
                {
                    let mut guard = state.results.write().await;
                    // Keep last result per probe (update or append)
                    if let Some(pos) = guard.iter().position(|r| r.probe_name == result.probe_name) {
                        guard[pos] = result.clone();
                    } else {
                        guard.push(result.clone());
                    }
                }
                let w = Arc::clone(&writer);
                tokio::spawn(async move {
                    if let Err(e) = w.write(&result).await {
                        error!("InfluxDB write error: {e}");
                    }
                });
            }
            Err(broadcast::error::RecvError::Lagged(n)) => {
                error!("Dropped {n} probe results (channel lagged)");
            }
            Err(broadcast::error::RecvError::Closed) => {
                info!("Result channel closed — shutting down");
                break;
            }
        }
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
// Debug web UI
// ---------------------------------------------------------------------------

async fn run_ui(port: u16, state: AppState) -> anyhow::Result<()> {
    let app = Router::new()
        .route("/", get(ui_index))
        .route("/health", get(ui_health))
        .route("/metrics/latest", get(ui_metrics))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(("0.0.0.0", port)).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn ui_health() -> &'static str {
    "OK"
}

async fn ui_metrics(State(s): State<AppState>) -> Json<Vec<ProbeResult>> {
    Json(s.results.read().await.clone())
}

async fn ui_index(State(s): State<AppState>) -> Html<String> {
    let results = s.results.read().await;
    let rows: String = results
        .iter()
        .map(|r| {
            let status = if r.up {
                "<td style='color:#22c55e'>✅ UP</td>"
            } else {
                "<td style='color:#ef4444'>❌ DOWN</td>"
            };
            let err = r.error.as_deref().unwrap_or("-");
            format!(
                "<tr><td>{}</td><td>{}</td><td>{}</td>{}<td>{:.1}ms</td><td>{}</td></tr>",
                r.probe_name, r.probe_type, r.target, status, r.response_ms, err
            )
        })
        .collect();

    Html(format!(
        r#"<!DOCTYPE html><html><head><meta charset="utf-8">
<meta http-equiv="refresh" content="5">
<title>hugin.dec</title>
<style>
  body{{font-family:monospace;background:#0f172a;color:#e2e8f0;padding:2rem}}
  h1{{color:#38bdf8}}
  table{{border-collapse:collapse;width:100%}}
  th,td{{text-align:left;padding:.4rem .8rem;border-bottom:1px solid #1e293b}}
  th{{color:#94a3b8}}
</style></head><body>
<h1>🦅 hugin.dec</h1>
<p style="color:#64748b">Auto-refresh every 5s &nbsp;|&nbsp;
  <a href="/metrics/latest" style="color:#38bdf8">JSON</a> &nbsp;|&nbsp;
  <a href="/health" style="color:#38bdf8">health</a></p>
<table>
<tr><th>Probe</th><th>Type</th><th>Target</th><th>Status</th><th>Response</th><th>Error</th></tr>
{rows}
</table></body></html>"#
    ))
}
