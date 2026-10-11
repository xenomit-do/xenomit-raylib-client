//! Ship window body: HP / shield / nano-hull bars, cargo, C1/C2
//! configuration switch and the cargo ore breakdown.

use super::{fmt_num_i64, fmt_thousands};
use crate::locale::LocaleCatalog;
use crate::net::attributes::ore_name;
use crate::net::SharedWorld;
use crate::render::fonts::Fonts;
use crate::ui::chrome::{self, ButtonState, TXT, TXT_BAD, TXT_DIM, TXT_WARN};
use crate::ui::panels::{self, SHIP_ORE_ROWS};
use crate::ui::window::{point_in, Rect};
use raylib::prelude::*;
use xenomit_packet_client::model::HeroState;

const ORE_ROW_H: f32 = 11.0;

pub(crate) fn draw_ship_body(
    d: &mut RaylibDrawHandle,
    h: &HeroState,
    world: &SharedWorld,
    fonts: &Fonts,
    body: Rect,
    mouse: (f32, f32),
) {
    let loc: &LocaleCatalog = &world.locale;
    let (bx, by, bw, _) = body;
    let bar_x = bx + 30.0;
    let bar_w = bw - 108.0;
    let val_right = bx + bw - 8.0;

    let hp_frac = if h.hitpoints_max > 0.0 {
        (h.hitpoints / h.hitpoints_max) as f32
    } else {
        0.0
    };
    let hp_col = if hp_frac > 0.5 {
        Color::new(70, 200, 100, 235)
    } else if hp_frac > 0.25 {
        Color::new(230, 190, 60, 235)
    } else {
        Color::new(225, 70, 50, 235)
    };
    vitals_row(
        d,
        fonts,
        "HP",
        &format!(
            "{}/{}",
            fmt_thousands(h.hitpoints, loc),
            fmt_thousands(h.hitpoints_max, loc)
        ),
        (bx + 8.0, bar_x, val_right, by + 7.0, bar_w, 8.0),
        hp_frac,
        hp_col,
    );
    let sh_frac = frac_i(h.shield, h.shield_max);
    vitals_row(
        d,
        fonts,
        "SH",
        &format!(
            "{}/{}",
            fmt_num_i64(h.shield as i64, loc),
            fmt_num_i64(h.shield_max as i64, loc)
        ),
        (bx + 8.0, bar_x, val_right, by + 19.0, bar_w, 8.0),
        sh_frac,
        Color::new(70, 150, 245, 235),
    );
    let nh_frac = frac_i(h.nano_hull, h.nano_hull_max);
    vitals_row(
        d,
        fonts,
        "NH",
        &format!(
            "{}/{}",
            fmt_num_i64(h.nano_hull as i64, loc),
            fmt_num_i64(h.nano_hull_max as i64, loc)
        ),
        (bx + 8.0, bar_x, val_right, by + 31.0, bar_w, 6.0),
        nh_frac,
        Color::new(240, 160, 55, 235),
    );

    // Cargo line + bar.
    let cargo_frac = frac_i(h.cargo_space, h.cargo_space_max);
    chrome::label(
        d,
        fonts,
        &format!(
            "CARGO {}/{}",
            loc.format_i64(h.cargo_space as i64),
            loc.format_i64(h.cargo_space_max as i64)
        ),
        bx + 8.0,
        by + 42.0,
        10.0,
        if cargo_frac >= 0.98 { TXT_WARN } else { TXT },
    );
    chrome::label_right(
        d,
        fonts,
        &format!("SPD {}", h.speed),
        val_right,
        by + 42.0,
        10.0,
        TXT_DIM,
    );
    chrome::draw_bar(
        d,
        (bx + 8.0, by + 55.0, bw - 16.0, 6.0),
        cargo_frac,
        if cargo_frac >= 0.98 {
            Color::new(225, 70, 50, 235)
        } else {
            Color::new(120, 150, 190, 235)
        },
    );

    // Configuration switch.
    let l = panels::ship_layout(body);
    chrome::label(
        d,
        fonts,
        "CONFIG",
        bx + 8.0,
        l.config[0].1 + 3.0,
        9.0,
        TXT_DIM,
    );
    let active = world.extras.config;
    for (i, r) in l.config.iter().enumerate() {
        chrome::draw_button(
            d,
            fonts,
            *r,
            &format!("C{}", i + 1),
            ButtonState {
                hover: point_in(*r, mouse),
                active: active == i as i32 + 1,
                disabled: false,
            },
        );
    }

    // Ore breakdown (2 columns × 3 rows).
    let rows = world.extras.ore_rows();
    let (ox, oy) = l.ore_origin;
    if rows.is_empty() {
        chrome::label(d, fonts, "No ore in the hold", ox, oy + 8.0, 9.0, TXT_DIM);
        return;
    }
    let col_w = (bw - 16.0) * 0.5;
    for (i, (ty, amount)) in rows.iter().take(SHIP_ORE_ROWS).enumerate() {
        let (col, row) = (i / 3, i % 3);
        let x = ox + col as f32 * col_w;
        let y = oy + row as f32 * ORE_ROW_H;
        let amt = loc.format_number(*amount);
        let amt_w = fonts.ui.measure_text(&amt, 9.0, 1.0).x;
        let name = chrome::fit_text(fonts, &ore_name(*ty), 9.0, col_w - amt_w - 12.0);
        chrome::label(d, fonts, &name, x, y, 9.0, TXT);
        chrome::label_right(d, fonts, &amt, x + col_w - 6.0, y, 9.0, TXT_DIM);
    }
    let hidden = rows.len().saturating_sub(SHIP_ORE_ROWS);
    if hidden > 0 {
        chrome::label_right(
            d,
            fonts,
            &format!("+{hidden} more"),
            bx + bw - 8.0,
            by + 79.0,
            8.0,
            TXT_BAD,
        );
    }
}

fn frac_i(v: i32, max: i32) -> f32 {
    if max > 0 {
        v as f32 / max as f32
    } else {
        0.0
    }
}

/// One labelled bar: `(label_x, bar_x, value_right, y, bar_w, bar_h)`.
pub(crate) fn vitals_row(
    d: &mut RaylibDrawHandle,
    fonts: &Fonts,
    label: &str,
    value: &str,
    geo: (f32, f32, f32, f32, f32, f32),
    frac: f32,
    col: Color,
) {
    let (lx, bx, vx, y, w, h) = geo;
    chrome::label(d, fonts, label, lx, y - 1.0, 10.0, TXT_DIM);
    chrome::draw_bar(d, (bx, y, w, h), frac, col);
    chrome::label_right(d, fonts, value, vx, y - 1.0, 8.0, TXT_DIM);
}
