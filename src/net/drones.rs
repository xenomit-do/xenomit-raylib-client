use xenomit_packet_client::packet::Packet;

use crate::net::world::SharedWorld;

/// `DroneFormationChangeCommand` — a ship switched drone formation.
pub(crate) const DRONE_FORMATION_CHANGE: i16 = 257;

/// `MapEventDroneInfoCommand` — wraps the legacy `n|d` drone-set string
/// (`legacyDroneCommandString`); how remote loadouts actually arrive.
pub(crate) const MAP_EVENT_DRONE_INFO: i16 = 180;

/// One drone of a ship's loadout (`Drone.as`). `stats_design` /
/// `visual_design` are `<drones><designs><design id>` ids
/// (`DroneDesign.getDesign`) — the asset key chain is resolved from them
/// by `AssetManager::drone_display`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DroneEntry {
    /// Position in the loadout (`Drone.index`) — picks the formation slot.
    pub index: i32,
    /// `Drone.type` (1 flax, 2 iris, 3 apis, 4 zeus).
    pub type_id: i32,
    /// `Drone.level`.
    pub level: i32,
    pub stats_design: i32,
    pub visual_design: i32,
}

/// Everything the drone view needs for one ship
/// (`Ship.drones: Drones`).
#[derive(Clone, Debug, PartialEq)]
pub struct DroneLoadout {
    /// `Drones.droneFormationId`.
    pub formation: i32,
    pub drones: Vec<DroneEntry>,
    /// `Drones.userScale` (formation spread multiplier, default 1).
    pub user_scale: f32,
}

impl Default for DroneLoadout {
    fn default() -> Self {
        Self {
            formation: 0,
            drones: Vec::new(),
            user_scale: 1.0,
        }
    }
}

pub fn handle(world: &mut SharedWorld, p: &Packet) {
    if p.id == MAP_EVENT_DRONE_INFO {
        // `simple` reduces the view to drone count only; the payload is
        // the same `0|n|d|…` string either way.
        if let Ok(msg) = p.string("legacyDroneCommandString") {
            if let Some((uid, _count, formation, drones)) = parse_set_drones(&msg) {
                apply_set_drones(world, uid, formation, drones);
            }
        }
        return;
    }
    if p.id != DRONE_FORMATION_CHANGE {
        return;
    }
    if let Ok(uid) = p.int("uid") {
        let formation = p.int("selectedFormationId").unwrap_or(0) as i32;
        set_formation(world, uid as i32, formation);
    }
}

/// Switch a ship's formation — keeps `ShipInfo` (name-plate icon) and the
/// `DroneLoadout` (placement) in step.
pub fn set_formation(world: &mut SharedWorld, uid: i32, formation: i32) {
    world.ship_info.entry(uid).or_default().drone_formation = formation;
    world.drones.entry(uid).or_default().formation = formation;
}

/// Fold a parsed `n|d` message into the world (`assembleSetDrones`):
/// replaces the drone list, switches the formation when the message
/// carried one, and refreshes `ShipInfo::drone_count`.
pub fn apply_set_drones(
    world: &mut SharedWorld,
    uid: i32,
    formation: Option<i32>,
    drones: Vec<DroneEntry>,
) {
    let known_formation = world
        .ship_info
        .get(&uid)
        .map(|i| i.drone_formation)
        .unwrap_or(0);
    let count = drones.len() as i32;
    let loadout = world.drones.entry(uid).or_insert_with(|| DroneLoadout {
        formation: known_formation,
        ..Default::default()
    });
    if let Some(f) = formation {
        loadout.formation = f;
    }
    loadout.drones = drones;
    let info = world.ship_info.entry(uid).or_default();
    info.drone_count = count;
    if let Some(f) = formation {
        info.drone_formation = f;
    }
}

/// `LegacyModule` `n|d` drone-set message (`assembleSetDrones`):
/// `0|n|d|<uid>|<formation?>|<4 fields per drone>`, each drone being
/// `type|level|statsDesign|visualDesign`. Returns
/// `(uid, drone_count, formation, drones)` — `formation` is `None` when
/// the message carries no formation field (`(len-4) % 4 == 0`); a
/// trailing partial drone group is ignored.
pub fn parse_set_drones(msg: &str) -> Option<(i32, i32, Option<i32>, Vec<DroneEntry>)> {
    let parts: Vec<&str> = msg.split('|').collect();
    if parts.len() < 4 || parts[1] != "n" || parts[2] != "d" {
        return None;
    }
    let uid: i32 = parts[3].parse().ok()?;
    let rest = parts.len() - 4;
    let (formation, first) = if rest % 4 == 0 {
        (None, 4)
    } else {
        (parts.get(4).and_then(|s| s.trim().parse::<i32>().ok()), 5)
    };
    let field = |s: &str| s.trim().parse::<i32>().unwrap_or(0);
    let drones: Vec<DroneEntry> = parts[first..]
        .chunks_exact(4)
        .enumerate()
        .map(|(i, c)| DroneEntry {
            index: i as i32,
            type_id: field(c[0]),
            level: field(c[1]),
            stats_design: field(c[2]),
            visual_design: field(c[3]),
        })
        .collect();
    Some((uid, drones.len() as i32, formation, drones))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(index: i32, type_id: i32, level: i32, stats: i32, visual: i32) -> DroneEntry {
        DroneEntry {
            index,
            type_id,
            level,
            stats_design: stats,
            visual_design: visual,
        }
    }

    #[test]
    fn set_drones_without_formation() {
        assert_eq!(
            parse_set_drones("0|n|d|12345|4|1|0|0|1|2|0|0"),
            Some((
                12345,
                2,
                None,
                vec![entry(0, 4, 1, 0, 0), entry(1, 1, 2, 0, 0)]
            ))
        );
        assert_eq!(parse_set_drones("0|n|d|77"), Some((77, 0, None, vec![])));
    }

    #[test]
    fn set_drones_with_formation() {
        assert_eq!(
            parse_set_drones("0|n|d|12345|3|4|1|0|0|1|2|0|0"),
            Some((
                12345,
                2,
                Some(3),
                vec![entry(0, 4, 1, 0, 0), entry(1, 1, 2, 0, 0)]
            ))
        );
    }

    #[test]
    fn set_drones_field_order_is_type_level_stats_visual() {
        // Flax lvl 6, stats design 1 (havok), visual design 2 (hercules).
        let (_, count, formation, drones) = parse_set_drones("0|n|d|9|0|1|6|1|2|2|5|0|3").unwrap();
        assert_eq!((count, formation), (2, Some(0)));
        assert_eq!(drones[0], entry(0, 1, 6, 1, 2));
        assert_eq!(drones[1], entry(1, 2, 5, 0, 3));
    }

    #[test]
    fn set_drones_ignores_other_messages() {
        assert_eq!(parse_set_drones("0|y|anything"), None);
        assert_eq!(parse_set_drones("0|n|x|5|1|2|3|4"), None);
        assert_eq!(parse_set_drones("w"), None);
    }

    #[test]
    fn set_drones_drops_partial_trailing_group() {
        // formation + one full drone + a dangling 2-field tail.
        let (_, count, formation, drones) = parse_set_drones("0|n|d|5|2|1|1|0|0|3|4").unwrap();
        assert_eq!((count, formation), (1, Some(2)));
        assert_eq!(drones, vec![entry(0, 1, 1, 0, 0)]);
    }

    #[test]
    fn apply_set_drones_stores_loadout_and_count() {
        let mut w = SharedWorld::default();
        apply_set_drones(
            &mut w,
            7,
            Some(4),
            vec![entry(0, 1, 3, 0, 0), entry(1, 1, 3, 0, 1)],
        );
        let l = w.drones.get(&7).unwrap();
        assert_eq!(l.formation, 4);
        assert_eq!(l.drones.len(), 2);
        assert_eq!(l.user_scale, 1.0);
        let info = w.ship_info.get(&7).unwrap();
        assert_eq!((info.drone_count, info.drone_formation), (2, 4));

        // A formation-less update keeps the formation, replaces the list.
        apply_set_drones(&mut w, 7, None, vec![entry(0, 2, 1, 0, 0)]);
        assert_eq!(w.drones[&7].formation, 4);
        assert_eq!(w.ship_info[&7].drone_count, 1);
    }

    #[test]
    fn apply_before_formation_change_seeds_from_ship_info() {
        let mut w = SharedWorld::default();
        set_formation(&mut w, 3, 6);
        apply_set_drones(&mut w, 3, None, vec![entry(0, 1, 1, 0, 0)]);
        assert_eq!(w.drones[&3].formation, 6);
    }

    #[test]
    fn legacy_message_populates_world() {
        use xenomit_packet_client::modules::id;
        use xenomit_packet_client::packet::{Packet, Value};
        let mut w = SharedWorld::default();
        let p = Packet {
            id: id::LEGACY,
            name: Some("LegacyModule"),
            fields: vec![(Some("message"), Value::Str("0|n|d|42|2|1|4|0|1".to_owned()))],
        };
        w.apply_packet(&p);
        assert_eq!(w.drones[&42].formation, 2);
        assert_eq!(w.drones[&42].drones, vec![entry(0, 1, 4, 0, 1)]);
        assert_eq!(w.ship_info[&42].drone_count, 1);
    }

    #[test]
    fn map_event_drone_info_populates_world() {
        use xenomit_packet_client::packet::{Packet, Value};
        let mut w = SharedWorld::default();
        let p = Packet {
            id: MAP_EVENT_DRONE_INFO,
            name: Some("MapEventDroneInfoCommand"),
            fields: vec![
                (Some("simple"), Value::Bool(false)),
                (
                    Some("legacyDroneCommandString"),
                    Value::Str("0|n|d|42|2|1|4|0|1".to_owned()),
                ),
            ],
        };
        w.apply_packet(&p);
        assert_eq!(w.drones[&42].formation, 2);
        assert_eq!(w.drones[&42].drones, vec![entry(0, 1, 4, 0, 1)]);
        assert_eq!(w.ship_info[&42].drone_count, 1);
    }

    #[test]
    fn drone_formation_change_captures_ship_info() {
        use xenomit_packet_client::packet::{Packet, Value};

        let mut w = SharedWorld::default();
        let p = Packet {
            id: DRONE_FORMATION_CHANGE,
            name: Some("DroneFormationChangeCommand"),
            fields: vec![
                (Some("uid"), Value::Int(9)),
                (Some("selectedFormationId"), Value::Int(5)),
            ],
        };
        w.apply_packet(&p);
        assert_eq!(w.ship_info.get(&9).unwrap().drone_formation, 5);
        assert_eq!(w.drones.get(&9).unwrap().formation, 5);
    }
}
