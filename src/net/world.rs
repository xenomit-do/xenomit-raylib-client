//! Shared world model: packet-client `WorldState` + presentation metadata.
//!
//! `WorldState` already folds every relevant inbound module (hero, ships,
//! collectables, POIs, laser links, vitals). `SharedWorld` wraps it with
//! per-entity presentation data the renderer needs — movement interpolation
//! anchors and spawn/despawn timestamps — computed by observing packet
//! edges on the pump thread.

use std::collections::{HashMap, HashSet};
use std::time::Instant;
use xenomit_packet_client::model::WorldState;
use xenomit_packet_client::modules::id;
use xenomit_packet_client::packet::Packet;

use crate::locale::LocaleCatalog;
use crate::net::log::{is_reward, LOG_CAP, TOAST_CAP};
use std::sync::Arc;

/// Toasts older than this are pruned on the next push (the HUD has
/// already faded them out by then).
const TOAST_MAX_AGE_SECS: f32 = 6.0;
use crate::net::movement::{handle_pre, hero_move_ms};

pub use crate::net::attributes::{Booster, HeroExtras};
pub use crate::net::chat::{ChatKind, ChatLine, CHAT_CAP};
pub use crate::net::combat::{EmpKind, StatusFx, WorldFx};
pub use crate::net::contacts::{Contact, ContactList};
pub use crate::net::drones::{DroneEntry, DroneLoadout};
pub use crate::net::group::{GroupMember, GroupState};
pub use crate::net::killscreen::{KillScreen, KillScreenOption};
pub use crate::net::log::{game_log, LogLine, Toast};
pub use crate::net::menu::{
    MenuActionStyle, MenuItem, MenuState, SLOT_BAR_SLOTS, STANDARD_SLOT_BAR,
};
pub use crate::net::movement::MoveTween;
pub use crate::net::pet::{PetGear, PetState, PetTable};
pub use crate::net::portals::Portal;
pub use crate::net::quests::QuestLog;
pub use crate::net::refinery::{LabItem, LabItemKind, LabState};
pub use crate::net::settings::{DisplaySettings, SettingFlag};
pub use crate::net::ships::ShipInfo;

#[derive(Default)]
pub struct SharedWorld {
    pub world: WorldState,
    /// entity id -> active movement tween (hero id = `world.hero.user_id`).
    pub tweens: HashMap<i32, MoveTween>,
    /// Last `BeaconCommand` position — tells a converging server update
    /// (beacon still advancing) from a stopped one.
    pub hero_beacon: Option<(f32, f32)>,
    /// Monotonic log lines for the HUD console — each carries its
    /// arrival time so old lines can fade out like the stock chat.
    pub log: Vec<LogLine>,
    /// Reward toasts (resolved loot lines) for the top-center HUD stack;
    /// the renderer fades them out, `push_toast` bounds the queue.
    pub toasts: Vec<Toast>,
    /// Language table used to resolve server keys (log lines, titles)
    /// and format numbers. Empty (identity) until the asset bootstrap
    /// installs the real one via [`Self::set_locale`].
    pub locale: Arc<LocaleCatalog>,
    /// Set when the socket dies — game shows a disconnect banner.
    pub disconnected: Option<String>,
    /// Jumpgates on the current map (`JumpgateCreateCommand`), keyed by
    /// `gateId` — the WorldState model does not track them.
    pub portals: HashMap<i32, Portal>,
    /// When the current map's lifetime started (`ScreenManager.map.lifeTime`).
    /// Gates that appear more than 10s later play `spawn_animation`.
    pub map_started: Option<Instant>,
    /// `gateId` → `JumpInitiatedCommand` time. `Portal.jump(10)` fades the
    /// jump clip in, holds, then fades it out.
    pub portal_jumps: HashMap<i32, Instant>,
    /// `ShipSelectionCommand`/`AttackHitCommand` nano-hull overlay —
    /// `Entity` has no nano-hull fields, so they live here keyed by uid.
    pub entity_nano: HashMap<i32, (i32, i32)>,
    /// `AssetCreateCommand.type.typeValue` per asset entity id — `Entity`
    /// only keeps `designId`, but the uberlib key is derived from the
    /// asset *type* (`AssetCreateHandlerCommand.MAP`:
    /// `48`→`hangar_home_<faction>`, `46/47`→`headquarters_*`, …).
    pub entity_asset_type: HashMap<i32, i32>,
    /// Name-plate data per ship id (`ShipInfo`) — hero id included.
    pub ship_info: HashMap<i32, ShipInfo>,
    /// Drone loadout per ship id (`LegacyModule` `n|d` +
    /// `DroneFormationChangeCommand`) — hero id included.
    pub drones: HashMap<i32, DroneLoadout>,
    /// P.E.T. companions (hero's pet + other players' on the map).
    pub pets: PetTable,
    /// The server is about to close this socket on purpose
    /// (`JumpInitiatedCommand`/`JumpToServerCommand` —
    /// `Main.scheduledDisconnect`): reconnect instead of erroring.
    pub expect_disconnect: bool,
    /// `JumpInitiatedCommand.mapId` — the map a jump is heading to (for
    /// the console log).
    pub jump_target: Option<i32>,
    /// Combat-fx events pending pickup by `Game::update` (drained each
    /// frame — they expire if never consumed).
    pub fx: Vec<WorldFx>,
    /// Kill-screen state while the hero is dead; `Some` shows the overlay
    /// (`ShipDestroyed` for the hero or `KillScreenPost` opens it).
    pub kill_screen: Option<KillScreen>,
    /// Session-scope menu/item slot bar (`MenuInitializationCommand`)
    /// — ammo/rocket/item slots; survives map changes.
    pub menu: MenuState,
    /// Hero attribute side-channels: active config, ore breakdown,
    /// boosters (session scope).
    pub extras: HeroExtras,
    /// Quest log (session scope — the server re-sends it on login).
    pub quests: QuestLog,
    /// Refinery/lab upgrade state (`LabUpdateItemCommand`); session
    /// scope — refreshed via `LabUpdateRequest` while the window is open.
    pub lab: LabState,
    /// Display flags (`UserSettingsCommand` / `SettingsOldFlashCommand`)
    /// — session scope, defaults show everything.
    pub settings: DisplaySettings,
    /// Current group roster (`GroupInitializationCommand`…); session scope.
    pub group: Option<GroupState>,
    /// Contact/friend list session state.
    pub contacts: ContactList,
    /// Chat + server message lines, bounded (`chat::CHAT_CAP`).
    pub chat: Vec<ChatLine>,
    /// Targets grayed out by `0|n|LSH` (neutral lock colour).
    pub gray_targets: HashSet<i32>,
    /// Players flagged by `0|n|pvp|uid|0` (ceasefire loss marker).
    pub criminals: HashSet<i32>,
    /// Collectable hashes removed by a map event (mine/firework went
    /// off) — their disappearance must not play the pickup beam.
    pub silent_removals: HashSet<String>,
    /// `Settings.combatMusicActive` — set by `CombatMusicCommand`
    /// (module -12650): `combatEvent` 0 = combat start, 1 = end. Global
    /// like stock's Settings, so `clear_map_state` keeps it.
    pub combat_music_active: bool,
    /// `0|n|sss|a|b` spaceball speed pair (HUD stub).
    pub spaceball_speed: Option<(i32, i32)>,
}

impl SharedWorld {
    /// Append a chat/system line and mirror it into the console log.
    pub fn push_chat(&mut self, kind: ChatKind, sender: String, text: String) {
        let line = ChatLine {
            at: Instant::now(),
            kind,
            sender,
            text,
        };
        self.push_log(line.display());
        self.chat.push(line);
        if self.chat.len() > CHAT_CAP {
            let excess = self.chat.len() - CHAT_CAP;
            self.chat.drain(..excess);
        }
    }

    /// Interpolated position of an entity (falls back to model pos).
    ///
    /// The hero's id is `world.hero.user_id`; the hero's own entity row
    /// stays at `(0, 0)`, so the hero must resolve via `hero` directly.
    pub fn pos(&self, id: i32) -> (f32, f32) {
        if let Some(t) = self.tweens.get(&id) {
            return t.pos();
        }
        if let Some(h) = &self.world.hero {
            if h.user_id == id {
                return (h.x as f32, h.y as f32);
            }
        }
        if let Some(e) = self.world.entities.get(&id) {
            return (e.x as f32, e.y as f32);
        }
        (0.0, 0.0)
    }

    /// Hero's interpolated position, if the hero exists.
    pub fn hero_pos(&self) -> Option<(f32, f32)> {
        self.world.hero.as_ref().map(|h| self.pos(h.user_id))
    }

    /// Write the interpolated hero position back into the model so
    /// position-dependent request checksums (`hero.position` in the AS3
    /// client) see the predicted pos, not the last server update.
    pub fn sync_hero_model(&mut self) {
        if let Some((x, y)) = self.hero_pos() {
            if let Some(h) = &mut self.world.hero {
                h.x = x.round() as i32;
                h.y = y.round() as i32;
            }
        }
    }

    /// Locally predicted hero move: the server answers a `MoveRequest`
    /// with `BeaconCommand`s only, so the tween is driven from here —
    /// the same `moveTrait.goto` prediction the stock client applies in
    /// `EventManager.queueMoveRequest`.
    pub fn start_hero_move(&mut self, target: (i32, i32)) {
        let Some(h) = &self.world.hero else {
            return;
        };
        let uid = h.user_id;
        let from = self.pos(uid);
        let to = (target.0 as f32, target.1 as f32);
        let duration_ms = hero_move_ms(h.speed, from, to);
        self.hero_beacon = None;
        self.tweens.insert(
            uid,
            MoveTween {
                from,
                to,
                duration_ms,
                started: Instant::now(),
            },
        );
    }

    /// Append a console line, keeping the log bounded.
    pub fn push_log(&mut self, line: impl Into<String>) {
        self.log.push(LogLine {
            at: Instant::now(),
            text: line.into(),
        });
        if self.log.len() > LOG_CAP {
            let excess = self.log.len() - LOG_CAP;
            self.log.drain(..excess);
        }
    }

    /// Append a reward toast, keeping the stack bounded.
    pub fn push_toast(&mut self, line: impl Into<String>) {
        self.toasts
            .retain(|t| t.at.elapsed().as_secs_f32() < TOAST_MAX_AGE_SECS);
        self.toasts.push(Toast {
            at: Instant::now(),
            text: line.into(),
        });
        if self.toasts.len() > TOAST_CAP {
            let excess = self.toasts.len() - TOAST_CAP;
            self.toasts.drain(..excess);
        }
    }

    /// Install the language table and re-resolve what was captured as raw
    /// keys before it was available (ship titles).
    pub fn set_locale(&mut self, locale: Arc<LocaleCatalog>) {
        self.locale = locale;
        for info in self.ship_info.values_mut() {
            if !info.title_key.is_empty() {
                info.title = crate::net::ships::title_text(&self.locale, &info.title_key);
            }
        }
    }

    /// Drop every per-map table — used on the map-change edge (jump,
    /// portal respawn): the server re-sends entities, collectables,
    /// portals, POIs and laser links for the new map. The hero row is
    /// re-seeded (like `on_ship_init` does) so `pos`/hit-test keep working
    /// until the fresh `ShipInitializationCommand` arrives.
    pub fn clear_map_state(&mut self) {
        self.world.entities.clear();
        self.world.collectables.clear();
        self.world.pois.clear();
        self.world.laser_links.clear();
        self.world.laser_active.clear();
        self.world.map_flags = Default::default();
        self.portals.clear();
        self.portal_jumps.clear();
        self.map_started = Some(Instant::now());
        self.tweens.clear();
        self.entity_nano.clear();
        self.entity_asset_type.clear();
        let hero_id = self.world.hero.as_ref().map(|h| h.user_id);
        self.ship_info.retain(|uid, _| Some(*uid) == hero_id);
        self.drones.retain(|uid, _| Some(*uid) == hero_id);
        self.pets.clear();
        self.hero_beacon = None;
        self.fx.clear();
        self.gray_targets.clear();
        self.criminals.clear();
        self.silent_removals.clear();
        self.kill_screen = None;
        if let Some(h) = &self.world.hero {
            self.world.entities.entry(h.user_id).or_default().id = h.user_id;
        }
    }

    /// Fold one decoded inbound packet into the model and update the
    /// presentation metadata (movement tweens, console log).
    ///
    /// Tween `from` anchors are sampled *before* `world.apply` mutates the
    /// model (notably `HeroMoveCommand` overwrites `hero.x/y` with the
    /// destination, which would collapse `from` onto `to`).
    pub fn apply_packet(&mut self, p: &Packet) {
        let now = Instant::now();
        let mut new_tween: Option<(i32, MoveTween)> = None;
        let mut hero_snap: Option<(i32, i32)> = None;

        if let Some(effect) = handle_pre(self, p, now) {
            new_tween = effect.new_tween;
            hero_snap = effect.hero_snap;
        } else {
            crate::net::ships::handle(self, p);
            crate::net::drones::handle(self, p);
            crate::net::pet::handle(self, p);
            crate::net::menu::handle(self, p);
            crate::net::attributes::handle(self, p);
            crate::net::quests::handle(self, p);
            crate::net::combat::handle(self, p);
            crate::net::portals::handle(self, p);
            crate::net::killscreen::handle(self, p);
            crate::net::settings::handle(self, p);
            crate::net::group::handle(self, p);
            crate::net::contacts::handle(self, p);
            crate::net::refinery::handle(self, p);
            crate::net::legacy::handle(self, p);
            crate::net::chat::handle(self, p);
            match p.id {
                id::ADD_BOX
                | id::ADD_ORE
                | id::ADD_BEACON
                | id::ADD_MINE
                | id::ADD_FIREWORK_BOX => {
                    if let (Ok(x), Ok(y)) = (p.int("x"), p.int("y")) {
                        ::log::debug!(
                            "collectable {} id={} at ({x},{y})",
                            p.name.unwrap_or("?"),
                            p.string("hash")
                                .map(|s| s.to_owned())
                                .or_else(|_| p.int("hash").map(|h| h.to_string()))
                                .unwrap_or_else(|_| "?".into())
                        );
                    }
                }
                _ => {}
            }
        }

        self.world.apply(p);
        if p.id == id::SHIP_CREATE {
            if let Ok(uid) = p.int("userId") {
                match self.world.entities.get(&(uid as i32)) {
                    Some(e) => ::log::debug!(
                        "ship {uid} stored: npc={} kind={:?} pos=({},{})",
                        e.npc,
                        e.kind,
                        e.x,
                        e.y
                    ),
                    None => ::log::debug!("ship {uid} NOT stored by model apply"),
                }
            }
        }
        if let Some((x, y)) = hero_snap {
            if let Some(h) = &mut self.world.hero {
                h.x = x;
                h.y = y;
            }
        }
        if let Some((uid, tween)) = new_tween {
            self.tweens.insert(uid, tween);
        }
        if let Some(line) = game_log(p, &self.locale) {
            if is_reward(p) {
                self.push_toast(line.clone());
            }
            self.push_log(line);
        }
    }
}
