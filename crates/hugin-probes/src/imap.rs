use std::time::Instant;

use hugin_core::config::ProbeConfig;
use hugin_core::types::ProbeResult;
use tokio::io::AsyncReadExt;
use tokio::net::TcpStream;
use tokio::time::timeout;

/// Connect to an IMAP port and verify the server greeting starts with `* OK`.
pub async fn probe(cfg: &ProbeConfig) -> ProbeResult {
    let start = Instant::now();
    let connect = timeout(cfg.timeout(), TcpStream::connect(&cfg.target)).await;
    let elapsed_ms = || start.elapsed().as_secs_f64() * 1000.0;

    let mut stream = match connect {
        Ok(Ok(s)) => s,
        Ok(Err(e)) => {
            return ProbeResult::failure(&cfg.name, "imap", &cfg.target, elapsed_ms(), e.to_string())
        }
        Err(_) => {
            return ProbeResult::failure(
                &cfg.name,
                "imap",
                &cfg.target,
                elapsed_ms(),
                format!("timeout after {}s", cfg.timeout_secs),
            )
        }
    };

    let mut buf = [0u8; 512];
    let read = timeout(cfg.timeout(), stream.read(&mut buf)).await;
    let elapsed = elapsed_ms();

    match read {
        Ok(Ok(n)) if n > 0 => {
            let greeting = String::from_utf8_lossy(&buf[..n]);
            if greeting.starts_with("* OK") {
                ProbeResult::success(&cfg.name, "imap", &cfg.target, elapsed, None)
            } else {
                ProbeResult::failure(
                    &cfg.name,
                    "imap",
                    &cfg.target,
                    elapsed,
                    format!("unexpected greeting: {}", greeting.trim()),
                )
            }
        }
        Ok(Ok(_)) => ProbeResult::failure(
            &cfg.name,
            "imap",
            &cfg.target,
            elapsed,
            "empty greeting".to_string(),
        ),
        Ok(Err(e)) => ProbeResult::failure(&cfg.name, "imap", &cfg.target, elapsed, e.to_string()),
        Err(_) => ProbeResult::failure(
            &cfg.name,
            "imap",
            &cfg.target,
            elapsed,
            "timeout reading greeting".to_string(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hugin_core::config::{ProbeConfig, ProbeType};
    use tokio::io::AsyncWriteExt;
    use tokio::net::TcpListener;

    fn imap_cfg(target: &str) -> ProbeConfig {
        ProbeConfig {
            name: "test-imap".into(),
            probe_type: ProbeType::Imap,
            target: target.into(),
            interval_secs: 10,
            timeout_secs: 2,
            expected_status: None,
            dns_query: None,
            dns_expected_ip: None,
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
        assert!(result.error.as_deref().unwrap_or("").contains("empty greeting"));
    }
}
