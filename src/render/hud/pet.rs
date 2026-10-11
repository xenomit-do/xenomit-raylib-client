//! P.E.T. window body: name/level, HP / shield / fuel / XP bars,
//! summon + dismiss buttons and the scrollable gear list.

use super::fmt_num_i64;
use crate::net::pet::gear_name;
use crate::net::SharedWorld;
use crate::render::fonts::Fonts;
use crate::ui::chrome::{self, ButtonState, ACCENT, TXT, TXT_DIM, TXT_GOOD};
use crate::ui::panels::{self, PET_GEAR_ROW_H};
use crate::ui::window::{point_in, Rect};
use raylib::prelude::*;

pub(crate) fn draw_pet_body(
    d: &mut RaylibDrawHandle,
    world: &SharedWorld,
    fonts: &Fonts,
    body: Rect,
    scroll: f32,
    mouse: (f32, f32),
) {
    let loc = &world.locale;
    let (bx, by, bw, _) = body;
    let pet = world.pets.hero_pet();
    let right = bx + bw - 8.0;

    match pet {
        Some(p) => {
            let title = chrome::fit_text(fonts, &p.plate_name(), 10.0, bw - 16.0);
            chrome::label(d, fonts, &title, bx + 8.0, by + 5.0, 10.0, Color::WHITE);
            let state = if p.destroyed {
                ("destroyed", Color::new(235, 90, 70, 255))
            } else if p.idle {
                ("idle", TXT_DIM)
            } else {
                ("active", TXT_GOOD)
            };
            chrome::label(d, fonts, state.0, bx + 8.0, by + 17.0, 9.0, state.1);

            let rows: [(&str, i32, i32, Color); 3] = [
                (
                    "HP",
                    p.hitpoints,
                    p.hitpoints_max,
                    Color::new(70, 200, 100, 235),
                ),
                ("SH", p.shield, p.shield_max, Color::new(70, 150, 245, 235)),
                ("FUEL", p.fuel, p.fuel_max, Color::new(240, 160, 55, 235)),
            ];
            let mut y = by + 30.0;
            for (label, v, max, col) in rows {
                if max <= 0 {
                    continue;
                }
                chrome::label(d, fonts, label, bx + 8.0, y - 1.0, 9.0, TXT_DIM);
                chrome::draw_bar(
                    d,
                    (bx + 38.0, y, bw - 38.0 - 86.0, 7.0),
                    v as f32 / max as f32,
                    col,
                );
                chrome::label_right(
                    d,
                    fonts,
                    &format!(
                        "{}/{}",
                        fmt_num_i64(v as i64, loc),
                        fmt_num_i64(max as i64, loc)
                    ),
                    right,
                    y - 1.0,
                    8.0,
                    TXT_DIM,
                );
                y += 11.0;
            }
            if p.experience_to_next > 0.0 {
                let total = p.experience + p.experience_to_next;
                chrome::label(d, fonts, "XP", bx + 8.0, y - 1.0, 9.0, TXT_DIM);
                chrome::draw_bar(
                    d,
                    (bx + 38.0, y, bw - 38.0 - 86.0, 7.0),
                    (p.experience / total.max(1.0)) as f32,
                    Color::new(190, 190, 90, 235),
                );
                chrome::label_right(
                    d,
                    fonts,
                    &format!("{}", loc.format_number(p.experience)),
                    right,
                    y - 1.0,
                    8.0,
                    TXT_DIM,
                );
            }
        }
        None => {
            let note = match world.pets.init {
                Some((false, _, _)) => "You do not own a P.E.T.",
                Some((_, _, false)) => "Your P.E.T. is destroyed.",
                _ => "P.E.T. is not summoned.",
            };
            chrome::label(d, fonts, note, bx + 8.0, by + 8.0, 10.0, TXT_DIM);
        }
    }

    let l = panels::pet_layout(body, world.pets.gears.len(), scroll);
    let summoned = pet.is_some();
    chrome::draw_button(
        d,
        fonts,
        l.activate,
        "Summon",
        ButtonState {
            hover: point_in(l.activate, mouse),
            active: false,
            disabled: summoned,
        },
    );
    chrome::draw_button(
        d,
        fonts,
        l.deactivate,
        "Dismiss",
        ButtonState {
            hover: point_in(l.deactivate, mouse),
            active: false,
            disabled: !summoned,
        },
    );

    chrome::label(d, fonts, "GEARS", bx + 8.0, by + 93.0, 9.0, TXT_DIM);
    chrome::draw_inset(
        d,
        (
            l.gear_view.0 - 1.0,
            l.gear_view.1 - 1.0,
            l.gear_view.2 + 2.0,
            l.gear_view.3 + 2.0,
        ),
    );
    if world.pets.gears.is_empty() {
        chrome::label(
            d,
            fonts,
            "No gears.",
            l.gear_view.0 + 6.0,
            l.gear_view.1 + 5.0,
            9.0,
            TXT_DIM,
        );
        return;
    }
    let view = l.gear_view;
    for (i, r) in &l.gears {
        // No scissor: rows straddling the view edge are skipped.
        if r.1 < view.1 || r.1 + r.3 > view.1 + view.3 + 1.0 {
            continue;
        }
        let Some(g) = world.pets.gears.get(*i) else {
            continue;
        };
        let hover = point_in(*r, mouse) && g.enabled;
        if g.active {
            d.draw_rectangle(
                r.0 as i32,
                r.1 as i32,
                r.2 as i32,
                r.3 as i32,
                Color::new(18, 70, 100, 200),
            );
        } else if hover {
            d.draw_rectangle(
                r.0 as i32,
                r.1 as i32,
                r.2 as i32,
                r.3 as i32,
                Color::new(40, 66, 100, 200),
            );
        }
        let col = if !g.enabled {
            TXT_DIM
        } else if g.active {
            Color::WHITE
        } else {
            TXT
        };
        // Active LED.
        let led = if g.active {
            TXT_GOOD
        } else {
            Color::new(54, 66, 84, 255)
        };
        d.draw_circle_v(Vector2::new(r.0 + 7.0, r.1 + r.3 * 0.5), 3.0, led);
        let suffix = if g.level > 0 {
            format!("  Lv {}", g.level)
        } else {
            String::new()
        };
        let name = chrome::fit_text(
            fonts,
            &format!("{}{}", gear_name(g.gear_type), suffix),
            10.0,
            r.2 - 22.0,
        );
        chrome::label(d, fonts, &name, r.0 + 15.0, r.1 + 3.0, 10.0, col);
        if world.pets.selected_gear == Some(g.gear_type) {
            d.draw_rectangle_lines_ex(chrome::rl_rect(*r), 1.0, ACCENT);
        }
    }
    // Scroll thumb.
    let max = panels::pet_max_scroll(body, world.pets.gears.len());
    if max > 0.0 {
        let total = world.pets.gears.len() as f32 * PET_GEAR_ROW_H;
        let th = (view.3 * view.3 / total).clamp(12.0, view.3);
        let ty = view.1 + (view.3 - th) * (scroll / max).clamp(0.0, 1.0);
        d.draw_rectangle(
            (view.0 + view.2 - 3.0) as i32,
            ty as i32,
            2,
            th as i32,
            Color::new(110, 150, 200, 200),
        );
    }
}
