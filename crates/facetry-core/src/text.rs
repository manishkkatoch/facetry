use facetry_font::FontMetrics;

/// What layout needs to know about text before anything is drawn: how big it is.
///
/// Layout must size margins from real text widths, so it asks a `TextMeasure`. The renderer's
/// font metrics implement it (see below), and tests use the very same metrics, so what layout
/// predicts is what the GPU draws.
pub trait TextMeasure {
    /// Advance width of `text` at `size_px` (the font size in device pixels).
    fn width(&self, text: &str, size_px: f32) -> f32;
    /// Distance from the baseline up to the top of the tallest glyphs.
    fn ascent(&self, size_px: f32) -> f32;
    /// Distance from the baseline down to the bottom of descenders (positive).
    fn descent(&self, size_px: f32) -> f32;
    /// Height of capital letters and digits above the baseline; used to centre labels
    /// vertically on a tick.
    fn cap_height(&self, size_px: f32) -> f32;
}

impl TextMeasure for FontMetrics {
    fn width(&self, text: &str, size_px: f32) -> f32 {
        self.text_width(text, size_px)
    }
    fn ascent(&self, size_px: f32) -> f32 {
        self.ascender * size_px
    }
    fn descent(&self, size_px: f32) -> f32 {
        -self.descender * size_px
    }
    fn cap_height(&self, size_px: f32) -> f32 {
        self.cap_height * size_px
    }
}
