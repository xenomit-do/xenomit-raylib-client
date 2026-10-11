//! Map backdrop: starfield + parallax backgrounds + planets —
//! `Background2D`/`starfield` equivalent.
//!
//! Ported semantics (stock `BasicElement2D`/`Planet2D`):
//! - background layer origin: `x = -cam / pFactor + center + shift + pattern.offset`
//! - planet origin: `x = (pos - cam) / pFactor + center`, image centred,
//!   `rotation`/`scale` applied about its own origin.
//! - Everything then scales by camera zoom (the Flash map container's scale).

use crate::assets::mapxml::{MapBackground, MapPlanet};
use crate::assets::tilemap::TileGrid;
use crate::assets::{AssetManager, MapDescriptor};
use crate::game::{Camera, Game};
use crate::render::sprites::TextureCache;
use raylib::prelude::*;

/// Deep-space fill under everything.
const SPACE: Color = Color::new(4, 5, 12, 255);
/// Max layers ordered per frame without allocating (stock maps use <10).
const MAX_LAYERS: usize = 32;

/// Draw the whole backdrop for `game.map_id`: space fill, procedural
/// starfield (always on — it is the fallback for maps without assets),
/// then descriptor backgrounds + planets in layer order.
pub fn draw_backdrop(
    d: &mut RaylibDrawHandle,
    camera: &Camera,
    game: &Game,
    assets: &AssetManager,
    tex: &TextureCache,
) {
    let scene_3d = super::backgrounds3d::ready(game, assets, tex);
    let Some(desc) = assets.maps.get(game.map_id) else {
        d.clear_background(SPACE);
        draw_starfield(d, camera);
        if scene_3d {
            super::backgrounds3d::draw(d, game, assets, tex);
        }
        return;
    };
    // `stage.color = 0x{map.@backgroundColor}` — the per-map deep-space
    // fill (0 when the attr is absent, like stock).
    let bgc = desc.background_color;
    d.clear_background(Color::new(
        (bgc >> 16) as u8,
        (bgc >> 8) as u8,
        bgc as u8,
        255,
    ));
    draw_starfield(d, camera);
    let total = desc.backgrounds.len() + desc.planets.len();
    if total == 0 {
        if scene_3d {
            super::backgrounds3d::draw(d, game, assets, tex);
        }
        return;
    }

    // Merge both lists into one layer-sorted draw order, zero-alloc.
    // Entries: (layer, is_planet, index).
    let mut order = [(0i32, false, 0usize); MAX_LAYERS];
    let n = total.min(MAX_LAYERS);
    let mut slots = 0usize;
    for (i, b) in desc.backgrounds.iter().enumerate() {
        if slots >= n {
            break;
        }
        order[slots] = (b.layer, false, i);
        slots += 1;
    }
    for (i, p) in desc.planets.iter().enumerate() {
        if slots >= n {
            break;
        }
        order[slots] = (p.layer, true, i);
        slots += 1;
    }
    order[..slots].sort_unstable();

    for &(_, is_planet, i) in &order[..slots] {
        if scene_3d && is_planet {
            continue;
        } else if scene_3d {
            let bg = &desc.backgrounds[i];
            if !assets
                .game_xml
                .backgrounds
                .get(&bg.type_id)
                .is_some_and(|p| p.tiled)
            {
                continue;
            }
            draw_background(d, camera, game, assets, tex, desc, i);
        } else if is_planet {
            draw_planet(d, camera, game, tex, &desc.planets[i], i);
        } else {
            draw_background(d, camera, game, assets, tex, desc, i);
        }
    }
    if scene_3d {
        super::backgrounds3d::draw(d, game, assets, tex);
    }
}

/// One background layer (solid fill, tile grid or image), parallax-scrolled.
fn draw_background(
    d: &mut RaylibDrawHandle,
    camera: &Camera,
    game: &Game,
    assets: &AssetManager,
    tex: &TextureCache,
    desc: &MapDescriptor,
    idx: usize,
) {
    let bg = &desc.backgrounds[idx];
    let pat = assets.game_xml.backgrounds.get(&bg.type_id);

    // `content="custom"` — solid-colour layer, no sprite (`CustomBackgroundPattern`).
    if let Some(p) = pat.filter(|p| p.is_custom) {
        let c = p.color;
        d.draw_rectangle(
            0,
            0,
            camera.viewport.0 as i32,
            camera.viewport.1 as i32,
            Color::new((c >> 16) as u8, (c >> 8) as u8, c as u8, 255),
        );
        return;
    }

    let key = format!("bg:{}:{idx}", game.map_id);
    let Some(&sprite) = game.map_visuals.get(&key) else {
        return;
    };
    let Some(clip) = tex.get(sprite) else {
        return;
    };

    // `StarlingTiledMap` layer — the clip's frames are the atlas tiles and
    // `tile_grids` holds the prepared cell->tile-id grid.
    if assets.get(sprite).is_some_and(|c| c.tiles) {
        if let (Some(grid), Some(p)) = (game.tile_grids.get(&key), pat) {
            draw_tile_grid(d, camera, bg, p, grid, clip);
        }
        return;
    }
    let Some(frame) = clip.frames.first() else {
        return;
    };

    // `BasicElement2D`: `offset = shift + pattern.offset` (the Starling
    // tiled path uses only `shift`). `scale` is NOT applied — stock
    // consumes it only in `StarlingTiledMap` (grid extent) and
    // `Planet2D`; the non-tiled view draws its region at 1:1.
    let off_x = bg.shift.0 as f32 + pat.map(|p| p.offset_x as f32).unwrap_or(0.0);
    let off_y = bg.shift.1 as f32 + pat.map(|p| p.offset_y as f32).unwrap_or(0.0);
    // 1/pFactor of camera scroll; pFactor == 0 pins the layer to the screen.
    let inv_pf = if bg.parallax_factor == 0 {
        0.0
    } else {
        1.0 / bg.parallax_factor as f32
    };
    let z = camera.zoom;
    let ox = camera.viewport.0 * 0.5 + (off_x - camera.center.0 * inv_pf) * z;
    let oy = camera.viewport.1 * 0.5 + (off_y - camera.center.1 * inv_pf) * z;
    let scale = z;

    // Quadrant mode (`{resKey}_01.._04` files): each 2048px tile draws at
    // its grid offset with the `region` crop intersected per tile —
    // `BasicElement2D.fixRegion` (which keeps `x,y` and caps `w,h` at
    // 2048; equivalent when the region is origin-anchored, saner when
    // not).
    if game.map_visuals.contains_key(&format!("{key}q1")) {
        for (q, (tx, ty)) in [
            (0.0f32, 0.0f32),
            (2048.0, 0.0),
            (0.0, 2048.0),
            (2048.0, 2048.0),
        ]
        .iter()
        .enumerate()
        {
            let Some(&sprite) = game.map_visuals.get(&format!("{key}q{}", q + 1)) else {
                continue;
            };
            let Some(qclip) = tex.get(sprite) else {
                continue;
            };
            let Some(f) = qclip.frames.first() else {
                continue;
            };
            let (rx, ry, rw, rh) =
                bg.region
                    .unwrap_or((0, 0, f.tex.width + *tx as i32, f.tex.height + *ty as i32));
            // Intersect the layer region with this tile's 2048 cell,
            // expressed in tile-local coordinates.
            let (lx0, ly0) = ((rx as f32).max(*tx) - *tx, (ry as f32).max(*ty) - *ty);
            let (lx1, ly1) = (
                ((rx + rw) as f32).min(*tx + 2048.0) - *tx,
                ((ry + rh) as f32).min(*ty + 2048.0) - *ty,
            );
            let (lw, lh) = (lx1 - lx0, ly1 - ly0);
            if lw <= 0.0 || lh <= 0.0 {
                continue;
            }
            let (tw, th) = (f.tex.width as f32, f.tex.height as f32);
            let (sw, sh) = (lw.min(tw - lx0), lh.min(th - ly0));
            if sw <= 0.0 || sh <= 0.0 {
                continue;
            }
            d.draw_texture_pro(
                &f.tex,
                Rectangle::new(lx0, ly0, sw, sh),
                Rectangle::new(ox + *tx * scale, oy + *ty * scale, sw * scale, sh * scale),
                Vector2::new(0.0, 0.0),
                0.0,
                Color::WHITE,
            );
        }
        return;
    }

    // `region` crops the source rect.
    if let Some((rx, ry, rw, rh)) = bg.region {
        let rx = (rx as f32).clamp(0.0, frame.tex.width as f32);
        let ry = (ry as f32).clamp(0.0, frame.tex.height as f32);
        let rw = (rw as f32).clamp(0.0, frame.tex.width as f32 - rx);
        let rh = (rh as f32).clamp(0.0, frame.tex.height as f32 - ry);
        if rw <= 0.0 || rh <= 0.0 {
            return;
        }
        d.draw_texture_pro(
            &frame.tex,
            Rectangle::new(rx, ry, rw, rh),
            Rectangle::new(ox, oy, rw * scale, rh * scale),
            Vector2::new(0.0, 0.0),
            0.0,
            Color::WHITE,
        );
    } else {
        // Whole-image layer: the clip's own frame offset applies.
        TextureCache::draw_frame(d, clip, 0, (ox, oy), 0.0, scale, Color::WHITE);
    }
}

/// `StarlingTiledMap` layer: `clip.frames` are the atlas tiles, `grid` the
/// prepared cell->tile-id map. Cell (c,r) lands at
/// `(-cam/pFactor + center + offset) + (c*tileW, r*tileH)` — the same
/// parallax math as the image path. Tiles that are not exactly cell-sized
/// are centred on the cell (the `OversizedTilesModifier` split-and-spill
/// reduces to centring for whole frames). `region` is ignored — stock's
/// tiled path does not use it.
fn draw_tile_grid(
    d: &mut RaylibDrawHandle,
    camera: &Camera,
    bg: &MapBackground,
    pat: &crate::assets::gamexml::BackgroundPattern,
    grid: &TileGrid,
    clip: &crate::render::sprites::GpuClip,
) {
    let tw = pat.tile_width as f32;
    let th = pat.tile_height as f32;
    if tw <= 0.0 || th <= 0.0 || clip.frames.is_empty() {
        return;
    }
    let z = camera.zoom;
    // `initTileMap`: `scale != 1` shifts the grid so it stays centred.
    let scale = bg.scale;
    let (off_x, off_y) = if scale != 1.0 && scale > 0.0 {
        (
            -((grid.w - grid.w / scale) / 2.0).round() + bg.shift.0 as f32,
            -((grid.h - grid.h / scale) / 2.0).round() + bg.shift.1 as f32,
        )
    } else {
        (bg.shift.0 as f32, bg.shift.1 as f32)
    };
    let inv_pf = if bg.parallax_factor == 0 {
        0.0
    } else {
        1.0 / bg.parallax_factor as f32
    };
    let ox = camera.viewport.0 * 0.5 + (off_x - camera.center.0 * inv_pf) * z;
    let oy = camera.viewport.1 * 0.5 + (off_y - camera.center.1 * inv_pf) * z;
    // Tiles draw at native px * zoom — `scale` shapes the grid extent and
    // offsets only (the stock TileMap itself is never scaled).
    let dtw = tw * z;
    let dth = th * z;
    // Oversized tiles spill past their cell — cull with the largest frame
    // extent as margin.
    let pad = clip
        .frames
        .iter()
        .map(|f| f.tex.width.max(f.tex.height) as f32)
        .fold(0.0, f32::max)
        * z;
    let n = clip.frames.len() as i32;
    for r in 0..grid.rows {
        let cell_y = oy + r as f32 * dth;
        if cell_y + pad < 0.0 || cell_y - pad > camera.viewport.1 {
            continue;
        }
        for c in 0..grid.cols {
            let id = grid.ids[(r * grid.cols + c) as usize];
            if !(0..n).contains(&id) {
                continue;
            }
            let f = &clip.frames[id as usize];
            let (fw, fh) = (f.tex.width as f32, f.tex.height as f32);
            let dx = ox + c as f32 * dtw + (tw - fw) * 0.5 * z;
            let dy = cell_y + (th - fh) * 0.5 * z;
            let (dw, dh) = (fw * z, fh * z);
            if dx + dw < -1.0
                || dx > camera.viewport.0 + 1.0
                || dy + dh < -1.0
                || dy > camera.viewport.1 + 1.0
            {
                continue;
            }
            d.draw_texture_pro(
                &f.tex,
                Rectangle::new(0.0, 0.0, fw, fh),
                Rectangle::new(dx, dy, dw, dh),
                Vector2::new(0.0, 0.0),
                0.0,
                Color::WHITE,
            );
        }
    }
}

/// One planet: image centred on `(pos - cam)/pFactor`, rotated & scaled.
fn draw_planet(
    d: &mut RaylibDrawHandle,
    camera: &Camera,
    game: &Game,
    tex: &TextureCache,
    planet: &MapPlanet,
    idx: usize,
) {
    let Some(&sprite) = game.map_visuals.get(&format!("pl:{}:{idx}", game.map_id)) else {
        return;
    };
    let Some(clip) = tex.get(sprite) else {
        return;
    };
    let Some(frame) = clip.frames.first() else {
        return;
    };
    let pf = if planet.parallax_factor == 0.0 {
        f32::MAX // effectively fixed to its screen position
    } else {
        planet.parallax_factor
    };
    let z = camera.zoom;
    let px = camera.viewport.0 * 0.5 + (planet.pos.0 as f32 - camera.center.0) / pf * z;
    let py = camera.viewport.1 * 0.5 + (planet.pos.1 as f32 - camera.center.1) / pf * z;
    let scale = z * if planet.scale > 0.0 {
        planet.scale
    } else {
        1.0
    };
    // Cheap reject: skip when fully off-screen.
    let hw = frame.tex.width as f32 * scale * 0.5;
    let hh = frame.tex.height as f32 * scale * 0.5;
    if px + hw < -64.0
        || px - hw > camera.viewport.0 + 64.0
        || py + hh < -64.0
        || py - hh > camera.viewport.1 + 64.0
    {
        return;
    }
    // Centred image rotating about its own middle — `Planet2D` puts the
    // bitmap at `-size/2` inside the rotated view.
    TextureCache::draw_frame_offset(
        d,
        clip,
        0,
        (px, py),
        planet.rotation,
        (scale, scale),
        Color::WHITE,
        (
            -frame.tex.width as f32 * 0.5,
            -frame.tex.height as f32 * 0.5,
        ),
    );
}

/// Cheap deterministic starfield: three parallax layers of hash-placed
/// stars. Always drawn — it is the fallback backdrop when a map has no
/// descriptor or its sprites have not arrived yet.
fn draw_starfield(d: &mut RaylibDrawHandle, camera: &Camera) {
    // (parallax, cell size, base alpha)
    const LAYERS: [(f32, f32, u8); 3] = [(0.22, 95.0, 150), (0.45, 150.0, 110), (0.7, 230.0, 70)];
    let (vw, vh) = camera.viewport;
    let t = d.get_time() as f32;
    for &(q, cell, base_a) in &LAYERS {
        let bx = camera.center.0 * q * camera.zoom;
        let by = camera.center.1 * q * camera.zoom;
        let x0 = (bx / cell).floor() as i64 - 1;
        let y0 = (by / cell).floor() as i64 - 1;
        let x1 = ((bx + vw) / cell).ceil() as i64 + 1;
        let y1 = ((by + vh) / cell).ceil() as i64 + 1;
        for iy in y0..=y1 {
            for ix in x0..=x1 {
                let h = hash_cell(ix, iy);
                let jx = (h & 0xff) as f32 / 255.0 * cell;
                let jy = ((h >> 8) & 0xff) as f32 / 255.0 * cell;
                let sx = ix as f32 * cell + jx - bx;
                let sy = iy as f32 * cell + jy - by;
                if sx < -2.0 || sx > vw + 2.0 || sy < -2.0 || sy > vh + 2.0 {
                    continue;
                }
                // Slow twinkle, deterministic per star.
                let phase = ((h >> 16) & 0xff) as f32 / 255.0 * std::f32::consts::TAU;
                let tw = 0.55 + 0.45 * (t * 1.7 + phase).sin();
                let a = (base_a as f32 * tw) as u8;
                // A few brighter, slightly tinted larger stars.
                let (sz, r, g, b) = if h & 0x2000 != 0 {
                    (2i32, 255u8, 240u8, 210u8)
                } else if h & 0x400 != 0 {
                    (2, 200, 215, 255)
                } else {
                    (1, 235, 235, 245)
                };
                d.draw_rectangle(sx as i32, sy as i32, sz, sz, Color::new(r, g, b, a));
            }
        }
    }
}

/// Integer hash of a grid cell — stable across frames (no per-frame RNG).
fn hash_cell(x: i64, y: i64) -> u32 {
    let mut h =
        (x as u64).wrapping_mul(0x9E3779B97F4A7C15) ^ (y as u64).wrapping_mul(0xC2B2AE3D27D4EB4F);
    h ^= h >> 33;
    h = h.wrapping_mul(0xFF51AFD7ED558CCD);
    h ^= h >> 29;
    (h & 0x7fff_ffff) as u32
}
