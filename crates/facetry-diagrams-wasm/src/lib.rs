//! JS-facing API of the diagrams add-on: one function that turns Mermaid text into a chart
//! spec. The TypeScript wrapper (`facetry/diagrams` in `packages/facetry`) loads this module
//! only when a page asks for a diagram, and hands the spec to the core module to draw.

use facetry_core::spec::DiagramStyle;
use facetry_diagrams::{DiagramIssue, Options, from_mermaid_with};
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

/// Lays out Mermaid `source` in `style` (`"flat"`, `"glossy"` or `"isometric"`; absent: flat),
/// with a card beside each node if `cards` (isometric only), and returns the
/// chart spec as JSON text. Throws an `Error` named `FacetryError` with
/// `code: "invalid-diagram"` and a `diagramIssues` list when the text has problems (or uses
/// Mermaid features Facetry does not support), and `code: "invalid-options"` for an unknown
/// style; nothing is laid out then.
#[wasm_bindgen(js_name = layoutMermaid)]
pub fn layout_mermaid(
    source: &str,
    style: Option<String>,
    cards: Option<bool>,
) -> Result<String, JsValue> {
    let style = match style.as_deref() {
        None | Some("flat") => DiagramStyle::Flat,
        Some("glossy") => DiagramStyle::Glossy,
        Some("isometric") => DiagramStyle::Isometric,
        Some(other) => {
            let error = js_sys::Error::new(&format!(
                "`style` must be \"flat\", \"glossy\" or \"isometric\", got {other:?}"
            ));
            error.set_name("FacetryError");
            let _ = js_sys::Reflect::set(&error, &"code".into(), &"invalid-options".into());
            return Err(error.into());
        }
    };
    let cards = cards.unwrap_or(false);
    from_mermaid_with(source, &Options { style, cards })
        .map(|d| d.to_json())
        .map_err(|issues| invalid_diagram(&issues))
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
