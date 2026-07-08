//! Route modules and the `/v1` sub-router.

pub mod auth;
pub mod health;
pub mod tokens;

use axum::Router;

use crate::AppState;

/// Everything mounted under `/v1`.
pub fn v1_router() -> Router<AppState> {
    Router::new().merge(auth::router()).merge(tokens::router())
}
