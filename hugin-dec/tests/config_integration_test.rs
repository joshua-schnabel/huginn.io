/// TDD: Integration tests for config loading + ENV overrides.
/// Written FIRST — RED phase.
use hugin_core::config::{AppConfig, LogFormat, ProbeType};
use std::io::Write;

fn write_config(content: &str) -> tempfile::NamedTempFile {
    let mut f = tempfile::NamedTempFile::new().unwrap();
    write!(f, "{content}").unwrap();
    f
}

const VALID_CONFIG: &str = r#"
influx:
  url: "http://localhost:8086"
  org: "testorg"
  bucket: "testbucket"
  token_file: "/dev/null"
probes:
  - name: "web"
    type: http
    target: "https://example.com"
    interval_secs: 30
    timeout_secs: 5
    expected_status: 200
"#;

#[test]
fn loads_valid_config_file() {
    let f = write_config(VALID_CONFIG);
    let cfg = AppConfig::load(f.path()).expect("should load");
    assert_eq!(cfg.influx.org, "testorg");
    assert_eq!(cfg.probes.len(), 1);
    assert_eq!(cfg.probes[0].probe_type, ProbeType::Http);
}

#[test]
fn returns_error_on_missing_file() {
    let result = AppConfig::load("/nonexistent/path/config.yaml");
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("Cannot read"));
}

#[test]
fn returns_error_on_invalid_yaml() {
    let f = write_config("not: valid: yaml: [[[");
    let result = AppConfig::load(f.path());
    assert!(result.is_err());
}

#[test]
fn env_overrides_influx_url() {
    std::env::set_var("INFLUX_URL", "http://override:9999");
    let f = write_config(VALID_CONFIG);
    let cfg = AppConfig::load(f.path()).unwrap();
    assert_eq!(cfg.influx.url, "http://override:9999");
    std::env::remove_var("INFLUX_URL");
}

#[test]
fn env_token_file_never_reads_inline_token() {
    // INFLUX_TOKEN (not INFLUX_TOKEN_FILE) must NOT be read — security requirement
    std::env::set_var("INFLUX_TOKEN", "should-be-ignored");
    let f = write_config(VALID_CONFIG);
    let cfg = AppConfig::load(f.path()).unwrap();
    // token_file should still be whatever is in YAML, not "should-be-ignored"
    assert_ne!(cfg.influx.token_file, "should-be-ignored");
    std::env::remove_var("INFLUX_TOKEN");
}

#[test]
fn env_log_format_json_overrides_pretty() {
    std::env::set_var("HUGIN_LOG_FORMAT", "json");
    let f = write_config(VALID_CONFIG);
    let cfg = AppConfig::load(f.path()).unwrap();
    assert_eq!(cfg.log.format, LogFormat::Json);
    std::env::remove_var("HUGIN_LOG_FORMAT");
}

#[test]
fn ui_disabled_by_default() {
    let f = write_config(VALID_CONFIG);
    let cfg = AppConfig::load(f.path()).unwrap();
    assert!(!cfg.ui.enabled);
    assert_eq!(cfg.ui.port, 9116);
}
