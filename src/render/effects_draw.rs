use crate::game::effects::{self, Anchor, BeamKind, Effect, PulseKind};
use crate::game::{input, Camera, Game};
use crate::net::SharedWorld;
use crate::render::draw_util::{clip_frame_once, color_u32, draw_centered_text, CULL_MARGIN};
use crate::render::fonts::Fonts;
use crate::render::sprites::TextureCache;
use raylib::prelude::*;
use std::collections::HashSet;

fn anchor_pos(world: &SharedWorld, a: Anchor) -> (f32, f32) {
    match a {
        Anchor::Pos(p) => p,
        Anchor::Entity(id) => world.pos(id),
    }
}

fn lerp2(a: (f32, f32), b: (f32, f32), t: f32) -> (f32, f32) {
    (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)
}

/// Heal ray: three phase-shifted sine strands between the two ships.
fn draw_heal_beam(
    d: &mut RaylibDrawHandle,
    a: (f32, f32),
    b: (f32, f32),
    t: f32,
    zoom: f32,
    col: Color,
    time: f32,
) {
    let fade = (t * 6.0).min((1.0 - t) * 6.0).clamp(0.0, 1.0);
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len = dx.hypot(dy).max(1.0);
    let n = (-dy / len, dx / len);
    let amp = 5.0 * zoom.clamp(0.6, 1.5);
    const STEPS: usize = 24;
    for strand in 0..3 {
        let phase = strand as f32 * 2.1;
        let mut prev = a;
        for i in 1..=STEPS {
            let u = i as f32 / STEPS as f32;
            let env = (u * (1.0 - u) * 4.0).min(1.0);
            let w = ((u * len / 28.0) - time * 9.0 + phase).sin() * amp * env;
            let base = lerp2(a, b, u);
            let p = (base.0 + n.0 * w, base.1 + n.1 * w);
            d.draw_line_ex(
                Vector2::new(prev.0, prev.1),
                Vector2::new(p.0, p.1),
                1.6,
                col.alpha(fade * if strand == 0 { 0.9 } else { 0.55 }),
            );
            prev = p;
        }
    }
}

/// Game-authored looping resource beam, drawn from the pickup to the ship.
fn draw_collector_beam(
    d: &mut RaylibDrawHandle,
    a: (f32, f32),
    b: (f32, f32),
    clip: &crate::render::sprites::GpuClip,
    elapsed_ms: u64,
    zoom: f32,
    alpha: f32,
) {
    let Some(frame_count) = (!clip.frames.is_empty()).then_some(clip.frames.len()) else {
        return;
    };
    let length = (b.0 - a.0).hypot(b.1 - a.1);
    if length < 1.0 {
        return;
    }
    let frame = input::anim_frame(&clip.durations, elapsed_ms).min(frame_count - 1);
    let (w, h) = (
        clip.frames[frame].tex.width as f32,
        clip.frames[frame].tex.height as f32,
    );
    if w <= 0.0 || h <= 0.0 {
        return;
    }
    let center = ((a.0 + b.0) * 0.5, (a.1 + b.1) * 0.5);
    let angle = (b.1 - a.1).atan2(b.0 - a.0).to_degrees();
    TextureCache::draw_frame_offset(
        d,
        clip,
        frame,
        center,
        angle,
        (length / w, zoom),
        Color::WHITE.alpha(alpha),
        (-w * 0.5, -h * 0.5),
    );
}

/// Engine puffs paint before the ships so their exhaust stays behind the hull.
pub(crate) fn draw_engine_smoke(
    d: &mut RaylibDrawHandle,
    game: &Game,
    tex: &TextureCache,
    camera: &Camera,
) {
    let now = std::time::Instant::now();
    for effect in &game.effects.active {
        let Effect::EngineSmoke {
            pos,
            drift,
            sprite,
            life,
            scale,
            alpha,
            rotation,
            started,
            ..
        } = effect
        else {
            continue;
        };
        let age = now.saturating_duration_since(*started).as_secs_f32();
        let t = (age / life.max(0.01)).clamp(0.0, 1.0);
        if t >= 1.0 {
            continue;
        }
        let world_pos = (pos.0 + drift.0 * t, pos.1 + drift.1 * t);
        if !camera.in_view(world_pos, CULL_MARGIN) {
            continue;
        }
        let Some(clip) = sprite
            .and_then(|id| tex.get(id))
            .filter(|clip| !clip.frames.is_empty())
        else {
            continue;
        };
        let frame = clip_frame_once(clip, age.mul_add(1000.0, 0.0) as u64);
        let (w, h) = (
            clip.frames[frame].tex.width as f32,
            clip.frames[frame].tex.height as f32,
        );
        if w <= 0.0 || h <= 0.0 {
            continue;
        }
        let screen = camera.to_screen(world_pos);
        let growth = 0.8 + t * 0.35;
        let fade = alpha * (1.0 - t).powf(1.4);
        TextureCache::draw_frame_offset(
            d,
            clip,
            frame,
            screen,
            *rotation,
            (camera.zoom * scale * growth, camera.zoom * scale * growth),
            Color::WHITE.alpha(fade),
            (-w * 0.5, -h * 0.5),
        );
    }
}

/// Procedural ring / flash for [`Effect::Pulse`].
fn draw_pulse(d: &mut RaylibDrawHandle, c: (f32, f32), kind: PulseKind, t: f32, zoom: f32) {
    let (rf, alpha) = effects::pulse_shape(kind, t);
    let r = kind.radius() * rf * zoom;
    if r < 1.0 || alpha <= 0.0 {
        return;
    }
    let col = color_u32(kind.color());
    let v = Vector2::new(c.0, c.1);
    match kind {
        PulseKind::InstaShield => {
            d.draw_circle_v(v, r, col.alpha(0.16 * alpha));
            d.draw_ring(
                v,
                (r - 2.5).max(0.5),
                r,
                0.0,
                360.0,
                40,
                col.alpha(0.85 * alpha),
            );
        }
        PulseKind::SpawnIn | PulseKind::SpawnOut => {
            d.draw_ring(v, (r * 0.85).max(0.5), r, 0.0, 360.0, 36, col.alpha(alpha));
            d.draw_circle_v(v, r * 0.5, Color::WHITE.alpha(0.35 * alpha));
            for k in 0..8 {
                let a = k as f32 * std::f32::consts::FRAC_PI_4 + t * 2.0;
                let (ca, sa) = (a.cos(), a.sin());
                d.draw_line_ex(
                    Vector2::new(c.0 + ca * r * 0.4, c.1 + sa * r * 0.4),
                    Vector2::new(c.0 + ca * r * 1.1, c.1 + sa * r * 1.1),
                    1.5,
                    col.alpha(0.6 * alpha),
                );
            }
        }
        PulseKind::Nuke | PulseKind::Smartbomb => {
            d.draw_circle_v(v, r, col.alpha(0.12 * alpha));
            d.draw_ring(
                v,
                (r * 0.8).max(0.5),
                r,
                0.0,
                360.0,
                48,
                col.alpha(0.9 * alpha),
            );
            d.draw_circle_v(v, r * 0.25, Color::WHITE.alpha(0.7 * alpha));
        }
        _ => {
            d.draw_ring(
                v,
                (r * 0.88).max(0.5),
                r,
                0.0,
                360.0,
                40,
                col.alpha(0.9 * alpha),
            );
            d.draw_circle_v(v, r * 0.9, col.alpha(0.08 * alpha));
        }
    }
}

pub(crate) fn draw_effects(
    d: &mut RaylibDrawHandle,
    game: &Game,
    world: &SharedWorld,
    tex: &TextureCache,
    fonts: &Fonts,
    camera: &Camera,
    rendered_3d: &HashSet<usize>,
) {
    let now = std::time::Instant::now();
    let time = d.get_time() as f32;
    for (effect_index, e) in game.effects.active.iter().enumerate() {
        match *e {
            Effect::Beam {
                from,
                to,
                kind,
                life,
                started,
            } => {
                let t = (now.saturating_duration_since(started).as_secs_f32() / life.max(0.01))
                    .clamp(0.0, 1.0);
                if t >= 1.0 {
                    continue;
                }
                let a = camera.to_screen(anchor_pos(world, from));
                let b = camera.to_screen(anchor_pos(world, to));
                let col = color_u32(kind.color());
                match kind {
                    BeamKind::Heal => draw_heal_beam(d, a, b, t, camera.zoom, col, time),
                }
            }
            Effect::CollectorBeam {
                from,
                to,
                sprite,
                life,
                started,
            } => {
                let elapsed = now.saturating_duration_since(started).as_secs_f32();
                let t = (elapsed / life.max(0.01)).clamp(0.0, 1.0);
                let alpha = if t > 0.7 {
                    ((1.0 - t) / 0.3).clamp(0.0, 1.0)
                } else {
                    1.0
                };
                if let Some(clip) = sprite
                    .and_then(|id| tex.get(id))
                    .filter(|clip| !clip.frames.is_empty())
                {
                    let a = camera.to_screen(anchor_pos(world, from));
                    let b = camera.to_screen(anchor_pos(world, to));
                    draw_collector_beam(
                        d,
                        a,
                        b,
                        clip,
                        elapsed.mul_add(1000.0, 0.0) as u64,
                        camera.zoom,
                        alpha,
                    );
                }
            }
            Effect::EngineSmoke { .. } => {}
            Effect::Pulse { at, kind, started } => {
                let t = (now.saturating_duration_since(started).as_secs_f32() / kind.life())
                    .clamp(0.0, 1.0);
                if t >= 1.0 {
                    continue;
                }
                let c = camera.to_screen(anchor_pos(world, at));
                draw_pulse(d, c, kind, t, camera.zoom);
            }
            Effect::Emp {
                at,
                kind,
                flash,
                blitz,
                ref rings,
                started,
            } => {
                let age = now.saturating_duration_since(started).as_secs_f32();
                let c = camera.to_screen(anchor_pos(world, at));
                let sc = camera.zoom;
                let (reach, a0) = match kind {
                    crate::net::EmpKind::Nuke => (430.0, 0.5),
                    crate::net::EmpKind::BurningTrail => (250.0, 0.5),
                    crate::net::EmpKind::Ultimate => (455.0, 0.7),
                    _ => (455.0, 0.3),
                };
                // Emperor rings stagger at `i*0.1*0.3` and grow to a
                // fixed scale 9 (`isEmperor ? 9 : range/(w/2)`).
                let stagger = if kind == crate::net::EmpKind::Emperor {
                    0.03
                } else {
                    0.1
                };
                let mut drew_clip = false;
                // `showEMPRing`: one clip per ring, staggered, each
                // scales 0.1 -> reach/(w/2) while fading over 1.5 s.
                for (i, id) in rings.iter().enumerate() {
                    let t = (age - i as f32 * stagger) / 1.5;
                    if !(0.0..1.0).contains(&t) {
                        continue;
                    }
                    let Some(clip) = tex.get(*id).filter(|c| !c.frames.is_empty()) else {
                        continue;
                    };
                    drew_clip = true;
                    let frame =
                        input::anim_frame(&clip.durations, (age * 1000.0) as u64)
                            .min(clip.frames.len() - 1);
                    let (w, h) = (
                        clip.frames[frame].tex.width as f32,
                        clip.frames[frame].tex.height as f32,
                    );
                    let target = if kind == crate::net::EmpKind::Emperor {
                        9.0
                    } else {
                        reach / (w * 0.5).max(1.0)
                    };
                    let s = (0.1 + t * (target - 0.1)) * sc;
                    TextureCache::draw_frame_offset(
                        d,
                        clip,
                        frame,
                        c,
                        0.0,
                        (s, s),
                        Color::WHITE.alpha(a0 * (1.0 - t)),
                        (-w * 0.5, -h * 0.5),
                    );
                }
                // `blitz` clip: plays at ~15 fps, scales 0.1 -> 3.5
                // (ultimate: 1.5, nuke/burning-trail: 1.0), fades out
                // 0.75 s in.
                if let Some(clip) = blitz
                    .and_then(|id| tex.get(id))
                    .filter(|c| !c.frames.is_empty())
                {
                    drew_clip = true;
                    let frame = ((age * 15.0) as usize) % clip.frames.len();
                    let (w, h) = (
                        clip.frames[frame].tex.width as f32,
                        clip.frames[frame].tex.height as f32,
                    );
                    let end = match kind {
                        crate::net::EmpKind::Nuke | crate::net::EmpKind::BurningTrail => 1.0,
                        crate::net::EmpKind::Ultimate => 1.5,
                        _ => 3.5,
                    };
                    let s = (0.1 + (age / 1.5).min(1.0) * (end - 0.1)) * sc;
                    let a = if age > 0.75 {
                        (1.0 - (age - 0.75) / 0.25).max(0.0)
                    } else {
                        1.0
                    };
                    TextureCache::draw_frame_offset(
                        d,
                        clip,
                        frame,
                        c,
                        0.0,
                        (s, s),
                        Color::WHITE.alpha(a),
                        (-w * 0.5, -h * 0.5),
                    );
                }
                // `lensFlash` bitmap, centred, alpha -> 0 over 0.25 s.
                if age < 0.25 {
                    if let Some(clip) = flash
                        .and_then(|id| tex.get(id))
                        .filter(|c| !c.frames.is_empty())
                    {
                        drew_clip = true;
                        let (w, h) = (
                            clip.frames[0].tex.width as f32,
                            clip.frames[0].tex.height as f32,
                        );
                        TextureCache::draw_frame_offset(
                            d,
                            clip,
                            0,
                            c,
                            0.0,
                            (sc, sc),
                            Color::WHITE.alpha(1.0 - age / 0.25),
                            (-w * 0.5, -h * 0.5),
                        );
                    }
                }
                // Procedural placeholder until the authored clips land.
                if !drew_clip {
                    let pk = match kind {
                        crate::net::EmpKind::Nuke => PulseKind::Nuke,
                        crate::net::EmpKind::BurningTrail => PulseKind::BurningTrail,
                        _ => PulseKind::Emp,
                    };
                    let t = (age / effects::emp_life(kind)).clamp(0.0, 1.0);
                    draw_pulse(d, c, pk, t, sc);
                }
            }
            Effect::ShieldDamage {
                victim,
                angle,
                delay,
                clip_life,
                sprite,
                started,
            } => {
                let age = now.saturating_duration_since(started).as_secs_f32();
                if age < delay || age >= delay + clip_life {
                    continue;
                }
                let pos = world.pos(victim);
                let click_r = game
                    .entity_visuals
                    .get(&victim)
                    .map(|v| v.click_radius)
                    .unwrap_or(input::DEFAULT_CLICK_RADIUS);
                let sc = camera.scale_at(pos);
                let s = camera.to_screen(pos);
                let a = angle.to_radians();
                let Some(clip) = sprite
                    .and_then(|id| tex.get(id))
                    .filter(|c| !c.frames.is_empty())
                else {
                    continue;
                };
                // Played once at a fixed 30 fps (`framesLoaded / 30` s).
                let frame = (((age - delay) * 30.0) as usize).min(clip.frames.len() - 1);
                let (w, h) = (
                    clip.frames[frame].tex.width as f32,
                    clip.frames[frame].tex.height as f32,
                );
                // Ship-view child: `(clickR*cos a, clickR*sin a)` with
                // `scale = clickR/100`, rotated to the hit angle.
                let scale = click_r / 100.0 * sc;
                TextureCache::draw_frame_offset(
                    d,
                    clip,
                    frame,
                    (s.0 + a.cos() * click_r * sc, s.1 + a.sin() * click_r * sc),
                    angle,
                    (scale, scale),
                    Color::WHITE,
                    (-w * 0.5, -h * 0.5),
                );
            }
            Effect::Emote {
                entity,
                sprite,
                started,
            } => {
                let age = now.saturating_duration_since(started).as_secs_f32();
                if age >= 5.0 {
                    continue;
                }
                let pos = world.pos(entity);
                let s = camera.to_screen(pos);
                let click_r = game
                    .entity_visuals
                    .get(&entity)
                    .map(|v| v.click_radius)
                    .unwrap_or(input::DEFAULT_CLICK_RADIUS)
                    * camera.zoom;
                // `EmoteEffect2D`: sits `clickRadius*zoom + 80` px above
                // the ship; pops in 0.2 s, squashes out 0.2 s.
                let (alpha, sx, sy) = if age < 0.2 {
                    let t = age / 0.2;
                    (t, t, t)
                } else if age > 4.8 {
                    let t = (age - 4.8) / 0.2;
                    (1.0 - t, 1.0 + 9.0 * t, (1.0 - t).max(0.01))
                } else {
                    (1.0, 1.0, 1.0)
                };
                let Some(clip) = sprite
                    .and_then(|id| tex.get(id))
                    .filter(|c| !c.frames.is_empty())
                else {
                    continue;
                };
                let frame =
                    input::anim_frame(&clip.durations, (age * 1000.0) as u64)
                        .min(clip.frames.len() - 1);
                let (w, h) = (
                    clip.frames[frame].tex.width as f32,
                    clip.frames[frame].tex.height as f32,
                );
                TextureCache::draw_frame_offset(
                    d,
                    clip,
                    frame,
                    (s.0, s.1 - click_r - 80.0),
                    0.0,
                    (sx, sy),
                    Color::WHITE.alpha(alpha),
                    (-w * 0.5, -h * 0.5),
                );
            }
            Effect::LaserShot {
                from,
                to,
                sprite,
                duration,
                delay,
                absorber,
                play_loop,
                color,
                started,
                ..
            } => {
                if rendered_3d.contains(&effect_index) {
                    continue;
                }
                if now.saturating_duration_since(started).as_secs_f32() < delay {
                    continue;
                }
                let t = e.flight_t(now);
                let pos = e.shot_pos(now);
                let path_len = (to.0 - from.0).hypot(to.1 - from.1).max(1.0);
                // Angle of the *projected* path: under the perspective camera
                // the world-space angle no longer matches what is on screen.
                let (sf, st) = (camera.to_screen(from), camera.to_screen(to));
                let heading = (st.1 - sf.1).atan2(st.0 - sf.0).to_degrees();
                let p = camera.to_screen(pos);
                let sc = camera.scale_at(pos);
                let shrink = if absorber { 1.0 - 0.9 * t } else { 1.0 };
                if let Some(clip) = sprite
                    .and_then(|id| tex.get(id))
                    .filter(|c| !c.frames.is_empty())
                {
                    let (frame, rot) = if play_loop {
                        (
                            input::anim_frame(&clip.durations, (t * duration * 1000.0) as u64)
                                .min(clip.frames.len() - 1),
                            0.0,
                        )
                    } else {
                        (0, heading + 180.0)
                    };
                    TextureCache::draw_frame_offset(
                        d,
                        clip,
                        frame,
                        (p.0, p.1),
                        rot,
                        (sc * shrink, sc * shrink),
                        Color::WHITE,
                        (0.0, 0.0),
                    );
                } else {
                    let screen_len = (st.0 - sf.0).hypot(st.1 - sf.1).max(1.0);
                    let dir = ((st.0 - sf.0) / screen_len, (st.1 - sf.1) / screen_len);
                    let half = ((30.0_f32).min(path_len * 0.5) * sc).max(4.0);
                    let c = color_u32(color);
                    d.draw_line_ex(
                        Vector2::new(p.0 - dir.0 * half, p.1 - dir.1 * half),
                        Vector2::new(p.0 + dir.0 * half, p.1 + dir.1 * half),
                        (2.0 * sc).clamp(1.0, 4.0),
                        c.alpha(0.9 * shrink),
                    );
                }
            }
            Effect::Rocket { .. } => {
                let Effect::Rocket {
                    from,
                    victim,
                    miss_offset,
                    sprite,
                    started,
                    duration,
                    ..
                } = *e
                else {
                    continue;
                };
                if now.saturating_duration_since(started).as_secs_f32() >= duration {
                    continue;
                }
                let target = world.pos(victim);
                let pos = e.rocket_pos(target, now);
                let p = camera.to_screen(pos);
                let aim = (target.0 + miss_offset.0, target.1 + miss_offset.1);
                let (sf, sa) = (camera.to_screen(from), camera.to_screen(aim));
                let heading = (sa.1 - sf.1).atan2(sa.0 - sf.0).to_degrees();
                let sc = camera.scale_at(pos);
                if let Some(clip) = sprite
                    .and_then(|id| tex.get(id))
                    .filter(|c| !c.frames.is_empty())
                {
                    let frame = clip_frame_once(clip, started.elapsed().as_millis() as u64);
                    let (w, h) = (
                        clip.frames[frame].tex.width as f32,
                        clip.frames[frame].tex.height as f32,
                    );
                    TextureCache::draw_frame_offset(
                        d,
                        clip,
                        frame,
                        (p.0, p.1),
                        heading,
                        (sc, sc),
                        Color::WHITE,
                        (-w * 0.5, -h * 0.5),
                    );
                } else {
                    let c = Vector2::new(p.0, p.1);
                    d.draw_circle_v(c, 3.0 * camera.zoom.max(0.5), Color::ORANGE);
                    d.draw_circle_lines_v(c, 5.0 * camera.zoom.max(0.5), Color::RED);
                }
            }
            Effect::SpriteBurst {
                pos,
                sprite,
                life,
                scale,
                fade,
                rotation,
                ring_fallback,
                started,
            } => {
                if rendered_3d.contains(&effect_index) {
                    continue;
                }
                let t = (started.elapsed().as_secs_f32() / life.max(0.01)).clamp(0.0, 1.0);
                if t >= 1.0 {
                    continue;
                }
                let s = camera.to_screen(pos);
                let alpha = if fade && t > 0.6 {
                    ((1.0 - t) / 0.4).clamp(0.0, 1.0)
                } else {
                    1.0
                };
                if let Some(clip) = sprite
                    .and_then(|id| tex.get(id))
                    .filter(|c| !c.frames.is_empty())
                {
                    let frame = clip_frame_once(clip, started.elapsed().as_millis() as u64);
                    let (w, h) = (
                        clip.frames[frame].tex.width as f32,
                        clip.frames[frame].tex.height as f32,
                    );
                    TextureCache::draw_frame_offset(
                        d,
                        clip,
                        frame,
                        (s.0, s.1),
                        rotation,
                        (camera.zoom * scale, camera.zoom * scale),
                        Color::WHITE.alpha(alpha),
                        (-w * 0.5, -h * 0.5),
                    );
                } else if ring_fallback {
                    let r = (10.0 + t * 70.0) * camera.zoom * scale;
                    let fade = 1.0 - t;
                    let c = Vector2::new(s.0, s.1);
                    d.draw_ring(
                        c,
                        (r * 0.55).max(1.0),
                        r,
                        0.0,
                        360.0,
                        32,
                        Color::ORANGE.alpha(fade),
                    );
                    d.draw_circle_v(
                        c,
                        (14.0 * fade + 2.0) * camera.zoom * scale,
                        Color::new(255, 240, 200, 255).alpha(fade * 0.85),
                    );
                }
            }
            Effect::CollectFlash { pos, started } => {
                let t = started.elapsed().as_secs_f32() / 0.35;
                if t >= 1.0 {
                    continue;
                }
                let s = camera.to_screen(pos);
                let c = Vector2::new(s.0, s.1);
                let r = (6.0 + t * 26.0) * camera.zoom;
                d.draw_ring(
                    c,
                    (r - 2.0).max(0.5),
                    r,
                    0.0,
                    360.0,
                    24,
                    Color::new(255, 230, 120, 255).alpha(1.0 - t),
                );
                d.draw_circle_v(
                    c,
                    4.0 * (1.0 - t) + 1.0,
                    Color::WHITE.alpha(0.9 * (1.0 - t)),
                );
            }
            Effect::Text {
                pos,
                ref text,
                color,
                started,
            } => {
                let t = (started.elapsed().as_secs_f32() / effects::TEXT_LIFE).clamp(0.0, 1.0);
                if t >= 1.0 || !world.settings.display_hitpoint_bubbles {
                    continue;
                }
                let s = camera.to_screen(pos);
                let col = color_u32(color).alpha(1.0 - t);
                draw_centered_text(d, fonts, text, s.0, s.1 - t * 26.0, 12.0, col);
            }
        }
    }
}

pub(crate) fn draw_selection(d: &mut RaylibDrawHandle, screen: (f32, f32), r: f32) {
    let col = Color::new(255, 214, 80, 230);
    let c = Vector2::new(screen.0, screen.1);
    d.draw_ring(c, r, r + 1.6, 0.0, 360.0, 36, col.alpha(0.75));
    let len = (r * 0.45).clamp(4.0, 12.0);
    for k in 0..4 {
        let a = k as f32 * std::f32::consts::FRAC_PI_2 + std::f32::consts::FRAC_PI_4;
        let (ca, sa) = (a.cos(), a.sin());
        d.draw_line_ex(
            Vector2::new(screen.0 + ca * (r + 2.0), screen.1 + sa * (r + 2.0)),
            Vector2::new(
                screen.0 + ca * (r + 2.0 + len),
                screen.1 + sa * (r + 2.0 + len),
            ),
            2.0,
            col,
        );
    }
}

pub(crate) fn draw_wreck(d: &mut RaylibDrawHandle, screen: (f32, f32), zoom: f32) {
    let r = (11.0 * zoom).clamp(5.0, 26.0);
    let c = Vector2::new(screen.0, screen.1);
    d.draw_circle_v(c, r, Color::new(60, 62, 70, 160));
    d.draw_circle_lines_v(c, r, Color::new(140, 145, 155, 140));
    let d45 = r * 0.55;
    d.draw_line_ex(
        Vector2::new(screen.0 - d45, screen.1 - d45),
        Vector2::new(screen.0 + d45, screen.1 + d45),
        2.0,
        Color::new(170, 175, 185, 160),
    );
    d.draw_line_ex(
        Vector2::new(screen.0 - d45, screen.1 + d45),
        Vector2::new(screen.0 + d45, screen.1 - d45),
        2.0,
        Color::new(170, 175, 185, 160),
    );
}
