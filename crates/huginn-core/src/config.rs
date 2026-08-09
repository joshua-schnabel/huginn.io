use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::Duration;

use crate::error::{HuginError, Result};

// ---------------------------------------------------------------------------
// Top-level config
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppConfig {
    pub influx: InfluxConfig,
    #[serde(default)]
    pub probes: Vec<ProbeConfig>,
    #[serde(default)]
    pub ui: UiConfig,
    #[serde(default)]
    pub metrics: MetricsConfig,
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
#[serde(deny_unknown_fields)]
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
    ///
    /// An empty (or whitespace-only) file is an error, not an empty token.
    /// Without this the process starts happily, sends `Authorization: Token `
    /// with no value, and InfluxDB answers 401 — a 4xx, which the writer
    /// classifies as permanent and drops the batch. The result is a monitor
    /// that looks healthy while discarding every measurement it takes. Same
    /// fail-closed rule as [`MetricsConfig::read_api_key`].
    pub fn read_token(&self) -> Result<String> {
        let token = std::fs::read_to_string(&self.token_file)
            .map(|s| s.trim().to_string())
            .map_err(|e| HuginError::Secret {
                path: self.token_file.clone(),
                message: e.to_string(),
            })?;
        warn_if_readable_by_others(self.token_file.as_str());
        if token.is_empty() {
            return Err(HuginError::Secret {
                path: self.token_file.clone(),
                message: "InfluxDB token file is empty".into(),
            });
        }
        Ok(token)
    }
}

/// Warn when a secret file is readable by anyone but its owner.
///
/// A warning, not a refusal — R5 of `docs/risks.md`. The documentation
/// prescribes `0600` for `influx.token_file` and `metrics.api_key_file` and
/// nothing checked it, so a file left `0644` in an image or a mount said
/// nothing at all. Refusing to start would be worse than the risk: a read-only
/// bind mount can carry permissions the operator does not control, and the
/// token still works.
///
/// The distroless image has no shell and no other processes, so "readable by
/// anything else in the container" is a narrow set here — narrower than in
/// muninn.io, whose runtime carries apt and a shell and where the same finding
/// (M-01) was raised first. Alignment is why this exists in both.
///
/// Unix only: mode bits are the check, and there is nothing equivalent
/// elsewhere. The path is named, never the contents.
#[cfg(unix)]
fn warn_if_readable_by_others(path: &str) {
    use std::os::unix::fs::PermissionsExt as _;

    // Best-effort: the file was just read, so a failing stat says something odd
    // about the filesystem rather than about the secret, and is no reason to
    // hold up the start.
    let Ok(meta) = std::fs::metadata(path) else {
        return;
    };
    let mode = meta.permissions().mode() & 0o777;
    if mode & 0o077 != 0 {
        tracing::warn!(
            path = %path,
            mode = format!("{mode:04o}"),
            "secret file is readable beyond its owner; 0600 is expected"
        );
    }
}

#[cfg(not(unix))]
fn warn_if_readable_by_others(_path: &str) {}

// ---------------------------------------------------------------------------
// Optional debug UI config
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UiConfig {
    #[serde(default)]
    pub enabled: bool,
    /// Address the UI listens on. Defaults to loopback: the UI has no
    /// authentication and publishes every probe target, so reaching a wider
    /// network has to be a deliberate act. Containers need `0.0.0.0` — Docker
    /// port publishing targets the container's bridge IP, not its loopback.
    #[serde(default = "default_ui_bind")]
    pub bind: String,
    #[serde(default = "default_ui_port")]
    pub port: u16,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            bind: default_ui_bind(),
            port: default_ui_port(),
        }
    }
}

fn default_ui_bind() -> String {
    "127.0.0.1".to_string()
}

fn default_ui_port() -> u16 {
    9116
}

// ---------------------------------------------------------------------------
// Optional Prometheus metrics config
// ---------------------------------------------------------------------------

/// Prometheus `/metrics` listener, gated independently of the debug UI so
/// scraping doesn't require exposing the UI (and vice versa).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MetricsConfig {
    #[serde(default)]
    pub enabled: bool,
    /// Address the metrics listener binds. Same reasoning as `ui.bind`:
    /// loopback by default, containers need `0.0.0.0`.
    #[serde(default = "default_metrics_bind")]
    pub bind: String,
    /// Default 9464 — the conventional Prometheus-exporter port used by the
    /// OpenTelemetry Prometheus exporter.
    #[serde(default = "default_metrics_port")]
    pub port: u16,
    /// Optional path to a file containing an API key. When set, `/metrics`
    /// requires `Authorization: Bearer <key>`. The key lives in a **file**,
    /// never in YAML or ENV — same policy as `influx.token_file`.
    #[serde(default)]
    pub api_key_file: Option<String>,
}

impl Default for MetricsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            bind: default_metrics_bind(),
            port: default_metrics_port(),
            api_key_file: None,
        }
    }
}

impl MetricsConfig {
    /// Read the API key from `api_key_file`, if one is configured.
    ///
    /// A configured-but-unreadable or empty file is an error, not `None`: the
    /// operator asked for auth, so silently serving unauthenticated would be
    /// the worst possible fallback. Mirrors [`InfluxConfig::read_token`].
    pub fn read_api_key(&self) -> Result<Option<String>> {
        let Some(path) = &self.api_key_file else {
            return Ok(None);
        };
        let key = std::fs::read_to_string(path)
            .map(|s| s.trim().to_string())
            .map_err(|e| HuginError::Secret {
                path: path.clone(),
                message: e.to_string(),
            })?;
        warn_if_readable_by_others(path.as_str());
        if key.is_empty() {
            return Err(HuginError::Secret {
                path: path.clone(),
                message: "metrics API key file is empty".into(),
            });
        }
        Ok(Some(key))
    }
}

fn default_metrics_bind() -> String {
    "127.0.0.1".to_string()
}

fn default_metrics_port() -> u16 {
    9464
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
#[serde(deny_unknown_fields)]
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
    Tls,
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
            ProbeType::Tls => "tls",
        };
        write!(f, "{s}")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    /// TLS only: report DOWN once the certificate expires in fewer than this
    /// many days. Unset means 0 — DOWN only once the certificate has expired.
    #[serde(default)]
    pub tls_expiry_fail_days: Option<f64>,
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
            tls_expiry_fail_days: None,
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
        if let Ok(v) = std::env::var("HUGINN_UI_BIND") {
            match v.parse::<std::net::IpAddr>() {
                Ok(_) => self.ui.bind = v,
                Err(_) => warnings.push(format!(
                    "HUGINN_UI_BIND='{v}' is not a valid IP address — keeping ui.bind={}",
                    self.ui.bind
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
        if let Ok(v) = std::env::var("HUGINN_METRICS_ENABLED") {
            match v.to_lowercase().as_str() {
                "true" | "1" => self.metrics.enabled = true,
                "false" | "0" => self.metrics.enabled = false,
                _ => warnings.push(format!(
                    "HUGINN_METRICS_ENABLED='{v}' is not a boolean (expected true/false/1/0) — \
                     keeping metrics.enabled={}",
                    self.metrics.enabled
                )),
            }
        }
        if let Ok(v) = std::env::var("HUGINN_METRICS_BIND") {
            match v.parse::<std::net::IpAddr>() {
                Ok(_) => self.metrics.bind = v,
                Err(_) => warnings.push(format!(
                    "HUGINN_METRICS_BIND='{v}' is not a valid IP address — keeping metrics.bind={}",
                    self.metrics.bind
                )),
            }
        }
        if let Ok(v) = std::env::var("HUGINN_METRICS_PORT") {
            match v.parse::<u16>() {
                Ok(p) => self.metrics.port = p,
                Err(_) => warnings.push(format!(
                    "HUGINN_METRICS_PORT='{v}' is not a valid port — keeping metrics.port={}",
                    self.metrics.port
                )),
            }
        }
        // A path to the key file, never the key itself (see read_api_key).
        if let Ok(v) = std::env::var("HUGINN_METRICS_API_KEY_FILE") {
            self.metrics.api_key_file = Some(v);
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
        // The URL is only ever used to build the write endpoint, and reqwest
        // does not parse it until the first batch is ready to go — so a typo
        // like `localhost:8086` with no scheme surfaced as a transport error on
        // every write, minutes in, looking exactly like an unreachable server.
        if !(self.influx.url.starts_with("http://") || self.influx.url.starts_with("https://")) {
            return Err(HuginError::Config(format!(
                "influx.url '{}' must be an absolute URL starting with http:// or https://",
                self.influx.url
            )));
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
        // A zero timeout makes the batcher's flush timer `sleep(Duration::ZERO)`,
        // which resolves immediately every loop — the select! spins that arm at
        // 100% CPU forever.
        if self.influx.batch_timeout_ms == 0 {
            return Err(HuginError::Config(
                "influx.batch_timeout_ms must be > 0 (0 spins the batcher's flush timer at 100% CPU)"
                    .into(),
            ));
        }
        // 0 makes the retry queue evict on every push, so it holds at most one
        // batch and drops essentially everything during an outage — the exact
        // opposite of what the buffer is for.
        if self.influx.max_buffered_bytes == 0 {
            return Err(HuginError::Config(
                "influx.max_buffered_bytes must be > 0 (0 makes the retry queue drop every batch during an outage)"
                    .into(),
            ));
        }
        // A zero initial backoff makes the writer retry in a tight loop against a
        // server that is already struggling; a zero ceiling caps every wait at
        // zero and does the same thing however large the initial value is.
        if self.influx.retry_initial_backoff_ms == 0 {
            return Err(HuginError::Config(
                "influx.retry_initial_backoff_ms must be > 0 (0 retries in a tight loop against a failing server)"
                    .into(),
            ));
        }
        if self.influx.retry_max_backoff_ms == 0 {
            return Err(HuginError::Config(
                "influx.retry_max_backoff_ms must be > 0 (0 caps every retry wait at zero)".into(),
            ));
        }
        // The ceiling below the first delay is not a smaller ceiling, it is a
        // contradiction: the backoff starts above its own maximum, so the value
        // an operator wrote as "wait at most this long" never applies.
        if self.influx.retry_max_backoff_ms < self.influx.retry_initial_backoff_ms {
            return Err(HuginError::Config(format!(
                "influx.retry_max_backoff_ms ({}) is below influx.retry_initial_backoff_ms ({}) — \
                 the ceiling must not be lower than the first delay",
                self.influx.retry_max_backoff_ms, self.influx.retry_initial_backoff_ms
            )));
        }
        // tokio::sync::broadcast::channel panics on a capacity of 0, so without
        // this the process dies at startup with no hint about which key is wrong.
        if self.event_hub_capacity == 0 {
            return Err(HuginError::Config("event_hub_capacity must be > 0".into()));
        }
        // Caught here rather than at bind time: each listener is spawned into
        // its own task, so a parse failure there would only surface as a logged
        // error while the daemon keeps running without the service.
        //
        // The parsed values are kept, because the collision check below has to
        // compare addresses rather than the strings they were written as: `::1`
        // and `0:0:0:0:0:0:0:1` are the same socket and were not the same
        // `String`, so two listeners could be configured onto one address and
        // the check would wave them through.
        let ui_addr = self.ui.bind.parse::<std::net::IpAddr>().map_err(|_| {
            HuginError::Config(format!(
                "ui.bind '{}' must be an IP address, e.g. '127.0.0.1', '0.0.0.0' or '::1'",
                self.ui.bind
            ))
        })?;
        let metrics_addr = self.metrics.bind.parse::<std::net::IpAddr>().map_err(|_| {
            HuginError::Config(format!(
                "metrics.bind '{}' must be an IP address, e.g. '127.0.0.1', '0.0.0.0' or '::1'",
                self.metrics.bind
            ))
        })?;
        // Port 0 asks the OS for an arbitrary free port. For a listener whose
        // whole purpose is to be connected to by something else — a browser, a
        // Prometheus scrape config — that produces a service on an address
        // nobody can predict, and a config that appears to work while nothing
        // can reach it.
        for (name, enabled, port) in [
            ("ui", self.ui.enabled, self.ui.port),
            ("metrics", self.metrics.enabled, self.metrics.port),
        ] {
            if enabled && port == 0 {
                return Err(HuginError::Config(format!(
                    "{name}.port must not be 0 — port 0 asks the OS for an arbitrary free port, \
                     so nothing could be configured to reach it"
                )));
            }
        }
        // An empty path is a config bug — the operator meant to set a file.
        if let Some(path) = &self.metrics.api_key_file {
            if path.is_empty() {
                return Err(HuginError::Config(
                    "metrics.api_key_file must not be empty (omit the key to disable auth)".into(),
                ));
            }
        }
        // Two listeners on one address can't both bind; the second would lose
        // at runtime with only a logged error. Compared as parsed addresses —
        // see above for why the string comparison this replaced was not enough.
        if self.ui.enabled
            && self.metrics.enabled
            && ui_addr == metrics_addr
            && self.ui.port == self.metrics.port
        {
            return Err(HuginError::Config(format!(
                "ui and metrics are both enabled on {}:{} — give metrics its own port (metrics.port)",
                self.ui.bind, self.ui.port
            )));
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
            // A negative threshold would mean "stay UP for a while after the
            // certificate has expired" — surely a sign error, never intent.
            //
            // The finiteness check is not pedantry: YAML accepts `.nan` and
            // `.inf`, and `NaN < 0.0` is false, so a NaN threshold passed this
            // check and then made every later comparison false too — the probe
            // reported UP whatever the certificate said, which is the one
            // outcome a certificate-expiry probe must never produce silently.
            // `.inf` passes for the opposite reason and reports DOWN forever.
            if let Some(days) = probe.tls_expiry_fail_days {
                if !days.is_finite() {
                    return Err(HuginError::Config(format!(
                        "probe '{}': tls_expiry_fail_days must be a finite number (got {days}) — \
                         NaN makes every expiry comparison false, so the probe would report UP \
                         however close the certificate is to expiring",
                        probe.name
                    )));
                }
                if days < 0.0 {
                    return Err(HuginError::Config(format!(
                        "probe '{}': tls_expiry_fail_days must be >= 0 (omit it to fail only once the certificate has expired)",
                        probe.name
                    )));
                }
            }
            // A status code outside the HTTP range can never be returned, so the
            // probe would report DOWN on every tick for ever — the exact failure
            // shape this whole function exists to prevent.
            if let Some(status) = probe.expected_status {
                if !(100..=599).contains(&status) {
                    return Err(HuginError::Config(format!(
                        "probe '{}': expected_status {status} is not a valid HTTP status code \
                         (100-599), so the probe could never match it",
                        probe.name
                    )));
                }
            }
            // Compared against the resolved answer as an IpAddr by the probe, so
            // an unparseable value never matches: the probe reports DOWN on every
            // tick and the resolver was working perfectly.
            if let Some(ip) = &probe.dns_expected_ip {
                if ip.parse::<std::net::IpAddr>().is_err() {
                    return Err(HuginError::Config(format!(
                        "probe '{}': dns_expected_ip '{ip}' is not an IP address — it is compared \
                         against the resolved address, so nothing would ever match it",
                        probe.name
                    )));
                }
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
            // Handed to TcpStream/UdpSocket connect, or (TLS) to reqwest as
            // https://host:port — all need host:port.
            ProbeType::Tcp
            | ProbeType::Smtp
            | ProbeType::Imap
            | ProbeType::Udp
            | ProbeType::Tls => {
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
        ProbeType::Tls => 443,
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
  - name: "cert"
    type: tls
    target: "example.com:443"
    tls_expiry_fail_days: 14
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
        assert_eq!(cfg.probes.len(), 7);
        assert_eq!(cfg.probes[0].probe_type, ProbeType::Http);
        assert_eq!(cfg.probes[0].expected_status, Some(200));
        assert_eq!(cfg.probes[1].probe_type, ProbeType::Tcp);
        assert_eq!(cfg.probes[2].probe_type, ProbeType::Udp);
        assert_eq!(cfg.probes[3].probe_type, ProbeType::Smtp);
        assert_eq!(cfg.probes[4].probe_type, ProbeType::Imap);
        assert_eq!(cfg.probes[5].probe_type, ProbeType::Dns);
        assert_eq!(cfg.probes[5].dns_query, Some("example.com".into()));
        assert_eq!(cfg.probes[5].dns_expected_ip, Some("93.184.216.34".into()));
        assert_eq!(cfg.probes[6].probe_type, ProbeType::Tls);
        assert_eq!(cfg.probes[6].tls_expiry_fail_days, Some(14.0));
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
        // Loopback by default — exposing an unauthenticated UI must be an
        // explicit act, never something an omitted key does for you.
        assert_eq!(cfg.ui.bind, "127.0.0.1");
    }

    #[test]
    fn metrics_disabled_on_loopback_9464_by_default() {
        let cfg = parse(MINIMAL_YAML);
        assert!(!cfg.metrics.enabled);
        assert_eq!(cfg.metrics.bind, "127.0.0.1");
        assert_eq!(cfg.metrics.port, 9464);
    }

    /// Same invariant as for `UiConfig`: a config without a `metrics:` block
    /// must behave like one with an empty block.
    #[test]
    fn metrics_default_impl_matches_serde_default() {
        let from_serde = parse(MINIMAL_YAML).metrics;
        let from_default = MetricsConfig::default();
        assert_eq!(from_serde.enabled, from_default.enabled);
        assert_eq!(from_serde.bind, from_default.bind);
        assert_eq!(from_serde.port, from_default.port);
    }

    #[test]
    fn validation_rejects_non_ip_metrics_bind() {
        let yaml = format!("{MINIMAL_YAML}\nmetrics:\n  bind: \"localhost\"\n");
        let cfg: AppConfig = serde_yaml_ng::from_str(&yaml).expect("parse failed");
        let err = cfg.validate().expect_err("hostname bind must be rejected");
        assert!(
            err.to_string().contains("metrics.bind"),
            "error must name the key: {err}"
        );
    }

    #[test]
    fn validation_rejects_ui_and_metrics_on_same_address() {
        let yaml = format!(
            "{MINIMAL_YAML}\nui:\n  enabled: true\n  port: 9464\nmetrics:\n  enabled: true\n  port: 9464\n"
        );
        let cfg: AppConfig = serde_yaml_ng::from_str(&yaml).expect("parse failed");
        let err = cfg
            .validate()
            .expect_err("shared bind:port must be rejected");
        assert!(
            err.to_string().contains("metrics.port"),
            "error must point at metrics.port: {err}"
        );
    }

    #[test]
    fn ui_and_metrics_may_share_a_port_when_only_one_is_enabled() {
        let yaml = format!("{MINIMAL_YAML}\nui:\n  port: 9464\nmetrics:\n  enabled: true\n");
        let cfg: AppConfig = serde_yaml_ng::from_str(&yaml).expect("parse failed");
        cfg.validate()
            .expect("a disabled listener must not count as a collision");
    }

    #[test]
    fn metrics_api_key_is_none_when_no_file_is_configured() {
        let cfg = parse(MINIMAL_YAML);
        assert_eq!(cfg.metrics.read_api_key().unwrap(), None);
    }

    #[test]
    fn metrics_api_key_is_read_and_trimmed_from_the_file() {
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), "  sekrit-key \n").unwrap();
        let cfg = MetricsConfig {
            api_key_file: Some(file.path().to_string_lossy().into_owned()),
            ..Default::default()
        };
        assert_eq!(cfg.read_api_key().unwrap(), Some("sekrit-key".to_string()));
    }

    #[test]
    fn metrics_api_key_missing_file_is_an_error_not_open_access() {
        let cfg = MetricsConfig {
            api_key_file: Some("/nonexistent/huginn-metrics-key".into()),
            ..Default::default()
        };
        assert!(cfg.read_api_key().is_err());
    }

    #[test]
    fn metrics_api_key_empty_file_is_an_error_not_open_access() {
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), "  \n").unwrap();
        let cfg = MetricsConfig {
            api_key_file: Some(file.path().to_string_lossy().into_owned()),
            ..Default::default()
        };
        assert!(cfg.read_api_key().is_err());
    }

    #[test]
    fn validation_rejects_empty_metrics_api_key_file_path() {
        let yaml = format!("{MINIMAL_YAML}\nmetrics:\n  api_key_file: \"\"\n");
        let cfg: AppConfig = serde_yaml_ng::from_str(&yaml).expect("parse failed");
        let err = cfg.validate().expect_err("empty path must be rejected");
        assert!(
            err.to_string().contains("api_key_file"),
            "error must name the key: {err}"
        );
    }

    /// `UiConfig::default()` must agree with the serde defaults, or a config
    /// without a `ui:` block would behave differently from one with an empty one.
    #[test]
    fn ui_default_impl_matches_serde_default() {
        let from_serde = parse(MINIMAL_YAML).ui;
        let from_default = UiConfig::default();
        assert_eq!(from_serde.enabled, from_default.enabled);
        assert_eq!(from_serde.bind, from_default.bind);
        assert_eq!(from_serde.port, from_default.port);
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
        assert_eq!(ProbeType::Tls.to_string(), "tls");
    }

    #[test]
    fn validation_rejects_negative_tls_expiry_fail_days() {
        let yaml = r#"
influx:
  url: "http://localhost:8086"
  org: "o"
  bucket: "b"
  token_file: "/tmp/t"
probes:
  - name: "cert"
    type: tls
    target: "example.com:443"
    tls_expiry_fail_days: -7
"#;
        let cfg: AppConfig = serde_yaml_ng::from_str(yaml).expect("parse failed");
        let err = cfg
            .validate()
            .expect_err("negative threshold must be rejected");
        assert!(
            err.to_string().contains("tls_expiry_fail_days"),
            "error must name the key: {err}"
        );
    }

    // -----------------------------------------------------------------------
    // Strict loading: unknown keys, and values that could never work
    // -----------------------------------------------------------------------

    /// A misspelled key used to be swallowed in silence, which is the worst
    /// possible outcome for a config: the operator believes they set something,
    /// the default applies, and nothing anywhere says otherwise. `batch_sizes`
    /// here is the shape of a real typo.
    #[test]
    fn loading_rejects_an_unknown_key() {
        let yaml = r#"
influx:
  url: "http://localhost:8086"
  org: "o"
  bucket: "b"
  token_file: "/tmp/t"
  batch_sizes: 10
"#;
        let err = serde_yaml_ng::from_str::<AppConfig>(yaml)
            .expect_err("an unknown key must not be accepted");
        assert!(
            err.to_string().contains("batch_sizes"),
            "the error must name the offending key: {err}"
        );
    }

    /// Same rule one level up, where a mistyped *section* would otherwise mean
    /// the whole block is ignored — `metric:` instead of `metrics:` silently
    /// disables the endpoint it was meant to configure.
    #[test]
    fn loading_rejects_an_unknown_section() {
        let yaml = r#"
influx:
  url: "http://localhost:8086"
  org: "o"
  bucket: "b"
  token_file: "/tmp/t"
metric:
  enabled: true
"#;
        assert!(
            serde_yaml_ng::from_str::<AppConfig>(yaml).is_err(),
            "an unknown section must not be accepted"
        );
    }

    /// A scheme-less InfluxDB URL parses fine as a string and fails only when
    /// the first batch is written, minutes later, looking like an unreachable
    /// server.
    #[test]
    fn validation_rejects_influx_url_without_a_scheme() {
        let yaml = r#"
influx:
  url: "localhost:8086"
  org: "o"
  bucket: "b"
  token_file: "/tmp/t"
"#;
        let cfg: AppConfig = serde_yaml_ng::from_str(yaml).unwrap();
        let err = cfg
            .validate()
            .expect_err("a scheme-less URL must be rejected");
        assert!(err.to_string().contains("influx.url"), "got: {err}");
    }

    /// The regression this check exists for: `NaN < 0.0` is false, so a NaN
    /// threshold passed the sign check and then made every expiry comparison
    /// false as well — the probe reported UP whatever the certificate said.
    #[test]
    fn validation_rejects_non_finite_tls_expiry_fail_days() {
        for value in [".nan", ".inf"] {
            let yaml = format!(
                r#"
influx:
  url: "http://localhost:8086"
  org: "o"
  bucket: "b"
  token_file: "/tmp/t"
probes:
  - name: "cert"
    type: tls
    target: "example.com:443"
    tls_expiry_fail_days: {value}
"#
            );
            let cfg: AppConfig = serde_yaml_ng::from_str(&yaml).expect("parse failed");
            let err = cfg
                .validate()
                .expect_err(&format!("{value} must be rejected"));
            assert!(
                err.to_string().contains("tls_expiry_fail_days"),
                "error must name the key for {value}: {err}"
            );
        }
    }

    #[test]
    fn validation_rejects_an_impossible_expected_status() {
        let yaml = r#"
influx:
  url: "http://localhost:8086"
  org: "o"
  bucket: "b"
  token_file: "/tmp/t"
probes:
  - name: "web"
    type: http
    target: "https://example.com"
    expected_status: 1000
"#;
        let cfg: AppConfig = serde_yaml_ng::from_str(yaml).unwrap();
        let err = cfg
            .validate()
            .expect_err("a status outside 100-599 must be rejected");
        assert!(err.to_string().contains("expected_status"), "got: {err}");
    }

    #[test]
    fn validation_rejects_an_unparseable_dns_expected_ip() {
        let yaml = r#"
influx:
  url: "http://localhost:8086"
  org: "o"
  bucket: "b"
  token_file: "/tmp/t"
probes:
  - name: "dns"
    type: dns
    target: "1.1.1.1:53"
    dns_expected_ip: "example.com"
"#;
        let cfg: AppConfig = serde_yaml_ng::from_str(yaml).unwrap();
        let err = cfg
            .validate()
            .expect_err("a hostname is not an IP address and could never match");
        assert!(err.to_string().contains("dns_expected_ip"), "got: {err}");
    }

    #[test]
    fn validation_rejects_a_retry_ceiling_below_the_first_delay() {
        let yaml = r#"
influx:
  url: "http://localhost:8086"
  org: "o"
  bucket: "b"
  token_file: "/tmp/t"
  retry_initial_backoff_ms: 5000
  retry_max_backoff_ms: 100
"#;
        let cfg: AppConfig = serde_yaml_ng::from_str(yaml).unwrap();
        let err = cfg
            .validate()
            .expect_err("a ceiling below the first delay must be rejected");
        assert!(
            err.to_string().contains("retry_max_backoff_ms"),
            "got: {err}"
        );
    }

    #[test]
    fn validation_rejects_zero_retry_backoffs() {
        for key in ["retry_initial_backoff_ms", "retry_max_backoff_ms"] {
            let yaml = format!(
                r#"
influx:
  url: "http://localhost:8086"
  org: "o"
  bucket: "b"
  token_file: "/tmp/t"
  {key}: 0
"#
            );
            let cfg: AppConfig = serde_yaml_ng::from_str(&yaml).unwrap();
            assert!(cfg.validate().is_err(), "{key}: 0 must be rejected");
        }
    }

    /// Port 0 asks the OS for an arbitrary free port — a listener nothing can be
    /// configured to reach.
    #[test]
    fn validation_rejects_port_zero_on_an_enabled_listener() {
        let yaml = r#"
influx:
  url: "http://localhost:8086"
  org: "o"
  bucket: "b"
  token_file: "/tmp/t"
ui:
  enabled: true
  port: 0
"#;
        let cfg: AppConfig = serde_yaml_ng::from_str(yaml).unwrap();
        let err = cfg.validate().expect_err("port 0 must be rejected");
        assert!(err.to_string().contains("ui.port"), "got: {err}");
    }

    /// The collision check compares parsed addresses, so two spellings of the
    /// same IPv6 address are caught. As raw strings they differ, and both
    /// listeners were allowed onto one socket.
    #[test]
    fn validation_rejects_equivalent_ipv6_addresses_on_one_port() {
        let yaml = r#"
influx:
  url: "http://localhost:8086"
  org: "o"
  bucket: "b"
  token_file: "/tmp/t"
ui:
  enabled: true
  bind: "::1"
  port: 9500
metrics:
  enabled: true
  bind: "0:0:0:0:0:0:0:1"
  port: 9500
"#;
        let cfg: AppConfig = serde_yaml_ng::from_str(yaml).unwrap();
        let err = cfg
            .validate()
            .expect_err("the same address written two ways is still one socket");
        assert!(err.to_string().contains("metrics.port"), "got: {err}");
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

    /// A bad `ui.bind` must fail the load, not the spawned UI task — there it
    /// would only be a log line while the daemon runs on without a UI.
    #[test]
    fn validation_rejects_invalid_ui_bind() {
        let mut cfg: AppConfig = serde_yaml_ng::from_str(MINIMAL_YAML).unwrap();
        cfg.ui.bind = "localhost".into(); // a hostname is not an IP address
        let err = cfg.validate().unwrap_err().to_string();
        assert!(err.contains("ui.bind"), "got: {err}");
    }

    #[test]
    fn validation_accepts_wildcard_and_ipv6_ui_bind() {
        for addr in ["0.0.0.0", "::1", "::"] {
            let mut cfg: AppConfig = serde_yaml_ng::from_str(MINIMAL_YAML).unwrap();
            cfg.ui.bind = addr.into();
            assert!(cfg.validate().is_ok(), "{addr} should be accepted");
        }
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
    fn validation_rejects_zero_batch_timeout() {
        let mut cfg = cfg_with_probes("  []\n");
        cfg.influx.batch_timeout_ms = 0;
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn validation_rejects_zero_max_buffered_bytes() {
        let mut cfg = cfg_with_probes("  []\n");
        cfg.influx.max_buffered_bytes = 0;
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

    #[test]
    fn env_override_metrics_enabled_bind_and_port() {
        with_env(
            &[
                ("HUGINN_METRICS_ENABLED", "true"),
                ("HUGINN_METRICS_BIND", "0.0.0.0"),
                ("HUGINN_METRICS_PORT", "9999"),
            ],
            || {
                let mut cfg: AppConfig = serde_yaml_ng::from_str(MINIMAL_YAML).unwrap();
                let w = cfg.apply_env_overrides();
                assert!(cfg.metrics.enabled);
                assert_eq!(cfg.metrics.bind, "0.0.0.0");
                assert_eq!(cfg.metrics.port, 9999);
                assert!(w.is_empty(), "all values valid, should not warn: {w:?}");
            },
        );
    }

    #[test]
    fn env_override_metrics_api_key_file_sets_the_path() {
        with_env(
            &[(
                "HUGINN_METRICS_API_KEY_FILE",
                "/run/secrets/metrics_api_key",
            )],
            || {
                let mut cfg: AppConfig = serde_yaml_ng::from_str(MINIMAL_YAML).unwrap();
                let w = cfg.apply_env_overrides();
                assert_eq!(
                    cfg.metrics.api_key_file.as_deref(),
                    Some("/run/secrets/metrics_api_key")
                );
                assert!(w.is_empty(), "a path is always accepted: {w:?}");
            },
        );
    }

    #[test]
    fn env_invalid_metrics_values_warn_and_keep_previous() {
        with_env(
            &[
                ("HUGINN_METRICS_ENABLED", "yes"),
                ("HUGINN_METRICS_BIND", "not-an-ip"),
                ("HUGINN_METRICS_PORT", "-1"),
            ],
            || {
                let mut cfg: AppConfig = serde_yaml_ng::from_str(MINIMAL_YAML).unwrap();
                let w = cfg.apply_env_overrides();
                assert!(!cfg.metrics.enabled);
                assert_eq!(cfg.metrics.bind, "127.0.0.1");
                assert_eq!(cfg.metrics.port, 9464);
                assert_eq!(w.len(), 3, "each bad value must warn: {w:?}");
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

    #[test]
    fn env_override_ui_bind() {
        with_env(&[("HUGINN_UI_BIND", "0.0.0.0")], || {
            let mut cfg: AppConfig = serde_yaml_ng::from_str(MINIMAL_YAML).unwrap();
            let w = cfg.apply_env_overrides();
            assert_eq!(cfg.ui.bind, "0.0.0.0");
            assert!(w.is_empty(), "valid address should not warn: {w:?}");
        });
    }

    /// A typo must not silently widen the bind address — nor silently narrow it.
    #[test]
    fn env_invalid_ui_bind_warns_and_keeps_previous() {
        with_env(&[("HUGINN_UI_BIND", "0.0.0.0.0")], || {
            let mut cfg: AppConfig = serde_yaml_ng::from_str(MINIMAL_YAML).unwrap();
            let w = cfg.apply_env_overrides();
            assert_eq!(cfg.ui.bind, "127.0.0.1", "should keep the previous value");
            assert_eq!(w.len(), 1);
            assert!(w[0].contains("HUGINN_UI_BIND"), "got: {w:?}");
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

    /// Audit F-05: an empty token file used to start the process, which then
    /// discarded every batch on InfluxDB's 401 — a monitor that looks healthy
    /// while silently losing all of its data. Fail closed instead.
    #[test]
    fn read_token_rejects_an_empty_file() {
        for content in ["", "   \n\t "] {
            use std::io::Write;
            let mut tmp = tempfile::NamedTempFile::new().unwrap();
            write!(tmp, "{content}").unwrap();
            let mut cfg: AppConfig = serde_yaml_ng::from_str(MINIMAL_YAML).unwrap();
            cfg.influx.token_file = tmp.path().to_string_lossy().into_owned();

            let err = cfg
                .influx
                .read_token()
                .expect_err("an empty token file must not yield an empty token");
            assert!(
                err.to_string().contains("empty"),
                "the error must name the cause: {err}"
            );
        }
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

    /// A loose mode is warned about, never fatal.
    ///
    /// The assertion is the behaviour: refusing would take down a deployment
    /// whose token works, which is the wrong trade for a mode bit (R5).
    #[cfg(unix)]
    #[test]
    fn a_world_readable_token_file_still_loads() {
        use std::io::Write as _;
        use std::os::unix::fs::PermissionsExt as _;

        let mut f = tempfile::NamedTempFile::new().unwrap();
        writeln!(f, "tok").unwrap();
        std::fs::set_permissions(f.path(), std::fs::Permissions::from_mode(0o644)).unwrap();

        let cfg = InfluxConfig {
            token_file: f.path().display().to_string(),
            ..Default::default()
        };
        assert_eq!(
            cfg.read_token().expect("a loose mode must not be fatal"),
            "tok"
        );
    }

    /// And a correctly-moded one is unaffected.
    #[cfg(unix)]
    #[test]
    fn a_private_token_file_loads() {
        use std::io::Write as _;
        use std::os::unix::fs::PermissionsExt as _;

        let mut f = tempfile::NamedTempFile::new().unwrap();
        writeln!(f, "tok").unwrap();
        std::fs::set_permissions(f.path(), std::fs::Permissions::from_mode(0o600)).unwrap();

        let cfg = InfluxConfig {
            token_file: f.path().display().to_string(),
            ..Default::default()
        };
        assert_eq!(cfg.read_token().unwrap(), "tok");
    }
}
