//! Hybrid search endpoint.

use axum::extract::{Query, State};
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use tessera_search::{SearchHit, SearchMode};

use crate::auth::{AuthContext, Scope};
use crate::error::ApiError;
use crate::AppState;

pub fn router() -> Router<AppState> {
    Router::new().route("/search", get(search))
}

#[derive(Debug, Deserialize)]
struct SearchParams {
    q: String,
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    limit: Option<i64>,
}

#[derive(Debug, Serialize)]
struct SearchResponse {
    query: String,
    mode: String,
    hits: Vec<SearchHit>,
}

async fn search(
    State(state): State<AppState>,
    ctx: AuthContext,
    Query(params): Query<SearchParams>,
) -> Result<Json<SearchResponse>, ApiError> {
    ctx.require(Scope::Read)?;
    let mode = SearchMode::parse(params.mode.as_deref().unwrap_or("hybrid"));
    let limit = params.limit.unwrap_or(20).clamp(1, 100);

    let hits = tessera_search::search(
        &state.db.api,
        &state.embedder,
        Some(&state.space),
        &params.q,
        mode,
        limit,
    )
    .await?;

    Ok(Json(SearchResponse {
        query: params.q,
        mode: params.mode.unwrap_or_else(|| "hybrid".to_string()),
        hits,
    }))
}
