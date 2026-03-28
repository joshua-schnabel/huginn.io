/// E2E test: graceful shutdown — the scheduler and all probe loops exit cleanly.
///
/// Uses `huginn_probes::scheduler::run` directly with a live TCP echo server.
/// Sends a shutdown signal and verifies the EventHub closes within 2 seconds.
use huginn_core::config::{AppConfig, InfluxConfig, LogConfig, ProbeConfig, ProbeType, UiConfig};
use huginn_core::event::{EventHub, ProbeEvent};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast;

/// Start a scheduler with two probes, wait for first results, then shut down.
/// Test fails if the scheduler doesn't stop within 2 seconds (deadlock guard).
#[tokio::test]
async fn graceful_shutdown_completes_within_deadline() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let _ = listener.accept().await;
        }
    });

    let cfg = Arc::new(AppConfig {
        influx: InfluxConfig {
            url: "http://localhost:8086".into(),
            org: "test".into(),
            bucket: "test".into(),
            token_file: "/dev/null".into(),
            batch_size: 10,
            batch_timeout_ms: 1000,
        },
        probes: vec![
            ProbeConfig {
                name: "probe-a".into(),
                probe_type: ProbeType::Tcp,
                target: addr.to_string(),
                interval_secs: 60, // long interval — only fires once on start
                timeout_secs: 1,
                expected_status: None,
                dns_query: None,
                dns_expected_ip: None,
            },
            ProbeConfig {
                name: "probe-b".into(),
                probe_type: ProbeType::Tcp,
                target: addr.to_string(),
                interval_secs: 60,
                timeout_secs: 1,
                expected_status: None,
                dns_query: None,
                dns_expected_ip: None,
            },
        ],
        ui: UiConfig::default(),
        log: LogConfig::default(),
        event_hub_capacity: 256,
    });

    let hub = Arc::new(EventHub::new(256));
    let mut rx = hub.subscribe();
    let (shutdown_tx, _) = broadcast::channel::<()>(1);

    let hub_clone = Arc::clone(&hub);
    let cfg_clone = Arc::clone(&cfg);
    let tx_clone = shutdown_tx.clone();
    tokio::spawn(async move {
        huginn_probes::scheduler::run(cfg_clone, hub_clone, tx_clone).await;
    });

    // Wait for both probes to fire at least once
    let mut seen = std::collections::HashSet::new();
    tokio::time::timeout(Duration::from_secs(5), async {
        while seen.len() < 2 {
            if let Ok(ProbeEvent::ProbeCompleted(r)) = rx.recv().await {
                seen.insert(r.probe_name.clone());
            }
        }
    })
    .await
    .expect("timed out waiting for initial probe results");

    // Send shutdown, drop the hub
    shutdown_tx.send(()).unwrap();
    drop(hub);

    // Verify channel closes within 2 seconds — no deadlock
    let closed = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            match rx.recv().await {
                Err(broadcast::error::RecvError::Closed) => return true,
                _ => {}
            }
        }
    })
    .await;

    assert!(
        closed.is_ok(),
        "Scheduler probe loops did not exit within 2 seconds after shutdown signal"
    );
}

/// Shutdown signal sent before any probes have fired — must still exit cleanly.
#[tokio::test]
async fn immediate_shutdown_before_first_probe() {
    let cfg = Arc::new(AppConfig {
        influx: InfluxConfig {
            url: "http://localhost:8086".into(),
            org: "test".into(),
            bucket: "test".into(),
            token_file: "/dev/null".into(),
            batch_size: 10,
            batch_timeout_ms: 1000,
        },
        probes: vec![ProbeConfig {
            name: "slow-probe".into(),
            probe_type: ProbeType::Tcp,
            target: "127.0.0.1:65534".into(), // port likely closed — fast fail
            interval_secs: 1,
            timeout_secs: 1,
            expected_status: None,
            dns_query: None,
            dns_expected_ip: None,
        }],
        ui: UiConfig::default(),
        log: LogConfig::default(),
        event_hub_capacity: 256,
    });

    let hub = Arc::new(EventHub::new(256));
    let mut rx = hub.subscribe();
    let (shutdown_tx, _) = broadcast::channel::<()>(1);

    let hub_clone = Arc::clone(&hub);
    let cfg_clone = Arc::clone(&cfg);
    let tx_clone = shutdown_tx.clone();
    tokio::spawn(async move {
        huginn_probes::scheduler::run(cfg_clone, hub_clone, tx_clone).await;
    });

    // Shut down immediately — before giving the probe a chance to fire
    tokio::task::yield_now().await;
    shutdown_tx.send(()).unwrap();
    drop(hub);

    let closed = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            match rx.recv().await {
                Err(broadcast::error::RecvError::Closed) => return true,
                _ => {}
            }
        }
    })
    .await;

    assert!(
        closed.is_ok(),
        "Scheduler did not exit after immediate shutdown"
    );
}
