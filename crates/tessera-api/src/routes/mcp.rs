//! HTTP MCP transport: JSON-RPC over a single POST endpoint, for remote agents
//! on the tailnet. Bearer-authenticated with the `mcp` scope. Delegates to the
//! same tool dispatch the stdio server uses.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use serde_json::Value;

use crate::auth::{AuthContext, Scope};
use crate::error::ApiError;
use crate::AppState;

pub fn router() -> Router<AppState> {
    Router::new().route("/mcp", post(mcp))
}

async fn mcp(
    State(state): State<AppState>,
    ctx: AuthContext,
    Json(request): Json<Value>,
) -> Result<Response, ApiError> {
    ctx.require(Scope::Mcp)?;

    let mcp_state = tessera_mcp::McpState::from_parts(
        state.db.clone(),
        state.cas.clone(),
        state.embedder.clone(),
        state.llm.clone(),
        state.space.clone(),
        state.mcp_source_id,
    );

    match tessera_mcp::dispatch_request(&mcp_state, &request).await {
        // A request (has an id) gets its JSON-RPC response.
        Some(response) => Ok((StatusCode::OK, Json(response)).into_response()),
        // A notification (no id) is acknowledged with no body.
        None => Ok(StatusCode::ACCEPTED.into_response()),
    }
}
