//! Log window body: the game console, newest line at the bottom,
//! wheel-scrollable into history.

use crate::net::world::LogLine;
use crate::render::fonts::Fonts;
use crate::ui::chrome::{self, INSET_EDGE};
use crate::ui::panels::{log_max_scroll, log_view, LOG_LINE_H};
use crate::ui::window::Rect;
use raylib::prelude::*;

const LOG_FADE_SECS: f32 = 30.0;
const LOG_TXT: (u8, u8, u8) = (178, 204, 184);

/// `scroll` = px distance from the newest line (0 = pinned to bottom).
pub(crate) fn draw_log_body(
    d: &mut RaylibDrawHandle,
    log: &[LogLine],
    fonts: &Fonts,
    body: Rect,
    scroll: f32,
) {
    let view = log_view(body);
    chrome::draw_inset(d, (view.0 - 2.0, view.1 - 2.0, view.2 + 4.0, view.3 + 4.0));
    let fit = (view.3 / LOG_LINE_H).floor() as usize;
    if fit == 0 || log.is_empty() {
        return;
    }
    let scroll = scroll.clamp(0.0, log_max_scroll(body, log.len()));
    let skip = (scroll / LOG_LINE_H).round() as usize;
    let end = log.len().saturating_sub(skip);
    let start = end.saturating_sub(fit);
    let now = std::time::Instant::now();
    let shown = &log[start..end];
    // Bottom-aligned: the newest visible line sits on the view's last row.
    let mut y = view.1 + view.3 - shown.len() as f32 * LOG_LINE_H;
    for l in shown {
        let age = now.saturating_duration_since(l.at).as_secs_f32();
        let a = (1.0 - (age / LOG_FADE_SECS) * 0.5).clamp(0.5, 1.0);
        let text = chrome::fit_text(fonts, &l.text, 10.0, view.2 - 8.0);
        chrome::label(
            d,
            fonts,
            &text,
            view.0 + 2.0,
            y,
            10.0,
            Color::new(LOG_TXT.0, LOG_TXT.1, LOG_TXT.2, (255.0 * a) as u8),
        );
        y += LOG_LINE_H;
    }
    // Scrolled into history: show where the bottom is.
    if skip > 0 {
        let m = fonts.ui.measure_text("v newer", 8.0, 1.0);
        chrome::label(
            d,
            fonts,
            "v newer",
            view.0 + view.2 - m.x - 4.0,
            view.1 + view.3 - 10.0,
            8.0,
            INSET_EDGE,
        );
    }
}
