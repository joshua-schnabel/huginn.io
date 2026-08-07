/// E2E test: multiple probe types all emit results into the web UI.
///
/// Uses `huginn_web::server::run_server` directly — no real InfluxDB needed.
/// Three probe result types (http, tcp, dns) are injected via the EventHub
/// and then verified via GET /metrics/latest.
use huginn_core::event::ProbeEvent;
use huginn_core::types::ProbeResult;
use std::time::Duration;

mod common;
use common::{free_port, start_server};

#[tokio::test]
async fn multiple_probes_all_appear_in_metrics() {
    let port = free_port();
    let hub = start_server(port).await;
    // Wait until the server is up *and* subscribed to the hub before publishing —
    // run_server subscribes before it binds, so a 200 from /health proves no
    // published event will be missed. A fixed sleep raced that subscription.
    assert!(
        common::wait_for_ready(port).await,
        "web server never became ready"
    );

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

    // Poll until all three results are visible instead of sleeping a fixed 100ms.
    let url = format!("http://127.0.0.1:{port}/metrics/latest");
    assert!(
        common::wait_until(Duration::from_secs(10), || {
            let url = url.clone();
            async move { metrics_len(&url).await == Some(3) }
        })
        .await,
        "expected 3 results to appear in /metrics/latest"
    );

    // Fetch once more for the detailed assertions.
    let body: serde_json::Value = reqwest::get(&url).await.unwrap().json().await.unwrap();
    let arr = body.as_array().expect("expected JSON array");
    assert_eq!(
        arr.len(),
        3,
        "expected 3 results, got {}: {body}",
        arr.len()
    );

    let names: Vec<&str> = arr
        .iter()
        .filter_map(|v| v["probe_name"].as_str())
        .collect();
    assert!(
        names.contains(&"http-check"),
        "http-check missing: {names:?}"
    );
    assert!(names.contains(&"tcp-check"), "tcp-check missing: {names:?}");
    assert!(names.contains(&"dns-check"), "dns-check missing: {names:?}");

    drop(hub);
}

#[tokio::test]
async fn all_probes_report_correct_probe_type_field() {
    let port = free_port();
    let hub = start_server(port).await;
    assert!(
        common::wait_for_ready(port).await,
        "web server never became ready"
    );

    hub.publish(ProbeEvent::ProbeCompleted(ProbeResult::success(
        "smtp-mon",
        "smtp",
        "mail.example.com:25",
        30.0,
        None,
    )));
    hub.publish(ProbeEvent::ProbeCompleted(ProbeResult::success(
        "imap-mon",
        "imap",
        "mail.example.com:143",
        28.0,
        None,
    )));
    hub.publish(ProbeEvent::ProbeCompleted(ProbeResult::success(
        "udp-mon",
        "udp",
        "8.8.8.8:53",
        2.0,
        None,
    )));

    let url = format!("http://127.0.0.1:{port}/metrics/latest");
    assert!(
        common::wait_until(Duration::from_secs(10), || {
            let url = url.clone();
            async move { metrics_len(&url).await == Some(3) }
        })
        .await,
        "expected 3 results to appear in /metrics/latest"
    );

    let body: serde_json::Value = reqwest::get(&url).await.unwrap().json().await.unwrap();
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

/// The number of results currently in `/metrics/latest`, or `None` on any error.
async fn metrics_len(url: &str) -> Option<usize> {
    let body: serde_json::Value = reqwest::get(url).await.ok()?.json().await.ok()?;
    body.as_array().map(|a| a.len())
}
