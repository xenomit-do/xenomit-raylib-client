//! Window chrome + small widgets in the DarkOrbit dark-blue/bevel look:
//! gradient fills, 1px dark outline, light inner highlight on the
//! top/left edges and a shadow edge bottom/right. Drawn procedurally —
//! or with the real `window` SWF composite (`SimpleWindow.as`: masked
//! `pattern` body texture + scale9 `window` chrome + `closeBtn`/
//! `minimizeBtn` state sprites), or the flat `questWin` scale9 export /
//! `main.swf` skin selected in settings.

use super::window::{Rect, Window, TITLE_H};
use crate::assets::SpriteId;
use crate::render::fonts::{draw_text_fallback, measure_text_fallback, Fonts};
use crate::render::sprites::GpuClip;
use crate::ui::preferences::WindowSkinChoice;
use raylib::prelude::*;

pub const WIN_TOP: Color = Color::new(16, 28, 49, 238);
pub const WIN_BOT: Color = Color::new(7, 13, 26, 238);
pub const EDGE_DARK: Color = Color::new(3, 7, 14, 255);
pub const EDGE_LIGHT: Color = Color::new(78, 122, 176, 200);
pub const EDGE_SHADOW: Color = Color::new(2, 5, 10, 210);
pub const TITLE_ACTIVE_TOP: Color = Color::new(50, 88, 136, 255);
pub const TITLE_ACTIVE_BOT: Color = Color::new(26, 50, 86, 255);
pub const TITLE_IDLE_TOP: Color = Color::new(34, 54, 84, 255);
pub const TITLE_IDLE_BOT: Color = Color::new(19, 32, 54, 255);
pub const TITLE_TXT: Color = Color::new(216, 228, 242, 255);
pub const ACCENT: Color = Color::new(0, 174, 239, 255);
pub const TXT: Color = Color::new(205, 215, 228, 255);
pub const TXT_DIM: Color = Color::new(140, 152, 168, 255);
pub const TXT_GOOD: Color = Color::new(110, 220, 130, 255);
pub const TXT_WARN: Color = Color::new(240, 200, 70, 255);
pub const TXT_BAD: Color = Color::new(235, 90, 70, 255);
pub const INSET: Color = Color::new(4, 8, 14, 225);
pub const INSET_EDGE: Color = Color::new(34, 50, 74, 255);

pub fn rl_rect(r: Rect) -> Rectangle {
    Rectangle::new(r.0, r.1, r.2, r.3)
}

fn lighten(c: Color, by: u8) -> Color {
    Color::new(
        c.r.saturating_add(by),
        c.g.saturating_add(by),
        c.b.saturating_add(by),
        c.a,
    )
}

fn darken(c: Color, by: u8) -> Color {
    Color::new(
        c.r.saturating_sub(by),
        c.g.saturating_sub(by),
        c.b.saturating_sub(by),
        c.a,
    )
}

/// Beveled gradient panel — the HUD panel/window body look.
pub fn draw_bevel_panel(d: &mut RaylibDrawHandle, r: Rect) {
    let (x, y, w, h) = (r.0 as i32, r.1 as i32, r.2 as i32, r.3 as i32);
    if w < 4 || h < 4 {
        return;
    }
    d.draw_rectangle_gradient_v(x, y, w, h, WIN_TOP, WIN_BOT);
    d.draw_rectangle_lines_ex(rl_rect(r), 1.0, EDGE_DARK);
    // Inner highlight (top/left) and shadow (bottom/right).
    d.draw_line(x + 1, y + 1, x + w - 2, y + 1, EDGE_LIGHT);
    d.draw_line(x + 1, y + 1, x + 1, y + h - 2, EDGE_LIGHT);
    d.draw_line(x + 1, y + h - 2, x + w - 2, y + h - 2, EDGE_SHADOW);
    d.draw_line(x + w - 2, y + 1, x + w - 2, y + h - 2, EDGE_SHADOW);
}

/// Recessed dark well (bars, lists, slots).
pub fn draw_inset(d: &mut RaylibDrawHandle, r: Rect) {
    d.draw_rectangle(r.0 as i32, r.1 as i32, r.2 as i32, r.3 as i32, INSET);
    d.draw_rectangle_lines_ex(rl_rect(r), 1.0, INSET_EDGE);
}

/// Window skin sprites resolved for one frame. `stock` holds the real
/// `window`-finisher composite parts (`backgroundWithTopControls` /
/// `backgroundNormal` children); `flat` is the single nine-slice export
/// used by `ModernBlue` (`questWin`) and `Legacy` (`main.swf`).
#[derive(Clone, Copy, Default)]
pub struct Skin<'a> {
    /// `backgroundWithTopControls>window` — scale9 chrome w/ title bar.
    pub chrome: Option<&'a GpuClip>,
    /// `backgroundNormal>window` — chrome without window controls,
    /// used by `draw_panel` (dialogs, flyouts).
    pub chrome_plain: Option<&'a GpuClip>,
    /// `backgroundWithTopControls>pattern` — masked body texture.
    pub pattern: Option<&'a GpuClip>,
    /// `closeBtn`/`minimizeBtn` `background` + `rollOverState` sprites.
    pub close: Option<&'a GpuClip>,
    pub close_hover: Option<&'a GpuClip>,
    pub minimize: Option<&'a GpuClip>,
    pub minimize_hover: Option<&'a GpuClip>,
    /// Flat nine-slice export (`questWin`/`main.swf`).
    pub flat: Option<&'a GpuClip>,
}

impl<'a> Skin<'a> {
    /// True when the stock `window` composite is available.
    pub fn is_stock(&self) -> bool {
        self.chrome.is_some()
    }
}

/// Resolve the configured window skin into drawable GPU clips — the
/// stock `window`-finisher composite for `Stock`, the flat export for
/// `ModernBlue`/`Legacy`. Fields stay `None` until the worker uploads
/// them; draw paths fall back to the procedural bevel meanwhile.
pub fn resolve_skin<'a>(
    assets: &'a crate::assets::AssetManager,
    tex: &'a crate::render::sprites::TextureCache,
    choice: WindowSkinChoice,
) -> Skin<'a> {
    let get = |id: Option<SpriteId>| id.and_then(|i| tex.get(i)).filter(|c| !c.frames.is_empty());
    match choice {
        WindowSkinChoice::Stock => Skin {
            chrome: get(assets.stock_window.chrome),
            chrome_plain: get(assets.stock_window.chrome_plain)
                .or_else(|| get(assets.stock_window.chrome)),
            pattern: get(assets.stock_window.pattern),
            close: get(assets.stock_window.close),
            close_hover: get(assets.stock_window.close_hover),
            minimize: get(assets.stock_window.minimize),
            minimize_hover: get(assets.stock_window.minimize_hover),
            flat: None,
        },
        _ => Skin {
            flat: match choice {
                WindowSkinChoice::ModernBlue => get(assets.modern_window_skin),
                _ => get(assets.legacy_window_skin),
            },
            ..Default::default()
        },
    }
}

/// Full window: title bar (active window brighter), `_` / `×` buttons,
/// and the body panel unless collapsed.
pub fn draw_window(
    d: &mut RaylibDrawHandle,
    fonts: &Fonts,
    w: &Window,
    active: bool,
    mouse: (f32, f32),
    skin: Skin,
    skin_choice: WindowSkinChoice,
) {
    if !w.visible() {
        return;
    }
    let stock = skin_choice == WindowSkinChoice::Stock && skin.chrome.is_some();
    match (w.collapsed, stock) {
        // `SimpleWindow.minimize` tweens `_rootContainerMask.height` to
        // WINDOW_TITLEBAR_HEIGHT — the full window clipped to the bar.
        (true, true) => {
            let t = w.title_bar();
            d.draw_scissor_mode(t.0 as i32, t.1 as i32, t.2 as i32, t.3 as i32, |mut s| {
                draw_stock_window(&mut s, &skin, w.rect)
            });
        }
        (false, true) => draw_stock_window(d, &skin, w.rect),
        (false, false) => match skin.flat {
            Some(clip) => draw_window_skin(d, clip, w.rect, skin_choice),
            None => draw_bevel_panel(d, w.rect),
        },
        (true, false) => {}
    }
    let t = w.title_bar();
    if !stock {
        let (top, bot) = if active {
            (TITLE_ACTIVE_TOP, TITLE_ACTIVE_BOT)
        } else {
            (TITLE_IDLE_TOP, TITLE_IDLE_BOT)
        };
        if skin.flat.is_some() && !w.collapsed {
            if active {
                d.draw_rectangle(
                    t.0 as i32 + 2,
                    t.1 as i32 + 2,
                    (t.2 - 4.0) as i32,
                    (t.3 - 4.0) as i32,
                    Color::new(50, 130, 190, 38),
                );
            }
        } else {
            d.draw_rectangle_gradient_v(t.0 as i32, t.1 as i32, t.2 as i32, t.3 as i32, top, bot);
            d.draw_rectangle_lines_ex(rl_rect(t), 1.0, EDGE_DARK);
            d.draw_line(
                t.0 as i32 + 1,
                t.1 as i32 + 1,
                (t.0 + t.2) as i32 - 2,
                t.1 as i32 + 1,
                lighten(top, 40),
            );
        }
        if active {
            // Accent underline on the focused window.
            d.draw_line(
                t.0 as i32 + 1,
                (t.1 + t.3) as i32 - 2,
                (t.0 + t.2) as i32 - 2,
                (t.1 + t.3) as i32 - 2,
                Color::new(ACCENT.r, ACCENT.g, ACCENT.b, 140),
            );
        }
    }
    // The stock container's `label` TextField sits at (37.3, 5) —
    // `windowTitleFmt` renders ~11px bright text on the teal bar.
    // (Procedural skins keep the tighter 7px indent.)
    let title_x = if stock { t.0 + 37.0 } else { t.0 + 7.0 };
    d.draw_text_ex(
        &fonts.ui,
        &w.title,
        Vector2::new(title_x, t.1 + (TITLE_H - 11.0) * 0.5),
        11.0,
        1.0,
        TITLE_TXT,
    );

    // Collapse (`minimizeBtn`) + close (`closeBtn`) — real state sprites
    // when the stock skin is resolved, procedural glyphs otherwise.
    let k = w.collapse_button();
    if !(stock
        && draw_stock_button(
            d,
            skin.minimize,
            skin.minimize_hover,
            w.hit_collapse(mouse),
            k,
        ))
    {
        title_button(d, k, w.hit_collapse(mouse), false);
        let (kx, ky) = (k.0, k.1);
        if w.collapsed {
            d.draw_rectangle_lines(kx as i32 + 7, ky as i32 + 5, 7, 6, TITLE_TXT);
            d.draw_line(
                kx as i32 + 7,
                ky as i32 + 5,
                kx as i32 + 13,
                ky as i32 + 5,
                TITLE_TXT,
            );
        } else {
            d.draw_rectangle(kx as i32 + 7, ky as i32 + 10, 7, 2, TITLE_TXT);
        }
    }
    if w.closable {
        let c = w.close_button();
        if !(stock && draw_stock_button(d, skin.close, skin.close_hover, w.hit_close(mouse), c)) {
            title_button(d, c, w.hit_close(mouse), true);
            let (cx, cy) = (c.0, c.1);
            let m = 6.0;
            d.draw_line_ex(
                Vector2::new(cx + m, cy + m),
                Vector2::new(cx + c.2 - m, cy + c.3 - m),
                1.5,
                TITLE_TXT,
            );
            d.draw_line_ex(
                Vector2::new(cx + c.2 - m, cy + m),
                Vector2::new(cx + m, cy + c.3 - m),
                1.5,
                TITLE_TXT,
            );
        }
    }
}

/// `backgroundWithTopControls` composite: the `pattern` body texture
/// cropped by `windowMask` (inset 4px sides, 4px bottom), then the
/// scale9 `window` chrome stretched over it (`_window.width = w`).
fn draw_stock_window<D: RaylibDraw>(d: &mut D, skin: &Skin, r: Rect) {
    if let Some(frame) = skin.pattern.and_then(|c| c.frames.first()) {
        // Pattern is placed unscaled at (0,0); the mask crops it to the
        // interior, so draw its `windowMask` rect 1:1.
        let pw = (r.2 - 8.0).clamp(0.0, frame.tex.width as f32 - 4.0);
        let ph = (r.3 - 4.0).clamp(0.0, frame.tex.height as f32);
        if pw > 0.0 && ph > 0.0 {
            d.draw_texture_pro(
                &frame.tex,
                Rectangle::new(4.0, 0.0, pw, ph),
                Rectangle::new(r.0 + 4.0, r.1, pw, ph),
                Vector2::zero(),
                0.0,
                Color::WHITE,
            );
        }
    }
    if let Some(chrome) = skin.chrome {
        draw_nine_patch(d, chrome, r);
    }
}

/// One `background`/`rollOverState` state sprite drawn at authored size
/// in `r` (21x13 `closeBtn`, 21x12 `minimizeBtn`). `false` when no
/// button art is available so the caller draws the procedural glyph.
fn draw_stock_button<D: RaylibDraw>(
    d: &mut D,
    up: Option<&GpuClip>,
    hover: Option<&GpuClip>,
    hovered: bool,
    r: Rect,
) -> bool {
    let clip = if hovered { hover.or(up) } else { up };
    let Some(frame) = clip.and_then(|c| c.frames.first()) else {
        return false;
    };
    d.draw_texture_pro(
        &frame.tex,
        Rectangle::new(0.0, 0.0, frame.tex.width as f32, frame.tex.height as f32),
        Rectangle::new(r.0 + frame.offset.0, r.1 + frame.offset.1, r.2, r.3),
        Vector2::zero(),
        0.0,
        Color::WHITE,
    );
    true
}

/// Nine-slice `clip` into `r`, honoring the authored `DefineScalingGrid`
/// (`GpuClip::scale9`) — scale9 sprites like the `window` chrome keep
/// their edges and corners fixed while the middle stretches.
fn draw_nine_patch<D: RaylibDraw>(d: &mut D, clip: &GpuClip, r: Rect) {
    let Some(frame) = clip.frames.first() else {
        return;
    };
    let (sw, sh) = (frame.tex.width as f32, frame.tex.height as f32);
    let caps = clip
        .scale9
        .map(|(gx, gy, gw, gh)| (gx, (sw - gx - gw).max(1.0), gy, (sh - gy - gh).max(1.0)))
        .unwrap_or((15.0, 42.0, 25.0, 15.0));
    nine_slice(d, &frame.tex, r, caps);
}

/// Stretch `tex` into `r` nine-slice style: `(left, right, top, bottom)`
/// fixed caps, center + edges stretched.
fn nine_slice<D: RaylibDraw>(d: &mut D, tex: &Texture2D, r: Rect, caps: (f32, f32, f32, f32)) {
    let sw = tex.width as f32;
    let sh = tex.height as f32;
    let (left, right, top, bottom) = caps;
    if sw < 4.0 || sh < 4.0 || r.2 <= 0.0 || r.3 <= 0.0 || left >= sw || top >= sh {
        return;
    }
    let sx = [0.0, left, sw - right, sw];
    let sy = [0.0, top, sh - bottom, sh];
    let dx = [
        r.0,
        r.0 + left.min(r.2 * 0.5),
        r.0 + r.2 - right.min(r.2 * 0.5),
        r.0 + r.2,
    ];
    let dy = [
        r.1,
        r.1 + top.min(r.3 * 0.5),
        r.1 + r.3 - bottom.min(r.3 * 0.5),
        r.1 + r.3,
    ];
    for row in 0..3 {
        for col in 0..3 {
            let (sw, sh) = (sx[col + 1] - sx[col], sy[row + 1] - sy[row]);
            let (dw, dh) = (dx[col + 1] - dx[col], dy[row + 1] - dy[row]);
            if sw <= 0.0 || sh <= 0.0 || dw <= 0.0 || dh <= 0.0 {
                continue;
            }
            d.draw_texture_pro(
                tex,
                Rectangle::new(sx[col], sy[row], sw, sh),
                Rectangle::new(dx[col], dy[row], dw, dh),
                Vector2::zero(),
                0.0,
                Color::WHITE,
            );
        }
    }
}

/// Borderless skinned panel — modal popups (refinery dialogs, the ammo
/// flyout) reuse the same window background as every other window.
pub fn draw_panel(d: &mut RaylibDrawHandle, r: Rect, skin: Skin, skin_choice: WindowSkinChoice) {
    match skin_choice {
        WindowSkinChoice::Stock if skin.chrome_plain.is_some() => {
            draw_stock_window(d, &skin.with_plain_chrome(), r)
        }
        _ => match skin.flat {
            Some(clip) => draw_window_skin(d, clip, r, skin_choice),
            None => draw_bevel_panel(d, r),
        },
    }
}

impl<'a> Skin<'a> {
    /// The same skin viewed through the control-less `backgroundNormal`
    /// chrome — what `draw_panel` uses for borderless dialogs.
    fn with_plain_chrome(&self) -> Skin<'a> {
        Skin {
            chrome: self.chrome_plain,
            ..*self
        }
    }
}

/// Nine-slice the selected flat export (`ModernBlue`/`Legacy`). The
/// modern Spark background uses the grid from `DOWindowSkin.as`
/// (10,37,1,1); the legacy export retains its older control-cap
/// geometry. Clips carrying an authored `DefineScalingGrid` use it
/// instead.
fn draw_window_skin(d: &mut RaylibDrawHandle, clip: &GpuClip, r: Rect, choice: WindowSkinChoice) {
    let Some(frame) = clip.frames.first() else {
        return;
    };
    let sw = frame.tex.width as f32;
    let sh = frame.tex.height as f32;
    let caps = if let Some((gx, gy, gw, gh)) = clip.scale9 {
        // Authored grid: left/top pinned, middle stretch.
        (gx, (sw - gx - gw).max(1.0), gy, (sh - gy - gh).max(1.0))
    } else if choice == WindowSkinChoice::ModernBlue {
        // Scale9Grid(10,37,1,1): preserve both edges and stretch the
        // declared 1x1 center region.
        (10.0f32, (sw - 11.0).max(1.0), 37.0f32, (sh - 38.0).max(1.0))
    } else {
        (15.0f32, 42.0f32, 25.0f32, 15.0f32)
    };
    nine_slice(d, &frame.tex, r, caps);
}

fn title_button(d: &mut RaylibDrawHandle, r: Rect, hover: bool, danger: bool) {
    let (top, bot) = match (hover, danger) {
        (true, true) => (Color::new(190, 60, 50, 255), Color::new(120, 30, 26, 255)),
        (true, false) => (Color::new(70, 118, 176, 255), Color::new(40, 74, 122, 255)),
        _ => (Color::new(30, 48, 76, 255), Color::new(16, 28, 48, 255)),
    };
    d.draw_rectangle_gradient_v(r.0 as i32, r.1 as i32, r.2 as i32, r.3 as i32, top, bot);
    d.draw_rectangle_lines_ex(rl_rect(r), 1.0, EDGE_DARK);
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ButtonState {
    pub hover: bool,
    /// Toggled-on look (active config, open window).
    pub active: bool,
    pub disabled: bool,
}

/// Bevel push button with a centred label.
pub fn draw_button(d: &mut RaylibDrawHandle, fonts: &Fonts, r: Rect, label: &str, st: ButtonState) {
    let (top, bot) = if st.disabled {
        (Color::new(22, 28, 38, 235), Color::new(14, 18, 26, 235))
    } else if st.active {
        (Color::new(24, 120, 170, 245), Color::new(12, 74, 116, 245))
    } else if st.hover {
        (Color::new(58, 96, 144, 245), Color::new(34, 60, 98, 245))
    } else {
        (Color::new(36, 58, 92, 240), Color::new(22, 38, 64, 240))
    };
    d.draw_rectangle_gradient_v(r.0 as i32, r.1 as i32, r.2 as i32, r.3 as i32, top, bot);
    d.draw_rectangle_lines_ex(
        rl_rect(r),
        1.0,
        if st.active {
            ACCENT
        } else if st.disabled {
            Color::new(30, 38, 52, 255)
        } else {
            EDGE_DARK
        },
    );
    d.draw_line(
        r.0 as i32 + 1,
        r.1 as i32 + 1,
        (r.0 + r.2) as i32 - 2,
        r.1 as i32 + 1,
        lighten(top, 36),
    );
    let col = if st.disabled { TXT_DIM } else { Color::WHITE };
    let m = fonts.ui.measure_text(label, 10.0, 1.0);
    d.draw_text_ex(
        &fonts.ui,
        label,
        Vector2::new(r.0 + (r.2 - m.x) * 0.5, r.1 + (r.3 - m.y) * 0.5 - 0.5),
        10.0,
        1.0,
        col,
    );
}

/// Inset progress bar with a lit top edge.
pub fn draw_bar(d: &mut RaylibDrawHandle, r: Rect, frac: f32, col: Color) {
    d.draw_rectangle(r.0 as i32, r.1 as i32, r.2 as i32, r.3 as i32, INSET);
    let fw = ((r.2 - 2.0) * frac.clamp(0.0, 1.0)).round() as i32;
    if fw > 0 {
        d.draw_rectangle_gradient_v(
            r.0 as i32 + 1,
            r.1 as i32 + 1,
            fw,
            (r.3 - 2.0).max(1.0) as i32,
            lighten(col, 38),
            darken(col, 38),
        );
    }
    d.draw_rectangle_lines_ex(rl_rect(r), 1.0, INSET_EDGE);
}

/// Plain label at `(x, y)`.
pub fn label(
    d: &mut RaylibDrawHandle,
    fonts: &Fonts,
    s: &str,
    x: f32,
    y: f32,
    size: f32,
    col: Color,
) {
    draw_text_fallback(
        d,
        &fonts.ui,
        fonts.symbols.as_ref(),
        s,
        Vector2::new(x, y),
        size,
        1.0,
        col,
    );
}

/// Label right-aligned to `right`.
pub fn label_right(
    d: &mut RaylibDrawHandle,
    fonts: &Fonts,
    s: &str,
    right: f32,
    y: f32,
    size: f32,
    col: Color,
) {
    let m = measure_text_fallback(&fonts.ui, fonts.symbols.as_ref(), s, size, 1.0);
    draw_text_fallback(
        d,
        &fonts.ui,
        fonts.symbols.as_ref(),
        s,
        Vector2::new(right - m.x, y),
        size,
        1.0,
        col,
    );
}

/// `s` cut with an ellipsis so it fits `max_w` px.
pub fn fit_text(fonts: &Fonts, s: &str, size: f32, max_w: f32) -> String {
    if max_w <= 0.0 {
        return String::new();
    }
    if measure_text_fallback(&fonts.ui, fonts.symbols.as_ref(), s, size, 1.0).x <= max_w {
        return s.to_owned();
    }
    let mut out = String::new();
    for ch in s.chars() {
        out.push(ch);
        if measure_text_fallback(
            &fonts.ui,
            fonts.symbols.as_ref(),
            &format!("{out}..."),
            size,
            1.0,
        )
        .x > max_w
        {
            out.pop();
            out.push_str("...");
            return out;
        }
    }
    out
}

/// Tooltip box near the cursor, kept on screen.
pub fn draw_tooltip(
    d: &mut RaylibDrawHandle,
    fonts: &Fonts,
    lines: &[String],
    mouse: (f32, f32),
    vw: f32,
    vh: f32,
) {
    if lines.is_empty() {
        return;
    }
    let size = 10.0;
    let w = lines
        .iter()
        .map(|l| measure_text_fallback(&fonts.ui, fonts.symbols.as_ref(), l, size, 1.0).x)
        .fold(0.0f32, f32::max)
        + 12.0;
    let h = lines.len() as f32 * 13.0 + 8.0;
    let x = (mouse.0 + 14.0).min(vw - w - 4.0).max(4.0);
    let y = if mouse.1 + 18.0 + h > vh {
        (mouse.1 - h - 6.0).max(4.0)
    } else {
        mouse.1 + 18.0
    };
    draw_bevel_panel(d, (x, y, w, h));
    for (i, l) in lines.iter().enumerate() {
        let col = if i == 0 { Color::WHITE } else { TXT_DIM };
        draw_text_fallback(
            d,
            &fonts.ui,
            fonts.symbols.as_ref(),
            l,
            Vector2::new(x + 6.0, y + 4.0 + i as f32 * 13.0),
            size,
            1.0,
            col,
        );
    }
}

/// Keeps `TITLE_H` in the public surface for callers laying out bodies.
pub const WINDOW_TITLE_H: f32 = TITLE_H;
