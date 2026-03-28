use std::sync::Arc;

use axum::extract::State;
use axum::response::Html;
use axum::routing::get;
use axum::{Json, Router};
use huginn_core::event::EventHub;
use huginn_core::types::ProbeResult;
use tracing::info;

use crate::sse::sse_handler;
use crate::state::WebState;

// Embed assets at compile time so the binary is self-contained.
const INDEX_HTML: &str = include_str!("../assets/index.html");
const STYLE_CSS: &str  = include_str!("../assets/style.css");
const APP_JS: &str     = include_str!("../assets/app.js");

/// Start the web UI server.
///
/// Subscribes to `hub` to keep the latest probe results in sync and push
/// updates to connected browsers via Server-Sent Events.
pub async fn run_server(port: u16, hub: Arc<EventHub>) -> anyhow::Result<()> {
    let state = Arc::new(WebState::new());
    Arc::clone(&state).start_event_loop(Arc::clone(&hub));

    let app = Router::new()
        .route("/",               get(handle_index))
        .route("/events",         get(sse_handler))
        .route("/metrics/latest", get(handle_metrics))
        .route("/health",         get(handle_health))
        .route("/assets/style.css", get(handle_css))
        .route("/assets/app.js",    get(handle_js))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(("0.0.0.0", port)).await?;
    info!("Web UI listening on http://0.0.0.0:{port}");
    axum::serve(listener, app).await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Route handlers
// ---------------------------------------------------------------------------

async fn handle_index() -> Html<&'static str> {
    Html(INDEX_HTML)
}

async fn handle_health() -> &'static str {
    "OK"
}

async fn handle_metrics(
    State(state): State<Arc<WebState>>,
) -> Json<Vec<ProbeResult>> {
    let guard = state.results.read().await;
    Json(guard.values().cloned().collect())
}

async fn handle_css() -> ([(&'static str, &'static str); 1], &'static str) {
    ([("content-type", "text/css; charset=utf-8")], STYLE_CSS)
}

async fn handle_js() -> ([(&'static str, &'static str); 1], &'static str) {
    ([("content-type", "application/javascript; charset=utf-8")], APP_JS)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::WebState;
    use axum::extract::State;
    use huginn_core::types::ProbeResult;
    use std::collections::HashMap;
    use std::sync::Arc;
    use tokio::sync::RwLock;

    fn make_state(results: Vec<ProbeResult>) -> Arc<WebState> {
        let mut map = HashMap::new();
        for r in results {
            map.insert(r.probe_name.clone(), r);
        }
        let (sse_tx, _) = tokio::sync::broadcast::channel(1);
        Arc::new(WebState {
            results: Arc::new(RwLock::new(map)),
            sse_tx,
        })
    }

    fn success_result() -> ProbeResult {
        ProbeResult::success("web", "http", "https://example.com", 42.5, Some(200))
    }

    fn failure_result() -> ProbeResult {
        ProbeResult::failure("db", "tcp", "host:5432", 5000.0, "connection refused")
    }

    #[tokio::test]
    async fn health_returns_ok() {
        assert_eq!(handle_health().await, "OK");
    }

    #[tokio::test]
    async fn metrics_returns_empty_when_no_results() {
        let state = make_state(vec![]);
        let Json(results) = handle_metrics(State(state)).await;
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn metrics_returns_stored_results() {
        let state = make_state(vec![success_result()]);
        let Json(results) = handle_metrics(State(state)).await;
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].probe_name, "web");
    }

    #[tokio::test]
    async fn metrics_returns_latest_per_probe() {
        let state = make_state(vec![success_result(), failure_result()]);
        let Json(results) = handle_metrics(State(state)).await;
        assert_eq!(results.len(), 2);
    }

    #[tokio::test]
    async fn index_returns_html() {
        let Html(html) = handle_index().await;
        assert!(html.contains("huginn.io"), "title missing");
        assert!(html.contains("/assets/app.js"), "JS reference missing");
        assert!(html.contains("/assets/style.css"), "CSS reference missing");
    }

    #[tokio::test]
    async fn css_handler_returns_css_content_type() {
        let (headers, body) = handle_css().await;
        assert_eq!(headers[0], ("content-type", "text/css; charset=utf-8"));
        assert!(!body.is_empty());
    }

    #[tokio::test]
    async fn js_handler_returns_js_content_type() {
        let (headers, body) = handle_js().await;
        assert_eq!(headers[0], ("content-type", "application/javascript; charset=utf-8"));
        assert!(!body.is_empty());
    }
}
