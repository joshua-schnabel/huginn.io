use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Well-known keys for [`ProbeResult::metrics`].
///
/// Constants rather than bare strings so a typo is a compile error on one side
/// of the producer/consumer pair at least, and so the set is discoverable.
pub mod metric_keys {
    /// Days until the TLS certificate expires. Negative once expired.
    pub const TLS_CERT_EXPIRY_DAYS: &str = "tls_cert_expiry_days";
}

/// Result of a single probe execution.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProbeResult {
    pub probe_name: String,
    pub probe_type: String,
    pub target: String,
    pub up: bool,
    pub response_ms: f64,
    pub status_code: Option<u16>,
    pub error: Option<String>,
    pub timestamp: DateTime<Utc>,
    /// Extra numeric readings that only some probe types produce — currently
    /// TLS days to expiry, and whatever comes next.
    ///
    /// A map rather than named `Option` fields, which is what `status_code`
    /// above did and what the obvious next step would be: every consumer
    /// (InfluxDB line protocol, JSON/SSE) handles the map with one loop, so a
    /// new probe metric needs no consumer change at all. See `metric_keys` for
    /// the known keys.
    ///
    /// `BTreeMap`, not `HashMap`: iteration order feeds InfluxDB line protocol,
    /// and random field order would make output irreproducible.
    ///
    /// `skip_serializing_if` keeps existing JSON byte-identical for every probe
    /// type that produces no metrics — the SSE payload and `/metrics/latest`
    /// contract are unchanged.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metrics: BTreeMap<String, f64>,
}

impl ProbeResult {
    pub fn success(
        probe_name: impl Into<String>,
        probe_type: impl Into<String>,
        target: impl Into<String>,
        response_ms: f64,
        status_code: Option<u16>,
    ) -> Self {
        Self {
            probe_name: probe_name.into(),
            probe_type: probe_type.into(),
            target: target.into(),
            up: true,
            response_ms,
            status_code,
            error: None,
            timestamp: Utc::now(),
            metrics: BTreeMap::new(),
        }
    }

    pub fn failure(
        probe_name: impl Into<String>,
        probe_type: impl Into<String>,
        target: impl Into<String>,
        response_ms: f64,
        error: impl Into<String>,
    ) -> Self {
        Self {
            probe_name: probe_name.into(),
            probe_type: probe_type.into(),
            target: target.into(),
            up: false,
            response_ms,
            status_code: None,
            error: Some(error.into()),
            timestamp: Utc::now(),
            metrics: BTreeMap::new(),
        }
    }

    /// Attach a metric, builder-style: `ProbeResult::success(..).with_metric(k, v)`.
    ///
    /// A builder rather than more constructor parameters — `success`/`failure`
    /// have 36 call sites, and none of them need to change for this.
    #[must_use]
    pub fn with_metric(mut self, key: impl Into<String>, value: f64) -> Self {
        self.metrics.insert(key.into(), value);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn success_result_is_up() {
        let r = ProbeResult::success("my-probe", "tcp", "localhost:80", 12.5, None);
        assert!(r.up);
        assert_eq!(r.response_ms, 12.5);
        assert!(r.error.is_none());
    }

    #[test]
    fn failure_result_is_down() {
        let r = ProbeResult::failure("my-probe", "tcp", "localhost:80", 5000.0, "timeout");
        assert!(!r.up);
        assert_eq!(r.error.as_deref(), Some("timeout"));
    }

    #[test]
    fn serialize_roundtrip() {
        let r = ProbeResult::success("p1", "http", "https://example.com", 42.0, Some(200));
        let json = serde_json::to_string(&r).unwrap();
        let decoded: ProbeResult = serde_json::from_str(&json).unwrap();
        assert_eq!(r, decoded);
    }

    /// The property that makes adding `metrics` safe: probe types that produce
    /// none serialise exactly as before, so the SSE payload and the
    /// /metrics/latest contract are untouched.
    #[test]
    fn empty_metrics_are_absent_from_json() {
        let r = ProbeResult::success("p1", "tcp", "host:80", 1.0, None);
        let json = serde_json::to_string(&r).unwrap();
        assert!(
            !json.contains("metrics"),
            "empty metrics must not appear in JSON: {json}"
        );
    }

    #[test]
    fn metrics_roundtrip_when_present() {
        let r = ProbeResult::success("p1", "tls", "example.com:443", 12.0, None)
            .with_metric(metric_keys::TLS_CERT_EXPIRY_DAYS, 47.0);

        let json = serde_json::to_string(&r).unwrap();
        assert!(json.contains("tls_cert_expiry_days"));

        let decoded: ProbeResult = serde_json::from_str(&json).unwrap();
        assert_eq!(r, decoded);
        assert_eq!(decoded.metrics["tls_cert_expiry_days"], 47.0);
    }

    /// Old payloads without the field must still deserialise.
    #[test]
    fn json_without_metrics_field_deserialises() {
        let json = r#"{"probe_name":"p","probe_type":"tcp","target":"h:80","up":true,
            "response_ms":1.0,"status_code":null,"error":null,
            "timestamp":"2024-01-15T10:00:00Z"}"#;
        let decoded: ProbeResult = serde_json::from_str(json).unwrap();
        assert!(decoded.metrics.is_empty());
    }

    #[test]
    fn with_metric_is_chainable() {
        let r = ProbeResult::success("p", "tls", "example.com:443", 5.0, None)
            .with_metric(metric_keys::TLS_CERT_EXPIRY_DAYS, 47.0)
            .with_metric("another_reading", 4.0);
        assert_eq!(r.metrics.len(), 2);
        assert_eq!(r.metrics["tls_cert_expiry_days"], 47.0);
    }

    /// Expired certificates are the case worth reporting, so the value has to
    /// carry a sign.
    #[test]
    fn metric_values_may_be_negative() {
        let r = ProbeResult::failure("p", "tls", "example.com:443", 5.0, "expired")
            .with_metric(metric_keys::TLS_CERT_EXPIRY_DAYS, -4.0);
        assert_eq!(r.metrics["tls_cert_expiry_days"], -4.0);
        assert!(!r.up);
    }
}
