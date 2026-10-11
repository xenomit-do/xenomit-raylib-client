//! Shared draw helpers used by scene submodules.

use crate::assets::AssetManager;
use crate::game::input;
use crate::game::Visual;
use crate::render::fonts::{draw_text_glow, Fonts};
use crate::render::sprites::{GpuClip, TextureCache};
use raylib::prelude::*;

/// Off-screen culling margin in screen px — overscan so objects entering
/// the screen are already drawn.
pub(crate) const CULL_MARGIN: f32 = 256.0;

/// Once-through clip frame: honors per-frame `durations`, then holds the
/// last frame once the clip's total runtime has passed (pyro explosions,
/// rocket gfx — stock plays these once, no wrap).
pub(crate) fn clip_frame_once(clip: &GpuClip, elapsed_ms: u64) -> usize {
    let n = clip.frames.len();
    if n == 0 {
        return 0;
    }
    let total: u64 = clip.durations.iter().map(|&d| d.max(1) as u64).sum();
    if total == 0 {
        return ((elapsed_ms / 33) as usize).min(n - 1);
    }
    if elapsed_ms >= total {
        return n - 1;
    }
    input::anim_frame(&clip.durations, elapsed_ms).min(n - 1)
}

/// Frame index + native rotation for a visual, following `ShipClip`:
/// multi-frame `useBitmapClip`/rotation strips index by heading;
/// single-frame `rotatable && !loop` clips rotate natively;
/// `autoPlay="false"` freezes on `frame`; otherwise loop by durations.
/// `None` when the clip has no frames.
pub(crate) fn pick_frame(
    v: &Visual,
    clip: &GpuClip,
    heading: f32,
    time_ms: u64,
    rotatable: bool,
    loop_play: bool,
    ship: bool,
) -> Option<(usize, f32)> {
    let n = clip.frames.len();
    if n == 0 {
        return None;
    }
    if ship && (clip.rotation_strip || v.bitmap_clip || (rotatable && n > 1)) {
        return Some((input::rotation_frame(heading, n), 0.0));
    }
    if ship && rotatable && !loop_play && n == 1 {
        return Some((0, heading));
    }
    if !v.auto_play {
        return Some(((v.still_frame - 1).clamp(0, n as i32 - 1) as usize, 0.0));
    }
    Some((input::anim_frame(&clip.durations, time_ms).min(n - 1), 0.0))
}

/// The one-time `center="true"` content shift: `int(w)>>1, int(h)>>1`
/// of the frame shown at set time — frame 0 — not the current frame
/// (strip frames differ in size, so per-frame centering jitters).
pub(crate) fn center_shift(clip: &GpuClip) -> (f32, f32) {
    clip.frames
        .first()
        .map(|f| {
            (
                (f.tex.width as f32 * 0.5).floor(),
                (f.tex.height as f32 * 0.5).floor(),
            )
        })
        .unwrap_or((0.0, 0.0))
}

/// `(lo, hi)` scaled by `s`, sign-aware (negative scale swaps the sides).
pub(crate) fn scaled_pair(lo: f32, hi: f32, s: f32) -> (f32, f32) {
    if s >= 0.0 {
        (lo * s, hi * s)
    } else {
        (hi * s, lo * s)
    }
}

/// World-space AABB of the visual drawn at world pos `pos`:
/// `pos + v.offset + (clip.bounds + center shift) * v.scale`. When the
/// visual rotates natively (`rotated`), the rect becomes its
/// circumscribed square — max corner distance — so every heading is
/// covered by the cull test and the label anchors.
pub(crate) fn visual_world_rect(
    v: &Visual,
    clip: &GpuClip,
    pos: (f32, f32),
    rotated: bool,
) -> (f32, f32, f32, f32) {
    let (bx0, by0, bx1, by1) = clip.bounds;
    let (cx, cy) = if v.center {
        center_shift(clip)
    } else {
        (0.0, 0.0)
    };
    let (x0, x1) = scaled_pair(bx0 - cx, bx1 - cx, v.scale.0);
    let (y0, y1) = scaled_pair(by0 - cy, by1 - cy, v.scale.1);
    let (x0, y0, x1, y1) = if rotated {
        let r = x0.abs().max(x1.abs()).hypot(y0.abs().max(y1.abs()));
        (-r, -r, r, r)
    } else {
        (x0, y0, x1, y1)
    };
    (
        pos.0 + v.offset.0 + x0,
        pos.1 + v.offset.1 + y0,
        pos.0 + v.offset.0 + x1,
        pos.1 + v.offset.1 + y1,
    )
}

/// Draw a recipe+clip at `pivot` (the entity's screen position).
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_visual(
    d: &mut RaylibDrawHandle,
    clip: &GpuClip,
    v: &Visual,
    frame: usize,
    rotation: f32,
    pivot: (f32, f32),
    scale: f32,
    tint: Color,
) {
    if clip.frames.get(frame).is_none() {
        return;
    }
    let pos = (pivot.0 + v.offset.0 * scale, pivot.1 + v.offset.1 * scale);
    let extra = if v.center {
        let (cx, cy) = center_shift(clip);
        (-cx, -cy)
    } else {
        (0.0, 0.0)
    };
    TextureCache::draw_frame_offset(
        d,
        clip,
        frame,
        pos,
        rotation,
        (scale * v.scale.0, scale * v.scale.1),
        tint,
        extra,
    );
}

/// Draw the extra `<displayObject>` layers of a multi-object recipe over
/// the main sprite, in document order. Each layer has its own offset /
/// scale and picks its frame the same way the main clip does.
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_visual_layers(
    d: &mut RaylibDrawHandle,
    tex: &TextureCache,
    v: &Visual,
    heading: f32,
    time_ms: u64,
    rotatable: bool,
    loop_play: bool,
    ship: bool,
    pivot: (f32, f32),
    scale: f32,
    tint: Color,
) {
    for layer in &v.layers {
        let Some(clip) = layer
            .sprite
            .and_then(|id| tex.get(id))
            .filter(|c| !c.frames.is_empty())
        else {
            continue;
        };
        if let Some((fi, rot)) =
            pick_frame(layer, clip, heading, time_ms, rotatable, loop_play, ship)
        {
            draw_visual(d, clip, layer, fi, rot, pivot, scale, tint);
        }
    }
}

/// Ship pattern for an entity `type_id` — primary lookup by `type`
/// (`ship_goliath`, `alien_streuner`), then by numeric npc design id.
pub(crate) fn ship_pattern<'a>(
    assets: &'a AssetManager,
    type_id: &str,
) -> Option<&'a crate::assets::gamexml::ShipPattern> {
    assets.game_xml.ship(type_id).or_else(|| {
        type_id
            .parse::<i32>()
            .ok()
            .and_then(|id| assets.game_xml.ship_by_npc(id))
    })
}

/// `"[TAG] name"` plate text; NPCs get the stock `-=[ Name ]=-` frame.
pub(crate) fn entity_label(name: &str, clan_tag: &str, npc: bool) -> String {
    let plain = if clan_tag.is_empty() {
        name.to_owned()
    } else {
        format!("[{clan_tag}] {name}")
    };
    if npc && !plain.is_empty() && !plain.contains("]=-") {
        format!("-=[ {plain} ]=-")
    } else {
        plain
    }
}

/// Plate color for a [`input::PlateKind`].
pub(crate) fn plate_color(kind: input::PlateKind) -> Color {
    match kind {
        input::PlateKind::Npc | input::PlateKind::Enemy => Color::new(255, 80, 70, 255),
        input::PlateKind::Ally => Color::new(110, 170, 255, 255),
        input::PlateKind::Neutral => Color::new(170, 180, 195, 255),
    }
}

/// Centered glowing map text at `(cx, y)` — Tahoma bold + black outer
/// glow (stock `Styles.nickFmt` + `GlowFilter`).
pub(crate) fn draw_centered_text(
    d: &mut RaylibDrawHandle,
    fonts: &Fonts,
    text: &str,
    cx: f32,
    y: f32,
    size: f32,
    color: Color,
) {
    let m = fonts.nick.measure_text(text, size, 1.0);
    draw_text_glow(
        d,
        &fonts.nick,
        text,
        Vector2::new(cx - m.x * 0.5, y),
        size,
        color,
    );
}

/// Frame-0 texture size of an uploaded clip, sprite px.
pub(crate) fn frame0_size(clip: &GpuClip) -> (f32, f32) {
    clip.frames
        .first()
        .map(|f| (f.tex.width as f32, f.tex.height as f32))
        .unwrap_or((0.0, 0.0))
}

/// A UI sprite's frame 0 at native pixel size (`(x, y)` is the clip
/// origin — HUD elements are positioned, not zoom-scaled).
pub(crate) fn draw_ui_icon(d: &mut RaylibDrawHandle, clip: &GpuClip, x: f32, y: f32) {
    TextureCache::draw_frame_offset(
        d,
        clip,
        0,
        (x, y),
        0.0,
        (1.0, 1.0),
        Color::WHITE,
        (0.0, 0.0),
    );
}

/// 0xRRGGBB effect text color.
pub(crate) fn color_u32(c: u32) -> Color {
    Color::new(
        ((c >> 16) & 0xff) as u8,
        ((c >> 8) & 0xff) as u8,
        (c & 0xff) as u8,
        255,
    )
}

/// f64 hp fraction.
pub(crate) fn ratio(now: Option<f64>, max: Option<f64>) -> f32 {
    match (now, max) {
        (Some(n), Some(m)) if m > 0.0 => (n / m) as f32,
        _ => 0.0,
    }
}

/// i32 shield fraction.
pub(crate) fn ratio_i(now: Option<i32>, max: Option<i32>) -> f32 {
    match (now, max) {
        (Some(n), Some(m)) if m > 0 => n as f32 / m as f32,
        _ => 0.0,
    }
}

/// Faction palette: 1=MMO red, 2=EIC blue, 3=VRU green, else neutral.
pub(crate) fn faction_color(faction_id: i32) -> Color {
    match faction_id {
        1 => Color::new(228, 62, 52, 255),
        2 => Color::new(64, 132, 236, 255),
        3 => Color::new(58, 205, 92, 255),
        _ => Color::new(215, 220, 225, 255),
    }
}

/// NPC / hostile red.
pub(crate) fn npc_color() -> Color {
    Color::new(255, 92, 66, 255)
}
