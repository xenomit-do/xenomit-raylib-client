//! Chat / server message lines.
//!
//! * `0|n|MSG|<mode>|<scope>|<key>[|<replacements>]` — the stock
//!   `assembleDisplayMessage` (`ServerCommands.DISPLAY_MESSAGE`): a
//!   localized system message, optionally with `w:%X%,t:0,v:…` wildcard
//!   rows. Captured live: `0|n|MSG|3|2|msg_equip_not_ready|`.
//! * `L|n|MSG|…` — an in-game chat line as the old text protocol relays
//!   it (`<sender>|<text>` or a bare text). The real DarkOrbit chat runs
//!   over a separate chat server (`DOChatProxy`), which is out of scope;
//!   this only keeps the relayed lines out of the raw log.
//!
//! Lines land in [`SharedWorld::chat`] (a bounded ring) and are mirrored
//! into the HUD console log, already resolved.

use std::time::Instant;
use xenomit_packet_client::modules::id;
use xenomit_packet_client::packet::Packet;

use crate::locale::LocaleCatalog;
use crate::net::world::SharedWorld;

/// Chat lines kept (oldest dropped first).
pub const CHAT_CAP: usize = 200;

/// Where a line came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChatKind {
    /// Relayed player chat (`L|n|MSG|`).
    Player,
    /// Server system message (`0|n|MSG|`).
    System,
}

#[derive(Clone, Debug)]
pub struct ChatLine {
    pub at: Instant,
    pub kind: ChatKind,
    /// Empty for system lines / unattributed text.
    pub sender: String,
    pub text: String,
}

impl ChatLine {
    /// Console form: `[sender] text` or just the text.
    pub fn display(&self) -> String {
        if self.sender.is_empty() {
            self.text.clone()
        } else {
            format!("[{}] {}", self.sender, self.text)
        }
    }
}

/// One `w:…,t:…,v:…,p:…` wildcard row (`TextReplacement`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Replacement {
    pub wildcard: String,
    /// 0 plain, 1 number, 2 locale key, 3 seconds as a clock.
    pub ty: i32,
    pub value: String,
    pub precision: usize,
}

/// A parsed `0|n|MSG|` command.
#[derive(Clone, Debug, PartialEq)]
pub struct DisplayMessage {
    /// `param1[3]` — display mode (2 log-only, 3 popup, 4 popup+log).
    pub mode: i32,
    /// `param1[4]` — scope flag (1 = global notification family).
    pub scope: i32,
    /// `category:key` or a bare key.
    pub key: String,
    pub replacements: Vec<Replacement>,
}

impl DisplayMessage {
    /// The stock switch: scope 1 + mode 4, or other scope + mode 3,
    /// raise a notification on top of (or instead of) the log line.
    pub fn is_popup(&self) -> bool {
        (self.scope == 1 && self.mode == 4) || (self.scope != 1 && self.mode == 3)
    }

    /// Resolve the localized text and fold the wildcard rows in.
    pub fn render(&self, locale: &LocaleCatalog) -> String {
        let mut text = match self.key.split_once(':') {
            Some((cat, key)) => locale
                .lookup(&format!("{cat}.{key}"))
                .map(str::to_owned)
                .unwrap_or_else(|| key.to_owned()),
            None => locale.resolve(&self.key),
        };
        for r in &self.replacements {
            let value = match r.ty {
                1 => match r.value.trim().parse::<f64>() {
                    Ok(v) => locale.format_decimal(v, r.precision),
                    Err(_) => r.value.clone(),
                },
                2 => locale.resolve(&r.value),
                3 => match r.value.trim().parse::<i64>() {
                    Ok(s) => format_clock(s),
                    Err(_) => r.value.clone(),
                },
                _ => r.value.clone(),
            };
            text = text.replacen(&r.wildcard, &value, 1);
        }
        text
    }
}

fn format_clock(secs: i64) -> String {
    let s = secs.max(0);
    let (h, m, s) = (s / 3600, (s / 60) % 60, s % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m:02}:{s:02}")
    }
}

/// `TextReplacementList.parseRawTextReplacement`.
pub fn parse_replacements(raw: &str) -> Vec<Replacement> {
    let mut s = raw.trim();
    s = s.strip_prefix('[').unwrap_or(s);
    s = s.strip_suffix(']').unwrap_or(s);
    s = s.strip_prefix('{').unwrap_or(s);
    s = s.strip_suffix('}').unwrap_or(s);
    if s.is_empty() {
        return Vec::new();
    }
    s.split("},{")
        .map(|row| {
            let mut r = Replacement::default();
            for kv in row.split(',') {
                let Some((k, v)) = kv.split_once(':') else {
                    continue;
                };
                let v = v.trim();
                match k.trim() {
                    "w" => r.wildcard = v.to_owned(),
                    "t" => r.ty = v.parse().unwrap_or(0),
                    "v" => r.value = v.to_owned(),
                    "p" => r.precision = v.parse().unwrap_or(0),
                    _ => {}
                }
            }
            r
        })
        .filter(|r| !r.wildcard.is_empty())
        .collect()
}

/// Parse `0|n|MSG|…` (also accepts the `L|` prefix).
pub fn parse_display_message(msg: &str) -> Option<DisplayMessage> {
    let parts: Vec<&str> = msg.split('|').collect();
    if parts.len() < 6 || parts[1] != "n" || parts[2] != "MSG" || parts[5].is_empty() {
        return None;
    }
    Some(DisplayMessage {
        mode: parts[3].trim().parse().unwrap_or(0),
        scope: parts[4].trim().parse().unwrap_or(0),
        key: parts[5].to_owned(),
        replacements: parts
            .get(6)
            .map(|r| parse_replacements(r))
            .unwrap_or_default(),
    })
}

/// `L|n|MSG|<text>` or `L|n|MSG|<sender>|<text…>`.
pub fn parse_chat_line(msg: &str) -> Option<(String, String)> {
    let rest = msg.strip_prefix("L|n|MSG|")?;
    if rest.is_empty() {
        return None;
    }
    // A pipe inside the text must survive, so only the first one splits.
    match rest.split_once('|') {
        Some((sender, text)) if !sender.is_empty() && !text.is_empty() => {
            Some((sender.to_owned(), text.to_owned()))
        }
        _ => Some((String::new(), rest.trim_end_matches('|').to_owned())),
    }
}

pub fn handle(world: &mut SharedWorld, p: &Packet) {
    if p.id != id::LEGACY {
        return;
    }
    let Ok(msg) = p.string("message") else {
        return;
    };
    if msg.starts_with("L|") {
        if let Some((sender, text)) = parse_chat_line(msg) {
            world.push_chat(ChatKind::Player, sender, text);
        }
        return;
    }
    if let Some(m) = parse_display_message(msg) {
        let text = m.render(&world.locale);
        if m.is_popup() {
            world.push_toast(text.clone());
        }
        world.push_chat(ChatKind::System, String::new(), text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captured_display_message_parses() {
        let m = parse_display_message("0|n|MSG|3|2|msg_equip_not_ready|").unwrap();
        assert_eq!((m.mode, m.scope), (3, 2));
        assert_eq!(m.key, "msg_equip_not_ready");
        assert!(m.replacements.is_empty());
        // scope != 1 && mode == 3 -> popup
        assert!(m.is_popup());
        let loc = LocaleCatalog::default();
        assert_eq!(m.render(&loc), "msg_equip_not_ready");
    }

    #[test]
    fn replacements_all_types() {
        let rows = parse_replacements("[{w:%A%,t:0,v:foo},{w:%B%,t:1,v:12,p:0},{w:%C%,t:3,v:75}]");
        assert_eq!(rows.len(), 3);
        let m = DisplayMessage {
            mode: 2,
            scope: 2,
            key: "x %A% %B% %C%".into(),
            replacements: rows,
        };
        assert_eq!(m.render(&LocaleCatalog::default()), "x foo 12 01:15");
        assert!(!m.is_popup());
    }

    #[test]
    fn chat_lines_split_sender() {
        assert_eq!(
            parse_chat_line("L|n|MSG|bob|hello | world"),
            Some(("bob".into(), "hello | world".into()))
        );
        assert_eq!(
            parse_chat_line("L|n|MSG|hi there"),
            Some((String::new(), "hi there".into()))
        );
        assert_eq!(parse_chat_line("0|n|MSG|3|2|k|"), None);
    }
}
