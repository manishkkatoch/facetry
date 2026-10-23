//! Turning a read-back render target into the RGBA8 bytes JS wants. Pure functions, unit-tested
//! natively because the GPU path that feeds them cannot run in CI.

/// What the bytes of a read-back target mean.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReadbackFormat {
    /// Channels are stored blue, green, red, alpha (`Bgra8*`) instead of red first.
    pub bgra: bool,
    /// The stored values are sRGB-encoded and blending happened in linear space (`*Srgb`).
    pub srgb: bool,
    /// The colours were blended onto a transparent clear, so they are *premultiplied* by
    /// alpha and must be divided by it for an image format that is not.
    pub premultiplied: bool,
}

/// Bytes per row of a buffer a texture of `width` 4-byte pixels is copied into: wgpu wants a
/// multiple of 256.
pub fn padded_bytes_per_row(width: u32) -> u32 {
    const ALIGN: u32 = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    (width * 4).div_ceil(ALIGN) * ALIGN
}

fn srgb_to_linear(v: f32) -> f32 {
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_to_srgb(v: f32) -> f32 {
    if v <= 0.003_130_8 {
        v * 12.92
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    }
}

/// Divides a premultiplied channel by `alpha`. For sRGB targets the blend happened on linear
/// values, so decode, divide, encode again.
fn unpremultiply(channel: u8, alpha: f32, srgb: bool) -> u8 {
    let v = f32::from(channel) / 255.0;
    let out = if srgb {
        linear_to_srgb((srgb_to_linear(v) / alpha).min(1.0))
    } else {
        (v / alpha).min(1.0)
    };
    (out * 255.0).round() as u8
}

/// Rows of `padded` (each `padded_row` bytes, the first `width * 4` of them pixels) as tightly
/// packed RGBA8, un-padded, with channels reordered and un-premultiplied as `format` says.
pub fn to_rgba(
    padded: &[u8],
    width: u32,
    height: u32,
    padded_row: u32,
    format: ReadbackFormat,
) -> Vec<u8> {
    let row = width as usize * 4;
    let mut out = Vec::with_capacity(row * height as usize);
    for y in 0..height as usize {
        let start = y * padded_row as usize;
        out.extend_from_slice(&padded[start..start + row]);
    }
    for px in out.chunks_exact_mut(4) {
        if format.bgra {
            px.swap(0, 2);
        }
        let a = px[3];
        if format.premultiplied && a != 255 {
            if a == 0 {
                px[..3].fill(0);
            } else {
                let alpha = f32::from(a) / 255.0;
                for c in &mut px[..3] {
                    *c = unpremultiply(*c, alpha, format.srgb);
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLAIN: ReadbackFormat = ReadbackFormat {
        bgra: false,
        srgb: false,
        premultiplied: false,
    };

    #[test]
    fn rows_are_padded_to_256_bytes() {
        assert_eq!(padded_bytes_per_row(1), 256);
        assert_eq!(padded_bytes_per_row(64), 256);
        assert_eq!(padded_bytes_per_row(65), 512);
        assert_eq!(padded_bytes_per_row(800), 3328); // 3200 bytes is 12.5 alignments
        assert_eq!(padded_bytes_per_row(832), 3328);
        assert_eq!(padded_bytes_per_row(833), 3584);
    }

    #[test]
    fn padding_is_removed_row_by_row() {
        // 3 x 2 pixels, rows padded to 16 bytes with 0xEE.
        let mut padded = Vec::new();
        for y in 0..2u8 {
            for x in 0..3u8 {
                padded.extend_from_slice(&[x, y, 7, 255]);
            }
            padded.extend_from_slice(&[0xEE; 4]);
        }
        let out = to_rgba(&padded, 3, 2, 16, PLAIN);
        assert_eq!(out.len(), 3 * 2 * 4);
        assert_eq!(&out[..4], &[0, 0, 7, 255]);
        assert_eq!(&out[12..16], &[0, 1, 7, 255]);
        assert!(!out.contains(&0xEE));
    }

    #[test]
    fn bgra_is_swapped_to_rgba() {
        let out = to_rgba(
            &[10, 20, 30, 255],
            1,
            1,
            4,
            ReadbackFormat {
                bgra: true,
                ..PLAIN
            },
        );
        assert_eq!(out, [30, 20, 10, 255]);
    }

    #[test]
    fn opaque_pixels_are_left_alone_even_when_premultiplied() {
        let f = ReadbackFormat {
            premultiplied: true,
            srgb: true,
            ..PLAIN
        };
        assert_eq!(to_rgba(&[12, 99, 200, 255], 1, 1, 4, f), [12, 99, 200, 255]);
    }

    #[test]
    fn premultiplied_gamma_pixels_are_divided_by_alpha() {
        let f = ReadbackFormat {
            premultiplied: true,
            ..PLAIN
        };
        // 50% alpha, colour stored as half of (200, 100, 50).
        assert_eq!(
            to_rgba(&[100, 50, 25, 128], 1, 1, 4, f),
            [199, 100, 50, 128]
        );
        // Fully transparent pixels become plain transparent black.
        assert_eq!(to_rgba(&[0, 0, 0, 0], 1, 1, 4, f), [0, 0, 0, 0]);
    }

    #[test]
    fn premultiplied_srgb_pixels_are_unpremultiplied_in_linear_space() {
        let f = ReadbackFormat {
            premultiplied: true,
            srgb: true,
            ..PLAIN
        };
        // An sRGB 200 channel over 50% alpha was blended as linear 0.578 * 0.5 = 0.289, which
        // is stored as sRGB 146; dividing that in gamma space would give 229, not 200.
        let stored = (linear_to_srgb(srgb_to_linear(200.0 / 255.0) * 0.5) * 255.0).round() as u8;
        assert_eq!(stored, 146);
        let out = to_rgba(&[stored, stored, stored, 128], 1, 1, 4, f);
        assert!((i32::from(out[0]) - 200).abs() <= 2, "{out:?}");
    }
}
