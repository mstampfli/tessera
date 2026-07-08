//! The HTTP API surface and shared application state.
//!
//! [`build_router`] wires the routes, the tracing/limit middleware, and the
//! application state. Handlers are thin: they authenticate, validate, call the
//! service layer, and render. The same service functions back the MCP server so
//! the two surfaces cannot drift.

pub mod auth;
pub mod error;
pub mod routes;

use std::sync::Arc;

use axum::Router;
use tessera_core::config::Config;
use tessera_db::Db;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::trace::TraceLayer;

/// Shared, cheaply cloneable application state.
#[derive(Clone)]
pub struct AppState {
    pub db: Db,
    pub config: Arc<Config>,
}

impl AppState {
    #[must_use]
    pub fn new(db: Db, config: Arc<Config>) -> Self {
        Self { db, config }
    }
}

/// Build the full application router.
pub fn build_router(state: AppState) -> Router {
    // A conservative global body cap; per-route ingest limits are larger and set
    // where the bulk endpoints live (M1).
    const GLOBAL_BODY_LIMIT: usize = 8 * 1024 * 1024;

    Router::new()
        .merge(routes::health::router())
        .nest("/v1", routes::v1_router())
        .layer(RequestBodyLimitLayer::new(GLOBAL_BODY_LIMIT))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
