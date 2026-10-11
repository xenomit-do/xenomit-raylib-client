//! `StarlingTiledMap` tile maps: a Starling `TextureAtlas` XML + one
//! texture file produce a `tiles == true` clip whose frames are the atlas
//! subtextures, plus the deterministic cell->tile-id grid the renderer
//! walks each frame.
//!
//! Ported semantics (`StarlingTiledMap.as` / `TileMap.as` /
//! `OversizedTilesModifier.as` / `prepareTileMap`):
//!
//! - `<SubTexture>` tiles are indexed in `TextureAtlas.getTextures()`
//!   order — names sorted `Array.CASEINSENSITIVE`, not document order.
//! - `frameX`/`frameY` are negative offsets: the tile's drawn canvas is
//!   `frameWidth`x`frameHeight` with the region placed at `(-frameX,-frameY)`
//!   (Starling frame semantics; falls back to the region size).
//! - Grid size: `pattern.width>0 && pattern.height>0` -> those px; pattern
//!   `order` `ASC`/`DESC` -> `round(map/10*scale)`; else
//!   `round(map/pFactor*scale)` (`tileMapSizeMultiplier` defaults 1).
//!   `cols = round(w/tileW)`, `rows = round(h/tileH)`, min 1.
//! - Cell ids: `order` set -> sequential 1..count (id == count draws
//!   nothing — stock's `tileID < tiles.length` check); else a random id
//!   avoiding equal left/up neighbours (`equalNeighborsAllowed` is always
//!   false for background tile maps). We substitute a deterministic hash of
//!   (seed, cell) for `FastMath` so the grid is stable across frames.
//! - `maskID`: the mask bitmap (`backgroundMasks[type].resKey`) blanks
//!   cells whose cell-centre sample is `0` (out-of-bounds reads count as 0
//!   — `getPixel32` semantics); the x/y ratio both use `width/maskW`
//!   (`_loc11_` — yes, stock uses the width ratio for Y too).

use anyhow::{Context, Result};

use crate::assets::gamexml::BackgroundPattern;
use crate::assets::mapxml::MapBackground;
use crate::assets::sprites::{SpriteClip, SpriteFrame};

/// Tile-frame cap — real atlases hold <100 tiles; bound the allocation.
const MAX_TILE_FRAMES: usize = 2048;
/// Sanity cap for one tile's canvas (frameWidth/Height are untrusted).
const MAX_TILE_CANVAS: i64 = 4096;
/// Grid dimension cap (cols/rows) — bounds the ids allocation and the
/// per-frame draw loop on degenerate inputs.
const MAX_GRID_DIM: i32 = 1024;
/// Re-roll bound for the no-equal-neighbours pick (stock loops forever —
/// count 2 with both neighbours pinned would hang it).
const MAX_PICK_TRIES: u64 = 32;

/// One `<SubTexture>` of a Starling `TextureAtlas` document.
#[derive(Clone, Debug, Default)]
struct AtlasSub {
    name: String,
    /// Region inside the texture (px).
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    /// Negative frame offsets (`-frameX` = paste position inside the
    /// `frameWidth`x`frameHeight` canvas); `frameW/H == 0` means untrimmed.
    frame_x: i32,
    frame_y: i32,
    frame_w: i32,
    frame_h: i32,
}

fn attr_i32(node: &roxmltree::Node, name: &str) -> i32 {
    node.attribute(name)
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or_default()
}

/// Decode an atlas document that may be UTF-16 — the CDN serves the
/// `mapAssetView` atlas XMLs (e.g. `portal_standard_atlas`) as UTF-16LE
/// with a BOM, and `String::from_utf8_lossy` would turn every byte into
/// `U+FFFD`, failing the parse with "invalid texture atlas xml".
fn xml_text(bytes: &[u8]) -> String {
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    if bytes.starts_with(&[0xFF, 0xFE]) {
        let (u16s, _) = bytes[2..].as_chunks::<2>();
        let u: Vec<u16> = u16s.iter().map(|c| u16::from_le_bytes(*c)).collect();
        String::from_utf16_lossy(&u)
    } else if bytes.starts_with(&[0xFE, 0xFF]) {
        let (u16s, _) = bytes[2..].as_chunks::<2>();
        let u: Vec<u16> = u16s.iter().map(|c| u16::from_be_bytes(*c)).collect();
        String::from_utf16_lossy(&u)
    } else {
        String::from_utf8_lossy(bytes).into_owned()
    }
}

/// Parse the atlas document and return subtextures in
/// `TextureAtlas.getTextures()` order (case-insensitive name sort).
fn parse_atlas(xml: &str) -> Result<Vec<AtlasSub>> {
    let doc = roxmltree::Document::parse(xml).context("invalid texture atlas xml")?;
    let mut subs: Vec<AtlasSub> = doc
        .descendants()
        .filter(|n| n.is_element() && n.has_tag_name("SubTexture"))
        .map(|st| AtlasSub {
            name: st.attribute("name").unwrap_or_default().to_string(),
            x: attr_i32(&st, "x"),
            y: attr_i32(&st, "y"),
            w: attr_i32(&st, "width"),
            h: attr_i32(&st, "height"),
            frame_x: attr_i32(&st, "frameX"),
            frame_y: attr_i32(&st, "frameY"),
            frame_w: attr_i32(&st, "frameWidth"),
            frame_h: attr_i32(&st, "frameHeight"),
        })
        .collect();
    if subs.is_empty() {
        anyhow::bail!("texture atlas has no SubTexture entries");
    }
    // `Array.CASEINSENSITIVE` name sort — this defines tile ids.
    subs.sort_by_key(|a| a.name.to_lowercase());
    subs.truncate(MAX_TILE_FRAMES);
    Ok(subs)
}

/// Build the tile clip: every `SubTexture` cropped onto its frame canvas,
/// in atlas order. `kind` is the resources.xml `type` of the texture file.
pub fn tilemap_clip(atlas_bytes: &[u8], kind: &str, texture_bytes: &[u8]) -> Result<SpriteClip> {
    let atlas = xml_text(atlas_bytes);
    let subs = parse_atlas(&atlas)?;
    let img = raster_image(kind, texture_bytes).context("tile texture decode")?;
    let mut frames = Vec::with_capacity(subs.len());
    for s in &subs {
        // Starling frame rect = (-frameX, -frameY, frameW, frameH).
        frames.push(SpriteFrame {
            image: sub_canvas(&img, s),
            offset: (0.0, 0.0),
            duration_ms: 0,
        });
    }
    Ok(SpriteClip {
        frames,
        rotation_strip: false,
        tiles: true,
        scale9: None,
    })
}

/// Single named subtexture → one-frame clip — the `mapAssetView` region
/// lookup (`<asset atlas="x" texture="name">` in the portal recipes).
/// Frame offsets get the same Starling `(-frameX,-frameY)` canvas
/// treatment as tiles; an unknown region name falls back to the first
/// subtexture so callers still get *something* on screen.
pub fn region_clip(
    atlas_bytes: &[u8],
    kind: &str,
    texture_bytes: &[u8],
    region: &str,
) -> Result<SpriteClip> {
    let atlas = xml_text(atlas_bytes);
    let subs = parse_atlas(&atlas)?;
    let img = raster_image(kind, texture_bytes).context("atlas texture decode")?;
    let sub = subs
        .iter()
        .find(|s| s.name == region)
        .or_else(|| subs.iter().find(|s| s.name.eq_ignore_ascii_case(region)))
        .or_else(|| subs.first())
        .context("texture atlas has no SubTexture entries")?;
    Ok(SpriteClip {
        frames: vec![SpriteFrame {
            image: sub_canvas(&img, sub),
            offset: (0.0, 0.0),
            duration_ms: 0,
        }],
        rotation_strip: false,
        tiles: false,
        scale9: None,
    })
}

/// Crop one `AtlasSub` onto its Starling frame canvas
/// (`frameWidth`x`frameHeight` with the region at `(-frameX,-frameY)`).
fn sub_canvas(img: &image::RgbaImage, s: &AtlasSub) -> image::RgbaImage {
    let (iw, ih) = (img.width() as i64, img.height() as i64);
    let x0 = (s.x as i64).clamp(0, iw);
    let y0 = (s.y as i64).clamp(0, ih);
    let w = ((s.w as i64).min(iw - x0)).max(0) as u32;
    let h = ((s.h as i64).min(ih - y0)).max(0) as u32;
    let cw = (if s.frame_w > 0 { s.frame_w } else { s.w } as i64).clamp(1, MAX_TILE_CANVAS) as u32;
    let ch = (if s.frame_h > 0 { s.frame_h } else { s.h } as i64).clamp(1, MAX_TILE_CANVAS) as u32;
    let mut canvas = image::RgbaImage::from_pixel(cw, ch, image::Rgba([0, 0, 0, 0]));
    if w > 0 && h > 0 {
        let crop = image::imageops::crop_imm(img, x0 as u32, y0 as u32, w, h).to_image();
        image::imageops::overlay(&mut canvas, &crop, -(s.frame_x as i64), -(s.frame_y as i64));
    }
    canvas
}

/// Region names a `mapAssetView` layer should show.
///
/// An exact `SubTexture` name is one still (`normal`, `portal_305`). A
/// stem that is not itself a region expands to every name that starts
/// with it (`portal_305_000` → `portal_305_00000..`). An empty prefix is
/// the whole atlas (`texture=""` on the jump clip).
pub(crate) fn map_layer_region_names<'a>(names: &'a [&str], prefix: &str) -> Vec<&'a str> {
    if prefix.is_empty() {
        return names.to_vec();
    }
    if names.iter().any(|n| *n == prefix) {
        return names.iter().copied().filter(|n| *n == prefix).collect();
    }
    names
        .iter()
        .copied()
        .filter(|n| n.starts_with(prefix))
        .collect()
}

/// `mapAssetView` layer clip: one still when `prefix` names a region,
/// otherwise every matching frame (`DOAtlas.getTextures`).
pub fn map_asset_clip(
    atlas_bytes: &[u8],
    kind: &str,
    texture_bytes: &[u8],
    prefix: &str,
    frame_ms: u32,
) -> Result<SpriteClip> {
    let atlas = xml_text(atlas_bytes);
    let subs = parse_atlas(&atlas)?;
    let img = raster_image(kind, texture_bytes).context("atlas texture decode")?;
    let names: Vec<&str> = subs.iter().map(|s| s.name.as_str()).collect();
    let picked = map_layer_region_names(&names, prefix);
    if picked.is_empty() {
        anyhow::bail!("no SubTexture matching prefix {prefix}");
    }
    let duration = if picked.len() == 1 {
        0
    } else if frame_ms == 0 {
        33
    } else {
        frame_ms
    };
    let frames = subs
        .iter()
        .filter(|s| picked.iter().any(|n| *n == s.name))
        .map(|s| SpriteFrame {
            image: sub_canvas(&img, s),
            offset: (0.0, 0.0),
            duration_ms: duration,
        })
        .collect();
    Ok(SpriteClip {
        frames,
        rotation_strip: false,
        tiles: false,
        scale9: None,
    })
}

/// All `SubTexture`s whose name starts with `prefix` → a multi-frame clip —
/// the `animated_base` sub-asset of a `mapAssetView` (e.g. the portal idle
/// rotation `portal_306_000` → `portal_306_00000..29`). `frame_ms` is the
/// recipe's `fps` converted to a frame duration.
pub fn region_clip_prefix(
    atlas_bytes: &[u8],
    kind: &str,
    texture_bytes: &[u8],
    prefix: &str,
    frame_ms: u32,
) -> Result<SpriteClip> {
    let atlas = xml_text(atlas_bytes);
    let subs = parse_atlas(&atlas)?;
    let img = raster_image(kind, texture_bytes).context("atlas texture decode")?;
    let mut frames: Vec<SpriteFrame> = subs
        .iter()
        .filter(|s| s.name.starts_with(prefix))
        .map(|s| SpriteFrame {
            image: sub_canvas(&img, s),
            offset: (0.0, 0.0),
            duration_ms: frame_ms,
        })
        .collect();
    if frames.is_empty() {
        anyhow::bail!("no SubTexture matching prefix {prefix}");
    }
    // The case-insensitive name sort already orders `prefix00..prefix29`;
    // make sure every frame carries a sane duration.
    for f in &mut frames {
        if f.duration_ms == 0 {
            f.duration_ms = 33;
        }
    }
    Ok(SpriteClip {
        frames,
        rotation_strip: false,
        tiles: false,
        scale9: None,
    })
}

/// Decode a whole-file raster image (png/jpg/jxr/wdp) → RGBA8.
pub fn raster_image(kind: &str, bytes: &[u8]) -> Result<image::RgbaImage> {
    let kind = kind.to_ascii_lowercase();
    if kind == "jxr" || kind == "wdp" || bytes.starts_with(b"\x49\x49\xbc\x01") {
        return jxr_image(bytes).or_else(|e| {
            // `type` lied — fall back to the generic decoder.
            image::load_from_memory(bytes)
                .map(|i| i.to_rgba8())
                .with_context(|| format!("jxr decode failed ({e}) and not another image format"))
        });
    }
    Ok(image::load_from_memory(bytes)?.to_rgba8())
}

/// JPEG-XR → RGBA8 through the pinned `jpegxr` crate: `convert_to_tiff`
/// normalizes every pixel format/alpha variant (`uAlphaMode = 2`, like
/// ruffle's ATF jpegxr path) and the `image` crate reads the TIFF.
fn jxr_image(bytes: &[u8]) -> Result<image::RgbaImage> {
    let mut decoder =
        jpegxr::ImageDecode::with_reader(std::io::Cursor::new(bytes)).context("jpegxr init")?;
    let mut tiff = std::io::Cursor::new(Vec::new());
    decoder
        .convert_to_tiff(&mut tiff)
        .context("jpegxr -> tiff convert")?;
    let img = image::ImageReader::with_format(
        std::io::Cursor::new(tiff.into_inner()),
        image::ImageFormat::Tiff,
    )
    .decode()
    .context("tiff decode")?;
    Ok(img.to_rgba8())
}

// ---------------------------------------------------------------------------
// Tile-id grid (`prepareTileMap`)
// ---------------------------------------------------------------------------

/// Precomputed `StarlingTiledMap` cell->tile-id grid for one background
/// layer. `w`/`h` are the layer-space grid size in px (pre-rounding) —
/// `initTileMap` needs them for the `scale != 1` offset compensation.
#[derive(Clone, Debug, Default)]
pub struct TileGrid {
    pub cols: i32,
    pub rows: i32,
    /// Layer-space grid extent in px (`StarlingTiledMap.width/height`).
    pub w: f32,
    pub h: f32,
    /// `rows*cols` tile ids; `-1` = blank cell (masked), `id >= tile_count`
    /// = stock's out-of-range ordered id (also blank).
    pub ids: Vec<i32>,
}

/// `StarlingTiledMap` ctor + `prepareTileMap`: compute the grid dims and
/// every cell's tile id. `mask` is the resolved `maskID` bitmap (single
/// frame); `None` when the pattern has no mask or it could not load.
/// `seed` folds (map id, layer slot) so different layers pick differently.
pub fn build_grid(
    map_size: (f32, f32),
    bg: &MapBackground,
    pat: &BackgroundPattern,
    seed: u64,
    mask: Option<&image::RgbaImage>,
    tile_count: usize,
) -> TileGrid {
    let scale = if bg.scale > 0.0 { bg.scale } else { 1.0 };
    // `this.width/height` — explicit pattern size, `ASC`/`DESC` map/10,
    // else map/pFactor (pFactor <= 0 would divide by zero in stock; clamp).
    let (w, h) = if pat.width > 0 && pat.height > 0 {
        (pat.width as f32, pat.height as f32)
    } else if pat.order == "ASC" || pat.order == "DESC" {
        (
            (map_size.0 / 10.0 * scale).round(),
            (map_size.1 / 10.0 * scale).round(),
        )
    } else {
        let pf = bg.parallax_factor.max(1) as f32;
        (
            (map_size.0 / pf * scale).round(),
            (map_size.1 / pf * scale).round(),
        )
    };
    let tw = pat.tile_width.max(1) as f32;
    let th = pat.tile_height.max(1) as f32;
    let cols = ((w / tw).round() as i32).clamp(1, MAX_GRID_DIM);
    let rows = ((h / th).round() as i32).clamp(1, MAX_GRID_DIM);
    let mut grid = TileGrid {
        cols,
        rows,
        w,
        h,
        ids: vec![-1; (cols as usize).saturating_mul(rows as usize)],
    };
    let count = tile_count as i32;
    if count <= 0 || grid.ids.is_empty() {
        return grid;
    }

    // Masked path ignores `order` entirely (stock picks randomly there).
    let ordered = mask.is_none() && !pat.order.is_empty();
    let mut seq = if pat.order == "DESC" { count } else { 1 };
    let mut prev_row = vec![-1i32; cols as usize];
    for r in 0..rows {
        let mut left = -1i32;
        for c in 0..cols {
            let i = (r * cols + c) as usize;
            if let Some(mask_img) = mask {
                // Cell-centre sample; `_loc11_ = width/maskW` for both axes.
                let ratio = w / mask_img.width().max(1) as f32;
                let mx = ((c as f32 * tw + tw * 0.5) / ratio).round() as i64;
                let my = ((r as f32 * th + th * 0.5) / ratio).round() as i64;
                let px = if mx < 0
                    || my < 0
                    || mx >= mask_img.width() as i64
                    || my >= mask_img.height() as i64
                {
                    0
                } else {
                    u32::from_be_bytes(mask_img.get_pixel(mx as u32, my as u32).0)
                };
                if px == 0 {
                    continue; // stays -1
                }
            }
            let id = if ordered {
                let id = seq;
                if pat.order == "DESC" {
                    seq -= 1;
                    if seq < 1 {
                        seq = count;
                    }
                } else {
                    seq += 1;
                    if seq > count {
                        seq = 1;
                    }
                }
                id
            } else {
                pick_tile(seed, c as i64, r as i64, left, prev_row[c as usize], count)
            };
            grid.ids[i] = id;
            left = id;
            prev_row[c as usize] = id;
        }
    }
    grid
}

/// Deterministic per-cell tile id with stock's no-equal-neighbours rule:
/// re-pick until the id differs from the left and up neighbours
/// (`-1` neighbours never match).
fn pick_tile(seed: u64, c: i64, r: i64, left: i32, up: i32, count: i32) -> i32 {
    if count <= 1 {
        return 0;
    }
    for salt in 0..MAX_PICK_TRIES {
        let p = (hash4(seed, c, r, salt) % count as u64) as i32;
        if (p != left && p != up) || salt == MAX_PICK_TRIES - 1 {
            return p;
        }
    }
    0
}

/// Stable hash of (seed, cell x, cell y, salt) — same family as
/// `render::map::hash_cell`.
fn hash4(seed: u64, x: i64, y: i64, salt: u64) -> u64 {
    let mut h = seed.wrapping_mul(0x9E3779B97F4A7C15)
        ^ (x as u64).wrapping_mul(0xC2B2AE3D27D4EB4F)
        ^ (y as u64).wrapping_mul(0x165667B19E3779F9)
        ^ salt.wrapping_mul(0x85EBCA6B);
    h ^= h >> 33;
    h = h.wrapping_mul(0xFF51AFD7ED558CCD);
    h ^= h >> 29;
    h
}

/// Seed for a layer's tile picks — folds map id + layer slot.
pub fn grid_seed(map_id: i32, layer_idx: usize) -> u64 {
    hash4(0x5449_4C45, map_id as i64, layer_idx as i64, 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sub(name: &str, x: i32, y: i32, w: i32, h: i32) -> String {
        format!(r#"<SubTexture name="{name}" x="{x}" y="{y}" width="{w}" height="{h}"/>"#)
    }

    #[test]
    fn atlas_subtextures_sort_case_insensitive_by_name() {
        let xml = format!(
            "<TextureAtlas>{}{}{}{}</TextureAtlas>",
            sub("tex_10", 0, 0, 8, 8),
            sub("Tex_2", 8, 0, 8, 8),
            sub("tex_1", 16, 0, 8, 8),
            sub("OTHER", 24, 0, 8, 8),
        );
        let subs = parse_atlas(&xml).unwrap();
        let names: Vec<_> = subs.iter().map(|s| s.name.as_str()).collect();
        // Case-insensitive alpha sort: "tex_10" < "tex_2" ('1' < '2').
        assert_eq!(names, ["OTHER", "tex_1", "tex_10", "Tex_2"]);
    }

    fn pattern(tile: i32) -> BackgroundPattern {
        BackgroundPattern {
            tiled: true,
            tile_width: tile,
            tile_height: tile,
            res_key_starling: "tex".into(),
            atlas_xml: "atlas".into(),
            ..Default::default()
        }
    }

    fn bg(pf: i32) -> MapBackground {
        MapBackground {
            parallax_factor: pf,
            ..Default::default()
        }
    }

    #[test]
    fn map_layer_names_prefer_an_exact_region() {
        let names = [
            "portal_305",
            "portal_305_00000",
            "portal_305_00001",
            "portal_305_active",
        ];
        assert_eq!(
            map_layer_region_names(&names, "portal_305"),
            vec!["portal_305"]
        );
        assert_eq!(
            map_layer_region_names(&names, "portal_305_000"),
            vec!["portal_305_00000", "portal_305_00001"]
        );
        assert_eq!(map_layer_region_names(&names, ""), names.to_vec());
    }

    #[test]
    fn grid_dims_follow_starling_tiled_map() {
        // pattern width/height wins outright
        let mut pat = pattern(100);
        pat.width = 1000;
        pat.height = 500;
        let g = build_grid((21000.0, 13100.0), &bg(10), &pat, 1, None, 4);
        assert_eq!((g.cols, g.rows), (10, 5));

        // ASC order -> round(map/10*scale)/tile
        pat.width = 0;
        pat.height = 0;
        pat.order = "ASC".into();
        let g = build_grid((21000.0, 13100.0), &bg(10), &pat, 1, None, 4);
        assert_eq!((g.cols, g.rows), (21, 13));
        // ASC ids run 1..=count and wrap (id == count is a blank in stock).
        assert_eq!(g.ids[0], 1);
        assert_eq!(g.ids[3], 4);
        assert_eq!(g.ids[4], 1);

        pat.order = "DESC".into();
        let g = build_grid((21000.0, 13100.0), &bg(10), &pat, 1, None, 4);
        assert_eq!(g.ids[0], 4); // starts at count (stock draws it blank)
        assert_eq!(g.ids[1], 3);
    }

    #[test]
    fn grid_random_picks_avoid_neighbours_and_are_stable() {
        let pat = pattern(50);
        let b = bg(10);
        let g1 = build_grid((21000.0, 13100.0), &b, &pat, 7, None, 6);
        let g2 = build_grid((21000.0, 13100.0), &b, &pat, 7, None, 6);
        assert_eq!(g1.ids, g2.ids, "deterministic for same seed");
        let g3 = build_grid((21000.0, 13100.0), &b, &pat, 8, None, 6);
        assert_ne!(g1.ids, g3.ids, "seed varies the lattice");
        for r in 0..g1.rows {
            for c in 0..g1.cols {
                let id = g1.ids[(r * g1.cols + c) as usize];
                assert!((0..6).contains(&id));
                if c > 0 {
                    assert_ne!(id, g1.ids[(r * g1.cols + c - 1) as usize]);
                }
                if r > 0 {
                    assert_ne!(id, g1.ids[((r - 1) * g1.cols + c) as usize]);
                }
            }
        }
    }

    #[test]
    fn grid_mask_blanks_zero_cells() {
        let pat = pattern(10);
        // 2x2 mask stretched over a 40x20 grid: left column blank.
        let mut mask = image::RgbaImage::new(2, 2);
        mask.put_pixel(0, 0, image::Rgba([0, 0, 0, 0]));
        mask.put_pixel(1, 0, image::Rgba([255, 255, 255, 255]));
        mask.put_pixel(0, 1, image::Rgba([0, 0, 0, 0]));
        mask.put_pixel(1, 1, image::Rgba([255, 255, 255, 255]));
        let mut p = pat;
        p.width = 40;
        p.height = 20;
        let g = build_grid((0.0, 0.0), &bg(10), &p, 3, Some(&mask), 4);
        assert_eq!((g.cols, g.rows), (4, 2));
        // ratio = 40/2 = 20: col 0 samples mask x=0 (blank), cols 1-2
        // sample x=1 (white), col 3 samples x=2 (out of bounds -> 0).
        for r in 0..2 {
            assert_eq!(g.ids[(r * 4) as usize], -1);
            assert!(g.ids[(r * 4 + 1) as usize] >= 0);
            assert!(g.ids[(r * 4 + 2) as usize] >= 0);
            assert_eq!(g.ids[(r * 4 + 3) as usize], -1);
        }
    }

    /// Real DarkOrbit JXR (blacklight background) — skip when the fixture
    /// is absent so the suite still passes without a network download.
    #[test]
    fn jxr_decodes_real_darkorbit_background() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/blacklight_map-2_background.jxr"
        );
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(_) => return, // fixture not vendored
        };
        let img = jxr_image(&bytes).expect("real .jxr must decode");
        assert!(img.width() > 0 && img.height() > 0);
    }
}
