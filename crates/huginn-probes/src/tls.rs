use std::time::Instant;

use chrono::{DateTime, Utc};
use huginn_core::config::ProbeConfig;
use huginn_core::types::metric_keys::TLS_CERT_EXPIRY_DAYS;
use huginn_core::types::ProbeResult;
use std::sync::Arc;

use crate::{with_probe_timeout, Probe};
use async_trait::async_trait;

/// TLS certificate check: completes a TLS handshake with a TLS-over-HTTP (HTTPS)
/// endpoint and reports how many days remain until the server certificate
/// expires — `tls_cert_expiry_days`, negative once expired.
///
/// Certificate verification is disabled **on purpose**: the goal is to *read*
/// the certificate (self-signed and already-expired ones included), not to trust
/// it. `up` reflects whether the TLS handshake completed **and** the certificate
/// has at least `tls_expiry_fail_days` days left (default 0 — an expired
/// certificate is DOWN).
///
/// The target is `host:port` (e.g. `example.com:443`, `imap.example.com:993`).
/// The certificate is taken from the TLS handshake itself, so **any** TLS port
/// works — IMAPS, SMTPS and LDAPS included. It used to go through an HTTP
/// client, which meant the endpoint had to speak HTTP over TLS.
pub struct TlsProbe {
    /// `None` when the rustls configuration could not be built. Kept as an
    /// error rather than a panic: `ProbeRegistry::new()` runs at startup, and a
    /// probe that cannot be constructed must still report DOWN with a reason
    /// like any other failure, not take the process down (see the `Probe`
    /// trait's contract).
    config: Result<Arc<rustls::ClientConfig>, String>,
}

impl TlsProbe {
    pub fn new() -> Self {
        Self {
            config: build_tls_config().map_err(|e| format!("could not build a TLS client: {e}")),
        }
    }
}

impl Default for TlsProbe {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Probe for TlsProbe {
    async fn probe(&self, cfg: &ProbeConfig) -> ProbeResult {
        match &self.config {
            Ok(config) => probe(cfg, Arc::clone(config)).await,
            Err(msg) => ProbeResult::failure(&cfg.name, "tls", &cfg.target, 0.0, msg.clone()),
        }
    }
}

/// Handshake with `target` (`host:port`), read the peer certificate, and report
/// days to expiry.
///
/// The handshake is the whole probe — no HTTP request is made. That is what
/// makes any TLS port probeable rather than only those that speak HTTP over it.
pub async fn probe(cfg: &ProbeConfig, config: Arc<rustls::ClientConfig>) -> ProbeResult {
    let start = Instant::now();

    let result = with_probe_timeout(
        cfg.timeout(),
        &format!("timeout after {}s", cfg.timeout_secs),
        fetch_peer_certificate(&cfg.target, config),
    )
    .await;
    let elapsed = start.elapsed().as_secs_f64() * 1000.0;

    let der = match result {
        Ok(der) => der,
        Err(msg) => return ProbeResult::failure(&cfg.name, "tls", &cfg.target, elapsed, msg),
    };

    match cert_expiry_days(&der, Utc::now()) {
        Ok(days) => {
            let threshold = cfg.tls_expiry_fail_days.unwrap_or(0.0);
            match expiry_verdict(days, threshold) {
                None => ProbeResult::success(&cfg.name, "tls", &cfg.target, elapsed, None)
                    .with_metric(TLS_CERT_EXPIRY_DAYS, days),
                // The metric stays attached on failure so dashboards and alerts
                // keep seeing how far past (or close to) expiry the cert is.
                Some(why) => ProbeResult::failure(&cfg.name, "tls", &cfg.target, elapsed, why)
                    .with_metric(TLS_CERT_EXPIRY_DAYS, days),
            }
        }
        Err(e) => ProbeResult::failure(&cfg.name, "tls", &cfg.target, elapsed, e.to_string()),
    }
}

/// `None` when the certificate has at least `threshold` days left, otherwise the
/// DOWN reason.
fn expiry_verdict(days: f64, threshold: f64) -> Option<String> {
    if days < 0.0 {
        Some(format!("certificate expired {:.1} days ago", -days))
    } else if days < threshold {
        Some(format!(
            "certificate expires in {days:.1} days (tls_expiry_fail_days: {threshold})"
        ))
    } else {
        None
    }
}

/// Days until the certificate's `notAfter`, negative once expired.
fn cert_expiry_days(der: &[u8], now: DateTime<Utc>) -> anyhow::Result<f64> {
    let (_, cert) = x509_parser::parse_x509_certificate(der)
        .map_err(|e| anyhow::anyhow!("could not parse certificate: {e:?}"))?;
    let not_after = cert.validity().not_after.timestamp();
    Ok((not_after - now.timestamp()) as f64 / 86_400.0)
}

/// Split `host:port`, keeping IPv6 literals in brackets intact.
///
/// `rsplit_once` rather than `split_once`: `[2001:db8::1]:993` contains colons
/// in the host, and only the last one separates the port.
fn split_host_port(target: &str) -> Option<(&str, u16)> {
    let (host, port) = target.rsplit_once(':')?;
    let host = host.strip_prefix('[').unwrap_or(host);
    let host = host.strip_suffix(']').unwrap_or(host);
    if host.is_empty() {
        return None;
    }
    Some((host, port.parse().ok()?))
}

/// A certificate verifier that verifies nothing.
///
/// This is [ADR-0006](../../../docs/adr/0006-tls-probe-skips-verification.md)
/// made explicit rather than borrowed from an HTTP client's
/// `danger_accept_invalid_certs`. A certificate-expiry probe exists to *read*
/// certificates — including the expired and self-signed ones, which are exactly
/// the interesting cases — so refusing to complete a handshake with them would
/// defeat the probe entirely. The connection carries no credentials, sends no
/// application data and trusts nothing it receives: the DER bytes are parsed for
/// `notAfter` and the socket is closed.
///
/// Confined to this one connector. Every other TLS client in the workspace
/// verifies normally, and `deny.toml` keeps the stack rustls-only.
#[derive(Debug)]
struct ReadOnlyCertVerifier {
    /// The provider's schemes, so TLS 1.2 and 1.3 signature checks report what
    /// the handshake can actually negotiate rather than an invented list.
    supported: rustls::crypto::WebPkiSupportedAlgorithms,
}

impl rustls::client::danger::ServerCertVerifier for ReadOnlyCertVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        // nosemgrep: rust.lang.security.rustls-dangerous.rustls-dangerous
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.supported.supported_schemes()
    }
}

/// The rustls configuration used for every TLS probe.
///
/// Built once and shared: it holds no per-target state, and constructing it per
/// tick would redo the provider setup on every probe interval.
fn build_tls_config() -> Result<Arc<rustls::ClientConfig>, rustls::Error> {
    let provider = rustls::crypto::ring::default_provider();
    let supported = provider.signature_verification_algorithms;

    let config = rustls::ClientConfig::builder_with_provider(Arc::new(provider))
        .with_safe_default_protocol_versions()?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(ReadOnlyCertVerifier { supported }))
        .with_no_client_auth();
    Ok(Arc::new(config))
}

/// Complete a TLS handshake with `target` and return the peer's leaf
/// certificate in DER form.
///
/// No application data is written and nothing is read back: the certificate is
/// available the moment the handshake completes, which is what makes this work
/// against IMAPS, SMTPS and LDAPS as well as HTTPS. A protocol that expects the
/// *server* to speak first still presents its certificate during the handshake,
/// before either side says anything.
async fn fetch_peer_certificate(
    target: &str,
    config: Arc<rustls::ClientConfig>,
) -> anyhow::Result<Vec<u8>> {
    let (host, _port) = split_host_port(target)
        .ok_or_else(|| anyhow::anyhow!("target '{target}' must be host:port"))?;

    // SNI. An IP literal is a valid ServerName in rustls, and a server that
    // requires SNI will simply present its default certificate — which is the
    // one an operator probing by IP is asking about.
    let server_name = rustls::pki_types::ServerName::try_from(host.to_string())
        .map_err(|_| anyhow::anyhow!("'{host}' is not a valid TLS server name"))?;

    let tcp = tokio::net::TcpStream::connect(target).await?;
    let stream = tokio_rustls::TlsConnector::from(config)
        .connect(server_name, tcp)
        .await?;

    let (_io, session) = stream.get_ref();
    let leaf = session
        .peer_certificates()
        .and_then(|chain| chain.first())
        .ok_or_else(|| {
            anyhow::anyhow!("TLS handshake completed but the peer presented no certificate")
        })?;
    Ok(leaf.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use huginn_core::config::{ProbeConfig, ProbeType};

    // A self-signed certificate with notAfter = 2126-07-09 (generated once with
    // openssl, 100-year validity), so the expiry maths is deterministic against a
    // fixed `now` no matter when the test runs.
    const CERT_DER: &[u8] = include_bytes!("testdata/expiry_test_cert.der");

    #[test]
    fn expiry_days_is_positive_long_before_notafter() {
        let now = Utc.with_ymd_and_hms(2026, 8, 2, 0, 0, 0).unwrap();
        let days = cert_expiry_days(CERT_DER, now).unwrap();
        // ~100 years out (2126 − 2026).
        assert!(
            (36_000.0..37_000.0).contains(&days),
            "expected ~36500 days, got {days}"
        );
    }

    #[test]
    fn expiry_days_is_negative_once_past_notafter() {
        // A `now` after the cert's 2126 notAfter → expired → negative days.
        let now = Utc.with_ymd_and_hms(2200, 1, 1, 0, 0, 0).unwrap();
        let days = cert_expiry_days(CERT_DER, now).unwrap();
        assert!(
            days < 0.0,
            "an expired cert must give negative days, got {days}"
        );
    }

    #[test]
    fn verdict_is_up_with_days_to_spare() {
        assert_eq!(expiry_verdict(90.0, 30.0), None);
    }

    #[test]
    fn verdict_is_up_exactly_at_the_threshold() {
        assert_eq!(expiry_verdict(30.0, 30.0), None);
    }

    #[test]
    fn verdict_is_down_below_the_threshold() {
        let why = expiry_verdict(12.3, 30.0).expect("below threshold must be DOWN");
        assert!(
            why.contains("12.3"),
            "reason must name the days left: {why}"
        );
        assert!(why.contains("30"), "reason must name the threshold: {why}");
    }

    #[test]
    fn verdict_is_down_once_expired_even_with_default_threshold() {
        let why = expiry_verdict(-2.5, 0.0).expect("expired must be DOWN");
        assert!(why.contains("expired"), "reason must say expired: {why}");
        assert!(
            why.contains("2.5"),
            "reason must name days past expiry: {why}"
        );
    }

    #[test]
    fn garbage_is_not_a_certificate() {
        assert!(cert_expiry_days(b"not a certificate at all", Utc::now()).is_err());
    }

    fn tls_cfg(target: &str) -> ProbeConfig {
        ProbeConfig {
            name: "test-tls".into(),
            probe_type: ProbeType::Tls,
            target: target.into(),
            interval_secs: 10,
            timeout_secs: 2,
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn down_when_port_closed() {
        let config = build_tls_config().expect("rustls config");
        let result = probe(&tls_cfg("127.0.0.1:1"), config).await;
        assert!(!result.up, "a refused connection must be DOWN");
        assert!(result.error.is_some());
    }

    /// A plaintext peer that never speaks TLS must fail as a probe result, not
    /// hang: the handshake waits for a ServerHello that is never coming, so this
    /// is the probe's own deadline doing the work.
    #[tokio::test]
    async fn down_when_the_peer_does_not_speak_tls() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let mut held = Vec::new();
            while let Ok((socket, _)) = listener.accept().await {
                held.push(socket);
            }
        });

        let mut cfg = tls_cfg(&addr.to_string());
        cfg.timeout_secs = 1;
        let config = build_tls_config().expect("rustls config");

        let started = std::time::Instant::now();
        let result = probe(&cfg, config).await;
        assert!(!result.up);
        assert!(
            started.elapsed() < std::time::Duration::from_millis(2_500),
            "the probe's deadline did not bound a stalled handshake"
        );
    }

    // -----------------------------------------------------------------------
    // Target splitting — the part that decides which port gets probed
    // -----------------------------------------------------------------------

    #[test]
    fn splits_an_ordinary_host_and_port() {
        assert_eq!(
            split_host_port("example.com:443"),
            Some(("example.com", 443))
        );
        assert_eq!(
            split_host_port("imap.example.com:993"),
            Some(("imap.example.com", 993))
        );
    }

    /// An IPv6 literal is full of colons; only the last one is the separator,
    /// and the brackets are not part of the name.
    #[test]
    fn splits_a_bracketed_ipv6_literal() {
        assert_eq!(
            split_host_port("[2001:db8::1]:993"),
            Some(("2001:db8::1", 993))
        );
    }

    #[test]
    fn rejects_a_target_without_a_usable_port() {
        assert_eq!(split_host_port("example.com"), None);
        assert_eq!(split_host_port("example.com:https"), None);
        assert_eq!(split_host_port(":443"), None);
    }

    /// The configuration has to be buildable, or every TLS probe reports DOWN
    /// with a construction error instead of measuring anything. Cheap to assert
    /// and it pins the provider wiring, which is the part most likely to break
    /// on a rustls upgrade.
    #[test]
    fn the_tls_configuration_builds() {
        assert!(build_tls_config().is_ok());
    }

    // -----------------------------------------------------------------------
    // The point of the change: a TLS port that does not speak HTTP
    // -----------------------------------------------------------------------

    // A self-signed certificate and its key, valid until 2126. Self-signed on
    // purpose — it also demonstrates that the probe reads certificates no
    // ordinary client would accept, which is ADR-0006's whole reason to exist.
    //
    // Stored as DER (and PKCS#8 for the key) rather than PEM so no PEM parser is
    // needed: `rustls-pemfile` is not in the tree, and pulling in a crate to
    // decode two test fixtures would be a real supply-chain addition for no
    // gain. rustls takes DER directly.
    const RAW_TLS_CERT_DER: &[u8] = include_bytes!("testdata/raw_tls_cert.der");
    const RAW_TLS_KEY_PK8: &[u8] = include_bytes!("testdata/raw_tls_key.pk8");

    /// Start a TLS server that completes handshakes and then says **nothing**,
    /// the way IMAPS and SMTPS do before their greeting — and unlike HTTPS,
    /// which the old reqwest-based probe required.
    async fn silent_tls_server() -> std::net::SocketAddr {
        let certs = vec![rustls::pki_types::CertificateDer::from(RAW_TLS_CERT_DER)];
        let key = rustls::pki_types::PrivateKeyDer::Pkcs8(
            rustls::pki_types::PrivatePkcs8KeyDer::from(RAW_TLS_KEY_PK8),
        );

        let config = rustls::ServerConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .expect("protocol versions")
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .expect("server config");

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
        tokio::spawn(async move {
            let mut held = Vec::new();
            while let Ok((socket, _)) = listener.accept().await {
                if let Ok(stream) = acceptor.accept(socket).await {
                    // Handshake done, no application data. Hold it open so the
                    // probe's own close is what ends the connection.
                    held.push(stream);
                }
            }
        });
        addr
    }

    /// The probe reads a certificate from a port that speaks TLS and
    /// nothing else — no HTTP request is made, and none would be answered.
    ///
    /// Through the previous reqwest-based implementation this target was
    /// unprobeable: the certificate was only reachable via an HTTP response, so
    /// IMAPS, SMTPS and LDAPS were out of scope even though their certificates
    /// expire like any other.
    #[tokio::test]
    async fn reads_a_certificate_from_a_port_that_does_not_speak_http() {
        let addr = silent_tls_server().await;
        let config = build_tls_config().expect("rustls config");

        let result = probe(&tls_cfg(&addr.to_string()), config).await;

        assert!(
            result.up,
            "a raw TLS port must be probeable, got: {:?}",
            result.error
        );
        let days = result
            .metrics
            .get(TLS_CERT_EXPIRY_DAYS)
            .copied()
            .expect("the expiry metric must be attached");
        assert!(
            days > 30_000.0,
            "the test certificate runs to 2126, so days-to-expiry should be large; got {days}"
        );
    }

    /// The expiry threshold still decides UP/DOWN over a raw TLS connection —
    /// the verdict logic is shared, and this proves the new transport feeds it.
    #[tokio::test]
    async fn the_expiry_threshold_still_applies_over_raw_tls() {
        let addr = silent_tls_server().await;
        let config = build_tls_config().expect("rustls config");

        let mut cfg = tls_cfg(&addr.to_string());
        // Far beyond the certificate's remaining life, so it must fail.
        cfg.tls_expiry_fail_days = Some(50_000.0);

        let result = probe(&cfg, config).await;
        assert!(
            !result.up,
            "a threshold above the remaining life must be DOWN"
        );
        assert!(
            result.metrics.contains_key(TLS_CERT_EXPIRY_DAYS),
            "the metric must survive a DOWN verdict, so alerts can see the number"
        );
    }
}
