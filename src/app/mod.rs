//! App state machine: SignIn -> Loading -> Playing -> Failed.
//! Owns the raylib window loop — `MainClientApplication` equivalent.
//!
//! Bootstrap runs on a worker thread (`bootstrap` below): backpage auth,
//! map-server resolution, game socket, asset databases. It reports
//! `(status, progress)` on a side channel and delivers the finished
//! [`BootstrapResult`] once on `LoadingState::rx`; the UI thread only
//! ever polls, never blocks.

use crate::assets::AssetManager;
use crate::game::{Game, BASE_MAP_SIZE};
use crate::net::{Connection, ConnectionInfo, NetCommand};
use crate::render::{self, Fonts, TextureCache};
use anyhow::{anyhow, Context, Result};
use raylib::prelude::*;
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::{Mutex, MutexGuard};
use std::thread;

/// Character cap for sign-in fields (dosids are ~30 chars).
const MAX_FIELD_LEN: usize = 128;
/// Sign-in field indices (`SignInState::focus`).
const FIELD_SERVER: usize = 0;
const FIELD_DOSID: usize = 1;
const FIELD_COUNT: usize = 2;

// DarkOrbit-ish chrome for the pre-game screens.
const BG: Color = Color::new(8, 10, 16, 255);
const FIELD_BG: Color = Color::new(11, 14, 21, 255);
const ACCENT: Color = Color::new(0, 174, 239, 255);
const BORDER: Color = Color::new(58, 68, 88, 255);
const TEXT: Color = Color::new(220, 226, 235, 255);
const PLACEHOLDER: Color = Color::new(96, 104, 120, 255);
const ERROR: Color = Color::new(235, 87, 87, 255);
const BUTTON: Color = Color::new(0, 120, 170, 255);
const BUTTON_HOVER: Color = Color::new(0, 150, 205, 255);
const BUTTON_DISABLED: Color = Color::new(35, 45, 60, 255);

/// Static configuration the binary hands to the app (the CLI surface).
pub struct AppConfig {
    /// `--server` — pre-fills/joins the sign-in form ("tr3" or a host/URL).
    pub server: Option<String>,
    /// `--dosid` — session cookie; with `server` set, skips the form.
    pub dosid: Option<String>,
    /// `--main-swf` — the stock client's main.swf (None until located).
    pub main_swf: Option<PathBuf>,
    /// `--assets-dir` — CDN cache directory for the asset pipeline.
    pub assets_dir: PathBuf,
    /// `--shot-dir` — screenshot output for F12/`--autoshot`.
    pub shot_dir: PathBuf,
    /// `--autoshot` — save a frame every N seconds (dev evidence runs).
    pub autoshot_secs: Option<u64>,
    /// `--shot-on-hit` — capture a frame ~250 ms after each inbound
    /// combat event (bolts mid-flight; dev evidence runs).
    pub shot_on_hit: bool,
    /// Force the touch-first layout while running the desktop preview.
    pub mobile_preview: bool,
}

pub enum App {
    /// Collecting dosid/server (CLI flags may pre-fill and skip this).
    SignIn(SignInState),
    /// Async bootstrap: flashvars -> resources.xml -> maps.php ->
    /// maps-config -> embedded XMLs -> socket connect.
    Loading(LoadingState),
    /// Live game.
    Playing(Box<PlayState>),
    /// Fatal error page with message.
    Failed(String),
}

#[derive(Default)]
pub struct SignInState {
    pub server: String,
    pub dosid: String,
    pub focus: usize,
    pub error: Option<String>,
}

pub struct LoadingState {
    /// Progress label shown on the loading screen.
    pub status: String,
    /// 0.0..=1.0 coarse progress.
    pub progress: f32,
    /// Receiver for the worker thread result.
    pub rx: mpsc::Receiver<anyhow::Result<BootstrapResult>>,
    /// Side channel for `(status, progress)` worker updates.
    progress_rx: mpsc::Receiver<(String, f32)>,
}

/// Everything bootstrap produced: session + asset database.
pub struct BootstrapResult {
    pub conn: Connection,
    pub assets: AssetManager,
    pub map_id: i32,
    pub map_bounds: (f32, f32),
    /// What a reconnect needs to re-dial: credentials + client version.
    pub reconnect: ReconnectInfo,
}

/// Credentials remembered for socket re-dials (portal jumps close the
/// connection on purpose — `Main.scheduledDisconnect` — and ordinary
/// drops get the same treatment with a small backoff).
#[derive(Clone)]
pub struct ReconnectInfo {
    pub server: String,
    pub dosid: String,
    /// Live `VersionHash` resolved during bootstrap — a reconnect skips
    /// the main.swf download and reuses it.
    pub version: String,
}

pub struct PlayState {
    pub conn: Connection,
    pub assets: AssetManager,
    pub game: Game,
    pub tex: TextureCache,
    /// Re-dial bookkeeping.
    pub reconnect: ReconnectInfo,
    /// In-flight re-dial (worker resolves fresh flashvars + endpoint).
    pub reconnect_rx: Option<mpsc::Receiver<Result<ConnectionInfo>>>,
    /// Re-dials attempted since the socket last carried a live hero.
    pub reconnect_attempts: u32,
    /// Earliest time the next attempt may start (backoff schedule).
    pub reconnect_at: std::time::Instant,
    /// When the current outage started — reconnect gives up after
    /// `RECONNECT_BUDGET` (the server ghosts a dead session for ~5 min).
    pub disconnected_at: Option<std::time::Instant>,
    /// `--autoshot` cadence (None = F12 captures only).
    pub autoshot_every: Option<std::time::Duration>,
    /// Next scheduled autoshot.
    pub next_autoshot: std::time::Instant,
    /// Render-thread audio device (`AudioManager`); inert when the
    /// device failed to open.
    pub audio: crate::audio::AudioDevice,
}

/// How long the client keeps re-dialling a dropped socket: the server
/// ghosts a dead session for ~5 min and refuses new logins meanwhile —
/// retries at +5s, +15s, +30s, then every 60s bridge that window.
const RECONNECT_BUDGET: std::time::Duration = std::time::Duration::from_secs(360);

/// Wait before reconnect attempt `n` (1-based).
fn reconnect_wait(attempt: u32) -> std::time::Duration {
    std::time::Duration::from_secs(match attempt {
        1 => 5,
        2 => 15,
        3 => 30,
        _ => 60,
    })
}

impl Drop for PlayState {
    /// Graceful logout on every exit path (window close included); the
    /// `Connection` drop afterwards stops the socket and joins the worker.
    fn drop(&mut self) {
        self.conn.send(NetCommand::Logout);
    }
}

impl App {
    /// Entry state: complete `--server`/`--dosid` credentials jump straight
    /// to [`App::Loading`]; anything missing pre-fills the sign-in form.
    pub fn initial(cfg: &AppConfig) -> Self {
        let server = cfg.server.as_deref().unwrap_or("").trim();
        let dosid = cfg.dosid.as_deref().unwrap_or("").trim();
        if !server.is_empty() && !dosid.is_empty() {
            return App::Loading(LoadingState::spawn(
                server.to_owned(),
                dosid.to_owned(),
                cfg,
            ));
        }
        App::SignIn(SignInState {
            server: server.to_owned(),
            dosid: dosid.to_owned(),
            // Skip over fields the CLI already supplied.
            focus: if server.is_empty() {
                FIELD_SERVER
            } else {
                FIELD_DOSID
            },
            error: None,
        })
    }

    /// One frame: poll input, advance the state machine, draw the screen.
    ///
    /// Each state handler returns `Some(next)` to transition; the draw for
    /// the *current* state always happens inside its handler.
    pub fn step(
        &mut self,
        rl: &mut RaylibHandle,
        thread: &RaylibThread,
        fonts: &Fonts,
        cfg: &AppConfig,
    ) {
        let next = match self {
            App::SignIn(state) => step_sign_in(state, rl, thread, cfg),
            App::Loading(state) => step_loading(state, rl, thread, cfg),
            App::Playing(state) => step_playing(state, rl, thread, fonts, cfg),
            App::Failed(msg) => step_failed(msg, rl, thread),
        };
        if let Some(next) = next {
            *self = next;
        }
    }

    /// Graceful shutdown hook for the binary's exit path. `PlayState`'s
    /// `Drop` already queues `NetCommand::Logout`; this exists so the
    /// intent is explicit at the call site.
    pub fn shutdown(&mut self) {
        if let App::Playing(state) = self {
            state.conn.send(NetCommand::Logout);
        }
    }
}

impl LoadingState {
    /// Spawn the bootstrap worker thread and return the receiving state.
    fn spawn(server: String, dosid: String, cfg: &AppConfig) -> Self {
        let (tx, rx) = mpsc::channel();
        let (ptx, progress_rx) = mpsc::channel();
        let main_swf = cfg.main_swf.clone();
        let assets_dir = cfg.assets_dir.clone();
        let worker_tx = tx.clone();
        let spawned = thread::Builder::new()
            .name("bootstrap".into())
            .spawn(move || {
                let result = bootstrap(&server, &dosid, main_swf, &assets_dir, &ptx);
                let _ = worker_tx.send(result);
            });
        if let Err(e) = spawned {
            // Channel still delivers — the UI lands on `Failed` cleanly.
            let _ = tx.send(Err(anyhow!("could not spawn bootstrap thread: {e}")));
        }
        Self {
            status: "starting".into(),
            progress: 0.0,
            rx,
            progress_rx,
        }
    }
}

/// Worker-side bootstrap: backpage auth -> endpoint resolve -> game socket
/// -> asset databases. Blocking; `progress` receives `(status, 0..=1)`.
fn bootstrap(
    server: &str,
    dosid: &str,
    main_swf: Option<PathBuf>,
    assets_dir: &std::path::Path,
    progress: &mpsc::Sender<(String, f32)>,
) -> Result<BootstrapResult> {
    let say = |status: &str, p: f32| {
        let _ = progress.send((status.to_owned(), p));
    };

    say("authenticating", 0.05);
    let session = xenomit_backpage::session::Session::new(dosid, server);
    let mut client = xenomit_backpage::client::BackpageClient::new(session)
        .context("could not create backpage client")?;
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("could not start the tokio runtime")?;
    rt.block_on(xenomit_backpage::launch::load(&mut client))
        .context("backpage launch failed (expired dosid?)")?;
    let vars = client.session().flashvars.clone();
    let instance = client.session().instance();

    say("resolving map server", 0.35);
    let mut info = rt
        .block_on(ConnectionInfo::from_flashvars(&vars, &instance))
        .context("could not resolve the map server")?;

    // The game client is versioned per build: the login `VersionHash` and
    // the embedded asset XMLs must come from the *deployed* main.swf, so
    // download it from the CDN (local file is the offline fallback).
    say("loading game client", 0.45);
    let http = crate::net::bootstrap::default_http_client()?;
    let swf = load_game_swf(&rt, &http, &vars, main_swf.as_deref(), assets_dir)?;
    if let Some(hash) = swf.version_hash() {
        info.version = hash;
    }

    say("connecting to the game server", 0.6);
    // `info` moves into the connection — keep the resolved version hash
    // for reconnects (they skip the main.swf download).
    let live_version = info.version.clone();
    let conn = Connection::start(info).context("could not start the game connection")?;

    say("loading assets", 0.8);
    let assets = AssetManager::load_with_library(assets_dir, &swf, &vars)
        .context("asset bootstrap failed")?;
    // Server keys (log lines, titles) resolve on the pump thread, so the
    // language table lives in the shared world too.
    lock(&conn.world).set_locale(assets.locale.clone());

    let map_id = vars.get("mapID").and_then(|v| v.parse().ok()).unwrap_or(0);
    let map_bounds = assets
        .maps
        .get(map_id)
        .map(|m| {
            (
                BASE_MAP_SIZE.0 * m.scale_factor,
                BASE_MAP_SIZE.1 * m.scale_factor,
            )
        })
        .unwrap_or(BASE_MAP_SIZE);

    say("entering map", 1.0);
    Ok(BootstrapResult {
        conn,
        assets,
        map_id,
        map_bounds,
        reconnect: ReconnectInfo {
            server: server.to_owned(),
            dosid: dosid.to_owned(),
            version: live_version,
        },
    })
}

/// Re-dial path for portal jumps and dropped sockets: fresh backpage
/// `launch::load` (the new `mapID` flashvar resolves the right map
/// server), then endpoint resolution. Runs on a worker thread.
fn reconnect_dial(info: &ReconnectInfo) -> Result<ConnectionInfo> {
    let session = xenomit_backpage::session::Session::new(&info.dosid, &info.server);
    let mut client = xenomit_backpage::client::BackpageClient::new(session)
        .context("could not create backpage client")?;
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("could not start the tokio runtime")?;
    rt.block_on(xenomit_backpage::launch::load(&mut client))
        .context("backpage re-launch failed")?;
    let vars = client.session().flashvars.clone();
    let instance = client.session().instance();
    let mut conn = rt
        .block_on(ConnectionInfo::from_flashvars(&vars, &instance))
        .context("could not resolve the map server")?;
    conn.version = info.version.clone();
    Ok(conn)
}

/// Download the deployed `main.swf` (`BootConfig::main_swf_url` —
/// `{cdn}{gameclientPath}/main.swf?__cv={gameclientHash}`) so the login
/// `VersionHash` and the embedded XMLs match the live build. Falls back
/// to the local file when the URL is unknown or the download fails.
fn load_game_swf(
    rt: &tokio::runtime::Runtime,
    http: &reqwest::Client,
    vars: &crate::net::FlashVars,
    local: Option<&std::path::Path>,
    cache_dir: &std::path::Path,
) -> Result<crate::swf_extract::SwfLibrary> {
    use crate::swf_extract::SwfLibrary;
    if let Ok(cfg) = crate::assets::bootstrap::BootConfig::from_flashvars(vars) {
        if let Some(url) = cfg.main_swf_url() {
            let cache_key = if cfg.gameclient_hash.is_empty() {
                format!("{:016x}", hash_path(&url))
            } else {
                cfg.gameclient_hash
                    .chars()
                    .map(|c| {
                        if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                            c
                        } else {
                            '_'
                        }
                    })
                    .collect()
            };
            let cached = cache_dir.join("main-swf").join(format!("{cache_key}.swf"));
            if cached.is_file() {
                match SwfLibrary::open(&cached) {
                    Ok(lib) => {
                        log::info!("game client cache hit: {}", cached.display());
                        return Ok(lib);
                    }
                    Err(e) => log::warn!("cached main.swf is invalid ({e}); downloading again"),
                }
            }
            // `send()`/`bytes()` must sit inside the async block — reqwest
            // builds the client timeout (`tokio::time::Sleep`) eagerly when the
            // future is *constructed*, which panics without a reactor.
            match rt.block_on(async {
                http.get(&url)
                    .send()
                    .await?
                    .error_for_status()?
                    .bytes()
                    .await
            }) {
                Ok(bytes) => match SwfLibrary::from_bytes(&bytes) {
                    Ok(lib) => {
                        if let Err(e) =
                            std::fs::create_dir_all(cached.parent().unwrap_or(cache_dir))
                                .and_then(|_| std::fs::write(&cached, &bytes))
                        {
                            log::warn!("main.swf cache write failed: {e}");
                        }
                        log::info!("game client downloaded from {url}");
                        return Ok(lib);
                    }
                    Err(e) => log::warn!("live main.swf unparsable ({e}) — using local copy"),
                },
                Err(e) => log::warn!("live main.swf download failed ({e}) — using local copy"),
            }
        }
    }
    let path = local.context("main.swf not found — pass --main-swf <path>")?;
    SwfLibrary::open(path).with_context(|| format!("cannot open {}", path.display()))
}

fn hash_path(s: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

// ---------------------------------------------------------------------------
// SignIn
// ---------------------------------------------------------------------------

/// Centered form geometry shared between the input and draw passes.
struct FormLayout {
    server: Rectangle,
    dosid: Rectangle,
    paste: Rectangle,
    button: Rectangle,
}

fn sign_in_layout(w: i32, h: i32) -> FormLayout {
    let fw = 420.0;
    let fh = 44.0;
    let cx = w as f32 * 0.5;
    let cy = h as f32 * 0.5;
    FormLayout {
        server: Rectangle::new(cx - fw * 0.5, cy - 72.0, fw, fh),
        dosid: Rectangle::new(cx - fw * 0.5, cy + 8.0, fw, fh),
        paste: Rectangle::new(cx - 110.0, cy + 64.0, 220.0, 36.0),
        button: Rectangle::new(cx - 110.0, cy + 108.0, 220.0, 44.0),
    }
}

fn step_sign_in(
    state: &mut SignInState,
    rl: &mut RaylibHandle,
    thread: &RaylibThread,
    cfg: &AppConfig,
) -> Option<App> {
    let layout = sign_in_layout(rl.get_screen_width(), rl.get_screen_height());
    let mouse = rl.get_mouse_position();
    let clicked = rl.is_mouse_button_pressed(MouseButton::MOUSE_BUTTON_LEFT);

    // Focus: Tab/Down cycles forward, Up back, clicking picks a field.
    if rl.is_key_pressed(KeyboardKey::KEY_TAB) || rl.is_key_pressed(KeyboardKey::KEY_DOWN) {
        state.focus = (state.focus + 1) % FIELD_COUNT;
    }
    if rl.is_key_pressed(KeyboardKey::KEY_UP) {
        state.focus = (state.focus + FIELD_COUNT - 1) % FIELD_COUNT;
    }
    if clicked {
        if layout.server.check_collision_point_rec(mouse) {
            state.focus = FIELD_SERVER;
        } else if layout.dosid.check_collision_point_rec(mouse) {
            state.focus = FIELD_DOSID;
        }
        #[cfg(target_os = "android")]
        if layout.server.check_collision_point_rec(mouse)
            || layout.dosid.check_collision_point_rec(mouse)
        {
            unsafe { raylib::ffi::XenomitShowSoftKeyboard(true) };
        }
    }

    // Clipboard access is explicit: Android reads clipboard contents only
    // after the user presses this visible paste action.
    if clicked && layout.paste.check_collision_point_rec(mouse) {
        state.focus = FIELD_DOSID;
        if let Ok(text) = rl.get_clipboard_text() {
            state.dosid = text
                .chars()
                .filter(|c| !c.is_control())
                .take(MAX_FIELD_LEN)
                .collect();
        }
    }

    // Text entry into the focused field.
    {
        let field = match state.focus {
            FIELD_DOSID => &mut state.dosid,
            _ => &mut state.server,
        };
        while let Some(c) = rl.get_char_pressed() {
            if !c.is_control() && field.len() < MAX_FIELD_LEN {
                field.push(c);
            }
        }
        if rl.is_key_pressed(KeyboardKey::KEY_BACKSPACE)
            || rl.is_key_pressed_repeat(KeyboardKey::KEY_BACKSPACE)
        {
            field.pop();
        }
        let paste = (rl.is_key_down(KeyboardKey::KEY_LEFT_CONTROL)
            || rl.is_key_down(KeyboardKey::KEY_RIGHT_CONTROL))
            && rl.is_key_pressed(KeyboardKey::KEY_V);
        if paste {
            if let Ok(text) = rl.get_clipboard_text() {
                for c in text.chars() {
                    if !c.is_control() && field.len() < MAX_FIELD_LEN {
                        field.push(c);
                    }
                }
            }
        }
    }

    // Submit on Enter or the Connect button.
    let submit = rl.is_key_pressed(KeyboardKey::KEY_ENTER)
        || rl.is_key_pressed(KeyboardKey::KEY_KP_ENTER)
        || (clicked && layout.button.check_collision_point_rec(mouse));
    let mut next = None;
    if submit {
        #[cfg(target_os = "android")]
        unsafe {
            raylib::ffi::XenomitShowSoftKeyboard(false);
        }
        let server = state.server.trim();
        let dosid = state.dosid.trim();
        if server.is_empty() || dosid.is_empty() {
            state.error = Some("server and dosid are both required".into());
        } else {
            next = Some(App::Loading(LoadingState::spawn(
                server.to_owned(),
                dosid.to_owned(),
                cfg,
            )));
        }
    }

    let mut d = rl.begin_drawing(thread);
    draw_sign_in(&mut d, state, &layout);
    next
}

fn draw_sign_in(d: &mut RaylibDrawHandle, state: &SignInState, layout: &FormLayout) {
    let w = d.get_screen_width();
    let h = d.get_screen_height();
    d.clear_background(BG);

    let title = "XENOMIT";
    let tw = d.measure_text(title, 44);
    d.draw_text(title, (w - tw) / 2, h / 2 - 170, 44, ACCENT);
    let subtitle = "native DarkOrbit client";
    let sw = d.measure_text(subtitle, 18);
    d.draw_text(subtitle, (w - sw) / 2, h / 2 - 118, 18, PLACEHOLDER);

    draw_field(
        d,
        "SERVER",
        &state.server,
        "e.g. tr3",
        &layout.server,
        state.focus == FIELD_SERVER,
        d.get_time(),
    );
    draw_field(
        d,
        "DOSID",
        &state.dosid,
        "session cookie",
        &layout.dosid,
        state.focus == FIELD_DOSID,
        d.get_time(),
    );

    let paste_hover = layout
        .paste
        .check_collision_point_rec(d.get_mouse_position());
    d.draw_rectangle_rec(
        layout.paste,
        if paste_hover { BUTTON_HOVER } else { FIELD_BG },
    );
    d.draw_rectangle_lines_ex(layout.paste, 1.0, BORDER);
    let paste_label = "PASTE DOSID";
    let paste_width = d.measure_text(paste_label, 15);
    d.draw_text(
        paste_label,
        (layout.paste.x + (layout.paste.width - paste_width as f32) * 0.5) as i32,
        (layout.paste.y + (layout.paste.height - 15.0) * 0.5) as i32,
        15,
        TEXT,
    );

    // Connect button (disabled look until the form validates).
    let ready = !state.server.trim().is_empty() && !state.dosid.trim().is_empty();
    let hover = layout
        .button
        .check_collision_point_rec(d.get_mouse_position());
    let bg = if !ready {
        BUTTON_DISABLED
    } else if hover {
        BUTTON_HOVER
    } else {
        BUTTON
    };
    let b = &layout.button;
    d.draw_rectangle_rec(*b, bg);
    d.draw_rectangle_lines_ex(*b, 1.0, BORDER);
    let label = "CONNECT";
    let lw = d.measure_text(label, 20);
    d.draw_text(
        label,
        (b.x + (b.width - lw as f32) * 0.5) as i32,
        (b.y + (b.height - 20.0) * 0.5) as i32,
        20,
        if ready { Color::WHITE } else { PLACEHOLDER },
    );

    if let Some(err) = &state.error {
        let ew = d.measure_text(err, 16);
        d.draw_text(
            err,
            (w - ew) / 2,
            (layout.button.y + layout.button.height + 18.0) as i32,
            16,
            ERROR,
        );
    }
}

/// One labeled text field with focus border, placeholder and blink cursor.
fn draw_field(
    d: &mut RaylibDrawHandle,
    label: &str,
    value: &str,
    placeholder: &str,
    rect: &Rectangle,
    focused: bool,
    time: f64,
) {
    d.draw_text(
        label,
        rect.x as i32,
        (rect.y - 20.0) as i32,
        14,
        PLACEHOLDER,
    );
    d.draw_rectangle_rec(*rect, FIELD_BG);
    d.draw_rectangle_lines_ex(
        *rect,
        if focused { 2.0 } else { 1.0 },
        if focused { ACCENT } else { BORDER },
    );

    let tx = rect.x as i32 + 10;
    let ty = (rect.y + (rect.height - 20.0) * 0.5) as i32;
    if value.is_empty() {
        d.draw_text(placeholder, tx, ty, 20, PLACEHOLDER);
    } else {
        d.draw_text(value, tx, ty, 20, TEXT);
        // Blinking cursor at the end of the input while focused.
        if focused && ((time * 2.0) as i64) % 2 == 0 {
            let cursor_x = tx + d.measure_text(value, 20) + 2;
            d.draw_text("_", cursor_x, ty, 20, ACCENT);
        }
    }
}

// ---------------------------------------------------------------------------
// Loading
// ---------------------------------------------------------------------------

fn step_loading(
    state: &mut LoadingState,
    rl: &mut RaylibHandle,
    thread: &RaylibThread,
    cfg: &AppConfig,
) -> Option<App> {
    // Drain progress updates, then the finished result.
    while let Ok((status, p)) = state.progress_rx.try_recv() {
        state.status = status;
        state.progress = p;
    }
    match state.rx.try_recv() {
        Ok(Ok(boot)) => {
            let mut game = Game::new(boot.map_id, boot.map_bounds);
            game.shot_on_fx = cfg.shot_on_hit;
            game.set_layout_path(cfg.assets_dir.join("window_layout.json"));
            game.set_toolbar_path(cfg.assets_dir.join("toolbar_layout.json"));
            game.set_ui_preferences_path(cfg.assets_dir.join("ui_settings.json"));
            if cfg.mobile_preview || cfg!(target_os = "android") {
                game.set_mobile_hud(true);
            }
            return Some(App::Playing(Box::new(PlayState {
                conn: boot.conn,
                assets: boot.assets,
                game,
                tex: TextureCache::default(),
                reconnect: boot.reconnect,
                reconnect_rx: None,
                reconnect_attempts: 0,
                reconnect_at: std::time::Instant::now(),
                disconnected_at: None,
                autoshot_every: cfg.autoshot_secs.map(std::time::Duration::from_secs),
                next_autoshot: std::time::Instant::now(),
                audio: crate::audio::AudioDevice::new(),
            })));
        }
        Ok(Err(e)) => return Some(App::Failed(format!("{e:#}"))),
        Err(mpsc::TryRecvError::Empty) => {}
        Err(mpsc::TryRecvError::Disconnected) => {
            return Some(App::Failed("bootstrap worker died without a result".into()));
        }
    }

    // Esc cancels back to the form; the detached worker exits quietly.
    let cancel = rl.is_key_pressed(KeyboardKey::KEY_ESCAPE);

    let mut d = rl.begin_drawing(thread);
    let w = d.get_screen_width();
    let h = d.get_screen_height();
    d.clear_background(BG);

    let title = "CONNECTING";
    let tw = d.measure_text(title, 36);
    d.draw_text(title, (w - tw) / 2, h / 2 - 90, 36, ACCENT);

    let status = format!("{}...", state.status);
    let sw = d.measure_text(&status, 18);
    d.draw_text(&status, (w - sw) / 2, h / 2 - 32, 18, TEXT);

    // Progress bar.
    let bar_w = 420.0;
    let bar = Rectangle::new(w as f32 * 0.5 - bar_w * 0.5, h as f32 + 8.0, bar_w, 14.0);
    d.draw_rectangle_rec(bar, FIELD_BG);
    let fill = Rectangle::new(
        bar.x,
        bar.y,
        bar.width * state.progress.clamp(0.0, 1.0),
        bar.height,
    );
    d.draw_rectangle_rec(fill, ACCENT);
    d.draw_rectangle_lines_ex(bar, 1.0, BORDER);

    let pct = format!("{:.0}%", state.progress.clamp(0.0, 1.0) * 100.0);
    let pw = d.measure_text(&pct, 16);
    d.draw_text(
        &pct,
        (w - pw) / 2,
        (bar.y + bar.height + 14.0) as i32,
        16,
        PLACEHOLDER,
    );

    let hint = "Esc to cancel";
    let hw = d.measure_text(hint, 14);
    d.draw_text(hint, (w - hw) / 2, h - 48, 14, PLACEHOLDER);
    drop(d);

    if cancel {
        Some(App::SignIn(SignInState::default()))
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Playing
// ---------------------------------------------------------------------------

fn step_playing(
    state: &mut PlayState,
    rl: &mut RaylibHandle,
    thread: &RaylibThread,
    fonts: &Fonts,
    cfg: &AppConfig,
) -> Option<App> {
    let dt = rl.get_frame_time();

    // Screenshots: F12 on demand + `--autoshot` cadence. Capturing here
    // (before this frame's BeginDrawing) reads the last presented buffer
    // — full framebuffer, so the image is DPI-correct regardless of the
    // logical window size.
    let auto_due = state
        .autoshot_every
        .is_some_and(|_| std::time::Instant::now() >= state.next_autoshot);
    // `--shot-on-hit`: the fx drain armed a capture ~250 ms after an
    // inbound combat event — bolts are mid-flight in this frame.
    let fx_due = state
        .game
        .shot_at
        .is_some_and(|t| t.elapsed() >= std::time::Duration::from_millis(250));
    if rl.is_key_pressed(KeyboardKey::KEY_F12) || auto_due || fx_due {
        state.game.shot_at = None;
        let ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let path = cfg.shot_dir.join(format!("shot_{ms}.png"));
        // `LoadImageFromScreen` reads `screen * GetWindowScaleDPI` — the
        // real framebuffer size under any DPI scale. (`TakeScreenshot`
        // mis-sizes by `render * scale` on Windows and also drops the
        // directory part of the path, so it can't be used.)
        rl.load_image_from_screen(thread)
            .export_image(&path.to_string_lossy());
        log::info!("screenshot saved: {}", path.display());
        if let Some(every) = state.autoshot_every {
            state.next_autoshot = std::time::Instant::now() + every;
        }
    }

    state.assets.pump();
    // GPU uploads need `&mut rl` — must run before `begin_drawing`.
    state.tex.sync(rl, thread, &state.assets.sprites);

    let mut cmds: Vec<NetCommand> = Vec::new();
    {
        // One lock per frame across update+draw (the pump thread's budget).
        let mut world = lock(&state.conn.world);
        state
            .game
            .update_with_rl(rl, dt, &mut world, &mut state.assets, &mut cmds);
        // Audio: drain the sfx queue, sequence map/combat music, pump
        // fades — once per frame inside the world lock.
        state.audio.tick(
            &mut state.assets,
            &world,
            &state.game.preferences.audio,
            state.game.map_id,
            dt,
        );
        state
            .tex
            .sync_models3d(rl, thread, &state.assets, &state.game, &world);

        let mut d = rl.begin_drawing(thread);
        render::draw_scene(
            &mut d,
            &mut state.game,
            &world,
            &mut state.assets,
            &mut state.tex,
            fonts,
        );
        // Release the world guard *before* `d` drops: EndDrawing swaps
        // buffers and waits on vsync, and holding the lock through that
        // starves the net thread for a whole frame.
        drop(world);
    }
    for cmd in cmds {
        state.conn.send(cmd);
    }

    // --- reconnect --------------------------------------------------------
    // Portal jumps close the socket on purpose (`scheduledDisconnect`);
    // ordinary drops get the same treatment: re-run the backpage launch
    // (fresh `mapID` flashvar → endpoint) on a worker thread. The first
    // attempt is immediate, then `reconnect_wait` spaces retries 5/15/30s
    // and every 60s after that until `RECONNECT_BUDGET` (360s — the
    // server ghosts a dead session for ~5 min). Esc aborts unexpected
    // drops; window close tears everything down via `PlayState::drop`.
    if let Some(rx) = &state.reconnect_rx {
        let result = match rx.try_recv() {
            Ok(r) => Some(r),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => Some(Err(anyhow!("reconnect worker died"))),
        };
        if let Some(result) = result {
            state.reconnect_rx = None;
            match result.and_then(|info| {
                Connection::start(info).context("could not start the game connection")
            }) {
                Ok(conn) => {
                    log::info!("re-dialed the game server");
                    {
                        let mut w = lock(&conn.world);
                        // A fresh socket gets a fresh SharedWorld — keep
                        // the already-loaded language table.
                        w.set_locale(state.assets.locale.clone());
                        w.push_log("reconnected");
                    }
                    state.conn = conn;
                }
                Err(e) => {
                    log::warn!("reconnect attempt failed: {e:#}");
                    let mut w = lock(&state.conn.world);
                    w.push_log(format!("reconnect failed: {e:#}"));
                }
            }
        }
    }
    if state.conn.is_alive() {
        // A connection that delivered the hero counts as healthy —
        // reset the backoff budget. (The fresh socket is "alive" while
        // it handshakes, so this only resets once the world populates.)
        if lock(&state.conn.world).world.hero.is_some() {
            state.reconnect_attempts = 0;
            state.disconnected_at = None;
        }
        state.game.reconnect_note = None;
    } else {
        let (expecting, jump_target) = {
            let w = lock(&state.conn.world);
            (w.expect_disconnect, w.jump_target)
        };
        // Esc aborts the retry loop for an *unexpected* drop (portal
        // jumps/relogins are deliberate — Esc leaves them alone).
        if !expecting && rl.is_key_pressed(KeyboardKey::KEY_ESCAPE) {
            log::info!("reconnect aborted by user");
            return Some(App::SignIn(SignInState::default()));
        }
        if state.disconnected_at.is_none() {
            state.disconnected_at = Some(std::time::Instant::now());
            // First try right away — jumps want zero added latency and
            // an unexpected drop still gets the 5/15/30/60 spacing on
            // subsequent attempts (the ghost-session window needs it).
            state.reconnect_at = std::time::Instant::now();
        }
        let now = std::time::Instant::now();
        let gave_up = state
            .disconnected_at
            .is_some_and(|t| now.duration_since(t) > RECONNECT_BUDGET);
        state.game.reconnect_note = if gave_up {
            None // falls back to the red "disconnected" banner
        } else if expecting {
            Some(match jump_target {
                Some(m) => format!("Jumping to map {m}…"),
                None => "Jumping…".to_owned(),
            })
        } else if state.reconnect_rx.is_some() {
            Some(format!(
                "Connection lost — reconnecting (attempt {})…",
                state.reconnect_attempts
            ))
        } else {
            let in_s = state.reconnect_at.saturating_duration_since(now).as_secs();
            Some(format!(
                "Connection lost — retry {} in {in_s}s (Esc to give up)",
                state.reconnect_attempts + 1
            ))
        };
        if !gave_up && state.reconnect_rx.is_none() && now >= state.reconnect_at {
            state.reconnect_attempts += 1;
            state.reconnect_at = now + reconnect_wait(state.reconnect_attempts);
            let params = state.reconnect.clone();
            let (tx, rx) = mpsc::channel();
            state.reconnect_rx = Some(rx);
            let _ = thread::Builder::new()
                .name("reconnect".into())
                .spawn(move || {
                    let _ = tx.send(reconnect_dial(&params));
                });
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Failed
// ---------------------------------------------------------------------------

fn step_failed(msg: &mut str, rl: &mut RaylibHandle, thread: &RaylibThread) -> Option<App> {
    let back = rl.is_key_pressed(KeyboardKey::KEY_ENTER)
        || rl.is_key_pressed(KeyboardKey::KEY_KP_ENTER)
        || rl.is_key_pressed(KeyboardKey::KEY_ESCAPE);

    let mut d = rl.begin_drawing(thread);
    let w = d.get_screen_width();
    let h = d.get_screen_height();
    d.clear_background(BG);

    let title = "CONNECTION FAILED";
    let tw = d.measure_text(title, 32);
    d.draw_text(title, (w - tw) / 2, h / 2 - 120, 32, ERROR);

    // `draw_text` builds a CString — keep control bytes (incl. NUL) out.
    let clean: String = msg
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    for (i, line) in wrap_text(&mut d, &clean, 16, w - 160).iter().enumerate() {
        let lw = d.measure_text(line, 16);
        d.draw_text(line, (w - lw) / 2, h / 2 - 60 + i as i32 * 22, 16, TEXT);
    }

    let hint = "Enter / Esc — back to sign-in";
    let hw = d.measure_text(hint, 16);
    d.draw_text(hint, (w - hw) / 2, h - 64, 16, PLACEHOLDER);
    drop(d);

    back.then(|| App::SignIn(SignInState::default()))
}

/// Word-wrap `text` to `max_width` px at `font_size` (measured with the
/// default font; good enough for error pages).
fn wrap_text(d: &mut RaylibDrawHandle, text: &str, font_size: i32, max_width: i32) -> Vec<String> {
    let mut lines = Vec::new();
    for raw in text.lines() {
        let mut line = String::new();
        for word in raw.split_whitespace() {
            let candidate = if line.is_empty() {
                word.to_owned()
            } else {
                format!("{line} {word}")
            };
            if !line.is_empty() && d.measure_text(&candidate, font_size) > max_width {
                lines.push(std::mem::replace(&mut line, word.to_owned()));
            } else {
                line = candidate;
            }
        }
        lines.push(line);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

/// Poison-tolerant mutex lock (same convention as `net`).
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}
