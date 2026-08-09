use std::sync::Arc;

use huginn_core::config::{AppConfig, ProbeConfig};
use huginn_core::event::{EventHub, ProbeEvent};
use huginn_core::types::ProbeResult;
use tokio::sync::broadcast;
use tokio::time::interval;
use tracing::{error, info};

use huginn_probes::ProbeRegistry;

use crate::Shutdown;

/// Run all probes from the config, each on its own tokio interval.
/// Results are published to the given `EventHub` as `ProbeEvent::ProbeCompleted`.
/// Each probe loop listens on its own shutdown receiver and exits cleanly.
///
/// Returns a handle per probe loop. The caller **must** wait on these before it
/// drops its `Arc<EventHub>`, because each loop holds one too: dropping only the
/// caller's clone leaves the hub's `Sender` alive, the batcher never observes
/// `Closed`, and the partial batch it is holding is never queued. The shutdown
/// drain would then time out and blame an InfluxDB that was reachable all along.
/// The handles are what make "every publisher has stopped" observable rather
/// than assumed.
#[must_use = "the caller must join these before dropping its EventHub clone, or \
              the batcher never sees the hub close"]
pub async fn run(
    cfg: Arc<AppConfig>,
    hub: Arc<EventHub>,
    shutdown_tx: Shutdown,
) -> Vec<tokio::task::JoinHandle<()>> {
    // One registry for every loop. It owns the shared HTTP client (and whatever
    // future probes need), so loops that don't speak HTTP no longer carry one.
    let registry = Arc::new(ProbeRegistry::new());

    let mut handles = Vec::with_capacity(cfg.probes.len());
    for probe_cfg in &cfg.probes {
        let probe_cfg = probe_cfg.clone();
        let hub = Arc::clone(&hub);
        let registry = Arc::clone(&registry);
        let shutdown_rx = shutdown_tx.subscribe();

        handles.push(tokio::spawn(run_probe_loop(
            probe_cfg,
            hub,
            registry,
            shutdown_rx,
        )));
    }
    handles
}

async fn run_probe_loop(
    cfg: ProbeConfig,
    hub: Arc<EventHub>,
    registry: Arc<ProbeRegistry>,
    mut shutdown_rx: broadcast::Receiver<()>,
) {
    let mut ticker = interval(cfg.interval());
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        // Two sequential `select!`s rather than one wrapping the probe, so that
        // shutdown is observed *during* a probe and not only between two of
        // them. It used to be one: the timer arm was chosen, and the loop then
        // awaited the whole probe before it looked at the shutdown channel
        // again. An SMTP or IMAP probe can spend a full timeout connecting and
        // another reading, so a loop could hold the process — and the hub —
        // open for twice `timeout_secs` after the signal.
        //
        // Sequential also because they cannot be nested: both arms need
        // `&mut shutdown_rx`, and inside a `select!` body the borrow taken by
        // its own arms is still live. Splitting them ends the first borrow
        // before the second begins.
        tokio::select! {
            _ = ticker.tick() => {}
            _ = shutdown_rx.recv() => {
                info!(probe = %cfg.name, "probe loop shutting down");
                break;
            }
        }

        tokio::select! {
            result = execute_probe(&cfg, &registry) => {
                hub.publish(ProbeEvent::ProbeCompleted(result));
            }
            // The in-flight probe is dropped at its next await point. Nothing is
            // published for it, which is right: an interrupted probe measured
            // nothing, and inventing a DOWN result here would write a fake
            // outage into InfluxDB on every deploy.
            _ = shutdown_rx.recv() => {
                info!(probe = %cfg.name, "probe loop shutting down mid-probe");
                break;
            }
        }
    }
}

/// Run one probe and log the outcome. Dispatch lives in the registry, in the
/// crate that owns the probes.
async fn execute_probe(cfg: &ProbeConfig, registry: &ProbeRegistry) -> ProbeResult {
    let result = registry.get(&cfg.probe_type).probe(cfg).await;

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
    use huginn_core::config::{
        InfluxConfig, LogConfig, MetricsConfig, ProbeConfig, ProbeType, UiConfig,
    };
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
                ..Default::default()
            },
            probes,
            ui: UiConfig::default(),
            metrics: MetricsConfig::default(),
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
            ..Default::default()
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
        let _handles = run(cfg, hub.clone(), shutdown_tx.clone()).await;

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
        let _handles = run(cfg, hub.clone(), shutdown_tx.clone()).await;

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
        let _handles = run(cfg, hub.clone(), shutdown_tx.clone()).await;

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
            ..Default::default()
        }]);

        let hub = Arc::new(EventHub::new(256));
        let mut rx = hub.subscribe();
        let (shutdown_tx, _) = broadcast::channel(1);
        let _handles = run(cfg, hub.clone(), shutdown_tx.clone()).await;

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
            ..Default::default()
        }]);

        let hub = Arc::new(EventHub::new(256));
        let mut rx = hub.subscribe();
        let (shutdown_tx, _) = broadcast::channel(1);
        let _handles = run(cfg, hub.clone(), shutdown_tx.clone()).await;

        let event = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .expect("timed out waiting for udp probe event")
            .expect("channel closed");

        let ProbeEvent::ProbeCompleted(result) = event;
        assert_eq!(result.probe_name, "sched-udp");
        assert!(result.up, "error: {:?}", result.error);

        drop(shutdown_tx);
    }

    /// Question section only — see `huginn_probes::dns` tests: the resolver
    /// appends an EDNS(0) OPT record, and copying it into a response that
    /// declares ARCOUNT=0 misplaces the answer.
    fn question_section(query: &[u8]) -> &[u8] {
        let mut i = 12;
        while i < query.len() && query[i] != 0 {
            i += 1 + query[i] as usize;
        }
        i += 1 + 4;
        &query[12..i.min(query.len())]
    }

    fn build_dns_a_response(query: &[u8], ip: [u8; 4]) -> Vec<u8> {
        let mut r = Vec::new();
        r.extend_from_slice(&query[0..2]);
        r.extend_from_slice(&[0x81, 0x80]);
        r.extend_from_slice(&[0x00, 0x01]);
        r.extend_from_slice(&[0x00, 0x01]);
        r.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]);
        r.extend_from_slice(question_section(query));
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
            dns_query: Some("example.com".into()),
            ..Default::default()
        }]);

        let hub = Arc::new(EventHub::new(256));
        let mut rx = hub.subscribe();
        let (shutdown_tx, _) = broadcast::channel(1);
        let _handles = run(cfg, hub.clone(), shutdown_tx.clone()).await;

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
