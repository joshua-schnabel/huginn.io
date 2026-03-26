use std::sync::Arc;

use hugin_core::config::InfluxConfig;
use hugin_core::error::{HuginError, Result};
use hugin_core::event::{EventHub, ProbeEvent};
use hugin_core::types::ProbeResult;
use tokio::sync::broadcast;
use tracing::{debug, error, warn};

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
            urlenccode(&cfg.org),
            urlenccode(&cfg.bucket),
        );
        let client = reqwest::Client::builder()
            .use_rustls_tls()
            .build()
            .map_err(|e| HuginError::Influx(e.to_string()))?;
        Ok(Self { client, write_url, token })
    }

    /// Convert a `ProbeResult` to InfluxDB line protocol and write it.
    pub async fn write(&self, result: &ProbeResult) -> Result<()> {
        let line = to_line_protocol(result);
        self.write_lines(&line).await
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

/// Subscribe to `hub` and write every `ProbeCompleted` event to InfluxDB.
/// Returns when the event hub is closed (all senders dropped).
pub async fn run_subscriber(writer: Arc<InfluxWriter>, hub: Arc<EventHub>) {
    let mut rx = hub.subscribe();
    // Drop the Arc so this task doesn't keep the hub alive by itself.
    drop(hub);
    loop {
        match rx.recv().await {
            Ok(ProbeEvent::ProbeCompleted(result)) => {
                let w = Arc::clone(&writer);
                tokio::spawn(async move {
                    if let Err(e) = w.write(&result).await {
                        error!("InfluxDB write error: {e}");
                    }
                });
            }
            Err(broadcast::error::RecvError::Lagged(n)) => {
                error!("InfluxDB subscriber dropped {n} events (channel lagged)");
            }
            Err(broadcast::error::RecvError::Closed) => {
                debug!("EventHub closed — InfluxDB subscriber exiting");
                break;
            }
        }
    }
}

/// Batched variant: buffers up to `batch_size` results or `batch_timeout_ms`
/// milliseconds (whichever comes first) before sending a single POST to InfluxDB.
pub async fn run_subscriber_batched(
    writer: Arc<InfluxWriter>,
    hub: Arc<EventHub>,
    batch_size: usize,
    batch_timeout_ms: u64,
) {
    use std::time::Duration;

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

fn escape_tag(s: &str) -> String {
    s.replace(',', "\\,")
        .replace('=', "\\=")
        .replace(' ', "\\ ")
}

fn escape_field_str(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

fn urlenccode(s: &str) -> String {
    s.chars()
        .flat_map(|c| match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => {
                vec![c]
            }
            c => format!("%{:02X}", c as u32).chars().collect(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use hugin_core::types::ProbeResult;
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
        let cfg = hugin_core::config::InfluxConfig {
            url: server.uri(),
            org: "testorg".into(),
            bucket: "testbucket".into(),
            token_file: tf.path().to_string_lossy().into_owned(),
            batch_size: 10,
            batch_timeout_ms: 1000,
        };

        let writer = InfluxWriter::new(&cfg).unwrap();
        writer.write(&fixed_result(true)).await.unwrap();
    }

    #[tokio::test]
    async fn writer_returns_error_on_non_2xx() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(401).set_body_string("unauthorized"))
            .mount(&server)
            .await;

        let tf = token_file("bad-token");
        let cfg = hugin_core::config::InfluxConfig {
            url: server.uri(),
            org: "o".into(),
            bucket: "b".into(),
            token_file: tf.path().to_string_lossy().into_owned(),
            batch_size: 10,
            batch_timeout_ms: 1000,
        };

        let writer = InfluxWriter::new(&cfg).unwrap();
        let result = writer.write(&fixed_result(true)).await;
        assert!(result.is_err());
    }

    // --- run_subscriber -------------------------------------------------------

    #[tokio::test]
    async fn subscriber_writes_to_influx_on_probe_completed() {
        use hugin_core::event::{EventHub, ProbeEvent};
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
        let cfg = hugin_core::config::InfluxConfig {
            url: server.uri(),
            org: "org".into(),
            bucket: "bkt".into(),
            token_file: tf.path().to_string_lossy().into_owned(),
            batch_size: 10,
            batch_timeout_ms: 1000,
        };

        let writer = Arc::new(InfluxWriter::new(&cfg).unwrap());
        let hub = Arc::new(EventHub::new(16));

        tokio::spawn(run_subscriber(Arc::clone(&writer), Arc::clone(&hub)));

        // Give the subscriber task time to start and call hub.subscribe()
        tokio::time::sleep(Duration::from_millis(20)).await;

        hub.publish(ProbeEvent::ProbeCompleted(fixed_result(true)));

        // Give the spawned write task time to complete
        tokio::time::sleep(Duration::from_millis(200)).await;

        server.verify().await;
    }

    #[tokio::test]
    async fn subscriber_exits_cleanly_when_hub_closed() {
        use hugin_core::event::EventHub;
        use std::time::Duration;

        let tf = token_file("mytoken");
        let cfg = hugin_core::config::InfluxConfig {
            url: "http://127.0.0.1:19999".into(),
            org: "o".into(),
            bucket: "b".into(),
            token_file: tf.path().to_string_lossy().into_owned(),
            batch_size: 10,
            batch_timeout_ms: 1000,
        };

        let writer = Arc::new(InfluxWriter::new(&cfg).unwrap());
        let hub = Arc::new(EventHub::new(16));

        let handle = tokio::spawn(run_subscriber(Arc::clone(&writer), Arc::clone(&hub)));

        drop(hub); // closing the hub must cause run_subscriber to return

        tokio::time::timeout(Duration::from_secs(1), handle)
            .await
            .expect("subscriber did not exit within 1s after hub was closed")
            .expect("subscriber task panicked");
    }

    #[tokio::test]
    async fn subscriber_handles_lagged_events() {        use hugin_core::event::{EventHub, ProbeEvent};
        use std::time::Duration;

        // Use a nonexistent server — writes fail but that's fine for this test.
        let tf = token_file("mytoken");
        let cfg = hugin_core::config::InfluxConfig {
            url: "http://127.0.0.1:19999".into(),
            org: "o".into(),
            bucket: "b".into(),
            token_file: tf.path().to_string_lossy().into_owned(),
            batch_size: 10,
            batch_timeout_ms: 1000,
        };
        let writer = Arc::new(InfluxWriter::new(&cfg).unwrap());

        // capacity=1: any second publish before recv() is processed causes Lagged.
        let hub = Arc::new(EventHub::new(1));
        let handle = tokio::spawn(run_subscriber(Arc::clone(&writer), Arc::clone(&hub)));

        // Let the subscriber task run until it is parked in rx.recv().await.
        tokio::task::yield_now().await;

        // Flood synchronously (no await) — with capacity=1 the receiver will see Lagged.
        for _ in 0..5 {
            hub.publish(ProbeEvent::ProbeCompleted(fixed_result(true)));
        }

        // Let the subscriber process the Lagged error, then close the hub.
        tokio::task::yield_now().await;
        drop(hub);

        tokio::time::timeout(Duration::from_secs(1), handle)
            .await
            .expect("subscriber did not exit within 1s")
            .expect("subscriber task panicked");
    }

    // --- batch subscriber -----------------------------------------------------

    fn influx_cfg(url: &str, tf: &tempfile::NamedTempFile) -> hugin_core::config::InfluxConfig {
        hugin_core::config::InfluxConfig {
            url: url.to_string(),
            org: "org".into(),
            bucket: "bkt".into(),
            token_file: tf.path().to_string_lossy().into_owned(),
            batch_size: 10,
            batch_timeout_ms: 60_000,
        }
    }

    /// 10 events → exactly 1 POST (batch flushed on count)
    #[tokio::test]
    async fn batch_writer_flushes_on_count() {
        use hugin_core::event::{EventHub, ProbeEvent};
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
            10,    // batch_size
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
        use hugin_core::event::{EventHub, ProbeEvent};
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
