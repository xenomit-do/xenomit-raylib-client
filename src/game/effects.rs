//! Map effects scheduled off `SharedWorld::fx` — the
//! `map.view2D.effects` equivalent (`LaserSalvo2D` / `Rocket2D` /
//! `PyroEffect2D` / `DamageReceivedEffect`): projectile laser shots,
//! rockets with smoke trails, sprite explosions, pickup flashes and
//! floating combat text.

use crate::assets::gamexml::LaserDefinition;
use crate::assets::SpriteId;
use std::collections::VecDeque;
use std::time::Instant;

/// Bolt flight time when the pattern carries no `speed`
/// (`LaserPattern.speed` default, seconds).
pub const LASER_SPEED_DEFAULT: f32 = 0.15;
/// Rocket floor duration (`Rocket2D`: `max(0.5, dist/500)`).
pub const ROCKET_MIN_DURATION: f32 = 0.5;
/// Rocket travel speed in world units/s (`dist/500` in stock).
pub const ROCKET_SPEED: f32 = 500.0;
/// Default smoke-puff lifetime when the pattern lacks `particleDuration`.
pub const SMOKE_LIFE_DEFAULT: f32 = 0.8;
/// How long a damage number floats (stock `DamageReceivedEffect` ~1s).
pub const TEXT_LIFE: f32 = 1.0;
/// White damage text for hits on other entities.
pub const COLOR_HIT_OTHER: u32 = 0xFFFFFF;
/// Red damage text for hits on the hero.
pub const COLOR_HIT_HERO: u32 = 0xFF5040;
/// `MISS` / green repair numbers.
pub const COLOR_MISS: u32 = 0xE8E8E8;
pub const COLOR_REPAIR: u32 = 0x6EFFA0;

/// Fallback bolt palette by attacker `laser_class` (0xRRGGBB) — used
/// only when neither the salvo definition's `lightColor` nor a real
/// laser sprite resolved.
pub fn laser_color_u32(class: i32) -> u32 {
    match class {
        1 => 0xFF3C28,
        2 => 0x5096FF,
        3 => 0x78FF5A,
        4 => 0xFFFFFF,
        5 => 0xFFAA3C,
        6 => 0xD25AFF,
        _ => 0xFF4632,
    }
}

/// What a beam end / pulse follows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Anchor {
    /// Fixed world position.
    Pos((f32, f32)),
    /// A ship/entity (hero included) — follows its interpolated position.
    Entity(i32),
}

/// Beam flavour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BeamKind {
    /// `HealBeamEffect` — green wavy link between two ships.
    Heal,
}

impl BeamKind {
    /// Seconds a beam of this kind lives.
    pub fn life(self) -> f32 {
        match self {
            BeamKind::Heal => HEAL_BEAM_LIFE,
        }
    }

    pub fn color(self) -> u32 {
        match self {
            BeamKind::Heal => 0x50FF8C,
        }
    }
}

/// Procedural ring/flash flavour for events without a bundled clip.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PulseKind {
    Emp,
    BurningTrail,
    Nuke,
    /// Teleport-in shimmer (grows then fades).
    SpawnIn,
    /// Teleport-out shimmer (collapses).
    SpawnOut,
    /// Bubble that follows the ship (`InstaShieldEffect`).
    InstaShield,
    /// Fallback when the smartbomb pyro clip is missing.
    Smartbomb,
    MegaMine,
}

impl PulseKind {
    pub fn color(self) -> u32 {
        match self {
            PulseKind::Emp => 0x78B4FF,
            PulseKind::BurningTrail => 0xFF7A28,
            PulseKind::Nuke => 0xFFE6A0,
            PulseKind::SpawnIn | PulseKind::SpawnOut => 0xB4F0FF,
            PulseKind::InstaShield => 0x66D8FF,
            PulseKind::Smartbomb => 0xFFB050,
            PulseKind::MegaMine => 0xFFFF78,
        }
    }

    /// Peak radius in world units.
    pub fn radius(self) -> f32 {
        match self {
            PulseKind::Emp => 600.0,
            PulseKind::BurningTrail => 350.0,
            PulseKind::Nuke => 1000.0,
            PulseKind::SpawnIn | PulseKind::SpawnOut => 130.0,
            PulseKind::InstaShield => 95.0,
            PulseKind::Smartbomb => 700.0,
            PulseKind::MegaMine => 400.0,
        }
    }

    pub fn life(self) -> f32 {
        match self {
            PulseKind::Emp | PulseKind::BurningTrail => 1.0,
            PulseKind::Nuke => 1.4,
            PulseKind::SpawnIn | PulseKind::SpawnOut => 0.8,
            PulseKind::InstaShield => 1.2,
            PulseKind::Smartbomb => 0.9,
            PulseKind::MegaMine => 1.0,
        }
    }
}

/// Heal ray duration (`HealBeamEffect(2000, ..)` ms).
pub const HEAL_BEAM_LIFE: f32 = 2.0;
/// Time to show the game-authored looping resource-collection beam.
pub const COLLECTOR_BEAM_LIFE: f32 = 0.7;
/// Engine smoke puff lifetime.
pub const ENGINE_SMOKE_LIFE: f32 = 0.75;

#[derive(Clone, Debug)]
pub enum Effect {
    /// Link between two anchors for `life` seconds (`HealBeamEffect`).
    Beam {
        from: Anchor,
        to: Anchor,
        kind: BeamKind,
        life: f32,
        started: Instant,
    },
    /// Game-authored `loopingCollectorBeam` stretched between the loot
    /// position and the player's ship.
    CollectorBeam {
        from: Anchor,
        to: Anchor,
        sprite: Option<SpriteId>,
        life: f32,
        started: Instant,
    },
    /// One animated game-authored engine smoke puff.
    EngineSmoke {
        pos: (f32, f32),
        drift: (f32, f32),
        smoke_type: i32,
        sprite: Option<SpriteId>,
        life: f32,
        scale: f32,
        alpha: f32,
        rotation: f32,
        started: Instant,
    },
    /// Expanding ring / flash at an anchor (EMP, spawn shimmer, shield).
    Pulse {
        at: Anchor,
        kind: PulseKind,
        started: Instant,
    },
    /// `EMPEffect` — `lensFlash` bitmap + `blitz` clip + the
    /// `shockring0*` rings of the `shockwaves` file (replaces the
    /// procedural `Pulse` for EMPs).
    Emp {
        at: Anchor,
        kind: crate::net::EmpKind,
        flash: Option<SpriteId>,
        blitz: Option<SpriteId>,
        /// One `shockring0i` clip per staggered ring (nuke: 00..06,
        /// else 01..05).
        rings: Vec<SpriteId>,
        started: Instant,
    },
    /// `LaserSalvo2D.showShieldDamageClip` — a `("shieldDamage","mc")`
    /// played once at 30 fps on the victim's edge facing the shot,
    /// `delay` = the bolt's flight time.
    ShieldDamage {
        victim: i32,
        /// `atan2(dy,dx) + 180` rounded — bolt travel direction, deg.
        angle: f32,
        /// Seconds before the clip appears (salvo delay + flight).
        delay: f32,
        /// Clip play seconds (`framesLoaded / 30`); refreshed once the
        /// sprite resolves (starts at a conservative bound).
        clip_life: f32,
        sprite: Option<SpriteId>,
        started: Instant,
    },
    /// `EmoteEffect2D` — an `emotes` SWF clip over the ship for
    /// `EmoteAttributes.duration` (5 s), scaling/fading out on expiry.
    Emote {
        entity: i32,
        sprite: Option<SpriteId>,
        started: Instant,
    },
    /// One laser bolt in flight (`LaserSalvo2D.shoot`): linear travel
    /// `from` -> `to` over `duration` s; `to` is already pulled back by
    /// the bolt's `laserLength` so its tip lands on the victim.
    LaserShot {
        from: (f32, f32),
        to: (f32, f32),
        /// Gfx sprite (`LaserPattern.resKey`/`skillResKey` -> "mc");
        /// `None` falls back to a thin colored bolt.
        sprite: Option<SpriteId>,
        /// Flight seconds (`LaserPattern.speed`).
        duration: f32,
        /// Launch delay in seconds — salvo groups fire staggered at
        /// 200 ms intervals (`shotSalvo` `setTimeout(idx * 200)`).
        delay: f32,
        /// SAB absorber: bolt shrinks to 0.1 as it reaches the shooter.
        absorber: bool,
        /// `playLoop` clips play their timeline instead of rotating.
        play_loop: bool,
        /// Fallback bolt color (0xRRGGBB, attacker laser class).
        color: u32,
        /// Stock `LaserSalvo3D`/`LaserBullet3D` data resolved at spawn —
        /// lets `render::effects3d` draw the authored 3D projectile
        /// (crossed planes / single plane / particle group) instead of
        /// the billboard. `None` = 2D/billboard paths only.
        vfx: Option<LaserShot3D>,
        started: Instant,
    },
    /// `Rocket2D`: linear flight at the live victim position, smoke
    /// puffs dropped along the path, an impact effect on arrival.
    /// Misses finish at a perpendicular offset from the victim.
    Rocket {
        from: (f32, f32),
        /// Victim id — the stock tween tracks the entity live.
        victim: i32,
        /// Victim position captured at spawn (used when it leaves).
        to: (f32, f32),
        /// Perpendicular endpoint offset applied when `!hit`.
        miss_offset: (f32, f32),
        /// `max(0.5, dist/500)` seconds.
        duration: f32,
        hit: bool,
        /// Hero fires or receives — a miss shows "MISS" (stock
        /// `DamageReceivedEffect(owner,0)`).
        hero_involved: bool,
        sprite: Option<SpriteId>,
        smoke_sprite: Option<SpriteId>,
        /// Manhattan distance between smoke puffs
        /// (`RocketSmokePattern.particleInterval`); <= 0 disables.
        smoke_interval: f32,
        /// Puff lifetime (`particleDuration`).
        smoke_life: f32,
        /// Position of the last emitted puff (`i32::MIN` = none yet).
        last_smoke: (i32, i32),
        /// Set once the arrival impact has been emitted.
        finished: bool,
        /// Impact pyro sprite (class 3 hit / class 2 miss).
        impact_sprite: Option<SpriteId>,
        impact_scale: f32,
        started: Instant,
    },
    /// A fixed-position clip burst: ship explosions (`pyroEffects`
    /// class 0), rocket impact flashes and smoke puffs. An unresolved
    /// `sprite` (still downloading) draws the placeholder ring instead.
    SpriteBurst {
        pos: (f32, f32),
        sprite: Option<SpriteId>,
        /// Seconds shown (clip's own durations pace the frames inside).
        life: f32,
        scale: f32,
        /// Fade out over the last 40% of `life` (smoke puffs).
        fade: bool,
        /// Spawn-time rotation in degrees (`rotatable` pyro patterns get
        /// a random one like stock `Math.random() * 360`).
        rotation: f32,
        /// Draw the placeholder ring while `sprite` has no frames —
        /// ship explosions keep the old ring as a stream-in stand-in.
        ring_fallback: bool,
        started: Instant,
    },
    CollectFlash {
        pos: (f32, f32),
        started: Instant,
    },
    /// Floating combat text — damage numbers (`DamageReceivedEffect`)
    /// and `MISS` indicators.
    Text {
        pos: (f32, f32),
        text: String,
        /// 0xRRGGBB.
        color: u32,
        started: Instant,
    },
}

impl Effect {
    /// Rocket position at `now` — linear interpolation (`TweenLite`
    /// `Linear.easeNone` for ordinary rockets).
    pub fn rocket_pos(&self, target: (f32, f32), now: Instant) -> (f32, f32) {
        let Effect::Rocket {
            from,
            miss_offset,
            duration,
            started,
            ..
        } = *self
        else {
            return target;
        };
        let to = (target.0 + miss_offset.0, target.1 + miss_offset.1);
        let t = (now.saturating_duration_since(started).as_secs_f32() / duration.max(0.01))
            .clamp(0.0, 1.0);
        (from.0 + (to.0 - from.0) * t, from.1 + (to.1 - from.1) * t)
    }

    /// Seconds a `LaserShot` has been flying (0 before `delay` elapses).
    pub fn flight_t(&self, now: Instant) -> f32 {
        match self {
            Effect::LaserShot {
                duration,
                delay,
                started,
                ..
            } => ((now.saturating_duration_since(*started).as_secs_f32() - delay)
                / duration.max(0.01))
            .clamp(0.0, 1.0),
            _ => 0.0,
        }
    }

    /// Laser-bolt position at `now`.
    pub fn shot_pos(&self, now: Instant) -> (f32, f32) {
        match self {
            Effect::LaserShot { from, to, .. } => {
                let t = self.flight_t(now);
                (from.0 + (to.0 - from.0) * t, from.1 + (to.1 - from.1) * t)
            }
            _ => (0.0, 0.0),
        }
    }
}

/// Which authored projectile body `render::effects3d` draws for a
/// `LaserShot` — `LaserBullet3D`'s `laserType` switch (`""` and unknown
/// values default to `"doublePlane"` in stock).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LaserKind3D {
    /// `"doublePlane"` (and default): two crossed planes textured with
    /// the laser clip, rolled so `Vector3D.Z_AXIS` is the lookAt up hint.
    Cross,
    /// `"singlePlane"`: one centered plane, twice the clip extent.
    Single,
    /// `"particle"`: the `{resKey}.zip` particle group, `lookAt`-ed along
    /// the flight direction.
    Particles,
}

impl LaserKind3D {
    /// `LaserPattern.laserType` -> kind (`""`/`"doublePlane"` -> `Cross`).
    pub fn of(laser_type: &str) -> Self {
        match laser_type {
            "singlePlane" => Self::Single,
            "particle" => Self::Particles,
            _ => Self::Cross,
        }
    }
}

/// Muzzle-flash spawn for a `LaserShot` that fired from an authored
/// `laserpoint_*` slot (`LaserSalvo3D.shotSalvo`: a one-shot
/// `{muzzleFlashResKey}.zip` particle group parented to the ship mesh at
/// the slot).
#[derive(Clone, Debug)]
pub struct LaserMuzzle3D {
    /// Particle collection stem (`muzzle_flash_red`).
    pub resource: String,
    /// Composed ship+mesh Euler degrees for the flash group (it inherits
    /// the mesh node's world rotation in stock).
    pub rot: (f32, f32, f32),
}

/// Stock-3D spawn state for a `LaserShot`, resolved in `spawn_world_fx`
/// while the attacker/victim positions and slot table are reachable.
/// All positions are Away3D scene coordinates (`map.y` is `-z`).
#[derive(Clone, Debug)]
pub struct LaserShot3D {
    /// Which projectile body the 3D path draws (`laserType`).
    pub kind: LaserKind3D,
    /// `LaserPattern.scale` — the bolt container's uniform scale
    /// (default 1); absorbers tween toward 0.4 in stock.
    pub scale: f32,
    /// `LaserSalvo3D._loc4_` before the absorber swap: the attacker's
    /// `position3D` + the resolved local cannon offset (authored
    /// `laserpoint_*` slot, else the 2D mount offset, else the ship
    /// center). Muzzle flashes anchor here too.
    pub spawn: (f32, f32, f32),
    /// `LaserSalvo3D._loc5_` BEFORE the shield pullback — the victim's
    /// `position3D` captured at fire time.
    pub end: (f32, f32, f32),
    /// Shield pullback distance (`_visualSize * 50`, `LaserSalvo3D.
    /// shoot`) — 0 when the victim had no shield or no `visualSize`.
    pub shield_pull: f32,
    /// `LaserBullet3D.bulletLength` (`laserLength || mc.width`), NOT
    /// scaled by `pattern.scale` — used for `checkDistance` and the
    /// end-of-tween pullback.
    pub bullet_len: f32,
    /// Muzzle flash emitted once when an authored slot fired the bolt.
    pub muzzle: Option<LaserMuzzle3D>,
    /// Particle collection stem for `Particles` bolts (the
    /// skill-resolved `resKey` — loaded as `{stem}.zip`).
    pub particle: String,
}

/// `LaserSalvo2D.shoot` bolt plan for one `LaserDefinition`:
/// - `absorber` (SAB) bolts fly victim -> shooter and shrink on arrival.
/// - Otherwise the endpoint is pulled back by `mcWidth` (else the clip
///   width, `laserLength` being our proxy) so the bolt tip (not its
///   tail) lands on the target; shots shorter than the bolt are dropped
///   entirely (stock: `if (_loc18_ < _loc15_ * _loc15_) return`).
/// - `duration` is the pattern `speed` in *seconds*, distance-free
///   (`TweenLite.to(clip, pattern.getSpeed(), …)`).
///
/// Returns `None` in the stock skip case. `delay` staggers the bolt's
/// launch (salvo groups fire 200 ms apart — `shotSalvo` timeouts).
pub fn laser_shot(
    from: (f32, f32),
    to: (f32, f32),
    def: &LaserDefinition,
    sprite: Option<SpriteId>,
    color: u32,
    delay: f32,
    vfx: Option<LaserShot3D>,
) -> Option<Effect> {
    let (mut a, mut b) = (from, to);
    if def.absorber {
        std::mem::swap(&mut a, &mut b);
    }
    let dx = b.0 - a.0;
    let dy = b.1 - a.1;
    let d2 = dx * dx + dy * dy;
    let len = (if def.mc_width > 0 {
        def.mc_width
    } else {
        def.laser_length
    })
    .max(0) as f32;
    if !def.absorber && len > 0.0 && d2 < len * len {
        return None;
    }
    let end = if !def.absorber && len > 0.0 {
        let d = d2.sqrt();
        (b.0 + (a.0 - b.0) / d * len, b.1 + (a.1 - b.1) / d * len)
    } else {
        b
    };
    Some(Effect::LaserShot {
        from: a,
        to: end,
        sprite,
        duration: if def.speed > 0.0 {
            def.speed
        } else {
            LASER_SPEED_DEFAULT
        },
        absorber: def.absorber,
        play_loop: def.play_loop,
        color,
        delay,
        vfx,
        started: Instant::now(),
    })
}

/// `Rocket2D` spawn: flight time `max(0.5, dist/500)` and, for misses,
/// a perpendicular endpoint offset of `miss_dist` world units.
/// `rand` (0..1) picks the miss side — deterministic in tests.
pub fn rocket_flight(from: (f32, f32), to: (f32, f32), hit: bool, rand: f32) -> (f32, (f32, f32)) {
    let dx = to.0 - from.0;
    let dy = to.1 - from.1;
    let dist = (dx * dx + dy * dy).sqrt();
    let duration = (dist / ROCKET_SPEED).max(ROCKET_MIN_DURATION);
    let miss_offset = if hit || dist < 1.0 {
        (0.0, 0.0)
    } else {
        // Stock's gaps rocket misses land a randomized ~70-120 units
        // beside the target; sign alternates by `rand`.
        let m = 80.0 + 40.0 * rand;
        let side = if rand >= 0.5 { 1.0 } else { -1.0 };
        (-dy / dist * m * side, dx / dist * m * side)
    };
    (duration, miss_offset)
}

/// Manhattan distance moved since the last smoke puff — the stock
/// `Rocket2D.update` gate (`abs(x1-x2)+abs(y1-y2) > particleInterval`).
pub fn smoke_moved(last: (i32, i32), pos: (f32, f32)) -> f32 {
    if last.0 == i32::MIN {
        return f32::MAX;
    }
    (pos.0 - last.0 as f32).abs() + (pos.1 - last.1 as f32).abs()
}

/// Damage-number text effect for an `AttackHit`/`AttackHitNoLock`
/// (`DamageReceivedEffect`). `damage <= 0` produces nothing (stock
/// gates on `displayHitpointBubbles && damage != 0`); `attack_type ==
/// 10` (REPAIR) is a green `+N`.
pub fn hit_text(
    pos: (f32, f32),
    damage: i64,
    attack_type: i32,
    on_hero: bool,
    strike: bool,
    locale: &crate::locale::LocaleCatalog,
) -> Option<Effect> {
    const ATTACK_TYPE_REPAIR: i32 = 10;
    if damage <= 0 {
        return None;
    }
    let (text, color) = if attack_type == ATTACK_TYPE_REPAIR {
        (format!("+{}", locale.format_i64(damage)), COLOR_REPAIR)
    } else {
        (
            format!(
                "{}{}",
                if strike { "CRIT " } else { "-" },
                locale.format_i64(damage)
            ),
            if on_hero {
                COLOR_HIT_HERO
            } else {
                COLOR_HIT_OTHER
            },
        )
    };
    Some(Effect::Text {
        pos,
        text,
        color,
        started: Instant::now(),
    })
}

/// Pulse radius fraction (of `kind.radius()`) and alpha at normalized
/// time `t` in `0..=1`.
pub fn pulse_shape(kind: PulseKind, t: f32) -> (f32, f32) {
    let t = t.clamp(0.0, 1.0);
    let ease_out = 1.0 - (1.0 - t) * (1.0 - t);
    match kind {
        PulseKind::SpawnOut => (1.0 - ease_out, 1.0 - t * t),
        PulseKind::InstaShield => (
            1.0 + 0.04 * (t * 18.0).sin(),
            (1.0 - t).min(t * 8.0).clamp(0.0, 1.0),
        ),
        _ => (ease_out, 1.0 - t),
    }
}

/// `EMPEffect` seconds on screen — the last staggered ring finishes at
/// `(rings-1)*stagger + 1.5` (emperor staggers at `i*0.1*0.3`); the
/// `blitz` clip's fade ends earlier.
pub fn emp_life(kind: crate::net::EmpKind) -> f32 {
    use crate::net::EmpKind;
    let (rings, stagger) = match kind {
        EmpKind::Nuke => (7, 0.1),
        EmpKind::Emperor => (4, 0.03),
        // `TYPE_2_RINGCOUNT[TYPE_ULTIMATE]` = 1, burning trail = 5.
        EmpKind::Ultimate => (1, 0.1),
        _ => (5, 0.1),
    };
    (rings - 1) as f32 * stagger + 1.5
}

#[derive(Default)]
pub struct EffectQueue {
    pub active: VecDeque<Effect>,
    /// Deterministic "random" for miss offsets + pyro rotations
    /// (stock's `Math.random`; tests need stable values).
    seed: u64,
}

impl EffectQueue {
    pub fn push(&mut self, e: Effect) {
        self.active.push_back(e);
    }

    /// splitmix64 -> [0,1).
    pub fn rand(&mut self) -> f32 {
        self.seed = self.seed.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = self.seed;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^= z >> 31;
        ((z >> 40) & 0xFF_FFFF) as f32 / (1u64 << 24) as f32
    }

    /// Drop expired effects. Rockets persist one extra beat past their
    /// flight so the arrival impact frame is not cut off.
    pub fn gc(&mut self) {
        let now = Instant::now();
        self.active.retain(|e| match e {
            Effect::LaserShot {
                duration,
                delay,
                started,
                ..
            } => now.saturating_duration_since(*started).as_secs_f32() < *duration + *delay + 0.05,
            // Once `finished` the arrival impact has been emitted as its
            // own effect — the rocket itself is done.
            Effect::Rocket { finished, .. } => !*finished,
            Effect::SpriteBurst { life, started, .. } => {
                now.saturating_duration_since(*started).as_secs_f32() < *life
            }
            Effect::CollectFlash { started, .. } => started.elapsed().as_secs_f32() < 0.5,
            Effect::Beam { life, started, .. } => {
                now.saturating_duration_since(*started).as_secs_f32() < *life
            }
            Effect::CollectorBeam { life, started, .. } => {
                now.saturating_duration_since(*started).as_secs_f32() < *life
            }
            Effect::EngineSmoke { life, started, .. } => {
                now.saturating_duration_since(*started).as_secs_f32() < *life
            }
            Effect::Pulse { kind, started, .. } => {
                now.saturating_duration_since(*started).as_secs_f32() < kind.life()
            }
            Effect::Emp { kind, started, .. } => {
                now.saturating_duration_since(*started).as_secs_f32() < emp_life(*kind)
            }
            Effect::ShieldDamage {
                delay,
                clip_life,
                started,
                ..
            } => now.saturating_duration_since(*started).as_secs_f32() < *delay + *clip_life,
            // `EmoteAttributes.duration` — every emote lasts 5 s.
            Effect::Emote { started, .. } => started.elapsed().as_secs_f32() < 5.0,
            Effect::Text { started, .. } => started.elapsed().as_secs_f32() < TEXT_LIFE,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn def(len: i32, speed: f32) -> LaserDefinition {
        LaserDefinition {
            laser_length: len,
            speed,
            ..Default::default()
        }
    }

    #[test]
    fn laser_shot_pulls_back_by_clip_width() {
        let Effect::LaserShot { to, duration, .. } = laser_shot(
            (0.0, 0.0),
            (400.0, 0.0),
            &def(80, 0.5),
            None,
            0xFF0000,
            0.0,
            None,
        )
        .expect("shot") else {
            panic!("not a LaserShot");
        };
        // 80u pull-back so the bolt tip (not tail) lands on the victim.
        assert_eq!(to, (320.0, 0.0));
        // Duration is the pattern `speed` in seconds — not dist/velocity.
        assert_eq!(duration, 0.5);
    }

    #[test]
    fn laser_shot_mc_width_overrides_laser_length() {
        let mut d = def(200, 0.15);
        d.mc_width = 40;
        let Effect::LaserShot { to, .. } =
            laser_shot((0.0, 0.0), (400.0, 0.0), &d, None, 0, 0.0, None).expect("shot")
        else {
            panic!("not a LaserShot");
        };
        assert_eq!(to, (360.0, 0.0));
    }

    #[test]
    fn laser_shot_drops_shorter_than_bolt() {
        // dist 50 < bolt width 80 -> stock `if (d2 < w*w) return`.
        assert!(laser_shot((0.0, 0.0), (50.0, 0.0), &def(80, 0.15), None, 0, 0.0, None).is_none());
    }

    #[test]
    fn laser_shot_no_pullback_without_length() {
        let Effect::LaserShot { to, duration, .. } =
            laser_shot((0.0, 0.0), (100.0, 0.0), &def(0, 0.0), None, 0, 0.0, None).expect("shot")
        else {
            panic!("not a LaserShot");
        };
        assert_eq!(to, (100.0, 0.0));
        assert_eq!(duration, LASER_SPEED_DEFAULT);
    }

    #[test]
    fn laser_shot_absorber_flies_victim_to_shooter() {
        let mut d = def(0, 0.5);
        d.absorber = true;
        let Effect::LaserShot {
            from, to, absorber, ..
        } = laser_shot((0.0, 0.0), (400.0, 0.0), &d, None, 0, 0.0, None).expect("shot")
        else {
            panic!("not a LaserShot");
        };
        assert!(absorber);
        // SAB drains victim -> shooter, so the endpoints swap.
        assert_eq!(from, (400.0, 0.0));
        assert_eq!(to, (0.0, 0.0));
    }

    #[test]
    fn rocket_flight_duration_and_endpoint() {
        // 1000 units at 500 u/s -> 2.0s flight, miss lands ~90u off.
        let (dur, off) = rocket_flight((0.0, 0.0), (1000.0, 0.0), false, 0.25);
        assert_eq!(dur, 2.0);
        assert_eq!(off, (0.0, -90.0)); // side -1, m = 80 + 40*0.25
                                       // A hit lands exactly on the victim.
        let (_, hit_off) = rocket_flight((0.0, 0.0), (1000.0, 0.0), true, 0.75);
        assert_eq!(hit_off, (0.0, 0.0));
        // Short hops clamp to the 0.5s floor.
        let (min_dur, _) = rocket_flight((0.0, 0.0), (100.0, 0.0), true, 0.0);
        assert_eq!(min_dur, ROCKET_MIN_DURATION);
    }

    #[test]
    fn smoke_moved_gates_puffs() {
        assert_eq!(smoke_moved((i32::MIN, i32::MIN), (0.0, 0.0)), f32::MAX);
        assert_eq!(smoke_moved((0, 0), (3.0, 4.0)), 7.0);
    }

    #[test]
    fn hit_text_colors_and_shapes() {
        let loc = crate::locale::LocaleCatalog::default();
        let Effect::Text { text, color, .. } =
            hit_text((0.0, 0.0), 1234, 1, false, false, &loc).expect("text")
        else {
            panic!("not a Text");
        };
        assert_eq!(text, "-1.234"); // locale grouping (fallback `.`)
        assert_eq!(color, COLOR_HIT_OTHER);

        let Effect::Text { color, .. } =
            hit_text((0.0, 0.0), 1234, 1, true, false, &loc).expect("text")
        else {
            panic!("not a Text");
        };
        assert_eq!(color, COLOR_HIT_HERO);

        let Effect::Text { text, color, .. } =
            hit_text((0.0, 0.0), 500, 10, false, false, &loc).expect("text")
        else {
            panic!("not a Text");
        };
        assert_eq!(text, "+500");
        assert_eq!(color, COLOR_REPAIR);

        let Effect::Text { text, .. } =
            hit_text((0.0, 0.0), 99, 1, false, true, &loc).expect("text")
        else {
            panic!("not a Text");
        };
        assert_eq!(text, "CRIT 99");

        // Stock gates on `damage != 0`.
        assert!(hit_text((0.0, 0.0), 0, 1, false, false, &loc).is_none());
    }

    #[test]
    fn gc_drops_finished_rocket_and_expired_text() {
        let mut q = EffectQueue::default();
        q.push(Effect::Rocket {
            from: (0.0, 0.0),
            victim: 1,
            to: (10.0, 0.0),
            miss_offset: (0.0, 0.0),
            duration: 0.5,
            hit: true,
            hero_involved: false,
            sprite: None,
            smoke_sprite: None,
            smoke_interval: 0.0,
            smoke_life: 0.0,
            last_smoke: (i32::MIN, i32::MIN),
            finished: true,
            impact_sprite: None,
            impact_scale: 1.0,
            started: Instant::now(),
        });
        q.push(Effect::Text {
            pos: (0.0, 0.0),
            text: "old".into(),
            color: 0,
            started: Instant::now() - Duration::from_secs(60),
        });
        q.push(Effect::Text {
            pos: (0.0, 0.0),
            text: "fresh".into(),
            color: 0,
            started: Instant::now(),
        });
        q.gc();
        assert_eq!(q.active.len(), 1);
        assert!(matches!(&q.active[0], Effect::Text { text, .. } if text == "fresh"));
    }

    #[test]
    fn pulse_shapes() {
        let (r0, a0) = pulse_shape(PulseKind::Emp, 0.0);
        let (r1, a1) = pulse_shape(PulseKind::Emp, 1.0);
        assert_eq!((r0, a0, r1, a1), (0.0, 1.0, 1.0, 0.0));
        // Despawn collapses inward.
        assert_eq!(pulse_shape(PulseKind::SpawnOut, 0.0).0, 1.0);
        assert_eq!(pulse_shape(PulseKind::SpawnOut, 1.0).0, 0.0);
        // Shield fades in fast, out at the end.
        assert!(pulse_shape(PulseKind::InstaShield, 0.05).1 < 1.0);
        assert_eq!(pulse_shape(PulseKind::InstaShield, 1.0).1, 0.0);
    }

    #[test]
    fn gc_expires_beams_and_pulses() {
        let mut q = EffectQueue::default();
        let old = Instant::now() - Duration::from_secs(60);
        q.push(Effect::Beam {
            from: Anchor::Pos((0.0, 0.0)),
            to: Anchor::Entity(1),
            kind: BeamKind::Heal,
            life: HEAL_BEAM_LIFE,
            started: old,
        });
        q.push(Effect::Pulse {
            at: Anchor::Entity(1),
            kind: PulseKind::Emp,
            started: old,
        });
        q.push(Effect::Pulse {
            at: Anchor::Entity(1),
            kind: PulseKind::Emp,
            started: Instant::now(),
        });
        q.gc();
        assert_eq!(q.active.len(), 1);
    }

    #[test]
    fn rand_is_deterministic() {
        let (mut a, mut b) = (EffectQueue::default(), EffectQueue::default());
        for _ in 0..8 {
            assert_eq!(a.rand(), b.rand());
        }
        // And stays inside [0,1).
        let mut q = EffectQueue::default();
        for _ in 0..64 {
            let r = q.rand();
            assert!((0.0..1.0).contains(&r));
        }
    }
}
