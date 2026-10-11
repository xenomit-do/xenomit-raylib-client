//! Dump one map's `<map>` node from the CDN `maps-config` (raw and, when
//! `MapDb` parsing drops markup like `<reference>`, that markup's text).
//! HTTP-only bootstrap — no game login, safe next to sidkeep.
//!
//!   cargo run --release --example mapcfg -- --server tr3 --dosid X [--map 307]

use anyhow::{Context, Result};
use xenomit_client::assets::bootstrap::BootConfig;
use xenomit_client::assets::resources::FileIndex;
use xenomit_client::net::bootstrap::default_http_client;

fn main() -> Result<()> {
    let mut server = "tr3".to_owned();
    let mut dosid = String::new();
    let mut map_id = 307i32;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--server" => server = args.next().context("--server needs a value")?,
            "--dosid" => dosid = args.next().context("--dosid needs a value")?,
            "--map" => map_id = args.next().context("--map needs a value")?.parse()?,
            _ => anyhow::bail!("unknown arg {a}"),
        }
    }
    if dosid.is_empty() {
        anyhow::bail!("usage: mapcfg --server tr3 --dosid X [--map 307]");
    }

    let session = xenomit_backpage::session::Session::new(&dosid, &server);
    let mut client = xenomit_backpage::client::BackpageClient::new(session)?;
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(xenomit_backpage::launch::load(&mut client))
        .context("backpage launch failed")?;
    let vars = client.session().flashvars.clone();
    let cfg = BootConfig::from_flashvars(&vars)?;

    let http = default_http_client()?;
    let resources = rt
        .block_on(async {
            http.get(cfg.resources_xml_url())
                .send()
                .await?
                .error_for_status()?
                .bytes()
                .await
        })
        .context("resources.xml fetch failed")?;
    let files = FileIndex::parse(&String::from_utf8_lossy(&resources))?;
    let url = files
        .url(&cfg.resource_base(), "maps-config")
        .context("resources.xml has no maps-config entry")?;
    println!("maps-config url: {url}");
    let cfg_xml = rt
        .block_on(async {
            http.get(&url)
                .send()
                .await?
                .error_for_status()?
                .bytes()
                .await
        })
        .context("maps-config fetch failed")?;
    let text = String::from_utf8_lossy(&cfg_xml);
    println!("maps-config: {} bytes", text.len());

    // Print the raw `<map id="N">` node (string-slice, references intact).
    let needle = format!("<map id=\"{map_id}\"");
    if let Some(start) = text.find(&needle) {
        let rest = &text[start..];
        let end = rest
            .find("</map>")
            .map(|e| start + e + "</map>".len())
            .unwrap_or_else(|| (start + 4000).min(text.len()));
        println!(
            "--- raw <map id=\"{map_id}\"> node ---\n{}",
            &text[start..end]
        );
    } else {
        println!("no <map id=\"{map_id}\"> node — listing all ids:");
        for m in text.match_indices("<map id=\"") {
            let tail = &text[m.0 + 9..];
            let end = tail.find('"').unwrap_or(0);
            print!("{} ", &tail[..end]);
        }
        println!();
    }

    // Which of the pattern's res keys exist in resources.xml?
    for id in [
        "blacklight_map-2_background",
        "blacklight_map-2_background_01",
        "blacklight_map-2_background_02",
        "blacklight_map-2_background_03",
        "blacklight_map-2_background_04",
        "blacklight_map-2_background_atf",
        "blacklight_map-2_stars",
        "blacklight_map-2_stars_texture",
        "blacklight_map-1_stars_atlas",
        "blacklight_map-1_stars",
        "blacklight_map-1_stars_texture",
        "maps-config",
    ] {
        println!(
            "resource {id:42} {}",
            if files.entry(id).is_some() {
                "OK"
            } else {
                "MISSING"
            }
        );
    }
    Ok(())
}
