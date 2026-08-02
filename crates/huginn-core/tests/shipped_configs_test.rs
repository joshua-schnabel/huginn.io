//! The configs we ship must satisfy the validation we ship.
//!
//! validate() rejects things that would otherwise surface as a startup panic or
//! a probe stuck DOWN forever. That is only worth anything if our own examples
//! pass it — and a stale example is the most likely way for them not to.
use huginn_core::config::{AppConfig, ProbeType};

#[test]
fn config_example_yaml_is_valid() {
    AppConfig::load("../../config/config.example.yaml").expect("config.example.yaml rejected");
}

/// The example is the de-facto reference for what can be configured — a probe
/// type it doesn't show may as well not exist for a new user. (The shipped
/// example really did lack `type: dns` for a while; this pins the invariant.)
#[test]
fn config_example_yaml_covers_every_probe_type() {
    let cfg =
        AppConfig::load("../../config/config.example.yaml").expect("config.example.yaml rejected");
    let all = [
        ProbeType::Tcp,
        ProbeType::Http,
        ProbeType::Https,
        ProbeType::Smtp,
        ProbeType::Imap,
        ProbeType::Udp,
        ProbeType::Dns,
        ProbeType::Tls,
    ];
    for t in all {
        assert!(
            cfg.probes.iter().any(|p| p.probe_type == t),
            "config.example.yaml has no `type: {t}` example"
        );
    }
}

#[test]
fn config_integration_yaml_is_valid() {
    AppConfig::load("../../config/config.integration.yaml")
        .expect("config.integration.yaml rejected");
}
