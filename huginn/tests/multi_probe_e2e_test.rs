/// E2E test: multiple probe types all emit results into the web UI.
///
/// Uses `huginn_web::server::run_server` directly — no real InfluxDB needed.
/// Three probe result types (http, tcp, dns) are injected via the EventHub
/// and then verified via GET /metrics/latest.
use huginn_core::event::{EventHub, ProbeEvent};
use huginn_core::types::ProbeResult;
use std::sync::Arc;
use std::time::Duration;

#[tokio::test]
async fn multiple_probes_all_appear_in_metrics() {
    let port = free_port();
    let hub = start_server(port).await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Inject results for three different probe types
    for (name, kind, target) in [
        ("http-check", "http", "https://example.com"),
        ("tcp-check", "tcp", "db.internal:5432"),
        ("dns-check", "dns", "8.8.8.8:53"),
    ] {
        hub.publish(ProbeEvent::ProbeCompleted(ProbeResult::success(
            name, kind, target, 10.0, None,
        )));
    }

    tokio::time::sleep(Duration::from_millis(100)).await;

    let resp = reqwest::get(format!("http://127.0.0.1:{port}/metrics/latest"))
        .await
        .expect("request failed");

    assert_eq!(resp.status().as_u16(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    let arr = body.as_array().expect("expected JSON array");

    assert_eq!(arr.len(), 3, "expected 3 results, got {}: {body}", arr.len());

    let names: Vec<&str> = arr.iter().filter_map(|v| v["probe_name"].as_str()).collect();
    assert!(names.contains(&"http-check"), "http-check missing: {names:?}");
    assert!(names.contains(&"tcp-check"), "tcp-check missing: {names:?}");
    assert!(names.contains(&"dns-check"), "dns-check missing: {names:?}");

    drop(hub);
}

#[tokio::test]
async fn all_probes_report_correct_probe_type_field() {
    let port = free_port();
    let hub = start_server(port).await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    hub.publish(ProbeEvent::ProbeCompleted(ProbeResult::success(
        "smtp-mon", "smtp", "mail.example.com:25", 30.0, None,
    )));
    hub.publish(ProbeEvent::ProbeCompleted(ProbeResult::success(
        "imap-mon", "imap", "mail.example.com:143", 28.0, None,
    )));
    hub.publish(ProbeEvent::ProbeCompleted(ProbeResult::success(
        "udp-mon", "udp", "8.8.8.8:53", 2.0, None,
    )));

    tokio::time::sleep(Duration::from_millis(100)).await;

    let body: serde_json::Value =
        reqwest::get(format!("http://127.0.0.1:{port}/metrics/latest"))
            .await
            .unwrap()
            .json()
            .await
            .unwrap();

    let arr = body.as_array().unwrap();
    assert_eq!(arr.len(), 3);

    for item in arr {
        let probe_type = item["probe_type"].as_str().unwrap();
        assert!(
            ["smtp", "imap", "udp"].contains(&probe_type),
            "unexpected probe_type: {probe_type}"
        );
    }

    drop(hub);
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

async fn start_server(port: u16) -> Arc<EventHub> {
    let hub = Arc::new(EventHub::new(256));
    let hub_clone = Arc::clone(&hub);
    tokio::spawn(async move {
        huginn_web::server::run_server(port, hub_clone).await.ok();
    });
    hub
}

fn free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().port()
}
