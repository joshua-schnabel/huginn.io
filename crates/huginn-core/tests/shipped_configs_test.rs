//! The configs we ship must satisfy the validation we ship.
//!
//! validate() rejects things that would otherwise surface as a startup panic or
//! a probe stuck DOWN forever. That is only worth anything if our own examples
//! pass it — and a stale example is the most likely way for them not to.
use huginn_core::config::AppConfig;

#[test]
fn config_example_yaml_is_valid() {
    AppConfig::load("../../config/config.example.yaml").expect("config.example.yaml rejected");
}

#[test]
fn config_integration_yaml_is_valid() {
    AppConfig::load("../../config/config.integration.yaml")
        .expect("config.integration.yaml rejected");
}
