//! Outbound actions — the packet-level `ClientCommands` equivalent.

use xenomit_packet_client::model::WorldState;
use xenomit_packet_client::modules::{self, id};
use xenomit_packet_client::packet::{Packet, Value};
use xenomit_packet_client::registry;
use xenomit_packet_client::session::LoginInfo;

/// `KillScreenOptionTypeModule` — the `selection` nested module in
/// `KillScreenRepairRequest`.
const KILL_SCREEN_OPTION_TYPE_MODULE: i16 = 254;
/// `OPTIONTYPE_BASIC_REPAIR` — default respawn the kill screen offers.
pub const REPAIR_TYPE_BASIC: i16 = 1;
/// `MenuActionRequest` — slot-bar/item-bar activation.
pub const MENU_ACTION_REQUEST: i16 = 8766;
/// `MenuActionRequest.sourceType` — the slot bar (item bar = 0).
pub const MENU_SOURCE_SLOT_BAR: i16 = 1;
/// `SlotBarConfigSetRequest` — server-side assignment/move/removal of an
/// item-control-menu item in the quickslot bars.
pub const SLOT_BAR_CONFIG_SET_REQUEST: i16 = 32104;

/// `ConfigChangeRequest` — switch C1/C2.
pub const CONFIG_CHANGE_REQUEST: i16 = 35;
/// `QuestCancelRequest`.
pub const QUEST_CANCEL_REQUEST: i16 = 2024;
/// `QuestListRequest`.
pub const QUEST_LIST_REQUEST: i16 = 2074;
/// `QuestInfoRequest`.
pub const QUEST_INFO_REQUEST: i16 = 2073;

fn simple_packet(id: i16, fields: Vec<(&'static str, Value)>) -> Packet {
    Packet {
        id,
        name: registry::module_name(id),
        fields: fields.into_iter().map(|(n, v)| (Some(n), v)).collect(),
    }
}

/// User/session actions sent to the game server.
#[derive(Clone, Debug)]
pub enum NetCommand {
    /// Fly toward world coordinate.
    Move { x: i32, y: i32 },
    /// Refresh a mouse-held movement destination at the faster hold interval.
    MoveHeld { x: i32, y: i32 },
    /// Select a ship/entity (ShipSelectRequest).
    Select { target_id: i32 },
    /// Start laser attack on selected/given target.
    AttackLaser { target_id: i32 },
    /// Fire a rocket at the target.
    AttackRocket { target_id: i32 },
    /// Abort current laser attack.
    AbortLaser,
    /// Collect a box/collectable by server hash.
    Collect { hash: String },
    /// Harvest an ore by server hash.
    Harvest { hash: String },
    /// Jump through a nearby portal.
    Jump,
    /// Legacy module text (`ClientCommands.legacy`).
    Legacy(String),
    /// Periodic keep-alive with mouse click flag.
    KeepAlive { mouse_click: bool },
    /// Ping/map-click position echo (PingPositionRequest).
    Ping { x: i32, y: i32 },
    /// Repair/drone revive etc. (KillScreenRepairRequest) —
    /// `repair_type` = `OPTIONTYPE_*` (`REPAIR_TYPE_BASIC` respawns at
    /// the home base).
    Repair { repair_type: i16 },
    /// Slot-bar item activation (MenuActionRequest) — `action_type` =
    /// SELECT(0)/ACTIVATE(1) per the item's `actionStyle`,
    /// `source_type` = SLOT_BAR(1).
    MenuAction {
        item_id: String,
        action_type: i16,
        source_type: i16,
    },
    /// Assign, move, swap, or remove a menu item in the slot bars.
    /// Empty source fields mean the item came from the category palette;
    /// empty destination fields mean it was dropped outside a slot.
    SlotBarConfigSet {
        slot_id_to: i32,
        menu_item_id: String,
        slot_bar_id_from: String,
        slot_id_from: i32,
        slot_bar_id_to: String,
    },
    /// Request a page of contact entries from the server.
    ContactsRequest { start_index: i32 },
    /// Add/remove/accept/decline/block a contact by id or nickname.
    ContactRequest {
        request_type: i16,
        user_id: i32,
        user_name: String,
    },
    /// Select a group member in the game's group panel.
    GroupMemberSelect { target_id: i32 },
    /// Follow a group member's ship.
    FollowGroupMember { member_id: i32 },
    /// Leave the current group.
    LeaveGroup,
    /// Invite a player by nickname to the current group.
    InviteToGroup { name: String },
    /// Summon (`enabled`) or dismiss the P.E.T. (`PetRequest`).
    Pet { enabled: bool },
    /// Activate a P.E.T. gear (`PetGearActivationRequest`) —
    /// `gear_type` = `PetGearTypeModule.typeValue`, `target` = the
    /// gear's `optParam` (target entity id, 0 when it takes none).
    PetGear { gear_type: i32, target: i32 },
    /// Switch ship configuration (`ConfigChangeRequest`, 1 = C1, 2 = C2).
    ConfigChange { preset_id: i16 },
    /// Cancel an accepted quest (`QuestCancelRequest`).
    QuestCancel { quest_id: i32 },
    /// Ask for the available-quest list (`QuestListRequest`).
    QuestList,
    /// Ask for one quest's definition (`QuestInfoRequest`).
    QuestInfo { quest_id: i32 },
    /// Refinery/lab: refresh the upgrade item amounts
    /// (`LabUpdateRequest` — sent on window open + periodically while
    /// the refinery window is expanded).
    LabUpdate,
    /// Refine `count` units of ore `ore_type` (`LabRefinementRequest`).
    LabRefine { ore_type: i32, count: u64 },
    /// Apply `count` units of `ore_type` to lab item `item`
    /// (`LabUpdateItemRequest`; `item` = `LabItemModule.itemValue`
    /// 0 LASER / 1 ROCKET / 2 DRIVING / 3 SHIELD).
    LabUpdateItem {
        item: i16,
        ore_type: i32,
        count: u64,
    },
    /// Logout (graceful).
    Logout,
}

impl NetCommand {
    /// Build the wire packet for this command.
    ///
    /// Returns `None` when a prerequisite is missing from the world —
    /// e.g. `Move` before the hero spawns, or `Select`/`AttackLaser` for
    /// an entity id the model has not seen.
    ///
    /// `Repair` needs the login credentials (the request embeds a fresh
    /// `LoginRequest`, mirroring `Killscreen.repair`); without them it
    /// returns `None` — use [`Self::to_packet_with`] when they're known.
    pub fn to_packet(&self, world: &WorldState) -> Option<Packet> {
        self.to_packet_with(world, None)
    }

    /// Like [`Self::to_packet`], with the session credentials available
    /// for commands that embed them (`Repair`).
    pub fn to_packet_with(&self, world: &WorldState, login: Option<&LoginInfo>) -> Option<Packet> {
        let packet = match self {
            // Coordinate checksums: the builders apply the client's
            // anti-tamper fixups; they only need the model positions.
            NetCommand::Move { x, y } | NetCommand::MoveHeld { x, y } => {
                modules::move_request(world.hero_pos()?, (*x, *y))
            }
            NetCommand::Select { target_id } => modules::ship_select_request(
                *target_id,
                entity_pos(world, *target_id)?,
                world.hero_pos()?,
                (0, 0),
                0,
            ),
            NetCommand::AttackLaser { target_id } => {
                modules::attack_laser_request(*target_id, entity_pos(world, *target_id)?)
            }
            NetCommand::AttackRocket { target_id } => {
                modules::attack_rocket_request(*target_id, entity_pos(world, *target_id)?)
            }
            NetCommand::AbortLaser => modules::attack_abort_laser_request(),
            NetCommand::Collect { hash } => {
                let c = world.collectables.get(hash)?;
                modules::collect_box_request(hash, (c.x, c.y), world.hero_pos()?)
            }
            NetCommand::Harvest { hash } => modules::harvest_request(hash),
            NetCommand::Jump => modules::jump_request(),
            NetCommand::Legacy(message) => modules::legacy(message),
            NetCommand::KeepAlive { mouse_click } => modules::keep_alive(*mouse_click),
            NetCommand::Ping { x, y } => modules::ping_position_request((*x, *y)),
            NetCommand::Repair { repair_type } => repair_request(login?, *repair_type),
            NetCommand::MenuAction {
                item_id,
                action_type,
                source_type,
            } => Packet {
                id: MENU_ACTION_REQUEST,
                name: registry::module_name(MENU_ACTION_REQUEST),
                fields: vec![
                    (Some("actionType"), Value::Int(*action_type as i64)),
                    (Some("sourceType"), Value::Int(*source_type as i64)),
                    (Some("menuItemId"), Value::Str(item_id.clone())),
                ],
            },
            NetCommand::SlotBarConfigSet {
                slot_id_to,
                menu_item_id,
                slot_bar_id_from,
                slot_id_from,
                slot_bar_id_to,
            } => Packet {
                id: SLOT_BAR_CONFIG_SET_REQUEST,
                name: registry::module_name(SLOT_BAR_CONFIG_SET_REQUEST),
                fields: vec![
                    (Some("slotIdTo"), Value::Int(*slot_id_to as i64)),
                    (Some("menuItemId"), Value::Str(menu_item_id.clone())),
                    (Some("slotBarIdFrom"), Value::Str(slot_bar_id_from.clone())),
                    (Some("slotIdFrom"), Value::Int(*slot_id_from as i64)),
                    (Some("slotBarIdTo"), Value::Str(slot_bar_id_to.clone())),
                ],
            },
            NetCommand::ContactsRequest { start_index } => {
                simple_packet(1883, vec![("startIndex", Value::Int(*start_index as i64))])
            }
            NetCommand::ContactRequest {
                request_type,
                user_id,
                user_name,
            } => simple_packet(
                13422,
                vec![
                    ("contactRequestType", Value::Int(*request_type as i64)),
                    ("userId", Value::Int(*user_id as i64)),
                    ("userName", Value::Str(user_name.clone())),
                ],
            ),
            NetCommand::GroupMemberSelect { target_id } => {
                simple_packet(-2023, vec![("targetId", Value::Int(*target_id as i64))])
            }
            NetCommand::FollowGroupMember { member_id } => {
                simple_packet(12117, vec![("memberId", Value::Int(*member_id as i64))])
            }
            NetCommand::LeaveGroup => simple_packet(11779, vec![]),
            NetCommand::InviteToGroup { name } => {
                simple_packet(9075, vec![("name", Value::Str(name.clone()))])
            }
            NetCommand::Pet { enabled } => modules::set_pet_request(*enabled),
            NetCommand::PetGear { gear_type, target } => {
                modules::select_pet_gear_request(*gear_type, *target)
            }
            NetCommand::ConfigChange { preset_id } => simple_packet(
                CONFIG_CHANGE_REQUEST,
                vec![("presetId", Value::Int(*preset_id as i64))],
            ),
            NetCommand::QuestCancel { quest_id } => simple_packet(
                QUEST_CANCEL_REQUEST,
                vec![("questId", Value::Int(*quest_id as i64))],
            ),
            NetCommand::QuestList => simple_packet(QUEST_LIST_REQUEST, vec![]),
            NetCommand::QuestInfo { quest_id } => simple_packet(
                QUEST_INFO_REQUEST,
                vec![("questId", Value::Int(*quest_id as i64))],
            ),
            NetCommand::LabUpdate => modules::lab_update_request(),
            NetCommand::LabRefine { ore_type, count } => {
                modules::refine_ore_request(*ore_type, *count)
            }
            NetCommand::LabUpdateItem {
                item,
                ore_type,
                count,
            } => modules::lab_update_item_request(*item, *ore_type, *count),
            // Graceful close — the same request `portalJump` sends before
            // the expected disconnect.
            NetCommand::Logout => modules::channel_close(true),
        };
        Some(packet)
    }
}

/// Model position of a map entity by id.
fn entity_pos(world: &WorldState, id: i32) -> Option<(i32, i32)> {
    world.entities.get(&id).map(|e| (e.x, e.y))
}

/// `KillScreenRepairRequest` — `selection` picks the respawn option, and
/// `requestModule` re-authenticates (`Killscreen.repair` sets userID /
/// sessionID / factionID on a fresh `LoginRequest`).
pub fn repair_request(login: &LoginInfo, repair_type: i16) -> Packet {
    let selection = Packet {
        id: KILL_SCREEN_OPTION_TYPE_MODULE,
        name: registry::module_name(KILL_SCREEN_OPTION_TYPE_MODULE),
        fields: vec![(Some("repairTypeValue"), Value::Int(repair_type as i64))],
    };
    let request = modules::login_request(
        login.user_id,
        login.faction_id,
        &login.session_id,
        &login.version,
        login.instance_id,
        login.is_mini_client,
    );
    Packet {
        id: id::KILL_SCREEN_REPAIR_REQUEST,
        name: registry::module_name(id::KILL_SCREEN_REPAIR_REQUEST),
        fields: vec![
            (Some("selection"), Value::Module(Box::new(selection))),
            (Some("requestModule"), Value::Module(Box::new(request))),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quest_and_config_commands_encode() {
        use xenomit_packet_client::packet::encode_body;
        let world = WorldState::default();
        for (cmd, id) in [
            (
                NetCommand::ConfigChange { preset_id: 2 },
                CONFIG_CHANGE_REQUEST,
            ),
            (
                NetCommand::QuestCancel { quest_id: 77 },
                QUEST_CANCEL_REQUEST,
            ),
            (NetCommand::QuestList, QUEST_LIST_REQUEST),
            (NetCommand::QuestInfo { quest_id: 5 }, QUEST_INFO_REQUEST),
        ] {
            let p = cmd.to_packet(&world).expect("packet");
            assert_eq!(p.id, id);
            let body = encode_body(&p).expect("encodes against the schema");
            assert_eq!(i16::from_be_bytes([body[0], body[1]]), id);
        }
    }

    #[test]
    fn pet_commands_build_wire_packets() {
        let world = WorldState::default();
        let on = NetCommand::Pet { enabled: true }.to_packet(&world).unwrap();
        assert_eq!((on.id, on.int("petRequestType").unwrap()), (153, 0));
        let off = NetCommand::Pet { enabled: false }
            .to_packet(&world)
            .unwrap();
        assert_eq!(off.int("petRequestType").unwrap(), 1);

        let gear = NetCommand::PetGear {
            gear_type: 4,
            target: 99,
        }
        .to_packet(&world)
        .unwrap();
        assert_eq!(gear.id, 154);
        assert_eq!(gear.int("optParam").unwrap(), 99);
        assert_eq!(
            gear.module("gearTypeToActivate")
                .unwrap()
                .int("typeValue")
                .unwrap(),
            4
        );
    }
}
