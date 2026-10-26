//! Picking: which bar is under the pointer? Done on the CPU, with the very matrix the GPU
//! draws with, so what you hover is what you see.
//!
//! In 3D a ray is cast from the camera through the pointer's pixel and intersected with every
//! bar's box (the "slab" test: the ray's span inside each pair of parallel planes must
//! overlap); the nearest hit wins. A chart has at most about a thousand bars, so this costs
//! microseconds and needs no GPU read-back (a colour-ID pass would be the alternative, with
//! a frame of latency and a `map_async` per pointer move). 2D picking is a point-in-rectangle
//! test and lives in `facetry_core::Scene::pick`.

use facetry_core::scene3d::{BoxMark, Scene3d};
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

/// Where the ray meets the horizontal plane `y = height` (a distance `t >= 0` along it), or
/// `None` for a ray that is level with it or heads away.
pub fn ray_plane_y(ray: &Ray, height: f32) -> Option<f32> {
    if ray.dir.y.abs() < 1e-9 {
        return None;
    }
    let t = (height - ray.origin.y) / ray.dir.y;
    (t >= 0.0).then_some(t)
}

/// The line point to pick at pixel `px`, as `(index into scene.points, distance)`: every point
/// whose projection is within `scene.pick_radius` pixels of the pointer and that no box hides
/// is a candidate, and **the one nearest the camera wins** (so when points overlap on screen
/// the front one is picked, even if a back one is a pixel or two closer to the pointer).
///
/// A point is hidden when its pixel ray enters a box before reaching the point (with the same
/// slack the labels use): hovering a bar in front of a point must not pick the point behind it.
pub fn pick_point(
    scene: &Scene3d,
    view_proj: Mat4,
    viewport: (f32, f32),
    px: (f32, f32),
) -> Option<(usize, f32)> {
    let radius2 = scene.pick_radius * scene.pick_radius;
    scene
        .points
        .iter()
        .enumerate()
        .filter_map(|(i, p)| {
            let pos = Vec3::from(p.pos);
            let (x, y) = project(view_proj, viewport, pos)?;
            if (x - px.0).powi(2) + (y - px.1).powi(2) > radius2 {
                return None;
            }
            // Looking at the point itself, is a box in the way?
            let ray = ray_through_pixel(view_proj, viewport, (x, y))?;
            let distance = (pos - ray.origin).dot(ray.dir);
            let hidden = nearest_hit(&scene.boxes, &ray)
                .is_some_and(|(_, t)| t < distance - crate::labels::POINT_SLACK);
            (!hidden).then_some((i, distance))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
}

/// The mark under pixel `px` of a 3D scene, in the scene's mark index space: the bars first,
/// then the pie slices, the diagram nodes, then the line points. A visible line point within reach of the pointer
/// wins over what is behind it, as in 2D (see [`pick_point`]). Bars are hit anywhere on their box; a slice is hit **on its top
/// face** only (the ray meets the plane of the top and the point lies on the slice's
/// footprint), and so is a diagram node, so the sides of an extruded pie are not pickable: from the elevations the
/// camera allows, the top is nearly always what the pointer is over. The nearest hit wins.
pub fn pick_scene(
    scene: &Scene3d,
    view_proj: Mat4,
    viewport: (f32, f32),
    px: (f32, f32),
) -> Option<usize> {
    if let Some((i, _)) = pick_point(scene, view_proj, viewport, px) {
        return Some(scene.first_point() + i);
    }
    let ray = ray_through_pixel(view_proj, viewport, px)?;
    let bar = nearest_hit(&scene.boxes, &ray);
    let slice = scene
        .slices
        .iter()
        .enumerate()
        .filter_map(|(i, s)| {
            let t = ray_plane_y(&ray, s.height)?;
            let p = ray.origin + ray.dir * t;
            s.contains(p.x, p.z).then_some((scene.boxes.len() + i, t))
        })
        .chain(scene.prisms.iter().enumerate().filter_map(|(i, p)| {
            // Diagram nodes, like slices, on their top face.
            let t = ray_plane_y(&ray, p.top)?;
            let q = ray.origin + ray.dir * t;
            p.contains(q.x, q.z)
                .then_some((scene.boxes.len() + scene.slices.len() + i, t))
        }))
        .min_by(|a, b| a.1.total_cmp(&b.1));
    match (bar, slice) {
        (Some(b), Some(s)) => Some(if s.1 < b.1 { s.0 } else { b.0 }),
        (Some(b), None) => Some(b.0),
        (None, Some(s)) => Some(s.0),
        (None, None) => None,
    }
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
    use crate::camera::{Camera, Projection};

    fn mark(key: &str, min: [f32; 3], max: [f32; 3]) -> BoxMark {
        BoxMark {
            key: key.into(),
            min,
            max,
            color: Color::linear(0.0, 0.0, 0.0, 1.0),
            layer: 0,
            row: 0,
            sum: None,
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
    fn pie_slices_are_picked_on_their_top_face_from_any_side() {
        let json = include_str!("../../../examples/specs/pie3d.json");
        let spec = parse_spec_json(json).unwrap();
        let scene = build_scene_3d(&spec, (800, 500), 1.0, FontMetrics::inter());
        assert!(scene.boxes.is_empty() && scene.slices.len() >= 5);
        let viewport = (800.0, 500.0);
        for elevation in [15.0, 40.0, 80.0] {
            for step in 0..12 {
                let mut c = Camera::default();
                c.set(Some(step as f32 * 30.0), Some(elevation), None);
                let vp = c.plot_view_proj(scene.size, viewport);
                for (i, s) in scene.slices.iter().enumerate() {
                    let (a0, a1) = s.span();
                    let mid = s.point((a0 + a1) / 2.0, (s.inner + s.outer) / 2.0, s.height);
                    let px = project(vp, viewport, Vec3::from(mid)).unwrap();
                    assert_eq!(
                        pick_scene(&scene, vp, viewport, px),
                        Some(i),
                        "slice {i} at azimuth {} elevation {elevation}",
                        step * 30
                    );
                }
            }
        }
        // Off the pie: nothing. (The corner of the canvas.)
        let vp = Camera::default().plot_view_proj(scene.size, viewport);
        assert_eq!(pick_scene(&scene, vp, viewport, (3.0, 3.0)), None);
    }

    #[test]
    fn a_donut_hole_and_the_gap_between_slices_pick_nothing() {
        let json = include_str!("../../../examples/specs/pie3d.json")
            .replace(r#""depth": 0.25"#, r#""depth": 0.25, "innerRadius": 0.5"#);
        let spec = parse_spec_json(&json).unwrap();
        let scene = build_scene_3d(&spec, (800, 500), 1.0, FontMetrics::inter());
        let viewport = (800.0, 500.0);
        let c = Camera::from_spec(spec.camera.as_ref());
        let vp = c.plot_view_proj(scene.size, viewport);
        let s = &scene.slices[0];
        // The middle of the hole, at the height of the top face.
        let hole = project(vp, viewport, Vec3::new(s.center[0], s.height, s.center[1])).unwrap();
        assert_eq!(pick_scene(&scene, vp, viewport, hole), None);
    }

    /// Diagram nodes are picked on their top face, in the index space after the boxes and
    /// slices, from every camera, and the floor between them picks nothing.
    #[test]
    fn diagram_nodes_are_picked_on_their_top_face() {
        let scene = scene_of("diagram-basic");
        let viewport = (800.0, 500.0);
        assert_eq!(scene.prisms.len(), 4);
        for kind in [Projection::Orthographic, Projection::Perspective] {
            for elevation in [20.0, 50.0, 80.0] {
                for step in 0..8 {
                    let mut c = Camera {
                        projection: kind,
                        ..Camera::default()
                    };
                    c.set(Some(step as f32 * 45.0), Some(elevation), Some(2.0));
                    let vp = c.plot_view_proj(scene.size, viewport);
                    for (i, p) in scene.prisms.iter().enumerate() {
                        let [x, z] = p.centre();
                        let px = project(vp, viewport, Vec3::new(x, p.top, z)).unwrap();
                        assert_eq!(
                            pick_scene(&scene, vp, viewport, px),
                            Some(i),
                            "node {i} at azimuth {} elevation {elevation} {kind:?}",
                            step * 45
                        );
                    }
                }
            }
        }
        // The corner of the floor, between nodes: nothing.
        let vp = Camera::default().plot_view_proj(scene.size, viewport);
        let corner = project(vp, viewport, Vec3::new(0.01, 0.0, 0.01)).unwrap();
        assert_eq!(pick_scene(&scene, vp, viewport, corner), None);
    }

    fn scene_of(name: &str) -> Scene3d {
        let json = std::fs::read_to_string(format!(
            "{}/../../examples/specs/{name}.json",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap();
        build_scene_3d(
            &parse_spec_json(&json).unwrap(),
            (800, 500),
            1.0,
            FontMetrics::inter(),
        )
    }

    /// A line point is found at its own projection from every side; where several points
    /// overlap on screen the one picked is the one nearest the camera.
    #[test]
    fn line_points_are_picked_where_they_are_drawn_after_orbiting() {
        let scene = scene_of("line3d");
        let viewport = (800.0, 500.0);
        let first_point = scene.boxes.len() + scene.slices.len();
        for elevation in [10.0, 30.0, 60.0, 80.0] {
            for step in 0..12 {
                let mut c = Camera::default();
                c.set(Some(step as f32 * 30.0), Some(elevation), None);
                let vp = c.plot_view_proj(scene.size, viewport);
                let eye = c.eye(
                    Camera::plot_frame(scene.size).0,
                    Camera::plot_frame(scene.size).1,
                );
                for (i, p) in scene.points.iter().enumerate() {
                    let px = project(vp, viewport, Vec3::from(p.pos)).unwrap();
                    let got = pick_scene(&scene, vp, viewport, px)
                        .unwrap_or_else(|| panic!("nothing at point {i}"));
                    let j = got.checked_sub(first_point).expect("a point, not a bar");
                    // Either this point, or another within reach that is nearer the camera.
                    let (a, b) = (Vec3::from(p.pos), Vec3::from(scene.points[j].pos));
                    let (qx, qy) = project(vp, viewport, b).unwrap();
                    assert!(
                        (qx - px.0).hypot(qy - px.1) <= scene.pick_radius + 1e-3,
                        "az {} el {elevation}: point {i} picked {j}",
                        step * 30
                    );
                    assert!(b.distance(eye) <= a.distance(eye) + 1e-3);
                }
            }
        }
    }

    #[test]
    fn the_point_nearest_the_camera_wins_even_if_another_is_closer_to_the_pointer() {
        let mut scene = scene_of("line3d");
        scene.points.truncate(1);
        let viewport = (800.0, 500.0);
        let c = Camera::default();
        let vp = c.plot_view_proj(scene.size, viewport);
        let far = Vec3::from(scene.points[0].pos);
        let px = project(vp, viewport, far).unwrap();
        // A second point on the same ray, nearer the camera, drawn 6 px to the side.
        let ray = ray_through_pixel(vp, viewport, (px.0 + 6.0, px.1)).unwrap();
        let near = ray.origin + ray.dir * ((far - ray.origin).dot(ray.dir) - 0.2);
        let mut second = scene.points[0].clone();
        second.pos = near.to_array();
        scene.points.push(second);
        let near_px = project(vp, viewport, near).unwrap();
        assert!((near_px.0 - px.0).abs() > 1.0 && (near_px.0 - px.0).abs() < scene.pick_radius);
        // Pointer exactly on the far point: the near one (within reach) is picked.
        assert_eq!(pick_scene(&scene, vp, viewport, px), Some(1));
        // Pointer beyond reach of the near point but on the far one: the far one.
        let off = (px.0 - scene.pick_radius + 0.5, px.1);
        assert!(
            (project(vp, viewport, near).unwrap().0 - off.0).abs() > scene.pick_radius,
            "the near point is out of reach from here"
        );
        assert_eq!(pick_scene(&scene, vp, viewport, off), Some(0));
        // Nothing near any point: nothing.
        assert_eq!(pick_scene(&scene, vp, viewport, (5.0, 5.0)), None);
    }

    /// A point inside a bar, or behind one, is not hoverable: what the viewer sees is the bar.
    #[test]
    fn a_point_hidden_by_a_bar_is_not_picked_but_the_bar_is() {
        let json = std::fs::read_to_string(format!(
            "{}/../../examples/specs/combo3d-bar-line.json",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap();
        // Targets far below the bars: every point is inside the bar of its own cell.
        let open = json.find("\"target\": [").unwrap() + "\"target\": [".len();
        let close = open + json[open..].find(']').unwrap();
        let low = format!(
            "{}{}{}",
            &json[..open],
            ["5"; 12].join(", "),
            &json[close..]
        );
        let scene = build_scene_3d(
            &parse_spec_json(&low).unwrap(),
            (800, 500),
            1.0,
            FontMetrics::inter(),
        );
        assert_eq!((scene.boxes.len(), scene.points.len()), (12, 12));
        let viewport = (800.0, 500.0);
        for (az, el) in [(0.0, 20.0), (35.0, 30.0), (200.0, 40.0)] {
            let mut c = Camera::default();
            c.set(Some(az), Some(el), None);
            let vp = c.plot_view_proj(scene.size, viewport);
            for p in &scene.points {
                let px = project(vp, viewport, Vec3::from(p.pos)).unwrap();
                // Either the bar it is inside of, or nothing: never the point.
                if let Some(i) = pick_scene(&scene, vp, viewport, px) {
                    assert!(i < scene.boxes.len(), "{}: picked mark {i}", p.key);
                }
            }
        }
        // Raise the line above the bars and every point is hoverable again.
        let scene = scene_of("combo3d-bar-line");
        let mut c = Camera::default();
        c.set(Some(30.0), Some(30.0), None);
        let vp = c.plot_view_proj(scene.size, viewport);
        for (k, p) in scene.points.iter().enumerate() {
            let px = project(vp, viewport, Vec3::from(p.pos)).unwrap();
            let got = pick_scene(&scene, vp, viewport, px).expect("a point");
            assert!(
                got >= scene.boxes.len(),
                "point {k} ({}) picked mark {got}",
                p.key
            );
        }
    }

    #[test]
    fn a_point_wins_over_the_bar_behind_it_within_reach() {
        let scene = scene_of("combo3d-bar-line");
        let viewport = (800.0, 500.0);
        let mut c = Camera::default();
        c.set(Some(30.0), Some(30.0), None);
        let vp = c.plot_view_proj(scene.size, viewport);
        let p = &scene.points[0];
        let px = project(vp, viewport, Vec3::from(p.pos)).unwrap();
        // 8 px from the point: still the point.
        let got = pick_scene(&scene, vp, viewport, (px.0 + 8.0, px.1));
        assert!(got.is_some_and(|i| i >= scene.boxes.len()));
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
