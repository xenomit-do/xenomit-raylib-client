//! `xenomit-client` — the playable binary.
//!
//! Boots a raylib window and drives `app::App`: the sign-in form (skipped
//! when `--server`+`--dosid` are both given) through the bootstrap pipeline
//! into the live scene.

use crate::app::{App, AppConfig};
use crate::render;
use anyhow::{Context, Result};
use clap::Parser;
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "xenomit-client",
    about = "Native DarkOrbit client built on raylib"
)]
struct Args {
    /// Game server — "tr3", "tr3.darkorbit.com" or a full URL. With
    /// `--dosid` set, sign-in is skipped entirely.
    #[arg(long)]
    server: Option<String>,
    /// `dosid` session cookie from a logged-in browser session.
    #[arg(long)]
    dosid: Option<String>,
    /// Path to the stock client's main.swf (embedded config + asset XMLs).
    /// Default: first existing of ../game_resources/main.swf,
    /// tests/fixtures/main.swf, main.swf.
    #[arg(long)]
    main_swf: Option<PathBuf>,
    /// CDN asset cache directory (created if missing).
    #[arg(long)]
    assets_dir: Option<PathBuf>,
    /// Window width in pixels.
    #[arg(long)]
    width: Option<i32>,
    /// Window height in pixels.
    #[arg(long)]
    height: Option<i32>,
    /// Render the touch-first HUD on a landscape desktop window.
    #[arg(long)]
    mobile_preview: bool,
    /// Exit cleanly after this many seconds (useful for repeatable benchmarks).
    #[arg(long)]
    exit_after: Option<u64>,
    /// Screenshot directory for F12 captures and `--autoshot`
    /// (`load_image_from_screen` — full framebuffer, DPI-correct).
    #[arg(long)]
    shot_dir: Option<PathBuf>,
    /// Dev flag: save one screenshot every N seconds.
    #[arg(long)]
    autoshot: Option<u64>,
    /// Dev flag: save a screenshot ~250 ms after each inbound combat
    /// event — catches laser bolts mid-salvo.
    #[arg(long)]
    shot_on_hit: bool,
    /// Debug-level logging (default: info; RUST_LOG overrides both).
    #[arg(short, long)]
    verbose: bool,
}

pub fn run_cli() -> Result<()> {
    run(Args::parse())
}

#[cfg(target_os = "android")]
pub fn run_android() -> i32 {
    let args = Args::parse_from(["xenomit-client"]);
    match run(args) {
        Ok(()) => 0,
        Err(error) => {
            log::error!("client failed: {error:#}");
            1
        }
    }
}

fn run(args: Args) -> Result<()> {
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or(if args.verbose { "debug" } else { "info" }),
    )
    .init();

    let assets_dir = args.assets_dir.unwrap_or_else(default_assets_dir);
    std::fs::create_dir_all(&assets_dir)
        .with_context(|| format!("could not create assets dir {}", assets_dir.display()))?;

    let shot_dir = args.shot_dir.unwrap_or_else(|| {
        if cfg!(target_os = "android") {
            assets_dir.join("shots")
        } else {
            PathBuf::from("target/shots")
        }
    });
    std::fs::create_dir_all(&shot_dir)
        .with_context(|| format!("could not create shot dir {}", shot_dir.display()))?;
    #[cfg(target_os = "android")]
    let main_swf = args
        .main_swf
        .or_else(|| bundled_android_main_swf(&assets_dir).ok());
    #[cfg(not(target_os = "android"))]
    let main_swf = args.main_swf.or_else(find_main_swf);

    let cfg = AppConfig {
        server: args.server,
        dosid: args.dosid,
        main_swf,
        assets_dir,
        shot_dir,
        autoshot_secs: args.autoshot,
        shot_on_hit: args.shot_on_hit,
        mobile_preview: args.mobile_preview,
    };
    if cfg.main_swf.is_none() {
        log::warn!("main.swf not found in the default locations — pass --main-swf <path>");
    }

    // FLAG_WINDOW_HIGHDPI: the builder can't express it, so set it via
    // FFI before `build()` (SetConfigFlags ORs bits in). Without it the
    // logical screen stays 1280x760 while the real framebuffer is
    // DPI-scaled (e.g. 1600x950 at 125%) — the scene renders squeezed
    // into the bottom-left and screenshots show black bands.
    unsafe {
        raylib::ffi::SetConfigFlags(raylib::ffi::ConfigFlags::FLAG_WINDOW_HIGHDPI as u32);
    }
    let (mut rl, thread) = raylib::init()
        .size(
            args.width.unwrap_or(1280),
            args.height
                .unwrap_or(if args.mobile_preview { 576 } else { 760 }),
        )
        .title("Xenomit")
        .build();
    // The 3D observer camera is 1,740 units from the map plane and the
    // independent overhead view uses an 80,000-unit camera. Raylib's default
    // 1,000-unit far plane clips authored ships and the whole overhead view.
    unsafe {
        raylib::ffi::rlSetClipPlanes(10.0, 80_000.0);
    }
    log::info!(
        "window: screen {}x{}, render {}x{}",
        rl.get_screen_width(),
        rl.get_screen_height(),
        rl.get_render_width(),
        rl.get_render_height()
    );
    // Desktop is intentionally uncapped for performance measurement
    // (raylib: SetTargetFPS(0) disables the frame limiter).
    #[cfg(not(target_os = "android"))]
    rl.set_target_fps(0);
    // Android keeps a 60 Hz ceiling to avoid burning battery.
    #[cfg(target_os = "android")]
    rl.set_target_fps(60);
    // Esc is a UI/game key (cancel, deselect, back) — not "quit".
    rl.set_exit_key(None);
    // Under FLAG_WINDOW_HIGHDPI the framebuffer is larger than the
    // logical screen and GetMousePosition reports framebuffer pixels —
    // rescale to logical coords so every hit-test (minimap, panels,
    // world clicks) matches what is drawn.
    let msx = rl.get_screen_width() as f32 / rl.get_render_width().max(1) as f32;
    let msy = rl.get_screen_height() as f32 / rl.get_render_height().max(1) as f32;
    if (msx - 1.0).abs() > f32::EPSILON || (msy - 1.0).abs() > f32::EPSILON {
        rl.set_mouse_scale(msx, msy);
        log::info!("mouse scale: {msx:.3}x{msy:.3} (high-DPI)");
    }

    let fonts = render::load_fonts(&mut rl, &thread);
    let mut app = App::initial(&cfg);
    let exit_at = args
        .exit_after
        .map(|seconds| std::time::Instant::now() + std::time::Duration::from_secs(seconds));
    let mut perf_started = std::time::Instant::now();
    let mut perf_frames = 0_u64;
    while !rl.window_should_close() && !exit_at.is_some_and(|at| std::time::Instant::now() >= at) {
        app.step(&mut rl, &thread, &fonts, &cfg);
        perf_frames += 1;
        let elapsed = perf_started.elapsed();
        if elapsed >= std::time::Duration::from_secs(5) {
            let fps = perf_frames as f64 / elapsed.as_secs_f64();
            log::info!(
                "performance: {:.1} average FPS, {:.2} ms last frame",
                fps,
                rl.get_frame_time() * 1000.0
            );
            perf_started = std::time::Instant::now();
            perf_frames = 0;
        }
    }
    // Graceful logout when a session is live; `PlayState`'s Drop also
    // covers every other exit path.
    app.shutdown();
    Ok(())
}

/// Default `--main-swf` search order (repo checkout, then test fixtures).
#[cfg(not(target_os = "android"))]
fn find_main_swf() -> Option<PathBuf> {
    [
        "../game_resources/main.swf",
        "tests/fixtures/main.swf",
        "main.swf",
    ]
    .iter()
    .map(PathBuf::from)
    .find(|p| p.is_file())
}

fn default_assets_dir() -> PathBuf {
    #[cfg(target_os = "android")]
    {
        // Raylib owns NativeActivity and exposes its private, writable
        // internalDataPath through the small local raylib-sys patch.
        let ptr = unsafe { raylib::ffi::XenomitGetInternalDataPath() };
        if ptr.is_null() {
            panic!("Android app-private storage path is unavailable");
        }
        let base = unsafe { std::ffi::CStr::from_ptr(ptr) }
            .to_str()
            .expect("Android app-private storage path is not UTF-8");
        PathBuf::from(base).join(".xenomit-assets")
    }
    #[cfg(not(target_os = "android"))]
    {
        PathBuf::from(".xenomit-assets")
    }
}

#[cfg(target_os = "android")]
fn bundled_android_main_swf(assets_dir: &std::path::Path) -> anyhow::Result<PathBuf> {
    // The binary bundle includes the stock SWF as the canonical source for
    // the client XML and patterns. It is unpacked into private storage once
    // because the existing asset loader reads the SWF from a filesystem path.
    const MAIN_SWF: &[u8] = include_bytes!("../../game_resources/main.swf");
    let path = assets_dir.join("main.swf");
    if !path.is_file() {
        std::fs::write(&path, MAIN_SWF)?;
    }
    Ok(path)
}
