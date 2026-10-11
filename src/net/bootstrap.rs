//! Pre-connect resolution: flashvars -> `LoginInfo` + map server endpoint
//! (mirrors `InitMapModelCommand` + `Map.getServerIP` + the bot's
//! `packet_client.rs` connect path).

use anyhow::{anyhow, Context, Result};
use std::collections::HashMap;
use std::time::Duration;
use xenomit_packet_client::modules::VERSION_HASH;
use xenomit_packet_client::session::LoginInfo;

/// FlashVars as produced by `xenomit_backpage`'s `launch::load`.
pub type FlashVars = HashMap<String, String>;

/// User-Agent matching `xenomit_backpage::client::USER_AGENT`
/// (`BigpointClient/…` is what the stock launcher sends).
const USER_AGENT: &str = "BigpointClient/1.6.9";

/// Default map-server port when `gameserverIP` carries no `:port`.
const DEFAULT_GAME_PORT: u16 = 5001;

/// Everything needed to open the game socket.
#[derive(Clone, Debug)]
pub struct ConnectionInfo {
    /// `host:port` of the map server for the account's current map.
    pub endpoint: String,
    pub user_id: i32,
    pub faction_id: i16,
    pub session_id: String,
    pub instance_id: i32,
    pub is_mini_client: bool,
    /// `VersionRequest`/`LoginRequest` hash — defaults to the bundled
    /// `VERSION_HASH`; bootstrap overrides it with the live main.swf's
    /// extracted `VersionHash` constant.
    pub version: String,
    /// `"2D"` — echoed in `ReadyRequest`.
    pub view_data: String,
}

impl ConnectionInfo {
    /// Parse the flashvars *and* resolve the map server endpoint via
    /// `{dynamicBase}/xml/maps.php`, using a default HTTP client.
    ///
    /// Async: call inside a tokio runtime (e.g. `rt.block_on(...)` from
    /// the loading worker thread).
    pub async fn from_flashvars(vars: &FlashVars, server_instance: &str) -> Result<Self> {
        Self::from_flashvars_with(vars, server_instance, &default_http_client()?).await
    }

    /// Same as [`Self::from_flashvars`], reusing a caller-provided client
    /// (shares its cookie jar / timeouts / proxy config).
    pub async fn from_flashvars_with(
        vars: &FlashVars,
        server_instance: &str,
        http: &reqwest::Client,
    ) -> Result<Self> {
        let mut info = Self::parse(vars)?;
        info.endpoint = resolve_endpoint(vars, server_instance, http).await?;
        Ok(info)
    }

    /// Parse the credential fields only; `endpoint` is left empty until
    /// [`resolve_endpoint`] fills it.
    ///
    /// Mirrors `FlashVarsParser.as` + the bot's `connect()`:
    ///
    /// | FlashVars key       | field           |
    /// |---------------------|-----------------|
    /// | `userID`            | `user_id`       |
    /// | `sessionID`         | `session_id`    |
    /// | `factionID`         | `faction_id` (0 when absent — factionless
    ///   accounts do not receive the key; `FlashVarsParser` leaves
    ///   `Hero.factionID` at its `BindableInt` default of 0 then too) |
    /// | `pid`/`projectID`   | `instance_id`   |
    /// | `platform=="client"`| `is_mini_client`|
    pub fn parse(vars: &FlashVars) -> Result<Self> {
        let required = |key: &str| -> Result<String> {
            vars.get(key)
                .filter(|v| !v.is_empty())
                .cloned()
                .ok_or_else(|| anyhow!("flashvars missing {key}"))
        };
        Ok(Self {
            endpoint: String::new(),
            user_id: required("userID")?
                .parse()
                .context("invalid userID flashvar")?,
            faction_id: vars
                .get("factionID")
                .filter(|v| !v.is_empty())
                .map(|v| v.parse().context("invalid factionID flashvar"))
                .transpose()?
                .unwrap_or(0),
            session_id: required("sessionID")?,
            instance_id: vars
                .get("pid")
                .or_else(|| vars.get("projectID"))
                .filter(|v| !v.is_empty())
                .ok_or_else(|| anyhow!("flashvars missing pid/projectID"))?
                .parse()
                .context("invalid pid/projectID flashvar")?,
            // `Settings.platformTypeClient == "PC_CLIENT"`
            // (`isMiniClient = platform == "PC_CLIENT"` in the login flow).
            is_mini_client: vars.get("platform").is_some_and(|v| v == "PC_CLIENT"),
            version: VERSION_HASH.into(),
            view_data: "2D".into(),
        })
    }

    /// Credentials for `Session::handshake` (version defaults to the
    /// bundled hash).
    pub fn login_info(&self) -> LoginInfo {
        LoginInfo {
            user_id: self.user_id,
            faction_id: self.faction_id,
            session_id: self.session_id.clone(),
            instance_id: self.instance_id,
            view_data: self.view_data.clone(),
            version: self.version.clone(),
            is_mini_client: self.is_mini_client,
        }
    }
}

/// HTTP client for the bootstrap fetches (20s timeout, Bigpoint UA).
pub fn default_http_client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .user_agent(USER_AGENT)
        .build()?)
}

/// `{dynamicBase}/{basePath}/xml/maps.php` — `base` defaults to
/// `spacemap/`; `host` may be a full URL, `//host`, a path on the
/// instance, or a bare host (HTTPS assumed). Port of the bot's
/// `map_config_url`.
pub fn map_config_url(server_instance: &str, host: &str, base: &str) -> Result<reqwest::Url> {
    let instance = reqwest::Url::parse(server_instance).context("invalid server instance URL")?;
    let host = if host.is_empty() || host == "/" {
        instance
    } else if host.starts_with("//") {
        reqwest::Url::parse(&format!("https:{host}")).context("invalid dynamicHost")?
    } else if host.starts_with("http://") || host.starts_with("https://") {
        reqwest::Url::parse(host).context("invalid dynamicHost")?
    } else if host.starts_with('/') {
        instance.join(host).context("invalid dynamicHost")?
    } else {
        reqwest::Url::parse(&format!("https://{host}")).context("invalid dynamicHost")?
    };
    let root = format!("{}/", host.as_str().trim_end_matches('/'));
    Ok(reqwest::Url::parse(&root)
        .and_then(|url| url.join(&format!("{}/xml/maps.php", base.trim_matches('/'))))?)
}

/// Fetch `maps.php` and return `host:port` for the account's `mapID`
/// flashvar (defaulting to port 5001).
///
/// When the resolved URL lives on the instance host, the `sessionID`
/// flashvar is sent as the `dosid` cookie — the same guard the bot uses
/// (`sessionID` *is* the dosid; CDN-hosted configs get no cookie).
pub async fn resolve_endpoint(
    vars: &FlashVars,
    server_instance: &str,
    http: &reqwest::Client,
) -> Result<String> {
    let map_id: i32 = vars
        .get("mapID")
        .filter(|v| !v.is_empty())
        .ok_or_else(|| anyhow!("flashvars missing mapID"))?
        .parse()
        .context("invalid mapID flashvar")?;
    let base = vars
        .get("basePath")
        .map(String::as_str)
        .unwrap_or("spacemap/");
    let host = vars
        .get("dynamicHost")
        .or_else(|| vars.get("host"))
        .map(String::as_str)
        .unwrap_or("");
    let url = map_config_url(server_instance, host, base)?;
    let instance_host = reqwest::Url::parse(server_instance)
        .ok()
        .and_then(|u| u.host_str().map(str::to_owned));
    let mut request = http.get(url.clone());
    if instance_host.as_deref().is_some() && url.host_str() == instance_host.as_deref() {
        if let Some(sid) = vars.get("sessionID").filter(|v| !v.is_empty()) {
            request = request.header(reqwest::header::COOKIE, format!("dosid={sid}"));
        }
    }
    let body = request
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .with_context(|| format!("could not load map servers from {url}"))?
        .text()
        .await
        .context("could not read maps.php body")?;
    let endpoint = MapEndpoint::find(&body, map_id).with_context(|| {
        // Include a preview of the response — a non-XML body means the
        // server answered with an error page, redirect HTML, or text we
        // need to see to diagnose.
        let preview: String = body.chars().take(300).collect();
        format!("maps.php response from {url}: {preview:?}")
    })?;
    Ok(endpoint.host_port())
}

/// One `<map>` entry from `maps.php`.
#[derive(Clone, Debug)]
pub struct MapEndpoint {
    pub id: i32,
    pub name: String,
    pub server_ip: String,
}

impl MapEndpoint {
    /// Parse every `<map>` entry of a `maps.php` response.
    /// Entries with a missing `gameserverIP` child are kept with an
    /// empty `server_ip` so they stay visible for diagnostics.
    pub fn parse_maps_php(xml: &str) -> Result<Vec<Self>> {
        let document =
            roxmltree::Document::parse(xml).context("invalid map server configuration")?;
        Ok(document
            .descendants()
            .filter(|n| n.has_tag_name("map"))
            .filter_map(|n| {
                Some(Self {
                    id: n.attribute("id")?.parse().ok()?,
                    name: n.attribute("name").unwrap_or_default().to_owned(),
                    server_ip: n
                        .children()
                        .find(|c| c.has_tag_name("gameserverIP"))
                        .and_then(|c| c.text())
                        .map(str::trim)
                        .unwrap_or_default()
                        .to_owned(),
                })
            })
            .collect())
    }

    /// First entry matching `map_id` with a usable `gameserverIP`.
    pub fn find(xml: &str, map_id: i32) -> Result<Self> {
        Self::parse_maps_php(xml)?
            .into_iter()
            .find(|m| m.id == map_id && !m.server_ip.is_empty())
            .ok_or_else(|| anyhow!("no game server for map {map_id}"))
    }

    /// `host` or `host:port` — appends the default game port when the
    /// configured `gameserverIP` carries none.
    pub fn host_port(&self) -> String {
        if self.server_ip.contains(':') {
            self.server_ip.clone()
        } else {
            format!("{}:{DEFAULT_GAME_PORT}", self.server_ip)
        }
    }
}
