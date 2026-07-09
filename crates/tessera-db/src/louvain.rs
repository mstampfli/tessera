//! Weighted Louvain community detection (modularity optimization).
//!
//! This replaces connected-components (single-linkage) community assignment. A
//! hub that connects to everything barely raises modularity, so it does not force
//! separate groups to merge - hubs are handled by the objective itself, with no
//! degree cap to tune - and the edge weights (idf-damped co-occurrence) are used
//! rather than discarded. Deterministic: nodes are processed in index order and
//! ties break toward the lower community id, so the same graph always yields the
//! same partition.

use std::collections::HashMap;

/// Detect communities in an undirected weighted graph over nodes `0..n`.
/// `edges` are `(u, v, weight)` with `weight > 0`; parallel edges are summed.
/// Returns a community label in `0..k` for each node, contiguous and stable.
#[must_use]
pub fn communities(n: usize, edges: &[(usize, usize, f64)]) -> Vec<usize> {
    if n == 0 {
        return Vec::new();
    }
    let mut graph = Graph::from_edges(n, edges);
    // `labels[original] = super-node index at the current level`.
    let mut labels: Vec<usize> = (0..n).collect();

    loop {
        let mut comm = graph.local_move();
        let k = renumber_in_place(&mut comm);
        // Converged when local moving no longer merges any super-node.
        if k == graph.n {
            break;
        }
        for l in &mut labels {
            *l = comm[*l];
        }
        if k == 1 {
            break;
        }
        graph = graph.aggregate(&comm, k);
    }
    renumber_in_place(&mut labels);
    labels
}

/// A weighted undirected graph with per-node neighbour lists and self-loops.
struct Graph {
    n: usize,
    /// `adj[i]` = neighbours `j != i` with the summed edge weight.
    adj: Vec<HashMap<usize, f64>>,
    /// Self-loop weight per node (accumulated during aggregation).
    selfw: Vec<f64>,
    /// Weighted degree: sum of incident edge weights, a self-loop counting twice.
    deg: Vec<f64>,
    /// Total edge weight (self-loops counted once); `2m = sum(deg)`.
    m: f64,
}

impl Graph {
    fn from_edges(n: usize, edges: &[(usize, usize, f64)]) -> Self {
        let mut adj: Vec<HashMap<usize, f64>> = vec![HashMap::new(); n];
        let mut selfw = vec![0.0; n];
        for &(u, v, w) in edges {
            if u >= n || v >= n || w <= 0.0 {
                continue;
            }
            if u == v {
                selfw[u] += w;
            } else {
                *adj[u].entry(v).or_insert(0.0) += w;
                *adj[v].entry(u).or_insert(0.0) += w;
            }
        }
        Self::finish(n, adj, selfw)
    }

    fn finish(n: usize, adj: Vec<HashMap<usize, f64>>, selfw: Vec<f64>) -> Self {
        let deg: Vec<f64> = adj
            .iter()
            .zip(&selfw)
            .map(|(a, &s)| a.values().sum::<f64>() + 2.0 * s)
            .collect();
        let m: f64 = deg.iter().sum::<f64>() / 2.0;
        Graph {
            n,
            adj,
            selfw,
            deg,
            m,
        }
    }

    /// One pass of local moving: each node starts in its own community; each is
    /// then greedily moved to the neighbouring community giving the best
    /// modularity gain, repeated until no node moves. Returns `community[node]`.
    fn local_move(&self) -> Vec<usize> {
        let mut comm: Vec<usize> = (0..self.n).collect();
        let mut tot: Vec<f64> = self.deg.clone(); // sum of degrees per community
        let two_m = 2.0 * self.m;
        if two_m <= 0.0 {
            return comm;
        }

        let mut improved = true;
        let mut guard = 0;
        while improved && guard < 100 {
            improved = false;
            guard += 1;
            for i in 0..self.n {
                let ci = comm[i];
                // Weight from i into each candidate community.
                let mut k_in: HashMap<usize, f64> = HashMap::new();
                for (&j, &w) in &self.adj[i] {
                    *k_in.entry(comm[j]).or_insert(0.0) += w;
                }
                // Remove i from its community.
                tot[ci] -= self.deg[i];
                // Best gain over own + neighbouring communities; staying isolated = 0.
                let mut best_c = ci;
                let mut best_gain =
                    k_in.get(&ci).copied().unwrap_or(0.0) - tot[ci] * self.deg[i] / two_m;
                for (&c, &kic) in &k_in {
                    let gain = kic - tot[c] * self.deg[i] / two_m;
                    if gain > best_gain + 1e-12 || (gain > best_gain - 1e-12 && c < best_c) {
                        best_gain = gain;
                        best_c = c;
                    }
                }
                tot[best_c] += self.deg[i];
                if best_c != ci {
                    comm[i] = best_c;
                    improved = true;
                }
            }
        }
        comm
    }

    /// Aggregate each community into one super-node; edge weights between
    /// communities sum, and intra-community weight becomes the super-node's
    /// self-loop, preserving total weight `m`.
    fn aggregate(&self, comm: &[usize], k: usize) -> Graph {
        let mut adj: Vec<HashMap<usize, f64>> = vec![HashMap::new(); k];
        let mut selfw = vec![0.0; k];
        for i in 0..self.n {
            let ci = comm[i];
            selfw[ci] += self.selfw[i];
            for (&j, &w) in &self.adj[i] {
                let cj = comm[j];
                if ci == cj {
                    // Counted from both i and j, so this self-loop sum double
                    // counts the intra edge; halve it.
                    selfw[ci] += w / 2.0;
                } else {
                    *adj[ci].entry(cj).or_insert(0.0) += w;
                }
            }
        }
        // adj is symmetric and double-counts inter-community edges (from both
        // endpoints); halve to get the true weight.
        for m in &mut adj {
            for w in m.values_mut() {
                *w /= 2.0;
            }
        }
        Graph::finish(k, adj, selfw)
    }
}

/// Renumber labels to a contiguous `0..k` (first-appearance order), in place,
/// returning `k`.
fn renumber_in_place(labels: &mut [usize]) -> usize {
    let mut map: HashMap<usize, usize> = HashMap::new();
    let mut next = 0;
    for l in &mut *labels {
        let id = *map.entry(*l).or_insert_with(|| {
            let v = next;
            next += 1;
            v
        });
        *l = id;
    }
    next
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_cliques_two_communities() {
        // Two triangles {0,1,2} and {3,4,5}, joined by one weak bridge 2-3.
        let edges = vec![
            (0, 1, 1.0),
            (1, 2, 1.0),
            (0, 2, 1.0),
            (3, 4, 1.0),
            (4, 5, 1.0),
            (3, 5, 1.0),
            (2, 3, 0.1),
        ];
        let c = communities(6, &edges);
        assert_eq!(c[0], c[1]);
        assert_eq!(c[1], c[2]);
        assert_eq!(c[3], c[4]);
        assert_eq!(c[4], c[5]);
        assert_ne!(c[0], c[3], "the two cliques must be different communities");
    }

    #[test]
    fn hub_does_not_merge_communities() {
        // Two triangles, plus a hub (6) weakly connected to everyone. The hub
        // must not chain the two cliques into one community.
        let mut edges = vec![
            (0, 1, 1.0),
            (1, 2, 1.0),
            (0, 2, 1.0),
            (3, 4, 1.0),
            (4, 5, 1.0),
            (3, 5, 1.0),
        ];
        for i in 0..6 {
            edges.push((6, i, 0.1));
        }
        let c = communities(7, &edges);
        assert_eq!(c[0], c[2]);
        assert_eq!(c[3], c[5]);
        assert_ne!(c[0], c[3], "a hub must not collapse the two cliques");
    }

    #[test]
    fn empty_and_singletons() {
        assert!(communities(0, &[]).is_empty());
        assert_eq!(communities(3, &[]).len(), 3);
    }
}
