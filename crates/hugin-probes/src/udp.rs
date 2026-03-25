use std::time::Instant;

use hugin_core::config::ProbeConfig;
use hugin_core::types::ProbeResult;
use tokio::net::UdpSocket;
use tokio::time::timeout;

/// Send a minimal DNS query payload to the target and wait for any response.
/// A non-empty response indicates the service is alive.
pub async fn probe(cfg: &ProbeConfig) -> ProbeResult {
    let start = Instant::now();
    let elapsed_ms = || start.elapsed().as_secs_f64() * 1000.0;

    let socket = match UdpSocket::bind("0.0.0.0:0").await {
        Ok(s) => s,
        Err(e) => return ProbeResult::failure(&cfg.name, "udp", &cfg.target, 0.0, e.to_string()),
    };

    if let Err(e) = socket.connect(&cfg.target).await {
        return ProbeResult::failure(&cfg.name, "udp", &cfg.target, elapsed_ms(), e.to_string());
    }

    // Minimal DNS query for "." (root), type A — safe, tiny, always valid
    let dns_query: &[u8] = &[
        0xAA, 0xBB, // Transaction ID
        0x01, 0x00, // Flags: standard query
        0x00, 0x01, // QDCOUNT = 1
        0x00, 0x00, // ANCOUNT = 0
        0x00, 0x00, // NSCOUNT = 0
        0x00, 0x00, // ARCOUNT = 0
        0x00,       // Root label (empty name)
        0x00, 0x01, // QTYPE = A
        0x00, 0x01, // QCLASS = IN
    ];

    if let Err(e) = socket.send(dns_query).await {
        return ProbeResult::failure(&cfg.name, "udp", &cfg.target, elapsed_ms(), e.to_string());
    }

    let mut buf = [0u8; 512];
    let recv = timeout(cfg.timeout(), socket.recv(&mut buf)).await;
    let elapsed = elapsed_ms();

    match recv {
        Ok(Ok(n)) if n > 0 => ProbeResult::success(&cfg.name, "udp", &cfg.target, elapsed, None),
        Ok(Ok(_)) => {
            ProbeResult::failure(&cfg.name, "udp", &cfg.target, elapsed, "empty response".to_string())
        }
        Ok(Err(e)) => ProbeResult::failure(&cfg.name, "udp", &cfg.target, elapsed, e.to_string()),
        Err(_) => ProbeResult::failure(
            &cfg.name,
            "udp",
            &cfg.target,
            elapsed,
            format!("timeout after {}s", cfg.timeout_secs),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hugin_core::config::{ProbeConfig, ProbeType};

    fn udp_cfg(target: &str) -> ProbeConfig {
        ProbeConfig {
            name: "test-udp".into(),
            probe_type: ProbeType::Udp,
            target: target.into(),
            interval_secs: 10,
            timeout_secs: 2,
            expected_status: None,
        }
    }

    #[tokio::test]
    async fn succeeds_when_server_responds() {
        // Bind a fake UDP server that echoes back one byte
        let server = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let addr = server.local_addr().unwrap();

        tokio::spawn(async move {
            let mut buf = [0u8; 512];
            if let Ok((n, peer)) = server.recv_from(&mut buf).await {
                let _ = server.send_to(&buf[..n], peer).await;
            }
        });

        let result = probe(&udp_cfg(&addr.to_string())).await;
        assert!(result.up, "error: {:?}", result.error);
    }

    #[tokio::test]
    async fn times_out_when_no_response() {
        // Bind a socket but never respond
        let server = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let addr = server.local_addr().unwrap();
        // keep server alive but don't respond
        let _server = server;

        let cfg = ProbeConfig {
            timeout_secs: 1,
            ..udp_cfg(&addr.to_string())
        };
        let result = probe(&cfg).await;
        assert!(!result.up);
        assert!(result.error.as_deref().unwrap_or("").contains("timeout"));
    }
}
