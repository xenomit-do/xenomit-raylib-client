use super::{PANEL_BG, PANEL_EDGE, TXT, TXT_DIM};
use crate::game::input;
use crate::locale::LocaleCatalog;
use crate::net::world::{KillScreen, KillScreenOption};
use crate::render::fonts::{draw_text_fallback, measure_text_fallback, Fonts};
use raylib::prelude::*;

const TITLE_BAR: Color = Color::new(40, 12, 14, 235);
const ROW_BG: Color = Color::new(18, 24, 36, 235);
const ROW_HOVER: Color = Color::new(30, 42, 62, 245);
const OK_BAR: Color = Color::new(90, 220, 140, 255);
const WARN_BAR: Color = Color::new(240, 190, 70, 255);
const BAD_BAR: Color = Color::new(210, 80, 70, 255);

/// Row label: the localized button key, else a readable fallback.
fn option_name(o: &KillScreenOption, locale: &LocaleCatalog) -> String {
    if !o.button_key.is_empty() {
        if let Some(t) = locale.lookup(&o.button_key).filter(|t| !t.is_empty()) {
            return t.to_owned();
        }
    }
    if o.repair_type == KillScreen::BASIC_REPAIR {
        "Basic repair (home base)".to_owned()
    } else if !o.button_key.is_empty() {
        o.button_key.clone()
    } else {
        format!("Repair option {}", o.repair_type)
    }
}

fn price_text(o: &KillScreenOption, locale: &LocaleCatalog) -> String {
    if o.cost <= 0 {
        "FREE".to_owned()
    } else {
        format!(
            "{} {}",
            locale.format_i64(o.cost),
            if o.currency == 1 { "URI" } else { "CRE" }
        )
    }
}

fn text_right(
    d: &mut RaylibDrawHandle,
    fonts: &Fonts,
    s: &str,
    right: f32,
    y: f32,
    size: f32,
    c: Color,
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
        c,
    );
}

fn text_center(
    d: &mut RaylibDrawHandle,
    fonts: &Fonts,
    s: &str,
    cx: f32,
    y: f32,
    size: f32,
    c: Color,
) {
    let m = measure_text_fallback(&fonts.ui, fonts.symbols.as_ref(), s, size, 1.0);
    draw_text_fallback(
        d,
        &fonts.ui,
        fonts.symbols.as_ref(),
        s,
        Vector2::new(cx - m.x * 0.5, y),
        size,
        1.0,
        c,
    );
}

pub(crate) fn draw_kill_screen(
    d: &mut RaylibDrawHandle,
    fonts: &Fonts,
    ks: &KillScreen,
    locale: &LocaleCatalog,
    vw: f32,
    vh: f32,
) {
    d.draw_rectangle(0, 0, vw as i32, vh as i32, Color::new(0, 0, 0, 150));
    let layout = input::kill_screen_layout_n(vw, vh, ks.options.len());
    let (x, y, w, h) = layout.panel;
    let mouse = d.get_mouse_position();
    let mouse = (mouse.x, mouse.y);

    // Window chrome: drop shadow, body, title bar, border.
    d.draw_rectangle(
        (x + 4.0) as i32,
        (y + 5.0) as i32,
        w as i32,
        h as i32,
        Color::new(0, 0, 0, 120),
    );
    d.draw_rectangle(x as i32, y as i32, w as i32, h as i32, PANEL_BG);
    d.draw_rectangle(x as i32, y as i32, w as i32, 28, TITLE_BAR);
    d.draw_rectangle(
        x as i32,
        y as i32 + 27,
        w as i32,
        1,
        Color::new(210, 80, 70, 200),
    );
    d.draw_rectangle_lines_ex(Rectangle::new(x, y, w, h), 1.0, PANEL_EDGE);
    d.draw_text_ex(
        &fonts.ui,
        "SHIP DESTROYED",
        Vector2::new(x + 12.0, y + 6.0),
        15.0,
        1.0,
        Color::new(255, 100, 85, 255),
    );

    let sub = if ks.killer_name.is_empty() {
        "Your ship was destroyed.".to_owned()
    } else {
        format!("Destroyed by {}", ks.killer_name)
    };
    draw_text_fallback(
        d,
        &fonts.ui,
        fonts.symbols.as_ref(),
        &sub,
        Vector2::new(x + 14.0, y + 36.0),
        11.0,
        1.0,
        TXT,
    );
    d.draw_text_ex(
        &fonts.ui,
        "Choose how to repair:",
        Vector2::new(x + 14.0, y + 52.0),
        10.0,
        1.0,
        TXT_DIM,
    );

    if ks.options.is_empty() {
        let (rx, ry) = (x + 14.0, y + input::KILL_HEADER_H);
        d.draw_text_ex(
            &fonts.ui,
            "Basic repair at the home base (free)",
            Vector2::new(rx, ry + 8.0),
            11.0,
            1.0,
            TXT_DIM,
        );
    }

    for (o, r) in ks.options.iter().zip(layout.rows.iter()) {
        let usable = o.affordable && o.cooldown_ms <= 0;
        let hover = usable && input::point_in(*r, mouse);
        d.draw_rectangle(
            r.0 as i32,
            r.1 as i32,
            r.2 as i32,
            r.3 as i32,
            if hover { ROW_HOVER } else { ROW_BG },
        );
        d.draw_rectangle_lines_ex(
            Rectangle::new(r.0, r.1, r.2, r.3),
            1.0,
            if hover {
                PANEL_EDGE
            } else {
                Color::new(50, 64, 88, 200)
            },
        );
        let bar = if usable {
            OK_BAR
        } else if !o.affordable {
            BAD_BAR
        } else {
            WARN_BAR
        };
        d.draw_rectangle(r.0 as i32, r.1 as i32, 4, r.3 as i32, bar);
        let name_col = if usable { TXT } else { TXT_DIM };
        draw_text_fallback(
            d,
            &fonts.ui,
            fonts.symbols.as_ref(),
            &option_name(o, locale),
            Vector2::new(r.0 + 12.0, r.1 + 7.0),
            12.0,
            1.0,
            name_col,
        );
        let price_col = if o.affordable { OK_BAR } else { BAD_BAR };
        text_right(
            d,
            fonts,
            &price_text(o, locale),
            r.0 + r.2 - 10.0,
            r.1 + 4.0,
            12.0,
            price_col,
        );
        let state = if o.cooldown_ms > 0 {
            format!("cooldown {}s", (o.cooldown_ms + 999) / 1000)
        } else if !o.affordable {
            "can't afford".to_owned()
        } else {
            String::new()
        };
        if !state.is_empty() {
            text_right(
                d,
                fonts,
                &state,
                r.0 + r.2 - 10.0,
                r.1 + 17.0,
                9.0,
                WARN_BAR,
            );
        }
    }

    let (bx, by, bw, bh) = layout.button;
    let hover = input::point_in(layout.button, mouse);
    d.draw_rectangle(
        bx as i32,
        by as i32,
        bw as i32,
        bh as i32,
        if hover {
            Color::new(40, 120, 80, 245)
        } else {
            Color::new(30, 90, 60, 230)
        },
    );
    d.draw_rectangle_lines_ex(
        Rectangle::new(bx, by, bw, bh),
        1.0,
        Color::new(120, 230, 160, 230),
    );
    text_center(
        d,
        fonts,
        "REPAIR  [R]",
        bx + bw * 0.5,
        by + bh * 0.5 - 7.0,
        13.0,
        Color::WHITE,
    );
}
