//! Settings window body: display-flag checkboxes (the same
//! `DisplaySettings` the F3 overlay flips), debug overlay and layout reset.

use crate::game::Game;
use crate::net::SharedWorld;
use crate::render::fonts::Fonts;
use crate::ui::chrome::{self, ButtonState, TXT, TXT_DIM, TXT_GOOD};
use crate::ui::panels::{self, SettingsItem};
use crate::ui::window::{point_in, Rect};
use raylib::prelude::*;

fn checkbox(d: &mut RaylibDrawHandle, r: Rect, on: bool) {
    let side = r.3.min(11.0).max(7.0);
    let b = (r.0, r.1 + (r.3 - side) * 0.5, side, side);
    chrome::draw_inset(d, b);
    if on {
        d.draw_line_ex(
            Vector2::new(b.0 + 2.5, b.1 + 6.0),
            Vector2::new(b.0 + 5.0, b.1 + 8.5),
            1.8,
            TXT_GOOD,
        );
        d.draw_line_ex(
            Vector2::new(b.0 + 5.0, b.1 + 8.5),
            Vector2::new(b.0 + 9.5, b.1 + 2.5),
            1.8,
            TXT_GOOD,
        );
    }
}

pub(crate) fn draw_prefs_body(
    d: &mut RaylibDrawHandle,
    game: &Game,
    world: &SharedWorld,
    fonts: &Fonts,
    body: Rect,
    mouse: (f32, f32),
) {
    for (i, item) in panels::settings_items().into_iter().enumerate() {
        let r = panels::settings_row_rect(body, i);
        let hover = point_in(r, mouse);
        if hover {
            d.draw_rectangle(
                r.0 as i32 - 2,
                r.1 as i32,
                r.2 as i32 + 4,
                r.3 as i32 + 1,
                Color::new(36, 56, 86, 150),
            );
        }
        match item {
            SettingsItem::Flag(f) => {
                checkbox(d, r, world.settings.get(f));
                chrome::label(
                    d,
                    fonts,
                    f.label(),
                    r.0 + 17.0,
                    r.1 + (r.3 - 10.0) * 0.5,
                    10.0,
                    if hover { Color::WHITE } else { TXT },
                );
            }
            SettingsItem::DebugOverlay => {
                checkbox(d, r, game.show_debug);
                chrome::label(
                    d,
                    fonts,
                    "Debug overlay",
                    r.0 + 17.0,
                    r.1 + (r.3 - 10.0) * 0.5,
                    10.0,
                    if hover { Color::WHITE } else { TXT },
                );
                chrome::label_right(d, fonts, "F2", r.0 + r.2 - 2.0, r.1 + 3.0, 8.0, TXT_DIM);
            }
            SettingsItem::WindowSkin => {
                chrome::draw_button(
                    d,
                    fonts,
                    (r.0, r.1, r.2, r.3),
                    game.preferences.window_skin.label(),
                    ButtonState {
                        hover,
                        active: matches!(
                            game.preferences.window_skin,
                            crate::ui::preferences::WindowSkinChoice::Legacy
                        ),
                        disabled: false,
                    },
                );
            }
            SettingsItem::MobileHud => {
                checkbox(d, r, game.preferences.mobile_hud);
                chrome::label(
                    d,
                    fonts,
                    "Touch controls / mobile preview",
                    r.0 + 17.0,
                    r.1 + (r.3 - 10.0) * 0.5,
                    10.0,
                    if hover { Color::WHITE } else { TXT },
                );
            }
            SettingsItem::RenderModes => {
                chrome::draw_button(
                    d,
                    fonts,
                    (r.0, r.1, r.2, r.3),
                    "Render modes  [F4]",
                    ButtonState {
                        hover,
                        active: game.show_render_settings,
                        disabled: false,
                    },
                );
            }
            SettingsItem::MinimapMapImage => {
                checkbox(d, r, game.preferences.minimap_map_image);
                chrome::label(
                    d,
                    fonts,
                    "Use game minimap image",
                    r.0 + 17.0,
                    r.1 + (r.3 - 10.0) * 0.5,
                    10.0,
                    if hover { Color::WHITE } else { TXT },
                );
            }
            SettingsItem::ToolbarBackground => {
                checkbox(d, r, game.preferences.toolbar_background);
                chrome::label(
                    d,
                    fonts,
                    "Use game menu button art",
                    r.0 + 17.0,
                    r.1 + (r.3 - 10.0) * 0.5,
                    10.0,
                    if hover { Color::WHITE } else { TXT },
                );
            }
            SettingsItem::AudioToggle(t) => {
                let (on, label) = match t {
                    panels::AudioToggle::Sfx => (game.preferences.audio.play_sfx, "Sound effects"),
                    panels::AudioToggle::Music => {
                        (game.preferences.audio.play_music, "Music")
                    }
                    panels::AudioToggle::CombatMusic => (
                        game.preferences.audio.play_combat_music,
                        "Combat music",
                    ),
                    panels::AudioToggle::Voice => {
                        (game.preferences.audio.play_voice, "Voice")
                    }
                };
                checkbox(d, r, on);
                chrome::label(
                    d,
                    fonts,
                    label,
                    r.0 + 17.0,
                    r.1 + (r.3 - 10.0) * 0.5,
                    10.0,
                    if hover { Color::WHITE } else { TXT },
                );
            }
            SettingsItem::AudioVolume(ch) => {
                let (value, label) = match ch {
                    panels::AudioChannel::Sound => {
                        (game.preferences.audio.sound_volume, "Sound volume")
                    }
                    panels::AudioChannel::Music => {
                        (game.preferences.audio.music_volume, "Music volume")
                    }
                    panels::AudioChannel::Voice => {
                        (game.preferences.audio.voice_volume, "Voice volume")
                    }
                };
                chrome::label(
                    d,
                    fonts,
                    label,
                    r.0,
                    r.1 + (r.3 - 10.0) * 0.5,
                    10.0,
                    if hover { Color::WHITE } else { TXT },
                );
                let (minus, plus) = panels::settings_vol_buttons(r);
                chrome::draw_button(
                    d,
                    fonts,
                    minus,
                    "-",
                    ButtonState {
                        hover: point_in(minus, mouse),
                        active: false,
                        disabled: value <= 0,
                    },
                );
                chrome::draw_button(
                    d,
                    fonts,
                    plus,
                    "+",
                    ButtonState {
                        hover: point_in(plus, mouse),
                        active: false,
                        disabled: value >= 100,
                    },
                );
                chrome::label_right(
                    d,
                    fonts,
                    &format!("{value}"),
                    minus.0 - 4.0,
                    r.1 + (r.3 - 10.0) * 0.5,
                    10.0,
                    TXT,
                );
            }
            SettingsItem::ResetLayout => {
                chrome::draw_button(
                    d,
                    fonts,
                    (r.0, r.1, r.2, r.3),
                    "Reset window layout",
                    ButtonState {
                        hover,
                        active: false,
                        disabled: false,
                    },
                );
            }
        }
    }
}
