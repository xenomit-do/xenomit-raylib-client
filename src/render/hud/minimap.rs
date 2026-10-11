//! Minimap window body: zoomable map centred on the hero, entity /
//! portal / collectable markers, ± zoom + "pp" scale readout and the
//! Map / Galaxy tabs (Galaxy is a stub).

use super::PANEL_BG;
use crate::assets::AssetManager;
use crate::game::input;
use crate::game::{Camera, Game};
use crate::net::SharedWorld;
use crate::render::fonts::Fonts;
use crate::render::npc_color;
use crate::render::sprites::TextureCache;
use crate::ui::chrome::{self, ButtonState, TXT_DIM};
use crate::ui::panels::{self, MinimapTab};
use crate::ui::window::{point_in, Rect};
use raylib::prelude::*;
use xenomit_packet_client::model::EntityKind;

/// "map name   x/y" — the minimap window's title text.
pub(crate) fn title_text(game: &Game, world: &SharedWorld, assets: &AssetManager) -> String {
    let name = assets
        .maps
        .get(game.map_id)
        .map(|m| m.name.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| format!("map {}", game.map_id));
    match world.hero_pos() {
        Some((hx, hy)) => format!("{name}   {:.0}/{:.0}", hx, hy),
        None => name,
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_minimap(
    d: &mut RaylibDrawHandle,
    game: &Game,
    world: &SharedWorld,
    _assets: &AssetManager,
    tex: &TextureCache,
    camera: &Camera,
    fonts: &Fonts,
    body: Rect,
    mouse: (f32, f32),
) {
    let l = panels::minimap_layout(body);
    let view = game.minimap_view(world, body);

    // Footer: zoom ±, scale readout, tabs.
    chrome::draw_button(
        d,
        fonts,
        l.zoom_out,
        "-",
        ButtonState {
            hover: point_in(l.zoom_out, mouse),
            active: false,
            disabled: game.ui.minimap_zoom <= panels::MINIMAP_ZOOM_MIN,
        },
    );
    chrome::draw_button(
        d,
        fonts,
        l.zoom_in,
        "+",
        ButtonState {
            hover: point_in(l.zoom_in, mouse),
            active: false,
            disabled: game.ui.minimap_zoom >= panels::MINIMAP_ZOOM_MAX,
        },
    );
    chrome::label(
        d,
        fonts,
        &format!(
            "x{:.1}  {:.0}pp",
            game.ui.minimap_zoom,
            view.units_per_pixel()
        ),
        l.readout.0,
        l.readout.1 + 3.0,
        9.0,
        TXT_DIM,
    );
    for (tab, rect, label) in [
        (MinimapTab::Map, l.tab_map, "Map"),
        (MinimapTab::Galaxy, l.tab_galaxy, "Galaxy"),
    ] {
        chrome::draw_button(
            d,
            fonts,
            rect,
            label,
            ButtonState {
                hover: point_in(rect, mouse),
                active: game.ui.minimap_tab == tab,
                disabled: false,
            },
        );
    }

    chrome::draw_inset(d, l.map);
    let (mx, my, mw, mh) = l.map;

    if game.ui.minimap_tab == MinimapTab::Galaxy {
        for (i, s) in ["Galaxy overview", "not available yet"].iter().enumerate() {
            let m = fonts.ui.measure_text(s, 10.0, 1.0);
            chrome::label(
                d,
                fonts,
                s,
                mx + (mw - m.x) * 0.5,
                my + mh * 0.5 - 12.0 + i as f32 * 14.0,
                10.0,
                TXT_DIM,
            );
        }
        return;
    }

    let (bw, bh) = game.bounds;
    if bw <= 0.0 || bh <= 0.0 {
        return;
    }
    // Stock always has a backdrop — `MiniMapBackground` falls back to a
    // 1px 0x090909 fill when `displayMiniMapBackgroundImage` is off.
    d.draw_rectangle(mx as i32, my as i32, mw as i32, mh as i32, PANEL_BG);
    if world.settings.display_minimap_background
        && game.preferences.minimap_map_image
        && bw > 0.0
        && bh > 0.0
    {
        // `MiniMapBackground` — resolved `minimap-*`/game.xml art, zoom-
        // aware source rect, alpha-tweened 0.3 s from the first frame.
        if let Some(bg) = game.minimap_bg.filter(|b| b.map_id == game.map_id) {
            let frame = tex
                .get(bg.sprite)
                .filter(|c| !c.frames.is_empty())
                .map(|c| &c.frames[0]);
            if let Some(frame) = frame {
                let alpha = bg
                    .loaded_since
                    .map(|at| (at.elapsed().as_secs_f32() / 0.3).min(1.0))
                    .unwrap_or(1.0);
                let tw = frame.tex.width as f32;
                let th = frame.tex.height as f32;
                let sx = (view.origin.0 / bw * tw).clamp(0.0, (tw - 1.0).max(0.0));
                let sy = (view.origin.1 / bh * th).clamp(0.0, (th - 1.0).max(0.0));
                let sw = (view.span.0 / bw * tw).clamp(1.0, (tw - sx).max(1.0));
                let sh = (view.span.1 / bh * th).clamp(1.0, (th - sy).max(1.0));
                let source = Rectangle::new(sx, sy, sw, sh);
                d.draw_texture_pro(
                    &frame.tex,
                    source,
                    Rectangle::new(mx, my, mw, mh),
                    Vector2::zero(),
                    0.0,
                    Color::new(255, 255, 255, (alpha * 220.0) as u8),
                );
            }
        }
    }
    // No GPU clipping: markers whose centre falls outside the map rect
    // (only possible when zoomed in) are skipped.
    let mm = |wx: f32, wy: f32| view.to_screen(wx, wy);
    let on_map = |p: (f32, f32)| point_in(l.map, p);

    // Camera frustum, clipped to the map.
    let (x0, y0, x1, y1) = camera.visible_rect(0.0);
    let (vx0, vy0) = mm(x0, y0);
    let (vx1, vy1) = mm(x1, y1);
    let (cx0, cy0) = (vx0.max(mx), vy0.max(my));
    let (cx1, cy1) = (vx1.min(mx + mw), vy1.min(my + mh));
    if cx1 > cx0 && cy1 > cy0 {
        d.draw_rectangle_lines_ex(
            Rectangle::new(cx0, cy0, cx1 - cx0, cy1 - cy0),
            1.0,
            Color::new(130, 150, 175, 150),
        );
    }

    for e in world.world.entities.values() {
        if e.kind != EntityKind::Asset || e.destroyed {
            continue;
        }
        let Some(v) = game.entity_visuals.get(&e.id) else {
            continue;
        };
        let (px, py) = mm(e.x as f32, e.y as f32);
        if !on_map((px, py)) {
            continue;
        }
        let icon = v
            .minimap_icon
            .and_then(|id| tex.get(id))
            .filter(|c| !c.frames.is_empty());
        if let Some(clip) = icon {
            let b = clip.bounds;
            TextureCache::draw_frame_offset(
                d,
                clip,
                0,
                (px, py),
                0.0,
                (1.0, 1.0),
                Color::WHITE,
                (-(b.0 + b.2) * 0.5, -(b.1 + b.3) * 0.5),
            );
        } else if let Some(c) = v.minimap_color {
            d.draw_rectangle(px as i32 - 1, py as i32 - 1, 3, 3, mm_color(c));
        }
    }
    for poi in world.world.pois.values() {
        if poi.coordinates.len() >= 2 {
            let (px, py) = mm(poi.coordinates[0] as f32, poi.coordinates[1] as f32);
            if !on_map((px, py)) {
                continue;
            }
            d.draw_rectangle(
                px as i32 - 2,
                py as i32 - 2,
                4,
                4,
                Color::new(90, 225, 235, 230),
            );
        }
    }
    for p in world.portals.values() {
        if !p.on_minimap {
            continue;
        }
        let (px, py) = mm(p.x as f32, p.y as f32);
        if !on_map((px, py)) {
            continue;
        }
        let icon = game
            .portal_visuals
            .get(&p.id)
            .and_then(|v| v.minimap_icon)
            .and_then(|id| tex.get(id))
            .filter(|c| !c.frames.is_empty());
        if let Some(clip) = icon {
            let b = clip.bounds;
            TextureCache::draw_frame_offset(
                d,
                clip,
                0,
                (px, py),
                0.0,
                (1.0, 1.0),
                Color::WHITE,
                (-(b.0 + b.2) * 0.5, -(b.1 + b.3) * 0.5),
            );
        } else {
            d.draw_ring(
                Vector2::new(px, py),
                2.5,
                4.0,
                0.0,
                360.0,
                20,
                Color::new(150, 240, 255, 240),
            );
        }
    }
    for c in world.world.collectables.values() {
        let (px, py) = mm(c.x as f32, c.y as f32);
        if on_map((px, py)) {
            d.draw_pixel(px as i32, py as i32, Color::new(255, 225, 90, 220));
        }
    }
    let hero_id = world.world.hero.as_ref().map(|h| h.user_id);
    let hero_faction = world.world.hero.as_ref().map(|h| h.faction_id).unwrap_or(0);
    for e in world.world.entities.values() {
        if Some(e.id) == hero_id || e.kind != EntityKind::Ship {
            continue;
        }
        if world.pets.is_hidden(e.id) {
            continue;
        }
        let p = game.draw_pos(world, e.id);
        let (px, py) = mm(p.0, p.1);
        if !on_map((px, py)) {
            continue;
        }
        let c = Vector2::new(px, py);
        // Uber `miniMapIcon` / `color` on the ship's own recipe wins over
        // the faction dot (rare for ships — mostly event ships).
        let visual = game.entity_visuals.get(&e.id);
        let uber_icon = visual
            .and_then(|v| v.minimap_icon)
            .and_then(|id| tex.get(id))
            .filter(|c| !c.frames.is_empty());
        let uber_color = visual.and_then(|v| v.minimap_color).map(mm_color);
        if e.destroyed {
            d.draw_rectangle(
                px as i32 - 1,
                py as i32 - 1,
                3,
                3,
                Color::new(90, 92, 100, 180),
            );
        } else if e.cloaked {
            d.draw_rectangle(
                px as i32 - 1,
                py as i32 - 1,
                3,
                3,
                Color::new(120, 120, 140, 120),
            );
        } else if let Some(clip) = uber_icon {
            let b = clip.bounds;
            TextureCache::draw_frame_offset(
                d,
                clip,
                0,
                (px, py),
                0.0,
                (1.0, 1.0),
                Color::WHITE,
                (-(b.0 + b.2) * 0.5, -(b.1 + b.3) * 0.5),
            );
        } else if let Some(col) = uber_color {
            d.draw_rectangle(px as i32 - 1, py as i32 - 1, 4, 4, col);
        } else if e.npc {
            d.draw_circle_v(c, 1.6, npc_color());
        } else {
            let col = match input::plate_kind(false, e.faction_id, hero_faction) {
                input::PlateKind::Ally => Color::new(110, 170, 255, 255),
                input::PlateKind::Neutral => Color::new(170, 180, 195, 255),
                _ => Color::new(255, 80, 70, 255),
            };
            d.draw_rectangle(px as i32 - 1, py as i32 - 1, 4, 4, col);
        }
        if game.selected == Some(e.id) {
            d.draw_rectangle_lines(
                px as i32 - 3,
                py as i32 - 3,
                7,
                7,
                Color::new(255, 214, 80, 230),
            );
        }
    }
    if let Some(hp) = world.hero_pos() {
        let (px, py) = mm(hp.0, hp.1);
        if on_map((px, py)) {
            d.draw_rectangle(
                px as i32 - 2,
                py as i32 - 2,
                4,
                4,
                Color::new(80, 255, 130, 255),
            );
            d.draw_rectangle_lines(
                px as i32 - 3,
                py as i32 - 3,
                7,
                7,
                Color::new(240, 250, 255, 240),
            );
        }
    }
    let flags = world.world.map_flags;
    let mut tag = String::new();
    if flags.in_radiation_zone {
        tag.push_str("RAD");
    }
    if flags.in_station {
        if !tag.is_empty() {
            tag.push_str("  ");
        }
        tag.push_str("STATION");
    }
    if !tag.is_empty() {
        let m = fonts.ui.measure_text(&tag, 10.0, 1.0);
        chrome::label(
            d,
            fonts,
            &tag,
            mx + mw - m.x - 5.0,
            my + 3.0,
            10.0,
            Color::new(255, 150, 90, 240),
        );
    }
}

fn mm_color(c: u32) -> Color {
    Color::new(
        ((c >> 16) & 0xff) as u8,
        ((c >> 8) & 0xff) as u8,
        (c & 0xff) as u8,
        255,
    )
}
