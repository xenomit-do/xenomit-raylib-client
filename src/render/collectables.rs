use crate::game::{input, Camera, Game};
use crate::net::SharedWorld;
use crate::render::draw_util::{
    draw_visual, draw_visual_layers, pick_frame, visual_world_rect, CULL_MARGIN,
};
use crate::render::sprites::TextureCache;
use raylib::prelude::*;
use xenomit_packet_client::model::CollectableKind;

pub(crate) fn draw_collectables(
    d: &mut RaylibDrawHandle,
    game: &Game,
    world: &SharedWorld,
    tex: &TextureCache,
    camera: &Camera,
    time_ms: u64,
    rendered_3d: &std::collections::HashSet<String>,
) {
    for c in world.world.collectables.values() {
        if rendered_3d.contains(&c.hash) {
            continue;
        }
        let p = (c.x as f32, c.y as f32);
        let v = game.collectable_visuals.get(&c.hash);
        let clip = v
            .and_then(|v| v.sprite)
            .and_then(|id| tex.get(id))
            .filter(|clip| !clip.frames.is_empty());
        let on_screen = match (v, clip) {
            (Some(v), Some(clip)) => {
                let (x0, y0, x1, y1) = visual_world_rect(v, clip, p, false);
                camera.rect_in_view((x0, y0), (x1, y1), CULL_MARGIN)
            }
            _ => camera.in_view(p, CULL_MARGIN),
        };
        if !on_screen {
            continue;
        }
        let screen = camera.to_screen(p);
        let sc = camera.scale_at(p);
        match (v, clip) {
            (Some(v), Some(clip)) => {
                // `abstract_3d_box` plays a one-second `spawnNew` scale
                // tween. The 2D recipe uses the same loot artwork but did
                // not carry that state over, so reproduce the authored
                // ease-in/out here. Pirate booty clips are rotation strips
                // (25 frames at 83 ms) and should keep turning in 2D.
                let mut animated = v.clone();
                let spawn_scale = if c.kind == CollectableKind::Box {
                    game.collectable_spawned
                        .get(&c.hash)
                        .map(|at| at.elapsed().as_secs_f32())
                        .filter(|age| *age < 1.0)
                        .map(|t| {
                            let eased = t.clamp(0.0, 1.0);
                            let eased = eased * eased * (3.0 - 2.0 * eased);
                            0.06 + eased * 0.94
                        })
                        .unwrap_or(1.0)
                } else {
                    1.0
                };
                if spawn_scale < 1.0 {
                    animated.offset.0 *= spawn_scale;
                    animated.offset.1 *= spawn_scale;
                    animated.scale.0 *= spawn_scale;
                    animated.scale.1 *= spawn_scale;
                }
                let picked = if c.kind == CollectableKind::Box && clip.rotation_strip {
                    Some((input::anim_frame(&clip.durations, time_ms), 0.0))
                } else {
                    pick_frame(&animated, clip, 0.0, time_ms, false, false, false)
                };
                let Some((fi, rot)) = picked else {
                    continue;
                };
                draw_visual(d, clip, &animated, fi, rot, screen, sc, Color::WHITE);
                draw_visual_layers(
                    d,
                    tex,
                    &animated,
                    0.0,
                    time_ms,
                    false,
                    false,
                    false,
                    screen,
                    sc,
                    Color::WHITE,
                );
                // `MineDecorator2D` — looping `minePulse` child, scaled
                // by the pattern and RGB-replaced by its colour (here a
                // multiply tint: the pulse art is white-authored).
                if let Some(pulse) = v.pulse {
                    if let Some(clip) = tex
                        .get(pulse.sprite)
                        .filter(|c| !c.frames.is_empty())
                    {
                        let fi = input::anim_frame(&clip.durations, time_ms)
                            .min(clip.frames.len() - 1);
                        let (r, g, b) = (
                            (pulse.color >> 16) as u8,
                            (pulse.color >> 8) as u8,
                            pulse.color as u8,
                        );
                        TextureCache::draw_frame_offset(
                            d,
                            clip,
                            fi,
                            screen,
                            0.0,
                            (sc * pulse.scale, sc * pulse.scale),
                            Color::new(r, g, b, (pulse.alpha.clamp(0.0, 1.0) * 255.0) as u8),
                            (0.0, 0.0),
                        );
                    }
                }
            }
            _ => {
                let col = collectable_color(c.kind);
                let r = (9.0 * sc).clamp(4.0, 14.0);
                d.draw_poly(Vector2::new(screen.0, screen.1), 4, r, 0.0, col.alpha(0.9));
                d.draw_poly_lines(
                    Vector2::new(screen.0, screen.1),
                    4,
                    r + 1.5,
                    0.0,
                    Color::new(255, 255, 255, 90),
                );
            }
        }
    }
}

fn collectable_color(kind: CollectableKind) -> Color {
    match kind {
        CollectableKind::Box => Color::new(255, 205, 70, 255),
        CollectableKind::Ore => Color::new(120, 200, 255, 255),
        CollectableKind::Beacon => Color::new(120, 255, 140, 255),
        CollectableKind::Mine => Color::new(255, 80, 70, 255),
        CollectableKind::FireworkBox => Color::new(220, 120, 255, 255),
    }
}
