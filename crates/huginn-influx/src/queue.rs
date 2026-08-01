//! A bounded FIFO of rendered line-protocol batches waiting to be written.
//!
//! This sits between the batcher and the writer so that retrying a failed write
//! never blocks reading from the EventHub. That separation is the whole design:
//! retrying inline would stop polling the broadcast receiver, the channel would
//! fill, and `Lagged` would drop results *at the source* — turning "some data
//! lost during a blip" into "everything lost during the outage", which is worse
//! than the bug it set out to fix.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;

use tokio::sync::Notify;
use tracing::warn;

/// A batch of line protocol, ready to POST.
///
/// `Arc<str>` so the writer can hold a batch across a retry without cloning the
/// payload — a peek is a refcount bump.
pub type Batch = std::sync::Arc<str>;

struct Inner {
    batches: VecDeque<Batch>,
    bytes: usize,
}

/// Bounded queue with drop-oldest eviction.
pub struct RetryQueue {
    // std::sync::Mutex, never held across an await — no async lock needed.
    inner: Mutex<Inner>,
    capacity_bytes: usize,
    notify: Notify,
    closed: AtomicBool,
    dropped_batches: AtomicU64,
    dropped_bytes: AtomicU64,
}

impl RetryQueue {
    pub fn new(capacity_bytes: usize) -> Self {
        Self {
            inner: Mutex::new(Inner {
                batches: VecDeque::new(),
                bytes: 0,
            }),
            capacity_bytes,
            notify: Notify::new(),
            closed: AtomicBool::new(false),
            dropped_batches: AtomicU64::new(0),
            dropped_bytes: AtomicU64::new(0),
        }
    }

    /// Append a batch, evicting the oldest ones if that would exceed capacity.
    ///
    /// Drop-oldest, not drop-newest, and it matters: during a long outage
    /// drop-newest would pin the *oldest* batches, so when InfluxDB came back
    /// you would write hours-stale points while still discarding current ones.
    /// Drop-oldest keeps the most recent window and catches up cleanly. (This is
    /// also why `tokio::mpsc` can't be used here — `try_send` fails, which drops
    /// the newest.)
    pub fn push(&self, batch: Batch) {
        let len = batch.len();
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());

        // A single batch larger than the whole capacity is still accepted: a hard
        // rejection would drop it forever, every time, for no benefit.
        if len > self.capacity_bytes {
            warn!(
                batch_bytes = len,
                capacity_bytes = self.capacity_bytes,
                "single InfluxDB batch exceeds max_buffered_bytes — buffering it anyway; \
                 consider lowering influx.batch_size"
            );
        }

        while !inner.batches.is_empty() && inner.bytes + len > self.capacity_bytes {
            if let Some(old) = inner.batches.pop_front() {
                inner.bytes -= old.len();
                self.dropped_batches.fetch_add(1, Ordering::Relaxed);
                self.dropped_bytes
                    .fetch_add(old.len() as u64, Ordering::Relaxed);
            }
        }

        inner.bytes += len;
        inner.batches.push_back(batch);
        drop(inner);

        self.notify.notify_one();
    }

    /// The oldest batch, without removing it.
    ///
    /// Peek-then-pop, so a batch is only removed once it has actually been
    /// written. A crash or a shutdown mid-retry leaves it queued.
    pub fn peek(&self) -> Option<Batch> {
        let inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.batches.front().cloned()
    }

    /// Remove the front batch, but only if it is still `written` (by identity).
    /// Returns whether it was removed. Call after a successful write, or to
    /// discard a permanently-rejected batch.
    ///
    /// The writer holds a peeked batch across a possibly-long retry. Meanwhile a
    /// `push` can evict that exact batch (drop-oldest) if the queue fills during
    /// the outage. An unconditional pop would then discard a *different*,
    /// unwritten batch that has since moved to the front — silent data loss in
    /// precisely the overflow scenario this queue exists to handle. Matching on
    /// `Arc::ptr_eq` makes the removal exactly "the batch I just wrote, if it is
    /// still here"; if it was already evicted, there is nothing to do.
    pub fn pop_if_front(&self, written: &Batch) -> bool {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        match inner.batches.front() {
            Some(front) if std::sync::Arc::ptr_eq(front, written) => {
                let b = inner.batches.pop_front().expect("front was just checked");
                inner.bytes -= b.len();
                true
            }
            _ => false,
        }
    }

    /// Wait until a batch is available, or the queue is closed and drained.
    ///
    /// Returns `None` only when closed *and* empty, which is what lets the
    /// writer finish its drain and exit.
    pub async fn wait_for_batch(&self) -> Option<Batch> {
        loop {
            if let Some(b) = self.peek() {
                return Some(b);
            }
            if self.closed.load(Ordering::Acquire) {
                return None;
            }
            // Register before re-checking: a push between the peek above and the
            // await would otherwise be missed and this would hang until the next.
            let notified = self.notify.notified();
            if let Some(b) = self.peek() {
                return Some(b);
            }
            if self.closed.load(Ordering::Acquire) {
                return None;
            }
            notified.await;
        }
    }

    /// Signal that no more batches will arrive. Wakes any waiter.
    pub fn close(&self) {
        self.closed.store(true, Ordering::Release);
        self.notify.notify_waiters();
    }

    pub fn is_empty(&self) -> bool {
        let inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.batches.is_empty()
    }

    pub fn len(&self) -> usize {
        let inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.batches.len()
    }

    pub fn bytes(&self) -> usize {
        let inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.bytes
    }

    /// How many batches were evicted because the queue was full.
    pub fn dropped_batches(&self) -> u64 {
        self.dropped_batches.load(Ordering::Relaxed)
    }

    pub fn dropped_bytes(&self) -> u64 {
        self.dropped_bytes.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::time::Duration;

    fn batch(s: &str) -> Batch {
        Arc::from(s)
    }

    #[test]
    fn fifo_order() {
        let q = RetryQueue::new(1024);
        q.push(batch("first"));
        q.push(batch("second"));

        let first = q.peek().unwrap();
        assert_eq!(&*first, "first");
        assert!(q.pop_if_front(&first));
        let second = q.peek().unwrap();
        assert_eq!(&*second, "second");
        assert!(q.pop_if_front(&second));
        assert!(q.peek().is_none());
    }

    /// The peek-then-pop invariant under overflow: if the batch the writer held
    /// was evicted (drop-oldest) and a different batch is now at the front,
    /// popping must NOT remove that unwritten front.
    #[test]
    fn pop_if_front_ignores_a_batch_that_is_no_longer_front() {
        let q = RetryQueue::new(12); // fits two 5-byte batches
        q.push(batch("aaaaa"));
        let held = q.peek().unwrap(); // writer peeks "aaaaa"
        q.push(batch("bbbbb"));
        q.push(batch("ccccc")); // evicts the held "aaaaa"

        assert!(!q.pop_if_front(&held), "must not pop an evicted batch");
        assert_eq!(&*q.peek().unwrap(), "bbbbb", "unwritten front must survive");
        assert_eq!(q.len(), 2);
    }

    #[test]
    fn peek_does_not_remove() {
        let q = RetryQueue::new(1024);
        q.push(batch("only"));
        assert_eq!(&*q.peek().unwrap(), "only");
        assert_eq!(&*q.peek().unwrap(), "only");
        assert_eq!(q.len(), 1);
    }

    /// The eviction policy, and the reason for it: the newest data survives.
    #[test]
    fn eviction_drops_oldest_not_newest() {
        // Capacity fits two 5-byte batches, not three.
        let q = RetryQueue::new(12);
        q.push(batch("aaaaa"));
        q.push(batch("bbbbb"));
        q.push(batch("ccccc")); // must evict "aaaaa"

        assert_eq!(q.len(), 2);
        let front = q.peek().unwrap();
        assert_eq!(&*front, "bbbbb", "oldest should have been dropped");
        assert!(q.pop_if_front(&front));
        assert_eq!(&*q.peek().unwrap(), "ccccc");
        assert_eq!(q.dropped_batches(), 1);
        assert_eq!(q.dropped_bytes(), 5);
    }

    #[test]
    fn bytes_tracks_contents() {
        let q = RetryQueue::new(1024);
        assert_eq!(q.bytes(), 0);
        q.push(batch("12345"));
        assert_eq!(q.bytes(), 5);
        q.push(batch("123"));
        assert_eq!(q.bytes(), 8);
        let front = q.peek().unwrap();
        assert!(q.pop_if_front(&front));
        assert_eq!(q.bytes(), 3);
    }

    /// Rejecting it outright would drop that batch forever, every time.
    #[test]
    fn oversized_single_batch_is_still_queued() {
        let q = RetryQueue::new(4);
        q.push(batch("way too long for this queue"));
        assert_eq!(q.len(), 1);
        assert!(q.peek().is_some());
    }

    #[tokio::test]
    async fn wait_for_batch_returns_none_once_closed_and_empty() {
        let q = RetryQueue::new(1024);
        q.close();
        assert!(q.wait_for_batch().await.is_none());
    }

    /// Closing must not discard what is already queued — that is the shutdown
    /// drain.
    #[tokio::test]
    async fn wait_for_batch_drains_before_reporting_closed() {
        let q = RetryQueue::new(1024);
        q.push(batch("pending"));
        q.close();

        let b = q.wait_for_batch().await.unwrap();
        assert_eq!(&*b, "pending");
        assert!(q.pop_if_front(&b));
        assert!(q.wait_for_batch().await.is_none());
    }

    #[tokio::test]
    async fn wait_for_batch_wakes_on_push() {
        let q = Arc::new(RetryQueue::new(1024));
        let q2 = Arc::clone(&q);

        let waiter = tokio::spawn(async move { q2.wait_for_batch().await.map(|b| b.to_string()) });

        tokio::time::sleep(Duration::from_millis(20)).await;
        q.push(batch("late arrival"));

        let got = tokio::time::timeout(Duration::from_secs(2), waiter)
            .await
            .expect("waiter did not wake on push")
            .unwrap();
        assert_eq!(got.as_deref(), Some("late arrival"));
    }

    #[tokio::test]
    async fn wait_for_batch_wakes_on_close() {
        let q = Arc::new(RetryQueue::new(1024));
        let q2 = Arc::clone(&q);

        let waiter = tokio::spawn(async move { q2.wait_for_batch().await });

        tokio::time::sleep(Duration::from_millis(20)).await;
        q.close();

        let got = tokio::time::timeout(Duration::from_secs(2), waiter)
            .await
            .expect("waiter did not wake on close")
            .unwrap();
        assert!(got.is_none());
    }
}
