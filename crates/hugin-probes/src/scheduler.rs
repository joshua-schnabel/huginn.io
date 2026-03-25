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
/// Each probe loop listens on its own shutdown receiver and exits cleanly.
pub async fn run(
    cfg: Arc<AppConfig>,
    shutdown_tx: broadcast::Sender<()>,
) -> ResultSender {
    let (tx, _) = broadcast::channel::<ProbeResult>(256);
    let http_client = Arc::new(http::build_client());

    for probe_cfg in &cfg.probes {
        let probe_cfg = probe_cfg.clone();
        let result_tx = tx.clone();
        let http_client = Arc::clone(&http_client);
        let shutdown_rx = shutdown_tx.subscribe();

        tokio::spawn(run_probe_loop(probe_cfg, result_tx, http_client, shutdown_rx));
    }

    tx
}

async fn run_probe_loop(
    cfg: ProbeConfig,
    tx: ResultSender,
    http_client: Arc<reqwest::Client>,
    mut shutdown_rx: broadcast::Receiver<()>,
) {
    let mut ticker = interval(cfg.interval());
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            _ = ticker.tick() => {
                let result = execute_probe(&cfg, &http_client).await;
                if tx.send(result).is_err() {
                    // No receivers — process shutting down
                    break;
                }
            }
            _ = shutdown_rx.recv() => {
                info!(probe = %cfg.name, "probe loop shutting down");
                break;
            }
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

    fn tcp_probe(addr: &str, interval_secs: u64) -> ProbeConfig {
        ProbeConfig {
            name: "sched-tcp".into(),
            probe_type: ProbeType::Tcp,
            target: addr.to_string(),
            interval_secs,
            timeout_secs: 2,
            expected_status: None,
        }
    }

    #[tokio::test]
    async fn scheduler_emits_probe_result() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { loop { let _ = listener.accept().await; } });

        let cfg = make_config(vec![tcp_probe(&addr.to_string(), 1)]);
        let (shutdown_tx, _) = broadcast::channel(1);
        let result_tx = run(cfg, shutdown_tx.clone()).await;
        let mut result_rx = result_tx.subscribe();

        let result = tokio::time::timeout(Duration::from_secs(3), result_rx.recv())
            .await
            .expect("timed out waiting for probe result")
            .expect("channel closed");

        assert_eq!(result.probe_name, "sched-tcp");
        assert!(result.up);
    }

    /// TDD: Shutdown signal must cause probe loops to exit cleanly.
    /// Write this test FIRST — it should FAIL until run_probe_loop uses tokio::select!
    #[tokio::test]
    async fn scheduler_stops_on_shutdown() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { loop { let _ = listener.accept().await; } });

        let cfg = make_config(vec![tcp_probe(&addr.to_string(), 60)]); // long interval
        let (shutdown_tx, _) = broadcast::channel(1);
        let result_tx = run(cfg, shutdown_tx.clone()).await;
        let mut result_rx = result_tx.subscribe();

        // Get the first result (fires immediately on start)
        let _ = tokio::time::timeout(Duration::from_secs(3), result_rx.recv())
            .await
            .expect("no initial result");

        // Send shutdown — probe loops must exit, channel must close
        shutdown_tx.send(()).unwrap();

        // Drop all senders so the channel closes
        drop(result_tx);

        // After shutdown, receiving should eventually return Closed
        let outcome = tokio::time::timeout(
            Duration::from_secs(2),
            async { loop {
                match result_rx.recv().await {
                    Err(broadcast::error::RecvError::Closed) => return true,
                    _ => {}
                }
            }}
        ).await;

        assert!(outcome.is_ok(), "probe loops did not exit after shutdown signal");
    }
}

