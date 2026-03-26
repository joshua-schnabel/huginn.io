/// TDD: Tests for CLI output formatting.
/// These tests are written FIRST — they define the contract.
/// Run `cargo test` now to see them FAIL, then implement to make them GREEN.
use hugin_core::types::ProbeResult;

// We test the formatting logic by calling the binary with --output json
// and checking stdout. The `print_result` function is private in main.rs,
// so we test via the public JSON serialisation contract of ProbeResult.

#[test]
fn probe_result_json_contains_required_fields() {
    let r = ProbeResult::success("web", "http", "https://example.com", 42.5, Some(200));
    let json = serde_json::to_string(&r).unwrap();

    assert!(json.contains("\"probe_name\""), "missing probe_name");
    assert!(json.contains("\"probe_type\""), "missing probe_type");
    assert!(json.contains("\"target\""), "missing target");
    assert!(json.contains("\"up\""), "missing up");
    assert!(json.contains("\"response_ms\""), "missing response_ms");
    assert!(json.contains("\"timestamp\""), "missing timestamp");
}

#[test]
fn probe_result_up_true_for_success() {
    let r = ProbeResult::success("p", "tcp", "host:80", 1.0, None);
    let json = serde_json::to_value(&r).unwrap();
    assert_eq!(json["up"], true);
}

#[test]
fn probe_result_up_false_for_failure() {
    let r = ProbeResult::failure("p", "tcp", "host:80", 5000.0, "timeout");
    let json = serde_json::to_value(&r).unwrap();
    assert_eq!(json["up"], false);
    assert!(json["error"].as_str().unwrap().contains("timeout"));
}

#[test]
fn probe_result_status_code_present_for_http() {
    let r = ProbeResult::success("web", "http", "https://example.com", 20.0, Some(200));
    let json = serde_json::to_value(&r).unwrap();
    assert_eq!(json["status_code"], 200);
}

#[test]
fn probe_result_status_code_null_for_tcp() {
    let r = ProbeResult::success("db", "tcp", "host:5432", 5.0, None);
    let json = serde_json::to_value(&r).unwrap();
    assert!(json["status_code"].is_null());
}
