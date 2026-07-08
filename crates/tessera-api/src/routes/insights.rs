//! Insight endpoints: the triage feed (list), detail with evidence, and triage
//! feedback (useful / dismissed).

use axum::extract::{Path, Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tessera_core::error::{Error, ErrorKind};
use tessera_db::repos::insights;
use uuid::Uuid;

use crate::auth::{AuthContext, Scope};
use crate::error::ApiError;
use crate::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/insights", get(list))
        .route("/insights/{id}", get(detail))
        .route("/insights/{id}/feedback", post(feedback))
}

#[derive(Debug, Deserialize)]
struct ListParams {
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    limit: Option<i64>,
}

#[derive(Debug, Serialize)]
struct InsightView {
    id: Uuid,
    cluster_id: Option<Uuid>,
    title: String,
    body_md: String,
    severity: String,
    confidence: f32,
    suggested_actions: Value,
    tags: Vec<String>,
    status: String,
    model: String,
    created_at: chrono::DateTime<chrono::Utc>,
}

fn view(i: insights::Insight) -> InsightView {
    InsightView {
        id: i.id,
        cluster_id: i.cluster_id,
        title: i.title,
        body_md: i.body_md,
        severity: i.severity,
        confidence: i.confidence,
        suggested_actions: i.suggested_actions,
        tags: i.tags,
        status: i.status,
        model: i.model,
        created_at: i.created_at,
    }
}

async fn list(
    State(state): State<AppState>,
    ctx: AuthContext,
    Query(params): Query<ListParams>,
) -> Result<Json<Vec<InsightView>>, ApiError> {
    ctx.require(Scope::Read)?;
    let limit = params.limit.unwrap_or(50).clamp(1, 200);
    let rows = insights::list(&state.db.api, params.status.as_deref(), limit).await?;
    Ok(Json(rows.into_iter().map(view).collect()))
}

#[derive(Debug, Serialize)]
struct EvidenceView {
    chunk_id: Uuid,
    document_id: Uuid,
    title: Option<String>,
    seq: i32,
    excerpt: String,
}

#[derive(Debug, Serialize)]
struct InsightDetail {
    insight: InsightView,
    evidence: Vec<EvidenceView>,
}

async fn detail(
    State(state): State<AppState>,
    ctx: AuthContext,
    Path(id): Path<Uuid>,
) -> Result<Json<InsightDetail>, ApiError> {
    ctx.require(Scope::Read)?;
    let insight = insights::get(&state.db.api, id)
        .await?
        .ok_or_else(|| ApiError(Error::not_found("insight")))?;
    let evidence = insights::evidence(&state.db.api, id).await?;
    Ok(Json(InsightDetail {
        insight: view(insight),
        evidence: evidence
            .into_iter()
            .map(|e| EvidenceView {
                chunk_id: e.chunk_id,
                document_id: e.document_id,
                title: e.title,
                seq: e.seq,
                excerpt: e.excerpt,
            })
            .collect(),
    }))
}

#[derive(Debug, Deserialize)]
struct FeedbackRequest {
    /// One of: surfaced, useful, dismissed.
    status: String,
}

async fn feedback(
    State(state): State<AppState>,
    ctx: AuthContext,
    Path(id): Path<Uuid>,
    Json(req): Json<FeedbackRequest>,
) -> Result<Json<Value>, ApiError> {
    ctx.require(Scope::Read)?;
    if !["surfaced", "useful", "dismissed"].contains(&req.status.as_str()) {
        return Err(ApiError(Error::new(
            ErrorKind::Invalid,
            "invalid feedback status",
        )));
    }
    let ok = insights::set_status(&state.db.api, id, &req.status).await?;
    if !ok {
        return Err(ApiError(Error::not_found("insight")));
    }
    let _ = tessera_db::repos::audit::record(
        &state.db.api,
        Some(&ctx.audit_id()),
        "insight.feedback",
        Some(&id.to_string()),
        &serde_json::json!({ "status": req.status }),
    )
    .await;
    Ok(Json(serde_json::json!({ "ok": true })))
}
