//! Refinery ("Refining"/"Upgrade") window body — `RefinementManager`
//! in the stock client.
//!
//! All chrome bitmaps (`sourceSlot`/`targetSlot`/`targetIcon`/
//! `labelSlot`, ore `resKey`s, `icon_*`/`item_*`) are symbols in the
//! `refinement` SWF (resources.xml file id `refinement`); `ore_tree`
//! ships in `ui`. `Game::update` requests them while the window is
//! expanded and caches the `SpriteId`s in `game.ui_sprites`.

use super::{fmt_num_i64, TXT, TXT_DIM};
use crate::assets::gamexml::{RefineryItem, RefineryOre};
use crate::assets::AssetManager;
use crate::game::Game;
use crate::net::refinery::{self, LabItemKind};
use crate::net::SharedWorld;
use crate::render::fonts::{measure_text_fallback, Fonts};
use crate::render::sprites::{GpuClip, TextureCache};
use crate::ui::chrome::{self, ButtonState};
use crate::ui::panels::{self, RefineryLayout};
use crate::ui::window::point_in;
use crate::ui::{LabDialog, RefineryTab, WindowId};
use raylib::prelude::*;

/// Target highlight colour (`TargetRefinementModule.HIGHLIGHT__COLOR`,
/// 0xFFE100 at 50% alpha).
const HIGHLIGHT: Color = Color::new(255, 225, 0, 128);
/// Drag-ghost tint.
const GHOST: Color = Color::new(255, 255, 255, 215);

/// `resolve(key)` with an english fallback when the catalog lacks it.
fn tr(world: &SharedWorld, key: &str, fallback: &str) -> String {
    let s = world.locale.resolve(key);
    if s == key {
        fallback.to_owned()
    } else {
        s
    }
}

fn clip<'a>(game: &Game, tex: &'a TextureCache, src: &str, res: &str) -> Option<&'a GpuClip> {
    game.ui_sprites
        .get(&(src.to_owned(), res.to_owned()))
        .and_then(|id| tex.get(*id))
}

/// Stretch a clip's first frame onto `r`.
fn blit(d: &mut RaylibDrawHandle, clip: &GpuClip, r: crate::ui::window::Rect, tint: Color) {
    let Some(f) = clip.frames.first() else {
        return;
    };
    let (tw, th) = (f.tex.width as f32, f.tex.height as f32);
    if tw <= 0.0 || th <= 0.0 || r.2 <= 0.0 || r.3 <= 0.0 {
        return;
    }
    d.draw_texture_pro(
        &f.tex,
        Rectangle::new(0.0, 0.0, tw, th),
        chrome::rl_rect(r),
        Vector2::zero(),
        0.0,
        tint,
    );
}

/// Ore bitmap centred on the slot at native size (clamped to fit), like
/// `bg.w/2 - img.w/2` in the module `init()`s.
fn blit_centered(
    d: &mut RaylibDrawHandle,
    clip: &GpuClip,
    slot: (f32, f32, f32, f32),
    tint: Color,
) {
    let Some(f) = clip.frames.first() else {
        return;
    };
    let (tw, th) = (f.tex.width as f32, f.tex.height as f32);
    if tw <= 0.0 || th <= 0.0 {
        return;
    }
    let s = (slot.2 / tw).min(slot.3 / th).min(1.0);
    let (w, h) = (tw * s, th * s);
    let r = (
        slot.0 + (slot.2 - w) * 0.5,
        slot.1 + (slot.3 - h) * 0.5,
        w,
        h,
    );
    blit(d, clip, r, tint);
}

/// `labelSlot` strip + centred caption — the name label above a slot and
/// the amount label below it (`RefinementLabel`).
fn slot_label(
    d: &mut RaylibDrawHandle,
    game: &Game,
    tex: &TextureCache,
    fonts: &Fonts,
    slot: (f32, f32, f32, f32),
    y: f32,
    text: &str,
) {
    let strip = (slot.0, y, slot.2, panels::REFINERY_LABEL_H);
    match clip(game, tex, "refinement", "labelSlot") {
        Some(c) => blit(d, c, strip, Color::WHITE),
        None => d.draw_rectangle(
            strip.0 as i32,
            strip.1 as i32,
            strip.2 as i32,
            strip.3 as i32,
            Color::new(10, 16, 28, 190),
        ),
    }
    let t = chrome::fit_text(fonts, text, 8.0, strip.2 - 4.0);
    let tw = measure_text_fallback(&fonts.ui, fonts.symbols.as_ref(), &t, 8.0, 1.0).x;
    chrome::label(
        d,
        fonts,
        &t,
        strip.0 + (strip.2 - tw) * 0.5,
        strip.1 + 3.0,
        8.0,
        TXT,
    );
}

/// Tooltip for one `<ore>` row — comma-separated locale keys joined by
/// newlines, then `%COUNT_N%`/`%ORE_N%` recipe substitution
/// (`createRefinementContainers`).
fn ore_tooltip(world: &SharedWorld, game: &Game, ore: &RefineryOre) -> Option<Vec<String>> {
    if ore.tooltip_key.is_empty() {
        return None;
    }
    let mut text = ore
        .tooltip_key
        .split(',')
        .map(|k| world.locale.resolve(k.trim()))
        .collect::<Vec<_>>()
        .join("\n");
    let recipe = game
        .refinery
        .as_ref()
        .and_then(|d| d.ores.get(&ore.ore_type))
        .map(|o| o.refiner.as_slice())
        .unwrap_or(&[]);
    // refiners[0] -> %COUNT_1%/%ORE_1%, [1] -> %COUNT_2%/%ORE_2%,
    // [2] -> %COUNT%/%ORE% (the stock indexing quirk).
    for (i, (input, count)) in recipe.iter().enumerate() {
        let name = game
            .refinery
            .as_ref()
            .and_then(|d| d.ores.get(input))
            .map(|o| world.locale.resolve(&o.language_key))
            .unwrap_or_default();
        let (ck, ok) = match i {
            0 => ("%COUNT_1%", "%ORE_1%"),
            1 => ("%COUNT_2%", "%ORE_2%"),
            _ => ("%COUNT%", "%ORE%"),
        };
        text = text.replace(ck, &count.to_string()).replace(ok, &name);
    }
    Some(text.lines().map(|l| l.to_owned()).collect())
}

/// One `sourceSlot` module: slot background, centred ore bitmap, name
/// label above, amount label below.
#[allow(clippy::too_many_arguments)]
fn draw_ore_slot(
    d: &mut RaylibDrawHandle,
    game: &Game,
    world: &SharedWorld,
    tex: &TextureCache,
    fonts: &Fonts,
    slot: (f32, f32, f32, f32),
    ore_type: i32,
    mouse: (f32, f32),
    tooltip: &mut Option<Vec<String>>,
    tip: Option<Vec<String>>,
) {
    match clip(game, tex, "refinement", "sourceSlot") {
        Some(c) => blit(d, c, slot, Color::WHITE),
        None => {
            d.draw_rectangle(
                slot.0 as i32,
                slot.1 as i32,
                slot.2 as i32,
                slot.3 as i32,
                Color::new(14, 22, 36, 220),
            );
            d.draw_rectangle_lines_ex(chrome::rl_rect(slot), 1.0, super::PANEL_EDGE);
        }
    }
    let name = game
        .refinery
        .as_ref()
        .and_then(|data| data.ores.get(&ore_type))
        .map(|o| (world.locale.resolve(&o.language_key), o.res_key.clone()))
        .unwrap_or_else(|| (ore_type.to_string(), String::new()));
    if !name.1.is_empty() {
        if let Some(c) = clip(game, tex, "refinement", &name.1) {
            blit_centered(d, c, slot, Color::WHITE);
        }
    }
    slot_label(d, game, tex, fonts, slot, slot.1 - 18.0, &name.0);
    let count = world.extras.ores.get(&ore_type).copied().unwrap_or(0.0);
    slot_label(
        d,
        game,
        tex,
        fonts,
        slot,
        slot.1 + slot.3 + 4.0,
        &fmt_num_i64(count as i64, &world.locale),
    );
    if tip.is_some() && point_in(slot, mouse) {
        *tooltip = tip;
    }
}

/// `RefinementManager.showRefinement` — static ingredient row on top,
/// producible ores with "Refine" buttons, `ore_tree` behind.
#[allow(clippy::too_many_arguments)]
fn draw_refining_tab(
    d: &mut RaylibDrawHandle,
    game: &Game,
    world: &SharedWorld,
    tex: &TextureCache,
    fonts: &Fonts,
    l: &RefineryLayout,
    mouse: (f32, f32),
    tooltip: &mut Option<Vec<String>>,
) {
    if let Some(c) = clip(game, tex, "ui", "ore_tree") {
        // Native size at the tree anchor (drawn under the slots).
        if let Some(f) = c.frames.first() {
            blit(
                d,
                c,
                (l.tree.0, l.tree.1, f.tex.width as f32, f.tex.height as f32),
                Color::WHITE,
            );
        }
    }
    let Some(data) = &game.refinery else {
        return;
    };
    let w = &data.window;
    for (i, slot) in l.refine_static.iter().enumerate() {
        let Some(ore) = w.refine_static.get(i) else {
            continue;
        };
        let tip = ore_tooltip(world, game, ore);
        draw_ore_slot(
            d,
            game,
            world,
            tex,
            fonts,
            *slot,
            ore.ore_type,
            mouse,
            tooltip,
            tip,
        );
    }
    for (i, slot) in l.refine_ores.iter().enumerate() {
        let Some(ore) = w.refine_ores.get(i) else {
            continue;
        };
        let tip = ore_tooltip(world, game, ore);
        draw_ore_slot(
            d,
            game,
            world,
            tex,
            fonts,
            *slot,
            ore.ore_type,
            mouse,
            tooltip,
            tip,
        );
        if ore.button {
            if let Some(r) = l.refine_buttons.get(i) {
                let recipe = data
                    .ores
                    .get(&ore.ore_type)
                    .map(|o| o.refiner.as_slice())
                    .unwrap_or(&[]);
                let max = refinery::refine_max(recipe, &world.extras.ores);
                chrome::draw_button(
                    d,
                    fonts,
                    *r,
                    &tr(world, "lab_btn_refine", "Refine"),
                    ButtonState {
                        hover: point_in(*r, mouse),
                        disabled: max < 1,
                        ..Default::default()
                    },
                );
            }
        }
    }
}

/// `RefinementManager.showUpdate` — static ore row, `labor_intro`, the
/// draggable source ores and the four target item slots.
#[allow(clippy::too_many_arguments)]
fn draw_upgrade_tab(
    d: &mut RaylibDrawHandle,
    game: &Game,
    world: &SharedWorld,
    tex: &TextureCache,
    fonts: &Fonts,
    l: &RefineryLayout,
    mouse: (f32, f32),
    tooltip: &mut Option<Vec<String>>,
) {
    let Some(data) = &game.refinery else {
        return;
    };
    let w = &data.window;
    chrome::label(
        d,
        fonts,
        &tr(world, "lab_title", "Laboratory"),
        l.upd_title.0,
        l.upd_title.1 + 2.0,
        10.0,
        TXT,
    );
    for (i, slot) in l.upd_static.iter().enumerate() {
        let Some(ore) = w.update_static.get(i) else {
            continue;
        };
        let tip = ore_tooltip(world, game, ore);
        draw_ore_slot(
            d,
            game,
            world,
            tex,
            fonts,
            *slot,
            ore.ore_type,
            mouse,
            tooltip,
            tip,
        );
    }
    // `labor_intro` wraps over ~2 lines.
    let intro = tr(
        world,
        "labor_intro",
        "Drag an ore onto an item to upgrade it.",
    );
    chrome::label(
        d,
        fonts,
        &chrome::fit_text(fonts, &intro, 9.0, l.upd_intro.2),
        l.upd_intro.0,
        l.upd_intro.1 + 2.0,
        9.0,
        TXT_DIM,
    );

    let dragging = game.ui.refinery_drag;
    for (i, slot) in l.upd_sources.iter().enumerate() {
        let Some(ore) = w.update_ores.get(i) else {
            continue;
        };
        let tip = ore_tooltip(world, game, ore);
        draw_ore_slot(
            d,
            game,
            world,
            tex,
            fonts,
            *slot,
            ore.ore_type,
            mouse,
            tooltip,
            tip,
        );
        // Corner effect icon (`iconResKey` — `createOreIcon`).
        if !ore.icon_res_key.is_empty() {
            if let Some(c) = clip(game, tex, "refinement", &ore.icon_res_key) {
                let s = (slot.0 + 4.0, slot.1 + 4.0, 18.0, 18.0);
                blit(d, c, s, Color::WHITE);
            }
        }
        if point_in(*slot, mouse) && dragging.is_none() {
            d.draw_rectangle_lines_ex(chrome::rl_rect(*slot), 1.0, TXT_DIM);
        }
    }

    // Target item slots (`TargetRefinementModule`): targetSlot bg, item
    // bitmap centred, `targetIcon` bottom-left holding the applied ore
    // icon; label strip shows the `ammountKey` template.
    for (i, slot) in l.upd_targets.iter().enumerate() {
        let Some(item) = w.update_items.get(i) else {
            continue;
        };
        draw_target_slot(d, game, world, tex, fonts, *slot, item, dragging, mouse);
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_target_slot(
    d: &mut RaylibDrawHandle,
    game: &Game,
    world: &SharedWorld,
    tex: &TextureCache,
    fonts: &Fonts,
    slot: (f32, f32, f32, f32),
    item: &RefineryItem,
    dragging: Option<i32>,
    mouse: (f32, f32),
) {
    match clip(game, tex, "refinement", "targetSlot") {
        Some(c) => blit(d, c, slot, Color::WHITE),
        None => {
            d.draw_rectangle(
                slot.0 as i32,
                slot.1 as i32,
                slot.2 as i32,
                slot.3 as i32,
                Color::new(14, 22, 36, 220),
            );
            d.draw_rectangle_lines_ex(chrome::rl_rect(slot), 1.0, super::PANEL_EDGE);
        }
    }
    if let Some(c) = clip(game, tex, "refinement", &item.res_key) {
        blit_centered(d, c, slot, Color::WHITE);
    }
    // `targetIcon` corner at (4, bg.h - icon.h - 4); the applied ore icon
    // sits inside it.
    let icon = (slot.0 + 4.0, slot.1 + slot.3 - 22.0, 18.0, 18.0);
    if let Some(c) = clip(game, tex, "refinement", "targetIcon") {
        blit(d, c, icon, Color::WHITE);
    } else {
        d.draw_rectangle(
            icon.0 as i32,
            icon.1 as i32,
            icon.2 as i32,
            icon.3 as i32,
            Color::new(8, 12, 22, 220),
        );
        d.draw_rectangle_lines_ex(chrome::rl_rect(icon), 1.0, super::PANEL_EDGE);
    }
    let kind = LabItemKind::of_identifier(&item.identifier);
    let state = kind.map(|k| world.lab.items[k.index()]).unwrap_or_default();
    // The applied icon is the SOURCE ore's `iconResKey` (`updateItem` →
    // `createOreIcon` on the source module).
    let applied_res = state.ore_type.and_then(|t| {
        game.refinery
            .as_ref()?
            .window
            .update_ores
            .iter()
            .find(|o| o.ore_type == t)
            .map(|o| o.icon_res_key.clone())
    });
    if let Some(res) = applied_res {
        if let Some(c) = clip(game, tex, "refinement", &res) {
            blit(
                d,
                c,
                (icon.0 + 1.0, icon.1 + 1.0, icon.2 - 2.0, icon.3 - 2.0),
                Color::WHITE,
            );
        }
    }
    // Valid-drop highlight while a source ore is dragged.
    if let (Some(ore), Some(k)) = (dragging, kind) {
        if refinery::valid_upgrade(ore, k, state.ore_type, state.count) {
            d.draw_rectangle_lines_ex(
                Rectangle::new(icon.0, icon.1, icon.2, icon.3),
                2.0,
                HIGHLIGHT,
            );
        }
    }
    if point_in(slot, mouse) {
        d.draw_rectangle_lines_ex(chrome::rl_rect(slot), 1.0, TXT_DIM);
    }
    let name = tr(world, &item.language_key, &item.identifier);
    slot_label(d, game, tex, fonts, slot, slot.1 - 18.0, &name);
    // `setAmmount`: `ammountKey` template with %COUNT% substituted.
    let label = world
        .locale
        .resolve(&item.amount_key)
        .replace("%COUNT%", &fmt_num_i64(state.count as i64, &world.locale));
    slot_label(d, game, tex, fonts, slot, slot.1 + slot.3 + 4.0, &label);
}

/// The modal refinery dialog (`lab_dialog`) — skinned panel with the
/// amount chips and OK/Cancel. Drawn after every window so it stays on
/// top of the refinery body.
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_lab_dialog(
    d: &mut RaylibDrawHandle,
    game: &Game,
    world: &SharedWorld,
    _tex: &TextureCache,
    fonts: &Fonts,
    window_skin: chrome::Skin,
    mouse: (f32, f32),
) {
    let Some(dlg) = &game.ui.lab_dialog else {
        return;
    };
    let anchor = game
        .windows
        .get(WindowId::Refinery)
        .map(|w| w.rect)
        .unwrap_or((0.0, 0.0, 0.0, 0.0));
    let (title, hint, ok_label, opts, selected) = match dlg {
        LabDialog::Refine {
            ore_type,
            options,
            amount,
        } => {
            let name = game
                .refinery
                .as_ref()
                .and_then(|d| d.ores.get(ore_type))
                .map(|o| world.locale.resolve(&o.language_key))
                .unwrap_or_else(|| ore_type.to_string());
            (
                name,
                tr(world, "lab_refine_question", "How much should be refined?"),
                tr(world, "lab_btn_refine", "Refine"),
                options.clone(),
                *amount,
            )
        }
        LabDialog::UpgradeAmount {
            options, amount, ..
        } => (
            tr(world, "lab_count", "Amount"),
            tr(world, "lab_count", "Amount"),
            tr(world, "lab_update", "Update"),
            options.clone(),
            *amount,
        ),
        LabDialog::UpgradeConfirm { .. } => (
            tr(world, "lab_alert", "Replace the current ore upgrade?"),
            String::new(),
            tr(world, "lab_update", "Update"),
            Vec::new(),
            0,
        ),
    };
    let l = panels::lab_dialog_layout(anchor, &opts, dlg.panel_size());
    chrome::draw_panel(d, l.panel, window_skin, game.preferences.window_skin);
    let mut y = l.header.1;
    chrome::label(d, fonts, &title, l.header.0, y, 10.0, TXT);
    y += 14.0;
    if !hint.is_empty() {
        chrome::label(
            d,
            fonts,
            &chrome::fit_text(fonts, &hint, 9.0, l.header.2),
            l.header.0,
            y,
            9.0,
            TXT_DIM,
        );
    }
    for (v, r) in &l.options {
        chrome::draw_button(
            d,
            fonts,
            *r,
            &fmt_num_i64(*v as i64, &world.locale),
            ButtonState {
                hover: point_in(*r, mouse),
                active: *v == selected,
                ..Default::default()
            },
        );
    }
    chrome::draw_button(
        d,
        fonts,
        l.ok,
        &ok_label,
        ButtonState {
            hover: point_in(l.ok, mouse),
            ..Default::default()
        },
    );
    chrome::draw_button(
        d,
        fonts,
        l.cancel,
        &tr(world, "lab_cancel", "Cancel"),
        ButtonState {
            hover: point_in(l.cancel, mouse),
            ..Default::default()
        },
    );
}

/// Drag ghost — the dragged source ore's `iconResKey` bitmap follows the
/// cursor (`SourceRefinementModule.createIcon`, startDrag).
pub(crate) fn draw_drag_ghost(
    d: &mut RaylibDrawHandle,
    game: &Game,
    tex: &TextureCache,
    mouse: (f32, f32),
) {
    let Some(ore_type) = game.ui.refinery_drag else {
        return;
    };
    let icon = game
        .refinery
        .as_ref()
        .and_then(|d| {
            d.window
                .update_ores
                .iter()
                .find(|o| o.ore_type == ore_type)
                .map(|o| o.icon_res_key.clone())
        })
        .or_else(|| {
            game.refinery
                .as_ref()
                .and_then(|d| d.ores.get(&ore_type).map(|o| o.res_key.clone()))
        });
    let Some(res) = icon else { return };
    let Some(c) = clip(game, tex, "refinement", &res) else {
        return;
    };
    let Some(f) = c.frames.first() else { return };
    let (tw, th) = (f.tex.width as f32, f.tex.height as f32);
    blit(
        d,
        c,
        (mouse.0 - tw * 0.5, mouse.1 - th * 0.5, tw, th),
        GHOST,
    );
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_refinery_body(
    d: &mut RaylibDrawHandle,
    game: &Game,
    world: &SharedWorld,
    _assets: &AssetManager,
    tex: &TextureCache,
    fonts: &Fonts,
    body: crate::ui::window::Rect,
    mouse: (f32, f32),
    tooltip: &mut Option<Vec<String>>,
) {
    let Some(data) = &game.refinery else {
        chrome::label(
            d,
            fonts,
            "Waiting for ship data...",
            body.0 + 8.0,
            body.1 + 8.0,
            10.0,
            TXT_DIM,
        );
        return;
    };
    let w = &data.window;
    let l = panels::refinery_layout(
        body,
        w.refine_static.len(),
        w.refine_ores.len(),
        w.update_static.len(),
        w.update_ores.len(),
        w.update_items.len(),
    );
    let tab = game.ui.refinery_tab;
    chrome::draw_button(
        d,
        fonts,
        l.tab_refining,
        &tr(world, "lab_btn_refinement", "Refining"),
        ButtonState {
            hover: point_in(l.tab_refining, mouse),
            active: tab == RefineryTab::Refining,
            ..Default::default()
        },
    );
    chrome::draw_button(
        d,
        fonts,
        l.tab_upgrade,
        &tr(world, "lab_btn_update", "Upgrade"),
        ButtonState {
            hover: point_in(l.tab_upgrade, mouse),
            active: tab == RefineryTab::Upgrade,
            ..Default::default()
        },
    );
    // The stock 1px separator is `lineStyle(1, 0x7E93BE)` — 8289918.
    d.draw_line(
        l.separator.0 as i32,
        l.separator.1 as i32,
        (l.separator.0 + l.separator.2) as i32,
        l.separator.1 as i32,
        Color::new(126, 147, 190, 255),
    );
    match tab {
        RefineryTab::Refining => draw_refining_tab(d, game, world, tex, fonts, &l, mouse, tooltip),
        RefineryTab::Upgrade => draw_upgrade_tab(d, game, world, tex, fonts, &l, mouse, tooltip),
    }
}
