//! Session/bootstrap data: parsed FlashVars and the pre-connect fetch
//! sequence (`resources.xml`, `maps.php`, `maps-config`, embedded XMLs).
//!
//! Mirrors `FlashVarsParser.parseVars` + `InitGameFileCollectionCommand`:
//! - `cdn`           -> `Settings.staticHost`  (static resource host)
//! - `dynamicHost`/`host` -> `Settings.dynamicHost` (+ trailing `/`)
//! - `basePath`      -> `Settings.basePath`    (+ trailing `/`)
//! - `resources.xml` is fetched from `{staticHost}{basePath}xml/resources.xml
//!   ?__cv={resourcesXmlHash}`.

use anyhow::{anyhow, Result};
use std::collections::HashMap;

/// FlashVars as produced by `xenomit_backpage`'s `launch::load`.
pub type FlashVars = HashMap<String, String>;

/// Derived connection parameters (mirrors `Settings` + `FlashVarsParser.as`).
#[derive(Clone, Debug, Default)]
pub struct BootConfig {
    pub user_id: i32,
    pub session_id: String,
    pub faction_id: i16,
    pub instance_id: i32,
    pub map_id: i32,
    /// `cdn` flashvar — static host (may be protocol-relative `//host/`).
    pub cdn_host: String,
    /// `dynamicHost`/`host` flashvar — dynamic host.
    pub dynamic_host: String,
    /// `basePath` flashvar (e.g. `spacemap/`).
    pub base_path: String,
    /// `gameclientPath` flashvar — directory of `main.swf` under the CDN
    /// (defaults to `base_path` when absent).
    pub gameclient_path: String,
    /// `gameclientHash` flashvar — the `?__cv=` cache buster on the
    /// main.swf URL. NOT the login `VersionHash` (that lives inside the
    /// SWF's `comHash.VersionHash` class).
    pub gameclient_hash: String,
    pub resources_xml_hash: String,
    /// World3D's separate geometry/texture and particle collection versions.
    pub resources_3d_xml_hash: String,
    pub resources_3d_particles_xml_hash: String,
    pub game_xml_hash: String,
    pub language: String,
    /// `languageXmlHash` flashvar — `?__cv=` of the language manifest.
    pub language_xml_hash: String,
    pub platform: String,
}

/// Scheme fix-up: `//host/...` -> `https://host/...`, bare `host/...` ->
/// `https://host/...` (the CDN endpoints are always TLS).
fn normalize_host(host: &str) -> String {
    let host = host.trim();
    if host.starts_with("//") {
        format!("https:{host}")
    } else if host.starts_with("http://") || host.starts_with("https://") {
        host.to_string()
    } else {
        format!("https://{host}")
    }
}

/// `{host}{base}` with exactly one `/` between host and base, and a single
/// trailing `/` — `Settings.dynamicHost = host + "/"`, `Settings.basePath
/// += "/"` semantics without the decompiler's `charAt(length)` bug
/// (which unconditionally appends `/`).
fn join_base(host: &str, base_path: &str) -> String {
    let host = normalize_host(host);
    let base = base_path.trim_matches('/');
    if base.is_empty() {
        return format!("{}/", host.trim_end_matches('/'));
    }
    format!("{}/{base}/", host.trim_end_matches('/'))
}

impl BootConfig {
    /// Parse flashvars exactly like `FlashVarsParser.parseVars`.
    ///
    /// Lenient: numeric fields default to 0 when missing/invalid — the
    /// asset bootstrap only strictly needs `cdn`/`basePath` (connection
    /// fields are validated by `net::bootstrap::ConnectionInfo`). Returns
    /// `Err` when neither `cdn` nor `dynamicHost`/`host` is present, since
    /// no resource URL can then be built.
    pub fn from_flashvars(vars: &FlashVars) -> Result<Self> {
        let get = |k: &str| vars.get(k).map(String::as_str).unwrap_or_default();
        let int = |k: &str| get(k).parse().unwrap_or_default();
        let cdn_host = get("cdn").to_string();
        // `host` and `dynamicHost` are aliases (parseVars maps both).
        let dynamic_host = vars
            .get("dynamicHost")
            .or_else(|| vars.get("host"))
            .cloned()
            .unwrap_or_default();
        if cdn_host.is_empty() && dynamic_host.is_empty() {
            return Err(anyhow!(
                "flashvars carry no cdn/dynamicHost — cannot build resource URLs"
            ));
        }
        // `lang` is split on "_" by parseVars (`en_US` -> `en`).
        let language = get("lang")
            .split('_')
            .next()
            .unwrap_or_default()
            .to_string();
        Ok(Self {
            user_id: int("userID"),
            session_id: get("sessionID").to_string(),
            faction_id: int("factionID") as i16,
            instance_id: vars
                .get("pid")
                .or_else(|| vars.get("projectID"))
                .and_then(|v| v.parse().ok())
                .unwrap_or_default(),
            map_id: int("mapID"),
            cdn_host,
            dynamic_host,
            base_path: if get("basePath").is_empty() {
                "spacemap/".to_string()
            } else {
                get("basePath").to_string()
            },
            gameclient_path: get("gameclientPath").to_string(),
            gameclient_hash: get("gameclientHash").to_string(),
            resources_xml_hash: get("resourcesXmlHash").to_string(),
            resources_3d_xml_hash: get("resources3dXmlHash").to_string(),
            resources_3d_particles_xml_hash: get("resources3dparticlesXmlHash").to_string(),
            game_xml_hash: get("gameXmlHash").to_string(),
            language,
            language_xml_hash: get("languageXmlHash").to_string(),
            platform: get("platform").to_string(),
        })
    }

    /// `{cdn}{basePath}` — prefix for resource files (see
    /// `InitGameFileCollectionCommand`: `staticHost + basePath`, used as
    /// `FileCollection` `m_sBasePath`). Ends with `/`. Falls back to
    /// `dynamicHost` when `cdn` is absent (some endpoints only ship the
    /// dynamic host).
    pub fn resource_base(&self) -> String {
        let host = if self.cdn_host.trim().is_empty() {
            &self.dynamic_host
        } else {
            &self.cdn_host
        };
        join_base(host, &self.base_path)
    }

    /// `{dynamicHost}{basePath}` — prefix for dynamic endpoints
    /// (maps.php). `dynamicHost` defaults to `"/"` in `Settings` (empty or
    /// bare `/` means "same origin") — fall back to `server_instance` in
    /// that case, like the map-endpoint resolution does.
    pub fn dynamic_base(&self, server_instance: &str) -> String {
        let host = self.dynamic_host.trim();
        if host.is_empty() || host == "/" {
            join_base(server_instance, &self.base_path)
        } else {
            join_base(host, &self.base_path)
        }
    }

    /// `{cdn}spacemap/templates/` — language file root.
    /// `InitBPLocaleCommand` uses the literal `Settings.staticHost +
    /// "spacemap/templates/"` (NOT `basePath`).
    pub fn language_base(&self) -> String {
        let host = if self.cdn_host.trim().is_empty() {
            &self.dynamic_host
        } else {
            &self.cdn_host
        };
        join_base(host, "spacemap/templates")
    }

    /// `{language_base}language_{lang}.xml?__cv={languageXmlHash}` — manifest of `<file name hash/>` rows.
    pub fn language_manifest_url(&self) -> String {
        format!(
            "{}language_{}.xml?__cv={}",
            self.language_base(),
            self.language,
            self.language_xml_hash
        )
    }

    /// `{resource_base}xml/resources.xml?__cv={resources_xml_hash}` —
    /// the `FileCollection.loadResourceFile` argument.
    pub fn resources_xml_url(&self) -> String {
        format!(
            "{}xml/resources.xml?__cv={}",
            self.resource_base(),
            self.resources_xml_hash
        )
    }

    /// `{cdn}{gameclientPath or basePath}/main.swf?__cv={gameclientHash}`
    /// — where the preloader fetches the game client. `None` when the
    /// flashvars carry no `gameclientHash` (older layouts).
    pub fn main_swf_url(&self) -> Option<String> {
        if self.gameclient_hash.trim().is_empty() || self.cdn_host.trim().is_empty() {
            return None;
        }
        let path = if self.gameclient_path.trim().is_empty() {
            self.base_path.as_str()
        } else {
            self.gameclient_path.as_str()
        };
        Some(format!(
            "{}main.swf?__cv={}",
            join_base(&self.cdn_host, path),
            self.gameclient_hash
        ))
    }
}
