use std::sync::Arc;

use huginn_core::config::InfluxConfig;
use huginn_core::error::{HuginError, Result};
use huginn_core::event::{EventHub, ProbeEvent};
use huginn_core::types::ProbeResult;
use std::time::Duration;
use tokio::sync::broadcast;
use tracing::{debug, error, warn};

/// Upper bound on a single write to InfluxDB. Generous — a batch is a few KiB of
/// line protocol — but finite, which is the point.
const HTTP_TIMEOUT: Duration = Duration::from_secs(10);

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
    pub async fn write_lines(&self, lines: &str) -> Result<()> {
        debug!(lines = %lines, "writing to InfluxDB");

        let resp = self
            .client
            .post(&self.write_url)
            .header("Authorization", format!("Token {}", self.token))
            .header("Content-Type", "text/plain; charset=utf-8")
            .body(lines.to_owned())
            .send()
            .await
            .map_err(|e| HuginError::Influx(e.to_string()))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            warn!(status = %status, body = %body, "InfluxDB write failed");
            return Err(HuginError::Influx(format!("HTTP {status}: {body}")));
        }
        Ok(())
    }
}

/// Buffers up to `batch_size` results or `batch_timeout_ms`
/// milliseconds (whichever comes first) before sending a single POST to InfluxDB.
pub async fn run_subscriber_batched(
    writer: Arc<InfluxWriter>,
    hub: Arc<EventHub>,
    batch_size: usize,
    batch_timeout_ms: u64,
) {
    let mut rx = hub.subscribe();
    drop(hub);

    let mut buffer: Vec<ProbeResult> = Vec::with_capacity(batch_size);
    let timeout_dur = Duration::from_millis(batch_timeout_ms);
    // Start the flush timer — resets after every flush.
    let mut flush_deadline = Box::pin(tokio::time::sleep(timeout_dur));

    loop {
        tokio::select! {
            event = rx.recv() => {
                match event {
                    Ok(ProbeEvent::ProbeCompleted(result)) => {
                        buffer.push(result);
                        if buffer.len() >= batch_size {
                            flush_buffer(&writer, &mut buffer).await;
                            flush_deadline = Box::pin(tokio::time::sleep(timeout_dur));
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        error!("InfluxDB batch subscriber dropped {n} events (channel lagged)");
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        if !buffer.is_empty() {
                            flush_buffer(&writer, &mut buffer).await;
                        }
                        debug!("EventHub closed — InfluxDB batch subscriber exiting");
                        break;
                    }
                }
            }
            _ = &mut flush_deadline => {
                if !buffer.is_empty() {
                    flush_buffer(&writer, &mut buffer).await;
                }
                flush_deadline = Box::pin(tokio::time::sleep(timeout_dur));
            }
        }
    }
}

async fn flush_buffer(writer: &InfluxWriter, buffer: &mut Vec<ProbeResult>) {
    let lines = buffer
        .iter()
        .map(to_line_protocol)
        .collect::<Vec<_>>()
        .join("\n");
    debug!(count = buffer.len(), "flushing InfluxDB batch");
    if let Err(e) = writer.write_lines(&lines).await {
        error!("InfluxDB batch write error: {e}");
    }
    buffer.clear();
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
        }
    }

    /// Closing the hub must make the subscriber return, or shutdown hangs.
    #[tokio::test]
    async fn batch_subscriber_exits_cleanly_when_hub_closed() {
        use huginn_core::event::EventHub;
        use std::time::Duration;

        let tf = token_file("mytoken");
        // Dead port: this test is about the exit path, not about writing.
        let cfg = influx_cfg("http://127.0.0.1:19999", &tf);
        let writer = Arc::new(InfluxWriter::new(&cfg).unwrap());
        let hub = Arc::new(EventHub::new(16));

        let handle = tokio::spawn(run_subscriber_batched(
            Arc::clone(&writer),
            Arc::clone(&hub),
            10,
            60_000,
        ));

        drop(hub);

        // 10s, not 2: the close path flushes the buffer, and on Windows a
        // connect to a closed local port takes ~2s to be refused.
        tokio::time::timeout(Duration::from_secs(10), handle)
            .await
            .expect("batch subscriber did not exit within 10s after the hub closed")
            .expect("batch subscriber task panicked");
    }

    /// A lagged broadcast receiver must not take the subscriber down.
    #[tokio::test]
    async fn batch_subscriber_survives_lagged_events() {
        use huginn_core::event::{EventHub, ProbeEvent};
        use std::time::Duration;

        let tf = token_file("mytoken");
        let cfg = influx_cfg("http://127.0.0.1:19999", &tf);
        let writer = Arc::new(InfluxWriter::new(&cfg).unwrap());

        // capacity=1: any second publish before recv() is processed causes Lagged.
        let hub = Arc::new(EventHub::new(1));
        let handle = tokio::spawn(run_subscriber_batched(
            Arc::clone(&writer),
            Arc::clone(&hub),
            10,
            60_000,
        ));

        // Let the task park in rx.recv().await.
        tokio::task::yield_now().await;

        // Flood synchronously so the receiver sees Lagged.
        for _ in 0..5 {
            hub.publish(ProbeEvent::ProbeCompleted(fixed_result(true)));
        }

        tokio::task::yield_now().await;
        drop(hub);

        tokio::time::timeout(Duration::from_secs(10), handle)
            .await
            .expect("batch subscriber did not exit within 10s")
            .expect("batch subscriber task panicked on a lagged receiver");
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
        let cfg = influx_cfg(&server.uri(), &tf);
        let writer = Arc::new(InfluxWriter::new(&cfg).unwrap());
        let hub = Arc::new(EventHub::new(256));

        tokio::spawn(run_subscriber_batched(
            Arc::clone(&writer),
            Arc::clone(&hub),
            10,     // batch_size
            60_000, // 60s timeout — should never trigger in this test
        ));
        tokio::task::yield_now().await;

        for _ in 0..10 {
            hub.publish(ProbeEvent::ProbeCompleted(fixed_result(true)));
        }

        tokio::time::sleep(Duration::from_millis(200)).await;

        drop(hub);
        server.verify().await;
    }

    /// 3 events + 200ms wait → exactly 1 POST (batch flushed on timeout)
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
        let cfg = influx_cfg(&server.uri(), &tf);
        let writer = Arc::new(InfluxWriter::new(&cfg).unwrap());
        let hub = Arc::new(EventHub::new(256));

        tokio::spawn(run_subscriber_batched(
            Arc::clone(&writer),
            Arc::clone(&hub),
            10, // batch_size=10 — won't be reached by only 3 events
            50, // 50ms timeout — will trigger well before batch fills
        ));
        tokio::task::yield_now().await;

        for _ in 0..3 {
            hub.publish(ProbeEvent::ProbeCompleted(fixed_result(true)));
        }

        // Wait longer than the 50ms batch timeout
        tokio::time::sleep(Duration::from_millis(300)).await;

        drop(hub);
        server.verify().await;
    }
}
