use tokio::sync::broadcast;

use crate::types::ProbeResult;

/// Events emitted by the probe scheduler.
#[derive(Debug, Clone)]
pub enum ProbeEvent {
    ProbeCompleted(ProbeResult),
}

/// Central event bus. All subscribers receive every published event.
pub struct EventHub {
    tx: broadcast::Sender<ProbeEvent>,
}

impl EventHub {
    pub fn new(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        Self { tx }
    }

    pub fn publish(&self, event: ProbeEvent) {
        // Ignore send errors — no subscribers is fine.
        let _ = self.tx.send(event);
    }

    pub fn subscribe(&self) -> broadcast::Receiver<ProbeEvent> {
        self.tx.subscribe()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn sample_result() -> ProbeResult {
        ProbeResult::success("web", "http", "https://example.com", 10.0, Some(200))
    }

    #[tokio::test]
    async fn publish_delivers_event_to_subscriber() {
        let hub = EventHub::new(16);
        let mut rx = hub.subscribe();

        hub.publish(ProbeEvent::ProbeCompleted(sample_result()));

        let event = tokio::time::timeout(Duration::from_secs(1), rx.recv())
            .await
            .expect("timed out")
            .expect("channel closed");

        let ProbeEvent::ProbeCompleted(r) = event;
        assert_eq!(r.probe_name, "web");
    }

    #[tokio::test]
    async fn multiple_subscribers_each_receive_event() {
        let hub = EventHub::new(16);
        let mut rx1 = hub.subscribe();
        let mut rx2 = hub.subscribe();

        hub.publish(ProbeEvent::ProbeCompleted(sample_result()));

        let e1 = tokio::time::timeout(Duration::from_secs(1), rx1.recv())
            .await.expect("rx1 timed out").expect("rx1 closed");
        let e2 = tokio::time::timeout(Duration::from_secs(1), rx2.recv())
            .await.expect("rx2 timed out").expect("rx2 closed");

        let ProbeEvent::ProbeCompleted(r1) = e1;
        let ProbeEvent::ProbeCompleted(r2) = e2;
        assert_eq!(r1.probe_name, "web");
        assert_eq!(r2.probe_name, "web");
    }

    #[test]
    fn publish_without_subscribers_does_not_panic() {
        let hub = EventHub::new(16);
        // No subscriber — must not panic.
        hub.publish(ProbeEvent::ProbeCompleted(sample_result()));
    }

    #[tokio::test]
    async fn subscriber_gets_closed_when_hub_dropped() {
        let hub = EventHub::new(16);
        let mut rx = hub.subscribe();

        drop(hub);

        let result = tokio::time::timeout(Duration::from_secs(1), rx.recv()).await
            .expect("timed out waiting for Closed");

        assert!(
            matches!(result, Err(broadcast::error::RecvError::Closed)),
            "expected Closed, got {result:?}"
        );
    }
}
