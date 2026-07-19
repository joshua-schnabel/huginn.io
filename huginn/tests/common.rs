use huginn_core::event::EventHub;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

pub async fn start_server(port: u16) -> Arc<EventHub> {
    let hub = Arc::new(EventHub::new(256));
    let hub_clone = Arc::clone(&hub);
    tokio::spawn(async move {
        huginn_web::server::run_server(port, hub_clone).await.ok();
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
