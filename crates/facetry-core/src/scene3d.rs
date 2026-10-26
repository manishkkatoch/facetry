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
use crate::scene::{Anchor, AxisMap, Base, RuleKind, TickRef};
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
    /// What a transition grows the box from (and shrinks it to): see [`crate::scene::Base`];
    /// the axis is one of [`Scene3d::axes`] and its pixels are world heights.
    #[serde(skip)]
    pub base: Option<Base>,
}

/// One slice of an extruded pie. The pie stands on the floor with its axis along `y`: the
/// slice covers radii `inner..=outer` around `center` (`[x, z]`) and heights `0..=height`.
/// Angles are radians, clockwise seen from above, from the far side (the -z direction,
/// 12 o'clock when looking down from the default camera).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SliceMark {
    pub key: String,
    pub color: Color,
    #[serde(serialize_with = "w2")]
    pub center: [f32; 2],
    #[serde(serialize_with = "w4")]
    pub inner: f32,
    #[serde(serialize_with = "w4")]
    pub outer: f32,
    #[serde(serialize_with = "w4")]
    pub height: f32,
    #[serde(serialize_with = "w4")]
    pub start: f32,
    #[serde(serialize_with = "w4")]
    pub end: f32,
    #[serde(serialize_with = "w4")]
    pub pad: f32,
    #[serde(skip)]
    pub layer: usize,
    #[serde(skip)]
    pub row: usize,
    #[serde(skip)]
    pub sum: Option<f64>,
}

fn w4<S: Serializer>(v: &f32, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_f64((f64::from(*v) * 10_000.0).round() / 10_000.0)
}

fn w2<S: Serializer>(v: &[f32; 2], s: S) -> Result<S::Ok, S::Error> {
    let r = |x: f32| (f64::from(x) * 10_000.0).round() / 10_000.0;
    (r(v[0]), r(v[1])).serialize(s)
}

impl SliceMark {
    /// The angles the slice really covers (padding cut off): see
    /// [`crate::scene::ArcMark::span`].
    pub fn span(&self) -> (f32, f32) {
        let sweep = self.end - self.start;
        if sweep >= std::f32::consts::TAU - 1e-4 {
            return (self.start, self.end);
        }
        let cut = (self.pad / 2.0).min(sweep / 4.0);
        (self.start + cut, self.end - cut)
    }

    /// Whether the floor point `(x, z)` is under the slice (the top face's footprint).
    pub fn contains(&self, x: f32, z: f32) -> bool {
        let (a0, a1) = self.span();
        crate::scene::sector_contains(
            x - self.center[0],
            self.center[1] - z,
            self.inner,
            self.outer,
            a0,
            a1,
        )
    }

    /// The world point at angle `angle`, radius `r` and height `y`.
    pub fn point(&self, angle: f32, r: f32, y: f32) -> [f32; 3] {
        [
            self.center[0] + r * angle.sin(),
            y,
            self.center[1] - r * angle.cos(),
        ]
    }
}

/// A line layer's series in 3D: runs of connected points in world space (a missing value ends
/// a run). It is drawn as thick screen-space segments with round joins, like the grid lines
/// but depth-tested against the bars, so the width is `width_px` at any distance and angle.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Polyline3 {
    pub key: String,
    pub color: Color,
    /// Device pixels.
    #[serde(serialize_with = "px")]
    pub width_px: f32,
    #[serde(serialize_with = "runs3")]
    pub runs: Vec<Vec<[f32; 3]>>,
    /// `style.area`: the world height of the baseline the translucent curtain under the line
    /// reaches down (or up) to, drawn at [`AREA_ALPHA`] of the colour.
    #[serde(skip_serializing_if = "Option::is_none", serialize_with = "opt_w4")]
    pub area: Option<f32>,
    #[serde(skip)]
    pub layer: usize,
    /// How much of the line is drawn, from its start (see [`crate::scene::Polyline::reveal`]).
    #[serde(skip)]
    pub reveal: f32,
    /// The key of the data point at each vertex, in the order of `runs` flattened.
    #[serde(skip)]
    pub keys: Vec<String>,
}

/// Opacity of the curtain under a 3D line (`style.area`).
pub const AREA_ALPHA: f32 = 0.28;

fn runs3<S: Serializer>(v: &[Vec<[f32; 3]>], s: S) -> Result<S::Ok, S::Error> {
    let r = |x: f32| (f64::from(x) * 10_000.0).round() / 10_000.0;
    let rounded: Vec<Vec<(f64, f64, f64)>> = v
        .iter()
        .map(|run| run.iter().map(|p| (r(p[0]), r(p[1]), r(p[2]))).collect())
        .collect();
    rounded.serialize(s)
}

fn opt_w4<S: Serializer>(v: &Option<f32>, s: S) -> Result<S::Ok, S::Error> {
    v.map(|x| (f64::from(x) * 10_000.0).round() / 10_000.0)
        .serialize(s)
}

/// A data point of a 3D line layer, anchored in the world. Every finite point exists (so it can
/// be hovered and clicked) but only those of a layer with `point` markers are drawn, as discs
/// that face the viewer.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PointMark3 {
    pub key: String,
    #[serde(serialize_with = "w4_3")]
    pub pos: [f32; 3],
    pub color: Color,
    /// Marker diameter in device pixels (also the size the hover marker grows from).
    #[serde(serialize_with = "px")]
    pub size_px: f32,
    pub visible: bool,
    #[serde(skip)]
    pub layer: usize,
    #[serde(skip)]
    pub row: usize,
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
    /// For a tick or gridline of the value axis: which value it marks (see [`TickRef`]).
    #[serde(skip)]
    pub tick: Option<TickRef>,
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
    /// Printed on the middle of box `mark` (a stacked segment's value): drawn only while the
    /// camera can see that point, that is, no other box is in front of it.
    OnMark(u32),
    /// Printed by a line's data point: drawn only while no box is in front of the point.
    OnPoint,
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
    /// Draw an outline in the background colour round the letters (the default). Labels that
    /// sit on a bar or slice, in a colour chosen to contrast with it, have none: an outline
    /// of the page colour would swallow letters of the same colour.
    #[serde(skip_serializing_if = "is_true")]
    pub halo: bool,
    /// For the label of a tick of the value axis: which value it marks.
    #[serde(skip)]
    pub tick: Option<TickRef>,
}

fn is_true(v: &bool) -> bool {
    *v
}

/// A 2D layer on top of the 3D picture.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Overlay {
    pub rules: Vec<crate::scene::Rule>,
    pub texts: Vec<crate::scene::Text>,
    /// Where the legend's entries can be clicked (canvas pixels).
    #[serde(skip)]
    pub hits: Vec<crate::scene::LegendHit>,
}

impl Overlay {
    pub fn is_empty(&self) -> bool {
        self.rules.is_empty() && self.texts.is_empty()
    }
}

fn plot_rect<S: Serializer>(v: &Option<[f32; 4]>, s: S) -> Result<S::Ok, S::Error> {
    let r = |x: f32| (f64::from(x) * 100.0).round() / 100.0;
    v.map(|v| (r(v[0]), r(v[1]), r(v[2]), r(v[3]))).serialize(s)
}

impl Scene3d {
    /// The scalar fields of the scene and nothing else (every list empty): the canvas, the box,
    /// the background, the sizes.
    pub fn clone_shell(&self) -> Scene3d {
        Scene3d {
            width: self.width,
            height: self.height,
            plot: self.plot,
            overlay: Overlay::default(),
            background: self.background,
            size: self.size,
            halo_px: self.halo_px,
            boxes: Vec::new(),
            slices: Vec::new(),
            lines: Vec::new(),
            polylines: Vec::new(),
            points: Vec::new(),
            pick_radius: self.pick_radius,
            axes: Vec::new(),
            texts: Vec::new(),
            labels: Vec::new(),
        }
    }

    /// The series whose legend entry is under the canvas pixel `(x, y)`.
    pub fn legend_pick(&self, x: f32, y: f32) -> Option<&str> {
        crate::scene::legend_pick(&self.overlay.hits, x, y).map(|h| h.series.as_str())
    }

    /// The middle of the legend entry of `series`, in canvas pixels.
    pub fn legend_center(&self, series: &str) -> Option<[f32; 2]> {
        let b = self
            .overlay
            .hits
            .iter()
            .find(|h| h.series == series)?
            .bounds;
        Some([b.x + b.w / 2.0, b.y + b.h / 2.0])
    }

    /// Marks that hover and click can report, in one index space: the boxes, the pie slices,
    /// then the line points.
    pub fn mark_count(&self) -> usize {
        self.boxes.len() + self.slices.len() + self.points.len()
    }

    /// The part of the canvas the plot is drawn in: `[x, y, width, height]`.
    pub fn plot_rect(&self) -> [f32; 4] {
        self.plot
            .unwrap_or([0.0, 0.0, self.width as f32, self.height as f32])
    }
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
    /// The part of the canvas the 3D plot is drawn in, `[x, y, width, height]` in device
    /// pixels: `None` means all of it. A legend takes a band of one side, and the plot box is
    /// framed in what is left so it never sits under the legend.
    #[serde(skip_serializing_if = "Option::is_none", serialize_with = "plot_rect")]
    pub plot: Option<[f32; 4]>,
    /// Screen-space items drawn over the finished 3D picture with the 2D pipelines: the
    /// legend. Positions are canvas pixels, independent of the camera.
    #[serde(skip_serializing_if = "Overlay::is_empty")]
    pub overlay: Overlay,
    pub background: Color,
    /// The plot box's extent: `[x, 1, z]`; it starts at the origin.
    #[serde(serialize_with = "w4_3")]
    pub size: [f32; 3],
    /// Labels get an outline of this width (device pixels) in the background colour, so
    /// they stay readable where a bar passes behind them.
    #[serde(serialize_with = "px")]
    pub halo_px: f32,
    pub boxes: Vec<BoxMark>,
    /// The slices of an extruded pie. Hover and click index the marks as boxes first, then
    /// slices.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub slices: Vec<SliceMark>,
    /// Gridlines on the back walls and floor, then axis edges and ticks.
    pub lines: Vec<Line3>,
    /// Line layers' series, after the boxes and slices in draw order.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub polylines: Vec<Polyline3>,
    /// Line layers' data points. Hover and click index the marks as boxes, then slices, then
    /// points.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub points: Vec<PointMark3>,
    /// How near (device pixels) the pointer must be to a point's projection to pick it.
    #[serde(skip)]
    pub pick_radius: f32,
    /// The value axis as a mapping to world units (`vertical`: the y axis, whose range is the
    /// box's height `0..1`), for transitions.
    #[serde(skip)]
    pub axes: Vec<AxisMap>,
    pub texts: Vec<Text3>,
    /// Value labels of the bars that have one.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<MarkLabel3>,
}
