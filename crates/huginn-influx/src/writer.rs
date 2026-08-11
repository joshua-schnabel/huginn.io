use std::sync::Arc;

use huginn_core::config::InfluxConfig;
use huginn_core::error::{HuginError, Result};
use huginn_core::event::ProbeEvent;
use huginn_core::stats::WriteStats;
use huginn_core::types::ProbeResult;
use std::time::Duration;
use tokio::sync::broadcast;
use tracing::{debug, error, warn};

use crate::queue::RetryQueue;

/// Upper bound on a single write to InfluxDB. Generous — a batch is a few KiB of
/// line protocol — but finite, which is the point.
const HTTP_TIMEOUT: Duration = Duration::from_secs(10);

/// Why a write failed, and — the part that matters — whether trying again could
/// possibly help.
///
/// The previous code collapsed every failure into one opaque
/// `HuginError::Influx(String)`. With unbounded retry that is unusable: a batch
/// InfluxDB will *never* accept (malformed line protocol, wrong bucket, bad
/// token) would sit at the head of the queue being retried forever, blocking
/// every good batch behind it. Classification is what makes head-of-line
/// blocking impossible.
#[derive(Debug, thiserror::Error)]
pub enum WriteError {
    /// Connection refused, DNS failure, TLS problem, timeout — the server never
    /// gave a verdict, so the batch is still good.
    #[error("transport error: {0}")]
    Transport(String),

    /// InfluxDB answered, but with a fault of its own (5xx) or backpressure
    /// (429). The batch is fine; the server isn't, yet.
    #[error("InfluxDB returned HTTP {status}: {body}")]
    Server {
        status: u16,
        body: String,
        /// Seconds from a `Retry-After` header, when the server sent one.
        retry_after_secs: Option<u64>,
    },

    /// InfluxDB rejected the request itself (4xx). Retrying sends the identical
    /// bytes to the identical endpoint and gets the identical answer.
    #[error("InfluxDB rejected the write: HTTP {status}: {body}")]
    Client { status: u16, body: String },
}

impl WriteError {
    /// Whether another attempt could plausibly succeed.
    pub fn is_retryable(&self) -> bool {
        !matches!(self, Self::Client { .. })
    }

    /// A server-supplied `Retry-After`, which InfluxDB Cloud sends on 429.
    pub fn retry_after_secs(&self) -> Option<u64> {
        match self {
            Self::Server {
                retry_after_secs, ..
            } => *retry_after_secs,
            _ => None,
        }
    }
}

/// Writes probe results to InfluxDB 2.x via the HTTP line-protocol API.
pub struct InfluxWriter {
    client: reqwest::Client,
    write_url: String,
    token: String,
}

impl InfluxWriter {
    /// Create a new writer. Reads the token from the file specified in config.
    pub fn new(cfg: &InfluxConfig) -> Result<Self> {
        let token = cfg.read_token()?;
        let write_url = format!(
            "{}/api/v2/write?org={}&bucket={}&precision=ms",
            cfg.url.trim_end_matches('/'),
            urlencode(&cfg.org),
            urlencode(&cfg.bucket),
        );
        let client = reqwest::Client::builder()
            .use_rustls_tls()
            // Without this the client waits forever. An InfluxDB that blackholes
            // packets rather than refusing them would then hang the batch
            // subscriber indefinitely — including the flush it performs on
            // shutdown, which would stop the process from exiting at all.
            .timeout(HTTP_TIMEOUT)
            .build()
            .map_err(|e| HuginError::Influx(e.to_string()))?;
        Ok(Self {
            client,
            write_url,
            token,
        })
    }

    /// Write one or more pre-formatted line-protocol lines (newline-separated).
    ///
    /// Failures are classified (see [`WriteError`]) so the caller can tell a
    /// transient outage from a batch that will never be accepted.
    pub async fn write_lines(&self, lines: &str) -> std::result::Result<(), WriteError> {
        debug!(lines = %lines, "writing to InfluxDB");

        let resp = self
            .client
            .post(&self.write_url)
            .header("Authorization", format!("Token {}", self.token))
            .header("Content-Type", "text/plain; charset=utf-8")
            .body(lines.to_owned())
            .send()
            .await
            .map_err(|e| WriteError::Transport(e.to_string()))?;

        if resp.status().is_success() {
            return Ok(());
        }

        let status = resp.status().as_u16();
        let retry_after_secs = resp
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.trim().parse::<u64>().ok());
        let body = resp.text().await.unwrap_or_default();

        // 408 and 429 are 4xx but transient: a request timeout and explicit
        // backpressure both mean "later", not "never".
        let err = if status >= 500 || status == 429 || status == 408 {
            warn!(status, body = %body, "InfluxDB write failed — will retry");
            WriteError::Server {
                status,
                body,
                retry_after_secs,
            }
        } else {
            error!(
                status,
                body = %body,
                "InfluxDB rejected the write — discarding this batch, retrying it would \
                 send identical bytes to the same endpoint"
            );
            WriteError::Client { status, body }
        };
        Err(err)
    }
}

/// Group results from `rx` into batches and hand them to `queue`.
///
/// This task never awaits I/O. That is the point: it is the only thing reading
/// the broadcast channel, and if it stalled — as it did when the flush was
/// inline — the channel would fill and `Lagged` would discard results before
/// they were ever buffered.
///
/// The receiver is subscribed by the caller *before* this task is spawned, so no
/// result published between spawn and first poll is missed (a receiver only sees
/// events sent after it subscribed). It takes a `Receiver`, not the hub, for
/// exactly that reason.
///
/// Flushes on `batch_size` results or after `batch_timeout_ms`, whichever comes
/// first. Renders line protocol exactly once, here.
pub async fn run_batcher(
    mut rx: broadcast::Receiver<ProbeEvent>,
    queue: Arc<RetryQueue>,
    batch_size: usize,
    batch_timeout_ms: u64,
) {
    let mut buffer: Vec<ProbeResult> = Vec::with_capacity(batch_size);
    let timeout_dur = Duration::from_millis(batch_timeout_ms);
    let mut flush_deadline = Box::pin(tokio::time::sleep(timeout_dur));

    loop {
        tokio::select! {
            event = rx.recv() => {
                match event {
                    Ok(ProbeEvent::ProbeCompleted(result)) => {
                        buffer.push(result);
                        if buffer.len() >= batch_size {
                            enqueue(&queue, &mut buffer);
                            flush_deadline = Box::pin(tokio::time::sleep(timeout_dur));
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        error!("InfluxDB batcher dropped {n} events (channel lagged)");
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        enqueue(&queue, &mut buffer);
                        // Tells the writer no more work is coming, so it can
                        // drain what's left and exit.
                        queue.close();
                        debug!("EventHub closed — InfluxDB batcher exiting");
                        break;
                    }
                }
            }
            _ = &mut flush_deadline => {
                enqueue(&queue, &mut buffer);
                flush_deadline = Box::pin(tokio::time::sleep(timeout_dur));
            }
        }
    }
}

/// Render the buffer to line protocol and queue it. No-op when empty.
fn enqueue(queue: &RetryQueue, buffer: &mut Vec<ProbeResult>) {
    if buffer.is_empty() {
        return;
    }
    let lines = buffer
        .iter()
        .map(to_line_protocol)
        .collect::<Vec<_>>()
        .join("\n");
    debug!(
        count = buffer.len(),
        bytes = lines.len(),
        "queueing InfluxDB batch"
    );
    queue.push(Arc::from(lines.as_str()));
    buffer.clear();
}

/// Drain `queue`, writing each batch to InfluxDB and retrying on transient
/// failure.
///
/// Retry of the head batch is **unbounded**, deliberately. Capping attempts is
/// self-defeating: during an outage every batch would exhaust its attempts in
/// seconds and be discarded, the queue would never fill, `max_buffered_bytes`
/// would never engage, and the buffer would be decorative — everything lost
/// anyway. The real bound is memory: the queue evicts oldest when full.
///
/// Unbounded retry is only safe because [`WriteError::is_retryable`] discards
/// batches InfluxDB will never accept, so a poisoned batch cannot block the
/// head — and because `shutdown_rx` caps the drain at exit.
pub async fn run_writer(
    writer: Arc<InfluxWriter>,
    queue: Arc<RetryQueue>,
    initial_backoff_ms: u64,
    max_backoff_ms: u64,
    stats: Arc<WriteStats>,
    mut shutdown_rx: broadcast::Receiver<()>,
) {
    let initial = Duration::from_millis(initial_backoff_ms);
    let max = Duration::from_millis(max_backoff_ms);

    while let Some(batch) = queue.wait_for_batch().await {
        let mut attempt: u32 = 0;

        loop {
            match writer.write_lines(&batch).await {
                Ok(()) => {
                    stats.record_written(batch.len() as u64, now_unix());
                    queue.pop_if_front(&batch);
                    break;
                }
                Err(e) if !e.is_retryable() => {
                    // Permanent. Dropping it is what keeps the head moving.
                    error!(error = %e, "discarding InfluxDB batch — not retryable");
                    stats.record_rejected();
                    queue.pop_if_front(&batch);
                    break;
                }
                Err(e) => {
                    let delay = e
                        .retry_after_secs()
                        .map(Duration::from_secs)
                        .unwrap_or_else(|| backoff_delay(initial, max, attempt))
                        .min(max);
                    warn!(
                        error = %e,
                        attempt = attempt + 1,
                        delay_ms = delay.as_millis() as u64,
                        queued_batches = queue.len(),
                        "InfluxDB write failed — retrying"
                    );
                    attempt = attempt.saturating_add(1);

                    // The sleep sits inside the select so a shutdown during a
                    // long backoff isn't stuck waiting it out.
                    tokio::select! {
                        _ = tokio::time::sleep(delay) => {}
                        _ = shutdown_rx.recv() => {
                            debug!("shutdown during backoff — abandoning retry");
                            return;
                        }
                    }
                }
            }
        }
    }

    let dropped = queue.dropped_batches();
    if dropped > 0 {
        warn!(
            dropped_batches = dropped,
            dropped_bytes = queue.dropped_bytes(),
            "InfluxDB writer exiting — batches were evicted while the queue was full"
        );
    }
    debug!("InfluxDB writer exiting");
}

/// Wall-clock seconds since the epoch, for the "last successful write" gauge.
///
/// Saturates to 0 before 1970 rather than propagating an error: the only caller
/// is a metric, and a clock that far wrong is not something a write path should
/// refuse to work over.
fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// `initial * 2^attempt`, capped at `max`.
///
/// No jitter, on purpose: jitter de-synchronises a thundering herd, and huginn
/// has exactly one writer task per process. Revisit if many instances ever point
/// at one InfluxDB.
fn backoff_delay(initial: Duration, max: Duration, attempt: u32) -> Duration {
    initial
        .checked_mul(1u32.checked_shl(attempt.min(31)).unwrap_or(u32::MAX))
        .unwrap_or(max)
        .min(max)
}

/// Build an InfluxDB line-protocol string from a ProbeResult.
pub fn to_line_protocol(r: &ProbeResult) -> String {
    let ts_ms = r.timestamp.timestamp_millis();
    let up_val = if r.up { 1i64 } else { 0i64 };

    let mut line = format!(
        "probe_result,probe_name={},probe_type={},target={} up={}i,response_ms={:.3}",
        escape_tag(&r.probe_name),
        escape_tag(&r.probe_type),
        escape_tag(&r.target),
        up_val,
        r.response_ms,
    );

    if let Some(code) = r.status_code {
        line.push_str(&format!(",status_code={}i", code));
    }
    if let Some(err) = &r.error {
        line.push_str(&format!(",error=\"{}\"", escape_field_str(err)));
    }
    // Probe-type-specific readings (TLS expiry, …). InfluxDB is
    // schemaless, so new keys need no migration. BTreeMap ⇒ deterministic order.
    for (key, value) in &r.metrics {
        line.push_str(&format!(",{}={}", escape_tag(key), value));
    }

    line.push_str(&format!(" {}", ts_ms));
    line
}

/// Escape a tag key/value for line protocol.
///
/// The backslash must go first: a value ending in one (`host\`) would otherwise
/// escape the delimiter that follows it and swallow the next tag.
fn escape_tag(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace(',', "\\,")
        .replace('=', "\\=")
        .replace(' ', "\\ ")
}

/// Escape a quoted string field value for line protocol.
///
/// Newlines matter as much as quotes here: line protocol is newline-delimited,
/// so a raw `\n` inside a field ends the line early and corrupts every
/// subsequent point in the same batch POST. Probe errors are the one field
/// carrying arbitrary text — reqwest and TLS errors do contain newlines — so
/// they are rendered as the two-character sequence `\n` instead. Backslash is
/// escaped first; the backslashes introduced afterwards are deliberate.
fn escape_field_str(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
}

/// Percent-encode a string for use in a query parameter.
///
/// Encodes UTF-8 *bytes*, not `char`s: encoding code points emits Latin-1 for
/// U+0080..U+00FF ("ä" as %E4 rather than %C3%A4) and invalid multi-byte output
/// above U+00FF, so a non-ASCII org or bucket name produced a broken URL.
fn urlencode(s: &str) -> String {
    s.bytes()
        .flat_map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                vec![b as char]
            }
            b => format!("%{:02X}", b).chars().collect(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use huginn_core::types::ProbeResult;
    use std::io::Write as _;
    use wiremock::matchers::{header_exists, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn fixed_result(up: bool) -> ProbeResult {
        ProbeResult {
            probe_name: "web".into(),
            probe_type: "http".into(),
            target: "https://example.com".into(),
            up,
            response_ms: 42.123,
            status_code: if up { Some(200) } else { None },
            error: if up { None } else { Some("timeout".into()) },
            timestamp: chrono::Utc.with_ymd_and_hms(2024, 1, 15, 10, 0, 0).unwrap(),
            metrics: Default::default(),
        }
    }

    /// Create a `NamedTempFile` pre-filled with `content`.
    /// The file is deleted automatically when the returned handle is dropped.
    fn token_file(content: &str) -> tempfile::NamedTempFile {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        write!(f, "{content}").unwrap();
        f
    }

    #[test]
    fn line_protocol_up() {
        let r = fixed_result(true);
        let line = to_line_protocol(&r);
        assert!(line.starts_with("probe_result,probe_name=web,probe_type=http"));
        assert!(line.contains("up=1i"));
        assert!(line.contains("response_ms=42.123"));
        assert!(line.contains("status_code=200i"));
        assert!(!line.contains("error="));
    }

    #[test]
    fn line_protocol_down() {
        let r = fixed_result(false);
        let line = to_line_protocol(&r);
        assert!(line.contains("up=0i"));
        assert!(line.contains("error=\"timeout\""));
        assert!(!line.contains("status_code="));
    }

    #[test]
    fn line_protocol_escapes_special_chars_in_tags() {
        let mut r = fixed_result(true);
        r.probe_name = "my probe,1".into();
        let line = to_line_protocol(&r);
        assert!(line.contains(r"probe_name=my\ probe\,1"));
    }

    /// A newline in an error must never reach the output: line protocol is
    /// newline-delimited, so a raw one ends the point early and every later
    /// point in the same batch POST is parsed as garbage.
    #[test]
    fn line_protocol_never_emits_a_raw_newline_from_an_error() {
        let mut r = fixed_result(false);
        r.error = Some("connect failed:\nno route to host\r\ngiving up".into());

        let line = to_line_protocol(&r);

        assert!(
            !line.contains('\n') && !line.contains('\r'),
            "raw newline leaked into line protocol: {line:?}"
        );
        assert!(line.contains(r#"error="connect failed:\nno route to host\r\ngiving up""#));
    }

    /// A batch is joined with newlines, so one bad point must not change how
    /// many lines the server sees.
    #[test]
    fn batch_with_newline_in_error_still_has_one_line_per_point() {
        let mut bad = fixed_result(false);
        bad.error = Some("line one\nline two".into());

        let batch = [fixed_result(true), bad, fixed_result(true)]
            .iter()
            .map(to_line_protocol)
            .collect::<Vec<_>>()
            .join("\n");

        assert_eq!(
            batch.lines().count(),
            3,
            "expected exactly 3 points, got:\n{batch}"
        );
    }

    /// A tag ending in a backslash would otherwise escape the delimiter that
    /// follows and swallow the next tag.
    #[test]
    fn line_protocol_escapes_trailing_backslash_in_tag() {
        let mut r = fixed_result(true);
        r.target = r"C:\path\".into();
        let line = to_line_protocol(&r);

        assert!(line.contains(r"target=C:\\path\\ "), "got: {line}");
        // The delimiter after the tag section must survive intact.
        assert!(line.contains(" up=1i"), "field section lost: {line}");
    }

    #[test]
    fn line_protocol_includes_metrics() {
        let r = fixed_result(true).with_metric("tls_cert_expiry_days", 47.0);
        let line = to_line_protocol(&r);
        assert!(line.contains(",tls_cert_expiry_days=47"), "got: {line}");
    }

    /// BTreeMap ordering is the reason the field order is reproducible; a
    /// HashMap here would emit a different line on every call.
    #[test]
    fn line_protocol_metric_order_is_deterministic() {
        let r = fixed_result(true)
            .with_metric("zzz_last", 2.0)
            .with_metric("aaa_first", 1.0);
        let line = to_line_protocol(&r);
        let first = line.find("aaa_first").expect("aaa_first missing");
        let last = line.find("zzz_last").expect("zzz_last missing");
        assert!(first < last, "metrics not sorted: {line}");
    }

    /// A probe with no metrics must produce exactly the line it always did.
    #[test]
    fn line_protocol_unchanged_without_metrics() {
        let line = to_line_protocol(&fixed_result(true));
        assert!(!line.contains(",tls_"), "unexpected metric fields: {line}");
        assert!(line.ends_with(" 1705312800000"), "timestamp moved: {line}");
    }

    #[test]
    fn urlencode_encodes_utf8_bytes_not_code_points() {
        // "ä" is U+00E4 but C3 A4 in UTF-8. Encoding the code point would emit
        // the Latin-1 %E4, which InfluxDB would reject or misread.
        assert_eq!(urlencode("mät"), "m%C3%A4t");
        assert_eq!(urlencode("a b&c"), "a%20b%26c");
        assert_eq!(urlencode("plain-org_1.0~x"), "plain-org_1.0~x");
    }

    #[tokio::test]
    async fn writer_posts_to_influx() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v2/write"))
            .and(header_exists("Authorization"))
            .respond_with(ResponseTemplate::new(204))
            .mount(&server)
            .await;

        let tf = token_file("mytoken");
        let cfg = huginn_core::config::InfluxConfig {
            url: server.uri(),
            org: "testorg".into(),
            bucket: "testbucket".into(),
            token_file: tf.path().to_string_lossy().into_owned(),
            batch_size: 10,
            batch_timeout_ms: 1000,
            ..Default::default()
        };

        let writer = InfluxWriter::new(&cfg).unwrap();
        writer
            .write_lines(&to_line_protocol(&fixed_result(true)))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn writer_returns_error_on_non_2xx() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(401).set_body_string("unauthorized"))
            .mount(&server)
            .await;

        let tf = token_file("bad-token");
        let cfg = huginn_core::config::InfluxConfig {
            url: server.uri(),
            org: "o".into(),
            bucket: "b".into(),
            token_file: tf.path().to_string_lossy().into_owned(),
            batch_size: 10,
            batch_timeout_ms: 1000,
            ..Default::default()
        };

        let writer = InfluxWriter::new(&cfg).unwrap();
        let result = writer
            .write_lines(&to_line_protocol(&fixed_result(true)))
            .await;
        assert!(result.is_err());
    }

    // --- batch subscriber -----------------------------------------------------

    fn influx_cfg(url: &str, tf: &tempfile::NamedTempFile) -> huginn_core::config::InfluxConfig {
        huginn_core::config::InfluxConfig {
            url: url.to_string(),
            org: "org".into(),
            bucket: "bkt".into(),
            token_file: tf.path().to_string_lossy().into_owned(),
            batch_size: 10,
            batch_timeout_ms: 60_000,
            ..Default::default()
        }
    }

    /// Closing the hub must make the batcher return, or shutdown hangs.
    #[tokio::test]
    async fn batcher_exits_cleanly_when_hub_closed() {
        use huginn_core::event::EventHub;
        use std::time::Duration;

        let hub = Arc::new(EventHub::new(16));
        let queue = Arc::new(RetryQueue::new(
            1024 * 1024,
            Arc::new(WriteStats::default()),
        ));

        let handle = tokio::spawn(run_batcher(hub.subscribe(), Arc::clone(&queue), 10, 60_000));

        drop(hub);

        // Fast now: the batcher never touches the network, it only queues.
        tokio::time::timeout(Duration::from_secs(2), handle)
            .await
            .expect("batcher did not exit within 2s after the hub closed")
            .expect("batcher task panicked");

        assert!(
            queue.is_empty(),
            "nothing was published, so nothing to queue"
        );
    }

    /// A lagged broadcast receiver must not take the batcher down.
    #[tokio::test]
    async fn batcher_survives_lagged_events() {
        use huginn_core::event::{EventHub, ProbeEvent};
        use std::time::Duration;

        // capacity=1: any second publish before recv() is processed causes Lagged.
        let hub = Arc::new(EventHub::new(1));
        let queue = Arc::new(RetryQueue::new(
            1024 * 1024,
            Arc::new(WriteStats::default()),
        ));
        let handle = tokio::spawn(run_batcher(hub.subscribe(), Arc::clone(&queue), 10, 60_000));

        // Let the task park in rx.recv().await.
        tokio::task::yield_now().await;

        // Flood synchronously so the receiver sees Lagged.
        for _ in 0..5 {
            hub.publish(ProbeEvent::ProbeCompleted(fixed_result(true)));
        }

        tokio::task::yield_now().await;
        drop(hub);

        tokio::time::timeout(Duration::from_secs(2), handle)
            .await
            .expect("batcher did not exit within 2s")
            .expect("batcher task panicked on a lagged receiver");
    }

    /// The batcher must queue what it holds when the hub closes, not drop it —
    /// this is the shutdown drain's first half.
    #[tokio::test]
    async fn batcher_queues_partial_batch_on_close() {
        use huginn_core::event::{EventHub, ProbeEvent};
        use std::time::Duration;

        let hub = Arc::new(EventHub::new(16));
        let queue = Arc::new(RetryQueue::new(
            1024 * 1024,
            Arc::new(WriteStats::default()),
        ));
        // batch_size 10 and a long timeout: 3 results would otherwise sit in the
        // buffer forever.
        let handle = tokio::spawn(run_batcher(hub.subscribe(), Arc::clone(&queue), 10, 60_000));
        tokio::task::yield_now().await;

        for _ in 0..3 {
            hub.publish(ProbeEvent::ProbeCompleted(fixed_result(true)));
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
        drop(hub);

        tokio::time::timeout(Duration::from_secs(2), handle)
            .await
            .expect("batcher did not exit")
            .unwrap();

        assert_eq!(queue.len(), 1, "the partial batch must have been queued");
        let batch = queue.peek().unwrap();
        assert_eq!(batch.lines().count(), 3, "all 3 results must be in it");
    }

    /// Start the full batcher → queue → writer pipeline against `url`.
    ///
    /// Backoff is 1ms rather than tokio::time::pause(): wiremock does real I/O,
    /// which fights a paused clock.
    fn start_pipeline(
        url: &str,
        tf: &tempfile::NamedTempFile,
        batch_size: usize,
        batch_timeout_ms: u64,
        hub: &Arc<huginn_core::event::EventHub>,
    ) -> (Arc<RetryQueue>, broadcast::Sender<()>) {
        let cfg = influx_cfg(url, tf);
        let writer = Arc::new(InfluxWriter::new(&cfg).unwrap());
        let queue = Arc::new(RetryQueue::new(
            8 * 1024 * 1024,
            Arc::new(WriteStats::default()),
        ));
        let (shutdown_tx, _) = broadcast::channel(1);

        tokio::spawn(run_batcher(
            hub.subscribe(),
            Arc::clone(&queue),
            batch_size,
            batch_timeout_ms,
        ));
        tokio::spawn(run_writer(
            writer,
            Arc::clone(&queue),
            1,
            10,
            Arc::new(WriteStats::default()),
            shutdown_tx.subscribe(),
        ));
        (queue, shutdown_tx)
    }

    /// 10 events → exactly 1 POST (batch flushed on count)
    #[tokio::test]
    async fn batch_writer_flushes_on_count() {
        use huginn_core::event::{EventHub, ProbeEvent};
        use std::time::Duration;

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v2/write"))
            .and(header_exists("Authorization"))
            .respond_with(ResponseTemplate::new(204))
            .expect(1) // must be exactly 1 POST for 10 events
            .mount(&server)
            .await;

        let tf = token_file("mytoken");
        let hub = Arc::new(EventHub::new(256));
        let (_queue, _sd) = start_pipeline(&server.uri(), &tf, 10, 60_000, &hub);
        tokio::task::yield_now().await;

        for _ in 0..10 {
            hub.publish(ProbeEvent::ProbeCompleted(fixed_result(true)));
        }

        tokio::time::sleep(Duration::from_millis(200)).await;

        drop(hub);
        server.verify().await;
    }

    /// 3 events + wait → exactly 1 POST (batch flushed on timeout)
    #[tokio::test]
    async fn batch_writer_flushes_on_timeout() {
        use huginn_core::event::{EventHub, ProbeEvent};
        use std::time::Duration;

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v2/write"))
            .and(header_exists("Authorization"))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;

        let tf = token_file("mytoken");
        let hub = Arc::new(EventHub::new(256));
        // batch_size 10 won't be reached by 3 events; the 50ms timeout fires.
        let (_queue, _sd) = start_pipeline(&server.uri(), &tf, 10, 50, &hub);
        tokio::task::yield_now().await;

        for _ in 0..3 {
            hub.publish(ProbeEvent::ProbeCompleted(fixed_result(true)));
        }

        tokio::time::sleep(Duration::from_millis(300)).await;

        drop(hub);
        server.verify().await;
    }

    // --- retry ----------------------------------------------------------------

    #[test]
    fn client_errors_are_permanent_server_errors_are_not() {
        let permanent = [400u16, 401, 403, 404, 413, 422];
        for status in permanent {
            let e = WriteError::Client {
                status,
                body: String::new(),
            };
            assert!(!e.is_retryable(), "HTTP {status} must not be retried");
        }

        for status in [500u16, 502, 503, 504, 429, 408] {
            let e = WriteError::Server {
                status,
                body: String::new(),
                retry_after_secs: None,
            };
            assert!(e.is_retryable(), "HTTP {status} must be retried");
        }

        assert!(WriteError::Transport("refused".into()).is_retryable());
    }

    #[test]
    fn backoff_doubles_and_is_capped() {
        let initial = Duration::from_millis(500);
        let max = Duration::from_secs(30);

        assert_eq!(backoff_delay(initial, max, 0), Duration::from_millis(500));
        assert_eq!(backoff_delay(initial, max, 1), Duration::from_secs(1));
        assert_eq!(backoff_delay(initial, max, 2), Duration::from_secs(2));
        assert_eq!(
            backoff_delay(initial, max, 6),
            Duration::from_secs(30),
            "capped"
        );
        // Must not overflow into a panic or a tiny delay.
        assert_eq!(backoff_delay(initial, max, 99), max);
    }

    /// A 401 must be discarded, not retried forever — otherwise a bad token
    /// parks a batch at the head of the queue and blocks every good one behind it.
    #[tokio::test]
    async fn permanent_error_discards_the_batch_and_unblocks_the_queue() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(401).set_body_string("unauthorized"))
            .expect(1) // exactly one attempt — no retry
            .mount(&server)
            .await;

        let tf = token_file("bad-token");
        let cfg = influx_cfg(&server.uri(), &tf);
        let writer = Arc::new(InfluxWriter::new(&cfg).unwrap());
        let stats = Arc::new(WriteStats::default());
        let queue = Arc::new(RetryQueue::new(1024 * 1024, Arc::clone(&stats)));
        let (shutdown_tx, _) = broadcast::channel(1);

        queue.push(Arc::from("probe_result,probe_name=x up=1i 1"));
        queue.close();

        let handle = tokio::spawn(run_writer(
            writer,
            Arc::clone(&queue),
            1,
            10,
            Arc::clone(&stats),
            shutdown_tx.subscribe(),
        ));

        tokio::time::timeout(Duration::from_secs(5), handle)
            .await
            .expect("writer did not exit — a permanent error was retried")
            .unwrap();

        assert!(queue.is_empty(), "poisoned batch must be discarded");
        server.verify().await;

        // Discarding it is correct, and silently discarding it is the failure
        // this counter exists to make visible.
        assert_eq!(stats.rejected_batches(), 1);
        assert_eq!(
            stats.dropped_batches(),
            0,
            "a rejection is not a queue eviction — they have different causes and different fixes"
        );
        assert_eq!(stats.written_batches(), 0);
        assert_eq!(
            stats.last_write_success_unix(),
            0,
            "nothing was ever accepted, so the last-success gauge must stay at zero"
        );
    }

    /// The counters the write path reports about itself, on the happy path.
    #[tokio::test]
    async fn a_successful_write_is_counted_and_timestamped() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;

        let tf = token_file("tok");
        let cfg = influx_cfg(&server.uri(), &tf);
        let writer = Arc::new(InfluxWriter::new(&cfg).unwrap());
        let stats = Arc::new(WriteStats::default());
        let queue = Arc::new(RetryQueue::new(1024 * 1024, Arc::clone(&stats)));
        let (shutdown_tx, _) = broadcast::channel(1);

        let line = "probe_result,probe_name=x up=1i 1";
        let before = now_unix();
        queue.push(Arc::from(line));
        queue.close();

        let handle = tokio::spawn(run_writer(
            writer,
            Arc::clone(&queue),
            1,
            10,
            Arc::clone(&stats),
            shutdown_tx.subscribe(),
        ));

        tokio::time::timeout(Duration::from_secs(5), handle)
            .await
            .expect("writer did not exit")
            .unwrap();

        assert_eq!(stats.written_batches(), 1);
        assert_eq!(stats.written_bytes(), line.len() as u64);
        assert_eq!(stats.rejected_batches(), 0);
        assert!(
            stats.last_write_success_unix() >= before,
            "the last-success gauge must move forward on an accepted write"
        );
        assert_eq!(stats.queue_batches(), 0, "the queue drained");
        server.verify().await;
    }

    /// Transient failures must not lose data: the batch stays queued until the
    /// server recovers.
    #[tokio::test]
    async fn transient_error_is_retried_until_it_succeeds() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let server = MockServer::start().await;
        let calls = Arc::new(AtomicUsize::new(0));
        let calls2 = Arc::clone(&calls);

        // Fail twice with 503, then accept.
        Mock::given(method("POST"))
            .respond_with(move |_: &wiremock::Request| {
                let n = calls2.fetch_add(1, Ordering::SeqCst);
                if n < 2 {
                    ResponseTemplate::new(503).set_body_string("unavailable")
                } else {
                    ResponseTemplate::new(204)
                }
            })
            .mount(&server)
            .await;

        let tf = token_file("mytoken");
        let cfg = influx_cfg(&server.uri(), &tf);
        let writer = Arc::new(InfluxWriter::new(&cfg).unwrap());
        let queue = Arc::new(RetryQueue::new(
            1024 * 1024,
            Arc::new(WriteStats::default()),
        ));
        let (shutdown_tx, _) = broadcast::channel(1);

        queue.push(Arc::from("probe_result,probe_name=x up=1i 1"));
        queue.close();

        let handle = tokio::spawn(run_writer(
            writer,
            Arc::clone(&queue),
            1,
            10,
            Arc::new(WriteStats::default()),
            shutdown_tx.subscribe(),
        ));

        tokio::time::timeout(Duration::from_secs(5), handle)
            .await
            .expect("writer did not finish retrying")
            .unwrap();

        assert_eq!(
            calls.load(Ordering::SeqCst),
            3,
            "expected 2 failures + 1 success"
        );
        assert!(queue.is_empty(), "batch must be popped after success");
    }

    /// Unbounded retry must not mean an unbounded process: a shutdown during
    /// backoff has to break the loop.
    #[tokio::test]
    async fn shutdown_during_backoff_stops_the_writer() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;

        let tf = token_file("mytoken");
        let cfg = influx_cfg(&server.uri(), &tf);
        let writer = Arc::new(InfluxWriter::new(&cfg).unwrap());
        let queue = Arc::new(RetryQueue::new(
            1024 * 1024,
            Arc::new(WriteStats::default()),
        ));
        let (shutdown_tx, _) = broadcast::channel(1);

        queue.push(Arc::from("probe_result,probe_name=x up=1i 1"));

        // Long backoff so the writer is definitely sleeping when the signal lands.
        let handle = tokio::spawn(run_writer(
            writer,
            Arc::clone(&queue),
            10_000,
            60_000,
            Arc::new(WriteStats::default()),
            shutdown_tx.subscribe(),
        ));

        tokio::time::sleep(Duration::from_millis(100)).await;
        shutdown_tx.send(()).unwrap();

        tokio::time::timeout(Duration::from_secs(2), handle)
            .await
            .expect("writer kept sleeping through shutdown")
            .unwrap();
    }
}
