//! Action bar (laser/rocket slots), hover tooltips and the ammo flyout.

use super::{panel, PANEL_EDGE, TXT, TXT_DIM};
use crate::assets::SpriteId;
use crate::game::input::{self, HudLayout};
use crate::locale::LocaleCatalog;
use crate::net::world::MenuItem;
use crate::net::SharedWorld;
use crate::render::fonts::Fonts;
use crate::render::sprites::TextureCache;
use crate::ui::chrome::{self, ACCENT};
use crate::ui::window::{point_in, Rect};
use raylib::prelude::*;
use std::collections::HashMap;

pub(crate) fn item_icon_key(item: &MenuItem) -> &str {
    if item.icon_loot_id.is_empty() {
        &item.id
    } else {
        &item.icon_loot_id
    }
}

/// Localised item name (`items.items_<id>_fullname`, then shorter keys),
/// falling back to the raw id.
pub(crate) fn item_name(loc: &LocaleCatalog, id: &str) -> String {
    [
        format!("items.items_{id}_fullname"),
        format!("items.items_{id}"),
        format!("items.{id}"),
    ]
    .iter()
    .find_map(|k| loc.lookup(k))
    .map(str::to_owned)
    .unwrap_or_else(|| id.to_owned())
}

pub(crate) fn draw_icon(
    d: &mut RaylibDrawHandle,
    icons: &HashMap<String, SpriteId>,
    tex: &TextureCache,
    key: &str,
    r: Rect,
    tint: Color,
) -> bool {
    let Some(clip) = icons
        .get(key)
        .and_then(|&id| tex.get(id))
        .filter(|c| !c.frames.is_empty())
    else {
        return false;
    };
    let (bx0, by0, bx1, by1) = clip.bounds;
    let (bw, bh) = (bx1 - bx0, by1 - by0);
    if bw <= 0.0 || bh <= 0.0 {
        return false;
    }
    let s = ((r.2 - 4.0) / bw).min((r.3 - 4.0) / bh);
    let pos = (r.0 + (r.2 - bw * s) * 0.5, r.1 + (r.3 - bh * s) * 0.5);
    TextureCache::draw_frame_offset(d, clip, 0, pos, 0.0, (s, s), tint, (-bx0, -by0));
    true
}

fn slot_frame(d: &mut RaylibDrawHandle, r: Rect, hot: bool, hover: bool) {
    let (top, bot) = if hot {
        (Color::new(44, 100, 144, 240), Color::new(24, 62, 98, 240))
    } else if hover {
        (Color::new(34, 52, 78, 240), Color::new(20, 32, 50, 240))
    } else {
        (Color::new(20, 28, 42, 240), Color::new(12, 18, 28, 240))
    };
    d.draw_rectangle_gradient_v(r.0 as i32, r.1 as i32, r.2 as i32, r.3 as i32, top, bot);
    d.draw_rectangle_lines_ex(
        chrome::rl_rect(r),
        1.0,
        if hot {
            Color::new(140, 220, 255, 255)
        } else if hover {
            ACCENT
        } else {
            PANEL_EDGE
        },
    );
}

fn item_tooltip(world: &SharedWorld, item: &MenuItem) -> Vec<String> {
    let mut lines = vec![item_name(&world.locale, &item.id)];
    if lines[0] != item.id {
        lines.push(item.id.clone());
    }
    if item.counter_value > 0.0 {
        let mut s = format!("Count: {}", world.locale.format_number(item.counter_value));
        if item.max_counter_value > 0.0 {
            s.push_str(&format!(
                " / {}",
                world.locale.format_number(item.max_counter_value)
            ));
        }
        lines.push(s);
    }
    if item.blocked {
        lines.push("Blocked / cooling down".to_owned());
    } else if item.selected {
        lines.push("Selected".to_owned());
    }
    lines
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_action_bar(
    d: &mut RaylibDrawHandle,
    world: &SharedWorld,
    fonts: &Fonts,
    hud: &HudLayout,
    icons: &HashMap<String, SpriteId>,
    tex: &TextureCache,
    bar_skin: Option<SpriteId>,
    mouse: (f32, f32),
    tooltip: &mut Option<Vec<String>>,
) {
    panel(d, fonts, "", hud.action_bar);
    draw_bar_skin(d, tex, bar_skin, hud.action_bar);
    let hero_id = world.world.hero.as_ref().map(|h| h.user_id);
    let laser_on = hero_id.is_some_and(|id| world.world.laser_active.contains(&id));

    let selected = |prefix: &str| -> Option<&MenuItem> {
        world
            .menu
            .items
            .values()
            .find(|it| it.selected && it.id.starts_with(prefix))
    };
    let laser = selected("ammunition_laser_");
    let rocket = selected("ammunition_rocket_");

    let mut draw_slot = |r: Rect, label: &str, hot: bool, item: Option<&MenuItem>| {
        let hover = point_in(r, mouse);
        slot_frame(d, r, hot, hover);
        let icon_drawn =
            item.is_some_and(|it| draw_icon(d, icons, tex, item_icon_key(it), r, Color::WHITE));
        if !icon_drawn {
            let m = fonts.ui.measure_text(label, 10.0, 1.0);
            chrome::label(
                d,
                fonts,
                label,
                r.0 + (r.2 - m.x) * 0.5,
                r.1 + r.3 * 0.5 - 5.0,
                10.0,
                if hot { Color::WHITE } else { TXT },
            );
        }
        if let Some(it) = item {
            if it.counter_value > 0.0 {
                counter_badge(d, fonts, r, it.counter_value, false);
            }
            if hover {
                *tooltip = Some(item_tooltip(world, it));
            }
        } else if hover {
            *tooltip = Some(vec![label.to_owned()]);
        }
    };
    draw_slot(hud.laser_slot, "LZR", laser_on, laser);
    draw_slot(hud.rocket_slot, "RKT", false, rocket);
}

fn counter_badge(d: &mut RaylibDrawHandle, fonts: &Fonts, r: Rect, value: f64, dim: bool) {
    let c = input::fmt_compact(value);
    let cm = fonts.ui.measure_text(&c, 8.0, 1.0);
    d.draw_rectangle(
        (r.0 + r.2 - cm.x - 5.0) as i32,
        (r.1 + r.3 - 11.0) as i32,
        (cm.x + 4.0) as i32,
        10,
        Color::new(6, 9, 14, 200),
    );
    chrome::label(
        d,
        fonts,
        &c,
        r.0 + r.2 - cm.x - 2.0,
        r.1 + r.3 - 10.0,
        8.0,
        if dim { TXT_DIM } else { TXT },
    );
}

/// Draw the stock `ui/bar_background` strip in its native 62px repeats over
/// the slot-bar chrome. The procedural panel remains behind it for a clean
/// fallback while the SWF clip is still being rasterized.
fn draw_bar_skin(d: &mut RaylibDrawHandle, tex: &TextureCache, id: Option<SpriteId>, r: Rect) {
    let Some(clip) = id.and_then(|id| tex.get(id)) else {
        return;
    };
    let Some(frame) = clip.frames.first() else {
        return;
    };
    let (tw, th) = (frame.tex.width as f32, frame.tex.height as f32);
    if tw <= 0.0 || th <= 0.0 {
        return;
    }
    let top = (th.min(r.3 - 2.0)).max(1.0);
    let mut x = r.0 + 2.0;
    let right = r.0 + r.2 - 2.0;
    while x < right {
        let width = tw.min(right - x);
        d.draw_texture_pro(
            &frame.tex,
            Rectangle::new(0.0, 0.0, width, th),
            Rectangle::new(x, r.1 + 2.0, width, top),
            Vector2::zero(),
            0.0,
            Color::new(255, 255, 255, 190),
        );
        x += width;
    }
}

/// Ammo flyout panel (right-click on a slot). Uses the real window
/// background like every other menu (`chrome::draw_panel`).
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_flyout(
    d: &mut RaylibDrawHandle,
    world: &SharedWorld,
    fonts: &Fonts,
    hud: &HudLayout,
    icons: &HashMap<String, SpriteId>,
    tex: &TextureCache,
    mouse: (f32, f32),
    window_skin: chrome::Skin,
    skin_choice: crate::ui::preferences::WindowSkinChoice,
) {
    let Some(f) = &hud.flyout else {
        return;
    };
    chrome::draw_panel(d, f.panel, window_skin, skin_choice);
    for (id, r) in &f.rows {
        let item = world.menu.items.get(id);
        let hover = point_in(*r, mouse);
        let hot = item.is_some_and(|it| it.selected);
        if hot {
            d.draw_rectangle(
                r.0 as i32,
                r.1 as i32,
                r.2 as i32,
                r.3 as i32,
                Color::new(24, 80, 120, 220),
            );
        } else if hover {
            d.draw_rectangle(
                r.0 as i32,
                r.1 as i32,
                r.2 as i32,
                r.3 as i32,
                Color::new(40, 66, 100, 220),
            );
        }
        let icon_r = (r.0 + 2.0, r.1 + 1.0, r.3 - 2.0, r.3 - 2.0);
        if let Some(it) = item {
            let drawn = draw_icon(d, icons, tex, item_icon_key(it), icon_r, Color::WHITE);
            if !drawn {
                d.draw_rectangle_lines_ex(chrome::rl_rect(icon_r), 1.0, PANEL_EDGE);
            }
        }
        let count = item
            .filter(|it| it.counter_value > 0.0)
            .map(|it| input::fmt_compact(it.counter_value))
            .unwrap_or_default();
        let cw = fonts.ui.measure_text(&count, 9.0, 1.0).x;
        let name = chrome::fit_text(
            fonts,
            &item_name(&world.locale, id),
            10.0,
            r.2 - icon_r.2 - cw - 14.0,
        );
        chrome::label(
            d,
            fonts,
            &name,
            icon_r.0 + icon_r.2 + 5.0,
            r.1 + 7.0,
            10.0,
            if hot { Color::WHITE } else { TXT },
        );
        chrome::label_right(d, fonts, &count, r.0 + r.2 - 4.0, r.1 + 8.0, 9.0, TXT_DIM);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn item_name_falls_back_to_id() {
        let loc = LocaleCatalog::new("en");
        assert_eq!(
            item_name(&loc, "ammunition_laser_lcb-10"),
            "ammunition_laser_lcb-10"
        );
    }
}
