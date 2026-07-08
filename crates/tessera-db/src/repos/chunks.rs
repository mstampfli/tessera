//! The `chunks` repository. A chunk is one embeddable/searchable unit of a
//! document. Inserts are idempotent on `(document_id, seq)`, so re-running the
//! processing stage after a crash converges rather than duplicating.

use serde_json::Value;
use sqlx::PgPool;
use tessera_core::error::Result;
use uuid::Uuid;

use crate::map_sqlx;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Chunk {
    pub id: Uuid,
    pub document_id: Uuid,
    pub seq: i32,
    pub text: String,
    pub token_count: i32,
    pub meta: Value,
}

/// One chunk to insert.
pub struct ChunkInput {
    pub seq: i32,
    pub text: String,
    pub token_count: i32,
}

/// Bulk-insert chunks for a document in one statement (UNNEST). Idempotent:
/// existing `(document_id, seq)` rows are left untouched. Returns the number of
/// rows newly inserted.
pub async fn insert_batch(pool: &PgPool, document_id: Uuid, chunks: &[ChunkInput]) -> Result<u64> {
    if chunks.is_empty() {
        return Ok(0);
    }
    let ids: Vec<Uuid> = chunks.iter().map(|_| tessera_core::new_id()).collect();
    let seqs: Vec<i32> = chunks.iter().map(|c| c.seq).collect();
    let texts: Vec<String> = chunks.iter().map(|c| c.text.clone()).collect();
    let tokens: Vec<i32> = chunks.iter().map(|c| c.token_count).collect();

    let affected = sqlx::query(
        "INSERT INTO chunks (id, document_id, seq, text, token_count)
         SELECT u.id, $2, u.seq, u.text, u.tc
         FROM UNNEST($1::uuid[], $3::int[], $4::text[], $5::int[]) AS u(id, seq, text, tc)
         ON CONFLICT (document_id, seq) DO NOTHING",
    )
    .bind(&ids)
    .bind(document_id)
    .bind(&seqs)
    .bind(&texts)
    .bind(&tokens)
    .execute(pool)
    .await
    .map_err(map_sqlx)?
    .rows_affected();
    Ok(affected)
}

/// Ids of this document's chunks that have no embedding in `space_id` yet. This
/// is what the embedding stage enqueues, so embedding is idempotent and
/// resumable regardless of how far a prior run got.
pub async fn ids_without_embedding(
    pool: &PgPool,
    document_id: Uuid,
    space_id: i16,
) -> Result<Vec<Uuid>> {
    sqlx::query_scalar::<_, Uuid>(
        "SELECT c.id FROM chunks c
         WHERE c.document_id = $1
           AND NOT EXISTS (
             SELECT 1 FROM chunk_embeddings e
             WHERE e.chunk_id = c.id AND e.space_id = $2)
         ORDER BY c.seq",
    )
    .bind(document_id)
    .bind(space_id)
    .fetch_all(pool)
    .await
    .map_err(map_sqlx)
}

/// Fetch the text of the given chunk ids, preserving the input order.
pub async fn texts_for(pool: &PgPool, ids: &[Uuid]) -> Result<Vec<(Uuid, String)>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    // Join against the input array with ordinality so the result matches the
    // requested order (the embedding batch must line up with its chunk ids).
    sqlx::query_as::<_, (Uuid, String)>(
        "SELECT c.id, c.text FROM chunks c
         JOIN UNNEST($1::uuid[]) WITH ORDINALITY AS u(id, ord) ON u.id = c.id
         ORDER BY u.ord",
    )
    .bind(ids)
    .fetch_all(pool)
    .await
    .map_err(map_sqlx)
}

pub async fn get(pool: &PgPool, id: Uuid) -> Result<Option<Chunk>> {
    sqlx::query_as::<_, Chunk>(
        "SELECT id, document_id, seq, text, token_count, meta FROM chunks WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(map_sqlx)
}

pub async fn list_by_document(pool: &PgPool, document_id: Uuid) -> Result<Vec<Chunk>> {
    sqlx::query_as::<_, Chunk>(
        "SELECT id, document_id, seq, text, token_count, meta
         FROM chunks WHERE document_id = $1 ORDER BY seq",
    )
    .bind(document_id)
    .fetch_all(pool)
    .await
    .map_err(map_sqlx)
}

/// Count a document's chunks and how many are embedded in a space (for progress).
pub async fn embedding_progress(
    pool: &PgPool,
    document_id: Uuid,
    space_id: i16,
) -> Result<(i64, i64)> {
    let row = sqlx::query_as::<_, (i64, i64)>(
        "SELECT
           count(*) AS total,
           count(*) FILTER (WHERE EXISTS (
             SELECT 1 FROM chunk_embeddings e
             WHERE e.chunk_id = c.id AND e.space_id = $2)) AS embedded
         FROM chunks c WHERE c.document_id = $1",
    )
    .bind(document_id)
    .bind(space_id)
    .fetch_one(pool)
    .await
    .map_err(map_sqlx)?;
    Ok(row)
}
