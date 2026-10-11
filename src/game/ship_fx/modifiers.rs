//! `VisualModifierAssembly.as` port — translates one
//! `VisualModifierCommand` row into ship-effect actions.
//!
//! Stock runs every modifier through one giant switch (all map objects)
//! plus a ship-only second switch; each case adds/removes a model
//! `Effect` via `map.effects.addMapObjectEffectOnce` /
//! `removeEffectFromMapObject`. We mirror the cases whose resulting
//! effect has a clip-based 2D view (`EffectsManager2D` `MAP`); the rest
//! are listed in `NOT_PORTED` with a reason.

use super::catalog;
use crate::net::ships::ModifierRow;

/// Effect lifetime, mirroring `Effect.NOT_PERSISTANT` (the clip plays
/// once and the effect removes itself), `PERSISTANT` (lives until a
/// deactivation removes it) and `TIME_OUT` (auto-removes after `secs`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Life {
    /// Clip plays once, then the effect disposes itself.
    Once,
    /// Stays until a `Remove` action or entity cleanup drops it.
    Persistent,
    /// Auto-removes `secs` after activation.
    Timed(f32),
}

/// `GlowingRingEffect`/`MapObjectGlowRingEffect` parameters —
/// `start_d -> end_d` expanding ring, `color`, `cycle` seconds per
/// pass (0 = static ring), looped (persistent) or single pass.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ring {
    pub start_d: f32,
    pub end_d: f32,
    pub color: u32,
    pub cycle: f32,
    pub looped: bool,
}

/// One `addMapObjectEffectOnce` call. `a`/`b` carry the few
/// case-specific numbers (glow color, bomb stage, shield %, boss
/// level, ring ids are the effect ids themselves).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AddFx {
    /// `EffectIDList` id — catalog/render key.
    pub effect: i32,
    pub life: Life,
    /// Primary case arg (color, stage, attribute, …).
    pub a: i64,
    /// Secondary case arg.
    pub b: i64,
    /// Procedural ring parameters when the effect is a `*GlowRingEffect`.
    pub ring: Option<Ring>,
    /// Delay before the effect becomes visible (drive-area 0.305 s).
    pub delay: f32,
}

/// What a modifier row does to an entity's effect list.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FxAction {
    /// `addMapObjectEffectOnce(new XEffect(entity, …))`.
    Add(AddFx),
    /// `removeEffectFromMapObject(entity.id, EffectIDList.X)`.
    Remove(i32),
    /// `playEffect(new PyroEffect(x, y, z, pyroPattern))` — positional
    /// explosion clip (sticky-bomb deactivate).
    Pyro { class: i32, variant: i32 },
}

const fn add(effect: i32, life: Life) -> FxAction {
    FxAction::Add(AddFx {
        effect,
        life,
        a: 0,
        b: 0,
        ring: None,
        delay: 0.0,
    })
}

const fn add_a(effect: i32, life: Life, a: i64) -> FxAction {
    FxAction::Add(AddFx {
        effect,
        life,
        a,
        b: 0,
        ring: None,
        delay: 0.0,
    })
}

const fn ring_add(effect: i32, ring: Ring, life: Life) -> FxAction {
    FxAction::Add(AddFx {
        effect,
        life,
        a: 0,
        b: 0,
        ring: Some(ring),
        delay: 0.0,
    })
}

const fn rm(effect: i32) -> FxAction {
    FxAction::Remove(effect)
}

// `EffectIDList` ids used below (kept inline so the table reads like
// the AS3 case list); `pub` so `ShipFxState`/legacy routing can name
// the same ids.
pub mod fx {
    pub const SOLACE: i32 = 3;
    pub const DIMINISHER: i32 = 4;
    pub const SLOW_DOWN: i32 = 9;
    pub const ENERGY_LEECH: i32 = 11;
    pub const REP_BOT: i32 = 12;
    pub const LEVEL_UP: i32 = 13;
    pub const SPEED_BUFF: i32 = 14;
    pub const INVINCIBILITY: i32 = 16;
    pub const RAGE: i32 = 17;
    pub const SKULL: i32 = 18;
    pub const SPAWN: i32 = 19;
    pub const SPAWN_CHILDREN: i32 = 24;
    pub const HOME_BONUS: i32 = 29;
    pub const FORTIFY: i32 = 35;
    pub const PROTECTED: i32 = 38;
    pub const DRAW_FIRE_VICTIM: i32 = 39;
    pub const SHIP_WARP: i32 = 41;
    pub const DEFLECTOR: i32 = 45;
    pub const EMERGENCY_REPAIR: i32 = 46;
    pub const CONSTRUCTING: i32 = 47;
    pub const GHOST: i32 = 51;
    pub const BIG_PUMPKIN: i32 = 52;
    pub const MODULE_INSTALL: i32 = 54;
    pub const MIRRORED: i32 = 55;
    pub const STICKYBOMB: i32 = 56;
    pub const GLOWING_RING: i32 = 58;
    pub const DECLOAK: i32 = 64;
    pub const POLARITY_POWERUP: i32 = 67;
    pub const WARP_POWERUP: i32 = 68;
    pub const LORDAKIUM_SPAWN: i32 = 71;
    pub const HEAL: i32 = 72;
    /// `REP_BOT_EFFECT` — the `BeaconCommand` repair bot (not a
    /// `VisualModifierCommand` case; added via `ShipFxState::add_repair_bot`).
    pub const REP_BOT_EFFECT: i32 = 76;
    pub const ICY_CUBE: i32 = 79;
    pub const MARK_TARGET: i32 = 80;
    pub const INSTA_SHIELD: i32 = 84;
    pub const PLAGUE: i32 = 85;
    pub const DISRUPTION: i32 = 89;
    pub const TARTARUS_SPEEDBUFF: i32 = 92;
    pub const RAPID_FIRE: i32 = 93;
    pub const SCRAMBLE: i32 = 94;
    pub const PHASE_OUT: i32 = 95;
    pub const HOLOGRAM: i32 = 96;
    pub const SPHERE_SHIELD: i32 = 99;
    pub const MAYHEM_RAGE: i32 = 287;
    pub const MAYHEM_WINNER: i32 = 288;
    pub const WARGAME_GLOW: i32 = 289;
    pub const PVP_PROTECTION: i32 = 290;
    pub const REP_BOT2: i32 = 302;
    pub const CHARGING_BLOB: i32 = 305;
    pub const KS_CLOUD: i32 = 310;
    pub const KS_TRAIL: i32 = 312;
    pub const KS_ACTIVATION: i32 = 313;
    pub const REDIRECT: i32 = 320;
    pub const SHIELD_DISARRAY: i32 = 321;
    pub const DDOL: i32 = 322;
    pub const PARTICLE_BEAM: i32 = 303;
    pub const SHIELD_LINK: i32 = 323;
    pub const BERSERK: i32 = 324;
    pub const HP_LINK: i32 = 326;
    pub const VOLTAGE_LINK: i32 = 352;
    pub const BAY_SHIELD_LINK: i32 = 412;
    pub const REVENGE: i32 = 325;
    pub const INCINERATE: i32 = 327;
    pub const VALENTINE: i32 = 328;
    pub const SPREAD: i32 = 330;
    pub const MAPOBJECT_RING: i32 = 331;
    pub const CHARGE_SHOT: i32 = 332;
    pub const SPEED_BURST: i32 = 333;
    pub const ANTI_MINE: i32 = 334;
    pub const SHIP_GLOW: i32 = 335;
    pub const EM_RESERVES: i32 = 336;
    pub const ASSIMILATE: i32 = 337;
    pub const PROM_CLOUD: i32 = 338;
    pub const PROM_TRAIL: i32 = 339;
    pub const PROM_ACTIVATION: i32 = 340;
    pub const FROST_FIELD: i32 = 342;
    pub const REVERSAL_SELF: i32 = 343;
    pub const REVERSAL_ENEMY: i32 = 344;
    pub const PRISMATIC: i32 = 345;
    pub const STOCKPILE: i32 = 347;
    pub const HEAT_RELEASE: i32 = 348;
    pub const JAMX_DAMAGE: i32 = 349;
    pub const NEUTRALIZING: i32 = 351;
    pub const STUN: i32 = 353;
    pub const VOLT_DISCHARGE: i32 = 354;
    pub const VOLT_BACKUP: i32 = 355;
    pub const EMBRACING: i32 = 356;
    pub const LAST_STAND: i32 = 404;
    pub const RIPPER: i32 = 405;
    pub const BOSS_LASER_CHARGE: i32 = 406;
    pub const BOSS_EMP_SHOCK: i32 = 407;
    pub const BOSS_BAY_ACTIVATE: i32 = 408;
    pub const BOSS_BAY_EMP: i32 = 409;
    pub const BOSS_ESCAPE: i32 = 413;
    pub const BOSS_SHIELD: i32 = 414;
    pub const ESCAPE_ZONE: i32 = 416;
    pub const CANNON_CORROSIVE: i32 = 420;
    pub const CHSP_UP: i32 = 2001;
    pub const CHSP_W_UP: i32 = 3001;
    pub const SPCP_UP: i32 = 2003;
    pub const SPCP_W_UP: i32 = 3003;
    pub const DA_START_ENEMY: i32 = 1510;
    pub const DA_END_ENEMY: i32 = 1511;
    pub const DA_OUTER_ENEMY: i32 = 151;
    pub const DA_START_FRIENDLY: i32 = 1520;
    pub const DA_END_FRIENDLY: i32 = 1521;
    pub const DA_OUTER_FRIENDLY: i32 = 152;
    pub const ANCHOR_START: i32 = 1530;
    pub const ANCHOR_END: i32 = 1531;
    // `GRAVITATIONAL_ANCHOR_ACTIVATE` shares id 407 with BOSS_EMP_SHOCK.
    pub const ANCHOR_CONTINUED: i32 = 407;
}

/// `VisualModifierAssembly` — every command row runs through the
/// all-entities switch; ships additionally run the second switch.
/// `ship` = `mapObject is Ship`.
pub fn actions(row: &ModifierRow, ship: bool) -> Vec<FxAction> {
    let on = row.activated;
    let mut out = Vec::new();
    // `attribute` as the cases consume it.
    let attr = row.attribute as i64;
    // `additionalAttribute` parsed as int where the case does
    // `int(param1.additionalAttribute)`.
    let extra = row.additional.trim().parse::<i64>().unwrap_or(0);

    // --- all map objects --------------------------------------------------
    match row.modifier {
        68 => {
            // FX_SHIELD — `SphereShieldEffect`, `percentage =
            // attribute / 100 * 10`; re-activating updates the pct.
            if on {
                out.push(add_a(fx::SPHERE_SHIELD, Life::Persistent, attr / 10));
            } else {
                out.push(rm(fx::SPHERE_SHIELD));
            }
        }
        10 => out.push(if on {
            add(fx::PROTECTED, Life::Persistent)
        } else {
            rm(fx::PROTECTED)
        }),
        // DRAW_FIRE_TARGET — `DrawFireVictimEffect(39)` on the victim
        // (the `MouseCursorChangeEffect` part is a cursor swap, not
        // ported).
        12 => out.push(if on {
            add(fx::DRAW_FIRE_VICTIM, Life::Persistent)
        } else {
            rm(fx::DRAW_FIRE_VICTIM)
        }),
        22 => out.push(if on {
            add(fx::SHIP_WARP, Life::Persistent)
        } else {
            rm(fx::SHIP_WARP)
        }),
        34 => out.push(if on {
            add(fx::DEFLECTOR, Life::Persistent)
        } else {
            rm(fx::DEFLECTOR)
        }),
        32 => out.push(if on {
            add(fx::EMERGENCY_REPAIR, Life::Persistent)
        } else {
            rm(fx::EMERGENCY_REPAIR)
        }),
        37 => out.push(if on {
            add(fx::CONSTRUCTING, Life::Persistent)
        } else {
            rm(fx::CONSTRUCTING)
        }),
        36 => {
            // INSTALLING — modules get `ModuleInstallEffect`,
            // battlestations `ConstructingEffect`; we can't tell a
            // module apart here so both ids are toggled together.
            out.push(if on {
                add(fx::CONSTRUCTING, Life::Persistent)
            } else {
                rm(fx::MODULE_INSTALL)
            });
            if !on {
                out.push(rm(fx::CONSTRUCTING));
            }
        }
        29 | 30 => {
            // GREEN/RED_GLOW — always drop the previous pumpkin glow
            // first, then add the new colour.
            out.push(rm(fx::BIG_PUMPKIN));
            if on {
                out.push(add_a(
                    fx::BIG_PUMPKIN,
                    Life::Persistent,
                    if row.modifier == 29 { 30464 } else { 16755200 },
                ));
            }
        }
        54 | 53 => {
            // ICY_ROCKET_TARGET_BLUE/RED — `MarkTargetEffect` variant.
            out.push(rm(fx::MARK_TARGET));
            if on {
                out.push(add_a(
                    fx::MARK_TARGET,
                    Life::Persistent,
                    if row.modifier == 54 { 0 } else { 1 },
                ));
            }
        }
        27 => out.push(if on {
            add(fx::MIRRORED, Life::Persistent)
        } else {
            rm(fx::MIRRORED)
        }),
        41 => out.push(if on {
            add(fx::DECLOAK, Life::Persistent)
        } else {
            rm(fx::DECLOAK)
        }),
        45 => out.push(if on {
            add(fx::POLARITY_POWERUP, Life::Once)
        } else {
            rm(fx::POLARITY_POWERUP)
        }),
        28 | 47 => {
            // STICKY_BOMB / _EMPEROR — stage arg = NORMAL/EMPEROR.
            if on {
                out.push(add_a(
                    fx::STICKYBOMB,
                    Life::Persistent,
                    if row.modifier == 28 { 0 } else { 1 },
                ));
            } else {
                out.push(rm(fx::STICKYBOMB));
                // `attribute == HALLOWEEN_STICKYBOMB_SHOW_EXPLOSION (0)`
                // plays `pyroEffects[0][0]` at the ship.
                if attr == 0 {
                    out.push(FxAction::Pyro {
                        class: 0,
                        variant: 0,
                    });
                }
            }
        }
        48 => out.push(if on {
            add(fx::LORDAKIUM_SPAWN, Life::Once)
        } else {
            rm(fx::LORDAKIUM_SPAWN)
        }),
        49 => out.push(if on {
            add(fx::HEAL, Life::Persistent)
        } else {
            rm(fx::HEAL)
        }),
        46 => out.push(if on {
            add(fx::WARP_POWERUP, Life::Persistent)
        } else {
            rm(fx::WARP_POWERUP)
        }),
        63 => out.push(if on {
            add(fx::DISRUPTION, Life::Persistent)
        } else {
            rm(fx::DISRUPTION)
        }),
        24 => out.push(if on {
            add(fx::HOME_BONUS, Life::Persistent)
        } else {
            rm(fx::HOME_BONUS)
        }),
        124 => {
            // NOXIOUS_NEBULA — pulsing ring `attr-300 -> attr`.
            if on {
                out.push(ring_add(
                    fx::MAPOBJECT_RING,
                    Ring {
                        start_d: (attr - 300) as f32,
                        end_d: attr as f32,
                        color: 8267696,
                        cycle: 1.0,
                        looped: true,
                    },
                    Life::Persistent,
                ));
            } else {
                out.push(rm(fx::MAPOBJECT_RING));
            }
        }
        _ => {}
    }
    if !ship {
        return out;
    }

    // --- ships only -------------------------------------------------------
    match row.modifier {
        8 => out.push(if on {
            add(fx::FORTIFY, Life::Persistent)
        } else {
            rm(fx::FORTIFY)
        }),
        26 => out.push(if on {
            add(fx::GHOST, Life::Persistent)
        } else {
            rm(fx::GHOST)
        }),
        57 => out.push(if on {
            add_a(fx::PLAGUE, Life::Persistent, extra)
        } else {
            rm(fx::PLAGUE)
        }),
        60 => out.push(if on {
            add(fx::REP_BOT, Life::Persistent)
        } else {
            rm(fx::REP_BOT)
        }),
        61 => out.push(if on {
            add(fx::REP_BOT2, Life::Persistent)
        } else {
            rm(fx::REP_BOT2)
        }),
        59 => out.push(if on {
            add(fx::ENERGY_LEECH, Life::Persistent)
        } else {
            rm(fx::ENERGY_LEECH)
        }),
        0 => out.push(if on {
            add_a(fx::TARTARUS_SPEEDBUFF, Life::Persistent, attr)
        } else {
            rm(fx::TARTARUS_SPEEDBUFF)
        }),
        1 => out.push(if on {
            add(fx::RAPID_FIRE, Life::Persistent)
        } else {
            rm(fx::RAPID_FIRE)
        }),
        2 => out.push(if on {
            add(fx::HOLOGRAM, Life::Persistent)
        } else {
            rm(fx::HOLOGRAM)
        }),
        3 => out.push(if on {
            add(fx::SCRAMBLE, Life::Persistent)
        } else {
            rm(fx::SCRAMBLE)
        }),
        4 => out.push(if on {
            add(fx::PHASE_OUT, Life::Persistent)
        } else {
            rm(fx::PHASE_OUT)
        }),
        70 => out.push(if on {
            add(fx::MAYHEM_RAGE, Life::Persistent)
        } else {
            rm(fx::MAYHEM_RAGE)
        }),
        69 => out.push(if on {
            add(fx::MAYHEM_WINNER, Life::Persistent)
        } else {
            rm(fx::MAYHEM_WINNER)
        }),
        76 | 77 => {
            // Mercenary wargame glow — red for modifier 76.
            if on {
                out.push(add_a(
                    fx::WARGAME_GLOW,
                    Life::Persistent,
                    i64::from(row.modifier == 76),
                ));
            } else {
                out.push(rm(fx::WARGAME_GLOW));
            }
        }
        79 => out.push(if on {
            add(fx::PVP_PROTECTION, Life::Persistent)
        } else {
            rm(fx::PVP_PROTECTION)
        }),
        78 | 101 => {
            // Kill-streak triple: looping cloud + trail, one-shot
            // activation flash (prometheus variants for 101).
            let (cloud, trail, activation) = if row.modifier == 78 {
                (fx::KS_CLOUD, fx::KS_TRAIL, fx::KS_ACTIVATION)
            } else {
                (fx::PROM_CLOUD, fx::PROM_TRAIL, fx::PROM_ACTIVATION)
            };
            if on {
                out.push(add(cloud, Life::Persistent));
                out.push(add(trail, Life::Persistent));
                out.push(add(activation, Life::Once));
            } else {
                out.push(rm(cloud));
                out.push(rm(trail));
                out.push(rm(activation));
            }
        }
        81 => out.push(if on {
            add(fx::REDIRECT, Life::Persistent)
        } else {
            rm(fx::REDIRECT)
        }),
        82 => out.push(if on {
            add(fx::DDOL, Life::Persistent)
        } else {
            rm(fx::DDOL)
        }),
        80 => out.push(if on {
            add(fx::SHIELD_DISARRAY, Life::Persistent)
        } else {
            rm(fx::SHIELD_DISARRAY)
        }),
        // ChainBoltEffect2D bolts — `attribute` is the chain target id.
        74 => out.push(if on {
            add_a(fx::PARTICLE_BEAM, Life::Persistent, attr)
        } else {
            rm(fx::PARTICLE_BEAM)
        }),
        83 => out.push(if on {
            add_a(fx::SHIELD_LINK, Life::Persistent, attr)
        } else {
            rm(fx::SHIELD_LINK)
        }),
        86 => out.push(if on {
            add_a(fx::HP_LINK, Life::Persistent, attr)
        } else {
            rm(fx::HP_LINK)
        }),
        // `NOT_PERSISTANT` — the bolt fades out at `boltFadeTime` 3 s.
        111 => out.push(if on {
            add_a(fx::VOLTAGE_LINK, Life::Timed(3.3), attr)
        } else {
            rm(fx::VOLTAGE_LINK)
        }),
        140 => out.push(if on {
            add_a(fx::BAY_SHIELD_LINK, Life::Persistent, attr)
        } else {
            rm(fx::BAY_SHIELD_LINK)
        }),
        84 => out.push(if on {
            add(fx::BERSERK, Life::Persistent)
        } else {
            rm(fx::BERSERK)
        }),
        85 => out.push(if on {
            add(fx::REVENGE, Life::Persistent)
        } else {
            rm(fx::REVENGE)
        }),
        87 => out.push(if on {
            add(fx::INCINERATE, Life::Persistent)
        } else {
            rm(fx::INCINERATE)
        }),
        88 => out.push(if on {
            add(fx::VALENTINE, Life::Persistent)
        } else {
            rm(fx::VALENTINE)
        }),
        89 => out.push(if on {
            add(fx::SKULL, Life::Persistent)
        } else {
            rm(fx::SKULL)
        }),
        91 => {
            // HEALING_GLOW — positional one-shot ring (deduped
            // globally by `addEffectOnce`, like stock).
            if on {
                out.push(ring_add(
                    fx::GLOWING_RING,
                    Ring {
                        start_d: 80.0,
                        end_d: 800.0,
                        color: 65280,
                        cycle: 1.5,
                        looped: false,
                    },
                    Life::Once,
                ));
            }
        }
        93 => {
            // SPREAD — hull-fit clip + pink ring 80->800.
            if on {
                out.push(add_a(fx::SPREAD, Life::Persistent, extra));
                out.push(ring_add(
                    fx::MAPOBJECT_RING,
                    Ring {
                        start_d: 80.0,
                        end_d: 800.0,
                        color: 16711935,
                        cycle: 1.5,
                        looped: true,
                    },
                    Life::Persistent,
                ));
            } else {
                out.push(rm(fx::SPREAD));
                out.push(rm(fx::MAPOBJECT_RING));
            }
        }
        94 => out.push(if on {
            add(fx::CHARGE_SHOT, Life::Persistent)
        } else {
            rm(fx::CHARGE_SHOT)
        }),
        95 => out.push(if on {
            add(fx::SPEED_BURST, Life::Persistent)
        } else {
            rm(fx::SPEED_BURST)
        }),
        96 => out.push(if on {
            add(fx::ANTI_MINE, Life::Persistent)
        } else {
            rm(fx::ANTI_MINE)
        }),
        97 | 98 => out.push(if on {
            add_a(
                fx::SHIP_GLOW,
                Life::Persistent,
                if row.modifier == 97 { 16711680 } else { 65280 },
            )
        } else {
            rm(fx::SHIP_GLOW)
        }),
        99 => out.push(if on {
            add(fx::ASSIMILATE, Life::Persistent)
        } else {
            rm(fx::ASSIMILATE)
        }),
        100 => out.push(if on {
            add(fx::EM_RESERVES, Life::Persistent)
        } else {
            rm(fx::EM_RESERVES)
        }),
        102 => out.push(if on {
            add(fx::FROST_FIELD, Life::Persistent)
        } else {
            rm(fx::FROST_FIELD)
        }),
        103 => out.push(if on {
            add(fx::REVERSAL_SELF, Life::Persistent)
        } else {
            rm(fx::REVERSAL_SELF)
        }),
        104 => out.push(if on {
            add(fx::REVERSAL_ENEMY, Life::Persistent)
        } else {
            rm(fx::REVERSAL_ENEMY)
        }),
        105 => out.push(if on {
            add(fx::PRISMATIC, Life::Persistent)
        } else {
            rm(fx::PRISMATIC)
        }),
        106 => out.push(if on {
            add(fx::STOCKPILE, Life::Persistent)
        } else {
            rm(fx::STOCKPILE)
        }),
        107 => out.push(if on {
            add_a(fx::HEAT_RELEASE, Life::Persistent, attr)
        } else {
            rm(fx::HEAT_RELEASE)
        }),
        108 => out.push(if on {
            add(fx::JAMX_DAMAGE, Life::Persistent)
        } else {
            rm(fx::JAMX_DAMAGE)
        }),
        110 => out.push(if on {
            add(fx::NEUTRALIZING, Life::Persistent)
        } else {
            rm(fx::NEUTRALIZING)
        }),
        112 => out.push(if on {
            add(fx::STUN, Life::Persistent)
        } else {
            rm(fx::STUN)
        }),
        113 => out.push(if on {
            add(fx::VOLT_DISCHARGE, Life::Persistent)
        } else {
            rm(fx::VOLT_DISCHARGE)
        }),
        114 => out.push(if on {
            add(fx::VOLT_BACKUP, Life::Persistent)
        } else {
            rm(fx::VOLT_BACKUP)
        }),
        115 => out.push(if on {
            add(fx::EMBRACING, Life::Persistent)
        } else {
            rm(fx::EMBRACING)
        }),
        117 => {
            // PAYLOAD_PROTECTION_ZONE — static ring at `attribute`
            // diameter (`cycle 0` = no expansion animation).
            if on {
                out.push(ring_add(
                    fx::MAPOBJECT_RING,
                    Ring {
                        start_d: attr as f32,
                        end_d: attr as f32,
                        color: 13759487,
                        cycle: 0.0,
                        looped: true,
                    },
                    Life::Persistent,
                ));
            } else {
                out.push(rm(fx::MAPOBJECT_RING));
            }
        }
        118 => {
            // FLUX_CRATE — HUD icon is not ported; the
            // `SpeedBuffEffect` part is.
            out.push(if on {
                add(fx::SPEED_BUFF, Life::Persistent)
            } else {
                rm(fx::SPEED_BUFF)
            });
        }
        119 => {
            if on {
                out.push(ring_add(
                    fx::MAPOBJECT_RING,
                    Ring {
                        start_d: 80.0,
                        end_d: attr as f32,
                        color: 15165749,
                        cycle: 1.5,
                        looped: false,
                    },
                    Life::Once,
                ));
            }
        }
        120 => out.push(if on {
            add(fx::DIMINISHER, Life::Persistent)
        } else {
            rm(fx::DIMINISHER)
        }),
        121 => {
            // WORLDBOSS_DAMAGING_ZONE — static red ring (`cycle 0`).
            if on {
                out.push(ring_add(
                    fx::MAPOBJECT_RING,
                    Ring {
                        start_d: attr as f32,
                        end_d: attr as f32,
                        color: 16711680,
                        cycle: 0.0,
                        looped: true,
                    },
                    Life::Persistent,
                ));
            } else {
                out.push(rm(fx::MAPOBJECT_RING));
            }
        }
        131 => out.push(if on {
            // `RipperEffect` — TIME_OUT 3000 ms.
            add(fx::RIPPER, Life::Timed(3.0))
        } else {
            rm(fx::RIPPER)
        }),
        132 => out.push(if on {
            add(fx::LAST_STAND, Life::Once)
        } else {
            rm(fx::LAST_STAND)
        }),
        133 => out.push(if on {
            add(fx::BOSS_ESCAPE, Life::Once)
        } else {
            rm(fx::BOSS_ESCAPE)
        }),
        135 => out.push(if on {
            add(fx::BOSS_LASER_CHARGE, Life::Once)
        } else {
            rm(fx::BOSS_LASER_CHARGE)
        }),
        137 => out.push(if on {
            add_a(fx::BOSS_EMP_SHOCK, Life::Once, attr)
        } else {
            rm(fx::BOSS_EMP_SHOCK)
        }),
        138 => out.push(if on {
            add(fx::BOSS_BAY_ACTIVATE, Life::Persistent)
        } else {
            rm(fx::BOSS_BAY_ACTIVATE)
        }),
        139 => out.push(if on {
            add(fx::BOSS_BAY_EMP, Life::Persistent)
        } else {
            rm(fx::BOSS_BAY_EMP)
        }),
        141 => out.push(if on {
            add_a(fx::BOSS_SHIELD, Life::Persistent, attr)
        } else {
            rm(fx::BOSS_SHIELD)
        }),
        142 => out.push(if on {
            add(fx::ESCAPE_ZONE, Life::Persistent)
        } else {
            rm(fx::ESCAPE_ZONE)
        }),
        145 => out.push(if on {
            add(fx::RAGE, Life::Persistent)
        } else {
            rm(fx::RAGE)
        }),
        146 => out.push(if on {
            add(fx::CANNON_CORROSIVE, Life::Persistent)
        } else {
            rm(fx::CANNON_CORROSIVE)
        }),
        147 => out.push(if on {
            add(fx::CHSP_UP, Life::Persistent)
        } else {
            rm(fx::CHSP_UP)
        }),
        149 => out.push(if on {
            add(fx::CHSP_W_UP, Life::Persistent)
        } else {
            rm(fx::CHSP_W_UP)
        }),
        148 => out.push(if on {
            add(fx::SPCP_UP, Life::Persistent)
        } else {
            rm(fx::SPCP_UP)
        }),
        150 => out.push(if on {
            add(fx::SPCP_W_UP, Life::Persistent)
        } else {
            rm(fx::SPCP_W_UP)
        }),
        73 => out.push(if on {
            add(fx::CHARGING_BLOB, Life::Persistent)
        } else {
            rm(fx::CHARGING_BLOB)
        }),
        151..=157 => {
            // Drive-area effects: activation plays the `*_start` clip
            // once, then the `*_continued` clip 0.305 s later; deacti-
            // vation removes the area and plays `*_end` once.
            let (start, outer, end) = match row.modifier {
                151 | 153 | 155 => (fx::DA_START_ENEMY, fx::DA_OUTER_ENEMY, fx::DA_END_ENEMY),
                152 | 154 | 156 => (
                    fx::DA_START_FRIENDLY,
                    fx::DA_OUTER_FRIENDLY,
                    fx::DA_END_FRIENDLY,
                ),
                _ => (fx::ANCHOR_START, fx::ANCHOR_CONTINUED, fx::ANCHOR_END),
            };
            if on {
                out.push(rm(end));
                out.push(add(start, Life::Once));
                out.push(FxAction::Add(AddFx {
                    effect: outer,
                    life: Life::Persistent,
                    a: 0,
                    b: 0,
                    ring: None,
                    delay: 0.305,
                }));
            } else {
                out.push(rm(outer));
                out.push(add(end, Life::Once));
            }
        }
        _ => {}
    }
    out
}

/// Effect ids a catalog lookup must exist for — used by tests to keep
/// the two tables in sync.
pub fn ported_ids() -> Vec<i32> {
    catalog::CATALOG.iter().map(|d| d.id).collect()
}

// not ported (VisualModifierCommand -> reason):
//   5 MIMESIS_HAS_SET          — no case in the assembly (set trait)
//   6 TRAVEL_MODE              — no case (3D/model only)
//   7 HEALING_POD              — PodCarrierTrait; pod overlay (W2)
//   9 PROTECT_OWNER            — no case (owner-side flag)
//   11 DRAW_FIRE_OWNER         — no case (owner-side flag)
//   13 ULTIMATE_EMP_TARGET     — empty case
//   14 INACTIVE                — deactivates the pod entity, model only
//   15 FORTRESS                — no case
//   16 PRISMATIC_SHIELD        — no case
//   17 PRISMATIC_REFLECTING    — no case
//   18 WEAKEN_SHIELDS          — no case
//   19 WEAKEN_SHIELDS_TARGET   — no case
//   20 SINGULARITY             — no case
//   21 SINGULARITY_TARGET      — no case
//   23 NPC_INFILTRATOR         — effect 42 has no 2D view
//   25 CHANGE_SHIP_DESIGN      — design override (existing net path)
//   31 GENERIC_GLOW            — effect 63 has no 2D view
//   33 INVINCIBILITY           — no case (legacy `fx|start|INVS` covers it)
//   35 DOWNTIME_TIMER          — TimerEffect: countdown UI, no 2D view
//   38 OWNS_BATTLESTATION      — no case
//   39 BUFFZONE                — no case
//   40 BLOCKED_ZONE_EXPLOSION  — no case
//   42 LEGENDARY_NPC_NAME      — `legendary` name flag, not a clip
//   43 POLARITY_POSITIVE       — effect 65 has no 2D view
//   44 POLARITY_NEGATIVE       — effect 66 has no 2D view
//   50 EMPEROR_SIBELON_BUFF    — effect 73 has no 2D view
//   51 EMPEROR_SIBELON_DEBUFF  — no case
//   52 CAMERA_DRONES           — model camera drones (effect 78 no view)
//   55 SHIP_SPAWN              — effect 81 has no 2D view
//   56 SHIP_DESPAWN            — effect 82 has no 2D view
//   58 ARENA_CHAMPION_ICON     — effect 88 has no 2D view
//   62 FLASHLIGHT              — map light, not a clip
//   64 CAMERA_SHAKE            — camera shake, not a clip
//   65 OVERRIDE_SHIP_MODEL     — asset-id extension, model only
//   66 DOCKING                 — effect 97 has no 2D view
//   67 LASERS_CHARGED          — ScatterLaser volley (effect 98 no view)
//   71 COUNTDOWN               — TimerEffect countdown UI
//   72 TIMER_COUNTDOWN         — TimerCountdownEffect countdown UI
//   75 INVADER                 — trait only
//   90 COLLECTOR_BEAM          — beam (existing CollectorBeam path)
//   92 PULSING_HEALING_GLOW    — PulsingRingTrait, not a clip
//   94 (laser part)            — `LaserEffect` beam alongside the clip
//   109 JAMX_REVERSAL_IMMUNITY — effect 350 has no 2D view
//   116 PULSING_RED_GLOW       — trait only
//   118 (HUD icon)             — CrateHUDEffect (speed-buff part ported)
//   120 (HUD icon)             — CrateHUDEffect (diminisher part ported)
//   122 WB_MISSILE_TELEGRAPH   — coordinate emitter markers
//   123 TITLE_BUFF             — GenericHUDIcon, HUD only
//   125 HEIGHTENED_VALOUR      — CrateHUDEffect, HUD only
//   126 CONTINUOUS_LASER       — beam
//   127 SNIPED_LASER           — procedural laser
//   128 ROCKET_ATK_MINIONS     — rocket barrage
//   129 M_ROCKET               — no case
//   130 M_LASER                — beam
//   134 BOSS_LASER_PREPARATION — laser warning beam
//   136 BOSS_LASER_SHOT        — procedural lightning
//   143 SPOT_LIGHT             — effect 415 has no 2D view
//   144 SHOW_NUMBER            — no case

#[cfg(test)]
mod tests {
    use super::*;

    fn row(modifier: i32, activated: bool) -> ModifierRow {
        ModifierRow {
            modifier,
            attribute: 0,
            additional: String::new(),
            activated,
        }
    }

    fn added(v: &[FxAction]) -> Vec<AddFx> {
        v.iter()
            .filter_map(|a| match a {
                FxAction::Add(f) => Some(*f),
                _ => None,
            })
            .collect()
    }

    fn removed(v: &[FxAction]) -> Vec<i32> {
        v.iter()
            .filter_map(|a| match a {
                FxAction::Remove(id) => Some(*id),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn persistent_toggle_adds_and_removes() {
        let a = actions(&row(1, true), true);
        assert_eq!(added(&a)[0].effect, fx::RAPID_FIRE);
        let a = actions(&row(1, false), true);
        assert_eq!(removed(&a), vec![fx::RAPID_FIRE]);
    }

    #[test]
    fn one_shot_lifetime() {
        // SpawnEffectWithFadeIn — `Effect.NOT_PERSISTANT`.
        let a = actions(&row(48, true), true);
        assert_eq!(added(&a)[0].life, Life::Once);
    }

    #[test]
    fn timed_ripper() {
        let a = actions(&row(131, true), true);
        assert_eq!(added(&a)[0].life, Life::Timed(3.0));
    }

    #[test]
    fn green_glow_drops_previous_then_adds_color() {
        let a = actions(&row(29, true), true);
        assert_eq!(removed(&a), vec![fx::BIG_PUMPKIN]);
        assert_eq!(added(&a)[0].a, 30464);
    }

    #[test]
    fn kill_streak_adds_three_effects() {
        let a = actions(&row(78, true), true);
        let ids: Vec<i32> = added(&a).iter().map(|f| f.effect).collect();
        assert_eq!(ids, vec![fx::KS_CLOUD, fx::KS_TRAIL, fx::KS_ACTIVATION]);
        let off = actions(&row(78, false), true);
        assert_eq!(removed(&off).len(), 3);
    }

    #[test]
    fn spread_adds_clip_and_ring() {
        let mut r = row(93, true);
        r.additional = "3".to_owned();
        let a = actions(&r, true);
        let adds = added(&a);
        assert_eq!(adds.len(), 2);
        assert_eq!(adds[1].ring.unwrap().end_d, 800.0);
    }

    #[test]
    fn ship_only_cases_ignored_for_assets() {
        assert!(added(&actions(&row(89, true), false)).is_empty());
        // All-entity cases still apply to assets.
        assert!(!added(&actions(&row(34, true), false)).is_empty());
    }

    #[test]
    fn drive_area_plays_start_then_delayed_area() {
        let a = actions(&row(151, true), true);
        let adds = added(&a);
        assert_eq!(adds[0].effect, fx::DA_START_ENEMY);
        assert_eq!(adds[1].effect, fx::DA_OUTER_ENEMY);
        assert!((adds[1].delay - 0.305).abs() < 1e-3);
        let off = actions(&row(151, false), true);
        assert_eq!(removed(&off), vec![fx::DA_OUTER_ENEMY]);
    }

    #[test]
    fn draw_fire_target_adds_victim_vortex() {
        let a = actions(&row(12, true), true);
        assert_eq!(added(&a)[0].effect, fx::DRAW_FIRE_VICTIM);
        let off = actions(&row(12, false), true);
        assert_eq!(removed(&off), vec![fx::DRAW_FIRE_VICTIM]);
    }
}
