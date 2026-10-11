//! Window pass: chrome + body for every open HUD window, bottom to top.

use super::{log, minimap, pet, prefs, quests, refinery, ship, social, user};
use crate::assets::AssetManager;
use crate::game::{Camera, Game};
use crate::net::SharedWorld;
use crate::render::fonts::Fonts;
use crate::render::sprites::TextureCache;
use crate::ui::chrome::{self, TXT_DIM};
use crate::ui::WindowId;
use raylib::prelude::*;

#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_windows(
    d: &mut RaylibDrawHandle,
    game: &Game,
    world: &SharedWorld,
    assets: &AssetManager,
    tex: &TextureCache,
    fonts: &Fonts,
    camera: &Camera,
    mouse: (f32, f32),
    tooltip: &mut Option<Vec<String>>,
) {
    let order = game.windows.draw_order();
    let top = order.last().copied();
    let window_skin = chrome::resolve_skin(assets, tex, game.preferences.window_skin);
    for id in order {
        let Some(win) = game.windows.get(id) else {
            continue;
        };
        let mut frame = win.clone();
        if id == WindowId::Minimap {
            frame.title = minimap::title_text(game, world, assets);
        }
        chrome::draw_window(
            d,
            fonts,
            &frame,
            top == Some(id),
            mouse,
            window_skin,
            game.preferences.window_skin,
        );
        if win.collapsed {
            continue;
        }
        let body = win.body_rect();
        let hero = world.world.hero.as_ref();
        match id {
            WindowId::Ship => match hero {
                Some(h) => ship::draw_ship_body(d, h, world, fonts, body, mouse),
                None => waiting(d, fonts, body),
            },
            WindowId::User => match hero {
                Some(h) => user::draw_user_body(d, h, world, fonts, body, mouse, tooltip),
                None => waiting(d, fonts, body),
            },
            WindowId::Minimap => {
                minimap::draw_minimap(d, game, world, assets, tex, camera, fonts, body, mouse)
            }
            WindowId::Quests => {
                quests::draw_quests_body(d, world, &game.ui, fonts, body, win.scroll, mouse)
            }
            WindowId::Pet => pet::draw_pet_body(d, world, fonts, body, win.scroll, mouse),
            WindowId::Log => log::draw_log_body(d, &world.log, fonts, body, win.scroll),
            WindowId::Settings => prefs::draw_prefs_body(d, game, world, fonts, body, mouse),
            WindowId::Group => social::draw_group(d, world, game, fonts, body, mouse),
            WindowId::Contacts => social::draw_contacts(d, world, game, fonts, body, mouse),
            WindowId::Chat => social::draw_chat(
                d,
                world,
                fonts,
                body,
                game.windows.get(id).map(|w| w.scroll).unwrap_or(0.0),
            ),
            WindowId::Refinery => refinery::draw_refinery_body(
                d, game, world, assets, tex, fonts, body, mouse, tooltip,
            ),
        }
    }
    // Modal refinery dialog + drag ghost stay on top of every window.
    refinery::draw_lab_dialog(d, game, world, tex, fonts, window_skin, mouse);
    refinery::draw_drag_ghost(d, game, tex, mouse);
}

fn waiting(d: &mut RaylibDrawHandle, fonts: &Fonts, body: crate::ui::window::Rect) {
    chrome::label(
        d,
        fonts,
        "Waiting for ship data...",
        body.0 + 8.0,
        body.1 + 8.0,
        10.0,
        TXT_DIM,
    );
}
