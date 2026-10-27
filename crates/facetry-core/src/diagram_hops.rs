//! Edges that cross in the plan: where two edges cross, the later one arcs over the earlier,
//! so in 3D they never run through each other.

use crate::diagram::Plan;
use crate::spec::DiagramEdge;

/// Where one edge arcs over others, and how wide its arcs are.
#[derive(Debug, Clone, Default)]
pub(crate) struct Hops {
    /// The crossings (layout units).
    at: Vec<[f64; 2]>,
    /// Half an arc's span, layout units.
    radius: f64,
}

/// For every edge, where it crosses an edge before it and arcs over it, in arcs `radius` long
/// each way: crossings at least `clear` beyond an arc's length from both its ends (layout
/// units), so an arc never runs into a node or what is drawn at an end.
pub(crate) fn hops(edges: &[DiagramEdge], radius: f64, clear: f64) -> Vec<Hops> {
    let paths: Vec<&[[f64; 2]]> = edges.iter().map(|e| e.points.as_slice()).collect();
    hops_of(&paths, radius, clear)
}

fn hops_of(paths: &[&[[f64; 2]]], radius: f64, clear: f64) -> Vec<Hops> {
    paths
        .iter()
        .enumerate()
        .map(|(i, path)| {
            let total = crate::diagram::path_length(path);
            let mut at: Vec<[f64; 2]> = Vec::new();
            let mut along = 0.0;
            for a in path.windows(2) {
                let len = (a[1][0] - a[0][0]).hypot(a[1][1] - a[0][1]);
                for other in &paths[..i] {
                    for b in other.windows(2) {
                        let Some(t) = cross(a[0], a[1], b[0], b[1]) else {
                            continue;
                        };
                        let s = along + t * len;
                        let p = [
                            a[0][0] + (a[1][0] - a[0][0]) * t,
                            a[0][1] + (a[1][1] - a[0][1]) * t,
                        ];
                        let seen = at.iter().any(|q| (q[0] - p[0]).hypot(q[1] - p[1]) < 1e-6);
                        let off = radius + clear;
                        if s >= off && s <= total - off && !seen {
                            at.push(p);
                        }
                    }
                }
                along += len;
            }
            Hops { at, radius }
        })
        .collect()
}

/// Where segment `a0`-`a1` crosses `b0`-`b1`, as a share of the way along the first; `None`
/// where they do not cross, or are parallel. Each segment includes its start and not its end,
/// so a crossing at a corner is found once.
fn cross(a0: [f64; 2], a1: [f64; 2], b0: [f64; 2], b1: [f64; 2]) -> Option<f64> {
    let (rx, ry) = (a1[0] - a0[0], a1[1] - a0[1]);
    let (sx, sy) = (b1[0] - b0[0], b1[1] - b0[1]);
    let d = rx * sy - ry * sx;
    if d.abs() < 1e-9 * (rx.hypot(ry) * sx.hypot(sy)).max(1e-12) {
        return None;
    }
    let (qx, qy) = (b0[0] - a0[0], b0[1] - a0[1]);
    let t = (qx * sy - qy * sx) / d;
    let u = (qx * ry - qy * rx) / d;
    ((0.0..1.0).contains(&t) && (0.0..1.0).contains(&u)).then_some(t)
}

impl Hops {
    #[cfg(test)]
    fn is_empty(&self) -> bool {
        self.at.is_empty()
    }

    /// How far up an arc `p` (layout units, on the edge) is, from 0 off every arc to 1 over
    /// the crossing: a smooth (cosine) rise and fall.
    pub(crate) fn rise(&self, p: [f64; 2]) -> f64 {
        self.at
            .iter()
            .map(|c| {
                let d = (p[0] - c[0]).hypot(p[1] - c[1]) / self.radius;
                if d >= 1.0 {
                    0.0
                } else {
                    (1.0 + (std::f64::consts::PI * d).cos()) / 2.0
                }
            })
            .fold(0.0, f64::max)
    }

    /// `path` with points added along the stretches that arc, so the arcs are smooth.
    pub(crate) fn densify(&self, path: &[[f64; 2]]) -> Vec<[f64; 2]> {
        if self.at.is_empty() || path.is_empty() {
            return path.to_vec();
        }
        let step = self.radius / 8.0;
        let mut out = vec![path[0]];
        for w in path.windows(2) {
            let (a, b) = (w[0], w[1]);
            let near = self
                .at
                .iter()
                .any(|&c| distance_to_segment(c, a, b) < self.radius);
            let len = (b[0] - a[0]).hypot(b[1] - a[1]);
            if near && len > step {
                // Steps all along the segment; those off the arcs (but next to them) are left
                // out, the segment is straight there.
                let n = (len / step).ceil() as usize;
                let at = |k: usize| {
                    let t = k as f64 / n as f64;
                    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
                };
                for k in 1..n {
                    if (k - 1..=k + 1).any(|j| self.rise(at(j)) > 0.0) {
                        out.push(at(k));
                    }
                }
            }
            out.push(b);
        }
        out
    }

    /// How far each point of `run` is raised (world units) by arcs `height` layout units high
    /// at their crests; empty where none of it arcs.
    pub(crate) fn lift(&self, plan: &Plan, run: &[[f64; 2]], height: f64) -> Vec<f32> {
        let lift: Vec<f32> = run
            .iter()
            .map(|&p| plan.len(height * self.rise(p)) as f32)
            .collect();
        if lift.iter().all(|&l| l == 0.0) {
            Vec::new()
        } else {
            lift
        }
    }
}

fn distance_to_segment(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let len2 = dx * dx + dy * dy;
    let t = if len2 > 0.0 {
        (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (p[0] - a[0] - dx * t).hypot(p[1] - a[1] - dy * t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_later_of_two_crossing_edges_arcs_over_the_earlier() {
        let (a, b) = ([[0.0, 50.0], [100.0, 50.0]], [[50.0, 0.0], [50.0, 100.0]]);
        let hops = hops_of(&[&a, &b], 10.0, 0.0);
        assert!(hops[0].is_empty());
        assert_eq!(hops[1].at, vec![[50.0, 50.0]]);
        let h = &hops[1];
        assert_eq!(h.rise([50.0, 50.0]), 1.0);
        assert!((h.rise([50.0, 45.0]) - 0.5).abs() < 1e-9);
        assert_eq!(h.rise([50.0, 39.0]), 0.0);
        // Points every 1.25 units over the arc (40 to 60, both kept), the ends kept.
        let path = h.densify(&b);
        assert_eq!(path.first(), Some(&[50.0, 0.0]));
        assert_eq!(path.last(), Some(&[50.0, 100.0]));
        assert_eq!(path.len(), 2 + 17);
        assert!((path[1][1] - 40.0).abs() < 1e-9 && (path[17][1] - 60.0).abs() < 1e-9);
    }

    #[test]
    fn crossings_near_an_edges_ends_and_touching_ends_are_not_arcs() {
        let edges: [&[[f64; 2]]; 3] = [
            &[[0.0, 5.0], [100.0, 5.0]],
            // Crosses 5 units from its start.
            &[[50.0, 0.0], [50.0, 100.0]],
            // Shares an end with the first.
            &[[100.0, 5.0], [100.0, 100.0]],
        ];
        let hops = hops_of(&edges, 10.0, 0.0);
        assert!(hops.iter().all(Hops::is_empty));
    }
}
