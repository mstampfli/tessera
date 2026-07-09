//! Structural community detection over the entity graph, and the bridges between
//! communities.
//!
//! A community is a connected component of the *direct co-occurrence* graph:
//! entities that are (transitively) mentioned together. It is deterministic and
//! cheap (union-find), and it is the right base for bridge detection: a semantic
//! `similar` edge whose endpoints fall in two different communities is a
//! non-obvious link between things that are never stated together, which is
//! exactly the kind of cross-domain correlation worth surfacing.

use std::collections::HashMap;

use sqlx::PgPool;
use tessera_core::error::Result;
use uuid::Uuid;

use crate::map_sqlx;

/// A disjoint-set (union-find) with path compression and union by size.
struct UnionFind {
    parent: HashMap<Uuid, Uuid>,
    size: HashMap<Uuid, u32>,
}

impl UnionFind {
    fn new() -> Self {
        Self {
            parent: HashMap::new(),
            size: HashMap::new(),
        }
    }

    fn add(&mut self, x: Uuid) {
        self.parent.entry(x).or_insert(x);
        self.size.entry(x).or_insert(1);
    }

    fn find(&mut self, x: Uuid) -> Uuid {
        let mut root = x;
        while self.parent[&root] != root {
            root = self.parent[&root];
        }
        // Path compression.
        let mut cur = x;
        while cur != root {
            let next = self.parent[&cur];
            self.parent.insert(cur, root);
            cur = next;
        }
        root
    }

    fn union(&mut self, a: Uuid, b: Uuid) {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra == rb {
            return;
        }
        let (big, small) = if self.size[&ra] >= self.size[&rb] {
            (ra, rb)
        } else {
            (rb, ra)
        };
        self.parent.insert(small, big);
        let s = self.size[&small];
        *self.size.get_mut(&big).unwrap() += s;
    }
}

/// Recompute every entity's `community_id` as the connected component of the
/// co-occurrence graph it belongs to. Deterministic: components are numbered by
/// their smallest member id, so ids are stable across runs given the same graph.
/// Returns the number of distinct communities.
pub async fn detect(pool: &PgPool, hub_max_degree: i64) -> Result<i64> {
    let entity_ids: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM entities")
        .fetch_all(pool)
        .await
        .map_err(map_sqlx)?;
    if entity_ids.is_empty() {
        return Ok(0);
    }
    let edges: Vec<(Uuid, Uuid)> =
        sqlx::query_as("SELECT src_id, dst_id FROM entity_edges WHERE rel = 'co_occurs'")
            .fetch_all(pool)
            .await
            .map_err(map_sqlx)?;

    // Hub guard: an entity that co-occurs with a huge number of others (a generic
    // date, a ubiquitous tool name) would otherwise chain every community into one
    // blob. Above `hub_max_degree` it is not allowed to merge communities, so the
    // structure stays meaningful. A non-positive cap disables the guard.
    let mut degree: HashMap<Uuid, i64> = HashMap::new();
    for (a, b) in &edges {
        *degree.entry(*a).or_insert(0) += 1;
        *degree.entry(*b).or_insert(0) += 1;
    }
    let is_hub =
        |id: &Uuid| hub_max_degree > 0 && degree.get(id).copied().unwrap_or(0) > hub_max_degree;

    let mut uf = UnionFind::new();
    for id in &entity_ids {
        uf.add(*id);
    }
    for (a, b) in &edges {
        uf.add(*a);
        uf.add(*b);
        if is_hub(a) || is_hub(b) {
            continue;
        }
        uf.union(*a, *b);
    }

    // Number components deterministically by their smallest member id.
    let mut smallest: HashMap<Uuid, Uuid> = HashMap::new();
    for id in &entity_ids {
        let root = uf.find(*id);
        let entry = smallest.entry(root).or_insert(*id);
        if *id < *entry {
            *entry = *id;
        }
    }
    let mut number: HashMap<Uuid, i32> = HashMap::new();
    let mut roots: Vec<Uuid> = smallest.keys().copied().collect();
    roots.sort_unstable_by_key(|r| smallest[r]);
    for (i, r) in roots.iter().enumerate() {
        number.insert(*r, i32::try_from(i).unwrap_or(i32::MAX));
    }

    let mut ids = Vec::with_capacity(entity_ids.len());
    let mut cids = Vec::with_capacity(entity_ids.len());
    for id in &entity_ids {
        ids.push(*id);
        cids.push(number[&uf.find(*id)]);
    }

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

    Ok(i64::try_from(roots.len()).unwrap_or(i64::MAX))
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
