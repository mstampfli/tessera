//! Embedding spaces and vectors.
//!
//! A "space" is one embedding model's output: a name, provider, dimensionality,
//! and metric. Vectors are stored in an untyped `vector` column; the per-space
//! HNSW index is a partial expression index (cast to halfvec) created here at
//! startup. Storing the space id per vector is what makes model swaps possible.

use pgvector::Vector;
use sqlx::PgPool;
use tessera_core::error::Result;
use uuid::Uuid;

use crate::map_sqlx;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct EmbeddingSpace {
    pub id: i16,
    pub name: String,
    pub provider: String,
    pub dim: i32,
    pub metric: String,
    pub active: bool,
}

/// Register a space by name (idempotent) and return it. A new space gets the
/// next small integer id. Safe to call at every startup.
pub async fn ensure(
    pool: &PgPool,
    name: &str,
    provider: &str,
    dim: i32,
    metric: &str,
) -> Result<EmbeddingSpace> {
    // Serialize id allocation with an advisory lock so two concurrent starts
    // cannot pick the same next id.
    let mut tx = pool.begin().await.map_err(map_sqlx)?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext('tessera.embedding_spaces'))")
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?;

    let space = sqlx::query_as::<_, EmbeddingSpace>(
        "INSERT INTO embedding_spaces (id, name, provider, dim, metric)
         VALUES ((SELECT coalesce(max(id), 0) + 1 FROM embedding_spaces), $1, $2, $3, $4)
         ON CONFLICT (name) DO UPDATE SET provider = EXCLUDED.provider
         RETURNING id, name, provider, dim, metric, active",
    )
    .bind(name)
    .bind(provider)
    .bind(dim)
    .bind(metric)
    .fetch_one(&mut *tx)
    .await
    .map_err(map_sqlx)?;

    tx.commit().await.map_err(map_sqlx)?;
    Ok(space)
}

/// Mark one space active and all others inactive.
pub async fn set_active(pool: &PgPool, id: i16) -> Result<()> {
    let mut tx = pool.begin().await.map_err(map_sqlx)?;
    sqlx::query("UPDATE embedding_spaces SET active = (id = $1)")
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?;
    tx.commit().await.map_err(map_sqlx)?;
    Ok(())
}

/// The currently active space, if any.
pub async fn active(pool: &PgPool) -> Result<Option<EmbeddingSpace>> {
    sqlx::query_as::<_, EmbeddingSpace>(
        "SELECT id, name, provider, dim, metric, active FROM embedding_spaces WHERE active LIMIT 1",
    )
    .fetch_optional(pool)
    .await
    .map_err(map_sqlx)
}

/// Create the per-space HNSW index if absent. The index is a partial expression
/// index over `embedding` cast to `halfvec(dim)` with cosine ops, so it is only
/// built for this space's rows and uses half the memory of full-precision.
/// Queries must use the same cast expression to hit it.
pub async fn ensure_hnsw_index(pool: &PgPool, space_id: i16, dim: i32) -> Result<()> {
    let index_name = format!("ce_hnsw_s{space_id}");
    let sql = format!(
        "CREATE INDEX IF NOT EXISTS {index_name}
         ON chunk_embeddings
         USING hnsw ((embedding::halfvec({dim})) halfvec_cosine_ops)
         WITH (m = 16, ef_construction = 64)
         WHERE space_id = {space_id}"
    );
    sqlx::query(&sql).execute(pool).await.map_err(map_sqlx)?;
    Ok(())
}

/// Insert a batch of `(chunk_id, embedding)` pairs for a space. Idempotent on
/// `(chunk_id, space_id)`.
pub async fn insert_batch(pool: &PgPool, space_id: i16, rows: &[(Uuid, Vec<f32>)]) -> Result<u64> {
    if rows.is_empty() {
        return Ok(0);
    }
    let mut tx = pool.begin().await.map_err(map_sqlx)?;
    let mut inserted = 0u64;
    for (chunk_id, vec) in rows {
        let v = Vector::from(vec.clone());
        inserted += sqlx::query(
            "INSERT INTO chunk_embeddings (chunk_id, space_id, embedding)
             VALUES ($1, $2, $3)
             ON CONFLICT (chunk_id, space_id) DO NOTHING",
        )
        .bind(chunk_id)
        .bind(space_id)
        .bind(v)
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?
        .rows_affected();
    }
    tx.commit().await.map_err(map_sqlx)?;
    Ok(inserted)
}

/// Total embeddings stored in a space (for diagnostics / doctor).
pub async fn count(pool: &PgPool, space_id: i16) -> Result<i64> {
    sqlx::query_scalar::<_, i64>("SELECT count(*) FROM chunk_embeddings WHERE space_id = $1")
        .bind(space_id)
        .fetch_one(pool)
        .await
        .map_err(map_sqlx)
}

/// Fetch a chunk's embedding vector in a space, if present.
pub async fn get_vector(pool: &PgPool, chunk_id: Uuid, space_id: i16) -> Result<Option<Vec<f32>>> {
    let row = sqlx::query_scalar::<_, Vector>(
        "SELECT embedding FROM chunk_embeddings WHERE chunk_id = $1 AND space_id = $2",
    )
    .bind(chunk_id)
    .bind(space_id)
    .fetch_optional(pool)
    .await
    .map_err(map_sqlx)?;
    Ok(row.map(|v| v.to_vec()))
}
