//! Incremental clustering over chunk embeddings.
//!
//! A cluster is a group of semantically near chunks with a stable id. New chunks
//! are assigned online to the nearest cluster within a distance threshold, or
//! seed a new cluster. The centroid is recomputed as the mean of the cluster's
//! members (pgvector `avg`), so it is idempotent and does not drift under
//! concurrent assignment. `dirty_count` tracks new members since the last
//! insight synthesis.

use chrono::{DateTime, Utc};
use pgvector::Vector;
use sqlx::PgPool;
use tessera_core::error::Result;
use uuid::Uuid;

use crate::map_sqlx;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Cluster {
    pub id: Uuid,
    pub space_id: i16,
    pub size: i32,
    pub label: Option<String>,
    pub dirty_count: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

const CLUSTER_COLS: &str = "id, space_id, size, label, dirty_count, created_at, updated_at";

/// Which cluster a chunk currently belongs to, if any.
pub async fn member_cluster(pool: &PgPool, chunk_id: Uuid) -> Result<Option<Uuid>> {
    sqlx::query_scalar::<_, Uuid>("SELECT cluster_id FROM cluster_members WHERE chunk_id = $1")
        .bind(chunk_id)
        .fetch_optional(pool)
        .await
        .map_err(map_sqlx)
}

/// The nearest cluster to a vector in a space, with its cosine distance. Linear
/// over clusters (there are far fewer clusters than chunks); an HNSW index on
/// centroids is a later optimization.
pub async fn nearest(pool: &PgPool, space_id: i16, vec: &[f32]) -> Result<Option<(Uuid, f64)>> {
    sqlx::query_as::<_, (Uuid, f64)>(
        "SELECT id, (centroid <=> $2)::float8 AS distance
         FROM clusters
         WHERE space_id = $1
         ORDER BY centroid <=> $2
         LIMIT 1",
    )
    .bind(space_id)
    .bind(Vector::from(vec.to_vec()))
    .fetch_optional(pool)
    .await
    .map_err(map_sqlx)
}

/// Create a new cluster seeded by one chunk. The caller holds the per-space
/// advisory lock so two workers cannot create twin clusters for the same region.
pub async fn create<'e, E: sqlx::PgExecutor<'e>>(
    exec: E,
    space_id: i16,
    centroid: &[f32],
    chunk_id: Uuid,
    similarity: f32,
) -> Result<Uuid> {
    let id = tessera_core::new_id();
    // Insert the cluster and its first member atomically via a CTE.
    sqlx::query_scalar::<_, Uuid>(
        "WITH c AS (
             INSERT INTO clusters (id, space_id, centroid, size, dirty_count)
             VALUES ($1, $2, $3, 1, 1)
             RETURNING id
         )
         INSERT INTO cluster_members (cluster_id, chunk_id, similarity)
         SELECT id, $4, $5 FROM c
         RETURNING cluster_id",
    )
    .bind(id)
    .bind(space_id)
    .bind(Vector::from(centroid.to_vec()))
    .bind(chunk_id)
    .bind(similarity)
    .fetch_one(exec)
    .await
    .map_err(map_sqlx)
}

/// Assign a chunk to an existing cluster: add the member (idempotent on
/// `chunk_id`), then recompute the centroid as the mean of members and bump
/// `size` + `dirty_count`. Returns true if the chunk was newly added.
pub async fn assign(
    pool: &PgPool,
    cluster_id: Uuid,
    chunk_id: Uuid,
    similarity: f32,
    space_id: i16,
) -> Result<bool> {
    let mut tx = pool.begin().await.map_err(map_sqlx)?;

    let inserted = sqlx::query(
        "INSERT INTO cluster_members (cluster_id, chunk_id, similarity)
         VALUES ($1, $2, $3)
         ON CONFLICT (chunk_id) DO NOTHING",
    )
    .bind(cluster_id)
    .bind(chunk_id)
    .bind(similarity)
    .execute(&mut *tx)
    .await
    .map_err(map_sqlx)?
    .rows_affected()
        > 0;

    if inserted {
        // Recompute the centroid as the member mean (idempotent, no drift).
        sqlx::query(
            "UPDATE clusters c SET
                 centroid = sub.centroid,
                 size = sub.n,
                 dirty_count = c.dirty_count + 1,
                 updated_at = now()
             FROM (
                 SELECT avg(e.embedding) AS centroid, count(*)::int AS n
                 FROM cluster_members m
                 JOIN chunk_embeddings e ON e.chunk_id = m.chunk_id AND e.space_id = $2
                 WHERE m.cluster_id = $1
             ) sub
             WHERE c.id = $1",
        )
        .bind(cluster_id)
        .bind(space_id)
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?;
    }

    tx.commit().await.map_err(map_sqlx)?;
    Ok(inserted)
}

/// Acquire the per-space clustering advisory lock inside a transaction, so
/// cluster creation is serialized per space.
pub async fn begin_locked(
    pool: &PgPool,
    space_id: i16,
) -> Result<sqlx::Transaction<'_, sqlx::Postgres>> {
    let mut tx = pool.begin().await.map_err(map_sqlx)?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext('tessera.clusters'), $1::int)")
        .bind(i32::from(space_id))
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?;
    Ok(tx)
}

pub async fn get(pool: &PgPool, id: Uuid) -> Result<Option<Cluster>> {
    sqlx::query_as::<_, Cluster>(&format!(
        "SELECT {CLUSTER_COLS} FROM clusters WHERE id = $1"
    ))
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(map_sqlx)
}

/// List clusters, largest first.
pub async fn list(pool: &PgPool, limit: i64) -> Result<Vec<Cluster>> {
    sqlx::query_as::<_, Cluster>(&format!(
        "SELECT {CLUSTER_COLS} FROM clusters ORDER BY size DESC, id LIMIT $1"
    ))
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(map_sqlx)
}

/// A cluster's member chunks (id, seq, text, document, title).
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct MemberChunk {
    pub chunk_id: Uuid,
    pub document_id: Uuid,
    pub title: Option<String>,
    pub text: String,
    pub similarity: f32,
}

pub async fn members(pool: &PgPool, cluster_id: Uuid, limit: i64) -> Result<Vec<MemberChunk>> {
    sqlx::query_as::<_, MemberChunk>(
        "SELECT c.id AS chunk_id, c.document_id, d.title, c.text, m.similarity
         FROM cluster_members m
         JOIN chunks c ON c.id = m.chunk_id
         JOIN documents d ON d.id = c.document_id
         WHERE m.cluster_id = $1
         ORDER BY m.similarity DESC
         LIMIT $2",
    )
    .bind(cluster_id)
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(map_sqlx)
}

/// The chunks nearest the cluster centroid, as representative evidence for
/// synthesis.
pub async fn representative_chunks(
    pool: &PgPool,
    cluster_id: Uuid,
    space_id: i16,
    limit: i64,
) -> Result<Vec<MemberChunk>> {
    sqlx::query_as::<_, MemberChunk>(
        "SELECT c.id AS chunk_id, c.document_id, d.title, c.text, m.similarity
         FROM cluster_members m
         JOIN chunks c ON c.id = m.chunk_id
         JOIN documents d ON d.id = c.document_id
         JOIN chunk_embeddings e ON e.chunk_id = c.id AND e.space_id = $2
         JOIN clusters cl ON cl.id = m.cluster_id
         WHERE m.cluster_id = $1
         ORDER BY e.embedding <=> cl.centroid
         LIMIT $3",
    )
    .bind(cluster_id)
    .bind(space_id)
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(map_sqlx)
}

/// Reset a cluster's dirty counter (after synthesizing an insight for it).
pub async fn reset_dirty(pool: &PgPool, id: Uuid) -> Result<()> {
    sqlx::query("UPDATE clusters SET dirty_count = 0 WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await
        .map_err(map_sqlx)?;
    Ok(())
}

/// Set a cluster's label (from synthesis).
pub async fn set_label(pool: &PgPool, id: Uuid, label: &str) -> Result<()> {
    sqlx::query("UPDATE clusters SET label = $2 WHERE id = $1")
        .bind(id)
        .bind(label)
        .execute(pool)
        .await
        .map_err(map_sqlx)?;
    Ok(())
}

/// A node in a cluster's entity co-occurrence graph: an entity mentioned in the
/// cluster, weighted by how often it is mentioned there.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct GraphNode {
    pub id: Uuid,
    pub kind: String,
    pub value: String,
    pub display_value: String,
    pub weight: i64,
}

/// An edge in an entity graph: a correlation between two entities, with the
/// method (`co_occurs` direct, or `similar` semantic) and a strength in `[0, 1]`.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct GraphEdge {
    pub src_id: Uuid,
    pub dst_id: Uuid,
    pub method: String,
    pub strength: f64,
}

/// The entity co-occurrence subgraph for a cluster: the entities mentioned in
/// its chunks (capped at `node_cap`, most-mentioned first) and the co-occurrence
/// edges among exactly that node set. This is the actionable "campaign network"
/// view. Edges are restricted to the returned nodes so the graph is always
/// internally consistent (no dangling endpoints).
pub async fn graph(
    pool: &PgPool,
    cluster_id: Uuid,
    node_cap: i64,
) -> Result<(Vec<GraphNode>, Vec<GraphEdge>)> {
    let nodes = sqlx::query_as::<_, GraphNode>(
        "SELECT e.id, e.kind, e.value, e.display_value, count(*)::bigint AS weight
         FROM cluster_members m
         JOIN entity_mentions em ON em.chunk_id = m.chunk_id
         JOIN entities e ON e.id = em.entity_id
         WHERE m.cluster_id = $1
         GROUP BY e.id, e.kind, e.value, e.display_value
         ORDER BY weight DESC, e.id
         LIMIT $2",
    )
    .bind(cluster_id)
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
                ({strength})::float8 AS strength
         FROM entity_edges edge
         WHERE edge.src_id = ANY($1) AND edge.dst_id = ANY($1)
         ORDER BY strength DESC, edge.src_id, edge.dst_id",
        strength = crate::repos::entities::EDGE_STRENGTH_SQL,
    ))
    .bind(&ids)
    .fetch_all(pool)
    .await
    .map_err(map_sqlx)?;

    Ok((nodes, edges))
}

/// The top entities mentioned across a cluster's chunks (for synthesis context
/// and labeling), by mention frequency within the cluster.
pub async fn top_entities(
    pool: &PgPool,
    cluster_id: Uuid,
    limit: i64,
) -> Result<Vec<(String, String)>> {
    sqlx::query_as::<_, (String, String)>(
        "SELECT e.kind, e.value
         FROM cluster_members m
         JOIN entity_mentions em ON em.chunk_id = m.chunk_id
         JOIN entities e ON e.id = em.entity_id
         WHERE m.cluster_id = $1
         GROUP BY e.id, e.kind, e.value
         ORDER BY count(*) DESC
         LIMIT $2",
    )
    .bind(cluster_id)
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(map_sqlx)
}
