//! Ask-with-citations endpoint (RAG).
//!
//! Returns a JSON answer with resolved citations. Token-streamed delivery is a
//! later enhancement; the citation correctness is the load-bearing part and is
//! fully in place here.

use axum::extract::State;
use axum::routing::post;
use axum::{Json, Router};
use serde::Deserialize;
use tessera_core::error::{Error, ErrorKind};
use tessera_search::AskAnswer;

use crate::auth::{AuthContext, Scope};
use crate::error::ApiError;
use crate::AppState;

pub fn router() -> Router<AppState> {
    Router::new().route("/ask", post(ask))
}

#[derive(Debug, Deserialize)]
struct AskRequest {
    question: String,
    #[serde(default)]
    k: Option<i64>,
}

async fn ask(
    State(state): State<AppState>,
    ctx: AuthContext,
    Json(req): Json<AskRequest>,
) -> Result<Json<AskAnswer>, ApiError> {
    ctx.require(Scope::Read)?;
    if req.question.trim().is_empty() {
        return Err(ApiError(Error::new(
            ErrorKind::Invalid,
            "question is required",
        )));
    }
    let k = req.k.unwrap_or(8).clamp(1, 30);

    let answer = tessera_search::ask(
        &state.db.api,
        &state.embedder,
        &state.llm,
        Some(&state.space),
        &req.question,
        k,
    )
    .await?;
    Ok(Json(answer))
}
