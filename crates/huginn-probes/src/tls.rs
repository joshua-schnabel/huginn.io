use std::time::Instant;

use chrono::{DateTime, Utc};
use huginn_core::config::ProbeConfig;
use huginn_core::types::metric_keys::TLS_CERT_EXPIRY_DAYS;
use huginn_core::types::ProbeResult;
use reqwest::Client;

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
/// The target is `host:port` (e.g. `example.com:443`). The certificate is read
/// from the HTTP response's TLS info, so the endpoint must speak HTTPS; raw
/// non-HTTP TLS ports (IMAPS, SMTPS, …) are out of scope for now.
pub struct TlsProbe {
    client: Client,
}

impl TlsProbe {
    pub fn new() -> Self {
        Self {
            client: build_tls_client(),
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
        probe(cfg, &self.client).await
    }
}

/// Connect to `https://<target>`, read the peer certificate, and report days to
/// expiry.
pub async fn probe(cfg: &ProbeConfig, client: &Client) -> ProbeResult {
    let start = Instant::now();
    let url = format!("https://{}", cfg.target);

    let result = with_probe_timeout(
        cfg.timeout(),
        &format!("timeout after {}s", cfg.timeout_secs),
        client.get(&url).send(),
    )
    .await;
    let elapsed = start.elapsed().as_secs_f64() * 1000.0;

    let resp = match result {
        Ok(r) => r,
        Err(msg) => return ProbeResult::failure(&cfg.name, "tls", &cfg.target, elapsed, msg),
    };

    // reqwest attaches the peer certificate (DER) as a TlsInfo response extension
    // when the client is built with `.tls_info(true)`.
    let der = match resp
        .extensions()
        .get::<reqwest::tls::TlsInfo>()
        .and_then(|info| info.peer_certificate())
    {
        Some(der) => der.to_vec(),
        None => {
            return ProbeResult::failure(
                &cfg.name,
                "tls",
                &cfg.target,
                elapsed,
                "TLS handshake completed but no peer certificate was exposed".to_string(),
            )
        }
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

/// reqwest client for TLS probing: exposes the peer cert (`tls_info`) and accepts
/// invalid certs (we read them, not trust them); doesn't follow redirects.
fn build_tls_client() -> Client {
    // A certificate-expiry probe must read the certificate even when it is
    // expired or self-signed (`.danger_accept_invalid_certs(true)` below) — that
    // is the whole point; the metric is negative once expired. The connection
    // carries no secrets and trusts nothing about the peer, so accepting an
    // invalid certificate is safe in this narrow read-only context. Verification
    // stays on everywhere else. The finding anchors on this builder line, so the
    // suppression sits here.
    Client::builder() // nosemgrep: rust.lang.security.reqwest-accept-invalid.reqwest-accept-invalid
        .use_rustls_tls()
        .tls_info(true)
        .danger_accept_invalid_certs(true)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("Failed to build TLS probe client")
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
        let client = build_tls_client();
        let result = probe(&tls_cfg("127.0.0.1:1"), &client).await;
        assert!(!result.up, "a refused connection must be DOWN");
        assert!(result.error.is_some());
    }
}
