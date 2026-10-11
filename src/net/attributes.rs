//! Hero attribute side-channels the packet-client `WorldState` ignores:
//! active ship configuration (C1/C2), cargo ore breakdown, cargo capacity
//! updates and attribute boosters.
//!
//! * `AttributeConfigCountUpdateCommand(98)` / legacy `0|S|CFG|n` — the
//!   stock client stores both in `Settings.selectedConfiguration`
//!   (`assembleConfigurationCountUpdate`, `ConnectionManager` SELECT/CFG).
//! * `AttributeOreCountUpdateCommand(106)` — `oreCountList` of
//!   `OreCountModule{oreType.typeValue, count}`.
//! * `UpdateCargoSpaceCommand(-11155)` — new `cargoSpaceMax`.
//! * `AttributeBoosterUpdateCommand(101)` — boosted attribute values plus
//!   `BoosterDetailModule` rows (`lootId`, `secondsLeft` for player boosters).

use std::collections::BTreeMap;
use xenomit_packet_client::packet::{Packet, Value};

use crate::net::world::SharedWorld;

pub(crate) const ATTRIBUTE_CONFIG_COUNT_UPDATE: i16 = 98;
pub(crate) const ATTRIBUTE_BOOSTER_UPDATE: i16 = 101;
pub(crate) const ATTRIBUTE_ORE_COUNT_UPDATE: i16 = 106;
pub(crate) const UPDATE_CARGO_SPACE: i16 = -11155;

/// `OreTypeModule.ORETYPE_*` display names, index = `typeValue`.
const ORE_NAMES: &[&str] = &[
    "Prometium",
    "Endurium",
    "Terbium",
    "Xenomit",
    "Prometid",
    "Duranium",
    "Promerium",
    "Seprom",
    "Palladium",
    "Mucosum",
    "Boltrum",
    "Scrapium",
    "Prismatium",
    "Proxim",
    "Duothrin",
    "Trittothrin",
    "Quadrothrin",
    "Kyhalon",
    "Tetrathrin",
    "Bifenon",
    "Indoctrine Oil",
    "Hybrid Alloy",
    "Rinusk",
    "Blacklight Trace",
    "Mindfire Cerebrum",
    "Tarnom",
    "Opitite",
    "Chromin",
    "Osmium",
    "Candy Corn",
    "Axion",
    "Isochronate",
    "Chaos Fragment",
    "Charopos Shard",
    "Styxus Shard",
    "Byte Remnant",
    "Agate Splinter",
    "Champion Token",
    "Flower Token",
    "Golden Candy Corn",
    "Eternal Fragment",
];

/// Display name of an `OreTypeModule` value (`Ore #n` past the table).
pub fn ore_name(type_value: i32) -> String {
    usize::try_from(type_value)
        .ok()
        .and_then(|i| ORE_NAMES.get(i))
        .map(|s| (*s).to_owned())
        .unwrap_or_else(|| format!("Ore #{type_value}"))
}

/// One active booster (`BoosterDetailModule`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Booster {
    pub loot_id: String,
    /// Remaining lifetime for player boosters (`BoosterPlayerDetailModule`);
    /// `None` for equipment/shared/battle-station boosters.
    pub seconds_left: Option<i64>,
    /// When the packet arrived — `seconds_left` counts down from here.
    pub received: Option<std::time::Instant>,
}

impl Booster {
    /// Seconds still left, counting down since arrival.
    pub fn remaining(&self) -> Option<i64> {
        let left = self.seconds_left?;
        let elapsed = self
            .received
            .map(|t| t.elapsed().as_secs() as i64)
            .unwrap_or(0);
        Some((left - elapsed).max(0))
    }
}

/// Hero attribute state outside `HeroState`. Session scope — survives
/// map changes.
#[derive(Clone, Debug, Default)]
pub struct HeroExtras {
    /// Active ship configuration (1 = C1, 2 = C2); 0 until the server
    /// announced it.
    pub config: i32,
    /// `OreTypeModule.typeValue` → amount in the cargo hold.
    pub ores: BTreeMap<i32, f64>,
    /// `loot_id` → active booster.
    pub boosters: BTreeMap<String, Booster>,
    /// `BoosterUpdateModule` rows: attribute name → boosted value.
    pub boosted: BTreeMap<String, f32>,
}

impl HeroExtras {
    /// Ores with a non-zero amount, largest first.
    pub fn ore_rows(&self) -> Vec<(i32, f64)> {
        let mut rows: Vec<(i32, f64)> = self
            .ores
            .iter()
            .filter(|(_, c)| **c > 0.0)
            .map(|(t, c)| (*t, *c))
            .collect();
        rows.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        rows
    }

    /// Total ore units in the hold.
    pub fn ore_total(&self) -> f64 {
        self.ores.values().filter(|c| **c > 0.0).sum()
    }
}

/// Legacy `0|S|CFG|n` → `Some(n)`.
pub fn parse_config_select(msg: &str) -> Option<i32> {
    let mut it = msg.split('|');
    if it.next()? != "0" || it.next()? != "S" || it.next()? != "CFG" {
        return None;
    }
    it.next()?.trim().parse().ok()
}

pub fn handle(world: &mut SharedWorld, p: &Packet) {
    match p.id {
        ATTRIBUTE_CONFIG_COUNT_UPDATE => {
            if let Ok(n) = p.int("configCount") {
                world.extras.config = n as i32;
            }
        }
        ATTRIBUTE_ORE_COUNT_UPDATE => {
            for v in p.list("oreCountList").unwrap_or(&[]) {
                let Value::Module(m) = v else {
                    continue;
                };
                let Ok(ty) = m.module("oreType").and_then(|t| t.int("typeValue")) else {
                    continue;
                };
                let count = m.f64("count").unwrap_or(0.0);
                if count > 0.0 {
                    world.extras.ores.insert(ty as i32, count);
                } else {
                    world.extras.ores.remove(&(ty as i32));
                }
            }
        }
        UPDATE_CARGO_SPACE => {
            if let (Some(h), Ok(max)) = (&mut world.world.hero, p.int("cargoSpaceMax")) {
                h.cargo_space_max = max as i32;
            }
        }
        ATTRIBUTE_BOOSTER_UPDATE => apply_boosters(world, p),
        xenomit_packet_client::modules::id::LEGACY => {
            if let Some(n) = p.string("message").ok().and_then(parse_config_select) {
                world.extras.config = n;
            }
        }
        _ => {}
    }
}

fn apply_boosters(world: &mut SharedWorld, p: &Packet) {
    let now = std::time::Instant::now();
    for v in p.list("boostedAttributes").unwrap_or(&[]) {
        let Value::Module(m) = v else {
            continue;
        };
        let Ok(name) = m.string("attributeType") else {
            continue;
        };
        let value = match m.get("value") {
            Some(Value::F32(f)) => *f,
            Some(Value::F64(f)) => *f as f32,
            Some(Value::Int(i)) => *i as f32,
            _ => continue,
        };
        world.extras.boosted.insert(name.to_owned(), value);
    }
    for v in p.list("boostedDetailsUpdate").unwrap_or(&[]) {
        let Value::Module(m) = v else {
            continue;
        };
        let Ok(loot) = m.string("lootId") else {
            continue;
        };
        world.extras.boosters.insert(
            loot.to_owned(),
            Booster {
                loot_id: loot.to_owned(),
                seconds_left: m.int("secondsLeft").ok(),
                received: Some(now),
            },
        );
    }
    for v in p.list("boostedDetailsRemove").unwrap_or(&[]) {
        let Value::Module(m) = v else {
            continue;
        };
        if let Ok(loot) = m.string("lootId") {
            world.extras.boosters.remove(loot);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pkt(id: i16, fields: Vec<(&'static str, Value)>) -> Packet {
        Packet {
            id,
            name: None,
            fields: fields.into_iter().map(|(n, v)| (Some(n), v)).collect(),
        }
    }

    fn ore(ty: i64, count: f64) -> Value {
        Value::Module(Box::new(pkt(
            105,
            vec![
                (
                    "oreType",
                    Value::Module(Box::new(pkt(-24450, vec![("typeValue", Value::Int(ty))]))),
                ),
                ("count", Value::F64(count)),
            ],
        )))
    }

    #[test]
    fn config_from_packet_and_legacy() {
        let mut w = SharedWorld::default();
        w.apply_packet(&pkt(98, vec![("configCount", Value::Int(2))]));
        assert_eq!(w.extras.config, 2);
        assert_eq!(parse_config_select("0|S|CFG|1"), Some(1));
        assert_eq!(parse_config_select("0|S|LAS|1"), None);
        assert_eq!(parse_config_select("0|n|d|1"), None);
        let msg = pkt(
            xenomit_packet_client::modules::id::LEGACY,
            vec![("message", Value::Str("0|S|CFG|1".into()))],
        );
        w.apply_packet(&msg);
        assert_eq!(w.extras.config, 1);
    }

    #[test]
    fn ore_counts_accumulate_and_clear() {
        let mut w = SharedWorld::default();
        w.apply_packet(&pkt(
            106,
            vec![(
                "oreCountList",
                Value::List(vec![ore(0, 120.0), ore(2, 40.0), ore(1, 300.0)]),
            )],
        ));
        let rows = w.extras.ore_rows();
        assert_eq!(rows, vec![(1, 300.0), (0, 120.0), (2, 40.0)]);
        assert_eq!(w.extras.ore_total(), 460.0);
        // Zero count removes the row.
        w.apply_packet(&pkt(
            106,
            vec![("oreCountList", Value::List(vec![ore(1, 0.0)]))],
        ));
        assert_eq!(w.extras.ore_rows(), vec![(0, 120.0), (2, 40.0)]);
        assert_eq!(ore_name(0), "Prometium");
        assert_eq!(ore_name(999), "Ore #999");
    }

    #[test]
    fn boosters_update_and_remove() {
        let mut w = SharedWorld::default();
        let attr = Value::Module(Box::new(pkt(
            100,
            vec![
                ("attributeType", Value::Str("DAMAGE".into())),
                ("value", Value::F32(0.1)),
            ],
        )));
        let detail = |loot: &str, secs: i64| {
            Value::Module(Box::new(pkt(
                3792,
                vec![
                    ("lootId", Value::Str(loot.into())),
                    ("secondsLeft", Value::Int(secs)),
                ],
            )))
        };
        w.apply_packet(&pkt(
            101,
            vec![
                ("boostedAttributes", Value::List(vec![attr])),
                (
                    "boostedDetailsUpdate",
                    Value::List(vec![detail("booster_dmg_b01", 3600)]),
                ),
                ("boostedDetailsRemove", Value::List(vec![])),
            ],
        ));
        assert_eq!(w.extras.boosted.get("DAMAGE"), Some(&0.1));
        let b = &w.extras.boosters["booster_dmg_b01"];
        assert!(b.remaining().unwrap() > 3590);
        w.apply_packet(&pkt(
            101,
            vec![
                ("boostedAttributes", Value::List(vec![])),
                ("boostedDetailsUpdate", Value::List(vec![])),
                (
                    "boostedDetailsRemove",
                    Value::List(vec![detail("booster_dmg_b01", 0)]),
                ),
            ],
        ));
        assert!(w.extras.boosters.is_empty());
    }

    #[test]
    fn cargo_max_update_touches_hero() {
        let mut w = SharedWorld::default();
        w.world.hero = Some(Default::default());
        w.apply_packet(&pkt(-11155, vec![("cargoSpaceMax", Value::Int(1500))]));
        assert_eq!(w.world.hero.as_ref().unwrap().cargo_space_max, 1500);
    }
}
