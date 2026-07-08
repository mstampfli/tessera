//! Route modules and the `/v1` sub-router.

pub mod ask;
pub mod auth;
pub mod clusters;
pub mod documents;
pub mod entities;
pub mod events;
pub mod health;
pub mod ingest;
pub mod insights;
pub mod jobs;
pub mod search;
pub mod sources;
pub mod tokens;

use axum::Router;

use crate::AppState;

/// Everything mounted under `/v1`.
pub fn v1_router() -> Router<AppState> {
    Router::new()
        .merge(auth::router())
        .merge(tokens::router())
        .merge(ingest::router())
        .merge(search::router())
        .merge(ask::router())
        .merge(documents::router())
        .merge(entities::router())
        .merge(insights::router())
        .merge(clusters::router())
        .merge(sources::router())
        .merge(jobs::router())
        .merge(events::router())
}
