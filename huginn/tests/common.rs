use huginn_core::event::EventHub;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

// `common` is compiled into every integration-test binary via `mod common;`;
// the metrics test starts its own server and doesn't use this helper.
#[allow(dead_code)]
pub async fn start_server(port: u16) -> Arc<EventHub> {
    let hub = Arc::new(EventHub::new(256));
    let hub_clone = Arc::clone(&hub);
    tokio::spawn(async move {
        huginn_web::server::run_server("127.0.0.1", port, hub_clone)
            .await
            .ok();
    });
    hub
}

pub fn free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().port()
}

/// Poll an async check until it returns `true` or the deadline elapses.
///
/// Replaces fixed `sleep`s before an assertion, which flake on a loaded or
/// coverage-instrumented CI runner — see `docs/testing.md` ("Don't sleep —
/// poll"). Returns `true` if the check passed within `timeout`.
///
// `common` is compiled into every integration-test binary via `mod common;`,
// but not every one uses this helper — allow it to be unused there.
#[allow(dead_code)]
pub async fn wait_until<F, Fut>(timeout: Duration, mut check: F) -> bool
where
    F: FnMut() -> Fut,
    Fut: Future<Output = bool>,
{
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if check().await {
            return true;
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// Poll `/health` until it returns 200. The server is then listening and —
/// because `run_server` subscribes to the hub *before* it binds — already
/// subscribed, so any event published afterwards is guaranteed to be delivered.
/// Use this instead of a fixed sleep before publishing to (or reading from) a
/// freshly started server. Returns `true` if it came up within 10s.
#[allow(dead_code)]
pub async fn wait_for_ready(port: u16) -> bool {
    let url = format!("http://127.0.0.1:{port}/health");
    wait_until(Duration::from_secs(10), move || {
        let url = url.clone();
        async move { matches!(reqwest::get(&url).await, Ok(r) if r.status().as_u16() == 200) }
    })
    .await
}
