//! Authored combat clips in the 3D scene. DarkOrbit's 3D client draws
//! laser bolts as `LaserBullet3D` projectiles — two crossed textured
//! planes (`"doublePlane"`, the default), a single flight-perpendicular
//! plane (`"singlePlane"`) or a particle group (`"particle"`,
//! `{resKey}.zip`) — and pyro clips as camera-facing billboards.
//! Effects whose authored representation (or its assets) are unavailable
//! keep their normal 2D renderer instead of being replaced by invented
//! geometry.

use crate::{
    game::effects::{Effect, LaserKind3D, LaserShot3D},
    game::{input, Game},
    render::{
        draw_util::clip_frame_once,
        particles3d, scene3d,
        sprites::{GpuClip, TextureCache},
    },
};
use raylib::prelude::*;
use std::{collections::HashSet, time::Instant};

type V3 = (f32, f32, f32);

fn vsub(a: V3, b: V3) -> V3 {
    (a.0 - b.0, a.1 - b.1, a.2 - b.2)
}

fn vscale(a: V3, s: f32) -> V3 {
    (a.0 * s, a.1 * s, a.2 * s)
}

fn vlen(a: V3) -> f32 {
    (a.0 * a.0 + a.1 * a.1 + a.2 * a.2).sqrt()
}

fn vnorm(a: V3) -> Option<V3> {
    let l = vlen(a);
    (l > 1e-5).then(|| vscale(a, 1.0 / l))
}

fn vcross(a: V3, b: V3) -> V3 {
    (
        a.1 * b.2 - a.2 * b.1,
        a.2 * b.0 - a.0 * b.2,
        a.0 * b.1 - a.1 * b.0,
    )
}

/// Away3D `Object3D.lookAt` basis (all vectors in Away3D coordinates):
/// `zAxis = normalize(dir)` is local +Z, `xAxis = normalize(up × zAxis)`
/// (`up` = `Vector3D.Z_AXIS` for bolt meshes, `Y_AXIS` for particle
/// groups) and `yAxis = zAxis × xAxis`. Returns `(x, y, z)` axes.
fn lookat_basis(dir: V3, up: V3) -> Option<(V3, V3, V3)> {
    let z = vnorm(dir)?;
    let mut x = vcross(up, z);
    if vlen(x) < 0.05 {
        // Stock fallback when the flight is ~parallel to the up hint.
        x = (up.1, up.0, 0.0);
    }
    let x = vnorm(x)?;
    let y = vnorm(vcross(z, x))?;
    Some((x, y, z))
}

/// Quaternion `[w, x, y, z]` whose columns are the basis vectors.
fn basis_quat(x: V3, y: V3, z: V3) -> [f32; 4] {
    // Rotation matrix rows (basis vectors are columns).
    let m = [[x.0, y.0, z.0], [x.1, y.1, z.1], [x.2, y.2, z.2]];
    let t = m[0][0] + m[1][1] + m[2][2];
    if t > 0.0 {
        let s = (t + 1.0).sqrt() * 2.0;
        [
            s * 0.25,
            (m[2][1] - m[1][2]) / s,
            (m[0][2] - m[2][0]) / s,
            (m[1][0] - m[0][1]) / s,
        ]
    } else if m[0][0] > m[1][1] && m[0][0] > m[2][2] {
        let s = (1.0 + m[0][0] - m[1][1] - m[2][2]).sqrt() * 2.0;
        [
            (m[2][1] - m[1][2]) / s,
            s * 0.25,
            (m[0][1] + m[1][0]) / s,
            (m[0][2] + m[2][0]) / s,
        ]
    } else if m[1][1] > m[2][2] {
        let s = (1.0 + m[1][1] - m[0][0] - m[2][2]).sqrt() * 2.0;
        [
            (m[0][2] - m[2][0]) / s,
            (m[0][1] + m[1][0]) / s,
            s * 0.25,
            (m[1][2] + m[2][1]) / s,
        ]
    } else {
        let s = (1.0 + m[2][2] - m[0][0] - m[1][1]).sqrt() * 2.0;
        [
            (m[1][0] - m[0][1]) / s,
            (m[0][2] + m[2][0]) / s,
            (m[1][2] + m[2][1]) / s,
            s * 0.25,
        ]
    }
}

/// Mirror an Away3D-space vector into raylib space (`rl_point`).
fn to_rl(v: V3) -> Vector3 {
    Vector3::new(v.0, v.1, -v.2)
}

/// What `draw_laser3d` did with the effect: `Fallback` = let the 2D
/// billboard/fallback path draw it; `Claimed` = the authored 3D path
/// handled it (drawn, queued, or dropped by `checkDistance`).
#[derive(PartialEq)]
enum Laser3D {
    Fallback,
    Claimed,
}

/// Textured double-sided quad via raw rl calls (called inside
/// `begin_mode3D`) — Away3D builds the bolt planes with
/// `doubleSided=true`, so both windings are emitted.
fn draw_tex_quad(tex_id: u32, corners: &[Vector3; 4], uvs: &[(f32, f32); 4], color: Color) {
    unsafe {
        raylib::ffi::rlSetTexture(tex_id);
        raylib::ffi::rlBegin(raylib::ffi::RL_QUADS as i32);
        raylib::ffi::rlColor4ub(color.r, color.g, color.b, color.a);
        for order in [[0usize, 1, 2, 3], [3, 2, 1, 0]] {
            for i in order {
                raylib::ffi::rlTexCoord2f(uvs[i].0, uvs[i].1);
                let c = corners[i];
                raylib::ffi::rlVertex3f(c.x, c.y, c.z);
            }
        }
        raylib::ffi::rlEnd();
        raylib::ffi::rlSetTexture(0);
    }
}

/// Stock `LaserSalvo3D.shoot` geometry for tween time `t` in [0,1]:
/// `(position, orient_dir, tween_end)` in Away3D coordinates — the
/// shield-pulled endpoint, `checkDistance` drop and absorber swap are
/// all applied. `orient_dir` is the FIXED fire-time flight direction
/// (`orientTo(_loc5_)` runs once at spawn, so absorber bolts keep their
/// heading while overshooting the attacker). `None` = stock drops the
/// bolt entirely.
fn bolt_tween(v: &LaserShot3D, absorber: bool, t: f32) -> Option<(V3, V3, V3)> {
    let span = vsub(v.end, v.spawn);
    // checkDistance before the shield pullback.
    if vlen(span) <= v.bullet_len {
        return None;
    }
    let dir = vnorm(span)?;
    let end = vsub(v.end, vscale(dir, v.shield_pull));
    // checkDistance after the shield pullback.
    if vlen(vsub(end, v.spawn)) <= v.bullet_len {
        return None;
    }
    // Absorber shots visually fly victim -> attacker (`_loc4_/_loc5_` swap).
    let (a, b) = if absorber {
        (end, v.spawn)
    } else {
        (v.spawn, end)
    };
    let flight = vnorm(vsub(b, a))?;
    // TweenLite target: `_loc5_ - _loc6_ * bulletLength` — `_loc6_` keeps
    // the pre-swap attacker->victim direction, so absorber bolts carry
    // one bolt length PAST the attacker.
    let tween_end = vsub(b, vscale(dir, v.bullet_len));
    let pos = (
        a.0 + (tween_end.0 - a.0) * t,
        a.1 + (tween_end.1 - a.1) * t,
        a.2 + (tween_end.2 - a.2) * t,
    );
    Some((pos, flight, tween_end))
}

/// Draw/queue the authored 3D body of one laser bolt at tween time `t`.
/// `key` uniquely identifies this `Effect` instance (stable across
/// frames) for the particle-instance cache.
fn draw_laser3d(
    tex: &TextureCache,
    key: &str,
    v: &LaserShot3D,
    absorber: bool,
    t: f32,
    clip: Option<&GpuClip>,
) -> Laser3D {
    // Resource gate first — an unready 3D asset falls back to the 2D
    // path instead of drawing nothing.
    let (x, y, z, pos) = match v.kind {
        LaserKind3D::Particles => {
            if v.particle.is_empty() || !tex.particle_group_ready(&v.particle) {
                return Laser3D::Fallback;
            }
            let Some((pos, flight, _)) = bolt_tween(v, absorber, t) else {
                return Laser3D::Claimed;
            };
            // `orientTo` -> `_particles.lookAt(TMP)` with the default
            // `Vector3D.Y_AXIS` up hint.
            let Some(b) = lookat_basis(flight, (0.0, 1.0, 0.0)) else {
                return Laser3D::Claimed;
            };
            (b.0, b.1, b.2, pos)
        }
        LaserKind3D::Cross | LaserKind3D::Single => {
            if !clip.is_some_and(|c| !c.frames.is_empty()) {
                return Laser3D::Fallback;
            }
            let Some((pos, flight, _)) = bolt_tween(v, absorber, t) else {
                return Laser3D::Claimed;
            };
            // `_mesh.lookAt(TMP, Vector3D.Z_AXIS)`.
            let Some(b) = lookat_basis(flight, (0.0, 0.0, 1.0)) else {
                return Laser3D::Claimed;
            };
            (b.0, b.1, b.2, pos)
        }
    };
    // `pattern.scale`, tweened toward 0.4 for absorbers.
    let s = if absorber {
        v.scale + (0.4 - v.scale) * t
    } else {
        v.scale
    };
    // Instance keys stay stable even when `index` shifts as older
    // effects expire — the spawn point distinguishes volley members.
    let key = format!("{key}:{:.0}:{:.0}:{:.0}", v.spawn.0, v.spawn.1, v.spawn.2);
    match v.kind {
        LaserKind3D::Particles => {
            let rot = scene3d::quat_to_euler(&basis_quat(x, y, z));
            particles3d::queue(particles3d::Request {
                key: format!("laser:{key}"),
                resource: v.particle.clone(),
                pos,
                rot,
                scale: (s, s, s),
            });
        }
        LaserKind3D::Cross | LaserKind3D::Single => {
            let Some(clip) = clip.filter(|c| !c.frames.is_empty()) else {
                return Laser3D::Fallback;
            };
            let gpu = &clip.frames[0];
            let (w, h) = (gpu.tex.width as f32 * s, gpu.tex.height as f32 * s);
            if w <= 0.0 || h <= 0.0 {
                return Laser3D::Claimed;
            }
            let center = to_rl(pos);
            let (xa, ya, za) = (to_rl(x), to_rl(y), to_rl(z));
            if v.kind == LaserKind3D::Cross {
                // Two crossed unit strips spanning z in [0,1] from the
                // bolt origin: mesh scale (H, H, W) gives a bolt `W*s`
                // long and `H*s` thick. Texture u runs base(1) -> tip(0).
                let tip = center + za * w;
                let hh = h * 0.5;
                for axis in [xa, ya] {
                    let o = axis * hh;
                    let corners = [center - o, center + o, tip + o, tip - o];
                    draw_tex_quad(
                        gpu.tex.id,
                        &corners,
                        &[(1.0, 0.0), (1.0, 1.0), (0.0, 1.0), (0.0, 0.0)],
                        Color::WHITE,
                    );
                }
            } else {
                // `singlePlane`: one quad perpendicular to the flight,
                // twice the clip extent (`scaleX/Y = height*2`, the
                // `scaleZ` is dead on a flat plane).
                let hs = h;
                let (ox, oy) = (xa * hs, ya * hs);
                let corners = [
                    center - ox - oy,
                    center + ox - oy,
                    center + ox + oy,
                    center - ox + oy,
                ];
                draw_tex_quad(
                    gpu.tex.id,
                    &corners,
                    &[(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)],
                    Color::WHITE,
                );
            }
        }
    }
    // `shotSalvo` muzzle flash: a `{muzzleFlashResKey}.zip` one-shot
    // group at the authored cannon slot (ship+mesh world rotation).
    if let Some(m) = &v.muzzle {
        particles3d::queue(particles3d::Request {
            key: format!("laser-mzl:{key}"),
            resource: m.resource.clone(),
            pos: v.spawn,
            rot: m.rot,
            scale: (1.0, 1.0, 1.0),
        });
    }
    Laser3D::Claimed
}

fn world_units_per_pixel(game: &Game) -> f32 {
    let height = game.camera.viewport.1.max(1.0);
    if game.preferences.render.camera_3d {
        let distance = 1740.0 / game.camera.zoom.max(0.05);
        (2.0 * distance * 15.0_f32.to_radians().tan()) / height
    } else {
        1.0 / game.camera.zoom.max(0.05)
    }
}

fn camera_facing_up(camera: Camera3D) -> Vector3 {
    let delta = Vector3::new(
        camera.target.x - camera.position.x,
        camera.target.y - camera.position.y,
        camera.target.z - camera.position.z,
    );
    let length = (delta.x * delta.x + delta.y * delta.y + delta.z * delta.z)
        .sqrt()
        .max(0.0001);
    let forward = Vector3::new(delta.x / length, delta.y / length, delta.z / length);
    let right0 = Vector3::new(
        forward.y * camera.up.z - forward.z * camera.up.y,
        forward.z * camera.up.x - forward.x * camera.up.z,
        forward.x * camera.up.y - forward.y * camera.up.x,
    );
    let right_len = (right0.x * right0.x + right0.y * right0.y + right0.z * right0.z)
        .sqrt()
        .max(0.0001);
    let right = Vector3::new(
        right0.x / right_len,
        right0.y / right_len,
        right0.z / right_len,
    );
    Vector3::new(
        right.y * forward.z - right.z * forward.y,
        right.z * forward.x - right.x * forward.z,
        right.x * forward.y - right.y * forward.x,
    )
}

/// Draw only clips that have an extracted GPU frame. The returned indices
/// tell the 2D pass exactly which effects were handled, so absent assets
/// remain visible through the established 2D fallback.
pub(crate) fn draw_effects_3d(
    d: &mut RaylibDrawHandle<'_>,
    game: &Game,
    tex: &TextureCache,
) -> HashSet<usize> {
    let settings = game.preferences.render;
    if !settings.effects_3d && !settings.explosions_3d {
        return HashSet::new();
    }
    let now = Instant::now();
    let view = scene3d::camera(game);
    let view_up = camera_facing_up(view);
    let scale = world_units_per_pixel(game);
    let mut rendered = HashSet::new();
    let mut mode = d.begin_mode3D(view);
    for (index, effect) in game.effects.active.iter().enumerate() {
        match effect {
            Effect::LaserShot {
                from,
                to,
                sprite,
                duration,
                delay,
                absorber,
                play_loop,
                vfx,
                started,
                ..
            } if settings.effects_3d => {
                let elapsed = now.saturating_duration_since(*started).as_secs_f32() - delay;
                if elapsed < 0.0 {
                    continue;
                }
                let t = (elapsed / duration.max(0.01)).clamp(0.0, 1.0);
                let clip = sprite.and_then(|id| tex.get(id));
                // The authored 3D projectile first; it claims the effect
                // only when its assets are actually drawn/queued (or the
                // stock `checkDistance` dropped the bolt).
                if let Some(v) = vfx {
                    // `started` + the spawn point (appended inside) are
                    // stable across `index` shifts as older effects die.
                    let key = format!("{started:?}");
                    if draw_laser3d(tex, &key, v, *absorber, t, clip) == Laser3D::Claimed {
                        rendered.insert(index);
                        continue;
                    }
                }
                // Billboard fallback — unchanged for unready 3D assets
                // and effects spawned without `vfx`.
                let Some(clip) = clip.filter(|clip| !clip.frames.is_empty()) else {
                    continue;
                };
                let pos = (from.0 + (to.0 - from.0) * t, from.1 + (to.1 - from.1) * t);
                let frame = if *play_loop {
                    input::anim_frame(&clip.durations, (t * duration * 1000.0) as u64)
                        .min(clip.frames.len() - 1)
                } else {
                    0
                };
                let gpu = &clip.frames[frame];
                let px_scale =
                    scale * game.camera.zoom * if *absorber { 1.0 - 0.9 * t } else { 1.0 };
                let center = scene3d::rl_point(pos.0, 8.0, -pos.1);
                let from_screen = game.camera.to_screen(*from);
                let to_screen = game.camera.to_screen(*to);
                let heading = (to_screen.1 - from_screen.1)
                    .atan2(to_screen.0 - from_screen.0)
                    .to_degrees()
                    + 180.0;
                let frame_offset = gpu.offset;
                let w = gpu.tex.width as f32;
                let h = gpu.tex.height as f32;
                mode.draw_billboard_pro(
                    view,
                    **&gpu.tex,
                    Rectangle::new(0.0, 0.0, w, h),
                    center,
                    view_up,
                    Vector2::new(w * px_scale, h * px_scale),
                    Vector2::new(-frame_offset.0 * px_scale, (frame_offset.1 + h) * px_scale),
                    // raylib rotates about right x up (counter-clockwise on
                    // screen); `heading` is a clockwise screen angle.
                    if *play_loop { 0.0 } else { -heading },
                    Color::WHITE,
                );
                rendered.insert(index);
            }
            Effect::SpriteBurst {
                pos,
                sprite: Some(id),
                life,
                scale: sprite_scale,
                fade,
                rotation,
                started,
                ..
            } if settings.explosions_3d => {
                let Some(clip) = tex.get(*id).filter(|clip| !clip.frames.is_empty()) else {
                    continue;
                };
                let elapsed = now.saturating_duration_since(*started);
                let t = (elapsed.as_secs_f32() / life.max(0.01)).clamp(0.0, 1.0);
                if t >= 1.0 {
                    continue;
                }
                let frame = clip_frame_once(clip, elapsed.as_millis() as u64);
                let gpu = &clip.frames[frame];
                let (w, h) = (gpu.tex.width as f32, gpu.tex.height as f32);
                let alpha = if *fade && t > 0.6 {
                    ((1.0 - t) / 0.4).clamp(0.0, 1.0)
                } else {
                    1.0
                };
                let factor = scale * game.camera.zoom * sprite_scale;
                let frame_offset = gpu.offset;
                mode.draw_billboard_pro(
                    view,
                    **&gpu.tex,
                    Rectangle::new(0.0, 0.0, w, h),
                    scene3d::rl_point(pos.0, 10.0, -pos.1),
                    view_up,
                    Vector2::new(w * factor, h * factor),
                    Vector2::new(
                        (w * 0.5 - frame_offset.0) * factor,
                        (h * 0.5 + frame_offset.1) * factor,
                    ),
                    -*rotation,
                    Color::WHITE.alpha(alpha),
                );
                rendered.insert(index);
            }
            _ => {}
        }
    }
    rendered
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::particles::{euler_quat, quat_rotate};

    fn vfx() -> LaserShot3D {
        LaserShot3D {
            kind: LaserKind3D::Cross,
            scale: 1.0,
            spawn: (0.0, 0.0, 0.0),
            end: (100.0, 0.0, 0.0),
            shield_pull: 0.0,
            bullet_len: 40.0,
            muzzle: None,
            particle: String::new(),
        }
    }

    fn approx3(a: V3, b: V3) {
        for k in 0..3 {
            let (av, bv) = ([a.0, a.1, a.2][k], [b.0, b.1, b.2][k]);
            assert!((av - bv).abs() < 1e-4, "{a:?} != {b:?}");
        }
    }

    #[test]
    fn bolt_tween_drops_shorter_than_bullet() {
        // dist 100 > bulletLength 40 -> fired.
        assert!(bolt_tween(&vfx(), false, 0.0).is_some());
        let mut v = vfx();
        v.end = (30.0, 0.0, 0.0);
        // `checkDistance` drops it — for absorbers too (the check runs
        // before the swap).
        assert!(bolt_tween(&v, false, 0.0).is_none());
        assert!(bolt_tween(&v, true, 0.0).is_none());
    }

    #[test]
    fn bolt_tween_pulls_back_shielded_endpoint() {
        let mut v = vfx();
        v.shield_pull = 50.0; // `_visualSize * 50` with visualSize 1
        let (pos, flight, _) = bolt_tween(&v, false, 1.0).unwrap();
        // Endpoint pulled to x=50; the tween target then sits
        // `bulletLength` short of it (x=10); flight stays +x.
        approx3(flight, (1.0, 0.0, 0.0));
        approx3(pos, (10.0, 0.0, 0.0));
        // The pulled-back drop check applies too: 50 < 60 -> nothing.
        v.bullet_len = 60.0;
        assert!(bolt_tween(&v, false, 0.0).is_none());
    }

    #[test]
    fn bolt_tween_absorber_flies_victim_to_attacker() {
        let v = vfx();
        let (pos0, _, _) = bolt_tween(&v, true, 0.0).unwrap();
        let (pos1, flight, _) = bolt_tween(&v, true, 1.0).unwrap();
        // Starts at the victim end...
        approx3(pos0, (100.0, 0.0, 0.0));
        // ...oriented at the attacker (fixed -x flight direction)...
        approx3(flight, (-1.0, 0.0, 0.0));
        // ...and the tween target carries `bulletLength` PAST the
        // attacker (`_loc6_` keeps the pre-swap direction).
        approx3(pos1, (-40.0, 0.0, 0.0));
    }

    #[test]
    fn bolt_tween_linear_flight() {
        let v = vfx();
        // target = end(100) - dir*40 = 60; midpoint t=0.5 -> x=30.
        let (pos, _, _) = bolt_tween(&v, false, 0.5).unwrap();
        approx3(pos, (30.0, 0.0, 0.0));
    }

    #[test]
    fn lookat_z_axis_points_along_flight() {
        // `lookAt(TMP, Z_AXIS)` for a bolt flying +x.
        let (x, y, z) = lookat_basis((1.0, 0.0, 0.0), (0.0, 0.0, 1.0)).unwrap();
        approx3(z, (1.0, 0.0, 0.0));
        approx3(x, (0.0, 1.0, 0.0)); // Z x X = Y
        approx3(y, (0.0, 0.0, 1.0)); // X x Y = Z
                                     // A diagonal flight keeps |z|=1.
        let (_, _, z) = lookat_basis((3.0, 0.0, -4.0), (0.0, 0.0, 1.0)).unwrap();
        approx3(z, (0.6, 0.0, -0.8));
        // Degenerate (zero) direction has no basis.
        assert!(lookat_basis((0.0, 0.0, 0.0), (0.0, 0.0, 1.0)).is_none());
    }

    #[test]
    fn basis_quat_roundtrips_through_euler() {
        let (x, y, z) = lookat_basis((0.3, 0.2, -0.9), (0.0, 1.0, 0.0)).unwrap();
        let q = basis_quat(x, y, z);
        let e = scene3d::quat_to_euler(&q);
        let q2 = euler_quat(&[e.0, e.1, e.2]);
        for v in [(1.0, 0.0, 0.0), (0.0, 1.0, 0.0), (0.0, 0.0, 1.0)] {
            let a = quat_rotate(&q, &[v.0, v.1, v.2]);
            let b = quat_rotate(&q2, &[v.0, v.1, v.2]);
            for k in 0..3 {
                assert!((a[k] - b[k]).abs() < 1e-4, "{a:?} != {b:?}");
            }
        }
    }
}
