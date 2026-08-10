use std::time::Instant;

use huginn_core::config::ProbeConfig;
use huginn_core::types::ProbeResult;

use tokio::net::TcpStream;

use crate::{with_probe_timeout, Probe};
use async_trait::async_trait;

/// IMAP greeting check. Stateless.
pub struct ImapProbe;

#[async_trait]
impl Probe for ImapProbe {
    async fn probe(&self, cfg: &ProbeConfig) -> ProbeResult {
        probe(cfg).await
    }
}

/// Connect to an IMAP port and verify the server greeting starts with `* OK`.
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

    let fail = |msg: String| ProbeResult::failure(&cfg.name, "imap", &cfg.target, elapsed, msg);

    match outcome {
        Ok(greeting) if greeting.starts_with("* OK") => {
            ProbeResult::success(&cfg.name, "imap", &cfg.target, elapsed, None)
        }
        Ok(greeting) if greeting.is_empty() => fail("empty greeting".to_string()),
        Ok(greeting) => fail(format!("unexpected greeting: {}", greeting.trim())),
        Err(msg) => fail(msg),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use huginn_core::config::{ProbeConfig, ProbeType};
    use tokio::io::AsyncWriteExt;
    use tokio::net::TcpListener;

    fn imap_cfg(target: &str) -> ProbeConfig {
        ProbeConfig {
            name: "test-imap".into(),
            probe_type: ProbeType::Imap,
            target: target.into(),
            interval_secs: 10,
            timeout_secs: 2,
            ..Default::default()
        }
    }

    async fn fake_imap_server(greeting: &'static str) -> std::net::SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            if let Ok((mut socket, _)) = listener.accept().await {
                let _ = socket.write_all(greeting.as_bytes()).await;
            }
        });
        addr
    }

    #[tokio::test]
    async fn succeeds_on_ok_greeting() {
        let addr = fake_imap_server("* OK Dovecot ready.\r\n").await;
        let result = probe(&imap_cfg(&addr.to_string())).await;
        assert!(result.up, "error: {:?}", result.error);
    }

    #[tokio::test]
    async fn fails_on_bad_greeting() {
        let addr = fake_imap_server("* BYE Server shutting down\r\n").await;
        let result = probe(&imap_cfg(&addr.to_string())).await;
        assert!(!result.up);
        assert!(result.error.as_deref().unwrap_or("").contains("BYE"));
    }

    #[tokio::test]
    async fn fails_when_port_closed() {
        let result = probe(&imap_cfg("127.0.0.1:1")).await;
        assert!(!result.up);
    }

    #[tokio::test]
    async fn fails_on_empty_greeting() {
        // Server accepts connection then immediately closes without sending anything.
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            if let Ok((socket, _)) = listener.accept().await {
                drop(socket); // EOF immediately
            }
        });

        let result = probe(&imap_cfg(&addr.to_string())).await;
        assert!(!result.up);
        assert!(result
            .error
            .as_deref()
            .unwrap_or("")
            .contains("empty greeting"));
    }
}
