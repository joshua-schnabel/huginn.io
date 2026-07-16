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
    /// Capacity of the central EventHub broadcast channel (default 256).
    #[serde(default = "default_hub_capacity")]
    pub event_hub_capacity: usize,
}

fn default_hub_capacity() -> usize {
    256
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
    /// Number of probe results to buffer before flushing to InfluxDB (default 10).
    #[serde(default = "default_batch_size")]
    pub batch_size: usize,
    /// Maximum time in milliseconds to wait before flushing a non-full batch (default 1000).
    #[serde(default = "default_batch_timeout_ms")]
    pub batch_timeout_ms: u64,
    /// Memory ceiling for batches waiting to be written while InfluxDB is
    /// unreachable (default 8 MiB ≈ 35k–55k results).
    ///
    /// Bytes rather than a point count: bytes are what actually bound RSS, and
    /// what the queue holds is already-rendered line protocol, so no estimation
    /// is involved. When full, the *oldest* batch is dropped.
    #[serde(default = "default_max_buffered_bytes")]
    pub max_buffered_bytes: usize,
    /// First retry delay after a failed write, in milliseconds (default 500).
    /// Doubles per attempt up to `retry_max_backoff_ms`.
    #[serde(default = "default_retry_initial_backoff_ms")]
    pub retry_initial_backoff_ms: u64,
    /// Ceiling for the retry backoff, in milliseconds (default 30000).
    #[serde(default = "default_retry_max_backoff_ms")]
    pub retry_max_backoff_ms: u64,
    /// How long to keep draining buffered batches after a shutdown signal,
    /// in milliseconds (default 5000).
    ///
    /// Retries are otherwise unbounded, so without a deadline a shutdown while
    /// InfluxDB is down would never complete.
    #[serde(default = "default_shutdown_drain_timeout_ms")]
    pub shutdown_drain_timeout_ms: u64,
}

fn default_token_file() -> String {
    "/run/secrets/influx_token".to_string()
}

fn default_batch_size() -> usize {
    10
}

fn default_batch_timeout_ms() -> u64 {
    1000
}

fn default_max_buffered_bytes() -> usize {
    8 * 1024 * 1024
}

fn default_retry_initial_backoff_ms() -> u64 {
    500
}

fn default_retry_max_backoff_ms() -> u64 {
    30_000
}

fn default_shutdown_drain_timeout_ms() -> u64 {
    5_000
}

/// Mirrors the serde defaults, so `InfluxConfig::default()` and an empty YAML
/// `influx:` block agree. Used by test fixtures; `url`/`org`/`bucket` have no
/// sensible default and `validate()` rejects them empty.
impl Default for InfluxConfig {
    fn default() -> Self {
        Self {
            url: String::new(),
            org: String::new(),
            bucket: String::new(),
            token_file: default_token_file(),
            batch_size: default_batch_size(),
            batch_timeout_ms: default_batch_timeout_ms(),
            max_buffered_bytes: default_max_buffered_bytes(),
            retry_initial_backoff_ms: default_retry_initial_backoff_ms(),
            retry_max_backoff_ms: default_retry_max_backoff_ms(),
            shutdown_drain_timeout_ms: default_shutdown_drain_timeout_ms(),
        }
    }
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "lowercase")]
pub enum LogFormat {
    #[default]
    Pretty,
    Json,
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
    Dns,
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
            ProbeType::Dns => "dns",
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
    /// DNS only: hostname to resolve (default: "example.com")
    #[serde(default)]
    pub dns_query: Option<String>,
    /// DNS only: expected IP address in the response (optional validation)
    #[serde(default)]
    pub dns_expected_ip: Option<String>,
}

/// Hand-written rather than `#[derive(Default)]`.
///
/// Deriving would need a `#[default]` variant on `ProbeType`. That variant would
/// then be sitting there as the obvious thing to pair with `#[serde(default)]`,
/// which would turn a config with a typo'd or missing `type:` into a silent TCP
/// probe. A missing probe type must stay a deserialisation error; this impl
/// exists for test fixtures, not for YAML.
impl Default for ProbeConfig {
    fn default() -> Self {
        Self {
            name: String::new(),
            probe_type: ProbeType::Tcp,
            target: String::new(),
            interval_secs: default_interval(),
            timeout_secs: default_timeout(),
            expected_status: None,
            dns_query: None,
            dns_expected_ip: None,
        }
    }
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
        Self::load_with_warnings(path).map(|(cfg, _)| cfg)
    }

    /// Like [`load`](Self::load), but also returns messages about ENV variables
    /// that were set but unusable.
    ///
    /// They are returned rather than logged because config is loaded *before*
    /// the tracing subscriber is initialised — the log level to initialise it
    /// with comes from this very config. Anything logged here would go nowhere.
    /// The caller emits them once tracing is up.
    pub fn load_with_warnings(path: impl AsRef<Path>) -> Result<(Self, Vec<String>)> {
        let content = std::fs::read_to_string(path.as_ref()).map_err(|e| {
            HuginError::Config(format!(
                "Cannot read config file '{}': {e}",
                path.as_ref().display()
            ))
        })?;
        let mut cfg: AppConfig = serde_yaml_ng::from_str(&content)
            .map_err(|e| HuginError::Config(format!("YAML parse error: {e}")))?;
        let warnings = cfg.apply_env_overrides();
        cfg.validate()?;
        Ok((cfg, warnings))
    }

    /// Override config values from environment variables (HUGINN_* / INFLUX_*).
    ///
    /// Returns a message for every variable that was set but could not be used.
    /// These were previously swallowed: `HUGINN_UI_PORT=abc` was ignored,
    /// `HUGINN_LOG_FORMAT=xml` quietly became `pretty`, and `HUGINN_UI_ENABLED=yes`
    /// quietly became `false` — a typo in a deployment looked exactly like a
    /// deliberate setting.
    pub fn apply_env_overrides(&mut self) -> Vec<String> {
        let mut warnings = Vec::new();

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
        if let Ok(v) = std::env::var("HUGINN_UI_ENABLED") {
            match v.to_lowercase().as_str() {
                "true" | "1" => self.ui.enabled = true,
                "false" | "0" => self.ui.enabled = false,
                _ => warnings.push(format!(
                    "HUGINN_UI_ENABLED='{v}' is not a boolean (expected true/false/1/0) — \
                     keeping ui.enabled={}",
                    self.ui.enabled
                )),
            }
        }
        if let Ok(v) = std::env::var("HUGINN_UI_PORT") {
            match v.parse::<u16>() {
                Ok(p) => self.ui.port = p,
                Err(_) => warnings.push(format!(
                    "HUGINN_UI_PORT='{v}' is not a valid port — keeping ui.port={}",
                    self.ui.port
                )),
            }
        }
        if let Ok(v) = std::env::var("HUGINN_LOG_FORMAT") {
            match v.to_lowercase().as_str() {
                "json" => self.log.format = LogFormat::Json,
                "pretty" => self.log.format = LogFormat::Pretty,
                _ => warnings.push(format!(
                    "HUGINN_LOG_FORMAT='{v}' is not a known format (expected json/pretty) — \
                     keeping log.format={:?}",
                    self.log.format
                )),
            }
        }
        if let Ok(v) = std::env::var("HUGINN_LOG_LEVEL") {
            self.log.level = v;
        }

        warnings
    }

    /// Reject configurations that would fail later, at a point where the cause
    /// is much harder to see: a panic during startup, or a probe that reports
    /// DOWN forever for a reason that was decidable here.
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
        if self.influx.batch_size == 0 {
            return Err(HuginError::Config(
                "influx.batch_size must be > 0 (0 would flush a POST per probe result)".into(),
            ));
        }
        // tokio::sync::broadcast::channel panics on a capacity of 0, so without
        // this the process dies at startup with no hint about which key is wrong.
        if self.event_hub_capacity == 0 {
            return Err(HuginError::Config("event_hub_capacity must be > 0".into()));
        }

        let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();

        for probe in &self.probes {
            if probe.name.is_empty() {
                return Err(HuginError::Config("probe name must not be empty".into()));
            }
            // Duplicate names collapse into one entry in the web UI's map and
            // share a tag series in InfluxDB, so two probes silently overwrite
            // each other's history.
            if !seen.insert(probe.name.as_str()) {
                return Err(HuginError::Config(format!(
                    "duplicate probe name '{}': names must be unique — they key the UI and the InfluxDB series",
                    probe.name
                )));
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
            probe.validate_target()?;
        }
        Ok(())
    }
}

impl ProbeConfig {
    /// Check that `target` has the shape this probe type needs.
    ///
    /// These are decidable up front, and getting them wrong otherwise produces a
    /// probe that reports DOWN on every tick, forever, for a reason that looks
    /// like an outage.
    fn validate_target(&self) -> Result<()> {
        let err = |msg: String| Err(HuginError::Config(format!("probe '{}': {msg}", self.name)));

        match self.probe_type {
            // The DNS probe parses target as a SocketAddr — no resolution, no
            // default port. Same check here, same rules.
            ProbeType::Dns => {
                if self.target.parse::<std::net::SocketAddr>().is_err() {
                    return err(format!(
                        "dns target '{}' must be a nameserver address with a port, \
                         e.g. '8.8.8.8:53' or '[2001:4860:4860::8888]:53'",
                        self.target
                    ));
                }
            }
            // Handed to TcpStream/UdpSocket connect, which needs host:port.
            ProbeType::Tcp | ProbeType::Smtp | ProbeType::Imap | ProbeType::Udp => {
                if !has_port_suffix(&self.target) {
                    return err(format!(
                        "{} target '{}' must include a port, e.g. '{}:{}'",
                        self.probe_type,
                        self.target,
                        self.target,
                        default_port_hint(&self.probe_type),
                    ));
                }
            }
            // Handed to reqwest, which needs an absolute URL. Note the scheme is
            // not required to match the probe type: the shipped example uses
            // `type: http` with an https:// target.
            ProbeType::Http | ProbeType::Https => {
                if !(self.target.starts_with("http://") || self.target.starts_with("https://")) {
                    return err(format!(
                        "{} target '{}' must be an absolute URL starting with http:// or https://",
                        self.probe_type, self.target
                    ));
                }
            }
        }
        Ok(())
    }
}

/// Whether `target` ends in `:<port>`.
///
/// Split from the right so that IPv6 literals work: `[::1]:53` yields "53",
/// while a bracketless `::1` yields "1" and is accepted. That false positive is
/// tolerated — the bracketless form is not valid input for these probes anyway,
/// and the alternative is resolving names during validation.
fn has_port_suffix(target: &str) -> bool {
    match target.rsplit_once(':') {
        Some((host, port)) => !host.is_empty() && port.parse::<u16>().is_ok(),
        None => false,
    }
}

fn default_port_hint(probe_type: &ProbeType) -> u16 {
    match probe_type {
        ProbeType::Smtp => 25,
        ProbeType::Imap => 143,
        ProbeType::Udp => 53,
        _ => 80,
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
  - name: "dns-probe"
    type: dns
    target: "1.1.1.1:53"
    dns_query: "example.com"
    dns_expected_ip: "93.184.216.34"
"#;

    fn parse(yaml: &str) -> AppConfig {
        let cfg: AppConfig = serde_yaml_ng::from_str(yaml).expect("parse failed");
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
        assert_eq!(cfg.probes.len(), 6);
        assert_eq!(cfg.probes[0].probe_type, ProbeType::Http);
        assert_eq!(cfg.probes[0].expected_status, Some(200));
        assert_eq!(cfg.probes[1].probe_type, ProbeType::Tcp);
        assert_eq!(cfg.probes[2].probe_type, ProbeType::Udp);
        assert_eq!(cfg.probes[3].probe_type, ProbeType::Smtp);
        assert_eq!(cfg.probes[4].probe_type, ProbeType::Imap);
        assert_eq!(cfg.probes[5].probe_type, ProbeType::Dns);
        assert_eq!(cfg.probes[5].dns_query, Some("example.com".into()));
        assert_eq!(cfg.probes[5].dns_expected_ip, Some("93.184.216.34".into()));
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
        assert_eq!(ProbeType::Dns.to_string(), "dns");
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
        let cfg: AppConfig = serde_yaml_ng::from_str(yaml).unwrap();
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
        let cfg: AppConfig = serde_yaml_ng::from_str(yaml).unwrap();
        assert!(cfg.validate().is_err());
    }

    /// Build a config from probe stanzas, with valid influx defaults.
    fn cfg_with_probes(probes: &str) -> AppConfig {
        let yaml = format!(
            r#"
influx:
  url: "http://localhost:8086"
  org: "o"
  bucket: "b"
  token_file: "/tmp/t"
probes:
{probes}"#
        );
        serde_yaml_ng::from_str(&yaml).unwrap()
    }

    /// Duplicate names silently merge in the UI map and share an InfluxDB series.
    #[test]
    fn validation_rejects_duplicate_probe_names() {
        let cfg = cfg_with_probes(
            r#"  - name: "dup"
    type: tcp
    target: "a:80"
  - name: "dup"
    type: tcp
    target: "b:80"
"#,
        );
        let err = cfg.validate().unwrap_err().to_string();
        assert!(err.contains("duplicate probe name 'dup'"), "got: {err}");
    }

    /// broadcast::channel(0) panics — this must be a config error, not a crash.
    #[test]
    fn validation_rejects_zero_hub_capacity() {
        let mut cfg = cfg_with_probes("  []\n");
        cfg.event_hub_capacity = 0;
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn validation_rejects_zero_batch_size() {
        let mut cfg = cfg_with_probes("  []\n");
        cfg.influx.batch_size = 0;
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn validation_rejects_dns_target_without_port() {
        let cfg = cfg_with_probes(
            r#"  - name: "d"
    type: dns
    target: "8.8.8.8"
"#,
        );
        let err = cfg.validate().unwrap_err().to_string();
        assert!(
            err.contains("must be a nameserver address with a port"),
            "got: {err}"
        );
    }

    #[test]
    fn validation_accepts_ipv6_dns_target() {
        let cfg = cfg_with_probes(
            r#"  - name: "d"
    type: dns
    target: "[2001:4860:4860::8888]:53"
"#,
        );
        assert!(cfg.validate().is_ok(), "{:?}", cfg.validate());
    }

    #[test]
    fn validation_rejects_tcp_target_without_port() {
        let cfg = cfg_with_probes(
            r#"  - name: "t"
    type: tcp
    target: "db.example.com"
"#,
        );
        let err = cfg.validate().unwrap_err().to_string();
        assert!(err.contains("must include a port"), "got: {err}");
    }

    #[test]
    fn validation_rejects_http_target_without_scheme() {
        let cfg = cfg_with_probes(
            r#"  - name: "h"
    type: http
    target: "example.com:80"
"#,
        );
        let err = cfg.validate().unwrap_err().to_string();
        assert!(err.contains("absolute URL"), "got: {err}");
    }

    /// The shipped example pairs `type: http` with an https:// target, so the
    /// scheme deliberately does not have to match the probe type.
    #[test]
    fn validation_allows_https_target_on_http_probe() {
        let cfg = cfg_with_probes(
            r#"  - name: "h"
    type: http
    target: "https://example.com"
"#,
        );
        assert!(cfg.validate().is_ok(), "{:?}", cfg.validate());
    }

    /// The environment is process-global while cargo runs these tests on
    /// parallel threads, so anything touching it must be serialised. Without
    /// this, one test's remove_var races another's set_var.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Lock the environment, set `vars`, run `f`, then always clean up.
    fn with_env<T>(vars: &[(&str, &str)], f: impl FnOnce() -> T) -> T {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        for (k, v) in vars {
            std::env::set_var(k, v);
        }
        let out = f();
        for (k, _) in vars {
            std::env::remove_var(k);
        }
        out
    }

    #[test]
    fn env_override_influx_url() {
        with_env(&[("INFLUX_URL", "http://remotehost:8086")], || {
            let mut cfg: AppConfig = serde_yaml_ng::from_str(MINIMAL_YAML).unwrap();
            cfg.apply_env_overrides();
            assert_eq!(cfg.influx.url, "http://remotehost:8086");
        });
    }

    #[test]
    fn env_override_log_format_json() {
        with_env(&[("HUGINN_LOG_FORMAT", "json")], || {
            let mut cfg: AppConfig = serde_yaml_ng::from_str(MINIMAL_YAML).unwrap();
            cfg.apply_env_overrides();
            assert_eq!(cfg.log.format, LogFormat::Json);
        });
    }

    #[test]
    fn env_override_ui_enabled() {
        with_env(
            &[("HUGINN_UI_ENABLED", "true"), ("HUGINN_UI_PORT", "8080")],
            || {
                let mut cfg: AppConfig = serde_yaml_ng::from_str(MINIMAL_YAML).unwrap();
                cfg.apply_env_overrides();
                assert!(cfg.ui.enabled);
                assert_eq!(cfg.ui.port, 8080);
            },
        );
    }

    /// `HUGINN_UI_ENABLED=false` must actually disable, not just "not equal true".
    #[test]
    fn env_ui_enabled_false_disables() {
        with_env(&[("HUGINN_UI_ENABLED", "false")], || {
            let mut cfg: AppConfig = serde_yaml_ng::from_str(MINIMAL_YAML).unwrap();
            cfg.ui.enabled = true;
            let w = cfg.apply_env_overrides();
            assert!(!cfg.ui.enabled);
            assert!(w.is_empty(), "false is valid, should not warn: {w:?}");
        });
    }

    /// A typo'd port used to be dropped on the floor and look deliberate.
    #[test]
    fn env_invalid_ui_port_warns_and_keeps_previous() {
        with_env(&[("HUGINN_UI_PORT", "not-a-port")], || {
            let mut cfg: AppConfig = serde_yaml_ng::from_str(MINIMAL_YAML).unwrap();
            let w = cfg.apply_env_overrides();
            assert_eq!(cfg.ui.port, 9116, "should keep the previous value");
            assert_eq!(w.len(), 1);
            assert!(w[0].contains("HUGINN_UI_PORT"), "got: {w:?}");
        });
    }

    /// `yes` is not a boolean here; it used to silently mean `false`.
    #[test]
    fn env_invalid_ui_enabled_warns_and_keeps_previous() {
        with_env(&[("HUGINN_UI_ENABLED", "yes")], || {
            let mut cfg: AppConfig = serde_yaml_ng::from_str(MINIMAL_YAML).unwrap();
            cfg.ui.enabled = true;
            let w = cfg.apply_env_overrides();
            assert!(cfg.ui.enabled, "must not silently flip to false");
            assert_eq!(w.len(), 1);
            assert!(w[0].contains("HUGINN_UI_ENABLED"), "got: {w:?}");
        });
    }

    /// An unknown format used to silently become `pretty`.
    #[test]
    fn env_invalid_log_format_warns_and_keeps_previous() {
        with_env(&[("HUGINN_LOG_FORMAT", "xml")], || {
            let mut cfg: AppConfig = serde_yaml_ng::from_str(MINIMAL_YAML).unwrap();
            cfg.log.format = LogFormat::Json;
            let w = cfg.apply_env_overrides();
            assert_eq!(cfg.log.format, LogFormat::Json, "must not silently reset");
            assert_eq!(w.len(), 1);
            assert!(w[0].contains("HUGINN_LOG_FORMAT"), "got: {w:?}");
        });
    }

    #[test]
    fn default_hub_capacity_is_256() {
        let cfg = parse(MINIMAL_YAML);
        assert_eq!(cfg.event_hub_capacity, 256);
    }

    #[test]
    fn default_batch_settings() {
        let cfg = parse(MINIMAL_YAML);
        assert_eq!(cfg.influx.batch_size, 10);
        assert_eq!(cfg.influx.batch_timeout_ms, 1000);
    }

    // --- read_token -----------------------------------------------------------

    #[test]
    fn read_token_fails_for_nonexistent_file() {
        let mut cfg: AppConfig = serde_yaml_ng::from_str(MINIMAL_YAML).unwrap();
        cfg.influx.token_file = "/nonexistent/path/to/huginn-token-xyz.txt".into();
        let result = cfg.influx.read_token();
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("huginn-token-xyz") || msg.contains("token"),
            "unexpected error: {msg}"
        );
    }

    #[test]
    fn read_token_trims_whitespace() {
        use std::io::Write;
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        writeln!(tmp, "  mytoken  ").unwrap();
        let path = tmp.path().to_string_lossy().into_owned();
        let mut cfg: AppConfig = serde_yaml_ng::from_str(MINIMAL_YAML).unwrap();
        cfg.influx.token_file = path;
        let token = cfg.influx.read_token().unwrap();
        assert_eq!(token, "mytoken");
    }

    #[test]
    fn env_token_file_override_applies() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), "envtoken").unwrap();
        let path_str = tmp.path().to_string_lossy().into_owned();

        with_env(&[("INFLUX_TOKEN_FILE", path_str.as_str())], || {
            let mut cfg: AppConfig = serde_yaml_ng::from_str(MINIMAL_YAML).unwrap();
            cfg.apply_env_overrides();
            assert_eq!(cfg.influx.token_file, path_str);
            assert_eq!(cfg.influx.read_token().unwrap(), "envtoken");
        });
    }
}
