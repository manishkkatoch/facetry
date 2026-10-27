//! Pie and donut geometry: flat ring sectors for 2D, extruded slices for 3D. Pure functions
//! that turn the scene's [`ArcMark`]s and [`SliceMark`]s into triangles, unit-tested natively.
//!
//! A curved edge is a polygon: every slice is cut into segments of at most 2 degrees (at
//! least 3), so even a big pie's rim stays within a fraction of a pixel of a true circle.
//! Neighbouring slices share their boundary angles exactly (the same numbers feed both), and
//! the rasteriser fills shared edges without gaps, so there are no seams.

use bytemuck::{Pod, Zeroable};
use facetry_core::scene::ArcMark;
use facetry_core::scene3d::{FillMark, PrismMark, RibbonMark, ShadowMark, SliceMark, Stand};

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
    /// Specular strength (0: matte, as the bars and slices are).
    pub gloss: f32,
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
        gloss: 0.0,
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

/// Appends the triangles of a fill (ink printed on a surface): as given, with a zero normal,
/// which the shader draws unlit in the fill's own colour.
pub fn fill(f: &FillMark, color: [f32; 4], out: &mut Vec<Vertex3>) {
    let usable = f.triangles.len() / 3 * 3;
    out.extend(f.triangles[..usable].iter().map(|&pos| Vertex3 {
        pos,
        normal: [0.0; 3],
        color,
        hover: 0.0,
        gloss: 0.0,
    }));
}

/// How far short of a standing prism's faces its walls stop, as a share of its depth.
const STAND_WALL_INSET: f32 = 0.03;

/// Appends the triangles of a prism (a diagram node or arrowhead): the top face, a fan over
/// the convex outline, and one wall per side. No bottom, as for slices.
///
/// Walls face away from the outline's centre whichever way it winds. A `smooth` prism (a
/// curve cut into segments) gets per-vertex normals averaged from the two sides that meet
/// there, so a cylinder shades round instead of faceted.
///
/// A standing prism is built the same way in its own frame and then turned into its vertical
/// plane; it also gets the face at its base, which can be seen.
pub fn prism(p: &PrismMark, color: [f32; 4], out: &mut Vec<Vertex3>) {
    let n = p.outline.len();
    if n < 3 || p.top <= p.base {
        return;
    }
    // From the prism's own frame (outline on x and z, heights on y) to the world.
    let place = |q: [f32; 3]| match p.stand {
        None => q,
        Some(Stand { origin, right }) => {
            let ([rx, rz], [x, y, z]) = (right, q);
            [
                origin[0] + x * rx - y * rz,
                origin[1] + z,
                origin[2] + x * rz + y * rx,
            ]
        }
    };
    let turn = |q: [f32; 3]| match p.stand {
        None => q,
        Some(Stand {
            right: [rx, rz], ..
        }) => [q[0] * rx - q[1] * rz, q[2], q[0] * rz + q[1] * rx],
    };
    let v = |pos: [f32; 3], normal: [f32; 3]| Vertex3 {
        pos: place(pos),
        normal: turn(normal),
        color,
        hover: 0.0,
        gloss: p.gloss,
    };
    let at = |i: usize, y: f32| {
        let [x, z] = p.outline[i % n];
        [x, y, z]
    };
    let up = [0.0, 1.0, 0.0];
    // A glossy top face is brightest at its far side and fades toward the viewer (the
    // reflection of a bright sky behind the scene); the shader reads the vertex's gloss as
    // how much sheen it gets. Linear in z, so a fan over a convex face draws it exactly.
    let (z0, z1) = p
        .outline
        .iter()
        .fold((f32::INFINITY, f32::NEG_INFINITY), |(a, b), q| {
            (a.min(q[1]), b.max(q[1]))
        });
    let sheen = |i: usize| {
        // A standing prism is one of many pieces of one face (a numeral's strokes): a gradient
        // per piece would show their seams, so its face takes the gradient's middle.
        if p.stand.is_some() {
            return p.gloss * 0.6;
        }
        let t = if z1 > z0 {
            (z1 - p.outline[i % n][1]) / (z1 - z0)
        } else {
            0.5
        };
        p.gloss * (0.2 + 0.8 * t)
    };
    let top = |i: usize| Vertex3 {
        gloss: sheen(i),
        ..v(at(i, p.top), up)
    };
    for i in 1..n - 1 {
        out.extend([top(0), top(i), top(i + 1)]);
    }
    if p.stand.is_some() {
        let down = [0.0, -1.0, 0.0];
        for i in 1..n - 1 {
            out.extend([
                v(at(0, p.base), down),
                v(at(i, p.base), down),
                v(at(i + 1, p.base), down),
            ]);
        }
    }
    let [cx, cz] = p.centre();
    // The outward normal of side i (from point i to point i + 1).
    let side = |i: usize| {
        let ([ax, az], [bx, bz]) = (p.outline[i % n], p.outline[(i + 1) % n]);
        let (dx, dz) = (bx - ax, bz - az);
        let len = (dx * dx + dz * dz).sqrt().max(1e-12);
        let (mut nx, mut nz) = (dz / len, -dx / len);
        let (mx, mz) = ((ax + bx) / 2.0 - cx, (az + bz) / 2.0 - cz);
        if nx * mx + nz * mz < 0.0 {
            (nx, nz) = (-nx, -nz);
        }
        [nx, 0.0, nz]
    };
    let blend = |a: [f32; 3], b: [f32; 3]| {
        let (x, z) = (a[0] + b[0], a[2] + b[2]);
        let len = (x * x + z * z).sqrt().max(1e-12);
        [x / len, 0.0, z / len]
    };
    for i in 0..n {
        let flat = side(i);
        let (na, nb) = if p.smooth {
            (blend(side(i + n - 1), flat), blend(flat, side(i + 1)))
        } else {
            (flat, flat)
        };
        // The pieces of a standing face overlap, and where one's wall met the face the two
        // would fight in the depth buffer and draw seams: its walls stop just short of it.
        let inset = if p.stand.is_some() {
            (p.top - p.base) * STAND_WALL_INSET
        } else {
            0.0
        };
        let (b0, b1) = (at(i, p.base + inset), at(i + 1, p.base + inset));
        let (t0, t1) = (at(i, p.top - inset), at(i + 1, p.top - inset));
        // Walls catch less of the sky than the top.
        let wall = |pos, normal| Vertex3 {
            gloss: p.gloss * 0.25,
            ..v(pos, normal)
        };
        out.extend([
            wall(b0, na),
            wall(b1, nb),
            wall(t0, na),
            wall(t0, na),
            wall(b1, nb),
            wall(t1, nb),
        ]);
    }
}

/// Appends the triangles of a ribbon (a thick edge on the floor): the top face, both side
/// walls and the two end caps. Corners are mitred, the mitre capped at twice the half width so a
/// sharp turn does not spike; normals of the walls follow the mitre, which shades the rounded
/// bends of a routed edge smoothly.
pub fn ribbon(r: &RibbonMark, color: [f32; 4], out: &mut Vec<Vertex3>) {
    let pts = &r.path;
    let n = pts.len();
    if n < 2 || r.top <= r.base || r.width <= 0.0 {
        return;
    }
    let v = |pos: [f32; 3], normal: [f32; 3]| Vertex3 {
        pos,
        normal,
        color,
        hover: 0.0,
        gloss: r.gloss,
    };
    let half = r.width / 2.0;
    // The unit left-hand normal of segment i (seen from above, x right and z toward the viewer).
    let seg = |i: usize| {
        let ([ax, az], [bx, bz]) = (pts[i], pts[i + 1]);
        let (dx, dz) = (bx - ax, bz - az);
        let len = (dx * dx + dz * dz).sqrt().max(1e-12);
        [dz / len, -dx / len]
    };
    // Per point: the offset to the left edge (mitred) and the wall normal there.
    let mut offsets = Vec::with_capacity(n);
    for i in 0..n {
        let a = seg(i.saturating_sub(1).min(n - 2));
        let b = seg(i.min(n - 2));
        let (mx, mz) = (a[0] + b[0], a[1] + b[1]);
        let ml = (mx * mx + mz * mz).sqrt();
        let (ux, uz) = if ml < 1e-6 {
            (b[0], b[1])
        } else {
            (mx / ml, mz / ml)
        };
        // Length along the mitre that keeps the edge `half` from both segments.
        let cos = (ux * b[0] + uz * b[1]).max(0.5);
        let k = half / cos;
        offsets.push(([ux * k, uz * k], [ux, 0.0, uz]));
    }
    let left = |i: usize, y: f32| [pts[i][0] + offsets[i].0[0], y, pts[i][1] + offsets[i].0[1]];
    let right = |i: usize, y: f32| [pts[i][0] - offsets[i].0[0], y, pts[i][1] - offsets[i].0[1]];
    let up = [0.0, 1.0, 0.0];
    let neg = |n: [f32; 3]| [-n[0], -n[1], -n[2]];
    for i in 0..n - 1 {
        let (j, top, base) = (i + 1, r.top, r.base);
        out.extend([
            v(left(i, top), up),
            v(right(i, top), up),
            v(left(j, top), up),
            v(left(j, top), up),
            v(right(i, top), up),
            v(right(j, top), up),
        ]);
        let (ni, nj) = (offsets[i].1, offsets[j].1);
        out.extend([
            v(left(i, base), ni),
            v(left(j, base), nj),
            v(left(i, top), ni),
            v(left(i, top), ni),
            v(left(j, base), nj),
            v(left(j, top), nj),
        ]);
        let (ni, nj) = (neg(ni), neg(nj));
        out.extend([
            v(right(i, base), ni),
            v(right(j, base), nj),
            v(right(i, top), ni),
            v(right(i, top), ni),
            v(right(j, base), nj),
            v(right(j, top), nj),
        ]);
    }
    // End caps, facing back along the path at the start and forward at the end.
    for (i, sign) in [(0usize, -1.0f32), (n - 1, 1.0)] {
        let s = seg(i.min(n - 2));
        // The direction of travel is the left normal turned a quarter clockwise.
        let normal = [-s[1] * sign, 0.0, s[0] * sign];
        out.extend([
            v(left(i, r.base), normal),
            v(right(i, r.base), normal),
            v(left(i, r.top), normal),
            v(left(i, r.top), normal),
            v(right(i, r.base), normal),
            v(right(i, r.top), normal),
        ]);
    }
}

/// A vertex of a translucent floor shadow: position and colour with its alpha (the layout of
/// the curtain pipeline's vertices).
pub type ShadowVertex = crate::solid::CurtainVertex;

/// Appends the triangles of a soft shadow: the outline, shrunk by half the blur, filled with
/// the shadow's colour, and a ring out to half the blur beyond the outline fading to nothing.
pub fn shadow(s: &ShadowMark, color: [f32; 4], y: f32, out: &mut Vec<ShadowVertex>) {
    let n = s.outline.len();
    if n < 3 {
        return;
    }
    let c = {
        let (x, z) = s
            .outline
            .iter()
            .fold((0.0, 0.0), |(x, z), p| (x + p[0], z + p[1]));
        [x / n as f32, z / n as f32]
    };
    // The outward unit normal at every vertex (the average of its two sides').
    let side = |i: usize| {
        let ([ax, az], [bx, bz]) = (s.outline[i % n], s.outline[(i + 1) % n]);
        let (dx, dz) = (bx - ax, bz - az);
        let len = (dx * dx + dz * dz).sqrt().max(1e-12);
        let (mut nx, mut nz) = (dz / len, -dx / len);
        if nx * ((ax + bx) / 2.0 - c[0]) + nz * ((az + bz) / 2.0 - c[1]) < 0.0 {
            (nx, nz) = (-nx, -nz);
        }
        [nx, nz]
    };
    let normal: Vec<[f32; 2]> = (0..n)
        .map(|i| {
            let (a, b) = (side(i + n - 1), side(i));
            let (x, z) = (a[0] + b[0], a[1] + b[1]);
            let l = (x * x + z * z).sqrt().max(1e-12);
            [x / l, z / l]
        })
        .collect();
    let at = |i: usize, d: f32| {
        let (p, nm) = (s.outline[i % n], normal[i % n]);
        [p[0] + nm[0] * d, y, p[1] + nm[1] * d]
    };
    let clear = [color[0], color[1], color[2], 0.0];
    let half = s.blur / 2.0;
    let centre = ShadowVertex {
        pos: [c[0], y, c[1]],
        color,
    };
    let vert = |pos, color| ShadowVertex { pos, color };
    for i in 0..n {
        let j = i + 1;
        out.extend([centre, vert(at(i, -half), color), vert(at(j, -half), color)]);
        out.extend([
            vert(at(i, -half), color),
            vert(at(i, half), clear),
            vert(at(j, -half), color),
            vert(at(j, -half), color),
            vert(at(i, half), clear),
            vert(at(j, half), clear),
        ]);
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
        assert_eq!(std::mem::size_of::<Vertex3>(), 48);
        assert_eq!(std::mem::offset_of!(Vertex3, hover), 40);
        assert_eq!(std::mem::offset_of!(Vertex3, gloss), 44);
    }

    fn prism(outline: &[[f32; 2]], smooth: bool) -> PrismMark {
        PrismMark {
            key: "p".into(),
            color: Color::linear(1.0, 0.0, 0.0, 1.0),
            outline: outline.to_vec(),
            base: 0.0,
            top: 0.5,
            smooth,
            gloss: 0.0,
            node: 0,
            stand: None,
        }
    }

    #[test]
    fn a_prism_has_a_top_and_a_wall_per_side_facing_out() {
        // A square, wound both ways.
        let square = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
        for outline in [square.to_vec(), square.iter().rev().copied().collect()] {
            let mut v = Vec::new();
            super::prism(&prism(&outline, false), [1.0; 4], &mut v);
            // 2 top triangles + 4 walls of 2.
            assert_eq!(v.len(), (2 + 4 * 2) * 3);
            for t in v.chunks_exact(3) {
                let n = t[0].normal;
                if n[1] > 0.5 {
                    assert!(t.iter().all(|v| v.pos[1] == 0.5), "top at the top");
                    continue;
                }
                // A wall's normal points away from the centre (0.5, 0.5).
                let mid = t.iter().fold([0.0f32; 2], |m, v| {
                    [m[0] + v.pos[0] / 3.0, m[1] + v.pos[2] / 3.0]
                });
                assert!(n[0] * (mid[0] - 0.5) + n[2] * (mid[1] - 0.5) > 0.0, "{n:?}");
            }
        }
    }

    #[test]
    fn a_smooth_prism_blends_its_normals() {
        let n = 24;
        let circle: Vec<[f32; 2]> = (0..n)
            .map(|k| {
                let a = TAU * k as f32 / n as f32;
                [a.cos(), a.sin()]
            })
            .collect();
        let mut v = Vec::new();
        super::prism(&prism(&circle, true), [1.0; 4], &mut v);
        // On a circle the blended normal at a corner is the radial direction itself.
        for vert in v.iter().filter(|v| v.normal[1] == 0.0) {
            let r = [vert.pos[0], vert.pos[2]];
            let dot = r[0] * vert.normal[0] + r[1] * vert.normal[2];
            assert!(dot > 0.999, "{vert:?}");
        }
    }

    #[test]
    fn a_standing_prism_is_drawn_in_its_vertical_plane_closed_at_both_ends() {
        // A unit square stood up at (1, 2, 3), its right along +x: so it faces +z, and its
        // heights 0..0.5 run toward +z.
        let mut p = prism(&[[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]], false);
        p.stand = Some(Stand {
            origin: [1.0, 2.0, 3.0],
            right: [1.0, 0.0],
        });
        let mut v = Vec::new();
        super::prism(&p, [1.0; 4], &mut v);
        // Front and back of 2 triangles each, plus 4 walls of 2.
        assert_eq!(v.len(), (2 + 2 + 4 * 2) * 3);
        for vert in &v {
            let [x, y, z] = vert.pos;
            assert!(
                (1.0..=2.0).contains(&x) && (2.0..=3.0).contains(&y),
                "{vert:?}"
            );
            assert!((3.0..=3.5).contains(&z), "{vert:?}");
        }
        let front = v.iter().filter(|v| v.normal == [0.0, 0.0, 1.0]);
        assert!(front.clone().count() == 6 && front.into_iter().all(|v| v.pos[2] == 3.5));
        let back = v.iter().filter(|v| v.normal == [0.0, 0.0, -1.0]);
        assert!(back.clone().count() == 6 && back.into_iter().all(|v| v.pos[2] == 3.0));
        // The walls face out within the plane: the top one up.
        assert!(
            v.iter()
                .any(|v| v.normal == [0.0, 1.0, 0.0] && v.pos[1] == 3.0)
        );
    }

    #[test]
    fn degenerate_prisms_draw_nothing() {
        let mut v = Vec::new();
        super::prism(&prism(&[[0.0, 0.0], [1.0, 0.0]], false), [1.0; 4], &mut v);
        let mut flat = prism(&[[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]], false);
        flat.top = 0.0;
        super::prism(&flat, [1.0; 4], &mut v);
        assert!(v.is_empty());
    }

    #[test]
    fn a_ribbon_has_a_top_two_walls_per_segment_and_two_caps() {
        let r = RibbonMark {
            key: "r".into(),
            color: Color::linear(0.0, 0.0, 0.0, 1.0),
            path: vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0]],
            width: 0.2,
            base: 0.0,
            top: 0.05,
            gloss: 0.5,
        };
        let mut v = Vec::new();
        super::ribbon(&r, [1.0; 4], &mut v);
        // 2 segments x (top + 2 walls) x 2 triangles, plus 2 caps x 2 triangles.
        assert_eq!(v.len(), (2 * 3 * 2 + 2 * 2) * 3);
        // The top stays within half the width of the path (the mitre at the corner reaches
        // half a width along both sides).
        for p in v.iter().filter(|p| p.normal[1] > 0.5) {
            assert!(p.pos[1] == 0.05);
            assert!((-0.1 - 1e-5..=1.1 + 1e-5).contains(&p.pos[0]));
            assert!((-0.1 - 1e-5..=1.0 + 1e-5).contains(&p.pos[2]));
        }
        assert!(v.iter().all(|p| p.gloss == 0.5));
    }

    #[test]
    fn a_shadow_is_dark_inside_and_fades_out() {
        let s = ShadowMark {
            outline: vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
            blur: 0.2,
            color: Color::linear(0.0, 0.0, 0.0, 0.3),
            lift: 0.0,
        };
        let mut v = Vec::new();
        super::shadow(&s, [0.0, 0.0, 0.0, 0.3], 0.001, &mut v);
        assert_eq!(v.len(), 4 * 3 * 3);
        for p in &v {
            let outside = p.pos[0] < 0.0 || p.pos[0] > 1.0 || p.pos[2] < 0.0 || p.pos[2] > 1.0;
            // Clear outside the outline, dark inside it.
            assert_eq!(p.color[3] == 0.0, outside, "{p:?}");
            assert_eq!(p.pos[1], 0.001);
        }
    }
}
