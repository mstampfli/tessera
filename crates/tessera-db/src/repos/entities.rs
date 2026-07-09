//! Entities, mentions, and the correlation edges between them.
//!
//! Entities dedup at the database via `UNIQUE (kind, value)`. Mentions are
//! idempotent on `(entity_id, chunk_id, span)`. Co-occurrence edges are DERIVED
//! from mentions by a set-based recompute (not incremented), so re-running entity
//! extraction on a document converges rather than double-counting.

use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::PgPool;
use tessera_core::error::Result;
use uuid::Uuid;

use crate::map_sqlx;
use crate::repos::clusters::{GraphEdge, GraphNode};

/// The global entity correlation graph: the most-connected entities (optionally
/// filtered to one kind) capped at `node_cap`, and the co-occurrence edges among
/// exactly that node set. Nodes are ranked by degree (edge count) so a capped
/// view shows the hubs first; isolated entities fall to the tail. Edges are
/// restricted to the returned nodes, so the graph is always self-consistent.
pub async fn graph(
    pool: &PgPool,
    kind: Option<&str>,
    node_cap: i64,
) -> Result<(Vec<GraphNode>, Vec<GraphEdge>)> {
    let nodes = sqlx::query_as::<_, GraphNode>(
        "SELECT e.id, e.kind, e.value, e.display_value, e.mention_count::bigint AS weight,
                e.community_id
         FROM entities e
         LEFT JOIN (
             SELECT id, count(*) AS deg FROM (
                 SELECT src_id AS id FROM entity_edges
                 UNION ALL
                 SELECT dst_id AS id FROM entity_edges
             ) x GROUP BY id
         ) d ON d.id = e.id
         WHERE ($1::text IS NULL OR e.kind = $1)
         ORDER BY COALESCE(d.deg, 0) DESC, e.mention_count DESC, e.id
         LIMIT $2",
    )
    .bind(kind)
    .bind(node_cap)
    .fetch_all(pool)
    .await
    .map_err(map_sqlx)?;

    if nodes.is_empty() {
        return Ok((nodes, Vec::new()));
    }

    let ids: Vec<Uuid> = nodes.iter().map(|n| n.id).collect();
    let edges = sqlx::query_as::<_, GraphEdge>(&format!(
        "SELECT edge.src_id, edge.dst_id, edge.rel AS method,
                ({EDGE_STRENGTH_SQL})::float8 AS strength
         FROM entity_edges edge
         WHERE edge.src_id = ANY($1) AND edge.dst_id = ANY($1)
         ORDER BY strength DESC, edge.src_id, edge.dst_id",
    ))
    .bind(&ids)
    .fetch_all(pool)
    .await
    .map_err(map_sqlx)?;

    Ok((nodes, edges))
}

/// The relation name for a semantic-similarity edge in `entity_edges`.
pub const REL_SIMILAR: &str = "similar";
/// The relation name for a direct co-occurrence edge.
pub const REL_CO_OCCURS: &str = "co_occurs";

/// SQL expression mapping an `entity_edges` row (aliased `edge`) to a correlation
/// strength in `[0, 1]`: direct co-occurrence sits in a strong band by shared-
/// chunk count; every other method (semantic, temporal) uses its stored weight.
/// One source of truth so the graph and the neighbourhood agree.
pub const EDGE_STRENGTH_SQL: &str = "CASE WHEN edge.rel = 'co_occurs' \
     THEN LEAST(1.0, 0.6 + 0.08 * edge.source_count) \
     ELSE GREATEST(0.0, LEAST(1.0, edge.weight)) END";

/// (Re)materialize the mean context embedding for each entity in `ids` from the
/// embeddings of the chunks it is mentioned in. Idempotent: recomputes from the
/// current mentions, so re-running after new mentions converges. Entities with
/// no embedded mention chunks yet are skipped (they get one on a later run).
pub async fn recompute_entity_embeddings(
    pool: &PgPool,
    space_id: i16,
    ids: &[Uuid],
) -> Result<u64> {
    if ids.is_empty() {
        return Ok(0);
    }
    let n = sqlx::query(
        "INSERT INTO entity_embeddings (entity_id, space_id, embedding, updated_at)
         SELECT em.entity_id, $2, avg(ce.embedding), now()
         FROM entity_mentions em
         JOIN chunk_embeddings ce ON ce.chunk_id = em.chunk_id AND ce.space_id = $2
         WHERE em.entity_id = ANY($1)
         GROUP BY em.entity_id
         ON CONFLICT (entity_id, space_id)
         DO UPDATE SET embedding = EXCLUDED.embedding, updated_at = now()",
    )
    .bind(ids)
    .bind(space_id)
    .execute(pool)
    .await
    .map_err(map_sqlx)?
    .rows_affected();
    Ok(n)
}

/// Materialize context embeddings for every entity with embedded mention chunks
/// (the backfill path for an existing corpus).
pub async fn recompute_all_entity_embeddings(pool: &PgPool, space_id: i16) -> Result<u64> {
    let n = sqlx::query(
        "INSERT INTO entity_embeddings (entity_id, space_id, embedding, updated_at)
         SELECT em.entity_id, $1, avg(ce.embedding), now()
         FROM entity_mentions em
         JOIN chunk_embeddings ce ON ce.chunk_id = em.chunk_id AND ce.space_id = $1
         GROUP BY em.entity_id
         ON CONFLICT (entity_id, space_id)
         DO UPDATE SET embedding = EXCLUDED.embedding, updated_at = now()",
    )
    .bind(space_id)
    .execute(pool)
    .await
    .map_err(map_sqlx)?
    .rows_affected();
    Ok(n)
}

/// One entity's global nearest neighbours by context-embedding cosine similarity,
/// via the entity HNSW index (O(log n), not an all-pairs scan). `min_sim` drops
/// near-orthogonal pairs; ranking is relative (top-k) because short technical
/// text embeds tightly.
#[derive(Debug, Clone, sqlx::FromRow)]
struct SimNeighbor {
    id: Uuid,
    sim: f64,
}

/// Add the `similar` edges out of one entity: find its global nearest neighbours
/// (via the entity HNSW index) and upsert an undirected `similar` edge (src < dst)
/// to each above `min_sim`, weighted by cosine similarity. Add-only by design:
/// the edge is undirected, so a neighbour that has *this* entity in its own top-k
/// must not be dropped just because this entity's top-k shifted. A full rebuild
/// ([`rebuild_similar_edges`]) prunes edges an embedding change made stale.
pub async fn add_similar_edges(
    pool: &PgPool,
    space_id: i16,
    dim: i32,
    entity_id: Uuid,
    k: i64,
    min_sim: f64,
) -> Result<u64> {
    // The `halfvec(dim)` cast matches the entity HNSW index expression, so this
    // is an index scan (O(log n)) rather than a full pass over every entity.
    let neighbors = sqlx::query_as::<_, SimNeighbor>(&format!(
        "SELECT e.entity_id AS id,
                (1.0 - (e.embedding::halfvec({dim}) <=> t.embedding::halfvec({dim})))::float8 AS sim
         FROM entity_embeddings e
         JOIN entity_embeddings t ON t.entity_id = $1 AND t.space_id = $2
         WHERE e.space_id = $2 AND e.entity_id <> $1
         ORDER BY e.embedding::halfvec({dim}) <=> t.embedding::halfvec({dim})
         LIMIT $3",
    ))
    .bind(entity_id)
    .bind(space_id)
    .bind(k)
    .fetch_all(pool)
    .await
    .map_err(map_sqlx)?;

    let mut written = 0u64;
    for n in neighbors {
        if n.sim < min_sim {
            continue;
        }
        let (a, b) = if entity_id <= n.id {
            (entity_id, n.id)
        } else {
            (n.id, entity_id)
        };
        written += sqlx::query(
            "INSERT INTO entity_edges (src_id, dst_id, rel, source_count, weight, first_seen, last_seen)
             VALUES ($1, $2, 'similar', 0, $3, now(), now())
             ON CONFLICT (src_id, dst_id, rel)
             DO UPDATE SET weight = EXCLUDED.weight, last_seen = now()",
        )
        .bind(a)
        .bind(b)
        .bind(n.sim)
        .execute(pool)
        .await
        .map_err(map_sqlx)?
        .rows_affected();
    }
    Ok(written)
}

/// Rebuild every `similar` edge from scratch: drop all of them, then recompute
/// each embedded entity's top-k. Used to backfill an existing corpus and as a
/// periodic maintenance pass that prunes edges made stale by embedding drift.
pub async fn rebuild_similar_edges(
    pool: &PgPool,
    space_id: i16,
    dim: i32,
    k: i64,
    min_sim: f64,
) -> Result<u64> {
    sqlx::query("DELETE FROM entity_edges WHERE rel = 'similar'")
        .execute(pool)
        .await
        .map_err(map_sqlx)?;
    let ids = ids_with_embeddings(pool, space_id).await?;
    let mut total = 0u64;
    for id in ids {
        total += add_similar_edges(pool, space_id, dim, id, k, min_sim).await?;
    }
    Ok(total)
}

/// Every entity id that currently has a materialized embedding in a space.
pub async fn ids_with_embeddings(pool: &PgPool, space_id: i16) -> Result<Vec<Uuid>> {
    sqlx::query_scalar::<_, Uuid>("SELECT entity_id FROM entity_embeddings WHERE space_id = $1")
        .bind(space_id)
        .fetch_all(pool)
        .await
        .map_err(map_sqlx)
}

/// The entity ids mentioned in a document (for the correlation stage).
pub async fn ids_for_document(pool: &PgPool, document_id: Uuid) -> Result<Vec<Uuid>> {
    sqlx::query_scalar::<_, Uuid>(
        "SELECT DISTINCT entity_id FROM entity_mentions WHERE document_id = $1",
    )
    .bind(document_id)
    .fetch_all(pool)
    .await
    .map_err(map_sqlx)
}

/// Total number of entities (optionally filtered to one kind), so callers can
/// tell the user how many of the full set a capped graph is showing.
pub async fn count(pool: &PgPool, kind: Option<&str>) -> Result<i64> {
    sqlx::query_scalar::<_, i64>(
        "SELECT count(*)::bigint FROM entities WHERE ($1::text IS NULL OR kind = $1)",
    )
    .bind(kind)
    .fetch_one(pool)
    .await
    .map_err(map_sqlx)
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Entity {
    pub id: Uuid,
    pub kind: String,
    pub value: String,
    pub display_value: String,
    pub attrs: Value,
    pub mention_count: i64,
    pub first_seen: DateTime<Utc>,
    pub last_seen: DateTime<Utc>,
}

/// Upsert an entity by `(kind, value)` and return its id. Refreshes `last_seen`;
/// `mention_count` is maintained separately by [`recompute_mention_counts`] so it
/// stays idempotent under re-extraction.
pub async fn upsert<'e, E: sqlx::PgExecutor<'e>>(
    exec: E,
    kind: &str,
    value: &str,
    display_value: &str,
) -> Result<Uuid> {
    let id = tessera_core::new_id();
    sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO entities (id, kind, value, display_value)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT (kind, value) DO UPDATE SET last_seen = now()
         RETURNING id",
    )
    .bind(id)
    .bind(kind)
    .bind(value)
    .bind(display_value)
    .fetch_one(exec)
    .await
    .map_err(map_sqlx)
}

/// Insert a mention. Idempotent on `(entity_id, chunk_id, span)`. Returns whether
/// a row was newly inserted.
#[allow(clippy::too_many_arguments)]
pub async fn insert_mention<'e, E: sqlx::PgExecutor<'e>>(
    exec: E,
    entity_id: Uuid,
    chunk_id: Uuid,
    document_id: Uuid,
    raw_surface: &str,
    span: Option<(i32, i32)>,
    extractor: &str,
    confidence: f32,
) -> Result<bool> {
    let (start, end) = span.unzip();
    let n = sqlx::query(
        "INSERT INTO entity_mentions
            (entity_id, chunk_id, document_id, raw_surface, span, extractor, confidence)
         VALUES ($1, $2, $3, $4,
                 CASE WHEN $5::int IS NULL THEN NULL ELSE int4range($5, $6) END,
                 $7, $8)
         ON CONFLICT (entity_id, chunk_id, span) DO NOTHING",
    )
    .bind(entity_id)
    .bind(chunk_id)
    .bind(document_id)
    .bind(raw_surface)
    .bind(start)
    .bind(end)
    .bind(extractor)
    .bind(confidence)
    .execute(exec)
    .await
    .map_err(map_sqlx)?
    .rows_affected();
    Ok(n > 0)
}

/// Recompute `mention_count` for the given entities from the mentions table
/// (idempotent). Call after inserting a document's mentions.
pub async fn recompute_mention_counts(pool: &PgPool, entity_ids: &[Uuid]) -> Result<()> {
    if entity_ids.is_empty() {
        return Ok(());
    }
    sqlx::query(
        "UPDATE entities e
         SET mention_count = (SELECT count(*) FROM entity_mentions m WHERE m.entity_id = e.id)
         WHERE e.id = ANY($1)",
    )
    .bind(entity_ids)
    .execute(pool)
    .await
    .map_err(map_sqlx)?;
    Ok(())
}

/// Recompute co-occurrence edges for every entity pair touching `document_id`,
/// across the whole corpus. Two entities co-occur when they are mentioned in the
/// same chunk; the edge's `source_count` is the number of chunks they share. This
/// is a full recompute for the affected pairs (not an increment), so it is
/// idempotent under re-extraction. Symmetric edges are stored once with src < dst.
pub async fn recompute_cooccurrence(pool: &PgPool, document_id: Uuid) -> Result<u64> {
    let n = sqlx::query(
        "INSERT INTO entity_edges (src_id, dst_id, rel, source_count, weight, first_seen, last_seen)
         SELECT p.a, p.b, 'co_occurs', p.cnt, p.cnt::float8, now(), now()
         FROM (
             SELECT LEAST(m1.entity_id, m2.entity_id) AS a,
                    GREATEST(m1.entity_id, m2.entity_id) AS b,
                    count(DISTINCT m1.chunk_id) AS cnt
             FROM entity_mentions m1
             JOIN entity_mentions m2
               ON m1.chunk_id = m2.chunk_id AND m1.entity_id < m2.entity_id
             WHERE m1.entity_id IN (SELECT DISTINCT entity_id FROM entity_mentions WHERE document_id = $1)
                OR m2.entity_id IN (SELECT DISTINCT entity_id FROM entity_mentions WHERE document_id = $1)
             GROUP BY 1, 2
         ) p
         ON CONFLICT (src_id, dst_id, rel)
         DO UPDATE SET source_count = EXCLUDED.source_count,
                       weight = EXCLUDED.weight,
                       last_seen = now()",
    )
    .bind(document_id)
    .execute(pool)
    .await
    .map_err(map_sqlx)?
    .rows_affected();
    Ok(n)
}

/// List entities, optionally filtered by kind and a value substring, ranked by
/// how often they are mentioned.
pub async fn list(
    pool: &PgPool,
    kind: Option<&str>,
    query: Option<&str>,
    limit: i64,
) -> Result<Vec<Entity>> {
    sqlx::query_as::<_, Entity>(
        "SELECT id, kind, value, display_value, attrs, mention_count, first_seen, last_seen
         FROM entities
         WHERE ($1::text IS NULL OR kind = $1)
           AND ($2::text IS NULL OR value ILIKE '%' || $2 || '%')
         ORDER BY mention_count DESC, id
         LIMIT $3",
    )
    .bind(kind)
    .bind(query)
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(map_sqlx)
}

pub async fn get(pool: &PgPool, id: Uuid) -> Result<Option<Entity>> {
    sqlx::query_as::<_, Entity>(
        "SELECT id, kind, value, display_value, attrs, mention_count, first_seen, last_seen
         FROM entities WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(map_sqlx)
}

/// A neighbor in an entity's correlation neighborhood.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Neighbor {
    pub id: Uuid,
    pub kind: String,
    pub value: String,
    pub display_value: String,
    /// The correlation method: `co_occurs` (direct) or `similar` (semantic).
    pub rel: String,
    /// Correlation strength in `[0, 1]`.
    pub strength: f64,
}

/// The correlation neighborhood of an entity: its strongest correlations across
/// the whole KB (direct co-occurrence and semantic similarity), ranked by
/// strength, with the neighbor's rarity (idf) breaking ties so a shared rare
/// hash outranks a shared common port.
pub async fn neighborhood(pool: &PgPool, entity_id: Uuid, limit: i64) -> Result<Vec<Neighbor>> {
    sqlx::query_as::<_, Neighbor>(&format!(
        "WITH tc AS (SELECT GREATEST(count(*), 1)::float8 AS n FROM chunks)
         SELECT e2.id, e2.kind, e2.value, e2.display_value, edge.rel,
                ({EDGE_STRENGTH_SQL})::float8 AS strength
         FROM entity_edges edge
         JOIN entities e2
           ON e2.id = CASE WHEN edge.src_id = $1 THEN edge.dst_id ELSE edge.src_id END
         WHERE edge.src_id = $1 OR edge.dst_id = $1
         ORDER BY strength DESC,
                  (edge.source_count * ln(1.0 + (SELECT n FROM tc) / GREATEST(e2.mention_count, 1))) DESC,
                  e2.id
         LIMIT $2",
    ))
    .bind(entity_id)
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(map_sqlx)
}

/// The documents an entity appears in (for the entity page occurrences list).
pub async fn documents_for(
    pool: &PgPool,
    entity_id: Uuid,
    limit: i64,
) -> Result<Vec<(Uuid, Option<String>)>> {
    sqlx::query_as::<_, (Uuid, Option<String>)>(
        "SELECT DISTINCT d.id, d.title
         FROM entity_mentions m
         JOIN documents d ON d.id = m.document_id
         WHERE m.entity_id = $1
         ORDER BY d.id DESC
         LIMIT $2",
    )
    .bind(entity_id)
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(map_sqlx)
}
