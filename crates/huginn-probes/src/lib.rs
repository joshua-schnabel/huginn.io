pub mod dns;
pub mod http;
pub mod imap;
pub mod smtp;
pub mod tcp;
pub mod udp;

use std::future::Future;
use std::time::Duration;

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
