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
/// Looking at the plot box's exact centre leaves the bottom of the picture (axis titles hang
/// below the front edge) cramped and the top empty, so the camera looks this far (in world
/// units) below the centre, which lifts the box a little.
pub const FRAMING_LIFT: f64 = 0.12;

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

/// A solid extruded straight up from a convex outline on the floor: a diagram node, or (in
/// [`Scene3d::decor`]) an arrowhead. Heights `base..=top`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PrismMark {
    pub key: String,
    pub color: Color,
    /// The footprint, `[x, z]` world units: a convex polygon, in either winding.
    #[serde(serialize_with = "outline2")]
    pub outline: Vec<[f32; 2]>,
    #[serde(serialize_with = "w4")]
    pub base: f32,
    #[serde(serialize_with = "w4")]
    pub top: f32,
    /// The outline approximates a curve (a cylinder, a stadium's ends): its walls are shaded
    /// smoothly instead of facet by facet.
    pub smooth: bool,
    /// How shiny the surface is: 0 is matte (lit only by the diffuse light, like the bars), up
    /// to 1 for a strong specular highlight.
    #[serde(skip_serializing_if = "is_zero")]
    pub gloss: f32,
    /// The diagram node it draws (an index into `diagram.nodes`); what hover and click report.
    #[serde(skip)]
    pub node: usize,
    /// Stood up: the outline is drawn in a vertical plane instead of on the floor, and
    /// `base..=top` runs across that plane (see [`Stand`]). Closed at both ends.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stand: Option<Stand>,
}

/// Where a standing prism's outline is drawn: point `[u, w]` of the outline is at
/// `origin + u * right + w * up` (`right` a horizontal unit `[x, z]`), and the prism's heights
/// run along the plane's normal toward the viewer that `right` points to the right of, `[x, z]` =
/// `[-right.z, right.x]`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Stand {
    pub origin: [f32; 3],
    pub right: [f32; 2],
}

fn is_zero(v: &f32) -> bool {
    *v == 0.0
}

/// Flat triangles printed on a surface in one colour, unlit, like ink: a filled icon. Not a
/// mark: never picked.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FillMark {
    pub key: String,
    pub color: Color,
    /// World positions, three per triangle.
    #[serde(serialize_with = "points3")]
    pub triangles: Vec<[f32; 3]>,
}

fn points3<S: Serializer>(v: &[[f32; 3]], s: S) -> Result<S::Ok, S::Error> {
    let r = |x: f32| (f64::from(x) * 10_000.0).round() / 10_000.0;
    let rounded: Vec<(f64, f64, f64)> = v.iter().map(|p| (r(p[0]), r(p[1]), r(p[2]))).collect();
    rounded.serialize(s)
}

/// A thick edge lying on the floor: a band `width` wide along `path` (`[x, z]` world units),
/// heights `base..=top`, lit like the prisms. Not a mark: never picked.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RibbonMark {
    pub key: String,
    pub color: Color,
    #[serde(serialize_with = "outline2")]
    pub path: Vec<[f32; 2]>,
    #[serde(serialize_with = "w4")]
    pub width: f32,
    #[serde(serialize_with = "w4")]
    pub base: f32,
    #[serde(serialize_with = "w4")]
    pub top: f32,
    #[serde(skip_serializing_if = "is_zero")]
    pub gloss: f32,
}

/// A soft shadow on the floor: the convex `outline` (`[x, z]` world units) filled with
/// `color` (its alpha the darkness), fading to nothing over `blur` world units outside it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ShadowMark {
    #[serde(serialize_with = "outline2")]
    pub outline: Vec<[f32; 2]>,
    #[serde(serialize_with = "w4")]
    pub blur: f32,
    pub color: Color,
}

impl PrismMark {
    /// Whether the floor point `(x, z)` is inside the footprint.
    pub fn contains(&self, x: f32, z: f32) -> bool {
        let n = self.outline.len();
        if n < 3 {
            return false;
        }
        // Convex: inside when the point is on the same side of every edge.
        let mut sign = 0.0f32;
        for i in 0..n {
            let [ax, az] = self.outline[i];
            let [bx, bz] = self.outline[(i + 1) % n];
            let cross = (bx - ax) * (z - az) - (bz - az) * (x - ax);
            if cross.abs() < 1e-9 {
                continue;
            }
            if sign == 0.0 {
                sign = cross.signum();
            } else if cross.signum() != sign {
                return false;
            }
        }
        true
    }

    /// The middle of the outline (the average of its points), `[x, z]`.
    pub fn centre(&self) -> [f32; 2] {
        let n = self.outline.len().max(1) as f32;
        let (x, z) = self
            .outline
            .iter()
            .fold((0.0, 0.0), |(x, z), p| (x + p[0], z + p[1]));
        [x / n, z / n]
    }
}

fn outline2<S: Serializer>(v: &[[f32; 2]], s: S) -> Result<S::Ok, S::Error> {
    let r = |x: f32| (f64::from(x) * 10_000.0).round() / 10_000.0;
    let rounded: Vec<(f64, f64)> = v.iter().map(|p| (r(p[0]), r(p[1]))).collect();
    rounded.serialize(s)
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
    /// `Some(k)`: the text is printed flat on the horizontal plane through `anchor` instead of
    /// facing the viewer, reading along +x with its lines going down toward +z, `k` world
    /// units per pixel of `size_px` and `offset_px`. It is then hidden by what stands in front
    /// of it and scales with the zoom, like the surface it is printed on.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flat: Option<f32>,
    /// With `flat`: printed on the vertical plane through `anchor` instead, reading along the
    /// horizontal unit direction `[x, z]` with its lines going down; a sign standing in the
    /// scene.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upright: Option<[f32; 2]>,
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
    /// the diagram nodes, then the line points.
    pub fn mark_count(&self) -> usize {
        self.boxes.len() + self.slices.len() + self.prisms.len() + self.points.len()
    }

    /// Where the line points start in the mark index space.
    pub fn first_point(&self) -> usize {
        self.boxes.len() + self.slices.len() + self.prisms.len()
    }

    /// An empty scene of `viewport_px` on `background`, for builders to fill in.
    pub fn empty(viewport_px: (u32, u32), background: Color, size: [f32; 3]) -> Self {
        Self {
            width: viewport_px.0,
            height: viewport_px.1,
            plot: None,
            overlay: Overlay::default(),
            background,
            size,
            halo_px: 1.0,
            boxes: Vec::new(),
            slices: Vec::new(),
            prisms: Vec::new(),
            decor: Vec::new(),
            ribbons: Vec::new(),
            shadows: Vec::new(),
            fills: Vec::new(),
            lines: Vec::new(),
            polylines: Vec::new(),
            points: Vec::new(),
            pick_radius: 0.0,
            texts: Vec::new(),
            labels: Vec::new(),
        }
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
    /// A diagram's nodes, drawn and lit like the slices. Hover and click index them after the
    /// slices.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub prisms: Vec<PrismMark>,
    /// Solids that are not marks (a diagram's arrowheads): drawn like `prisms`, never picked.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub decor: Vec<PrismMark>,
    /// Thick edges on the floor (a glossy diagram's), drawn with the prisms; never picked.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub ribbons: Vec<RibbonMark>,
    /// Soft shadows on the floor, drawn translucent over it after the solids.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub shadows: Vec<ShadowMark>,
    /// Filled icons, printed on the surfaces they belong to.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub fills: Vec<FillMark>,
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
    pub texts: Vec<Text3>,
    /// Value labels of the bars that have one.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<MarkLabel3>,
}
