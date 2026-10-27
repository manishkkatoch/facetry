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
//!
//! With groups (subgraphs) every layer is **regrouped** after each step: the members of a group
//! are gathered into one run, the group's border nodes at its ends, and the runs and the nodes
//! around them keep the order of their mean positions. Transposition only swaps two nodes of
//! the same innermost group, so a step never breaks a group apart.

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
    /// Groups, if the graph has any.
    pub nest: Option<Nest>,
}

/// A node being regrouped: itself, its position in the layer, and the groups round it
/// (outermost first).
type Item = (usize, f64, Vec<usize>);

/// Which side of its group a border node marks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

/// The groups of a layered graph: every node's innermost group, which nodes are a group's
/// border, and each group's parent.
#[derive(Debug, Clone)]
pub struct Nest {
    pub member: Vec<Option<usize>>,
    pub border: Vec<Option<(usize, Side)>>,
    pub parent: Vec<Option<usize>>,
}

impl Nest {
    /// The groups around `v`, outermost first.
    fn path(&self, v: usize) -> Vec<usize> {
        let mut path = Vec::new();
        let mut g = self.member[v];
        while let Some(i) = g {
            path.push(i);
            g = self.parent[i];
        }
        path.reverse();
        path
    }

    /// `layer` with every group's members together and its borders at its ends; otherwise
    /// in the order of the current positions (a run counts at the mean of its members').
    pub fn regroup(&self, layer: &[usize]) -> Vec<usize> {
        let items: Vec<Item> = layer
            .iter()
            .enumerate()
            .map(|(i, &v)| (v, i as f64, self.path(v)))
            .collect();
        let mut out = Vec::with_capacity(layer.len());
        self.arrange(&items, 0, &mut out);
        out
    }

    /// Lays out `items`, which all share their first `depth` groups.
    fn arrange(&self, items: &[Item], depth: usize, out: &mut Vec<usize>) {
        // Entries at this level: a node directly here, or the run of a group one level in.
        let mut entries: Vec<(f64, Option<usize>, Vec<&Item>)> = Vec::new();
        let (mut left, mut right) = (Vec::new(), Vec::new());
        for item in items {
            let (v, _, path) = item;
            match path.get(depth) {
                // A border of the group we are in: at its end.
                None if depth > 0 && self.border[*v].is_some() => {
                    match self.border[*v].map(|b| b.1) {
                        Some(Side::Left) => left.push(*v),
                        _ => right.push(*v),
                    }
                }
                None => entries.push((item.1, None, vec![item])),
                Some(&g) => match entries.iter_mut().find(|e| e.1 == Some(g)) {
                    Some(e) => e.2.push(item),
                    None => entries.push((0.0, Some(g), vec![item])),
                },
            }
        }
        // A run counts at the mean position of what it holds; its borders only follow the
        // borders above them, so they would pin the runs in their first order.
        for e in &mut entries {
            if e.1.is_some() {
                let inner: Vec<f64> =
                    e.2.iter()
                        .filter(|i| self.border[i.0].is_none())
                        .map(|i| i.1)
                        .collect();
                let keys: Vec<f64> = if inner.is_empty() {
                    e.2.iter().map(|i| i.1).collect()
                } else {
                    inner
                };
                e.0 = keys.iter().sum::<f64>() / keys.len() as f64;
            }
        }
        // Stable, by key: an insertion sort (a layer holds few runs, and the standard library's
        // sort would add kilobytes of wasm for this one entry type).
        for i in 1..entries.len() {
            let mut j = i;
            while j > 0 && entries[j - 1].0 > entries[j].0 {
                entries.swap(j - 1, j);
                j -= 1;
            }
        }
        out.extend(left);
        for (_, group, members) in entries {
            match group {
                None => out.push(members[0].0),
                Some(_) => {
                    let inner: Vec<Item> = members.into_iter().cloned().collect();
                    self.arrange(&inner, depth + 1, out);
                }
            }
        }
        out.extend(right);
    }

    /// Whether swapping neighbours `a` and `b` keeps every group together.
    fn may_swap(&self, a: usize, b: usize) -> bool {
        self.member[a] == self.member[b] && self.border[a].is_none() && self.border[b].is_none()
    }
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
        self.regroup();
        let mut best = self.layers.clone();
        let mut best_crossings = self.crossings();
        let mut idle = 0;
        for sweep in 0..MAX_SWEEPS {
            if best_crossings == 0 || idle >= PATIENCE {
                break;
            }
            let downward = sweep % 2 == 0;
            self.sweep(downward);
            self.regroup();
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

    /// Gathers every group's members (see [`Nest::regroup`]); nothing without groups.
    fn regroup(&mut self) {
        if let Some(nest) = &self.nest {
            for layer in &mut self.layers {
                *layer = nest.regroup(layer);
            }
        }
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
                    if self.nest.as_ref().is_some_and(|n| !n.may_swap(a, b)) {
                        continue;
                    }
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
            nest: None,
        }
    }

    #[test]
    fn groups_stay_together_with_their_borders_at_the_ends() {
        // Groups 0 (outer) and 1 (in 0). Nodes 0..6; 4 and 5 borders of group 1.
        let nest = Nest {
            member: vec![None, Some(0), Some(1), None, Some(1), Some(1), Some(0)],
            border: vec![
                None,
                None,
                None,
                None,
                Some((1, Side::Left)),
                Some((1, Side::Right)),
                None,
            ],
            parent: vec![None, Some(0)],
        };
        // Scattered: 1 (in 0) .. 3 (out) .. 2 (in 1) .. 6 (in 0)...
        let got = nest.regroup(&[1, 5, 3, 2, 0, 6, 4]);
        // Group 0's run: 1, then group 1's run [4, 2, 5], then 6; around it 3 and 0.
        let at = |v: usize| got.iter().position(|&x| x == v).unwrap();
        assert!(at(4) + 1 == at(2) && at(2) + 1 == at(5), "{got:?}");
        let run: Vec<usize> =
            got[at(1).min(at(6)).min(at(4))..=at(1).max(at(6)).max(at(5))].to_vec();
        assert_eq!(run.len(), 5, "group 0 together: {got:?}");
        assert!(!nest.may_swap(1, 4) && !nest.may_swap(1, 3) && nest.may_swap(0, 3));
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
