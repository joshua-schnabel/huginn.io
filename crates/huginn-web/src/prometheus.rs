//! Prometheus exposition endpoint (`GET /metrics`).
//!
//! Hand-rolled text format (version 0.0.4) rather than a client-library crate:
//! the whole surface is a handful of gauges over [`WebState`]'s latest-result
//! map, and adding a dependency needs approval (AGENTS.md §3). Only the latest
//! sample per probe exists, so everything is a gauge — no counters, no
//! histograms.

use std::collections::{BTreeMap, HashMap};
use std::net::IpAddr;
use std::sync::Arc;

use anyhow::Context;
use axum::extract::State;
use axum::routing::get;
use axum::Router;
use huginn_core::types::ProbeResult;
use tracing::info;

use crate::state::WebState;

/// Content type of the Prometheus text exposition format.
pub const CONTENT_TYPE: &str = "text/plain; version=0.0.4; charset=utf-8";

/// Start the metrics listener on `bind`:`port`, serving only `GET /metrics`.
///
/// Separate from the debug-UI server on purpose: scraping must not require
/// exposing the UI. `bind` must parse as an [`IpAddr`]; `AppConfig::validate`
/// rejects anything else before startup.
pub async fn run_metrics_server(bind: &str, port: u16, state: Arc<WebState>) -> anyhow::Result<()> {
    let addr: IpAddr = bind
        .parse()
        .with_context(|| format!("metrics.bind '{bind}' is not a valid IP address"))?;

    let app = Router::new()
        .route("/metrics", get(handle_prometheus))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind((addr, port)).await?;
    info!("Prometheus metrics listening on http://{addr}:{port}/metrics");
    axum::serve(listener, app).await?;
    Ok(())
}

async fn handle_prometheus(
    State(state): State<Arc<WebState>>,
) -> ([(&'static str, &'static str); 1], String) {
    let guard = state.results.read().await;
    ([("content-type", CONTENT_TYPE)], render(&guard))
}

/// Render the latest results as Prometheus text format.
///
/// Output is deterministic: probes are sorted by name, and each metric family
/// appears exactly once with its `# HELP`/`# TYPE` header (Prometheus rejects
/// duplicate `# TYPE` lines). The per-probe `metrics` map is passed through as
/// `huginn_probe_<key>` families.
pub fn render(results: &HashMap<String, ProbeResult>) -> String {
    let mut probes: Vec<&ProbeResult> = results.values().collect();
    probes.sort_by(|a, b| a.probe_name.cmp(&b.probe_name));

    let mut out = String::new();

    render_family(
        &mut out,
        "huginn_probe_success",
        "Whether the last probe run succeeded (1) or failed (0).",
        probes.iter().map(|r| (*r, if r.up { 1.0 } else { 0.0 })),
    );
    render_family(
        &mut out,
        "huginn_probe_duration_seconds",
        "Duration of the last probe run in seconds.",
        probes.iter().map(|r| (*r, r.response_ms / 1000.0)),
    );
    render_family(
        &mut out,
        "huginn_probe_http_status_code",
        "HTTP status code of the last probe run (HTTP probes only).",
        probes
            .iter()
            .filter_map(|r| r.status_code.map(|c| (*r, f64::from(c)))),
    );
    render_family(
        &mut out,
        "huginn_probe_last_run_timestamp_seconds",
        "Unix timestamp of the last probe run.",
        probes.iter().map(|r| (*r, r.timestamp.timestamp() as f64)),
    );

    // Probe-specific readings (e.g. tls_cert_expiry_days), grouped by key so
    // every family still gets exactly one header.
    let mut extra: BTreeMap<String, Vec<(&ProbeResult, f64)>> = BTreeMap::new();
    for r in &probes {
        for (key, value) in &r.metrics {
            extra
                .entry(sanitize_metric_name(key))
                .or_default()
                .push((r, *value));
        }
    }
    for (key, samples) in extra {
        let name = format!("huginn_probe_{key}");
        let help = format!("Probe-specific reading '{key}'.");
        render_family(&mut out, &name, &help, samples.into_iter());
    }

    out
}

/// Append one gauge family: header once, then one sample line per probe.
/// Families without samples are omitted entirely.
fn render_family<'a>(
    out: &mut String,
    name: &str,
    help: &str,
    samples: impl Iterator<Item = (&'a ProbeResult, f64)>,
) {
    let mut lines = String::new();
    for (r, value) in samples {
        lines.push_str(&format!(
            "{name}{{probe=\"{}\",type=\"{}\",target=\"{}\"}} {value}\n",
            escape_label(&r.probe_name),
            escape_label(&r.probe_type),
            escape_label(&r.target),
        ));
    }
    if !lines.is_empty() {
        out.push_str(&format!("# HELP {name} {help}\n# TYPE {name} gauge\n"));
        out.push_str(&lines);
    }
}

/// Escape a label value per the exposition format: backslash, double quote and
/// newline must be escaped; everything else passes through.
fn escape_label(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '\n' => escaped.push_str("\\n"),
            _ => escaped.push(c),
        }
    }
    escaped
}

/// Metric names must match `[a-zA-Z_:][a-zA-Z0-9_:]*`. The keys come from
/// `metric_keys` constants today, but a probe could grow a dynamic key —
/// mapping anything else to `_` keeps the output parseable no matter what.
fn sanitize_metric_name(key: &str) -> String {
    key.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == ':' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use huginn_core::types::metric_keys::TLS_CERT_EXPIRY_DAYS;
    use tokio::sync::RwLock;

    fn map_of(results: Vec<ProbeResult>) -> HashMap<String, ProbeResult> {
        results
            .into_iter()
            .map(|r| (r.probe_name.clone(), r))
            .collect()
    }

    #[test]
    fn empty_state_renders_empty_output() {
        assert_eq!(render(&HashMap::new()), "");
    }

    #[test]
    fn success_renders_one_for_probe_success() {
        let out = render(&map_of(vec![ProbeResult::success(
            "web",
            "http",
            "https://example.com",
            42.5,
            Some(200),
        )]));
        assert!(
            out.contains("huginn_probe_success{probe=\"web\",type=\"http\",target=\"https://example.com\"} 1\n"),
            "got:\n{out}"
        );
        assert!(
            out.contains("huginn_probe_duration_seconds{probe=\"web\",type=\"http\",target=\"https://example.com\"} 0.0425\n"),
            "response_ms must be converted to seconds, got:\n{out}"
        );
        assert!(
            out.contains("huginn_probe_http_status_code{probe=\"web\",type=\"http\",target=\"https://example.com\"} 200\n"),
            "got:\n{out}"
        );
    }

    #[test]
    fn failure_renders_zero_and_omits_status_code() {
        let out = render(&map_of(vec![ProbeResult::failure(
            "db",
            "tcp",
            "host:5432",
            5000.0,
            "connection refused",
        )]));
        assert!(
            out.contains(
                "huginn_probe_success{probe=\"db\",type=\"tcp\",target=\"host:5432\"} 0\n"
            ),
            "got:\n{out}"
        );
        assert!(
            !out.contains("huginn_probe_http_status_code"),
            "a probe without a status code must not emit the family, got:\n{out}"
        );
    }

    #[test]
    fn each_family_header_appears_exactly_once_for_multiple_probes() {
        let out = render(&map_of(vec![
            ProbeResult::success("a", "tcp", "a:1", 1.0, None),
            ProbeResult::success("b", "tcp", "b:2", 2.0, None),
        ]));
        assert_eq!(
            out.matches("# TYPE huginn_probe_success gauge").count(),
            1,
            "duplicate TYPE headers are invalid, got:\n{out}"
        );
        let type_line = out.find("# TYPE huginn_probe_success gauge").unwrap();
        let a_line = out.find("huginn_probe_success{probe=\"a\"").unwrap();
        let b_line = out.find("huginn_probe_success{probe=\"b\"").unwrap();
        assert!(
            type_line < a_line && a_line < b_line,
            "sorted samples after header:\n{out}"
        );
    }

    #[test]
    fn probe_metrics_map_is_passed_through() {
        let out = render(&map_of(vec![ProbeResult::success(
            "cert",
            "tls",
            "example.com:443",
            10.0,
            None,
        )
        .with_metric(TLS_CERT_EXPIRY_DAYS, -4.5)]));
        assert!(
            out.contains(
                "huginn_probe_tls_cert_expiry_days{probe=\"cert\",type=\"tls\",target=\"example.com:443\"} -4.5\n"
            ),
            "got:\n{out}"
        );
    }

    #[test]
    fn label_values_are_escaped() {
        assert_eq!(escape_label(r#"a\b"c"#), r#"a\\b\"c"#);
        assert_eq!(escape_label("line1\nline2"), "line1\\nline2");
        let out = render(&map_of(vec![ProbeResult::success(
            "quo\"te", "tcp", "t:1", 1.0, None,
        )]));
        assert!(out.contains("probe=\"quo\\\"te\""), "got:\n{out}");
    }

    #[test]
    fn dynamic_metric_keys_are_sanitized() {
        assert_eq!(sanitize_metric_name("ok_key:x1"), "ok_key:x1");
        assert_eq!(sanitize_metric_name("bad key-ä"), "bad_key__");
    }

    #[tokio::test]
    async fn handler_sets_prometheus_content_type() {
        let (sse_tx, _) = tokio::sync::broadcast::channel(1);
        let state = Arc::new(WebState {
            results: Arc::new(RwLock::new(map_of(vec![ProbeResult::success(
                "web", "http", "t", 1.0, None,
            )]))),
            sse_tx,
        });
        let (headers, body) = handle_prometheus(State(state)).await;
        assert_eq!(headers[0], ("content-type", CONTENT_TYPE));
        assert!(body.contains("huginn_probe_success"));
    }
}
