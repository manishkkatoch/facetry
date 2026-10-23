use std::fmt;

/// An RGBA colour with **linear** (not gamma-encoded) `f32` channels in `0.0..=1.0`.
///
/// GPUs blend and shade in linear space; user-facing colours (CSS hex) are sRGB,
/// so conversion happens at the edges: [`Color::from_hex`] in, [`Color::to_srgb`] out.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

/// Why a colour string could not be parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseColorError(String);

impl fmt::Display for ParseColorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "invalid colour {:?}: expected #rrggbb or #rrggbbaa",
            self.0
        )
    }
}

impl std::error::Error for ParseColorError {}

fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_to_srgb(c: f32) -> f32 {
    if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

impl Color {
    /// Builds a colour from linear channels.
    pub const fn linear(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
    }

    /// Builds a colour from 8-bit sRGB channels; alpha is already linear.
    pub fn from_srgb8(r: u8, g: u8, b: u8, a: u8) -> Self {
        let ch = |v: u8| srgb_to_linear(f32::from(v) / 255.0);
        Self::linear(ch(r), ch(g), ch(b), f32::from(a) / 255.0)
    }

    /// Parses `#rrggbb` or `#rrggbbaa` (sRGB).
    pub fn from_hex(s: &str) -> Result<Self, ParseColorError> {
        let err = || ParseColorError(s.to_owned());
        let digits = s.strip_prefix('#').ok_or_else(err)?;
        if !matches!(digits.len(), 6 | 8) || !digits.is_ascii() {
            return Err(err());
        }
        let byte = |i: usize| u8::from_str_radix(&digits[i..i + 2], 16).map_err(|_| err());
        let a = if digits.len() == 8 { byte(6)? } else { 255 };
        Ok(Self::from_srgb8(byte(0)?, byte(2)?, byte(4)?, a))
    }

    /// The colour with gamma-encoded RGB channels, for non-sRGB render targets.
    pub fn to_srgb(self) -> [f32; 4] {
        [
            linear_to_srgb(self.r),
            linear_to_srgb(self.g),
            linear_to_srgb(self.b),
            self.a,
        ]
    }

    /// The channels as `[r, g, b, a]` (linear).
    pub const fn to_array(self) -> [f32; 4] {
        [self.r, self.g, self.b, self.a]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn hex_endpoints() {
        assert_eq!(
            Color::from_hex("#000000").unwrap(),
            Color::linear(0.0, 0.0, 0.0, 1.0)
        );
        assert_eq!(
            Color::from_hex("#ffffff").unwrap(),
            Color::linear(1.0, 1.0, 1.0, 1.0)
        );
    }

    #[test]
    fn hex_alpha_is_not_gamma_encoded() {
        let c = Color::from_hex("#ff000080").unwrap();
        assert!(close(c.a, 128.0 / 255.0));
    }

    #[test]
    fn mid_grey_is_darker_in_linear_space() {
        // sRGB 0x80 is about 21.6% linear.
        let c = Color::from_hex("#808080").unwrap();
        assert!(close(c.r, 0.2158));
    }

    #[test]
    fn srgb_round_trip() {
        let c = Color::from_srgb8(51, 102, 153, 255);
        let [r, g, b, a] = c.to_srgb();
        assert!(close(r, 51.0 / 255.0) && close(g, 102.0 / 255.0) && close(b, 153.0 / 255.0));
        assert_eq!(a, 1.0);
    }

    #[test]
    fn rejects_malformed() {
        for bad in [
            "",
            "#",
            "336699",
            "#12345",
            "#gggggg",
            "#33669",
            "#336699ff0",
            "#é23456",
        ] {
            assert!(Color::from_hex(bad).is_err(), "{bad:?} should fail");
        }
    }
}
