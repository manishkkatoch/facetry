//! Coordinates within layers by Brandes and Köpf ("Fast and Simple Horizontal Coordinate
//! Assignment", 2001), as dagre does it: four alignments (each pairing nodes with their upper or
//! lower median neighbour, scanning from the left or the right) are compacted, shifted onto the
//! narrowest one, and every node takes the mean of its two middle candidates. Long edges (chains
//! of dummy nodes) come out straight, and nodes sit between their neighbours.

use std::collections::{BTreeMap, HashSet};

/// What positioning needs to know about the layered graph.
#[derive(Debug)]
pub struct Input<'a> {
    pub layers: &'a [Vec<usize>],
    pub up: &'a [Vec<(usize, f64)>],
    pub down: &'a [Vec<(usize, f64)>],
    pub width: &'a [f64],
    pub dummy: &'a [bool],
    /// Space between two real nodes, and the share a dummy node (an edge) takes.
    pub node_sep: f64,
    pub edge_sep: f64,
}

/// The x of every node's centre.
pub fn assign_x(input: &Input) -> Vec<f64> {
    let n = input.width.len();
    if n == 0 {
        return Vec::new();
    }
    let conflicts = type1_conflicts(input);
    let mut candidates: Vec<Vec<f64>> = Vec::with_capacity(4);
    for upward in [true, false] {
        let mut layering: Vec<Vec<usize>> = input.layers.to_vec();
        if !upward {
            layering.reverse();
        }
        for leftward in [true, false] {
            let layering: Vec<Vec<usize>> = if leftward {
                layering.clone()
            } else {
                layering
                    .iter()
                    .map(|l| l.iter().rev().copied().collect())
                    .collect()
            };
            let neighbours = if upward { input.up } else { input.down };
            let (root, align) = vertical_alignment(&layering, neighbours, &conflicts, n);
            let mut xs = horizontal_compaction(input, &layering, &root, &align);
            if !leftward {
                xs.iter_mut().for_each(|x| *x = -*x);
            }
            candidates.push(xs);
        }
    }
    // Align every candidate to the narrowest: left scans by their minimum, right scans by
    // their maximum.
    let extent = |xs: &[f64]| {
        let lo = (0..n)
            .map(|v| xs[v] - input.width[v] / 2.0)
            .fold(f64::INFINITY, f64::min);
        let hi = (0..n)
            .map(|v| xs[v] + input.width[v] / 2.0)
            .fold(f64::NEG_INFINITY, f64::max);
        (lo, hi)
    };
    let narrowest = (0..4)
        .min_by(|&a, &b| {
            let (la, ha) = extent(&candidates[a]);
            let (lb, hb) = extent(&candidates[b]);
            (ha - la).total_cmp(&(hb - lb)).then(a.cmp(&b))
        })
        .unwrap_or(0);
    let (target_lo, target_hi) = extent(&candidates[narrowest]);
    for (i, xs) in candidates.iter_mut().enumerate() {
        let (lo, hi) = extent(xs);
        let leftward = i % 2 == 0;
        let delta = if leftward {
            target_lo - lo
        } else {
            target_hi - hi
        };
        xs.iter_mut().for_each(|x| *x += delta);
    }
    let mut xs: Vec<f64> = (0..n)
        .map(|v| {
            let mut four = [
                candidates[0][v],
                candidates[1][v],
                candidates[2][v],
                candidates[3][v],
            ];
            four.sort_by(f64::total_cmp);
            (four[1] + four[2]) / 2.0
        })
        .collect();
    // Each candidate keeps nodes apart, but a mix of candidates (each node takes its own
    // middle two) can squeeze two neighbours: push any such pair apart.
    for layer in input.layers {
        for i in 1..layer.len() {
            let (u, v) = (layer[i - 1], layer[i]);
            let min = xs[u]
                + input.width[u] / 2.0
                + gap(input, u) / 2.0
                + gap(input, v) / 2.0
                + input.width[v] / 2.0;
            if xs[v] < min - 1e-9 {
                xs[v] = min;
            }
        }
    }
    xs
}

/// Type 1 conflicts: an edge between two dummies (an inner segment of a long edge) crossed
/// by an edge with a real end. The inner segment wins: the other edge is never aligned, so
/// long edges stay straight. Stored as unordered pairs.
fn type1_conflicts(input: &Input) -> HashSet<(usize, usize)> {
    let mut conflicts = HashSet::new();
    let pos = positions(input.layers, input.width.len());
    for r in 1..input.layers.len() {
        let (prev, layer) = (&input.layers[r - 1], &input.layers[r]);
        let mut k0 = 0usize;
        let mut scan = 0usize;
        let last = layer.last().copied();
        for (i, &v) in layer.iter().enumerate() {
            // The upper end of v's inner segment, if v is a dummy fed by a dummy.
            let w = if input.dummy[v] {
                input.up[v]
                    .iter()
                    .map(|&(u, _)| u)
                    .find(|&u| input.dummy[u])
            } else {
                None
            };
            let k1 = w.map_or(prev.len(), |w| pos[w]);
            if w.is_some() || Some(v) == last {
                for &s in &layer[scan..=i] {
                    for &(u, _) in &input.up[s] {
                        let p = pos[u];
                        if (p < k0 || k1 < p) && !(input.dummy[u] && input.dummy[s]) {
                            conflicts.insert(pair(u, s));
                        }
                    }
                }
                scan = i + 1;
                k0 = k1;
            }
        }
    }
    conflicts
}

fn pair(a: usize, b: usize) -> (usize, usize) {
    (a.min(b), a.max(b))
}

fn positions(layers: &[Vec<usize>], n: usize) -> Vec<usize> {
    let mut pos = vec![0; n];
    for layer in layers {
        for (i, &v) in layer.iter().enumerate() {
            pos[v] = i;
        }
    }
    pos
}

/// Groups nodes into vertical blocks: each node joins the block of one of its median
/// neighbours in the previous layer of `layering`, unless that would cross an alignment
/// already made or a type 1 conflict. Returns each node's block root and the next node down
/// the block (cyclic).
fn vertical_alignment(
    layering: &[Vec<usize>],
    neighbours: &[Vec<(usize, f64)>],
    conflicts: &HashSet<(usize, usize)>,
    n: usize,
) -> (Vec<usize>, Vec<usize>) {
    let mut root: Vec<usize> = (0..n).collect();
    let mut align: Vec<usize> = (0..n).collect();
    let pos = positions(layering, n);
    for layer in layering {
        let mut prev_idx: isize = -1;
        for &v in layer {
            let mut ws: Vec<usize> = neighbours[v].iter().map(|&(u, _)| u).collect();
            if ws.is_empty() {
                continue;
            }
            ws.sort_by_key(|&u| pos[u]);
            // The median neighbour, or both middle ones of an even count, left first.
            let (lo, hi) = ((ws.len() - 1) / 2, ws.len() / 2);
            for &w in &ws[lo..=hi] {
                if align[v] == v && prev_idx < pos[w] as isize && !conflicts.contains(&pair(v, w)) {
                    align[w] = v;
                    root[v] = root[w];
                    align[v] = root[v];
                    prev_idx = pos[w] as isize;
                }
            }
        }
    }
    (root, align)
}

/// Places the blocks: a block graph (block A before block B in some layer, at least their
/// separation apart) is laid out by longest path from the left, then each block slides right
/// as far as its successors allow, which centres blocks that only had room on one side.
fn horizontal_compaction(
    input: &Input,
    layering: &[Vec<usize>],
    root: &[usize],
    align: &[usize],
) -> Vec<f64> {
    let n = root.len();
    let _ = align;
    // Block graph edges: (from root, to root) -> min separation.
    let mut sep: BTreeMap<(usize, usize), f64> = BTreeMap::new();
    for layer in layering {
        for w in layer.windows(2) {
            let (u, v) = (w[0], w[1]);
            let s = input.width[u] / 2.0
                + gap(input, u) / 2.0
                + gap(input, v) / 2.0
                + input.width[v] / 2.0;
            let key = (root[u], root[v]);
            let entry = sep.entry(key).or_insert(0.0);
            *entry = entry.max(s);
        }
    }
    let mut preds: Vec<Vec<(usize, f64)>> = vec![Vec::new(); n];
    let mut succs: Vec<Vec<(usize, f64)>> = vec![Vec::new(); n];
    for (&(a, b), &s) in &sep {
        succs[a].push((b, s));
        preds[b].push((a, s));
    }
    let roots: Vec<usize> = (0..n).filter(|&v| root[v] == v).collect();
    // Topological order of the block graph (it is acyclic by construction).
    let mut indegree = vec![0usize; n];
    for &(_, b) in sep.keys() {
        indegree[b] += 1;
    }
    let mut ready: Vec<usize> = roots
        .iter()
        .copied()
        .filter(|&r| indegree[r] == 0)
        .rev()
        .collect();
    let mut topo = Vec::with_capacity(roots.len());
    while let Some(v) = ready.pop() {
        topo.push(v);
        for &(w, _) in succs[v].iter().rev() {
            indegree[w] -= 1;
            if indegree[w] == 0 {
                ready.push(w);
            }
        }
    }
    let mut xs = vec![0.0; n];
    // Pass 1: as far left as the predecessors allow.
    for &v in &topo {
        xs[v] = preds[v].iter().map(|&(u, s)| xs[u] + s).fold(0.0, f64::max);
    }
    // Pass 2: as far right as the successors allow, never left of pass 1.
    for &v in topo.iter().rev() {
        let limit = succs[v]
            .iter()
            .map(|&(w, s)| xs[w] - s)
            .fold(f64::INFINITY, f64::min);
        if limit.is_finite() {
            xs[v] = xs[v].max(limit);
        }
    }
    (0..n).map(|v| xs[root[v]]).collect()
}

/// The separation a node asks for on each side: edges pack closer than nodes.
fn gap(input: &Input, v: usize) -> f64 {
    if input.dummy[v] {
        input.edge_sep
    } else {
        input.node_sep
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_node_sits_between_its_two_children() {
        // 0 over 1 2.
        let layers = vec![vec![0], vec![1, 2]];
        let up = vec![vec![], vec![(0, 1.0)], vec![(0, 1.0)]];
        let down = vec![vec![(1, 1.0), (2, 1.0)], vec![], vec![]];
        let width = vec![40.0, 40.0, 40.0];
        let dummy = vec![false; 3];
        let xs = assign_x(&Input {
            layers: &layers,
            up: &up,
            down: &down,
            width: &width,
            dummy: &dummy,
            node_sep: 50.0,
            edge_sep: 20.0,
        });
        assert!((xs[2] - xs[1] - 90.0).abs() < 1e-9, "{xs:?}");
        assert!((xs[0] - (xs[1] + xs[2]) / 2.0).abs() < 1e-9, "{xs:?}");
    }

    #[test]
    fn nodes_in_a_layer_never_overlap() {
        let layers = vec![vec![0, 1, 2], vec![3, 4], vec![5, 6, 7]];
        let edges = [(0, 3), (1, 3), (2, 4), (3, 5), (3, 6), (4, 7), (0, 4)];
        let mut up = vec![Vec::new(); 8];
        let mut down = vec![Vec::new(); 8];
        for &(a, b) in &edges {
            down[a].push((b, 1.0));
            up[b].push((a, 1.0));
        }
        let width = vec![30.0, 80.0, 20.0, 60.0, 40.0, 10.0, 90.0, 30.0];
        let dummy = vec![false; 8];
        let xs = assign_x(&Input {
            layers: &layers,
            up: &up,
            down: &down,
            width: &width,
            dummy: &dummy,
            node_sep: 50.0,
            edge_sep: 20.0,
        });
        for layer in &layers {
            for w in layer.windows(2) {
                let (a, b) = (w[0], w[1]);
                let gap = xs[b] - xs[a] - (width[a] + width[b]) / 2.0;
                assert!(gap >= 50.0 - 1e-9, "{a} {b} {xs:?}");
            }
        }
    }
}
