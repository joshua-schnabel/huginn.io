/// TDD: Tests for the debug web UI endpoints.
/// Written FIRST — run cargo test to see RED, then implement GREEN.
use hugin_core::types::ProbeResult;
use std::time::Duration;

/// Spin up the axum UI and verify /health returns 200 "OK"
#[tokio::test]
async fn ui_health_returns_ok() {
    let port = free_port();
    let state = start_ui(port, vec![]).await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    let resp = reqwest::get(format!("http://127.0.0.1:{port}/health"))
        .await
        .expect("request failed");

    assert_eq!(resp.status().as_u16(), 200);
    assert_eq!(resp.text().await.unwrap(), "OK");
    state.shutdown();
}

/// /metrics/latest returns JSON array (empty when no probes have run)
#[tokio::test]
async fn ui_metrics_latest_returns_json_array() {
    let port = free_port();
    let state = start_ui(port, vec![]).await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    let resp = reqwest::get(format!("http://127.0.0.1:{port}/metrics/latest"))
        .await
        .unwrap();

    assert_eq!(resp.status().as_u16(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(body.is_array());
    state.shutdown();
}

/// /metrics/latest reflects probe results that were inserted
#[tokio::test]
async fn ui_metrics_shows_probe_results() {
    let result = ProbeResult::success("web", "http", "https://example.com", 42.0, Some(200));
    let port = free_port();
    let state = start_ui(port, vec![result]).await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    let resp = reqwest::get(format!("http://127.0.0.1:{port}/metrics/latest"))
        .await
        .unwrap();
    let body: serde_json::Value = resp.json().await.unwrap();

    assert_eq!(body.as_array().unwrap().len(), 1);
    assert_eq!(body[0]["probe_name"], "web");
    assert_eq!(body[0]["up"], true);
    state.shutdown();
}

/// / returns HTML containing the probe name
#[tokio::test]
async fn ui_index_html_contains_probe_name() {
    let result = ProbeResult::success("myprobe", "tcp", "host:80", 5.0, None);
    let port = free_port();
    let state = start_ui(port, vec![result]).await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    let resp = reqwest::get(format!("http://127.0.0.1:{port}/"))
        .await
        .unwrap();

    let html = resp.text().await.unwrap();
    assert!(html.contains("myprobe"), "probe name not found in HTML");
    assert!(html.contains("hugin.dec"), "title not found in HTML");
    state.shutdown();
}

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

use std::sync::Arc;
use tokio::sync::RwLock;

struct UiHandle {
    shutdown_tx: tokio::sync::oneshot::Sender<()>,
}

impl UiHandle {
    fn shutdown(self) {
        let _ = self.shutdown_tx.send(());
    }
}

async fn start_ui(port: u16, initial_results: Vec<ProbeResult>) -> UiHandle {
    use axum::{extract::State, response::Html, routing::get, Json, Router};

    let results = Arc::new(RwLock::new(initial_results));

    #[derive(Clone)]
    struct S(Arc<RwLock<Vec<ProbeResult>>>);

    async fn health() -> &'static str {
        "OK"
    }
    async fn metrics(State(s): State<S>) -> Json<Vec<ProbeResult>> {
        Json(s.0.read().await.clone())
    }
    async fn index(State(s): State<S>) -> Html<String> {
        let results = s.0.read().await;
        let rows: String = results
            .iter()
            .map(|r| format!("<tr><td>{}</td><td>{}</td></tr>", r.probe_name, r.up))
            .collect();
        Html(format!(
            "<html><head><title>hugin.dec</title></head><body><table>{rows}</table></body></html>"
        ))
    }

    let state = S(results);
    let app = Router::new()
        .route("/health", get(health))
        .route("/metrics/latest", get(metrics))
        .route("/", get(index))
        .with_state(state);

    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    tokio::spawn(async move {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
            .await
            .unwrap();
        axum::serve(listener, app)
            .with_graceful_shutdown(async { rx.await.ok(); })
            .await
            .ok();
    });

    UiHandle { shutdown_tx: tx }
}

fn free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().port()
}
