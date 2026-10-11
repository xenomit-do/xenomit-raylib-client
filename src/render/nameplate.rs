use crate::game::Game;
use crate::net::{DisplaySettings, ShipInfo};
use crate::render::draw_util::{color_u32, draw_ui_icon, frame0_size};
use crate::render::fonts::{draw_text_glow_fallback, measure_text_fallback, Fonts};
use crate::render::sprites::{GpuClip, TextureCache};
use raylib::prelude::*;

/// Title line color (warm gold — titles read as an honorific).
const TITLE_COLOR: u32 = 0xffd070;

/// Height of the `TopDisplay` hp/shield micro-bars (two 3px lanes).
pub(crate) const MICRO_BARS_H: f32 = 6.0;

/// `ShipInfoDisplay` gg-badge overlays: `galaxyGatesDone` bit i →
/// (`ui` symbol, offset in `ggIconBg`-local px).
const GG_OVERLAYS: &[(u8, &str, (f32, f32))] = &[
    (0, "alphaToDelta_gg", (0.0, 6.0)),
    (1, "alphaToDelta_gg", (6.0, 6.0)),
    (2, "alphaToDelta_gg", (12.0, 6.0)),
    (3, "alphaToDelta_gg", (6.0, 0.0)),
    (4, "epsilonZeta_gg", (0.0, 0.0)),
    (5, "epsilonZeta_gg", (12.0, 0.0)),
    (6, "kronos_gg", (0.0, 0.0)),
];

/// `LockDisplay2D` border frame: `ship_border` frames are `lockType - 1`
/// — NPCs and the default are ENEMY(1); players key off
/// `clanDiplomacy`: allied(1) → FRIEND(4), NAP(2) → NEUTRAL(2), else
/// ENEMY.
pub(crate) fn lock_type(npc: bool, info: &ShipInfo) -> usize {
    if npc {
        1
    } else {
        match info.clan_diplomacy {
            1 => 4,
            2 => 2,
            _ => 1,
        }
    }
}

/// `getNameColorID` name color (game.xml `<colors>`): hero `eeeeee`,
/// same clan `33ff33`, same faction `0099ff`, else `cc0000` — NPCs
/// included.
pub(crate) fn name_color(
    is_hero: bool,
    clan_id: i32,
    faction_id: i32,
    hero_clan: i32,
    hero_faction: i32,
) -> Color {
    let c = if is_hero {
        0xeeeeee
    } else if clan_id != 0 && clan_id == hero_clan {
        0x33ff33
    } else if faction_id != 0 && faction_id == hero_faction {
        0x0099ff
    } else {
        0xcc0000
    };
    color_u32(c)
}

/// Clan-tag color by `clanDiplomacy`: 1 allied, 2 NAP, 3 war, else grey.
pub(crate) fn tag_color(clan_diplomacy: i32) -> Color {
    color_u32(match clan_diplomacy {
        1 => 0x33ff33,
        2 => 0xffcc00,
        3 => 0xcc0000,
        _ => 0xeeeeee,
    })
}

fn draw_micro_bars(d: &mut RaylibDrawHandle, cx: f32, y: f32, w: f32, frac_hp: f32, frac_sh: f32) {
    let x = (cx - w * 0.5) as i32;
    let yi = y as i32;
    d.draw_rectangle(x, yi, w as i32, 3, Color::new(10, 12, 18, 200));
    if frac_sh > 0.0 {
        d.draw_rectangle(
            x,
            yi,
            (w * frac_sh.clamp(0.0, 1.0)) as i32,
            3,
            Color::new(80, 160, 255, 230),
        );
    }
    d.draw_rectangle(x, yi + 3, w as i32, 3, Color::new(10, 12, 18, 200));
    if frac_hp > 0.0 {
        let col = if frac_hp > 0.5 {
            Color::new(70, 220, 100, 230)
        } else if frac_hp > 0.25 {
            Color::new(240, 200, 60, 230)
        } else {
            Color::new(235, 70, 50, 230)
        };
        d.draw_rectangle(x, yi + 3, (w * frac_hp.clamp(0.0, 1.0)) as i32, 3, col);
    }
    d.draw_rectangle_lines(x, yi, w as i32, 6, Color::new(120, 140, 170, 160));
}

/// `Settings.displayPlayerName`: hides *player* plates (hero included);
/// NPC and pet plates always show.
pub(crate) fn plate_visible(settings: &DisplaySettings, is_player: bool) -> bool {
    !is_player || settings.display_player_name
}

/// Ceasefire-loss marker (`0|n|pvp|uid|0`): a small red warning
/// triangle with `!`, centred at `(cx, y)`.
pub(crate) fn draw_criminal_marker(d: &mut RaylibDrawHandle, fonts: &Fonts, cx: f32, y: f32) {
    let red = Color::new(235, 60, 50, 240);
    d.draw_triangle(
        Vector2::new(cx, y - 7.0),
        Vector2::new(cx - 7.0, y + 6.0),
        Vector2::new(cx + 7.0, y + 6.0),
        red,
    );
    let m = fonts.ui.measure_text("!", 10.0, 1.0);
    d.draw_text_ex(
        &fonts.ui,
        "!",
        Vector2::new(cx - m.x * 0.5, y - 3.0),
        10.0,
        1.0,
        Color::WHITE,
    );
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_name_plate(
    d: &mut RaylibDrawHandle,
    fonts: &Fonts,
    tex: &TextureCache,
    game: &Game,
    info: &ShipInfo,
    cx: f32,
    row_y: f32,
    name: &str,
    clan_tag: &str,
    name_col: Color,
    tag_col: Color,
    faction_id: i32,
) {
    const LINE: f32 = 13.0;
    const GAP: f32 = 3.0;

    let ui_clip = |src: &str, res: String| -> Option<&GpuClip> {
        game.ui_sprites
            .get(&(src.to_owned(), res))
            .and_then(|id| tex.get(*id))
            .filter(|c| !c.frames.is_empty())
    };
    let rank = (info.daily_rank != 0)
        .then(|| ui_clip("icons", format!("rank{}", info.daily_rank)))
        .flatten();
    let gg = (info.gg_done > 0)
        .then(|| ui_clip("ui", "ggIconBg".to_owned()))
        .flatten();
    let faction = (faction_id != 0)
        .then(|| ui_clip("icons", format!("fraction{faction_id}")))
        .flatten();
    let tag = if clan_tag.is_empty() {
        String::new()
    } else {
        format!("[{clan_tag}]")
    };
    let measure =
        |t: &str| measure_text_fallback(&fonts.nick, fonts.symbols.as_ref(), t, LINE, 1.0).x;

    let icon_w = [rank, gg]
        .into_iter()
        .flatten()
        .map(|c| frame0_size(c).0)
        .fold(0.0, f32::max);
    let sizes = [
        icon_w,
        if tag.is_empty() { 0.0 } else { measure(&tag) },
        if name.is_empty() { 0.0 } else { measure(name) },
        faction.map(|c| frame0_size(c).0).unwrap_or(0.0),
    ];
    let mut total = 0.0;
    let mut count = 0;
    for w in sizes {
        if w > 0.0 {
            if count > 0 {
                total += GAP;
            }
            total += w;
            count += 1;
        }
    }
    if count == 0 {
        return;
    }
    // `MapObjectInfoTrait` title: its own centred line above the plate.
    if !info.title.is_empty() {
        let tw = measure(&info.title);
        draw_text_glow_fallback(
            d,
            &fonts.nick,
            fonts.symbols.as_ref(),
            &info.title,
            Vector2::new(cx - tw * 0.5, row_y - LINE - 1.0),
            LINE,
            color_u32(TITLE_COLOR),
        );
    }
    let mut x = cx - total * 0.5;
    let gg_h = gg.map(|c| frame0_size(c).1).unwrap_or(0.0);
    let text_y = row_y + gg_h;

    if icon_w > 0.0 {
        let rank_top = rank
            .map(|r| text_y + (LINE - frame0_size(r).1) * 0.5)
            .unwrap_or(text_y);
        if let Some(bg) = gg {
            let (bw, _bh) = frame0_size(bg);
            let gx = x + (icon_w - bw) * 0.5;
            let gy = rank_top - gg_h - GAP;
            draw_ui_icon(d, bg, gx, gy);
            for (bit, res, (ox, oy)) in GG_OVERLAYS {
                if info.gg_done & (1 << bit) != 0 {
                    if let Some(ov) = ui_clip("ui", res.to_string()) {
                        draw_ui_icon(d, ov, gx + ox, gy + oy);
                    }
                }
            }
        }
        if let Some(r) = rank {
            let (w, _h) = frame0_size(r);
            draw_ui_icon(d, r, x + (icon_w - w) * 0.5, rank_top);
        }
        x += icon_w + GAP;
    }
    if !tag.is_empty() {
        draw_text_glow_fallback(
            d,
            &fonts.nick,
            fonts.symbols.as_ref(),
            &tag,
            Vector2::new(x, text_y),
            LINE,
            tag_col,
        );
        x += sizes[1] + GAP;
    }
    if !name.is_empty() {
        draw_text_glow_fallback(
            d,
            &fonts.nick,
            fonts.symbols.as_ref(),
            name,
            Vector2::new(x, text_y),
            LINE,
            name_col,
        );
        x += sizes[2] + GAP;
    }
    if let Some(f) = faction {
        let (_w, h) = frame0_size(f);
        draw_ui_icon(d, f, x, text_y + (LINE - h) * 0.5);
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_top_bars(
    d: &mut RaylibDrawHandle,
    tex: &TextureCache,
    game: &Game,
    info: &ShipInfo,
    cx: f32,
    y: f32,
    frac_hp: f32,
    frac_sh: f32,
) {
    const BARS_W: f32 = 46.0;
    const GAP: f32 = 5.0;
    let icon = (info.drone_count > 0)
        .then(|| {
            game.ui_sprites
                .get(&(
                    "icons".to_owned(),
                    format!("drone_formation{}_simple.png", info.drone_formation),
                ))
                .and_then(|id| tex.get(*id))
                .filter(|c| !c.frames.is_empty())
        })
        .flatten();
    let (iw, ih) = icon.map(frame0_size).unwrap_or((0.0, 0.0));
    let block_w = BARS_W + if iw > 0.0 { iw + GAP } else { 0.0 };
    let x0 = cx - block_w * 0.5;
    if let Some(clip) = icon {
        draw_ui_icon(d, clip, x0, y + MICRO_BARS_H - ih);
    }
    draw_micro_bars(d, x0 + block_w - BARS_W * 0.5, y, BARS_W, frac_hp, frac_sh);
}
