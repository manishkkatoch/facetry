use crate::Color;

/// Visual defaults shared by every chart.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Theme {
    pub background: Color,
    /// Axis titles.
    pub text: Color,
    /// Tick labels.
    pub text_muted: Color,
    /// Axis lines and tick marks.
    pub axis: Color,
    pub grid: Color,
    /// Categorical colours, assigned to categories in order and cycled when exhausted.
    pub palette: [Color; 8],
}

impl Default for Theme {
    /// The light theme. The palette is the first eight colours of Vega's `tableau10`,
    /// which stay distinguishable under the common kinds of colour blindness.
    fn default() -> Self {
        let rgb =
            |hex: u32| Color::from_srgb8((hex >> 16) as u8, (hex >> 8) as u8, hex as u8, 0xff);
        Self {
            background: rgb(0xffffff),
            text: rgb(0x1f2328),
            text_muted: rgb(0x57606a),
            axis: rgb(0x6e7781),
            grid: rgb(0xe1e4e8),
            palette: [
                rgb(0x4c78a8),
                rgb(0xf58518),
                rgb(0xe45756),
                rgb(0x72b7b2),
                rgb(0x54a24b),
                rgb(0xeeca3b),
                rgb(0xb279a2),
                rgb(0xff9da6),
            ],
        }
    }
}

impl Theme {
    /// The default theme with a different background.
    pub fn with_background(background: Color) -> Self {
        Self {
            background,
            ..Self::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_background_is_opaque() {
        assert_eq!(Theme::default().background.a, 1.0);
    }

    #[test]
    fn with_background_overrides_only_the_background() {
        let red = Color::linear(1.0, 0.0, 0.0, 1.0);
        let t = Theme::with_background(red);
        assert_eq!(t.background, red);
        assert_eq!(t.palette, Theme::default().palette);
    }

    #[test]
    fn palette_has_distinct_colours() {
        let p = Theme::default().palette;
        for i in 0..p.len() {
            for j in i + 1..p.len() {
                assert_ne!(p[i], p[j]);
            }
        }
    }
}
