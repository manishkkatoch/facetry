//! JS-facing API of the diagrams add-on: one function that turns Mermaid text into a chart
//! spec. The TypeScript wrapper (`facetry/diagrams` in `packages/facetry`) loads this module
//! only when a page asks for a diagram, and hands the spec to the core module to draw.

use facetry_core::spec::{CustomIcon, DiagramStyle, FillRule, IconFill};
use facetry_diagrams::{DiagramIssue, Options, chart_json};
use wasm_bindgen::prelude::*;

#[wasm_bindgen(typescript_custom_section)]
const TS_TYPES: &str = r#"
/** One problem with Mermaid text. */
export interface DiagramIssue {
    /** 1-based. */
    line: number;
    /** 1-based, in characters. */
    column: number;
    /** The source line the problem is on. */
    excerpt: string;
    message: string;
}
"#;

#[wasm_bindgen(start)]
fn start() {
    console_error_panic_hook::set_once();
}

/// Lays out Mermaid `source` in `style` (`"flat"`, `"glossy"`, `"isometric"` or `"schematic"`;
/// absent: flat), with a card beside each node if `cards` (isometric only), and returns the
/// chart spec as JSON text. The user's own icons come as flat lists, one entry per icon:
/// `icon_names`, `icon_strokes` (path data drawn as lines, empty for none), `icon_fill_counts`
/// (how many filled shapes it has) and `icon_boxes` (four numbers, its viewBox, or NaNs for the
/// default); and one entry per filled shape, icon after icon: `icon_fills` (path data) and
/// `icon_fill_rules` (1 for even-odd, 0 for nonzero). Throws an `Error` named `FacetryError` with `code: "invalid-diagram"` and a
/// `diagramIssues` list when the text has problems (or uses Mermaid features Facetry does not
/// support), and `code: "invalid-options"` for an unknown style or a broken icon; nothing is
/// laid out then.
#[wasm_bindgen(js_name = layoutMermaid)]
// Flat lists across the JS boundary keep the module free of a JSON reader.
#[allow(clippy::too_many_arguments)]
pub fn layout_mermaid(
    source: &str,
    style: Option<String>,
    cards: Option<bool>,
    icon_names: Option<Vec<String>>,
    icon_strokes: Option<Vec<String>>,
    icon_fill_counts: Option<Vec<u32>>,
    icon_fills: Option<Vec<String>>,
    icon_fill_rules: Option<Vec<u8>>,
    icon_boxes: Option<Vec<f64>>,
) -> Result<String, JsValue> {
    let style = match style.as_deref() {
        None | Some("flat") => DiagramStyle::Flat,
        Some("glossy") => DiagramStyle::Glossy,
        Some("isometric") => DiagramStyle::Isometric,
        Some("schematic") => DiagramStyle::Schematic,
        Some(other) => {
            return Err(invalid_options(&format!(
                "`style` must be \"flat\", \"glossy\", \"isometric\" or \"schematic\", got {other:?}"
            )));
        }
    };
    let cards = cards.unwrap_or(false);
    let strokes = icon_strokes.unwrap_or_default();
    let counts = icon_fill_counts.unwrap_or_default();
    let rules = icon_fill_rules.unwrap_or_default();
    let mut fills = icon_fills.unwrap_or_default().into_iter().enumerate();
    let boxes = icon_boxes.unwrap_or_default();
    let mut icons: Vec<(String, CustomIcon)> = Vec::new();
    for (i, name) in icon_names.unwrap_or_default().into_iter().enumerate() {
        let stroke = strokes.get(i).filter(|d| !d.is_empty()).cloned();
        let count = counts.get(i).copied().unwrap_or(0) as usize;
        let fill = fills
            .by_ref()
            .take(count)
            .map(|(k, d)| IconFill {
                d,
                fill_rule: if rules.get(k) == Some(&1) {
                    FillRule::EvenOdd
                } else {
                    FillRule::NonZero
                },
            })
            .collect();
        let view_box = boxes
            .get(4 * i..4 * i + 4)
            .filter(|b| !b.iter().any(|v| v.is_nan()))
            .map(|b| [b[0], b[1], b[2], b[3]]);
        if name.is_empty() || name.contains(['"', '\n']) {
            return Err(invalid_options(&format!(
                "icon name {name:?} must not be empty or hold a quote or a line break"
            )));
        }
        let icon = CustomIcon {
            stroke,
            fill,
            view_box,
        };
        if let Err((at, why)) = facetry_core::icons::check_custom(&icon) {
            let at = if at.is_empty() {
                String::new()
            } else {
                format!(", {}", at.join("/"))
            };
            return Err(invalid_options(&format!("icon `{name}`{at}: {why}")));
        }
        icons.retain(|(n, _)| *n != name);
        icons.push((name, icon));
    }
    chart_json(
        source,
        &Options {
            style,
            cards,
            icons,
        },
    )
    .map_err(|issues| invalid_diagram(&issues))
}

fn invalid_options(message: &str) -> JsValue {
    let error = js_sys::Error::new(message);
    error.set_name("FacetryError");
    let _ = js_sys::Reflect::set(&error, &"code".into(), &"invalid-options".into());
    error.into()
}

fn invalid_diagram(issues: &[DiagramIssue]) -> JsValue {
    let summary = match issues {
        [only] => format!("invalid diagram: {only}"),
        many => format!(
            "invalid diagram: {} issues, first: {}",
            many.len(),
            many.first().map(ToString::to_string).unwrap_or_default()
        ),
    };
    let error = js_sys::Error::new(&summary);
    error.set_name("FacetryError");
    let set = |o: &JsValue, key: &str, value: JsValue| {
        // Setting a plain property on a fresh object cannot fail.
        let _ = js_sys::Reflect::set(o, &key.into(), &value);
    };
    set(&error, "code", "invalid-diagram".into());
    let list = js_sys::Array::new();
    for issue in issues {
        let item: JsValue = js_sys::Object::new().into();
        set(&item, "line", issue.line.into());
        set(&item, "column", issue.column.into());
        set(&item, "excerpt", issue.excerpt.as_str().into());
        set(&item, "message", issue.message.as_str().into());
        list.push(&item);
    }
    set(&error, "diagramIssues", list.into());
    error.into()
}
