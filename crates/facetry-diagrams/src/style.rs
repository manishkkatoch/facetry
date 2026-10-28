//! Mermaid styling: the property lists of `classDef` and `style` statements
//! (`fill:#f96,stroke:#333,stroke-width:2px,color:#fff`).

/// The look a node asks for. Colours are normalised to `#rrggbb` (or `#rrggbbaa`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NodeStyle {
    pub fill: Option<String>,
    pub stroke: Option<String>,
    /// CSS px.
    pub stroke_width: Option<f64>,
    /// The label's colour.
    pub color: Option<String>,
}

impl NodeStyle {
    /// `other`'s properties over these.
    pub fn apply(&mut self, other: &NodeStyle) {
        if other.fill.is_some() {
            self.fill.clone_from(&other.fill);
        }
        if other.stroke.is_some() {
            self.stroke.clone_from(&other.stroke);
        }
        if other.stroke_width.is_some() {
            self.stroke_width = other.stroke_width;
        }
        if other.color.is_some() {
            self.color.clone_from(&other.color);
        }
    }
}

/// CSS colour names Mermaid diagrams commonly use, with their `#rrggbb`.
const NAMED: &[(&str, &str)] = &[
    ("black", "#000000"),
    ("white", "#ffffff"),
    ("red", "#ff0000"),
    ("green", "#008000"),
    ("blue", "#0000ff"),
    ("yellow", "#ffff00"),
    ("orange", "#ffa500"),
    ("purple", "#800080"),
    ("pink", "#ffc0cb"),
    ("gray", "#808080"),
    ("grey", "#808080"),
    ("lightgray", "#d3d3d3"),
    ("lightgrey", "#d3d3d3"),
    ("darkgray", "#a9a9a9"),
    ("darkgrey", "#a9a9a9"),
    ("brown", "#a52a2a"),
    ("cyan", "#00ffff"),
    ("magenta", "#ff00ff"),
    ("lime", "#00ff00"),
    ("navy", "#000080"),
    ("teal", "#008080"),
    ("maroon", "#800000"),
    ("olive", "#808000"),
    ("silver", "#c0c0c0"),
    ("gold", "#ffd700"),
    ("coral", "#ff7f50"),
    ("salmon", "#fa8072"),
    ("tomato", "#ff6347"),
    ("crimson", "#dc143c"),
    ("indigo", "#4b0082"),
    ("violet", "#ee82ee"),
    ("lightblue", "#add8e6"),
    ("lightgreen", "#90ee90"),
    ("lightyellow", "#ffffe0"),
    ("darkblue", "#00008b"),
    ("darkgreen", "#006400"),
    ("darkred", "#8b0000"),
    ("skyblue", "#87ceeb"),
    ("steelblue", "#4682b4"),
    ("royalblue", "#4169e1"),
    ("dodgerblue", "#1e90ff"),
    ("lavender", "#e6e6fa"),
    ("beige", "#f5f5dc"),
    ("khaki", "#f0e68c"),
    ("turquoise", "#40e0d0"),
];

/// A CSS colour: `#rgb`, `#rgba`, `#rrggbb`, `#rrggbbaa` or one of [`NAMED`].
pub fn color(value: &str) -> Result<String, String> {
    let v = value.trim();
    if let Some(hex) = v.strip_prefix('#') {
        if hex.chars().all(|c| c.is_ascii_hexdigit()) {
            match hex.len() {
                3 | 4 => {
                    let doubled: String = hex.chars().flat_map(|c| [c, c]).collect();
                    return Ok(format!("#{}", doubled.to_ascii_lowercase()));
                }
                6 | 8 => return Ok(format!("#{}", hex.to_ascii_lowercase())),
                _ => {}
            }
        }
        return Err(format!("`{v}` is not a colour: use `#rgb` or `#rrggbb`"));
    }
    let lower = v.to_ascii_lowercase();
    NAMED
        .iter()
        .find(|(n, _)| *n == lower)
        .map(|(_, hex)| (*hex).to_owned())
        .ok_or_else(|| {
            format!("`{v}` is not a colour Facetry knows: use `#rgb`, `#rrggbb` or a common CSS colour name")
        })
}

/// A CSS length in px (`2px`, `1.5px` or `2`). Read digit by digit: `str::parse::<f64>` would
/// link the standard library's general float parser into the wasm for this one use.
fn px(value: &str) -> Result<f64, String> {
    let v = value.trim();
    let bad = || format!("`{v}` is not a stroke width: use a number of px from 0 to 20");
    let n = v.strip_suffix("px").unwrap_or(v).trim();
    let (whole, fraction) = n.split_once('.').unwrap_or((n, ""));
    if whole.is_empty() && fraction.is_empty() {
        return Err(bad());
    }
    let digits = |s: &str| s.chars().all(|c| c.is_ascii_digit()) && s.len() <= 6;
    if !digits(whole) || !digits(fraction) {
        return Err(bad());
    }
    let mut out = 0.0;
    for c in whole.chars() {
        out = out * 10.0 + f64::from(c as u8 - b'0');
    }
    let mut scale = 0.1;
    for c in fraction.chars() {
        out += f64::from(c as u8 - b'0') * scale;
        scale /= 10.0;
    }
    if out <= 20.0 { Ok(out) } else { Err(bad()) }
}

/// Parses `key:value,key:value`. Each problem is reported with its character offset within
/// `text`.
pub fn parse(text: &str) -> Result<NodeStyle, (usize, String)> {
    parse_for(text, false)
}

/// [`parse`] for a link (`linkStyle`): `fill` may only be `none` (links have no inside; Mermaid's
/// examples write it), which changes nothing.
pub fn parse_link(text: &str) -> Result<NodeStyle, (usize, String)> {
    parse_for(text, true)
}

fn parse_for(text: &str, link: bool) -> Result<NodeStyle, (usize, String)> {
    let mut style = NodeStyle::default();
    let mut offset = 0;
    for part in text.split(',') {
        let at = offset + (part.len() - part.trim_start().len());
        offset += part.len() + 1;
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let Some((key, value)) = part.split_once(':') else {
            return Err((at, format!("expected `property:value`, found `{part}`")));
        };
        let at_value = at + key.len() + 1;
        let located = |m: String| (at_value, m);
        match key.trim() {
            "fill" if link => {
                if value.trim() != "none" {
                    return Err(located(
                        "a link has no fill: only `fill:none` is allowed".into(),
                    ));
                }
            }
            "fill" => style.fill = Some(color(value).map_err(located)?),
            "stroke" => style.stroke = Some(color(value).map_err(located)?),
            "stroke-width" => style.stroke_width = Some(px(value).map_err(located)?),
            "color" => style.color = Some(color(value).map_err(located)?),
            other => {
                return Err((
                    at,
                    format!(
                        "`{other}` is not supported in styles (only `fill`, `stroke`, `stroke-width` and `color`)"
                    ),
                ));
            }
        }
    }
    Ok(style)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn properties_and_colours() {
        let s = parse("fill:#f96,stroke:#333333, stroke-width:4px,color:White").unwrap();
        assert_eq!(s.fill.as_deref(), Some("#ff9966"));
        assert_eq!(s.stroke.as_deref(), Some("#333333"));
        assert_eq!(s.stroke_width, Some(4.0));
        assert_eq!(s.color.as_deref(), Some("#ffffff"));
    }

    #[test]
    fn problems_name_the_property_and_where_it_is() {
        let (at, m) = parse("fill:#f96,stroke-dasharray: 5 5").unwrap_err();
        assert_eq!(at, 10);
        assert!(m.contains("`stroke-dasharray` is not supported"));
        let (at, m) = parse("fill:#ggg").unwrap_err();
        assert_eq!(at, 5);
        assert!(m.contains("not a colour"));
        assert!(
            parse("fill:chartreuse")
                .unwrap_err()
                .1
                .contains("colour Facetry knows")
        );
        assert!(parse("stroke-width:99px").is_err());
        assert_eq!(parse("stroke-width:1.5px").unwrap().stroke_width, Some(1.5));
        assert!(parse("stroke-width:-1px").is_err());
        assert!(parse("stroke-width:px").is_err());
        assert!(parse("fill").is_err());
    }

    #[test]
    fn later_styles_win() {
        let mut a = parse("fill:#111,stroke:#222").unwrap();
        a.apply(&parse("fill:#333").unwrap());
        assert_eq!(a.fill.as_deref(), Some("#333333"));
        assert_eq!(a.stroke.as_deref(), Some("#222222"));
    }
}
