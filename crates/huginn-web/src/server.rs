use std::net::IpAddr;
use std::sync::Arc;

use anyhow::Context;
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
const STYLE_CSS: &str = include_str!("../assets/style.css");
const APP_JS: &str = include_str!("../assets/app.js");

/// Start the web UI server on `bind`:`port`, creating its own [`WebState`].
///
/// Subscribes to `hub` to keep the latest probe results in sync and push
/// updates to connected browsers via Server-Sent Events. When the metrics
/// listener is enabled too, use [`run_server_with_state`] with a shared state
/// instead, so both servers feed from one event loop.
pub async fn run_server(bind: &str, port: u16, hub: Arc<EventHub>) -> anyhow::Result<()> {
    let state = Arc::new(WebState::new());
    Arc::clone(&state).start_event_loop(Arc::clone(&hub));
    // Drop our Arc: `axum::serve` below never returns, so holding it would keep
    // the hub's Sender alive for the life of the process and no subscriber would
    // ever observe Closed. start_event_loop has its own clone and drops it too.
    drop(hub);
    run_server_with_state(bind, port, state).await
}

/// Start the web UI server against an existing [`WebState`] whose event loop
/// is already running.
///
/// `bind` must parse as an [`IpAddr`]; `AppConfig::validate` rejects anything
/// else before startup, so reaching the error here means the caller bypassed it.
pub async fn run_server_with_state(
    bind: &str,
    port: u16,
    state: Arc<WebState>,
) -> anyhow::Result<()> {
    let listener = bind_ui(bind, port).await?;
    serve_ui(listener, state).await
}

/// Bind the UI's socket, returning it before anything is served.
///
/// Split from [`serve_ui`] so the caller can bind on the main task and fail
/// startup on a taken port. Binding inside the spawned task meant a port clash
/// produced one logged error while the process carried on without the UI the
/// operator had explicitly enabled — a service that is configured on, reports
/// nothing wrong, and is not there.
pub async fn bind_ui(bind: &str, port: u16) -> anyhow::Result<tokio::net::TcpListener> {
    let addr: IpAddr = bind
        .parse()
        .with_context(|| format!("ui.bind '{bind}' is not a valid IP address"))?;
    tokio::net::TcpListener::bind((addr, port))
        .await
        .with_context(|| format!("could not bind the debug UI on {bind}:{port}"))
}

/// Serve the debug UI on an already-bound listener.
pub async fn serve_ui(
    listener: tokio::net::TcpListener,
    state: Arc<WebState>,
) -> anyhow::Result<()> {
    let app = Router::new()
        .route("/", get(handle_index))
        .route("/events", get(sse_handler))
        .route("/metrics/latest", get(handle_metrics))
        .route("/health", get(handle_health))
        .route("/assets/style.css", get(handle_css))
        .route("/assets/app.js", get(handle_js))
        .layer(axum::middleware::from_fn(crate::headers::security_headers))
        .with_state(state);

    if let Ok(addr) = listener.local_addr() {
        info!("Web UI listening on http://{addr}");
    }
    crate::serve::serve_with_limits(listener, app).await?;
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

async fn handle_metrics(State(state): State<Arc<WebState>>) -> Json<Vec<ProbeResult>> {
    let guard = state.results.read().await;
    Json(guard.values().cloned().collect())
}

async fn handle_css() -> ([(&'static str, &'static str); 1], &'static str) {
    ([("content-type", "text/css; charset=utf-8")], STYLE_CSS)
}

async fn handle_js() -> ([(&'static str, &'static str); 1], &'static str) {
    (
        [("content-type", "application/javascript; charset=utf-8")],
        APP_JS,
    )
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
        assert_eq!(
            headers[0],
            ("content-type", "application/javascript; charset=utf-8")
        );
        assert!(!body.is_empty());
    }
}
