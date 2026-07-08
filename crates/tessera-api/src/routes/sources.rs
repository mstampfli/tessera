//! Source listing endpoints.

use axum::extract::{Path, Query, State};
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use tessera_core::error::Error;
use uuid::Uuid;

use crate::auth::{AuthContext, Scope};
use crate::error::ApiError;
use crate::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/sources", get(list_sources))
        .route("/sources/{id}", get(get_source))
        .route("/sources/{id}/documents", get(list_documents))
}

#[derive(Debug, Deserialize)]
struct Page {
    #[serde(default)]
    before: Option<Uuid>,
    #[serde(default)]
    limit: Option<i64>,
}

#[derive(Debug, Serialize)]
struct SourceView {
    id: Uuid,
    kind: String,
    name: String,
    created_at: chrono::DateTime<chrono::Utc>,
}

async fn list_sources(
    State(state): State<AppState>,
    ctx: AuthContext,
    Query(page): Query<Page>,
) -> Result<Json<Vec<SourceView>>, ApiError> {
    ctx.require(Scope::Read)?;
    let limit = page.limit.unwrap_or(50).clamp(1, 200);
    let rows = tessera_db::repos::sources::list(&state.db.api, page.before, limit).await?;
    Ok(Json(rows.into_iter().map(source_view).collect()))
}

async fn get_source(
    State(state): State<AppState>,
    ctx: AuthContext,
    Path(id): Path<Uuid>,
) -> Result<Json<SourceView>, ApiError> {
    ctx.require(Scope::Read)?;
    let src = tessera_db::repos::sources::get(&state.db.api, id)
        .await?
        .ok_or_else(|| ApiError(Error::not_found("source")))?;
    Ok(Json(source_view(src)))
}

#[derive(Debug, Serialize)]
struct DocumentSummary {
    id: Uuid,
    title: Option<String>,
    media_type: String,
    status: String,
    size_bytes: i64,
}

async fn list_documents(
    State(state): State<AppState>,
    ctx: AuthContext,
    Path(id): Path<Uuid>,
    Query(page): Query<Page>,
) -> Result<Json<Vec<DocumentSummary>>, ApiError> {
    ctx.require(Scope::Read)?;
    let limit = page.limit.unwrap_or(50).clamp(1, 200);
    let docs =
        tessera_db::repos::documents::list_by_source(&state.db.api, id, page.before, limit).await?;
    Ok(Json(
        docs.into_iter()
            .map(|d| DocumentSummary {
                id: d.id,
                title: d.title,
                media_type: d.media_type,
                status: d.status,
                size_bytes: d.size_bytes,
            })
            .collect(),
    ))
}

fn source_view(s: tessera_db::repos::sources::Source) -> SourceView {
    SourceView {
        id: s.id,
        kind: s.kind,
        name: s.name,
        created_at: s.created_at,
    }
}
