//! Layered ("Sugiyama") layout of a directed graph, top to bottom. Callers that want another
//! direction rotate the result (and swap node sizes going in).
//!
//! 1. Self-loops are set aside; cycles are broken by reversing some edges ([`rank`]).
//! 2. Every edge's minimum length is **doubled** and ranks are assigned by network simplex.
//!    Doubling (as dagre does) leaves a rank between any two connected nodes for the edge's
//!    label to sit in, so labels take real space instead of being drawn over other edges.
//! 3. Edges longer than one rank become chains of dummy nodes, one per rank; a labelled edge's
//!    middle dummy is as big as its label.
//! 4. The nodes of each rank are ordered to minimise crossings ([`order`]).
//! 5. x by Brandes-Köpf ([`position`]); y rank by rank, each rank as tall as its tallest node.
//!
//! The result gives each node its centre and each edge the polyline through its dummies, from
//! the centre of its source to the centre of its target; routing (clipping to the node
//! outlines, smoothing) is left to the caller, which knows the shapes.

pub mod order;
pub mod position;
pub mod rank;

use rank::{RankEdge, feedback_edges, network_simplex};

/// A node's size: `width` along its rank, `height` across ranks.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Size {
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Edge {
    pub from: usize,
    pub to: usize,
    /// Ranks the edge spans at least (1 = adjacent ranks).
    pub minlen: u32,
    pub weight: f64,
    pub label: Option<Size>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Graph {
    pub nodes: Vec<Size>,
    pub edges: Vec<Edge>,
}

/// Spacing, in layout units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Spacing {
    /// Between nodes in a rank.
    pub node: f64,
    /// Between edges (and between an edge and a node) in a rank.
    pub edge: f64,
    /// Between ranks.
    pub rank: f64,
}

impl Default for Spacing {
    /// Mermaid's defaults for nodes and ranks.
    fn default() -> Self {
        Self {
            node: 50.0,
            edge: 20.0,
            rank: 50.0,
        }
    }
}

/// An edge's path through the layout.
#[derive(Debug, Clone, PartialEq)]
pub struct Route {
    /// From the centre of `from` through each dummy to the centre of `to`; a self-loop has
    /// just the node's centre twice (the caller draws the loop).
    pub points: Vec<[f64; 2]>,
    /// The middle of the label, if the edge has one.
    pub label: Option<[f64; 2]>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    pub nodes: Vec<[f64; 2]>,
    pub edges: Vec<Route>,
    /// The rank of every node (after doubling: real nodes on adjacent ranks are 2 apart).
    pub ranks: Vec<i32>,
}

/// Lays `graph` out top to bottom.
pub fn layout(graph: &Graph, spacing: Spacing) -> Layout {
    let n = graph.nodes.len();
    let pairs: Vec<(usize, usize)> = graph.edges.iter().map(|e| (e.from, e.to)).collect();
    let reversed = feedback_edges(n, &pairs);
    // Oriented edges (after reversal), self-loops left out.
    let oriented: Vec<Option<(usize, usize)>> = graph
        .edges
        .iter()
        .zip(&reversed)
        .map(|(e, &rev)| {
            (e.from != e.to).then_some(if rev { (e.to, e.from) } else { (e.from, e.to) })
        })
        .collect();
    let rank_edges: Vec<RankEdge> = graph
        .edges
        .iter()
        .zip(&oriented)
        .filter_map(|(e, o)| {
            let (from, to) = (*o)?;
            Some(RankEdge {
                from,
                to,
                minlen: 2 * e.minlen.max(1) as i32,
                weight: e.weight,
            })
        })
        .collect();
    let ranks = network_simplex(n, &rank_edges);

    // --- dummy nodes ------------------------------------------------------------------------
    let mut width: Vec<f64> = graph.nodes.iter().map(|s| s.width).collect();
    let mut height: Vec<f64> = graph.nodes.iter().map(|s| s.height).collect();
    let mut rank_of: Vec<i32> = ranks.clone();
    let mut dummy = vec![false; n];
    let mut chains: Vec<Vec<usize>> = Vec::with_capacity(graph.edges.len());
    let mut label_node: Vec<Option<usize>> = vec![None; graph.edges.len()];
    let mut down: Vec<Vec<(usize, f64)>> = vec![Vec::new(); n];
    let mut up: Vec<Vec<(usize, f64)>> = vec![Vec::new(); n];
    for (i, (e, o)) in graph.edges.iter().zip(&oriented).enumerate() {
        let Some((u, v)) = *o else {
            chains.push(Vec::new());
            continue;
        };
        let (ru, rv) = (rank_of[u], rank_of[v]);
        let label_rank = ru + (rv - ru) / 2;
        let mut chain = vec![u];
        for r in ru + 1..rv {
            let d = width.len();
            let (w, h) = match (e.label, r == label_rank) {
                (Some(s), true) => {
                    label_node[i] = Some(d);
                    (s.width, s.height)
                }
                _ => (0.0, 0.0),
            };
            width.push(w);
            height.push(h);
            rank_of.push(r);
            dummy.push(true);
            down.push(Vec::new());
            up.push(Vec::new());
            chain.push(d);
        }
        chain.push(v);
        for w in chain.windows(2) {
            down[w[0]].push((w[1], e.weight));
            up[w[1]].push((w[0], e.weight));
        }
        chains.push(chain);
    }

    // --- order --------------------------------------------------------------------------------
    let max_rank = rank_of.iter().copied().max().unwrap_or(0).max(0) as usize;
    let mut layers: Vec<Vec<usize>> = vec![Vec::new(); max_rank + 1];
    for (v, &r) in rank_of.iter().enumerate() {
        layers[r.max(0) as usize].push(v);
    }
    let mut ordering = order::Layers { layers, down, up };
    ordering.order();

    // --- coordinates ----------------------------------------------------------------------------
    let xs = position::assign_x(&position::Input {
        layers: &ordering.layers,
        up: &ordering.up,
        down: &ordering.down,
        width: &width,
        dummy: &dummy,
        node_sep: spacing.node,
        edge_sep: spacing.edge,
    });
    let rank_height: Vec<f64> = ordering
        .layers
        .iter()
        .map(|l| l.iter().map(|&v| height[v]).fold(0.0, f64::max))
        .collect();
    let mut rank_y = Vec::with_capacity(rank_height.len());
    let mut y = 0.0;
    for (r, &h) in rank_height.iter().enumerate() {
        if r > 0 {
            y += rank_height[r - 1] / 2.0 + spacing.rank / 2.0 + h / 2.0;
        } else {
            y = h / 2.0;
        }
        rank_y.push(y);
    }
    let at = |v: usize| [xs[v], rank_y[rank_of[v].max(0) as usize]];

    let nodes: Vec<[f64; 2]> = (0..n).map(at).collect();
    let edges = graph
        .edges
        .iter()
        .enumerate()
        .map(|(i, e)| {
            if e.from == e.to {
                let c = nodes[e.from];
                return Route {
                    points: vec![c, c],
                    label: None,
                };
            }
            let mut points: Vec<[f64; 2]> = chains[i].iter().map(|&v| at(v)).collect();
            if reversed[i] {
                points.reverse();
            }
            Route {
                points,
                label: label_node[i].map(at),
            }
        })
        .collect();
    Layout {
        nodes,
        edges,
        ranks,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn size(w: f64, h: f64) -> Size {
        Size {
            width: w,
            height: h,
        }
    }

    fn edge(from: usize, to: usize) -> Edge {
        Edge {
            from,
            to,
            minlen: 1,
            weight: 1.0,
            label: None,
        }
    }

    #[test]
    fn a_chain_runs_down() {
        let g = Graph {
            nodes: vec![size(40.0, 20.0); 3],
            edges: vec![edge(0, 1), edge(1, 2)],
        };
        let l = layout(&g, Spacing::default());
        assert_eq!(l.ranks, [0, 2, 4]);
        // Straight down, a rank apart (20 high + 50 between).
        assert_eq!(l.nodes[0][0], l.nodes[1][0]);
        assert!((l.nodes[1][1] - l.nodes[0][1] - 70.0).abs() < 1e-9);
        assert_eq!(l.edges[0].points.len(), 3); // through the label rank's dummy
    }

    #[test]
    fn cycles_still_point_the_right_way() {
        let g = Graph {
            nodes: vec![size(40.0, 20.0); 2],
            edges: vec![edge(0, 1), edge(1, 0)],
        };
        let l = layout(&g, Spacing::default());
        let back = &l.edges[1].points;
        // Edge 1 -> 0 starts at node 1 and ends at node 0.
        assert_eq!(back[0], l.nodes[1]);
        assert_eq!(*back.last().unwrap(), l.nodes[0]);
    }

    #[test]
    fn labels_take_space_between_ranks() {
        let g = Graph {
            nodes: vec![size(40.0, 20.0); 2],
            edges: vec![Edge {
                label: Some(size(60.0, 30.0)),
                ..edge(0, 1)
            }],
        };
        let l = layout(&g, Spacing::default());
        let gap = l.nodes[1][1] - l.nodes[0][1];
        // 10 + 25 + 30 + 25 + 10.
        assert!((gap - 100.0).abs() < 1e-9, "{gap}");
        let label = l.edges[0].label.unwrap();
        assert!((label[1] - (l.nodes[0][1] + 50.0)).abs() < 1e-9);
    }

    #[test]
    fn self_loops_do_not_break_ranking() {
        let g = Graph {
            nodes: vec![size(40.0, 20.0); 2],
            edges: vec![edge(0, 0), edge(0, 1)],
        };
        let l = layout(&g, Spacing::default());
        assert_eq!(l.ranks, [0, 2]);
        assert_eq!(l.edges[0].points, vec![l.nodes[0]; 2]);
    }
}
