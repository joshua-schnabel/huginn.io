use std::time::Instant;

use huginn_core::config::ProbeConfig;
use huginn_core::types::ProbeResult;

use tokio::net::TcpStream;

use crate::{with_probe_timeout, Probe};
use async_trait::async_trait;

/// SMTP banner check. Stateless.
pub struct SmtpProbe;

#[async_trait]
impl Probe for SmtpProbe {
    async fn probe(&self, cfg: &ProbeConfig) -> ProbeResult {
        probe(cfg).await
    }
}

/// Connect to an SMTP port, read the 220 banner line and measure response time.
///
/// Connect and banner share **one** deadline, and the banner is read until the
/// line is complete — see [`with_probe_timeout`] and
/// [`read_greeting_line`](crate::read_greeting_line) for why each of those is
/// the way it is.
pub async fn probe(cfg: &ProbeConfig) -> ProbeResult {
    let start = Instant::now();
    let elapsed_ms = || start.elapsed().as_secs_f64() * 1000.0;

    let outcome = with_probe_timeout(
        cfg.timeout(),
        &format!("timeout after {}s", cfg.timeout_secs),
        async {
            let mut stream = TcpStream::connect(&cfg.target).await?;
            crate::read_greeting_line(&mut stream).await
        },
    )
    .await;
    let elapsed = elapsed_ms();

    let fail = |msg: String| ProbeResult::failure(&cfg.name, "smtp", &cfg.target, elapsed, msg);

    match outcome {
        Ok(banner) if banner.starts_with("220") => {
            ProbeResult::success(&cfg.name, "smtp", &cfg.target, elapsed, None)
        }
        Ok(banner) if banner.is_empty() => fail("empty banner".to_string()),
        Ok(banner) => fail(format!("unexpected banner: {}", banner.trim())),
        Err(msg) => fail(msg),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use huginn_core::config::{ProbeConfig, ProbeType};
    use tokio::io::AsyncWriteExt;
    use tokio::net::TcpListener;

    fn smtp_cfg(target: &str) -> ProbeConfig {
        ProbeConfig {
            name: "test-smtp".into(),
            probe_type: ProbeType::Smtp,
            target: target.into(),
            interval_secs: 10,
            timeout_secs: 2,
            ..Default::default()
        }
    }

    async fn fake_smtp_server(banner: &'static str) -> std::net::SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            if let Ok((mut socket, _)) = listener.accept().await {
                let _ = socket.write_all(banner.as_bytes()).await;
            }
        });
        addr
    }

    #[tokio::test]
    async fn succeeds_on_220_banner() {
        let addr = fake_smtp_server("220 mail.example.com ESMTP\r\n").await;
        let result = probe(&smtp_cfg(&addr.to_string())).await;
        assert!(result.up, "error: {:?}", result.error);
    }

    #[tokio::test]
    async fn fails_on_non_220_banner() {
        let addr = fake_smtp_server("554 Service unavailable\r\n").await;
        let result = probe(&smtp_cfg(&addr.to_string())).await;
        assert!(!result.up);
        assert!(result.error.as_deref().unwrap_or("").contains("554"));
    }

    #[tokio::test]
    async fn fails_when_port_closed() {
        let result = probe(&smtp_cfg("127.0.0.1:1")).await;
        assert!(!result.up);
    }

    /// A banner split across TCP segments must still be recognised.
    ///
    /// This is the regression, end to end. TCP is a byte stream and may split
    /// anywhere; the probe used to take one `read()` and test its prefix, so a
    /// server whose banner arrived as `22` + the rest was reported DOWN while
    /// being entirely healthy. Timing-dependent, so in production it looked like
    /// a monitor inventing occasional outages.
    #[tokio::test]
    async fn succeeds_on_a_banner_split_across_segments() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            if let Ok((mut socket, _)) = listener.accept().await {
                // Deliberately split inside the status code, and flush between
                // the halves so they cannot coalesce into one segment.
                let _ = socket.write_all(b"22").await;
                let _ = socket.flush().await;
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                let _ = socket.write_all(b"0 mail.example.com ESMTP\r\n").await;
            }
        });

        let result = probe(&smtp_cfg(&addr.to_string())).await;
        assert!(
            result.up,
            "a split banner must be reassembled, got error: {:?}",
            result.error
        );
    }

    /// Connect and banner share one deadline, so the probe's worst case is
    /// `timeout_secs` — not twice it, which is what two separately-timed steps
    /// produced.
    #[tokio::test]
    async fn one_deadline_covers_connect_and_banner() {
        // Accepts, then never speaks: the connect succeeds instantly and the
        // read is what runs out the clock.
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let mut held = Vec::new();
            while let Ok((socket, _)) = listener.accept().await {
                held.push(socket);
            }
        });

        let mut cfg = smtp_cfg(&addr.to_string());
        cfg.timeout_secs = 1;

        let started = std::time::Instant::now();
        let result = probe(&cfg).await;
        let elapsed = started.elapsed();

        assert!(!result.up);
        assert!(
            elapsed < std::time::Duration::from_millis(1_800),
            "the probe took {elapsed:?} for a 1s timeout — connect and read are not sharing a deadline"
        );
    }

    #[tokio::test]
    async fn fails_on_empty_banner() {
        // Server accepts connection then immediately closes without sending anything.
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            if let Ok((socket, _)) = listener.accept().await {
                drop(socket); // EOF immediately
            }
        });

        let result = probe(&smtp_cfg(&addr.to_string())).await;
        assert!(!result.up);
        assert!(result
            .error
            .as_deref()
            .unwrap_or("")
            .contains("empty banner"));
    }
}
