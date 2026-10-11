//! Quests window body: accepted quests with objective checkboxes and a
//! cancel button, the available list (click for details) and recent
//! results.

use crate::net::quests::QuestEnd;
use crate::net::SharedWorld;
use crate::render::fonts::Fonts;
use crate::ui::chrome::{self, ButtonState, ACCENT, TXT, TXT_BAD, TXT_DIM, TXT_GOOD, TXT_WARN};
use crate::ui::panels::{self, QuestRowKind};
use crate::ui::window::{point_in, Rect};
use crate::ui::UiState;
use raylib::prelude::*;

fn checkbox(d: &mut RaylibDrawHandle, x: f32, y: f32, done: bool, active: bool) {
    let r = (x, y, 9.0, 9.0);
    chrome::draw_inset(d, r);
    if done {
        let c = TXT_GOOD;
        d.draw_line_ex(
            Vector2::new(x + 2.0, y + 5.0),
            Vector2::new(x + 4.0, y + 7.0),
            1.5,
            c,
        );
        d.draw_line_ex(
            Vector2::new(x + 4.0, y + 7.0),
            Vector2::new(x + 8.0, y + 2.0),
            1.5,
            c,
        );
    } else if !active {
        d.draw_line(
            x as i32 + 2,
            y as i32 + 4,
            x as i32 + 6,
            y as i32 + 4,
            TXT_DIM,
        );
    }
}

pub(crate) fn draw_quests_body(
    d: &mut RaylibDrawHandle,
    world: &SharedWorld,
    ui: &UiState,
    fonts: &Fonts,
    body: Rect,
    scroll: f32,
    mouse: (f32, f32),
) {
    let level = world.world.hero.as_ref().map(|h| h.level).unwrap_or(0);
    let (rows, content_h) =
        panels::quest_rows(&world.quests, &world.locale, ui.selected_quest, level);
    let scroll = scroll.clamp(0.0, panels::quest_max_scroll(body, content_h));
    let view = panels::quest_view(body);

    for row in &rows {
        let r = panels::quest_row_rect(body, row, scroll);
        // No scissor: rows straddling the view edge are skipped.
        if r.1 < view.1 || r.1 + r.3 > view.1 + view.3 {
            continue;
        }
        let (x, w) = (r.0, r.2);
        match &row.kind {
            QuestRowKind::Section(s) => {
                chrome::label(d, fonts, s, x + 2.0, r.1 + 4.0, 9.0, ACCENT);
                d.draw_line(
                    x as i32 + 2,
                    (r.1 + r.3 - 1.0) as i32,
                    (x + w - 2.0) as i32,
                    (r.1 + r.3 - 1.0) as i32,
                    Color::new(40, 64, 96, 255),
                );
            }
            QuestRowKind::Active {
                id,
                title,
                badge,
                done,
            } => {
                let cancel = panels::quest_cancel_rect(body, row.y, scroll);
                let badge_w = if badge.is_empty() {
                    0.0
                } else {
                    fonts.ui.measure_text(badge, 8.0, 1.0).x + 8.0
                };
                let max_w = cancel.0 - x - 6.0 - badge_w;
                let t = chrome::fit_text(fonts, title, 10.0, max_w);
                chrome::label(
                    d,
                    fonts,
                    &t,
                    x + 2.0,
                    r.1 + 5.0,
                    10.0,
                    if *done { TXT_GOOD } else { Color::WHITE },
                );
                if !badge.is_empty() {
                    chrome::label_right(d, fonts, badge, cancel.0 - 6.0, r.1 + 6.0, 8.0, TXT_WARN);
                }
                chrome::draw_button(
                    d,
                    fonts,
                    cancel,
                    "Cancel",
                    ButtonState {
                        hover: point_in(cancel, mouse),
                        active: false,
                        disabled: false,
                    },
                );
                let _ = id;
            }
            QuestRowKind::Objective(o) => {
                let ind = 6.0 + o.depth as f32 * 10.0;
                checkbox(d, x + ind, r.1 + 2.0, o.done, o.active);
                let prog = o
                    .progress
                    .map(|(c, t)| format!("{}/{}", short_num(c), short_num(t)));
                let prog_w = prog
                    .as_ref()
                    .map(|p| fonts.ui.measure_text(p, 9.0, 1.0).x + 4.0)
                    .unwrap_or(0.0);
                let col = if o.done {
                    TXT_DIM
                } else if o.mandatory {
                    TXT
                } else {
                    Color::new(170, 180, 196, 255)
                };
                let t = chrome::fit_text(fonts, &o.text, 9.0, w - ind - 20.0 - prog_w);
                chrome::label(d, fonts, &t, x + ind + 13.0, r.1 + 2.0, 9.0, col);
                if let Some(p) = prog {
                    chrome::label_right(
                        d,
                        fonts,
                        &p,
                        x + w - 4.0,
                        r.1 + 2.0,
                        9.0,
                        if o.done { TXT_GOOD } else { TXT_DIM },
                    );
                }
            }
            QuestRowKind::Available {
                title,
                min_level,
                selected,
                locked,
                ..
            } => {
                if *selected {
                    d.draw_rectangle(
                        r.0 as i32,
                        r.1 as i32,
                        r.2 as i32,
                        r.3 as i32,
                        Color::new(24, 60, 92, 200),
                    );
                } else if point_in(r, mouse) {
                    d.draw_rectangle(
                        r.0 as i32,
                        r.1 as i32,
                        r.2 as i32,
                        r.3 as i32,
                        Color::new(36, 56, 86, 170),
                    );
                }
                let lv = if *min_level > 0 {
                    format!("Lv {min_level}")
                } else {
                    String::new()
                };
                let lv_w = fonts.ui.measure_text(&lv, 8.0, 1.0).x;
                let t = chrome::fit_text(fonts, title, 10.0, w - lv_w - 18.0);
                chrome::label(
                    d,
                    fonts,
                    &t,
                    x + 8.0,
                    r.1 + 2.0,
                    10.0,
                    if *locked { TXT_DIM } else { TXT },
                );
                chrome::label_right(
                    d,
                    fonts,
                    &lv,
                    x + w - 4.0,
                    r.1 + 3.0,
                    8.0,
                    if *locked { TXT_BAD } else { TXT_DIM },
                );
            }
            QuestRowKind::Finished { title, end } => {
                let (txt, col) = match end {
                    QuestEnd::Completed => ("completed", TXT_GOOD),
                    QuestEnd::Failed => ("failed", TXT_BAD),
                    QuestEnd::Cancelled => ("cancelled", TXT_DIM),
                };
                let ew = fonts.ui.measure_text(txt, 8.0, 1.0).x;
                let t = chrome::fit_text(fonts, title, 9.0, w - ew - 18.0);
                chrome::label(d, fonts, &t, x + 8.0, r.1 + 2.0, 9.0, TXT_DIM);
                chrome::label_right(d, fonts, txt, x + w - 4.0, r.1 + 3.0, 8.0, col);
            }
            QuestRowKind::Note(s) => {
                chrome::label(d, fonts, s, x + 8.0, r.1 + 3.0, 9.0, TXT_DIM);
            }
        }
    }

    // Scroll thumb.
    let max = panels::quest_max_scroll(body, content_h);
    if max > 0.0 {
        let th = (view.3 * view.3 / content_h).clamp(14.0, view.3);
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

/// `1234` → `1234`, `1500000` → `1.5M` — objective counters.
fn short_num(v: f64) -> String {
    let a = v.abs();
    if a >= 1.0e6 {
        format!("{:.1}M", v / 1.0e6)
    } else if a >= 1.0e4 {
        format!("{:.1}k", v / 1.0e3)
    } else {
        format!("{}", v.round() as i64)
    }
}

#[cfg(test)]
mod tests {
    use super::short_num;

    #[test]
    fn counters_abbreviate() {
        assert_eq!(short_num(7.0), "7");
        assert_eq!(short_num(9999.0), "9999");
        assert_eq!(short_num(25000.0), "25.0k");
        assert_eq!(short_num(1_500_000.0), "1.5M");
    }
}
