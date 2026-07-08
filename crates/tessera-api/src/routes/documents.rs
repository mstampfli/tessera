//! Document and chunk read endpoints (the citation terminus).

use axum::extract::{Path, State};
use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;
use tessera_core::error::Error;
use uuid::Uuid;

use crate::auth::{AuthContext, Scope};
use crate::error::ApiError;
use crate::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/documents/{id}", get(get_document))
        .route("/documents/{id}/chunks", get(get_chunks))
}

#[derive(Debug, Serialize)]
struct DocumentView {
    id: Uuid,
    source_id: Uuid,
    media_type: String,
    size_bytes: i64,
    title: Option<String>,
    uri: Option<String>,
    status: String,
    error: Option<String>,
    content_hash: String,
}

async fn get_document(
    State(state): State<AppState>,
    ctx: AuthContext,
    Path(id): Path<Uuid>,
) -> Result<Json<DocumentView>, ApiError> {
    ctx.require(Scope::Read)?;
    let doc = tessera_db::repos::documents::get(&state.db.api, id)
        .await?
        .ok_or_else(|| ApiError(Error::not_found("document")))?;
    Ok(Json(DocumentView {
        id: doc.id,
        source_id: doc.source_id,
        media_type: doc.media_type,
        size_bytes: doc.size_bytes,
        title: doc.title,
        uri: doc.uri,
        status: doc.status,
        error: doc.error,
        content_hash: hex(&doc.content_hash),
    }))
}

#[derive(Debug, Serialize)]
struct ChunkView {
    id: Uuid,
    seq: i32,
    text: String,
    token_count: i32,
}

async fn get_chunks(
    State(state): State<AppState>,
    ctx: AuthContext,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<ChunkView>>, ApiError> {
    ctx.require(Scope::Read)?;
    let chunks = tessera_db::repos::chunks::list_by_document(&state.db.api, id).await?;
    Ok(Json(
        chunks
            .into_iter()
            .map(|c| ChunkView {
                id: c.id,
                seq: c.seq,
                text: c.text,
                token_count: c.token_count,
            })
            .collect(),
    ))
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}
