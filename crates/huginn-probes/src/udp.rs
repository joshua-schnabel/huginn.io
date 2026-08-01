use std::time::Instant;

use huginn_core::config::ProbeConfig;
use huginn_core::types::ProbeResult;
use tokio::net::{lookup_host, UdpSocket};

use crate::{with_probe_timeout, Probe};
use async_trait::async_trait;

/// UDP liveness check. Stateless.
pub struct UdpProbe;

#[async_trait]
impl Probe for UdpProbe {
    async fn probe(&self, cfg: &ProbeConfig) -> ProbeResult {
        probe(cfg).await
    }
}

/// Send a minimal DNS query payload to the target and wait for any response.
/// A non-empty response indicates the service is alive.
pub async fn probe(cfg: &ProbeConfig) -> ProbeResult {
    let start = Instant::now();
    let elapsed_ms = || start.elapsed().as_secs_f64() * 1000.0;

    // Resolve the target first, so the local socket is bound in the *same*
    // address family. An IPv4-wildcard socket ("0.0.0.0:0") cannot connect to an
    // IPv6 peer, so a validated IPv6 target would otherwise report DOWN forever.
    let addr = match with_probe_timeout(
        cfg.timeout(),
        &format!("timeout after {}s", cfg.timeout_secs),
        lookup_host(&cfg.target),
    )
    .await
    {
        Ok(mut addrs) => match addrs.next() {
            Some(a) => a,
            None => {
                return ProbeResult::failure(
                    &cfg.name,
                    "udp",
                    &cfg.target,
                    elapsed_ms(),
                    "no address resolved".to_string(),
                )
            }
        },
        Err(msg) => return ProbeResult::failure(&cfg.name, "udp", &cfg.target, elapsed_ms(), msg),
    };

    let bind_addr = if addr.is_ipv6() {
        "[::]:0"
    } else {
        "0.0.0.0:0"
    };
    let socket = match UdpSocket::bind(bind_addr).await {
        Ok(s) => s,
        Err(e) => {
            return ProbeResult::failure(&cfg.name, "udp", &cfg.target, elapsed_ms(), e.to_string())
        }
    };

    if let Err(e) = socket.connect(addr).await {
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
        0x00, // Root label (empty name)
        0x00, 0x01, // QTYPE = A
        0x00, 0x01, // QCLASS = IN
    ];

    if let Err(e) = socket.send(dns_query).await {
        return ProbeResult::failure(&cfg.name, "udp", &cfg.target, elapsed_ms(), e.to_string());
    }

    let mut buf = [0u8; 512];
    let recv = with_probe_timeout(
        cfg.timeout(),
        &format!("timeout after {}s", cfg.timeout_secs),
        socket.recv(&mut buf),
    )
    .await;
    let elapsed = elapsed_ms();

    match recv {
        Ok(n) if n > 0 => ProbeResult::success(&cfg.name, "udp", &cfg.target, elapsed, None),
        Ok(_) => ProbeResult::failure(
            &cfg.name,
            "udp",
            &cfg.target,
            elapsed,
            "empty response".to_string(),
        ),
        Err(msg) => ProbeResult::failure(&cfg.name, "udp", &cfg.target, elapsed, msg),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use huginn_core::config::{ProbeConfig, ProbeType};

    fn udp_cfg(target: &str) -> ProbeConfig {
        ProbeConfig {
            name: "test-udp".into(),
            probe_type: ProbeType::Udp,
            target: target.into(),
            interval_secs: 10,
            timeout_secs: 2,
            ..Default::default()
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

    #[tokio::test]
    async fn fails_on_empty_response() {
        // UDP server that responds with a zero-byte datagram.
        let server = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let addr = server.local_addr().unwrap();
        tokio::spawn(async move {
            let mut buf = [0u8; 512];
            if let Ok((_, peer)) = server.recv_from(&mut buf).await {
                let _ = server.send_to(&[], peer).await; // 0 bytes
            }
        });

        let result = probe(&udp_cfg(&addr.to_string())).await;
        assert!(!result.up);
        assert_eq!(result.error.as_deref().unwrap_or(""), "empty response");
    }

    /// An IPv6 target must work — the probe has to bind an IPv6 local socket to
    /// connect to it. Skipped where IPv6 loopback isn't available.
    #[tokio::test]
    async fn succeeds_on_ipv6_target() {
        let server = match UdpSocket::bind("[::1]:0").await {
            Ok(s) => s,
            Err(_) => return, // no IPv6 loopback in this environment — nothing to test
        };
        let addr = server.local_addr().unwrap();
        tokio::spawn(async move {
            let mut buf = [0u8; 512];
            if let Ok((n, peer)) = server.recv_from(&mut buf).await {
                let _ = server.send_to(&buf[..n], peer).await;
            }
        });

        let result = probe(&udp_cfg(&addr.to_string())).await;
        assert!(result.up, "IPv6 probe failed: {:?}", result.error);
    }
}
