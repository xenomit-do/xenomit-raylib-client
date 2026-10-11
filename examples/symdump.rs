//! Dump the exported symbol names of one or more SWF files.
//!
//! usage: `cargo run --example symdump -- <swf> [<swf>…] [substr]`
//! With a trailing `substr` argument (no .swf extension) only symbols
//! containing it are listed. `substr` = `tree:<symbol>` dumps that
//! symbol's first-frame display list (child names, ids, positions).

use xenomit_client::swf_extract::SwfLibrary;

fn main() {
    let mut args = std::env::args().skip(1).collect::<Vec<_>>();
    let filter = match args.last() {
        Some(a)
            if a.starts_with("png:")
                || a.starts_with("id:")
                || a.starts_with("tree:")
                || (!a.to_lowercase().ends_with(".swf")
                    && !a.contains('\\')
                    && !a.contains('/')
                    && !std::path::Path::new(a).exists()) =>
        {
            args.pop()
        }
        _ => None,
    };
    for path in args {
        let Ok(bytes) = std::fs::read(&path) else {
            eprintln!("{path}: unreadable");
            continue;
        };
        match SwfLibrary::from_bytes(&bytes) {
            Ok(lib) => {
                if let Some(tree) = filter.as_deref().and_then(|f| f.strip_prefix("tree:")) {
                    dump_tree(&bytes, &lib, tree);
                    continue;
                }
                if filter.as_deref() == Some("grids") {
                    dump_grids(&bytes);
                    continue;
                }
                if let Some(cid) = filter
                    .as_deref()
                    .and_then(|f| f.strip_prefix("id:"))
                    .and_then(|f| f.parse::<u16>().ok())
                {
                    dump_id(&bytes, &lib, cid);
                    continue;
                }
                // `png:<char-id|symbol>:<out.png>` — rasterize one character
                // to a PNG for visual inspection.
                if let Some(spec) = filter.as_deref().and_then(|f| f.strip_prefix("png:")) {
                    if let Some((key, out)) = spec.split_once(':') {
                        let id = key
                            .parse::<u16>()
                            .ok()
                            .or_else(|| lib.symbols.get(key).copied());
                        match id.map(|i| lib.render_character(i)) {
                            Some(Ok(frames)) => {
                                for (i, fr) in frames.iter().enumerate() {
                                    let p = if frames.len() > 1 {
                                        out.replace(".png", &format!("_f{i:02}.png"))
                                    } else {
                                        out.to_string()
                                    };
                                    match fr.image.save(&p) {
                                        Ok(()) => println!(
                                            "wrote {p} ({}x{})",
                                            fr.image.width(),
                                            fr.image.height()
                                        ),
                                        Err(e) => eprintln!("save {p}: {e}"),
                                    }
                                }
                            }
                            _ => eprintln!("render failed for {key}"),
                        }
                    }
                    continue;
                }
                let mut names: Vec<&String> = lib.symbols.keys().collect();
                names.sort();
                println!("== {path} ({} exports)", names.len());
                for n in names {
                    if filter.as_ref().is_none_or(|f| n.contains(f.as_str())) {
                        println!("   {n}");
                    }
                }
            }
            Err(e) => eprintln!("{path}: {e}"),
        }
    }
}

/// Print the frame-1 display list of `symbol` — child names, character
/// ids, matrices, clip depths (using `swf`'s own parser so `pub(crate)`
/// internals aren't needed).
fn dump_tree(bytes: &[u8], lib: &SwfLibrary, symbol: &str) {
    let Some(id) = lib.symbols.get(symbol).copied() else {
        eprintln!("unknown symbol {symbol}");
        return;
    };
    dump_id(bytes, lib, id);
}

fn dump_id(bytes: &[u8], lib: &SwfLibrary, id: u16) {
    let Ok(buf) = swf::decompress_swf(bytes) else {
        eprintln!("reparse failed");
        return;
    };
    let Ok(parsed) = swf::parse_swf(&buf) else {
        eprintln!("parse failed");
        return;
    };
    for tag in &parsed.tags {
        match tag {
            swf::Tag::DefineSprite(s) if s.id == id => {
                println!("sprite #{id}:");
                dump_frame_tags(lib, &s.tags, 1);
                return;
            }
            swf::Tag::DefineShape(s) if s.id == id => {
                let b = &s.shape_bounds;
                let fills: Vec<String> = s
                    .styles
                    .fill_styles
                    .iter()
                    .map(|f| match f {
                        swf::FillStyle::Color(c) => {
                            format!("rgba({},{},{},{})", c.r, c.g, c.b, c.a)
                        }
                        swf::FillStyle::Bitmap { id, .. } => format!("bitmap#{id}"),
                        swf::FillStyle::LinearGradient(_) => "lgrad".into(),
                        swf::FillStyle::RadialGradient(_) => "rgrad".into(),
                        _ => "other".into(),
                    })
                    .collect();
                println!(
                    "shape #{id}: bounds=({:.1},{:.1},{:.1},{:.1}) fills={fills:?}",
                    b.x_min.to_pixels(),
                    b.y_min.to_pixels(),
                    b.x_max.to_pixels(),
                    b.y_max.to_pixels()
                );
                return;
            }
            _ => {}
        }
    }
    eprintln!("char id {id}: not a sprite or shape (or not found)");
}

/// List every `DefineScalingGrid` tag: char id → splitter rect (px).
fn dump_grids(bytes: &[u8]) {
    let Ok(buf) = swf::decompress_swf(bytes) else {
        eprintln!("reparse failed");
        return;
    };
    let Ok(parsed) = swf::parse_swf(&buf) else {
        eprintln!("parse failed");
        return;
    };
    for tag in &parsed.tags {
        if let swf::Tag::DefineScalingGrid { id, splitter_rect } = tag {
            println!(
                "scale9 char={id} rect=({:.1},{:.1})-({:.1},{:.1})",
                splitter_rect.x_min.to_pixels(),
                splitter_rect.y_min.to_pixels(),
                splitter_rect.x_max.to_pixels(),
                splitter_rect.y_max.to_pixels()
            );
        }
    }
}

fn dump_frame_tags(lib: &SwfLibrary, tags: &[swf::Tag<'_>], level: usize) {
    let pad = "   ".repeat(level);
    for tag in tags {
        match tag {
            swf::Tag::PlaceObject(po) => {
                let Some(char_id) = place_char(po) else {
                    continue;
                };
                let m = po.matrix.unwrap_or_default();
                let name = po
                    .name
                    .map(|n| n.to_string_lossy(swf::WINDOWS_1252))
                    .unwrap_or_default();
                let kind = lib
                    .symbols
                    .iter()
                    .find(|(_, v)| **v == char_id)
                    .map(|(k, _)| k.clone())
                    .unwrap_or_default();
                println!(
                    "{pad}depth={:>3} char={:<4} {:<28} clip={:?} @({:.1},{:.1}) {kind}",
                    po.depth,
                    char_id,
                    name,
                    po.clip_depth,
                    m.tx.get() as f32 / 20.0,
                    m.ty.get() as f32 / 20.0,
                );
            }
            swf::Tag::ShowFrame => break,
            _ => {}
        }
    }
}

fn place_char(po: &swf::PlaceObject<'_>) -> Option<u16> {
    match po.action {
        swf::PlaceObjectAction::Place(id) | swf::PlaceObjectAction::Replace(id) => Some(id),
        swf::PlaceObjectAction::Modify => None,
    }
}
