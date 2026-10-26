//! The laid-out diagram as a Facetry `ChartSpec`, in JSON: what the add-on hands to the core
//! module. Written by hand (no serde in the wasm), in a fixed key order, numbers to a tenth.

use facetry_core::spec::{DiagramSpec, EdgeEnd, EdgeStroke};

/// `{"dimension":"3d","diagram":{...}}`.
pub fn chart_spec_json(spec: &DiagramSpec) -> String {
    let mut out = String::with_capacity(256 + 160 * (spec.nodes.len() + spec.edges.len()));
    out.push_str("{\"dimension\":\"3d\",\"diagram\":{");
    if let Some(size) = spec.font_size {
        out.push_str("\"fontSize\":");
        number(&mut out, size);
        out.push(',');
    }
    out.push_str("\"nodes\":[");
    for (i, n) in spec.nodes.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str("{\"id\":");
        string(&mut out, &n.id);
        if let Some(label) = &n.label {
            out.push_str(",\"label\":");
            string(&mut out, label);
        }
        out.push_str(",\"shape\":");
        string(&mut out, n.shape.name());
        for (key, v) in [
            ("x", n.x),
            ("y", n.y),
            ("width", n.width),
            ("height", n.height),
        ] {
            out.push_str(",\"");
            out.push_str(key);
            out.push_str("\":");
            number(&mut out, v);
        }
        if let Some(c) = &n.color {
            out.push_str(",\"color\":");
            string(&mut out, c);
        }
        out.push('}');
    }
    out.push_str("],\"edges\":[");
    for (i, e) in spec.edges.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str("{\"from\":");
        string(&mut out, &e.from);
        out.push_str(",\"to\":");
        string(&mut out, &e.to);
        out.push_str(",\"points\":[");
        for (k, p) in e.points.iter().enumerate() {
            if k > 0 {
                out.push(',');
            }
            pair(&mut out, *p);
        }
        out.push(']');
        if let Some(label) = &e.label {
            out.push_str(",\"label\":");
            string(&mut out, label);
        }
        if let Some(p) = e.label_position {
            out.push_str(",\"labelPosition\":");
            pair(&mut out, p);
        }
        if e.stroke != EdgeStroke::Solid {
            out.push_str(",\"stroke\":");
            string(
                &mut out,
                match e.stroke {
                    EdgeStroke::Solid => "solid",
                    EdgeStroke::Dotted => "dotted",
                    EdgeStroke::Thick => "thick",
                },
            );
        }
        for (key, end) in [("head", e.head), ("tail", e.tail)] {
            if let Some(end) = end {
                out.push_str(",\"");
                out.push_str(key);
                out.push_str("\":");
                string(&mut out, end_name(end));
            }
        }
        out.push('}');
    }
    out.push_str("]}}");
    out
}

fn end_name(end: EdgeEnd) -> &'static str {
    match end {
        EdgeEnd::None => "none",
        EdgeEnd::Arrow => "arrow",
        EdgeEnd::Circle => "circle",
        EdgeEnd::Cross => "cross",
    }
}

fn pair(out: &mut String, p: [f64; 2]) {
    out.push('[');
    number(out, p[0]);
    out.push(',');
    number(out, p[1]);
    out.push(']');
}

/// A finite number to one decimal, without a trailing `.0`; anything else as 0.
fn number(out: &mut String, v: f64) {
    let v = if v.is_finite() {
        (v * 10.0).round() / 10.0
    } else {
        0.0
    };
    let v = if v == 0.0 { 0.0 } else { v }; // no "-0"
    let text = format!("{v:.1}");
    out.push_str(text.strip_suffix(".0").unwrap_or(&text));
}

fn string(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_and_strings() {
        let mut s = String::new();
        number(&mut s, 12.0);
        s.push(' ');
        number(&mut s, -0.04);
        s.push(' ');
        number(&mut s, 3.25);
        s.push(' ');
        string(&mut s, "a\"b\\c\nd\u{1}");
        assert_eq!(s, r#"12 0 3.3 "a\"b\\c\nd\u0001""#);
    }
}
