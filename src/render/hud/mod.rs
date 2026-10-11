//! HUD: minimap, vitals, log console, target info, tooltips —
//! the `map/hud` + minimal `mvc/gui` equivalent.

mod debug;
mod group;
mod killscreen;
mod log;
mod minimap;
mod mobile;
mod pet;
mod prefs;
mod quests;
mod refinery;
mod settings;
mod ship;
mod slotbar;
mod social;
mod target;
mod toasts;
mod user;
mod windows;

use crate::assets::AssetManager;
use crate::game::input;
use crate::game::{Camera, Game};
use crate::locale::LocaleCatalog;
use crate::net::SharedWorld;
use crate::render::fonts::Fonts;
use crate::render::sprites::TextureCache;
use raylib::prelude::*;

pub(crate) const PANEL_BG: Color = Color::new(8, 12, 20, 190);
pub(crate) const PANEL_EDGE: Color = Color::new(70, 95, 130, 200);
pub(crate) const TXT: Color = Color::new(205, 215, 228, 255);
pub(crate) const TXT_DIM: Color = Color::new(140, 152, 168, 255);

/// Whole HUD pass — cheap primitives only; everything offscreen-culled
/// upstream already.
#[allow(clippy::too_many_arguments)]
pub fn draw_hud(
    d: &mut RaylibDrawHandle,
    game: &Game,
    world: &SharedWorld,
    assets: &AssetManager,
    tex: &TextureCache,
    fonts: &Fonts,
    camera: &Camera,
) {
    let vw = d.get_screen_width() as f32;
    let vh = d.get_screen_height() as f32;
    let hud = game.hud_for_frame();
    let m = d.get_mouse_position();
    let mouse = (m.x, m.y);
    let mut tooltip: Option<Vec<String>> = None;
    let bar_skin = game
        .ui_sprites
        .iter()
        .find(|((src, res), _)| src == "ui" && res == "bar_background")
        .map(|(_, id)| *id);
    // The real window background — the `window` finisher composite for
    // `Stock`, the flat questWin/legacy exports otherwise. Shared by
    // floating windows, the ammo flyout and lab dialogs.
    let window_skin = crate::ui::chrome::resolve_skin(assets, tex, game.preferences.window_skin);

    if !game.preferences.mobile_hud {
        slotbar::draw_action_bar(
            d,
            world,
            fonts,
            &hud,
            &game.item_icons,
            tex,
            bar_skin,
            mouse,
            &mut tooltip,
        );
    }
    // Floating windows (ship, user, minimap, quests, pet, log, settings),
    // then the toolbar that opens them.
    windows::draw_windows(
        d,
        game,
        world,
        assets,
        tex,
        fonts,
        camera,
        mouse,
        &mut tooltip,
    );
    if !game.preferences.mobile_hud || !game.mobile_toolbar_collapsed {
        crate::ui::toolbar::draw_toolbar_config(
            d,
            fonts,
            &game.windows,
            &game.toolbar,
            game.toolbar_editing,
            mouse,
            &game.ui_sprites,
            tex,
            game.preferences.toolbar_background,
        );
    }
    let toolbar_buttons = game.toolbar.buttons(game.toolbar_editing);
    if input::point_in(
        crate::ui::toolbar::toolbar_controls_bounds(&toolbar_buttons, game.toolbar_editing),
        mouse,
    ) {
        if let Some(b) = crate::ui::toolbar::toolbar_hit_for(mouse, &toolbar_buttons) {
            tooltip = Some(vec![if game.toolbar_editing {
                format!("{} · drag to reorder · right-click to hide", b.tooltip())
            } else {
                b.tooltip().to_owned()
            }]);
        } else {
            tooltip = Some(vec![if game.toolbar_editing {
                "Toolbar edit mode"
            } else {
                "Edit toolbar"
            }
            .to_owned()]);
        }
    }
    slotbar::draw_flyout(
        d,
        world,
        fonts,
        &hud,
        &game.item_icons,
        tex,
        mouse,
        window_skin,
        game.preferences.window_skin,
    );
    if let (Some(sel), Some(rect)) = (game.selected, hud.target_panel) {
        if let Some(e) = world.world.entities.get(&sel) {
            let e_pos = world.pos(e.id);
            target::draw_target_panel(d, e, e_pos, world.hero_pos(), fonts, rect, world);
        }
    }
    group::draw_group_indicators(d, game, world, tex, fonts, camera, vw, vh);
    toasts::draw_toasts(d, &world.toasts, fonts, vw);
    debug::draw_hints(d, world, game, fonts, vw, vh);
    if game.show_debug {
        debug::draw_debug(d, game, world, assets, tex, fonts, camera, vw);
    }
    if let Some(msg) = &world.disconnected {
        if game.reconnect_note.is_none() {
            debug::draw_disconnected(d, fonts, msg, vw, vh);
        }
    }
    if let Some(lines) = &tooltip {
        if hud.flyout.is_none() {
            crate::ui::chrome::draw_tooltip(d, fonts, lines, mouse, vw, vh);
        }
    }
    if game.show_settings {
        settings::draw_settings(d, fonts, &world.settings, vw, vh);
    }
    if game.show_render_settings {
        settings::draw_render_settings(d, fonts, game.render_settings_draft.as_ref(), vw, vh);
    }
    if game.preferences.mobile_hud && !game.show_settings && !game.show_render_settings {
        mobile::draw(d, game, world, fonts, tex, mouse);
    }
    if let Some(ks) = &world.kill_screen {
        killscreen::draw_kill_screen(d, fonts, ks, &world.locale, vw, vh);
    }
}

/// Panel frame + title shared by the static HUD panels (same bevel as
/// the floating windows).
pub(crate) fn panel(d: &mut RaylibDrawHandle, fonts: &Fonts, title: &str, r: input::Rect) {
    crate::ui::chrome::draw_bevel_panel(d, r);
    if !title.is_empty() {
        d.draw_text_ex(
            &fonts.ui,
            title,
            Vector2::new(r.0 + 6.0, r.1 + 3.0),
            9.0,
            1.0,
            TXT_DIM,
        );
    }
}

/// Locale-grouped integer: `1234567` -> `1.234.567` for `tr`,
/// `1,234,567` for `en` (separators come from the loaded language;
/// `TextFormatter.roundInteger`).
pub(crate) fn fmt_thousands(v: f64, loc: &LocaleCatalog) -> String {
    loc.format_number(v)
}

pub(crate) fn fmt_num_i64(v: i64, loc: &LocaleCatalog) -> String {
    loc.format_i64(v)
}
