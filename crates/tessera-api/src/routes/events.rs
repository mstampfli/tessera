//! Server-Sent Events: live pipeline progress for the jobs tray and feed.
//!
//! The stream carries the small JSON payloads that pipeline stages emit via
//! Postgres NOTIFY (document processing, embed progress, document ready). Browser
//! `EventSource` authenticates with the session cookie; programmatic clients that
//! cannot stream can poll the job status endpoint instead.

use std::convert::Infallible;

use axum::extract::State;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::routing::get;
use axum::Router;
use futures::Stream;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;

use crate::auth::{AuthContext, Scope};
use crate::error::ApiError;
use crate::AppState;

pub fn router() -> Router<AppState> {
    Router::new().route("/events", get(events))
}

async fn events(
    State(state): State<AppState>,
    ctx: AuthContext,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, ApiError> {
    ctx.require(Scope::Read)?;
    let rx = state.events.subscribe();
    let stream = BroadcastStream::new(rx)
        // Drop lag errors (a slow consumer that fell behind) rather than closing
        // the stream; progress is ephemeral.
        .filter_map(Result::ok)
        .map(|payload| Ok(Event::default().data(payload)));

    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}
