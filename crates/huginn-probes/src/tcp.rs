use std::time::Instant;

use huginn_core::config::ProbeConfig;
use huginn_core::types::ProbeResult;
use tokio::net::TcpStream;

use crate::{with_probe_timeout, Probe};
use async_trait::async_trait;

/// TCP connect check. Stateless.
pub struct TcpProbe;

#[async_trait]
impl Probe for TcpProbe {
    async fn probe(&self, cfg: &ProbeConfig) -> ProbeResult {
        probe(cfg).await
    }
}

/// Connect to a TCP host:port and measure the handshake time.
pub async fn probe(cfg: &ProbeConfig) -> ProbeResult {
    let start = Instant::now();
    let result = with_probe_timeout(
        cfg.timeout(),
        &format!("timeout after {}s", cfg.timeout_secs),
        TcpStream::connect(&cfg.target),
    )
    .await;
    let elapsed = start.elapsed().as_secs_f64() * 1000.0;

    match result {
        Ok(_) => ProbeResult::success(&cfg.name, "tcp", &cfg.target, elapsed, None),
        Err(msg) => ProbeResult::failure(&cfg.name, "tcp", &cfg.target, elapsed, msg),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use huginn_core::config::{ProbeConfig, ProbeType};
    use tokio::net::TcpListener;

    fn tcp_cfg(target: &str) -> ProbeConfig {
        ProbeConfig {
            name: "test-tcp".into(),
            probe_type: ProbeType::Tcp,
            target: target.into(),
            interval_secs: 10,
            timeout_secs: 2,
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn succeeds_when_port_open() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { listener.accept().await });

        let result = probe(&tcp_cfg(&addr.to_string())).await;
        assert!(result.up, "expected up=true, got: {:?}", result.error);
        assert!(result.response_ms >= 0.0);
    }

    #[tokio::test]
    async fn fails_when_port_closed() {
        // Port 1 is nearly always closed/refused
        let result = probe(&tcp_cfg("127.0.0.1:1")).await;
        assert!(!result.up);
        assert!(result.error.is_some());
    }

    #[tokio::test]
    async fn times_out_on_unreachable_host() {
        let cfg = ProbeConfig {
            timeout_secs: 1,
            ..tcp_cfg("192.0.2.1:9999") // TEST-NET-1, unroutable
        };
        let result = probe(&cfg).await;
        assert!(!result.up);
        assert!(result.error.as_deref().unwrap_or("").contains("timeout"));
    }
}
