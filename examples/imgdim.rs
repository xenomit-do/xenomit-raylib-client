//! `imgdim <file>` — decode an image (incl. JPEG-XR) and print WxH plus a
//! coarse color survey of the quadrants (helps spot seams).
fn main() -> anyhow::Result<()> {
    let path = std::env::args().nth(1).expect("usage: imgdim <file>");
    let bytes = std::fs::read(&path)?;
    let img = if bytes.starts_with(b"\x49\x49\xbc") || bytes.starts_with(b"II\xbc") {
        let mut dec = jpegxr::ImageDecode::with_reader(std::io::Cursor::new(&bytes))?;
        let mut tiff = std::io::Cursor::new(Vec::new());
        dec.convert_to_tiff(&mut tiff)?;
        image::ImageReader::with_format(
            std::io::Cursor::new(tiff.into_inner()),
            image::ImageFormat::Tiff,
        )
        .decode()?
    } else {
        image::load_from_memory(&bytes)?
    };
    let img = img.to_rgba8();
    let (w, h) = (img.width(), img.height());
    println!("{path}: {w}x{h}");
    // Average color of a 4x4 sample grid.
    for gy in 0..4u32 {
        for gx in 0..4u32 {
            let (mut r, mut g, mut b, mut n) = (0u64, 0u64, 0u64, 0u64);
            for y in (gy * h / 4..(gy + 1) * h / 4).step_by(16) {
                for x in (gx * w / 4..(gx + 1) * w / 4).step_by(16) {
                    let p = img.get_pixel(x, y);
                    r += p[0] as u64;
                    g += p[1] as u64;
                    b += p[2] as u64;
                    n += 1;
                }
            }
            print!("[{:3},{:3},{:3}] ", r / n, g / n, b / n);
        }
        println!();
    }
    Ok(())
}
