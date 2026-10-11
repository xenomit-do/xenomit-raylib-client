use crate::locale::LocaleCatalog;
use std::time::Instant;
use xenomit_packet_client::packet::{Packet, Value};

/// Keep the last N console lines for the HUD.
pub(crate) const LOG_CAP: usize = 500;

/// Reward toasts kept for the HUD stack (oldest dropped first).
pub(crate) const TOAST_CAP: usize = 8;

/// One console line with its arrival time — the HUD fades lines out
/// after ~10 s like the stock chat/log.
#[derive(Debug)]
pub struct LogLine {
    pub at: Instant,
    pub text: String,
}

/// A short-lived top-center reward line (`+1.250 Credits`…), fed from the
/// same resolved text as the console.
#[derive(Debug)]
pub struct Toast {
    pub at: Instant,
    pub text: String,
}

/// `BoxCollectResponseAssembly` reward codes with a dedicated
/// `getXXX` language key (`0|y|CODE|amount|total`).
const REWARD_KEYS: &[(&str, &str)] = &[
    ("CRE", "getcredits"),
    ("URI", "geturidium"),
    ("EP", "getexp"),
    ("HON", "gatherep"),
    ("HTP", "gethp"),
    ("EPP", "geteventprogresspoints"),
];

/// Other `BOX_CONTENT_*` codes — still rewards for the toast stack even
/// when we cannot localize them yet.
const OTHER_REWARD_CODES: &[&str] = &[
    "CAR", "JPE", "ROK", "SLU", "BAT", "BMP", "XEN", "JV", "AMI", "PFL", "FW", "SPL", "NL",
];

/// `ReplacementTypeModule` values that name an item (`items_<id><postfix>`).
fn item_postfix(ty: i64) -> Option<&'static str> {
    Some(match ty {
        1 => "_code",
        2 => "_short",
        3 => "_fullname",
        4 => "_description",
        7 => "_tooltip_description",
        8 => "_sources",
        _ => return None,
    })
}

/// `TextUtils.getTranslationByType`. An unknown/localized key without a
/// catalog entry stays as is, so the raw-key output of the pre-locale
/// client is preserved.
fn translate(locale: &LocaleCatalog, text: &str, ty: i64) -> String {
    const PLAIN: i64 = 0;
    if ty == PLAIN {
        return text.to_owned();
    }
    if let Some(postfix) = item_postfix(ty) {
        let key = format!("items.items_{text}{postfix}");
        return locale.lookup(&key).unwrap_or(text).to_owned();
    }
    // LOCALIZED (5) and anything unknown.
    locale.resolve(text)
}

/// `typeValue` of a nested `ReplacementTypeModule`, if present.
fn type_value(module: &Packet, field: &str) -> Option<i64> {
    module.module(field).ok()?.int("typeValue").ok()
}

/// Localized/plain-text console line for an inbound packet, or `None`.
///
/// `LogMessageCommand`: `baseKey` is translated by `baseKeyType`
/// (default: localized) and every `wildCardReplacements` row is
/// substituted (`TextUtils.translateMessageKeyListWildcardModule`). When
/// the catalog does not know the base key the line falls back to the raw
/// key plus ` wildcard=value` suffixes, so nothing is lost.
///
/// `LegacyModule` lines are kept only for the message families that carry
/// rewards (`0|y|…`, `0|LM|…`); reward codes are resolved through the
/// catalog when it has the key. Chat / `MSG` lines are NOT logged here —
/// `net::chat` parses them into [`SharedWorld::chat`] and mirrors the
/// resolved text into the log itself.
pub fn game_log(packet: &Packet, locale: &LocaleCatalog) -> Option<String> {
    if packet.name == Some("LegacyModule") {
        let text = packet.string("message").ok()?;
        if !(text.starts_with("0|y|") || text.starts_with("0|LM|")) {
            return None;
        }
        return Some(resolve_reward_line(locale, text).unwrap_or_else(|| text.to_owned()));
    }
    if packet.name != Some("LogMessageCommand") {
        return None;
    }
    let message = packet.module("message").ok()?;
    let key = message.string("baseKey").ok()?;
    let base_ty = type_value(message, "baseKeyType").unwrap_or(5);
    let mut text = translate(locale, key, base_ty);
    let resolved = text != key || base_ty == 0;
    if let Ok(replacements) = message.list("wildCardReplacements") {
        for value in replacements {
            if let Value::Module(replacement) = value {
                if let (Ok(wild), Ok(value)) = (
                    replacement.string("wildcard"),
                    replacement.string("replacement"),
                ) {
                    let ty = type_value(replacement, "replacementType").unwrap_or(0);
                    let value = translate(locale, value, ty);
                    if text.contains(wild) {
                        text = text.replace(wild, &value);
                    } else if !resolved {
                        text.push_str(&format!(" {wild}={value}"));
                    }
                }
            }
        }
    }
    Some(text)
}

/// Split a reward-bearing legacy line into `(code, args…)`:
/// `0|y|CRE|1250|99000` and `0|LM|ST|CRE|1250|99000` both give
/// `["CRE", "1250", "99000"]` (`BoxCollectResponseAssembly.formatLegacy`).
fn legacy_args(text: &str) -> Option<Vec<&str>> {
    let parts: Vec<&str> = text.split('|').collect();
    match parts.get(1).copied()? {
        "y" if parts.len() > 2 => Some(parts[2..].to_vec()),
        "LM" if parts.len() > 3 => Some(parts[3..].to_vec()),
        _ => None,
    }
}

/// Replace the first `%WILDCARD%` token of `template` with `value`
/// (the stock `replace(this.param1, …)` target is not part of the
/// decompile, so any single token is accepted).
fn fill_first_token(template: &str, value: &str) -> String {
    // `NumberPlaceholder` in current DarkOrbit locale files uses `%!`
    // rather than a paired `%AMOUNT%` token (e.g. Turkish
    // "%! Credits kazandın").
    if let Some(i) = template.find("%!") {
        return format!("{}{}{}", &template[..i], value, &template[i + 2..]);
    }
    let bytes = template.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            if let Some(len) = template[i + 1..].find('%') {
                let inner = &template[i + 1..i + 1 + len];
                if !inner.is_empty()
                    && inner
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'|')
                {
                    return format!("{}{}{}", &template[..i], value, &template[i + len + 2..]);
                }
            }
        }
        i += 1;
    }
    template.to_owned()
}

/// Localized text for a `0|y|CODE|amount|…` / `0|LM|ST|CODE|amount|…`
/// reward line, or `None` when the catalog cannot render it.
pub fn resolve_reward_line(locale: &LocaleCatalog, text: &str) -> Option<String> {
    if locale.is_empty() {
        return None;
    }
    let args = legacy_args(text)?;
    let code = *args.first()?;
    if let Some((_, key)) = REWARD_KEYS.iter().find(|(c, _)| *c == code) {
        let raw_amount = args.get(1)?.trim();
        let amount: f64 = raw_amount.parse().ok()?;
        // Deductions use a different message in the stock client.
        if amount <= 0.0 {
            return None;
        }
        let template = locale.lookup(key)?;
        let display_amount = raw_amount
            .parse::<i64>()
            .map(|n| locale.format_i64(n))
            .unwrap_or_else(|_| locale.format_number(amount));
        return Some(fill_first_token(template, &display_amount));
    }
    // `BPLocale.getItem(code)` for sub-commands without a delegate.
    if !OTHER_REWARD_CODES.contains(&code) {
        return locale
            .lookup(code)
            .filter(|t| !t.is_empty())
            .map(str::to_owned);
    }
    None
}

/// True when the packet is a loot/reward message worth a toast.
pub fn is_reward(packet: &Packet) -> bool {
    if packet.name == Some("LegacyModule") {
        let Ok(text) = packet.string("message") else {
            return false;
        };
        return legacy_args(text)
            .and_then(|a| a.first().copied())
            .is_some_and(|c| {
                REWARD_KEYS.iter().any(|(k, _)| *k == c) || OTHER_REWARD_CODES.contains(&c)
            });
    }
    if packet.name == Some("LogMessageCommand") {
        return packet
            .module("message")
            .ok()
            .and_then(|m| m.string("baseKey").ok())
            .is_some_and(|k| {
                k.starts_with("log_msg_gather")
                    || k.starts_with("log_collected")
                    || k.starts_with("log_message_item")
                    || REWARD_KEYS.iter().any(|(_, rk)| *rk == k)
            });
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog() -> LocaleCatalog {
        let xml = std::fs::read_to_string("tests/fixtures/locale_sample.xml").unwrap();
        let mut c = LocaleCatalog::new("tr");
        c.load_xml(&xml, "").unwrap();
        c
    }

    fn legacy(msg: &str) -> Packet {
        Packet {
            id: 0,
            name: Some("LegacyModule"),
            fields: vec![(Some("message"), Value::Str(msg.to_owned()))],
        }
    }

    fn log_message(key: &str, wild: &[(&str, &str, i64)]) -> Packet {
        let rows = wild
            .iter()
            .map(|(w, r, ty)| {
                Value::Module(Box::new(Packet {
                    id: 0,
                    name: Some("MessageWildcardReplacementModule"),
                    fields: vec![
                        (Some("wildcard"), Value::Str((*w).to_owned())),
                        (Some("replacement"), Value::Str((*r).to_owned())),
                        (
                            Some("replacementType"),
                            Value::Module(Box::new(Packet {
                                id: 0,
                                name: Some("ReplacementTypeModule"),
                                fields: vec![(Some("typeValue"), Value::Int(*ty))],
                            })),
                        ),
                    ],
                }))
            })
            .collect();
        Packet {
            id: 0,
            name: Some("LogMessageCommand"),
            fields: vec![(
                Some("message"),
                Value::Module(Box::new(Packet {
                    id: 0,
                    name: Some("MessageLocalizedWildcardCommand"),
                    fields: vec![
                        (Some("baseKey"), Value::Str(key.to_owned())),
                        (Some("wildCardReplacements"), Value::List(rows)),
                    ],
                })),
            )],
        }
    }

    #[test]
    fn reward_codes_resolve_with_locale_grouping() {
        let c = catalog();
        assert_eq!(
            game_log(&legacy("0|y|CRE|6350|1000000"), &c).as_deref(),
            Some("6.350 Credits kazandın")
        );
        // `0|LM|<profile>|CODE|…` form.
        assert_eq!(
            game_log(&legacy("0|LM|ST|EP|1200|5"), &c).as_deref(),
            Some("1.200 Deneyim Puanı kazandın")
        );
        assert!(is_reward(&legacy("0|y|CRE|6350|1")));
        assert!(!is_reward(&legacy("0|LM|ST|something|1")));
    }

    #[test]
    fn reward_without_catalog_stays_raw() {
        let raw = "0|y|CRE|6350|1000000";
        assert_eq!(
            game_log(&legacy(raw), &LocaleCatalog::default()).as_deref(),
            Some(raw)
        );
        // Unrelated legacy lines are still dropped.
        assert_eq!(game_log(&legacy("0|n|t|1|0|x"), &catalog()), None);
    }

    #[test]
    fn base_key_resolves_and_wildcards_apply() {
        let c = catalog();
        let p = log_message("getcredits", &[("%AMOUNT%", "42", 0)]);
        assert_eq!(game_log(&p, &c).as_deref(), Some("42 Credits kazandın"));
        // Localized replacement.
        let p = log_message("getcredits", &[("%AMOUNT%", "title_top_gunner", 5)]);
        assert_eq!(
            game_log(&p, &c).as_deref(),
            Some("Üst Nişancı Credits kazandın")
        );
        // Item-name replacement (`items.items_<id>_fullname`).
        let mut c2 = catalog();
        c2.load_xml(
            r#"<root><item name="items_laser_lf4_fullname">LF-4 Lazer</item></root>"#,
            "items",
        )
        .unwrap();
        let p = log_message("getcredits", &[("%AMOUNT%", "laser_lf4", 3)]);
        assert_eq!(
            game_log(&p, &c2).as_deref(),
            Some("LF-4 Lazer Credits kazandın")
        );
    }

    #[test]
    fn unknown_key_keeps_legacy_raw_format() {
        let p = log_message("some_unknown_key", &[("%X%", "7", 0)]);
        assert_eq!(
            game_log(&p, &catalog()).as_deref(),
            Some("some_unknown_key %X%=7")
        );
        assert_eq!(
            game_log(&p, &LocaleCatalog::default()).as_deref(),
            Some("some_unknown_key %X%=7")
        );
    }

    #[test]
    fn log_message_rewards_are_toasted() {
        assert!(is_reward(&log_message("log_msg_gather_ore_s", &[])));
        assert!(!is_reward(&log_message("chat_hello", &[])));
    }
}
