//! SWF -> raster extraction.
//!
//! Parses a Flash SWF (FWS/CWS/ZWS) via the `swf` crate and rasterizes
//! library symbols (DefineSprite timelines, shapes, bitmaps, text) into
//! PNG-ready RGBA frames. Mirrors what the Flash client's `SWFFinisher`
//! produces when it instantiates embedded symbols: a `MovieClip` symbol
//! becomes a frame sequence (rotation strips for ships), a `BitmapAsset`
//! becomes a single frame.
//!
//! Used both by the `do-extract` CLI (offline asset dumps) and by the
//! runtime asset pipeline (lazy conversion of downloaded resource SWFs).

mod bitmap;
mod distill;
mod render;
pub mod sound;
mod video;

use anyhow::{anyhow, Result};
use image::RgbaImage;
use std::collections::HashMap;
use std::path::Path;
use swf::extensions::ReadSwfExt;

pub(crate) use render::CharDef;

/// One rendered frame of a symbol.
#[derive(Clone, Debug)]
pub struct Frame {
    /// RGBA pixels, cropped to the frame's bounds.
    pub image: RgbaImage,
    /// Top-left corner of `image` relative to the symbol origin, in pixels.
    /// Draw the frame at `entity_pos + offset` to reproduce Flash placement.
    pub offset: (f32, f32),
}

/// A parsed SWF: character table + symbol-name -> character id map.
///
/// The decompressed SWF body (`SwfBuf`) is kept owned inside the library.
/// `swf::Tag<'a>` borrows from that buffer, so instead of storing tags (which
/// would require a self-referential struct) each method re-walks the tag
/// stream on demand — parsing is cheap for game resource SWFs.
pub struct SwfLibrary {
    /// SymbolClass / ExportAssets name -> character id.
    pub symbols: HashMap<String, u16>,
    /// character id -> raw DefineBinaryData payload.
    pub binary_data: HashMap<u16, Vec<u8>>,
    /// Owned decompressed tag stream; re-parsed inside each method.
    buf: swf::SwfBuf,
    /// `DefineScalingGrid` splitter rects per character id, px
    /// `(x, y, w, h)` — authored scale9 grids (window skins, panels).
    grids: HashMap<u16, (f32, f32, f32, f32)>,
    /// Movie clip frame rate (for animated clips).
    pub frame_rate: f32,
}

impl SwfLibrary {
    /// Parse a SWF file from disk.
    pub fn open(path: &Path) -> Result<Self> {
        Self::from_bytes(&std::fs::read(path)?)
    }

    /// Parse a SWF from in-memory bytes.
    ///
    /// Handles FWS (uncompressed), CWS (zlib) and ZWS (LZMA, via the `swf`
    /// crate's `lzma` feature) signatures.
    pub fn from_bytes(data: &[u8]) -> Result<Self> {
        let buf = swf::read::decompress_swf(data)
            .map_err(|e| anyhow!("failed to decompress SWF: {e}"))?;
        let frame_rate = buf.header.frame_rate().to_f32();

        // Walk the top-level tags once to collect the export table and the
        // embedded binary blobs. Malformed tags are skipped, not fatal.
        let mut symbols = HashMap::new();
        let mut binary_data = HashMap::new();
        let mut grids = HashMap::new();
        for tag in &collect_tags(&buf) {
            match tag {
                swf::Tag::SymbolClass(links) => {
                    for link in links {
                        symbols
                            .insert(link.class_name.to_string_lossy(encoding_for(&buf)), link.id);
                    }
                }
                swf::Tag::ExportAssets(assets) => {
                    for asset in assets {
                        symbols.insert(asset.name.to_string_lossy(encoding_for(&buf)), asset.id);
                    }
                }
                swf::Tag::DefineBinaryData(bin) => {
                    binary_data.insert(bin.id, bin.data.to_vec());
                }
                swf::Tag::DefineScalingGrid { id, splitter_rect } => {
                    grids.insert(
                        *id,
                        (
                            splitter_rect.x_min.to_pixels() as f32,
                            splitter_rect.y_min.to_pixels() as f32,
                            (splitter_rect.x_max - splitter_rect.x_min).to_pixels() as f32,
                            (splitter_rect.y_max - splitter_rect.y_min).to_pixels() as f32,
                        ),
                    );
                }
                _ => {}
            }
        }

        Ok(Self {
            symbols,
            binary_data,
            buf,
            grids,
            frame_rate,
        })
    }

    /// `VersionHash.VERSION_HASH` — the login version stamped into each
    /// `main.swf` build (`net.bigpoint.darkorbit.comHash.VersionHash`).
    /// In the DoABC constant pool the name and its 32-hex value are
    /// adjacent strings (`\x0cVERSION_HASH ` + 32 hex chars — the ` `
    /// is the pool's 0x20 length byte). Verified on two builds:
    /// fixture `43e918…`, live `bd171f…`.
    pub fn version_hash(&self) -> Option<String> {
        const NAME: &[u8] = b"VERSION_HASH";
        let data = &self.buf.data[..];
        let mut at = 0;
        while let Some(off) = data[at..].windows(NAME.len()).position(|w| w == NAME) {
            let start = at + off + NAME.len();
            let end = (start + 64).min(data.len());
            for i in start..end {
                let candidate = &data[i..i.saturating_add(32).min(data.len())];
                if candidate.len() == 32
                    && candidate
                        .iter()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(b))
                {
                    return String::from_utf8(candidate.to_vec()).ok();
                }
            }
            at = start;
        }
        None
    }

    /// `DefineSound` tag of an exported sound class (`SymbolClass`/
    /// `ExportAssets` name -> character id), for the audio SWFs. The
    /// returned struct borrows the tag buffer.
    pub fn define_sound(&self, name: &str) -> Option<swf::Sound<'_>> {
        let id = *self.symbols.get(name)?;
        collect_tags(&self.buf)
            .into_iter()
            .find_map(|t| match t {
                swf::Tag::DefineSound(s) if s.id == id => Some(s),
                _ => None,
            })
    }

    /// `name` -> character id. Besides plain export names, a `>`-joined
    /// path descends into named display-list children:
    /// `"backgroundWithTopControls>closeBtn>background"` resolves the
    /// `closeBtn` instance inside the container, then its `background`
    /// child — the same lookup `SWFFinisher.getChildByName` chains do.
    /// Only the first frame of each sprite is searched.
    fn resolve_path(&self, name: &str) -> Option<u16> {
        if !name.contains('>') {
            return self.symbols.get(name).copied();
        }
        let mut parts = name.split('>');
        let mut id = *self.symbols.get(parts.next()?)?;
        let lib = ParsedLib::new(&self.buf);
        for part in parts {
            let Some(CharDef::Sprite(sprite)) = lib.def(id) else {
                return None;
            };
            let mut next = None;
            for tag in &sprite.tags {
                match tag {
                    swf::Tag::PlaceObject(po) => {
                        let placed = match po.action {
                            swf::PlaceObjectAction::Place(c)
                            | swf::PlaceObjectAction::Replace(c) => Some(c),
                            swf::PlaceObjectAction::Modify => None,
                        };
                        let named = po
                            .name
                            .map(|n| n.to_string_lossy(encoding_for(&self.buf)))
                            .unwrap_or_default();
                        if named == part {
                            next = placed.or(next);
                        }
                    }
                    swf::Tag::ShowFrame => break,
                    _ => {}
                }
            }
            id = next?;
        }
        Some(id)
    }

    /// Authored `DefineScalingGrid` splitter rect `(x, y, w, h)` in px for
    /// a symbol or `>` path — `Some` on scale9 skins (e.g. the `window`
    /// finisher's chrome sprite).
    pub fn scaling_grid(&self, name: &str) -> Option<(f32, f32, f32, f32)> {
        self.grids.get(&self.resolve_path(name)?).copied()
    }

    /// Rasterize a library symbol to one or more frames.
    /// Sprites render every timeline frame; shapes/bitmaps render one.
    /// `name` may be an export name or a `parent>child>…` path.
    pub fn render_symbol(&self, name: &str) -> Result<Vec<Frame>> {
        let id = self
            .resolve_path(name)
            .ok_or_else(|| anyhow!("unknown symbol {name}"))?;
        self.render_character(id)
    }

    /// Rasterize a character id directly.
    pub fn render_character(&self, character_id: u16) -> Result<Vec<Frame>> {
        let lib = ParsedLib::new(&self.buf);
        let renderer = render::Renderer::new(&lib);
        renderer.render_character(character_id)
    }

    /// Temporary diagnostic: dump the character table + every sprite's
    /// frame/tag summary to stdout.
    pub fn debug_dump(&self) {
        let lib = ParsedLib::new(&self.buf);
        let mut kinds: std::collections::HashMap<&'static str, usize> = HashMap::new();
        for tag in &lib.tags {
            let k = match tag {
                swf::Tag::DefineShape(_) => "shape",
                swf::Tag::DefineSprite(_) => "sprite",
                swf::Tag::DefineBits { .. } => "bits",
                swf::Tag::DefineBitsJpeg2 { .. } => "bits2",
                swf::Tag::DefineBitsJpeg3(_) => "bits3",
                swf::Tag::DefineBitsLossless(_) => "lossless",
                swf::Tag::DefineText(_) | swf::Tag::DefineText2(_) => "text",
                swf::Tag::PlaceObject(_) => "place",
                swf::Tag::RemoveObject(_) => "remove",
                swf::Tag::ShowFrame => "showframe",
                swf::Tag::SymbolClass(_) => "symclass",
                swf::Tag::ExportAssets(_) => "exports",
                swf::Tag::DefineBinaryData(_) => "bin",
                swf::Tag::DefineMorphShape(_) => "morph",
                swf::Tag::DefineVideoStream(_) => "video",
                swf::Tag::DefineSound(_)
                | swf::Tag::SoundStreamHead(_)
                | swf::Tag::SoundStreamHead2(_) => "sound",
                swf::Tag::DoAbc(_) => "abc",
                _ => "other",
            };
            *kinds.entry(k).or_default() += 1;
        }
        let mut v: Vec<_> = kinds.iter().collect();
        v.sort();
        eprintln!("top-level tags: {v:?}");
        // Bitmap-fill ids each shape references + whether defs knows them.
        for tag in &lib.tags {
            if let swf::Tag::DefineShape(s) = tag {
                let mut fills = Vec::new();
                for st in &s.styles.fill_styles {
                    if let swf::FillStyle::Bitmap { id, .. } = st {
                        fills.push((*id, lib.defs.contains_key(id)));
                    }
                }
                let b = &s.shape_bounds;
                let eb = &s.edge_bounds;
                eprintln!(
                    "shape {} v{} bounds=({},{},{},{}) edges=({},{},{},{}) recs={} fills={fills:?}",
                    s.id,
                    s.version,
                    b.x_min.to_pixels(),
                    b.y_min.to_pixels(),
                    b.x_max.to_pixels(),
                    b.y_max.to_pixels(),
                    eb.x_min.to_pixels(),
                    eb.y_min.to_pixels(),
                    eb.x_max.to_pixels(),
                    eb.y_max.to_pixels(),
                    s.shape.len()
                );
                if s.id == 2 {
                    for (i, r) in s.shape.iter().enumerate() {
                        eprintln!("  rec{i}: {r:?}");
                    }
                    for (i, st) in s.styles.fill_styles.iter().enumerate() {
                        eprintln!("  fill[{i}]: {st:?}");
                    }
                }
            }
        }
        for tag in &lib.tags {
            if let swf::Tag::DefineSprite(s) = tag {
                eprintln!("sprite {} frames={}", s.id, s.num_frames);
                let mut counts: std::collections::HashMap<&'static str, usize> = HashMap::new();
                let mut frame = 0usize;
                for t in &s.tags {
                    match t {
                        swf::Tag::ShowFrame => frame += 1,
                        swf::Tag::PlaceObject(p) => {
                            *counts.entry("place").or_default() += 1;
                            eprintln!(
                                "  f{frame}: place v{} action={:?} depth={} matrix={} cxform={}",
                                p.version,
                                p.action,
                                p.depth,
                                p.matrix.is_some(),
                                p.color_transform.is_some()
                            );
                        }
                        swf::Tag::RemoveObject(r) => {
                            eprintln!(
                                "  f{frame}: remove id={:?} depth={}",
                                r.character_id, r.depth
                            );
                        }
                        swf::Tag::FrameLabel(l) => {
                            eprintln!(
                                "  f{frame}: label {:?}",
                                l.label.to_string_lossy(encoding_for(&self.buf))
                            );
                        }
                        _ => {
                            *counts.entry("other").or_default() += 1;
                        }
                    }
                }
                eprintln!("  sprite-tag totals: {counts:?}");
            }
        }
    }

    /// Extract every DefineBinaryData payload, keyed by symbol name
    /// when the character id is exported, else `bin_<id>`.
    pub fn binary_blobs(&self) -> Vec<(String, Vec<u8>)> {
        // id -> exported name (first wins; ids are unique per name).
        let mut names: HashMap<u16, &String> = HashMap::new();
        for (name, id) in &self.symbols {
            names.entry(*id).or_insert(name);
        }
        let mut out: Vec<(String, Vec<u8>)> = self
            .binary_data
            .iter()
            .map(|(id, data)| {
                let name = names
                    .get(id)
                    .map(|s| (*s).clone())
                    .unwrap_or_else(|| format!("bin_{id}"));
                (name, data.clone())
            })
            .collect();
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }
}

/// The string encoding suggested by the SWF version (UTF-8 for SWF >= 6).
fn encoding_for(buf: &swf::SwfBuf) -> &'static swf::Encoding {
    swf::SwfStr::encoding_for_version(buf.header.version())
}

/// Collect the top-level tag list into an owned `Vec`, tolerating
/// malformed tags.
///
/// `parse_swf` aborts the whole tag list on the first bad tag; game resource
/// SWFs are occasionally sloppy, so this loops manually and skips errors.
/// Tags borrow from `buf` and are returned owned — the caller keeps them
/// alive for as long as `CharDef`s need to reference their fields.
pub(crate) fn collect_tags<'a>(buf: &'a swf::SwfBuf) -> Vec<swf::Tag<'a>> {
    let mut reader = swf::read::Reader::new(&buf.data[..], buf.header.version());
    let mut tags = Vec::new();
    let mut errors = 0u32;
    // A tag header is at least 2 bytes; stop when the stream is exhausted.
    while reader.as_mut_slice().len() >= 2 && errors < 1024 {
        match reader.read_tag() {
            Ok(swf::Tag::End) => break,
            Ok(tag) => tags.push(tag),
            Err(e) => {
                errors += 1;
                if errors <= 8 {
                    log::debug!("skipping malformed SWF tag: {e}");
                }
            }
        }
    }
    tags
}

/// A per-call view over the parsed tag stream: character table + shared tables.
///
/// Owns the parsed `Tag<'a>`s (which borrow from `SwfLibrary::buf`); the
/// character table indexes into `tags` so `CharDef`s borrow from `self`
/// instead of outliving a callback — no leaking, no self-references.
pub(crate) struct ParsedLib<'a> {
    /// Parsed top-level tags, kept alive for `CharDef` borrows.
    tags: Vec<swf::Tag<'a>>,
    /// character id -> index into `tags` holding its definition.
    defs: HashMap<u16, usize>,
    /// Shared JPEG encoding tables (DefineBits v1 payloads reference them).
    pub jpeg_tables: Option<&'a [u8]>,
    /// Video stream id -> encoded `VideoFrame` payloads, sorted by frame
    /// number. VideoFrame tags can appear at top level or inside a
    /// DefineSprite's control tags (bonus boxes keep them in `mc`).
    video_frames: HashMap<u16, Vec<(u16, &'a [u8])>>,
}

impl<'a> ParsedLib<'a> {
    fn new(buf: &'a swf::SwfBuf) -> Self {
        let mut lib = ParsedLib {
            tags: collect_tags(buf),
            defs: HashMap::new(),
            jpeg_tables: None,
            video_frames: HashMap::new(),
        };
        collect_video_frames(&lib.tags, &mut lib.video_frames);
        for frames in lib.video_frames.values_mut() {
            frames.sort_by_key(|(n, _)| *n);
        }
        for (i, tag) in lib.tags.iter().enumerate() {
            match tag {
                swf::Tag::DefineShape(s) => {
                    lib.defs.insert(s.id, i);
                }
                swf::Tag::DefineSprite(s) => {
                    lib.defs.insert(s.id, i);
                }
                swf::Tag::DefineBits { id, .. } | swf::Tag::DefineBitsJpeg2 { id, .. } => {
                    lib.defs.insert(*id, i);
                }
                swf::Tag::DefineBitsJpeg3(bits) => {
                    lib.defs.insert(bits.id, i);
                }
                swf::Tag::DefineBitsLossless(bits) => {
                    lib.defs.insert(bits.id, i);
                }
                swf::Tag::DefineVideoStream(v) => {
                    lib.defs.insert(v.id, i);
                }
                swf::Tag::JpegTables(data) => {
                    lib.jpeg_tables = Some(*data);
                }
                // Character kinds we cannot rasterize are not indexed:
                // `def` returns `None`, callers report "unsupported".
                swf::Tag::DefineText(_)
                | swf::Tag::DefineText2(_)
                | swf::Tag::DefineEditText(_) => {
                    log::debug!("swf_extract: text characters are not rendered");
                }
                _ => {}
            }
        }
        lib
    }

    /// Character definition lookup.
    pub fn def<'b>(&'b self, id: u16) -> Option<CharDef<'b>>
    where
        'a: 'b,
    {
        match self.tags.get(*self.defs.get(&id)?) {
            Some(swf::Tag::DefineShape(s)) => Some(CharDef::Shape(s)),
            Some(swf::Tag::DefineSprite(s)) => Some(CharDef::Sprite(s)),
            Some(swf::Tag::DefineBits { jpeg_data, .. }) => Some(CharDef::Bits {
                data: jpeg_data,
                alpha: None,
                needs_tables: true,
            }),
            Some(swf::Tag::DefineBitsJpeg2 { jpeg_data, .. }) => Some(CharDef::Bits {
                data: jpeg_data,
                alpha: None,
                needs_tables: false,
            }),
            Some(swf::Tag::DefineBitsJpeg3(bits)) => Some(CharDef::Bits {
                data: bits.data,
                alpha: Some(bits.alpha_data),
                needs_tables: false,
            }),
            Some(swf::Tag::DefineBitsLossless(bits)) => Some(CharDef::Lossless(bits)),
            Some(swf::Tag::DefineVideoStream(stream)) => Some(CharDef::Video {
                stream,
                frames: self
                    .video_frames
                    .get(&stream.id)
                    .map(|f| &f[..])
                    .unwrap_or(&[]),
            }),
            _ => None,
        }
    }
}

/// Recursively collect `VideoFrame` payloads (top level and nested inside
/// sprite timelines) into `out`, keyed by stream id.
fn collect_video_frames<'a>(tags: &[swf::Tag<'a>], out: &mut HashMap<u16, Vec<(u16, &'a [u8])>>) {
    for tag in tags {
        match tag {
            swf::Tag::VideoFrame(v) => {
                out.entry(v.stream_id)
                    .or_default()
                    .push((v.frame_num, v.data));
            }
            swf::Tag::DefineSprite(s) => collect_video_frames(&s.tags, out),
            _ => {}
        }
    }
}
