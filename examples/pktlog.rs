//! Live packet logger: performs the real bootstrap (backpage → map server →
//! live main.swf version hash → game socket), then dumps every inbound frame
//! for `--seconds`. `--move` sends one `MoveRequest` near the hero and tags
//! the following 10 s of traffic so the server's response is easy to spot.
//!
//!   cargo run --release --example pktlog -- --server tr3 --dosid X --move
//!
//! Output format: one line per frame —
//!   `PKT  <id> <name> {field=value, ...}`   decoded module
//!   `RAW  <id> <n bytes> <hex>`             unregistered module id
//!   `BAD  <id?> <error>`                    schema decode failure
//! Full field dumps are printed for entity/movement modules; other modules
//! print a compact `(n fields)` summary plus a per-id tally at the end.

use anyhow::{Context, Result};
use std::collections::BTreeMap;
use std::io::Write;
use std::net::{TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant};
use xenomit_client::net::bootstrap::{default_http_client, ConnectionInfo};
use xenomit_client::swf_extract::SwfLibrary;
use xenomit_packet_client::packet::{Packet, Value};
use xenomit_packet_client::session::{Event, Session};
use xenomit_packet_client::{modules, session::LoginInfo};

/// Module ids whose packets get a full field dump (entity + movement +
/// lifecycle). Everything else prints a summary only.
const VERBOSE_IDS: &[i16] = &[
    29,     // HeroMoveCommand
    30,     // ShipDestroyedCommand
    31,     // MoveDeniedCommand / similar
    49,     // ShipInitializationCommand
    52,     // ShipRemoveCommand
    83,     // ShipCreateCommand
    90,     // MoveCommand
    147,    // AddOreCommand
    2850,   // AddBeaconCommand
    3245,   // AssetRemoveCommand
    -14859, // AssetCreateCommand
    -29100, // MapAddPOICommand
    -4037,  // AddBoxCommand
    -9316,  // AddFireworkBoxCommand
    14466,  // AddMineCommand
    19063,  // RemoveMineCommand
    18,     // JumpDeniedCommand
    19,     // JumpInitiatedCommand
    20,     // JumpToServerCommand
    138,    // MapChangedCommand
    3389,   // JumpgateCreateCommand
    4274,   // JumpGateRemoveCommand
    29630,  // BeaconCommand (position + nearJumpGate)
];

fn main() -> Result<()> {
    let mut server = "tr3".to_owned();
    let mut dosid = String::new();
    let mut seconds = 60u64;
    let mut do_move = false;
    let mut do_jump = false;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--server" => server = args.next().context("--server needs a value")?,
            "--dosid" => dosid = args.next().context("--dosid needs a value")?,
            "--seconds" => seconds = args.next().context("--seconds needs a value")?.parse()?,
            "--move" => do_move = true,
            // Fly to the nearest jumpgate, send JumpRequest when close,
            // and keep dumping through the (expected) socket close.
            "--jump" => {
                do_jump = true;
                seconds = seconds.max(180);
            }
            "-v" | "--verbose" => {
                env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
                    .init()
            }
            _ => anyhow::bail!("unknown arg {a}"),
        }
    }
    if dosid.is_empty() {
        anyhow::bail!("usage: pktlog --server tr3 --dosid X [--seconds N] [--move] [--jump]");
    }

    // --- bootstrap (same path as the app) --------------------------------
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

    // Live version hash (same as the app).
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
    println!(
        "login user={} faction={} instance={} mini={}",
        info.user_id, info.faction_id, info.instance_id, info.is_mini_client
    );

    // --- game socket ------------------------------------------------------
    let addr = info
        .endpoint
        .to_socket_addrs()?
        .next()
        .context("no addresses")?;
    let stream = TcpStream::connect_timeout(&addr, Duration::from_secs(15))?;
    stream.set_nodelay(true)?;
    let mut s = Session::new(stream);
    let login: LoginInfo = info.login_info();
    s.handshake(&login).context("handshake failed")?;
    println!("handshake ok — waiting for hero init");

    let mut tally: BTreeMap<i16, u64> = BTreeMap::new();
    // jumpgate-create packets arrive during the map load — keep them so
    // --jump can pick a target (gateId, designId, x, y).
    let mut gates: Vec<(i32, i32, i32, i32)> = Vec::new();
    // Last beacon position — the server's authoritative hero pos.
    let mut beacon_pos: Option<(i32, i32)> = None;

    // Feed packets until the hero is initialized, dumping them too —
    // JumpgateCreate lands here.
    let mut frames = 0;
    while s.world.hero.is_none() && frames < 2048 {
        match s.recv() {
            Ok(ev) => {
                observe(&ev, &mut gates, &mut beacon_pos);
                dump(&ev, &mut tally, None);
            }
            Err(e) => {
                println!("!!! init recv: {e}");
                break;
            }
        }
        frames += 1;
    }
    if s.world.hero.is_none() {
        anyhow::bail!("hero never initialized");
    }
    for (gid, design, gx, gy) in &gates {
        println!("gate {gid} design {design} at ({gx},{gy})");
    }
    s.complete_ready(&login.view_data)?;
    println!("ready — hero {:?}", s.world.hero_pos());
    println!("hero {:?}", s.world.hero);

    // --- dump loop ---------------------------------------------------------
    let deadline = Instant::now() + Duration::from_secs(seconds);
    let mut moved_at: Option<Instant> = None;
    // --jump state machine: fly to the nearest gate, press J when close.
    let mut jump_gate: Option<(i32, i32, i32, i32)> = None;
    let mut last_jump_try = Instant::now() - Duration::from_secs(10);
    let mut last_move = Instant::now() - Duration::from_secs(10);
    let start = Instant::now();
    while Instant::now() < deadline {
        let hero = beacon_pos.or_else(|| s.world.hero_pos());
        if do_move && moved_at.is_none() && start.elapsed() > Duration::from_secs(5) {
            if let Some((x, y)) = hero {
                let target = (x + 2000, y);
                let p = modules::move_request((x, y), target);
                println!(">>> move_request {x},{y} -> {target:?} (id {})", p.id);
                if let Err(e) = s.send(&p) {
                    println!("!!! send failed: {e}");
                    break;
                }
                moved_at = Some(Instant::now());
            }
        }
        if do_jump && start.elapsed() > Duration::from_secs(3) {
            if let Some((hx, hy)) = hero {
                // pick the nearest *jumpable* gate once we know our
                // position — designId 1 is the standard map portal; the
                // 30x ids are special gates (galaxy-gate/event portals)
                // the server silently ignores JumpRequest on.
                if jump_gate.is_none() && !gates.is_empty() {
                    let best = gates
                        .iter()
                        .copied()
                        .filter(|(_, d, _, _)| *d == 1)
                        .min_by_key(|(_, _, gx, gy)| (gx - hx) * (gx - hx) + (gy - hy) * (gy - hy))
                        .or_else(|| {
                            gates.iter().copied().min_by_key(|(_, _, gx, gy)| {
                                (gx - hx) * (gx - hx) + (gy - hy) * (gy - hy)
                            })
                        });
                    if let Some(best) = best {
                        println!(
                            ">>> gate {} design {} at ({},{})",
                            best.0, best.1, best.2, best.3
                        );
                        jump_gate = Some(best);
                    }
                    last_move = Instant::now() - Duration::from_secs(10);
                }
                if let Some((_, _, gx, gy)) = jump_gate {
                    let dist = (((gx - hx).pow(2) + (gy - hy).pow(2)) as f64).sqrt();
                    // steer to the gate; re-issue every 5s so the move
                    // target tracks the server's hero position
                    if dist > 700.0 && last_move.elapsed() > Duration::from_secs(5) {
                        let p = modules::move_request((hx, hy), (gx, gy));
                        println!(">>> move to gate ({gx},{gy}) from ({hx},{hy}) dist {dist:.0}");
                        if let Err(e) = s.send(&p) {
                            println!("!!! send failed: {e}");
                            break;
                        }
                        last_move = Instant::now();
                    }
                    // close enough → press J (id 18 = jumpDenied → retry
                    // when closer; nearJumpGate beacon is a hint only)
                    if dist < 900.0 && last_jump_try.elapsed() > Duration::from_secs(3) {
                        let p = modules::jump_request();
                        println!(
                            ">>> jump_request at ({hx},{hy}) dist {dist:.0} (id {})",
                            p.id
                        );
                        if let Err(e) = s.send(&p) {
                            println!("!!! send failed: {e}");
                            break;
                        }
                        last_jump_try = Instant::now();
                    }
                }
            }
        }
        match s.poll(Duration::from_millis(50)) {
            Ok(Some(ev)) => {
                observe(&ev, &mut gates, &mut beacon_pos);
                dump(&ev, &mut tally, moved_at);
            }
            Ok(None) => {}
            Err(e) => {
                println!("!!! recv error (socket closed?): {e}");
                break;
            }
        }
        for ev in s.pump()? {
            observe(&ev, &mut gates, &mut beacon_pos);
            dump(&ev, &mut tally, moved_at);
        }
    }
    println!("\n=== per-id tally (top 30) ===");
    let mut v: Vec<_> = tally.into_iter().collect();
    v.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    for (id, n) in v.into_iter().take(30) {
        println!("  id {id:5}  x{n}");
    }
    Ok(())
}

/// Track the two packets the jump flow cares about.
fn observe(ev: &Event, gates: &mut Vec<(i32, i32, i32, i32)>, beacon: &mut Option<(i32, i32)>) {
    let Event::Packet(p) = ev else { return };
    match p.id {
        // JumpgateCreate — remember gate position + designId.
        3389 => {
            let gid = p.int("gateId").unwrap_or(0) as i32;
            let design = p.int("designId").unwrap_or(0) as i32;
            let x = p.int("x").unwrap_or(0) as i32;
            let y = p.int("y").unwrap_or(0) as i32;
            if gid != 0 && !gates.iter().any(|g| g.0 == gid) {
                gates.push((gid, design, x, y));
            }
        }
        // Beacon — authoritative hero pos for distance checks.
        29630 => {
            let x = p.int("positionX").unwrap_or(0) as i32;
            let y = p.int("positionY").unwrap_or(0) as i32;
            if x != 0 || y != 0 {
                *beacon = Some((x, y));
            }
        }
        _ => {}
    }
}

fn dump(ev: &Event, tally: &mut BTreeMap<i16, u64>, moved_at: Option<Instant>) {
    let tag = if moved_at.is_some_and(|t| t.elapsed() < Duration::from_secs(10)) {
        "*"
    } else {
        " "
    };
    match ev {
        Event::Packet(p) => {
            *tally.entry(p.id).or_default() += 1;
            if VERBOSE_IDS.contains(&p.id) || moved_at.is_some() {
                println!(
                    "{tag}PKT {:5} {} {}",
                    p.id,
                    p.name.unwrap_or("?"),
                    fields(p)
                );
            }
        }
        Event::Raw(r) => {
            *tally.entry(r.id).or_default() += 1;
            let hex: String = r.body.iter().take(24).map(|b| format!("{b:02x}")).collect();
            println!("{tag}RAW {:5} {}B {}", r.id, r.body.len(), hex);
        }
        Event::Malformed { error, body } => {
            let hex: String = body.iter().take(24).map(|b| format!("{b:02x}")).collect();
            println!("{tag}BAD  {error}  {}B {hex}", body.len());
        }
    }
    let _ = std::io::stdout().flush();
}

/// Compact `name=value` rendering of every top-level field.
fn fields(p: &Packet) -> String {
    let mut out = String::from("{");
    for (name, v) in &p.fields {
        if out.len() > 1 {
            out.push_str(", ");
        }
        out.push_str(name.unwrap_or("?"));
        out.push('=');
        out.push_str(&short(v));
        if out.len() > 900 {
            out.push_str(" …");
            break;
        }
    }
    out.push('}');
    out
}

fn short(v: &Value) -> String {
    match v {
        Value::Int(i) => i.to_string(),
        Value::Str(s) => format!("{s:?}"),
        Value::Bool(b) => b.to_string(),
        Value::F64(f) => format!("{f:.2}"),
        Value::Module(m) => {
            format!("Module#{:?}({})", m.id, m.fields.len())
        }
        Value::List(l) => format!("[{}]", l.len()),
        _ => "?".into(),
    }
}
