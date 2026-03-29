/// Integration test: SSE endpoint delivers probe events via push.
use huginn_core::event::ProbeEvent;
use huginn_core::types::ProbeResult;
use std::time::Duration;

mod common;
use common::{free_port, start_server};

/// Connect to /events, publish a ProbeCompleted, verify the SSE message arrives.
#[tokio::test]
async fn sse_endpoint_delivers_probe_event_as_data_message() {
    let port = free_port();
    let hub = start_server(port).await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Open the SSE stream — use reqwest in streaming mode
    let client = reqwest::Client::new();
    let mut response = client
        .get(format!("http://127.0.0.1:{port}/events"))
        .header("Accept", "text/event-stream")
        .send()
        .await
        .expect("failed to connect to /events");

    assert_eq!(response.status().as_u16(), 200);

    // Publish an event on the hub
    let result = ProbeResult::success("sse-probe", "http", "https://example.com", 7.0, Some(200));
    hub.publish(ProbeEvent::ProbeCompleted(result));

    // Read chunks until we find the "data:" line (or time out)
    let received = tokio::time::timeout(Duration::from_secs(3), async {
        let mut buf = String::new();
        while let Some(chunk) = response.chunk().await.expect("chunk error") {
            buf.push_str(&String::from_utf8_lossy(&chunk));
            if buf.contains("data:") {
                return buf;
            }
        }
        buf
    })
    .await
    .expect("timed out waiting for SSE message");

    assert!(
        received.contains("sse-probe"),
        "probe_name not found in SSE stream:\n{received}"
    );
}

// ---------------------------------------------------------------------------
// Helpers (shared with debug_ui_test)
// ---------------------------------------------------------------------------
