//! The liveness listener: `GET /health`, and nothing else.
//!
//! Separate from the debug UI's identically-named route on purpose. The UI is
//! off by default and, when on, serves the complete probe inventory; this
//! listener is **on by default** and serves four bytes. Those two things cannot
//! live on the same socket, because the reason each is safe is the opposite of
//! the other's: the UI is safe because you have to ask for it, and this is safe
//! because there is nothing in it.
//!
//! It exists so the distroless image can carry a `HEALTHCHECK`. There is no
//! shell and no `curl` in that image, so the check is `huginn healthcheck`,
//! which needs something to ask — and it has to answer on a stock config, or the
//! container reports unhealthy out of the box. See ADR-0008.
//!
//! **Liveness, not readiness.** A 200 here means the process is running and its
//! async runtime is still scheduling work. It says nothing about whether probes
//! are succeeding or whether InfluxDB is reachable, and it deliberately must
//! not: a monitor that shuts itself down because the thing it monitors is
//! broken has stopped being a monitor. Backend health is what
//! `/metrics` and the probe results are for.

use std::net::IpAddr;

use anyhow::Context;
use axum::routing::get;
use axum::Router;
use tokio::net::TcpListener;
use tracing::info;

/// Bind the liveness listener, returning it before anything is served.
///
/// Split from [`serve_health`] so a port clash is a startup error in the main
/// task rather than a line in the log of a detached one. The listener is on by
/// default, so it is the one an operator is least likely to suspect when a port
/// is taken — failing at startup names it.
pub async fn bind_health(bind: &str, port: u16) -> anyhow::Result<TcpListener> {
    let addr: IpAddr = bind
        .parse()
        .with_context(|| format!("health bind '{bind}' is not a valid IP address"))?;
    TcpListener::bind((addr, port))
        .await
        .with_context(|| format!("could not bind the health listener on {bind}:{port}"))
}

/// Serve `/health` on an already-bound listener until the process ends.
pub async fn serve_health(listener: TcpListener) -> anyhow::Result<()> {
    if let Ok(addr) = listener.local_addr() {
        info!("Health listening on http://{addr}/health");
    }
    let app = Router::new()
        .route("/health", get(handle_health))
        .layer(axum::middleware::from_fn(crate::headers::security_headers));
    crate::serve::serve_with_limits(listener, app).await
}

async fn handle_health() -> &'static str {
    "OK"
}

/// Ask a running huginn whether it is alive. `Ok(())` iff it answered `200`.
///
/// Used by the `healthcheck` subcommand, which is what the image's `HEALTHCHECK`
/// runs. Deliberately a raw TCP request rather than a `reqwest` client: this
/// runs as a short-lived second process every interval for the life of the
/// container, and building a TLS-capable HTTP client to send eleven plaintext
/// bytes to loopback is most of that process's cost. It also keeps the check
/// free of the client that the probes use, so a change there cannot alter what
/// "healthy" means.
pub async fn check_health(port: u16, timeout: std::time::Duration) -> anyhow::Result<()> {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    let bind = huginn_core::config::HEALTH_BIND;
    let result = tokio::time::timeout(timeout, async {
        let mut sock = tokio::net::TcpStream::connect((bind, port)).await?;
        sock.write_all(
            format!("GET /health HTTP/1.0\r\nHost: {bind}\r\nConnection: close\r\n\r\n").as_bytes(),
        )
        .await?;
        // The status line is all that is needed, and it arrives first. Reading
        // to EOF would work too, but a bounded read cannot be made to wait on a
        // peer that stops talking mid-body.
        let mut buf = [0u8; 64];
        let n = sock.read(&mut buf).await?;
        Ok::<_, std::io::Error>(String::from_utf8_lossy(&buf[..n]).into_owned())
    })
    .await
    .with_context(|| format!("no answer from {bind}:{port} within {timeout:?}"))?
    .with_context(|| format!("could not reach the health listener on {bind}:{port}"))?;

    if result.starts_with("HTTP/1.0 200") || result.starts_with("HTTP/1.1 200") {
        return Ok(());
    }
    anyhow::bail!(
        "health listener on {bind}:{port} did not answer 200: {}",
        result.lines().next().unwrap_or("<empty response>")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn health_handler_returns_ok() {
        assert_eq!(handle_health().await, "OK");
    }

    #[tokio::test]
    async fn bind_rejects_a_non_ip_address() {
        // `AppConfig` never produces this — the address is a constant — but the
        // function is public and the error should name the input either way.
        assert!(bind_health("localhost", 0).await.is_err());
    }

    /// The round trip the `healthcheck` subcommand actually performs.
    #[tokio::test]
    async fn check_health_succeeds_against_a_running_listener() {
        let listener = bind_health(huginn_core::config::HEALTH_BIND, 0)
            .await
            .expect("bind failed");
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(serve_health(listener));

        // Poll rather than sleep: the listener is bound already, but the task
        // that serves it may not have been scheduled yet.
        let ok = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if check_health(port, Duration::from_secs(2)).await.is_ok() {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await;
        assert!(ok.is_ok(), "health check never succeeded");
    }

    /// Nothing listening must be an error, not a hang — this is what tells the
    /// container the process is gone.
    #[tokio::test]
    async fn check_health_fails_when_nothing_is_listening() {
        // Bind to claim a free port, then drop it so the port is closed.
        let listener = bind_health(huginn_core::config::HEALTH_BIND, 0)
            .await
            .unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);

        let err = check_health(port, Duration::from_secs(2)).await;
        assert!(err.is_err(), "a closed port must not report healthy");
    }

    /// A listener that accepts and then says nothing must fail on the timeout
    /// rather than blocking the container's health check for ever.
    #[tokio::test]
    async fn check_health_times_out_on_a_silent_peer() {
        let listener = TcpListener::bind((huginn_core::config::HEALTH_BIND, 0))
            .await
            .unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let mut held = Vec::new();
            while let Ok((stream, _)) = listener.accept().await {
                held.push(stream);
            }
        });

        let started = tokio::time::Instant::now();
        let err = check_health(port, Duration::from_millis(300)).await;
        assert!(err.is_err(), "a silent peer must not report healthy");
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "the timeout did not bound the check"
        );
    }
}
