//! Morphing polylines: pairing the vertices of two versions of a line so they can be
//! interpolated, and cutting a line short to draw it on. Generic over the number of
//! coordinates, so the 2D and the 3D lines share it (`N` is 2 or 3).
//!
//! A line is a list of *runs* (a missing value ends a run and starts the next), each a list of
//! points. Two versions can only be interpolated vertex by vertex when they have the same runs
//! with the same number of points, so first one of two ways makes them so:
//!
//! * [`keyed_runs`]: when both lines know which data point each vertex is (their keys), match
//!   by point. This is what makes a marker stay on its line while the line changes.
//! * [`match_runs`]: otherwise pair the runs by position and add points on the longest
//!   segments of the shorter run until the counts agree ([`subdivide`]). The original vertices
//!   all stay, so the first and last frames are exactly the old and the new line.

use std::collections::HashMap;

use crate::anim::lerp;

/// The runs of a line: each a list of points.
pub(crate) type Runs<const N: usize> = Vec<Vec<[f32; N]>>;

fn distance<const N: usize>(a: [f32; N], b: [f32; N]) -> f32 {
    a.iter()
        .zip(&b)
        .map(|(a, b)| (b - a) * (b - a))
        .sum::<f32>()
        .sqrt()
}

fn midpoint<const N: usize>(a: [f32; N], b: [f32; N]) -> [f32; N] {
    std::array::from_fn(|i| (a[i] + b[i]) / 2.0)
}

/// Matches the vertices of two versions of a data line by the point each one stands for, so
/// that points which stay move along the line and the others come and go at their neighbours:
/// a vertex only in the new line starts on the nearest earlier vertex that stays (where it was),
/// one only in the old line ends on it (where it is now). Both lines then have the same list of
/// vertices, in the order the line runs, so the first and last frames are exactly the old and the
/// new line.
///
/// Only for a line of a single run on each side whose points that stay keep their order;
/// anything else returns `None` and the caller falls back to [`match_runs`]. The anchors
/// of the vertices that come and go are recorded for the markers (keyed by point).
pub(crate) fn keyed_runs<'a, const N: usize>(
    (a_runs, a_keys): (&[Vec<[f32; N]>], &'a [String]),
    (b_runs, b_keys): (&[Vec<[f32; N]>], &'a [String]),
    anchors: &mut HashMap<&'a str, [f32; N]>,
) -> Option<(Runs<N>, Runs<N>)> {
    let ([a], [b]) = (a_runs, b_runs) else {
        return None;
    };
    if a.len() != a_keys.len() || b.len() != b_keys.len() || a.is_empty() || b.is_empty() {
        return None;
    }
    let in_old: HashMap<&str, usize> = a_keys
        .iter()
        .enumerate()
        .map(|(i, k)| (k.as_str(), i))
        .collect();
    // The vertices that stay, as (index in old, index in new); they must keep their order.
    let common: Vec<(usize, usize)> = b_keys
        .iter()
        .enumerate()
        .filter_map(|(j, k)| in_old.get(k.as_str()).map(|&i| (i, j)))
        .collect();
    if common.is_empty() || common.windows(2).any(|w| w[0].0 >= w[1].0) {
        return None;
    }
    let (mut from, mut to) = (Vec::new(), Vec::new());
    let (mut i, mut j) = (0, 0);
    // The nearest vertex that stays before, as (its place in old, its place in new).
    let mut before: Option<([f32; N], [f32; N])> = None;
    let first = common[0];
    for &(ci, cj) in &common {
        // New vertices up to this one that stays: they open out of the vertex before (or, at
        // the start of the line, out of the first one that stays).
        let (old_at, _) = before.unwrap_or((a[first.0], b[first.1]));
        for k in j..cj {
            anchors.insert(&b_keys[k], old_at);
            from.push(old_at);
            to.push(b[k]);
        }
        // Old vertices up to this one: they close onto the same neighbour in the new line.
        let (_, new_at) = before.unwrap_or((a[first.0], b[first.1]));
        for k in i..ci {
            anchors.insert(&a_keys[k], new_at);
            from.push(a[k]);
            to.push(new_at);
        }
        from.push(a[ci]);
        to.push(b[cj]);
        before = Some((a[ci], b[cj]));
        (i, j) = (ci + 1, cj + 1);
    }
    // The tail after the last vertex that stays.
    let (old_at, new_at) = before?;
    for k in j..b.len() {
        anchors.insert(&b_keys[k], old_at);
        from.push(old_at);
        to.push(b[k]);
    }
    for k in i..a.len() {
        anchors.insert(&a_keys[k], new_at);
        from.push(a[k]);
        to.push(new_at);
    }
    Some((vec![from], vec![to]))
}

/// Pairs the runs of two versions of a polyline so that they can be interpolated point by
/// point: the n-th run with the n-th, both given the same number of points by subdividing the
/// longer segments (so every original vertex is kept and the first and last frames show exactly
/// the old and the new line). A run with no partner grows out of its own first point, or closes
/// onto its last.
pub(crate) fn match_runs<const N: usize>(
    a: &[Vec<[f32; N]>],
    b: &[Vec<[f32; N]>],
) -> (Runs<N>, Runs<N>) {
    let (mut out_a, mut out_b) = (Vec::new(), Vec::new());
    for i in 0..a.len().max(b.len()) {
        match (a.get(i), b.get(i)) {
            (Some(ra), Some(rb)) => {
                let n = ra.len().max(rb.len());
                out_a.push(subdivide(ra, n));
                out_b.push(subdivide(rb, n));
            }
            (None, Some(rb)) => {
                out_a.push(vec![rb[0]; rb.len()]);
                out_b.push(rb.clone());
            }
            (Some(ra), None) => {
                out_a.push(ra.clone());
                out_b.push(vec![ra[ra.len() - 1]; ra.len()]);
            }
            (None, None) => {}
        }
    }
    (out_a, out_b)
}

/// `run` with extra points on its longest segments until it has `n` points. The original
/// points all stay (so the shape is exactly the same); a run of one point is repeated.
pub fn subdivide<const N: usize>(run: &[[f32; N]], n: usize) -> Vec<[f32; N]> {
    let mut out = run.to_vec();
    if out.len() == 1 {
        out.resize(n, run[0]);
        return out;
    }
    while out.len() < n {
        let len = |i: usize| distance(out[i], out[i + 1]);
        let longest =
            (0..out.len() - 1).fold(0, |best, i| if len(i) > len(best) { i } else { best });
        let at = midpoint(out[longest], out[longest + 1]);
        out.insert(longest + 1, at);
    }
    out
}

/// The first `fraction` of a line's length (all its runs counted together, in order), cut off
/// part-way along a segment. `fraction >= 1` is the whole line, unchanged.
pub fn reveal_runs<const N: usize>(runs: &[Vec<[f32; N]>], fraction: f32) -> Vec<Vec<[f32; N]>> {
    if fraction >= 1.0 {
        return runs.to_vec();
    }
    let total: f32 = runs
        .iter()
        .map(|r| r.windows(2).map(|w| distance(w[0], w[1])).sum::<f32>())
        .sum();
    // A line with no length (a dot) is there or not, as a whole, from the first moment.
    let mut budget = if total > 0.0 {
        total * fraction.max(0.0)
    } else {
        0.0
    };
    let mut out = Vec::new();
    for run in runs {
        if budget <= 0.0 && total > 0.0 {
            break;
        }
        let mut shown = vec![run[0]];
        for w in run.windows(2) {
            let length = distance(w[0], w[1]);
            if length <= budget {
                budget -= length;
                shown.push(w[1]);
            } else {
                let f = budget / length;
                shown.push(std::array::from_fn(|i| lerp(w[0][i], w[1][i], f)));
                budget = 0.0;
                break;
            }
        }
        out.push(shown);
        if budget <= 0.0 {
            break;
        }
    }
    out
}

/// The runs of `from` and `to` (the same shape, see the module docs) mixed by `e`.
pub fn mix_runs<const N: usize>(
    from: &[Vec<[f32; N]>],
    to: &[Vec<[f32; N]>],
    e: f32,
) -> Vec<Vec<[f32; N]>> {
    from.iter()
        .zip(to)
        .map(|(a, b)| {
            a.iter()
                .zip(b)
                .map(|(a, b)| std::array::from_fn(|i| lerp(a[i], b[i], e)))
                .collect()
        })
        .collect()
}
