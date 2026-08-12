//! Serving with a connection cap and a header-read timeout.
//!
//! `axum::serve` accepts without limit and applies no deadline to reading a
//! request line, which is finding F-03 of the 2026-08-02 audit: a client that
//! opens a socket and sends a partial request holds the connection, and its
//! task, indefinitely. Measured on the shipped image — 4 000 idle half-open
//! connections, none refused, RSS from 29.5 MiB to 113.3 MiB, both listeners
//! still answering normally. Nothing capped the count.
//!
//! **A `tower` layer cannot fix that, which is worth stating because it is the
//! obvious first idea and the audit itself suggested it.** `TimeoutLayer` and
//! `ConcurrencyLimitLayer` wrap the *service*, and the service is not reached
//! until hyper has parsed a request. A request line that never completes never
//! arrives, so the layer never runs. The limits have to sit below the service:
//!
//!   * **the connection cap** is a semaphore permit taken *before* `accept`, so
//!     at capacity the connections wait in the kernel's backlog instead of each
//!     costing a task and its buffers;
//!   * **the header-read timeout** is hyper's own, which needs the connection
//!     built by hand — `axum::serve` does not expose hyper's builder.
//!
//! Both listeners are off by default and bind loopback, so this is defence for
//! the deployment that turns one on, not a claim that they are safe to expose.
//!
//! **What the fix trades, rather than removes.** The 2026-08-12 audit measured
//! the same flood again: RSS 8.55 → 19.12 MiB where it had been 29.5 → 113.3,
//! tasks flat at 40, and 3 894 of 4 000 connections closed by the server. The
//! memory exhaustion is gone. In its place, while the flood runs the *flooded*
//! listener stops serving — three of five legitimate requests to it timed out,
//! because a new peer waits for a permit and permits free only when
//! `HEADER_READ_TIMEOUT` expires. That is F-09, and it is accepted: bounding
//! memory at the cost of latency on an optional debug surface is the right way
//! round. The permits are per listener, which is what keeps it narrow — during
//! that flood the other two answered in 0.3–4 ms throughout, the container
//! stayed healthy, and `huginn healthcheck` kept exiting 0, so a flood of a
//! published debug port cannot make an orchestrator restart a working monitor.
//! `docs/risks.md` carries it as R7.

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use hyper_util::rt::{TokioExecutor, TokioIo, TokioTimer};
use hyper_util::server::conn::auto::Builder;
use hyper_util::service::TowerToHyperService;
use tokio::net::TcpListener;
use tokio::sync::Semaphore;
use tracing::{debug, warn};

/// Connections served at once, per listener.
///
/// Generous for what these serve — one dashboard holds one connection open for
/// its SSE stream, and a Prometheus scrape is a single short request — while
/// still bounding the memory an unauthenticated peer can make the process use.
/// At roughly 21 KiB per connection (measured, see above) this caps the growth
/// at about 5 MiB rather than letting it run to the container limit.
const MAX_CONNECTIONS: usize = 256;

/// How long a peer may take to send its request head.
///
/// This is the one that closes F-03: a connection that has sent nothing
/// complete by then is dropped, so a half-open socket costs a slot for three
/// seconds instead of for ever. It bounds only the head — a slow *body* or a
/// long-lived response is not affected, which is why the SSE stream on
/// `/events` keeps working.
///
/// Three seconds rather than the ten this shipped with, because the deadline is
/// also the length of the denial it leaves behind (F-09 of the 2026-08-12
/// audit): an attacker holding all 256 permits with slow heads makes legitimate
/// requests to that listener wait until permits free, and they free when this
/// expires. The window shrinks linearly with the number, and three seconds is
/// still far more than any real client needs to put a request head on a socket
/// it has already connected — these listeners serve a browser on loopback and a
/// Prometheus scrape, not a modem.
const HEADER_READ_TIMEOUT: Duration = Duration::from_secs(3);

/// Serve `app` on `listener` until the process ends.
///
/// Replaces `axum::serve`, which cannot express either limit.
pub(crate) async fn serve_with_limits(listener: TcpListener, app: Router) -> anyhow::Result<()> {
    serve_with(listener, app, MAX_CONNECTIONS, HEADER_READ_TIMEOUT).await
}

/// The body of [`serve_with_limits`], with the limits as arguments.
///
/// Split out only so the tests can use a header timeout measured in
/// milliseconds instead of waiting ten seconds for the real one. Callers use
/// the wrapper; the constants are the contract.
async fn serve_with(
    listener: TcpListener,
    app: Router,
    max_connections: usize,
    header_read_timeout: Duration,
) -> anyhow::Result<()> {
    let permits = Arc::new(Semaphore::new(max_connections));

    loop {
        // Before `accept`, deliberately. Acquiring after would mean accepting
        // the connection first and then holding it — the task and its buffers
        // would already exist, which is the cost being avoided. Waiting here
        // leaves the peer in the listen backlog, and the kernel refuses it once
        // that fills, which is the behaviour we want at capacity.
        // `acquire_owned` fails only if the semaphore has been closed, and
        // `close()` is never called on it: it is created here, moved nowhere,
        // and lives as long as this loop. A genuinely unreachable invariant, so
        // it is one of the few places a panic is the right expression of "this
        // cannot happen" — the alternative would be an error branch no test
        // could ever reach and no reader could ever check.
        let permit = Arc::clone(&permits)
            .acquire_owned()
            .await
            .expect("the semaphore is never closed");

        let (stream, peer) = match listener.accept().await {
            Ok(v) => v,
            // A single failed accept is not fatal — a peer that vanishes
            // between the SYN and our accept produces one, and returning would
            // take the listener down with it.
            Err(e) => {
                warn!(error = %e, "accept failed");
                continue;
            }
        };

        let service = TowerToHyperService::new(app.clone());
        tokio::spawn(async move {
            let mut builder = Builder::new(TokioExecutor::new());
            // The timer is not optional: hyper panics with "timeout
            // `header_read_timeout` set, but no timer set" the first time it
            // arms the deadline. It is a runtime panic, not a type error, so
            // nothing catches it at compile time — the test suite did.
            builder
                .http1()
                .timer(TokioTimer::new())
                .header_read_timeout(header_read_timeout);

            if let Err(e) = builder
                .serve_connection(TokioIo::new(stream), service)
                .await
            {
                // Expected in normal use: a browser closing a tab mid-stream, a
                // scraper hanging up, a half-open connection hitting the
                // timeout above. Debug, not warn — at warn level an idle
                // dashboard would fill the log.
                debug!(%peer, error = %e, "connection closed");
            }
            drop(permit);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// A connection that never finishes its request head must be dropped.
    ///
    /// This is F-03 itself, and the reason a `tower` layer was the wrong tool:
    /// the service is never reached, so nothing above hyper can time it out.
    /// The assertion is that the socket is *closed by the server* — `read`
    /// returning 0 — not that some duration elapsed.
    #[tokio::test]
    async fn half_open_connection_is_closed_by_the_header_timeout() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();
        let app = Router::new();
        tokio::spawn(serve_with(listener, app, 8, Duration::from_millis(200)));

        let mut sock = tokio::net::TcpStream::connect(addr).await.unwrap();
        // A request line that is deliberately never terminated.
        sock.write_all(b"GET / HTT").await.unwrap();

        let started = Instant::now();
        let mut buf = [0u8; 64];
        let n = tokio::time::timeout(Duration::from_secs(5), sock.read(&mut buf))
            .await
            .expect("the server never closed the half-open connection")
            .unwrap();
        assert_eq!(n, 0, "expected EOF, got {n} bytes: {:?}", &buf[..n]);
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "closed, but not by the timeout"
        );
    }

    /// A well-formed request is still answered — the timeout bounds the head,
    /// not the connection.
    #[tokio::test]
    async fn a_complete_request_is_served_normally() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();
        let app = Router::new().route("/", axum::routing::get(|| async { "ok" }));
        tokio::spawn(serve_with(listener, app, 8, Duration::from_millis(200)));

        let mut sock = tokio::net::TcpStream::connect(addr).await.unwrap();
        sock.write_all(
            b"GET / HTTP/1.1
Host: x

",
        )
        .await
        .unwrap();
        let mut buf = vec![0u8; 256];
        let n = tokio::time::timeout(Duration::from_secs(5), sock.read(&mut buf))
            .await
            .expect("no response")
            .unwrap();
        let head = String::from_utf8_lossy(&buf[..n]);
        assert!(
            head.starts_with("HTTP/1.1 200"),
            "unexpected response: {head}"
        );
    }
}
