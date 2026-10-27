//! Layout of an extruded 3D pie or donut: spec -> [`Scene3d`] with one [`SliceMark`] per slice.
//!
//! The pie stands on the floor around the vertical axis through `(RADIUS, RADIUS)`, so the
//! plot box is `2 * RADIUS` square and as high as the pie is thick, and the usual orbit camera
//! frames it. There are no axes and no walls. Slices follow the colour field's order
//! clockwise seen from above, starting `startAngle` degrees clockwise from the far side.
//!
//! Labels are placed like the 2D ones (inside a slice when the text fits, else outside on a
//! short leader line), but the fit is judged at the *starting* camera, as for stacked bars:
//! a label is screen-sized while the pie is not. An outside label is centred on a point
//! beyond the leader's end (pushed out by half the text's size) rather than anchored to one
//! side of it, because which side is "outside" on screen depends on where the camera is.

use std::f64::consts::TAU;

use crate::Color;
use crate::Theme;
use crate::build_arc::{LEADER, TextBox, fits_inside, middle_radius};
use crate::framing_radius;
use crate::scale::{Ordinal, format_percent, format_value};
use crate::scene::{Anchor, RuleKind};
use crate::scene3d::{
    DEFAULT_DISTANCE, FOV_Y_DEGREES, LabelRole, Line3, MarkLabel3, Scene3d, SliceMark, Text3,
    VAlign,
};
use crate::spec::{ChartSpec, LabelShow};
use crate::text::TextMeasure;
use crate::validate::{ResolvedArc, arc_style};

/// The pie's outer radius in world units.
const RADIUS: f64 = 0.65;
const LABEL_SIZE: f64 = 12.0;
const VALUE_LABEL_GAP: f64 = 4.0;

pub(crate) fn build_pie_3d(
    spec: &ChartSpec,
    arc: &ResolvedArc,
    viewport_px: (u32, u32),
    dpr: f32,
    measure: &dyn TextMeasure,
    theme: &Theme,
) -> Scene3d {
    let s = f64::from(dpr).clamp(0.25, 16.0);
    let layer = &spec.layers[arc.index];
    let style = arc_style(layer);
    let legend = crate::legend::layout(
        spec,
        theme,
        (f64::from(viewport_px.0), f64::from(viewport_px.1)),
        s,
        crate::build::PADDING * s,
        measure,
    );
    let plot = legend
        .as_ref()
        .map(|l| crate::build3d::plot_area(viewport_px, l));
    let plot_h = plot.map_or(viewport_px.1 as f32, |p| p[3]);
    let label = layer.label.as_ref();
    let percent = label
        .and_then(|l| l.format.as_ref())
        .and_then(|f| f.percent)
        .unwrap_or(false);
    let decimals = label
        .and_then(|l| l.format.as_ref())
        .and_then(|f| f.decimals);

    let height = style.depth * RADIUS;
    let size = [(2.0 * RADIUS) as f32, height as f32, (2.0 * RADIUS) as f32];
    let mut scene = Scene3d {
        width: viewport_px.0,
        height: viewport_px.1,
        plot,
        overlay: legend.map(crate::build3d::overlay).unwrap_or_default(),
        background: theme.background,
        size,
        halo_px: s.round().max(1.0) as f32,
        boxes: Vec::new(),
        slices: Vec::new(),
        prisms: Vec::new(),
        decor: Vec::new(),
        ribbons: Vec::new(),
        shadows: Vec::new(),
        lines: Vec::new(),
        polylines: Vec::new(),
        points: Vec::new(),
        pick_radius: 0.0,
        texts: Vec::new(),
        labels: Vec::new(),
    };
    let cells = arc.cells();
    let total: f64 = cells.iter().map(|c| c.value).sum();
    if total <= 0.0 {
        return scene;
    }

    let ordinal = Ordinal::new(arc.dict, theme.palette.to_vec());
    let mut used_keys = std::collections::HashMap::<String, usize>::new();
    let mut angle = style.start;
    let mut colors: Vec<Color> = Vec::new();
    for cell in &cells {
        let (start, end) = (angle, angle + cell.value / total * TAU);
        angle = end;
        let name = &arc.dict[cell.series];
        let base = format!(
            "layer{}/{}",
            arc.index,
            arc.key_text(cell.row).unwrap_or_else(|| name.clone())
        );
        let n = used_keys.entry(base.clone()).or_insert(0);
        *n += 1;
        let key = if *n == 1 { base } else { format!("{base}#{n}") };
        let color = ordinal.color(name);
        colors.push(color);
        scene.slices.push(SliceMark {
            key,
            color,
            center: [RADIUS as f32, RADIUS as f32],
            inner: (style.inner * RADIUS) as f32,
            outer: RADIUS as f32,
            height: height as f32,
            start: start as f32,
            end: end as f32,
            pad: style.pad as f32,
            layer: arc.index,
            row: cell.row,
            sum: cell.sum(),
        });
    }

    // --- labels -----------------------------------------------------------------------------
    let Some(label) = label else { return scene };
    // How big the pie looks at the starting camera, to judge what fits.
    let distance = spec
        .camera
        .as_ref()
        .and_then(|c| c.distance)
        .unwrap_or(DEFAULT_DISTANCE)
        .clamp(0.5, 20.0);
    let half_fov = (FOV_Y_DEGREES / 2.0).to_radians().tan();
    let px_per_unit =
        f64::from(plot_h.max(1.0)) / (2.0 * distance * f64::from(framing_radius(size)) * half_fov);
    let label_size = (LABEL_SIZE * s) as f32;
    let cap = f64::from(measure.cap_height(label_size));
    let gap = VALUE_LABEL_GAP * s;
    let hair = s.round().max(1.0) as f32;
    let mut outside_kept: Vec<([f32; 3], f64)> = Vec::new();

    for (i, (cell, slice)) in cells.iter().zip(&scene.slices.clone()).enumerate() {
        let text = if percent {
            format_percent(cell.value / total, decimals)
        } else {
            format_value(cell.value, decimals)
        };
        let width = f64::from(measure.width(&text, label_size));
        let (a0, a1) = slice.span();
        let (a0, a1) = (f64::from(a0), f64::from(a1));
        let mid = (a0 + a1) / 2.0;
        let (inner, outer) = (f64::from(slice.inner), f64::from(slice.outer));
        let fits = fits_inside(
            TextBox { width, height: cap },
            (a0, a1),
            inner * px_per_unit,
            outer * px_per_unit,
            gap / 2.0,
        );
        let key = format!("{}/label", slice.key);
        let make = |anchor: [f32; 3], color: Color, halo: bool| Text3 {
            key: key.clone(),
            text: text.clone(),
            anchor,
            offset_px: [0.0, 0.0],
            size_px: label_size,
            color,
            align: Anchor::Middle,
            valign: VAlign::Middle,
            azimuths: None,
            role: LabelRole::Fixed,
            halo,
            flat: None,
            upright: None,
        };
        let top = height as f32;
        let t3 = if fits {
            let at = middle_radius(inner, outer);
            let color =
                if theme.text.contrast(slice.color) >= theme.background.contrast(slice.color) {
                    theme.text
                } else {
                    theme.background
                };
            make(slice.point(mid as f32, at as f32, top), color, false)
        } else {
            // Past a leader, centred on a point half the text's size beyond its end.
            let leader = LEADER * s / px_per_unit;
            let reach = outer + leader + (width.max(cap * 2.0) / 2.0 + gap) / px_per_unit;
            let anchor = slice.point(mid as f32, reach as f32, top);
            // Keep clear of the labels already placed.
            let crowded = outside_kept.iter().any(|(p, w)| {
                let d = ((p[0] - anchor[0]).powi(2) + (p[2] - anchor[2]).powi(2)).sqrt();
                f64::from(d) * px_per_unit < (w.max(width) * 0.8).max(cap * 2.2)
            });
            if label.show == LabelShow::Always {
                if crowded {
                    continue;
                }
                outside_kept.push((anchor, width));
                scene.lines.push(Line3 {
                    key: format!("{}/leader", slice.key),
                    kind: RuleKind::Axis,
                    a: slice.point(mid as f32, outer as f32, top),
                    b: slice.point(mid as f32, (outer + leader) as f32, top),
                    width_px: hair,
                    color: theme.axis,
                    azimuths: None,
                });
            }
            make(anchor, theme.text, true)
        };
        scene.labels.push(MarkLabel3 {
            mark: scene.boxes.len() + i,
            show: label.show,
            text: t3,
        });
    }
    scene
}
