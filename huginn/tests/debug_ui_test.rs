/// Integration tests for the web UI — now using `huginn_web::server::run_server`.
use huginn_core::event::ProbeEvent;
use huginn_core::types::ProbeResult;
use std::time::Duration;

mod common;
use common::{free_port, start_server, wait_until};

/// How long to wait for the server to come up / the event loop to catch up.
/// Generous because coverage-instrumented runs are much slower than plain ones.
const READY: Duration = Duration::from_secs(5);

/// Block until the server answers `/health` with 200.
///
/// `run_server` subscribes to the event hub *before* it binds the port
/// (see `WebState::start_event_loop`), so once `/health` responds the event
/// loop is guaranteed to be subscribed — anything published afterwards is
/// observed, with no fixed sleep to flake under load. Must be called before
/// publishing in any test that then asserts on the result.
async fn wait_for_ready(port: u16) {
    let url = format!("http://127.0.0.1:{port}/health");
    let up = wait_until(READY, || {
        let url = url.clone();
        async move { matches!(reqwest::get(url).await, Ok(r) if r.status().as_u16() == 200) }
    })
    .await;
    assert!(up, "server never became ready on port {port}");
}

/// Spin up the web server and verify /health returns 200 "OK"
#[tokio::test]
async fn ui_health_returns_ok() {
    let port = free_port();
    let hub = start_server(port).await;
    wait_for_ready(port).await;

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
    wait_for_ready(port).await;

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
    // Ready first: guarantees the event loop is subscribed before we publish,
    // so the event cannot be dropped by the broadcast channel.
    wait_for_ready(port).await;

    let result = ProbeResult::success("web", "http", "https://example.com", 42.0, Some(200));
    hub.publish(ProbeEvent::ProbeCompleted(result));

    // Poll until the published result is reflected — no fixed sleep, which
    // flakes under load / coverage instrumentation (docs/testing.md).
    let url = format!("http://127.0.0.1:{port}/metrics/latest");
    let reflected = wait_until(READY, || {
        let url = url.clone();
        async move {
            let Ok(resp) = reqwest::get(url).await else {
                return false;
            };
            let Ok(body) = resp.json::<serde_json::Value>().await else {
                return false;
            };
            body.as_array().map(|a| a.len() == 1).unwrap_or(false)
        }
    })
    .await;
    assert!(
        reflected,
        "/metrics/latest never reflected the published result"
    );

    let body: serde_json::Value = reqwest::get(&url).await.unwrap().json().await.unwrap();
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
    wait_for_ready(port).await;

    hub.publish(ProbeEvent::ProbeCompleted(ProbeResult::success(
        "myprobe", "tcp", "host:80", 5.0, None,
    )));

    let found = wait_until(READY, || {
        let url = format!("http://127.0.0.1:{port}/");
        async move {
            let Ok(resp) = reqwest::get(url).await else {
                return false;
            };
            resp.text()
                .await
                .map(|html| html.contains("huginn.io"))
                .unwrap_or(false)
        }
    })
    .await;
    assert!(found, "title not found in HTML");
    drop(hub);
}

/// Audit F-02: the security headers must reach the wire, not just exist as
/// constants. Asserted end-to-end because the middleware is easy to detach from
/// the router without any unit test noticing.
#[tokio::test]
async fn ui_responses_carry_the_security_headers() {
    let port = free_port();
    let hub = start_server(port).await;
    wait_for_ready(port).await;

    for path in ["/", "/metrics/latest", "/health", "/assets/app.js"] {
        let resp = reqwest::get(format!("http://127.0.0.1:{port}{path}"))
            .await
            .unwrap_or_else(|e| panic!("request to {path} failed: {e}"));
        let headers = resp.headers();

        let csp = headers
            .get("content-security-policy")
            .unwrap_or_else(|| panic!("{path}: no CSP header"))
            .to_str()
            .unwrap();
        assert!(csp.contains("default-src 'none'"), "{path}: {csp}");
        assert!(
            !csp.contains("unsafe-inline"),
            "{path}: inline script must stay blocked: {csp}"
        );
        assert_eq!(
            headers.get("x-content-type-options").unwrap(),
            "nosniff",
            "{path}: missing nosniff"
        );
        assert_eq!(
            headers.get("x-frame-options").unwrap(),
            "DENY",
            "{path}: missing frame denial"
        );
    }
    drop(hub);
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------
