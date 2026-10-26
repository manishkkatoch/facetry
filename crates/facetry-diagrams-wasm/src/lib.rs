//! JS-facing API of the diagrams add-on: one function that turns Mermaid text into a chart
//! spec. The TypeScript wrapper (`facetry/diagrams` in `packages/facetry`) loads this module
//! only when a page asks for a diagram, and hands the spec to the core module to draw.

use facetry_diagrams::{DiagramIssue, from_mermaid};
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

/// Lays out Mermaid `source` and returns the chart spec as JSON text. Throws an `Error` named
/// `FacetryError` with `code: "invalid-diagram"` and a `diagramIssues` list when the text has
/// problems (or uses Mermaid features Facetry does not support); nothing is laid out then.
#[wasm_bindgen(js_name = layoutMermaid)]
pub fn layout_mermaid(source: &str) -> Result<String, JsValue> {
    from_mermaid(source)
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
