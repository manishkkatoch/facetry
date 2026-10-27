//! Square-cornered edge routing on a layered layout (the glossy style's). Works in the
//! layout's own top-to-bottom frame; the caller rotates the result, fits its ends to the node
//! outlines and rounds its corners.
//!
//! Every edge leaves the bottom of its source and enters the top of its target, at a **port**:
//! a node's outgoing edges are spread along its bottom side in the order of where they go
//! (and incoming ones along its top), so they do not pile onto one point. Between ports the
//! edge runs straight down the columns the layout gave its dummy nodes; where two consecutive
//! columns differ it turns, runs across in the gap between the ranks, and turns down again.
//! Edges crossing the same gap at overlapping spans get their own **tracks** (heights in the
//! gap), so they never run along each other. The run into a target is kept long enough for
//! an arrowhead.

use crate::layout::{Layout, Size};

/// Spacing of the ports along a side, at most.
const PORT_STEP: f64 = 16.0;
/// Ports use at most this share of a side.
const PORT_SPREAD: f64 = 0.6;
/// Straight run kept before a target (arrowhead and gap), where the gap allows.
const LEAD_IN: f64 = 34.0;
/// Space between tracks in a gap, and kept from the gap's ends.
const TRACK: f64 = 8.0;
const GAP_MARGIN: f64 = 4.0;
/// How far a self-loop reaches out of its node's side.
const LOOP_REACH: f64 = 26.0;

/// A routed edge: its polyline from the source's port to the target's, and for each end a
/// point inside the node on the line of the first (last) segment, from which the caller can
/// clip the end to the node's real outline.
#[derive(Debug, Clone, PartialEq)]
pub struct Route {
    pub points: Vec<[f64; 2]>,
    pub start_inside: [f64; 2],
    pub end_inside: [f64; 2],
}

/// A turn across a gap: which edge, the ranks above and below the gap, where it comes down
/// from and where it goes on down, and (once assigned) its track.
#[derive(Debug, Clone)]
struct Jog {
    edge: usize,
    upper: i32,
    lower: i32,
    entry: f64,
    exit: f64,
    track: usize,
}

impl Jog {
    fn span(&self) -> (f64, f64) {
        (self.entry.min(self.exit), self.entry.max(self.exit))
    }
}

/// An edge's path through the layout, oriented downward (a reversed edge turned back).
#[derive(Debug)]
struct Oriented {
    src: usize,
    tgt: usize,
    pts: Vec<[f64; 2]>,
    ranks: Vec<i32>,
    reversed: bool,
}

/// Routes every edge of `laid` (from `edges`, `(from, to)` node indices) between nodes of
/// `sizes` centred at `centres` (the layout's own nodes, or the part of each that edges should
/// attach to, such as a pedestal beside its card), top to bottom.
pub fn route(
    laid: &Layout,
    centres: &[[f64; 2]],
    sizes: &[Size],
    edges: &[(usize, usize)],
) -> Vec<Route> {
    let n = laid.nodes.len();
    let real_rank = |r: i32| laid.ranks.contains(&r);
    // Each edge's points through the layout, oriented downward, source first.
    let mut oriented: Vec<Option<Oriented>> = Vec::new();
    for (i, &(from, to)) in edges.iter().enumerate() {
        let r = &laid.edges[i];
        if from == to || r.points.len() < 2 {
            oriented.push(None);
            continue;
        }
        let mut pts = r.points.clone();
        let mut ranks = r.ranks.clone();
        let reversed = ranks[0] > ranks[ranks.len() - 1];
        let (mut src, mut tgt) = (from, to);
        if reversed {
            pts.reverse();
            ranks.reverse();
            std::mem::swap(&mut src, &mut tgt);
        }
        oriented.push(Some(Oriented {
            src,
            tgt,
            pts,
            ranks,
            reversed,
        }));
    }

    // Ports: per node, the edges leaving its bottom and entering its top, sorted by where
    // their other end goes.
    let mut out_ports: Vec<Vec<(f64, usize)>> = vec![Vec::new(); n];
    let mut in_ports: Vec<Vec<(f64, usize)>> = vec![Vec::new(); n];
    for (i, o) in oriented.iter().enumerate() {
        if let Some(Oriented { src, tgt, pts, .. }) = o {
            out_ports[*src].push((pts[1][0], i));
            in_ports[*tgt].push((pts[pts.len() - 2][0], i));
        }
    }
    let spread = |list: &mut Vec<(f64, usize)>, node: usize| -> Vec<(usize, f64)> {
        list.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        let m = list.len();
        let cx = centres[node][0];
        let step = if m > 1 {
            PORT_STEP.min(PORT_SPREAD * sizes[node].width / (m - 1) as f64)
        } else {
            0.0
        };
        list.iter()
            .enumerate()
            .map(|(k, &(_, e))| (e, cx + (k as f64 - (m as f64 - 1.0) / 2.0) * step))
            .collect()
    };
    let mut start_x = vec![0.0; edges.len()];
    let mut end_x = vec![0.0; edges.len()];
    for v in 0..n {
        for (e, x) in spread(&mut out_ports[v], v) {
            start_x[e] = x;
        }
        for (e, x) in spread(&mut in_ports[v], v) {
            end_x[e] = x;
        }
    }

    // The columns each edge runs down (its stations), and the jogs between them. A dummy
    // node is a station only where it matters: where its edge's label is, and in a rank that
    // has real nodes (its column runs between them). Elsewhere the edge is free to turn
    // anywhere between its stations, which saves a needless S-bend.
    let mut columns: Vec<Vec<(f64, i32)>> = vec![Vec::new(); edges.len()];
    let mut jogs: Vec<Jog> = Vec::new();
    for (i, o) in oriented.iter().enumerate() {
        let Some(Oriented { pts, ranks, .. }) = o else {
            continue;
        };
        let label = laid.edges[i].label;
        let last = pts.len() - 1;
        let col: Vec<(f64, i32)> = (0..=last)
            .filter(|&j| {
                let is_label = label.is_some_and(|l| {
                    (l[0] - pts[j][0]).abs() < 1e-6 && (l[1] - pts[j][1]).abs() < 1e-6
                });
                j == 0 || j == last || is_label || real_rank(ranks[j])
            })
            .map(|j| {
                let x = match j {
                    0 => start_x[i],
                    j if j == last => end_x[i],
                    _ => pts[j][0],
                };
                (x, ranks[j])
            })
            .collect();
        for w in col.windows(2) {
            if (w[0].0 - w[1].0).abs() > 1e-6 {
                jogs.push(Jog {
                    edge: i,
                    upper: w[0].1,
                    lower: w[1].1,
                    entry: w[0].0,
                    exit: w[1].0,
                    track: 0,
                });
            }
        }
        columns[i] = col;
    }

    // Tracks: where jogs in the same gap overlap, each gets its own height. The order of the
    // heights decides whether they cross: of two jogs heading right, the one that comes down
    // further right must run above the other (its downstroke would cut the other's run
    // otherwise), and mirrored for jogs heading left. So right-heading jogs take tracks from
    // the top in order of entry from the right, then left-heading ones in order of entry from
    // the left; a jog takes the topmost track none of whose jogs it overlaps.
    let mut order: Vec<usize> = (0..jogs.len()).collect();
    order.sort_by(|&a, &b| {
        let key = |j: &Jog| {
            let rightward = j.exit > j.entry;
            let x = if rightward { -j.entry } else { j.entry };
            (j.upper, !rightward, x, j.edge)
        };
        let (ka, kb) = (key(&jogs[a]), key(&jogs[b]));
        (ka.0, ka.1)
            .cmp(&(kb.0, kb.1))
            .then(ka.2.total_cmp(&kb.2))
            .then(ka.3.cmp(&kb.3))
    });
    let mut tracks: std::collections::BTreeMap<i32, Vec<Vec<(f64, f64)>>> = Default::default();
    for &j in &order {
        let (lo, hi) = jogs[j].span();
        let gap = tracks.entry(jogs[j].upper).or_default();
        let free = gap
            .iter()
            .position(|spans| spans.iter().all(|&(a, b)| hi < a - TRACK || lo > b + TRACK));
        let track = free.unwrap_or(gap.len());
        if track == gap.len() {
            gap.push(Vec::new());
        }
        gap[track].push((lo, hi));
        jogs[j].track = track;
    }
    let jog_y = |jog: &Jog| {
        let top = laid.rank_bands[jog.upper as usize][1] + GAP_MARGIN;
        let bottom = laid.rank_bands[jog.lower as usize][0] - GAP_MARGIN;
        let mid = (top + bottom) / 2.0;
        // Into a node, leave it room for the arrowhead; elsewhere, the middle of the gap.
        let base = if real_rank(jog.lower) {
            (bottom - LEAD_IN).clamp(top.min(mid), mid)
        } else {
            mid
        };
        let count = tracks.get(&jog.upper).map_or(1, Vec::len) as f64;
        let y = base + (jog.track as f64 - (count - 1.0) / 2.0) * TRACK;
        y.clamp(top.min(bottom), bottom.max(top))
    };

    let mut routes = Vec::with_capacity(edges.len());
    for (i, &(from, _)) in edges.iter().enumerate() {
        let Some(Oriented {
            src, tgt, reversed, ..
        }) = &oriented[i]
        else {
            routes.push(self_loop(centres, sizes, from));
            continue;
        };
        let col = &columns[i];
        let (cs, ct) = (centres[*src], centres[*tgt]);
        let mut path = vec![[col[0].0, cs[1] + sizes[*src].height / 2.0]];
        let mut my_jogs = jogs.iter().filter(|j| j.edge == i);
        for w in col.windows(2) {
            if (w[0].0 - w[1].0).abs() > 1e-6 {
                let jog = my_jogs.next().expect("a jog per change of column");
                let y = jog_y(jog);
                path.push([w[0].0, y]);
                path.push([w[1].0, y]);
            }
        }
        let last = col[col.len() - 1].0;
        path.push([last, ct[1] - sizes[*tgt].height / 2.0]);
        let mut r = Route {
            points: path,
            start_inside: [col[0].0, cs[1]],
            end_inside: [last, ct[1]],
        };
        if *reversed {
            r.points.reverse();
            std::mem::swap(&mut r.start_inside, &mut r.end_inside);
        }
        routes.push(r);
    }
    routes
}

/// A loop out of the node's right side and back.
fn self_loop(centres: &[[f64; 2]], sizes: &[Size], from: usize) -> Route {
    let c = centres[from];
    let s = sizes[from];
    let x = c[0] + s.width / 2.0;
    let (y0, y1) = (c[1] - s.height / 4.0, c[1] + s.height / 4.0);
    Route {
        points: vec![[x, y0], [x + LOOP_REACH, y0], [x + LOOP_REACH, y1], [x, y1]],
        start_inside: [c[0], y0],
        end_inside: [c[0], y1],
    }
}

/// Segments per rounded corner.
const ARC_STEPS: usize = 6;

/// Rounds the corners of a polyline with arcs of up to `radius` (less where a segment is too
/// short), sampled as polylines.
pub fn round_corners(points: &[[f64; 2]], radius: f64) -> Vec<[f64; 2]> {
    if points.len() < 3 {
        return points.to_vec();
    }
    let mut out = vec![points[0]];
    for w in points.windows(3) {
        let (a, b, c) = (w[0], w[1], w[2]);
        let (l1, l2) = (
            (b[0] - a[0]).hypot(b[1] - a[1]),
            (c[0] - b[0]).hypot(c[1] - b[1]),
        );
        if l1 < 1e-9 || l2 < 1e-9 {
            continue;
        }
        let d1 = [(b[0] - a[0]) / l1, (b[1] - a[1]) / l1];
        let d2 = [(c[0] - b[0]) / l2, (c[1] - b[1]) / l2];
        if (d1[0] * d2[1] - d1[1] * d2[0]).abs() < 1e-6 {
            continue; // straight on
        }
        let r = radius.min(l1 / 2.0).min(l2 / 2.0);
        let p = [b[0] - d1[0] * r, b[1] - d1[1] * r];
        let q = [b[0] + d2[0] * r, b[1] + d2[1] * r];
        // A quadratic Bézier with its control point on the corner.
        for k in 0..=ARC_STEPS {
            let t = k as f64 / ARC_STEPS as f64;
            let u = 1.0 - t;
            out.push([
                u * u * p[0] + 2.0 * u * t * b[0] + t * t * q[0],
                u * u * p[1] + 2.0 * u * t * b[1] + t * t * q[1],
            ]);
        }
    }
    out.push(points[points.len() - 1]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::{Edge, Graph, Spacing, layout};

    fn size(w: f64, h: f64) -> Size {
        Size {
            width: w,
            height: h,
        }
    }

    fn graph(n: usize, edges: &[(usize, usize)]) -> Graph {
        Graph {
            nodes: vec![size(80.0, 40.0); n],
            edges: edges
                .iter()
                .map(|&(from, to)| Edge {
                    from,
                    to,
                    minlen: 1,
                    weight: 1.0,
                    label: None,
                })
                .collect(),
            ..Graph::default()
        }
    }

    fn is_orthogonal(points: &[[f64; 2]]) -> bool {
        points
            .windows(2)
            .all(|w| (w[0][0] - w[1][0]).abs() < 1e-6 || (w[0][1] - w[1][1]).abs() < 1e-6)
    }

    #[test]
    fn edges_are_square_cornered_from_bottom_ports_to_top_ports() {
        let edges = [(0, 1), (0, 2), (0, 3), (1, 4), (2, 4), (3, 4)];
        let g = graph(5, &edges);
        let laid = layout(&g, Spacing::default());
        let routes = route(&laid, &laid.nodes, &g.nodes, &edges);
        for (r, &(from, to)) in routes.iter().zip(&edges) {
            assert!(is_orthogonal(&r.points), "{r:?}");
            let first = r.points[0];
            let last = r.points[r.points.len() - 1];
            assert!((first[1] - (laid.nodes[from][1] + 20.0)).abs() < 1e-6);
            assert!((last[1] - (laid.nodes[to][1] - 20.0)).abs() < 1e-6);
            // Ends run vertically.
            assert!((r.points[0][0] - r.points[1][0]).abs() < 1e-6);
        }
        // The three edges out of node 0 leave from three different ports.
        let mut xs: Vec<f64> = routes[..3].iter().map(|r| r.points[0][0]).collect();
        xs.dedup_by(|a, b| (*a - *b).abs() < 1e-6);
        assert_eq!(xs.len(), 3);
    }

    #[test]
    fn back_edges_run_from_their_source() {
        let edges = [(0, 1), (1, 2), (2, 0)];
        let g = graph(3, &edges);
        let laid = layout(&g, Spacing::default());
        let r = &route(&laid, &laid.nodes, &g.nodes, &edges)[2];
        // Leaves node 2 (the lowest) through its top, arrives at node 0 through its bottom.
        assert!(r.points[0][1] < laid.nodes[2][1]);
        assert!(r.points[r.points.len() - 1][1] > laid.nodes[0][1]);
        assert!(is_orthogonal(&r.points));
    }

    #[test]
    fn rounded_corners_keep_the_ends_and_stay_near_the_corner() {
        let p = round_corners(
            &[[0.0, 0.0], [0.0, 50.0], [40.0, 50.0], [40.0, 100.0]],
            10.0,
        );
        assert_eq!(p[0], [0.0, 0.0]);
        assert_eq!(*p.last().unwrap(), [40.0, 100.0]);
        assert!(p.len() > 4);
        assert!(
            p.iter()
                .all(|q| q[0] >= 0.0 && q[0] <= 40.0 && q[1] >= 0.0 && q[1] <= 100.0)
        );
    }
}
