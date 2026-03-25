use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

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
        }
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
}
