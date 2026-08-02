//! Maps a [`ProbeType`] to the [`Probe`] that handles it, and owns whatever
//! shared state those probes need.
//!
//! ## Why this exists
//!
//! Not to save you from forgetting a match arm — the compiler already does that.
//! `ProbeType` is a closed enum, so the `match` below and the `Display` impl in
//! `huginn-core` are both exhaustive and both fail to compile if you add a
//! variant without handling it. Adding a probe type will always mean touching
//! `ProbeType` (it is the serde target for YAML) plus this match. That floor is
//! two edits, and no amount of indirection removes it.
//!
//! What it does solve is shared state. The scheduler used to build an
//! `Arc<reqwest::Client>` and pass it into *every* probe loop — TCP, DNS, UDP,
//! SMTP and IMAP all carried an HTTP client they never touched, because HTTP
//! needed one. Add a TLS verifier and an ICMP socket and that becomes three
//! unrelated resources threaded through every signature. Here each probe owns
//! what it needs, built once, and the scheduler passes exactly one thing.

use huginn_core::config::ProbeType;

use crate::dns::DnsProbe;
use crate::http::HttpProbe;
use crate::imap::ImapProbe;
use crate::smtp::SmtpProbe;
use crate::tcp::TcpProbe;
use crate::tls::TlsProbe;
use crate::udp::UdpProbe;
use crate::Probe;

/// All probe implementations, each holding its own shared resources.
pub struct ProbeRegistry {
    tcp: TcpProbe,
    http: HttpProbe,
    smtp: SmtpProbe,
    imap: ImapProbe,
    udp: UdpProbe,
    dns: DnsProbe,
    tls: TlsProbe,
}

impl ProbeRegistry {
    /// Build every probe once. The HTTP client's connection pool is shared
    /// across all HTTP/HTTPS probes, which is why this is built here and not
    /// per tick.
    pub fn new() -> Self {
        Self {
            tcp: TcpProbe,
            http: HttpProbe::new(),
            smtp: SmtpProbe,
            imap: ImapProbe,
            udp: UdpProbe,
            dns: DnsProbe,
            tls: TlsProbe::new(),
        }
    }

    /// The probe for `probe_type`.
    ///
    /// Infallible on purpose. A `HashMap<ProbeType, Box<dyn Probe>>` would make
    /// this return `Option`, forcing every tick to handle a "no probe
    /// registered" case that a closed enum makes impossible — trading a
    /// compile-time guarantee for a runtime one, and gaining nothing.
    pub fn get(&self, probe_type: &ProbeType) -> &dyn Probe {
        match probe_type {
            ProbeType::Tcp => &self.tcp,
            ProbeType::Http | ProbeType::Https => &self.http,
            ProbeType::Smtp => &self.smtp,
            ProbeType::Imap => &self.imap,
            ProbeType::Udp => &self.udp,
            ProbeType::Dns => &self.dns,
            ProbeType::Tls => &self.tls,
        }
    }
}

impl Default for ProbeRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use huginn_core::config::ProbeConfig;

    /// Every variant must resolve to a probe that reports the matching type.
    ///
    /// The list is written out rather than iterated so that adding a variant to
    /// `ProbeType` without adding it here is visible in review — the compiler
    /// catches the `get()` match, but not a missing test case.
    #[tokio::test]
    async fn every_probe_type_resolves_and_reports_its_own_type() {
        let registry = ProbeRegistry::new();

        // Targets are unreachable on purpose: this asserts dispatch, not I/O.
        // A failed probe is still a ProbeResult carrying the right probe_type.
        let cases = [
            (ProbeType::Tcp, "127.0.0.1:1", "tcp"),
            (ProbeType::Http, "http://127.0.0.1:1", "http"),
            (ProbeType::Https, "https://127.0.0.1:1", "https"),
            (ProbeType::Smtp, "127.0.0.1:1", "smtp"),
            (ProbeType::Imap, "127.0.0.1:1", "imap"),
            (ProbeType::Udp, "127.0.0.1:1", "udp"),
            (ProbeType::Dns, "127.0.0.1:1", "dns"),
            (ProbeType::Tls, "127.0.0.1:1", "tls"),
        ];

        for (probe_type, target, expected) in cases {
            let cfg = ProbeConfig {
                name: format!("t-{expected}"),
                probe_type,
                target: target.into(),
                timeout_secs: 1,
                ..Default::default()
            };

            let result = registry.get(&cfg.probe_type).probe(&cfg).await;

            assert_eq!(
                result.probe_type, expected,
                "dispatched to the wrong probe for {expected}"
            );
            assert_eq!(result.probe_name, cfg.name);
        }
    }

    /// http and https share one implementation — and so one connection pool.
    #[test]
    fn http_and_https_dispatch_to_the_same_probe() {
        let registry = ProbeRegistry::new();
        let a = registry.get(&ProbeType::Http) as *const dyn Probe;
        let b = registry.get(&ProbeType::Https) as *const dyn Probe;
        assert!(
            std::ptr::addr_eq(a, b),
            "http and https must share a client"
        );
    }
}
