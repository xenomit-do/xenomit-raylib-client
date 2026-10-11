//! Font loading + label drawing.
//!
//! Stock uses device `Tahoma` (`Styles.nickFmt` = Tahoma bold 13px for
//! ship name plates); `Fonts::nick` loads `tahomabd.ttf`, `Fonts::ui`
//! loads `tahoma.ttf` for HUD chrome. The font is loaded via
//! `LoadFontEx` at the design size × the HiDPI scale so the glyph atlas
//! stays crisp on scaled displays, with bilinear texture filtering.
//!
//! A missing TTF falls back to raylib's built-in default: `get_font_default`
//! hands out a borrowed `WeakFont`, but the public contract wants an owned
//! `Font`. We wrap the same raw handle: it is safe to drop because raylib's
//! own `UnloadFont` refuses to free the default font (`rtext.c`:
//! `font.texture.id != GetFontDefault().texture.id` guard), so `Font::drop`
//! on this alias is a no-op.

use raylib::ffi;
use raylib::prelude::*;
use std::ffi::CString;

/// Fonts the HUD/scene layer draws text with.
pub struct Fonts {
    /// General UI text font (`tahoma.ttf`).
    pub ui: Font,
    /// Ship name-plate font (`tahomabd.ttf`, stock `Styles.nickFmt`).
    pub nick: Font,
    /// Segoe UI Symbol fallback for mathematical alphabets, marks and
    /// decorative Unicode characters used in player names/chat.
    pub symbols: Option<Font>,
}

/// Design size of `Styles.nickFmt` — the fonts are authored for 13px.
const FONT_SIZE: i32 = 13;

const UI_FONT_PATH: &str = r"C:\Windows\Fonts\tahoma.ttf";
const NICK_FONT_PATH: &str = r"C:\Windows\Fonts\tahomabd.ttf";
const SYMBOL_FONT_PATH: &str = r"C:\Windows\Fonts\seguisym.ttf";

/// Load HUD fonts once after window creation.
pub fn load_fonts(rl: &mut RaylibHandle, _thread: &RaylibThread) -> Fonts {
    let dpi = rl.get_window_scale_dpi().x.round().max(1.0) as i32;
    let size = FONT_SIZE * dpi;
    let latin = latin_codepoints();
    Fonts {
        ui: load_ttf(UI_FONT_PATH, size, latin.clone()).unwrap_or_else(|| default_font(rl)),
        nick: load_ttf(NICK_FONT_PATH, size, latin).unwrap_or_else(|| default_font(rl)),
        symbols: load_ttf(SYMBOL_FONT_PATH, size, symbol_codepoints()),
    }
}

/// `LoadFontEx` with the fixed glyph set below; `None` when the file is
/// missing or the load produced no atlas.
fn load_ttf(path: &str, size_px: i32, mut cps: Vec<i32>) -> Option<Font> {
    if !std::path::Path::new(path).exists() {
        return None;
    }
    let c_path = CString::new(path).ok()?;
    cps.sort_unstable();
    cps.dedup();
    // SAFETY: `cps` outlives the call; raylib copies the codepoint list.
    let raw =
        unsafe { ffi::LoadFontEx(c_path.as_ptr(), size_px, cps.as_mut_ptr(), cps.len() as i32) };
    if raw.texture.id == 0 || raw.glyphs.is_null() {
        return None;
    }
    // SAFETY: `raw` is a valid freshly-loaded font.
    unsafe {
        ffi::SetTextureFilter(
            raw.texture,
            ffi::TextureFilter::TEXTURE_FILTER_BILINEAR as i32,
        );
        Some(Font::from_raw(raw))
    }
}

fn latin_codepoints() -> Vec<i32> {
    (32..=0x24F)
        .chain(0x370..=0x52F) // Greek and extended Cyrillic.
        .chain(0x1E00..=0x1EFF) // Latin Extended Additional.
        .collect()
}

fn symbol_codepoints() -> Vec<i32> {
    [
        (0x2000, 0x206F),   // punctuation and formatting marks
        (0x20A0, 0x20CF),   // currency symbols
        (0x2100, 0x218F),   // letterlike symbols and arrows
        (0x2190, 0x23FF),   // arrows, math and technical symbols
        (0x2460, 0x27BF),   // enclosed letters, shapes and dingbats
        (0x1D400, 0x1D7FF), // mathematical alphanumeric alphabets
        (0x1F300, 0x1F5FF), // common pictographs
        (0xFF00, 0xFFEF),   // full-width forms
    ]
    .into_iter()
    .flat_map(|(start, end)| start..=end)
    .collect()
}

fn has_glyph(font: &Font, ch: char) -> bool {
    font.get_glyph_index(ch) != 0
}

/// Measure mixed-font text without replacing unsupported player characters
/// with Tahoma's missing-glyph box.
pub fn measure_text_fallback(
    primary: &Font,
    fallback: Option<&Font>,
    text: &str,
    size: f32,
    spacing: f32,
) -> Vector2 {
    let mut width = 0.0;
    let mut run = String::new();
    let mut run_fallback = false;
    for ch in text.chars() {
        let use_fallback = fallback.is_some_and(|f| !has_glyph(primary, ch) && has_glyph(f, ch));
        if !run.is_empty() && use_fallback != run_fallback {
            let font = if run_fallback {
                fallback.unwrap()
            } else {
                primary
            };
            width += font.measure_text(&run, size, spacing).x;
            run.clear();
        }
        run_fallback = use_fallback;
        run.push(ch);
    }
    if !run.is_empty() {
        let font = if run_fallback {
            fallback.unwrap()
        } else {
            primary
        };
        width += font.measure_text(&run, size, spacing).x;
    }
    Vector2::new(width, primary.measure_text("Mg", size, spacing).y)
}

/// Draw mixed Latin and decorative Unicode runs using the best available
/// glyph atlas for each run.
pub fn draw_text_fallback(
    d: &mut RaylibDrawHandle,
    primary: &Font,
    fallback: Option<&Font>,
    text: &str,
    pos: Vector2,
    size: f32,
    spacing: f32,
    color: Color,
) {
    let mut x = pos.x;
    let mut run = String::new();
    let mut run_fallback = false;
    let mut flush = |run: &mut String, use_fallback: bool, x: &mut f32| {
        if run.is_empty() {
            return;
        }
        let font = if use_fallback {
            fallback.unwrap()
        } else {
            primary
        };
        d.draw_text_ex(font, run, Vector2::new(*x, pos.y), size, spacing, color);
        *x += font.measure_text(run, size, spacing).x;
        run.clear();
    };
    for ch in text.chars() {
        let use_fallback = fallback.is_some_and(|f| !has_glyph(primary, ch) && has_glyph(f, ch));
        if !run.is_empty() && use_fallback != run_fallback {
            flush(&mut run, run_fallback, &mut x);
        }
        run_fallback = use_fallback;
        run.push(ch);
    }
    flush(&mut run, run_fallback, &mut x);
}

/// Built-in pixel font — the `UnloadFont` default-font guard makes the
/// owned alias a no-op drop.
fn default_font(rl: &mut RaylibHandle) -> Font {
    // SAFETY: `GetFontDefault` borrows the internal default font —
    // `UnloadFont` refuses to unload it (rtext.c texture-id guard).
    unsafe { Font::from_raw(std::ptr::read(rl.get_font_default().as_ref())) }
}

/// Stock label glow (`GlowFilter(0x000000, alpha 0.7, blur 5, strength 6)`
/// approximated): black copies at the 8 one-pixel offsets plus the 4
/// two-pixel diagonals, then the text in `color`.
pub fn draw_text_glow(
    d: &mut RaylibDrawHandle,
    font: &Font,
    text: &str,
    pos: Vector2,
    size: f32,
    color: Color,
) {
    if text.is_empty() {
        return;
    }
    let glow = Color::new(0, 0, 0, 178);
    for (dx, dy) in [
        (-1.0, -1.0),
        (0.0, -1.0),
        (1.0, -1.0),
        (-1.0, 0.0),
        (1.0, 0.0),
        (-1.0, 1.0),
        (0.0, 1.0),
        (1.0, 1.0),
        (-2.0, -2.0),
        (2.0, -2.0),
        (-2.0, 2.0),
        (2.0, 2.0),
    ] {
        d.draw_text_ex(
            font,
            text,
            Vector2::new(pos.x + dx, pos.y + dy),
            size,
            1.0,
            glow,
        );
    }
    d.draw_text_ex(font, text, pos, size, 1.0, color);
}

/// Stock nickname glow with per-run Unicode fallback.
pub fn draw_text_glow_fallback(
    d: &mut RaylibDrawHandle,
    font: &Font,
    fallback: Option<&Font>,
    text: &str,
    pos: Vector2,
    size: f32,
    color: Color,
) {
    if text.is_empty() {
        return;
    }
    let glow = Color::new(0, 0, 0, 178);
    for (dx, dy) in [
        (-1.0, -1.0),
        (0.0, -1.0),
        (1.0, -1.0),
        (-1.0, 0.0),
        (1.0, 0.0),
        (-1.0, 1.0),
        (0.0, 1.0),
        (1.0, 1.0),
        (-2.0, -2.0),
        (2.0, -2.0),
        (-2.0, 2.0),
        (2.0, 2.0),
    ] {
        draw_text_fallback(
            d,
            font,
            fallback,
            text,
            Vector2::new(pos.x + dx, pos.y + dy),
            size,
            1.0,
            glow,
        );
    }
    draw_text_fallback(d, font, fallback, text, pos, size, 1.0, color);
}
