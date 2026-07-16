use std::sync::Arc;

use huginn_core::config::{AppConfig, ProbeConfig, ProbeType};
use huginn_core::event::{EventHub, ProbeEvent};
use huginn_core::types::ProbeResult;
use tokio::sync::broadcast;
use tokio::time::interval;
use tracing::{error, info};

use huginn_probes::{dns, http, imap, smtp, tcp, udp};

use crate::Shutdown;

/// Run all probes from the config, each on its own tokio interval.
/// Results are published to the given `EventHub` as `ProbeEvent::ProbeCompleted`.
/// Each probe loop listens on its own shutdown receiver and exits cleanly.
pub async fn run(cfg: Arc<AppConfig>, hub: Arc<EventHub>, shutdown_tx: Shutdown) {
    let http_client = Arc::new(http::build_client());

    for probe_cfg in &cfg.probes {
        let probe_cfg = probe_cfg.clone();
        let hub = Arc::clone(&hub);
        let http_client = Arc::clone(&http_client);
        let shutdown_rx = shutdown_tx.subscribe();

        tokio::spawn(run_probe_loop(probe_cfg, hub, http_client, shutdown_rx));
    }
}

async fn run_probe_loop(
    cfg: ProbeConfig,
    hub: Arc<EventHub>,
    http_client: Arc<reqwest::Client>,
    mut shutdown_rx: broadcast::Receiver<()>,
) {
    let mut ticker = interval(cfg.interval());
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            _ = ticker.tick() => {
                let result = execute_probe(&cfg, &http_client).await;
                hub.publish(ProbeEvent::ProbeCompleted(result));
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
        ProbeType::Dns => dns::probe(cfg).await,
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
    use huginn_core::config::{InfluxConfig, LogConfig, ProbeConfig, ProbeType, UiConfig};
    use huginn_core::event::{EventHub, ProbeEvent};
    use std::time::Duration;
    use tokio::net::TcpListener;
    use tokio::sync::broadcast;

    fn make_config(probes: Vec<ProbeConfig>) -> Arc<AppConfig> {
        Arc::new(AppConfig {
            influx: InfluxConfig {
                url: "http://localhost:8086".into(),
                org: "test".into(),
                bucket: "test".into(),
                token_file: "/dev/null".into(),
                batch_size: 10,
                batch_timeout_ms: 1000,
            },
            probes,
            ui: UiConfig::default(),
            log: LogConfig::default(),
            event_hub_capacity: 256,
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
            dns_query: None,
            dns_expected_ip: None,
        }
    }

    #[tokio::test]
    async fn scheduler_emits_probe_result() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let _ = listener.accept().await;
            }
        });

        let cfg = make_config(vec![tcp_probe(&addr.to_string(), 1)]);
        let hub = Arc::new(EventHub::new(256));
        let mut rx = hub.subscribe();
        let (shutdown_tx, _) = broadcast::channel(1);
        run(cfg, hub.clone(), shutdown_tx.clone()).await;

        let event = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .expect("timed out waiting for probe event")
            .expect("channel closed");

        let ProbeEvent::ProbeCompleted(result) = event;
        assert_eq!(result.probe_name, "sched-tcp");
        assert!(result.up);

        drop(shutdown_tx);
    }

    #[tokio::test]
    async fn immediate_shutdown_before_first_probe() {
        let cfg = make_config(vec![tcp_probe("127.0.0.1:65534", 1)]);
        let hub = Arc::new(EventHub::new(256));
        let mut rx = hub.subscribe();
        let (shutdown_tx, _) = broadcast::channel(1);
        run(cfg, hub.clone(), shutdown_tx.clone()).await;

        // Shut down immediately without waiting for any probe to fire
        tokio::task::yield_now().await;
        shutdown_tx.send(()).unwrap();
        drop(hub);

        let closed = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if let Err(broadcast::error::RecvError::Closed) = rx.recv().await {
                    return true;
                }
            }
        })
        .await;

        assert!(
            closed.is_ok(),
            "Scheduler did not exit after immediate shutdown"
        );
    }

    #[tokio::test]
    async fn graceful_shutdown_completes_within_deadline() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let _ = listener.accept().await;
            }
        });

        let cfg = make_config(vec![tcp_probe(&addr.to_string(), 60)]); // long interval
        let hub = Arc::new(EventHub::new(256));
        let mut rx = hub.subscribe();
        let (shutdown_tx, _) = broadcast::channel(1);
        run(cfg, hub.clone(), shutdown_tx.clone()).await;

        let _ = tokio::time::timeout(Duration::from_secs(3), rx.recv())
            .await
            .expect("no initial result");

        shutdown_tx.send(()).unwrap();
        drop(hub);

        let outcome = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if let Err(broadcast::error::RecvError::Closed) = rx.recv().await {
                    return true;
                }
            }
        })
        .await;

        assert!(
            outcome.is_ok(),
            "probe loops did not exit after shutdown signal"
        );
    }

    #[tokio::test]
    async fn scheduler_emits_http_probe_result() {
        use wiremock::matchers::method;
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        let cfg = make_config(vec![ProbeConfig {
            name: "sched-http".into(),
            probe_type: ProbeType::Http,
            target: server.uri(),
            interval_secs: 1,
            timeout_secs: 2,
            expected_status: Some(200),
            dns_query: None,
            dns_expected_ip: None,
        }]);

        let hub = Arc::new(EventHub::new(256));
        let mut rx = hub.subscribe();
        let (shutdown_tx, _) = broadcast::channel(1);
        run(cfg, hub.clone(), shutdown_tx.clone()).await;

        let event = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .expect("timed out waiting for http probe event")
            .expect("channel closed");

        let ProbeEvent::ProbeCompleted(result) = event;
        assert_eq!(result.probe_name, "sched-http");
        assert!(result.up, "error: {:?}", result.error);

        drop(shutdown_tx);
    }

    #[tokio::test]
    async fn scheduler_emits_udp_probe_result() {
        use tokio::net::UdpSocket;

        let server = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let addr = server.local_addr().unwrap();
        tokio::spawn(async move {
            let mut buf = [0u8; 512];
            loop {
                if let Ok((n, peer)) = server.recv_from(&mut buf).await {
                    let _ = server.send_to(&buf[..n], peer).await;
                }
            }
        });

        let cfg = make_config(vec![ProbeConfig {
            name: "sched-udp".into(),
            probe_type: ProbeType::Udp,
            target: addr.to_string(),
            interval_secs: 1,
            timeout_secs: 2,
            expected_status: None,
            dns_query: None,
            dns_expected_ip: None,
        }]);

        let hub = Arc::new(EventHub::new(256));
        let mut rx = hub.subscribe();
        let (shutdown_tx, _) = broadcast::channel(1);
        run(cfg, hub.clone(), shutdown_tx.clone()).await;

        let event = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .expect("timed out waiting for udp probe event")
            .expect("channel closed");

        let ProbeEvent::ProbeCompleted(result) = event;
        assert_eq!(result.probe_name, "sched-udp");
        assert!(result.up, "error: {:?}", result.error);

        drop(shutdown_tx);
    }

    fn build_dns_a_response(query: &[u8], ip: [u8; 4]) -> Vec<u8> {
        let mut r = Vec::new();
        r.extend_from_slice(&query[0..2]);
        r.extend_from_slice(&[0x81, 0x80]);
        r.extend_from_slice(&[0x00, 0x01]);
        r.extend_from_slice(&[0x00, 0x01]);
        r.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]);
        r.extend_from_slice(&query[12..]);
        r.extend_from_slice(&[0xC0, 0x0C, 0x00, 0x01, 0x00, 0x01]);
        r.extend_from_slice(&[0x00, 0x00, 0x00, 0x3C, 0x00, 0x04]);
        r.extend_from_slice(&ip);
        r
    }

    #[tokio::test]
    async fn scheduler_emits_dns_probe_result() {
        use tokio::net::UdpSocket;

        let server = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let addr = server.local_addr().unwrap();
        tokio::spawn(async move {
            let mut buf = [0u8; 512];
            loop {
                if let Ok((n, peer)) = server.recv_from(&mut buf).await {
                    let resp = build_dns_a_response(&buf[..n], [1, 2, 3, 4]);
                    let _ = server.send_to(&resp, peer).await;
                }
            }
        });

        let cfg = make_config(vec![ProbeConfig {
            name: "sched-dns".into(),
            probe_type: ProbeType::Dns,
            target: addr.to_string(),
            interval_secs: 1,
            timeout_secs: 2,
            expected_status: None,
            dns_query: Some("example.com".into()),
            dns_expected_ip: None,
        }]);

        let hub = Arc::new(EventHub::new(256));
        let mut rx = hub.subscribe();
        let (shutdown_tx, _) = broadcast::channel(1);
        run(cfg, hub.clone(), shutdown_tx.clone()).await;

        let event = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .expect("timed out waiting for dns probe event")
            .expect("channel closed");

        let ProbeEvent::ProbeCompleted(result) = event;
        assert_eq!(result.probe_name, "sched-dns");
        assert!(result.up, "error: {:?}", result.error);

        drop(shutdown_tx);
    }
}
