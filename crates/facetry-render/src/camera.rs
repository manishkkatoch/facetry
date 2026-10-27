//! The orbit camera: azimuth, elevation and distance around the centre of the plot box, with
//! a perspective or orthographic lens. Pure maths on top of `glam`; no GPU in here, so it is
//! unit-tested natively.
//!
//! Conventions (right-handed, y up, like the world the scene is built in):
//! * azimuth 0 puts the camera on the +z side looking toward -z, i.e. at the front of the
//!   chart; positive azimuth moves the camera toward +x (to the right);
//! * elevation is the angle above the floor;
//! * distance is in units of the plot box's radius (half its diagonal, at least 1: see
//!   `facetry_core::framing_radius`), so the same number frames a small and a big chart alike.

use facetry_core::framing_radius;
use facetry_core::scene3d::{DEFAULT_AZIMUTH, DEFAULT_DISTANCE, DEFAULT_ELEVATION, FOV_Y_DEGREES};
use facetry_core::spec::{CameraKind, CameraSpec};
use glam::{Mat4, Vec3};

/// Looking straight down or along the floor makes orbiting awkward (the up vector degenerates
/// at 90 degrees) and bar charts unreadable, so elevation stays between these.
pub const MIN_ELEVATION: f32 = 5.0;
pub const MAX_ELEVATION: f32 = 85.0;
/// Closer than this the box fills the screen and clips the near plane; farther it is a dot.
pub const MIN_DISTANCE: f32 = 1.3;
/// How close a camera that may pan (a diagram's) may come: a large diagram is then read a
/// corner at a time.
pub const CLOSE_UP_DISTANCE: f32 = 0.15;
pub const MAX_DISTANCE: f32 = 8.0;

/// See [`facetry_core::scene3d::FRAMING_LIFT`].
const FRAMING_LIFT: f32 = facetry_core::scene3d::FRAMING_LIFT as f32;

/// Degrees of rotation per dragged pixel.
const ORBIT_SPEED: f32 = 0.4;
/// Zoom: the distance is multiplied by `exp(delta * ZOOM_SPEED)`, so equal wheel steps
/// change the size by equal *percentages* however far in you are.
const ZOOM_SPEED: f32 = 0.0015;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Projection {
    Perspective,
    Orthographic,
}

impl Projection {
    pub fn name(self) -> &'static str {
        match self {
            Self::Perspective => "perspective",
            Self::Orthographic => "orthographic",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera {
    pub projection: Projection,
    /// Degrees, wrapped to `0..360`.
    pub azimuth: f32,
    /// Degrees, within [`MIN_ELEVATION`]..=[`MAX_ELEVATION`].
    pub elevation: f32,
    /// In box radii, within [`min_distance`](Self::min_distance)..=[`MAX_DISTANCE`].
    pub distance: f32,
    /// How far the point looked at has been moved from the plot box's centre, `[x, z]` world
    /// units on the floor plane (a pan). Kept within the box.
    pub pan: [f32; 2],
    /// The closest `distance` allowed: [`MIN_DISTANCE`], or [`CLOSE_UP_DISTANCE`] for a
    /// camera that may pan.
    pub min_distance: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            projection: Projection::Perspective,
            azimuth: DEFAULT_AZIMUTH as f32,
            elevation: DEFAULT_ELEVATION as f32,
            distance: DEFAULT_DISTANCE as f32,
            pan: [0.0; 2],
            min_distance: MIN_DISTANCE,
        }
    }
}

impl Camera {
    /// The spec's camera with defaults for whatever it leaves out.
    pub fn from_spec(spec: Option<&CameraSpec>) -> Self {
        Self::from_spec_with_min(spec, MIN_DISTANCE)
    }

    /// [`from_spec`](Self::from_spec) for a camera that may come as close as `min_distance`
    /// (box radii).
    pub fn from_spec_with_min(spec: Option<&CameraSpec>, min_distance: f32) -> Self {
        let mut camera = Self {
            min_distance,
            ..Self::default()
        };
        if let Some(spec) = spec {
            camera.projection = match spec.kind.unwrap_or_default() {
                CameraKind::Perspective => Projection::Perspective,
                CameraKind::Orthographic => Projection::Orthographic,
            };
            camera.set(
                spec.azimuth.map(|v| v as f32),
                spec.elevation.map(|v| v as f32),
                spec.distance.map(|v| v as f32),
            );
        }
        camera
    }

    /// Sets any of the three values, then clamps. Non-finite numbers are ignored.
    pub fn set(&mut self, azimuth: Option<f32>, elevation: Option<f32>, distance: Option<f32>) {
        let finite = |v: Option<f32>| v.filter(|v| v.is_finite());
        self.azimuth = finite(azimuth).unwrap_or(self.azimuth);
        self.elevation = finite(elevation).unwrap_or(self.elevation);
        self.distance = finite(distance).unwrap_or(self.distance);
        self.clamp();
    }

    pub fn clamp(&mut self) {
        self.azimuth = self.azimuth.rem_euclid(360.0);
        // `rem_euclid` of a tiny negative number rounds up to exactly 360.
        if self.azimuth >= 360.0 {
            self.azimuth = 0.0;
        }
        self.elevation = self.elevation.clamp(MIN_ELEVATION, MAX_ELEVATION);
        self.distance = self.distance.clamp(self.min_distance, MAX_DISTANCE);
    }

    /// A drag of `(dx, dy)` pixels. The scene follows the pointer: dragging right turns the
    /// chart to the right (the camera moves left), dragging down tips it toward you (the
    /// camera rises).
    pub fn orbit(&mut self, dx_px: f32, dy_px: f32) {
        self.set(
            Some(self.azimuth - dx_px * ORBIT_SPEED),
            Some(self.elevation + dy_px * ORBIT_SPEED),
            None,
        );
    }

    /// Positive `delta` (wheel down) zooms out, negative zooms in.
    pub fn zoom(&mut self, delta: f32) {
        // Clamping the exponent keeps a huge wheel delta from overflowing to infinity (which
        // `set` would then ignore instead of zooming all the way out).
        let factor = (delta * ZOOM_SPEED).clamp(-5.0, 5.0).exp();
        self.set(None, None, Some(self.distance * factor));
    }

    /// The point an unpanned camera looks at and the unit its distance is measured in, for a
    /// plot box of `size`.
    pub fn plot_frame(size: [f32; 3]) -> (Vec3, f32) {
        (
            Vec3::from(size) / 2.0 - Vec3::Y * FRAMING_LIFT,
            framing_radius(size),
        )
    }

    /// The point this camera looks at, for a plot box of `size`: the frame's centre moved by
    /// the pan.
    pub fn target(&self, size: [f32; 3]) -> Vec3 {
        Self::plot_frame(size).0 + Vec3::new(self.pan[0], 0.0, self.pan[1])
    }

    /// World to clip space for a plot box of `size` shown in a viewport of `viewport` pixels.
    /// The one matrix used for drawing, for placing labels and for picking, so they can never
    /// disagree about where a bar is on screen.
    pub fn plot_view_proj(&self, size: [f32; 3], viewport: (f32, f32)) -> Mat4 {
        let radius = Self::plot_frame(size).1;
        self.view_proj(viewport.0 / viewport.1.max(1.0), self.target(size), radius)
    }

    /// A pan by a pointer drag of `(dx, dy)` pixels in a viewport `viewport_h` pixels high,
    /// for a plot box of `size`: the floor follows the pointer (at the point looked at), and
    /// the point looked at stays inside the box.
    pub fn pan_by(&mut self, dx_px: f32, dy_px: f32, viewport_h: f32, size: [f32; 3]) {
        let radius = Self::plot_frame(size).1;
        // World units per pixel at the point looked at (both lenses show the same height
        // there, see `view_proj`).
        let half_tan = (FOV_Y_DEGREES as f32 / 2.0).to_radians().tan();
        let per_px = 2.0 * self.distance * radius * half_tan / viewport_h.max(1.0);
        let az = self.azimuth.to_radians();
        // Screen right, and "toward the viewer" on the floor (foreshortened by the elevation).
        let right = [az.cos(), -az.sin()];
        let toward = [az.sin(), az.cos()];
        let depth = per_px / self.elevation.to_radians().sin().max(0.1);
        self.move_pan(
            [
                -(right[0] * dx_px * per_px) - toward[0] * dy_px * depth,
                -(right[1] * dx_px * per_px) - toward[1] * dy_px * depth,
            ],
            size,
        );
    }

    /// Moves the point looked at by `delta` (`[x, z]` world units), staying inside the box.
    pub fn move_pan(&mut self, delta: [f32; 2], size: [f32; 3]) {
        let finite = delta.iter().all(|v| v.is_finite());
        if !finite {
            return;
        }
        self.pan = [
            (self.pan[0] + delta[0]).clamp(-size[0] / 2.0, size[0] / 2.0),
            (self.pan[1] + delta[1]).clamp(-size[2] / 2.0, size[2] / 2.0),
        ];
    }

    /// Where the camera is, for a box with this `centre` and `radius`.
    pub fn eye(&self, centre: Vec3, radius: f32) -> Vec3 {
        let (az, el) = (self.azimuth.to_radians(), self.elevation.to_radians());
        let dir = Vec3::new(az.sin() * el.cos(), el.sin(), az.cos() * el.cos());
        centre + dir * (self.distance * radius)
    }

    /// World to clip space (depth 0..1, as wgpu wants) for a viewport of the given aspect
    /// ratio (width / height).
    pub fn view_proj(&self, aspect: f32, centre: Vec3, radius: f32) -> Mat4 {
        let eye = self.eye(centre, radius);
        let view = Mat4::look_at_rh(eye, centre, Vec3::Y);
        let dist = self.distance * radius;
        // The box (plus a little for tick marks) lies within 1.1 radii of its centre, and the
        // point looked at is at most the pan away from that, so this near/far pair clips
        // nothing and wastes little depth precision.
        let reach = radius * 1.1 + (self.pan[0] * self.pan[0] + self.pan[1] * self.pan[1]).sqrt();
        let near = (dist - reach).max(dist * 0.01).max(0.001);
        let far = dist + reach;
        let half_tan = (FOV_Y_DEGREES as f32 / 2.0).to_radians().tan();
        let proj = match self.projection {
            Projection::Perspective => {
                Mat4::perspective_rh(FOV_Y_DEGREES.to_radians() as f32, aspect, near, far)
            }
            Projection::Orthographic => {
                // Same visible height at the centre as the perspective lens, so switching
                // lens (or zooming) keeps the chart about the same size.
                let h = dist * half_tan;
                Mat4::orthographic_rh(-h * aspect, h * aspect, -h, h, near, far)
            }
        };
        proj * view
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CENTRE: Vec3 = Vec3::new(0.6, 0.5, 0.45);
    const RADIUS: f32 = 0.9;

    #[test]
    fn defaults_come_from_the_shared_constants() {
        let c = Camera::from_spec(None);
        assert_eq!(c.projection, Projection::Perspective);
        assert_eq!((c.azimuth, c.elevation), (35.0, 25.0));
    }

    #[test]
    fn spec_values_are_used_and_clamped() {
        let spec = CameraSpec {
            kind: Some(CameraKind::Orthographic),
            azimuth: Some(-90.0),
            elevation: Some(400.0),
            distance: Some(0.1),
        };
        let c = Camera::from_spec(Some(&spec));
        assert_eq!(c.projection, Projection::Orthographic);
        assert_eq!(c.azimuth, 270.0);
        assert_eq!(c.elevation, MAX_ELEVATION);
        assert_eq!(c.distance, MIN_DISTANCE);
    }

    #[test]
    fn azimuth_never_reaches_360() {
        let mut c = Camera::default();
        c.set(Some(-1e-9), None, None);
        assert!((0.0..360.0).contains(&c.azimuth), "{}", c.azimuth);
    }

    #[test]
    fn non_finite_input_is_ignored() {
        let mut c = Camera::default();
        c.set(Some(f32::NAN), Some(f32::INFINITY), Some(f32::NAN));
        assert_eq!(c, Camera::default());
    }

    #[test]
    fn azimuth_zero_is_the_front_and_positive_goes_right() {
        let mut c = Camera::default();
        c.set(Some(0.0), Some(5.0), Some(2.0));
        let eye = c.eye(CENTRE, RADIUS);
        assert!(eye.z > CENTRE.z && (eye.x - CENTRE.x).abs() < 1e-5);
        c.set(Some(90.0), None, None);
        assert!(c.eye(CENTRE, RADIUS).x > CENTRE.x);
    }

    #[test]
    fn eye_is_at_the_requested_distance() {
        let c = Camera::default();
        let d = c.eye(CENTRE, RADIUS).distance(CENTRE);
        assert!((d - c.distance * RADIUS).abs() < 1e-4);
    }

    #[test]
    fn orbit_follows_the_pointer_and_clamps_elevation() {
        let mut c = Camera::default();
        c.orbit(10.0, 0.0);
        assert!(c.azimuth < 35.0);
        c.orbit(0.0, 10_000.0);
        assert_eq!(c.elevation, MAX_ELEVATION);
        c.orbit(0.0, -10_000.0);
        assert_eq!(c.elevation, MIN_ELEVATION);
        c.orbit(-10_000.0, 0.0);
        assert!((0.0..360.0).contains(&c.azimuth));
    }

    #[test]
    fn zoom_is_multiplicative_and_clamped() {
        let mut c = Camera::default();
        let before = c.distance;
        c.zoom(100.0);
        assert!(c.distance > before);
        c.zoom(-100.0);
        assert!((c.distance - before).abs() < 1e-4);
        c.zoom(1e6);
        assert_eq!(c.distance, MAX_DISTANCE);
        c.zoom(-1e6);
        assert_eq!(c.distance, MIN_DISTANCE);
    }

    #[test]
    fn the_centre_projects_to_the_middle_of_the_screen_with_either_lens() {
        for projection in [Projection::Perspective, Projection::Orthographic] {
            let c = Camera {
                projection,
                ..Camera::default()
            };
            let clip = c.view_proj(1.6, CENTRE, RADIUS) * CENTRE.extend(1.0);
            let ndc = clip.truncate() / clip.w;
            assert!(ndc.x.abs() < 1e-4 && ndc.y.abs() < 1e-4, "{ndc:?}");
            assert!((0.0..1.0).contains(&ndc.z), "{ndc:?}");
        }
    }

    #[test]
    fn the_whole_box_is_in_front_of_the_camera_and_inside_the_depth_range() {
        let c = Camera {
            distance: MIN_DISTANCE,
            ..Camera::default()
        };
        let vp = c.view_proj(1.0, CENTRE, RADIUS);
        for x in [0.0, 1.2] {
            for y in [0.0, 1.0] {
                for z in [0.0, 0.9] {
                    let clip = vp * Vec3::new(x, y, z).extend(1.0);
                    assert!(clip.w > 0.0);
                    let depth = clip.z / clip.w;
                    assert!((0.0..=1.0).contains(&depth), "{depth}");
                }
            }
        }
    }

    #[test]
    fn orthographic_has_no_perspective_division() {
        let c = Camera {
            projection: Projection::Orthographic,
            ..Camera::default()
        };
        let clip = c.view_proj(1.0, CENTRE, RADIUS) * Vec3::new(1.0, 0.2, 0.3).extend(1.0);
        assert!((clip.w - 1.0).abs() < 1e-6);
    }

    #[test]
    fn panning_moves_the_point_looked_at_with_the_pointer_and_stays_in_the_box() {
        let size = [2.0, 0.1, 1.0];
        let mut c = Camera {
            azimuth: 0.0,
            elevation: 45.0,
            ..Camera::default()
        };
        let centre = c.target(size);
        // Dragging right moves the picture right: the point looked at moves left (-x).
        c.pan_by(50.0, 0.0, 500.0, size);
        assert!(c.target(size).x < centre.x);
        assert!((c.target(size).z - centre.z).abs() < 1e-6);
        // Dragging down brings the far side nearer: the point looked at moves away (-z).
        let before = c.target(size);
        c.pan_by(0.0, 50.0, 500.0, size);
        assert!(c.target(size).z < before.z);
        // A huge drag stops at the box's edge.
        c.pan_by(1e6, 1e6, 500.0, size);
        assert!(c.pan[0].abs() <= 1.0 + 1e-6 && c.pan[1].abs() <= 0.5 + 1e-6);
        // Non-finite input is ignored.
        let p = c.pan;
        c.move_pan([f32::NAN, 0.0], size);
        assert_eq!(c.pan, p);
    }

    #[test]
    fn a_panned_camera_still_sees_the_whole_box_between_its_clip_planes() {
        let size = [2.0, 0.1, 2.0];
        let mut c = Camera::from_spec_with_min(None, CLOSE_UP_DISTANCE);
        c.set(None, None, Some(0.2));
        assert_eq!(c.distance, 0.2);
        c.move_pan([1.0, 1.0], size);
        let vp = c.plot_view_proj(size, (800.0, 500.0));
        // The far corner of the box is not cut off by the far plane.
        let p = vp * glam::Vec4::new(0.0, 0.0, 0.0, 1.0);
        assert!(p.z / p.w <= 1.0, "{p:?}");
        // Charts keep their limit.
        let mut chart = Camera::default();
        chart.set(None, None, Some(0.2));
        assert_eq!(chart.distance, MIN_DISTANCE);
    }
}
