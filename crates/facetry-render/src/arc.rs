//! Pie and donut geometry: flat ring sectors for 2D, extruded slices for 3D. Pure functions
//! that turn the scene's [`ArcMark`]s and [`SliceMark`]s into triangles, unit-tested natively.
//!
//! A curved edge is a polygon: every slice is cut into segments of at most 2 degrees (at
//! least 3), so even a big pie's rim stays within a fraction of a pixel of a true circle.
//! Neighbouring slices share their boundary angles exactly (the same numbers feed both), and
//! the rasteriser fills shared edges without gaps, so there are no seams.

use bytemuck::{Pod, Zeroable};
use facetry_core::scene::ArcMark;
use facetry_core::scene3d::SliceMark;

/// Widest angular step of a slice's rim.
const MAX_STEP: f32 = 2.0 * std::f32::consts::PI / 180.0;

/// How many segments cover the angles `a0..a1`.
fn segments(a0: f32, a1: f32) -> usize {
    (((a1 - a0) / MAX_STEP).ceil() as usize).max(3)
}

/// The angle at step `k` of `n` between `a0` and `a1`.
fn lerp(a0: f32, a1: f32, k: usize, n: usize) -> f32 {
    a0 + (a1 - a0) * k as f32 / n as f32
}

/// Appends the triangles of a flat slice (a pie wedge, or a donut's ring sector).
pub fn sector(arc: &ArcMark, out: &mut Vec<[f32; 2]>) {
    let (a0, a1) = arc.span();
    if a1 <= a0 || arc.outer <= 0.0 {
        return;
    }
    let n = segments(a0, a1);
    let [cx, cy] = arc.center;
    // Angles run clockwise from 12 o'clock; screen y grows downward.
    let at = |a: f32, r: f32| [cx + r * a.sin(), cy - r * a.cos()];
    for k in 0..n {
        let (p, q) = (lerp(a0, a1, k, n), lerp(a0, a1, k + 1, n));
        if arc.inner <= 0.0 {
            out.extend([arc.center, at(p, arc.outer), at(q, arc.outer)]);
        } else {
            let (o0, o1) = (at(p, arc.outer), at(q, arc.outer));
            let (i0, i1) = (at(p, arc.inner), at(q, arc.inner));
            out.extend([o0, o1, i0, i0, o1, i1]);
        }
    }
}

/// A vertex of a lit 3D mesh: position, the surface's normal there, the colour, and the
/// hover flag (1 lightens it; rewritten alone when the pointer moves between slices).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
pub struct Vertex3 {
    pub pos: [f32; 3],
    pub normal: [f32; 3],
    pub color: [f32; 4],
    pub hover: f32,
}

/// Appends the triangles of an extruded slice: the top face, the outer wall, the inner wall
/// of a donut, and the two flat faces where it meets its neighbours. There is no bottom: the
/// camera never goes below the floor (elevation is at least 5 degrees).
///
/// Flat faces have flat normals. The curved walls use the true radial normal at every
/// vertex, so their shading is smooth instead of faceted by the 2-degree segments.
pub fn slice(s: &SliceMark, color: [f32; 4], out: &mut Vec<Vertex3>) {
    let (a0, a1) = s.span();
    if a1 <= a0 || s.outer <= 0.0 {
        return;
    }
    let n = segments(a0, a1);
    let v = |pos: [f32; 3], normal: [f32; 3]| Vertex3 {
        pos,
        normal,
        color,
        hover: 0.0,
    };
    let up = [0.0, 1.0, 0.0];
    let radial = |a: f32| [a.sin(), 0.0, -a.cos()];
    let neg = |n: [f32; 3]| [-n[0], -n[1], -n[2]];
    let top = s.height;

    for k in 0..n {
        let (p, q) = (lerp(a0, a1, k, n), lerp(a0, a1, k + 1, n));
        // Top face.
        let (o0, o1) = (s.point(p, s.outer, top), s.point(q, s.outer, top));
        if s.inner <= 0.0 {
            let c = s.point(p, 0.0, top);
            out.extend([v(c, up), v(o0, up), v(o1, up)]);
        } else {
            let (i0, i1) = (s.point(p, s.inner, top), s.point(q, s.inner, top));
            out.extend([
                v(o0, up),
                v(o1, up),
                v(i0, up),
                v(i0, up),
                v(o1, up),
                v(i1, up),
            ]);
        }
        // Outer wall: a quad from the floor to the top edge, normals pointing outward.
        let (b0, b1) = (s.point(p, s.outer, 0.0), s.point(q, s.outer, 0.0));
        let (np, nq) = (radial(p), radial(q));
        out.extend([
            v(b0, np),
            v(b1, nq),
            v(o0, np),
            v(o0, np),
            v(b1, nq),
            v(o1, nq),
        ]);
        // Inner wall: the hole's side, seen from inside the hole, normals pointing at the axis.
        if s.inner > 0.0 {
            let (b0, b1) = (s.point(p, s.inner, 0.0), s.point(q, s.inner, 0.0));
            let (t0, t1) = (s.point(p, s.inner, top), s.point(q, s.inner, top));
            let (np, nq) = (neg(np), neg(nq));
            out.extend([
                v(b0, np),
                v(b1, nq),
                v(t0, np),
                v(t0, np),
                v(b1, nq),
                v(t1, nq),
            ]);
        }
    }

    // The flat faces at the slice's two edges, unless it is the whole circle. The tangent
    // (direction of increasing angle) is (cos a, 0, sin a): the start face looks against it,
    // the end face along it.
    if s.end - s.start < std::f32::consts::TAU - 1e-4 {
        for (angle, normal) in [
            (a0, neg([a0.cos(), 0.0, a0.sin()])),
            (a1, [a1.cos(), 0.0, a1.sin()]),
        ] {
            let (lo, hi) = (s.point(angle, s.inner, 0.0), s.point(angle, s.outer, 0.0));
            let (lo_t, hi_t) = (s.point(angle, s.inner, top), s.point(angle, s.outer, top));
            out.extend([
                v(lo, normal),
                v(hi, normal),
                v(lo_t, normal),
                v(lo_t, normal),
                v(hi, normal),
                v(hi_t, normal),
            ]);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::f32::consts::{PI, TAU};

    use facetry_core::Color;

    use super::*;

    fn arc(inner: f32, start: f32, end: f32, pad: f32) -> ArcMark {
        ArcMark {
            key: "a".into(),
            color: Color::linear(1.0, 0.0, 0.0, 1.0),
            center: [100.0, 100.0],
            inner,
            outer: 50.0,
            start,
            end,
            pad,
            layer: 0,
            row: 0,
            sum: None,
        }
    }

    fn area(tris: &[[f32; 2]]) -> f32 {
        tris.chunks_exact(3)
            .map(|t| {
                let (ax, ay) = (t[1][0] - t[0][0], t[1][1] - t[0][1]);
                let (bx, by) = (t[2][0] - t[0][0], t[2][1] - t[0][1]);
                (ax * by - ay * bx).abs() / 2.0
            })
            .sum()
    }

    #[test]
    fn a_pie_wedge_has_the_area_of_its_sector() {
        let a = arc(0.0, 0.0, PI / 2.0, 0.0);
        let mut t = Vec::new();
        sector(&a, &mut t);
        let want = PI * 50.0 * 50.0 / 4.0;
        assert!(
            (area(&t) - want).abs() / want < 0.002,
            "{} vs {want}",
            area(&t)
        );
        assert_eq!(t.len() % 3, 0);
        // 90 degrees in 2-degree steps: 45 segments, one triangle each.
        assert_eq!(t.len(), 45 * 3);
    }

    #[test]
    fn a_donut_sector_has_the_ring_area_and_stays_in_the_ring() {
        let a = arc(20.0, 1.0, 2.5, 0.0);
        let mut t = Vec::new();
        sector(&a, &mut t);
        let want = 1.5 / 2.0 * (50.0 * 50.0 - 20.0 * 20.0);
        assert!((area(&t) - want).abs() / want < 0.002);
        for v in &t {
            let r = ((v[0] - 100.0).powi(2) + (v[1] - 100.0).powi(2)).sqrt();
            assert!((20.0 - 1e-3..=50.0 + 1e-3).contains(&r), "{r}");
        }
    }

    #[test]
    fn padding_shrinks_the_slice_and_leaves_a_gap() {
        let pad = 10.0_f32.to_radians();
        let (mut with, mut without) = (Vec::new(), Vec::new());
        sector(&arc(0.0, 0.0, PI, pad), &mut with);
        sector(&arc(0.0, 0.0, PI, 0.0), &mut without);
        let lost = 1.0 - area(&with) / area(&without);
        assert!((lost - pad / PI).abs() < 0.005, "{lost}");
    }

    #[test]
    fn tiny_and_full_slices_are_still_fine() {
        let mut t = Vec::new();
        sector(&arc(0.0, 0.0, 0.001, 0.0), &mut t);
        assert_eq!(t.len(), 9, "at least three segments");
        t.clear();
        sector(&arc(0.0, 0.0, TAU, 0.0), &mut t);
        let want = PI * 2500.0;
        assert!((area(&t) - want).abs() / want < 0.002);
        t.clear();
        sector(&arc(0.0, 1.0, 1.0, 0.0), &mut t);
        assert!(t.is_empty(), "no sweep, no triangles");
    }

    #[test]
    fn neighbouring_slices_meet_on_the_same_vertices() {
        // The shared edge's two endpoints appear, bit for bit, in both slices: nothing for
        // the rasteriser to leave a seam between.
        let (mut a, mut b) = (Vec::new(), Vec::new());
        sector(&arc(0.0, 0.0, 1.0, 0.0), &mut a);
        sector(&arc(0.0, 1.0, 2.0, 0.0), &mut b);
        let edge_end = a.last().copied().unwrap();
        assert!(b.contains(&edge_end), "{edge_end:?}");
    }

    fn slice3(inner: f32, start: f32, end: f32) -> SliceMark {
        SliceMark {
            key: "s".into(),
            color: facetry_core::Color::linear(0.0, 1.0, 0.0, 1.0),
            center: [1.0, 1.0],
            inner,
            outer: 0.5,
            height: 0.2,
            start,
            end,
            pad: 0.0,
            layer: 0,
            row: 0,
            sum: None,
        }
    }

    fn unit(n: [f32; 3]) -> bool {
        ((n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt() - 1.0).abs() < 1e-4
    }

    #[test]
    fn an_extruded_slice_fits_its_box_and_has_unit_normals() {
        for inner in [0.0, 0.25] {
            let mut out = Vec::new();
            slice(&slice3(inner, 0.5, 2.0), [1.0; 4], &mut out);
            assert_eq!(out.len() % 3, 0);
            for v in &out {
                assert!(unit(v.normal), "{v:?}");
                let r = ((v.pos[0] - 1.0).powi(2) + (v.pos[2] - 1.0).powi(2)).sqrt();
                assert!(r <= 0.5 + 1e-4 && r >= inner - 1e-4, "{r}");
                assert!((0.0..=0.2 + 1e-6).contains(&v.pos[1]));
            }
        }
    }

    #[test]
    fn top_faces_point_up_walls_point_away_from_the_slice() {
        let s = slice3(0.2, 0.5, 2.0);
        let mut out = Vec::new();
        slice(&s, [1.0; 4], &mut out);
        for tri in out.chunks_exact(3) {
            let (a, b, c) = (tri[0].pos, tri[1].pos, tri[2].pos);
            let centroid = [
                (a[0] + b[0] + c[0]) / 3.0,
                (a[1] + b[1] + c[1]) / 3.0,
                (a[2] + b[2] + c[2]) / 3.0,
            ];
            let n = tri[0].normal;
            if n == [0.0, 1.0, 0.0] {
                assert!(
                    tri.iter().all(|v| (v.pos[1] - 0.2).abs() < 1e-6),
                    "top face at the top"
                );
                continue;
            }
            // A wall's normal points away from the solid: nudging the centroid along it must
            // leave the slice (leave the ring sector or its angles); nudging against it stays.
            let eps = 1e-3;
            let out_pt = [centroid[0] + n[0] * eps, centroid[2] + n[2] * eps];
            let in_pt = [centroid[0] - n[0] * eps, centroid[2] - n[2] * eps];
            let (x, z) = (centroid[0], centroid[2]);
            assert!(
                !s.contains(out_pt[0], out_pt[1]) || (x, z) == (0.0, 0.0),
                "{n:?} at {centroid:?}"
            );
            assert!(s.contains(in_pt[0], in_pt[1]), "{n:?} at {centroid:?}");
        }
    }

    #[test]
    fn a_whole_circle_has_no_edge_faces() {
        let (mut whole, mut part) = (Vec::new(), Vec::new());
        slice(&slice3(0.0, 0.0, TAU), [1.0; 4], &mut whole);
        slice(&slice3(0.0, 0.0, TAU - 0.1), [1.0; 4], &mut part);
        // Top (1 tri per segment) + outer wall (2 per segment) = 3 triangles per segment.
        assert_eq!(whole.len(), 180 * 3 * 3);
        assert!(
            part.len() > whole.len() - 3 * 3 * 6,
            "edge faces add 4 triangles"
        );
        assert!(whole.iter().all(|v| v.pos.iter().all(|c| c.is_finite())));
    }

    #[test]
    fn vertex_layout_matches_the_shader() {
        assert_eq!(std::mem::size_of::<Vertex3>(), 44);
        assert_eq!(std::mem::offset_of!(Vertex3, hover), 40);
    }
}
