//! F3 display-settings overlay (stub window): one checkbox row per
//! [`SettingFlag`]; clicks / digit keys are handled in `Game::poll_input`.

use super::{panel, PANEL_EDGE, TXT, TXT_DIM};
use crate::assets::uberlib::TextureQuality;
use crate::game::input;
use crate::game::settings_ui::{self, RenderSetting, RenderSettingsDraft};
use crate::net::DisplaySettings;
use crate::render::fonts::Fonts;
use raylib::prelude::*;

pub(crate) fn draw_settings(
    d: &mut RaylibDrawHandle,
    fonts: &Fonts,
    settings: &DisplaySettings,
    vw: f32,
    vh: f32,
) {
    let (rect, rows) = settings_ui::layout(vw, vh);
    panel(d, fonts, "", rect);
    d.draw_text_ex(
        &fonts.ui,
        "DISPLAY SETTINGS  [F3]",
        Vector2::new(rect.0 + 10.0, rect.1 + 7.0),
        11.0,
        1.0,
        TXT,
    );
    let mouse = d.get_mouse_position();
    for (i, (flag, r)) in rows.iter().enumerate() {
        let on = settings.get(*flag);
        if input::point_in(*r, (mouse.x, mouse.y)) {
            d.draw_rectangle(
                r.0 as i32,
                r.1 as i32,
                r.2 as i32,
                r.3 as i32,
                Color::new(40, 56, 82, 200),
            );
        }
        let bx = r.0 + 6.0;
        let by = r.1 + 5.0;
        d.draw_rectangle_lines_ex(Rectangle::new(bx, by, 12.0, 12.0), 1.0, PANEL_EDGE);
        if on {
            d.draw_rectangle(
                bx as i32 + 3,
                by as i32 + 3,
                7,
                7,
                Color::new(90, 220, 140, 255),
            );
        }
        d.draw_text_ex(
            &fonts.ui,
            flag.label(),
            Vector2::new(r.0 + 26.0, r.1 + 5.0),
            11.0,
            1.0,
            if on { TXT } else { TXT_DIM },
        );
        d.draw_text_ex(
            &fonts.ui,
            &format!("{}", i + 1),
            Vector2::new(r.0 + r.2 - 12.0, r.1 + 5.0),
            10.0,
            1.0,
            TXT_DIM,
        );
    }
    d.draw_text_ex(
        &fonts.ui,
        "click or press 1-8 to toggle",
        Vector2::new(rect.0 + 10.0, rect.1 + rect.3 - 15.0),
        9.0,
        1.0,
        TXT_DIM,
    );
}

pub(crate) fn draw_render_settings(
    d: &mut RaylibDrawHandle,
    fonts: &Fonts,
    draft: Option<&RenderSettingsDraft>,
    vw: f32,
    vh: f32,
) {
    let Some(draft) = draft else {
        return;
    };
    let (rect, rows) = settings_ui::render_layout(vw, vh);
    d.draw_rectangle(0, 0, vw as i32, vh as i32, Color::new(0, 0, 0, 110));
    panel(d, fonts, "", rect);
    // Graphite body and beveled header follow the game's custom-settings
    // window; geometry and hit regions are shared with the input pass.
    let inner = Rectangle::new(rect.0 + 10.0, rect.1 + 75.0, rect.2 - 20.0, rect.3 - 134.0);
    d.draw_rectangle_rec(inner, Color::new(12, 17, 22, 247));
    for y in (0..rect.3 as i32).step_by(5) {
        d.draw_line(
            rect.0 as i32 + 3,
            rect.1 as i32 + y,
            rect.0 as i32 + 8,
            rect.1 as i32 + y + 5,
            Color::new(90, 110, 125, 35),
        );
    }
    d.draw_rectangle_gradient_v(
        rect.0 as i32 + 8,
        rect.1 as i32 + 7,
        rect.2 as i32 - 16,
        27,
        Color::new(74, 88, 99, 255),
        Color::new(22, 29, 37, 255),
    );
    d.draw_text_ex(
        &fonts.ui,
        "Settings",
        Vector2::new(rect.0 + 23.0, rect.1 + 11.0),
        18.0,
        1.0,
        Color::new(229, 237, 243, 255),
    );
    d.draw_text_ex(
        &fonts.ui,
        "CUSTOM",
        Vector2::new(rect.0 + rect.2 - 95.0, rect.1 + 16.0),
        12.0,
        1.0,
        Color::new(178, 213, 228, 255),
    );
    let general = rows
        .iter()
        .find(|(f, _)| *f == RenderSetting::General)
        .unwrap()
        .1;
    let tab = Rectangle::new(
        general.0 + general.2 + 3.0,
        general.1,
        rect.2 - 46.0 - general.2 - 3.0,
        general.3,
    );
    d.draw_rectangle_rec(tab, Color::new(24, 58, 72, 255));
    d.draw_rectangle_lines_ex(tab, 1.5, Color::new(111, 177, 200, 230));
    centered(
        d,
        fonts,
        "3D SETTINGS",
        (tab.x, tab.y, tab.width, tab.height),
        14.0,
        Color::new(230, 232, 166, 255),
    );
    let mouse = d.get_mouse_position();
    for (field, r) in &rows {
        let hovered = input::point_in(*r, (mouse.x, mouse.y));
        if let Some(is_3d) = field.get(draft.0) {
            if hovered {
                d.draw_rectangle_rec(
                    Rectangle::new(r.0, r.1, r.2, r.3),
                    Color::new(43, 71, 86, 130),
                );
            }
            let font_size = (r.3 * 0.37).clamp(10.0, 15.0);
            d.draw_text_ex(
                &fonts.ui,
                field.label(),
                Vector2::new(r.0 + 12.0, r.1 + (r.3 - font_size) * 0.5),
                font_size,
                1.0,
                if hovered {
                    Color::WHITE
                } else {
                    Color::new(184, 195, 205, 255)
                },
            );
            let (two_d, three_d) = settings_ui::component_mode_buttons(*r);
            draw_mode_button(
                d,
                fonts,
                two_d,
                "2D",
                !is_3d,
                input::point_in(two_d, (mouse.x, mouse.y)),
            );
            draw_mode_button(
                d,
                fonts,
                three_d,
                "3D",
                is_3d,
                input::point_in(three_d, (mouse.x, mouse.y)),
            );
        } else if *field == RenderSetting::TextureQuality {
            if hovered {
                d.draw_rectangle_rec(
                    Rectangle::new(r.0, r.1, r.2, r.3),
                    Color::new(43, 71, 86, 130),
                );
            }
            let font_size = (r.3 * 0.37).clamp(10.0, 15.0);
            d.draw_text_ex(
                &fonts.ui,
                field.label(),
                Vector2::new(r.0 + 12.0, r.1 + (r.3 - font_size) * 0.5),
                font_size,
                1.0,
                if hovered {
                    Color::WHITE
                } else {
                    Color::new(184, 195, 205, 255)
                },
            );
            let buttons = settings_ui::quality_mode_buttons(*r);
            for (b, quality) in buttons.iter().zip(TextureQuality::ALL) {
                draw_mode_button(
                    d,
                    fonts,
                    *b,
                    quality.label(),
                    draft.0.texture_quality == quality,
                    input::point_in(*b, (mouse.x, mouse.y)),
                );
            }
        } else {
            let color = match field {
                RenderSetting::Save => Color::new(72, 128, 38, 255),
                RenderSetting::Cancel => Color::new(33, 104, 126, 255),
                RenderSetting::Reset => Color::new(137, 45, 52, 255),
                _ => Color::new(24, 55, 69, 255),
            };
            d.draw_rectangle_rounded(
                Rectangle::new(r.0, r.1, r.2, r.3),
                if *field == RenderSetting::General {
                    0.0
                } else {
                    0.30
                },
                6,
                color,
            );
            d.draw_rectangle_lines_ex(
                Rectangle::new(r.0 + 1.0, r.1 + 1.0, r.2 - 2.0, r.3 - 2.0),
                if hovered { 2.0 } else { 1.0 },
                Color::new(120, 176, 192, 200),
            );
            centered(
                d,
                fonts,
                field.label(),
                *r,
                if *field == RenderSetting::General {
                    14.0
                } else {
                    16.0
                },
                Color::new(229, 240, 246, 255),
            );
        }
    }
    d.draw_text_ex(
        &fonts.ui,
        "Choose 2D or 3D per component — Save to apply",
        Vector2::new(rect.0 + 16.0, rect.1 + rect.3 - 18.0),
        9.0,
        1.0,
        TXT_DIM,
    );
}

fn draw_mode_button(
    d: &mut RaylibDrawHandle,
    fonts: &Fonts,
    r: input::Rect,
    label: &str,
    selected: bool,
    hover: bool,
) {
    let top = if selected {
        Color::new(36, 118, 142, 255)
    } else if hover {
        Color::new(28, 62, 78, 255)
    } else {
        Color::new(16, 38, 48, 255)
    };
    let bottom = if selected {
        Color::new(12, 52, 66, 255)
    } else {
        Color::new(5, 18, 24, 255)
    };
    d.draw_rectangle_gradient_v(r.0 as i32, r.1 as i32, r.2 as i32, r.3 as i32, top, bottom);
    d.draw_rectangle_lines_ex(
        Rectangle::new(r.0, r.1, r.2, r.3),
        if selected { 2.0 } else { 1.0 },
        if selected {
            Color::new(171, 246, 255, 255)
        } else {
            Color::new(77, 150, 179, 200)
        },
    );
    centered(
        d,
        fonts,
        label,
        r,
        (r.3 * 0.42).clamp(11.0, 16.0),
        if selected {
            Color::new(230, 248, 252, 255)
        } else {
            Color::new(160, 178, 188, 255)
        },
    );
}

fn centered(
    d: &mut RaylibDrawHandle,
    fonts: &Fonts,
    text: &str,
    r: input::Rect,
    size: f32,
    color: Color,
) {
    let measured = fonts.ui.measure_text(text, size, 1.0);
    d.draw_text_ex(
        &fonts.ui,
        text,
        Vector2::new(
            r.0 + (r.2 - measured.x) * 0.5,
            r.1 + (r.3 - measured.y) * 0.5,
        ),
        size,
        1.0,
        color,
    );
}
