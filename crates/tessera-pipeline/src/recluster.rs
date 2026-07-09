//! Batch reclustering of chunk embeddings with HDBSCAN.
//!
//! The online nearest-centroid assignment is fast but, on an embedding model
//! that packs short technical text tightly, its centroid drifts and absorbs
//! everything into one cluster. HDBSCAN is density-based: it finds clusters as
//! dense regions and leaves the rest as noise, so distinct topics stay apart and
//! nothing is forced together. Cosine geometry is obtained by L2-normalizing the
//! vectors and letting HDBSCAN use Euclidean distance (equivalent on the sphere).
//!
//! Cluster ids are kept stable across a recluster by matching each new group to
//! the existing cluster it most overlaps, so insights and labels do not churn.

use std::collections::{HashMap, HashSet};

use hdbscan::{Hdbscan, HdbscanHyperParams};
use serde_json::json;
use tessera_core::error::{Error, ErrorKind, Result};
use tessera_db::queue::{self, EnqueueOpts};
use tessera_db::repos::clusters;
use tessera_db::Db;
use uuid::Uuid;

use crate::KIND_SYNTHESIZE_INSIGHT;

/// Outcome of a recluster pass.
pub struct Reclustered {
    pub clusters: usize,
    pub noise: usize,
    pub changed: usize,
}

fn normalize(v: &[f32]) -> Vec<f32> {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        v.iter().map(|x| x / norm).collect()
    } else {
        v.to_vec()
    }
}

/// Recluster all chunk embeddings in the space. Returns without touching the
/// existing clustering if HDBSCAN finds no dense cluster at all (so a sparse or
/// tiny corpus never loses its clusters/insights). Enqueues synthesis (debounced)
/// for every cluster whose membership actually changed.
pub async fn run(
    db: &Db,
    space_id: i16,
    min_cluster_size: usize,
    synth_debounce_secs: i64,
) -> Result<Reclustered> {
    let pool = &db.worker;
    let rows = clusters::chunk_embeddings_for_space(pool, space_id).await?;
    if rows.len() < min_cluster_size.max(2) {
        return Ok(Reclustered {
            clusters: 0,
            noise: rows.len(),
            changed: 0,
        });
    }

    let ids: Vec<Uuid> = rows.iter().map(|(id, _)| *id).collect();
    let data: Vec<Vec<f32>> = rows.iter().map(|(_, v)| normalize(v)).collect();

    let params = HdbscanHyperParams::builder()
        .min_cluster_size(min_cluster_size.max(2))
        .build();
    let labels = Hdbscan::new(&data, params)
        .cluster()
        .map_err(|e| Error::new(ErrorKind::Internal, format!("hdbscan: {e:?}")))?;

    // Group chunk ids by HDBSCAN label; label -1 is noise (left unclustered).
    let mut groups: HashMap<i32, Vec<Uuid>> = HashMap::new();
    let mut noise = 0usize;
    for (idx, &label) in labels.iter().enumerate() {
        if label < 0 {
            noise += 1;
            continue;
        }
        groups.entry(label).or_default().push(ids[idx]);
    }
    // Guard: never wipe the existing clustering if HDBSCAN found no clusters.
    if groups.is_empty() {
        return Ok(Reclustered {
            clusters: 0,
            noise,
            changed: 0,
        });
    }

    // Stable ids: match each new group to the existing cluster it overlaps most,
    // largest groups first, each existing id claimed at most once.
    let existing = clusters::members_by_cluster(pool, space_id).await?;
    let existing_sets: Vec<(Uuid, HashSet<Uuid>)> = existing
        .into_iter()
        .map(|(id, m)| (id, m.into_iter().collect()))
        .collect();

    let mut new_groups: Vec<Vec<Uuid>> = groups.into_values().collect();
    new_groups.sort_by_key(|g| std::cmp::Reverse(g.len()));

    let mut claimed: HashSet<Uuid> = HashSet::new();
    let mut assigned: Vec<(Uuid, Vec<Uuid>)> = Vec::new();
    let mut changed = 0usize;
    for group in new_groups {
        let gset: HashSet<Uuid> = group.iter().copied().collect();
        let best = existing_sets
            .iter()
            .filter(|(id, _)| !claimed.contains(id))
            .map(|(id, m)| (*id, gset.intersection(m).count()))
            .filter(|(_, overlap)| *overlap > 0)
            .max_by_key(|(_, overlap)| *overlap);

        let (id, unchanged) = match best {
            Some((old_id, _)) => {
                claimed.insert(old_id);
                let old = existing_sets
                    .iter()
                    .find(|(i, _)| *i == old_id)
                    .map(|(_, m)| m);
                let same = old.is_some_and(|m| m == &gset);
                (old_id, same)
            }
            None => (tessera_core::new_id(), false),
        };
        if !unchanged {
            changed += 1;
        }
        assigned.push((id, group));
    }

    clusters::apply_reclustering(pool, space_id, &assigned).await?;

    // Re-synthesize every applied cluster; the input-hash dedup in the synthesis
    // stage makes the unchanged ones a cheap no-op, so this stays correct.
    for (id, _) in &assigned {
        queue::enqueue(
            pool,
            KIND_SYNTHESIZE_INSIGHT,
            &json!({ "cluster_id": id }),
            &EnqueueOpts {
                dedupe_key: Some(format!("synth:{id}")),
                delay_secs: Some(synth_debounce_secs),
                ..Default::default()
            },
        )
        .await?;
    }

    Ok(Reclustered {
        clusters: assigned.len(),
        noise,
        changed,
    })
}
