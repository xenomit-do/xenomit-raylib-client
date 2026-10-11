//! Group (party) state — `GroupInitializationCommand`,
//! `GroupMemberInitializationCommand`, `GroupMemberUpdateCommand`,
//! `MemberLeaveCommand`, `GroupCloseCommand` (`GroupProxy` in the stock
//! client). Only the data the HUD needs: roster, vitals, map position.

use xenomit_packet_client::packet::{Packet, Value};

use crate::net::world::SharedWorld;

pub(crate) const GROUP_INIT: i16 = 11796;
pub(crate) const GROUP_MEMBER_INIT: i16 = 30950;
pub(crate) const GROUP_MEMBER_UPDATE: i16 = -4047;
pub(crate) const MEMBER_LEAVE: i16 = -7923;
pub(crate) const GROUP_CLOSE: i16 = 18787;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct GroupMember {
    pub id: i32,
    pub name: String,
    pub level: i32,
    pub hp: f64,
    pub hp_max: f64,
    pub shield: i32,
    pub shield_max: i32,
    pub nano_hull: i32,
    pub nano_hull_max: i32,
    pub map_id: i32,
    pub x: i32,
    pub y: i32,
    pub faction: i32,
    pub clan_tag: String,
    pub ship_icon: i32,
    pub active: bool,
    pub cloaked: bool,
    pub in_fight: bool,
    pub logged_out: bool,
}

impl GroupMember {
    pub fn hp_frac(&self) -> f32 {
        if self.hp_max > 0.0 {
            (self.hp / self.hp_max).clamp(0.0, 1.0) as f32
        } else {
            0.0
        }
    }

    pub fn shield_frac(&self) -> f32 {
        if self.shield_max > 0 {
            (self.shield as f32 / self.shield_max as f32).clamp(0.0, 1.0)
        } else {
            0.0
        }
    }
}

/// `InGameCatalog.nettyShipIcons` — `shipIcon` -> `groupSystemShipIcons`
/// bitmap name. Table generated from `InGameCatalog.as` (direct rows +
/// alias rows resolved); `shipIcons[0]` fallback = `iconShipNull`
/// (`GroupUI.getShipIconBitmap`).
pub fn ship_icon_res(icon: i32) -> &'static str {
    match icon {
        0 => "iconShipNull",
        1 => "iconShip0",
        2 => "iconShip1",
        3 => "iconShip2",
        4 => "iconShip3",
        5 => "iconShip4",
        6 => "iconShip5",
        7 => "iconShip6",
        8 => "iconShip7",
        9 => "iconShip8",
        10 => "iconShip9",
        11 => "iconShip10",
        12..=14 => "iconShip8",
        15 => "iconShip10",
        16 => "iconShip98",
        17 => "iconShip3",
        18 => "iconShip49",
        19..=20 => "iconShip9",
        21..=25 => "iconShip10",
        26 | 28 => "iconShip8",
        27 | 29..=35 | 40 | 43..=47 => "iconShip10",
        36..=39 => "iconShip0",
        41 => "iconShip69",
        42 => "iconShip70",
        48 | 58 | 61 | 71 | 74..=75 | 85 => "iconShip2",
        49 | 62 => "iconShip71",
        50 | 65 => "iconShip72",
        51 | 63 | 153 => "iconShip73",
        52 | 64 | 67 | 189 => "iconShip74",
        53 => "iconShip75",
        54 | 66 => "iconShip76",
        55 | 68 | 154..=155 => "iconShip77",
        56 | 69 => "iconShip78",
        57 | 70 => "iconShip79",
        59..=60 | 98 => "iconShip80",
        72..=73 | 76..=84 | 89 | 95 | 100 | 113 | 149..=150 | 156..=165 | 171..=172 => {
            "iconShip0"
        }
        86 => "iconShip11",
        87..=88 => "iconShip82",
        90 | 114 => "iconShip93",
        91 | 115 => "iconShip90",
        92..=94 | 117..=119 => "iconShip91",
        96 | 109 => "iconShip100",
        97 | 110 => "iconShip99",
        99 => "iconShip81",
        101..=102 | 190 => "iconShip101",
        103 | 120..=122 => "iconShip104",
        104..=108 | 151..=152 => "iconShip111",
        111 => "iconShip102",
        112 => "iconShip105",
        116 | 130 | 138..=140 | 191 | 211 | 213 | 246..=248 | 255..=257 => "iconShip8",
        123 => "iconShip1",
        124 => "iconShip2",
        125 => "iconShip4",
        126 => "iconShip5",
        127 | 180..=182 | 187 | 212 => "iconShip7",
        128 => "iconShip6",
        129 | 176 => "iconShip9",
        131 | 133..=137 | 141..=142 | 146..=148 | 183..=186 | 188 | 194..=197 | 207
        | 214..=216 | 219..=245 | 258..=261 => "iconShip10",
        132 => "iconShip3",
        143 | 175 | 179 | 209 => "iconShip70",
        144 | 173 | 177 | 210 => "iconShip69",
        145 | 174 | 178 | 208 | 249..=254 => "iconShip49",
        168 => "iconShip_EmperorSibelon",
        169 => "iconShip_EmperorLordakium",
        170 => "iconShip_EmperorKristallon",
        192 => "iconShip_PlagueBoss",
        193 => "iconShip_PlagueMinion",
        199 => "iconShip148",
        200 | 203 => "iconShip150",
        201 | 204 => "iconShip149",
        202 => "iconShip147",
        217 => "iconShip_Mimesis",
        218 => "iconShip_Tartarus",
        270 => "iconShip_Cyborg",
        271 => "iconShip_Hammerclaw",
        279 => "iconShip_centurion",
        280..=281 | 285..=287 => "iconShip_bl1",
        282 | 288..=290 => "iconShip_bl2",
        283 | 291..=293 => "iconShip_bl3",
        284 | 294..=296 => "iconShip_bl4",
        297 => "iconShip_Hecate",
        306 => "iconShip_Disruptor",
        307 => "iconShip_Berserker",
        308 => "iconShip_Zephyr",
        313 => "iconShip_Solaris",
        339 => "iconShip_Keres",
        340 => "iconShip_Agatus",
        341 => "iconShip_Spinel",
        343 => "iconShip_Retiarus",
        345 => "iconShip_Orcus",
        348 => "iconShipNull",
        349 => "iconShip_Holo",
        355 => "iconShip_Goliath-Plus",
        356 => "iconShip_Citadel-Plus",
        357 => "iconShip_Liberator-Plus",
        358 => "iconShip_Solace-Plus",
        359 => "iconShip_Solaris-Plus",
        360 => "iconShip_Pusat-Plus",
        361 => "iconShip_Hecate-Plus",
        362 => "iconShip_Hammerclaw-Plus",
        363 => "iconShip_Spearhead-Plus",
        364 => "iconShip_Tempest",
        365 => "iconShip_Tartarus-Plus",
        371 => "iconShip_Basilisk",
        372 => "iconShip_Spectrum-Plus",
        377 => "iconShip_Paladin",
        _ => "iconShipNull",
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct GroupState {
    pub id: i32,
    pub leader_id: i32,
    pub max_size: i32,
    pub invitation_mode: i32,
    pub members: Vec<GroupMember>,
}

impl GroupState {
    pub fn member(&self, id: i32) -> Option<&GroupMember> {
        self.members.iter().find(|m| m.id == id)
    }
}

fn health_into(m: &mut GroupMember, h: &Packet) {
    m.hp = h.f64("hp").unwrap_or(m.hp);
    m.hp_max = h.f64("hpMax").unwrap_or(m.hp_max);
    m.shield = h.int("shield").map(|v| v as i32).unwrap_or(m.shield);
    m.shield_max = h.int("shieldMax").map(|v| v as i32).unwrap_or(m.shield_max);
    m.nano_hull = h.int("nanoHull").map(|v| v as i32).unwrap_or(m.nano_hull);
    m.nano_hull_max = h
        .int("nanoHullMax")
        .map(|v| v as i32)
        .unwrap_or(m.nano_hull_max);
}

fn location_into(m: &mut GroupMember, l: &Packet) {
    m.map_id = l.int("mapId").map(|v| v as i32).unwrap_or(m.map_id);
    m.x = l.int("x").map(|v| v as i32).unwrap_or(m.x);
    m.y = l.int("y").map(|v| v as i32).unwrap_or(m.y);
}

fn member_from_init(p: &Packet) -> Option<GroupMember> {
    let mut m = GroupMember {
        id: p.int("id").ok()? as i32,
        name: p.string("name").unwrap_or("").to_owned(),
        level: p.int("level").unwrap_or(0) as i32,
        active: p.boolean("active").unwrap_or(false),
        cloaked: p.boolean("cloaked").unwrap_or(false),
        in_fight: p.boolean("inFight").unwrap_or(false),
        logged_out: p.boolean("loggedOut").unwrap_or(false),
        ..Default::default()
    };
    if let Ok(h) = p.module("health") {
        health_into(&mut m, h);
    }
    if let Ok(l) = p.module("location") {
        location_into(&mut m, l);
    }
    if let Ok(f) = p.module("faction") {
        m.faction = f.int("faction").unwrap_or(0) as i32;
    }
    if let Ok(s) = p.module("shipIcon") {
        m.ship_icon = s.int("shipIcon").unwrap_or(0) as i32;
    }
    if let Ok(c) = p.module("clan") {
        m.clan_tag = c.string("clanTag").unwrap_or("").to_owned();
    }
    Some(m)
}

/// `GroupProxy.updateMemberProperty` — one `PlayerAttributeModule`.
fn apply_update(m: &mut GroupMember, u: &Packet) {
    match u.name {
        Some("ActivityModule") => m.active = u.boolean("active").unwrap_or(m.active),
        Some("ClanModule") => m.clan_tag = u.string("clanTag").unwrap_or("").to_owned(),
        Some("CloakDeviceActivityModule") => m.cloaked = u.boolean("active").unwrap_or(m.cloaked),
        Some("FactionModule") => {
            m.faction = u.int("faction").map(|v| v as i32).unwrap_or(m.faction)
        }
        Some("FightActivityModule") => m.in_fight = u.boolean("active").unwrap_or(m.in_fight),
        Some("HealthModule") => health_into(m, u),
        Some("LevelModule") => m.level = u.int("level").map(|v| v as i32).unwrap_or(m.level),
        Some("LocationModule") => location_into(m, u),
        Some("MemberLoggedOutModule") => {
            m.logged_out = u.boolean("loggedOut").unwrap_or(m.logged_out)
        }
        Some("ShipIconModule") => {
            m.ship_icon = u.int("shipIcon").map(|v| v as i32).unwrap_or(m.ship_icon)
        }
        _ => {}
    }
}

pub fn handle(world: &mut SharedWorld, p: &Packet) {
    match p.id {
        GROUP_INIT => {
            let mut g = GroupState {
                id: p.int("id").unwrap_or(0) as i32,
                leader_id: p.int("leaderId").unwrap_or(0) as i32,
                max_size: p.int("maxSize").unwrap_or(0) as i32,
                invitation_mode: p
                    .module("invitationMode")
                    .ok()
                    .and_then(|m| m.int("mode").ok())
                    .unwrap_or(0) as i32,
                members: Vec::new(),
            };
            for v in p.list("members").unwrap_or(&[]) {
                if let Value::Module(m) = v {
                    if let Some(member) = member_from_init(m) {
                        g.members.push(member);
                    }
                }
            }
            ::log::debug!(
                "group {} leader={} members={}",
                g.id,
                g.leader_id,
                g.members.len()
            );
            world.group = Some(g);
        }
        GROUP_MEMBER_INIT => {
            if let Some(member) = member_from_init(p) {
                let g = world.group.get_or_insert_with(GroupState::default);
                match g.members.iter_mut().find(|m| m.id == member.id) {
                    Some(slot) => *slot = member,
                    None => g.members.push(member),
                }
            }
        }
        GROUP_MEMBER_UPDATE => {
            let (Ok(pid), Some(g)) = (p.int("playerId"), world.group.as_mut()) else {
                return;
            };
            let Some(m) = g.members.iter_mut().find(|m| m.id == pid as i32) else {
                return;
            };
            for v in p.list("updates").unwrap_or(&[]) {
                if let Value::Module(u) = v {
                    apply_update(m, u);
                }
            }
        }
        MEMBER_LEAVE => {
            let Ok(pid) = p.int("playerId") else {
                return;
            };
            let hero = world.world.hero.as_ref().map(|h| h.user_id);
            if Some(pid as i32) == hero {
                world.group = None;
            } else if let Some(g) = world.group.as_mut() {
                g.members.retain(|m| m.id != pid as i32);
            }
        }
        GROUP_CLOSE => world.group = None,
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pkt(id: i16, name: &'static str, fields: Vec<(&'static str, Value)>) -> Packet {
        Packet {
            id,
            name: Some(name),
            fields: fields.into_iter().map(|(n, v)| (Some(n), v)).collect(),
        }
    }

    fn module(name: &'static str, fields: Vec<(&'static str, Value)>) -> Value {
        Value::Module(Box::new(pkt(0, name, fields)))
    }

    /// `nettyShipIcons` mapping + `shipIcons[0]` fallback.
    #[test]
    fn ship_icon_table_and_fallback() {
        assert_eq!(ship_icon_res(0), "iconShipNull");
        assert_eq!(ship_icon_res(11), "iconShip10");
        assert_eq!(ship_icon_res(18), "iconShip49");
        assert_eq!(ship_icon_res(41), "iconShip69");
        assert_eq!(ship_icon_res(42), "iconShip70");
        assert_eq!(ship_icon_res(86), "iconShip11");
        // Aliases resolve to the aliased art, unknowns to `shipIcons[0]`.
        assert_eq!(ship_icon_res(33), "iconShip10"); // SPECTRUM -> GOLIATH
        assert_eq!(ship_icon_res(74), "iconShip2"); // AGGRO_STREUNER -> YAMATO
        assert_eq!(ship_icon_res(-1), "iconShipNull");
        assert_eq!(ship_icon_res(9999), "iconShipNull");
    }

    fn member(id: i64, name: &str, x: i64) -> Packet {
        pkt(
            GROUP_MEMBER_INIT,
            "GroupMemberInitializationCommand",
            vec![
                ("id", Value::Int(id)),
                ("name", Value::Str(name.into())),
                ("level", Value::Int(12)),
                (
                    "health",
                    module(
                        "HealthModule",
                        vec![("hp", Value::F64(500.0)), ("hpMax", Value::F64(1000.0))],
                    ),
                ),
                (
                    "location",
                    module(
                        "LocationModule",
                        vec![
                            ("mapId", Value::Int(3)),
                            ("x", Value::Int(x)),
                            ("y", Value::Int(20)),
                        ],
                    ),
                ),
                (
                    "clan",
                    module("ClanModule", vec![("clanTag", Value::Str("TAG".into()))]),
                ),
            ],
        )
    }

    #[test]
    fn init_update_and_leave() {
        let mut w = SharedWorld::default();
        let init = pkt(
            GROUP_INIT,
            "GroupInitializationCommand",
            vec![
                ("id", Value::Int(9)),
                ("leaderId", Value::Int(1)),
                ("maxSize", Value::Int(10)),
                (
                    "members",
                    Value::List(vec![Value::Module(Box::new(member(1, "alpha", 100)))]),
                ),
            ],
        );
        w.apply_packet(&init);
        w.apply_packet(&member(2, "bravo", 200));
        let g = w.group.as_ref().unwrap();
        assert_eq!((g.id, g.leader_id, g.max_size), (9, 1, 10));
        assert_eq!(g.members.len(), 2);
        assert_eq!(g.member(2).unwrap().clan_tag, "TAG");
        assert_eq!(g.member(2).unwrap().hp_frac(), 0.5);

        w.apply_packet(&pkt(
            GROUP_MEMBER_UPDATE,
            "GroupMemberUpdateCommand",
            vec![
                ("playerId", Value::Int(2)),
                (
                    "updates",
                    Value::List(vec![
                        module(
                            "LocationModule",
                            vec![
                                ("mapId", Value::Int(4)),
                                ("x", Value::Int(999)),
                                ("y", Value::Int(1)),
                            ],
                        ),
                        module(
                            "MemberLoggedOutModule",
                            vec![("loggedOut", Value::Bool(true))],
                        ),
                    ]),
                ),
            ],
        ));
        let m = w.group.as_ref().unwrap().member(2).unwrap().clone();
        assert_eq!((m.map_id, m.x, m.y), (4, 999, 1));
        assert!(m.logged_out);

        w.apply_packet(&pkt(
            MEMBER_LEAVE,
            "MemberLeaveCommand",
            vec![("playerId", Value::Int(1)), ("reason", Value::Int(0))],
        ));
        assert_eq!(w.group.as_ref().unwrap().members.len(), 1);
        w.apply_packet(&pkt(GROUP_CLOSE, "GroupCloseCommand", vec![]));
        assert!(w.group.is_none());
    }
}
