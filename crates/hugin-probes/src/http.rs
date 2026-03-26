use std::time::Instant;

use hugin_core::config::ProbeConfig;
use hugin_core::types::ProbeResult;
use reqwest::Client;
use tokio::time::timeout;

/// Perform an HTTP/HTTPS GET request and measure response time.
pub async fn probe(cfg: &ProbeConfig, client: &Client) -> ProbeResult {
    let expected = cfg.expected_status.unwrap_or(200);
    let start = Instant::now();

    let fut = client.get(&cfg.target).send();
    let result = timeout(cfg.timeout(), fut).await;
    let elapsed = start.elapsed().as_secs_f64() * 1000.0;
    let probe_type = cfg.probe_type.to_string();

    match result {
        Ok(Ok(resp)) => {
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
        Ok(Err(e)) => {
            ProbeResult::failure(&cfg.name, &probe_type, &cfg.target, elapsed, e.to_string())
        }
        Err(_) => ProbeResult::failure(
            &cfg.name,
            &probe_type,
            &cfg.target,
            elapsed,
            format!("timeout after {}s", cfg.timeout_secs),
        ),
    }
}

/// Build a shared reqwest client with a default timeout.
pub fn build_client() -> Client {
    Client::builder()
        .use_rustls_tls()
        .build()
        .expect("Failed to build HTTP client")
}

#[cfg(test)]
mod tests {
    use super::*;
    use hugin_core::config::{ProbeConfig, ProbeType};
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
            dns_query: None,
            dns_expected_ip: None,
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
