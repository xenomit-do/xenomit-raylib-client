//! `LockDisplay3D` — the 3D target-lock indicator.
//!
//! The stock client keeps one `ParticleGroup` per `lockType` (`TYPE_2_URL`:
//! the `abstract_ui_mark_target_*.zip` files in the particle collection)
//! inside a container that `updateObj` moves to the target's
//! `x3D/y3D/z3D` every frame. On each target change the container pops in
//! at `1.5x` the target's `UberDisplayTrait.visualSize3D` and TweenLite's
//! it back to `1x` over 0.3 s (default `Quad.easeOut`); the group's
//! rotating ring/wing/arrow external meshes animate inside the group
//! data, so the container itself only positions and scales.
//!
//! Ours is a `particles3d` instance queued at the locked entity's Away3D
//! position each frame — [`queue_lock_indicator`] runs in the render pass
//! before `particles3d::flush` (wired in `render/mod.rs`). `Game::update`
//! fetches the four groups once per session (`LOCK3D_STEMS` fan-out next
//! to the recipe-particle requests), so requests render as soon as the
//! group and its meshes arrive.

use crate::game::Game;
use crate::net::SharedWorld;
use crate::render::{nameplate, particles3d};
use std::cell::Cell;
use std::time::Instant;

/// `LockDisplay3D.TYPE_2_URL` — particle-group stems per `lockType`
/// (1..=4: enemy, neutral, draw-fire, friend). `Game::update` keeps the
/// same list (`game::LOCK3D_STEMS`) for its once-per-session fetch —
/// `render/mod.rs` only wires this module after merge, so the const
/// cannot live here and be seen from `game` yet.
const STEMS: [&str; 4] = [
    "abstract_ui_mark_target_red",
    "abstract_ui_mark_target_gray_light",
    "abstract_ui_mark_target_purple",
    "abstract_ui_mark_target_gray_dark",
];

/// `lockType` -> particle group stem; stock falls back to
/// `TYPE_2_URL[1]` for types outside the table.
fn stem_for_lock_type(lock_type: usize) -> &'static str {
    lock_type
        .checked_sub(1)
        .and_then(|i| STEMS.get(i))
        .copied()
        .unwrap_or(STEMS[0])
}

/// `TweenLite.to(this, 0.3, {scale*: _scale})` from `set currentView` —
/// `1.5*base` down to `base` over 0.3 s with TweenLite's default
/// `Quad.easeOut` (`t*(2-t)`).
fn scale_in(base: f32, elapsed: f32) -> f32 {
    let t = (elapsed / 0.3).clamp(0.0, 1.0);
    base * (1.5 - 0.5 * t * (2.0 - t))
}

thread_local! {
    /// `(target id, shown_at)` — restarts the scale-in whenever the lock
    /// target changes (or the indicator leaves the scene and returns).
    static SHOWN: Cell<Option<(i32, Instant)>> = const { Cell::new(None) };
}

fn shown_at(target: i32) -> Instant {
    SHOWN.with(|cell| match cell.get() {
        Some((id, at)) if id == target => at,
        _ => {
            let now = Instant::now();
            cell.set(Some((target, now)));
            now
        }
    })
}

fn hidden() {
    SHOWN.with(|cell| cell.set(None));
}

/// Queue the lock indicator on the current `game.selected` entity, as a
/// `particles3d` request in Away3D scene coordinates.
///
/// Shown while the 3D ship scene can draw the target — the full
/// `camera_3d` mode, or the target's own component toggle
/// (`npcs_3d`/`ships_3d`, the same per-entity gate `draw_ship_models`
/// uses) in independent component mode. The stock display lives on the
/// 3D map layer, so nothing queues when only 2D rendering is active.
pub(crate) fn queue_lock_indicator(game: &Game, world: &SharedWorld) {
    let Some(id) = game.selected else {
        return hidden();
    };
    let Some(e) = world.world.entities.get(&id) else {
        return hidden();
    };
    let rp = game.preferences.render;
    if !(rp.camera_3d || if e.npc { rp.npcs_3d } else { rp.ships_3d }) {
        return hidden();
    }
    let shown_at = shown_at(id);
    // `lock_type` reads `clanDiplomacy` from `ShipInfo`; assets and ships
    // without a row fall back to the default (enemy) like the 2D
    // `ship_border` path.
    let info = world.ship_info.get(&id).cloned().unwrap_or_default();
    let lock_type = nameplate::lock_type(e.npc, &info);
    // `UberDisplayTrait.visualSize3D || 1` — the recipe `visualSize`
    // attr, defaulting to 1 when absent or authored 0.
    let base = game
        .entity_visuals
        .get(&id)
        .and_then(|v| v.display_3d.as_ref())
        .map(|d| d.visual_size)
        .filter(|s| *s != 0.0)
        .unwrap_or(1.0);
    let scale = scale_in(base, shown_at.elapsed().as_secs_f32());
    // `moveTo(target.owner.x3D, y3D, z3D)` — the map point on the Away3D
    // ground plane; the container is never rotated.
    let (x, y) = game.draw_pos(world, id);
    particles3d::queue(particles3d::Request {
        // Keyed per target AND lock type: a key change restarts the group
        // animator (stock `animator.time = 0` on `currentView`) and drops
        // anims instantiated from the previous type's group.
        key: format!("lock:{id}:{lock_type}"),
        resource: stem_for_lock_type(lock_type).to_owned(),
        pos: (x, 0.0, -y),
        rot: (0.0, 0.0, 0.0),
        scale: (scale, scale, scale),
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lock_types_map_to_type_2_url_stems() {
        assert_eq!(stem_for_lock_type(1), "abstract_ui_mark_target_red");
        assert_eq!(stem_for_lock_type(2), "abstract_ui_mark_target_gray_light");
        assert_eq!(stem_for_lock_type(3), "abstract_ui_mark_target_purple");
        assert_eq!(stem_for_lock_type(4), "abstract_ui_mark_target_gray_dark");
        // Out-of-table types fall back to the enemy marker (`TYPE_2_URL[1]`).
        assert_eq!(stem_for_lock_type(0), "abstract_ui_mark_target_red");
        assert_eq!(stem_for_lock_type(9), "abstract_ui_mark_target_red");
    }

    #[test]
    fn scale_in_eases_1_5_to_1_over_0_3s() {
        // TweenLite `Quad.easeOut`: 1.5*base -> base over 0.3 s.
        assert!((scale_in(2.0, 0.0) - 3.0).abs() < 1e-6);
        assert!((scale_in(2.0, 0.3) - 2.0).abs() < 1e-6);
        assert!((scale_in(2.0, 5.0) - 2.0).abs() < 1e-6);
        // t=0.5 -> ease 0.75 -> 1.5 - 0.5*0.75 = 1.125.
        assert!((scale_in(1.0, 0.15) - 1.125).abs() < 1e-6);
        let mut prev = f32::MAX;
        for i in 0..=30 {
            let s = scale_in(1.0, i as f32 * 0.01);
            assert!(s < prev && s >= 1.0);
            prev = s;
        }
    }
}
