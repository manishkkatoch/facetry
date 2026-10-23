//! The scene: everything to draw, already positioned, as flat lists. `build_scene` makes it,
//! the renderer consumes it, tests snapshot it. Geometry is in **device pixels**, origin
//! top-left, y down.
//!
//! Every item carries a `key` that identifies it across rebuilds (`layer0/Q1`,
//! `axis:y/tick:100`). Nothing uses keys yet; they are the hook for picking and transitions.

use serde::{Serialize, Serializer};

use crate::Color;

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

/// A filled rectangle (a bar).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Rect {
    pub key: String,
    #[serde(flatten)]
    pub bounds: Bounds,
    pub color: Color,
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
        }
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
