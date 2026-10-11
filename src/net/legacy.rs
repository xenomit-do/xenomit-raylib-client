//! `LegacyModule` map-event dispatch — `MapEventsAssembly` in the stock
//! client. Messages are `|`-separated: `0|n|<CMD>|<args…>` (`param1[1]`
//! is `ServerCommands.MAP_EVENT` = `n`, `param1[2]` the sub-command).
//!
//! Visual events become [`WorldFx`] items (drained by `Game::update`);
//! state events (cloak, gray-out, ceasefire, status auras) mutate
//! `SharedWorld` directly so the renderer just reads them.

use xenomit_packet_client::modules::id;
use xenomit_packet_client::packet::Packet;

use crate::net::combat::{EmpKind, StatusFx, WorldFx};
use crate::net::drones::{apply_set_drones, parse_set_drones};
use crate::net::ships::{parse_remove_title, parse_set_title, set_title};
use crate::net::util::log_once;
use crate::net::world::SharedWorld;

/// A parsed `0|n|…` map event (only the ones with a visible/state effect).
#[derive(Clone, Debug, PartialEq)]
pub enum MapEvent {
    /// `SAB_SHOT|from|to`
    SabShot { from: i32, to: i32 },
    /// `HEAL_RAY|healer|target`
    HealRay { healer: i32, target: i32 },
    /// `MIN|hash`
    MineExplode { hash: String },
    /// `SMB|uid`
    Smartbomb { uid: i32 },
    /// `ISH|uid`
    InstaShield { uid: i32 },
    /// `EMP|id`, `BT|id`, `NNK|id`, `GOPM|id`
    Emp { id: i32, kind: EmpKind },
    /// `PMME|id`
    MegaMine { id: i32 },
    /// `Spawn|uid` / `Spawn|x|y` (and `Despawn…`)
    Spawn {
        ship: Option<i32>,
        pos: Option<(i32, i32)>,
        despawn: bool,
    },
    /// `FWI|hash` / `FWG|hash|hash…`
    FireworksIgnite { hashes: Vec<String> },
    /// `p|…` — `assembleSetPortal` (empty in the stock client).
    SetPortal { args: Vec<String> },
    /// `sss|a|b` — spaceball speed.
    SetSpeed { a: i32, b: i32 },
    /// `LSH|uid|byUid`
    FadeToGray { uid: i32, by: i32 },
    /// `USH|uid`
    FadeToGrayAbort { uid: i32 },
    /// `INV|uid|0/1`
    Invisible { uid: i32, on: bool },
    /// `pvp|uid|0/1` — `SET_PLAYER_ATTACKABLE`
    Attackable { uid: i32, flag: bool },
    /// `fx|start/end|NAME|uid`
    GraphicFx { start: bool, name: String, uid: i32 },
    /// `BOOOM|uid`
    SpecialExplosion { uid: i32 },
    /// `MAL|SET/REM|uid`
    Malus { set: bool, uid: i32 },
}

/// Parse one legacy map-event string. `None` for anything that is not a
/// handled `n` event (including drone/title messages — those have their
/// own parsers in `net::drones` / `net::ships`).
pub fn parse_map_event(msg: &str) -> Option<MapEvent> {
    let parts: Vec<&str> = msg.split('|').collect();
    if parts.len() < 3 || parts[1] != "n" {
        return None;
    }
    let int = |i: usize| -> Option<i32> { parts.get(i)?.trim().parse().ok() };
    let cmd = parts[2];
    Some(match cmd {
        "SAB_SHOT" => MapEvent::SabShot {
            from: int(3)?,
            to: int(4)?,
        },
        "HEAL_RAY" => MapEvent::HealRay {
            healer: int(3)?,
            target: int(4)?,
        },
        "MIN" => MapEvent::MineExplode {
            hash: parts.get(3)?.to_string(),
        },
        "SMB" => MapEvent::Smartbomb { uid: int(3)? },
        "ISH" => MapEvent::InstaShield { uid: int(3)? },
        "EMP" | "GOPM" => MapEvent::Emp {
            id: int(3)?,
            kind: EmpKind::Normal,
        },
        "BT" => MapEvent::Emp {
            id: int(3)?,
            kind: EmpKind::BurningTrail,
        },
        "NNK" => MapEvent::Emp {
            id: int(3)?,
            kind: EmpKind::Nuke,
        },
        "PMME" => MapEvent::MegaMine { id: int(3)? },
        "Spawn" | "Despawn" => {
            let despawn = cmd == "Despawn";
            match parts.len() {
                4 => MapEvent::Spawn {
                    ship: Some(int(3)?),
                    pos: None,
                    despawn,
                },
                5 => MapEvent::Spawn {
                    ship: None,
                    pos: Some((int(3)?, int(4)?)),
                    despawn,
                },
                _ => return None,
            }
        }
        "FWI" | "FWG" => MapEvent::FireworksIgnite {
            hashes: parts[3..]
                .iter()
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .collect(),
        },
        "p" => MapEvent::SetPortal {
            args: parts[3..].iter().map(|s| s.to_string()).collect(),
        },
        "sss" => MapEvent::SetSpeed {
            a: int(3)?,
            b: int(4)?,
        },
        "LSH" => MapEvent::FadeToGray {
            uid: int(3)?,
            by: int(4)?,
        },
        "USH" => MapEvent::FadeToGrayAbort { uid: int(3)? },
        "INV" => MapEvent::Invisible {
            uid: int(3)?,
            on: int(4)? != 0,
        },
        "pvp" => MapEvent::Attackable {
            uid: int(3)?,
            flag: int(4)? != 0,
        },
        "fx" => MapEvent::GraphicFx {
            start: match *parts.get(3)? {
                "start" => true,
                "end" => false,
                _ => return None,
            },
            name: parts.get(4)?.to_string(),
            uid: int(5)?,
        },
        "BOOOM" => MapEvent::SpecialExplosion { uid: int(3)? },
        "MAL" => MapEvent::Malus {
            set: match *parts.get(3)? {
                "SET" => true,
                "REM" => false,
                _ => return None,
            },
            uid: int(4)?,
        },
        _ => return None,
    })
}

/// Display name for a ship id (hero or map entity).
fn ship_name(world: &SharedWorld, uid: i32) -> String {
    if let Some(h) = world.world.hero.as_ref().filter(|h| h.user_id == uid) {
        return h.user_name.clone();
    }
    world
        .world
        .entities
        .get(&uid)
        .map(|e| e.name.clone())
        .unwrap_or_default()
}

/// A ship id we can place on the map right now.
fn known_ship(world: &SharedWorld, uid: i32) -> bool {
    world.world.hero.as_ref().is_some_and(|h| h.user_id == uid)
        || world.world.entities.contains_key(&uid)
}

/// Legacy `0|A|LUP|<level>|<ep>` (`SetAttributeAssembly.
/// assembleLevelUpdate`) — returns the new level.
fn parse_level_up(msg: &str) -> Option<i32> {
    let parts: Vec<&str> = msg.split('|').collect();
    if parts.len() < 4 || parts[1] != "A" || parts[2] != "LUP" {
        return None;
    }
    parts[3].trim().parse().ok()
}

/// Locale text for `key`, else `fallback` (an empty catalog resolves to
/// the key itself, which is useless in a log line).
fn text_or(world: &SharedWorld, key: &str, fallback: &str) -> String {
    world
        .locale
        .lookup(key)
        .filter(|t| !t.is_empty())
        .unwrap_or(fallback)
        .to_owned()
}

/// Fold one parsed event into the world.
pub fn apply_event(world: &mut SharedWorld, ev: MapEvent) {
    let hero_id = world.world.hero.as_ref().map(|h| h.user_id);
    match ev {
        MapEvent::SabShot { from, to } => world.fx.push(WorldFx::SabShot {
            attacker: from,
            victim: to,
        }),
        MapEvent::HealRay { healer, target } => world.fx.push(WorldFx::HealRay { healer, target }),
        MapEvent::MineExplode { hash } => {
            // Stock: `mine.bang(); map.removeEntityByHash(hash)`.
            if let Some(c) = world.world.collectables.remove(&hash) {
                world.fx.push(WorldFx::MineExplode {
                    pos: (c.x as f32, c.y as f32),
                });
            }
            world.silent_removals.insert(hash);
        }
        MapEvent::Smartbomb { uid } => {
            if known_ship(world, uid) {
                world.fx.push(WorldFx::Smartbomb { owner: uid });
            }
        }
        MapEvent::InstaShield { uid } => {
            if known_ship(world, uid) {
                world.fx.push(WorldFx::InstaShield { owner: uid });
            }
        }
        MapEvent::Emp { id, kind } => world.fx.push(WorldFx::Emp { target: id, kind }),
        MapEvent::MegaMine { id } => world.fx.push(WorldFx::MegaMine { target: id }),
        MapEvent::Spawn { ship, pos, despawn } => {
            let at = match (ship, pos) {
                (Some(uid), _) if known_ship(world, uid) => Some(world.pos(uid)),
                (_, Some((x, y))) => Some((x as f32, y as f32)),
                _ => None,
            };
            if let Some(pos) = at {
                world.fx.push(WorldFx::Spawn { pos, despawn });
            }
        }
        MapEvent::FireworksIgnite { hashes } => {
            for hash in hashes {
                if let Some(c) = world.world.collectables.remove(&hash) {
                    world.fx.push(WorldFx::FireworksIgnite {
                        pos: (c.x as f32, c.y as f32),
                        type_id: c.type_id,
                        modifier: c.event_modifier.clone(),
                        colour: c.colour_code,
                    });
                }
                world.silent_removals.insert(hash);
            }
        }
        MapEvent::SetPortal { args } => {
            // `assembleSetPortal` is empty in the stock client; portals
            // arrive via `JumpgateCreateCommand`. Keep a breadcrumb only.
            log_once("legacy-set-portal", format!("{args:?}"));
        }
        MapEvent::SetSpeed { a, b } => world.spaceball_speed = Some((a, b)),
        MapEvent::FadeToGray { uid, by } => {
            // Stock skips the hero's own lock and group members.
            let member = world
                .group
                .as_ref()
                .is_some_and(|g| g.member(uid).is_some());
            if Some(by) != hero_id && !member {
                world.gray_targets.insert(uid);
            }
        }
        MapEvent::FadeToGrayAbort { uid } => {
            world.gray_targets.remove(&uid);
        }
        MapEvent::Invisible { uid, on } => {
            if let Some(h) = world.world.hero.as_mut().filter(|h| h.user_id == uid) {
                h.cloaked = on;
            }
            if let Some(e) = world.world.entities.get_mut(&uid) {
                e.cloaked = on;
            }
        }
        MapEvent::Attackable { uid, flag } => {
            // Mirrors the stock branch literally: `true` logs the
            // "recovery of ceasefire" line, `false` the "loss" line and
            // marks the player (criminal marker).
            let nick = ship_name(world, uid);
            let line = if flag {
                world.criminals.remove(&uid);
                text_or(
                    world,
                    "msg_recovery_of_ceasefire_user",
                    "%nick% regained ceasefire",
                )
            } else {
                world.criminals.insert(uid);
                text_or(world, "msg_loss_of_ceasefire_user", "%nick% lost ceasefire")
            };
            world.push_log(line.replace("%nick%", &nick));
        }
        MapEvent::GraphicFx { start, name, uid } => {
            // `assembleGraphicFx`: the names map onto `EffectIDList`
            // clips (`RageEffect`, `SlowDownEffect`, `IcyCubeEffect`,
            // `SPEED_BUFF_EFFECT`, `SKULL`, `InvincibilityEffect`) —
            // they ride the shared `ShipFxState` like modifier effects.
            if let Some(fx) = StatusFx::from_name(&name) {
                world.fx.push(WorldFx::ShipFx {
                    entity: uid,
                    effect: fx.effect_id(),
                    activated: start,
                });
            }
        }
        MapEvent::SpecialExplosion { uid } => {
            if known_ship(world, uid) {
                world.fx.push(WorldFx::SpecialExplosion { ship: uid });
            }
        }
        MapEvent::Malus { set, uid } => {
            if Some(uid) == hero_id {
                let line = if set {
                    text_or(world, "msg_arms_harmed", "Your weapons are impaired")
                } else {
                    text_or(world, "msg_arms_recovered", "Your weapons have recovered")
                };
                world.push_log(line);
            }
        }
    }
}

pub fn handle(world: &mut SharedWorld, p: &Packet) {
    if p.id != id::LEGACY {
        return;
    }
    if let Ok(msg) = p.string("message") {
        if let Some((uid, _count, formation, drones)) = parse_set_drones(msg) {
            log_once("set-drones-msg", format!("raw: {msg}"));
            apply_set_drones(world, uid, formation, drones);
        } else if let Some((uid, key)) = parse_set_title(msg) {
            set_title(world, uid, &key);
        } else if let Some(uid) = parse_remove_title(msg) {
            set_title(world, uid, "");
        } else if let Some(_level) = parse_level_up(msg) {
            // `SetAttributeAssembly.assembleLevelUpdate`: hero-only
            // `LevelUpEffect` (EffectIDList.LEVEL_UP = 13).
            if let Some(uid) = world.world.hero.as_ref().map(|h| h.user_id) {
                world.fx.push(WorldFx::ShipFx {
                    entity: uid,
                    effect: 13,
                    activated: true,
                });
            }
        } else if let Some(ev) = parse_map_event(msg) {
            apply_event(world, ev);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_visual_events() {
        assert_eq!(
            parse_map_event("0|n|SAB_SHOT|10|20"),
            Some(MapEvent::SabShot { from: 10, to: 20 })
        );
        assert_eq!(
            parse_map_event("0|n|HEAL_RAY|1|2"),
            Some(MapEvent::HealRay {
                healer: 1,
                target: 2
            })
        );
        assert_eq!(
            parse_map_event("0|n|MIN|abc123"),
            Some(MapEvent::MineExplode {
                hash: "abc123".into()
            })
        );
        assert_eq!(
            parse_map_event("0|n|SMB|7"),
            Some(MapEvent::Smartbomb { uid: 7 })
        );
        assert_eq!(
            parse_map_event("0|n|ISH|7"),
            Some(MapEvent::InstaShield { uid: 7 })
        );
        assert_eq!(
            parse_map_event("0|n|NNK|9"),
            Some(MapEvent::Emp {
                id: 9,
                kind: EmpKind::Nuke
            })
        );
        assert_eq!(
            parse_map_event("0|n|BT|9"),
            Some(MapEvent::Emp {
                id: 9,
                kind: EmpKind::BurningTrail
            })
        );
        assert_eq!(
            parse_map_event("0|n|Despawn|500|600"),
            Some(MapEvent::Spawn {
                ship: None,
                pos: Some((500, 600)),
                despawn: true
            })
        );
        assert_eq!(
            parse_map_event("0|n|Spawn|55"),
            Some(MapEvent::Spawn {
                ship: Some(55),
                pos: None,
                despawn: false
            })
        );
        assert_eq!(
            parse_map_event("0|n|FWG|a|b|c"),
            Some(MapEvent::FireworksIgnite {
                hashes: vec!["a".into(), "b".into(), "c".into()]
            })
        );
        assert_eq!(
            parse_map_event("0|n|fx|start|RAGE|42"),
            Some(MapEvent::GraphicFx {
                start: true,
                name: "RAGE".into(),
                uid: 42
            })
        );
        assert_eq!(
            parse_map_event("0|n|MAL|SET|3"),
            Some(MapEvent::Malus { set: true, uid: 3 })
        );
        assert_eq!(
            parse_map_event("0|n|INV|3|1"),
            Some(MapEvent::Invisible { uid: 3, on: true })
        );
        assert_eq!(
            parse_map_event("0|n|pvp|3|0"),
            Some(MapEvent::Attackable {
                uid: 3,
                flag: false
            })
        );
        assert_eq!(
            parse_map_event("0|n|sss|4|5"),
            Some(MapEvent::SetSpeed { a: 4, b: 5 })
        );
    }

    #[test]
    fn ignores_foreign_messages() {
        assert_eq!(parse_map_event("0|y|CRE|1|2"), None);
        assert_eq!(parse_map_event("0|n|MSG|3|2|k|"), None);
        assert_eq!(parse_map_event("0|n|SAB_SHOT|x|y"), None);
        assert_eq!(parse_map_event("w"), None);
    }

    #[test]
    fn state_events_mutate_world() {
        let mut w = SharedWorld::default();
        apply_event(&mut w, MapEvent::FadeToGray { uid: 5, by: 1 });
        assert!(w.gray_targets.contains(&5));
        apply_event(&mut w, MapEvent::FadeToGrayAbort { uid: 5 });
        assert!(!w.gray_targets.contains(&5));

        apply_event(
            &mut w,
            MapEvent::GraphicFx {
                start: true,
                name: "SKULL".into(),
                uid: 8,
            },
        );
        assert!(matches!(
            w.fx.last(),
            Some(WorldFx::ShipFx {
                entity: 8,
                effect: 18,
                activated: true
            })
        ));
        apply_event(
            &mut w,
            MapEvent::GraphicFx {
                start: false,
                name: "SKULL".into(),
                uid: 8,
            },
        );
        assert!(matches!(
            w.fx.last(),
            Some(WorldFx::ShipFx {
                entity: 8,
                effect: 18,
                activated: false
            })
        ));

        apply_event(
            &mut w,
            MapEvent::Attackable {
                uid: 3,
                flag: false,
            },
        );
        assert!(w.criminals.contains(&3));
        assert_eq!(w.log.len(), 1);
        apply_event(&mut w, MapEvent::Attackable { uid: 3, flag: true });
        assert!(!w.criminals.contains(&3));

        apply_event(&mut w, MapEvent::SetSpeed { a: 1, b: 2 });
        assert_eq!(w.spaceball_speed, Some((1, 2)));
    }

    #[test]
    fn mine_without_collectable_is_silent() {
        let mut w = SharedWorld::default();
        apply_event(&mut w, MapEvent::MineExplode { hash: "zz".into() });
        assert!(w.fx.is_empty());
        assert!(w.silent_removals.contains("zz"));
    }
}
