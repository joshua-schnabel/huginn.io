//! E2E tests for the Prometheus `/metrics` listener: real TCP port, real HTTP.

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::{free_port, wait_until};
use huginn_core::event::{EventHub, ProbeEvent};
use huginn_core::stats::WriteStats;
use huginn_core::types::ProbeResult;
use huginn_web::state::WebState;

const READY: Duration = Duration::from_secs(5);

/// Spawn the metrics server the way `main.rs` does: shared `WebState`, event
/// loop subscribed before the port binds — so once `/metrics` answers, any
/// subsequently published event is guaranteed observed.
async fn start_metrics_server(
    port: u16,
    api_key: Option<&str>,
) -> (Arc<EventHub>, Arc<WriteStats>) {
    let hub = Arc::new(EventHub::new(256));
    let write_stats = Arc::new(WriteStats::default());
    let state = Arc::new(WebState::new(Arc::clone(&write_stats)));
    Arc::clone(&state).start_event_loop(Arc::clone(&hub));
    let api_key = api_key.map(str::to_string);
    tokio::spawn(async move {
        huginn_web::prometheus::run_metrics_server("127.0.0.1", port, state, api_key)
            .await
            .ok();
    });
    (hub, write_stats)
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
    let (hub, _write_stats) = start_metrics_server(port, None).await;
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

/// Before any probe has reported there are no probe families — but the
/// exposition is no longer empty, and that is the intended change.
///
/// It used to be. A scrape that returns nothing is indistinguishable from a
/// target that is down, whereas huginn that has started and not yet measured
/// anything is up and has written nothing — a distinction worth being able to
/// make, and exactly what the write-path block now says.
#[tokio::test]
async fn before_any_result_only_the_write_path_block_is_served() {
    let port = free_port();
    let (hub, _write_stats) = start_metrics_server(port, None).await;
    wait_for_ready(port).await;

    let body = reqwest::get(format!("http://127.0.0.1:{port}/metrics"))
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(
        !body.contains("huginn_probe_"),
        "no probes have reported yet, so no probe family may appear: {body}"
    );
    assert!(
        body.contains("huginn_influx_batches_written_total 0"),
        "the write-path block must be served from startup, with zeroes: {body}"
    );
    drop(hub);
}

#[tokio::test]
async fn metrics_endpoint_enforces_the_api_key_when_configured() {
    let port = free_port();
    let (hub, _write_stats) = start_metrics_server(port, Some("e2e-sekrit")).await;
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

/// The write path's own numbers reach the endpoint.
///
/// This is the point of the whole feature: a probe gauge says a target is up,
/// and cannot say that huginn measured it and then threw the measurement away.
/// The counters are moved directly here rather than through a real InfluxDB
/// outage — that path is covered by `influx_retry_test.rs`; what is under test
/// is that the endpoint reports them at all, with the right types.
#[tokio::test]
async fn metrics_endpoint_exposes_the_write_path_counters() {
    let port = free_port();
    let (hub, write_stats) = start_metrics_server(port, None).await;
    wait_for_ready(port).await;

    write_stats.set_queue_depth(3, 512);
    write_stats.record_eviction(128);
    write_stats.record_rejected();
    write_stats.record_written(256, 1_700_000_000);

    let url = format!("http://127.0.0.1:{port}/metrics");
    let body = reqwest::get(&url).await.unwrap().text().await.unwrap();

    for expected in [
        "# TYPE huginn_influx_queue_batches gauge\nhuginn_influx_queue_batches 3\n",
        "# TYPE huginn_influx_queue_bytes gauge\nhuginn_influx_queue_bytes 512\n",
        "# TYPE huginn_influx_batches_written_total counter\nhuginn_influx_batches_written_total 1\n",
        "# TYPE huginn_influx_bytes_written_total counter\nhuginn_influx_bytes_written_total 256\n",
        "# TYPE huginn_influx_batches_dropped_total counter\nhuginn_influx_batches_dropped_total 1\n",
        "# TYPE huginn_influx_bytes_dropped_total counter\nhuginn_influx_bytes_dropped_total 128\n",
        "# TYPE huginn_influx_batches_rejected_total counter\nhuginn_influx_batches_rejected_total 1\n",
        "# TYPE huginn_influx_last_write_success_timestamp_seconds gauge\n\
         huginn_influx_last_write_success_timestamp_seconds 1700000000\n",
    ] {
        assert!(
            body.contains(expected),
            "missing from /metrics:\n{expected}\n--- body ---\n{body}"
        );
    }

    // Counters, not gauges: a scraper computing rate() over a gauge silently
    // gets nonsense rather than an error.
    assert!(
        !body.contains("# TYPE huginn_influx_batches_dropped_total gauge"),
        "the loss counters must be counters"
    );
    drop(hub);
}
