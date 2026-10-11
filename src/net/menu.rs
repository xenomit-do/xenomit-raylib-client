use std::collections::HashMap;
use xenomit_packet_client::packet::{Packet, Value};

use crate::net::world::SharedWorld;

/// `MenuInitializationCommand` — full session-scope item/slot-bar
/// model (categories + slot bars); replaces `SharedWorld::menu`.
pub(crate) const MENU_INITIALIZATION: i16 = -8140;
/// `MenuItemStatusCommand` — standalone status update for one menu
/// item (also nested as `MenuItemInitializationCommand.status`).
pub(crate) const MENU_ITEM_STATUS: i16 = 24049;
/// `SlotBarInitializationCommand` (element of `slotBars`).
const SLOT_BAR_INIT_MODULE: i16 = -620;
/// `SlotInfoCommand` (element of `slots`) — `slotId` is 1-based.
const SLOT_INFO_MODULE: i16 = -20408;
/// `MenuCategoryInitializationCommand` (element of `categories`).
const MENU_CATEGORY_MODULE: i16 = -3461;
/// `MenuItemInitializationCommand` (element of `menuItems`).
const MENU_ITEM_INIT_MODULE: i16 = 23633;

/// The default hotkey bar's `slotBarId` (`premiumSlotBar` /
/// `proActionBar` are stored too but take no keys).
pub const STANDARD_SLOT_BAR: &str = "standardSlotBar";
/// Slot ids run 1..=10 on every bar (`SlotInfoCommand.slotId`).
pub const SLOT_BAR_SLOTS: usize = 10;

/// `actionStyle` of a `MenuItemInitializationCommand` — drives which
/// `MenuActionRequest.actionType` a slot click/hotkey sends.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MenuActionStyle {
    /// `actionStyle` 0 — fires once per activation.
    #[default]
    OneShot,
    /// `actionStyle` 1 — toggles on/off.
    Toggle,
    /// `actionStyle` 2 — selects this option (ammo selection).
    Selection,
    /// `actionStyle` 3 — cooldown/timer item.
    Timer,
}

impl MenuActionStyle {
    /// `actionStyle` wire value (unknown → one-shot).
    pub fn from_wire(v: i64) -> Self {
        match v {
            1 => Self::Toggle,
            2 => Self::Selection,
            3 => Self::Timer,
            _ => Self::OneShot,
        }
    }

    /// `MenuActionRequest.actionType` for a slot-bar click/hotkey
    /// (`SendHotKeyActionRequestCommand`: selection → ACTIVATE(1),
    /// toggle → SELECT(0), oneShot → ACTIVATE(1), timer → 0).
    pub fn action_type(self) -> i16 {
        match self {
            Self::OneShot | Self::Selection => 1,
            Self::Toggle | Self::Timer => 0,
        }
    }
}

/// One menu item — `MenuItemInitializationCommand` fields plus the
/// current `MenuItemStatusCommand` state (a standalone status update
/// replaces the status in place, keeping category/actionStyle).
#[derive(Clone, Debug, Default)]
pub struct MenuItem {
    /// `menuItemId` (e.g. `"ammunition_laser_ucb-100"`).
    pub id: String,
    /// Owning `categoryId` ("" when only a status update was seen).
    pub category: String,
    /// `actionStyle` — see [`MenuActionStyle`].
    pub action_style: MenuActionStyle,
    /// `counterValue` (ammo count etc.).
    pub counter_value: f64,
    /// `maxCounterValue`.
    pub max_counter_value: f64,
    /// `selected` — highlighted slot (active ammo/config).
    pub selected: bool,
    /// `available`.
    pub available: bool,
    /// `blocked` — greyed out (cooldown/requirement).
    pub blocked: bool,
    /// `activatable`.
    pub activatable: bool,
    /// `visible`.
    pub visible: bool,
    /// `iconLootId` — loot icon key (icon atlas not implemented).
    pub icon_loot_id: String,
}

/// Session-scope menu model (`MenuInitializationCommand`): every known
/// item plus each slot bar's `slotId → menuItemId` mapping. Not cleared
/// on map change — the menu is account data, not map data.
#[derive(Clone, Debug, Default)]
pub struct MenuState {
    /// `menuItemId` → item (from `categories` + standalone status
    /// updates).
    pub items: HashMap<String, MenuItem>,
    /// `slotBarId` → 10 slots (index = `slotId - 1`; `None` = empty —
    /// a slot whose id isn't among the category items is an EMPTY slot,
    /// `ParseItemsControlMenuDataCommand` semantics).
    pub slot_bars: HashMap<String, [Option<String>; SLOT_BAR_SLOTS]>,
}

impl MenuState {
    /// Item behind `bar` slot `slot` (1-based) — `None` when the bar is
    /// unknown, the slot is out of range/empty, or the id isn't in the
    /// item table.
    pub fn slot_item(&self, bar: &str, slot: usize) -> Option<&MenuItem> {
        let id = self
            .slot_bars
            .get(bar)?
            .get(slot.checked_sub(1)?)?
            .as_ref()?;
        self.items.get(id)
    }
}

pub fn handle(world: &mut SharedWorld, p: &Packet) {
    match p.id {
        MENU_INITIALIZATION => {
            world.menu = parse_menu_init(p);
            ::log::info!(
                "menu init: {} items, {} slot bars",
                world.menu.items.len(),
                world.menu.slot_bars.len()
            );
        }
        MENU_ITEM_STATUS => {
            if let Ok(item_id) = p.string("menuItemId") {
                let item = world
                    .menu
                    .items
                    .entry(item_id.to_owned())
                    .or_insert_with(|| MenuItem {
                        id: item_id.to_owned(),
                        ..Default::default()
                    });
                apply_menu_status(item, p);
            }
        }
        _ => {}
    }
}

/// `MenuInitializationCommand` → full `MenuState`: categories first
/// (each `menuItems` row plus its nested `status`), then the slot bars.
fn parse_menu_init(p: &Packet) -> MenuState {
    let mut menu = MenuState::default();
    for v in p.list("categories").unwrap_or(&[]) {
        let Value::Module(cat) = v else {
            continue;
        };
        if cat.id != MENU_CATEGORY_MODULE {
            continue;
        }
        let category_id = cat.string("categoryId").unwrap_or("");
        for iv in cat.list("menuItems").unwrap_or(&[]) {
            let Value::Module(im) = iv else {
                continue;
            };
            if im.id != MENU_ITEM_INIT_MODULE {
                continue;
            }
            let Ok(status) = im.module("status") else {
                continue;
            };
            let Ok(id) = status.string("menuItemId") else {
                continue;
            };
            let mut item = MenuItem {
                id: id.to_owned(),
                category: category_id.to_owned(),
                action_style: MenuActionStyle::from_wire(im.int("actionStyle").unwrap_or(0)),
                ..Default::default()
            };
            apply_menu_status(&mut item, status);
            menu.items.insert(id.to_owned(), item);
        }
    }
    for v in p.list("slotBars").unwrap_or(&[]) {
        let Value::Module(bar) = v else {
            continue;
        };
        if bar.id != SLOT_BAR_INIT_MODULE {
            continue;
        }
        let Ok(bar_id) = bar.string("slotBarId") else {
            continue;
        };
        let mut slots: [Option<String>; SLOT_BAR_SLOTS] = Default::default();
        for sv in bar.list("slots").unwrap_or(&[]) {
            let Value::Module(s) = sv else {
                continue;
            };
            if s.id != SLOT_INFO_MODULE {
                continue;
            }
            let slot = s.int("slotId").unwrap_or(0);
            let item_id = s.string("menuItemId").unwrap_or("");
            if (1..=SLOT_BAR_SLOTS as i64).contains(&slot) && menu.items.contains_key(item_id) {
                slots[slot as usize - 1] = Some(item_id.to_owned());
            }
        }
        menu.slot_bars.insert(bar_id.to_owned(), slots);
    }
    menu
}

/// Copy the `MenuItemStatusCommand` fields into `item` — shared by the
/// init path (nested `status` module) and standalone updates.
fn apply_menu_status(item: &mut MenuItem, status: &Packet) {
    item.counter_value = status.f64("counterValue").unwrap_or(0.0);
    item.max_counter_value = status.f64("maxCounterValue").unwrap_or(0.0);
    item.selected = status.boolean("selected").unwrap_or(false);
    item.available = status.boolean("available").unwrap_or(false);
    item.blocked = status.boolean("blocked").unwrap_or(false);
    item.activatable = status.boolean("activatable").unwrap_or(false);
    item.visible = status.boolean("visible").unwrap_or(false);
    item.icon_loot_id = status.string("iconLootId").unwrap_or("").to_owned();
}
