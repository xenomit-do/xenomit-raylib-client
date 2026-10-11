//! Drone placement — the `DronesFormation2D` equivalent.
//!
//! Each drone sits at a ship-local offset taken from the loadout's
//! formation table (`DroneFormation.positionToRelativePoint(count,
//! index)`, scaled by `Drones.userScale`). The offsets are expressed for
//! a ship facing east and are rotated by the owner's heading when drawn
//! (Flash convention: 0 = east, clockwise positive, y down). When the
//! target offset changes — formation switch, drone added/removed — the
//! drone glides to it over [`TWEEN_SECS`] with a quadratic ease-out
//! (`TweenLite` default); a brand-new drone starts at the ship centre.

use std::collections::HashMap;
use std::f32::consts::PI;
use std::time::{Duration, Instant};

use crate::assets::formations::{fallback_ring_position, DroneFormationPattern};
use crate::assets::AssetManager;
use crate::net::drones::{DroneEntry, DroneLoadout};
use crate::net::SharedWorld;

use super::visuals::{self, Visual};
use super::Game;

/// Formation tween duration (`DronesFormation2D.TWEEN_TIME`).
pub const TWEEN_SECS: f32 = 0.5;

/// Cache key of a drone's sprite recipe: `(type, level, stats, visual)`.
pub type DroneKey = (i32, i32, i32, i32);

/// Ship-local offset of drone `index` of `count` for `formation`, with
/// `user_scale` applied. Unknown formation ids fall back to the STANDARD
/// table (id 0), and a table without a row for this count falls back to
/// an even ring.
pub fn formation_offset(
    formations: &HashMap<i32, DroneFormationPattern>,
    formation: i32,
    count: usize,
    index: usize,
    user_scale: f32,
) -> (f32, f32) {
    let pattern = formations.get(&formation).or_else(|| formations.get(&0));
    let (x, y) = pattern
        .and_then(|p| p.position(count, index))
        .unwrap_or_else(|| fallback_ring_position(count, index));
    (x * user_scale, y * user_scale)
}

/// Rotate a ship-local offset by the ship's heading (degrees, Flash
/// convention) into a world-space delta.
pub fn rotate(off: (f32, f32), heading_deg: f32) -> (f32, f32) {
    let a = heading_deg * PI / 180.0;
    let (s, c) = a.sin_cos();
    (off.0 * c - off.1 * s, off.0 * s + off.1 * c)
}

/// Quadratic ease-out (`Quad.easeOut`): fast start, soft landing.
pub fn ease_out(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t) * (1.0 - t)
}

#[derive(Clone, Debug)]
struct Slot {
    entry: DroneEntry,
    from: (f32, f32),
    to: (f32, f32),
    started: Instant,
}

impl Slot {
    fn at(&self, now: Instant) -> (f32, f32) {
        let t = now.saturating_duration_since(self.started).as_secs_f32() / TWEEN_SECS;
        let k = ease_out(t);
        (
            self.from.0 + (self.to.0 - self.from.0) * k,
            self.from.1 + (self.to.1 - self.from.1) * k,
        )
    }
}

/// Tweened drone positions of one ship.
#[derive(Clone, Debug, Default)]
pub struct DroneRig {
    slots: Vec<Slot>,
}

impl DroneRig {
    /// Fold the current loadout in: retarget moved drones (starting from
    /// where they are *now*), spawn new ones at the ship centre, drop
    /// the rest.
    pub fn sync(
        &mut self,
        loadout: &DroneLoadout,
        formations: &HashMap<i32, DroneFormationPattern>,
        now: Instant,
    ) {
        let count = loadout.drones.len();
        self.slots.truncate(count);
        for (i, entry) in loadout.drones.iter().enumerate() {
            let to = formation_offset(formations, loadout.formation, count, i, loadout.user_scale);
            match self.slots.get_mut(i) {
                Some(slot) => {
                    slot.entry = *entry;
                    if slot.to != to {
                        slot.from = slot.at(now);
                        slot.to = to;
                        slot.started = now;
                    }
                }
                None => self.slots.push(Slot {
                    entry: *entry,
                    from: (0.0, 0.0),
                    to,
                    started: now,
                }),
            }
        }
    }

    /// Every drone with its current ship-local offset.
    pub fn offsets(&self, now: Instant) -> Vec<(DroneEntry, (f32, f32))> {
        self.slots.iter().map(|s| (s.entry, s.at(now))).collect()
    }

    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    /// True once every tween has landed.
    pub fn settled(&self, now: Instant) -> bool {
        self.slots.iter().all(|s| {
            now.saturating_duration_since(s.started) >= Duration::from_secs_f32(TWEEN_SECS)
        })
    }
}

impl Game {
    /// Per-frame drone pass: keep one [`DroneRig`] per ship that has a
    /// loadout, and resolve each drone's sprite recipe
    /// (`lib_drones` key chain) once per `(type, level, stats, visual)`.
    pub(super) fn update_drones(&mut self, world: &SharedWorld, assets: &mut AssetManager) {
        let now = Instant::now();
        for (&uid, loadout) in &world.drones {
            if loadout.drones.is_empty() {
                self.drone_rigs.remove(&uid);
                continue;
            }
            self.drone_rigs.entry(uid).or_default().sync(
                loadout,
                &assets.game_xml.drone_formations,
                now,
            );
            for d in &loadout.drones {
                self.ensure_drone_visual(d, assets);
            }
        }
        self.drone_rigs
            .retain(|uid, _| world.drones.get(uid).is_some_and(|l| !l.drones.is_empty()));
    }

    fn ensure_drone_visual(&mut self, d: &DroneEntry, assets: &mut AssetManager) {
        let key: DroneKey = (d.type_id, d.level, d.stats_design, d.visual_design);
        if self.drone_visuals.contains_key(&key) {
            if self.preferences.render.drones_3d {
                let quality = self.preferences.render.texture_quality;
                let recipe = assets
                    .drone_display(d.type_id, d.level, d.stats_design, d.visual_design)
                    .and_then(|(_, id)| assets.uber_display3d(&id));
                if let Some(recipe) = recipe {
                    for mesh in &recipe.meshes {
                        visuals::request_mesh_resources(
                            assets,
                            &mesh.geometry,
                            &mesh.material,
                            quality,
                        );
                    }
                    if let Some(visual) = self.drone_visuals.get_mut(&key) {
                        visual.display_3d = Some(recipe);
                    }
                }
            }
            return;
        }
        let name = format!("drone:{}:{}:{}:{}", key.0, key.1, key.2, key.3);
        let visual = match assets.drone_display(d.type_id, d.level, d.stats_design, d.visual_design)
        {
            Some((disp, asset_id)) => {
                let sprite = (!disp.src_key.is_empty())
                    .then(|| assets.request(&disp.src_key, &disp.res_key));
                let layers = Visual::build_layers(&name, &disp, assets);
                let mut v = Visual::from_display(name, &disp, sprite);
                v.layers = layers;
                if self.preferences.render.drones_3d {
                    let quality = self.preferences.render.texture_quality;
                    v.display_3d = assets.uber_display3d(&asset_id);
                    if let Some(recipe) = &v.display_3d {
                        for mesh in &recipe.meshes {
                            visuals::request_mesh_resources(
                                assets,
                                &mesh.geometry,
                                &mesh.material,
                                quality,
                            );
                        }
                    }
                }
                v
            }
            None => Visual::placeholder(name),
        };
        self.drone_visuals.insert(key, visual);
    }

    /// World-space drone positions of ship `uid` standing at `owner_pos`
    /// facing `heading`: `(entry, position)`.
    pub fn drone_positions(
        &self,
        uid: i32,
        owner_pos: (f32, f32),
        heading: f32,
    ) -> Vec<(DroneEntry, (f32, f32))> {
        let Some(rig) = self.drone_rigs.get(&uid) else {
            return Vec::new();
        };
        let now = Instant::now();
        rig.offsets(now)
            .into_iter()
            .map(|(e, off)| {
                let r = rotate(off, heading);
                (e, (owner_pos.0 + r.0, owner_pos.1 + r.1))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn std_formation() -> HashMap<i32, DroneFormationPattern> {
        let mut m = HashMap::new();
        m.insert(
            0,
            DroneFormationPattern {
                id: 0,
                name: "standard".into(),
                scale: 1.0,
                locations: vec![(150.0, 0.0), (0.0, 100.0)],
                locations_by_count: vec![None, Some(vec![1]), Some(vec![1, 2])],
                display3d: None,
            },
        );
        m
    }

    fn entry(i: i32) -> DroneEntry {
        DroneEntry {
            index: i,
            type_id: 1,
            level: 1,
            ..Default::default()
        }
    }

    fn loadout(formation: i32, n: i32) -> DroneLoadout {
        DroneLoadout {
            formation,
            drones: (0..n).map(entry).collect(),
            user_scale: 1.0,
        }
    }

    fn near(a: (f32, f32), b: (f32, f32)) -> bool {
        (a.0 - b.0).abs() < 1e-3 && (a.1 - b.1).abs() < 1e-3
    }

    #[test]
    fn rotate_follows_flash_heading() {
        // 0 deg = east: unchanged.
        assert!(near(rotate((150.0, 0.0), 0.0), (150.0, 0.0)));
        // 90 deg = south (y down): +x offset swings to +y.
        assert!(near(rotate((150.0, 0.0), 90.0), (0.0, 150.0)));
        // 180 deg = west.
        assert!(near(rotate((150.0, 0.0), 180.0), (-150.0, 0.0)));
        // A south-side offset at 90 deg lands west.
        assert!(near(rotate((0.0, 100.0), 90.0), (-100.0, 0.0)));
    }

    #[test]
    fn offsets_scale_with_user_scale_and_fall_back() {
        let f = std_formation();
        assert_eq!(formation_offset(&f, 0, 2, 1, 1.0), (0.0, 100.0));
        assert_eq!(formation_offset(&f, 0, 2, 1, 2.0), (0.0, 200.0));
        // Unknown formation id -> STANDARD.
        assert_eq!(formation_offset(&f, 99, 2, 0, 1.0), (150.0, 0.0));
        // No row for 5 drones -> even ring, drone 0 at +x.
        let (x, y) = formation_offset(&f, 0, 5, 0, 1.0);
        assert!((x - 150.0).abs() < 1e-3 && y.abs() < 1e-3);
        // Empty table still places drones (ring).
        let empty = HashMap::new();
        let (x, _) = formation_offset(&empty, 0, 4, 0, 1.0);
        assert!((x - 150.0).abs() < 1e-3);
    }

    #[test]
    fn ease_out_is_monotonic_and_clamped() {
        assert_eq!(ease_out(0.0), 0.0);
        assert_eq!(ease_out(1.0), 1.0);
        assert_eq!(ease_out(5.0), 1.0);
        assert!(ease_out(0.5) > 0.5);
        assert!((ease_out(0.5) - 0.75).abs() < 1e-6);
    }

    #[test]
    fn new_drones_start_at_ship_centre_and_glide_out() {
        let f = std_formation();
        let t0 = Instant::now();
        let mut rig = DroneRig::default();
        rig.sync(&loadout(0, 2), &f, t0);
        let start = rig.offsets(t0);
        assert!(near(start[0].1, (0.0, 0.0)));
        assert!(near(start[1].1, (0.0, 0.0)));
        // Halfway through: ease_out(0.5) = 0.75 of the way.
        let mid = rig.offsets(t0 + Duration::from_secs_f32(TWEEN_SECS * 0.5));
        assert!(near(mid[0].1, (112.5, 0.0)));
        // Landed.
        let end = rig.offsets(t0 + Duration::from_secs(1));
        assert!(near(end[0].1, (150.0, 0.0)));
        assert!(near(end[1].1, (0.0, 100.0)));
        assert!(rig.settled(t0 + Duration::from_secs(1)));
        assert!(!rig.settled(t0));
    }

    #[test]
    fn retarget_starts_from_current_position() {
        let f = std_formation();
        let t0 = Instant::now();
        let mut rig = DroneRig::default();
        rig.sync(&loadout(0, 2), &f, t0);
        let t1 = t0 + Duration::from_secs(1);
        // Same loadout again: nothing restarts.
        rig.sync(&loadout(0, 2), &f, t1);
        assert!(rig.settled(t1));
        // Dropping to one drone retargets drone 0 from (150,0) to the
        // single-drone slot (also (150,0)) -> no tween; adding a third
        // (no row -> ring) retargets from the resting spot.
        rig.sync(&loadout(0, 3), &f, t1);
        let o = rig.offsets(t1);
        assert_eq!(o.len(), 3);
        assert!(near(o[0].1, (150.0, 0.0)), "drone 0 begins where it rested");
        assert!(near(o[2].1, (0.0, 0.0)), "new drone spawns at the ship");
        // Shrinking truncates.
        rig.sync(&loadout(0, 1), &f, t1 + Duration::from_secs(2));
        assert_eq!(rig.offsets(t1 + Duration::from_secs(2)).len(), 1);
    }
}
