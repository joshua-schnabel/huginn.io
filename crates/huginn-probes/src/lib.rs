pub mod dns;
pub mod http;
pub mod imap;
pub mod registry;
pub mod smtp;
pub mod tcp;
pub mod udp;

pub use registry::ProbeRegistry;

use async_trait::async_trait;
use huginn_core::config::ProbeConfig;
use huginn_core::types::ProbeResult;
use std::future::Future;
use std::time::Duration;

/// One protocol's liveness check.
///
/// Note the return type: a probe yields a [`ProbeResult`], never a `Result`. A
/// failed probe is *data* — `up: false` with the reason in `error` — not an
/// error condition. That distinction is the whole reason a monitoring tool
/// exists, and implementations must preserve it: report the failure, don't
/// propagate it.
///
/// The trait carries no `cfg` state; per-protocol shared resources (an HTTP
/// client, a TLS verifier, a ping socket) live in the implementing struct and
/// are built once by [`ProbeRegistry`].
#[async_trait]
pub trait Probe: Send + Sync {
    async fn probe(&self, cfg: &ProbeConfig) -> ProbeResult;
}

/// Runs `fut` with a timeout, flattening the `Ok(Ok(v))` / `Ok(Err(e))` / `Err(elapsed)` triple
/// into a simple `Result<T, String>`.  Used by every probe to eliminate the repeated
/// nested-result boilerplate.
pub async fn with_probe_timeout<F, T, E>(
    duration: Duration,
    timeout_msg: &str,
    fut: F,
) -> Result<T, String>
where
    F: Future<Output = Result<T, E>>,
    E: std::fmt::Display,
{
    match tokio::time::timeout(duration, fut).await {
        Ok(Ok(val)) => Ok(val),
        Ok(Err(e)) => Err(e.to_string()),
        Err(_) => Err(timeout_msg.to_string()),
    }
}
