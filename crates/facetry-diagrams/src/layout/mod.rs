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
//!
//! **Groups** (Mermaid's subgraphs) are boxes round their nodes, nested to any depth. A long
//! edge's dummies belong to the innermost group holding both its ends, so an edge between
//! outsiders never runs through a group. Ordering keeps every group's members together
//! ([`order::Nest`]). Each group has a border node on either side in every rank it spans,
//! chained down the ranks; Brandes-Köpf keeps such chains straight, so the borders are the
//! sides of a rectangle, and the separations next to them give the group its inner room and
//! the space round it. Room before the first rank and after the last (a title, padding) is
//! added between the ranks.

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

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Graph {
    pub nodes: Vec<Size>,
    pub edges: Vec<Edge>,
    /// Boxes round nodes; may be empty.
    pub groups: Vec<Group>,
    /// Every node's innermost group, or empty for none at all.
    pub node_group: Vec<Option<usize>>,
}

/// A group of nodes, drawn as a box round them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Group {
    pub parent: Option<usize>,
    /// Room inside the box: before its first rank, after its last, and along the ranks on the
    /// left and the right (in the top-to-bottom frame).
    pub before: f64,
    pub after: f64,
    pub left: f64,
    pub right: f64,
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
    /// Round a group's box, to whatever is next to it.
    pub group: f64,
}

impl Default for Spacing {
    /// Mermaid's defaults for nodes and ranks.
    fn default() -> Self {
        Self {
            node: 50.0,
            edge: 20.0,
            rank: 50.0,
            group: 20.0,
        }
    }
}

/// An edge's path through the layout.
#[derive(Debug, Clone, PartialEq)]
pub struct Route {
    /// From the centre of `from` through each dummy to the centre of `to`; a self-loop has
    /// just the node's centre twice (the caller draws the loop).
    pub points: Vec<[f64; 2]>,
    /// The rank of every point (the same order as `points`).
    pub ranks: Vec<i32>,
    /// The middle of the label, if the edge has one.
    pub label: Option<[f64; 2]>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    pub nodes: Vec<[f64; 2]>,
    pub edges: Vec<Route>,
    /// The rank of every node (after doubling: real nodes on adjacent ranks are 2 apart).
    pub ranks: Vec<i32>,
    /// The y extent of every rank, `[top, bottom]`: its tallest node (or label). A rank of
    /// nothing but edges is a line (`top == bottom`).
    pub rank_bands: Vec<[f64; 2]>,
    /// Every group's box, `[left, top, right, bottom]`; all zero for a group with no nodes.
    pub groups: Vec<[f64; 4]>,
}

/// The innermost group holding both `a` and `b`.
fn common_group(groups: &[Group], a: Option<usize>, b: Option<usize>) -> Option<usize> {
    let chain = |g: Option<usize>| {
        let mut out = Vec::new();
        let mut at = g;
        while let Some(i) = at {
            out.push(i);
            at = groups[i].parent;
        }
        out
    };
    let outer = chain(b);
    chain(a).into_iter().find(|g| outer.contains(g))
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
    let mut member: Vec<Option<usize>> = (0..n)
        .map(|v| graph.node_group.get(v).copied().flatten())
        .collect();
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
            member.push(common_group(&graph.groups, member[u], member[v]));
            chain.push(d);
        }
        chain.push(v);
        for w in chain.windows(2) {
            down[w[0]].push((w[1], e.weight));
            up[w[1]].push((w[0], e.weight));
        }
        chains.push(chain);
    }

    // --- group borders -------------------------------------------------------------------------
    // Every group's ranks: those of everything in it, nested groups included.
    let groups = &graph.groups;
    let mut span: Vec<Option<[i32; 2]>> = vec![None; groups.len()];
    for (v, &m) in member.iter().enumerate() {
        let mut at = m;
        while let Some(g) = at {
            let r = rank_of[v];
            span[g] = Some(span[g].map_or([r, r], |[a, b]| [a.min(r), b.max(r)]));
            at = groups[g].parent;
        }
    }
    let mut border: Vec<Option<(usize, order::Side)>> = vec![None; width.len()];
    for (g, s) in span.iter().enumerate() {
        let Some([first, last]) = *s else { continue };
        for side in [order::Side::Left, order::Side::Right] {
            let mut prev: Option<usize> = None;
            for r in first..=last {
                let b = width.len();
                width.push(0.0);
                height.push(0.0);
                rank_of.push(r);
                dummy.push(true);
                down.push(Vec::new());
                up.push(Vec::new());
                member.push(Some(g));
                border.push(Some((g, side)));
                if let Some(p) = prev {
                    down[p].push((b, 1.0));
                    up[b].push((p, 1.0));
                }
                prev = Some(b);
            }
        }
    }

    // --- order --------------------------------------------------------------------------------
    let max_rank = rank_of.iter().copied().max().unwrap_or(0).max(0) as usize;
    let mut layers: Vec<Vec<usize>> = vec![Vec::new(); max_rank + 1];
    for (v, &r) in rank_of.iter().enumerate() {
        layers[r.max(0) as usize].push(v);
    }
    let nest = (!groups.is_empty()).then(|| order::Nest {
        member: member.clone(),
        border: border.clone(),
        parent: groups.iter().map(|g| g.parent).collect(),
    });
    let mut ordering = order::Layers {
        layers,
        down,
        up,
        nest,
    };
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
        border: &border,
        room: &groups.iter().map(|g| [g.left, g.right]).collect::<Vec<_>>(),
        member: &member,
        parent: &groups.iter().map(|g| g.parent).collect::<Vec<_>>(),
        group_sep: spacing.group,
    });
    let rank_height: Vec<f64> = ordering
        .layers
        .iter()
        .map(|l| l.iter().map(|&v| height[v]).fold(0.0, f64::max))
        .collect();
    // Room for the groups before their first rank and after their last: a group's own, plus
    // that of the groups inside it that start (or end) on the same rank.
    let mut lead = vec![0.0f64; groups.len()];
    let mut trail = vec![0.0f64; groups.len()];
    // Inner groups first: a group's index may come before or after its parent's, so repeat
    // over the depth.
    for _ in 0..groups.len().max(1) {
        for g in 0..groups.len() {
            let Some([first, last]) = span[g] else {
                continue;
            };
            let (mut inner_lead, mut inner_trail) = (0.0f64, 0.0f64);
            for c in 0..groups.len() {
                if groups[c].parent == Some(g) {
                    if let Some([cf, cl]) = span[c] {
                        if cf == first {
                            inner_lead = inner_lead.max(lead[c]);
                        }
                        if cl == last {
                            inner_trail = inner_trail.max(trail[c]);
                        }
                    }
                }
            }
            lead[g] = groups[g].before + inner_lead;
            trail[g] = groups[g].after + inner_trail;
        }
    }
    let ranks_count = rank_height.len();
    let mut above = vec![0.0f64; ranks_count];
    let mut below = vec![0.0f64; ranks_count];
    for g in 0..groups.len() {
        if let Some([first, last]) = span[g] {
            above[first as usize] = above[first as usize].max(lead[g]);
            below[last as usize] = below[last as usize].max(trail[g]);
        }
    }
    let mut rank_y = Vec::with_capacity(ranks_count);
    let mut y = 0.0;
    for (r, &h) in rank_height.iter().enumerate() {
        if r > 0 {
            y += rank_height[r - 1] / 2.0 + below[r - 1] + spacing.rank / 2.0 + above[r] + h / 2.0;
        } else {
            y = above[0] + h / 2.0;
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
                    ranks: vec![ranks[e.from]; 2],
                    label: None,
                };
            }
            let mut points: Vec<[f64; 2]> = chains[i].iter().map(|&v| at(v)).collect();
            let mut point_ranks: Vec<i32> = chains[i].iter().map(|&v| rank_of[v]).collect();
            if reversed[i] {
                points.reverse();
                point_ranks.reverse();
            }
            Route {
                points,
                ranks: point_ranks,
                label: label_node[i].map(at),
            }
        })
        .collect();
    let rank_bands: Vec<[f64; 2]> = rank_y
        .iter()
        .zip(&rank_height)
        .map(|(&y, &h)| [y - h / 2.0, y + h / 2.0])
        .collect();
    // Boxes: the border chains for the sides, the room past the first and last ranks.
    let mut boxes = vec![[0.0; 4]; groups.len()];
    for (v, b) in border.iter().enumerate() {
        let Some((g, side)) = *b else { continue };
        let Some([first, last]) = span[g] else {
            continue;
        };
        let x = xs[v];
        let top = rank_bands[first as usize][0] - lead[g];
        let bottom = rank_bands[last as usize][1] + trail[g];
        let edge = match side {
            order::Side::Left => 0,
            order::Side::Right => 2,
        };
        boxes[g][edge] = x;
        boxes[g][1] = top;
        boxes[g][3] = bottom;
    }
    Layout {
        nodes,
        edges,
        ranks,
        rank_bands,
        groups: boxes,
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
            ..Graph::default()
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
            ..Graph::default()
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
            ..Graph::default()
        };
        let l = layout(&g, Spacing::default());
        let gap = l.nodes[1][1] - l.nodes[0][1];
        // 10 + 25 + 30 + 25 + 10.
        assert!((gap - 100.0).abs() < 1e-9, "{gap}");
        let label = l.edges[0].label.unwrap();
        assert!((label[1] - (l.nodes[0][1] + 50.0)).abs() < 1e-9);
    }

    fn group(parent: Option<usize>) -> Group {
        Group {
            parent,
            before: 30.0,
            after: 10.0,
            left: 12.0,
            right: 12.0,
        }
    }

    /// Whether box `a` holds box `b` (or a node box) with at least `room` on every side.
    fn holds(a: [f64; 4], b: [f64; 4], room: [f64; 4]) -> bool {
        b[0] - a[0] >= room[0] - 1e-6
            && b[1] - a[1] >= room[1] - 1e-6
            && a[2] - b[2] >= room[2] - 1e-6
            && a[3] - b[3] >= room[3] - 1e-6
    }

    fn overlap(a: [f64; 4], b: [f64; 4]) -> bool {
        a[0] < b[2] - 1e-6 && b[0] < a[2] - 1e-6 && a[1] < b[3] - 1e-6 && b[1] < a[3] - 1e-6
    }

    #[test]
    fn groups_are_boxes_round_their_nodes_and_keep_outsiders_out() {
        // 0 -> 1 -> 2 -> 3 and 0 -> 4 -> 5 -> 3; group 0 holds 1 and 2 with group 1 (holding
        // 2) inside it; group 2 holds 4 and 5. Node 6 hangs off 0 outside every group.
        let mut edges = vec![
            edge(0, 1),
            edge(1, 2),
            edge(2, 3),
            edge(0, 4),
            edge(4, 5),
            edge(5, 3),
            edge(0, 6),
        ];
        edges.push(edge(6, 3));
        let g = Graph {
            nodes: vec![size(40.0, 20.0); 7],
            edges,
            groups: vec![group(None), group(Some(0)), group(None)],
            node_group: vec![None, Some(0), Some(1), None, Some(2), Some(2), None],
        };
        let l = layout(&g, Spacing::default());
        let node_box = |v: usize| {
            let [x, y] = l.nodes[v];
            [x - 20.0, y - 10.0, x + 20.0, y + 10.0]
        };
        let room = [12.0, 30.0, 12.0, 10.0];
        assert!(holds(l.groups[0], node_box(1), room), "{:?}", l.groups);
        assert!(holds(l.groups[1], node_box(2), room), "{:?}", l.groups);
        // The inner group sits inside the outer with the outer's room round it.
        assert!(
            holds(l.groups[0], l.groups[1], [12.0, 0.0, 12.0, 0.0]),
            "{:?}",
            l.groups
        );
        assert!(holds(l.groups[2], node_box(4), room) && holds(l.groups[2], node_box(5), room));
        // Siblings apart, outsiders out.
        assert!(!overlap(l.groups[0], l.groups[2]), "{:?}", l.groups);
        for v in [0, 3, 6] {
            for b in &l.groups {
                assert!(!overlap(*b, node_box(v)), "node {v} in {b:?}");
            }
        }
    }

    #[test]
    fn a_long_edge_between_outsiders_goes_round_a_group() {
        // 0 -> 3 spans the ranks of group 0 (1 -> 2): its dummies are not in the group, so the
        // edge passes beside the box.
        let g = Graph {
            nodes: vec![size(40.0, 20.0); 4],
            edges: vec![edge(0, 1), edge(1, 2), edge(2, 3), edge(0, 3)],
            groups: vec![group(None)],
            node_group: vec![None, Some(0), Some(0), None],
        };
        let l = layout(&g, Spacing::default());
        let b = l.groups[0];
        for p in &l.edges[3].points[1..l.edges[3].points.len() - 1] {
            assert!(p[0] <= b[0] + 1e-6 || p[0] >= b[2] - 1e-6, "{p:?} in {b:?}");
        }
    }

    #[test]
    fn self_loops_do_not_break_ranking() {
        let g = Graph {
            nodes: vec![size(40.0, 20.0); 2],
            edges: vec![edge(0, 0), edge(0, 1)],
            ..Graph::default()
        };
        let l = layout(&g, Spacing::default());
        assert_eq!(l.ranks, [0, 2]);
        assert_eq!(l.edges[0].points, vec![l.nodes[0]; 2]);
    }
}
