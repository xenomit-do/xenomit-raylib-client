//! Away3D particle groups (`UberParticles3D` / `Particle3D`) drawn in the
//! right-handed raylib scene.
//!
//! Callers describe *where* an effect lives in Away3D coordinates with
//! [`queue`] while they draw their meshes; [`flush`] then evaluates every
//! queued group on the CPU (`assets::particles`) and draws it once, with
//! additive or alpha blending and no depth writes, like the stock
//! `TextureMaterial` particles.

use crate::assets::particles::{
    euler_quat, quat_mul, quat_rotate, Blend, Particle, ParticleAnim, Quad, Shape,
};
use crate::game::Game;
use crate::render::{scene3d, sprites::TextureCache};
use raylib::prelude::*;
use std::cell::RefCell;
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// One effect instance to draw this frame, in Away3D scene coordinates.
pub struct Request {
    /// Stable identity (`e12:0`, `c<hash>:0`): keeps start time and random
    /// properties across frames.
    pub key: String,
    /// Particle collection id (`box_2`).
    pub resource: String,
    pub pos: (f32, f32, f32),
    /// Euler degrees (Z*Y*X, Away3D).
    pub rot: (f32, f32, f32),
    pub scale: (f32, f32, f32),
}

struct Instance {
    started: Instant,
    last_seen: Instant,
    anims: Vec<Vec<Particle>>,
}

thread_local! {
    static QUEUE: RefCell<Vec<Request>> = const { RefCell::new(Vec::new()) };
    static INSTANCES: RefCell<HashMap<String, Instance>> = RefCell::new(HashMap::new());
}

/// Ask for `request` to be drawn by the next [`flush`].
pub fn queue(request: Request) {
    QUEUE.with(|q| q.borrow_mut().push(request));
}

/// True when every particle node of `recipe` has its group, textures and
/// meshes ready (or definitively unavailable), so 3D can replace the 2D art.
pub fn recipe_ready(tex: &TextureCache, recipe: &crate::assets::uberlib::Display3D) -> bool {
    !recipe.particles.is_empty()
        && recipe
            .particles
            .iter()
            .all(|p| tex.particle_group_ready(&p.resource))
}

fn v3(v: [f32; 3]) -> Vector3 {
    Vector3::new(v[0], v[1], v[2])
}

struct Basis {
    right: Vector3,
    up: Vector3,
    forward: Vector3,
}

fn basis(camera: &Camera3D) -> Basis {
    let sub = |a: Vector3, b: Vector3| Vector3::new(a.x - b.x, a.y - b.y, a.z - b.z);
    let cross = |a: Vector3, b: Vector3| {
        Vector3::new(
            a.y * b.z - a.z * b.y,
            a.z * b.x - a.x * b.z,
            a.x * b.y - a.y * b.x,
        )
    };
    let norm = |a: Vector3| {
        let l = (a.x * a.x + a.y * a.y + a.z * a.z).sqrt().max(1e-6);
        Vector3::new(a.x / l, a.y / l, a.z / l)
    };
    let forward = norm(sub(camera.target, camera.position));
    let right = norm(cross(forward, camera.up));
    let up = cross(right, forward);
    Basis { right, up, forward }
}

/// Where particle groups, textures and external meshes come from: the game's
/// [`TextureCache`], or loose files in the `view3d` example.
pub trait ParticleResources {
    fn group(&self, id: &str) -> Option<std::sync::Arc<crate::assets::particles::ParticleGroup>>;
    /// OpenGL texture id of a particle texture.
    fn texture_id(&self, id: &str) -> Option<u32>;
    fn model(&self, geometry: &str, texture: &str) -> Option<&Model>;
}

impl ParticleResources for TextureCache {
    fn group(&self, id: &str) -> Option<std::sync::Arc<crate::assets::particles::ParticleGroup>> {
        self.particle_group(id).cloned()
    }

    fn texture_id(&self, id: &str) -> Option<u32> {
        self.particle_texture(id).map(|t| t.id)
    }

    fn model(&self, geometry: &str, texture: &str) -> Option<&Model> {
        self.particle_model(geometry, texture)
    }
}

enum Item {
    Quad {
        texture: Option<String>,
        corners: [Vector3; 4],
        uv: [f32; 4],
        color: Color,
    },
    Mesh {
        geometry: String,
        texture: String,
        pos: Vector3,
        axis: Vector3,
        angle: f32,
        scale: Vector3,
        color: Color,
    },
}

struct Drawn {
    distance: f32,
    blend: Blend,
    item: Item,
}

fn quat_from_x_to(dir: [f32; 3]) -> Option<[f32; 4]> {
    let len = (dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2]).sqrt();
    if len < 1e-4 {
        return None;
    }
    let d = [dir[0] / len, dir[1] / len, dir[2] / len];
    // Rotation taking +X to `d`: axis = X x d.
    let axis = [0.0, -d[2], d[1]];
    let axis_len = (axis[1] * axis[1] + axis[2] * axis[2]).sqrt();
    if axis_len < 1e-5 {
        return Some(if d[0] >= 0.0 {
            [1.0, 0.0, 0.0, 0.0]
        } else {
            [0.0, 0.0, 1.0, 0.0]
        });
    }
    Some(crate::assets::particles::quat_axis(
        &[0.0, axis[1] / axis_len, axis[2] / axis_len],
        d[0].clamp(-1.0, 1.0).acos(),
    ))
}

fn tint(color: [f32; 4]) -> Color {
    Color::new(
        (color[0] * 255.0) as u8,
        (color[1] * 255.0) as u8,
        (color[2] * 255.0) as u8,
        (color[3] * 255.0) as u8,
    )
}

#[allow(clippy::too_many_arguments)]
fn collect(
    anim: &ParticleAnim,
    q: &Quad,
    req: &Request,
    qb: &[f32; 4],
    qi: &[f32; 4],
    base: &Basis,
    camera_pos: Vector3,
    out: &mut Vec<Drawn>,
) {
    let sb = [req.scale.0, req.scale.1, req.scale.2];
    let si = anim.scale;
    // instance space -> base space -> world (Away3D).
    let local = [q.pos[0] * si[0], q.pos[1] * si[1], q.pos[2] * si[2]];
    let in_base = quat_rotate(qi, &local);
    let in_base = [
        anim.position[0] + in_base[0],
        anim.position[1] + in_base[1],
        anim.position[2] + in_base[2],
    ];
    let scaled = [in_base[0] * sb[0], in_base[1] * sb[1], in_base[2] * sb[2]];
    let rotated = quat_rotate(qb, &scaled);
    let pw = [
        req.pos.0 + rotated[0],
        req.pos.1 + rotated[1],
        req.pos.2 + rotated[2],
    ];
    let scale = [
        q.scale[0] * si[0] * sb[0],
        q.scale[1] * si[1] * sb[1],
        q.scale[2] * si[2] * sb[2],
    ];
    let center = scene3d::rl_point(pw[0], pw[1], pw[2]);
    let distance = {
        let d = Vector3::new(
            center.x - camera_pos.x,
            center.y - camera_pos.y,
            center.z - camera_pos.z,
        );
        d.x * d.x + d.y * d.y + d.z * d.z
    };
    let color = tint(q.color);
    if color.a == 0 {
        return;
    }
    match &anim.shape {
        Shape::Plane { w, h } => {
            let (hw, hh) = (w * 0.5 * scale[0], h * 0.5 * scale[1]);
            let local_corners = [
                [-hw, -hh, 0.0],
                [hw, -hh, 0.0],
                [hw, hh, 0.0],
                [-hw, hh, 0.0],
            ];
            let corners: [Vector3; 4] = if q.billboard {
                // `ParticleBillboardNode`: local x/y become camera right/up.
                let mut rot = q.rot;
                if q.rotate_to_heading {
                    let v = quat_rotate(qb, &quat_rotate(qi, &q.velocity));
                    let v_rl = scene3d::rl_point(v[0], v[1], v[2]);
                    let (vx, vy) = (
                        v_rl.x * base.right.x + v_rl.y * base.right.y + v_rl.z * base.right.z,
                        v_rl.x * base.up.x + v_rl.y * base.up.y + v_rl.z * base.up.z,
                    );
                    if vx * vx + vy * vy > 1e-6 {
                        let heading =
                            crate::assets::particles::quat_axis(&[0.0, 0.0, 1.0], vy.atan2(vx));
                        rot = quat_mul(&heading, &rot);
                    }
                }
                std::array::from_fn(|k| {
                    let c = quat_rotate(&rot, &local_corners[k]);
                    Vector3::new(
                        center.x + base.right.x * c[0] + base.up.x * c[1] + base.forward.x * c[2],
                        center.y + base.right.y * c[0] + base.up.y * c[1] + base.forward.y * c[2],
                        center.z + base.right.z * c[0] + base.up.z * c[1] + base.forward.z * c[2],
                    )
                })
            } else {
                let mut qt = quat_mul(qb, &quat_mul(qi, &q.rot));
                if q.rotate_to_heading {
                    let v = quat_rotate(qb, &quat_rotate(qi, &q.velocity));
                    if let Some(heading) = quat_from_x_to(v) {
                        qt = quat_mul(&heading, &qt);
                    }
                }
                std::array::from_fn(|k| {
                    let c = quat_rotate(&qt, &local_corners[k]);
                    scene3d::rl_point(pw[0] + c[0], pw[1] + c[1], pw[2] + c[2])
                })
            };
            out.push(Drawn {
                distance,
                blend: anim.blend,
                item: Item::Quad {
                    texture: anim.texture.clone(),
                    corners,
                    uv: q.uv,
                    color,
                },
            });
        }
        Shape::External(geometry) => {
            let qt = quat_mul(qb, &quat_mul(qi, &q.rot));
            let (axis, angle) = scene3d::mirrored_axis_angle(&qt);
            out.push(Drawn {
                distance,
                blend: anim.blend,
                item: Item::Mesh {
                    geometry: geometry.clone(),
                    texture: anim.texture.clone().unwrap_or_default(),
                    pos: center,
                    axis,
                    angle,
                    scale: v3(scale),
                    color,
                },
            });
        }
        Shape::Unsupported => {}
    }
}

fn set_blend(blend: Blend) {
    unsafe {
        raylib::ffi::rlDrawRenderBatchActive();
        raylib::ffi::BeginBlendMode(match blend {
            Blend::Add => raylib::ffi::BlendMode::BLEND_ADDITIVE as i32,
            Blend::Normal => raylib::ffi::BlendMode::BLEND_ALPHA as i32,
        });
    }
}

fn draw_quad(
    res: &impl ParticleResources,
    texture: &Option<String>,
    corners: &[Vector3; 4],
    uv: &[f32; 4],
    color: Color,
) {
    let id = match texture {
        Some(name) => match res.texture_id(name) {
            Some(id) => id,
            None => return,
        },
        None => 0,
    };
    unsafe {
        raylib::ffi::rlSetTexture(id);
        raylib::ffi::rlBegin(raylib::ffi::RL_QUADS as i32);
        raylib::ffi::rlColor4ub(color.r, color.g, color.b, color.a);
        // corners: bottom-left, bottom-right, top-right, top-left.
        let coords = [
            (uv[0], uv[3]),
            (uv[2], uv[3]),
            (uv[2], uv[1]),
            (uv[0], uv[1]),
        ];
        for (c, (u, v)) in corners.iter().zip(coords) {
            raylib::ffi::rlTexCoord2f(u, v);
            raylib::ffi::rlVertex3f(c.x, c.y, c.z);
        }
        raylib::ffi::rlEnd();
        raylib::ffi::rlSetTexture(0);
    }
}

/// Evaluate and draw every queued effect. Call after the opaque 3D passes.
pub(crate) fn flush(d: &mut RaylibDrawHandle<'_>, game: &Game, tex: &TextureCache) {
    let requests = QUEUE.with(|q| std::mem::take(&mut *q.borrow_mut()));
    if requests.is_empty() {
        let now = Instant::now();
        INSTANCES.with(|i| {
            i.borrow_mut()
                .retain(|_, inst| now.duration_since(inst.last_seen) < Duration::from_secs(3))
        });
        return;
    }
    render(d, scene3d::camera(game), &requests, tex);
}

/// Evaluate `requests` and draw them with `camera`.
pub fn render(
    d: &mut RaylibDrawHandle<'_>,
    camera: Camera3D,
    requests: &[Request],
    res: &impl ParticleResources,
) {
    let now = Instant::now();
    let base = basis(&camera);
    let mut drawn: Vec<Drawn> = Vec::new();
    INSTANCES.with(|instances| {
        let mut instances = instances.borrow_mut();
        for req in requests {
            let Some(group) = res.group(&req.resource) else {
                continue;
            };
            let inst = instances.entry(req.key.clone()).or_insert_with(|| {
                use std::hash::{Hash, Hasher};
                let mut h = std::collections::hash_map::DefaultHasher::new();
                req.key.hash(&mut h);
                let seed = h.finish();
                Instance {
                    started: now,
                    last_seen: now,
                    anims: group
                        .anims
                        .iter()
                        .enumerate()
                        .map(|(i, a)| a.instantiate(seed.wrapping_add(i as u64)))
                        .collect(),
                }
            });
            inst.last_seen = now;
            let time = now.duration_since(inst.started).as_secs_f32();
            let qb = euler_quat(&[req.rot.0, req.rot.1, req.rot.2]);
            for (anim, particles) in group.anims.iter().zip(&inst.anims) {
                let qi = euler_quat(&anim.rotation);
                for particle in particles {
                    if let Some(q) = anim.eval(particle, time) {
                        collect(anim, &q, req, &qb, &qi, &base, camera.position, &mut drawn);
                    }
                }
            }
        }
        instances.retain(|_, inst| now.duration_since(inst.last_seen) < Duration::from_secs(3));
    });
    if drawn.is_empty() {
        return;
    }
    // Alpha-blended effects back to front first, then the additive glows.
    drawn.sort_by(|a, b| {
        let order = |x: Blend| if x == Blend::Normal { 0 } else { 1 };
        order(a.blend).cmp(&order(b.blend)).then(
            b.distance
                .partial_cmp(&a.distance)
                .unwrap_or(std::cmp::Ordering::Equal),
        )
    });
    let mut mode = d.begin_mode3D(camera);
    unsafe {
        raylib::ffi::rlDisableBackfaceCulling();
        raylib::ffi::rlDisableDepthMask();
    }
    let mut current: Option<Blend> = None;
    for item in &drawn {
        if current != Some(item.blend) {
            if current.is_some() {
                unsafe { raylib::ffi::EndBlendMode() };
            }
            set_blend(item.blend);
            current = Some(item.blend);
        }
        match &item.item {
            Item::Quad {
                texture,
                corners,
                uv,
                color,
            } => draw_quad(res, texture, corners, uv, *color),
            Item::Mesh {
                geometry,
                texture,
                pos,
                axis,
                angle,
                scale,
                color,
            } => {
                if let Some(model) = res.model(geometry, texture) {
                    mode.draw_model_ex(model, *pos, *axis, *angle, *scale, *color);
                }
            }
        }
    }
    unsafe {
        raylib::ffi::rlDrawRenderBatchActive();
        if current.is_some() {
            raylib::ffi::EndBlendMode();
        }
        raylib::ffi::rlEnableDepthMask();
        raylib::ffi::rlEnableBackfaceCulling();
    }
}
