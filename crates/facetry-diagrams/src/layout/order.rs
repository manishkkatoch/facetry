//! Ordering: the left-to-right order of the nodes in each layer, chosen to minimise edge
//! crossings. Layered graphs with long edges already split into chains of dummy nodes, so
//! every edge joins consecutive layers.
//!
//! The heuristic is the classic one (Sugiyama; Gansner et al.): start from a depth-first
//! order, then sweep down and up the layers sorting each by the **barycenter** (mean position)
//! of its neighbours in the layer before, with a **transpose** pass (swap neighbours while that
//! removes crossings) after each sweep. The best ordering seen is kept. Crossings are counted
//! with Barth, Jünger and Mutzel's accumulator tree (O(E log V) per pair of layers).
//!
//! Everything is deterministic and ties keep the current order, so the author's node order
//! survives wherever it does not cost crossings.

/// Sweeps without an improvement before giving up, and sweeps at most.
const PATIENCE: usize = 4;
const MAX_SWEEPS: usize = 24;

/// A layered graph for ordering: `down[v]` are v's neighbours one layer below, `up[v]` one
/// layer above, each with the edge's weight.
#[derive(Debug, Clone)]
pub struct Layers {
    pub layers: Vec<Vec<usize>>,
    pub down: Vec<Vec<(usize, f64)>>,
    pub up: Vec<Vec<(usize, f64)>>,
}

impl Layers {
    /// The position of every node within its layer.
    pub fn positions(&self) -> Vec<usize> {
        let n = self.down.len();
        let mut pos = vec![0; n];
        for layer in &self.layers {
            for (i, &v) in layer.iter().enumerate() {
                pos[v] = i;
            }
        }
        pos
    }

    /// Total crossings of the current order.
    pub fn crossings(&self) -> usize {
        let pos = self.positions();
        (1..self.layers.len())
            .map(|r| self.crossings_between(r - 1, &pos))
            .sum()
    }

    /// Crossings between layer `r` and `r + 1`.
    fn crossings_between(&self, r: usize, pos: &[usize]) -> usize {
        let (top, bottom) = (&self.layers[r], &self.layers[r + 1]);
        if top.len() < 2 && bottom.len() < 2 {
            return 0;
        }
        // Edges sorted by (top position, bottom position); count inversions of the bottom
        // positions with an accumulator tree.
        let mut targets: Vec<usize> = Vec::new();
        for &u in top {
            let mut ends: Vec<usize> = self.down[u].iter().map(|&(v, _)| pos[v]).collect();
            ends.sort_unstable();
            targets.extend(ends);
        }
        let mut first = 1;
        while first < bottom.len() {
            first *= 2;
        }
        let mut tree = vec![0usize; 2 * first];
        let mut count = 0;
        for t in targets {
            let mut i = t + first;
            tree[i] += 1;
            while i > 1 {
                if i % 2 == 0 {
                    count += tree[i + 1];
                }
                i /= 2;
                tree[i] += 1;
            }
        }
        count
    }

    /// Orders every layer.
    pub fn order(&mut self) {
        self.initial_order();
        let mut best = self.layers.clone();
        let mut best_crossings = self.crossings();
        let mut idle = 0;
        for sweep in 0..MAX_SWEEPS {
            if best_crossings == 0 || idle >= PATIENCE {
                break;
            }
            let downward = sweep % 2 == 0;
            self.sweep(downward);
            self.transpose();
            let c = self.crossings();
            if c < best_crossings {
                best_crossings = c;
                best.clone_from(&self.layers);
                idle = 0;
            } else {
                idle += 1;
            }
        }
        self.layers = best;
    }

    /// Depth-first from the nodes of the first layers, in input order: connected nodes start
    /// next to each other, which is already a good order for trees.
    fn initial_order(&mut self) {
        let n = self.down.len();
        let mut layer_of = vec![0; n];
        for (r, layer) in self.layers.iter().enumerate() {
            for &v in layer {
                layer_of[v] = r;
            }
        }
        let starts: Vec<usize> = self.layers.iter().flatten().copied().collect();
        let mut seen = vec![false; n];
        let mut out: Vec<Vec<usize>> = vec![Vec::new(); self.layers.len()];
        for s in starts {
            if seen[s] {
                continue;
            }
            let mut stack = vec![s];
            while let Some(v) = stack.pop() {
                if seen[v] {
                    continue;
                }
                seen[v] = true;
                out[layer_of[v]].push(v);
                // Push in reverse so the first neighbour is visited first.
                for &(w, _) in self.down[v].iter().rev() {
                    if !seen[w] {
                        stack.push(w);
                    }
                }
            }
        }
        self.layers = out;
    }

    /// One pass sorting each layer by the barycenters of its neighbours in the previous layer
    /// (above when going down, below when going up). Nodes without such neighbours keep their
    /// place.
    fn sweep(&mut self, downward: bool) {
        let count = self.layers.len();
        let order: Vec<usize> = if downward {
            (1..count).collect()
        } else {
            (0..count.saturating_sub(1)).rev().collect()
        };
        for r in order {
            let pos = self.positions();
            let neighbours = if downward { &self.up } else { &self.down };
            let layer = &self.layers[r];
            let bary: Vec<Option<f64>> = layer
                .iter()
                .map(|&v| {
                    let ns = &neighbours[v];
                    let total: f64 = ns.iter().map(|&(_, w)| w).sum();
                    (total > 0.0)
                        .then(|| ns.iter().map(|&(u, w)| pos[u] as f64 * w).sum::<f64>() / total)
                })
                .collect();
            self.layers[r] = sort_keeping_free(layer, &bary);
        }
    }

    /// Swaps neighbours in a layer while that lowers the crossings with both adjacent layers.
    fn transpose(&mut self) {
        let mut improved = true;
        let mut rounds = 0;
        while improved && rounds < 8 {
            improved = false;
            rounds += 1;
            for r in 0..self.layers.len() {
                for i in 1..self.layers[r].len() {
                    let pos = self.positions();
                    let (a, b) = (self.layers[r][i - 1], self.layers[r][i]);
                    let before = self.pair_crossings(a, b, &pos);
                    let after = self.pair_crossings(b, a, &pos);
                    if after < before {
                        self.layers[r].swap(i - 1, i);
                        improved = true;
                    }
                }
            }
        }
    }

    /// Crossings among the edges of `a` and `b` (a left of b) with both neighbouring layers.
    fn pair_crossings(&self, a: usize, b: usize, pos: &[usize]) -> usize {
        let mut c = 0;
        for adj in [&self.up, &self.down] {
            for &(u, _) in &adj[a] {
                for &(v, _) in &adj[b] {
                    if pos[u] > pos[v] {
                        c += 1;
                    }
                }
            }
        }
        c
    }
}

/// Sorts `layer` by `keys` (stable), leaving the nodes without a key at their indices.
fn sort_keeping_free(layer: &[usize], keys: &[Option<f64>]) -> Vec<usize> {
    let mut keyed: Vec<(f64, usize)> = layer
        .iter()
        .zip(keys)
        .filter_map(|(&v, k)| k.map(|k| (k, v)))
        .collect();
    keyed.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut keyed = keyed.into_iter().map(|(_, v)| v);
    layer
        .iter()
        .zip(keys)
        .map(|(&v, k)| {
            if k.is_some() {
                keyed.next().unwrap_or(v)
            } else {
                v
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Layers from edges between consecutive layers.
    fn layers(layers: &[&[usize]], edges: &[(usize, usize)]) -> Layers {
        let n = layers.iter().map(|l| l.len()).sum();
        let mut down = vec![Vec::new(); n];
        let mut up = vec![Vec::new(); n];
        for &(a, b) in edges {
            down[a].push((b, 1.0));
            up[b].push((a, 1.0));
        }
        Layers {
            layers: layers.iter().map(|l| l.to_vec()).collect(),
            down,
            up,
        }
    }

    #[test]
    fn crossings_are_counted() {
        // 0 1 over 2 3, edges 0-3 and 1-2 cross once.
        assert_eq!(
            layers(&[&[0, 1], &[2, 3]], &[(0, 3), (1, 2)]).crossings(),
            1
        );
        assert_eq!(
            layers(&[&[0, 1], &[2, 3]], &[(0, 2), (1, 3)]).crossings(),
            0
        );
        // K(3,3) drawn in order: 9 edges, 9 crossings... (3 choose 2)^2 = 9.
        let mut e = Vec::new();
        for a in 0..3 {
            for b in 3..6 {
                e.push((a, b));
            }
        }
        assert_eq!(layers(&[&[0, 1, 2], &[3, 4, 5]], &e).crossings(), 9);
    }

    #[test]
    fn a_crossing_that_can_be_removed_is() {
        let mut l = layers(
            &[&[0, 1], &[2, 3], &[4, 5]],
            &[(0, 3), (1, 2), (2, 4), (3, 5)],
        );
        l.order();
        assert_eq!(l.crossings(), 0);
    }

    #[test]
    fn free_nodes_keep_their_place() {
        let got = sort_keeping_free(&[10, 11, 12, 13], &[Some(3.0), None, Some(1.0), Some(2.0)]);
        assert_eq!(got, [12, 11, 13, 10]);
    }
}
