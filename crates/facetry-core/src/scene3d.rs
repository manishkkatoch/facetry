//! The 3D scene: bars as boxes, axes and gridlines as lines, labels as text, all positioned
//! in **world units** instead of pixels. Like the 2D [`crate::Scene`] it is flat, keyed and
//! snapshot-testable, and it knows nothing about the camera.
//!
//! The plot is a box: `x` in `0..=size[0]`, `y` (up) in `0..=1`, `z` (depth) in
//! `0..=size[2]`. Which walls are "back" walls and which edges should carry the axes depends
//! on where the camera is, but the scene must not be rebuilt as the camera orbits. So every
//! item that only suits some viewpoints carries an [`AzimuthRange`] ("draw me while the
//! camera azimuth is in this range") and the renderer's shaders do the test per frame.

use serde::{Serialize, Serializer};

use crate::Color;
use crate::scene::{Anchor, RuleKind};
use crate::spec::LabelShow;

/// World units are fractions of the plot box, so 4 decimals is far below a pixel.
fn w4_3<S: Serializer>(v: &[f32; 3], s: S) -> Result<S::Ok, S::Error> {
    let r = |x: f32| (f64::from(x) * 10_000.0).round() / 10_000.0;
    (r(v[0]), r(v[1]), r(v[2])).serialize(s)
}

fn px2<S: Serializer>(v: &[f32; 2], s: S) -> Result<S::Ok, S::Error> {
    let r = |x: f32| (f64::from(x) * 100.0).round() / 100.0;
    (r(v[0]), r(v[1])).serialize(s)
}

/// Starting camera when the spec leaves a field out. The renderer uses the same numbers; the
/// layout reads them to guess how big the plot will be on screen.
pub const DEFAULT_AZIMUTH: f64 = 35.0;
pub const DEFAULT_ELEVATION: f64 = 25.0;
/// In units of the box's radius, see [`crate::framing_radius`].
pub const DEFAULT_DISTANCE: f64 = 2.7;
/// Vertical field of view of the perspective camera, in degrees.
pub const FOV_Y_DEGREES: f64 = 40.0;

/// "Draw while the camera's azimuth is in `from..to` degrees": start inclusive, end
/// exclusive, and a range with `from > to` wraps through 360 (`[270, 90)` is "in front").
///
/// Azimuth 0 is the camera in front of the box (on the +z side) and it grows toward +x, so
/// `[0, 180)` is "the camera is on the +x side of the box". Ranges are tested against the
/// camera's own azimuth, so every viewpoint has exactly one answer, with no ties.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct AzimuthRange {
    #[serde(serialize_with = "deg")]
    pub from: f32,
    #[serde(serialize_with = "deg")]
    pub to: f32,
}

fn deg<S: Serializer>(v: &f32, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_f64((f64::from(*v) * 100.0).round() / 100.0)
}

impl AzimuthRange {
    pub const fn new(from: f32, to: f32) -> Self {
        Self { from, to }
    }

    /// The same test the shaders do (`azimuth` in `0..360`).
    pub fn contains(&self, azimuth: f32) -> bool {
        if self.from <= self.to {
            azimuth >= self.from && azimuth < self.to
        } else {
            azimuth >= self.from || azimuth < self.to
        }
    }
}

/// A bar.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BoxMark {
    pub key: String,
    #[serde(serialize_with = "w4_3")]
    pub min: [f32; 3],
    #[serde(serialize_with = "w4_3")]
    pub max: [f32; 3],
    pub color: Color,
    /// The layer index and table row the bar came from (what hover and click events report).
    /// Not in the snapshot: the key already encodes it.
    #[serde(skip)]
    pub layer: usize,
    #[serde(skip)]
    pub row: usize,
    /// The summed value if the box stands for several merged rows (see
    /// [`crate::scene::Rect::sum`]).
    #[serde(skip)]
    pub sum: Option<f64>,
}

/// A straight line segment in world space (gridline, axis edge or tick mark). It is drawn as
/// a screen-space quad `width_px` wide, so it is as crisp as 2D lines whatever the angle.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Line3 {
    pub key: String,
    pub kind: RuleKind,
    #[serde(serialize_with = "w4_3")]
    pub a: [f32; 3],
    #[serde(serialize_with = "w4_3")]
    pub b: [f32; 3],
    /// Device pixels.
    #[serde(serialize_with = "px")]
    pub width_px: f32,
    pub color: Color,
    /// Draw only while the camera azimuth is in this range. `None`: always.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub azimuths: Option<AzimuthRange>,
}

fn px<S: Serializer>(v: &f32, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_f64((f64::from(*v) * 100.0).round() / 100.0)
}

/// Which point of the text's height sits at the anchor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VAlign {
    /// The top of capital letters.
    Top,
    /// The middle of capital letters.
    Middle,
    /// The baseline.
    Baseline,
}

/// What a label is, which decides whether the renderer may hide or move it as the camera
/// turns. Axis labels depend on the viewpoint (an edge seen end-on squeezes its labels
/// together), and that cannot be decided when the scene is built: it is decided per frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LabelRole {
    /// Always drawn where it is (value labels).
    #[default]
    Fixed,
    /// The `index`-th of `count` tick labels along axis `group` (see [`AXIS_X`], [`AXIS_Z`],
    /// [`AXIS_Y`]).
    Tick { group: u8, index: u16, count: u16 },
    /// The title of axis `group`; shown in preference to tick labels.
    Title { group: u8 },
}

/// Label groups: which axis a [`LabelRole`] belongs to.
pub const AXIS_X: u8 = 0;
pub const AXIS_Z: u8 = 1;
pub const AXIS_Y: u8 = 2;

/// A label. `anchor` is a point in the world; the text is then placed relative to where that
/// point lands on screen: `align`/`valign` pick which point of the text sits there and
/// `offset_px` (x right, y down, device pixels) nudges it. So labels always face the viewer
/// and keep their pixel size however the camera moves.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Text3 {
    pub key: String,
    pub text: String,
    #[serde(serialize_with = "w4_3")]
    pub anchor: [f32; 3],
    #[serde(serialize_with = "px2")]
    pub offset_px: [f32; 2],
    #[serde(serialize_with = "px")]
    pub size_px: f32,
    pub color: Color,
    pub align: Anchor,
    pub valign: VAlign,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub azimuths: Option<AzimuthRange>,
    /// Not in the snapshot: the key says which axis and category it is.
    #[serde(skip)]
    pub role: LabelRole,
}

/// The value label of a bar (`boxes[mark]`); see [`crate::scene::MarkLabel`].
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MarkLabel3 {
    /// Index into [`Scene3d::boxes`].
    pub mark: usize,
    pub show: LabelShow,
    pub text: Text3,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Scene3d {
    pub width: u32,
    pub height: u32,
    pub background: Color,
    /// The plot box's extent: `[x, 1, z]`; it starts at the origin.
    #[serde(serialize_with = "w4_3")]
    pub size: [f32; 3],
    /// Labels get an outline of this width (device pixels) in the background colour, so
    /// they stay readable where a bar passes behind them.
    #[serde(serialize_with = "px")]
    pub halo_px: f32,
    pub boxes: Vec<BoxMark>,
    /// Gridlines on the back walls and floor, then axis edges and ticks.
    pub lines: Vec<Line3>,
    pub texts: Vec<Text3>,
    /// Value labels of the bars that have one.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<MarkLabel3>,
}
