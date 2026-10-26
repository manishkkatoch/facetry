//! Offline tool: renders printable ASCII from the bundled TTF into one MSDF atlas.
//!
//! Run `cargo run -p font-atlas --release` (see the README). It rewrites the files in
//! `crates/facetry-font/assets/`, which are committed; nothing at build or run time needs
//! this tool or the TTF. Outputs:
//!
//! - `inter-regular-msdf.rgb`: the atlas as raw 8-bit RGB, row-major, top row first,
//!   `width * height * 3` bytes. This is what `facetry-font` embeds, so the shipped wasm needs
//!   no image decoder.
//! - `inter-regular-msdf.png`: the same pixels as a PNG, for looking at (and for the test
//!   below that proves the two agree). Not embedded anywhere.
//! - `inter-regular-metrics.rs`: glyph metrics as a Rust expression of type `FontMetrics`.
//!   `facetry-font` `include!`s it into a `static`, so the shipped wasm holds the table as plain
//!   data and needs no JSON parser.
//!
//! How an MSDF atlas works, briefly: for every pixel we store, in R/G/B, the signed distance
//! to the nearest edge of the glyph outline, but each channel only "sees" a different subset
//! of the edges. Taking the median of the three channels in a shader reconstructs sharp
//! corners that a plain (single-channel) distance field would round off.

use std::path::{Path, PathBuf};

use fdsm::bezier::scanline::FillRule;
use fdsm::generate::generate_msdf;
use fdsm::render::correct_sign_msdf;
use fdsm::shape::Shape;
use fdsm::transform::Transform;
use image::{RgbImage, imageops};
use nalgebra::{Affine2, Similarity2, Vector2};
use ttf_parser::Face;

/// Atlas pixels per em. 32 keeps the atlas small; the median-of-three shader scales it
/// cleanly from about 8 px up to roughly 3x this size.
const EM_PX: f64 = 32.0;
/// Total distance range encoded (see `AtlasInfo::px_range`).
const RANGE: f64 = 4.0;
/// Empty border around each glyph, so the distance field can fade out inside the quad.
const PAD: f64 = RANGE / 2.0 + 1.0;
/// Atlas width; the height follows from packing.
const ATLAS_WIDTH: u32 = 512;
/// Gap between packed glyphs, so linear filtering never bleeds a neighbour in.
const GUTTER: u32 = 1;

struct Rendered {
    ch: char,
    advance: f32,
    /// `None` for outline-less glyphs (space).
    image: Option<(RgbImage, [f32; 4])>,
}

fn round5(v: f64) -> f32 {
    ((v * 1e5).round() / 1e5) as f32
}

fn render_glyph(face: &Face, ch: char) -> Rendered {
    let upem = f64::from(face.units_per_em());
    let id = face
        .glyph_index(ch)
        .unwrap_or_else(|| panic!("font lacks {ch:?}"));
    let advance = round5(f64::from(face.glyph_hor_advance(id).unwrap()) / upem);

    let (Some(shape), Some(bbox)) = (
        fdsm_ttf_parser::load_shape_from_face(face, id),
        face.glyph_bounding_box(id),
    ) else {
        return Rendered {
            ch,
            advance,
            image: None,
        };
    };

    // Font units per atlas pixel.
    let shrink = upem / EM_PX;
    let (x0, y0) = (f64::from(bbox.x_min), f64::from(bbox.y_min));
    let to_pixels: Affine2<f64> = nalgebra::convert(Similarity2::new(
        Vector2::new(PAD - x0 / shrink, PAD - y0 / shrink),
        0.0,
        1.0 / shrink,
    ));
    let width = ((f64::from(bbox.x_max) - x0) / shrink + 2.0 * PAD).ceil() as u32;
    let height = ((f64::from(bbox.y_max) - y0) / shrink + 2.0 * PAD).ceil() as u32;

    let mut shape = shape;
    shape.transform(&to_pixels);
    let colored = Shape::edge_coloring_simple(shape, 0.03, 69_441_337_420);
    let prepared = colored.prepare();
    let mut msdf = RgbImage::new(width, height);
    generate_msdf(&prepared, RANGE, &mut msdf);
    correct_sign_msdf(&mut msdf, &prepared, FillRule::Nonzero);
    // Font space has y pointing up, images y down.
    imageops::flip_vertical_in_place(&mut msdf);

    // The image spans (width x height) / EM_PX em, starting PAD pixels before the bbox.
    let left = x0 / upem - PAD / EM_PX;
    let bottom = y0 / upem - PAD / EM_PX;
    let plane = [
        round5(left),
        round5(bottom),
        round5(left + f64::from(width) / EM_PX),
        round5(bottom + f64::from(height) / EM_PX),
    ];
    Rendered {
        ch,
        advance,
        image: Some((msdf, plane)),
    }
}

/// Shelf packing: tallest glyphs first, left to right, new row when the row is full.
/// Returns each item's top-left corner and the atlas height.
fn pack(items: &[(usize, u32, u32)]) -> (Vec<(usize, u32, u32)>, u32) {
    let mut order = items.to_vec();
    order.sort_by_key(|&(i, _, h)| (std::cmp::Reverse(h), i));
    let (mut x, mut y, mut row_h) = (GUTTER, GUTTER, 0);
    let mut placed = Vec::new();
    for (i, w, h) in order {
        if x + w + GUTTER > ATLAS_WIDTH {
            x = GUTTER;
            y += row_h + GUTTER;
            row_h = 0;
        }
        placed.push((i, x, y));
        x += w + GUTTER;
        row_h = row_h.max(h);
    }
    (placed, (y + row_h + GUTTER).next_multiple_of(4))
}

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let ttf = std::fs::read(root.join("fonts/Inter-Regular.ttf")).expect("read TTF");
    let face = Face::parse(&ttf, 0).expect("parse TTF");
    let out: PathBuf = root.join("../../crates/facetry-font/assets");

    let rendered: Vec<Rendered> = (' '..='~').map(|c| render_glyph(&face, c)).collect();
    let sizes: Vec<_> = rendered
        .iter()
        .enumerate()
        .filter_map(|(i, r)| {
            r.image
                .as_ref()
                .map(|(img, _)| (i, img.width(), img.height()))
        })
        .collect();
    let (placed, atlas_height) = pack(&sizes);

    let mut atlas = RgbImage::new(ATLAS_WIDTH, atlas_height);
    let mut rects = vec![None; rendered.len()];
    for (i, x, y) in placed {
        let (img, _) = rendered[i].image.as_ref().unwrap();
        imageops::replace(&mut atlas, img, i64::from(x), i64::from(y));
        rects[i] = Some([x, y, img.width(), img.height()]);
    }

    let upem = f64::from(face.units_per_em());
    let glyphs: Vec<_> = rendered
        .iter()
        .zip(&rects)
        .map(|(r, atlas)| (r.ch, r.advance, r.image.as_ref().map(|(_, p)| *p), *atlas))
        .collect();
    let source = metrics_source(
        atlas_height,
        [
            round5(f64::from(face.ascender()) / upem),
            round5(f64::from(face.descender()) / upem),
            round5(f64::from(face.capital_height().unwrap()) / upem),
        ],
        &glyphs,
    );

    std::fs::write(out.join("inter-regular-msdf.rgb"), atlas.as_raw()).expect("write raw atlas");
    atlas
        .save(out.join("inter-regular-msdf.png"))
        .expect("write atlas preview");
    std::fs::write(out.join("inter-regular-metrics.rs"), source).expect("write metrics");
    println!(
        "wrote {ATLAS_WIDTH}x{atlas_height} atlas, {} glyphs",
        glyphs.len()
    );
}

/// One glyph as `main` collects it: character, advance, plane quad, atlas rectangle.
type GlyphRow = (char, f32, Option<[f32; 4]>, Option<[u32; 4]>);

/// The metrics as the Rust expression `facetry-font` includes. `{:?}` prints the shortest
/// text that parses back to the same `f32`, so the table holds exactly the values computed here.
fn metrics_source(
    atlas_height: u32,
    [ascender, descender, cap_height]: [f32; 3],
    glyphs: &[GlyphRow],
) -> String {
    use std::fmt::Write;
    let mut out = String::from(
        "// Generated by `cargo run -p font-atlas --release`; do not edit.\n\
         // `include!`d by crates/facetry-font/src/lib.rs.\n\
         FontMetrics {\n",
    );
    let _ = writeln!(out, "    family: \"Inter Regular\",");
    let _ = writeln!(
        out,
        "    atlas: AtlasInfo {{ width: {ATLAS_WIDTH}, height: {atlas_height}, em_px: {:?}, px_range: {:?} }},",
        EM_PX as f32, RANGE as f32
    );
    let _ = writeln!(
        out,
        "    ascender: {ascender:?},\n    descender: {descender:?},\n    cap_height: {cap_height:?},"
    );
    out.push_str("    glyphs: &[\n");
    for (ch, advance, plane, atlas) in glyphs {
        let _ = writeln!(
            out,
            "        Glyph {{ ch: {ch:?}, advance: {advance:?}, plane: {plane:?}, atlas: {atlas:?} }},"
        );
    }
    out.push_str("    ],\n}\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use facetry_font::FontMetrics;

    fn assets() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../crates/facetry-font/assets")
    }

    /// The embedded raw atlas is byte-for-byte what decoding the PNG gives, and matches the
    /// size recorded in the metrics.
    #[test]
    fn raw_atlas_equals_decoded_png() {
        let png = image::open(assets().join("inter-regular-msdf.png"))
            .expect("decode PNG")
            .into_rgb8();
        let raw = std::fs::read(assets().join("inter-regular-msdf.rgb")).expect("read raw atlas");
        let info = &FontMetrics::inter().atlas;
        assert_eq!((png.width(), png.height()), (info.width, info.height));
        assert_eq!(raw.len(), (info.width * info.height * 3) as usize);
        assert!(
            raw == png.as_raw().as_slice(),
            "raw atlas differs from the PNG"
        );
    }
}
