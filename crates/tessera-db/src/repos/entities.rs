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
        "SELECT e.id, e.kind, e.value, e.display_value, e.mention_count::bigint AS weight
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
    let edges = sqlx::query_as::<_, GraphEdge>(
        "SELECT edge.src_id, edge.dst_id, edge.rel, edge.source_count
         FROM entity_edges edge
         WHERE edge.src_id = ANY($1) AND edge.dst_id = ANY($1)
         ORDER BY edge.source_count DESC, edge.src_id, edge.dst_id",
    )
    .bind(&ids)
    .fetch_all(pool)
    .await
    .map_err(map_sqlx)?;

    Ok((nodes, edges))
}

/// A semantic-similarity edge between two entities: they are discussed in
/// similar contexts (cosine similarity of their mean mention-chunk embeddings),
/// even if they never co-occur in the same chunk.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct SemanticEdge {
    pub src_id: Uuid,
    pub dst_id: Uuid,
    pub sim: f64,
}

/// For each entity in `ids`, its `k` most semantically similar peers within the
/// same set, by cosine similarity of the mean of the entity's mention-chunk
/// embeddings. Relative (top-k), not an absolute threshold: embeddings of short
/// technical text cluster tightly, so a fixed cutoff would either connect
/// everything or nothing; top-k surfaces each entity's nearest peers regardless.
/// `min_sim` is only a floor to drop near-orthogonal pairs. Bounded work: the
/// caller passes an already-capped node set, so this is O(cap^2).
pub async fn semantic_edges(
    pool: &PgPool,
    space_id: i16,
    ids: &[Uuid],
    k: i64,
    min_sim: f64,
) -> Result<Vec<SemanticEdge>> {
    if ids.len() < 2 {
        return Ok(Vec::new());
    }
    sqlx::query_as::<_, SemanticEdge>(
        "WITH emb AS (
             SELECT em.entity_id AS id, avg(ce.embedding) AS v
             FROM entity_mentions em
             JOIN chunk_embeddings ce ON ce.chunk_id = em.chunk_id AND ce.space_id = $2
             WHERE em.entity_id = ANY($1)
             GROUP BY em.entity_id
         ),
         pairs AS (
             SELECT a.id AS src_id, b.id AS dst_id,
                    (1.0 - (a.v <=> b.v))::float8 AS sim,
                    row_number() OVER (PARTITION BY a.id ORDER BY a.v <=> b.v) AS rnk
             FROM emb a JOIN emb b ON a.id <> b.id
         )
         SELECT src_id, dst_id, sim FROM pairs
         WHERE rnk <= $3 AND sim >= $4",
    )
    .bind(ids)
    .bind(space_id)
    .bind(k)
    .bind(min_sim)
    .fetch_all(pool)
    .await
    .map_err(map_sqlx)
}

/// A unified correlation edge: either a direct co-occurrence (strong) or a
/// semantic similarity (contextual), each with a `strength` in `[0, 1]` so the
/// UI can render one continuous "how correlated" scale.
#[derive(Debug, Clone)]
pub struct CorrelationEdge {
    pub src_id: Uuid,
    pub dst_id: Uuid,
    pub method: &'static str,
    pub strength: f64,
}

/// Merge co-occurrence and semantic edges into one strength-scored set. Direct
/// co-occurrence is the stronger, precise signal and always wins for a pair;
/// semantic similarity fills in the "slighter" correlations between entities
/// that never share a chunk. Both endpoints are normalized to one unordered key.
#[must_use]
pub fn merge_correlation_edges(
    cooccurrence: &[GraphEdge],
    semantic: &[SemanticEdge],
) -> Vec<CorrelationEdge> {
    use std::collections::HashMap;
    let key = |a: Uuid, b: Uuid| if a <= b { (a, b) } else { (b, a) };
    let mut map: HashMap<(Uuid, Uuid), CorrelationEdge> = HashMap::new();

    for e in cooccurrence {
        // Direct co-occurrence sits in the strong band; more shared chunks -> stronger.
        let strength = (0.6 + 0.08 * f64::from(e.source_count)).min(1.0);
        let (a, b) = key(e.src_id, e.dst_id);
        map.insert(
            (a, b),
            CorrelationEdge {
                src_id: a,
                dst_id: b,
                method: "co_occurs",
                strength,
            },
        );
    }
    for s in semantic {
        if s.src_id == s.dst_id {
            continue;
        }
        let (a, b) = key(s.src_id, s.dst_id);
        // Only fill a pair the stronger co-occurrence signal did not already claim.
        map.entry((a, b)).or_insert(CorrelationEdge {
            src_id: a,
            dst_id: b,
            method: "similar",
            strength: s.sim.clamp(0.0, 1.0),
        });
    }
    map.into_values().collect()
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
    pub rel: String,
    pub source_count: i32,
    /// idf-weighted correlation score (rarer shared neighbors rank higher).
    pub score: f64,
}

/// The correlation neighborhood of an entity: its strongest co-occurring
/// entities, ranked by shared-chunk count weighted by the neighbor's rarity
/// (idf), so a shared rare hash outranks a shared common port.
pub async fn neighborhood(pool: &PgPool, entity_id: Uuid, limit: i64) -> Result<Vec<Neighbor>> {
    sqlx::query_as::<_, Neighbor>(
        "WITH tc AS (SELECT GREATEST(count(*), 1)::float8 AS n FROM chunks)
         SELECT e2.id, e2.kind, e2.value, e2.display_value, edge.rel, edge.source_count,
                (edge.source_count * ln(1.0 + (SELECT n FROM tc) / GREATEST(e2.mention_count, 1)))::float8 AS score
         FROM entity_edges edge
         JOIN entities e2
           ON e2.id = CASE WHEN edge.src_id = $1 THEN edge.dst_id ELSE edge.src_id END
         WHERE edge.src_id = $1 OR edge.dst_id = $1
         ORDER BY score DESC, e2.id
         LIMIT $2",
    )
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
