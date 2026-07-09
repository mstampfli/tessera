//! The shared ingest core: sniff, store, record, enqueue. Both the REST API and
//! the MCP server call this so there is exactly one ingestion path.

use serde_json::Value;
use tessera_core::error::{Error, ErrorKind, Result};
use tessera_db::cas::CasStore;
use tessera_db::queue::{self, EnqueueOpts};
use tessera_db::repos::documents;
use tessera_db::Db;
use uuid::Uuid;

use crate::KIND_PROCESS_DOCUMENT;

/// The outcome of ingesting one item.
pub struct Ingested {
    pub document_id: Uuid,
    pub deduped: bool,
    pub status: String,
}

/// Fields for one ingestion.
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

/// Store bytes in the content store (deduped by hash), record the document, and
/// enqueue processing in the same transaction (exactly-once handoff). This is the
/// cheap synchronous part; the pipeline does the heavy work.
pub async fn ingest_bytes(db: &Db, cas: &CasStore, item: IngestBytes<'_>) -> Result<Ingested> {
    if item.bytes.is_empty() {
        return Err(Error::new(ErrorKind::Invalid, "empty content"));
    }

    let sniffed = tessera_extract::sniff(item.bytes, item.media_type_hint);
    let (hash, size) = cas.write_bytes(item.bytes).await?;

    let mut tx = db.api.begin().await.map_err(tessera_db::map_sqlx)?;
    let outcome = documents::create_pending_tx(
        &mut tx,
        &documents::NewDocument {
            source_id: item.source_id,
            content_hash: &hash,
            media_type: &sniffed.media_type,
            size_bytes: i64::try_from(size).unwrap_or(i64::MAX),
            title: item.title,
            uri: item.uri,
            meta: &item.meta,
            event_time: item.event_time,
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
