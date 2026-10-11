//! Refinery/lab window state — the "Refining"/"Upgrade" window
//! (`RefinementManager` in the stock client).
//!
//! Inbound: `LabUpdateItemCommand` (135) carries `itemsUpdatedInfo` — for
//! each lab target item the applied ore type + unit count. Outbound
//! packets are `NetCommand` variants in `commands.rs`.

use xenomit_packet_client::modules::id;
use xenomit_packet_client::packet::{Packet, Value};

use crate::net::world::SharedWorld;

/// Lab target item ids (`LabItemModule.itemValue` on the wire), in
/// `<update><items>` document order — `RefinementManager.getItemTypeUint`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LabItemKind {
    Laser,
    Rocket,
    Driving,
    Shield,
}

impl LabItemKind {
    /// `identifier` attr in `refinementWindow/update/items/item`
    /// (LASER/ROCKET/DRIVING/SHIELD) — `None` for unknown ids.
    pub fn of_identifier(identifier: &str) -> Option<Self> {
        Some(match identifier {
            "LASER" => Self::Laser,
            "ROCKET" => Self::Rocket,
            "DRIVING" => Self::Driving,
            "SHIELD" => Self::Shield,
            _ => return None,
        })
    }

    /// `LabItemModule.itemValue` sent in `LabUpdateItemRequest`.
    pub fn item_value(self) -> i16 {
        match self {
            Self::Laser => 0,
            Self::Rocket => 1,
            Self::Driving => 2,
            Self::Shield => 3,
        }
    }

    /// Index into `LabState::items` (same order as the wire values).
    pub fn index(self) -> usize {
        self.item_value() as usize
    }
}

/// One lab target item's applied-ore state (`itemsUpdatedInfo` row).
#[derive(Clone, Copy, Debug, Default)]
pub struct LabItem {
    /// Applied ore type (`oreType.typeValue`); `None` when empty.
    pub ore_type: Option<i32>,
    /// Ore units already applied (`count`).
    pub count: f64,
}

/// Maximum upgrade amount per item (`TargetRefinementModule.MAX_VALUE`).
pub const LAB_ITEM_MAX: f64 = 1_000_000.0;

/// Fixed combo-box amounts offered by the count dialogs
/// (`RefinementManager.fillComboBox`): every listed value below the max,
/// then the max itself appended.
pub const COMBO_AMOUNTS: [u64; 7] = [1, 5, 10, 50, 100, 500, 1000];

/// Session-scope lab state (server re-sends `LabUpdateItemCommand` on
/// every `LabUpdateRequest` while the window is open).
#[derive(Clone, Debug, Default)]
pub struct LabState {
    /// Applied ore per target item, indexed by `LabItemKind::index`.
    pub items: [LabItem; 4],
    /// At least one `LabUpdateItemCommand` has been applied.
    pub loaded: bool,
}

/// `RefinementManager.isValid` — which source ores can upgrade which
/// lab item. `ore`/`target_ore` are `typeValue`s; `applied` is the
/// amount already on the item.
pub fn valid_upgrade(ore: i32, item: LabItemKind, target_ore: Option<i32>, applied: f64) -> bool {
    const PROMETID: i32 = 4;
    const DURANIUM: i32 = 5;
    const PROMERIUM: i32 = 6;
    const SEPROM: i32 = 7;
    const OSMIUM: i32 = 28;
    use LabItemKind::*;
    let kind_ok = match ore {
        PROMETID => matches!(item, Laser | Rocket),
        DURANIUM => matches!(item, Driving | Shield),
        PROMERIUM => matches!(item, Laser | Rocket | Driving | Shield),
        SEPROM | OSMIUM => matches!(item, Laser | Rocket | Shield),
        _ => false,
    };
    // Same ore already applied and nearly maxed -> rejected in the stock
    // client (`maxAmount < 10` makes the drop visually invalid).
    let near_full = target_ore == Some(ore) && (LAB_ITEM_MAX - applied) < 10.0;
    kind_ok && !near_full
}

/// Drop target for a dragged source ore — `loc=0` drops over the item
/// slot; resolves the "same ore / empty / replace-confirm" dialog path.
pub fn upgrade_max_count(item: LabItem, ore_count: f64) -> u64 {
    // `Math.min(target.maxAmount * 0.1, oreCount)` — the stock client
    // caps a single update at 10% of the item's remaining headroom.
    let headroom = (LAB_ITEM_MAX - item.count).max(0.0);
    (headroom * 0.1).min(ore_count).floor().max(0.0) as u64
}

/// `fillComboBox` option list for a maximum amount — presets below max
/// followed by max itself (max replaces nothing when it equals a preset).
pub fn combo_options(max: u64) -> Vec<u64> {
    let mut opts: Vec<u64> = COMBO_AMOUNTS.iter().copied().filter(|v| *v < max).collect();
    opts.push(max);
    opts
}

/// Max refinable amount of `ore_type` given cargo counts — min over the
/// recipe inputs (`setAvailableAmmount` in `RefinementManager`). An empty
/// recipe means the ore isn't refinable at all.
pub fn refine_max(recipe: &[(i32, i32)], ores: &std::collections::BTreeMap<i32, f64>) -> u64 {
    if recipe.is_empty() {
        return 0;
    }
    recipe
        .iter()
        .map(|(input, need)| {
            let avail = ores.get(input).copied().unwrap_or(0.0);
            (avail / f64::from(*need).max(1.0)).floor()
        })
        .fold(f64::INFINITY, f64::min) as u64
}

/// `LabUpdateItemCommand` (135): `itemsUpdatedInfo` = list of
/// `UpdateItemModule` — `itemToUpdate` (`LabItemModule.itemValue`) +
/// `updateWith` (`OreCountModule` of `oreType.typeValue` + `count`).
pub fn handle(world: &mut SharedWorld, p: &Packet) {
    if p.id != id::LAB_UPDATE_ITEM_COMMAND {
        return;
    }
    for v in p.list("itemsUpdatedInfo").unwrap_or(&[]) {
        let Value::Module(m) = v else {
            continue;
        };
        let Ok(item) = m.module("itemToUpdate").and_then(|m| m.int("itemValue")) else {
            continue;
        };
        let Ok(update) = m.module("updateWith") else {
            continue;
        };
        let ore_type = update
            .module("oreType")
            .and_then(|m| m.int("typeValue"))
            .map(|v| v as i32)
            .ok();
        let count = update.f64("count").unwrap_or(0.0);
        if let Some(slot) = world.lab.items.get_mut(item.max(0) as usize) {
            slot.ore_type = ore_type.filter(|t| *t > 0);
            slot.count = count;
            world.lab.loaded = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    /// `SourceRefinementModule.isValid` — ore/item compatibility matrix.
    #[test]
    fn valid_upgrade_matches_stock_matrix() {
        use LabItemKind::*;
        // prometid → lasers+rockets only; duranium → driving+shield only.
        assert!(valid_upgrade(4, Laser, None, 0.0));
        assert!(valid_upgrade(4, Rocket, None, 0.0));
        assert!(!valid_upgrade(4, Shield, None, 0.0));
        assert!(!valid_upgrade(4, Driving, None, 0.0));
        assert!(!valid_upgrade(5, Laser, None, 0.0));
        assert!(!valid_upgrade(5, Rocket, None, 0.0));
        assert!(valid_upgrade(5, Driving, None, 0.0));
        assert!(valid_upgrade(5, Shield, None, 0.0));
        // promerium → everything; seprom/osmium → not driving.
        for k in [Laser, Rocket, Driving, Shield] {
            assert!(valid_upgrade(6, k, None, 0.0));
        }
        for ore in [7, 28] {
            assert!(valid_upgrade(ore, Laser, None, 0.0));
            assert!(valid_upgrade(ore, Rocket, None, 0.0));
            assert!(!valid_upgrade(ore, Driving, None, 0.0));
            assert!(valid_upgrade(ore, Shield, None, 0.0));
        }
        // Unknown ore / same ore nearly maxed → rejected.
        assert!(!valid_upgrade(0, Laser, None, 0.0));
        assert!(!valid_upgrade(4, Laser, Some(4), LAB_ITEM_MAX - 5.0));
        assert!(valid_upgrade(4, Laser, Some(4), LAB_ITEM_MAX - 50.0));
    }

    #[test]
    fn combo_options_appends_the_max() {
        assert_eq!(combo_options(0), [0]);
        assert_eq!(combo_options(1), [1]);
        assert_eq!(combo_options(100), [1, 5, 10, 50, 100]);
        assert_eq!(combo_options(3500), [1, 5, 10, 50, 100, 500, 1000, 3500]);
    }

    #[test]
    fn upgrade_max_count_caps_at_ten_percent_headroom() {
        let empty = LabItem::default();
        assert_eq!(upgrade_max_count(empty, 5000.0), 5000); // 100k cap above 5k
        assert_eq!(upgrade_max_count(empty, 500_000.0), 100_000);
        let half = LabItem {
            ore_type: Some(4),
            count: LAB_ITEM_MAX * 0.5,
        };
        assert_eq!(upgrade_max_count(half, 1_000_000.0), 50_000);
        assert_eq!(upgrade_max_count(half, 10.0), 10);
        let full = LabItem {
            ore_type: Some(4),
            count: LAB_ITEM_MAX,
        };
        assert_eq!(upgrade_max_count(full, 100.0), 0);
    }

    #[test]
    fn refine_max_is_limited_by_the_scarcest_input() {
        let mut ores = BTreeMap::new();
        ores.insert(0, 200.0); // needs 20/unit
        ores.insert(1, 55.0); // needs 10/unit → 5 prometid
        assert_eq!(refine_max(&[(0, 20), (1, 10)], &ores), 5);
        // Missing input → 0.
        assert_eq!(refine_max(&[(0, 20), (2, 1)], &ores), 0);
        // No recipe → 0 (nothing refines it).
        assert_eq!(refine_max(&[], &ores), 0);
    }

    #[test]
    fn lab_update_item_command_populates_item_slots() {
        let mut world = SharedWorld::default();
        let item = |item_value: i64| Packet {
            id: id::LAB_UPDATE_ITEM_COMMAND,
            name: None,
            fields: vec![(
                Some("itemsUpdatedInfo"),
                Value::List(vec![Value::Module(Box::new(Packet {
                    id: 0,
                    name: None,
                    fields: vec![
                        (
                            Some("itemToUpdate"),
                            Value::Module(Box::new(Packet {
                                id: id::LAB_ITEM_MODULE,
                                name: None,
                                fields: vec![(Some("itemValue"), Value::Int(item_value))],
                            })),
                        ),
                        (
                            Some("updateWith"),
                            Value::Module(Box::new(Packet {
                                id: 0,
                                name: None,
                                fields: vec![
                                    (
                                        Some("oreType"),
                                        Value::Module(Box::new(Packet {
                                            id: 0,
                                            name: None,
                                            fields: vec![(Some("typeValue"), Value::Int(4))],
                                        })),
                                    ),
                                    (Some("count"), Value::F64(1234.0)),
                                ],
                            })),
                        ),
                    ],
                }))]),
            )],
        };
        handle(&mut world, &item(1));
        assert!(world.lab.loaded);
        assert_eq!(world.lab.items[1].ore_type, Some(4));
        assert_eq!(world.lab.items[1].count, 1234.0);
        assert_eq!(world.lab.items[0].ore_type, None);
    }
}
