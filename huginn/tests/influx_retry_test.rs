//! The acceptance test for the InfluxDB retry pipeline.
//!
//! One assertion covers both halves of the tension the design had to resolve:
//!
//! 1. **No loss on a transient outage.** The old `flush_buffer` cleared the
//!    buffer whether or not the write succeeded, so results vanished exactly
//!    when InfluxDB was unhealthy.
//! 2. **No loss at the source.** The obvious fix — retry inside the flush —
//!    would be worse. The flush ran in the same task that read the EventHub, so
//!    a retry loop stops polling `rx.recv()`, the broadcast channel fills, and
//!    `Lagged` discards results *before they are ever buffered*. That turns
//!    "some data lost during a blip" into "everything lost during the outage".
//!
//! Publishing 100 results faster than a hub of capacity 16 can hold, while the
//! server is failing, and then demanding all 100 land, can only pass if the
//! batcher never blocks on I/O and the queue never drops on the happy path.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use huginn_core::event::{EventHub, ProbeEvent};
use huginn_core::types::ProbeResult;
use huginn_influx::queue::RetryQueue;
use huginn_influx::writer::{run_batcher, run_writer, InfluxWriter};
use tokio::sync::broadcast;
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

fn token_file(content: &str) -> tempfile::NamedTempFile {
    use std::io::Write as _;
    let mut f = tempfile::NamedTempFile::new().unwrap();
    write!(f, "{content}").unwrap();
    f
}

fn influx_cfg(url: &str, tf: &tempfile::NamedTempFile) -> huginn_core::config::InfluxConfig {
    huginn_core::config::InfluxConfig {
        url: url.to_string(),
        org: "org".into(),
        bucket: "bkt".into(),
        token_file: tf.path().to_string_lossy().into_owned(),
        batch_size: 5,
        batch_timeout_ms: 20,
        ..Default::default()
    }
}

/// Count the points in a line-protocol body — one per line.
fn count_points(body: &[u8]) -> usize {
    String::from_utf8_lossy(body).lines().count()
}

#[tokio::test]
async fn no_results_are_lost_while_influxdb_is_briefly_down() {
    let server = MockServer::start().await;

    // Every point the server has actually accepted.
    let accepted = Arc::new(AtomicUsize::new(0));
    let accepted_mock = Arc::clone(&accepted);
    // Reject the first 3 writes with 503, then accept everything.
    let attempts = Arc::new(AtomicUsize::new(0));

    Mock::given(method("POST"))
        .respond_with(move |req: &Request| {
            let n = attempts.fetch_add(1, Ordering::SeqCst);
            if n < 3 {
                ResponseTemplate::new(503).set_body_string("temporarily unavailable")
            } else {
                accepted_mock.fetch_add(count_points(&req.body), Ordering::SeqCst);
                ResponseTemplate::new(204)
            }
        })
        .mount(&server)
        .await;

    let tf = token_file("mytoken");
    let cfg = influx_cfg(&server.uri(), &tf);
    let writer = Arc::new(InfluxWriter::new(&cfg).unwrap());
    let queue = Arc::new(RetryQueue::new(8 * 1024 * 1024));
    let (shutdown_tx, _) = broadcast::channel(1);

    // Capacity 16 against 100 rapid publishes: if the batcher ever stalls on the
    // network, the channel overflows and Lagged eats the difference.
    let hub = Arc::new(EventHub::new(16));

    tokio::spawn(run_batcher(
        Arc::clone(&hub),
        Arc::clone(&queue),
        cfg.batch_size,
        cfg.batch_timeout_ms,
    ));
    tokio::spawn(run_writer(
        writer,
        Arc::clone(&queue),
        5, // fast backoff — this is a test, not an outage drill
        50,
        shutdown_tx.subscribe(),
    ));
    tokio::task::yield_now().await;

    for i in 0..100 {
        hub.publish(ProbeEvent::ProbeCompleted(ProbeResult::success(
            format!("probe-{i}"),
            "tcp",
            "host:80",
            1.0,
            None,
        )));
        // Yield without sleeping: fast enough to pressure a 16-slot channel,
        // but lets the batcher run — it is cooperative, not preemptive.
        tokio::task::yield_now().await;
    }

    drop(hub);

    // Wait for the queue to drain rather than sleeping a guessed duration.
    let drained = tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            if queue.is_empty() && accepted.load(Ordering::SeqCst) >= 100 {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;

    let got = accepted.load(Ordering::SeqCst);
    assert!(
        drained.is_ok(),
        "queue did not drain: {got}/100 points accepted, {} batches still queued",
        queue.len()
    );
    assert_eq!(
        got, 100,
        "every published result must reach InfluxDB despite the 503s"
    );
    assert_eq!(
        queue.dropped_batches(),
        0,
        "nothing should have been evicted"
    );
}

/// A batch InfluxDB will never accept must not park at the head of the queue and
/// block the good ones behind it. This is what makes unbounded retry safe.
#[tokio::test]
async fn a_permanently_rejected_batch_does_not_block_the_queue() {
    let server = MockServer::start().await;
    let accepted = Arc::new(AtomicUsize::new(0));
    let accepted_mock = Arc::clone(&accepted);
    let attempts = Arc::new(AtomicUsize::new(0));

    // First write is permanently rejected (400), the rest are fine.
    Mock::given(method("POST"))
        .respond_with(move |req: &Request| {
            let n = attempts.fetch_add(1, Ordering::SeqCst);
            if n == 0 {
                ResponseTemplate::new(400).set_body_string("unable to parse")
            } else {
                accepted_mock.fetch_add(count_points(&req.body), Ordering::SeqCst);
                ResponseTemplate::new(204)
            }
        })
        .mount(&server)
        .await;

    let tf = token_file("mytoken");
    let cfg = influx_cfg(&server.uri(), &tf);
    let writer = Arc::new(InfluxWriter::new(&cfg).unwrap());
    let queue = Arc::new(RetryQueue::new(8 * 1024 * 1024));
    let (shutdown_tx, _) = broadcast::channel(1);

    queue.push(Arc::from("poisoned,probe_name=bad up=1i 1"));
    queue.push(Arc::from("probe_result,probe_name=good up=1i 2"));
    queue.close();

    let handle = tokio::spawn(run_writer(
        writer,
        Arc::clone(&queue),
        1,
        10,
        shutdown_tx.subscribe(),
    ));

    tokio::time::timeout(Duration::from_secs(5), handle)
        .await
        .expect("writer never finished — the rejected batch blocked the head")
        .unwrap();

    assert!(queue.is_empty());
    assert_eq!(
        accepted.load(Ordering::SeqCst),
        1,
        "the good batch behind the poisoned one must still be written"
    );
}
