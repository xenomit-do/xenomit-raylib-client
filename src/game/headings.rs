//! Entity facing: movement tweens, laser-link aim, hero lock fallback.

use crate::assets::particles::{euler_quat, quat_mul, quat_rotate};
use crate::assets::{AssetManager, Display3D};
use crate::net::{MoveTween, SharedWorld};
use std::collections::HashMap;
use xenomit_packet_client::model::EntityKind;

use super::input;

/// Per-frame heading update — `MoveTrait` facing while a tween runs,
/// then aim while attacking: every `laser_links` attacker faces its
/// victim (overrides the movement heading). `hero_link` supplies the
/// hero's `(id, target)` pair while its laser is active but the link
/// hasn't registered — same override for the local ship. `pos` resolves
/// entity positions (`SharedWorld::pos`, hero-aware).
pub(crate) fn update_headings(
    headings: &mut HashMap<i32, f32>,
    tweens: &HashMap<i32, MoveTween>,
    links: &HashMap<i32, i32>,
    hero_link: Option<(i32, i32)>,
    pos: impl Fn(i32) -> (f32, f32),
) {
    for (id, tw) in tweens {
        // A finished tween doesn't snap a parked ship back to its last
        // move direction — only seeds the heading once if unknown.
        if !tw.done() || !headings.contains_key(id) {
            headings.insert(*id, input::heading_deg(tw.from, tw.to));
        }
    }
    for (attacker, victim) in links.iter().map(|(&a, &v)| (a, v)).chain(hero_link) {
        let (a, v) = (pos(attacker), pos(victim));
        let dx = v.0 - a.0;
        let dy = v.1 - a.1;
        if dx * dx + dy * dy > 1.0 {
            headings.insert(attacker, input::heading_deg(a, v));
        }
    }
}

/// Ease drone headings toward the latest movement/aim target. Ship
/// headings remain immediate; this smoothed value only drives drone
/// rotation strips and ship-local formation offsets.
pub(crate) fn ease_headings(current: &mut HashMap<i32, f32>, targets: &HashMap<i32, f32>, dt: f32) {
    let dt = dt.clamp(0.0, 0.1);
    let blend = 1.0 - (-dt * 5.0).exp();
    let max_step = 600.0 * dt;
    for (&id, &target) in targets {
        let Some(angle) = current.get_mut(&id) else {
            current.insert(id, target.rem_euclid(360.0));
            continue;
        };
        let delta = (target - *angle + 180.0).rem_euclid(360.0) - 180.0;
        if delta.abs() < 0.25 {
            *angle = target.rem_euclid(360.0);
        } else {
            let step = (delta * blend).clamp(-max_step, max_step);
            *angle = (*angle + step).rem_euclid(360.0);
        }
    }
}

/// Laser class of an attacker — colors the beam. Resolved through the ship
/// pattern (`game.xml`) of the attacker's `type_id`; 0 = stock red fallback.
pub(crate) fn laser_class_of(attacker: i32, world: &SharedWorld, assets: &AssetManager) -> i32 {
    let type_id = if Some(attacker) == world.world.hero.as_ref().map(|h| h.user_id) {
        world.world.hero.as_ref().map(|h| h.type_id.as_str())
    } else {
        world
            .world
            .entities
            .get(&attacker)
            .map(|e| e.type_id.as_str())
    };
    type_id
        .and_then(|t| assets.game_xml.ship_by_type(t))
        .map(|p| p.laser_id)
        .unwrap_or(0)
}

/// Rocket class of an attacker — the `rocketClassID` of its ship pattern
/// (`rocketClasses[class][rocketType]`); 0 = stock default row.
pub(crate) fn rocket_class_of(attacker: i32, world: &SharedWorld, assets: &AssetManager) -> i32 {
    let type_id = if Some(attacker) == world.world.hero.as_ref().map(|h| h.user_id) {
        world.world.hero.as_ref().map(|h| h.type_id.as_str())
    } else {
        world
            .world
            .entities
            .get(&attacker)
            .map(|e| e.type_id.as_str())
    };
    type_id
        .and_then(|t| assets.game_xml.ship_by_type(t))
        .map(|p| p.rocket_id)
        .unwrap_or(0)
}

/// `Lasers2D` mount groups for an attacker — the ship pattern's
/// `expansionClassID` resolves salvo groups of ship-local mount offsets
/// at the given heading, keeping the authored mount names: each
/// `(name, offset)` lets the 3D path resolve the ship's
/// `laserpoint_{name}` AWD slot (`Lasers3D.getLaserCannonSlot`) while the
/// 2D/billboard path keeps using `offset`.
pub(crate) fn attacker_salvo_mounts(
    attacker: i32,
    heading: f32,
    world: &SharedWorld,
    assets: &AssetManager,
) -> Vec<Vec<(String, (f32, f32))>> {
    let type_id = if Some(attacker) == world.world.hero.as_ref().map(|h| h.user_id) {
        world.world.hero.as_ref().map(|h| h.type_id.as_str())
    } else {
        world
            .world
            .entities
            .get(&attacker)
            .map(|e| e.type_id.as_str())
    };
    type_id
        .and_then(|t| assets.game_xml.ship_by_type(t))
        .filter(|p| !p.expansion_id.is_empty())
        .map(|p| {
            assets
                .game_xml
                .expansion_salvo_mounts(&p.expansion_id, heading)
        })
        .unwrap_or_default()
}

/// Authored `laserpoint_*` slots of one attacker plus the transform chain
/// that places them in the world (`Lasers3D`/`Slot3D.fromMesh`): the AWD
/// gives slot positions in model-local space; the recipe's first mesh
/// node (`Lasers3D.mesh3D` = `uberMeshes[0]`) and the eased ship rotation
/// map them into the scene.
pub(crate) struct ShipSlots3D {
    /// `laserpoint_`-stripped mount name -> model-local position.
    pub slots: HashMap<String, [f32; 3]>,
    /// `meshes[0]` node translation (Away3D recipe space).
    pub mesh_pos: (f32, f32, f32),
    /// `meshes[0]` node Euler rotation (degrees, Z*Y*X).
    pub mesh_rot: (f32, f32, f32),
    /// Uniform node scale (`recipe.scale * mesh.scale`, `1` when the
    /// recipe scale is 0 — `draw_owned_mesh` convention).
    pub scale: f32,
    /// Eased ship parent quaternion (`Ship3D.updateRotation`).
    pub parent: [f32; 4],
}

impl ShipSlots3D {
    /// `Lasers3D.getLaserCannonLocalPosition`: the authored slot pushed
    /// through node scale/rotation/translation and the ship parent
    /// rotation — a ship-local Away3D offset (add the attacker's
    /// `(x, 0, -y)` position for the world point). `None` when no slot
    /// with that name exists; stock fires the bolt from the ship center
    /// then (`param2.setTo(0,0,0)`).
    pub fn offset(&self, name: &str) -> Option<[f32; 3]> {
        let slot = self
            .slots
            .get(name)
            .or_else(|| self.slots.get(name.strip_prefix("laserpoint_")?))?;
        let node = euler_quat(&[self.mesh_rot.0, self.mesh_rot.1, self.mesh_rot.2]);
        let scaled = [
            slot[0] * self.scale,
            slot[1] * self.scale,
            slot[2] * self.scale,
        ];
        let local = quat_rotate(&node, &scaled);
        let in_ship = [
            local[0] + self.mesh_pos.0 as f32,
            local[1] + self.mesh_pos.1 as f32,
            local[2] + self.mesh_pos.2 as f32,
        ];
        Some(quat_rotate(&self.parent, &in_ship))
    }

    /// Composed ship+mesh Euler rotation — the world rotation a group
    /// parented to `mesh3D` inherits (muzzle flashes).
    pub fn mesh_world_rot(&self) -> (f32, f32, f32) {
        crate::render::scene3d::quat_to_euler(&quat_mul(
            &self.parent,
            &euler_quat(&[self.mesh_rot.0, self.mesh_rot.1, self.mesh_rot.2]),
        ))
    }
}

/// The attacker's authored `laserpoint_*` table — `None` unless the ship
/// has an expansion class (`ShipPattern.hasExpansion`, stock's
/// `Lasers3D.updateProvider` gate) and its `display3D` AWD parsed slots.
/// The table is recomputed per volley; the AWD bytes are already cached
/// by the mesh pipeline.
pub(crate) fn attacker_slots3d(
    attacker: i32,
    display_3d: Option<&Display3D>,
    heading: f32,
    world: &SharedWorld,
    assets: &AssetManager,
) -> Option<ShipSlots3D> {
    let hero = world.world.hero.as_ref().filter(|h| h.user_id == attacker);
    let entity = world.world.entities.get(&attacker);
    let type_id = hero
        .map(|h| h.type_id.as_str())
        .or_else(|| entity.map(|e| e.type_id.as_str()))?;
    // `Lasers3D.updateProvider` requires `shipPattern.hasExpansion()`.
    let expansion = assets.game_xml.ship_by_type(type_id)?;
    if expansion.expansion_id.is_empty() {
        return None;
    }
    let mesh = display_3d?.meshes.first()?;
    let bytes = assets.geometry(&mesh.geometry)?;
    let slots: HashMap<String, [f32; 3]> =
        crate::assets::awd::parse_aw2_slots(&bytes, "laserpoint_")
            .ok()?
            .into_iter()
            .collect();
    let recipe = display_3d?;
    let is_ship = hero.is_some()
        || entity.is_some_and(|e| matches!(e.kind, EntityKind::Ship | EntityKind::Other));
    let parent = if is_ship {
        crate::render::scene3d::peek_ship_parent_rotation(attacker, heading - 90.0)
    } else {
        [1.0, 0.0, 0.0, 0.0]
    };
    Some(ShipSlots3D {
        slots,
        mesh_pos: mesh.position,
        mesh_rot: mesh.rotation,
        scale: recipe.scale * if mesh.scale == 0.0 { 1.0 } else { mesh.scale },
        parent,
    })
}

#[cfg(test)]
mod tests {
    use super::{update_headings, ShipSlots3D};
    use crate::assets::particles::quat_axis;
    use crate::game::input::heading_deg;
    use crate::net::MoveTween;
    use std::collections::HashMap;
    use std::time::Instant;

    #[test]
    fn headings_link_overrides_active_tween() {
        // Attacker 1 flies east (tween running) but attacks victim 2 to
        // the south — the laser link wins.
        let mut headings = HashMap::new();
        let mut tweens = HashMap::new();
        tweens.insert(
            1,
            MoveTween {
                from: (0.0, 0.0),
                to: (100.0, 0.0),
                duration_ms: 60_000,
                started: Instant::now(),
            },
        );
        let links = HashMap::from([(1, 2)]);
        let pos = |id: i32| if id == 2 { (0.0, 50.0) } else { (0.0, 0.0) };
        update_headings(&mut headings, &tweens, &links, None, pos);
        assert_eq!(headings[&1], heading_deg((0.0, 0.0), (0.0, 50.0)));
    }

    #[test]
    fn headings_finished_tween_keeps_facing() {
        // A done tween doesn't snap a parked ship back; only seeds a
        // heading when the id has none.
        let mut headings = HashMap::from([(1, 42.0)]);
        let mut tweens = HashMap::new();
        for id in [1, 2] {
            tweens.insert(
                id,
                MoveTween {
                    from: (0.0, 0.0),
                    to: (10.0, 0.0),
                    duration_ms: 0, // instantly done
                    started: Instant::now(),
                },
            );
        }
        update_headings(&mut headings, &tweens, &HashMap::new(), None, |_| {
            (0.0, 0.0)
        });
        assert_eq!(headings[&1], 42.0, "done tween must not overwrite");
        assert_eq!(
            headings[&2],
            heading_deg((0.0, 0.0), (10.0, 0.0)),
            "unknown heading seeded once"
        );
    }

    #[test]
    fn headings_attacker_faces_victim() {
        // Victim east then south of the attacker — headings match
        // `heading_deg` (Flash convention: 0 = east + 180 offset).
        let mut headings = HashMap::new();
        let mut links = HashMap::from([(1, 2)]);
        let pos = |id: i32| if id == 2 { (100.0, 0.0) } else { (0.0, 0.0) };
        update_headings(&mut headings, &HashMap::new(), &links, None, pos);
        assert_eq!(headings[&1], heading_deg((0.0, 0.0), (100.0, 0.0)));

        headings.clear();
        links.insert(1, 3);
        let pos = |id: i32| if id == 3 { (0.0, 100.0) } else { (0.0, 0.0) };
        update_headings(&mut headings, &HashMap::new(), &links, None, pos);
        assert_eq!(headings[&1], heading_deg((0.0, 0.0), (0.0, 100.0)));

        // Coincident positions (< 1 unit) don't aim.
        headings.clear();
        links.insert(1, 4);
        let pos = |_: i32| (7.0, 7.0);
        update_headings(&mut headings, &HashMap::new(), &links, None, pos);
        assert!(!headings.contains_key(&1));
    }

    #[test]
    fn headings_hero_fallback_faces_selection() {
        // Hero laser active, no link yet: `hero_link` aims the hero at
        // the locked target.
        let mut headings = HashMap::new();
        let pos = |id: i32| if id == 9 { (200.0, 0.0) } else { (0.0, 0.0) };
        update_headings(
            &mut headings,
            &HashMap::new(),
            &HashMap::new(),
            Some((1, 9)),
            pos,
        );
        assert_eq!(headings[&1], heading_deg((0.0, 0.0), (200.0, 0.0)));
    }

    fn slots(map: &[(&str, [f32; 3])]) -> ShipSlots3D {
        ShipSlots3D {
            slots: map.iter().map(|(n, p)| (n.to_string(), *p)).collect(),
            mesh_pos: (0.0, 0.0, 0.0),
            mesh_rot: (0.0, 0.0, 0.0),
            scale: 1.0,
            parent: [1.0, 0.0, 0.0, 0.0],
        }
    }

    fn approx3(a: [f32; 3], b: [f32; 3]) {
        for k in 0..3 {
            assert!((a[k] - b[k]).abs() < 1e-4, "{a:?} != {b:?}");
        }
    }

    #[test]
    fn laser_slot_offset_identity() {
        let s = slots(&[("centerFront", [40.0, 6.0, 0.0])]);
        approx3(s.offset("centerFront").unwrap(), [40.0, 6.0, 0.0]);
        // Missing mounts resolve to `None` — stock fires from center.
        assert!(s.offset("leftRearInner").is_none());
        // Salvo names carrying the prefix resolve too.
        approx3(
            s.offset("laserpoint_centerFront").unwrap(),
            [40.0, 6.0, 0.0],
        );
    }

    #[test]
    fn laser_slot_offset_scales_and_rotates() {
        let mut s = slots(&[("left", [10.0, 5.0, 0.0])]);
        // Node scale applies to the slot position.
        s.scale = 2.0;
        approx3(s.offset("left").unwrap(), [20.0, 10.0, 0.0]);
        // ...and the mesh-node euler composes before the ship parent:
        // Ry(+90) maps +x -> -z (Away3D Z*Y*X euler convention).
        s.scale = 1.0;
        s.mesh_rot = (0.0, 90.0, 0.0);
        approx3(s.offset("left").unwrap(), [0.0, 5.0, -10.0]);
        // Ship yaw composes on top of the node transform — two +90 yaws
        // flip the point around (180 total).
        s.parent = quat_axis(&[0.0, 1.0, 0.0], 90.0f32.to_radians());
        approx3(s.offset("left").unwrap(), [-10.0, 5.0, 0.0]);
    }

    #[test]
    fn laser_slot_offset_translates_node() {
        let mut s = slots(&[("front", [1.0, 0.0, 0.0])]);
        s.mesh_pos = (100.0, 20.0, -50.0);
        approx3(s.offset("front").unwrap(), [101.0, 20.0, -50.0]);
    }
}
