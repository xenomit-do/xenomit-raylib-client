//! DarkOrbit's ATF image payloads. Its `ATFData` loader recognizes an
//! extended header whose marker sits at byte 6 (three bytes after the
//! standard ATF v2 marker), so normalize that variant before decoding.

use anyhow::{bail, ensure, Context, Result};
use image::{Rgba, RgbaImage};
use std::io::{Cursor, Read, Seek};

enum MipData<'a> {
    JpegXr(&'a [u8]),
    CompressedOpaque {
        lzma_indices: &'a [u8],
        jpegxr_endpoints: &'a [u8],
    },
    Dxt5(&'a [u8]),
    CompressedAlpha {
        dxt1_alpha: &'a [u8],
        jpegxr_alpha: &'a [u8],
        dxt5_rgb: &'a [u8],
        jpegxr_bgr: &'a [u8],
    },
    RawCompressedAlpha(&'a [u8]),
}

/// Decode the first ATF mip level to CPU RGBA pixels.
pub fn decode(bytes: &[u8]) -> Result<RgbaImage> {
    let (data_offset, version) = header(bytes)?;
    ensure!(
        bytes.len() >= data_offset + 4,
        "truncated ATF texture header"
    );
    let format = bytes[data_offset];
    let width = 1u32
        .checked_shl(u32::from(bytes[data_offset + 1]))
        .context("invalid ATF width exponent")?;
    let height = 1u32
        .checked_shl(u32::from(bytes[data_offset + 2]))
        .context("invalid ATF height exponent")?;
    ensure!(
        width <= 8192 && height <= 8192,
        "ATF dimensions exceed limit"
    );
    let _mip_count = bytes[data_offset + 3];
    let mut cursor = data_offset + 4;
    let mip = match format {
        // RGB888/RGBA8888 ATF mips carry a JPEG-XR bitmap.
        0 | 1 => MipData::JpegXr(read_record(bytes, &mut cursor, version)?),
        // Format 2 splits each DXT1 mip into LZMA-compressed 2-bit indices
        // (4 bytes/block) and a JPEG-XR RGB565 endpoint image. The remaining
        // records of the mip are alternate GPU encodings and are ignored.
        2 => MipData::CompressedOpaque {
            lzma_indices: read_record(bytes, &mut cursor, version)?,
            jpegxr_endpoints: read_record(bytes, &mut cursor, version)?,
        },
        3 => MipData::Dxt5(read_record(bytes, &mut cursor, version)?),
        // Compressed alpha stores DXT lookup data plus JPEG-XR pixel data.
        4 => MipData::CompressedAlpha {
            dxt1_alpha: read_record(bytes, &mut cursor, version)?,
            jpegxr_alpha: read_record(bytes, &mut cursor, version)?,
            dxt5_rgb: read_record(bytes, &mut cursor, version)?,
            jpegxr_bgr: read_record(bytes, &mut cursor, version)?,
        },
        // Raw compressed alpha includes DXT5 first on desktop builds.
        5 => MipData::RawCompressedAlpha(read_record(bytes, &mut cursor, version)?),
        _ => bail!("unsupported DarkOrbit ATF format {format} (version {version})"),
    };

    match mip {
        MipData::JpegXr(data) => jpegxr_image(data, width, height),
        MipData::CompressedOpaque {
            lzma_indices,
            jpegxr_endpoints,
        } => {
            let dxt1 = reconstruct_dxt1(width, height, lzma_indices, jpegxr_endpoints)?;
            decode_dxt1(&dxt1, width, height)
        }
        MipData::Dxt5(data) => decode_dxt5(data, width, height),
        MipData::RawCompressedAlpha(dxt5) => decode_dxt5(&dxt5, width, height),
        MipData::CompressedAlpha {
            dxt1_alpha,
            jpegxr_alpha,
            dxt5_rgb,
            jpegxr_bgr,
        } => {
            let dxt5 = reconstruct_compressed_alpha(
                width,
                height,
                jpegxr_alpha,
                dxt1_alpha,
                jpegxr_bgr,
                dxt5_rgb,
            )?;
            decode_dxt5(&dxt5, width, height)
        }
    }
}

/// Returns the texture-header offset and ATF record-length version.
fn header(bytes: &[u8]) -> Result<(usize, u8)> {
    ensure!(bytes.starts_with(b"ATF"), "invalid ATF signature");
    if bytes.get(6) == Some(&0xff) {
        // DarkOrbit's deployed client seeks directly to byte 12 when the
        // marker is present at byte 6. The following version and LE length
        // mirror the standard extended ATF header.
        ensure!(bytes.len() >= 12, "truncated DarkOrbit ATF extended header");
        let version = bytes[7];
        return Ok((12, version));
    }
    if bytes.get(3) == Some(&0xff) {
        ensure!(bytes.len() >= 10, "truncated ATF extended header");
        return Ok((9, bytes[4]));
    }
    ensure!(bytes.len() >= 10, "truncated ATF header");
    Ok((6, 0))
}

fn read_record<'a>(bytes: &'a [u8], cursor: &mut usize, version: u8) -> Result<&'a [u8]> {
    let length = if version == 0 {
        let end = cursor.checked_add(3).context("ATF offset overflow")?;
        let size = bytes
            .get(*cursor..end)
            .context("truncated ATF record length")?;
        *cursor = end;
        (usize::from(size[0]) << 16) | (usize::from(size[1]) << 8) | usize::from(size[2])
    } else {
        let end = cursor.checked_add(4).context("ATF offset overflow")?;
        let size = bytes
            .get(*cursor..end)
            .context("truncated ATF record length")?;
        *cursor = end;
        u32::from_be_bytes(size.try_into()?) as usize
    };
    let end = cursor
        .checked_add(length)
        .context("ATF record size overflow")?;
    let data = bytes.get(*cursor..end).context("truncated ATF record")?;
    *cursor = end;
    Ok(data)
}

fn jpegxr_raw<R: Read + Seek>(
    bytes: R,
    width: u32,
    height: u32,
) -> Result<(Vec<u8>, jpegxr::PixelFormat)> {
    let mut decoder = jpegxr::ImageDecode::with_reader(bytes).context("JPEG-XR init")?;
    let format = decoder.get_pixel_format().context("JPEG-XR pixel format")?;
    let (actual_w, actual_h) = decoder.get_size().context("JPEG-XR dimensions")?;
    ensure!(
        actual_w as u32 == width && actual_h as u32 == height,
        "JPEG-XR dimensions {actual_w}x{actual_h}, expected {width}x{height}"
    );
    let stride = width as usize * jpegxr::PixelInfo::from_format(format).bits_per_pixel() / 8;
    let mut raw = vec![
        0;
        stride
            .checked_mul(height as usize)
            .context("JPEG-XR buffer size overflow")?
    ];
    decoder
        .copy_all(&mut raw, stride)
        .context("JPEG-XR decode")?;
    Ok((raw, format))
}

fn jpegxr_image(bytes: &[u8], width: u32, height: u32) -> Result<RgbaImage> {
    let mut decoder =
        jpegxr::ImageDecode::with_reader(Cursor::new(bytes)).context("JPEG-XR init")?;
    let mut tiff = Cursor::new(Vec::new());
    decoder
        .convert_to_tiff(&mut tiff)
        .context("JPEG-XR to TIFF")?;
    let image =
        image::ImageReader::with_format(Cursor::new(tiff.into_inner()), image::ImageFormat::Tiff)
            .decode()
            .context("JPEG-XR TIFF decode")?
            .to_rgba8();
    ensure!(
        image.width() == width && image.height() == height,
        "JPEG-XR image dimensions mismatch"
    );
    Ok(image)
}

/// Reassemble DarkOrbit's unusual JPEG-XR + LZMA compressed-alpha records
/// into a standard DXT5 block stream (ported from Ruffle's ATF decoder).
fn reconstruct_compressed_alpha(
    width: u32,
    height: u32,
    jpegxr_alpha: &[u8],
    dxt1_alpha: &[u8],
    jpegxr_bgr: &[u8],
    dxt5_rgb: &[u8],
) -> Result<Vec<u8>> {
    ensure!(
        width % 4 == 0 && height % 4 == 0,
        "compressed-alpha dimensions are not block aligned"
    );
    let block_count = (width / 4) as u64 * (height / 4) as u64;
    let dxt1 = decompress_lzma(dxt1_alpha, block_count * 6)?;
    let dxt5 = decompress_lzma(dxt5_rgb, block_count * 4)?;
    let half_w = width / 4;
    let half_h = height / 2;
    let (_alpha, alpha_format) = jpegxr_raw(Cursor::new(jpegxr_alpha), half_w, half_h)?;
    ensure!(
        alpha_format == jpegxr::PixelFormat::PixelFormat8bppGray,
        "unexpected ATF alpha pixel format: {alpha_format:?}"
    );
    let alpha_img = jpegxr_image(jpegxr_alpha, half_w, half_h)?;
    let bgr_info = jpegxr::ImageDecode::with_reader(Cursor::new(jpegxr_bgr))
        .context("ATF JPEG-XR BGR init")?;
    let bgr_format = bgr_info
        .get_pixel_format()
        .context("ATF JPEG-XR BGR format")?;
    let (bgr_w, bgr_h) = bgr_info.get_size().context("ATF JPEG-XR BGR dimensions")?;
    ensure!(
        bgr_w as u32 == half_w && bgr_h as u32 == half_h,
        "ATF JPEG-XR BGR dimensions mismatch"
    );
    let (bgr, _) = jpegxr_raw(Cursor::new(jpegxr_bgr), half_w, half_h)?;
    ensure!(
        dxt1.len() % 6 == 0 && dxt5.len() % 4 == 0 && dxt1.len() / 6 == dxt5.len() / 4,
        "invalid ATF DXT lookup data"
    );
    ensure!(
        alpha_img.as_raw().len() % 8 == 0
            && bgr.len() % 4 == 0
            && alpha_img.as_raw().len() / 8 == bgr.len() / 4,
        "invalid ATF JPEG-XR packed data"
    );
    let bytes_per_pixel = jpegxr::PixelInfo::from_format(bgr_format).bits_per_pixel() / 8;
    ensure!(
        bytes_per_pixel == 2,
        "unexpected ATF JPEG-XR BGR format {bgr_format:?}"
    );
    let blocks = dxt1.len() / 6;
    ensure!(
        blocks == alpha_img.as_raw().len() / 8,
        "ATF DXT and JPEG-XR data size mismatch"
    );
    let second_half = (half_w as usize)
        .checked_mul((half_h / 2) as usize)
        .context("ATF dimensions overflow")?;
    let mut out = Vec::with_capacity(blocks * 16);
    for i in 0..blocks {
        let alpha_first = &alpha_img.as_raw()[i * 4..i * 4 + 4];
        let alpha_second =
            &alpha_img.as_raw()[second_half * 4 + i * 4..second_half * 4 + i * 4 + 4];
        let bgr_first = &bgr[i * 2..i * 2 + 2];
        let bgr_second = &bgr[second_half * 2 + i * 2..second_half * 2 + i * 2 + 2];
        out.push(alpha_first[0]);
        out.push(alpha_second[0]);
        out.extend_from_slice(&dxt1[i * 6..i * 6 + 6]);
        out.extend_from_slice(bgr_first);
        out.extend_from_slice(bgr_second);
        out.extend_from_slice(&dxt5[i * 4..i * 4 + 4]);
    }
    Ok(out)
}

/// Reassemble a standard DXT1 block stream from format-2 records: JPEG-XR
/// holds (w/4)x(h/2) RGB565 pixels (first half = color0, second half =
/// color1 per block); LZMA holds the 4 index bytes per block.
fn reconstruct_dxt1(
    width: u32,
    height: u32,
    lzma_indices: &[u8],
    jpegxr_endpoints: &[u8],
) -> Result<Vec<u8>> {
    ensure!(
        width % 4 == 0 && height % 4 == 0,
        "compressed dimensions are not block aligned"
    );
    let blocks = (width / 4) as usize * (height / 4) as usize;
    let indices = decompress_lzma(lzma_indices, (blocks * 4) as u64)?;
    let half_w = width / 4;
    let half_h = height / 2;
    let (endpoints, format) = jpegxr_raw(Cursor::new(jpegxr_endpoints), half_w, half_h)?;
    let bytes_per_pixel = jpegxr::PixelInfo::from_format(format).bits_per_pixel() / 8;
    ensure!(
        bytes_per_pixel == 2,
        "unexpected ATF JPEG-XR endpoint format {format:?}"
    );
    ensure!(
        indices.len() == blocks * 4 && endpoints.len() == blocks * 4,
        "ATF DXT1 data size mismatch: {} index bytes, {} endpoint bytes, {blocks} blocks",
        indices.len(),
        endpoints.len()
    );
    let second_half = blocks * 2;
    let mut out = Vec::with_capacity(blocks * 8);
    for i in 0..blocks {
        out.extend_from_slice(&endpoints[i * 2..i * 2 + 2]);
        out.extend_from_slice(&endpoints[second_half + i * 2..second_half + i * 2 + 2]);
        out.extend_from_slice(&indices[i * 4..i * 4 + 4]);
    }
    Ok(out)
}

/// ATF LZMA records omit the uncompressed-size header field. Splice in
/// the known output size rather than `u64::MAX`: without it the decoder
/// must infer the end from the range coder's EOS marker, which some
/// encoders leave out — the last literal then never lands (observed: a
/// 512² index stream yielding 65535 of 65536 bytes).
fn decompress_lzma(data: &[u8], unpacked_size: u64) -> Result<Vec<u8>> {
    ensure!(data.len() >= 5, "truncated ATF LZMA record");
    let mut stream = data.to_vec();
    stream.splice(5..5, unpacked_size.to_le_bytes());
    let mut out = Vec::new();
    lzma_rs::lzma_decompress(&mut stream.as_slice(), &mut out).context("ATF LZMA decode")?;
    ensure!(
        out.len() as u64 == unpacked_size,
        "ATF LZMA stream short: {} bytes, expected {unpacked_size}",
        out.len()
    );
    Ok(out)
}

fn decode_dxt5(bytes: &[u8], width: u32, height: u32) -> Result<RgbaImage> {
    ensure!(width > 0 && height > 0, "empty ATF dimensions");
    let blocks_x = width.div_ceil(4) as usize;
    let blocks_y = height.div_ceil(4) as usize;
    let block_count = blocks_x
        .checked_mul(blocks_y)
        .context("ATF block count overflow")?;
    let required = block_count
        .checked_mul(16)
        .context("ATF DXT5 size overflow")?;
    ensure!(
        bytes.len() >= required,
        "truncated ATF DXT5 image: {} bytes, expected {required}",
        bytes.len()
    );
    let mut image = RgbaImage::new(width, height);
    for by in 0..blocks_y {
        for bx in 0..blocks_x {
            let i = (by * blocks_x + bx) * 16;
            let alpha = alpha_palette(bytes[i], bytes[i + 1]);
            let alpha_bits = bytes[i + 2..i + 8]
                .iter()
                .enumerate()
                .fold(0u64, |acc, (n, b)| acc | (u64::from(*b) << (n * 8)));
            let c0 = u16::from_le_bytes([bytes[i + 8], bytes[i + 9]]);
            let c1 = u16::from_le_bytes([bytes[i + 10], bytes[i + 11]]);
            let colors = color_palette(c0, c1);
            let color_bits = u32::from_le_bytes(bytes[i + 12..i + 16].try_into()?);
            for py in 0..4 {
                for px in 0..4 {
                    let x = bx * 4 + px;
                    let y = by * 4 + py;
                    if x >= width as usize || y >= height as usize {
                        continue;
                    }
                    let pixel = py * 4 + px;
                    let mut rgba = colors[((color_bits >> (pixel * 2)) & 3) as usize];
                    rgba[3] = alpha[((alpha_bits >> (pixel * 3)) & 7) as usize];
                    image.put_pixel(x as u32, y as u32, Rgba(rgba));
                }
            }
        }
    }
    Ok(image)
}

fn decode_dxt1(bytes: &[u8], width: u32, height: u32) -> Result<RgbaImage> {
    ensure!(width > 0 && height > 0, "empty ATF dimensions");
    let blocks_x = width.div_ceil(4) as usize;
    let blocks_y = height.div_ceil(4) as usize;
    let block_count = blocks_x
        .checked_mul(blocks_y)
        .context("ATF block count overflow")?;
    let required = block_count
        .checked_mul(8)
        .context("ATF DXT1 size overflow")?;
    ensure!(
        bytes.len() >= required,
        "truncated ATF DXT1 image: {} bytes, expected {required}",
        bytes.len()
    );
    let mut image = RgbaImage::new(width, height);
    for by in 0..blocks_y {
        for bx in 0..blocks_x {
            let i = (by * blocks_x + bx) * 8;
            let c0 = u16::from_le_bytes([bytes[i], bytes[i + 1]]);
            let c1 = u16::from_le_bytes([bytes[i + 2], bytes[i + 3]]);
            let mut colors = [rgb565(c0), rgb565(c1), [0; 4], [0; 4]];
            if c0 > c1 {
                colors[2] = interpolate(colors[0], colors[1], 2, 1);
                colors[3] = interpolate(colors[0], colors[1], 1, 2);
            } else {
                colors[2] = interpolate(colors[0], colors[1], 1, 1);
                colors[3] = [0, 0, 0, 0];
            }
            let bits = u32::from_le_bytes(bytes[i + 4..i + 8].try_into()?);
            for py in 0..4 {
                for px in 0..4 {
                    let x = bx * 4 + px;
                    let y = by * 4 + py;
                    if x >= width as usize || y >= height as usize {
                        continue;
                    }
                    let index = (bits >> ((py * 4 + px) * 2)) & 3;
                    image.put_pixel(x as u32, y as u32, Rgba(colors[index as usize]));
                }
            }
        }
    }
    Ok(image)
}

fn rgb565(color: u16) -> [u8; 4] {
    let r = ((color >> 11) & 31) as u8;
    let g = ((color >> 5) & 63) as u8;
    let b = (color & 31) as u8;
    [
        (r << 3) | (r >> 2),
        (g << 2) | (g >> 4),
        (b << 3) | (b >> 2),
        255,
    ]
}

fn interpolate(a: [u8; 4], b: [u8; 4], wa: u16, wb: u16) -> [u8; 4] {
    let denominator = wa + wb;
    std::array::from_fn(|i| ((u16::from(a[i]) * wa + u16::from(b[i]) * wb) / denominator) as u8)
}

fn alpha_palette(a0: u8, a1: u8) -> [u8; 8] {
    let mut p = [0; 8];
    p[0] = a0;
    p[1] = a1;
    if a0 > a1 {
        for i in 1..=6 {
            p[i + 1] = (((7 - i) as u16 * a0 as u16 + i as u16 * a1 as u16) / 7) as u8;
        }
    } else {
        for i in 1..=4 {
            p[i + 1] = (((5 - i) as u16 * a0 as u16 + i as u16 * a1 as u16) / 5) as u8;
        }
        p[6] = 0;
        p[7] = 255;
    }
    p
}

fn color_palette(c0: u16, c1: u16) -> [[u8; 4]; 4] {
    let expand = |c: u16| -> [u8; 3] {
        let r = ((c >> 11) & 31) as u8;
        let g = ((c >> 5) & 63) as u8;
        let b = (c & 31) as u8;
        [
            (r << 3) | (r >> 2),
            (g << 2) | (g >> 4),
            (b << 3) | (b >> 2),
        ]
    };
    let a = expand(c0);
    let b = expand(c1);
    let mix =
        |x: u8, y: u8, n: u16| -> u8 { ((u16::from(x) * (3 - n) + u16::from(y) * n) / 3) as u8 };
    [
        [a[0], a[1], a[2], 255],
        [b[0], b[1], b[2], 255],
        [
            mix(a[0], b[0], 1),
            mix(a[1], b[1], 1),
            mix(a[2], b[2], 1),
            255,
        ],
        [
            mix(a[0], b[0], 2),
            mix(a[1], b[1], 2),
            mix(a[2], b[2], 2),
            255,
        ],
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_darkorbit_extended_atf_header_and_dxt5() {
        let mut data = b"ATF\0\0\x01\xff\x02".to_vec();
        data.extend_from_slice(&24u32.to_le_bytes()); // texture header + one DXT5 record
        data.extend_from_slice(&[5, 2, 2, 1]); // raw compressed alpha, 4x4, one mip
        data.extend_from_slice(&16u32.to_be_bytes());
        let mut block = [0u8; 16];
        block[0] = 255;
        block[1] = 0;
        block[8..10].copy_from_slice(&0xf800u16.to_le_bytes()); // red
        block[10..12].copy_from_slice(&0x07e0u16.to_le_bytes());
        data.extend_from_slice(&block);

        let image = decode(&data).unwrap();
        assert_eq!(image.dimensions(), (4, 4));
        assert_eq!(image.get_pixel(0, 0).0, [255, 0, 0, 255]);
    }

    #[test]
    fn decodes_real_cached_darkorbit_atf_files_when_requested() {
        let Some(dir) = std::env::var_os("XENOMIT_ATF_FIXTURE_DIR").map(std::path::PathBuf::from)
        else {
            return;
        };
        let files = std::fs::read_dir(dir)
            .expect("ATF fixture directory")
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().ends_with("_atf"))
            .collect::<Vec<_>>();
        assert!(!files.is_empty(), "no cached *_atf fixtures");
        for file in files {
            let bytes = std::fs::read(file.path()).expect("read ATF fixture");
            let image =
                decode(&bytes).unwrap_or_else(|e| panic!("{}: {e:#}", file.path().display()));
            assert!(image.width() > 0 && image.height() > 0);
            assert!(
                image.pixels().any(|pixel| pixel.0[3] != 0),
                "{} is transparent",
                file.path().display()
            );
        }
    }
}
