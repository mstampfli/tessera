//! Community detection over the entity graph, and the bridges between
//! communities.
//!
//! Communities come from weighted Louvain modularity optimization on the
//! co-occurrence graph, with edges weighted by idf-damped co-occurrence (rare
//! shared entities count for more, ubiquitous ones for less). Modularity handles
//! hubs by construction - a node connected to everything barely raises it, so it
//! does not force merges - which is why this replaced the old connected-components
//! (single-linkage) assignment that chained everything a hub touched into one
//! blob. A semantic `similar` edge whose endpoints fall in two different
//! communities is a bridge: a non-obvious link between things never stated
//! together, which is exactly the cross-domain correlation worth surfacing.

use std::collections::HashMap;

use sqlx::PgPool;
use tessera_core::error::Result;
use uuid::Uuid;

use crate::louvain;
use crate::map_sqlx;

/// Recompute every entity's `community_id` by weighted Louvain over the
/// idf-weighted co-occurrence graph. Returns the number of communities.
pub async fn detect(pool: &PgPool) -> Result<i64> {
    // Entity id -> dense index, plus mention counts for idf.
    let ents: Vec<(Uuid, i64)> =
        sqlx::query_as("SELECT id, mention_count FROM entities ORDER BY id")
            .fetch_all(pool)
            .await
            .map_err(map_sqlx)?;
    if ents.is_empty() {
        return Ok(0);
    }
    let total_chunks: i64 = sqlx::query_scalar("SELECT GREATEST(count(*), 1) FROM chunks")
        .fetch_one(pool)
        .await
        .map_err(map_sqlx)?;
    let n_docs = total_chunks as f64;

    let mut index: HashMap<Uuid, usize> = HashMap::with_capacity(ents.len());
    let mut idf: Vec<f64> = Vec::with_capacity(ents.len());
    for (i, (id, mc)) in ents.iter().enumerate() {
        index.insert(*id, i);
        idf.push((1.0 + n_docs / (*mc).max(1) as f64).ln());
    }

    let raw: Vec<(Uuid, Uuid, i32)> = sqlx::query_as(
        "SELECT src_id, dst_id, source_count FROM entity_edges WHERE rel = 'co_occurs'",
    )
    .fetch_all(pool)
    .await
    .map_err(map_sqlx)?;

    // Edge weight = shared-chunk count damped by the rarity of both endpoints, so
    // a hub pair (both common, low idf) weighs little and rare pairs weigh a lot.
    let edges: Vec<(usize, usize, f64)> = raw
        .iter()
        .filter_map(|(a, b, sc)| {
            let (ia, ib) = (*index.get(a)?, *index.get(b)?);
            let w = f64::from(*sc) * (idf[ia] * idf[ib]).sqrt();
            Some((ia, ib, w))
        })
        .collect();

    let labels = louvain::communities(ents.len(), &edges);
    let ids: Vec<Uuid> = ents.iter().map(|(id, _)| *id).collect();
    let cids: Vec<i32> = labels
        .iter()
        .map(|&c| i32::try_from(c).unwrap_or(i32::MAX))
        .collect();

    sqlx::query(
        "UPDATE entities e SET community_id = d.cid
         FROM (SELECT unnest($1::uuid[]) AS id, unnest($2::int[]) AS cid) d
         WHERE e.id = d.id",
    )
    .bind(&ids)
    .bind(&cids)
    .execute(pool)
    .await
    .map_err(map_sqlx)?;

    let k = labels.iter().copied().max().map_or(0, |m| m + 1);
    Ok(i64::try_from(k).unwrap_or(i64::MAX))
}

/// A bridge: a semantic (`similar`) edge whose endpoints lie in different
/// co-occurrence communities, i.e. a link between two things never stated
/// together. Ranked by similarity strength.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Bridge {
    pub a_id: Uuid,
    pub a_kind: String,
    pub a_value: String,
    pub a_community: i32,
    pub b_id: Uuid,
    pub b_kind: String,
    pub b_value: String,
    pub b_community: i32,
    pub strength: f64,
}

/// The strongest cross-community bridges (requires [`detect`] to have run).
pub async fn bridges(pool: &PgPool, limit: i64) -> Result<Vec<Bridge>> {
    sqlx::query_as::<_, Bridge>(
        "SELECT a.id AS a_id, a.kind AS a_kind, a.value AS a_value, a.community_id AS a_community,
                b.id AS b_id, b.kind AS b_kind, b.value AS b_value, b.community_id AS b_community,
                edge.weight AS strength
         FROM entity_edges edge
         JOIN entities a ON a.id = edge.src_id
         JOIN entities b ON b.id = edge.dst_id
         WHERE edge.rel = 'similar'
           AND a.community_id IS NOT NULL AND b.community_id IS NOT NULL
           AND a.community_id <> b.community_id
         ORDER BY edge.weight DESC, a.id, b.id
         LIMIT $1",
    )
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(map_sqlx)
}
