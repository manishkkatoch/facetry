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

use facetry_core::scene3d::{DEFAULT_AZIMUTH, DEFAULT_DISTANCE, DEFAULT_ELEVATION, FOV_Y_DEGREES};
use facetry_core::spec::{CameraKind, CameraSpec};
use glam::{Mat4, Vec3};

/// Looking straight down or along the floor makes orbiting awkward (the up vector degenerates
/// at 90 degrees) and bar charts unreadable, so elevation stays between these.
pub const MIN_ELEVATION: f32 = 5.0;
pub const MAX_ELEVATION: f32 = 85.0;
/// Closer than this the box fills the screen and clips the near plane; farther it is a dot.
pub const MIN_DISTANCE: f32 = 1.3;
pub const MAX_DISTANCE: f32 = 8.0;

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
    /// In box radii, within [`MIN_DISTANCE`]..=[`MAX_DISTANCE`].
    pub distance: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            projection: Projection::Perspective,
            azimuth: DEFAULT_AZIMUTH as f32,
            elevation: DEFAULT_ELEVATION as f32,
            distance: DEFAULT_DISTANCE as f32,
        }
    }
}

impl Camera {
    /// The spec's camera with defaults for whatever it leaves out.
    pub fn from_spec(spec: Option<&CameraSpec>) -> Self {
        let mut camera = Self::default();
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
        self.distance = self.distance.clamp(MIN_DISTANCE, MAX_DISTANCE);
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
        // The box (plus a little for tick marks) lies within 1.1 radii of the centre, so
        // this near/far pair clips nothing and wastes little depth precision.
        let near = (dist - radius * 1.1).max(0.01);
        let far = dist + radius * 1.1;
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
}
