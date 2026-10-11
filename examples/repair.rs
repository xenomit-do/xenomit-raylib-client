//! Dev tool: repair a destroyed ship (LoginResponse status 7 =
//! `STATUS_ShipIsDestroyed`) by driving the kill-screen flow the stock
//! client uses:
//!
//!   handshake → LoginResponse(7)
//!             ← KillScreenPostCommand { options:[KillScreenOptionModule] }
//!             → KillScreenRepairRequest { selection, requestModule: LoginRequest }
//!             ← LoginResponse(0) + normal init traffic
//!
//! (ConnectionManager LOGIN_WHITELIST allows KillScreenRepairRequest before
//! a successful login; Killscreen.repair() sends it the same way.)
//!
//!   cargo run --release --example repair -- --server tr3 --dosid X [--option N]
//!
//! Option picking prefers the free `OPTIONTYPE_BASIC_REPAIR` (respawn at
//! the home base). If no option list arrives, we still try BASIC_REPAIR —
//! it is offered for every ordinary ship destruction.

use anyhow::{Context, Result};
use std::io::Write;
use std::net::{TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant};
use xenomit_client::net::bootstrap::{default_http_client, ConnectionInfo};
use xenomit_client::swf_extract::SwfLibrary;
use xenomit_packet_client::error::ClientError;
use xenomit_packet_client::packet::Value;
use xenomit_packet_client::session::{Event, Session};
use xenomit_packet_client::{modules, session::LoginInfo};

/// modules::id values not already covered by an exported name.
const KILL_SCREEN_POST_COMMAND: i16 = 252;
const KILL_SCREEN_OPTION_MODULE: i16 = 253;
const KILL_SCREEN_UPDATE_COMMAND: i16 = 6777;
const OPTIONTYPE_BASIC_REPAIR: i64 = xenomit_client::net::commands::REPAIR_TYPE_BASIC as i64;

/// `repairTypeValue` + price/affordability of one offered option.
fn option_info(opt: &Value) -> Option<(i64, i64, bool)> {
    let Value::Module(m) = opt else { return None };
    if m.id != KILL_SCREEN_OPTION_MODULE {
        return None;
    }
    let ty = m
        .module("repairType")
        .ok()
        .and_then(|t| t.int("repairTypeValue").ok())
        .unwrap_or(-1);
    let amount = m
        .module("price")
        .ok()
        .and_then(|p| p.int("amount").ok())
        .unwrap_or(0);
    let affordable = m.boolean("affordableForPlayer").unwrap_or(false);
    Some((ty, amount, affordable))
}

/// Pick the option to send: explicit `--option`, else BASIC_REPAIR when
/// offered+affordable, else the cheapest affordable free option.
fn pick_option(options: &[(i64, i64, bool)], forced: Option<i64>) -> i64 {
    if let Some(o) = forced {
        return o;
    }
    if options
        .iter()
        .any(|(t, _, a)| *t == OPTIONTYPE_BASIC_REPAIR && *a)
    {
        return OPTIONTYPE_BASIC_REPAIR;
    }
    options
        .iter()
        .filter(|(_, amt, a)| *a && *amt == 0)
        .map(|(t, _, _)| *t)
        .min()
        .unwrap_or(OPTIONTYPE_BASIC_REPAIR)
}

fn main() -> Result<()> {
    let mut server = "tr3".to_owned();
    let mut dosid = String::new();
    let mut forced: Option<i64> = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--server" => server = args.next().context("--server needs a value")?,
            "--dosid" => dosid = args.next().context("--dosid needs a value")?,
            "--option" => forced = Some(args.next().context("--option needs a value")?.parse()?),
            _ => anyhow::bail!("unknown arg {a}"),
        }
    }
    if dosid.is_empty() {
        anyhow::bail!("usage: repair --server tr3 --dosid X [--option N]");
    }
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    // --- bootstrap (same path as pktlog / the app) ---------------------
    let session = xenomit_backpage::session::Session::new(&dosid, &server);
    let mut client = xenomit_backpage::client::BackpageClient::new(session)?;
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(xenomit_backpage::launch::load(&mut client))
        .context("backpage launch failed")?;
    let vars = client.session().flashvars.clone();
    let instance = client.session().instance();
    let mut info = rt
        .block_on(ConnectionInfo::from_flashvars(&vars, &instance))
        .context("endpoint resolve failed")?;

    let http = default_http_client()?;
    if let Some(url) = xenomit_client::assets::bootstrap::BootConfig::from_flashvars(&vars)
        .ok()
        .and_then(|c| c.main_swf_url())
    {
        if let Ok(bytes) = rt.block_on(async {
            http.get(&url)
                .send()
                .await?
                .error_for_status()?
                .bytes()
                .await
        }) {
            if let Ok(lib) = SwfLibrary::from_bytes(&bytes) {
                if let Some(h) = lib.version_hash() {
                    info.version = h;
                }
            }
        }
    }
    println!("endpoint {} version {}", info.endpoint, info.version);

    // --- game socket ---------------------------------------------------
    let addr = info
        .endpoint
        .to_socket_addrs()?
        .next()
        .context("no addresses")?;
    let stream = TcpStream::connect_timeout(&addr, Duration::from_secs(15))?;
    stream.set_nodelay(true)?;
    stream.set_read_timeout(Some(Duration::from_secs(30)))?;
    let mut s = Session::new(stream);
    let login: LoginInfo = info.login_info();

    match s.handshake(&login) {
        Ok(()) => {
            println!("login accepted — ship is not destroyed, nothing to do");
        }
        Err(ClientError::LoginRejected(7)) => {
            println!("login rejected status 7 (ship destroyed) — kill-screen flow");

            // Read until the kill-screen options arrive. Note they may
            // already have been consumed inside `handshake`'s read loop;
            // in that case we fall back to BASIC_REPAIR anyway.
            let mut options: Vec<(i64, i64, bool)> = Vec::new();
            let mut got_post = false;
            let stream_deadline = Instant::now() + Duration::from_secs(10);
            while !got_post && Instant::now() < stream_deadline {
                match s.recv() {
                    Ok(Event::Packet(p))
                        if p.id == KILL_SCREEN_POST_COMMAND
                            || p.id == KILL_SCREEN_UPDATE_COMMAND =>
                    {
                        println!(
                            "<- {} killer={} options:",
                            p.name.unwrap_or("?"),
                            p.string("killerName").unwrap_or("?")
                        );
                        for (ty, amt, afford) in p
                            .list("options")
                            .unwrap_or(&[])
                            .iter()
                            .filter_map(option_info)
                        {
                            println!("   option type={ty} price={amt} affordable={afford}");
                            options.push((ty, amt, afford));
                        }
                        got_post = true;
                    }
                    Ok(ev) => {
                        if let Event::Packet(p) = &ev {
                            println!("   (pre-repair pkt {} {:?})", p.id, p.name);
                        }
                    }
                    Err(e) => {
                        println!("   recv while waiting for kill screen: {e}");
                        break;
                    }
                }
            }

            let choice = pick_option(&options, forced);
            println!("-> KillScreenRepairRequest selection={choice}");
            s.send(&xenomit_client::net::commands::repair_request(
                &login,
                choice as i16,
            ))
            .context("send repair request")?;

            // The server treats the embedded LoginRequest as the real
            // login: expect LoginResponse(0) then normal init traffic.
            let deadline = Instant::now() + Duration::from_secs(60);
            let mut ok = false;
            while Instant::now() < deadline && !ok {
                match s.recv() {
                    Ok(Event::Packet(p)) if p.id == modules::id::LOGIN_RESPONSE => {
                        let status = p.int("status").unwrap_or(-1);
                        println!("<- LoginResponse status={status}");
                        if status == 0 {
                            ok = true;
                        } else {
                            anyhow::bail!("repair login rejected again: status {status}");
                        }
                    }
                    Ok(Event::Packet(p)) if p.id == modules::id::SHIP_INITIALIZATION => {
                        println!("<- ShipInitialization — repair accepted");
                        ok = true;
                    }
                    Ok(Event::Packet(p)) => {
                        println!("   pkt {} {:?}", p.id, p.name);
                    }
                    Ok(_) => {}
                    Err(e) => anyhow::bail!("recv after repair request: {e}"),
                }
            }
            if !ok {
                anyhow::bail!("no LoginResponse/ShipInitialization after repair request");
            }

            s.complete_ready(&login.view_data)?;
            // Drain a few frames to confirm hero init.
            for _ in 0..512 {
                if s.world.hero.is_some() {
                    break;
                }
                match s.recv() {
                    Ok(_) => {}
                    Err(e) => {
                        println!("   recv during init drain: {e}");
                        break;
                    }
                }
            }
            match &s.world.hero {
                Some(h) => println!(
                    "REPAIRED — hero {} '{}' on map {} at ({},{}) hp {:.0}/{:.0}",
                    h.user_id, h.user_name, h.map_id, h.x, h.y, h.hitpoints, h.hitpoints_max
                ),
                None => println!("repair accepted; hero init not seen in drain"),
            }
        }
        Err(e) => return Err(e).context("handshake failed"),
    }
    std::io::stdout().flush()?;
    Ok(())
}
