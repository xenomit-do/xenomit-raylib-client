//! `LensFlaresLayer` — the flare pass above the world, under the HUD.
//!
//! Per frame each live flare resolves its provider position (2D
//! parallax anchor or the 3D scene point through the orbit camera),
//! is hidden off-screen or while a ship's hull covers the point
//! (`isViewPortCollided` — stock hit-tests entities/backgrounds; our
//! equivalent is the drawn hull radius), and fades in over 0.1 s on
//! every fresh show. `star` clips rotate −0.15°/frame; `blendMode="add"`
//! children draw additively; `lensFlash` fires once per becoming-visible.

use crate::game::lensflares::{fade_alpha, flash_alpha, lens_step, screen_pos_2d, LensFlares};
use crate::game::{Camera, Game};
use crate::net::SharedWorld;
use crate::render::{scene3d, sprites::TextureCache};
use raylib::prelude::*;
use std::time::Instant;

/// `LensFlareView` draws `lens{i}` children centred at their step.
fn draw_centered(
    d: &mut RaylibDrawHandle,
    tex: &TextureCache,
    sprite: crate::assets::SpriteId,
    pos: (f32, f32),
    rot: f32,
    alpha: f32,
) {
    let Some(clip) = tex.get(sprite).filter(|c| !c.frames.is_empty()) else {
        return;
    };
    let frame = &clip.frames[0];
    TextureCache::draw_frame_offset(
        d,
        clip,
        0,
        pos,
        rot,
        (1.0, 1.0),
        Color::WHITE.alpha(alpha),
        (
            -frame.tex.width as f32 * 0.5,
            -frame.tex.height as f32 * 0.5,
        ),
    );
}

/// Ship-hull cover test — `LensFlaresLayer.isViewPortCollided`
/// equivalent: a drawn hull circle (hero included) covering the
/// flare's screen point hides it.
fn occluded(
    p: (f32, f32),
    game: &Game,
    world: &SharedWorld,
    tex: &TextureCache,
    camera: &Camera,
) -> bool {
    let mut ids: Vec<i32> = world.world.entities.keys().copied().collect();
    if let Some(h) = &world.world.hero {
        ids.push(h.user_id);
    }
    ids.into_iter().any(|id| {
        let pos = game.draw_pos(world, id);
        let r = crate::render::ship_fx::hull_radius(game, world, tex, id);
        let (sx, sy) = camera.to_screen(pos);
        let rr = (r * camera.scale_at(pos)).max(8.0);
        (sx - p.0).hypot(sy - p.1) < rr
    })
}

pub(crate) fn draw(
    d: &mut RaylibDrawHandle,
    game: &mut Game,
    world: &SharedWorld,
    tex: &TextureCache,
    camera: &Camera,
) {
    let perspective = camera.perspective;
    if game.lensflares.active(perspective).is_empty() {
        return;
    }
    if perspective && game.lensflares.hide3d {
        // A `hideLensflare` tilemap covers the scene — no per-pixel
        // mask test, so the scene's flares stay hidden (documented).
        return;
    }
    let now = Instant::now();
    let viewport = camera.viewport;
    let cam3d = perspective.then(|| scene3d::camera(game));
    // Provider positions + collider check (immutable borrows) before
    // the mutable visibility/draw pass (`updateObj` drives both in
    // stock's frame tick).
    let spots: Vec<(f32, f32)> = game
        .lensflares
        .active(perspective)
        .iter()
        .map(|flare| match flare.scene_pos {
            Some(pos) if perspective => {
                let p = d.get_world_to_screen(
                    scene3d::rl_point(pos.0, pos.1, pos.2),
                    cam3d.unwrap(),
                );
                (p.x, p.y)
            }
            _ => screen_pos_2d(
                flare.anchor,
                flare.p_factor,
                camera.center,
                viewport,
                camera.zoom,
            ),
        })
        .collect();
    let shown: Vec<bool> = spots
        .iter()
        .map(|s| {
            s.0 > -64.0
                && s.0 < viewport.0 + 64.0
                && s.1 > -64.0
                && s.1 < viewport.1 + 64.0
                && !occluded(*s, game, world, tex, camera)
        })
        .collect();
    let flash_sprite = game.lensflares.flash;
    for (flare, (screen, show)) in game
        .lensflares
        .active_mut(perspective)
        .iter_mut()
        .zip(spots.iter().zip(shown))
    {
        LensFlares::set_visible(flare, show, now);
        if !flare.visible {
            continue;
        }
        let screen = *screen;
        let alpha = fade_alpha(now.saturating_duration_since(flare.fade_at).as_secs_f32());
        // `n` = resolved `lens{i}` children (missing trailing exports
        // and still-streaming ids stay empty and are dropped).
        let n = flare
            .lenses
            .iter()
            .take_while(|id| tex.get(**id).is_some_and(|c| !c.frames.is_empty()))
            .count();
        if n == 0 {
            continue;
        }
        let step = lens_step(screen, viewport, n);
        let draw_one = |d: &mut RaylibDrawHandle,
                        flare: &crate::game::lensflares::LensFlare| {
            for (i, sprite) in flare.lenses.iter().take(n).enumerate() {
                draw_centered(
                    d,
                    tex,
                    *sprite,
                    (screen.0 + step.0 * i as f32, screen.1 + step.1 * i as f32),
                    0.0,
                    alpha,
                );
            }
            if flare.star {
                if let Some(star) = flare.star_sprite {
                    draw_centered(d, tex, star, screen, flare.star_rot, alpha);
                }
            }
            if let (Some(flash), Some(at)) = (flash_sprite, flare.flash_at) {
                let fa = flash_alpha(now.saturating_duration_since(at).as_secs_f32());
                if fa > 0.0 {
                    draw_centered(d, tex, flash, screen, 0.0, fa);
                }
            }
        };
        if flare.additive {
            let mut b = d.begin_blend_mode(BlendMode::BLEND_ADDITIVE);
            draw_one(&mut b, flare);
        } else {
            draw_one(d, flare);
        }
    }
}
