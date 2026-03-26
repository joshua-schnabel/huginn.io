use std::net::{IpAddr, SocketAddr};
use std::time::Instant;

use hickory_resolver::config::{NameServerConfig, Protocol, ResolverConfig, ResolverOpts};
use hickory_resolver::TokioAsyncResolver;
use hugin_core::config::ProbeConfig;
use hugin_core::types::ProbeResult;
use tokio::time::timeout;

pub async fn probe(cfg: &ProbeConfig) -> ProbeResult {
    let query = cfg.dns_query.as_deref().unwrap_or("example.com");
    let nameserver: SocketAddr = match cfg.target.parse() {
        Ok(addr) => addr,
        Err(e) => {
            return ProbeResult::failure(
                &cfg.name,
                "dns",
                &cfg.target,
                0.0,
                format!("invalid target address: {e}"),
            )
        }
    };

    let resolver = build_resolver(nameserver, cfg.timeout_secs);
    let start = Instant::now();
    let result = timeout(cfg.timeout(), resolver.lookup_ip(query)).await;
    let elapsed = start.elapsed().as_secs_f64() * 1000.0;

    match result {
        Err(_) => ProbeResult::failure(
            &cfg.name,
            "dns",
            &cfg.target,
            elapsed,
            format!("timeout after {}s", cfg.timeout_secs),
        ),
        Ok(Err(e)) => ProbeResult::failure(&cfg.name, "dns", &cfg.target, elapsed, e.to_string()),
        Ok(Ok(lookup)) => {
            if let Some(expected) = &cfg.dns_expected_ip {
                let expected_ip: IpAddr = match expected.parse() {
                    Ok(ip) => ip,
                    Err(_) => {
                        return ProbeResult::failure(
                            &cfg.name,
                            "dns",
                            &cfg.target,
                            elapsed,
                            format!("invalid dns_expected_ip: {expected}"),
                        )
                    }
                };
                let resolved: Vec<IpAddr> = lookup.iter().collect();
                if !resolved.contains(&expected_ip) {
                    return ProbeResult::failure(
                        &cfg.name,
                        "dns",
                        &cfg.target,
                        elapsed,
                        format!("expected IP {expected_ip} not in response: {resolved:?}"),
                    );
                }
            }
            ProbeResult::success(&cfg.name, "dns", &cfg.target, elapsed, None)
        }
    }
}

fn build_resolver(nameserver: SocketAddr, timeout_secs: u64) -> TokioAsyncResolver {
    let mut config = ResolverConfig::new();
    config.add_name_server(NameServerConfig {
        socket_addr: nameserver,
        protocol: Protocol::Udp,
        tls_dns_name: None,
        trust_negative_responses: false,
        bind_addr: None,
    });
    let mut opts = ResolverOpts::default();
    opts.timeout = std::time::Duration::from_secs(timeout_secs);
    opts.attempts = 1;
    TokioAsyncResolver::tokio(config, opts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hugin_core::config::ProbeType;
    use tokio::net::UdpSocket;

    fn dns_cfg(addr: &str) -> ProbeConfig {
        ProbeConfig {
            name: "test-dns".into(),
            probe_type: ProbeType::Dns,
            target: addr.to_string(),
            interval_secs: 1,
            timeout_secs: 2,
            expected_status: None,
            dns_query: Some("example.com".into()),
            dns_expected_ip: None,
        }
    }

    /// Build a minimal valid DNS A-record response for the given query.
    /// Copies the transaction ID and question from the query, appends one answer.
    fn build_a_response(query: &[u8], ip: [u8; 4]) -> Vec<u8> {
        let mut r = Vec::new();
        r.extend_from_slice(&query[0..2]);          // transaction ID
        r.extend_from_slice(&[0x81, 0x80]);         // flags: response, RCODE=0
        r.extend_from_slice(&[0x00, 0x01]);         // QDCOUNT=1
        r.extend_from_slice(&[0x00, 0x01]);         // ANCOUNT=1
        r.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]); // NS, AR = 0
        r.extend_from_slice(&query[12..]);          // question section
        // answer
        r.extend_from_slice(&[0xC0, 0x0C]);         // name ptr → offset 12
        r.extend_from_slice(&[0x00, 0x01]);         // type A
        r.extend_from_slice(&[0x00, 0x01]);         // class IN
        r.extend_from_slice(&[0x00, 0x00, 0x00, 0x3C]); // TTL=60
        r.extend_from_slice(&[0x00, 0x04]);         // rdlength=4
        r.extend_from_slice(&ip);                   // rdata
        r
    }

    fn build_nxdomain_response(query: &[u8]) -> Vec<u8> {
        let mut r = Vec::new();
        r.extend_from_slice(&query[0..2]);          // transaction ID
        r.extend_from_slice(&[0x81, 0x83]);         // flags: response, RCODE=3 (NXDOMAIN)
        r.extend_from_slice(&[0x00, 0x01]);         // QDCOUNT=1
        r.extend_from_slice(&[0x00, 0x00]);         // ANCOUNT=0
        r.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]);
        r.extend_from_slice(&query[12..]);          // question section
        r
    }

    #[tokio::test]
    async fn dns_probe_resolves_known_host() {
        let server = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let addr = server.local_addr().unwrap();
        tokio::spawn(async move {
            let mut buf = [0u8; 512];
            let (n, peer) = server.recv_from(&mut buf).await.unwrap();
            let resp = build_a_response(&buf[..n], [1, 2, 3, 4]);
            let _ = server.send_to(&resp, peer).await;
        });

        let result = probe(&dns_cfg(&addr.to_string())).await;
        assert!(result.up, "error: {:?}", result.error);
        assert_eq!(result.probe_type, "dns");
    }

    #[tokio::test]
    async fn dns_probe_fails_on_nxdomain() {
        let server = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let addr = server.local_addr().unwrap();
        tokio::spawn(async move {
            let mut buf = [0u8; 512];
            let (n, peer) = server.recv_from(&mut buf).await.unwrap();
            let resp = build_nxdomain_response(&buf[..n]);
            let _ = server.send_to(&resp, peer).await;
        });

        let cfg = ProbeConfig {
            name: "dns-nxdomain".into(),
            probe_type: ProbeType::Dns,
            target: addr.to_string(),
            interval_secs: 1,
            timeout_secs: 2,
            expected_status: None,
            dns_query: Some("nonexistent.example".into()),
            dns_expected_ip: None,
        };
        let result = probe(&cfg).await;
        assert!(!result.up, "should fail on NXDOMAIN");
    }

    #[tokio::test]
    async fn dns_probe_times_out() {
        let server = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let addr = server.local_addr().unwrap();
        let _keep = server; // bound but never responds

        let cfg = ProbeConfig {
            name: "dns-timeout".into(),
            probe_type: ProbeType::Dns,
            target: addr.to_string(),
            interval_secs: 1,
            timeout_secs: 1,
            expected_status: None,
            dns_query: Some("example.com".into()),
            dns_expected_ip: None,
        };
        let result = probe(&cfg).await;
        assert!(!result.up);
    }

    #[tokio::test]
    async fn dns_probe_fails_on_invalid_target() {
        let cfg = ProbeConfig {
            name: "dns-invalid".into(),
            probe_type: ProbeType::Dns,
            target: "not_a_valid_address_xyz".into(),
            interval_secs: 1,
            timeout_secs: 2,
            expected_status: None,
            dns_query: Some("example.com".into()),
            dns_expected_ip: None,
        };
        let result = probe(&cfg).await;
        assert!(!result.up);
        assert!(result.error.unwrap().contains("invalid target"));
    }

    #[tokio::test]
    async fn dns_probe_validates_expected_ip_match() {
        let server = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let addr = server.local_addr().unwrap();
        tokio::spawn(async move {
            let mut buf = [0u8; 512];
            let (n, peer) = server.recv_from(&mut buf).await.unwrap();
            let resp = build_a_response(&buf[..n], [1, 2, 3, 4]);
            let _ = server.send_to(&resp, peer).await;
        });

        let cfg = ProbeConfig {
            name: "dns-ip-match".into(),
            probe_type: ProbeType::Dns,
            target: addr.to_string(),
            interval_secs: 1,
            timeout_secs: 2,
            expected_status: None,
            dns_query: Some("example.com".into()),
            dns_expected_ip: Some("1.2.3.4".into()),
        };
        let result = probe(&cfg).await;
        assert!(result.up, "error: {:?}", result.error);
    }

    #[tokio::test]
    async fn dns_probe_fails_on_ip_mismatch() {
        let server = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let addr = server.local_addr().unwrap();
        tokio::spawn(async move {
            let mut buf = [0u8; 512];
            let (n, peer) = server.recv_from(&mut buf).await.unwrap();
            let resp = build_a_response(&buf[..n], [1, 2, 3, 4]);
            let _ = server.send_to(&resp, peer).await;
        });

        let cfg = ProbeConfig {
            name: "dns-ip-mismatch".into(),
            probe_type: ProbeType::Dns,
            target: addr.to_string(),
            interval_secs: 1,
            timeout_secs: 2,
            expected_status: None,
            dns_query: Some("example.com".into()),
            dns_expected_ip: Some("9.9.9.9".into()),
        };
        let result = probe(&cfg).await;
        assert!(!result.up);
        assert!(result.error.as_deref().unwrap_or("").contains("9.9.9.9"));
    }
}
