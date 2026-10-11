//! Rasterize every exported symbol in the cached SWFs for the offline asset catalogue.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::UNIX_EPOCH;
use xenomit_client::swf_extract::SwfLibrary;

#[derive(Debug, Deserialize)]
struct Input {
    key: String,
    path: PathBuf,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Manifest {
    key: String,
    source_file: String,
    source_bytes: u64,
    source_modified_ns: u128,
    frame_rate: f32,
    parse_error: Option<String>,
    exports: Vec<Export>,
}

#[derive(Debug, Serialize, Deserialize)]
struct Export {
    name: String,
    frames: Vec<String>,
    offsets: Vec<(f32, f32)>,
    error: Option<String>,
}

fn main() -> Result<()> {
    let mut args = std::env::args_os().skip(1);
    let inputs_path = args
        .next()
        .map(PathBuf::from)
        .context("input JSON required")?;
    let output = args
        .next()
        .map(PathBuf::from)
        .context("output directory required")?;
    anyhow::ensure!(
        args.next().is_none(),
        "usage: swf-catalog <inputs.json> <output-dir>"
    );
    let inputs: Vec<Input> = serde_json::from_slice(
        &std::fs::read(&inputs_path).with_context(|| format!("read {}", inputs_path.display()))?,
    )?;
    std::fs::create_dir_all(&output).with_context(|| format!("create {}", output.display()))?;

    let total = inputs.len();
    let workers = std::thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(2)
        .clamp(1, 8);
    println!("  using {workers} SWF raster workers for {total} sources");
    let queue = Mutex::new(VecDeque::from(inputs));
    let rendered = AtomicUsize::new(0);
    let reused = AtomicUsize::new(0);
    let failures = AtomicUsize::new(0);
    let completed = AtomicUsize::new(0);
    let errors = Mutex::new(Vec::<String>::new());
    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| loop {
                let input = match queue.lock().expect("SWF work queue poisoned").pop_front() {
                    Some(input) => input,
                    None => break,
                };
                let result = if !input.key.starts_with("swf-")
                    || !input.key[4..].chars().all(|c| c.is_ascii_hexdigit())
                {
                    Err(anyhow::anyhow!("unsafe output key: {}", input.key))
                } else {
                    let target = output.join(&input.key);
                    let manifest_path = target.join("manifest.json");
                    match manifest_is_current(&manifest_path, &input) {
                        Ok(true) => {
                            reused.fetch_add(1, Ordering::Relaxed);
                            Ok(())
                        }
                        _ => match render_swf(&input, &target) {
                            Ok(()) => {
                                rendered.fetch_add(1, Ordering::Relaxed);
                                Ok(())
                            }
                            Err(error) => Err(error),
                        },
                    }
                };
                if let Err(error) = result {
                    failures.fetch_add(1, Ordering::Relaxed);
                    let mut samples = errors.lock().expect("SWF error list poisoned");
                    if samples.len() < 20 {
                        samples.push(format!("{}: {error:#}", input.path.display()));
                    }
                }
                let done = completed.fetch_add(1, Ordering::Relaxed) + 1;
                if done % 20 == 0 || done == total {
                    println!(
                        "  SWFs {done}/{total} · {} rasterized · {} cached · {} failed",
                        rendered.load(Ordering::Relaxed),
                        reused.load(Ordering::Relaxed),
                        failures.load(Ordering::Relaxed)
                    );
                    let _ = std::io::stdout().flush();
                }
            });
        }
    });
    for error in errors.into_inner().unwrap_or_default() {
        eprintln!("{error}");
    }
    let rendered = rendered.load(Ordering::Relaxed);
    let reused = reused.load(Ordering::Relaxed);
    let failures = failures.load(Ordering::Relaxed);
    println!(
        "SWF rasterization complete: {} rasterized, {} reused, {} failed.",
        rendered, reused, failures
    );
    Ok(())
}

fn manifest_is_current(path: &Path, input: &Input) -> Result<bool> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(_) => return Ok(false),
    };
    let manifest: Manifest = match serde_json::from_slice(&bytes) {
        Ok(manifest) => manifest,
        Err(_) => return Ok(false),
    };
    let metadata = match input.path.metadata() {
        Ok(metadata) => metadata,
        Err(_) => return Ok(false),
    };
    Ok(manifest.key == input.key
        && manifest.source_bytes == metadata.len()
        && manifest.source_modified_ns == modified_ns(&metadata)
        && manifest.exports.iter().all(|export| {
            export.frames.iter().all(|frame| {
                path.parent()
                    .unwrap_or(Path::new("."))
                    .join(frame)
                    .is_file()
            })
        }))
}

fn render_swf(input: &Input, target: &Path) -> Result<()> {
    // The key is a generated hex id, so this directory is confined to the
    // catalogue cache and can be replaced as one unit when the source changes.
    if target.exists() {
        std::fs::remove_dir_all(target).with_context(|| format!("clear {}", target.display()))?;
    }
    std::fs::create_dir_all(target).with_context(|| format!("create {}", target.display()))?;
    let metadata = input
        .path
        .metadata()
        .with_context(|| format!("stat {}", input.path.display()))?;
    let mut manifest = Manifest {
        key: input.key.clone(),
        source_file: input
            .path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
        source_bytes: metadata.len(),
        source_modified_ns: modified_ns(&metadata),
        ..Manifest::default()
    };
    match SwfLibrary::open(&input.path) {
        Err(error) => manifest.parse_error = Some(format!("{error:#}")),
        Ok(lib) => {
            manifest.frame_rate = if lib.frame_rate.is_finite() && lib.frame_rate > 0.0 {
                lib.frame_rate
            } else {
                12.0
            };
            let mut symbols: Vec<_> = lib.symbols.keys().cloned().collect();
            symbols.sort();
            for (symbol_index, name) in symbols.iter().enumerate() {
                let symbol_dir = format!("{:04}_{}", symbol_index, safe_name(name));
                let symbol_path = target.join(&symbol_dir);
                std::fs::create_dir_all(&symbol_path)?;
                let mut export = Export {
                    name: name.clone(),
                    frames: Vec::new(),
                    offsets: Vec::new(),
                    error: None,
                };
                match lib.render_symbol(name) {
                    Ok(frames) => {
                        for (frame_index, frame) in frames.into_iter().enumerate() {
                            let frame_rel = format!("{symbol_dir}/f{:04}.png", frame_index + 1);
                            let frame_path = target.join(&frame_rel);
                            if let Err(error) = frame.image.save(&frame_path) {
                                export.error = Some(format!("frame {}: {error}", frame_index + 1));
                                break;
                            }
                            export.frames.push(frame_rel);
                            export.offsets.push(frame.offset);
                        }
                    }
                    Err(error) => export.error = Some(format!("{error:#}")),
                }
                manifest.exports.push(export);
            }
        }
    }
    let manifest_path = target.join("manifest.json");
    let temp = target.join("manifest.json.tmp");
    std::fs::write(&temp, serde_json::to_vec_pretty(&manifest)?)?;
    std::fs::rename(temp, manifest_path)?;
    Ok(())
}

fn modified_ns(metadata: &std::fs::Metadata) -> u128 {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos())
        .unwrap_or_default()
}

fn safe_name(name: &str) -> String {
    let safe: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '_'
            }
        })
        .take(72)
        .collect();
    if safe.is_empty() {
        "unnamed".into()
    } else {
        safe
    }
}
