//! Ask-with-citations endpoint (RAG).
//!
//! Returns a JSON answer with resolved citations. Token-streamed delivery is a
//! later enhancement; the citation correctness is the load-bearing part and is
//! fully in place here.

use axum::extract::{Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use tessera_core::error::{Error, ErrorKind};
use tessera_db::repos::ask_history;
use tessera_search::AskAnswer;

use crate::auth::{AuthContext, Scope};
use crate::error::ApiError;
use crate::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/ask", post(ask))
        .route("/ask/history", get(history))
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

    // Log the question and answer (best-effort; never fail the ask over history).
    if let Ok(value) = serde_json::to_value(&answer) {
        let _ =
            ask_history::record(&state.db.api, Some(&ctx.audit_id()), &req.question, &value).await;
    }
    Ok(Json(answer))
}

#[derive(Debug, Deserialize)]
struct HistoryParams {
    #[serde(default)]
    limit: Option<i64>,
}

/// Recent ask questions and their stored answers, newest first.
async fn history(
    State(state): State<AppState>,
    ctx: AuthContext,
    Query(params): Query<HistoryParams>,
) -> Result<Json<Vec<serde_json::Value>>, ApiError> {
    ctx.require(Scope::Read)?;
    let limit = params.limit.unwrap_or(50).clamp(1, 200);
    let rows = ask_history::list(&state.db.api, limit).await?;
    Ok(Json(
        rows.into_iter()
            .map(|r| {
                serde_json::json!({
                    "id": r.id,
                    "question": r.question,
                    "answer": r.answer,
                    "created_at": r.created_at,
                })
            })
            .collect(),
    ))
}
