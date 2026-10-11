//! User window body: name + premium badge, currencies with coin glyphs,
//! level / EP / honour, jackpot and the active-booster row.

use super::fmt_thousands;
use crate::game::input;
use crate::net::attributes::Booster;
use crate::net::SharedWorld;
use crate::render::fonts::Fonts;
use crate::ui::chrome::{self, TXT, TXT_DIM};
use crate::ui::window::{point_in, Rect};
use raylib::prelude::*;
use xenomit_packet_client::model::HeroState;

/// Booster tile size / gap in the bottom row.
const BOOST_W: f32 = 30.0;
const BOOST_H: f32 = 20.0;
const BOOST_GAP: f32 = 3.0;

/// `1:23:45` / `12:05` countdown text.
pub(crate) fn fmt_duration(secs: i64) -> String {
    let s = secs.max(0);
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

/// Procedural currency coin (no atlas dependency): rim + face + mark.
fn draw_coin(d: &mut RaylibDrawHandle, c: (f32, f32), face: Color, rim: Color) {
    let v = Vector2::new(c.0, c.1);
    d.draw_circle_v(v, 6.0, rim);
    d.draw_circle_v(v, 4.6, face);
    d.draw_circle_lines(c.0 as i32, c.1 as i32, 2.4, rim);
}

pub(crate) fn draw_user_body(
    d: &mut RaylibDrawHandle,
    h: &HeroState,
    world: &SharedWorld,
    fonts: &Fonts,
    body: Rect,
    mouse: (f32, f32),
    tooltip: &mut Option<Vec<String>>,
) {
    let loc = &world.locale;
    let (bx, by, bw, _) = body;
    let right = bx + bw - 8.0;

    // Name + premium badge.
    let name = if h.user_name.is_empty() {
        format!("ID {}", h.user_id)
    } else {
        h.user_name.clone()
    };
    let badge_w = if h.premium { 54.0 } else { 0.0 };
    let name = chrome::fit_text(fonts, &name, 11.0, bw - 16.0 - badge_w - 4.0);
    chrome::label(d, fonts, &name, bx + 8.0, by + 5.0, 11.0, Color::WHITE);
    if h.premium {
        let r = (right - badge_w, by + 4.0, badge_w, 13.0);
        d.draw_rectangle_gradient_v(
            r.0 as i32,
            r.1 as i32,
            r.2 as i32,
            r.3 as i32,
            Color::new(236, 196, 70, 255),
            Color::new(176, 128, 28, 255),
        );
        d.draw_rectangle_lines_ex(chrome::rl_rect(r), 1.0, Color::new(70, 48, 8, 255));
        let m = fonts.ui.measure_text("PREMIUM", 8.0, 1.0);
        chrome::label(
            d,
            fonts,
            "PREMIUM",
            r.0 + (r.2 - m.x) * 0.5,
            r.1 + 2.0,
            8.0,
            Color::new(40, 26, 4, 255),
        );
    }

    chrome::label(
        d,
        fonts,
        &format!("ID {}   LV {}", h.user_id, h.level),
        bx + 8.0,
        by + 20.0,
        10.0,
        TXT_DIM,
    );

    // Currencies.
    draw_coin(
        d,
        (bx + 14.0, by + 40.0),
        Color::new(240, 200, 70, 255),
        Color::new(150, 104, 20, 255),
    );
    chrome::label(d, fonts, "Credits", bx + 25.0, by + 34.0, 10.0, TXT_DIM);
    chrome::label_right(
        d,
        fonts,
        &fmt_thousands(h.credits, loc),
        right,
        by + 34.0,
        10.0,
        TXT,
    );
    draw_coin(
        d,
        (bx + 14.0, by + 55.0),
        Color::new(70, 190, 235, 255),
        Color::new(20, 90, 130, 255),
    );
    chrome::label(d, fonts, "Uridium", bx + 25.0, by + 49.0, 10.0, TXT_DIM);
    chrome::label_right(
        d,
        fonts,
        &fmt_thousands(h.uridium, loc),
        right,
        by + 49.0,
        10.0,
        TXT,
    );

    // Experience / honour / jackpot.
    chrome::label(
        d,
        fonts,
        &format!("EP {}", fmt_thousands(h.ep, loc)),
        bx + 8.0,
        by + 64.0,
        10.0,
        TXT_DIM,
    );
    chrome::label_right(
        d,
        fonts,
        &format!("HON {}", fmt_thousands(h.honour, loc)),
        right,
        by + 64.0,
        10.0,
        TXT_DIM,
    );
    chrome::label(
        d,
        fonts,
        &format!("Jackpot {:.2}", h.jackpot),
        bx + 8.0,
        by + 76.0,
        10.0,
        TXT_DIM,
    );

    // Active boosters.
    let boosters: Vec<&Booster> = world.extras.boosters.values().collect();
    let row_y = by + 90.0;
    if boosters.is_empty() {
        chrome::label(
            d,
            fonts,
            "No active boosters",
            bx + 8.0,
            row_y + 5.0,
            9.0,
            TXT_DIM,
        );
        return;
    }
    let max_tiles = (((bw - 16.0) / (BOOST_W + BOOST_GAP)) as usize).max(1);
    for (i, b) in boosters.iter().take(max_tiles).enumerate() {
        let r = (
            bx + 8.0 + i as f32 * (BOOST_W + BOOST_GAP),
            row_y,
            BOOST_W,
            BOOST_H,
        );
        chrome::draw_inset(d, r);
        let label = input::short_item_label(&b.loot_id);
        let m = fonts.ui.measure_text(&label, 8.0, 1.0);
        chrome::label(
            d,
            fonts,
            &label,
            r.0 + (r.2 - m.x) * 0.5,
            r.1 + 1.0,
            8.0,
            TXT,
        );
        if let Some(left) = b.remaining() {
            let t = fmt_duration(left);
            let m = fonts.ui.measure_text(&t, 7.0, 1.0);
            chrome::label(
                d,
                fonts,
                &t,
                r.0 + (r.2 - m.x) * 0.5,
                r.1 + 10.0,
                7.0,
                Color::new(120, 210, 255, 255),
            );
        }
        if point_in(r, mouse) {
            let mut lines = vec![b.loot_id.clone()];
            if let Some(left) = b.remaining() {
                lines.push(format!("{} left", fmt_duration(left)));
            }
            *tooltip = Some(lines);
        }
    }
    if boosters.len() > max_tiles {
        chrome::label_right(
            d,
            fonts,
            &format!("+{}", boosters.len() - max_tiles),
            right,
            row_y + 5.0,
            9.0,
            TXT_DIM,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::fmt_duration;

    #[test]
    fn durations_format() {
        assert_eq!(fmt_duration(65), "1:05");
        assert_eq!(fmt_duration(3725), "1:02:05");
        assert_eq!(fmt_duration(-4), "0:00");
    }
}
