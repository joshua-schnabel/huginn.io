use hugin_core::config::InfluxConfig;
use hugin_core::error::{HuginError, Result};
use hugin_core::types::ProbeResult;
use tracing::{debug, warn};

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
        debug!(line = %line, "writing to InfluxDB");

        let resp = self
            .client
            .post(&self.write_url)
            .header("Authorization", format!("Token {}", self.token))
            .header("Content-Type", "text/plain; charset=utf-8")
            .body(line)
            .send()
            .await
            .map_err(|e| HuginError::Influx(e.to_string()))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            warn!(status = %status, body = %body, "InfluxDB write failed");
            return Err(HuginError::Influx(format!(
                "HTTP {status}: {body}"
            )));
        }
        Ok(())
    }
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

        // Write token to a temp file
        let token_file = tempfile_with("mytoken");
        let cfg = hugin_core::config::InfluxConfig {
            url: server.uri(),
            org: "testorg".into(),
            bucket: "testbucket".into(),
            token_file: token_file.clone(),
        };

        let writer = InfluxWriter::new(&cfg).unwrap();
        let result = fixed_result(true);
        writer.write(&result).await.unwrap();

        // Cleanup
        let _ = std::fs::remove_file(token_file);
    }

    #[tokio::test]
    async fn writer_returns_error_on_non_2xx() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(401).set_body_string("unauthorized"))
            .mount(&server)
            .await;

        let token_file = tempfile_with("bad-token");
        let cfg = hugin_core::config::InfluxConfig {
            url: server.uri(),
            org: "o".into(),
            bucket: "b".into(),
            token_file: token_file.clone(),
        };

        let writer = InfluxWriter::new(&cfg).unwrap();
        let result = writer.write(&fixed_result(true)).await;
        assert!(result.is_err());

        let _ = std::fs::remove_file(token_file);
    }

    fn tempfile_with(content: &str) -> String {
        use std::io::Write;
        let path = std::env::temp_dir().join(format!(
            "hugin-test-{}-{}.token",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .subsec_nanos()
        ));
        let mut f = std::fs::File::create(&path).unwrap();
        write!(f, "{content}").unwrap();
        path.to_string_lossy().into_owned()
    }
}
