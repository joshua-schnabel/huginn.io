pub mod dns;
pub mod http;
pub mod imap;
pub mod registry;
pub mod smtp;
pub mod tcp;
pub mod tls;
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

/// Upper bound on a line-oriented greeting read from a monitored host.
///
/// The SMTP and IMAP probes read a peer's opening line into memory before
/// looking at it, and the peer decides how much to send. 512 bytes is the SMTP
/// line limit in RFC 5321 §4.5.3.1 and comfortably more than any real IMAP
/// greeting, so a longer one is either not the protocol we asked for or is
/// trying to make us hold something.
pub const MAX_GREETING_BYTES: usize = 512;

/// Read one complete greeting line from a freshly opened connection.
///
/// Reads until a line ending arrives, the peer closes, or
/// [`MAX_GREETING_BYTES`] is reached — whichever comes first.
///
/// A single `read()` was what this replaced, and it was wrong for a reason that
/// only shows up against real servers: TCP is a byte stream, so a perfectly
/// valid `220 mail.example.com ESMTP` can arrive as `22` and then the rest. The
/// prefix check then ran against `22`, the probe reported DOWN, and the server
/// was fine. It is timing-dependent, so it appears as a monitor that
/// occasionally invents outages — the least believable kind of alert.
///
/// Bounded on purpose, and the caller wraps this in the probe's overall
/// deadline: without both, a peer that sends one byte at a time and never a
/// newline holds the loop for as long as it likes.
pub async fn read_greeting_line<S>(stream: &mut S) -> std::io::Result<String>
where
    S: tokio::io::AsyncRead + Unpin,
{
    use tokio::io::AsyncReadExt as _;

    let mut collected: Vec<u8> = Vec::with_capacity(128);
    let mut chunk = [0u8; 128];
    loop {
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            break; // peer closed — return whatever it managed to say
        }
        collected.extend_from_slice(&chunk[..n]);
        if collected.contains(&b'\n') || collected.len() >= MAX_GREETING_BYTES {
            break;
        }
    }
    collected.truncate(MAX_GREETING_BYTES);
    // Lossy: the greeting is remote input and may be any bytes at all. Control
    // characters are escaped later, by ProbeResult::failure.
    Ok(String::from_utf8_lossy(&collected).into_owned())
}

/// Runs `fut` with a timeout, flattening the `Ok(Ok(v))` / `Ok(Err(e))` / `Err(elapsed)` triple
/// into a simple `Result<T, String>`.  Used by every probe to eliminate the repeated
/// nested-result boilerplate.
///
/// **One call per probe, covering every step.** Applying it separately to a
/// connect and then to a read gives each the full `timeout_secs`, so the probe's
/// worst case is twice what the operator configured — and `timeout_secs` stops
/// meaning anything you can reason about. Where a probe has several sequential
/// operations, they belong inside one future passed here.
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[tokio::test]
    async fn reads_a_greeting_that_arrives_in_one_piece() {
        let mut src = Cursor::new(b"220 mail.example.com ESMTP\r\n".to_vec());
        let line = read_greeting_line(&mut src).await.unwrap();
        assert_eq!(line, "220 mail.example.com ESMTP\r\n");
    }

    /// The regression. A byte stream may split anywhere, so a single `read()`
    /// could see `22` and conclude the server was not speaking SMTP.
    #[tokio::test]
    async fn reassembles_a_greeting_split_across_reads() {
        // A reader that hands over one byte at a time — the pathological case
        // of the same behaviour a real network produces occasionally.
        struct DribbleReader(Vec<u8>, usize);
        impl tokio::io::AsyncRead for DribbleReader {
            fn poll_read(
                mut self: std::pin::Pin<&mut Self>,
                _cx: &mut std::task::Context<'_>,
                buf: &mut tokio::io::ReadBuf<'_>,
            ) -> std::task::Poll<std::io::Result<()>> {
                if self.1 < self.0.len() {
                    let b = self.0[self.1];
                    self.1 += 1;
                    buf.put_slice(&[b]);
                }
                std::task::Poll::Ready(Ok(()))
            }
        }

        let mut src = DribbleReader(b"220 split.example.com ESMTP\r\n".to_vec(), 0);
        let line = read_greeting_line(&mut src).await.unwrap();
        assert!(
            line.starts_with("220"),
            "a split greeting must still be recognised, got {line:?}"
        );
    }

    /// A peer that closes without a newline still yields what it sent, rather
    /// than looping.
    #[tokio::test]
    async fn returns_what_was_sent_when_the_peer_closes_early() {
        let mut src = Cursor::new(b"220 partial".to_vec());
        let line = read_greeting_line(&mut src).await.unwrap();
        assert_eq!(line, "220 partial");
    }

    /// A peer that never sends a newline must not be able to make us hold an
    /// unbounded buffer.
    #[tokio::test]
    async fn bounds_a_greeting_with_no_line_ending() {
        let flood = vec![b'x'; MAX_GREETING_BYTES * 4];
        let mut src = Cursor::new(flood);
        let line = read_greeting_line(&mut src).await.unwrap();
        assert_eq!(line.len(), MAX_GREETING_BYTES);
    }
}
