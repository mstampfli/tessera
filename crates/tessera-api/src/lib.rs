//! The HTTP API surface and shared application state.
//!
//! [`build_router`] wires the routes, middleware, and application state.
//! Handlers are thin: they authenticate, validate, call the service layer
//! (ingest, search, ask), and render. The same service functions back the MCP
//! server so the two surfaces cannot drift.

pub mod auth;
pub mod error;
pub mod events;
pub mod rate_limit;
pub mod routes;

use std::sync::Arc;

use axum::Router;
use tessera_core::config::Config;
use tessera_db::cas::CasStore;
use tessera_db::repos::embeddings::EmbeddingSpace;
use tessera_db::Db;
use tessera_providers::{EmbeddingProvider, LlmProvider};
use tokio::sync::broadcast;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::trace::TraceLayer;

use crate::events::EventBus;
use crate::rate_limit::RateLimiter;

/// Shared, cheaply cloneable application state.
#[derive(Clone)]
pub struct AppState {
    pub db: Db,
    pub config: Arc<Config>,
    pub cas: CasStore,
    pub embedder: Arc<dyn EmbeddingProvider>,
    pub llm: Arc<dyn LlmProvider>,
    /// The active embedding space (id, dim) used for storage and search.
    pub space: EmbeddingSpace,
    /// Live pipeline progress events, fanned out to SSE subscribers.
    pub events: EventBus,
    /// Login attempt rate limiter (per client IP).
    pub login_limiter: Arc<RateLimiter>,
}

/// Inputs needed to build the application state.
pub struct AppStateParts {
    pub db: Db,
    pub config: Arc<Config>,
    pub cas: CasStore,
    pub embedder: Arc<dyn EmbeddingProvider>,
    pub llm: Arc<dyn LlmProvider>,
    pub space: EmbeddingSpace,
}

impl AppState {
    #[must_use]
    pub fn new(parts: AppStateParts) -> Self {
        let (tx, _rx) = broadcast::channel(1024);
        Self {
            db: parts.db,
            config: parts.config,
            cas: parts.cas,
            embedder: parts.embedder,
            llm: parts.llm,
            space: parts.space,
            events: EventBus::new(tx),
            login_limiter: Arc::new(RateLimiter::new(10, std::time::Duration::from_mins(1))),
        }
    }
}

/// Build the full application router.
pub fn build_router(state: AppState) -> Router {
    // Global body cap for ingestion (uploads, bulk NDJSON). Per-item limits are
    // enforced in the ingest handlers. Both axum's extractor limit and the
    // tower-http layer are raised so large uploads are accepted.
    const GLOBAL_BODY_LIMIT: usize = 64 * 1024 * 1024;

    Router::new()
        .merge(routes::health::router())
        .nest("/v1", routes::v1_router())
        .layer(axum::extract::DefaultBodyLimit::max(GLOBAL_BODY_LIMIT))
        .layer(RequestBodyLimitLayer::new(GLOBAL_BODY_LIMIT))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
