use std::time::Instant;

use huginn_core::config::ProbeConfig;
use huginn_core::types::ProbeResult;
use tokio::io::AsyncReadExt;
use tokio::net::TcpStream;
use tokio::time::timeout;

/// Connect to an SMTP port, read the 220 banner line and measure response time.
pub async fn probe(cfg: &ProbeConfig) -> ProbeResult {
    let start = Instant::now();
    let connect = timeout(cfg.timeout(), TcpStream::connect(&cfg.target)).await;
    let elapsed_ms = || start.elapsed().as_secs_f64() * 1000.0;

    let mut stream = match connect {
        Ok(Ok(s)) => s,
        Ok(Err(e)) => {
            return ProbeResult::failure(
                &cfg.name,
                "smtp",
                &cfg.target,
                elapsed_ms(),
                e.to_string(),
            )
        }
        Err(_) => {
            return ProbeResult::failure(
                &cfg.name,
                "smtp",
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
            let banner = String::from_utf8_lossy(&buf[..n]);
            if banner.starts_with("220") {
                ProbeResult::success(&cfg.name, "smtp", &cfg.target, elapsed, None)
            } else {
                ProbeResult::failure(
                    &cfg.name,
                    "smtp",
                    &cfg.target,
                    elapsed,
                    format!("unexpected banner: {}", banner.trim()),
                )
            }
        }
        Ok(Ok(_)) => ProbeResult::failure(
            &cfg.name,
            "smtp",
            &cfg.target,
            elapsed,
            "empty banner".to_string(),
        ),
        Ok(Err(e)) => ProbeResult::failure(&cfg.name, "smtp", &cfg.target, elapsed, e.to_string()),
        Err(_) => ProbeResult::failure(
            &cfg.name,
            "smtp",
            &cfg.target,
            elapsed,
            "timeout reading banner".to_string(),
        ),
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
            expected_status: None,
            dns_query: None,
            dns_expected_ip: None,
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
