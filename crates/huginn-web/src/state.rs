use std::collections::HashMap;
use std::sync::Arc;

use huginn_core::event::{EventHub, ProbeEvent};
use huginn_core::types::ProbeResult;
use serde_json;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::{broadcast, RwLock};
use tracing::error;

/// Shared state for web handlers.
#[derive(Clone)]
pub struct WebState {
    /// Latest result per probe name.
    pub results: Arc<RwLock<HashMap<String, ProbeResult>>>,
    /// SSE broadcast: each message is a JSON-encoded `ProbeResult`.
    pub sse_tx: broadcast::Sender<String>,
}

impl WebState {
    pub fn new() -> Self {
        let (sse_tx, _) = broadcast::channel(256);
        Self {
            results: Arc::new(RwLock::new(HashMap::new())),
            sse_tx,
        }
    }

    /// Spawn a task that subscribes to `hub` and keeps this state in sync.
    pub fn start_event_loop(self: Arc<Self>, hub: Arc<EventHub>) {
        // Subscribe before spawning so no events are missed after this call returns.
        let mut rx = hub.subscribe();
        // Drop the Arc so this task doesn't prevent the hub from closing.
        drop(hub);
        let state = Arc::clone(&self);
        tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(ProbeEvent::ProbeCompleted(result)) => {
                        {
                            let mut guard = state.results.write().await;
                            guard.insert(result.probe_name.clone(), result.clone());
                        }
                        if let Ok(json) = serde_json::to_string(&result) {
                            // Ignore send errors — no SSE clients connected is fine.
                            let _ = state.sse_tx.send(json);
                        }
                    }
                    Err(RecvError::Lagged(n)) => {
                        error!("Web state dropped {n} events (channel lagged)");
                    }
                    Err(RecvError::Closed) => break,
                }
            }
        });
    }
}

impl Default for WebState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use huginn_core::event::{EventHub, ProbeEvent};
    use huginn_core::types::ProbeResult;
    use std::time::Duration;

    fn result(name: &str) -> ProbeResult {
        ProbeResult::success(name, "http", "https://example.com", 5.0, Some(200))
    }

    #[tokio::test]
    async fn event_loop_inserts_result_on_probe_completed() {
        let hub = Arc::new(EventHub::new(16));
        let state = Arc::new(WebState::new());
        Arc::clone(&state).start_event_loop(Arc::clone(&hub));

        hub.publish(ProbeEvent::ProbeCompleted(result("web")));
        tokio::time::sleep(Duration::from_millis(50)).await;

        let guard = state.results.read().await;
        assert!(guard.contains_key("web"), "result not stored");
        assert_eq!(guard["web"].probe_name, "web");
    }

    #[tokio::test]
    async fn event_loop_updates_existing_probe_keeps_latest() {
        let hub = Arc::new(EventHub::new(16));
        let state = Arc::new(WebState::new());
        Arc::clone(&state).start_event_loop(Arc::clone(&hub));

        let mut r1 = result("db");
        r1.response_ms = 10.0;
        let mut r2 = result("db");
        r2.response_ms = 99.0;

        hub.publish(ProbeEvent::ProbeCompleted(r1));
        hub.publish(ProbeEvent::ProbeCompleted(r2));
        tokio::time::sleep(Duration::from_millis(50)).await;

        let guard = state.results.read().await;
        assert_eq!(guard.len(), 1, "should only keep one entry per probe");
        assert_eq!(guard["db"].response_ms, 99.0, "should keep latest value");
    }

    #[tokio::test]
    async fn event_loop_broadcasts_json_on_sse_tx() {
        let hub = Arc::new(EventHub::new(16));
        let state = Arc::new(WebState::new());
        let mut sse_rx = state.sse_tx.subscribe();
        Arc::clone(&state).start_event_loop(Arc::clone(&hub));

        hub.publish(ProbeEvent::ProbeCompleted(result("smtp")));

        let json = tokio::time::timeout(Duration::from_secs(1), sse_rx.recv())
            .await
            .expect("timed out waiting for SSE broadcast")
            .expect("sse_tx closed");

        assert!(
            json.contains("smtp"),
            "probe_name missing from SSE JSON: {json}"
        );
    }

    #[tokio::test]
    async fn event_loop_handles_lagged_events() {
        // capacity=1: any second publish before recv() is processed causes Lagged.
        let hub = Arc::new(EventHub::new(1));
        let state = Arc::new(WebState::new());
        Arc::clone(&state).start_event_loop(Arc::clone(&hub));

        // Yield once so the spawned task runs until it is parked in rx.recv().await.
        tokio::task::yield_now().await;

        // Flood synchronously (no await) — the receiver will see Lagged.
        for i in 0..5u32 {
            hub.publish(ProbeEvent::ProbeCompleted(result(&format!("probe-{i}"))));
        }

        // Let the event loop process the Lagged error without panicking.
        tokio::task::yield_now().await;

        // Hub closes → event loop exits cleanly (no panic is the assertion here).
        drop(hub);
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
