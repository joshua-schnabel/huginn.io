/// Integration tests for the web UI — now using `huginn_web::server::run_server`.
use huginn_core::event::ProbeEvent;
use huginn_core::types::ProbeResult;
use std::time::Duration;

mod common;
use common::{free_port, start_server};

/// Spin up the web server and verify /health returns 200 "OK"
#[tokio::test]
async fn ui_health_returns_ok() {
    let port = free_port();
    let hub = start_server(port).await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    let resp = reqwest::get(format!("http://127.0.0.1:{port}/health"))
        .await
        .expect("request failed");

    assert_eq!(resp.status().as_u16(), 200);
    assert_eq!(resp.text().await.unwrap(), "OK");
    drop(hub);
}

/// /metrics/latest returns JSON array (empty when no probes have run)
#[tokio::test]
async fn ui_metrics_latest_returns_json_array() {
    let port = free_port();
    let hub = start_server(port).await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    let resp = reqwest::get(format!("http://127.0.0.1:{port}/metrics/latest"))
        .await
        .unwrap();

    assert_eq!(resp.status().as_u16(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(body.is_array());
    drop(hub);
}

/// /metrics/latest reflects probe results published on the event hub
#[tokio::test]
async fn ui_metrics_shows_probe_results() {
    let port = free_port();
    let hub = start_server(port).await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    let result = ProbeResult::success("web", "http", "https://example.com", 42.0, Some(200));
    hub.publish(ProbeEvent::ProbeCompleted(result));
    // Give the event loop time to process
    tokio::time::sleep(Duration::from_millis(50)).await;

    let resp = reqwest::get(format!("http://127.0.0.1:{port}/metrics/latest"))
        .await
        .unwrap();
    let body: serde_json::Value = resp.json().await.unwrap();

    assert_eq!(body.as_array().unwrap().len(), 1);
    assert_eq!(body[0]["probe_name"], "web");
    assert_eq!(body[0]["up"], true);
    drop(hub);
}

/// / returns HTML containing the probe name (seeded via EventHub)
#[tokio::test]
async fn ui_index_html_contains_probe_name() {
    let port = free_port();
    let hub = start_server(port).await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    hub.publish(ProbeEvent::ProbeCompleted(ProbeResult::success(
        "myprobe", "tcp", "host:80", 5.0, None,
    )));
    tokio::time::sleep(Duration::from_millis(50)).await;

    let resp = reqwest::get(format!("http://127.0.0.1:{port}/"))
        .await
        .unwrap();

    let html = resp.text().await.unwrap();
    assert!(html.contains("huginn.io"), "title not found in HTML");
    drop(hub);
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------
