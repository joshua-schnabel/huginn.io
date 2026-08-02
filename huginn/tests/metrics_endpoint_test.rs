//! E2E tests for the Prometheus `/metrics` listener: real TCP port, real HTTP.

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::{free_port, wait_until};
use huginn_core::event::{EventHub, ProbeEvent};
use huginn_core::types::ProbeResult;
use huginn_web::state::WebState;

const READY: Duration = Duration::from_secs(5);

/// Spawn the metrics server the way `main.rs` does: shared `WebState`, event
/// loop subscribed before the port binds — so once `/metrics` answers, any
/// subsequently published event is guaranteed observed.
async fn start_metrics_server(port: u16, api_key: Option<&str>) -> Arc<EventHub> {
    let hub = Arc::new(EventHub::new(256));
    let state = Arc::new(WebState::new());
    Arc::clone(&state).start_event_loop(Arc::clone(&hub));
    let api_key = api_key.map(str::to_string);
    tokio::spawn(async move {
        huginn_web::prometheus::run_metrics_server("127.0.0.1", port, state, api_key)
            .await
            .ok();
    });
    hub
}

/// Ready = the server answers at all; 401 (auth enabled, no key sent) counts.
async fn wait_for_ready(port: u16) {
    let url = format!("http://127.0.0.1:{port}/metrics");
    let up = wait_until(READY, || {
        let url = url.clone();
        async move {
            match reqwest::get(&url).await {
                Ok(resp) => matches!(resp.status().as_u16(), 200 | 401),
                Err(_) => false,
            }
        }
    })
    .await;
    assert!(up, "/metrics never became ready");
}

#[tokio::test]
async fn metrics_endpoint_serves_prometheus_text_format() {
    let port = free_port();
    let hub = start_metrics_server(port, None).await;
    wait_for_ready(port).await;

    let result = ProbeResult::success("web", "http", "https://example.com", 42.0, Some(200));
    hub.publish(ProbeEvent::ProbeCompleted(result));

    let url = format!("http://127.0.0.1:{port}/metrics");
    let reflected = wait_until(READY, || {
        let url = url.clone();
        async move {
            match reqwest::get(&url).await {
                Ok(resp) => {
                    let ct = resp
                        .headers()
                        .get("content-type")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or_default()
                        .to_string();
                    let body = resp.text().await.unwrap_or_default();
                    ct.starts_with("text/plain")
                        && body.contains(
                            "huginn_probe_success{probe=\"web\",type=\"http\",target=\"https://example.com\"} 1",
                        )
                        && body.contains("# TYPE huginn_probe_duration_seconds gauge")
                }
                Err(_) => false,
            }
        }
    })
    .await;
    assert!(reflected, "/metrics never reflected the published result");
    drop(hub);
}

#[tokio::test]
async fn metrics_endpoint_is_empty_before_any_result() {
    let port = free_port();
    let hub = start_metrics_server(port, None).await;
    wait_for_ready(port).await;

    let body = reqwest::get(format!("http://127.0.0.1:{port}/metrics"))
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(
        body.is_empty(),
        "no probes have reported yet, expected an empty exposition: {body}"
    );
    drop(hub);
}

#[tokio::test]
async fn metrics_endpoint_enforces_the_api_key_when_configured() {
    let port = free_port();
    let hub = start_metrics_server(port, Some("e2e-sekrit")).await;
    wait_for_ready(port).await;
    let url = format!("http://127.0.0.1:{port}/metrics");

    let unauthed = reqwest::get(&url).await.unwrap();
    assert_eq!(unauthed.status().as_u16(), 401, "no key must be rejected");
    assert_eq!(
        unauthed
            .headers()
            .get("www-authenticate")
            .and_then(|v| v.to_str().ok()),
        Some("Bearer")
    );

    let client = reqwest::Client::new();
    let wrong = client
        .get(&url)
        .header("authorization", "Bearer nope")
        .send()
        .await
        .unwrap();
    assert_eq!(wrong.status().as_u16(), 401, "wrong key must be rejected");

    let ok = client
        .get(&url)
        .header("authorization", "Bearer e2e-sekrit")
        .send()
        .await
        .unwrap();
    assert_eq!(ok.status().as_u16(), 200, "correct key must be accepted");
    drop(hub);
}
