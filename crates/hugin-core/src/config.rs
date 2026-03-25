use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::Duration;

use crate::error::{HuginError, Result};

// ---------------------------------------------------------------------------
// Top-level config
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub influx: InfluxConfig,
    #[serde(default)]
    pub probes: Vec<ProbeConfig>,
    #[serde(default)]
    pub ui: UiConfig,
    #[serde(default)]
    pub log: LogConfig,
}

// ---------------------------------------------------------------------------
// InfluxDB config
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InfluxConfig {
    pub url: String,
    pub org: String,
    pub bucket: String,
    /// Path to a file containing the InfluxDB token.
    /// The token is read from the file at runtime — never stored in env directly.
    #[serde(default = "default_token_file")]
    pub token_file: String,
}

fn default_token_file() -> String {
    "/run/secrets/influx_token".to_string()
}

impl InfluxConfig {
    /// Read the token from the file specified by `token_file`.
    pub fn read_token(&self) -> Result<String> {
        std::fs::read_to_string(&self.token_file)
            .map(|s| s.trim().to_string())
            .map_err(|e| HuginError::Secret {
                path: self.token_file.clone(),
                message: e.to_string(),
            })
    }
}

// ---------------------------------------------------------------------------
// Optional debug UI config
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_ui_port")]
    pub port: u16,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            port: default_ui_port(),
        }
    }
}

fn default_ui_port() -> u16 {
    9116
}

// ---------------------------------------------------------------------------
// Log / output config
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum LogFormat {
    Pretty,
    Json,
}

impl Default for LogFormat {
    fn default() -> Self {
        LogFormat::Pretty
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogConfig {
    #[serde(default)]
    pub format: LogFormat,
    #[serde(default = "default_log_level")]
    pub level: String,
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            format: LogFormat::Pretty,
            level: default_log_level(),
        }
    }
}

fn default_log_level() -> String {
    "info".to_string()
}

// ---------------------------------------------------------------------------
// Probe config
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ProbeType {
    Tcp,
    Http,
    Https,
    Smtp,
    Imap,
    Udp,
}

impl std::fmt::Display for ProbeType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            ProbeType::Tcp => "tcp",
            ProbeType::Http => "http",
            ProbeType::Https => "https",
            ProbeType::Smtp => "smtp",
            ProbeType::Imap => "imap",
            ProbeType::Udp => "udp",
        };
        write!(f, "{s}")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProbeConfig {
    pub name: String,
    #[serde(rename = "type")]
    pub probe_type: ProbeType,
    pub target: String,
    #[serde(default = "default_interval")]
    pub interval_secs: u64,
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
    /// HTTP/HTTPS only: expected HTTP status code.
    pub expected_status: Option<u16>,
}

impl ProbeConfig {
    pub fn timeout(&self) -> Duration {
        Duration::from_secs(self.timeout_secs)
    }

    pub fn interval(&self) -> Duration {
        Duration::from_secs(self.interval_secs)
    }
}

fn default_interval() -> u64 {
    30
}

fn default_timeout() -> u64 {
    5
}

// ---------------------------------------------------------------------------
// Loading & validation
// ---------------------------------------------------------------------------

impl AppConfig {
    /// Load config from a YAML file and apply ENV overrides.
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let content = std::fs::read_to_string(path.as_ref()).map_err(|e| {
            HuginError::Config(format!(
                "Cannot read config file '{}': {e}",
                path.as_ref().display()
            ))
        })?;
        let mut cfg: AppConfig = serde_yaml::from_str(&content)
            .map_err(|e| HuginError::Config(format!("YAML parse error: {e}")))?;
        cfg.apply_env_overrides();
        cfg.validate()?;
        Ok(cfg)
    }

    /// Override config values from environment variables (HUGIN_* / INFLUX_*).
    pub fn apply_env_overrides(&mut self) {
        if let Ok(v) = std::env::var("INFLUX_URL") {
            self.influx.url = v;
        }
        if let Ok(v) = std::env::var("INFLUX_ORG") {
            self.influx.org = v;
        }
        if let Ok(v) = std::env::var("INFLUX_BUCKET") {
            self.influx.bucket = v;
        }
        if let Ok(v) = std::env::var("INFLUX_TOKEN_FILE") {
            self.influx.token_file = v;
        }
        if let Ok(v) = std::env::var("HUGIN_UI_ENABLED") {
            self.ui.enabled = v.to_lowercase() == "true" || v == "1";
        }
        if let Ok(v) = std::env::var("HUGIN_UI_PORT") {
            if let Ok(p) = v.parse::<u16>() {
                self.ui.port = p;
            }
        }
        if let Ok(v) = std::env::var("HUGIN_LOG_FORMAT") {
            self.log.format = match v.to_lowercase().as_str() {
                "json" => LogFormat::Json,
                _ => LogFormat::Pretty,
            };
        }
        if let Ok(v) = std::env::var("HUGIN_LOG_LEVEL") {
            self.log.level = v;
        }
    }

    fn validate(&self) -> Result<()> {
        if self.influx.url.is_empty() {
            return Err(HuginError::Config("influx.url must not be empty".into()));
        }
        if self.influx.org.is_empty() {
            return Err(HuginError::Config("influx.org must not be empty".into()));
        }
        if self.influx.bucket.is_empty() {
            return Err(HuginError::Config("influx.bucket must not be empty".into()));
        }
        for probe in &self.probes {
            if probe.name.is_empty() {
                return Err(HuginError::Config("probe name must not be empty".into()));
            }
            if probe.target.is_empty() {
                return Err(HuginError::Config(format!(
                    "probe '{}': target must not be empty",
                    probe.name
                )));
            }
            if probe.interval_secs == 0 {
                return Err(HuginError::Config(format!(
                    "probe '{}': interval_secs must be > 0",
                    probe.name
                )));
            }
            if probe.timeout_secs == 0 {
                return Err(HuginError::Config(format!(
                    "probe '{}': timeout_secs must be > 0",
                    probe.name
                )));
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL_YAML: &str = r#"
influx:
  url: "http://localhost:8086"
  org: "testorg"
  bucket: "testbucket"
  token_file: "/tmp/token"
"#;

    const FULL_YAML: &str = r#"
influx:
  url: "http://localhost:8086"
  org: "myorg"
  bucket: "monitoring"
  token_file: "/run/secrets/influx_token"

probes:
  - name: "web"
    type: http
    target: "https://example.com"
    interval_secs: 30
    timeout_secs: 5
    expected_status: 200
  - name: "db"
    type: tcp
    target: "db.example.com:5432"
  - name: "dns"
    type: udp
    target: "8.8.8.8:53"
  - name: "mail-smtp"
    type: smtp
    target: "mail.example.com:25"
  - name: "mail-imap"
    type: imap
    target: "mail.example.com:143"
"#;

    fn parse(yaml: &str) -> AppConfig {
        let cfg: AppConfig = serde_yaml::from_str(yaml).expect("parse failed");
        cfg.validate().expect("validation failed");
        cfg
    }

    #[test]
    fn parses_minimal_config() {
        let cfg = parse(MINIMAL_YAML);
        assert_eq!(cfg.influx.url, "http://localhost:8086");
        assert_eq!(cfg.influx.org, "testorg");
        assert!(cfg.probes.is_empty());
    }

    #[test]
    fn parses_full_config_all_probe_types() {
        let cfg = parse(FULL_YAML);
        assert_eq!(cfg.probes.len(), 5);
        assert_eq!(cfg.probes[0].probe_type, ProbeType::Http);
        assert_eq!(cfg.probes[0].expected_status, Some(200));
        assert_eq!(cfg.probes[1].probe_type, ProbeType::Tcp);
        assert_eq!(cfg.probes[2].probe_type, ProbeType::Udp);
        assert_eq!(cfg.probes[3].probe_type, ProbeType::Smtp);
        assert_eq!(cfg.probes[4].probe_type, ProbeType::Imap);
    }

    #[test]
    fn default_interval_and_timeout() {
        let cfg = parse(FULL_YAML);
        let tcp = &cfg.probes[1];
        assert_eq!(tcp.interval_secs, 30);
        assert_eq!(tcp.timeout_secs, 5);
        assert_eq!(tcp.timeout(), Duration::from_secs(5));
    }

    #[test]
    fn default_ui_disabled() {
        let cfg = parse(MINIMAL_YAML);
        assert!(!cfg.ui.enabled);
        assert_eq!(cfg.ui.port, 9116);
    }

    #[test]
    fn default_log_pretty() {
        let cfg = parse(MINIMAL_YAML);
        assert_eq!(cfg.log.format, LogFormat::Pretty);
        assert_eq!(cfg.log.level, "info");
    }

    #[test]
    fn probe_type_display() {
        assert_eq!(ProbeType::Http.to_string(), "http");
        assert_eq!(ProbeType::Https.to_string(), "https");
        assert_eq!(ProbeType::Tcp.to_string(), "tcp");
        assert_eq!(ProbeType::Udp.to_string(), "udp");
        assert_eq!(ProbeType::Smtp.to_string(), "smtp");
        assert_eq!(ProbeType::Imap.to_string(), "imap");
    }

    #[test]
    fn validation_rejects_empty_url() {
        let yaml = r#"
influx:
  url: ""
  org: "o"
  bucket: "b"
  token_file: "/tmp/t"
"#;
        let cfg: AppConfig = serde_yaml::from_str(yaml).unwrap();
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn validation_rejects_zero_interval() {
        let yaml = r#"
influx:
  url: "http://localhost:8086"
  org: "o"
  bucket: "b"
  token_file: "/tmp/t"
probes:
  - name: "x"
    type: tcp
    target: "host:80"
    interval_secs: 0
"#;
        let cfg: AppConfig = serde_yaml::from_str(yaml).unwrap();
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn env_override_influx_url() {
        std::env::set_var("INFLUX_URL", "http://remotehost:8086");
        let mut cfg: AppConfig = serde_yaml::from_str(MINIMAL_YAML).unwrap();
        cfg.apply_env_overrides();
        assert_eq!(cfg.influx.url, "http://remotehost:8086");
        std::env::remove_var("INFLUX_URL");
    }

    #[test]
    fn env_override_log_format_json() {
        std::env::set_var("HUGIN_LOG_FORMAT", "json");
        let mut cfg: AppConfig = serde_yaml::from_str(MINIMAL_YAML).unwrap();
        cfg.apply_env_overrides();
        assert_eq!(cfg.log.format, LogFormat::Json);
        std::env::remove_var("HUGIN_LOG_FORMAT");
    }

    #[test]
    fn env_override_ui_enabled() {
        std::env::set_var("HUGIN_UI_ENABLED", "true");
        std::env::set_var("HUGIN_UI_PORT", "8080");
        let mut cfg: AppConfig = serde_yaml::from_str(MINIMAL_YAML).unwrap();
        cfg.apply_env_overrides();
        assert!(cfg.ui.enabled);
        assert_eq!(cfg.ui.port, 8080);
        std::env::remove_var("HUGIN_UI_ENABLED");
        std::env::remove_var("HUGIN_UI_PORT");
    }
}
