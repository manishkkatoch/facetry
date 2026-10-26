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
//!
//! **Hiding series.** An entry whose series is hidden (`ChartSpec::hidden_series`) stays in
//! the legend, with its swatch and text drawn at 40% opacity, so it can be
//! clicked back on. The legend also reports where each entry is ([`LegendHit`]), in the same
//! device pixels, so the host can find the entry under the pointer without any GPU work.

use crate::Color;
use crate::Theme;
use crate::scale::Ordinal;
use crate::scene::{Anchor, Bounds, LegendHit, Rotation, Rule, RuleKind, Text};
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
/// How much of its opacity a hidden series' entry keeps.
const HIDDEN_OPACITY: f32 = 0.4;

/// One legend entry.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Entry {
    pub label: String,
    pub color: Color,
    /// A line swatch (line layers) instead of a square, and how thick, in CSS px.
    pub line: Option<f64>,
    /// The series is hidden: the entry is drawn dimmed.
    pub hidden: bool,
}

/// What the legend asks of the chart's layout, in device pixels.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Legend {
    pub rules: Vec<Rule>,
    pub texts: Vec<Text>,
    /// Where each entry can be clicked; empty when the legend is not interactive.
    pub hits: Vec<LegendHit>,
    /// How much of each side of the canvas the legend takes, padding included.
    pub left: f64,
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
}

/// The categories of every layer that colours by a field, layer by layer: (categories, line
/// width when it is a line layer).
fn sources(spec: &ChartSpec) -> Vec<(Vec<String>, Option<f64>)> {
    let mut sources = Vec::new();
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
    sources
}

/// The names of the series: every colour category of every layer, once, in the order they
/// first appear (the legend's order). These are the names `legend.hidden` and
/// `Chart.setSeriesVisible` take.
pub fn series_names(spec: &ChartSpec) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for (dict, _) in sources(spec) {
        for c in dict {
            if !names.contains(&c) {
                names.push(c);
            }
        }
    }
    names
}

/// The legend's entries: the categories of every layer that colours by a field.
pub(crate) fn entries(spec: &ChartSpec, theme: &Theme) -> Vec<Entry> {
    let sources = sources(spec);
    // The shared colour scale: first appearance across layers, as `build::color_scale` does.
    // It is built from every category, hidden or not, so hiding never changes a colour.
    let mut domain: Vec<String> = Vec::new();
    for (dict, _) in &sources {
        for c in dict {
            if !domain.contains(c) {
                domain.push(c.clone());
            }
        }
    }
    let ordinal = Ordinal::new(&domain, theme.palette.to_vec());
    let hidden = spec.hidden_series();
    let mut out: Vec<Entry> = Vec::new();
    for (dict, line) in sources {
        for c in dict {
            if !out.iter().any(|e| e.label == c) {
                out.push(Entry {
                    color: ordinal.color(&c),
                    hidden: hidden.contains(&c),
                    label: c,
                    line,
                });
            }
        }
    }
    out
}

/// Settles which series are hidden for a spec the host has just received, and stores it in
/// `spec.hidden_override` (always `Some` afterwards).
///
/// `legend.hidden`, when the spec has it, is the answer. Otherwise `previous` (what the
/// viewer had hidden before an update) is kept for the series that still exist. Names that
/// are not a series are dropped, and so is a set that would hide every series (the new data
/// has nothing else to show, so everything is shown).
pub fn settle_hidden(spec: &mut ChartSpec, previous: Option<&[String]>) {
    let explicit = match &spec.legend {
        Some(LegendSpec::Config(c)) => c.hidden.clone(),
        _ => None,
    };
    let wanted = explicit.or_else(|| previous.map(<[String]>::to_vec));
    spec.hidden_override = None;
    let names = series_names(spec);
    let mut hidden: Vec<String> = names
        .iter()
        .filter(|n| wanted.as_ref().is_some_and(|w| w.contains(n)))
        .cloned()
        .collect();
    if !names.is_empty() && hidden.len() == names.len() {
        hidden.clear();
    }
    spec.hidden_override = Some(hidden);
}

/// Hides or shows one series of `spec` (in `spec.hidden_override`). Returns whether anything
/// changed: `false` for a name that is not a series, a series already in that state, and an
/// attempt to hide the last visible series (a blank chart helps nobody; the legend entry
/// stays clickable, so nothing is ever stuck).
pub fn set_series_visible(spec: &mut ChartSpec, name: &str, visible: bool) -> bool {
    let names = series_names(spec);
    if !names.iter().any(|n| n == name) {
        return false;
    }
    let mut hidden: Vec<String> = spec.hidden_series().to_vec();
    let is_hidden = hidden.iter().any(|h| h == name);
    if is_hidden == !visible {
        return false;
    }
    if visible {
        hidden.retain(|h| h != name);
    } else {
        let shown = names.iter().filter(|n| !hidden.contains(n)).count();
        if shown <= 1 {
            return false;
        }
        hidden.push(name.to_owned());
    }
    // In legend order, so the list does not depend on the order of the clicks.
    hidden = names.into_iter().filter(|n| hidden.contains(n)).collect();
    spec.hidden_override = Some(hidden);
    true
}

/// `color` at the opacity of a hidden entry.
fn dimmed(color: Color) -> Color {
    Color {
        a: color.a * HIDDEN_OPACITY,
        ..color
    }
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
    let mut hits = Vec::new();
    let interactive = spec.legend_interactive();
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
        let (entry, (text, entry_w)) = (&entries[i], &sized[i]);
        let fade = |c: Color| if entry.hidden { dimmed(c) } else { c };
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
            color: fade(entry.color),
        });
        texts.push(Text {
            key: format!("legend/label:{}", entry.label),
            text: text.clone(),
            x: (ex + sw + swatch_gap).round() as f32,
            y: (mid + cap / 2.0).round() as f32,
            size_px: label_size,
            color: fade(theme.text),
            anchor: Anchor::Start,
            rotation: Rotation::None,
        });
        if interactive {
            // The entry's whole row cell, a little generous so it is easy to hit: the
            // padding stays within the gaps between entries, so rectangles never overlap.
            let (pad_x, pad_y) = ((item_gap / 4.0).floor(), (row_gap / 2.0).floor());
            hits.push(LegendHit {
                series: entry.label.clone(),
                bounds: Bounds::new(
                    ex.round() - pad_x,
                    ey.round() - pad_y,
                    entry_w.ceil() + 2.0 * pad_x,
                    row_h + 2.0 * pad_y,
                ),
            });
        }
    }
    Some(Legend {
        rules,
        texts,
        hits,
        left,
        top,
        right,
        bottom,
    })
}
