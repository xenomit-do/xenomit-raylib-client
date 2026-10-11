use crate::assets::AssetManager;
use crate::game::{Camera, Game};
use crate::net::SharedWorld;
use crate::render::fonts::Fonts;
use crate::render::sprites::TextureCache;
use raylib::prelude::*;

pub(crate) fn draw_hints(
    d: &mut RaylibDrawHandle,
    world: &SharedWorld,
    game: &Game,
    fonts: &Fonts,
    vw: f32,
    vh: f32,
) {
    let flags = world.world.map_flags;
    if let Some(note) = &game.reconnect_note {
        centered_banner(
            d,
            fonts,
            note,
            vw * 0.5,
            vh * 0.36,
            16.0,
            Color::new(255, 200, 90, 255),
        );
    }
    if flags.in_radiation_zone {
        let c = Color::new(255, 60, 40, 120);
        d.draw_rectangle_lines_ex(Rectangle::new(2.0, 2.0, vw - 4.0, vh - 4.0), 3.0, c);
    }
    if flags.near_jump_gate {
        centered_banner(
            d,
            fonts,
            "[J]  Jump through gate",
            vw * 0.5,
            vh - 90.0,
            14.0,
            Color::new(140, 235, 255, 235),
        );
    }
    let dead = world
        .world
        .hero
        .as_ref()
        .map(|h| h.hitpoints <= 0.0 && h.hitpoints_max > 0.0)
        .unwrap_or(false);
    if dead {
        centered_banner(
            d,
            fonts,
            "Destroyed — press B / R to repair",
            vw * 0.5,
            vh * 0.42,
            18.0,
            Color::new(255, 90, 70, 255),
        );
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_debug(
    d: &mut RaylibDrawHandle,
    game: &Game,
    world: &SharedWorld,
    assets: &AssetManager,
    tex: &TextureCache,
    fonts: &Fonts,
    camera: &Camera,
    vw: f32,
) {
    let pending = game
        .entity_visuals
        .values()
        .chain(game.collectable_visuals.values())
        .filter(|v| v.sprite.is_some_and(|id| tex.get(id).is_none()))
        .count();
    let cpu_clips = assets
        .sprites
        .iter()
        .filter(|(_, c)| !c.frames.is_empty())
        .count();
    let (mut spr, mut empty, mut no_recipe) = (0usize, 0usize, 0usize);
    let mut missing: Vec<String> = Vec::new();
    for (id, v) in &game.entity_visuals {
        match v.sprite {
            Some(id) => match tex.get(id) {
                Some(c) if !c.frames.is_empty() => spr += 1,
                _ => empty += 1,
            },
            None => {
                no_recipe += 1;
                if missing.len() < 4 {
                    let ty = world
                        .world
                        .entities
                        .get(id)
                        .map(|e| e.type_id.clone())
                        .unwrap_or_else(|| "?".into());
                    missing.push(ty);
                }
            }
        }
    }
    for v in game
        .collectable_visuals
        .values()
        .chain(game.portal_visuals.values())
    {
        if v.sprite.is_none() {
            no_recipe += 1;
            if missing.len() < 10 {
                missing.push(v.key.clone());
            }
        }
    }
    let mut lines = vec![
        format!("FPS {}", d.get_fps()),
        format!(
            "map {}  cam {:.0},{:.0} x{:.2}",
            game.map_id, camera.center.0, camera.center.1, camera.zoom
        ),
        format!(
            "ents {}  cols {}  pois {}",
            world.world.entities.len(),
            world.world.collectables.len(),
            world.world.pois.len()
        ),
        format!(
            "sprites gpu {} / cpu {}  pending {}",
            tex.len(),
            cpu_clips,
            pending
        ),
        format!("vis sprite {spr}  empty {empty}  noRecipe {no_recipe}"),
        format!(
            "hero type {} pos {:?} tweens {}",
            world
                .world
                .hero
                .as_ref()
                .map(|h| h.type_id.clone())
                .unwrap_or_else(|| "-".into()),
            world.world.hero.as_ref().map(|h| (h.x, h.y)),
            world.tweens.len()
        ),
        format!("missing {}", missing.join(" ")),
    ];
    lines.retain(|l| !l.is_empty());
    let mut y = 8.0;
    let size = 10.0;
    let margin = 10.0;
    for line in &lines {
        let w = fonts.ui.measure_text(line, size, 1.0).x;
        d.draw_text_ex(
            &fonts.ui,
            line,
            Vector2::new((vw - margin - w).max(margin), y),
            size,
            1.0,
            Color::new(160, 230, 160, 230),
        );
        y += 13.0;
    }
}

pub(crate) fn draw_disconnected(
    d: &mut RaylibDrawHandle,
    fonts: &Fonts,
    msg: &str,
    vw: f32,
    vh: f32,
) {
    let text = format!("Connection lost: {msg}");
    let size = 20.0;
    let m = fonts.ui.measure_text(&text, size, 1.0);
    let (bw, bh) = (m.x + 40.0, m.y + 24.0);
    let (bx, by) = (vw * 0.5 - bw * 0.5, vh * 0.4 - bh * 0.5);
    d.draw_rectangle(
        bx as i32,
        by as i32,
        bw as i32,
        bh as i32,
        Color::new(60, 8, 10, 235),
    );
    d.draw_rectangle_lines_ex(
        Rectangle::new(bx, by, bw, bh),
        2.0,
        Color::new(230, 60, 50, 255),
    );
    d.draw_text_ex(
        &fonts.ui,
        &text,
        Vector2::new(bx + 20.0, by + 12.0),
        size,
        1.0,
        Color::new(255, 120, 110, 255),
    );
}

pub(crate) fn centered_banner(
    d: &mut RaylibDrawHandle,
    fonts: &Fonts,
    text: &str,
    cx: f32,
    y: f32,
    size: f32,
    col: Color,
) {
    let m = fonts.ui.measure_text(text, size, 1.0);
    let pos = Vector2::new(cx - m.x * 0.5, y);
    d.draw_text_ex(
        &fonts.ui,
        text,
        Vector2::new(pos.x + 1.0, pos.y + 1.0),
        size,
        1.0,
        Color::new(0, 0, 0, 200),
    );
    d.draw_text_ex(&fonts.ui, text, pos, size, 1.0, col);
}
