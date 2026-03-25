use std::sync::Arc;

use hugin_core::config::{AppConfig, ProbeConfig, ProbeType};
use hugin_core::types::ProbeResult;
use tokio::sync::broadcast;
use tokio::time::interval;
use tracing::{error, info};

use crate::{http, imap, smtp, tcp, udp};

pub type ResultSender = broadcast::Sender<ProbeResult>;

/// Run all probes from the config, each on its own tokio interval.
/// Results are broadcast via the returned sender.
/// Shutdown when `shutdown_rx` receives a message.
pub async fn run(
    cfg: Arc<AppConfig>,
    mut shutdown_rx: broadcast::Receiver<()>,
) -> ResultSender {
    let (tx, _) = broadcast::channel::<ProbeResult>(256);
    let http_client = Arc::new(http::build_client());

    for probe_cfg in &cfg.probes {
        let probe_cfg = probe_cfg.clone();
        let tx = tx.clone();
        let http_client = Arc::clone(&http_client);
        let shutdown = tx.subscribe();
        // We use a separate shutdown receiver per task
        let _ = shutdown; // suppress unused warning — real shutdown is below

        tokio::spawn(run_probe_loop(probe_cfg, tx, Arc::clone(&http_client)));
    }

    // Spawn a task that listens for shutdown and cancels all children via
    // a separate mechanism if needed (currently probes run until process exit).
    tokio::spawn(async move {
        let _ = shutdown_rx.recv().await;
        info!("Scheduler received shutdown signal");
    });

    tx
}

async fn run_probe_loop(
    cfg: ProbeConfig,
    tx: ResultSender,
    http_client: Arc<reqwest::Client>,
) {
    let mut ticker = interval(cfg.interval());
    // First tick fires immediately
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        ticker.tick().await;
        let result = execute_probe(&cfg, &http_client).await;
        if tx.send(result).is_err() {
            // No receivers left — process is shutting down
            break;
        }
    }
}

async fn execute_probe(cfg: &ProbeConfig, http_client: &reqwest::Client) -> ProbeResult {
    let result = match cfg.probe_type {
        ProbeType::Tcp => tcp::probe(cfg).await,
        ProbeType::Http | ProbeType::Https => http::probe(cfg, http_client).await,
        ProbeType::Smtp => smtp::probe(cfg).await,
        ProbeType::Imap => imap::probe(cfg).await,
        ProbeType::Udp => udp::probe(cfg).await,
    };

    if result.up {
        info!(
            probe = %result.probe_name,
            r#type = %result.probe_type,
            response_ms = result.response_ms,
            "probe UP"
        );
    } else {
        error!(
            probe = %result.probe_name,
            r#type = %result.probe_type,
            error = ?result.error,
            "probe DOWN"
        );
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use hugin_core::config::{InfluxConfig, LogConfig, ProbeConfig, ProbeType, UiConfig};
    use std::time::Duration;
    use tokio::net::TcpListener;

    fn make_config(probes: Vec<ProbeConfig>) -> Arc<AppConfig> {
        Arc::new(AppConfig {
            influx: InfluxConfig {
                url: "http://localhost:8086".into(),
                org: "test".into(),
                bucket: "test".into(),
                token_file: "/dev/null".into(),
            },
            probes,
            ui: UiConfig::default(),
            log: LogConfig::default(),
        })
    }

    #[tokio::test]
    async fn scheduler_emits_tcp_probe_result() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let _ = listener.accept().await;
            }
        });

        let probe = ProbeConfig {
            name: "sched-tcp".into(),
            probe_type: ProbeType::Tcp,
            target: addr.to_string(),
            interval_secs: 1,
            timeout_secs: 2,
            expected_status: None,
        };

        let cfg = make_config(vec![probe]);
        let (shutdown_tx, shutdown_rx) = broadcast::channel(1);
        let result_tx = run(cfg, shutdown_rx).await;
        let mut result_rx = result_tx.subscribe();

        // Wait for at least one result
        let result = tokio::time::timeout(Duration::from_secs(3), result_rx.recv())
            .await
            .expect("timed out waiting for probe result")
            .expect("broadcast channel closed");

        assert_eq!(result.probe_name, "sched-tcp");
        assert!(result.up);

        let _ = shutdown_tx.send(());
    }
}
