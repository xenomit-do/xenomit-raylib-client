//! BPLocale XML parser — `BPLocaleParser4DO.parseXML` equivalent.
//!
//! Document shape (after `BPLocalLoader` wraps a language file in
//! `<root><category name="…">…</category></root>`):
//!
//! ```xml
//! <root decimalMark="," thousandsSeparator="." separatorPattern="3"
//!       currencyRatio="1" key="tr">
//!   <category name="items">
//!     <item name="foo">Plain text</item>
//!     <item name="bar"><content>Also plain</content></item>
//!     <item name="baz"><content>
//!       <case quantity="1">1 kutu</case>
//!       <case quantity="2+">%COUNT% kutu</case>
//!     </content></item>
//!   </category>
//! </root>
//! ```
//!
//! Raw language files (as served by the CDN) have `<item>` children
//! directly under the root; those land in the caller-supplied default
//! category (the `BPLocalLoadVO.category`).
//!
//! Text post-processing matches the stock client: `\n`, `\'` and `\"`
//! escape masks are unmasked (`removeSpecialCharMasks`) and `<br>` becomes
//! a newline (`Item.setContent`).

use anyhow::{Context, Result};

/// One `<item>` content variant — `BPLocale.setItemInCategory` arguments.
#[derive(Clone, Debug, PartialEq)]
pub struct ParsedEntry {
    pub category: String,
    pub name: String,
    /// `<case quantity>` (0 for plain content).
    pub quantity: i32,
    /// Trailing `+` on the quantity — "this many and above".
    pub and_up: bool,
    pub text: String,
}

/// Everything a document contributes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ParsedLocale {
    /// Root `key` attr (`BPLocale.currentLocaleID`).
    pub key: Option<String>,
    /// Root `decimalMark` attr.
    pub decimal_mark: Option<String>,
    /// Root `thousandsSeparator` attr.
    pub thousands_separator: Option<String>,
    /// Root `separatorPattern` attr (`"3"` or `"3, 2"` → digit group sizes).
    pub separator_pattern: Option<Vec<usize>>,
    /// Root `currencyRatio` attr.
    pub currency_ratio: Option<f64>,
    pub entries: Vec<ParsedEntry>,
    /// Items the stock parser would have thrown on (nameless item, item
    /// without content) — skipped here, reported for logging.
    pub warnings: Vec<String>,
}

/// `removeSpecialCharMasks` + `Item.setContent`'s `<br>` rewrite.
pub fn unmask(text: &str) -> String {
    text.replace("\\n", "\n")
        .replace("\\'", "'")
        .replace("\\\"", "\"")
        .replace("<br>", "\n")
}

/// Text of an element with no element children (`hasSimpleContent`):
/// concatenated text/CDATA nodes, trimmed like `ignoreWhitespace`.
fn simple_text(node: roxmltree::Node) -> String {
    let raw: String = node
        .children()
        .filter(|c| c.is_text())
        .filter_map(|c| c.text())
        .collect();
    unmask(raw.trim())
}

fn has_element_children(node: roxmltree::Node) -> bool {
    node.children().any(|c| c.is_element())
}

/// `separatorPattern` — `/,\s*/` split, each part an `int`.
fn parse_pattern(s: &str) -> Vec<usize> {
    s.split(',')
        .map(|p| p.trim().parse::<usize>().unwrap_or(0))
        .collect()
}

/// Parse one BPLocale document. Root-level `<item>`s (raw language file)
/// go to `default_category`; `<category name>` blocks use their own name.
pub fn parse(xml: &str, default_category: &str) -> Result<ParsedLocale> {
    let opts = roxmltree::ParsingOptions {
        allow_dtd: true,
        ..Default::default()
    };
    let doc = roxmltree::Document::parse_with_options(xml.trim_start_matches('\u{feff}'), opts)
        .context("locale XML is not well-formed")?;
    let root = doc.root_element();
    let mut out = ParsedLocale::default();

    let attr = |name: &str| root.attribute(name).map(str::to_owned);
    out.key = attr("key");
    out.decimal_mark = attr("decimalMark");
    out.thousands_separator = attr("thousandsSeparator");
    out.separator_pattern = attr("separatorPattern").map(|s| parse_pattern(&s));
    out.currency_ratio = attr("currencyRatio").and_then(|s| s.trim().parse().ok());

    for child in root.children().filter(|c| c.is_element()) {
        match child.tag_name().name() {
            "category" => {
                let category = child.attribute("name").unwrap_or("");
                for item in child
                    .children()
                    .filter(|c| c.is_element() && c.has_tag_name("item"))
                {
                    parse_item(item, category, &mut out);
                }
            }
            "item" => parse_item(child, default_category, &mut out),
            _ => {}
        }
    }
    Ok(out)
}

fn parse_item(item: roxmltree::Node, category: &str, out: &mut ParsedLocale) {
    let Some(name) = item.attribute("name").filter(|n| !n.is_empty()) else {
        out.warnings
            .push(format!("an item in category '{category}' has no name"));
        return;
    };
    let mut push = |quantity: i32, and_up: bool, text: String| {
        out.entries.push(ParsedEntry {
            category: category.to_owned(),
            name: name.to_owned(),
            quantity,
            and_up,
            text,
        });
    };

    if !has_element_children(item) {
        push(0, true, simple_text(item));
        return;
    }
    let Some(content) = item
        .children()
        .find(|c| c.is_element() && c.has_tag_name("content"))
    else {
        out.warnings
            .push(format!("item {category}.{name} has no content"));
        return;
    };
    if !has_element_children(content) {
        push(0, true, simple_text(content));
        return;
    }
    for case in content
        .children()
        .filter(|c| c.is_element() && c.has_tag_name("case"))
    {
        let q = case.attribute("quantity").unwrap_or("").trim();
        let (digits, and_up) = match q.strip_suffix('+') {
            Some(d) => (d, true),
            None => (q, false),
        };
        push(
            digits.trim().parse().unwrap_or(0),
            and_up,
            simple_text(case),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> String {
        std::fs::read_to_string("tests/fixtures/locale_sample.xml")
            .expect("tests/fixtures/locale_sample.xml")
    }

    #[test]
    fn root_attributes() {
        let p = parse(&fixture(), "").unwrap();
        assert_eq!(p.key.as_deref(), Some("tr"));
        assert_eq!(p.decimal_mark.as_deref(), Some(","));
        assert_eq!(p.thousands_separator.as_deref(), Some("."));
        assert_eq!(p.separator_pattern, Some(vec![3]));
        assert_eq!(p.currency_ratio, Some(1.0));
    }

    #[test]
    fn categories_and_simple_items() {
        let p = parse(&fixture(), "").unwrap();
        let find = |c: &str, n: &str| {
            p.entries
                .iter()
                .filter(|e| e.category == c && e.name == n)
                .collect::<Vec<_>>()
        };
        let t = find("", "title_top_gunner");
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].text, "Üst Nişancı");
        assert_eq!((t[0].quantity, t[0].and_up), (0, true));
        // `<item><content>…</content></item>` form.
        assert_eq!(find("items", "items_laser_lf4")[0].text, "LF-4");
        // Escape masks + `<br>`.
        assert_eq!(
            find("", "multiline")[0].text,
            "ilk\nikinci\nüçüncü \"x\" 'y'"
        );
    }

    #[test]
    fn quantity_cases() {
        let p = parse(&fixture(), "").unwrap();
        let cases: Vec<_> = p
            .entries
            .iter()
            .filter(|e| e.name == "log_msg_gather_box")
            .collect();
        assert_eq!(cases.len(), 2);
        assert_eq!((cases[0].quantity, cases[0].and_up), (1, false));
        assert_eq!((cases[1].quantity, cases[1].and_up), (2, true));
        assert_eq!(cases[1].text, "%COUNT% kutu toplandı");
    }

    #[test]
    fn bad_items_are_reported_not_fatal() {
        let p = parse(&fixture(), "").unwrap();
        assert_eq!(p.warnings.len(), 2, "{:?}", p.warnings);
        assert!(p.entries.iter().all(|e| !e.name.is_empty()));
    }

    #[test]
    fn raw_file_uses_default_category() {
        let xml = r#"<root><item name="a">x</item><item name="b">y</item></root>"#;
        let p = parse(xml, "quests").unwrap();
        assert_eq!(p.entries.len(), 2);
        assert!(p.entries.iter().all(|e| e.category == "quests"));
    }

    #[test]
    fn separator_pattern_list() {
        let p = parse(r#"<root separatorPattern="3, 2"/>"#, "").unwrap();
        assert_eq!(p.separator_pattern, Some(vec![3, 2]));
    }

    #[test]
    fn malformed_xml_errors() {
        assert!(parse("<root><item", "").is_err());
    }
}
