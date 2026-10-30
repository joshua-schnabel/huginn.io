//! Counters for the InfluxDB write path.
//!
//! These live here rather than next to the code that fills them because two
//! crates need them and neither may depend on the other: `huginn-influx` writes
//! them, `huginn-web` renders them, and the dependency graph points one way —
//! every crate to `huginn-core` and no further. Same reason the `EventHub` is
//! here.
//!
//! Why they exist at all: the probe gauges say whether a *target* is up. They
//! cannot say that a measurement was taken and then thrown away, which is what
//! happens when the retry queue evicts under a long outage or InfluxDB rejects a
//! batch permanently. A monitor that keeps reporting while silently discarding
//! its own data is the failure this project is built to avoid, and until now it
//! was the one failure huginn could not report about itself.

use std::sync::atomic::{AtomicU64, Ordering};

/// Write-path counters, shared between the queue, the writer and the exposition
/// endpoint.
///
/// `Relaxed` throughout, deliberately: each counter is read on its own, nothing
/// is published *through* them, and no reader draws a conclusion from two of
/// them being consistent with each other at one instant. A scrape that catches a
/// counter a few nanoseconds stale is indistinguishable from one taken a moment
/// earlier — which is the resolution Prometheus works at anyway.
#[derive(Debug, Default)]
pub struct WriteStats {
    queue_batches: AtomicU64,
    queue_bytes: AtomicU64,
    dropped_batches: AtomicU64,
    dropped_bytes: AtomicU64,
    rejected_batches: AtomicU64,
    written_batches: AtomicU64,
    written_bytes: AtomicU64,
    last_write_success_unix: AtomicU64,
}

impl WriteStats {
    /// Overwrite the queue depth. Called by the queue whenever it changes, under
    /// the lock that made the change, so the two numbers always describe the
    /// same moment.
    pub fn set_queue_depth(&self, batches: u64, bytes: u64) {
        self.queue_batches.store(batches, Ordering::Relaxed);
        self.queue_bytes.store(bytes, Ordering::Relaxed);
    }

    /// One batch evicted because the queue was full. This is data that was
    /// measured and will never be written.
    pub fn record_eviction(&self, bytes: u64) {
        self.dropped_batches.fetch_add(1, Ordering::Relaxed);
        self.dropped_bytes.fetch_add(bytes, Ordering::Relaxed);
    }

    /// One batch discarded because InfluxDB will never accept it — a 4xx that is
    /// not worth retrying. Also permanent loss, but with a different cause and a
    /// different fix, which is why it is counted apart from an eviction.
    pub fn record_rejected(&self) {
        self.rejected_batches.fetch_add(1, Ordering::Relaxed);
    }

    /// One batch accepted by InfluxDB. `at_unix` is the wall-clock second it
    /// happened, passed in because this crate deliberately knows nothing about
    /// clocks.
    pub fn record_written(&self, bytes: u64, at_unix: u64) {
        self.written_batches.fetch_add(1, Ordering::Relaxed);
        self.written_bytes.fetch_add(bytes, Ordering::Relaxed);
        self.last_write_success_unix
            .store(at_unix, Ordering::Relaxed);
    }

    pub fn queue_batches(&self) -> u64 {
        self.queue_batches.load(Ordering::Relaxed)
    }

    pub fn queue_bytes(&self) -> u64 {
        self.queue_bytes.load(Ordering::Relaxed)
    }

    pub fn dropped_batches(&self) -> u64 {
        self.dropped_batches.load(Ordering::Relaxed)
    }

    pub fn dropped_bytes(&self) -> u64 {
        self.dropped_bytes.load(Ordering::Relaxed)
    }

    pub fn rejected_batches(&self) -> u64 {
        self.rejected_batches.load(Ordering::Relaxed)
    }

    pub fn written_batches(&self) -> u64 {
        self.written_batches.load(Ordering::Relaxed)
    }

    pub fn written_bytes(&self) -> u64 {
        self.written_bytes.load(Ordering::Relaxed)
    }

    /// Unix seconds of the last accepted write, or `0` if none has succeeded
    /// since startup.
    ///
    /// Zero rather than an `Option` because the exposition format has no way to
    /// say "absent": a series that disappears between scrapes reads as a target
    /// going away, not as a value not existing yet. Zero is far enough in the
    /// past that any `time() - metric` alert fires on it, which is the intended
    /// reading — nothing has been written yet.
    pub fn last_write_success_unix(&self) -> u64 {
        self.last_write_success_unix.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counters_start_at_zero() {
        let s = WriteStats::default();
        assert_eq!(s.queue_batches(), 0);
        assert_eq!(s.queue_bytes(), 0);
        assert_eq!(s.dropped_batches(), 0);
        assert_eq!(s.dropped_bytes(), 0);
        assert_eq!(s.rejected_batches(), 0);
        assert_eq!(s.written_batches(), 0);
        assert_eq!(s.written_bytes(), 0);
        assert_eq!(s.last_write_success_unix(), 0);
    }

    #[test]
    fn evictions_accumulate_batches_and_bytes() {
        let s = WriteStats::default();
        s.record_eviction(120);
        s.record_eviction(80);
        assert_eq!(s.dropped_batches(), 2);
        assert_eq!(s.dropped_bytes(), 200);
    }

    #[test]
    fn rejections_are_counted_apart_from_evictions() {
        let s = WriteStats::default();
        s.record_rejected();
        assert_eq!(s.rejected_batches(), 1);
        assert_eq!(s.dropped_batches(), 0, "a rejection is not an eviction");
    }

    #[test]
    fn writes_accumulate_and_move_the_timestamp_forward() {
        let s = WriteStats::default();
        s.record_written(50, 1_700_000_000);
        s.record_written(70, 1_700_000_060);
        assert_eq!(s.written_batches(), 2);
        assert_eq!(s.written_bytes(), 120);
        assert_eq!(s.last_write_success_unix(), 1_700_000_060);
    }

    /// Depth is a gauge, not a counter: it is overwritten, never added to.
    #[test]
    fn queue_depth_is_replaced_rather_than_accumulated() {
        let s = WriteStats::default();
        s.set_queue_depth(3, 300);
        s.set_queue_depth(1, 100);
        assert_eq!(s.queue_batches(), 1);
        assert_eq!(s.queue_bytes(), 100);
    }
}
