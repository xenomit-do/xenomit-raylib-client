//! `EffectsManager2D` — draws `Game::ship_fx` in three layers:
//! [`Layer::BelowHull`]/`AboveHull` are ship-view children drawn inside
//! `entities.rs` around the hull; `Layer::Fx` effects follow the entity
//! on the fx layer and are drawn for the whole map by
//! [`draw_fx_layer`] after entity rendering.
//!
//! Clip playback follows `TweenClip.playAnimation`: looped clips cycle
//! by authored `durations`, one-shots run once (`clip_frame_once`) and
//! hold the last frame until `ShipFxState` expires them. Every clip
//! fades in over `FADE_IN` like `AbstractEffect2D.fadeInClip`.

use crate::assets::AssetManager;
use crate::game::ship_fx::catalog::{self, Anim, FxDesc, Kind, Rot};
pub use crate::game::ship_fx::catalog::Layer;
use crate::game::ship_fx::ActiveFx;
use crate::game::{input, Camera, Game};
use crate::net::SharedWorld;
use crate::render::draw_util::{
    center_shift, clip_frame_once, color_u32, visual_world_rect, CULL_MARGIN,
};
use crate::render::sprites::TextureCache;
use raylib::prelude::*;
use std::time::Instant;

/// `fadeInClip` — clip alpha ramp after `start()`.
const FADE_IN: f32 = 0.3;
/// `SphereShieldEffect2D` ring gap around the clip.
const SHIELD_RING_PAD: f32 = 6.0;

/// `mapObjectView` child draw — `BelowHull` before the hull,
/// `AboveHull` after hull + decorators. `hull_r` is the hull's drawn
/// world-radius (bounds estimate for hull-fit clips and glow rings).
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_entity_fx(
    d: &mut RaylibDrawHandle,
    game: &Game,
    assets: &AssetManager,
    tex: &TextureCache,
    camera: &Camera,
    entity: i32,
    pos: (f32, f32),
    heading: f32,
    hull_r: f32,
    moving: bool,
    layer: Layer,
) {
    let now = Instant::now();
    for e in game.ship_fx.for_entity(entity) {
        let Some(desc) = catalog::lookup(e.effect) else {
            continue;
        };
        if desc.layer != layer {
            continue;
        }
        draw_one(d, assets, tex, camera, e, desc, pos, heading, hull_r, moving, now);
    }
}


/// The `mapView.fxLayer` pass — every `Layer::Fx` effect on the map
/// plus positional free effects, drawn after all entities.
pub(crate) fn draw_fx_layer(
    d: &mut RaylibDrawHandle,
    game: &Game,
    world: &SharedWorld,
    assets: &AssetManager,
    tex: &TextureCache,
    camera: &Camera,
) {
    let now = Instant::now();
    for (&entity, list) in &game.ship_fx.effects {
        let pos = game.draw_pos(world, entity);
        if !camera.in_view(pos, CULL_MARGIN) {
            continue;
        }
        let heading = game.headings.get(&entity).copied().unwrap_or(0.0);
        let hull_r = hull_radius(game, world, tex, entity);
        let moving = world.tweens.contains_key(&entity);
        for e in list {
            let Some(desc) = catalog::lookup(e.effect) else {
                continue;
            };
            if desc.layer != Layer::Fx {
                continue;
            }
            if desc.kind == Kind::ChainLink {
                draw_chain_link(d, game, world, camera, entity, e, desc, now);
                continue;
            }
            let pos = e.pos.unwrap_or(pos);
            draw_one(
                d, assets, tex, camera, e, desc, pos, heading, hull_r, moving, now,
            );
        }
    }
    for e in game.ship_fx.free.values() {
        let Some(desc) = catalog::lookup(e.effect) else {
            continue;
        };
        let pos = e.pos.unwrap_or((0.0, 0.0));
        draw_one(d, assets, tex, camera, e, desc, pos, 0.0, 30.0, false, now);
    }
}

/// Drawn world-radius of the entity's hull (visual bounds), used by
/// `Kind::Glow`/`HullFitClip`/`SphereShield`/`RepairRobots` and the
/// lensflare collider test.
pub(crate) fn hull_radius(game: &Game, world: &SharedWorld, tex: &TextureCache, entity: i32) -> f32 {
    let r = game
        .entity_visuals
        .get(&entity)
        .and_then(|v| v.sprite.map(|id| (v, id)))
        .and_then(|(v, id)| tex.get(id).filter(|c| !c.frames.is_empty()).map(|c| (v, c)))
        .map(|(v, clip)| {
            let (x0, y0, x1, y1) =
                visual_world_rect(v, clip, game.draw_pos(world, entity), false);
            (x1 - x0).max(y1 - y0) * 0.5
        })
        .unwrap_or(24.0);
    r.max(24.0)
}

#[allow(clippy::too_many_arguments)]
fn draw_one(
    d: &mut RaylibDrawHandle,
    assets: &AssetManager,
    tex: &TextureCache,
    camera: &Camera,
    e: &ActiveFx,
    desc: &FxDesc,
    pos: (f32, f32),
    heading: f32,
    hull_r: f32,
    moving: bool,
    now: Instant,
) {
    let age = now
        .saturating_duration_since(e.started)
        .as_secs_f32()
        - e.delay;
    if age < 0.0 {
        return;
    }
    if !camera.in_view(pos, CULL_MARGIN) {
        return;
    }
    let sc = camera.scale_at(pos);
    let screen = camera.to_screen(pos);
    match desc.kind {
        // Bolts are handled in `draw_fx_layer` (needs the world map).
        Kind::ChainLink | Kind::HullAlpha => {}
        Kind::Glow => draw_glow(d, screen, hull_r * sc, desc, e, age),
        Kind::Ring => draw_ring(d, screen, e, age, sc),
        Kind::Shockring => {
            let phase = (age % 2.0) / 2.0;
            let scale = (0.5 + 2.5 * phase) * sc;
            let alpha = 0.7 * (1.0 - phase);
            draw_clip(d, tex, e, desc, screen, sc, scale / sc, alpha, 0.0, age);
        }
        Kind::StickyBomb => {
            // Rotating bomb clip; the emperor stage blinks a light.
            let rot = age * 90.0;
            draw_clip(d, tex, e, desc, screen, sc, 1.0, 1.0, rot, age);
            if e.a == 1 && (age * 4.0) as i32 % 2 == 0 {
                let r = (hull_r * 0.35 * sc).max(2.0);
                d.draw_circle_v(
                    Vector2::new(screen.0, screen.1),
                    r,
                    Color::new(255, 60, 40, 200),
                );
            }
        }
        Kind::SphereShield => {
            draw_clip(d, tex, e, desc, screen, sc, 1.0, 1.0, 0.0, age);
            // Inner/outer shield rings at `a` tenths of the clip size.
            let r = hull_r * sc * (e.a as f32 / 10.0).clamp(0.2, 1.0);
            let v = Vector2::new(screen.0, screen.1);
            let col = Color::new(120, 200, 255, 140);
            d.draw_ring(
                v,
                (r - 1.0).max(0.5),
                r + SHIELD_RING_PAD * sc * 0.2,
                0.0,
                360.0,
                40,
                col,
            );
        }
        Kind::RepairRobots => {
            draw_robots(d, tex, e, screen, sc, hull_r * sc, heading, age)
        }
        Kind::Turbo => {
            // Engine overlay shown only while the ship moves.
            if moving {
                draw_clip(d, tex, e, desc, screen, sc, 1.0, 1.0, heading, age);
            }
        }
        Kind::HullFitClip => {
            let clip_w = e
                .sprite
                .and_then(|id| tex.get(id))
                .map(|c| (c.bounds.2 - c.bounds.0).max(c.bounds.3 - c.bounds.1))
                .filter(|w| *w > 0.0)
                .unwrap_or(hull_r * 2.0);
            let fit = hull_r * 2.0 * (1.0 + desc.hull_extra) / clip_w;
            draw_clip(d, tex, e, desc, screen, sc, fit, 1.0, 0.0, age);
        }
        Kind::Clip => {
            let rot = match desc.rotate {
                Rot::None => 0.0,
                Rot::Ship => heading,
                Rot::HeroMinus180 | Rot::ShipMinus180 => heading - 180.0,
                Rot::Random => e.random_rot,
            };
            let scale = clip_scale(assets, tex, e, desc, age);
            let alpha = clip_alpha(assets, e, desc, age);
            draw_clip(d, tex, e, desc, screen, sc, scale, alpha, rot, age);
        }
    }
}

/// `effectMc.scaleX` chain: authored `scale`, `a / scale_a` radius
/// scaling, `grow` tween.
fn clip_scale(
    assets: &AssetManager,
    tex: &TextureCache,
    e: &ActiveFx,
    desc: &FxDesc,
    age: f32,
) -> f32 {
    let mut s = desc.scale;
    if desc.scale_a > 0.0 {
        s *= e.a as f32 / desc.scale_a;
    }
    // Pattern `scale`/`scale2D` multipliers authored on the effect row.
    if let Some(p) = assets.game_xml.effects.get(&e.effect) {
        if p.scale > 0.0 {
            s *= p.scale;
        }
        if p.scale2d > 0.0 {
            s *= p.scale2d;
        }
    }
    if let Some((from, to, secs)) = desc.grow {
        let t = (age / secs.max(0.01)).min(1.0);
        let target = if to > 0.0 {
            to
        } else {
            // `grow` to `a * 2 / clip width` (boss EMP shock range).
            let w = e
                .sprite
                .and_then(|id| tex.get(id))
                .map(|c| c.bounds.2 - c.bounds.0)
                .filter(|w| *w > 0.0)
                .unwrap_or(64.0);
            e.a as f32 * 2.0 / w
        };
        s *= from + (target - from) * t;
    }
    s
}

/// Clip alpha: pattern `initAlpha`, yoyo `alpha_cycle`, fade-in.
fn clip_alpha(assets: &AssetManager, e: &ActiveFx, desc: &FxDesc, age: f32) -> f32 {
    let mut a = assets
        .game_xml
        .effects
        .get(&e.effect)
        .map(|p| p.init_alpha)
        .unwrap_or(1.0);
    if let Some((lo, hi, secs)) = desc.alpha_cycle {
        let ph = ((age / secs.max(0.01)) * std::f32::consts::TAU).cos();
        a *= lo + (hi - lo) * (0.5 - 0.5 * ph);
    }
    a * (age / FADE_IN).min(1.0)
}

/// Draw `e`'s clip centred on `screen`; `scale`/`alpha` multiply the
/// descriptor/authored values, `rot` is degrees.
#[allow(clippy::too_many_arguments)]
fn draw_clip(
    d: &mut RaylibDrawHandle,
    tex: &TextureCache,
    e: &ActiveFx,
    desc: &FxDesc,
    screen: (f32, f32),
    sc: f32,
    scale: f32,
    alpha: f32,
    rot: f32,
    age: f32,
) {
    let Some(clip) = e
        .sprite
        .and_then(|id| tex.get(id))
        .filter(|c| !c.frames.is_empty())
    else {
        return;
    };
    let ms = (age * 1000.0 * desc.rate.max(0.1)) as u64;
    let fi = match desc.anim {
        Anim::Loop => input::anim_frame(&clip.durations, ms).min(clip.frames.len() - 1),
        Anim::Once => clip_frame_once(clip, ms),
    };
    let (cx, cy) = center_shift(clip);
    let off = (desc.offset.0 * sc - cx, desc.offset.1 * sc - cy);
    TextureCache::draw_frame_offset(
        d,
        clip,
        fi,
        screen,
        rot,
        (sc * scale, sc * scale),
        Color::WHITE.alpha(alpha.clamp(0.0, 1.0)),
        off,
    );
}

/// `ChainBoltEffect2D` — a `Lightning` bolt owner -> chain target
/// (`e.a`). Positions refresh live (`ChainBoltPositionProvider` binds
/// the entity positions), the bolt fades in over 0.5 s and — for
/// `NOT_PERSISTANT` links — out at `boltFadeTime` 3 s.
#[allow(clippy::too_many_arguments)]
fn draw_chain_link(
    d: &mut RaylibDrawHandle,
    game: &Game,
    world: &SharedWorld,
    camera: &Camera,
    entity: i32,
    e: &ActiveFx,
    desc: &FxDesc,
    now: Instant,
) {
    let Some(bolt) = desc.bolt else { return };
    let age = now
        .saturating_duration_since(e.started)
        .as_secs_f32()
        - e.delay;
    if age < 0.0 {
        return;
    }
    let a = game.draw_pos(world, entity);
    let b = world.pos(e.a as i32);
    if (a.0 - b.0).hypot(a.1 - b.1) < 1.0 {
        return;
    }
    let (sa, sb) = (camera.to_screen(a), camera.to_screen(b));
    let len = (sb.0 - sa.0).hypot(sb.1 - sa.1).max(1.0);
    // `addAndFadeIn` alpha 0 -> 1 over 0.5 s; `fadeOutAndRemove` at 3 s.
    let mut alpha = (age / 0.5).min(1.0);
    if age > 3.0 {
        alpha *= (1.0 - (age - 3.0) / 0.2).max(0.0);
    }
    if alpha <= 0.0 {
        return;
    }
    let col = color_u32(bolt.color);
    let steps = bolt.steps.max(4) as usize;
    let (dx, dy) = (sb.0 - sa.0, sb.1 - sa.1);
    let (nx, ny) = (-dy / len, dx / len);
    // Perlin-`boff` stand-in: a deterministic zig-zag that re-seeds
    // ~every update tick (`Lightning.update` at 30 ms).
    let slice = (age * 33.0) as u64;
    let w = bolt.thickness * camera.zoom.clamp(0.4, 2.0);
    let mut prev = sa;
    for i in 1..steps {
        let u = i as f32 / steps as f32;
        let mut h = (i as u64)
            .wrapping_mul(0x9E3779B97F4A7C15)
            .wrapping_add(slice.wrapping_mul(0xD1B54A32D192ED03));
        h ^= h >> 29;
        h = h.wrapping_mul(0xBF58476D1CE4E5B9);
        let noise = (((h >> 33) & 0xFFFF) as f32 / 65536.0) - 0.5;
        // Endpoints stay anchored — the offset envelope vanishes at
        // both ends like the Lightning displacement does.
        let env = (u * (1.0 - u) * 4.0).min(1.0);
        let off = noise * 2.0 * len * bolt.amplitude * 0.1 * env;
        let p = (
            sa.0 + dx * u + nx * off,
            sa.1 + dy * u + ny * off,
        );
        d.draw_line_ex(
            Vector2::new(prev.0, prev.1),
            Vector2::new(p.0, p.1),
            w,
            col.alpha(alpha * 0.95),
        );
        prev = p;
    }
    d.draw_line_ex(
        Vector2::new(prev.0, prev.1),
        Vector2::new(sb.0, sb.1),
        w,
        col.alpha(alpha * 0.95),
    );
    // `BOLT_FILTER` glow — a second wider pass underneath is not
    // possible at this point in the pass, so a soft halo line sits on
    // top at low alpha.
    d.draw_line_ex(
        Vector2::new(sa.0, sa.1),
        Vector2::new(sb.0, sb.1),
        w * 2.6,
        col.alpha(alpha * 0.12),
    );
}

/// `ShipGlowEffect2D` — pulsing `GlowFilter` on the ship view,
/// approximated by a soft pulsing ring at the hull edge.
fn draw_glow(d: &mut RaylibDrawHandle, screen: (f32, f32), r: f32, desc: &FxDesc, e: &ActiveFx, age: f32) {
    let color = if e.a != 0 { e.a as u32 } else { desc.glow };
    if color == 0 {
        return;
    }
    let pulse = 0.5 - 0.5 * (age * std::f32::consts::TAU / 1.0).cos();
    let col = color_u32(color);
    let v = Vector2::new(screen.0, screen.1);
    let r = r.max(6.0);
    d.draw_ring(v, r * 0.96, r * 1.04, 0.0, 360.0, 40, col.alpha(0.25 + 0.35 * pulse));
    d.draw_ring(
        v,
        r * 1.04,
        r * 1.16,
        0.0,
        360.0,
        40,
        col.alpha(0.12 + 0.2 * pulse),
    );
}

/// `GlowingRingEffect`/`MapObjectGlowRingEffect` — expanding ring
/// `start_d -> end_d` world units over `cycle` s; `cycle 0` = static.
fn draw_ring(d: &mut RaylibDrawHandle, screen: (f32, f32), e: &ActiveFx, age: f32, sc: f32) {
    let Some(ring) = e.ring else { return };
    let col = color_u32(ring.color);
    let v = Vector2::new(screen.0, screen.1);
    let (r, alpha) = if ring.cycle <= 0.0 {
        (ring.end_d, 0.8)
    } else {
        let ph = if ring.looped {
            (age % ring.cycle) / ring.cycle
        } else {
            (age / ring.cycle).min(1.0)
        };
        (
            ring.start_d + (ring.end_d - ring.start_d) * ph,
            0.8 * (1.0 - ph * if ring.looped { 0.0 } else { 1.0 }),
        )
    };
    let r = r * sc * 0.5;
    if r < 1.0 {
        return;
    }
    d.draw_ring(v, (r - 1.5).max(0.5), r + 1.5, 0.0, 360.0, 48, col.alpha(alpha));
}

/// `RepairRobotEffect2D` — `robotsNum` clips orbiting on each robot's
/// tweened radius/angle (state ticked in `ShipFxState::update`), each
/// with a sparks clip on the smaller orbit (`updateDesiredPositions`).
#[allow(clippy::too_many_arguments)]
fn draw_robots(
    d: &mut RaylibDrawHandle,
    tex: &TextureCache,
    e: &ActiveFx,
    screen: (f32, f32),
    sc: f32,
    click_r: f32,
    heading: f32,
    age: f32,
) {
    let Some(clip) = e
        .sprite
        .and_then(|id| tex.get(id))
        .filter(|c| !c.frames.is_empty())
    else {
        return;
    };
    let head = heading.to_radians();
    let (cx, cy) = center_shift(clip);
    for r in &e.robots {
        // `tweenObject` — 0.2 s linear ease toward the desired angle.
        let t = ((age - r.tween_at) / 0.2).clamp(0.0, 1.0);
        let a = r.from + (r.angle - r.from) * t;
        let x = (head + a).cos() * click_r * r.radius;
        let y = (head + a).sin() * click_r * r.radius;
        let fi =
            input::anim_frame(&clip.durations, (age * 1000.0) as u64).min(clip.frames.len() - 1);
        // `view.scaleX` flips the clip left of the ship.
        let flip = if x > 0.0 { -sc } else { sc };
        TextureCache::draw_frame_offset(
            d,
            clip,
            fi,
            (
                screen.0 + x + e.robot_off.0 * sc,
                screen.1 + y + e.robot_off.1 * sc,
            ),
            0.0,
            (flip, sc),
            Color::WHITE,
            (-cx, -cy),
        );
        if let Some(sp) = e
            .sparks
            .and_then(|id| tex.get(id))
            .filter(|c| !c.frames.is_empty())
        {
            let sa = head + a + r.sparks_a;
            let p = (
                screen.0 + sa.cos() * click_r * r.sparks_r + e.sparks_off.0 * sc,
                screen.1 + sa.sin() * click_r * r.sparks_r + e.sparks_off.1 * sc,
            );
            // `currentFrame = int(numFrames * Math.random())` — a
            // per-robot stream offset stands in for the random start.
            let seed_ms = (r.seed & 0x3FF) * 33;
            let sfi = input::anim_frame(&sp.durations, (age * 1000.0) as u64 + seed_ms)
                .min(sp.frames.len() - 1);
            let (sx, sy) = center_shift(sp);
            TextureCache::draw_frame_offset(
                d,
                sp,
                sfi,
                p,
                0.0,
                (sc, sc),
                Color::WHITE,
                (-sx, -sy),
            );
        }
    }
}
