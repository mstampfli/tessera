//! The shared ingest core: sniff, store, record, enqueue. Both the REST API and
//! the MCP server call this so there is exactly one ingestion path. Content held
//! in memory uses [`ingest_bytes`]; content arriving as a stream (an upload) uses
//! [`ingest_stream`] so the whole body is never buffered. Both share the same
//! record-and-enqueue tail.

use serde_json::Value;
use tessera_core::error::{Error, ErrorKind, Result};
use tessera_core::ContentHash;
use tessera_db::cas::CasStore;
use tessera_db::queue::{self, EnqueueOpts};
use tessera_db::repos::documents;
use tessera_db::Db;
use uuid::Uuid;

use crate::KIND_PROCESS_DOCUMENT;

/// Bytes of the content head kept, when streaming, for sniffing the type.
const SNIFF_HEAD: usize = 8 * 1024;

/// The outcome of ingesting one item.
pub struct Ingested {
    pub document_id: Uuid,
    pub deduped: bool,
    pub status: String,
}

/// Fields for one in-memory ingestion.
pub struct IngestBytes<'a> {
    pub source_id: Uuid,
    pub bytes: &'a [u8],
    /// Client-declared media type (advisory; sniffing is authoritative).
    pub media_type_hint: Option<&'a str>,
    pub title: Option<&'a str>,
    pub uri: Option<&'a str>,
    pub meta: Value,
    /// When the event happened (caller-provided); auto-extracted from content
    /// during processing if absent.
    pub event_time: Option<chrono::DateTime<chrono::Utc>>,
}

/// Fields for one streaming ingestion (the bytes come from the reader).
pub struct IngestStream<'a> {
    pub source_id: Uuid,
    pub media_type_hint: Option<&'a str>,
    pub title: Option<&'a str>,
    pub uri: Option<&'a str>,
    pub meta: Value,
    pub event_time: Option<chrono::DateTime<chrono::Utc>>,
}

/// Store bytes already in memory, then record and enqueue. Idempotent by content
/// hash.
pub async fn ingest_bytes(db: &Db, cas: &CasStore, item: IngestBytes<'_>) -> Result<Ingested> {
    if item.bytes.is_empty() {
        return Err(Error::new(ErrorKind::Invalid, "empty content"));
    }
    let sniffed = tessera_extract::sniff(item.bytes, item.media_type_hint);
    let (hash, size) = cas.write_bytes(item.bytes).await?;
    record_and_enqueue(
        db,
        item.source_id,
        &hash,
        size,
        &sniffed.media_type,
        item.title,
        item.uri,
        &item.meta,
        item.event_time,
    )
    .await
}

/// Stream content from `reader` into the content store (never buffering the whole
/// body), capped at `max_bytes`, then record and enqueue. Idempotent by content
/// hash, exactly like [`ingest_bytes`].
pub async fn ingest_stream<R: tokio::io::AsyncRead + Unpin>(
    db: &Db,
    cas: &CasStore,
    reader: R,
    item: IngestStream<'_>,
    max_bytes: u64,
) -> Result<Ingested> {
    let stored = cas.write_streaming(reader, max_bytes, SNIFF_HEAD).await?;
    if stored.size == 0 {
        return Err(Error::new(ErrorKind::Invalid, "empty content"));
    }
    let sniffed = tessera_extract::sniff(&stored.head, item.media_type_hint);
    record_and_enqueue(
        db,
        item.source_id,
        &stored.hash,
        stored.size,
        &sniffed.media_type,
        item.title,
        item.uri,
        &item.meta,
        item.event_time,
    )
    .await
}

/// Record the document and enqueue processing in one transaction (exactly-once
/// handoff). Shared by the in-memory and streaming ingest paths.
#[allow(clippy::too_many_arguments)]
async fn record_and_enqueue(
    db: &Db,
    source_id: Uuid,
    hash: &ContentHash,
    size: u64,
    media_type: &str,
    title: Option<&str>,
    uri: Option<&str>,
    meta: &Value,
    event_time: Option<chrono::DateTime<chrono::Utc>>,
) -> Result<Ingested> {
    let mut tx = db.api.begin().await.map_err(tessera_db::map_sqlx)?;
    let outcome = documents::create_pending_tx(
        &mut tx,
        &documents::NewDocument {
            source_id,
            content_hash: hash,
            media_type,
            size_bytes: i64::try_from(size).unwrap_or(i64::MAX),
            title,
            uri,
            meta,
            event_time,
        },
    )
    .await?;

    if !outcome.deduped {
        queue::enqueue(
            &mut *tx,
            KIND_PROCESS_DOCUMENT,
            &serde_json::json!({ "document_id": outcome.document.id }),
            &EnqueueOpts::default(),
        )
        .await?;
    }
    tx.commit().await.map_err(tessera_db::map_sqlx)?;

    Ok(Ingested {
        document_id: outcome.document.id,
        deduped: outcome.deduped,
        status: outcome.document.status,
    })
}
