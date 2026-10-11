//! Authored Away3D static scene pass. The mesh/texture data comes from the
//! separate World3D collection; 2D HUD and labels remain in the normal pass.
use crate::assets::formations::DroneFormationPattern;
use crate::assets::particles::{euler_quat, quat_axis, quat_mul, quat_rotate};
use crate::assets::uberlib::{Display3D, Particles3D, TextureQuality};
use crate::render::particles3d;
use crate::{game::Game, net::SharedWorld, render::sprites::TextureCache};
use raylib::prelude::*;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::time::Instant;
use xenomit_packet_client::model::EntityKind;

extern "C" {
    fn rlSetClipPlanes(near_plane: f64, far_plane: f64);
}
static SET_CLIP_PLANES: std::sync::Once = std::sync::Once::new();

/// Away3D scene point -> raylib world point.
///
/// Away3D is left-handed (Y up, +Z into the screen); raylib is right-handed.
/// Mirroring Z keeps the picture identical: map `(x, y)` is `(x, 0, -y)` in
/// Away3D (`MapEntity.x3D/y3D/z3D`) and therefore `(x, 0, y)` in raylib.
/// Authored meshes are flipped the same way in `AwdMesh::to_obj`.
pub fn rl_point(x: f32, y: f32, z: f32) -> Vector3 {
    Vector3::new(x, y, -z)
}

/// Raylib position of a model-local Away3D offset `local` at map position
/// `map` (map y is Away3D `-z`).
fn rl_map_point(map: (f32, f32), local: (f32, f32, f32)) -> Vector3 {
    rl_point(map.0 + local.0, local.1, -map.1 + local.2)
}

/// Away3D position of map point `map` plus the model-local `y` lift.
fn away_pos(map: (f32, f32)) -> (f32, f32, f32) {
    (map.0, 0.0, -map.1)
}

// --- Ship3D rotation easing --------------------------------------------------

/// `Ship3D.updateRotation` state: the displayed yaw/bank chase the target.
struct ShipRot {
    yaw: f32,
    tilt: f32,
    last: Instant,
}

thread_local! {
    static SHIP_ROT: RefCell<HashMap<i32, ShipRot>> = RefCell::new(HashMap::new());
}

fn short_angle(from: f32, to: f32) -> f32 {
    let mut d = (to - from) % 360.0;
    if d > 180.0 {
        d -= 360.0;
    } else if d < -180.0 {
        d += 360.0;
    }
    d
}

/// Eased `(yaw, bank)` for ship `id` chasing `target_yaw` (degrees).
///
/// The stock client calls `Quad.easeOut(dt, current, delta, 0.2)` every
/// frame (rotation) and banks the hull toward the remaining turn, clamped to
/// 20 degrees. Only the 3D model uses it; the 2D sprite keeps snapping.
fn eased_ship_rotation(id: i32, target_yaw: f32) -> (f32, f32) {
    SHIP_ROT.with(|cell| {
        let mut map = cell.borrow_mut();
        let now = Instant::now();
        map.retain(|_, r| now.duration_since(r.last).as_secs_f32() < 5.0);
        let rot = map.entry(id).or_insert(ShipRot {
            yaw: target_yaw,
            tilt: 0.0,
            last: now,
        });
        let dt = now.duration_since(rot.last).as_secs_f32().clamp(0.0, 0.1);
        rot.last = now;
        let delta = short_angle(rot.yaw, target_yaw);
        let f = (dt / 0.2).min(1.0);
        let ease = f * (2.0 - f);
        rot.yaw += delta * ease;
        let tilt_delta = short_angle(rot.tilt, delta);
        rot.tilt = (rot.tilt + tilt_delta * ease).clamp(-20.0, 20.0);
        (rot.yaw, rot.tilt)
    })
}

/// Parent rotation of a ship: yaw about Y, after the bank about Z
/// (`_m.appendRotation(-rotZ, Z); _m.appendRotation(rotY, Y)`).
fn ship_parent_rotation(yaw: f32, tilt: f32) -> [f32; 4] {
    quat_mul(
        &quat_axis(&[0.0, 1.0, 0.0], yaw.to_radians()),
        &quat_axis(&[0.0, 0.0, 1.0], (-tilt).to_radians()),
    )
}

/// Current eased `Ship3D` parent rotation of ship `id` WITHOUT advancing
/// the ease — for spawn-time world anchors (laser `laserpoint_*` slot
/// transforms resolve against the ship's rendered rotation).
/// `fallback_yaw` matches the draw path's seed (`heading - 90`).
pub fn peek_ship_parent_rotation(id: i32, fallback_yaw: f32) -> [f32; 4] {
    let (yaw, tilt) = SHIP_ROT.with(|cell| {
        cell.borrow()
            .get(&id)
            .map(|rot| (rot.yaw, rot.tilt))
            .unwrap_or((fallback_yaw, 0.0))
    });
    ship_parent_rotation(yaw, tilt)
}

// --- Drone3D / DronesFormation3D -------------------------------------------

/// `DronesFormation3D` state per owner: the eased `_formationRotation`
/// plus each drone's `position3D` — a ship-local Away3D point chasing the
/// (possibly spinning) `rotationGroup` target, `Quad.easeOut` over 0.3 s.
struct OwnerDrones3D {
    yaw: f32,
    pos: Vec<[f32; 3]>,
    last: Instant,
}

thread_local! {
    static DRONES3D: RefCell<HashMap<i32, OwnerDrones3D>> = RefCell::new(HashMap::new());
    /// `TweenPlayer` clock for the rotation-group tweens.
    static DRONE3D_EPOCH: Instant = Instant::now();
}

// --- owners (ships, assets, hero) -----------------------------------------------

struct Owner<'a> {
    id: i32,
    pos: (f32, f32),
    recipe: &'a Display3D,
    /// `Some(heading)` for ships (`Ship3D`), `None` for static assets.
    heading: Option<f32>,
}

fn meshes_ready(tex: &TextureCache, recipe: &Display3D, quality: TextureQuality) -> bool {
    !recipe.meshes.is_empty()
        && recipe
            .meshes
            .iter()
            .all(|m| tex.model3d(&m.geometry, &m.material, quality).is_some())
}

/// A recipe is drawable once its meshes are uploaded, or — for particle-only
/// recipes — once its particle groups are.
fn recipe_ready(tex: &TextureCache, recipe: &Display3D, quality: TextureQuality) -> bool {
    if recipe.meshes.is_empty() {
        particles3d::recipe_ready(tex, recipe)
    } else {
        meshes_ready(tex, recipe, quality)
    }
}

/// Draw loaded ship, hero and asset (station, relay, turret…) meshes using
/// DarkOrbit's map-plane convention `(map_x, map_y) -> (x, 0, -y)` and
/// CameraManager3D's Y-up orbit camera. Returns entity ids whose authored
/// model was actually drawn; callers keep all 2D annotations and use the
/// sprite when the model is still unavailable.
pub(crate) fn draw_ship_models(
    d: &mut RaylibDrawHandle<'_>,
    game: &Game,
    world: &SharedWorld,
    tex: &TextureCache,
) -> HashSet<i32> {
    let rp = game.preferences.render;
    let mut drawn = HashSet::new();
    if !rp.camera_3d && !rp.npcs_3d && !rp.ships_3d {
        return drawn;
    }

    let mut owners: Vec<Owner> = Vec::new();
    for e in world.world.entities.values() {
        let is_ship = matches!(e.kind, EntityKind::Ship | EntityKind::Other);
        if !is_ship && e.kind != EntityKind::Asset {
            continue;
        }
        if (e.npc && !rp.npcs_3d) || (!e.npc && !rp.ships_3d) {
            continue;
        }
        if e.destroyed || world.pets.is_hidden(e.id) {
            continue;
        }
        let Some(recipe) = game
            .entity_visuals
            .get(&e.id)
            .and_then(|v| v.display_3d.as_ref())
        else {
            continue;
        };
        owners.push(Owner {
            id: e.id,
            pos: game.draw_pos(world, e.id),
            recipe,
            heading: is_ship.then(|| game.headings.get(&e.id).copied().unwrap_or(0.0)),
        });
    }
    // The hero is not part of `entities`; it has its own row and visual.
    if let Some(h) = world.world.hero.as_ref().filter(|_| rp.ships_3d) {
        let alive = !(h.hitpoints <= 0.0 && h.hitpoints_max > 0.0);
        if let Some(recipe) = game
            .entity_visuals
            .get(&h.user_id)
            .and_then(|v| v.display_3d.as_ref())
            .filter(|_| alive)
        {
            owners.push(Owner {
                id: h.user_id,
                pos: game.draw_pos(world, h.user_id),
                recipe,
                heading: Some(game.headings.get(&h.user_id).copied().unwrap_or(0.0)),
            });
        }
    }

    let quality = rp.texture_quality;
    let camera = camera(game);
    let mut mode = d.begin_mode3D(camera);
    for owner in &owners {
        if !recipe_ready(tex, owner.recipe, quality) {
            continue;
        }
        // Ship3D.updateRotation: rotationY = rotation.degrees - 90, eased.
        let parent = match owner.heading {
            Some(heading) => {
                let (yaw, tilt) = eased_ship_rotation(owner.id, heading - 90.0);
                ship_parent_rotation(yaw, tilt)
            }
            None => [1.0, 0.0, 0.0, 0.0],
        };
        for mesh in &owner.recipe.meshes {
            let Some(model) = tex.model3d(&mesh.geometry, &mesh.material, quality) else {
                continue;
            };
            let scale = owner.recipe.scale * if mesh.scale == 0.0 { 1.0 } else { mesh.scale };
            draw_owned_mesh(
                &mut mode,
                model,
                owner.pos,
                &parent,
                mesh.position,
                mesh.rotation,
                scale,
            );
        }
        if !owner.recipe.particles.is_empty() {
            queue_owner_particles(
                &format!("e{}", owner.id),
                owner.recipe,
                owner.pos,
                &parent,
                1.0,
                0.0,
            );
        }
        drawn.insert(owner.id);
    }
    drawn
}

/// Queue a recipe's particle nodes for an owner at map position `pos` whose
/// own rotation is the quaternion `parent`.
fn queue_owner_particles(
    key: &str,
    recipe: &Display3D,
    pos: (f32, f32),
    parent: &[f32; 4],
    extra_scale: f32,
    lift: f32,
) {
    queue_particle_list(key, &recipe.particles, pos, parent, extra_scale, lift);
}

/// [`queue_owner_particles`] for an explicit particle list (portal jump
/// groups live beside the idle ones).
fn queue_particle_list(
    key: &str,
    particles: &[Particles3D],
    pos: (f32, f32),
    parent: &[f32; 4],
    extra_scale: f32,
    lift: f32,
) {
    queue_particles_away(key, particles, away_pos(pos), parent, extra_scale, lift);
}

/// `queue_owner_particles` for an owner whose origin is already an
/// Away3D world point (e.g. a drone on a tilted formation ring).
fn queue_owner_particles_away(
    key: &str,
    recipe: &Display3D,
    origin: (f32, f32, f32),
    parent: &[f32; 4],
    extra_scale: f32,
    lift: f32,
) {
    queue_particles_away(key, &recipe.particles, origin, parent, extra_scale, lift);
}

fn queue_particles_away(
    key: &str,
    particles: &[Particles3D],
    origin: (f32, f32, f32),
    parent: &[f32; 4],
    extra_scale: f32,
    lift: f32,
) {
    for (index, p) in particles.iter().enumerate() {
        let local = quat_rotate(parent, &[p.position.0, p.position.1, p.position.2]);
        let rot = quat_mul(
            parent,
            &euler_quat(&[p.rotation.0, p.rotation.1, p.rotation.2]),
        );
        particles3d::queue(particles3d::Request {
            key: format!("{key}:{index}"),
            resource: p.resource.clone(),
            pos: (
                origin.0 + local[0],
                origin.1 + local[1] + lift,
                origin.2 + local[2],
            ),
            rot: quat_to_euler(&rot),
            scale: (
                p.scale.0 * extra_scale,
                p.scale.1 * extra_scale,
                p.scale.2 * extra_scale,
            ),
        });
    }
}

/// Z*Y*X Euler degrees of a quaternion (inverse of `euler_quat`).
pub(crate) fn quat_to_euler(q: &[f32; 4]) -> (f32, f32, f32) {
    let (w, x, y, z) = (q[0], q[1], q[2], q[3]);
    let roll = (2.0 * (w * x + y * z)).atan2(1.0 - 2.0 * (x * x + y * y));
    let sin_pitch = (2.0 * (w * y - z * x)).clamp(-1.0, 1.0);
    let pitch = if sin_pitch.abs() >= 0.999_999 {
        std::f32::consts::FRAC_PI_2.copysign(sin_pitch)
    } else {
        sin_pitch.asin()
    };
    let yaw = (2.0 * (w * z + x * y)).atan2(1.0 - 2.0 * (y * y + z * z));
    (roll.to_degrees(), pitch.to_degrees(), yaw.to_degrees())
}

/// Match MapView3D's perspective and Observer3D's orbit position, expressed
/// in raylib's right-handed world (see [`rl_point`]).
///
/// `Observer3D.validate`: position = look + d * (sin(tilt) sin(pan),
/// -cos(tilt), -sin(tilt) cos(pan)) with the default pan 0 / tilt 135°, so
/// the camera sits above and on the map's south side (larger map y) looking
/// north. `CameraManager3D.update` then sets near=10, far=80000, fov=30.
pub(crate) fn camera(game: &Game) -> Camera3D {
    orbit_camera(
        game.camera.center,
        game.camera.zoom,
        game.camera.viewport.1,
        game.preferences.render.camera_3d,
    )
}

/// [`camera`] for an explicit map-space look-at point; also used by the
/// `view3d` example.
pub fn orbit_camera(center: (f32, f32), zoom: f32, viewport_h: f32, perspective: bool) -> Camera3D {
    SET_CLIP_PLANES.call_once(|| unsafe { rlSetClipPlanes(10.0, 80_000.0) });
    // look = (mapX, 0, -mapY) in Away3D.
    let away_look_z = -center.1;
    let look = rl_point(center.0, 0.0, away_look_z);
    if !perspective {
        // Independent component mode: an overhead orthographic camera keeps
        // model anchors aligned with the established 2D map transform.
        let distance = 40_000.0;
        return Camera3D::orthographic(
            Vector3::new(look.x, distance, look.z),
            look,
            // Screen-up is map-north (-y), i.e. raylib -z.
            Vector3::new(0.0, 0.0, -1.0),
            (viewport_h / zoom.max(0.05)).max(1.0),
        );
    }
    let tilt = 135.0_f32.to_radians();
    let distance = 1740.0 / zoom.max(0.05);
    let position = rl_point(
        look.x,
        -distance * tilt.cos(),
        away_look_z - distance * tilt.sin(),
    );
    Camera3D::perspective(position, look, Vector3::new(0.0, 1.0, 0.0), 30.0)
}

/// Draw one authored mesh whose owner (at map position `pos`) is rotated by
/// the quaternion `parent`; the recipe's local transform is its child.
pub fn draw_owned_mesh(
    mode: &mut impl RaylibDraw3D,
    model: &Model,
    pos: (f32, f32),
    parent: &[f32; 4],
    mesh_position: (f32, f32, f32),
    mesh_rotation: (f32, f32, f32),
    scale: f32,
) {
    let local = quat_rotate(parent, &[mesh_position.0, mesh_position.1, mesh_position.2]);
    let rot = quat_mul(
        parent,
        &euler_quat(&[mesh_rotation.0, mesh_rotation.1, mesh_rotation.2]),
    );
    let (axis, angle) = mirrored_axis_angle(&rot);
    mode.draw_model_ex(
        model,
        rl_map_point(pos, (local[0], local[1], local[2])),
        axis,
        angle,
        Vector3::new(scale, scale, scale),
        Color::WHITE,
    );
}

/// `draw_owned_mesh` for an owner already placed at an Away3D world point
/// (e.g. a drone on a tilted formation ring).
pub fn draw_owned_mesh_at(
    mode: &mut impl RaylibDraw3D,
    model: &Model,
    pos: (f32, f32, f32),
    parent: &[f32; 4],
    mesh_position: (f32, f32, f32),
    mesh_rotation: (f32, f32, f32),
    scale: f32,
) {
    let local = quat_rotate(parent, &[mesh_position.0, mesh_position.1, mesh_position.2]);
    let rot = quat_mul(
        parent,
        &euler_quat(&[mesh_rotation.0, mesh_rotation.1, mesh_rotation.2]),
    );
    let (axis, angle) = mirrored_axis_angle(&rot);
    mode.draw_model_ex(
        model,
        rl_point(pos.0 + local[0], pos.1 + local[1], pos.2 + local[2]),
        axis,
        angle,
        Vector3::new(scale, scale, scale),
        Color::WHITE,
    );
}

/// Draw one authored ship mesh (`Ship3D.updateRotation`: the owner rotates by
/// `heading - 90` and the recipe's local transform is a child of it).
pub fn draw_ship_mesh(
    mode: &mut impl RaylibDraw3D,
    model: &Model,
    pos: (f32, f32),
    heading: f32,
    mesh_position: (f32, f32, f32),
    mesh_rotation: (f32, f32, f32),
    scale: f32,
) {
    let parent = ship_parent_rotation(heading - 90.0, 0.0);
    draw_owned_mesh(
        mode,
        model,
        pos,
        &parent,
        mesh_position,
        mesh_rotation,
        scale,
    );
}

/// Axis-angle of an Away3D-space quaternion `[w, x, y, z]` in raylib's
/// mirrored world: Rz is kept and Rx/Ry reverse (see [`rl_point`]).
pub fn mirrored_axis_angle(q: &[f32; 4]) -> (Vector3, f32) {
    let (w, x, y, z) = (q[0].clamp(-1.0, 1.0), -q[1], -q[2], q[3]);
    let s = (1.0 - w * w).max(0.0).sqrt();
    if s < 1e-5 {
        return (Vector3::new(0.0, 1.0, 0.0), 0.0);
    }
    (
        Vector3::new(x / s, y / s, z / s),
        2.0 * w.acos().to_degrees(),
    )
}

/// Convert an authored Away3D Z*Y*X Euler rotation to raylib's arbitrary-axis
/// model rotation, preserving parent-composed rotations.
pub fn euler_axis_angle(rot: (f32, f32, f32)) -> (Vector3, f32) {
    mirrored_axis_angle(&euler_quat(&[rot.0, rot.1, rot.2]))
}

fn ease_in_out(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    if t < 0.5 {
        2.0 * t * t
    } else {
        1.0 - 2.0 * (1.0 - t) * (1.0 - t)
    }
}

pub(crate) fn draw_collectable_models(
    d: &mut RaylibDrawHandle<'_>,
    game: &Game,
    world: &SharedWorld,
    tex: &TextureCache,
) -> HashSet<String> {
    let mut drawn = HashSet::new();
    if !game.preferences.render.collectables_3d {
        return drawn;
    }
    let quality = game.preferences.render.texture_quality;
    let mut mode = d.begin_mode3D(camera(game));
    for c in world.world.collectables.values() {
        let Some(visual) = game.collectable_visuals.get(&c.hash) else {
            continue;
        };
        let Some(recipe) = visual.display_3d.as_ref() else {
            continue;
        };
        if !recipe_ready(tex, recipe, quality) {
            continue;
        }
        let map = (c.x as f32, c.y as f32);
        for mesh in &recipe.meshes {
            let Some(model) = tex.model3d(&mesh.geometry, &mesh.material, quality) else {
                continue;
            };
            let scale = recipe.scale * mesh.scale;
            let position = rl_map_point(map, mesh.position);
            let (axis, angle) = euler_axis_angle(mesh.rotation);
            mode.draw_model_ex(
                model,
                position,
                axis,
                angle,
                Vector3::new(scale, scale, scale),
                Color::WHITE,
            );
        }
        if !recipe.particles.is_empty() {
            // `abstract_3d_box` animation `spawnNew`: the box node starts at
            // y=100, scale 0 and tweens to y=0, scale 1 in a second.
            let (scale, lift) = match game
                .collectable_spawned
                .get(&c.hash)
                .map(|at| at.elapsed().as_secs_f32())
            {
                Some(age) if age < 1.0 => {
                    let e = ease_in_out(age);
                    (e.max(0.001), 100.0 * (1.0 - e))
                }
                _ => (1.0, 0.0),
            };
            queue_owner_particles(
                &format!("c{}", c.hash),
                recipe,
                map,
                &[1.0, 0.0, 0.0, 0.0],
                scale,
                lift,
            );
        }
        drawn.insert(c.hash.clone());
    }
    drawn
}

pub(crate) fn draw_portal_models(
    d: &mut RaylibDrawHandle<'_>,
    game: &Game,
    world: &SharedWorld,
    tex: &TextureCache,
) -> HashSet<i32> {
    let mut drawn = HashSet::new();
    if !game.preferences.render.portals_3d {
        return drawn;
    }
    let quality = game.preferences.render.texture_quality;
    let mut mode = d.begin_mode3D(camera(game));
    for p in world.portals.values() {
        let Some(visual) = game.portal_visuals.get(&p.id) else {
            continue;
        };
        let Some(recipe) = visual.display_3d.as_ref() else {
            continue;
        };
        if !recipe_ready(tex, recipe, quality) {
            continue;
        }
        let map = (p.x as f32, p.y as f32);
        for mesh in &recipe.meshes {
            let Some(model) = tex.model3d(&mesh.geometry, &mesh.material, quality) else {
                continue;
            };
            let scale = recipe.scale * mesh.scale;
            let position = rl_map_point(map, mesh.position);
            let (axis, angle) = euler_axis_angle(mesh.rotation);
            mode.draw_model_ex(
                model,
                position,
                axis,
                angle,
                Vector3::new(scale, scale, scale),
                Color::WHITE,
            );
        }
        if !recipe.particles.is_empty() {
            queue_owner_particles(
                &format!("p{}", p.id),
                recipe,
                map,
                &[1.0, 0.0, 0.0, 0.0],
                1.0,
                0.0,
            );
        }
        // `PortalDecorator3D.startJumpAnimation` → `playAnimation("jump")`.
        if let Some(started) = world.portal_jumps.get(&p.id) {
            if crate::net::portals::jump_strength(*started, std::time::Instant::now()) > 0.0
                && !recipe.jump_particles.is_empty()
            {
                queue_particle_list(
                    &format!("pj{}", p.id),
                    &recipe.jump_particles,
                    map,
                    &[1.0, 0.0, 0.0, 0.0],
                    1.0,
                    0.0,
                );
            }
        }
        drawn.insert(p.id);
    }
    drawn
}

/// `DronesFormation3D.updateObj` + `Drone3D.updateObj`: each drone's
/// ship-local Away3D point is its `display3D` slot pushed through the
/// spinning `rotationGroup` container (when authored), scaled, yawed by
/// the eased `_formationRotation` and tilted by the group's base euler;
/// the drone model itself only yaws (`rotationY = _formationRotation -
/// 90`), so the tilt shows up in the orbit positions, not the hull.
pub(crate) fn draw_drone_models(
    d: &mut RaylibDrawHandle<'_>,
    game: &Game,
    world: &SharedWorld,
    formations: &HashMap<i32, DroneFormationPattern>,
    tex: &TextureCache,
) -> HashSet<(i32, i32)> {
    let mut drawn = HashSet::new();
    if !game.preferences.render.drones_3d {
        return drawn;
    }
    let quality = game.preferences.render.texture_quality;
    let mut mode = d.begin_mode3D(camera(game));
    let hero_id = world.world.hero.as_ref().map(|hero| hero.user_id);
    let t = DRONE3D_EPOCH.with(|e| e.elapsed().as_secs_f32());
    for (&owner_id, loadout) in &world.drones {
        let owner_is_hero = Some(owner_id) == hero_id;
        if !crate::render::drones::drones_visible(&world.settings, owner_is_hero) {
            continue;
        }
        let owner_pos = game.draw_pos(world, owner_id);
        let heading = game
            .drone_headings
            .get(&owner_id)
            .copied()
            .or_else(|| game.headings.get(&owner_id).copied())
            .unwrap_or(0.0);
        let count = loadout.drones.len();
        if count == 0 {
            continue;
        }
        let pattern = formations
            .get(&loadout.formation)
            .or_else(|| formations.get(&0));
        // `Quad.easeOut(dt, current, delta, 0.3)` — the same 0.3 s ease
        // drives the formation yaw and each drone's chase of its slot.
        let (formation_yaw, positions) = DRONES3D.with(|cell| {
            let mut map = cell.borrow_mut();
            let now = Instant::now();
            map.retain(|_, o| now.duration_since(o.last).as_secs_f32() < 5.0);
            let owner = map.entry(owner_id).or_insert(OwnerDrones3D {
                yaw: heading,
                pos: Vec::new(),
                last: now,
            });
            let dt = now.duration_since(owner.last).as_secs_f32().clamp(0.0, 0.1);
            owner.last = now;
            let f = (dt / 0.3).min(1.0);
            let ease = f * (2.0 - f);
            owner.yaw += short_angle(owner.yaw, heading) * ease;
            owner.pos.resize(count, [0.0; 3]);
            for (i, p) in owner.pos.iter_mut().enumerate() {
                let target = pattern.map_or_else(
                    || {
                        let (x, y) = crate::assets::formations::fallback_ring_position(count, i);
                        let (s, c) = owner.yaw.to_radians().sin_cos();
                        [x * c - y * s, 0.0, -(x * s + y * c)]
                    },
                    |pf| pf.target3d(count, i, loadout.user_scale, owner.yaw, t),
                );
                for k in 0..3 {
                    p[k] += (target[k] - p[k]) * ease;
                }
            }
            (owner.yaw, owner.pos.clone())
        });
        let parent = ship_parent_rotation(formation_yaw - 90.0, 0.0);
        for (i, entry) in loadout.drones.iter().enumerate() {
            let key = (
                entry.type_id,
                entry.level,
                entry.stats_design,
                entry.visual_design,
            );
            let Some(recipe) = game
                .drone_visuals
                .get(&key)
                .and_then(|v| v.display_3d.as_ref())
            else {
                continue;
            };
            if !recipe_ready(tex, recipe, quality) {
                continue;
            }
            let pos = positions[i];
            let away = (owner_pos.0 + pos[0], pos[1], -owner_pos.1 + pos[2]);
            for mesh in &recipe.meshes {
                let Some(model) = tex.model3d(&mesh.geometry, &mesh.material, quality) else {
                    continue;
                };
                let scale = recipe.scale * mesh.scale;
                draw_owned_mesh_at(
                    &mut mode,
                    model,
                    away,
                    &parent,
                    mesh.position,
                    mesh.rotation,
                    scale,
                );
            }
            if !recipe.particles.is_empty() {
                queue_owner_particles_away(
                    &format!("d{owner_id}:{}", entry.index),
                    recipe,
                    away,
                    &parent,
                    1.0,
                    0.0,
                );
            }
            drawn.insert((owner_id, entry.index));
        }
    }
    drawn
}
