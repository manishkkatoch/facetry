//! The legend: one entry per colour category, laid out before the plot so the plot can use
//! what is left.
//!
//! **What is listed.** Every layer that colours by a field contributes its categories;
//! a category that several layers share is listed once (by value), with the swatch of the
//! first layer that has it: a short line for a line layer, a square for bars and slices.
//! Colours come from the same shared scale the marks use, so a swatch always matches.
//!
//! **Where it goes.** `right` and `left` legends are a column of entries (more columns when
//! the chart is too short for one); `top` and `bottom` legends are rows (more rows when the
//! chart is too narrow). The legend reserves a band of the canvas on its side, and the
//! chart (axes and plot, or the pie) is laid out in the rest. Long labels are cut with
//! `...` (the bundled font has plain ASCII only).
//!
//! The result is plain [`Rule`]s (swatches) and [`Text`]s in device pixels, so it is
//! snapshotted, drawn and exported like everything else, in 2D and in 3D (where it is an
//! overlay drawn after the 3D pass).

use crate::Color;
use crate::Theme;
use crate::scale::Ordinal;
use crate::scene::{Anchor, Bounds, Rotation, Rule, RuleKind, Text};
use crate::spec::{ChartSpec, LegendOrient, LegendSpec, Mark};
use crate::text::TextMeasure;
use crate::validate::{ColorSource, resolve_arc, resolve_layer};

// Sizes in CSS px.
const SWATCH: f64 = 10.0;
const LINE_SWATCH_WIDTH: f64 = 18.0;
/// Between a swatch and its label.
const SWATCH_GAP: f64 = 6.0;
/// Between entries side by side (top and bottom legends) and between columns.
const ITEM_GAP: f64 = 16.0;
/// Between rows of entries.
const ROW_GAP: f64 = 6.0;
/// Between the title and the entries below it (or the first entry beside it).
const TITLE_GAP: f64 = 8.0;
/// Longest label in a left or right legend, and in a top or bottom one.
const MAX_LABEL_SIDE: f64 = 150.0;
const MAX_LABEL_ROW: f64 = 240.0;
const LABEL_SIZE: f64 = 12.0;
const TITLE_SIZE: f64 = 13.0;
const ELLIPSIS: &str = "...";

/// One legend entry.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Entry {
    pub label: String,
    pub color: Color,
    /// A line swatch (line layers) instead of a square, and how thick, in CSS px.
    pub line: Option<f64>,
}

/// What the legend asks of the chart's layout, in device pixels.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Legend {
    pub rules: Vec<Rule>,
    pub texts: Vec<Text>,
    /// How much of each side of the canvas the legend takes, padding included.
    pub left: f64,
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
}

/// The legend's entries: the categories of every layer that colours by a field.
pub(crate) fn entries(spec: &ChartSpec, theme: &Theme) -> Vec<Entry> {
    // (categories, whether the layer is a line layer, line width) per layer, in layer order.
    let mut sources: Vec<(Vec<String>, Option<f64>)> = Vec::new();
    for i in 0..spec.layers.len() {
        let layer = &spec.layers[i];
        if layer.mark == Mark::Arc {
            if let Ok(arc) = resolve_arc(spec, i) {
                sources.push((arc.dict.to_vec(), None));
            }
        } else if let Ok(r) = resolve_layer(spec, i) {
            if let ColorSource::Field { dict, .. } = r.color {
                let line = (layer.mark == Mark::Line).then(|| {
                    layer
                        .style
                        .as_ref()
                        .and_then(|s| s.stroke_width)
                        .unwrap_or(crate::spec::DEFAULT_STROKE_WIDTH)
                });
                sources.push((dict.to_vec(), line));
            }
        }
    }
    // The shared colour scale: first appearance across layers, as `build::color_scale` does.
    let mut domain: Vec<String> = Vec::new();
    for (dict, _) in &sources {
        for c in dict {
            if !domain.contains(c) {
                domain.push(c.clone());
            }
        }
    }
    let ordinal = Ordinal::new(&domain, theme.palette.to_vec());
    let mut out: Vec<Entry> = Vec::new();
    for (dict, line) in sources {
        for c in dict {
            if !out.iter().any(|e| e.label == c) {
                out.push(Entry {
                    color: ordinal.color(&c),
                    label: c,
                    line,
                });
            }
        }
    }
    out
}

/// Cuts `text` so that it is at most `max` pixels wide, ending in `...` when it had to be cut.
fn truncate(text: &str, max: f64, size: f32, measure: &dyn TextMeasure) -> String {
    let width = |t: &str| f64::from(measure.width(t, size));
    if width(text) <= max {
        return text.to_owned();
    }
    let mut chars: Vec<char> = text.chars().collect();
    while !chars.is_empty() {
        chars.pop();
        let cut: String = chars.iter().collect::<String>() + ELLIPSIS;
        if width(&cut) <= max {
            return cut;
        }
    }
    ELLIPSIS.to_owned()
}

/// Lays the legend out for a canvas of `viewport` device pixels (`pad` is the canvas padding,
/// `s` the pixel ratio). `None` when the spec shows no legend.
pub(crate) fn layout(
    spec: &ChartSpec,
    theme: &Theme,
    viewport: (f64, f64),
    s: f64,
    pad: f64,
    measure: &dyn TextMeasure,
) -> Option<Legend> {
    let (orient, title) = match &spec.legend {
        Some(LegendSpec::Shown(false)) => return None,
        Some(LegendSpec::Config(c)) => (c.orient.unwrap_or_default(), c.title.as_deref()),
        _ => (LegendOrient::default(), None),
    };
    let entries = entries(spec, theme);
    if entries.is_empty() {
        return None;
    }
    let (w, h) = viewport;
    let label_size = (LABEL_SIZE * s) as f32;
    let title_size = (TITLE_SIZE * s) as f32;
    let (asc, desc) = (
        f64::from(measure.ascent(label_size)),
        f64::from(measure.descent(label_size)),
    );
    let cap = f64::from(measure.cap_height(label_size));
    // Whole pixels, so rows are evenly spaced after rounding.
    let row_h = (asc + desc).max(SWATCH * s).ceil();
    let (title_asc, title_desc) = (
        f64::from(measure.ascent(title_size)),
        f64::from(measure.descent(title_size)),
    );
    let title_h = title_asc + title_desc;
    let vertical = matches!(orient, LegendOrient::Right | LegendOrient::Left);
    let (item_gap, row_gap, swatch_gap) = (
        (ITEM_GAP * s).round(),
        (ROW_GAP * s).round(),
        (SWATCH_GAP * s).round(),
    );

    // Entry sizes: swatch, gap, (possibly cut) label.
    let room = if vertical { h } else { w } - 2.0 * pad;
    let max_label = if vertical {
        MAX_LABEL_SIDE
    } else {
        MAX_LABEL_ROW
    } * s;
    let swatch_w = |e: &Entry| e.line.map_or(SWATCH, |_| LINE_SWATCH_WIDTH) * s;
    let sized: Vec<(String, f64)> = entries
        .iter()
        .map(|e| {
            let max = max_label.min((room - swatch_w(e) - swatch_gap).max(8.0 * s));
            let text = truncate(&e.label, max, label_size, measure);
            let width = swatch_w(e) + swatch_gap + f64::from(measure.width(&text, label_size));
            (text, width)
        })
        .collect();
    let title_w = title.map_or(0.0, |t| f64::from(measure.width(t, title_size)));

    // Positions relative to the legend's top-left: (entry index, x, y of its row's top).
    let mut at: Vec<(usize, f64, f64)> = Vec::new();
    let (mut width, mut height) = (0.0_f64, 0.0_f64);
    if vertical {
        let head = if title.is_some() {
            title_h + TITLE_GAP * s
        } else {
            0.0
        };
        let per_column = (((room - head + row_gap) / (row_h + row_gap)).floor() as usize).max(1);
        let mut x = 0.0;
        for (c, column) in sized.chunks(per_column).enumerate() {
            let column_w = column.iter().map(|(_, w)| *w).fold(0.0, f64::max);
            for (r, _) in column.iter().enumerate() {
                at.push((c * per_column + r, x, head + r as f64 * (row_h + row_gap)));
            }
            height = height.max(
                head + column.len() as f64 * row_h
                    + column.len().saturating_sub(1) as f64 * row_gap,
            );
            x += column_w + item_gap;
            width = x - item_gap;
        }
        width = width.max(title_w);
    } else {
        // The title, if any, leads the first row; entries then flow and wrap.
        let lead = if title.is_some() {
            title_w + TITLE_GAP * s * 1.5
        } else {
            0.0
        };
        let (mut x, mut y) = (lead, 0.0);
        let mut first_in_row = true;
        for (i, (_, w_e)) in sized.iter().enumerate() {
            if !first_in_row && x + w_e > room {
                x = 0.0;
                y += row_h + row_gap;
            }
            at.push((i, x, y));
            first_in_row = false;
            x += w_e + item_gap;
            width = width.max(x - item_gap);
        }
        height = y + row_h;
    }
    let (width, height) = (width.min(room.max(1.0)), height);

    // Where the block sits on the canvas.
    let (x0, y0, left, top, right, bottom) = match orient {
        LegendOrient::Right => (w - pad - width, pad, 0.0, 0.0, pad + width, 0.0),
        LegendOrient::Left => (pad, pad, pad + width, 0.0, 0.0, 0.0),
        LegendOrient::Top => ((w - width) / 2.0, pad, 0.0, pad + height, 0.0, 0.0),
        LegendOrient::Bottom => (
            (w - width) / 2.0,
            h - pad - height,
            0.0,
            0.0,
            0.0,
            pad + height,
        ),
    };
    let (x0, y0) = (x0.round(), y0.round());

    let mut rules = Vec::new();
    let mut texts = Vec::new();
    if let Some(t) = title {
        let baseline = y0
            + title_asc
            + if vertical {
                0.0
            } else {
                (row_h - title_h) / 2.0
            };
        texts.push(Text {
            key: "legend/title".into(),
            text: t.to_owned(),
            x: x0 as f32,
            y: baseline.round() as f32,
            size_px: title_size,
            color: theme.text,
            anchor: Anchor::Start,
            rotation: Rotation::None,
        });
    }
    for (i, x, y) in at {
        let (entry, (text, _)) = (&entries[i], &sized[i]);
        let (ex, ey) = (x0 + x, y0 + y);
        let mid = ey + row_h / 2.0;
        let sw = swatch_w(entry);
        let bounds = match entry.line {
            Some(width) => {
                let thick = (width * s)
                    .clamp(s.round().max(1.0), 6.0 * s)
                    .round()
                    .max(1.0);
                Bounds::new(ex.round(), (mid - thick / 2.0).round(), sw.round(), thick)
            }
            None => Bounds::new(
                ex.round(),
                (mid - SWATCH * s / 2.0).round(),
                (SWATCH * s).round(),
                (SWATCH * s).round(),
            ),
        };
        rules.push(Rule {
            key: format!("legend/swatch:{}", entry.label),
            kind: RuleKind::Swatch,
            bounds,
            color: entry.color,
        });
        texts.push(Text {
            key: format!("legend/label:{}", entry.label),
            text: text.clone(),
            x: (ex + sw + swatch_gap).round() as f32,
            y: (mid + cap / 2.0).round() as f32,
            size_px: label_size,
            color: theme.text,
            anchor: Anchor::Start,
            rotation: Rotation::None,
        });
    }
    Some(Legend {
        rules,
        texts,
        left,
        top,
        right,
        bottom,
    })
}
