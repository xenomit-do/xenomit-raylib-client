//! `resources.xml` parsing — the `FileCollection` file index.
//!
//! Mirrors `FileCollection.extractLocationsFromXML` /
//! `FileCollection.extractFilesFromXML` + `FileVO.getFileName`:
//!
//! ```xml
//! <resources>
//!   <location id="do_img" path="do_img/"/>
//!   <file id="ship_goliath" location="do_img" name="ships/goliath"
//!         type="swf" version="123" hash="abc"/>
//! </resources>
//! ```
//!
//! yields URLs `{base}{location.path}{name}.{type}[?__cv={hash}]`.
//! `version` is parsed but intentionally NOT used in the URL — the real
//! client's `FileVO` stores it as a ctor arg that is never read; the
//! cache-buster is exclusively `hash` (`cdnHash`), and only when the
//! collection was created with `useHash` — the game collection always
//! passes `true` (`InitGameFileCollectionCommand`: `new FileCollection
//! (base, true)`).
//!
//! `extractFilesFromXML` keys `fileList` by bare `id` normally, but by
//! `id.type` when the collection was built with `_useFilesExtensions` —
//! `World3D.as:86` constructs the `resources_3d_particles` collection as
//! `new FileCollection(base, true, true)`, and that manifest reuses one
//! id for zip/awd/atf triples (`box_2.zip`, `box_2.atf`, `box_2.awd`).
//! `FileIndex` therefore keeps BOTH key spaces (`files` + `files_typed`)
//! for every manifest; `entry`/`url` consult the typed space only on a
//! bare-id miss, so `resources_3d` (no duplicate ids) is unaffected.

use anyhow::{Context, Result};
use std::collections::HashMap;

/// One `<file>` entry from resources.xml.
#[derive(Clone, Debug)]
pub struct FileEntry {
    pub id: String,
    /// `location` id resolved through `<location path>`.
    pub path: String,
    pub name: String,
    /// `swf` | `png` | `jpg` | `xml` | `js` | `txt` | `flv` | `jxr` ...
    pub kind: String,
    /// Parsed but unused (kept for parity with `FileVO`).
    pub version: String,
    /// `hash` attribute (`cdnHash`) — appended as `?__cv=` when present.
    pub hash: String,
    /// Whether the `hash` attribute existed at all — `FileVO.getFileName`
    /// appends `?__cv=` whenever `cdnHash != null` (an empty-but-present
    /// attribute still produces `?__cv=`).
    pub hash_present: bool,
}

#[derive(Clone, Default)]
pub struct FileIndex {
    /// Bare `id` -> entry; the last `<file>` per id wins, matching
    /// stock's `fileList[id] = fileVO` overwrite.
    pub files: HashMap<String, FileEntry>,
    /// `"{id}.{type}"` -> entry — the `_useFilesExtensions` key shape
    /// (`FileCollection.extractFilesFromXML`). Consulted only when
    /// `files` misses.
    pub files_typed: HashMap<String, FileEntry>,
    /// file id -> location path prefix (kept for debugging/tests).
    pub locations: HashMap<String, String>,
}

impl FileIndex {
    /// Parse the `<resources><location/><file/></resources>` document.
    ///
    /// Tolerant to a missing outer wrapper: every `<location>` / `<file>`
    /// descendant of the document is considered, matching the Flash
    /// client's `xml.location` / `xml.file` list semantics.
    pub fn parse(xml: &str) -> Result<Self> {
        let doc = roxmltree::Document::parse(xml).context("invalid resources.xml")?;
        let mut index = FileIndex::default();
        for node in doc.descendants() {
            if node.has_tag_name("location") {
                if let Some(id) = node.attribute("id") {
                    index.locations.insert(
                        id.to_string(),
                        node.attribute("path").unwrap_or_default().to_string(),
                    );
                }
            }
        }
        for node in doc.descendants() {
            if !node.has_tag_name("file") {
                continue;
            }
            let Some(id) = node.attribute("id") else {
                continue;
            };
            let location = node.attribute("location").unwrap_or_default();
            // Flash resolves `locationList[file.@location]`; an unknown
            // location yields `undefined` which stringifies — we use "".
            let path = index.locations.get(location).cloned().unwrap_or_default();
            let hash = node.attribute("hash");
            let entry = FileEntry {
                id: id.to_string(),
                path,
                name: node.attribute("name").unwrap_or_default().to_string(),
                kind: node.attribute("type").unwrap_or_default().to_string(),
                version: node.attribute("version").unwrap_or_default().to_string(),
                hash: hash.unwrap_or_default().to_string(),
                hash_present: hash.is_some(),
            };
            index
                .files_typed
                .insert(format!("{}.{}", entry.id, entry.kind), entry.clone());
            index.files.insert(id.to_string(), entry);
        }
        Ok(index)
    }

    /// Look up a file entry by id. Bare `id` resolves through `files`;
    /// on a miss the `"{id}.{type}"` space is tried, covering
    /// `_useFilesExtensions` collections (`resources_3d_particles`) where
    /// `box_2.zip`/`box_2.atf`/`box_2.awd` share the bare id.
    pub fn entry(&self, id: &str) -> Option<&FileEntry> {
        self.files.get(id).or_else(|| self.files_typed.get(id))
    }

    /// CDN URL for a file id: `FileVO.getFileName(useHash)` —
    /// `{base}{location.path}{name}.{type}` plus `?__cv={hash}` when the
    /// `hash` attribute was present (the game collection always uses
    /// `useHash == true`).
    ///
    /// `base` is the collection base path (`Settings.staticHost +
    /// Settings.basePath` — see `BootConfig::resource_base`) and must end
    /// with `/`, matching `m_sBasePath` semantics where the location path
    /// is appended verbatim.
    ///
    /// `id` resolves through [`Self::entry`]: bare id first, then the
    /// `"{id}.{type}"` extension key.
    pub fn url(&self, base: &str, id: &str) -> Option<String> {
        let entry = self.entry(id)?;
        let mut url = String::with_capacity(
            base.len() + entry.path.len() + entry.name.len() + entry.kind.len() + 2,
        );
        url.push_str(base);
        url.push_str(&entry.path);
        url.push_str(&entry.name);
        url.push('.');
        url.push_str(&entry.kind);
        if entry.hash_present {
            url.push_str("?__cv=");
            url.push_str(&entry.hash);
        }
        Some(url)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `resources_3d_particles` reuses one id for zip/awd/atf triples;
    /// `_useFilesExtensions` keeps all three under `id.type` keys while
    /// the bare id stays last-wins.
    #[test]
    fn typed_keys_keep_duplicate_ids_separate() {
        let index = FileIndex::parse(
            r#"<resources>
                <location id="p" path="particles/"/>
                <file id="box_2" location="p" name="box_2" type="zip" hash="hz"/>
                <file id="box_2" location="p" name="box_2_tex" type="atf" hash="ht"/>
                <file id="box_2" location="p" name="box_2_mesh" type="awd"/>
                <file id="mark_target" location="p" name="mark_target" type="atf" hash="hm"/>
            </resources>"#,
        )
        .unwrap();
        assert_eq!(index.files_typed.len(), 4);
        let zip = index.entry("box_2.zip").expect("typed zip entry");
        assert_eq!(zip.kind, "zip");
        assert_eq!(zip.name, "box_2");
        assert_eq!(zip.hash, "hz");
        let mesh = index.entry("box_2.awd").expect("typed awd entry");
        assert_eq!(mesh.name, "box_2_mesh");
        // Bare id keeps stock last-`<file>`-wins semantics.
        assert_eq!(index.entry("box_2").unwrap().kind, "awd");
        assert_eq!(index.entry("mark_target").unwrap().kind, "atf");
        // `url` resolves through the same fallback.
        assert_eq!(
            index.url("https://cdn/x/", "box_2.atf").unwrap(),
            "https://cdn/x/particles/box_2_tex.atf?__cv=ht"
        );
        assert_eq!(
            index.url("https://cdn/x/", "mark_target.atf").unwrap(),
            "https://cdn/x/particles/mark_target.atf?__cv=hm"
        );
        assert!(index.entry("box_2.obj").is_none());
    }

    /// `resources_3d` ids are unique — bare-id lookups are unaffected by
    /// the typed fallback.
    #[test]
    fn bare_id_manifests_still_resolve() {
        let index = FileIndex::parse(
            r#"<resources>
                <location id="m" path="3d/"/>
                <file id="sibelon_diffuse_512" location="m" name="ships/sibelon/diffuse_512" type="atf" version="7"/>
            </resources>"#,
        )
        .unwrap();
        let entry = index.entry("sibelon_diffuse_512").unwrap();
        assert_eq!(entry.name, "ships/sibelon/diffuse_512");
        assert_eq!(entry.version, "7");
        assert_eq!(
            index.url("https://cdn/x/", "sibelon_diffuse_512").unwrap(),
            "https://cdn/x/3d/ships/sibelon/diffuse_512.atf"
        );
    }
}
