//! The scene: everything to draw, already positioned, as flat lists. `build_scene` makes it,
//! the renderer consumes it, tests snapshot it. Geometry is in **device pixels**, origin
//! top-left, y down.
//!
//! Every item carries a `key` that identifies it across rebuilds (`layer0/Q1`,
//! `axis:y/tick:100`). Nothing uses keys yet; they are the hook for picking and transitions.

use serde::{Serialize, Serializer};

use crate::Color;
use crate::spec::LabelShow;

/// Snapshots round to 2 decimals so harmless float noise cannot churn them.
fn px<S: Serializer>(v: &f32, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_f64((f64::from(*v) * 100.0).round() / 100.0)
}

/// An axis-aligned box.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Bounds {
    #[serde(serialize_with = "px")]
    pub x: f32,
    #[serde(serialize_with = "px")]
    pub y: f32,
    #[serde(serialize_with = "px")]
    pub w: f32,
    #[serde(serialize_with = "px")]
    pub h: f32,
}

impl Bounds {
    pub fn new(x: f64, y: f64, w: f64, h: f64) -> Self {
        Self {
            x: x as f32,
            y: y as f32,
            w: w as f32,
            h: h as f32,
        }
    }
}

impl Bounds {
    /// Whether the point is inside (left and top edges count, right and bottom do not, so
    /// two bars that touch never both claim a pixel).
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }
}

/// A filled rectangle (a bar).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Rect {
    pub key: String,
    #[serde(flatten)]
    pub bounds: Bounds,
    pub color: Color,
    /// Where the bar came from: the layer index and the row of that layer's table. What
    /// hover and click events report; not part of the snapshot (the key already encodes it).
    #[serde(skip)]
    pub layer: usize,
    #[serde(skip)]
    pub row: usize,
    /// The summed value when the bar stands for several rows that were merged (`stack`,
    /// `normalize`, `group` add up rows with the same x and series). Hover and click report
    /// the first row's data with its y value replaced by this.
    #[serde(skip)]
    pub sum: Option<f64>,
}

/// What a rule is, which decides when it is drawn: gridlines go *behind* bars, axis lines
/// and ticks in front.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleKind {
    Grid,
    Axis,
    Tick,
}

/// A line. In M1 all lines are axis-aligned, so they are thin boxes (drawn like rects).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Rule {
    pub key: String,
    pub kind: RuleKind,
    #[serde(flatten)]
    pub bounds: Bounds,
    pub color: Color,
}

/// Which point of the text sits at the anchor position, along the reading direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Anchor {
    Start,
    Middle,
    End,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Rotation {
    None,
    /// Rotated 90 degrees counter-clockwise: reads bottom to top, glyph tops point left.
    Minus90,
}

/// A line of text. `(x, y)` is a point on the **baseline**, at the position along it
/// given by `anchor`. For rotated text the baseline is vertical, so `x` is the baseline.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Text {
    pub key: String,
    pub text: String,
    #[serde(serialize_with = "px")]
    pub x: f32,
    #[serde(serialize_with = "px")]
    pub y: f32,
    #[serde(serialize_with = "px")]
    pub size_px: f32,
    pub color: Color,
    pub anchor: Anchor,
    pub rotation: Rotation,
}

/// A value label that belongs to a bar. The text is laid out like any other, but whether it
/// is drawn can depend on the pointer, so it lives apart from [`Scene::texts`]: the renderer
/// draws `Always` labels with the rest and the `Hover` one of the hovered bar on top.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MarkLabel {
    /// Index into [`Scene::rects`].
    pub mark: usize,
    pub show: LabelShow,
    pub text: Text,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Scene {
    pub width: u32,
    pub height: u32,
    pub background: Color,
    /// Bars.
    pub rects: Vec<Rect>,
    /// Gridlines, axis lines and ticks, gridlines first.
    pub rules: Vec<Rule>,
    pub texts: Vec<Text>,
    /// Value labels of the bars that have one.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<MarkLabel>,
}

impl Scene {
    /// A blank scene with the default theme's background, for something to hold before the
    /// first layout.
    pub fn empty() -> Self {
        Self {
            width: 0,
            height: 0,
            background: crate::Theme::default().background,
            rects: Vec::new(),
            rules: Vec::new(),
            texts: Vec::new(),
            labels: Vec::new(),
        }
    }

    /// The bar under the pixel `(x, y)`, as an index into `rects`. Bars drawn later are on
    /// top, so the last one that contains the point wins.
    pub fn pick(&self, x: f32, y: f32) -> Option<usize> {
        self.rects.iter().rposition(|r| r.bounds.contains(x, y))
    }

    /// Back-to-front draw order: gridlines, bars, then axis lines and ticks. Text goes on top
    /// of everything and is drawn separately.
    pub fn boxes_in_draw_order(&self) -> impl Iterator<Item = (Bounds, Color)> + '_ {
        let grid = self.rules.iter().filter(|r| r.kind == RuleKind::Grid);
        let front = self.rules.iter().filter(|r| r.kind != RuleKind::Grid);
        grid.map(|r| (r.bounds, r.color))
            .chain(self.rects.iter().map(|r| (r.bounds, r.color)))
            .chain(front.map(|r| (r.bounds, r.color)))
    }
}
