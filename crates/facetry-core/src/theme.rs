use crate::Color;

/// Visual defaults shared by every chart. M0 only needs the background.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Theme {
    pub background: Color,
}

impl Default for Theme {
    /// A dark slate background (`#12161c`).
    fn default() -> Self {
        Self {
            background: Color::from_srgb8(0x12, 0x16, 0x1c, 0xff),
        }
    }
}

impl Theme {
    /// The default theme with a different background.
    pub fn with_background(background: Color) -> Self {
        Self { background }
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
    fn with_background_overrides() {
        let red = Color::linear(1.0, 0.0, 0.0, 1.0);
        assert_eq!(Theme::with_background(red).background, red);
    }
}
