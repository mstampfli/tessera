//! Ingestion endpoints: single item, bulk NDJSON, and multipart upload.
//!
//! Ingestion is deliberately cheap: sniff, write the bytes to the content store
//! (deduped by hash), record the document, and enqueue processing in the SAME
//! transaction (exactly-once handoff). The heavy work (extract, chunk, embed)
//! happens in the pipeline. Requires the `ingest` scope.

use axum::extract::{Multipart, State};
use axum::routing::post;
use axum::{Json, Router};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tessera_core::error::{Error, ErrorKind};
use tessera_db::queue;
use tessera_db::repos::sources;
use uuid::Uuid;

use crate::auth::{AuthContext, Scope};
use crate::error::ApiError;
use crate::AppState;

/// Backpressure: reject new ingestion when the queue is this deep.
const QUEUE_HIGH_WATER: i64 = 200_000;
/// Cap on a single item's decoded content, independent of the transport limit.
const MAX_ITEM_BYTES: usize = 32 * 1024 * 1024;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/ingest", post(ingest_single))
        .route("/ingest/bulk", post(ingest_bulk))
        .route("/ingest/upload", post(ingest_upload))
        .route("/ingest/url", post(ingest_url))
}

#[derive(Debug, Serialize)]
pub struct IngestResult {
    pub document_id: Uuid,
    pub deduped: bool,
    pub status: String,
}

#[derive(Debug, Deserialize)]
struct IngestItem {
    /// UTF-8 content, or use `content_base64` for binary/precise bytes.
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    content_base64: Option<String>,
    #[serde(default)]
    media_type: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    uri: Option<String>,
    #[serde(default)]
    meta: Option<Value>,
}

impl IngestItem {
    fn decode_bytes(&self) -> Result<Vec<u8>, ApiError> {
        let bytes = if let Some(b64) = &self.content_base64 {
            BASE64.decode(b64.trim()).map_err(|_| {
                ApiError(Error::new(
                    ErrorKind::Invalid,
                    "content_base64 is not valid base64",
                ))
            })?
        } else if let Some(text) = &self.content {
            text.clone().into_bytes()
        } else {
            return Err(ApiError(Error::new(
                ErrorKind::Invalid,
                "provide content or content_base64",
            )));
        };
        if bytes.is_empty() {
            return Err(ApiError(Error::new(ErrorKind::Invalid, "empty content")));
        }
        if bytes.len() > MAX_ITEM_BYTES {
            return Err(ApiError(Error::new(
                ErrorKind::TooLarge,
                "item exceeds size limit",
            )));
        }
        Ok(bytes)
    }
}

#[derive(Debug, Deserialize)]
struct SingleRequest {
    #[serde(flatten)]
    item: IngestItem,
    #[serde(default)]
    source_id: Option<Uuid>,
    #[serde(default)]
    source_name: Option<String>,
}

async fn check_backpressure(state: &AppState) -> Result<(), ApiError> {
    let depth = queue::depth(&state.db.api).await?;
    if depth > QUEUE_HIGH_WATER {
        return Err(ApiError(Error::new(
            ErrorKind::RateLimited,
            "ingestion queue is full, retry later",
        )));
    }
    Ok(())
}

/// Resolve or create the source this ingestion attaches to.
async fn resolve_source(
    state: &AppState,
    ctx: &AuthContext,
    source_id: Option<Uuid>,
    name: Option<&str>,
    kind: &str,
) -> Result<Uuid, ApiError> {
    if let Some(id) = source_id {
        // Confirm it exists so we do not create dangling documents.
        sources::get(&state.db.api, id)
            .await?
            .ok_or_else(|| ApiError(Error::not_found("source")))?;
        return Ok(id);
    }
    let name = name.unwrap_or("ingest");
    let src = sources::create(
        &state.db.api,
        kind,
        name,
        &json!({ "principal": ctx.audit_id() }),
    )
    .await?;
    Ok(src.id)
}

/// The core ingest step, shared by all three endpoints. Delegates to the single
/// ingestion path in the pipeline crate (the same one the MCP server uses).
async fn ingest_one(
    state: &AppState,
    source_id: Uuid,
    item: &IngestItem,
) -> Result<IngestResult, ApiError> {
    let bytes = item.decode_bytes()?;
    let meta = item.meta.clone().unwrap_or_else(|| json!({}));

    let ingested = tessera_pipeline::ingest_bytes(
        &state.db,
        &state.cas,
        tessera_pipeline::IngestBytes {
            source_id,
            bytes: &bytes,
            media_type_hint: item.media_type.as_deref(),
            title: item.title.as_deref(),
            uri: item.uri.as_deref(),
            meta,
        },
    )
    .await
    .map_err(ApiError)?;

    Ok(IngestResult {
        document_id: ingested.document_id,
        deduped: ingested.deduped,
        status: ingested.status,
    })
}

async fn ingest_single(
    State(state): State<AppState>,
    ctx: AuthContext,
    Json(req): Json<SingleRequest>,
) -> Result<Json<IngestResult>, ApiError> {
    ctx.require(Scope::Ingest)?;
    check_backpressure(&state).await?;

    let source_id = resolve_source(
        &state,
        &ctx,
        req.source_id,
        req.source_name.as_deref(),
        source_kind(&ctx),
    )
    .await?;
    let result = ingest_one(&state, source_id, &req.item).await?;

    let _ = tessera_db::repos::audit::record(
        &state.db.api,
        Some(&ctx.audit_id()),
        "ingest",
        Some(&result.document_id.to_string()),
        &json!({ "deduped": result.deduped }),
    )
    .await;
    Ok(Json(result))
}

#[derive(Debug, Serialize)]
struct BulkResponse {
    source_id: Uuid,
    accepted: usize,
    deduped: usize,
    failed: usize,
    results: Vec<Value>,
}

async fn ingest_bulk(
    State(state): State<AppState>,
    ctx: AuthContext,
    body: String,
) -> Result<Json<BulkResponse>, ApiError> {
    ctx.require(Scope::Ingest)?;
    check_backpressure(&state).await?;

    let source_id =
        resolve_source(&state, &ctx, None, Some("bulk ingest"), source_kind(&ctx)).await?;
    let mut results = Vec::new();
    let (mut accepted, mut deduped, mut failed) = (0usize, 0usize, 0usize);

    for (line_no, line) in body.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        match serde_json::from_str::<IngestItem>(line) {
            Ok(item) => match ingest_one(&state, source_id, &item).await {
                Ok(r) => {
                    if r.deduped {
                        deduped += 1;
                    } else {
                        accepted += 1;
                    }
                    results.push(json!({ "line": line_no, "document_id": r.document_id, "deduped": r.deduped }));
                }
                Err(e) => {
                    failed += 1;
                    results.push(json!({ "line": line_no, "error": e.to_string() }));
                }
            },
            Err(e) => {
                failed += 1;
                results.push(json!({ "line": line_no, "error": format!("invalid json: {e}") }));
            }
        }
    }

    let _ = tessera_db::repos::audit::record(
        &state.db.api,
        Some(&ctx.audit_id()),
        "ingest.bulk",
        Some(&source_id.to_string()),
        &json!({ "accepted": accepted, "deduped": deduped, "failed": failed }),
    )
    .await;

    Ok(Json(BulkResponse {
        source_id,
        accepted,
        deduped,
        failed,
        results,
    }))
}

async fn ingest_upload(
    State(state): State<AppState>,
    ctx: AuthContext,
    mut multipart: Multipart,
) -> Result<Json<BulkResponse>, ApiError> {
    ctx.require(Scope::Ingest)?;
    check_backpressure(&state).await?;

    let source_id = resolve_source(&state, &ctx, None, Some("upload"), "upload").await?;
    let mut results = Vec::new();
    let (mut accepted, mut deduped, mut failed) = (0usize, 0usize, 0usize);

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| ApiError(Error::new(ErrorKind::Invalid, format!("multipart: {e}"))))?
    {
        let filename = field.file_name().map(ToString::to_string);
        let content_type = field.content_type().map(ToString::to_string);
        let data = field.bytes().await.map_err(|e| {
            ApiError(Error::new(
                ErrorKind::Invalid,
                format!("multipart field: {e}"),
            ))
        })?;

        let item = IngestItem {
            content: None,
            content_base64: Some(BASE64.encode(&data)),
            media_type: content_type,
            title: filename.clone(),
            uri: None,
            meta: Some(json!({ "filename": filename })),
        };
        match ingest_one(&state, source_id, &item).await {
            Ok(r) => {
                if r.deduped {
                    deduped += 1;
                } else {
                    accepted += 1;
                }
                results.push(json!({ "filename": filename, "document_id": r.document_id, "deduped": r.deduped }));
            }
            Err(e) => {
                failed += 1;
                results.push(json!({ "filename": filename, "error": e.to_string() }));
            }
        }
    }

    Ok(Json(BulkResponse {
        source_id,
        accepted,
        deduped,
        failed,
        results,
    }))
}

#[derive(Debug, Deserialize)]
struct UrlRequest {
    url: String,
    #[serde(default)]
    source_name: Option<String>,
}

async fn ingest_url(
    State(state): State<AppState>,
    ctx: AuthContext,
    Json(req): Json<UrlRequest>,
) -> Result<Json<IngestResult>, ApiError> {
    ctx.require(Scope::Ingest)?;
    check_backpressure(&state).await?;

    // Fetch through the SSRF guard (validates scheme, resolves and rejects
    // private/loopback/link-local/tailnet addresses, caps size and redirects).
    let fetched = crate::url_guard::fetch(&req.url).await?;
    let media_type = fetched
        .content_type
        .as_deref()
        .map(|ct| ct.split(';').next().unwrap_or(ct).trim().to_string());

    let source_id = resolve_source(
        &state,
        &ctx,
        None,
        req.source_name.as_deref().or(Some("url")),
        "url",
    )
    .await?;

    let item = IngestItem {
        content: None,
        content_base64: Some(BASE64.encode(&fetched.bytes)),
        media_type,
        title: Some(fetched.final_url.clone()),
        uri: Some(fetched.final_url),
        meta: Some(json!({ "fetched_from": req.url })),
    };
    let result = ingest_one(&state, source_id, &item).await?;

    let _ = tessera_db::repos::audit::record(
        &state.db.api,
        Some(&ctx.audit_id()),
        "ingest.url",
        Some(&result.document_id.to_string()),
        &json!({ "url": req.url, "deduped": result.deduped }),
    )
    .await;
    Ok(Json(result))
}

/// The source kind to record based on who is ingesting.
fn source_kind(ctx: &AuthContext) -> &'static str {
    match &ctx.principal {
        crate::auth::Principal::User => "upload",
        crate::auth::Principal::Token { .. } => "api",
    }
}
