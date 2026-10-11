//! Client display settings pushed by the server.
//!
//! The stock client keeps these in `Settings.as` and fills them from
//! `UserSettingsCommand` (the new `DisplaySettingsModule`) and the legacy
//! `SettingsOldFlashCommand` (`displayAllLasers`, `simpleGates`,
//! `simpleOpponents` only exist there). `SettingSetCommand` is a bare
//! `key`/`value` pair with no stock handler in the decompile; we accept
//! the same flag names there too (best effort) so a server that echoes a
//! toggle keeps the local copy in sync.
//!
//! Defaults show everything (`simple_*` are the "reduced detail"
//! switches, so they default to off).

use xenomit_packet_client::packet::Packet;

use crate::net::world::SharedWorld;

/// `SettingSetCommand` — one `key`/`value` pair.
pub(crate) const SETTING_SET: i16 = 85;
/// `SettingsOldFlashCommand` — the legacy flag bundle.
pub(crate) const SETTINGS_OLD_FLASH: i16 = 88;
/// `UserSettingsCommand` — display/audio/gameplay/... modules.
pub(crate) const USER_SETTINGS: i16 = -27265;

/// Flags consumed by the renderer. `Default` = current behaviour.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DisplaySettings {
    /// Draw drones of other ships (`Settings.displayDrones`).
    pub display_drones: bool,
    /// Draw the hero's own drones (`displayMyDrones`).
    pub display_my_drones: bool,
    /// Draw player name plates (NPC plates are unaffected).
    pub display_player_name: bool,
    /// Floating damage numbers (`displayHitpointBubbles`).
    pub display_hitpoint_bubbles: bool,
    /// Show laser bolts between *other* ships, not just the hero's.
    pub display_all_lasers: bool,
    /// Fill behind the minimap (`displayMinimapBackground`).
    pub display_minimap_background: bool,
    /// Simplify the gate shell while keeping lightweight authored idle effects.
    pub simple_gates: bool,
    /// Draw other players' ships as plain dots instead of sprites.
    pub simple_opponents: bool,
}

impl Default for DisplaySettings {
    fn default() -> Self {
        Self {
            display_drones: true,
            display_my_drones: true,
            display_player_name: true,
            display_hitpoint_bubbles: true,
            display_all_lasers: true,
            display_minimap_background: true,
            simple_gates: false,
            simple_opponents: false,
        }
    }
}

/// One toggleable row of the F3 settings overlay.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingFlag {
    DisplayDrones,
    DisplayMyDrones,
    DisplayPlayerName,
    DisplayHitpointBubbles,
    DisplayAllLasers,
    DisplayMinimapBackground,
    SimpleGates,
    SimpleOpponents,
}

impl SettingFlag {
    /// Overlay order.
    pub const ALL: [SettingFlag; 8] = [
        SettingFlag::DisplayPlayerName,
        SettingFlag::DisplayHitpointBubbles,
        SettingFlag::DisplayDrones,
        SettingFlag::DisplayMyDrones,
        SettingFlag::DisplayAllLasers,
        SettingFlag::DisplayMinimapBackground,
        SettingFlag::SimpleGates,
        SettingFlag::SimpleOpponents,
    ];

    pub fn label(self) -> &'static str {
        match self {
            SettingFlag::DisplayDrones => "Show drones",
            SettingFlag::DisplayMyDrones => "Show my drones",
            SettingFlag::DisplayPlayerName => "Show player names",
            SettingFlag::DisplayHitpointBubbles => "Show damage numbers",
            SettingFlag::DisplayAllLasers => "Show all lasers",
            SettingFlag::DisplayMinimapBackground => "Minimap background",
            SettingFlag::SimpleGates => "Simple gate shell",
            SettingFlag::SimpleOpponents => "Simple opponents",
        }
    }
}

impl DisplaySettings {
    pub fn get(&self, flag: SettingFlag) -> bool {
        match flag {
            SettingFlag::DisplayDrones => self.display_drones,
            SettingFlag::DisplayMyDrones => self.display_my_drones,
            SettingFlag::DisplayPlayerName => self.display_player_name,
            SettingFlag::DisplayHitpointBubbles => self.display_hitpoint_bubbles,
            SettingFlag::DisplayAllLasers => self.display_all_lasers,
            SettingFlag::DisplayMinimapBackground => self.display_minimap_background,
            SettingFlag::SimpleGates => self.simple_gates,
            SettingFlag::SimpleOpponents => self.simple_opponents,
        }
    }

    pub fn set(&mut self, flag: SettingFlag, value: bool) {
        match flag {
            SettingFlag::DisplayDrones => self.display_drones = value,
            SettingFlag::DisplayMyDrones => self.display_my_drones = value,
            SettingFlag::DisplayPlayerName => self.display_player_name = value,
            SettingFlag::DisplayHitpointBubbles => self.display_hitpoint_bubbles = value,
            SettingFlag::DisplayAllLasers => self.display_all_lasers = value,
            SettingFlag::DisplayMinimapBackground => self.display_minimap_background = value,
            SettingFlag::SimpleGates => self.simple_gates = value,
            SettingFlag::SimpleOpponents => self.simple_opponents = value,
        }
    }

    pub fn toggle(&mut self, flag: SettingFlag) {
        let v = self.get(flag);
        self.set(flag, !v);
    }

    /// Apply a `SettingSetCommand` pair; returns false for unknown keys.
    pub fn apply_key(&mut self, key: &str, value: &str) -> bool {
        let Some(flag) = flag_for_key(key) else {
            return false;
        };
        let v = matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        );
        self.set(flag, v);
        true
    }

    /// Fold `DisplaySettingsModule` (skipped while `notSet`).
    fn apply_display_module(&mut self, m: &Packet) {
        if m.boolean("notSet").unwrap_or(false) {
            return;
        }
        let take = |name: &str, flag: SettingFlag, me: &mut Self| {
            if let Ok(v) = m.boolean(name) {
                me.set(flag, v);
            }
        };
        take("displayPlayerName", SettingFlag::DisplayPlayerName, self);
        take(
            "displayHitpointBubbles",
            SettingFlag::DisplayHitpointBubbles,
            self,
        );
        take("displayDrones", SettingFlag::DisplayDrones, self);
        take("displayMyDrones", SettingFlag::DisplayMyDrones, self);
        take(
            "displayMinimapBackground",
            SettingFlag::DisplayMinimapBackground,
            self,
        );
    }

    /// Fold `SettingsOldFlashCommand`.
    fn apply_old_flash(&mut self, p: &Packet) {
        let take = |name: &str, flag: SettingFlag, me: &mut Self| {
            if let Ok(v) = p.boolean(name) {
                me.set(flag, v);
            }
        };
        take("displayPlayerName", SettingFlag::DisplayPlayerName, self);
        take("displayAllLasers", SettingFlag::DisplayAllLasers, self);
        take("simpleGates", SettingFlag::SimpleGates, self);
        take("simpleOpponents", SettingFlag::SimpleOpponents, self);
        take(
            "displayHitpointsBubbles",
            SettingFlag::DisplayHitpointBubbles,
            self,
        );
        take("dsplyDrones", SettingFlag::DisplayDrones, self);
    }
}

/// Setting key (several spellings seen across the old/new settings
/// stacks) -> flag.
fn flag_for_key(key: &str) -> Option<SettingFlag> {
    Some(match key.trim().to_ascii_lowercase().as_str() {
        "displaydrones" | "dsplydrones" => SettingFlag::DisplayDrones,
        "displaymydrones" => SettingFlag::DisplayMyDrones,
        "displayplayername" => SettingFlag::DisplayPlayerName,
        "displayhitpointbubbles" | "displayhitpointsbubbles" => SettingFlag::DisplayHitpointBubbles,
        "displayalllasers" => SettingFlag::DisplayAllLasers,
        "displayminimapbackground" => SettingFlag::DisplayMinimapBackground,
        "simplegates" => SettingFlag::SimpleGates,
        "simpleopponents" => SettingFlag::SimpleOpponents,
        _ => return None,
    })
}

pub fn handle(world: &mut SharedWorld, p: &Packet) {
    match p.id {
        USER_SETTINGS => {
            if let Ok(m) = p.module("displaySettingsModule") {
                world.settings.apply_display_module(m);
                ::log::debug!("settings: user display module -> {:?}", world.settings);
            }
        }
        SETTINGS_OLD_FLASH => {
            world.settings.apply_old_flash(p);
            ::log::debug!("settings: old flash flags -> {:?}", world.settings);
        }
        SETTING_SET => {
            if let (Ok(k), Ok(v)) = (p.string("key"), p.string("value")) {
                if world.settings.apply_key(k, v) {
                    ::log::debug!("settings: {k}={v} -> {:?}", world.settings);
                }
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xenomit_packet_client::packet::Value;

    fn pkt(id: i16, name: &'static str, fields: Vec<(&'static str, Value)>) -> Packet {
        Packet {
            id,
            name: Some(name),
            fields: fields.into_iter().map(|(n, v)| (Some(n), v)).collect(),
        }
    }

    #[test]
    fn defaults_show_everything() {
        let s = DisplaySettings::default();
        assert!(s.display_drones && s.display_player_name && s.display_all_lasers);
        assert!(s.display_hitpoint_bubbles && s.display_minimap_background);
        assert!(!s.simple_gates && !s.simple_opponents);
    }

    #[test]
    fn user_settings_display_module_applies_unless_not_set() {
        let mut w = SharedWorld::default();
        let display = |not_set: bool| {
            Value::Module(Box::new(pkt(
                270,
                "DisplaySettingsModule",
                vec![
                    ("notSet", Value::Bool(not_set)),
                    ("displayPlayerName", Value::Bool(false)),
                    ("displayDrones", Value::Bool(false)),
                    ("displayHitpointBubbles", Value::Bool(false)),
                    ("displayMinimapBackground", Value::Bool(false)),
                ],
            )))
        };
        w.apply_packet(&pkt(
            USER_SETTINGS,
            "UserSettingsCommand",
            vec![("displaySettingsModule", display(true))],
        ));
        assert_eq!(w.settings, DisplaySettings::default());
        w.apply_packet(&pkt(
            USER_SETTINGS,
            "UserSettingsCommand",
            vec![("displaySettingsModule", display(false))],
        ));
        assert!(!w.settings.display_player_name);
        assert!(!w.settings.display_drones);
        assert!(!w.settings.display_hitpoint_bubbles);
        assert!(!w.settings.display_minimap_background);
        assert!(w.settings.display_all_lasers);
    }

    #[test]
    fn old_flash_sets_gate_flags() {
        let mut w = SharedWorld::default();
        w.apply_packet(&pkt(
            SETTINGS_OLD_FLASH,
            "SettingsOldFlashCommand",
            vec![
                ("displayAllLasers", Value::Bool(false)),
                ("simpleGates", Value::Bool(true)),
                ("simpleOpponents", Value::Bool(true)),
                ("dsplyDrones", Value::Bool(false)),
            ],
        ));
        assert!(!w.settings.display_all_lasers);
        assert!(w.settings.simple_gates && w.settings.simple_opponents);
        assert!(!w.settings.display_drones);
    }

    #[test]
    fn setting_set_pair_and_toggle() {
        let mut w = SharedWorld::default();
        w.apply_packet(&pkt(
            SETTING_SET,
            "SettingSetCommand",
            vec![
                ("key", Value::Str("simpleGates".into())),
                ("value", Value::Str("1".into())),
            ],
        ));
        assert!(w.settings.simple_gates);
        w.apply_packet(&pkt(
            SETTING_SET,
            "SettingSetCommand",
            vec![
                ("key", Value::Str("unknown".into())),
                ("value", Value::Str("1".into())),
            ],
        ));
        w.settings.toggle(SettingFlag::SimpleGates);
        assert!(!w.settings.simple_gates);
    }
}
