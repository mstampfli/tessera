//! Stage handlers. Each is idempotent: re-running it after a crash converges to
//! the same state (chunks are unique on `(document_id, seq)`, embeddings on
//! `(chunk_id, space_id)`, and both insert with `ON CONFLICT DO NOTHING`).

use std::collections::{HashMap, HashSet};

use serde_json::{json, Value};
use tessera_core::error::{Error, ErrorKind, Result};
use tessera_core::ContentHash;
use tessera_db::queue::{self, EnqueueOpts};
use tessera_db::repos::clusters::MemberChunk;
use tessera_db::repos::insights::{EvidenceInput, InsightInput};
use tessera_db::repos::{chunks, clusters, documents, embeddings, entities, insights};
use tessera_providers::EmbedKind;
use uuid::Uuid;

use crate::context::PipelineContext;
use crate::synth;
use crate::{
    KIND_ASSIGN_CLUSTERS, KIND_CORRELATE_ENTITIES, KIND_DETECT_COMMUNITIES, KIND_EMBED_CHUNKS,
    KIND_EXTRACT_ENTITIES, KIND_PROCESS_DOCUMENT, KIND_RECLUSTER, KIND_SYNTHESIZE_INSIGHT,
};

/// Dispatch a claimed job to its handler.
pub async fn dispatch(ctx: &PipelineContext, kind: &str, payload: &Value) -> Result<()> {
    match kind {
        KIND_PROCESS_DOCUMENT => process_document(ctx, payload).await,
        KIND_EMBED_CHUNKS => embed_chunks(ctx, payload).await,
        KIND_EXTRACT_ENTITIES => extract_entities(ctx, payload).await,
        KIND_ASSIGN_CLUSTERS => assign_clusters(ctx, payload).await,
        KIND_SYNTHESIZE_INSIGHT => synthesize_insight(ctx, payload).await,
        KIND_CORRELATE_ENTITIES => correlate_entities(ctx, payload).await,
        KIND_DETECT_COMMUNITIES => detect_communities(ctx, payload).await,
        KIND_RECLUSTER => recluster(ctx, payload).await,
        other => Err(Error::new(
            ErrorKind::Invalid,
            format!("unknown job kind: {other}"),
        )),
    }
}

/// Authoritative HDBSCAN recluster over the whole space. Global and debounced, so
/// a burst of ingestion collapses to one pass that corrects the online-centroid
/// provisional assignment into density clusters.
async fn recluster(ctx: &PipelineContext, _payload: &Value) -> Result<()> {
    let r = crate::recluster::run(
        &ctx.db,
        ctx.space_id,
        ctx.cluster_min_size,
        ctx.synth_debounce_secs,
    )
    .await?;
    let _ = queue::notify(
        &ctx.db.worker,
        &json!({ "type": "reclustered", "clusters": r.clusters, "noise": r.noise, "changed": r.changed }),
    )
    .await;
    Ok(())
}

/// Recompute entity communities across the whole KB. Global and debounced (one
/// run collapses a burst of ingestion), so it runs after correlation settles.
async fn detect_communities(ctx: &PipelineContext, _payload: &Value) -> Result<()> {
    let pool = &ctx.db.worker;
    let n = tessera_db::repos::communities::detect(pool).await?;
    let _ = queue::notify(
        pool,
        &json!({ "type": "communities.detected", "communities": n }),
    )
    .await;
    Ok(())
}

/// Materialize the document's entity embeddings and (re)compute their global
/// semantic-similarity edges across the whole KB. Enqueued after both entity
/// extraction and embedding, and idempotent: it recomputes from current state,
/// so running before every chunk is embedded just converges on a later pass.
async fn correlate_entities(ctx: &PipelineContext, payload: &Value) -> Result<()> {
    let document_id = payload_uuid(payload, "document_id")?;
    let pool = &ctx.db.worker;

    let ids = entities::ids_for_document(pool, document_id).await?;
    if ids.is_empty() {
        return Ok(());
    }
    // Update each entity's context embedding from its (currently embedded) chunks,
    // then link it to its global nearest neighbours. Add-only per entity; a
    // full rebuild (the backfill CLI) prunes edges made stale by embedding drift.
    entities::recompute_entity_embeddings(pool, ctx.space_id, &ids).await?;
    let day = 86_400.0;
    let window = ctx.temporal_window_days * day;
    let tau = ctx.temporal_tau_days * day;
    let mut linked = 0u64;
    for id in &ids {
        linked += entities::add_similar_edges(
            pool,
            ctx.space_id,
            ctx.space_dim,
            *id,
            ctx.semantic_k,
            ctx.semantic_min_sim,
        )
        .await?;
        // Temporal edges (no-op unless documents carry event_time).
        linked += entities::add_temporal_edges(pool, *id, window, tau, ctx.semantic_k).await?;
    }

    // Communities depend on the co-occurrence graph, which extraction already
    // updated; recompute them globally, debounced so a burst collapses to one run.
    queue::enqueue(
        pool,
        KIND_DETECT_COMMUNITIES,
        &json!({}),
        &EnqueueOpts {
            dedupe_key: Some("detect_communities".to_string()),
            delay_secs: Some(ctx.synth_debounce_secs),
            ..Default::default()
        },
    )
    .await?;

    let _ = queue::notify(
        pool,
        &json!({ "type": "entities.correlated", "document_id": document_id, "edges": linked }),
    )
    .await;
    Ok(())
}

/// If the caller did not supply an event time, read the earliest date from the
/// document's leading text (bounded) and set it, so temporal correlation has an
/// axis. A no-op when the content carries no date.
async fn auto_event_time(
    pool: &tessera_db::Pool,
    document_id: Uuid,
    prepared: &tessera_extract::Prepared,
) -> Result<()> {
    let sample: String = prepared
        .chunks
        .iter()
        .map(|c| c.text.as_str())
        .take(8)
        .collect::<Vec<_>>()
        .join("\n");
    if let Some(event_time) = tessera_extract::dates::extract_earliest(&sample) {
        documents::set_event_time_if_absent(pool, document_id, event_time).await?;
    }
    Ok(())
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

    // Sniff (using the stored media type as a hint), then normalize. A configured
    // plugin that handles this media type takes precedence over the built-ins.
    let sniffed = tessera_extract::sniff(&bytes, Some(&doc.media_type));
    let prepared_result =
        if let Some(manifest) = ctx.plugins.find(&sniffed.media_type, &sniffed.label) {
            tessera_extract::plugin::run_plugin(manifest, &bytes)
                .await
                .map(tessera_extract::extractors::events_to_prepared)
                .map_err(|e| Error::new(ErrorKind::Extract, e.to_string()))
        } else {
            tessera_extract::normalize(&bytes, &sniffed)
                .map_err(|e| Error::new(ErrorKind::Extract, e.to_string()))
        };
    let prepared = match prepared_result {
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
    auto_event_time(pool, document_id, &prepared).await?;

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

    // Entity extraction is independent of embedding (it only needs the chunks),
    // so enqueue it now regardless of embedding state. The dedupe key drops
    // duplicate enqueues while one is queued/running.
    queue::enqueue(
        pool,
        KIND_EXTRACT_ENTITIES,
        &json!({ "document_id": document_id }),
        &EnqueueOpts {
            dedupe_key: Some(format!("extract:{document_id}")),
            ..Default::default()
        },
    )
    .await?;

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

/// Extract security entities from a document's chunks, record mentions, and
/// recompute its correlation edges. Fully idempotent: entity upserts dedup on
/// `(kind, value)`, mentions on `(entity_id, chunk_id, span)`, and co-occurrence
/// is recomputed (not incremented).
async fn extract_entities(ctx: &PipelineContext, payload: &Value) -> Result<()> {
    let document_id = payload_uuid(payload, "document_id")?;
    let pool = &ctx.db.worker;

    let doc_chunks = chunks::list_by_document(pool, document_id).await?;
    if doc_chunks.is_empty() {
        return Ok(());
    }

    // Cache entity ids by (kind, value) so a value seen across many chunks is
    // upserted once.
    let mut cache: HashMap<(&'static str, String), Uuid> = HashMap::new();
    let mut affected: HashSet<Uuid> = HashSet::new();

    for chunk in &doc_chunks {
        for m in tessera_extract::security::extract(&chunk.text) {
            let key = (m.kind, m.value.clone());
            let entity_id = if let Some(id) = cache.get(&key) {
                *id
            } else {
                let id = entities::upsert(pool, m.kind, &m.value, &m.raw).await?;
                cache.insert(key, id);
                id
            };
            affected.insert(entity_id);

            let span = Some((
                i32::try_from(m.start).unwrap_or(0),
                i32::try_from(m.end).unwrap_or(0),
            ));
            entities::insert_mention(
                pool,
                entity_id,
                chunk.id,
                document_id,
                &m.raw,
                span,
                "security",
                1.0,
            )
            .await?;
        }
    }

    let ids: Vec<Uuid> = affected.into_iter().collect();
    entities::recompute_mention_counts(pool, &ids).await?;
    entities::recompute_cooccurrence(pool, document_id).await?;

    // Entities now exist; correlate them once their chunks are embedded. Enqueued
    // from here and from embed completion so whichever finishes last does the work.
    queue::enqueue(
        pool,
        KIND_CORRELATE_ENTITIES,
        &json!({ "document_id": document_id }),
        &EnqueueOpts::default(),
    )
    .await?;

    let _ = queue::notify(
        pool,
        &json!({ "type": "entities.extracted", "document_id": document_id, "entities": ids.len() }),
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
    tessera_db::repos::embeddings::insert_batch(pool, ctx.space_id, ctx.space_dim, &rows).await?;

    // Assign the freshly embedded chunks to clusters.
    let embedded_ids: Vec<String> = rows.iter().map(|(id, _)| id.to_string()).collect();
    queue::enqueue(
        pool,
        KIND_ASSIGN_CLUSTERS,
        &json!({ "document_id": document_id, "chunk_ids": embedded_ids }),
        &EnqueueOpts::default(),
    )
    .await?;

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
        // All chunks embedded: (re)correlate this document's entities globally.
        queue::enqueue(
            pool,
            KIND_CORRELATE_ENTITIES,
            &json!({ "document_id": document_id }),
            &EnqueueOpts::default(),
        )
        .await?;
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

/// Assign a batch of freshly embedded chunks to clusters (online nearest
/// centroid, or seed a new cluster). Enqueues insight synthesis for clusters
/// that have accumulated enough new members. Idempotent: a chunk already in a
/// cluster is skipped.
async fn assign_clusters(ctx: &PipelineContext, payload: &Value) -> Result<()> {
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

    let mut dirty: HashSet<Uuid> = HashSet::new();

    for chunk_id in chunk_ids {
        // Idempotent: skip chunks already assigned.
        if clusters::member_cluster(pool, chunk_id).await?.is_some() {
            continue;
        }
        let Some(vec) = embeddings::get_vector(pool, chunk_id, ctx.space_id).await? else {
            continue;
        };

        let cluster_id = match clusters::nearest(pool, ctx.space_id, &vec).await? {
            Some((cid, dist)) if dist <= ctx.cluster_max_distance => {
                clusters::assign(pool, cid, chunk_id, similarity(dist), ctx.space_id).await?;
                cid
            }
            _ => {
                // No cluster close enough: create one under the per-space advisory
                // lock, re-checking after the lock so concurrent workers cannot
                // birth twin clusters for the same region.
                let mut tx = clusters::begin_locked(pool, ctx.space_id).await?;
                match clusters::nearest(pool, ctx.space_id, &vec).await? {
                    Some((cid, dist)) if dist <= ctx.cluster_max_distance => {
                        drop(tx); // release the lock; the winner already created it
                        clusters::assign(pool, cid, chunk_id, similarity(dist), ctx.space_id)
                            .await?;
                        cid
                    }
                    _ => {
                        let cid =
                            clusters::create(&mut *tx, ctx.space_id, &vec, chunk_id, 1.0).await?;
                        tx.commit().await.map_err(|e| {
                            Error::new(ErrorKind::Db, format!("cluster create: {e}"))
                        })?;
                        cid
                    }
                }
            }
        };
        dirty.insert(cluster_id);
    }

    // Debounced synthesis for clusters that gained enough new members.
    for cid in &dirty {
        if let Some(cluster) = clusters::get(pool, *cid).await? {
            if cluster.dirty_count >= ctx.cluster_dirty_threshold {
                queue::enqueue(
                    pool,
                    KIND_SYNTHESIZE_INSIGHT,
                    &json!({ "cluster_id": cid }),
                    &EnqueueOpts {
                        dedupe_key: Some(format!("synth:{cid}")),
                        delay_secs: Some(ctx.synth_debounce_secs),
                        ..Default::default()
                    },
                )
                .await?;
            }
        }
    }

    // The online assignment above is a fast provisional; enqueue a debounced
    // authoritative HDBSCAN recluster to correct it into density clusters.
    queue::enqueue(
        pool,
        KIND_RECLUSTER,
        &json!({}),
        &EnqueueOpts {
            dedupe_key: Some("recluster".to_string()),
            delay_secs: Some(ctx.synth_debounce_secs),
            ..Default::default()
        },
    )
    .await?;

    let _ = queue::notify(
        pool,
        &json!({ "type": "clusters.assigned", "document_id": document_id, "clusters": dirty.len() }),
    )
    .await;
    Ok(())
}

/// Cosine similarity from cosine distance, clamped to [0, 1] and narrowed to f32.
#[allow(clippy::cast_possible_truncation)]
fn similarity(distance: f64) -> f32 {
    (1.0 - distance).clamp(0.0, 1.0) as f32
}

/// Synthesize (or re-synthesize) the insight for a dirty cluster. Skips when the
/// cluster's content signature is unchanged since the last insight.
async fn synthesize_insight(ctx: &PipelineContext, payload: &Value) -> Result<()> {
    let cluster_id = payload_uuid(payload, "cluster_id")?;
    let pool = &ctx.db.worker;

    if clusters::get(pool, cluster_id).await?.is_none() {
        return Ok(());
    }
    let reps = clusters::representative_chunks(pool, cluster_id, ctx.space_id, 20).await?;
    if reps.is_empty() {
        return Ok(());
    }
    let entities = clusters::top_entities(pool, cluster_id, 15).await?;

    // Dedup: if the cluster's content signature is unchanged, do not re-synthesize.
    let input_hash = input_signature(cluster_id, &reps, &entities);
    if insights::live_input_hash(pool, cluster_id).await? == Some(input_hash.clone()) {
        clusters::reset_dirty(pool, cluster_id).await?;
        return Ok(());
    }

    let synth = synth::synthesize(&ctx.llm, &reps, &entities).await?;

    // Evidence comes from the cited context items; the citation leash means only
    // resolving markers count. If the model cited nothing, fall back to the most
    // representative chunk so the card is never uncorroborated.
    let mut evidence = Vec::new();
    if synth.cited.is_empty() {
        if let Some(c) = reps.first() {
            evidence.push(evidence_of(c));
        }
    } else {
        for marker in &synth.cited {
            if let Some(c) = reps.get(marker - 1) {
                evidence.push(evidence_of(c));
            }
        }
    }

    let actions = serde_json::to_value(&synth.suggested_actions).unwrap_or_else(|_| json!([]));
    insights::create(
        pool,
        &InsightInput {
            cluster_id,
            title: synth.title.clone(),
            body_md: synth.narrative,
            tags: Vec::new(),
            severity: synth.severity,
            confidence: synth.confidence,
            suggested_actions: actions,
            entity_ids: Vec::new(),
            model: synth.model,
            input_hash,
            evidence,
        },
    )
    .await?;

    clusters::set_label(pool, cluster_id, &synth.title).await?;
    clusters::reset_dirty(pool, cluster_id).await?;

    let _ = queue::notify(
        pool,
        &json!({ "type": "insight.created", "cluster_id": cluster_id }),
    )
    .await;
    Ok(())
}

fn evidence_of(c: &MemberChunk) -> EvidenceInput {
    EvidenceInput {
        chunk_id: c.chunk_id,
        document_id: c.document_id,
        entity_id: None,
        note: None,
    }
}

/// A stable signature of a cluster's synthesis inputs, used to skip
/// re-synthesizing when nothing material changed.
fn input_signature(
    cluster_id: Uuid,
    reps: &[MemberChunk],
    entities: &[(String, String)],
) -> Vec<u8> {
    let mut parts: Vec<String> = reps.iter().map(|c| c.chunk_id.to_string()).collect();
    parts.sort();
    let mut ent: Vec<String> = entities.iter().map(|(k, v)| format!("{k}:{v}")).collect();
    ent.sort();
    let material = format!("{cluster_id}|{}|{}", parts.join(","), ent.join(","));
    ContentHash::of(material.as_bytes()).as_bytes().to_vec()
}
