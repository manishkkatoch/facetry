//! The bundled UI font (Inter Regular, SIL OFL) as a multi-channel signed distance field
//! (MSDF) atlas, plus the metrics needed to lay text out.
//!
//! Both files are generated offline by `tools/font-atlas` (see the README) and committed,
//! so neither the build nor the browser ever rasterises a font. This crate is pure data
//! and arithmetic: it needs no GPU, so layout code and its tests can use the real metrics.

use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

/// The atlas image: an 8-bit RGB PNG whose three channels are the MSDF.
pub const ATLAS_PNG: &[u8] = include_bytes!("../assets/inter-regular-msdf.png");
const METRICS_JSON: &str = include_str!("../assets/inter-regular-metrics.json");

/// Where the atlas came from and how to read distances out of it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AtlasInfo {
    pub width: u32,
    pub height: u32,
    /// Atlas pixels per em: a glyph drawn at `size` screen px is scaled by `size / em_px`.
    pub em_px: f32,
    /// Total distance range encoded, in atlas pixels. A channel value of 0.5 is the glyph
    /// outline, 0 is `px_range / 2` pixels outside it, 1 is that far inside.
    pub px_range: f32,
}

/// One glyph. Lengths in `advance`/`plane` are in em (1.0 = the font size), y pointing up.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Glyph {
    #[serde(rename = "char")]
    pub ch: char,
    /// Horizontal distance to the next pen position.
    pub advance: f32,
    /// Quad to draw, `[left, bottom, right, top]` relative to the pen on the baseline.
    /// Includes the padding that holds the distance field outside the outline.
    /// `None` for glyphs with no outline (space).
    pub plane: Option<[f32; 4]>,
    /// Where that quad lives in the atlas, `[x, y, width, height]` in pixels, y down.
    pub atlas: Option<[u32; 4]>,
}

/// Font-wide vertical metrics (em, y up) and every glyph, sorted by character.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FontMetrics {
    pub family: String,
    pub atlas: AtlasInfo,
    pub ascender: f32,
    /// Negative: below the baseline.
    pub descender: f32,
    pub cap_height: f32,
    pub glyphs: Vec<Glyph>,
}

impl FontMetrics {
    /// The bundled font's metrics, parsed once.
    pub fn inter() -> &'static FontMetrics {
        static METRICS: OnceLock<FontMetrics> = OnceLock::new();
        METRICS.get_or_init(|| {
            // The JSON is embedded at compile time and checked by a unit test.
            serde_json::from_str(METRICS_JSON).expect("embedded font metrics are valid")
        })
    }

    /// The glyph for `c`, or the `?` glyph when the font (printable ASCII only) lacks it.
    pub fn glyph(&self, c: char) -> &Glyph {
        let find = |c: char| {
            self.glyphs
                .binary_search_by_key(&c, |g| g.ch)
                .ok()
                .map(|i| &self.glyphs[i])
        };
        // `?` is always present in the generated set.
        find(c).or_else(|| find('?')).unwrap_or(&self.glyphs[0])
    }

    /// Width of `text` at `size_px`: the sum of advances (no kerning in M1).
    pub fn text_width(&self, text: &str, size_px: f32) -> f32 {
        text.chars().map(|c| self.glyph(c).advance).sum::<f32>() * size_px
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metrics_cover_printable_ascii() {
        let m = FontMetrics::inter();
        for c in ' '..='~' {
            assert_eq!(m.glyph(c).ch, c, "missing {c:?}");
        }
        assert!(m.glyphs.windows(2).all(|w| w[0].ch < w[1].ch), "sorted");
    }

    #[test]
    fn unknown_chars_fall_back_to_question_mark() {
        assert_eq!(FontMetrics::inter().glyph('é').ch, '?');
    }

    #[test]
    fn atlas_rects_fit_inside_the_atlas() {
        let m = FontMetrics::inter();
        for g in &m.glyphs {
            if let Some([x, y, w, h]) = g.atlas {
                assert!(
                    x + w <= m.atlas.width && y + h <= m.atlas.height,
                    "{:?}",
                    g.ch
                );
            }
            assert_eq!(g.plane.is_some(), g.atlas.is_some());
        }
    }

    #[test]
    fn digits_are_wider_than_a_space() {
        let m = FontMetrics::inter();
        assert!(m.text_width("0", 16.0) > m.text_width(" ", 16.0));
        assert!((m.text_width("00", 10.0) - 2.0 * m.text_width("0", 10.0)).abs() < 1e-4);
    }
}
