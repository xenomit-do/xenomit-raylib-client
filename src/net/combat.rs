use xenomit_packet_client::modules::id;
use xenomit_packet_client::packet::Packet;

use crate::net::world::SharedWorld;

/// `AttackRocketCommand` — a rocket was fired (not in `modules::id`).
pub(crate) const ATTACK_ROCKET: i16 = 6;
/// `AttackMissedCommand` — an attack whiffed (not in `modules::id`).
pub(crate) const ATTACK_MISSED: i16 = 163;

/// Combat-fx work item produced by `apply_packet` and drained by
/// `Game::update` — keeps `SharedWorld` renderer-free (mirrors how the
/// stock handlers turn commands into `map.effects.playEffect` calls).
#[derive(Clone, Debug)]
pub enum WorldFx {
    /// `AttackLaserRunCommand` — one salvo volley; the server re-sends
    /// roughly once a second while the laser link runs.
    LaserRun {
        attacker: i32,
        victim: i32,
        /// `laserColor` — the ammo `type` into `laserClasses[class][type]`
        /// (`LaserEffect.laserTypeID`, not a colour).
        laser_type: i32,
        skilled: bool,
        /// `isDiminishedBySkillShield` — `LaserSalvo` `showShieldDamage`
        /// flag: shield-hit clips play on the victim at bolt landing.
        shield_damage: bool,
        /// `visualType` — `LaserSalvo3D._shieldVisualType`, the
        /// `<shields><shield type>` key for the 3D shield-impact group
        /// (0 = `NORMAL_LASER_AND_ROCKET`).
        shield_type: i32,
    },
    /// `AttackRocketCommand`.
    Rocket {
        attacker: i32,
        victim: i32,
        /// `rocketType` — id into `rocketClasses[class][id]`.
        rocket_type: i32,
        /// `smokeId` — `rocketSmokePatterns` key.
        smoke_id: i32,
        hit: bool,
        heat_seeking: bool,
    },
    /// `AttackHitCommand`/`AttackHitNoLockCommand` — a damage number.
    Hit {
        victim: i32,
        attacker: i32,
        damage: i64,
        /// `attackType.attackTypeValue` (`AttackTypeModule`: LASER=1).
        attack_type: i32,
        strike: bool,
    },
    /// `AttackMissedCommand` — "MISS" over the target.
    Miss { victim: i32 },
    /// Legacy `0|n|SAB_SHOT|from|to` — `ChasingShotEffect("sab_shot")`.
    SabShot { attacker: i32, victim: i32 },
    /// Legacy `0|n|HEAL_RAY|healer|target` — `HealBeamEffect(2000, ..)`.
    HealRay { healer: i32, target: i32 },
    /// Legacy `0|n|MIN|hash` — `MineCommon.bang()`; `pos` is the mine's
    /// position read from the collectable table at packet time.
    MineExplode { pos: (f32, f32) },
    /// Legacy `0|n|FWI|hash` / `FWG|hash…` — `FireworkBox.fire()`.
    /// Carries the removed box's pattern inputs for `FireworksEffect`:
    /// `type_id` (`fireworkDescriptor` order key), `modifier`
    /// (`fireworkPatterns` class), `colour` (`colourCode`).
    FireworksIgnite {
        pos: (f32, f32),
        type_id: i32,
        modifier: String,
        colour: i32,
    },
    /// Legacy `0|n|SMB|uid` — smartbomb pyro (class 4) on the ship.
    Smartbomb { owner: i32 },
    /// Legacy `0|n|ISH|uid` — `InstaShieldEffect`.
    InstaShield { owner: i32 },
    /// Legacy `0|n|EMP|id` / `BT|id` / `NNK|id` / `GOPM|id` — `EMPEffect`.
    Emp { target: i32, kind: EmpKind },
    /// Legacy `0|n|PMME|id` — pet mega-mine `GlowingRingEffect`.
    MegaMine { target: i32 },
    /// Legacy `0|n|Spawn|uid` / `Despawn|x|y` — teleport shimmer
    /// (`SpawnEffect`, same class for both directions).
    Spawn { pos: (f32, f32), despawn: bool },
    /// Legacy `0|n|BOOOM|uid` — kamikaze "soundword" explosion.
    SpecialExplosion { ship: i32 },
    /// `VisualModifierCommand` (280) or an activated initial modifier
    /// row carried by `ShipCreate`/`ShipInitialization`/`AssetCreate`.
    /// `game::ship_fx` translates the row (`VisualModifierAssembly`).
    VisualModifier {
        entity: i32,
        row: crate::net::ships::ModifierRow,
    },
    /// A clip effect by `EffectIDList` id, toggled outside
    /// `VisualModifierCommand`: legacy `0|n|fx|start/end|NAME` auras
    /// (`MapEventsAssembly.assembleGraphicFx`) and `0|A|LUP` level-up
    /// (`SetAttributeAssembly.assembleLevelUpdate`).
    ShipFx {
        entity: i32,
        effect: i32,
        activated: bool,
    },
    /// `BeaconCommand.repairBotActive` — the hero's `RepairRobotEffect`
    /// (`REP_BOT_EFFECT`); `loot_id` selects the `<repairRobots>` row,
    /// `skilled` the sparks table. `active=false` removes the effect.
    RepairBot {
        active: bool,
        loot_id: String,
        skilled: bool,
    },
    /// `EmoteActivationCommand` — `EmoteEffect(entity, lootId,
    /// duration)`: the `emotes` SWF clip floating over the ship.
    Emote { entity: i32, loot_id: String },
    /// `PetActivationCommand`/`PetHeroActivationCommand` — the pet entity
    /// just spawned (`PetSpawnEffect` in stock).
    PetSpawn { entity: i32 },
}

/// `EMPEffect` flavour (`TYPE_*`); `Ultimate` comes from
/// `AbilityEffectActivationCommand` (JAMX), `Emperor` follows the
/// polarity power-up bolt (`PolarityPowerupEffect2D`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EmpKind {
    Normal,
    BurningTrail,
    Nuke,
    Ultimate,
    Emperor,
}

/// `0|n|fx|start|NAME` (`assembleGraphicFx`) — the name maps onto the
/// same `EffectIDList` clips the visual-modifier path drives.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StatusFx {
    Rage,
    SaboteurDebuff,
    IcyCube,
    SpeedBuff,
    Skull,
    Invincibility,
}

impl StatusFx {
    /// `ServerCommands.GRAPHIC_FX_*` / `SPEED_BUFF` / `RESPAWN_PROTECTION`.
    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "RAGE" => StatusFx::Rage,
            "SABOTEUR_DEBUFF" => StatusFx::SaboteurDebuff,
            "ICY_CUBE" => StatusFx::IcyCube,
            "SPEED_BUFF" => StatusFx::SpeedBuff,
            "SKULL" => StatusFx::Skull,
            "INVINCIBILITY" | "RESPAWN_PROTECTION" => StatusFx::Invincibility,
            _ => return None,
        })
    }

    /// `EffectIDList` id `assembleGraphicFx` adds/removes for the name:
    /// `RageEffect`, `SlowDownEffect`, `IcyCubeEffect`,
    /// `MapObjectEffect(SPEED_BUFF_EFFECT/SKULL)` and
    /// `InvincibilityEffect`.
    pub fn effect_id(self) -> i32 {
        match self {
            StatusFx::Rage => 17,
            StatusFx::SaboteurDebuff => 9,
            StatusFx::IcyCube => 79,
            StatusFx::SpeedBuff => 14,
            StatusFx::Skull => 18,
            StatusFx::Invincibility => 16,
        }
    }
}

/// `CombatMusicCommand.getLibcomModuleId` — not in `modules::id`.
const COMBAT_MUSIC: i16 = -12650;

pub fn handle(world: &mut SharedWorld, p: &Packet) {
    match p.id {
        // `CombatMusicHandler.run` → `Settings.combatMusicActive`.
        COMBAT_MUSIC => {
            world.combat_music_active = p.int("combatEvent").unwrap_or(1) == 0;
        }
        // `assembleEffectActivation`: Spearhead JAMX (110) /
        // PLUS_JAMX_CREED (146) plays `EMPEffect(TYPE_ULTIMATE)` on
        // the activator ship.
        id::ABILITY_EFFECT_ACTIVATION => {
            if matches!(p.int("selectedAbilityId"), Ok(110 | 146)) {
                if let Ok(u) = p.int("activatorId") {
                    world.fx.push(WorldFx::Emp {
                        target: u as i32,
                        kind: EmpKind::Ultimate,
                    });
                }
            }
        }
        id::EMOTE_ACTIVATION => {
            if let (Ok(u), Ok(loot)) = (p.int("userId"), p.string("lootId")) {
                world.fx.push(WorldFx::Emote {
                    entity: u as i32,
                    loot_id: loot.to_owned(),
                });
            }
        }
        id::ATTACK_LASER_RUN => {
            if let (Ok(a), Ok(t)) = (p.int("attackerId"), p.int("targetId")) {
                ::log::debug!(
                    "laser run {a} -> {t} color={} skilled={}",
                    p.int("laserColor").unwrap_or(0),
                    p.boolean("skilledLaser").unwrap_or(false)
                );
                world.fx.push(WorldFx::LaserRun {
                    attacker: a as i32,
                    victim: t as i32,
                    laser_type: p.int("laserColor").unwrap_or(0) as i32,
                    skilled: p.boolean("skilledLaser").unwrap_or(false),
                    shield_damage: p
                        .boolean("isDiminishedBySkillShield")
                        .unwrap_or(false),
                    shield_type: p.int("visualType").unwrap_or(0) as i32,
                });
            }
        }
        ATTACK_ROCKET => {
            if let (Ok(a), Ok(t)) = (p.int("attackerId"), p.int("targetId")) {
                ::log::debug!("rocket {a} -> {t}");
                world.fx.push(WorldFx::Rocket {
                    attacker: a as i32,
                    victim: t as i32,
                    rocket_type: p.int("rocketType").unwrap_or(0) as i32,
                    smoke_id: p.int("smokeId").unwrap_or(0) as i32,
                    hit: p.boolean("hit").unwrap_or(true),
                    heat_seeking: p.boolean("isHeatSeeking").unwrap_or(false),
                });
            }
        }
        ATTACK_MISSED => {
            if let Ok(t) = p.int("targetUserId") {
                ::log::debug!("miss on {t}");
                world.fx.push(WorldFx::Miss { victim: t as i32 });
            }
        }
        id::ATTACK_HIT | id::ATTACK_HIT_NO_LOCK => {
            if let (Ok(victim), Ok(nh)) = (p.int("victimId"), p.int("victimNanoHull")) {
                world.entity_nano.entry(victim as i32).or_default().0 = nh as i32;
            }
            if let Ok(victim) = p.int("victimId") {
                if Some(victim as i32) == world.world.hero.as_ref().map(|h| h.user_id) {
                    let sh = p.int("victimShield").ok().map(|v| v as i32);
                    let nh = p.int("victimNanoHull").ok().map(|v| v as i32);
                    if sh.is_some() || nh.is_some() {
                        ::log::debug!("hero hit shield={sh:?} nano={nh:?}");
                    }
                    if let Some(h) = &mut world.world.hero {
                        if let Some(v) = sh {
                            h.shield = v;
                        }
                        if let Some(v) = nh {
                            h.nano_hull = v;
                        }
                    }
                }
            }
            if let Ok(victim) = p.int("victimId") {
                ::log::debug!(
                    "hit victim={victim} dmg={} strike={}",
                    p.int("damage").unwrap_or(0),
                    p.boolean("strike").unwrap_or(false)
                );
                let attack_type = p
                    .module("attackType")
                    .ok()
                    .and_then(|m| m.int("attackTypeValue").ok())
                    .unwrap_or(0) as i32;
                world.fx.push(WorldFx::Hit {
                    victim: victim as i32,
                    attacker: p.int("attackerId").unwrap_or(0) as i32,
                    damage: p.int("damage").unwrap_or(0),
                    attack_type,
                    strike: p.boolean("strike").unwrap_or(false),
                });
            }
        }
        _ => {}
    }
}
