//! Ship metadata applied on the game side (needs the asset patterns):
//! `VisualModifier` design swaps and swarm-minion docking.

use crate::assets::AssetManager;
use crate::game::effects::{Effect, ENGINE_SMOKE_LIFE};
use crate::net::SharedWorld;
use std::collections::HashSet;
use std::time::{Duration, Instant};
use xenomit_packet_client::model::EntityKind;

use super::Game;

/// `dock_{N}` pivot id of a mothership's N-th mount.
pub fn dock_pivot_id(position_index: i32) -> String {
    format!("dock_{position_index}")
}

/// The ship type an entity is drawn as: its `VisualModifier`
/// `CHANGE_SHIP_DESIGN` override when that names a known ship pattern,
/// else its own `type_id`.
pub fn effective_type_id<'a>(
    base: &'a str,
    design_override: Option<&'a str>,
    assets: &AssetManager,
) -> &'a str {
    match design_override {
        Some(ov) if !ov.is_empty() && assets.game_xml.ship(ov).is_some() => ov,
        _ => base,
    }
}

impl Game {
    /// Emit the configured `engineSmokeID` resource at each authored engine
    /// mount while its ship has an active movement tween.
    pub(super) fn update_engine_smoke(
        &mut self,
        world: &SharedWorld,
        assets: &mut AssetManager,
        now: Instant,
    ) {
        const EMIT_INTERVAL: Duration = Duration::from_millis(130);
        const PUFF_SCALE: f32 = 1.0;
        const DRIFT_DISTANCE: f32 = 75.0;

        let hero_id = world.world.hero.as_ref().map(|hero| hero.user_id);
        let mut ships: Vec<(i32, String)> = Vec::new();
        if let Some(hero) = &world.world.hero {
            let design = world
                .ship_info
                .get(&hero.user_id)
                .and_then(|info| info.design_override.as_deref());
            ships.push((
                hero.user_id,
                effective_type_id(&hero.type_id, design, assets).to_owned(),
            ));
        }
        ships.extend(
            world
                .world
                .entities
                .values()
                .filter(|entity| {
                    Some(entity.id) != hero_id
                        && !entity.destroyed
                        && matches!(entity.kind, EntityKind::Ship | EntityKind::Other)
                        && !world.pets.is_hidden(entity.id)
                })
                .map(|entity| {
                    let design = world
                        .ship_info
                        .get(&entity.id)
                        .and_then(|info| info.design_override.as_deref());
                    (
                        entity.id,
                        effective_type_id(&entity.type_id, design, assets).to_owned(),
                    )
                }),
        );

        let mut moving_ids = HashSet::new();
        let mut smoke_counts: std::collections::HashMap<i32, usize> = self
            .effects
            .active
            .iter()
            .filter_map(|effect| match effect {
                Effect::EngineSmoke { smoke_type, .. } => Some(*smoke_type),
                _ => None,
            })
            .fold(std::collections::HashMap::new(), |mut counts, id| {
                *counts.entry(id).or_default() += 1;
                counts
            });

        for (id, type_id) in ships {
            let Some(tween) = world.tweens.get(&id).filter(|tween| !tween.done()) else {
                continue;
            };
            if self.docked.contains_key(&id) {
                continue;
            }
            let center = world.pos(id);
            if !self.camera.in_view(center, 256.0) {
                continue;
            }
            let Some((smoke_id, position_class)) = assets
                .game_xml
                .ship_by_type(&type_id)
                .map(|pattern| (pattern.smoke_id, pattern.engine_position_id.clone()))
            else {
                continue;
            };
            let Some((res_key, alpha, rotate, pool_size)) = assets
                .game_xml
                .engine_smokes
                .get(&smoke_id)
                .map(|smoke| {
                    (
                        smoke.res_key.clone(),
                        smoke.alpha.clamp(0.0, 1.0),
                        smoke.rotation,
                        smoke.pool_size,
                    )
                })
                .filter(|(res_key, _, _, _)| !res_key.is_empty())
            else {
                continue;
            };
            let Some(positions) = assets
                .game_xml
                .engine_positions
                .get(&position_class)
                .map(|pattern| pattern.positions.clone())
                .filter(|positions| !positions.is_empty())
            else {
                continue;
            };
            moving_ids.insert(id);
            if self
                .engine_smoke_next
                .get(&id)
                .is_some_and(|next| now < *next)
            {
                continue;
            }
            self.engine_smoke_next.insert(id, now + EMIT_INTERVAL);

            if pool_size > 0
                && smoke_counts.get(&smoke_id).copied().unwrap_or_default() >= pool_size as usize
            {
                continue;
            }

            let sprite = *self
                .engine_smoke_sprites
                .entry(res_key.clone())
                .or_insert_with(|| assets.request(&res_key, "mc"));
            // Engine smoke is authored as a multi-frame SWF clip. Let the
            // puff live for its full timeline instead of cutting the game
            // animation off at a generic sub-second lifetime.
            let life = assets
                .get(sprite)
                .map(|clip| {
                    clip.frames
                        .iter()
                        .map(|frame| frame.duration_ms as u64)
                        .sum::<u64>()
                })
                .filter(|duration_ms| *duration_ms > 0)
                .map(|duration_ms| duration_ms as f32 / 1000.0)
                .unwrap_or(ENGINE_SMOKE_LIFE);
            let heading = self.headings.get(&id).copied().unwrap_or(0.0);
            let frame = ((32.0 * heading.rem_euclid(360.0) / 360.0 + 0.5) as usize) % 32;
            let (dx, dy) = (tween.to.0 - tween.from.0, tween.to.1 - tween.from.1);
            let distance = dx.hypot(dy).max(1.0);
            let drift = (
                -dx / distance * DRIFT_DISTANCE,
                -dy / distance * DRIFT_DISTANCE,
            );

            for mount in &positions {
                let Some(offset) = mount.get(frame % mount.len().max(1)).copied() else {
                    continue;
                };
                self.effects.push(Effect::EngineSmoke {
                    pos: (center.0 + offset.0, center.1 + offset.1),
                    drift,
                    smoke_type: smoke_id,
                    sprite: Some(sprite),
                    life,
                    scale: PUFF_SCALE,
                    alpha,
                    rotation: if rotate { heading } else { 0.0 },
                    started: now,
                });
                *smoke_counts.entry(smoke_id).or_default() += 1;
                if pool_size > 0
                    && smoke_counts.get(&smoke_id).copied().unwrap_or_default()
                        >= pool_size as usize
                {
                    break;
                }
            }
        }
        self.engine_smoke_next
            .retain(|id, _| moving_ids.contains(id));
    }

    /// Pet level of entity `id` (0 when it isn't a pet).
    pub(super) fn pet_level(world: &SharedWorld, id: i32) -> i32 {
        world.pets.get(id).map(|p| p.level).unwrap_or(0)
    }

    /// Type id + faction of ship `id` (hero or map entity).
    fn ship_type_faction(world: &SharedWorld, id: i32) -> Option<(String, i32)> {
        if let Some(h) = world.world.hero.as_ref().filter(|h| h.user_id == id) {
            return Some((h.type_id.clone(), h.faction_id));
        }
        world
            .world
            .entities
            .get(&id)
            .map(|e| (e.type_id.clone(), e.faction_id))
    }

    /// Recompute docked-minion positions: a swarm child
    /// (`mother_ship_id != 0`) renders on its mother's `dock_N` pivot
    /// instead of its own server position. Falls back to the mother's
    /// centre when the pattern declares no such pivot.
    pub(super) fn update_docks(&mut self, world: &SharedWorld, assets: &AssetManager) {
        self.docked.clear();
        for (&id, info) in &world.ship_info {
            if info.mother_ship_id == 0 || info.mother_ship_id == id {
                continue;
            }
            let Some((m_type, m_faction)) = Self::ship_type_faction(world, info.mother_ship_id)
            else {
                continue;
            };
            if !world.world.entities.contains_key(&id) {
                continue;
            }
            let base = world.pos(info.mother_ship_id);
            let pivot = assets
                .ship_pivot(&m_type, m_faction, &dock_pivot_id(info.position_index))
                .unwrap_or((0.0, 0.0));
            self.docked.insert(id, (base.0 + pivot.0, base.1 + pivot.1));
        }
    }

    /// Where entity `id` is drawn: its dock slot when docked, else the
    /// tweened server position.
    pub fn draw_pos(&self, world: &SharedWorld, id: i32) -> (f32, f32) {
        self.docked
            .get(&id)
            .copied()
            .unwrap_or_else(|| world.pos(id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dock_pivot_ids() {
        assert_eq!(dock_pivot_id(0), "dock_0");
        assert_eq!(dock_pivot_id(7), "dock_7");
    }
}
