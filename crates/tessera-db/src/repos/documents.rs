//! The `documents` repository. A document is one ingested item; its blake3
//! content hash is the CAS key and the idempotency anchor.

use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::PgPool;
use tessera_core::error::Result;
use tessera_core::ContentHash;
use uuid::Uuid;

use crate::map_sqlx;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Document {
    pub id: Uuid,
    pub source_id: Uuid,
    pub content_hash: Vec<u8>,
    pub media_type: String,
    pub size_bytes: i64,
    pub title: Option<String>,
    pub uri: Option<String>,
    pub meta: Value,
    pub status: String,
    pub error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub processed_at: Option<DateTime<Utc>>,
}

/// The outcome of an ingest attempt.
pub struct IngestOutcome {
    pub document: Document,
    /// True when this exact content was already present (idempotent no-op).
    pub deduped: bool,
}

/// Fields needed to record a new document.
pub struct NewDocument<'a> {
    pub source_id: Uuid,
    pub content_hash: &'a ContentHash,
    pub media_type: &'a str,
    pub size_bytes: i64,
    pub title: Option<&'a str>,
    pub uri: Option<&'a str>,
    pub meta: &'a Value,
}

/// Insert a pending document, deduping on content hash. Returns the row and
/// whether it was newly created. On a fresh insert this enqueues nothing; the
/// caller enqueues the processing job in the same transaction for exactly-once
/// handoff (see [`create_pending_tx`]).
pub async fn create_pending(pool: &PgPool, doc: &NewDocument<'_>) -> Result<IngestOutcome> {
    let mut tx = pool.begin().await.map_err(map_sqlx)?;
    let outcome = create_pending_tx(&mut tx, doc).await?;
    tx.commit().await.map_err(map_sqlx)?;
    Ok(outcome)
}

/// Transactional variant so the caller can enqueue the processing job in the
/// same transaction as the insert.
pub async fn create_pending_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    doc: &NewDocument<'_>,
) -> Result<IngestOutcome> {
    let id = tessera_core::new_id();
    let hash = doc.content_hash.as_bytes().as_slice();

    // Try to insert; on hash conflict, do nothing and fetch the existing row.
    let inserted = sqlx::query_as::<_, Document>(
        "INSERT INTO documents
            (id, source_id, content_hash, media_type, size_bytes, title, uri, meta)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
         ON CONFLICT (content_hash) DO NOTHING
         RETURNING id, source_id, content_hash, media_type, size_bytes, title, uri, meta,
                   status, error, created_at, processed_at",
    )
    .bind(id)
    .bind(doc.source_id)
    .bind(hash)
    .bind(doc.media_type)
    .bind(doc.size_bytes)
    .bind(doc.title)
    .bind(doc.uri)
    .bind(doc.meta)
    .fetch_optional(&mut **tx)
    .await
    .map_err(map_sqlx)?;

    if let Some(document) = inserted {
        return Ok(IngestOutcome {
            document,
            deduped: false,
        });
    }

    // Conflict: the content already exists. Return the existing row.
    let existing = sqlx::query_as::<_, Document>(
        "SELECT id, source_id, content_hash, media_type, size_bytes, title, uri, meta,
                status, error, created_at, processed_at
         FROM documents WHERE content_hash = $1",
    )
    .bind(hash)
    .fetch_one(&mut **tx)
    .await
    .map_err(map_sqlx)?;
    Ok(IngestOutcome {
        document: existing,
        deduped: true,
    })
}

pub async fn get(pool: &PgPool, id: Uuid) -> Result<Option<Document>> {
    sqlx::query_as::<_, Document>(
        "SELECT id, source_id, content_hash, media_type, size_bytes, title, uri, meta,
                status, error, created_at, processed_at
         FROM documents WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(map_sqlx)
}

/// Set a document's status (and optional error / processed timestamp).
pub async fn set_status(
    pool: &PgPool,
    id: Uuid,
    status: &str,
    error: Option<&str>,
    processed: bool,
) -> Result<()> {
    sqlx::query(
        "UPDATE documents
         SET status = $2, error = $3,
             processed_at = CASE WHEN $4 THEN now() ELSE processed_at END
         WHERE id = $1",
    )
    .bind(id)
    .bind(status)
    .bind(error)
    .bind(processed)
    .execute(pool)
    .await
    .map_err(map_sqlx)?;
    Ok(())
}

/// Set a document's title only if it does not already have one (an extractor may
/// discover a better title than the ingest request supplied).
pub async fn set_title_if_absent(pool: &PgPool, id: Uuid, title: &str) -> Result<()> {
    sqlx::query("UPDATE documents SET title = $2 WHERE id = $1 AND title IS NULL")
        .bind(id)
        .bind(title)
        .execute(pool)
        .await
        .map_err(map_sqlx)?;
    Ok(())
}

/// List documents in a source, newest-first, keyset paginated.
pub async fn list_by_source(
    pool: &PgPool,
    source_id: Uuid,
    before: Option<Uuid>,
    limit: i64,
) -> Result<Vec<Document>> {
    sqlx::query_as::<_, Document>(
        "SELECT id, source_id, content_hash, media_type, size_bytes, title, uri, meta,
                status, error, created_at, processed_at
         FROM documents
         WHERE source_id = $1 AND ($2::uuid IS NULL OR id < $2)
         ORDER BY id DESC LIMIT $3",
    )
    .bind(source_id)
    .bind(before)
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(map_sqlx)
}
