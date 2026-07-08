//! Stage handlers. Each is idempotent: re-running it after a crash converges to
//! the same state (chunks are unique on `(document_id, seq)`, embeddings on
//! `(chunk_id, space_id)`, and both insert with `ON CONFLICT DO NOTHING`).

use serde_json::{json, Value};
use tessera_core::error::{Error, ErrorKind, Result};
use tessera_core::ContentHash;
use tessera_db::queue::{self, EnqueueOpts};
use tessera_db::repos::{chunks, documents};
use tessera_providers::EmbedKind;
use uuid::Uuid;

use crate::context::PipelineContext;
use crate::{KIND_EMBED_CHUNKS, KIND_PROCESS_DOCUMENT};

/// Dispatch a claimed job to its handler.
pub async fn dispatch(ctx: &PipelineContext, kind: &str, payload: &Value) -> Result<()> {
    match kind {
        KIND_PROCESS_DOCUMENT => process_document(ctx, payload).await,
        KIND_EMBED_CHUNKS => embed_chunks(ctx, payload).await,
        other => Err(Error::new(
            ErrorKind::Invalid,
            format!("unknown job kind: {other}"),
        )),
    }
}

fn payload_uuid(payload: &Value, key: &str) -> Result<Uuid> {
    payload
        .get(key)
        .and_then(Value::as_str)
        .and_then(|s| Uuid::parse_str(s).ok())
        .ok_or_else(|| Error::new(ErrorKind::Invalid, format!("job payload missing {key}")))
}

/// Normalize + chunk a document, then fan out embedding jobs for its chunks.
async fn process_document(ctx: &PipelineContext, payload: &Value) -> Result<()> {
    let document_id = payload_uuid(payload, "document_id")?;
    let pool = &ctx.db.worker;

    let Some(doc) = documents::get(pool, document_id).await? else {
        // The document was removed; nothing to do (idempotent no-op).
        return Ok(());
    };

    // Read the raw bytes back from the content store, verifying integrity.
    let hash = ContentHash::from_slice(&doc.content_hash)?;
    let bytes = ctx.cas.read_verified(&hash).await?;

    // Sniff (using the stored media type as a hint) and normalize.
    let sniffed = tessera_extract::sniff(&bytes, Some(&doc.media_type));
    let prepared = match tessera_extract::normalize(&bytes, &sniffed) {
        Ok(p) => p,
        Err(e) => {
            // Unsupported or malformed content is a document-level failure, not a
            // job crash: record it and stop cleanly.
            documents::set_status(pool, document_id, "failed", Some(&e.to_string()), true).await?;
            let _ = queue::notify(
                pool,
                &json!({ "type": "document.failed", "document_id": document_id, "error": e.to_string() }),
            )
            .await;
            return Ok(());
        }
    };

    if let Some(title) = &prepared.title {
        documents::set_title_if_absent(pool, document_id, title).await?;
    }

    let inputs: Vec<chunks::ChunkInput> = prepared
        .chunks
        .iter()
        .enumerate()
        .map(|(i, c)| chunks::ChunkInput {
            seq: i32::try_from(i).unwrap_or(i32::MAX),
            text: c.text.clone(),
            token_count: i32::try_from(c.token_count).unwrap_or(i32::MAX),
        })
        .collect();

    chunks::insert_batch(pool, document_id, &inputs).await?;

    // Empty document (no extractable text): it is done.
    if inputs.is_empty() {
        documents::set_status(pool, document_id, "ready", None, true).await?;
        let _ = queue::notify(
            pool,
            &json!({ "type": "document.ready", "document_id": document_id, "chunks": 0, "embedded": 0 }),
        )
        .await;
        return Ok(());
    }

    // Which chunks still need embedding in the active space?
    let pending = chunks::ids_without_embedding(pool, document_id, ctx.space_id).await?;

    // Idempotent re-run: if a redelivered job finds everything already embedded,
    // converge straight to ready instead of leaving the doc stuck at 'processing'
    // (no embed job would be enqueued to flip it back).
    if pending.is_empty() {
        documents::set_status(pool, document_id, "ready", None, true).await?;
        let (total, embedded) = chunks::embedding_progress(pool, document_id, ctx.space_id).await?;
        let _ = queue::notify(
            pool,
            &json!({ "type": "document.ready", "document_id": document_id, "chunks": total, "embedded": embedded }),
        )
        .await;
        return Ok(());
    }

    documents::set_status(pool, document_id, "processing", None, false).await?;

    // Fan out embedding jobs for the chunks that still need it.
    for batch in pending.chunks(ctx.embed_batch) {
        let ids: Vec<String> = batch.iter().map(ToString::to_string).collect();
        queue::enqueue(
            pool,
            KIND_EMBED_CHUNKS,
            &json!({ "document_id": document_id, "chunk_ids": ids }),
            &EnqueueOpts {
                priority: 0,
                ..Default::default()
            },
        )
        .await?;
    }

    let _ = queue::notify(
        pool,
        &json!({ "type": "document.processing", "document_id": document_id, "chunks": inputs.len() }),
    )
    .await;
    Ok(())
}

/// Embed a batch of chunks into the active space, then, if the document is now
/// fully embedded, mark it ready.
async fn embed_chunks(ctx: &PipelineContext, payload: &Value) -> Result<()> {
    let document_id = payload_uuid(payload, "document_id")?;
    let pool = &ctx.db.worker;

    let chunk_ids: Vec<Uuid> = payload
        .get("chunk_ids")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str())
                .filter_map(|s| Uuid::parse_str(s).ok())
                .collect()
        })
        .unwrap_or_default();
    if chunk_ids.is_empty() {
        return Ok(());
    }

    let pairs = chunks::texts_for(pool, &chunk_ids).await?;
    if pairs.is_empty() {
        return Ok(());
    }
    let texts: Vec<String> = pairs.iter().map(|(_, t)| t.clone()).collect();

    let vectors = ctx
        .embedder
        .embed(&texts, EmbedKind::Document)
        .await
        .map_err(|e| Error::new(ErrorKind::Provider, format!("embed: {e}")))?;
    if vectors.len() != pairs.len() {
        return Err(Error::new(
            ErrorKind::Provider,
            "embedder returned wrong batch size",
        ));
    }

    let rows: Vec<(Uuid, Vec<f32>)> = pairs.iter().map(|(id, _)| *id).zip(vectors).collect();
    tessera_db::repos::embeddings::insert_batch(pool, ctx.space_id, &rows).await?;

    // Progress + readiness. Whichever embed job observes the document fully
    // embedded flips it to ready (set_status is idempotent).
    let (total, embedded) = chunks::embedding_progress(pool, document_id, ctx.space_id).await?;
    let _ = queue::notify(
        pool,
        &json!({
            "type": "embed.progress",
            "document_id": document_id,
            "embedded": embedded,
            "total": total,
        }),
    )
    .await;

    if total > 0 && embedded >= total {
        documents::set_status(pool, document_id, "ready", None, true).await?;
        let _ = queue::notify(
            pool,
            &json!({
                "type": "document.ready",
                "document_id": document_id,
                "chunks": total,
                "embedded": embedded,
            }),
        )
        .await;
    }
    Ok(())
}
