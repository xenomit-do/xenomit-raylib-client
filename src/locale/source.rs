//! Where language files come from — `InitBPLocaleCommand`.
//!
//! The language is NOT in `resources.xml`; it has its own CDN layout keyed
//! by the `lang` / `languageXmlHash` flashvars:
//!
//! 1. manifest `{cdn}spacemap/templates/language_{lang}.xml?__cv=
//!    {languageXmlHash}` — `<x name="flashres" hash="…"/>` rows;
//! 2. per file `{cdn}spacemap/templates/{lang}/{stem}.xml?__cv={hash}`
//!    (hash = manifest row named `stem`), each loaded into a BPLocale
//!    category (`BPLocalLoadVO`):
//!
//! | stem                   | category        |
//! |------------------------|-----------------|
//! | `flashres`             | `""` (root)     |
//! | `resource_quest`       | `quests`        |
//! | `resource_items`       | `items`         |
//! | `resource_achievement` | `achievements`  |
//! | `resource_inventory`   | `inventory`     |
//! | `resource_galaxyGates` | `galaxyGates`   |
//! | `resource_faq`         | `faq`           |
//! | `resource_eic`         | `eic`           |
//! | `resource_inc`         | `inc`           |
//! | `resource_news`        | `bn`            |
//! | `resource`             | `base`          |
//!
//! (The two `unityApi/events/*.php` space-search documents are skipped.)
//!
//! Local layout under the assets dir (`--assets-dir`, `cache_dir`):
//!
//! - `lang/{lang}/{stem}.{hash}.xml` — download cache, keyed by hash. When
//!   the CDN is unreachable the newest cached `{stem}.*.xml` is used.
//! - `lang/{lang}.xml` — optional hand-made override, parsed *after* the
//!   CDN data (so its entries win). Uses the full BPLocale shape with
//!   `<category name="…">` blocks; root-level `<item>`s land in category
//!   `""`. Also works fully offline.

use super::LocaleCatalog;
use crate::assets::bootstrap::BootConfig;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// `(file stem on the CDN, BPLocale category)` in `InitBPLocaleCommand`
/// order.
pub const LANGUAGE_FILES: &[(&str, &str)] = &[
    ("flashres", ""),
    ("resource_quest", "quests"),
    ("resource_items", "items"),
    ("resource_achievement", "achievements"),
    ("resource_inventory", "inventory"),
    ("resource_galaxyGates", "galaxyGates"),
    ("resource_faq", "faq"),
    ("resource_eic", "eic"),
    ("resource_inc", "inc"),
    ("resource_news", "bn"),
    ("resource", "base"),
];

/// Parse the language manifest: `name` → `hash` for every child row.
pub fn parse_manifest(xml: &str) -> HashMap<String, String> {
    let Ok(doc) = roxmltree::Document::parse(xml) else {
        return HashMap::new();
    };
    doc.root_element()
        .children()
        .filter(|c| c.is_element())
        .filter_map(|c| {
            Some((
                c.attribute("name")?.to_owned(),
                c.attribute("hash")?.to_owned(),
            ))
        })
        .collect()
}

/// `{language_base}{lang}/{stem}.xml?__cv={hash}`.
pub fn file_url(base: &str, lang: &str, stem: &str, hash: &str) -> String {
    format!("{base}{lang}/{stem}.xml?__cv={hash}")
}

/// Newest `{stem}.*.xml` in `dir` (offline fallback).
fn newest_cached(dir: &Path, stem: &str) -> Option<PathBuf> {
    let prefix = format!("{stem}.");
    std::fs::read_dir(dir)
        .ok()?
        .filter_map(Result::ok)
        .filter(|e| {
            let n = e.file_name().to_string_lossy().into_owned();
            n.starts_with(&prefix) && n.ends_with(".xml")
        })
        .max_by_key(|e| e.metadata().and_then(|m| m.modified()).ok())
        .map(|e| e.path())
}

/// One language file: disk cache hit → CDN download (cached) → newest
/// stale cache entry.
async fn fetch_one(
    http: reqwest::Client,
    base: String,
    lang: String,
    stem: &'static str,
    hash: Option<String>,
    dir: PathBuf,
) -> Option<String> {
    if let Some(hash) = hash.as_deref() {
        let cached = dir.join(format!("{stem}.{hash}.xml"));
        if let Ok(text) = std::fs::read_to_string(&cached) {
            return Some(text);
        }
        let url = file_url(&base, &lang, stem, hash);
        match crate::assets::fetch_bytes(&http, &url).await {
            Ok(bytes) => {
                let text = String::from_utf8_lossy(&bytes).into_owned();
                if std::fs::create_dir_all(&dir)
                    .and_then(|_| std::fs::write(&cached, &bytes))
                    .is_err()
                {
                    log::warn!("locale: could not cache {}", cached.display());
                }
                return Some(text);
            }
            Err(e) => log::warn!("locale: {stem} download failed: {e:#}"),
        }
    }
    newest_cached(&dir, stem).and_then(|p| std::fs::read_to_string(p).ok())
}

/// Fetch every language document: `(category, xml)` in
/// [`LANGUAGE_FILES`] order. Missing documents are skipped.
async fn fetch_documents(
    http: &reqwest::Client,
    cfg: &BootConfig,
    dir: &Path,
) -> Vec<(&'static str, String)> {
    let manifest = match crate::assets::fetch_bytes(http, &cfg.language_manifest_url()).await {
        Ok(bytes) => parse_manifest(&String::from_utf8_lossy(&bytes)),
        Err(e) => {
            log::warn!("locale: manifest fetch failed: {e:#}");
            HashMap::new()
        }
    };
    let base = cfg.language_base();
    let mut set = tokio::task::JoinSet::new();
    for (idx, (stem, _)) in LANGUAGE_FILES.iter().enumerate() {
        let (http, base, lang) = (http.clone(), base.clone(), cfg.language.clone());
        let hash = manifest.get(*stem).cloned();
        let dir = dir.to_owned();
        set.spawn(async move { (idx, fetch_one(http, base, lang, stem, hash, dir).await) });
    }
    let mut docs: Vec<(usize, String)> = Vec::new();
    while let Some(res) = set.join_next().await {
        if let Ok((idx, Some(text))) = res {
            docs.push((idx, text));
        }
    }
    docs.sort_by_key(|(idx, _)| *idx);
    docs.into_iter()
        .map(|(idx, text)| (LANGUAGE_FILES[idx].1, text))
        .collect()
}

/// Blocking loader used by asset bootstrap. Never fails: an unreachable
/// CDN without a cache just yields an (almost) empty catalog and the
/// client keeps showing raw keys.
pub fn load_catalog(
    rt: &tokio::runtime::Runtime,
    http: &reqwest::Client,
    cfg: &BootConfig,
    cache_dir: &Path,
) -> LocaleCatalog {
    let lang = cfg.language.trim();
    let mut catalog = LocaleCatalog::new(lang);
    if lang.is_empty() {
        log::warn!("locale: no `lang` flashvar — keys stay unresolved");
        return catalog;
    }
    let lang_dir = cache_dir.join("lang");
    let docs = rt.block_on(fetch_documents(http, cfg, &lang_dir.join(lang)));
    for (category, xml) in &docs {
        if let Err(e) = catalog.load_xml(xml, category) {
            log::warn!("locale: category '{category}' unparsable: {e:#}");
        }
    }
    load_override(&mut catalog, &lang_dir.join(format!("{lang}.xml")));
    log::info!(
        "locale '{lang}': {} items from {} documents",
        catalog.len(),
        docs.len()
    );
    catalog
}

/// Apply a local `lang/{lang}.xml` override file on top of `catalog`.
/// Returns `true` when the file existed and parsed.
pub fn load_override(catalog: &mut LocaleCatalog, path: &Path) -> bool {
    let Ok(xml) = std::fs::read_to_string(path) else {
        return false;
    };
    match catalog.load_xml(&xml, "") {
        Ok(n) => {
            log::info!("locale: {n} entries from {}", path.display());
            true
        }
        Err(e) => {
            log::warn!("locale: {} unparsable: {e:#}", path.display());
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_rows() {
        let m = parse_manifest(
            r#"<root><file name="flashres" hash="abc"/><file name="resource_quest" hash="d1"/><junk/></root>"#,
        );
        assert_eq!(m.get("flashres").map(String::as_str), Some("abc"));
        assert_eq!(m.get("resource_quest").map(String::as_str), Some("d1"));
        assert_eq!(m.len(), 2);
    }

    #[test]
    fn urls_follow_stock_layout() {
        let cfg = BootConfig {
            cdn_host: "//darkorbit-22.bpsecure.com".into(),
            language: "tr".into(),
            language_xml_hash: "h1".into(),
            ..Default::default()
        };
        assert_eq!(
            cfg.language_manifest_url(),
            "https://darkorbit-22.bpsecure.com/spacemap/templates/language_tr.xml?__cv=h1"
        );
        assert_eq!(
            file_url(&cfg.language_base(), "tr", "resource_quest", "q9"),
            "https://darkorbit-22.bpsecure.com/spacemap/templates/tr/resource_quest.xml?__cv=q9"
        );
    }

    #[test]
    fn local_override_wins() {
        let dir = std::env::temp_dir().join("xenomit-locale-override-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("tr.xml");
        std::fs::write(&path, r#"<root><item name="a">override</item></root>"#).unwrap();
        let mut c = LocaleCatalog::new("tr");
        c.load_xml(r#"<root><item name="a">cdn</item></root>"#, "")
            .unwrap();
        assert!(load_override(&mut c, &path));
        assert_eq!(c.resolve("a"), "override");
        assert!(!load_override(&mut c, &dir.join("missing.xml")));
    }
}
