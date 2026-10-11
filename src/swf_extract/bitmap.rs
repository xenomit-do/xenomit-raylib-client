//! Bitmap character decoding (DefineBits / Jpeg2-4 / Lossless).
//!
//! Produces premultiplied `tiny_skia::Pixmap`s for internal compositing.
//! JPEG plumbing follows Ruffle's `render/src/utils.rs`: DefineBits v1 data
//! needs the shared `JpegTables` tables glued on, and an erroneous
//! `FF D9 FF D8` sequence can appear before the SOF marker.

use anyhow::{anyhow, Result};
use std::borrow::Cow;
use std::io::Read;
use tiny_skia::Pixmap;

/// Decode a DefineBits*/DefineBitsJpeg2/3/4 payload (JPEG/PNG) plus the
/// optional zlib-compressed alpha channel (DefineBitsJpeg3/4).
/// `jpeg_tables` is only used for DefineBits v1 payloads.
pub(crate) fn decode_bits(
    data: &[u8],
    alpha: Option<&[u8]>,
    jpeg_tables: Option<&[u8]>,
    needs_tables: bool,
) -> Result<Pixmap> {
    let jpeg_data = if needs_tables {
        glue_tables_to_jpeg(data, jpeg_tables)
    } else {
        Cow::Borrowed(data)
    };
    let jpeg_data = remove_invalid_jpeg_data(&jpeg_data);

    let img = image::load_from_memory(&jpeg_data)
        .map_err(|e| anyhow!("bitmap decode failed: {e}"))?
        .to_rgba8();
    let (w, h) = img.dimensions();
    let mut rgba = img.into_raw();

    // DefineBitsJpeg3/4 carry a separate zlib-compressed grayscale alpha map.
    if let Some(alpha_data) = alpha {
        let alpha = decompress_zlib(alpha_data)?;
        if alpha.len() == (w as usize) * (h as usize) {
            for (px, &a) in rgba.as_chunks_mut::<4>().0.iter_mut().zip(alpha.iter()) {
                // Per the spec these pixels are premultiplied already; some
                // SWFs ship straight colors instead, so clamp like Flash does.
                px[0] = px[0].min(a);
                px[1] = px[1].min(a);
                px[2] = px[2].min(a);
                px[3] = a;
            }
        } else {
            log::warn!(
                "DefineBitsJpeg3 alpha size mismatch: {} != {}x{}",
                alpha.len(),
                w,
                h
            );
        }
    }

    pixmap_from_straight_rgba(&mut rgba, w, h)
}

/// Decode a DefineBitsLossless(2) payload: zlib-compressed, optionally
/// palletized pixel data.
pub(crate) fn decode_lossless(tag: &swf::DefineBitsLossless<'_>) -> Result<Pixmap> {
    let data = decompress_zlib(&tag.data)?;
    let (w, h) = (tag.width as usize, tag.height as usize);
    validate_size(w, h)?;
    let has_alpha = tag.version == 2;

    let mut out = Vec::with_capacity(w * h * 4);
    match tag.format {
        swf::BitmapFormat::Rgb15 => {
            // 2 bytes/pixel, rows padded to a 4-byte boundary.
            let padded_w = (w + 1) & !1;
            let need = padded_w * h * 2;
            if data.len() < need {
                return Err(anyhow!("lossless Rgb15 data truncated"));
            }
            let mut i = 0usize;
            for _ in 0..h {
                for _ in 0..w {
                    let v = u16::from_be_bytes([data[i], data[i + 1]]);
                    let c = |shift: u16| (((v >> shift) & 0x1F) * 255 + 15) as u8 / 31;
                    out.extend([c(10), c(5), c(0), u8::MAX]);
                    i += 2;
                }
                i += (padded_w - w) * 2;
            }
        }
        swf::BitmapFormat::Rgb32 => {
            // Stored as [x/alpha, r, g, b] quads.
            let need = w * h * 4;
            if data.len() < need {
                return Err(anyhow!("lossless Rgb32 data truncated"));
            }
            for px in data[..need].as_chunks::<4>().0 {
                let a = if has_alpha { px[0] } else { u8::MAX };
                out.extend([px[1], px[2], px[3], a]);
            }
        }
        swf::BitmapFormat::ColorMap8 { num_colors } => {
            // Palette of num_colors+1 entries (3 bytes v1, 4 bytes v2),
            // then 1 index per pixel, rows padded to a 4-byte boundary.
            let stride = if has_alpha { 4 } else { 3 };
            let palette_len = (num_colors as usize + 1) * stride;
            if data.len() < palette_len {
                return Err(anyhow!("lossless ColorMap8 palette truncated"));
            }
            let mut palette = Vec::with_capacity(num_colors as usize + 1);
            for i in 0..=num_colors as usize {
                let o = i * stride;
                let a = if has_alpha { data[o + 3] } else { u8::MAX };
                palette.push([data[o], data[o + 1], data[o + 2], a]);
            }

            let padded_w = (w + 3) & !3;
            let mut i = palette_len;
            if data.len() < palette_len + padded_w * h {
                return Err(anyhow!("lossless ColorMap8 data truncated"));
            }
            for _ in 0..h {
                for _ in 0..w {
                    let entry = data[i] as usize;
                    let color = palette.get(entry).copied().unwrap_or(if has_alpha {
                        [0, 0, 0, 0]
                    } else {
                        [0, 0, 0, u8::MAX]
                    });
                    out.extend(color);
                    i += 1;
                }
                i += padded_w - w;
            }
        }
    }

    pixmap_from_straight_rgba(&mut out, w as u32, h as u32)
}

fn validate_size(w: usize, h: usize) -> Result<()> {
    // ~128 MP guard against corrupt headers.
    if w.saturating_mul(h) >= 0x0800_0000 {
        return Err(anyhow!("bitmap size unreasonable: {w}x{h}"));
    }
    Ok(())
}

/// Build a premultiplied `Pixmap` from straight RGBA bytes (in place).
fn pixmap_from_straight_rgba(rgba: &mut [u8], w: u32, h: u32) -> Result<Pixmap> {
    for px in rgba.as_chunks_mut::<4>().0.iter_mut() {
        let a = px[3] as u16;
        px[0] = ((px[0] as u16 * a + 127) / 255) as u8;
        px[1] = ((px[1] as u16 * a + 127) / 255) as u8;
        px[2] = ((px[2] as u16 * a + 127) / 255) as u8;
    }
    let size = tiny_skia::IntSize::from_wh(w, h).ok_or_else(|| anyhow!("empty bitmap"))?;
    Pixmap::from_vec(rgba.to_vec(), size).ok_or_else(|| anyhow!("bitmap buffer mismatch"))
}

fn decompress_zlib(data: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    flate2::read::ZlibDecoder::new(data).read_to_end(&mut out)?;
    Ok(out)
}

/// Glue shared JPEG encoding tables onto a DefineBits v1 image payload.
/// (Port of ruffle `glue_tables_to_jpeg`.)
fn glue_tables_to_jpeg<'a>(jpeg_data: &'a [u8], jpeg_tables: Option<&'a [u8]>) -> Cow<'a, [u8]> {
    if let Some(tables) = jpeg_tables.filter(|t| t.len() >= 2) {
        let mut full = Vec::with_capacity(tables.len() + jpeg_data.len());
        // Tables end with EOI; image data starts with SOI — the pair is dropped.
        full.extend_from_slice(&tables[..tables.len() - 2]);
        if jpeg_data.len() >= 2 {
            full.extend_from_slice(&jpeg_data[2..]);
        } else {
            full.extend_from_slice(jpeg_data);
        }
        return full.into();
    }
    jpeg_data.into()
}

/// Remove the invalid `FF D9 FF D8` (EOI+SOI) sequence some SWF encoders
/// leave before the SOF marker, and ensure a trailing EOI marker.
/// (Port of ruffle `remove_invalid_jpeg_data`.)
fn remove_invalid_jpeg_data(data: &[u8]) -> Cow<'_, [u8]> {
    const SOF0: u8 = 0xC0;
    const RST0: u8 = 0xD0;
    const RST7: u8 = 0xD7;
    const SOI: u8 = 0xD8;
    const EOI: u8 = 0xD9;

    let mut data: Cow<[u8]> = if let Some(stripped) = data.strip_prefix(&[0xFF, EOI, 0xFF, SOI]) {
        stripped.into()
    } else {
        let mut jpeg_data = data;
        let mut pos = 0usize;
        loop {
            if jpeg_data.len() < 4 {
                break data.into();
            }
            let payload_len: usize = match &jpeg_data[..4] {
                [0xFF, EOI, 0xFF, SOI] => {
                    let mut out = Vec::with_capacity(data.len() - 4);
                    out.extend_from_slice(&data[..pos]);
                    out.extend_from_slice(&data[pos + 4..]);
                    break out.into();
                }
                // EOI, SOI, RST markers carry no size field.
                [0xFF, EOI | SOI | RST0..=RST7, _, _] => 0,
                [0xFF, SOF0, _, _] => break data.into(),
                [0xFF, _, a, b] => u16::from_be_bytes([*a, *b]).into(),
                _ => break data.into(),
            };
            jpeg_data = jpeg_data.get(payload_len + 2..).unwrap_or_default();
            pos += payload_len + 2;
        }
    };

    if !data.ends_with(&[0xFF, EOI]) {
        data.to_mut().extend_from_slice(&[0xFF, EOI]);
    }
    data
}
