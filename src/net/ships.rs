use xenomit_packet_client::modules::{self, id};
use xenomit_packet_client::packet::{Packet, Value};

use crate::net::world::{SharedWorld, WorldFx};

/// `AssetRemoveCommand` (station assets leave differently than ships).
pub(crate) const ASSET_REMOVE_COMMAND: i16 = 3245;
/// `ShipSelectionCommand` — full vitals of the ship you clicked on.
pub(crate) const SHIP_SELECTION: i16 = 50;

/// Per-ship name-plate data for `ShipInfoDisplay`/`TopDisplay` — rank,
/// galaxy gates, clan diplomacy and the drone loadout. `Entity` keeps
/// none of these; they ride `ShipCreateCommand`/
/// `ShipInitializationCommand`, `DroneFormationChangeCommand` and the
/// `LegacyModule` `n|d` channel, so they are captured at the packet edge.
#[derive(Clone, Debug, Default)]
pub struct ShipInfo {
    /// `dailyRank` — `icons`/`rank{N}` sprite when non-zero.
    pub daily_rank: i32,
    /// `galaxyGatesDone` — bit i set = gate i finished (gg icon row).
    pub gg_done: i32,
    /// `clanDiplomacy.type` — 1 alliance, 2 NAP, 3 war.
    pub clan_diplomacy: i32,
    /// Selected drone formation id (`DroneFormationChangeCommand`).
    pub drone_formation: i32,
    /// Number of drones (from the `LegacyModule` drone-set message).
    pub drone_count: i32,
    /// Title shown above the name plate (`MapObjectInfoTrait.titleID`),
    /// from `MapEventTitleUpdateCommand` / legacy `n|t` — already
    /// humanized for display, `""` = none.
    pub title: String,
    /// Raw title key behind [`Self::title`] — kept so a late-arriving
    /// language table can re-resolve it.
    pub title_key: String,
    /// `ShipCreateCommand.motherShipId` — a swarm minion docked to this
    /// mothership (0 = free-flying).
    pub mother_ship_id: i32,
    /// `ShipCreateCommand.positionIndex` — the mothership's `dock_N`
    /// pivot this minion sits on.
    pub position_index: i32,
    /// `VISUALMODIFIER_CHANGE_SHIP_DESIGN.additionalAttribute` — the ship
    /// type the entity is currently skinned as. Applied to the entity's
    /// `type_id` by the game layer once the pattern is known to exist.
    pub design_override: Option<String>,
    /// Currently active `VisualModifierCommand.modifier` ids (activated
    /// ones only; `CHANGE_SHIP_DESIGN` is tracked via `design_override`).
    pub modifiers: Vec<i32>,
}

/// `VisualModifierCommand` (280).
pub(crate) const VISUAL_MODIFIER: i16 = 280;
/// `MapEventTitleUpdateCommand` (183).
pub(crate) const TITLE_UPDATE: i16 = 183;
/// `MapEventTitleUpdateCommand.ACTION_REMOVE_TITLE`.
const TITLE_ACTION_REMOVE: i64 = 1;
/// `VisualModifierCommand.VISUALMODIFIER_CHANGE_SHIP_DESIGN`.
pub const MODIFIER_CHANGE_SHIP_DESIGN: i32 = 25;

/// One decoded `VisualModifierCommand` row.
#[derive(Clone, Debug, PartialEq)]
pub struct ModifierRow {
    pub modifier: i32,
    pub attribute: i32,
    pub additional: String,
    pub activated: bool,
}

impl ModifierRow {
    fn from_packet(m: &Packet) -> Option<Self> {
        Some(Self {
            modifier: m.int("modifier").ok()? as i32,
            attribute: m.int("attribute").unwrap_or(0) as i32,
            additional: m.string("additionalAttribute").unwrap_or("").to_owned(),
            activated: m.boolean("activated").unwrap_or(false),
        })
    }
}

/// Decode the `modifier` list of `ShipCreateCommand` /
/// `ShipInitializationCommand`.
fn modifier_rows(p: &Packet) -> Vec<ModifierRow> {
    p.list("modifier")
        .map(|vals| {
            vals.iter()
                .filter_map(|v| match v {
                    Value::Module(m) => ModifierRow::from_packet(m),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Activated modifier rows carried inside `ShipCreate`/
/// `ShipInitialization`/`AssetCreate` are forwarded as
/// `WorldFx::VisualModifier` so `ShipFxState` seeds the same effects a
/// live `VisualModifierCommand` would.
fn push_initial_modifiers(world: &mut SharedWorld, entity: i32, rows: Vec<ModifierRow>) {
    for row in rows {
        if row.activated {
            world.fx.push(WorldFx::VisualModifier { entity, row });
        }
    }
}

impl ShipInfo {
    /// Fold one visual modifier in: `CHANGE_SHIP_DESIGN` swaps the
    /// design, everything else toggles membership in `modifiers`.
    pub fn apply_modifier(&mut self, row: &ModifierRow) {
        if row.modifier == MODIFIER_CHANGE_SHIP_DESIGN {
            if !row.additional.is_empty() {
                self.design_override = Some(row.additional.clone());
            }
            return;
        }
        self.modifiers.retain(|m| *m != row.modifier);
        if row.activated {
            self.modifiers.push(row.modifier);
        }
    }
}

/// Display text for a title key: the language table entry when the
/// catalog has one (`InGameCatalog.getTitleKey` → `BPLocale`), else the
/// humanized key (see [`humanize_title_key`]).
pub fn title_text(locale: &crate::locale::LocaleCatalog, key: &str) -> String {
    let key = key.trim();
    match locale.lookup(key) {
        Some(text) if !text.is_empty() => text.to_owned(),
        _ => humanize_title_key(key),
    }
}

/// Readable fallback for a title key without a language entry: a
/// `title_` prefix is dropped and separators become spaces, words
/// capitalized.
pub fn humanize_title_key(key: &str) -> String {
    let key = key.trim();
    let key = key
        .strip_prefix("title_")
        .or_else(|| key.strip_prefix("title-"))
        .unwrap_or(key);
    key.split(['_', '-', ' '])
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Set (non-empty key) or clear (empty key) a ship's title.
pub fn set_title(world: &mut SharedWorld, uid: i32, key: &str) {
    let locale = world.locale.clone();
    let info = world.ship_info.entry(uid).or_default();
    info.title_key = key.trim().to_owned();
    info.title = title_text(&locale, key);
}

/// Legacy `0|n|t|<uid>|<n>|<titleKey>` (`assembleSetTitle`).
pub fn parse_set_title(msg: &str) -> Option<(i32, String)> {
    let parts: Vec<&str> = msg.split('|').collect();
    if parts.len() < 6 || parts[1] != "n" || parts[2] != "t" {
        return None;
    }
    Some((parts[3].parse().ok()?, parts[5].to_owned()))
}

/// Legacy `0|n|trm|<uid>` (`assembleRemoveTitle`).
pub fn parse_remove_title(msg: &str) -> Option<i32> {
    let parts: Vec<&str> = msg.split('|').collect();
    if parts.len() < 4 || parts[1] != "n" || parts[2] != "trm" {
        return None;
    }
    parts[3].parse().ok()
}

pub fn handle(world: &mut SharedWorld, p: &Packet) {
    match p.id {
        VISUAL_MODIFIER => {
            if let (Ok(uid), Some(row)) = (p.int("userId"), ModifierRow::from_packet(p)) {
                world
                    .ship_info
                    .entry(uid as i32)
                    .or_default()
                    .apply_modifier(&row);
                world.fx.push(WorldFx::VisualModifier {
                    entity: uid as i32,
                    row,
                });
            }
        }
        TITLE_UPDATE => {
            if let Ok(uid) = p.int("userId") {
                let remove = p.int("titleAction").unwrap_or(0) == TITLE_ACTION_REMOVE;
                let key = if remove {
                    ""
                } else {
                    p.string("titleKey").unwrap_or("")
                };
                set_title(world, uid as i32, key);
            }
        }
        id::SHIP_INITIALIZATION => {
            if let Some(h) = &world.world.hero {
                world.tweens.remove(&h.user_id);
            }
            if let Ok(uid) = p.int("userId") {
                world.tweens.remove(&(uid as i32));
                let rows = modifier_rows(p);
                let info = world.ship_info.entry(uid as i32).or_default();
                info.daily_rank = p.int("dailyRank").unwrap_or(0) as i32;
                info.gg_done = p.int("galaxyGatesDone").unwrap_or(0) as i32;
                for row in &rows {
                    info.apply_modifier(row);
                }
                push_initial_modifiers(world, uid as i32, rows);
            }
            world.hero_beacon = None;
            world.kill_screen = None;
            ::log::debug!(
                "ship init vitals hp={}/{} shield={}/{} nano={}/{}",
                p.f64("hitPoints").unwrap_or(0.0),
                p.f64("hitMax").unwrap_or(0.0),
                p.int("shield").unwrap_or(0),
                p.int("shieldMax").unwrap_or(0),
                p.int("nanoHull").unwrap_or(0),
                p.int("maxNanoHull").unwrap_or(0)
            );
        }
        id::ATTRIBUTE_SHIELD_UPDATE => {
            ::log::debug!(
                "shield update now={} max={}",
                p.int("shieldNow").unwrap_or(0),
                p.int("shieldMax").unwrap_or(0)
            );
        }
        id::HITPOINT_INFO => {
            ::log::debug!(
                "hitpoint info hp={}/{} nano={}/{}",
                p.f64("hitpoints").unwrap_or(0.0),
                p.f64("hitpointsMax").unwrap_or(0.0),
                p.int("nanoHull").unwrap_or(0),
                p.int("nanoHullMax").unwrap_or(0)
            );
        }
        SHIP_SELECTION => {
            if let Ok(uid) = p.int("userId") {
                let uid = uid as i32;
                if let Some(e) = world.world.entities.get_mut(&uid) {
                    if let Ok(v) = p.f64("hitpoints") {
                        e.hitpoints = Some(v);
                    }
                    if let Ok(v) = p.f64("hitpointsMax") {
                        e.hitpoints_max = Some(v);
                    }
                    if let Ok(v) = p.int("shield") {
                        e.shield = Some(v as i32);
                    }
                    if let Ok(v) = p.int("shieldMax") {
                        e.shield_max = Some(v as i32);
                    }
                    if e.type_id.is_empty() {
                        if let Ok(t) = p.int("shipType") {
                            e.type_id = t.to_string();
                        }
                    }
                }
                if let (Ok(nh), Ok(nhm)) = (p.int("nanoHull"), p.int("maxNanoHull")) {
                    world.entity_nano.insert(uid, (nh as i32, nhm as i32));
                }
            }
        }
        id::ASSET_CREATE => {
            if let Ok(uid) = p.int("assetId") {
                push_initial_modifiers(world, uid as i32, modifier_rows(p));
                let tv = p
                    .fields
                    .iter()
                    .filter(|(n, _)| *n == Some("type"))
                    .filter_map(|(_, v)| match v {
                        Value::Module(m) => m.int("typeValue").ok(),
                        _ => None,
                    })
                    .next()
                    .unwrap_or(0) as i32;
                world.entity_asset_type.insert(uid as i32, tv);
                ::log::info!(
                    "asset {} type={tv} design={} faction={} stage={} name={} at ({},{})",
                    uid,
                    p.int("designId").unwrap_or(0),
                    p.int("factionId").unwrap_or(0),
                    p.int("expansionStage").unwrap_or(0),
                    p.string("userName").unwrap_or(""),
                    p.int("posX").unwrap_or(0),
                    p.int("posY").unwrap_or(0)
                );
                ::log::debug!(
                    "assetcreate {uid} fields: {:?}",
                    p.fields
                        .iter()
                        .map(|(n, v)| format!(
                            "{}={}",
                            n.unwrap_or("?"),
                            match v {
                                Value::Module(m) => format!(
                                    "module#{}:{:?}",
                                    m.id,
                                    m.fields
                                        .iter()
                                        .map(|(n, v)| format!("{}={v:?}", n.unwrap_or("?")))
                                        .collect::<Vec<_>>()
                                ),
                                _ => format!("{v:?}"),
                            }
                        ))
                        .collect::<Vec<_>>()
                );
            }
        }
        id::SHIP_CREATE => {
            if let Ok(uid) = p.int("userId") {
                ::log::debug!(
                    "ship create uid={uid} name={} npc={} at ({},{})",
                    p.string("userName").unwrap_or("?"),
                    p.boolean("npc").unwrap_or(false),
                    p.int("x").unwrap_or(0),
                    p.int("y").unwrap_or(0)
                );
                let rows = modifier_rows(p);
                let info = world.ship_info.entry(uid as i32).or_default();
                info.daily_rank = p.int("dailyRank").unwrap_or(0) as i32;
                info.gg_done = p.int("galaxyGatesDone").unwrap_or(0) as i32;
                info.clan_diplomacy = p
                    .module("clanDiplomacy")
                    .ok()
                    .and_then(|m| m.int("type").ok())
                    .unwrap_or(0) as i32;
                info.mother_ship_id = p.int("motherShipId").unwrap_or(0) as i32;
                info.position_index = p.int("positionIndex").unwrap_or(0) as i32;
                for row in &rows {
                    info.apply_modifier(row);
                }
                push_initial_modifiers(world, uid as i32, rows);
            }
        }
        id::SHIP_REMOVE | ASSET_REMOVE_COMMAND => {
            let uid = modules::parse_ship_remove(p).or_else(|| p.int("uid").ok().map(|v| v as i32));
            if let Some(uid) = uid {
                world.tweens.remove(&uid);
                world.entity_nano.remove(&uid);
                world.entity_asset_type.remove(&uid);
                world.ship_info.remove(&uid);
                world.drones.remove(&uid);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xenomit_packet_client::modules::id;
    use xenomit_packet_client::packet::{Packet, Value};

    #[test]
    fn ship_create_captures_ship_info() {
        let mut w = SharedWorld::default();
        let diplo = Packet {
            id: 0,
            name: Some("ClanRelationModule"),
            fields: vec![(Some("type"), Value::Int(2))],
        };
        let p = Packet {
            id: id::SHIP_CREATE,
            name: Some("ShipCreateCommand"),
            fields: vec![
                (Some("userId"), Value::Int(42)),
                (Some("dailyRank"), Value::Int(7)),
                (Some("galaxyGatesDone"), Value::Int(3)),
                (Some("clanDiplomacy"), Value::Module(Box::new(diplo))),
            ],
        };
        w.apply_packet(&p);
        let info = w.ship_info.get(&42).expect("ship_info row");
        assert_eq!(info.daily_rank, 7);
        assert_eq!(info.gg_done, 3);
        assert_eq!(info.clan_diplomacy, 2);
    }

    fn modifier_packet(modifier: i64, attribute: i64, extra: &str, activated: bool) -> Packet {
        Packet {
            id: VISUAL_MODIFIER,
            name: Some("VisualModifierCommand"),
            fields: vec![
                (Some("userId"), Value::Int(42)),
                (Some("modifier"), Value::Int(modifier)),
                (Some("attribute"), Value::Int(attribute)),
                (Some("additionalAttribute"), Value::Str(extra.to_owned())),
                (Some("count"), Value::Int(0)),
                (Some("activated"), Value::Bool(activated)),
            ],
        }
    }

    #[test]
    fn ship_create_captures_swarm_and_modifiers() {
        let mut w = SharedWorld::default();
        let design = modifier_packet(25, 0, "ship_goliath", true);
        let glow = modifier_packet(29, 0, "", true);
        let p = Packet {
            id: id::SHIP_CREATE,
            name: Some("ShipCreateCommand"),
            fields: vec![
                (Some("userId"), Value::Int(42)),
                (Some("motherShipId"), Value::Int(7)),
                (Some("positionIndex"), Value::Int(3)),
                (
                    Some("modifier"),
                    Value::List(vec![
                        Value::Module(Box::new(design)),
                        Value::Module(Box::new(glow)),
                    ]),
                ),
            ],
        };
        w.apply_packet(&p);
        let info = &w.ship_info[&42];
        assert_eq!((info.mother_ship_id, info.position_index), (7, 3));
        assert_eq!(info.design_override.as_deref(), Some("ship_goliath"));
        assert_eq!(info.modifiers, vec![29]);
    }

    #[test]
    fn visual_modifier_command_toggles_and_swaps_design() {
        let mut w = SharedWorld::default();
        w.apply_packet(&modifier_packet(2, 0, "", true));
        w.apply_packet(&modifier_packet(3, 0, "", true));
        assert_eq!(w.ship_info[&42].modifiers, vec![2, 3]);
        w.apply_packet(&modifier_packet(2, 0, "", false));
        assert_eq!(w.ship_info[&42].modifiers, vec![3]);
        w.apply_packet(&modifier_packet(25, 4, "ship_aegis", true));
        assert_eq!(
            w.ship_info[&42].design_override.as_deref(),
            Some("ship_aegis")
        );
        // A design swap never lands in the toggle list.
        assert_eq!(w.ship_info[&42].modifiers, vec![3]);
    }

    #[test]
    fn title_update_sets_and_clears() {
        let mut w = SharedWorld::default();
        let set = Packet {
            id: TITLE_UPDATE,
            name: Some("MapEventTitleUpdateCommand"),
            fields: vec![
                (Some("type"), Value::Int(0)),
                (Some("titleAction"), Value::Int(0)),
                (Some("userId"), Value::Int(11)),
                (Some("titleKey"), Value::Str("title_top_gunner".to_owned())),
            ],
        };
        w.apply_packet(&set);
        assert_eq!(w.ship_info[&11].title, "Top Gunner");
        let remove = Packet {
            fields: vec![
                (Some("type"), Value::Int(0)),
                (Some("titleAction"), Value::Int(1)),
                (Some("userId"), Value::Int(11)),
                (Some("titleKey"), Value::Str("title_top_gunner".to_owned())),
            ],
            ..set
        };
        w.apply_packet(&remove);
        assert_eq!(w.ship_info[&11].title, "");
    }

    #[test]
    fn legacy_title_messages() {
        assert_eq!(
            parse_set_title("0|n|t|123|0|title_pvp_king"),
            Some((123, "title_pvp_king".to_owned()))
        );
        assert_eq!(parse_set_title("0|n|t|123"), None);
        assert_eq!(parse_set_title("0|n|d|123|0|x"), None);
        assert_eq!(parse_remove_title("0|n|trm|123"), Some(123));
        assert_eq!(parse_remove_title("0|n|t|123|0|k"), None);

        let mut w = SharedWorld::default();
        let msg = |m: &str| Packet {
            id: id::LEGACY,
            name: Some("LegacyModule"),
            fields: vec![(Some("message"), Value::Str(m.to_owned()))],
        };
        w.apply_packet(&msg("0|n|t|5|0|title_pvp_king"));
        assert_eq!(w.ship_info[&5].title, "Pvp King");
        w.apply_packet(&msg("0|n|trm|5"));
        assert_eq!(w.ship_info[&5].title, "");
    }

    #[test]
    fn titles_resolve_through_locale_and_late_catalog() {
        let mut w = SharedWorld::default();
        set_title(&mut w, 7, "title_top_gunner");
        // No catalog yet: humanized fallback.
        assert_eq!(w.ship_info[&7].title, "Top Gunner");

        let mut cat = crate::locale::LocaleCatalog::new("tr");
        cat.load_xml(
            r#"<root><item name="title_top_gunner">Üst Nişancı</item></root>"#,
            "",
        )
        .unwrap();
        // Installing the catalog re-resolves the already captured key.
        w.set_locale(std::sync::Arc::new(cat));
        assert_eq!(w.ship_info[&7].title, "Üst Nişancı");
        // New titles resolve immediately; unknown keys still humanize.
        set_title(&mut w, 8, "title_top_gunner");
        assert_eq!(w.ship_info[&8].title, "Üst Nişancı");
        set_title(&mut w, 9, "title_pvp_king");
        assert_eq!(w.ship_info[&9].title, "Pvp King");
        set_title(&mut w, 8, "");
        assert_eq!(w.ship_info[&8].title, "");
    }

    #[test]
    fn humanize_title_keys() {
        assert_eq!(humanize_title_key("title_top_gunner"), "Top Gunner");
        assert_eq!(humanize_title_key("Hero"), "Hero");
        assert_eq!(humanize_title_key("  "), "");
    }
}
