//! Ship/asset visual effects driven by `VisualModifierCommand` — the
//! `map/model` `EffectManager` + `EffectsManager2D` equivalent.
//!
//! `net/ships.rs` forwards every modifier row as
//! `WorldFx::VisualModifier`; `modifiers::actions` ports the
//! `VisualModifierAssembly` switch to `FxAction`s; `catalog` holds the
//! per-`EffectIDList` view descriptors and `render/ship_fx.rs` draws
//! them in three layers (below hull / above hull / fx layer).

pub mod catalog;
pub mod modifiers;

use crate::assets::{AssetManager, SpriteId};
use catalog::{FxDesc, Kind};
use modifiers::{AddFx, FxAction, Life, Ring};
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// One live effect on an entity (or a positional free effect).
#[derive(Clone, Debug)]
pub struct ActiveFx {
    /// `EffectIDList` id.
    pub effect: i32,
    pub life: Life,
    /// Case args (glow color, bomb stage, shield pct, boss level, …).
    pub a: i64,
    pub b: i64,
    /// Ring parameters for `Kind::Ring`.
    pub ring: Option<Ring>,
    /// Delayed start (drive-area continued clip).
    pub delay: f32,
    /// World position captured at add — positional effects
    /// (`GlowingRing`, spawn clips) don't follow the entity.
    pub pos: Option<(f32, f32)>,
    pub started: Instant,
    /// Resolved sprite (once the resKey was known).
    pub sprite: Option<SpriteId>,
    /// Random rotation picked at spawn (`Rot::Random`).
    pub random_rot: f32,
    /// `RepairRobotEffect.repairBotId` — `<repairRobots>` loot_id
    /// override (the `BeaconCommand` rep-bot); `None` = the
    /// effect-table defaults for ids 12/302.
    pub loot: Option<String>,
    /// `RobotClipBehavior` orbit state — one per `robotsNum`.
    pub robots: Vec<RobotFx>,
    /// Shared `_nextRobotAngleRadians` accumulator.
    pub robot_angle: f32,
    /// `_sparksRes` atlas sprite + the sparks clip's pattern offset.
    pub sparks: Option<SpriteId>,
    pub sparks_off: (f32, f32),
    /// `_robotPattern` clip offset.
    pub robot_off: (f32, f32),
}

/// `RobotClipBehavior` — one orbiting repair clip's tween state.
#[derive(Clone, Debug)]
pub struct RobotFx {
    /// Current target angle (rad, relative to owner rotation).
    pub angle: f32,
    /// Angle the running 0.2 s linear tween started from.
    pub from: f32,
    /// Age (s) the running tween started.
    pub tween_at: f32,
    /// Age (s) of the next retarget — `nextMoveTimer` fires every 2-3 s.
    pub next: f32,
    /// Orbit radius factor — `randomBetween(1,1.2) × clickRadius`.
    pub radius: f32,
    /// Sparks orbit — `randomBetween(0.3,0.6) × radius`, ±0.1 rad jitter.
    pub sparks_r: f32,
    pub sparks_a: f32,
    /// Deterministic LCG stream for the retarget rolls.
    pub seed: u64,
}

impl RobotFx {
    /// `nextMoveTimer = 0` at spawn — the first update retargets.
    fn new(seed: u64) -> Self {
        Self {
            angle: 0.0,
            from: 0.0,
            tween_at: 0.0,
            next: 0.0,
            radius: 1.0,
            sparks_r: 0.0,
            sparks_a: 0.0,
            seed,
        }
    }
}

/// Per-entity effect lists — `EffectManager._effects`.
#[derive(Default)]
pub struct ShipFxState {
    /// entity id -> active effects.
    pub effects: HashMap<i32, Vec<ActiveFx>>,
    /// Positional `addEffectOnce` effects keyed by effect id —
    /// `GlowingRingEffect`, spawn clips not bound to an entity.
    pub free: HashMap<i32, ActiveFx>,
    /// Entities whose effects should drop (despawned).
    pub gone: std::collections::HashSet<i32>,
    /// Ships whose `POLARITY_POWERUP` one-shot just finished —
    /// `PolarityPowerupEffect2D.delayCompleteHandler` ends it in an
    /// `EMPEffect(TYPE_EMPEROR)`; `Game` drains this into `Effect::Emp`.
    pub pending_emps: Vec<i32>,
}

impl ShipFxState {
    /// `map.effects` translation of a `VisualModifierCommand` row.
    /// `ship` = whether the entity satisfies `mapObject is Ship`.
    /// Returns the `FxAction`s applied (test hook / pyro dispatch).
    pub fn apply_modifier(
        &mut self,
        entity: i32,
        entity_pos: (f32, f32),
        row: &crate::net::ships::ModifierRow,
        ship: bool,
    ) -> Vec<FxAction> {
        let actions = modifiers::actions(row, ship);
        for action in &actions {
            self.apply_action(entity, entity_pos, *action);
        }
        actions
    }

    /// Apply one action (also used by the legacy aura routing).
    pub fn apply_action(&mut self, entity: i32, entity_pos: (f32, f32), action: FxAction) {
        match action {
            FxAction::Add(add) => self.add(entity, entity_pos, add),
            FxAction::Remove(id) => self.remove(entity, id),
            // Pyro bursts are dispatched by the game layer (they need
            // the game.xml pyro table + effects queue).
            FxAction::Pyro { .. } => {}
        }
    }

    /// `addMapObjectEffectOnce`: dedupe by (entity, effect id) for
    /// persistent/timed effects — re-adding `reactivate`s (timer reset).
    /// `Once` effects always stack, like `NOT_PERSISTANT` effects.
    pub fn add(&mut self, entity: i32, entity_pos: (f32, f32), add: AddFx) {
        // `SphereShieldEffect`: a re-activation updates the percentage
        // on the existing effect instead of re-adding.
        if add.effect == 99 {
            if let Some(existing) = self
                .effects
                .get_mut(&entity)
                .and_then(|list| list.iter_mut().find(|e| e.effect == 99))
            {
                existing.a = add.a;
                existing.started = Instant::now();
                return;
            }
        }
        if matches!(add.life, Life::Once) && add.effect == 58 {
            // Positional `GlowingRingEffect` — `addEffectOnce` dedupes
            // by id across the whole map.
            self.free.insert(
                add.effect,
                ActiveFx {
                    effect: add.effect,
                    life: add.life,
                    a: add.a,
                    b: add.b,
                    ring: add.ring,
                    delay: add.delay,
                    pos: Some(entity_pos),
                    started: Instant::now(),
                    sprite: None,
                    random_rot: 0.0,
                    loot: None,
                    robots: Vec::new(),
                    robot_angle: 0.0,
                    sparks: None,
                    sparks_off: (0.0, 0.0),
                    robot_off: (0.0, 0.0),
                },
            );
            return;
        }
        let list = self.effects.entry(entity).or_default();
        if matches!(add.life, Life::Persistent | Life::Timed(_)) {
            if let Some(existing) = list.iter_mut().find(|e| e.effect == add.effect) {
                // `reactivate` — keep the sprite, refresh args + timer.
                existing.a = add.a;
                existing.b = add.b;
                existing.ring = add.ring;
                existing.delay = add.delay;
                existing.started = Instant::now();
                return;
            }
        }
        list.push(ActiveFx {
            effect: add.effect,
            life: add.life,
            a: add.a,
            b: add.b,
            ring: add.ring,
            delay: add.delay,
            pos: if matches!(add.effect, 19 | 71 | 96 | 413) {
                Some(entity_pos)
            } else {
                None
            },
            started: Instant::now(),
            sprite: None,
            random_rot: rand_rot(),
            loot: None,
            robots: Vec::new(),
            robot_angle: 0.0,
            sparks: None,
            sparks_off: (0.0, 0.0),
            robot_off: (0.0, 0.0),
        });
    }

    /// `RepairRobotEffect(REP_BOT_EFFECT, hero, lootId, skilled?1:0)` —
    /// the `BeaconCommand` repair bot (`BeaconHandler`); `loot` selects
    /// the `<repairRobots>` row, `skilled` the sparks table.
    pub fn add_repair_bot(
        &mut self,
        entity: i32,
        entity_pos: (f32, f32),
        loot: &str,
        skilled: bool,
    ) {
        self.add(
            entity,
            entity_pos,
            AddFx {
                effect: modifiers::fx::REP_BOT_EFFECT,
                life: Life::Persistent,
                a: skilled as i64,
                b: 0,
                ring: None,
                delay: 0.0,
            },
        );
        if let Some(e) = self
            .effects
            .get_mut(&entity)
            .and_then(|list| {
                list.iter_mut()
                    .find(|e| e.effect == modifiers::fx::REP_BOT_EFFECT)
            })
        {
            // `repairBotId.changed` → `reloadView`: a loot-id change
            // re-requests the atlas and rebuilds the robot clips.
            if e.loot.as_deref() != Some(loot) {
                e.sprite = None;
                e.robots.clear();
                e.sparks = None;
                e.robot_angle = 0.0;
            }
            e.loot = Some(loot.to_owned());
        }
    }

    /// Direct clip add for triggers that skip `modifiers::actions`
    /// (`WorldFx::ShipFx` auras, `InstaShield`, `LUP`).
    pub fn add_clip(&mut self, entity: i32, entity_pos: (f32, f32), effect: i32, life: Life) {
        self.add(
            entity,
            entity_pos,
            AddFx {
                effect,
                life,
                a: 0,
                b: 0,
                ring: None,
                delay: 0.0,
            },
        );
    }

    /// Toggle a direct `EffectIDList` effect — `WorldFx::ShipFx`
    /// events (legacy `fx|start` auras are `PERSISTANT`, `LUP`'s
    /// `LevelUpEffect` is `NOT_PERSISTANT`).
    pub fn toggle(&mut self, entity: i32, entity_pos: (f32, f32), effect: i32, activated: bool) {
        if activated {
            let life = if effect == modifiers::fx::LEVEL_UP {
                Life::Once
            } else {
                Life::Persistent
            };
            self.add_clip(entity, entity_pos, effect, life);
        } else {
            self.remove(entity, effect);
        }
    }

    /// `removeEffectFromMapObject(entity, effect)`.
    pub fn remove(&mut self, entity: i32, effect: i32) {
        if let Some(list) = self.effects.get_mut(&entity) {
            list.retain(|e| e.effect != effect);
            if list.is_empty() {
                self.effects.remove(&entity);
            }
        }
        self.free.remove(&effect);
    }

    /// `removeAllEffectsForMapObject` — entity despawned.
    pub fn remove_entity(&mut self, entity: i32) {
        self.effects.remove(&entity);
    }

    /// Whole-map clear (`ScreenManager` reset).
    pub fn clear(&mut self) {
        self.effects.clear();
        self.free.clear();
    }

    /// Drop effects whose entity left the map (`Map.removeMapObject`).
    /// `alive` reports entity/hero presence.
    pub fn cleanup(&mut self, mut alive: impl FnMut(i32) -> bool) {
        self.effects.retain(|id, _| alive(*id));
    }

    /// Effect list for an entity (draw iteration).
    pub fn for_entity(&self, entity: i32) -> &[ActiveFx] {
        self.effects.get(&entity).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Aggregate hull alpha (ghost 0.4, scramble 0.2) — multiplicative.
    pub fn hull_alpha(&self, entity: i32) -> f32 {
        self.effects.get(&entity).map_or(1.0, |list| {
            list.iter()
                .filter_map(hull_alpha_of)
                .fold(1.0, |acc, a| acc * a)
        })
    }

    /// Ship-view jitter amplitude for the entity (berserk/rapid-fire).
    pub fn shake(&self, entity: i32, time: f32) -> (f32, f32) {
        let amp = self.effects.get(&entity).map_or(0.0, |list| {
            list.iter()
                .filter_map(|e| catalog::lookup(e.effect).map(|d| d.shake))
                .fold(0.0_f32, f32::max)
        });
        if amp <= 0.0 {
            return (0.0, 0.0);
        }
        // ±2 px jitter at ~13 Hz (TweenMax 0.075 s steps).
        let x = (time * 13.0).sin() * amp;
        let y = (time * 17.0 + 1.3).cos() * amp;
        (x, y)
    }

    /// Per-frame bookkeeping: expire timed/one-shot effects, tick the
    /// repair-robot orbit tweens, and lazily request the clip each
    /// effect needs.
    pub fn update(&mut self, assets: &mut AssetManager) {
        let now = Instant::now();
        let pending = &mut self.pending_emps;
        for (&entity, list) in self.effects.iter_mut() {
            list.retain(|e| {
                if expired(e, assets, now) {
                    if e.effect == modifiers::fx::POLARITY_POWERUP {
                        pending.push(entity);
                    }
                    return false;
                }
                true
            });
            for e in list.iter_mut() {
                tick_robots(e, now);
                request_sprite(e, assets);
            }
        }
        self.effects.retain(|_, list| !list.is_empty());
        self.free.retain(|_, e| !expired(e, assets, now));
        for e in self.free.values_mut() {
            request_sprite(e, assets);
        }
    }
}

/// `HullAlpha` descriptors also dim the hull for `Clip` kinds carrying
/// `hull_alpha` (scramble).
fn hull_alpha_of(e: &ActiveFx) -> Option<f32> {
    catalog::lookup(e.effect).and_then(|d| match d.kind {
        Kind::HullAlpha => Some(d.hull_alpha),
        _ if d.hull_alpha > 0.0 => Some(d.hull_alpha),
        _ => None,
    })
}

/// Resolve the sprite for clip-based effects — `(resKey, symbol)`.
/// Non-clip kinds (glow/ring/hull-alpha/turbo handled separately)
/// request nothing here.
fn request_sprite(e: &mut ActiveFx, assets: &mut AssetManager) {
    if e.sprite.is_some() {
        return;
    }
    let Some(desc) = catalog::lookup(e.effect) else {
        return;
    };
    let res = match desc.res {
        catalog::Res::Fixed(id) => id.to_owned(),
        catalog::Res::Pattern => pattern_res(assets, e.effect).unwrap_or_default(),
        catalog::Res::PatternOr(fallback) => {
            pattern_res(assets, e.effect).filter(|r| !r.is_empty()).unwrap_or_else(|| fallback.to_owned())
        }
    };
    if res.is_empty() {
        return;
    }
    // `GRAVITATIONAL_ANCHOR_ACTIVATE` shares effect id 407 with
    // `BOSS_EMP_SHOCK`: the anchor action carries no attribute, the
    // boss shock carries the range — disambiguate on `a`.
    let res = if e.effect == 407 && e.a == 0 {
        pattern_res(assets, 407).unwrap_or_else(|| "ship_hyperion_effect_continued".to_owned())
    } else {
        res
    };
    let symbol: String = match desc.kind {
        Kind::StickyBomb => sticky_symbol(e.a).to_owned(),
        _ if desc.symbol == "level" => assets
            .game_xml
            .boss_shields
            .get(&(e.a.max(1) as i32))
            .cloned()
            .unwrap_or_else(|| "level1".to_owned()),
        _ => desc.symbol.to_owned(),
    };
    let res = match desc.kind {
        Kind::StickyBomb => sticky_res(e.a).to_owned(),
        _ => res,
    };
    // Repair robots get their res from the `<repairRobots>` table —
    // effect 76 has no `<effects>` row to key off. Chain bolts are
    // procedural (no clip).
    if !matches!(desc.kind, Kind::RepairRobots | Kind::ChainLink)
        && (res.is_empty() || symbol.is_empty())
    {
        return;
    }
    e.sprite = match desc.kind {
        Kind::ChainLink => None,
        // `{resKeyPrefix}{_atlas,_texture}` — Starling atlas, every
        // `SubTexture` is one robot frame (`DOBitmapAtlasResource` +
        // `DOFlashMovieClip(res,"",30)`).
        Kind::RepairRobots => {
            let Some((pat, sparks)) = robot_patterns(assets, e) else {
                return;
            };
            e.robot_off = pat.offset;
            if e.robots.is_empty() {
                let seed = e.random_rot.to_bits() as u64;
                e.robots = (0..pat.robots)
                    .map(|i| RobotFx::new(seed ^ (i as u64 * 0x9E37_79B9)))
                    .collect();
                e.robot_angle = 0.0;
            }
            if e.sparks.is_none() {
                if let Some(sp) = sparks {
                    e.sparks_off = sp.offset;
                    e.sparks = Some(assets.request_region_anim(
                        &format!("{}_atlas", sp.res_key),
                        &format!("{}_texture", sp.res_key),
                        "",
                        33,
                    ));
                }
            }
            Some(assets.request_region_anim(
                &format!("{}_atlas", pat.res_key),
                &format!("{}_texture", pat.res_key),
                "",
                33,
            ))
        }
        _ => Some(assets.request(&res, &symbol)),
    };
}

fn pattern_res(assets: &AssetManager, id: i32) -> Option<String> {
    assets
        .game_xml
        .effects
        .get(&id)
        .map(|p| p.res_key.clone())
        .filter(|r| !r.is_empty())
}

/// `StickyBombEffect2D.DATA[type].libId`.
fn sticky_res(stage: i64) -> &'static str {
    match stage {
        1 => "emperorStickyBomb",
        2 => "explosiveChargingBlob",
        _ => "stickyBomb",
    }
}

fn sticky_symbol(_stage: i64) -> &'static str {
    "mc"
}

/// `RepairRobotEffect` -> `<repairRobots>` row (`reloadView`: loot_id
/// miss falls back to `_default`; sparks fall back to skill level 0).
fn robot_patterns(
    assets: &AssetManager,
    e: &ActiveFx,
) -> Option<(
    crate::assets::gamexml::RepairRobotPattern,
    Option<crate::assets::gamexml::RepairRobotPattern>,
)> {
    let default_loot = match e.effect {
        12 => "_tech_battle_robot",
        302 => "_tech_battle_robot2",
        _ => "_default",
    };
    let loot = e.loot.as_deref().unwrap_or(default_loot);
    let robot = assets
        .game_xml
        .repair_robots
        .get(loot)
        .or_else(|| assets.game_xml.repair_robots.get("_default"))?
        .clone();
    let sparks = assets
        .game_xml
        .repair_sparks
        .get(&(e.a as i32))
        .or_else(|| assets.game_xml.repair_sparks.get(&0))
        .cloned();
    Some((robot, sparks))
}

/// `updateTimer` — each robot retargets every 2-3 s (`randomBetween(
/// -2000,-3000)`), sharing the `_nextRobotAngleRadians` accumulator,
/// and linear-tweens to it over 0.2 s. Rolls are LCG-deterministic so
/// the pure state stays reproducible.
fn tick_robots(e: &mut ActiveFx, now: Instant) {
    if e.robots.is_empty()
        || !matches!(
            catalog::lookup(e.effect).map(|d| d.kind),
            Some(Kind::RepairRobots)
        )
    {
        return;
    }
    let age = now
        .saturating_duration_since(e.started)
        .as_secs_f32()
        - e.delay;
    if age < 0.0 {
        return;
    }
    for r in e.robots.iter_mut() {
        while age >= r.next {
            // Finish the running tween at the retarget instant.
            let t = ((r.next - r.tween_at) / 0.2).clamp(0.0, 1.0);
            r.from += (r.angle - r.from) * t;
            r.tween_at = r.next;
            e.robot_angle += roll(&mut r.seed, 0.2, 1.0) * std::f32::consts::PI;
            r.angle = e.robot_angle;
            r.radius = roll(&mut r.seed, 1.0, 1.2);
            r.sparks_r = r.radius * roll(&mut r.seed, 0.3, 0.6);
            r.sparks_a = roll(&mut r.seed, -0.1, 0.1);
            r.next += roll(&mut r.seed, 2.0, 3.0);
        }
    }
}

/// `randomBetween` — deterministic LCG float in `[lo, hi)`.
fn roll(seed: &mut u64, lo: f32, hi: f32) -> f32 {
    *seed = seed
        .wrapping_mul(6_364_136_223_846_793_005)
        .wrapping_add(1_442_695_040_888_963_407);
    let u = ((*seed >> 33) & 0xFF_FFFF) as f32 / 16_777_216.0;
    lo + u * (hi - lo)
}

/// `Effect.TIME_OUT` expiry + one-shot clip completion (`handleEffectComplete`).
fn expired(e: &ActiveFx, assets: &AssetManager, now: Instant) -> bool {
    if e.delay > 0.0 && now < e.started + Duration::from_secs_f32(e.delay) {
        return false;
    }
    let age = now.saturating_duration_since(e.started).as_secs_f32() - e.delay;
    if let Life::Timed(secs) = e.life {
        return age >= secs;
    }
    if e.life != Life::Once {
        return false;
    }
    // One-shot: done when the clip finished; fall back to a fixed
    // lifetime while the sprite is still streaming or unavailable.
    let Some(desc) = catalog::lookup(e.effect) else {
        return age >= 2.0;
    };
    if desc.kind == Kind::Ring {
        let cycle = e.ring.map(|r| r.cycle).unwrap_or(1.5);
        // `cycle 0` rings are static — they live until removed.
        return cycle > 0.0 && age >= cycle;
    }
    match e.sprite.and_then(|id| assets.get(id)) {
        Some(clip) if !clip.frames.is_empty() => {
            let total: f32 = clip
                .frames
                .iter()
                .map(|f| (f.duration_ms.max(1) as f32) / 1000.0)
                .sum();
            let total = if total > 0.0 {
                total / desc.rate.max(0.1)
            } else {
                clip.frames.len() as f32 / 30.0 / desc.rate.max(0.1)
            };
            age >= total + 0.05
        }
        _ => age >= 3.0,
    }
}

/// `InstaShieldEffect2D` picks a random rotation once.
fn rand_rot() -> f32 {
    (std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0)
        % 360_000) as f32
        / 1000.0
}

/// `BossShieldEffect2D` clip scale: 4 on level 4, else 2.
pub fn boss_shield_scale(level: i64) -> f32 {
    if level == 4 {
        4.0
    } else {
        2.0
    }
}

/// The `desc` of an active effect (render convenience).
pub fn desc(e: &ActiveFx) -> Option<&'static FxDesc> {
    catalog::lookup(e.effect)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(modifier: i32, activated: bool) -> crate::net::ships::ModifierRow {
        crate::net::ships::ModifierRow {
            modifier,
            attribute: 0,
            additional: String::new(),
            activated,
        }
    }

    fn test_assets() -> AssetManager {
        AssetManager::new(
            std::path::PathBuf::from("target/ship-fx-test"),
            crate::assets::FileIndex::default(),
            crate::assets::gamexml::GameXml::default(),
            crate::assets::UberAssets::default(),
            crate::assets::mapxml::MapDb::default(),
        )
    }

    #[test]
    fn add_once_dedupes_persistent() {
        let mut st = ShipFxState::default();
        st.apply_modifier(7, (0.0, 0.0), &row(89, true), true);
        st.apply_modifier(7, (0.0, 0.0), &row(89, true), true);
        assert_eq!(st.for_entity(7).len(), 1);
    }

    #[test]
    fn once_effects_stack() {
        // `NOT_PERSISTANT` effects are never deduped (SpawnEffectWithFadeIn).
        let mut st = ShipFxState::default();
        st.apply_modifier(7, (0.0, 0.0), &row(48, true), true);
        st.apply_modifier(7, (0.0, 0.0), &row(48, true), true);
        assert_eq!(st.for_entity(7).len(), 2);
    }

    #[test]
    fn remove_drops_the_effect() {
        let mut st = ShipFxState::default();
        st.apply_modifier(7, (0.0, 0.0), &row(89, true), true);
        st.apply_modifier(7, (0.0, 0.0), &row(89, false), true);
        assert!(st.for_entity(7).is_empty());
    }

    #[test]
    fn entity_cleanup_drops_list() {
        let mut st = ShipFxState::default();
        st.apply_modifier(7, (0.0, 0.0), &row(89, true), true);
        st.remove_entity(7);
        assert!(st.for_entity(7).is_empty());
        st.apply_modifier(7, (0.0, 0.0), &row(89, true), true);
        st.cleanup(|id| id != 7);
        assert!(st.for_entity(7).is_empty());
    }

    #[test]
    fn timed_effects_expire() {
        let mut st = ShipFxState::default();
        st.apply_modifier(7, (0.0, 0.0), &row(131, true), true);
        let e = &mut st.effects.get_mut(&7).unwrap()[0];
        e.started = Instant::now() - Duration::from_secs(4);
        st.update(&mut test_assets());
        assert!(st.for_entity(7).is_empty());
    }

    /// `RepairRobotEffect.reloadView` — the `BeaconCommand` rep-bot
    /// resolves its `<repairRobots>` row by loot id and falls back to
    /// `_default`; sparks fall back to skill level 0.
    #[test]
    fn repair_bot_pattern_fallback() {
        let xml = std::fs::read_to_string("tests/fixtures/game.xml").expect("fixture");
        let mut assets = test_assets();
        assets.game_xml = crate::assets::gamexml::GameXml::parse(&xml).unwrap();
        let mut st = ShipFxState::default();
        st.add_repair_bot(7, (0.0, 0.0), "equipment_extra_repbot_rep-3", true);
        let e = &st.for_entity(7)[0];
        let (pat, sparks) = robot_patterns(&assets, e).unwrap();
        assert_eq!(pat.res_key, "repair_robot3");
        assert_eq!(pat.robots, 3);
        assert_eq!(sparks.unwrap().res_key, "repair_sparksblue");
        // Unknown loot id → `_default` (`repair_robot1`), unskilled →
        // level-0 sparks.
        st.add_repair_bot(8, (0.0, 0.0), "missing-loot", false);
        let e = &st.for_entity(8)[0];
        let (pat, sparks) = robot_patterns(&assets, e).unwrap();
        assert_eq!(pat.res_key, "repair_robot1");
        assert_eq!(pat.robots, 1);
        assert_eq!(sparks.unwrap().res_key, "repair_sparks");
    }

    #[test]
    fn sphere_shield_pct_updates_in_place() {
        let mut st = ShipFxState::default();
        let mut r = row(68, true);
        r.attribute = 500;
        st.apply_modifier(7, (0.0, 0.0), &r, true);
        r.attribute = 800;
        st.apply_modifier(7, (0.0, 0.0), &r, true);
        assert_eq!(st.for_entity(7).len(), 1);
        assert_eq!(st.for_entity(7)[0].a, 80);
    }
}
