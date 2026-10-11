//! Combat-FX spawn helpers — the `EffectManager`/`EMPEffect2D`/
//! `LaserSalvo2D` side of `Game::update`, split out of `mod.rs`.

use crate::assets::AssetManager;
use crate::game::effects::{self, Effect};
use crate::game::ship_fx;
use crate::game::Game;
use crate::net::ships::ModifierRow;
use crate::net::SharedWorld;
use std::time::Instant;
use xenomit_packet_client::model::EntityKind;

impl Game {
    /// Per-frame combat-FX housekeeping: resolve `shieldDamage` clip
    /// play time once the sprite lands (`framesLoaded / 30`) and turn
    /// expired `POLARITY_POWERUP` bolts into `EMPEffect(TYPE_EMPEROR)`
    /// spawns (`PolarityPowerupEffect2D.delayCompleteHandler`).
    pub(super) fn update_combat_fx(
        &mut self,
        world: &SharedWorld,
        assets: &mut AssetManager,
        now: Instant,
    ) {
        for e in self.effects.active.iter_mut() {
            if let Effect::ShieldDamage {
                clip_life, sprite, ..
            } = e
            {
                if let Some(clip) = sprite.and_then(|id| assets.get(id)) {
                    *clip_life = clip.frames.len() as f32 / 30.0;
                }
            }
        }
        for entity in std::mem::take(&mut self.ship_fx.pending_emps) {
            self.spawn_emp(world, assets, entity, crate::net::EmpKind::Emperor, now);
        }
    }

    /// `WorldFx::VisualModifier` — `VisualModifierAssembly` plus the
    /// `mapObject is Ship` gate and `FxAction::Pyro` dispatch
    /// (sticky-bomb deactivate explosion through `pyroEffects`).
    pub(super) fn apply_visual_modifier(
        &mut self,
        world: &SharedWorld,
        assets: &mut AssetManager,
        entity: i32,
        row: &ModifierRow,
        now: Instant,
    ) {
        let hero_id = world.world.hero.as_ref().map(|h| h.user_id);
        let is_ship = Some(entity) == hero_id
            || world
                .world
                .entities
                .get(&entity)
                .is_some_and(|e| matches!(e.kind, EntityKind::Ship | EntityKind::Other));
        let pos = world.pos(entity);
        let actions = self.ship_fx.apply_modifier(entity, pos, row, is_ship);
        for action in actions {
            let ship_fx::modifiers::FxAction::Pyro { class, variant } = action else {
                continue;
            };
            let s = self.pyro_sound_id(assets, class, &[variant]);
            if s > 0 {
                crate::audio::play_sound(s, Some(pos));
            } else {
                crate::audio::play_sfx_at(crate::audio::Sfx::Explosion, pos.0, pos.1);
            }
            if let Some(e) = self.pyro_burst(assets, class, &[variant], pos, 1.4, true, now) {
                self.effects.push(e);
            }
        }
    }

    /// `EMPEffect2D.initEffectVisuals` — `lensFlash` bitmap + the
    /// `blitz`/`blitzUltimate` bolt + `TYPE_2_RINGCOUNT` staggered
    /// `shockring0*` clips from the `shockwaves` file (emperor's first
    /// two rings are `shockring0Red`).
    pub(super) fn spawn_emp(
        &mut self,
        world: &SharedWorld,
        assets: &mut AssetManager,
        target: i32,
        kind: crate::net::EmpKind,
        now: Instant,
    ) {
        use crate::net::EmpKind;
        let pos = world.pos(target);
        crate::audio::play_sfx_at(crate::audio::Sfx::Emp, pos.0, pos.1);
        let flash = Some(assets.request("lensFlash", "lensFlash"));
        let blitz = Some(assets.request(
            "shockwaves",
            if kind == EmpKind::Ultimate {
                "blitzUltimate"
            } else {
                "blitz"
            },
        ));
        let ring_res = match kind {
            EmpKind::Nuke => "shockring0Purple",
            EmpKind::BurningTrail => "shockring0Orange",
            EmpKind::Ultimate => "shockringUltimate",
            _ => "shockring0",
        };
        let count = match kind {
            EmpKind::Nuke => 7,
            EmpKind::Emperor => 4,
            EmpKind::Ultimate => 1,
            _ => 5,
        };
        let rings = (0..count)
            .map(|i| {
                let res = if kind == EmpKind::Emperor && i < 2 {
                    "shockring0Red"
                } else {
                    ring_res
                };
                assets.request("shockwaves", res)
            })
            .collect();
        self.effects.push(Effect::Emp {
            at: effects::Anchor::Entity(target),
            kind,
            flash,
            blitz,
            rings,
            started: now,
        });
    }

    /// `LaserSalvo2D.showShieldDamageClip` — `("shieldDamage","mc")`
    /// clip on the victim, `delay` = bolt flight; capped at 9 live
    /// clips per target (`shieldDamageLookUp`).
    pub(super) fn shield_damage_hit(
        &mut self,
        assets: &mut AssetManager,
        victim: i32,
        angle: f32,
        delay: f32,
    ) {
        let live = self
            .effects
            .active
            .iter()
            .filter(|e| {
                matches!(e, Effect::ShieldDamage { victim: v, .. } if *v == victim)
            })
            .count();
        if live >= 9 {
            return;
        }
        let sprite = Some(assets.request("shieldDamage", "mc"));
        self.effects.push(Effect::ShieldDamage {
            victim,
            angle,
            delay,
            clip_life: 1.0,
            sprite,
            started: Instant::now(),
        });
    }

    /// `pattern.soundID` of `pyroEffects[class][first-existing id]`
    /// (the `MapObjectEffect` audio hook).
    pub(super) fn pyro_sound_id(&self, assets: &AssetManager, class: i32, ids: &[i32]) -> i32 {
        ids.iter()
            .find_map(|id| assets.game_xml.pyro(class, *id))
            .map(|p| p.sound_id)
            .unwrap_or(0)
    }
}
