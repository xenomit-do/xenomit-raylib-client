//! Localization: BPLocale catalog (`net.bigpoint.as3toolbox.bplocale`).
//!
//! Server log/quest/title strings arrive as *keys* (`LogMessageCommand
//! .baseKey`, `titleKey`, `0|y|…` reward codes). The stock client resolves
//! them with `BPLocale.getItem(key)` against the language files loaded by
//! `InitBPLocaleCommand`; [`LocaleCatalog`] is that table.
//!
//! - [`parser`] — `BPLocaleParser4DO` XML → entries.
//! - [`source`] — where the language files come from (CDN, disk cache,
//!   local override) — see its module docs for the resource-id/path
//!   convention.
//!
//! Key syntax (`BPLocale.getItem`): `category.name` splits at the *last*
//! `.` when the tail is a plain word; a bare key lives in category `""`.
//! A missing key yields the key itself from [`LocaleCatalog::resolve`]
//! (stock shows `{category.name}`; callers here want the raw key so the
//! old fallbacks keep working) — use [`LocaleCatalog::lookup`] to tell.
//!
//! Number formatting follows `NumberPlaceholder`/`TextFormatter`: the
//! `decimal_separator` / `thousands_separator` items (or the root
//! `decimalMark`/`thousandsSeparator` attrs) pick the marks; the group
//! sizes come from `separatorPattern`. Before any language is loaded the
//! catalog formats like the old HUD did (`.` thousands, `,` decimal).

pub mod parser;
pub mod source;

use anyhow::Result;
use std::collections::HashMap;

/// One quantity variant of an item (`structure/Content`).
#[derive(Clone, Debug, PartialEq)]
struct Case {
    quantity: i32,
    and_up: bool,
    text: String,
}

/// `Item.getContentObject`: exact quantity, else the nearest lower case
/// flagged "and above". Falls back to the first case so a sloppy file
/// never hides a string entirely.
fn pick_case(cases: &[Case], quantity: i32) -> Option<&Case> {
    let mut prev: Option<&Case> = None;
    for c in cases {
        if c.quantity == quantity {
            return Some(c);
        }
        if c.quantity > quantity {
            return prev.filter(|p| p.and_up).or_else(|| cases.first());
        }
        prev = Some(c);
    }
    prev.filter(|p| p.and_up).or_else(|| cases.first())
}

/// `BPLocale.getItem` key split: category = everything before the last
/// `.` when the tail is `\w+`, else the whole key in category `""`.
fn split_key(key: &str) -> (&str, &str) {
    if let Some(i) = key.rfind('.') {
        let tail = &key[i + 1..];
        if !tail.is_empty() && tail.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
            return (&key[..i], tail);
        }
    }
    ("", key)
}

/// Loaded language table + number-format settings.
#[derive(Clone, Debug)]
pub struct LocaleCatalog {
    language: String,
    items: HashMap<String, HashMap<String, Vec<Case>>>,
    decimal_mark: String,
    thousands_separator: String,
    /// Digit group sizes from the right; the last entry repeats.
    separator_pattern: Vec<usize>,
    currency_ratio: Option<f64>,
}

impl Default for LocaleCatalog {
    fn default() -> Self {
        Self {
            language: String::new(),
            items: HashMap::new(),
            decimal_mark: ",".into(),
            thousands_separator: ".".into(),
            separator_pattern: vec![3],
            currency_ratio: None,
        }
    }
}

impl LocaleCatalog {
    /// Empty catalog for `language` (e.g. `"tr"`) — resolves nothing,
    /// formats numbers with the fallback separators.
    pub fn new(language: &str) -> Self {
        Self {
            language: language.to_owned(),
            ..Self::default()
        }
    }

    pub fn language(&self) -> &str {
        &self.language
    }

    /// Number of distinct `(category, name)` items.
    pub fn len(&self) -> usize {
        self.items.values().map(HashMap::len).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn currency_ratio(&self) -> Option<f64> {
        self.currency_ratio
    }

    pub fn thousands_separator(&self) -> &str {
        &self.thousands_separator
    }

    pub fn decimal_mark(&self) -> &str {
        &self.decimal_mark
    }

    /// `BPLocale.setItemInCategory`.
    pub fn set_item(
        &mut self,
        category: &str,
        name: &str,
        text: &str,
        quantity: i32,
        and_up: bool,
    ) {
        let cases = self
            .items
            .entry(category.to_owned())
            .or_default()
            .entry(name.to_owned())
            .or_default();
        let case = Case {
            quantity,
            and_up,
            text: text.to_owned(),
        };
        match cases.iter().position(|c| c.quantity >= quantity) {
            Some(i) if cases[i].quantity == quantity => cases[i] = case,
            Some(i) => cases.insert(i, case),
            None => cases.push(case),
        }
    }

    /// Parse one language document into the catalog (later documents win
    /// on key collisions). `default_category` receives root-level
    /// `<item>`s — the `BPLocalLoadVO.category`. Returns the number of
    /// entries added.
    pub fn load_xml(&mut self, xml: &str, default_category: &str) -> Result<usize> {
        let parsed = parser::parse(xml, default_category)?;
        for w in &parsed.warnings {
            log::warn!("locale: {w}");
        }
        if let Some(k) = parsed.key.as_deref().filter(|k| !k.is_empty()) {
            if self.language.is_empty() {
                self.language = k.to_owned();
            }
        }
        if let Some(m) = parsed.decimal_mark {
            self.decimal_mark = m;
        }
        if let Some(s) = parsed.thousands_separator {
            self.thousands_separator = s;
        }
        if let Some(p) = parsed.separator_pattern.filter(|p| !p.is_empty()) {
            self.separator_pattern = p;
        }
        if parsed.currency_ratio.is_some() {
            self.currency_ratio = parsed.currency_ratio;
        }
        let n = parsed.entries.len();
        for e in &parsed.entries {
            self.set_item(&e.category, &e.name, &e.text, e.quantity, e.and_up);
        }
        self.refresh_separators();
        Ok(n)
    }

    /// `TextFormatter.updateNumberSeparators`: the `decimal_separator` /
    /// `thousands_separator` items win when both exist, are not a lone
    /// space, and differ.
    pub fn refresh_separators(&mut self) {
        let (Some(dec), Some(th)) = (
            self.lookup("decimal_separator"),
            self.lookup("thousands_separator"),
        ) else {
            return;
        };
        if dec != " " && th != " " && dec != th && !dec.is_empty() {
            let (dec, th) = (dec.to_owned(), th.to_owned());
            self.decimal_mark = dec;
            self.thousands_separator = th;
        }
    }

    /// Text of `key` for quantity 0, if present.
    pub fn lookup(&self, key: &str) -> Option<&str> {
        self.lookup_quantity(key, 0)
    }

    /// Text of `key` for a plural `quantity` (`getItem(key, null, qty)`).
    pub fn lookup_quantity(&self, key: &str, quantity: i32) -> Option<&str> {
        let (category, name) = split_key(key);
        let cases = self.items.get(category)?.get(name)?;
        pick_case(cases, quantity).map(|c| c.text.as_str())
    }

    pub fn contains(&self, key: &str) -> bool {
        self.lookup(key).is_some()
    }

    /// Localized text for `key`, or `key` itself when the catalog has no
    /// entry (empty catalog = identity).
    pub fn resolve(&self, key: &str) -> String {
        self.lookup(key).unwrap_or(key).to_owned()
    }

    /// [`Self::resolve`] for a plural `quantity`.
    pub fn resolve_quantity(&self, key: &str, quantity: i32) -> String {
        self.lookup_quantity(key, quantity)
            .unwrap_or(key)
            .to_owned()
    }

    /// Insert thousands separators into a run of ASCII digits.
    fn group_digits(&self, digits: &str) -> String {
        let sep = &self.thousands_separator;
        if self.separator_pattern.is_empty() || sep.is_empty() {
            return digits.to_owned();
        }
        let mut groups: Vec<&str> = Vec::new();
        let mut end = digits.len();
        let mut idx = 0;
        loop {
            let size = self.separator_pattern[idx.min(self.separator_pattern.len() - 1)];
            if size == 0 || end <= size {
                break;
            }
            groups.push(&digits[end - size..end]);
            end -= size;
            idx += 1;
        }
        groups.push(&digits[..end]);
        groups.reverse();
        groups.join(sep)
    }

    /// `v` rounded to `precision` decimals with the locale's marks
    /// (`TextFormatter.round`; `NumberPlaceholder`).
    pub fn format_decimal(&self, v: f64, precision: usize) -> String {
        if !v.is_finite() {
            return "0".into();
        }
        let scale = 10f64.powi(precision as i32);
        let rounded = (v.abs() * scale).round() / scale;
        let fixed = format!("{rounded:.precision$}");
        let (int_part, frac) = match fixed.split_once('.') {
            Some((i, f)) => (i, Some(f)),
            None => (fixed.as_str(), None),
        };
        let mut out = String::new();
        if v < 0.0 && rounded != 0.0 {
            out.push('-');
        }
        out.push_str(&self.group_digits(int_part));
        if let Some(f) = frac {
            out.push_str(&self.decimal_mark);
            out.push_str(f);
        }
        out
    }

    /// Integer-rounded, grouped (`TextFormatter.roundInteger`):
    /// `6350` → `6.350` for `tr`, `6,350` for `en`.
    pub fn format_number(&self, v: f64) -> String {
        self.format_decimal(v, 0)
    }

    /// [`Self::format_number`] for integers (exact beyond 2^53).
    pub fn format_i64(&self, v: i64) -> String {
        let digits = v.unsigned_abs().to_string();
        let grouped = self.group_digits(&digits);
        if v < 0 {
            format!("-{grouped}")
        } else {
            grouped
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> LocaleCatalog {
        let xml = std::fs::read_to_string("tests/fixtures/locale_sample.xml").unwrap();
        let mut c = LocaleCatalog::new("tr");
        c.load_xml(&xml, "").unwrap();
        c
    }

    #[test]
    fn resolve_known_and_unknown() {
        let c = sample();
        assert_eq!(c.resolve("title_top_gunner"), "Üst Nişancı");
        assert_eq!(c.lookup("title_missing"), None);
        assert_eq!(c.resolve("title_missing"), "title_missing");
        // Category.name addressing.
        assert_eq!(c.resolve("items.items_laser_lf4"), "LF-4");
        assert_eq!(c.resolve("quests.quest_title_1"), "İlk Görev");
        assert_eq!(c.resolve("items.nope"), "items.nope");
        assert_eq!(c.language(), "tr");
    }

    #[test]
    fn plural_cases() {
        let c = sample();
        assert_eq!(
            c.resolve_quantity("log_msg_gather_box", 1),
            "1 kutu toplandı"
        );
        assert_eq!(
            c.resolve_quantity("log_msg_gather_box", 5),
            "%COUNT% kutu toplandı"
        );
        // Below the lowest case: falls back to the first.
        assert_eq!(
            c.resolve_quantity("log_msg_gather_box", 0),
            "1 kutu toplandı"
        );
    }

    #[test]
    fn empty_catalog_is_identity() {
        let c = LocaleCatalog::default();
        assert!(c.is_empty());
        assert_eq!(c.resolve("anything"), "anything");
    }

    #[test]
    fn turkish_number_format() {
        let c = sample();
        assert_eq!(c.thousands_separator(), ".");
        assert_eq!(c.decimal_mark(), ",");
        assert_eq!(c.format_number(6350.0), "6.350");
        assert_eq!(c.format_number(1234567.0), "1.234.567");
        assert_eq!(c.format_number(999.0), "999");
        assert_eq!(c.format_number(-12345.4), "-12.345");
        assert_eq!(c.format_i64(-1000), "-1.000");
        assert_eq!(c.format_decimal(1234.5, 2), "1.234,50");
        assert_eq!(c.format_decimal(0.004, 2), "0,00");
    }

    #[test]
    fn english_style_and_indian_pattern() {
        let mut c = LocaleCatalog::new("en");
        c.load_xml(
            r#"<root decimalMark="." thousandsSeparator="," separatorPattern="3, 2"/>"#,
            "",
        )
        .unwrap();
        assert_eq!(c.format_i64(6350), "6,350");
        assert_eq!(c.format_i64(12345678), "1,23,45,678");
        assert_eq!(c.format_decimal(1234.5, 1), "1,234.5");
    }

    #[test]
    fn separator_items_override_attrs() {
        let mut c = LocaleCatalog::new("de");
        c.load_xml(
            r#"<root thousandsSeparator=","><item name="thousands_separator">.</item><item name="decimal_separator">,</item></root>"#,
            "",
        )
        .unwrap();
        assert_eq!(c.format_i64(1000), "1.000");
        // Equal separators are rejected (TextFormatter guard).
        let mut c = LocaleCatalog::new("xx");
        c.load_xml(
            r#"<root><item name="thousands_separator">,</item><item name="decimal_separator">,</item></root>"#,
            "",
        )
        .unwrap();
        assert_eq!(c.format_i64(1000), "1.000"); // untouched fallback
    }

    #[test]
    fn later_documents_override() {
        let mut c = LocaleCatalog::new("tr");
        c.load_xml(r#"<root><item name="a">one</item></root>"#, "")
            .unwrap();
        c.load_xml(r#"<root><item name="a">two</item></root>"#, "")
            .unwrap();
        assert_eq!(c.resolve("a"), "two");
        assert_eq!(c.len(), 1);
    }
}
