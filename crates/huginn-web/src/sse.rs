use std::convert::Infallible;
use std::sync::Arc;

use axum::extract::State;
use axum::response::sse::{Event, KeepAlive, Sse};
use tokio_stream::wrappers::errors::BroadcastStreamRecvError;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;
use tracing::warn;

use crate::state::WebState;

/// GET /events — Server-Sent Events push stream.
///
/// Each probe result is sent as a JSON-encoded `ProbeResult` message.
/// The browser keeps this connection open and receives updates without polling.
pub async fn sse_handler(
    State(state): State<Arc<WebState>>,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, Infallible>>> {
    let rx = state.sse_tx.subscribe();
    // `msg.ok()` alone would drop Lagged silently: a browser too slow to keep up
    // would just stop seeing results, with nothing anywhere saying so. Dropping
    // them is still the right call for a debug UI — the alternative is stalling
    // the publisher — but it should not be invisible.
    let stream = BroadcastStream::new(rx).filter_map(|msg| match msg {
        Ok(data) => Some(Ok(Event::default().data(data))),
        Err(BroadcastStreamRecvError::Lagged(n)) => {
            warn!("SSE client too slow — dropped {n} probe results");
            None
        }
    });

    Sse::new(stream).keep_alive(KeepAlive::default())
}
