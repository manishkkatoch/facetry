//! Turning thick polylines (and dots) into triangles on the CPU. Pure geometry, no GPU:
//! the output is a flat list of vertices, three per triangle, in the same pixel space as the
//! input, so it is unit-tested natively and drawn by the mesh pipeline as is.
//!
//! **Why on the CPU.** Hardware lines are one pixel wide (and WebGL2 cannot do more), and
//! joining segments in a shader needs a neighbour's direction per vertex. A chart's lines
//! have at most a few thousand points, so building a triangle strip once per scene is cheap
//! and MSAA smooths the edges.
//!
//! **How.** Every segment is a quad `width` wide. At a joint the two quads meet at an angle:
//! on the inner side their edges cross, so both stop at that crossing (the *miter* point of
//! the inner side); on the outer side they leave a wedge, which is filled either by moving
//! both corners to the outer miter tip (a sharp point) or, when that tip would stick out
//! further than `miter_limit` line widths, by a single *bevel* triangle that just cuts the
//! corner. The quads therefore share their inner corner and never overlap, so a translucent
//! colour does not double up where a line bends. Ends are butt caps (flat); a run of one
//! point is drawn as a dot of the line's width so it does not vanish.

use glam::Vec2;

/// How far (in line widths, SVG's `stroke-miterlimit`) a miter tip may stick out of the
/// joint before the corner is bevelled instead. 4 turns joints sharper than about 29 degrees
/// into bevels, so a spiky zigzag does not grow spikes.
pub const MITER_LIMIT: f32 = 4.0;

/// Points closer together than this (pixels) are one point: a zero-length segment has no
/// direction to extrude along.
const MIN_SEGMENT: f32 = 1e-3;

/// Appends the triangles of `points` stroked `width` wide, with miter joins that fall back
/// to bevels above `miter_limit`. Non-finite points are skipped and repeated points merged.
pub fn polyline(points: &[[f32; 2]], width: f32, miter_limit: f32, out: &mut Vec<[f32; 2]>) {
    let half = width / 2.0;
    let mut p: Vec<Vec2> = Vec::with_capacity(points.len());
    for &q in points {
        let q = Vec2::from(q);
        if q.is_finite() && p.last().is_none_or(|l| l.distance(q) > MIN_SEGMENT) {
            p.push(q);
        }
    }
    match p.len() {
        0 => return,
        1 => {
            disc(p[0].to_array(), half, out);
            return;
        }
        _ => {}
    }

    let segments = p.len() - 1;
    // Left normal of a unit direction (the +90 degree turn).
    let normal = |d: Vec2| Vec2::new(-d.y, d.x);
    let dirs: Vec<Vec2> = (0..segments)
        .map(|i| (p[i + 1] - p[i]).normalize())
        .collect();
    // The four corners of every segment's quad: (left, right) at its start and at its end.
    // They begin as the plain offsets and the joints below move the ones they touch.
    let mut start: Vec<(Vec2, Vec2)> = (0..segments)
        .map(|i| {
            let o = normal(dirs[i]) * half;
            (p[i] + o, p[i] - o)
        })
        .collect();
    let mut end: Vec<(Vec2, Vec2)> = (0..segments)
        .map(|i| {
            let o = normal(dirs[i]) * half;
            (p[i + 1] + o, p[i + 1] - o)
        })
        .collect();
    let mut bevels: Vec<[Vec2; 3]> = Vec::new();

    for j in 1..segments {
        let (a, b) = (j - 1, j);
        let (d0, d1) = (dirs[a], dirs[b]);
        let cross = d0.perp_dot(d1);
        // Straight on: the plain quads already meet.
        if cross.abs() < 1e-6 && d0.dot(d1) > 0.0 {
            continue;
        }
        let (n0, n1) = (normal(d0), normal(d1));
        // Which side the path turns toward: the inner side of the bend. (`perp_dot` is the
        // z of the cross product: positive turns toward the left normal.)
        let inner_is_left = cross > 0.0;
        let side = if inner_is_left { 1.0 } else { -1.0 };
        let miter_sum = n0 + n1;
        // A full U-turn has no bisector: both quads keep their own corners and the outer
        // wedge is a bevel across the joint.
        // (The bisector's cosine is computed from nearly cancelling numbers there, so it
        // can come out zero or even slightly negative: treat that like a U-turn too.)
        let m = miter_sum.normalize_or_zero();
        let cos_half = m.dot(n0);
        let reversing = cos_half < 1e-4;
        let (inner, miter_ratio) = if reversing {
            (None, f32::INFINITY)
        } else {
            // The inner corners meet where the offset lines cross, a distance
            // `half / cos_half` along the bisector. Only if both segments are long enough
            // for that crossing to lie on them; for very short segments next to a sharp
            // bend it would stick out behind, and the quads overlap a little instead.
            let reach = half * (1.0 - cos_half * cos_half).max(0.0).sqrt() / cos_half;
            let room = (p[j] - p[a]).length().min((p[b + 1] - p[j]).length());
            let inner = (reach <= room).then(|| p[j] + m * side * (half / cos_half));
            (inner, 1.0 / cos_half)
        };

        // Outer corners: where the plain offsets of the two segments sit on the outer side.
        let outer = |n: Vec2| p[j] - n * side * half;
        if miter_ratio <= miter_limit {
            let tip = p[j] - m * side * (half * miter_ratio);
            set_corners(&mut end[a], &mut start[b], inner_is_left, inner, Some(tip));
        } else {
            set_corners(&mut end[a], &mut start[b], inner_is_left, inner, None);
            // Bevel: the triangle between the joint's inner corner (or the joint itself) and
            // the two outer corners.
            bevels.push([inner.unwrap_or(p[j]), outer(n0), outer(n1)]);
        }
    }

    for i in 0..segments {
        let ((ls, rs), (le, re)) = (start[i], end[i]);
        for v in [ls, rs, le, re, le, rs] {
            out.push(v.to_array());
        }
    }
    for tri in bevels {
        out.extend(tri.iter().map(|v| v.to_array()));
    }
}

/// Moves the corners at a joint: the inner side of both quads to `inner` (if the segments
/// are long enough), the outer side of both to `tip` (a miter; `None` keeps the plain
/// offsets and the caller adds a bevel).
fn set_corners(
    end_a: &mut (Vec2, Vec2),
    start_b: &mut (Vec2, Vec2),
    inner_is_left: bool,
    inner: Option<Vec2>,
    tip: Option<Vec2>,
) {
    if let Some(inner) = inner {
        if inner_is_left {
            end_a.0 = inner;
            start_b.0 = inner;
        } else {
            end_a.1 = inner;
            start_b.1 = inner;
        }
    }
    if let Some(tip) = tip {
        if inner_is_left {
            end_a.1 = tip;
            start_b.1 = tip;
        } else {
            end_a.0 = tip;
            start_b.0 = tip;
        }
    }
}

/// Appends a filled circle as a fan of triangles around `center`.
pub fn disc(center: [f32; 2], radius: f32, out: &mut Vec<[f32; 2]>) {
    if radius <= 0.0 || !radius.is_finite() {
        return;
    }
    let n = disc_segments(radius);
    let c = Vec2::from(center);
    let at = |k: usize| {
        let a = k as f32 / n as f32 * std::f32::consts::TAU;
        c + Vec2::new(a.cos(), a.sin()) * radius
    };
    for k in 0..n {
        out.extend([c.to_array(), at(k).to_array(), at(k + 1).to_array()]);
    }
}

/// Segments of a circle of `radius` pixels: enough that the polygon is within about a tenth
/// of a pixel of the circle.
fn disc_segments(radius: f32) -> usize {
    let r = radius.max(0.5);
    let step = 2.0 * (1.0 - 0.1 / r).clamp(-1.0, 1.0).acos();
    ((std::f32::consts::TAU / step.max(1e-3)).ceil() as usize).clamp(10, 96)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Sum of the triangles' areas (they never overlap in these tests, so this is the
    /// covered area).
    fn area(tris: &[[f32; 2]]) -> f32 {
        tris.chunks_exact(3)
            .map(|t| {
                let (a, b, c) = (Vec2::from(t[0]), Vec2::from(t[1]), Vec2::from(t[2]));
                ((b - a).perp_dot(c - a) / 2.0).abs()
            })
            .sum()
    }

    fn stroke(points: &[[f32; 2]], width: f32, limit: f32) -> Vec<[f32; 2]> {
        let mut out = Vec::new();
        polyline(points, width, limit, &mut out);
        out
    }

    fn bounds(tris: &[[f32; 2]]) -> ([f32; 2], [f32; 2]) {
        let mut lo = [f32::MAX; 2];
        let mut hi = [f32::MIN; 2];
        for v in tris {
            for k in 0..2 {
                lo[k] = lo[k].min(v[k]);
                hi[k] = hi[k].max(v[k]);
            }
        }
        (lo, hi)
    }

    #[test]
    fn a_two_point_line_is_one_rectangle() {
        let t = stroke(&[[0.0, 0.0], [10.0, 0.0]], 2.0, MITER_LIMIT);
        assert_eq!(t.len(), 6, "two triangles");
        assert!((area(&t) - 20.0).abs() < 1e-4);
        let (lo, hi) = bounds(&t);
        assert_eq!(
            (lo, hi),
            ([0.0, -1.0], [10.0, 1.0]),
            "butt caps: no overhang"
        );
    }

    #[test]
    fn collinear_points_join_without_a_seam_or_overlap() {
        let t = stroke(&[[0.0, 0.0], [5.0, 0.0], [10.0, 0.0]], 2.0, MITER_LIMIT);
        assert_eq!(t.len(), 12, "two quads, no join geometry");
        assert!((area(&t) - 20.0).abs() < 1e-4);
    }

    #[test]
    fn a_right_angle_gets_a_sharp_miter_without_overlap() {
        // Right then down (y grows downward on screen; the maths does not care).
        let t = stroke(&[[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]], 2.0, MITER_LIMIT);
        let (lo, hi) = bounds(&t);
        // The outer corner is the square tip (11, -1); nothing sticks out further.
        assert_eq!((lo, hi), ([0.0, -1.0], [11.0, 10.0]));
        assert!(
            t.iter()
                .any(|v| (v[0] - 11.0).abs() < 1e-4 && (v[1] + 1.0).abs() < 1e-4)
        );
        // [0, 11] x [-1, 1] joined with [9, 11] x [-1, 10]: 22 + 22 - the 2x2 corner they share.
        assert!((area(&t) - 40.0).abs() < 1e-3, "{}", area(&t));
        assert_eq!(t.len(), 12, "no bevel triangle needed");
    }

    #[test]
    fn a_turn_to_either_side_is_the_mirror_image() {
        let left = stroke(&[[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]], 2.0, MITER_LIMIT);
        let right = stroke(&[[0.0, 0.0], [10.0, 0.0], [10.0, -10.0]], 2.0, MITER_LIMIT);
        assert!((area(&left) - area(&right)).abs() < 1e-3);
        let (lo, hi) = bounds(&right);
        assert_eq!((lo, hi), ([0.0, -10.0], [11.0, 1.0]));
    }

    #[test]
    fn sharp_corners_fall_back_to_a_bevel_above_the_limit() {
        // A turn of about 164 degrees: the miter tip would be ~7 line widths long.
        let pts = [[0.0, 0.0], [10.0, 0.0], [0.0, 3.0]];
        let beveled = stroke(&pts, 2.0, MITER_LIMIT);
        // The corner is cut: nothing goes past the outer offsets, one half-width beyond x = 10.
        assert!(
            bounds(&beveled).1[0] <= 11.0 + 1e-3,
            "bevel sticks out: {:?}",
            bounds(&beveled)
        );
        assert_eq!(beveled.len(), 15, "two quads and one bevel triangle");
        // A generous limit lets the spike through.
        let spiked = stroke(&pts, 2.0, 100.0);
        assert!(bounds(&spiked).1[0] > 15.0, "{:?}", bounds(&spiked));
        assert_eq!(spiked.len(), 12);
    }

    #[test]
    fn the_limit_is_in_line_widths_like_svg() {
        // A 90 degree turn has a miter ratio of sqrt(2) = 1.414.
        let pts = [[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]];
        assert_eq!(stroke(&pts, 2.0, 1.5).len(), 12, "mitered");
        assert_eq!(stroke(&pts, 2.0, 1.4).len(), 15, "bevelled");
    }

    #[test]
    fn repeated_and_non_finite_points_are_ignored() {
        let clean = stroke(&[[0.0, 0.0], [10.0, 0.0]], 2.0, MITER_LIMIT);
        let messy = stroke(
            &[
                [0.0, 0.0],
                [0.0, 0.0],
                [f32::NAN, 1.0],
                [10.0, 0.0],
                [10.0, 0.0],
                [10.0, 0.0],
            ],
            2.0,
            MITER_LIMIT,
        );
        assert_eq!(clean, messy);
    }

    #[test]
    fn nothing_in_nothing_out_and_one_point_is_a_dot() {
        assert!(stroke(&[], 2.0, MITER_LIMIT).is_empty());
        assert!(stroke(&[[f32::NAN, 0.0]], 2.0, MITER_LIMIT).is_empty());
        let dot = stroke(&[[5.0, 5.0], [5.0, 5.0]], 6.0, MITER_LIMIT);
        assert!(!dot.is_empty());
        let (lo, hi) = bounds(&dot);
        // (A polygon with an odd number of sides does not touch the circle on every side.)
        assert!(
            (lo[0] - 2.0).abs() < 0.15 && (hi[0] - 8.0).abs() < 0.15,
            "{lo:?} {hi:?}"
        );
        assert!(
            (area(&dot) - std::f32::consts::PI * 9.0).abs() < 1.5,
            "{}",
            area(&dot)
        );
    }

    #[test]
    fn a_u_turn_stays_finite() {
        let t = stroke(&[[0.0, 0.0], [10.0, 0.0], [0.0, 0.0]], 2.0, MITER_LIMIT);
        assert!(t.iter().all(|v| v[0].is_finite() && v[1].is_finite()));
        let (lo, hi) = bounds(&t);
        assert!(lo[0] >= -1e-3 && hi[0] <= 10.0 + 1e-3, "no spike");
    }

    #[test]
    fn short_segments_next_to_a_sharp_bend_do_not_poke_out_behind_the_line() {
        // The inner miter point of a tight bend lies further back than the 0.5-px segment is
        // long; it must not be used (the quads would fold over themselves).
        let t = stroke(
            &[[0.0, 0.0], [10.0, 0.0], [10.0, 0.5], [0.0, 1.0]],
            8.0,
            MITER_LIMIT,
        );
        let (lo, hi) = bounds(&t);
        assert!(
            lo[0] > -9.0 && hi[0] < 20.0 && lo[1] > -9.0 && hi[1] < 9.0,
            "{lo:?} {hi:?}"
        );
    }

    #[test]
    fn discs_approximate_a_circle() {
        for radius in [1.0_f32, 4.0, 30.0] {
            let mut out = Vec::new();
            disc([3.0, 4.0], radius, &mut out);
            let want = std::f32::consts::PI * radius * radius;
            // A 10-gon is the fewest segments used (tiny radii); it is 6% short of a circle.
            assert!((area(&out) - want).abs() / want < 0.07, "radius {radius}");
            assert_eq!(out.len() % 3, 0);
        }
        let mut none = Vec::new();
        disc([0.0, 0.0], 0.0, &mut none);
        disc([0.0, 0.0], f32::NAN, &mut none);
        assert!(none.is_empty());
    }

    #[cfg(not(target_arch = "wasm32"))]
    mod properties {
        use proptest::prelude::*;

        use super::*;

        proptest! {
            /// Whatever the path, the output is whole, finite triangles that stay within the
            /// input's bounding box grown by the miter limit: a miter never sticks out past it.
            #[test]
            fn output_is_finite_and_bounded(
                pts in prop::collection::vec((-200.0f32..200.0, -200.0f32..200.0), 0..40),
                width in 0.5f32..12.0,
            ) {
                let pts: Vec<[f32; 2]> = pts.into_iter().map(|(x, y)| [x, y]).collect();
                let t = stroke(&pts, width, MITER_LIMIT);
                prop_assert_eq!(t.len() % 3, 0);
                let reach = MITER_LIMIT * width / 2.0 + 1e-2;
                if let Some((lo, hi)) = (!pts.is_empty()).then(|| bounds(&pts)) {
                    for v in &t {
                        prop_assert!(v[0].is_finite() && v[1].is_finite());
                        for k in 0..2 {
                            prop_assert!(v[k] >= lo[k] - reach && v[k] <= hi[k] + reach, "{v:?}");
                        }
                    }
                }
            }

            /// Reversing the path covers the same area.
            #[test]
            fn direction_does_not_matter(
                pts in prop::collection::vec((0.0f32..100.0, 0.0f32..100.0), 2..12),
            ) {
                let pts: Vec<[f32; 2]> = pts.into_iter().map(|(x, y)| [x, y]).collect();
                let mut back = pts.clone();
                back.reverse();
                let (a, b) = (area(&stroke(&pts, 3.0, MITER_LIMIT)), area(&stroke(&back, 3.0, MITER_LIMIT)));
                prop_assert!((a - b).abs() <= 1e-2 * a.max(1.0), "{a} vs {b}");
            }
        }
    }
}
