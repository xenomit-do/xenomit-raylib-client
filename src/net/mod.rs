//! Network layer: session thread over `xenomit-packet-client`.
//!
//! Owns the TCP session + world model, pumps inbound frames on a worker
//! thread, exposes an `Arc<Mutex<SharedWorld>>` the game thread snapshots
//! each frame, and takes `NetCommand`s over an mpsc channel for outbound
//! actions (mirroring `ClientCommands`/`ServerCommands` in the AS3 client).

use anyhow::{anyhow, bail, Context, Result};
use std::io;
use std::net::{Shutdown, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};
use xenomit_packet_client::model::WorldState;
use xenomit_packet_client::modules::{self, id};
use xenomit_packet_client::session::{Event, LoginInfo, Phase, Session};

pub mod attributes;
pub mod bootstrap;
pub mod chat;
pub mod combat;
pub mod commands;
pub mod contacts;
pub mod drones;
pub mod group;
pub mod killscreen;
pub mod legacy;
pub mod log;
pub mod menu;
pub mod movement;
pub mod pet;
pub mod portals;
pub mod quests;
pub mod refinery;
pub mod settings;
pub mod ships;
pub mod util;
pub mod world;

pub use attributes::{Booster, HeroExtras};
pub use bootstrap::{map_config_url, resolve_endpoint, ConnectionInfo, FlashVars, MapEndpoint};
pub use chat::{ChatKind, ChatLine};
pub use combat::{EmpKind, StatusFx, WorldFx};
pub use commands::NetCommand;
pub use contacts::{Contact, ContactList};
pub use drones::{DroneEntry, DroneLoadout};
pub use group::{GroupMember, GroupState};
pub use killscreen::KillScreen;
pub use log::{game_log, LogLine, Toast};
pub use menu::{MenuActionStyle, MenuItem, MenuState, SLOT_BAR_SLOTS, STANDARD_SLOT_BAR};
pub use movement::MoveTween;
pub use pet::{PetGear, PetState, PetTable};
pub use portals::Portal;
pub use quests::QuestLog;
pub use refinery::{LabItem, LabItemKind, LabState};
pub use settings::{DisplaySettings, SettingFlag};
pub use ships::ShipInfo;
pub use world::SharedWorld;

/// Socket connect / handshake stall budget.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// Hero-init wait budget (mirrors the bot's ~30s deadline).
const HERO_INIT_TIMEOUT: Duration = Duration::from_secs(30);
/// Post-handshake write budget; reads are governed by `poll`.
const WRITE_TIMEOUT: Duration = Duration::from_secs(15);
/// How often the heartbeat `KeepAlive` goes out (bot uses 20s).
const KEEPALIVE_EVERY: Duration = Duration::from_secs(20);
/// Inbound wait per loop iteration — also the max command latency.
const POLL_WAIT: Duration = Duration::from_millis(20);
/// Frames the pump thread waits for `ShipInitializationCommand`.
const HERO_INIT_FRAMES: usize = 2048;
/// Min interval between `MoveRequest`s on the wire —
/// `EventManager.queueMoveRequest`'s 200 ms pacing.
const MOVE_REQUEST_EVERY: Duration = Duration::from_millis(200);
/// Held-mouse steering samples the cursor more often for more responsive turns.
const HELD_MOVE_REQUEST_EVERY: Duration = Duration::from_millis(100);

/// Coalesces `NetCommand::Move`s: at most one `MoveRequest` per
/// [`MOVE_REQUEST_EVERY`], keeping only the latest target.
pub struct MovePacer {
    pending: Option<(i32, i32, Duration)>,
    last_sent: Option<Instant>,
}

impl Default for MovePacer {
    fn default() -> Self {
        Self::new()
    }
}

impl MovePacer {
    pub fn new() -> Self {
        Self {
            pending: None,
            last_sent: None,
        }
    }

    /// Queue a move target; only the latest one survives.
    pub fn push(&mut self, target: (i32, i32)) {
        self.push_with_interval(target, MOVE_REQUEST_EVERY);
    }

    /// Queue the latest held-mouse target with the faster steering interval.
    pub fn push_held(&mut self, target: (i32, i32)) {
        self.push_with_interval(target, HELD_MOVE_REQUEST_EVERY);
    }

    fn push_with_interval(&mut self, target: (i32, i32), interval: Duration) {
        self.pending = Some((target.0, target.1, interval));
    }

    /// Yield the pending target once the pacing interval has elapsed
    /// (the first push is due immediately). Consumes the target.
    pub fn due(&mut self, now: Instant) -> Option<(i32, i32)> {
        let Some((x, y, interval)) = self.pending else {
            return None;
        };
        if self
            .last_sent
            .is_some_and(|t| now.duration_since(t) < interval)
        {
            return None;
        }
        self.last_sent = Some(now);
        self.pending.take().map(|_| (x, y))
    }
}

/// Live connection handle.
pub struct Connection {
    /// Shared world model; updated by the pump thread, read by the game.
    pub world: Arc<Mutex<SharedWorld>>,
    tx: mpsc::Sender<NetCommand>,
    stop: Arc<AtomicBool>,
    join: Option<thread::JoinHandle<()>>,
    /// Clone of the socket used to unblock the worker on drop.
    control: Arc<Mutex<Option<TcpStream>>>,
}

impl Connection {
    /// Perform the whole connect+login+ready sequence on a worker thread.
    /// `info` carries the game endpoint + login credentials.
    ///
    /// Returns immediately; watch `world.lock().world.hero` /
    /// `world.lock().disconnected` (or `is_alive`/`last_error`) for the
    /// outcome.
    pub fn start(info: ConnectionInfo) -> Result<Self> {
        let world = Arc::new(Mutex::new(SharedWorld::default()));
        let (tx, rx) = mpsc::channel::<NetCommand>();
        let stop = Arc::new(AtomicBool::new(false));
        let control: Arc<Mutex<Option<TcpStream>>> = Arc::new(Mutex::new(None));
        let join = {
            let world = world.clone();
            let stop = stop.clone();
            let control = control.clone();
            thread::Builder::new()
                .name("net-session".into())
                .spawn(move || worker(info, rx, world, stop, control))
                .context("could not spawn net session thread")?
        };
        Ok(Self {
            world,
            tx,
            stop,
            join: Some(join),
            control,
        })
    }

    /// Queue an outbound action (non-blocking).
    pub fn send(&self, cmd: NetCommand) {
        if let Err(e) = self.tx.send(cmd) {
            ::log::info!("net command undelivered (session thread gone): {e}");
        }
    }

    /// True while the pump thread is alive and the socket healthy.
    pub fn is_alive(&self) -> bool {
        !self.stop.load(Ordering::Acquire)
            && self.join.as_ref().is_some_and(|j| !j.is_finished())
            && lock(&self.world).disconnected.is_none()
    }

    /// Last transport error, if the thread died.
    pub fn last_error(&self) -> Option<String> {
        lock(&self.world).disconnected.clone()
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        // Unblock an in-flight poll/handshake read so join can't stall.
        if let Some(stream) = lock(&self.control).take() {
            let _ = stream.shutdown(Shutdown::Both);
        }
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn worker(
    info: ConnectionInfo,
    rx: mpsc::Receiver<NetCommand>,
    shared: Arc<Mutex<SharedWorld>>,
    stop: Arc<AtomicBool>,
    control: Arc<Mutex<Option<TcpStream>>>,
) {
    match run(info, rx, &shared, &stop, &control) {
        Ok(()) => ::log::info!("net session closed"),
        Err(e) => {
            ::log::warn!("net session ended: {e:#}");
            // Don't overwrite a deliberate stop with a shutdown race.
            if !stop.load(Ordering::Acquire) {
                lock(&shared).disconnected = Some(format!("{e:#}"));
            }
        }
    }
    *lock(&control) = None;
}

fn run(
    info: ConnectionInfo,
    rx: mpsc::Receiver<NetCommand>,
    shared: &Mutex<SharedWorld>,
    stop: &AtomicBool,
    control: &Mutex<Option<TcpStream>>,
) -> Result<()> {
    let login = info.login_info();
    let stream = connect_tcp(&info.endpoint)?;
    stream.set_nodelay(true)?;
    stream.set_read_timeout(Some(HERO_INIT_TIMEOUT))?;
    stream.set_write_timeout(Some(WRITE_TIMEOUT))?;
    *lock(control) = stream.try_clone().ok();

    let mut session = Session::new(stream);
    handshake(&mut session, &login, shared).context("game login failed")?;
    // `wait_hero_init`/`handshake` would `recv` internally — the
    // world-init burst (AssetCreateCommand `type.typeValue`, collectables,
    // early ship creates) would be folded into `session.world` and
    // bulk-moved below, so `apply_packet` metadata (asset types, portal
    // rows, fx edges, diagnostics) would never populate. `handshake` and
    // this loop forward each event through `handle_event` instead.
    let mut initialized = session.world.hero.is_some();
    for _ in 0..HERO_INIT_FRAMES {
        if initialized {
            break;
        }
        let event = session.recv().context("could not initialize the hero")?;
        handle_event(event, &mut lock(shared));
        initialized = session.world.hero.is_some();
    }
    if !initialized {
        bail!("game server did not initialize the hero");
    }
    session
        .complete_ready(&login.view_data)
        .context("ready sequence failed")?;
    // `poll` sets/clears its own read timeout per call.
    session.stream.set_read_timeout(None)?;

    // Everything folded in during the handshake (hero, early traffic)
    // becomes the shared model's seed.
    lock(shared).world = std::mem::take(&mut session.world);
    // Vitals trace — ShipInitializationCommand + handshake-time attribute
    // updates are consumed inside `Session::wait_hero_init`, so the only
    // place to see them is the seeded hero state.
    if let Some(h) = &lock(shared).world.hero {
        ::log::debug!(
            "seeded hero vitals hp={}/{} shield={}/{} nano={}/{}",
            h.hitpoints,
            h.hitpoints_max,
            h.shield,
            h.shield_max,
            h.nano_hull,
            h.nano_hull_max
        );
    }

    let mut heartbeat = Instant::now();
    let mut pacer = MovePacer::new();
    loop {
        if stop.load(Ordering::Acquire) {
            let _ = session.send(&modules::channel_close(true));
            return Ok(());
        }

        // Outbound: drain queued commands. Position-dependent packets
        // read the *model* positions (the AS3 checksums use them).
        let mut packets = Vec::new();
        let mut logout = false;
        while let Ok(cmd) = rx.try_recv() {
            if matches!(cmd, NetCommand::Logout) {
                logout = true;
            }
            match cmd {
                // Moves are paced like `queueMoveRequest`: the local
                // prediction starts right away (server sends no
                // HeroMoveCommand), but the wire send is deferred.
                NetCommand::Move { x, y } => {
                    lock(shared).start_hero_move((x, y));
                    pacer.push((x, y));
                }
                NetCommand::MoveHeld { x, y } => {
                    lock(shared).start_hero_move((x, y));
                    pacer.push_held((x, y));
                }
                _ => {
                    let packet = {
                        let mut g = lock(shared);
                        // Checksums must see the *predicted* hero pos.
                        g.sync_hero_model();
                        cmd.to_packet_with(&g.world, Some(&login))
                    };
                    match packet {
                        Some(p) => {
                            ::log::info!("send {:?} -> module {}", cmd, p.id);
                            packets.push(p);
                        }
                        None => ::log::info!("net command dropped (missing prerequisite): {cmd:?}"),
                    }
                }
            }
        }
        // A paced move is due — its position fields must reflect the
        // prediction *now*, not when the click arrived.
        if let Some((x, y)) = pacer.due(Instant::now()) {
            let packet = {
                let mut g = lock(shared);
                g.sync_hero_model();
                NetCommand::Move { x, y }.to_packet_with(&g.world, Some(&login))
            };
            match packet {
                Some(p) => {
                    ::log::info!("send Move {{ x: {x}, y: {y} }} -> module {}", p.id);
                    packets.push(p);
                }
                None => ::log::info!("move dropped (missing prerequisite): {x},{y}"),
            }
        }
        for p in &packets {
            if logout {
                let _ = session.send(p);
            } else {
                session.send(p).context("send failed")?;
            }
        }
        if logout {
            return Ok(());
        }

        // Inbound: wait briefly for one frame, then drain the rest and
        // fold them all under a single world lock.
        let mut events = Vec::new();
        if let Some(event) = session.poll(POLL_WAIT)? {
            events.push(event);
        }
        events.extend(session.pump()?);
        if !events.is_empty() {
            let mut g = lock(shared);
            for event in events {
                handle_event(event, &mut g);
            }
        }

        if heartbeat.elapsed() >= KEEPALIVE_EVERY {
            heartbeat = Instant::now();
            session.send(&modules::keep_alive(false))?;
        }
    }
}

/// `Session::handshake` consumes every frame until `LoginResponse`
/// internally — the server streams the world init *before/around* it
/// (station `AssetCreateCommand`s etc.), so those packets would never
/// reach `apply_packet`. This is the same greeting → version → login
/// sequence, but every non-handshake frame goes through `handle_event`.
fn handshake(
    session: &mut Session<TcpStream>,
    login: &LoginInfo,
    shared: &Mutex<SharedWorld>,
) -> Result<()> {
    use std::io::Write;
    session.stream.write_all(&[0u8])?;
    session.stream.flush()?;
    session.send(&modules::version_request(&login.version))?;
    session.phase = Phase::VersionSent;
    let mut got_version = false;
    for _ in 0..64 {
        match session.recv()? {
            Event::Packet(p) if p.id == id::VERSION_COMMAND => {
                if let Some((_v, equal)) = modules::parse_version_command(&p) {
                    if !equal {
                        bail!("version hash rejected by server");
                    }
                }
                got_version = true;
                break;
            }
            Event::Malformed { error, .. } => return Err(error.into()),
            event => handle_event(event, &mut lock(shared)),
        }
    }
    if !got_version {
        bail!("no VersionCommand within 64 frames");
    }

    session.send(&modules::login_request(
        login.user_id,
        login.faction_id,
        &login.session_id,
        &login.version,
        login.instance_id,
        login.is_mini_client,
    ))?;
    session.phase = Phase::LoginSent;
    for _ in 0..128 {
        match session.recv()? {
            Event::Packet(p) if p.id == id::LOGIN_RESPONSE => {
                let status = p.int("status").unwrap_or(-1) as i16;
                if status != 0 {
                    bail!("game login rejected (status {status})");
                }
                session.world.logged_in = true;
                session.phase = Phase::Ready;
                return Ok(());
            }
            Event::Packet(p) if p.id == id::LOGIN_ERROR => {
                let code = p.int("errorCode").unwrap_or(-1) as i16;
                bail!("game login rejected (code {code})");
            }
            Event::Malformed { error, .. } => return Err(error.into()),
            event => handle_event(event, &mut lock(shared)),
        }
    }
    bail!("no LoginResponse within 128 frames");
}

fn handle_event(event: Event, world: &mut SharedWorld) {
    match event {
        // `apply_packet` folds the module into the shared model and does
        // tween/log bookkeeping. (`Session` already applied it to its own
        // world copy, which is seeded once and otherwise unused.)
        Event::Packet(p) => world.apply_packet(&p),
        Event::Raw(raw) => {
            static SEEN: Mutex<Option<std::collections::HashSet<i16>>> = Mutex::new(None);
            let mut seen = lock(&SEEN);
            let set = seen.get_or_insert_with(std::collections::HashSet::new);
            if set.insert(raw.id) {
                ::log::info!("unhandled module id {}", raw.id);
            }
        }
        Event::Malformed { error, .. } => {
            ::log::info!("malformed inbound module (schema drift?): {error}")
        }
    }
}

/// Resolve `host:port` and try each address with a connect timeout
/// (mirrors the bot's 15s budget).
fn connect_tcp(endpoint: &str) -> Result<TcpStream> {
    let mut last: Option<io::Error> = None;
    for addr in endpoint
        .to_socket_addrs()
        .with_context(|| format!("could not resolve map server {endpoint}"))?
    {
        match TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT) {
            Ok(s) => return Ok(s),
            Err(e) => last = Some(e),
        }
    }
    Err(match last {
        Some(e) => e.into(),
        None => anyhow!("no addresses for map server {endpoint}"),
    })
}

/// Re-export for convenience.
pub type World = WorldState;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pacer_coalesces_to_latest_target() {
        let t0 = Instant::now();
        let mut p = MovePacer::new();
        // First send is immediate.
        p.push((1, 1));
        assert_eq!(p.due(t0), Some((1, 1)));
        // Two pushes inside the window -> nothing due yet.
        p.push((2, 2));
        p.push((3, 3));
        assert_eq!(p.due(t0 + Duration::from_millis(150)), None);
        // Once the 200 ms are up, only the latest target goes out.
        assert_eq!(p.due(t0 + Duration::from_millis(200)), Some((3, 3)));
        // Queue is drained — nothing further is pending.
        assert_eq!(p.due(t0 + Duration::from_millis(400)), None);
    }

    #[test]
    fn pacer_sends_once_interval_elapses() {
        let t0 = Instant::now();
        let mut p = MovePacer::new();
        p.push((10, 20));
        assert_eq!(p.due(t0), Some((10, 20)));
        p.push((30, 40));
        assert_eq!(p.due(t0 + Duration::from_millis(199)), None);
        assert_eq!(p.due(t0 + Duration::from_millis(200)), Some((30, 40)));
    }
}
