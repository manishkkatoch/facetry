//! Picking: which bar is under the pointer? Done on the CPU, with the very matrix the GPU
//! draws with, so what you hover is what you see.
//!
//! In 3D a ray is cast from the camera through the pointer's pixel and intersected with every
//! bar's box (the "slab" test: the ray's span inside each pair of parallel planes must
//! overlap); the nearest hit wins. A chart has at most about a thousand bars, so this costs
//! microseconds and needs no GPU read-back (a colour-ID pass would be the alternative, with
//! a frame of latency and a `map_async` per pointer move). 2D picking is a point-in-rectangle
//! test and lives in `facetry_core::Scene::pick`.

use facetry_core::scene3d::BoxMark;
use glam::{Mat4, Vec3, Vec4};

/// A half-line: `origin + t * dir` for `t >= 0`, `dir` of unit length.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ray {
    pub origin: Vec3,
    pub dir: Vec3,
}

/// The ray from the camera through pixel `(x, y)` (origin top-left, y down) of a viewport
/// `(width, height)`, for the world-to-clip matrix `view_proj`. It starts on the near plane.
/// `None` if the matrix cannot be inverted or the viewport is empty.
pub fn ray_through_pixel(view_proj: Mat4, viewport: (f32, f32), px: (f32, f32)) -> Option<Ray> {
    if viewport.0 <= 0.0 || viewport.1 <= 0.0 {
        return None;
    }
    let inverse = view_proj.inverse();
    if !inverse.is_finite() {
        return None;
    }
    // Pixels to normalised device coordinates (the inverse of what the shaders do).
    let (nx, ny) = (px.0 / viewport.0 * 2.0 - 1.0, 1.0 - px.1 / viewport.1 * 2.0);
    // Depth runs 0 (near) to 1 (far) in wgpu's clip space.
    let at_depth = |z: f32| {
        let p = inverse * Vec4::new(nx, ny, z, 1.0);
        p.truncate() / p.w
    };
    let (near, far) = (at_depth(0.0), at_depth(1.0));
    let dir = (far - near).try_normalize()?;
    Some(Ray { origin: near, dir })
}

/// Where the ray first enters the box `min..=max`: the distance `t` along it (0 when the
/// origin is already inside), or `None` for a miss or a box behind the ray.
pub fn ray_box(ray: &Ray, min: Vec3, max: Vec3) -> Option<f32> {
    let (mut t_in, mut t_out) = (0.0_f32, f32::INFINITY);
    for axis in 0..3 {
        let (o, d) = (ray.origin[axis], ray.dir[axis]);
        if d.abs() < 1e-9 {
            // Parallel to this pair of planes: inside the slab or never.
            if o < min[axis] || o > max[axis] {
                return None;
            }
            continue;
        }
        let (a, b) = ((min[axis] - o) / d, (max[axis] - o) / d);
        t_in = t_in.max(a.min(b));
        t_out = t_out.min(a.max(b));
        if t_in > t_out {
            return None;
        }
    }
    Some(t_in)
}

/// The nearest bar the ray hits as `(index, distance)`. Ties go to the earlier bar.
pub fn nearest_hit(boxes: &[BoxMark], ray: &Ray) -> Option<(usize, f32)> {
    boxes
        .iter()
        .enumerate()
        .filter_map(|(i, b)| Some((i, ray_box(ray, Vec3::from(b.min), Vec3::from(b.max))?)))
        .min_by(|a, b| a.1.total_cmp(&b.1))
}

/// The bar under pixel `px`, as an index into `boxes`.
pub fn pick_3d(
    boxes: &[BoxMark],
    view_proj: Mat4,
    viewport: (f32, f32),
    px: (f32, f32),
) -> Option<usize> {
    let ray = ray_through_pixel(view_proj, viewport, px)?;
    nearest_hit(boxes, &ray).map(|(i, _)| i)
}

/// Where a world point lands on screen, in pixels (what the shaders compute), or `None`
/// when it is behind the camera.
pub fn project(view_proj: Mat4, viewport: (f32, f32), p: Vec3) -> Option<(f32, f32)> {
    let c = view_proj * p.extend(1.0);
    (c.w > 0.0).then(|| {
        (
            (c.x / c.w * 0.5 + 0.5) * viewport.0,
            (0.5 - c.y / c.w * 0.5) * viewport.1,
        )
    })
}

#[cfg(test)]
mod tests {
    use facetry_core::{Color, build_scene_3d, parse_spec_json};
    use facetry_font::FontMetrics;

    use super::*;
    use crate::camera::Camera;

    fn mark(key: &str, min: [f32; 3], max: [f32; 3]) -> BoxMark {
        BoxMark {
            key: key.into(),
            min,
            max,
            color: Color::linear(0.0, 0.0, 0.0, 1.0),
            layer: 0,
            row: 0,
        }
    }

    fn ray(origin: [f32; 3], dir: [f32; 3]) -> Ray {
        Ray {
            origin: Vec3::from(origin),
            dir: Vec3::from(dir).normalize(),
        }
    }

    const MIN: Vec3 = Vec3::new(1.0, 1.0, 1.0);
    const MAX: Vec3 = Vec3::new(2.0, 3.0, 2.0);

    #[test]
    fn rays_hit_a_box_from_every_side_and_report_the_entry_distance() {
        let cases = [
            ([1.5, 2.0, 10.0], [0.0, 0.0, -1.0], 8.0), // from +z onto the z = 2 face
            ([1.5, 2.0, -5.0], [0.0, 0.0, 1.0], 6.0),  // from -z
            ([-4.0, 2.0, 1.5], [1.0, 0.0, 0.0], 5.0),  // from -x
            ([9.0, 2.0, 1.5], [-1.0, 0.0, 0.0], 7.0),  // from +x
            ([1.5, 9.0, 1.5], [0.0, -1.0, 0.0], 6.0),  // from above, onto the y = 3 face
            ([1.5, -9.0, 1.5], [0.0, 1.0, 0.0], 10.0), // from below
        ];
        for (origin, dir, want) in cases {
            let got = ray_box(&ray(origin, dir), MIN, MAX).unwrap_or(f32::NAN);
            assert!(
                (got - want).abs() < 1e-4,
                "{origin:?} -> {got}, want {want}"
            );
        }
        // A diagonal ray through the corner region.
        assert!(ray_box(&ray([0.0, 0.0, 0.0], [1.0, 1.0, 1.0]), MIN, MAX).is_some());
    }

    #[test]
    fn rays_miss_beside_behind_and_parallel_to_a_box() {
        // Beside.
        assert_eq!(
            ray_box(&ray([3.0, 2.0, 10.0], [0.0, 0.0, -1.0]), MIN, MAX),
            None
        );
        // The box is behind the origin.
        assert_eq!(
            ray_box(&ray([1.5, 2.0, -5.0], [0.0, 0.0, -1.0]), MIN, MAX),
            None
        );
        // Parallel to the x planes and outside them.
        assert_eq!(
            ray_box(&ray([0.0, 2.0, 0.0], [0.0, 0.0, 1.0]), MIN, MAX),
            None
        );
        // Grazing past an edge.
        assert_eq!(
            ray_box(&ray([0.9, 2.0, 10.0], [0.0, 0.0, -1.0]), MIN, MAX),
            None
        );
    }

    #[test]
    fn a_ray_starting_inside_hits_at_zero() {
        assert_eq!(
            ray_box(&ray([1.5, 2.0, 1.5], [0.3, 0.2, 0.9]), MIN, MAX),
            Some(0.0)
        );
    }

    #[test]
    fn a_flat_box_can_still_be_hit() {
        // A zero-value bar has no height.
        let flat = ray_box(
            &ray([1.5, 10.0, 1.5], [0.0, -1.0, 0.0]),
            Vec3::new(1.0, 2.0, 1.0),
            Vec3::new(2.0, 2.0, 2.0),
        );
        assert_eq!(flat, Some(8.0));
    }

    #[test]
    fn the_nearest_box_wins_and_ties_go_to_the_first() {
        let boxes = [
            mark("far", [0.0, 0.0, 0.0], [1.0, 1.0, 1.0]),
            mark("near", [0.0, 0.0, 3.0], [1.0, 1.0, 4.0]),
            mark("also near", [0.0, 0.0, 3.0], [1.0, 1.0, 4.0]),
            mark("aside", [5.0, 0.0, 3.0], [6.0, 1.0, 4.0]),
        ];
        let r = ray([0.5, 0.5, 10.0], [0.0, 0.0, -1.0]);
        assert_eq!(nearest_hit(&boxes, &r), Some((1, 6.0)));
        // From behind, the other end is nearest.
        let back = ray([0.5, 0.5, -10.0], [0.0, 0.0, 1.0]);
        assert_eq!(nearest_hit(&boxes, &back), Some((0, 10.0)));
        assert_eq!(
            nearest_hit(&boxes, &ray([20.0, 0.5, 10.0], [0.0, 0.0, -1.0])),
            None
        );
    }

    #[test]
    fn the_pixel_ray_runs_through_the_projected_point() {
        let boxes = [mark("b", [0.2, 0.0, 0.2], [0.5, 0.6, 0.5])];
        let size = [1.2, 1.0, 0.9];
        for (az, el) in [(0.0, 25.0), (35.0, 25.0), (123.0, 60.0), (300.0, 10.0)] {
            let mut c = Camera::default();
            c.set(Some(az), Some(el), None);
            let vp = c.plot_view_proj(size, (800.0, 500.0));
            let centre = Vec3::new(0.35, 0.3, 0.35);
            let px = project(vp, (800.0, 500.0), centre).unwrap();
            let r = ray_through_pixel(vp, (800.0, 500.0), px).unwrap();
            // The ray passes within a hair of the point it was cast through...
            let to_centre = centre - r.origin;
            let off_axis = (to_centre - r.dir * to_centre.dot(r.dir)).length();
            assert!(off_axis < 1e-3, "{az}/{el}: {off_axis}");
            // ...and so hits the box that contains it.
            assert_eq!(
                pick_3d(&boxes, vp, (800.0, 500.0), px),
                Some(0),
                "{az}/{el}"
            );
        }
    }

    #[test]
    fn pixels_off_the_bar_pick_nothing() {
        let boxes = [mark("b", [0.2, 0.0, 0.2], [0.5, 0.6, 0.5])];
        let c = Camera::default();
        let vp = c.plot_view_proj([1.2, 1.0, 0.9], (800.0, 500.0));
        assert_eq!(pick_3d(&boxes, vp, (800.0, 500.0), (5.0, 5.0)), None);
        assert_eq!(pick_3d(&boxes, vp, (0.0, 0.0), (5.0, 5.0)), None);
    }

    /// After orbiting, a click on the projected centre of a bar's top face picks that bar
    /// unless a nearer bar is in the way, in which case it picks that one instead: never a
    /// bar that is farther away.
    #[test]
    fn picks_match_the_rendered_positions_after_orbiting() {
        let json = include_str!("../../../examples/specs/bar3d-grid.json");
        let spec = parse_spec_json(json).unwrap();
        let scene = build_scene_3d(&spec, (800, 500), 1.0, FontMetrics::inter());
        let viewport = (800.0, 500.0);
        let mut direct = 0;
        let mut total = 0;
        for elevation in [15.0, 25.0, 50.0, 80.0] {
            for step in 0..24 {
                let mut c = Camera::default();
                c.set(Some(step as f32 * 15.0), Some(elevation), None);
                let vp = c.plot_view_proj(scene.size, viewport);
                for (i, b) in scene.boxes.iter().enumerate() {
                    let top = Vec3::new(
                        (b.min[0] + b.max[0]) / 2.0,
                        b.max[1],
                        (b.min[2] + b.max[2]) / 2.0,
                    );
                    // Just under the top face, so the point is inside the box, not on it.
                    let probe = top - Vec3::Y * 1e-3;
                    let px = project(vp, viewport, probe).unwrap();
                    let r = ray_through_pixel(vp, viewport, px).unwrap();
                    let (hit, t) = nearest_hit(&scene.boxes, &r).expect("the bar itself is hit");
                    let own = ray_box(&r, Vec3::from(b.min), Vec3::from(b.max)).unwrap();
                    assert!(t <= own + 1e-4, "bar {i} at {c:?}: picked a farther bar");
                    total += 1;
                    direct += usize::from(hit == i);
                }
            }
        }
        // Most bars of this chart are visible from most directions.
        assert!(direct * 10 >= total * 6, "{direct} of {total}");
    }
}
