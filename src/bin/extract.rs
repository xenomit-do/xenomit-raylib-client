//! `do-extract` — dump embedded resources and rasterized symbols from a SWF.
//!
//!   do-extract <swf> [--out DIR] [--blobs] [--symbol NAME]... [--all-symbols] [--list]
//!
//! With no extraction flags, `--blobs` is implied: every DefineBinaryData
//! payload lands in `DIR/<symbol-name>[.xml]`. Symbols render to
//! `DIR/<name>_fNN.png`.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::Parser;
use xenomit_client::swf_extract::SwfLibrary;

#[derive(Parser)]
#[command(name = "do-extract", about = "Extract resources from a Flash SWF")]
struct Args {
    /// Path to the SWF file (FWS/CWS/ZWS).
    swf: PathBuf,

    /// Output directory.
    #[arg(long, default_value = "extracted")]
    out: PathBuf,

    /// Dump embedded binary data (default when no other action is given).
    #[arg(long)]
    blobs: bool,

    /// Render a symbol to PNG(s). Repeatable.
    #[arg(long)]
    symbol: Vec<String>,

    /// Render every exported symbol to PNG(s).
    #[arg(long)]
    all_symbols: bool,

    /// Print symbols and binary blob ids, then exit.
    #[arg(long)]
    list: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let lib = SwfLibrary::open(&args.swf)
        .with_context(|| format!("failed to open {}", args.swf.display()))?;

    if args.list {
        let mut names: Vec<_> = lib.symbols.iter().collect();
        names.sort_by(|a, b| a.0.cmp(b.0));
        println!("{} symbols:", names.len());
        for (name, id) in names {
            println!("  {id:>5}  {name}");
        }
        let mut ids: Vec<_> = lib.binary_data.keys().collect();
        ids.sort();
        println!("{} binary blobs:", ids.len());
        for id in ids {
            println!("  {id:>5}  ({} bytes)", lib.binary_data[id].len());
        }
        return Ok(());
    }

    let want_blobs = args.blobs || (!args.all_symbols && args.symbol.is_empty());
    std::fs::create_dir_all(&args.out)
        .with_context(|| format!("failed to create {}", args.out.display()))?;

    if want_blobs {
        let blobs = lib.binary_blobs();
        println!("extracting {} binary blobs...", blobs.len());
        for (name, data) in blobs {
            let xml = is_xml(&data);
            let file = args.out.join(format!(
                "{}{}",
                sanitize(&name),
                if xml { ".xml" } else { "" }
            ));
            write_file(&file, &data)?;
        }
    }

    if args.all_symbols {
        let mut names: Vec<_> = lib.symbols.keys().cloned().collect();
        names.sort();
        for name in names {
            render_symbol(&lib, &name, &args.out);
        }
    }
    for name in &args.symbol {
        render_symbol(&lib, name, &args.out);
    }
    Ok(())
}

/// Render one symbol, logging failures instead of aborting the batch.
fn render_symbol(lib: &SwfLibrary, name: &str, out: &Path) {
    match lib.render_symbol(name) {
        Ok(frames) => {
            for (i, frame) in frames.iter().enumerate() {
                let file = out.join(format!("{}_f{:02}.png", sanitize(name), i + 1));
                if let Err(e) = frame.image.save(&file) {
                    eprintln!("error: {name}: cannot write {}: {e}", file.display());
                }
            }
            println!("{name}: {} frame(s)", frames.len());
        }
        Err(e) => eprintln!("error: {name}: {e:#}"),
    }
}

/// Payload looks like XML when it starts with `<` after whitespace/BOM.
fn is_xml(data: &[u8]) -> bool {
    let data = data.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(data);
    let data = data
        .iter()
        .position(|b| !b.is_ascii_whitespace())
        .map(|i| &data[i..])
        .unwrap_or(&[]);
    data.starts_with(b"<")
}

fn write_file(path: &Path, data: &[u8]) -> Result<()> {
    std::fs::write(path, data).with_context(|| format!("failed to write {}", path.display()))
}

/// Keep a symbol name safe for use as a file name.
fn sanitize(name: &str) -> String {
    let out: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '_'
            }
        })
        .collect();
    if out.is_empty() {
        "unnamed".into()
    } else {
        out
    }
}
