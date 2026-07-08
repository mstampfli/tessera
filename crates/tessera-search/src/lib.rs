//! Hybrid retrieval and ask-with-citations.
//!
//! Retrieval fuses two signals with Reciprocal Rank Fusion in a single SQL
//! statement: pgvector kNN (semantic nearness) and Postgres full-text search
//! (exact keyword match, essential because identifiers like IPs and hashes do
//! not embed meaningfully). The vector distance uses the same `halfvec` cast as
//! the per-space HNSW index, so the index is actually used.

pub mod ask;

pub use ask::{ask, AskAnswer, Citation};

use std::sync::Arc;

use pgvector::Vector;
use serde::Serialize;
use sqlx::PgPool;
use tessera_core::error::{Error, ErrorKind, Result};
use tessera_db::repos::embeddings::EmbeddingSpace;
use tessera_providers::{EmbedKind, EmbeddingProvider};
use uuid::Uuid;

/// Which signals to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchMode {
    /// Vector + keyword, fused (the default).
    Hybrid,
    /// Vector only.
    Semantic,
    /// Keyword only.
    Keyword,
}

impl SearchMode {
    #[must_use]
    pub fn parse(s: &str) -> SearchMode {
        match s {
            "semantic" => SearchMode::Semantic,
            "keyword" => SearchMode::Keyword,
            _ => SearchMode::Hybrid,
        }
    }
    fn uses_vector(self) -> bool {
        matches!(self, SearchMode::Hybrid | SearchMode::Semantic)
    }
    fn uses_keyword(self) -> bool {
        matches!(self, SearchMode::Hybrid | SearchMode::Keyword)
    }
}

/// One search result, with the evidence needed to render and cite it.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct SearchHit {
    pub chunk_id: Uuid,
    pub document_id: Uuid,
    pub seq: i32,
    pub text: String,
    pub title: Option<String>,
    pub score: f64,
    /// True if the vector signal surfaced this hit.
    pub semantic: bool,
    /// True if the keyword signal surfaced this hit.
    pub keyword: bool,
    /// Cosine distance (0 = identical, up to 2) when the vector signal matched;
    /// `None` for keyword-only hits. Used as a relevance signal (see `ask`).
    pub distance: Option<f64>,
}

/// RRF constant; a rank-smoothing term that keeps any single list from
/// dominating. 60 is the value from the original RRF paper.
const RRF_K: i32 = 60;

/// Run a hybrid search. `space` may be `None` when nothing has been embedded yet,
/// in which case only the keyword signal is available.
pub async fn search(
    pool: &PgPool,
    embedder: &Arc<dyn EmbeddingProvider>,
    space: Option<&EmbeddingSpace>,
    query: &str,
    mode: SearchMode,
    limit: i64,
) -> Result<Vec<SearchHit>> {
    let query = query.trim();
    if query.is_empty() {
        return Ok(Vec::new());
    }
    // Retrieve a wider candidate pool per signal than the final limit so fusion
    // has room to reorder.
    let per_signal = (limit * 4).clamp(10, 200);

    // Decide whether the vector signal is actually available.
    let want_vector = mode.uses_vector() && space.is_some();
    let query_vec = if want_vector {
        let v = embedder
            .embed(&[query.to_string()], EmbedKind::Query)
            .await
            .map_err(|e| Error::new(ErrorKind::Provider, format!("embed query: {e}")))?;
        v.into_iter().next()
    } else {
        None
    };

    match (query_vec, space, mode.uses_keyword()) {
        // Vector + keyword.
        (Some(vec), Some(space), true) => {
            hybrid_query(pool, query, &vec, space, per_signal, limit).await
        }
        // Vector only.
        (Some(vec), Some(space), false) => vector_query(pool, &vec, space, limit).await,
        // Keyword only (either requested, or no embeddings available yet).
        _ => keyword_query(pool, query, limit).await,
    }
}

async fn hybrid_query(
    pool: &PgPool,
    text: &str,
    vec: &[f32],
    space: &EmbeddingSpace,
    per_signal: i64,
    limit: i64,
) -> Result<Vec<SearchHit>> {
    let dim = space.dim;
    // dim and RRF_K are integers we control, safe to interpolate; all user input
    // is bound.
    let sql = format!(
        "WITH vec AS (
             SELECT chunk_id,
                    row_number() OVER (ORDER BY embedding::halfvec({dim}) <=> $1::halfvec({dim})) AS rank,
                    (embedding::halfvec({dim}) <=> $1::halfvec({dim}))::float8 AS dist
             FROM chunk_embeddings
             WHERE space_id = $2
             ORDER BY embedding::halfvec({dim}) <=> $1::halfvec({dim})
             LIMIT $3
         ),
         kw AS (
             SELECT c.id AS chunk_id,
                    row_number() OVER (ORDER BY ts_rank_cd(c.tsv, query) DESC) AS rank,
                    NULL::float8 AS dist
             FROM chunks c, websearch_to_tsquery('english', $4) query
             WHERE c.tsv @@ query
             ORDER BY ts_rank_cd(c.tsv, query) DESC
             LIMIT $3
         ),
         fused AS (
             SELECT chunk_id,
                    sum(1.0 / ({RRF_K} + rank)) AS score,
                    bool_or(src = 'vec') AS semantic,
                    bool_or(src = 'kw') AS keyword,
                    min(dist) AS distance
             FROM (
                 SELECT chunk_id, rank, dist, 'vec' AS src FROM vec
                 UNION ALL
                 SELECT chunk_id, rank, dist, 'kw' AS src FROM kw
             ) u
             GROUP BY chunk_id
         )
         SELECT f.chunk_id, c.document_id, c.seq, c.text, d.title,
                f.score::float8 AS score, f.semantic, f.keyword, f.distance
         FROM fused f
         JOIN chunks c ON c.id = f.chunk_id
         JOIN documents d ON d.id = c.document_id
         ORDER BY f.score DESC, f.chunk_id
         LIMIT $5"
    );
    sqlx::query_as::<_, SearchHit>(&sql)
        .bind(Vector::from(vec.to_vec()))
        .bind(space.id)
        .bind(per_signal)
        .bind(text)
        .bind(limit)
        .fetch_all(pool)
        .await
        .map_err(tessera_db::map_sqlx)
}

async fn vector_query(
    pool: &PgPool,
    vec: &[f32],
    space: &EmbeddingSpace,
    limit: i64,
) -> Result<Vec<SearchHit>> {
    let dim = space.dim;
    let sql = format!(
        "SELECT c.id AS chunk_id, c.document_id, c.seq, c.text, d.title,
                (1.0 / (1.0 + (e.embedding::halfvec({dim}) <=> $1::halfvec({dim}))))::float8 AS score,
                true AS semantic, false AS keyword,
                (e.embedding::halfvec({dim}) <=> $1::halfvec({dim}))::float8 AS distance
         FROM chunk_embeddings e
         JOIN chunks c ON c.id = e.chunk_id
         JOIN documents d ON d.id = c.document_id
         WHERE e.space_id = $2
         ORDER BY e.embedding::halfvec({dim}) <=> $1::halfvec({dim})
         LIMIT $3"
    );
    sqlx::query_as::<_, SearchHit>(&sql)
        .bind(Vector::from(vec.to_vec()))
        .bind(space.id)
        .bind(limit)
        .fetch_all(pool)
        .await
        .map_err(tessera_db::map_sqlx)
}

async fn keyword_query(pool: &PgPool, text: &str, limit: i64) -> Result<Vec<SearchHit>> {
    sqlx::query_as::<_, SearchHit>(
        "SELECT c.id AS chunk_id, c.document_id, c.seq, c.text, d.title,
                ts_rank_cd(c.tsv, websearch_to_tsquery('english', $1))::float8 AS score,
                false AS semantic, true AS keyword, NULL::float8 AS distance
         FROM chunks c
         JOIN documents d ON d.id = c.document_id
         WHERE c.tsv @@ websearch_to_tsquery('english', $1)
         ORDER BY score DESC, c.id
         LIMIT $2",
    )
    .bind(text)
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(tessera_db::map_sqlx)
}
