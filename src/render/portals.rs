use crate::game::input;
use crate::game::{Camera, Game, PortalLayer};
use crate::net::portals::jump_strength;
use crate::net::SharedWorld;
use crate::render::draw_util::{draw_visual, pick_frame, visual_world_rect, CULL_MARGIN};
use crate::render::sprites::TextureCache;
use raylib::prelude::*;
use std::time::Instant;

/// `spawn_animation` keeps the shell scaled down until this delay.
const SPAWN_DELAY: f32 = 0.5;
/// Base / animated_base scale+alpha tween after [`SPAWN_DELAY`].
const SPAWN_IN: f32 = 1.0;
/// `spawn_effect` fade-out length after the same delay.
const SPAWN_FADE: f32 = 3.0;
/// Highlight stays hidden until this far into `spawn_animation`.
const SPAWN_HIGHLIGHT: f32 = 4.0;

pub(crate) fn draw_portals(
    d: &mut RaylibDrawHandle,
    game: &Game,
    world: &SharedWorld,
    tex: &TextureCache,
    camera: &Camera,
    time_ms: u64,
    rendered_3d: &std::collections::HashSet<i32>,
) {
    // `Settings.simpleGates`: plain ring instead of the gate shell.
    // The lightweight `animated_base` idle clip stays.
    let simple = world.settings.simple_gates;
    let now = Instant::now();
    for p in world.portals.values() {
        let pos = (p.x as f32, p.y as f32);
        let v = game.portal_visuals.get(&p.id);
        let age = p
            .created_at
            .map(|t| now.saturating_duration_since(t).as_secs_f32())
            .unwrap_or(0.0);
        let jump = world
            .portal_jumps
            .get(&p.id)
            .map(|t| jump_strength(*t, now))
            .unwrap_or(0.0);
        let in_3d = rendered_3d.contains(&p.id);
        let layers = v.map(|vis| vis.map_layers.as_slice()).unwrap_or(&[]);
        let on_screen = if layers.is_empty() {
            legacy_on_screen(v, tex, camera, pos)
        } else {
            layers_on_screen(layers, tex, camera, pos)
        };
        if !on_screen {
            continue;
        }
        let screen = camera.to_screen(pos);
        let sc = camera.scale_at(pos);
        let mut drew = false;
        if !layers.is_empty() {
            let Some(vis) = v else { continue };
            for layer in &vis.map_layers {
                if simple && layer.id != "animated_base" {
                    continue;
                }
                // 3D replaces the shell and plays jump as particles.
                // `animated_base` is the 2D idle swirl the mesh does not carry.
                if in_3d && layer.id != "animated_base" {
                    continue;
                }
                let Some((alpha, scale_mul)) = layer_style(layer, age, p.spawn_anim, jump) else {
                    continue;
                };
                let Some(clip) = tex.get(layer.sprite).filter(|c| !c.frames.is_empty()) else {
                    continue;
                };
                let fi = input::anim_frame(&clip.durations, time_ms).min(clip.frames.len() - 1);
                let frame = &clip.frames[fi];
                let (w, h) = (frame.tex.width as f32, frame.tex.height as f32);
                let (sx, sy) = layer.scale;
                // `transformAroundCenter` during spawn: the bitmap center
                // stays put while scale eases 0.3 → 1.
                let ox = layer.offset.0 + w * sx * (1.0 - scale_mul) * 0.5;
                let oy = layer.offset.1 + h * sy * (1.0 - scale_mul) * 0.5;
                let tint = if alpha >= 0.999 {
                    Color::WHITE
                } else {
                    Color::new(255, 255, 255, (alpha * 255.0).round() as u8)
                };
                TextureCache::draw_frame_offset(
                    d,
                    clip,
                    fi,
                    (screen.0 + ox * sc, screen.1 + oy * sc),
                    0.0,
                    (sc * sx * scale_mul, sc * sy * scale_mul),
                    tint,
                    (0.0, 0.0),
                );
                drew = true;
            }
        } else if !in_3d {
            drew = draw_legacy(d, v, tex, screen, sc, time_ms, simple);
        }
        if !drew && !in_3d {
            let r = (46.0 * sc).clamp(12.0, 90.0);
            let c = Vector2::new(screen.0, screen.1);
            d.draw_ring(c, r, r + 2.5, 0.0, 360.0, 40, Color::new(90, 200, 235, 220));
            d.draw_ring(
                c,
                r * 0.55,
                r * 0.55 + 1.5,
                0.0,
                360.0,
                28,
                Color::new(70, 150, 200, 160),
            );
        }
    }
}

/// Alpha and extra scale for one layer.
///
/// `None` means the layer is hidden. Highlight loops 2s up / 2s down
/// (`abstract_portal`). `active` and `jump` follow `jumpOn` / `jumpOff`.
/// A gate that appears after the map has been up for 10s plays
/// `spawn_animation`; otherwise `spawn_effect` stays cleared.
fn layer_style(layer: &PortalLayer, age: f32, spawn: bool, jump: f32) -> Option<(f32, f32)> {
    let spawning = spawn && age < SPAWN_HIGHLIGHT;
    match layer.id.as_str() {
        "spawn_effect" => {
            if !spawn {
                return None;
            }
            if age < SPAWN_DELAY {
                return Some((1.0, 1.0));
            }
            let u = ((age - SPAWN_DELAY) / SPAWN_FADE).clamp(0.0, 1.0);
            let alpha = 1.0 - u;
            (alpha > 0.01).then_some((alpha, 1.0))
        }
        "highlight" => {
            if spawning {
                return None;
            }
            // Looping tween: 2s to alpha 1, 2s back to 0, from portal birth.
            let phase = age.rem_euclid(4.0);
            let alpha = if phase < 2.0 {
                phase / 2.0
            } else {
                1.0 - (phase - 2.0) / 2.0
            };
            (alpha > 0.01).then_some((alpha, 1.0))
        }
        "active" | "jump" => {
            if jump <= 0.0 {
                return None;
            }
            Some((jump, 1.0))
        }
        "base" | "animated_base" if spawning => {
            if age < SPAWN_DELAY {
                return None;
            }
            let u = ((age - SPAWN_DELAY) / SPAWN_IN).clamp(0.0, 1.0);
            let alpha = u;
            let scale = 0.3 + 0.7 * u;
            (alpha > 0.01).then_some((alpha, scale))
        }
        _ => {
            if !layer.visible || layer.alpha <= 0.01 {
                return None;
            }
            Some((layer.alpha, 1.0))
        }
    }
}

fn layers_on_screen(
    layers: &[PortalLayer],
    tex: &TextureCache,
    camera: &Camera,
    pos: (f32, f32),
) -> bool {
    let mut any = false;
    let mut rect: Option<(f32, f32, f32, f32)> = None;
    for layer in layers {
        let Some(clip) = tex.get(layer.sprite).filter(|c| !c.frames.is_empty()) else {
            continue;
        };
        any = true;
        let (x0, y0, x1, y1) = clip.bounds;
        let sx = layer.scale.0.abs().max(0.01);
        let sy = layer.scale.1.abs().max(0.01);
        let r = (
            pos.0 + layer.offset.0 + x0 * sx,
            pos.1 + layer.offset.1 + y0 * sy,
            pos.0 + layer.offset.0 + x1 * sx,
            pos.1 + layer.offset.1 + y1 * sy,
        );
        rect = Some(match rect {
            Some(a) => (a.0.min(r.0), a.1.min(r.1), a.2.max(r.2), a.3.max(r.3)),
            None => r,
        });
    }
    match rect {
        Some((x0, y0, x1, y1)) => camera.rect_in_view((x0, y0), (x1, y1), CULL_MARGIN),
        None if !any => camera.in_view(pos, CULL_MARGIN * 3.0),
        None => false,
    }
}

fn legacy_on_screen(
    v: Option<&crate::game::Visual>,
    tex: &TextureCache,
    camera: &Camera,
    pos: (f32, f32),
) -> bool {
    let clip = v
        .and_then(|v| v.sprite)
        .and_then(|id| tex.get(id))
        .filter(|c| !c.frames.is_empty());
    let oclip = v
        .and_then(|v| v.overlay)
        .and_then(|id| tex.get(id))
        .filter(|c| !c.frames.is_empty());
    match v {
        Some(v) if clip.is_some() || oclip.is_some() => {
            let mut rect = clip.map(|c| visual_world_rect(v, c, pos, false));
            if let Some(oc) = oclip {
                let (bx0, by0, bx1, by1) = oc.bounds;
                let hw = (bx1 - bx0) * 0.5 * v.overlay_scale;
                let hh = (by1 - by0) * 0.5 * v.overlay_scale;
                let c = (pos.0 + v.overlay_offset.0, pos.1 + v.overlay_offset.1);
                let or_ = (c.0 - hw, c.1 - hh, c.0 + hw, c.1 + hh);
                rect = Some(match rect {
                    Some(r) => (
                        r.0.min(or_.0),
                        r.1.min(or_.1),
                        r.2.max(or_.2),
                        r.3.max(or_.3),
                    ),
                    None => or_,
                });
            }
            let (x0, y0, x1, y1) = rect.unwrap_or((pos.0, pos.1, pos.0, pos.1));
            camera.rect_in_view((x0, y0), (x1, y1), CULL_MARGIN)
        }
        _ => camera.in_view(pos, CULL_MARGIN * 3.0),
    }
}

fn draw_legacy(
    d: &mut RaylibDrawHandle,
    v: Option<&crate::game::Visual>,
    tex: &TextureCache,
    screen: (f32, f32),
    sc: f32,
    time_ms: u64,
    simple: bool,
) -> bool {
    let clip = v
        .and_then(|v| v.sprite)
        .and_then(|id| tex.get(id))
        .filter(|c| !c.frames.is_empty())
        .filter(|_| !simple);
    let oclip = v
        .and_then(|v| v.overlay)
        .and_then(|id| tex.get(id))
        .filter(|c| !c.frames.is_empty());
    let mut drew = false;
    if let (Some(v), Some(clip)) = (v, clip) {
        if let Some((fi, rot)) = pick_frame(v, clip, 0.0, time_ms, false, false, false) {
            draw_visual(d, clip, v, fi, rot, screen, sc, Color::WHITE);
            drew = true;
        }
    }
    if let (Some(v), Some(oclip)) = (v, oclip) {
        let fi = input::anim_frame(&oclip.durations, time_ms).min(oclip.frames.len() - 1);
        let f = &oclip.frames[fi];
        TextureCache::draw_frame_offset(
            d,
            oclip,
            fi,
            (
                screen.0 + v.overlay_offset.0 * sc,
                screen.1 + v.overlay_offset.1 * sc,
            ),
            0.0,
            (sc * v.overlay_scale, sc * v.overlay_scale),
            Color::WHITE,
            (-f.tex.width as f32 * 0.5, -f.tex.height as f32 * 0.5),
        );
        drew = true;
    }
    drew
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::PortalLayer;

    fn layer(id: &str, alpha: f32, visible: bool) -> PortalLayer {
        PortalLayer {
            id: id.into(),
            sprite: crate::assets::SpriteId(0),
            offset: (0.0, 0.0),
            scale: (1.0, 1.0),
            alpha,
            visible,
        }
    }

    #[test]
    fn jump_layers_fade_with_the_jump_curve() {
        let jump = layer("jump", 0.0, false);
        assert!(layer_style(&jump, 1.0, false, 0.0).is_none());
        let (alpha, scale) = layer_style(&jump, 1.0, false, 0.5).unwrap();
        assert!((alpha - 0.5).abs() < 1e-4);
        assert!((scale - 1.0).abs() < 1e-4);
        let active = layer("active", 0.0, false);
        assert!((layer_style(&active, 0.0, false, 1.0).unwrap().0 - 1.0).abs() < 1e-4);
    }

    #[test]
    fn spawn_hides_the_shell_then_the_burst() {
        let base = layer("base", 1.0, true);
        assert!(layer_style(&base, 0.1, true, 0.0).is_none());
        let (alpha, scale) = layer_style(&base, 1.0, true, 0.0).unwrap();
        assert!(alpha > 0.4 && alpha < 0.6);
        assert!(scale > 0.6 && scale < 0.7);
        // Past the spawn window the authored alpha shows.
        assert!((layer_style(&base, 5.0, true, 0.0).unwrap().0 - 1.0).abs() < 1e-4);

        let burst = layer("spawn_effect", 1.0, true);
        assert!(layer_style(&burst, 0.1, true, 0.0).is_some());
        assert!(layer_style(&burst, 0.1, false, 0.0).is_none());
        assert!(layer_style(&burst, 4.0, true, 0.0).is_none());

        let highlight = layer("highlight", 0.0, true);
        assert!(layer_style(&highlight, 1.0, true, 0.0).is_none());
        // One second into the 2s rise (age 4 is the loop boundary, alpha 0).
        let (alpha, _) = layer_style(&highlight, 5.0, true, 0.0).unwrap();
        assert!((alpha - 0.5).abs() < 1e-4);
    }
}
