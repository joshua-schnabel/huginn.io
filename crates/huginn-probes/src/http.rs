use std::time::Instant;

use huginn_core::config::ProbeConfig;
use huginn_core::types::ProbeResult;
use reqwest::Client;

use crate::{with_probe_timeout, Probe};
use async_trait::async_trait;

/// HTTP/HTTPS status check.
///
/// Owns the `reqwest::Client` — the one piece of genuinely shared probe state.
/// It carries the connection pool, so it is built once here rather than per
/// tick, and it no longer has to be threaded through probe loops that don't
/// speak HTTP.
pub struct HttpProbe {
    client: Client,
}

impl HttpProbe {
    pub fn new() -> Self {
        Self {
            client: build_client(),
        }
    }
}

impl Default for HttpProbe {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Probe for HttpProbe {
    async fn probe(&self, cfg: &ProbeConfig) -> ProbeResult {
        probe(cfg, &self.client).await
    }
}

/// Perform an HTTP/HTTPS GET request and measure response time.
pub async fn probe(cfg: &ProbeConfig, client: &Client) -> ProbeResult {
    let expected = cfg.expected_status.unwrap_or(200);
    let start = Instant::now();
    let probe_type = cfg.probe_type.to_string();

    let result = with_probe_timeout(
        cfg.timeout(),
        &format!("timeout after {}s", cfg.timeout_secs),
        client.get(&cfg.target).send(),
    )
    .await;
    let elapsed = start.elapsed().as_secs_f64() * 1000.0;

    match result {
        Ok(resp) => {
            let status = resp.status().as_u16();
            if status == expected {
                ProbeResult::success(&cfg.name, &probe_type, &cfg.target, elapsed, Some(status))
            } else {
                ProbeResult::failure(
                    &cfg.name,
                    &probe_type,
                    &cfg.target,
                    elapsed,
                    format!("unexpected status {status}, expected {expected}"),
                )
            }
        }
        Err(msg) => ProbeResult::failure(&cfg.name, &probe_type, &cfg.target, elapsed, msg),
    }
}

/// Build a shared reqwest client.
pub fn build_client() -> Client {
    Client::builder()
        .use_rustls_tls()
        // Don't follow redirects: an uptime check must judge the URL it was
        // given. Following (reqwest's default, up to 10 hops) would let
        // `expected_status: 200` silently pass for a 301→200 chain and fold the
        // extra round-trips into the measured response time.
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("Failed to build HTTP client")
}

#[cfg(test)]
mod tests {
    use super::*;
    use huginn_core::config::{ProbeConfig, ProbeType};
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn http_cfg(url: &str, expected_status: Option<u16>) -> ProbeConfig {
        ProbeConfig {
            name: "test-http".into(),
            probe_type: ProbeType::Http,
            target: url.into(),
            interval_secs: 10,
            timeout_secs: 5,
            expected_status,
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn succeeds_on_200() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        let client = build_client();
        let cfg = http_cfg(&server.uri(), Some(200));
        let result = probe(&cfg, &client).await;

        assert!(result.up);
        assert_eq!(result.status_code, Some(200));
    }

    #[tokio::test]
    async fn fails_on_unexpected_status() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;

        let client = build_client();
        let cfg = http_cfg(&server.uri(), Some(200));
        let result = probe(&cfg, &client).await;

        assert!(!result.up);
        assert!(result.error.as_deref().unwrap_or("").contains("503"));
    }

    #[tokio::test]
    async fn defaults_expected_status_200() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        let client = build_client();
        // No expected_status set → defaults to 200
        let cfg = http_cfg(&server.uri(), None);
        let result = probe(&cfg, &client).await;

        assert!(result.up);
    }

    #[tokio::test]
    async fn fails_on_unreachable_host() {
        let client = build_client();
        let cfg = ProbeConfig {
            timeout_secs: 1,
            ..http_cfg("http://127.0.0.1:19999", Some(200))
        };
        let result = probe(&cfg, &client).await;
        assert!(!result.up);
    }

    #[tokio::test]
    async fn fails_on_404_status() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        let client = build_client();
        let cfg = http_cfg(&server.uri(), Some(200));
        let result = probe(&cfg, &client).await;
        assert!(!result.up);
        assert!(result.error.as_deref().unwrap_or("").contains("404"));
    }

    #[tokio::test]
    async fn fails_on_500_status() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;

        let client = build_client();
        let cfg = http_cfg(&server.uri(), Some(200));
        let result = probe(&cfg, &client).await;
        assert!(!result.up);
        assert!(result.error.as_deref().unwrap_or("").contains("500"));
    }

    /// A redirect must not be followed: the probe judges the URL it was given,
    /// so a 301 with `expected_status: 200` is DOWN, not a silent pass.
    #[tokio::test]
    async fn does_not_follow_redirects() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(301).insert_header("Location", "/elsewhere"))
            .mount(&server)
            .await;

        let client = build_client();
        let cfg = http_cfg(&server.uri(), Some(200));
        let result = probe(&cfg, &client).await;

        assert!(!result.up, "a 301 must not be followed to a 200");
        assert!(
            result.error.as_deref().unwrap_or("").contains("301"),
            "error should report the redirect status: {:?}",
            result.error
        );
    }

    #[tokio::test]
    async fn https_probe_type_is_reflected_in_result() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        let client = build_client();
        let cfg = ProbeConfig {
            probe_type: ProbeType::Https,
            ..http_cfg(&server.uri(), Some(200))
        };
        let result = probe(&cfg, &client).await;
        assert!(result.up);
        assert_eq!(result.probe_type, "https");
    }
}
