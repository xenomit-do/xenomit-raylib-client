//! Top-center reward toast stack — resolved loot lines (`+6.350 Credits`)
//! that fade out after a few seconds, newest at the top.

use crate::net::world::Toast;
use crate::render::fonts::Fonts;
use raylib::prelude::*;
use std::time::Instant;

/// Seconds a toast stays fully visible before fading.
const HOLD_SECS: f32 = 2.5;
/// Fade-out duration after the hold.
const FADE_SECS: f32 = 1.5;
/// Row pitch / font size.
const ROW_H: f32 = 17.0;
const FONT: f32 = 12.0;
/// Distance from the top edge of the window.
const TOP: f32 = 44.0;

/// Opacity 0..=1 for a toast of `age` seconds.
pub(crate) fn toast_alpha(age: f32) -> f32 {
    if age <= HOLD_SECS {
        1.0
    } else {
        (1.0 - (age - HOLD_SECS) / FADE_SECS).clamp(0.0, 1.0)
    }
}

pub(crate) fn draw_toasts(d: &mut RaylibDrawHandle, toasts: &[Toast], fonts: &Fonts, vw: f32) {
    let now = Instant::now();
    let mut y = TOP;
    for t in toasts.iter().rev() {
        let alpha = toast_alpha(now.saturating_duration_since(t.at).as_secs_f32());
        if alpha <= 0.02 {
            continue;
        }
        // Rewards may carry several lines (`farmresult\n5 Prometium`).
        for line in t.text.lines().filter(|l| !l.trim().is_empty()) {
            let m = fonts.ui.measure_text(line, FONT, 1.0);
            let x = (vw - m.x) * 0.5;
            d.draw_rectangle(
                (x - 8.0) as i32,
                (y - 2.0) as i32,
                (m.x + 16.0) as i32,
                (ROW_H - 1.0) as i32,
                Color::new(8, 12, 20, (150.0 * alpha) as u8),
            );
            d.draw_text_ex(
                &fonts.ui,
                line,
                Vector2::new(x, y),
                FONT,
                1.0,
                Color::new(255, 224, 120, (240.0 * alpha) as u8),
            );
            y += ROW_H;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alpha_holds_then_fades() {
        assert_eq!(toast_alpha(0.0), 1.0);
        assert_eq!(toast_alpha(HOLD_SECS), 1.0);
        let mid = toast_alpha(HOLD_SECS + FADE_SECS * 0.5);
        assert!((mid - 0.5).abs() < 1e-5);
        assert_eq!(toast_alpha(HOLD_SECS + FADE_SECS + 1.0), 0.0);
    }
}
