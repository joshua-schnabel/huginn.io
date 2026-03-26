use std::convert::Infallible;
use std::sync::Arc;

use axum::extract::State;
use axum::response::sse::{Event, KeepAlive, Sse};
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;

use crate::state::WebState;

/// GET /events — Server-Sent Events push stream.
///
/// Each probe result is sent as a JSON-encoded `ProbeResult` message.
/// The browser keeps this connection open and receives updates without polling.
pub async fn sse_handler(
    State(state): State<Arc<WebState>>,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, Infallible>>> {
    let rx = state.sse_tx.subscribe();
    let stream = BroadcastStream::new(rx).filter_map(|msg| {
        msg.ok().map(|data| Ok(Event::default().data(data)))
    });

    Sse::new(stream).keep_alive(KeepAlive::default())
}
