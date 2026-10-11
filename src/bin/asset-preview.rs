//! Decode cached JPEG-XR and ATF files for the offline asset catalogue.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

fn main() -> Result<()> {
    let mut args = std::env::args_os().skip(1);
    let input = args
        .next()
        .map(PathBuf::from)
        .context("input directory required")?;
    let output = args
        .next()
        .map(PathBuf::from)
        .context("output directory required")?;
    anyhow::ensure!(
        args.next().is_none(),
        "usage: asset-preview <input-dir> <output-dir>"
    );
    std::fs::create_dir_all(&output).with_context(|| format!("create {}", output.display()))?;

    let files = std::fs::read_dir(&input).with_context(|| format!("read {}", input.display()))?;
    let mut decoded = 0usize;
    let mut failed = 0usize;
    let mut cached_failures = 0usize;
    let mut scanned = 0usize;
    let mut error_samples = Vec::new();
    for entry in files {
        let path = entry?.path();
        if !path.is_file() {
            continue;
        }
        scanned += 1;
        if scanned % 500 == 0 {
            println!("  scanned {scanned} cached files for game texture previews…");
        }
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(_) => continue,
        };
        if !(bytes.starts_with(b"ATF")
            || bytes.starts_with(b"II\xbc\x01")
            || bytes.starts_with(b"MM\x01\xbc"))
        {
            continue;
        }
        let target = output.join(format!(
            "{}.png",
            path.file_name().unwrap_or_default().to_string_lossy()
        ));
        let failure_marker = target.with_file_name(format!(
            "{}.error",
            target.file_name().unwrap_or_default().to_string_lossy()
        ));
        if is_fresh(&target, &path) {
            continue;
        }
        if is_fresh(&failure_marker, &path) {
            cached_failures += 1;
            continue;
        }
        match xenomit_client::assets::decode_catalog_image(&bytes) {
            Ok(image) => {
                let thumbnail = image::imageops::thumbnail(&image, 600, 400);
                if let Err(error) = thumbnail.save(&target) {
                    failed += 1;
                    let _ = std::fs::write(&failure_marker, format!("save failed: {error}"));
                    if error_samples.len() < 20 {
                        error_samples.push(format!(
                            "{}: save failed: {error}",
                            path.file_name().unwrap_or_default().to_string_lossy()
                        ));
                    }
                } else {
                    let _ = std::fs::remove_file(&failure_marker);
                    decoded += 1;
                }
            }
            Err(error) => {
                failed += 1;
                let _ = std::fs::write(&failure_marker, format!("{error:#}"));
                if error_samples.len() < 20 {
                    error_samples.push(format!(
                        "{}: {error:#}",
                        path.file_name().unwrap_or_default().to_string_lossy()
                    ));
                }
            }
        }
    }
    for error in error_samples {
        eprintln!("{error}");
    }
    println!("Native previews: {decoded} decoded, {failed} failed, {cached_failures} cached failures ({scanned} cache files scanned).",);
    Ok(())
}

fn is_fresh(target: &Path, source: &Path) -> bool {
    match (
        target.metadata().and_then(|m| m.modified()),
        source.metadata().and_then(|m| m.modified()),
    ) {
        (Ok(preview), Ok(source)) => preview >= source,
        _ => false,
    }
}
