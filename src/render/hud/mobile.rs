//! Touch controls shown in mobile mode and desktop mobile-HUD preview.

use crate::game::{input, Game};
use crate::net::SharedWorld;
use crate::render::fonts::Fonts;
use crate::render::sprites::TextureCache;
use raylib::prelude::*;

fn button(
    d: &mut RaylibDrawHandle,
    fonts: &Fonts,
    r: input::Rect,
    label: &str,
    accent: bool,
    large: bool,
) {
    let c = Vector2::new(r.0 + r.2 * 0.5, r.1 + r.3 * 0.5);
    let radius = r.2.min(r.3) * 0.5;
    let fill = if accent {
        Color::new(14, 72, 113, 218)
    } else {
        Color::new(8, 20, 38, 205)
    };
    d.draw_circle_v(c, radius, fill);
    d.draw_circle_lines_v(c, radius, Color::new(111, 184, 237, 245));
    d.draw_circle_lines_v(c, radius - 3.0, Color::new(57, 111, 157, 220));
    d.draw_circle_v(
        Vector2::new(c.x - radius * 0.22, c.y - radius * 0.25),
        radius * 0.16,
        Color::new(190, 226, 251, 35),
    );
    let size = if large {
        (radius * 0.48).clamp(18.0, 38.0)
    } else {
        (radius * 0.68).clamp(14.0, 28.0)
    };
    let measure = fonts.ui.measure_text(label, size, 1.0);
    d.draw_text_ex(
        &fonts.ui,
        label,
        Vector2::new(c.x - measure.x * 0.5, c.y - measure.y * 0.5),
        size,
        1.0,
        Color::new(221, 239, 255, 245),
    );
}

pub(crate) fn draw(
    d: &mut RaylibDrawHandle,
    game: &Game,
    world: &SharedWorld,
    fonts: &Fonts,
    tex: &TextureCache,
    mouse: (f32, f32),
) {
    let vw = d.get_screen_width() as f32;
    let vh = d.get_screen_height() as f32;
    let layout = input::mobile_hud_layout(vw, vh);
    let toggle = input::mobile_toolbar_toggle_rect(vw, game.mobile_toolbar_collapsed);
    crate::ui::chrome::draw_button(
        d,
        fonts,
        toggle,
        if game.mobile_toolbar_collapsed {
            ">>"
        } else {
            "<<"
        },
        crate::ui::chrome::ButtonState {
            hover: input::point_in(toggle, mouse),
            active: false,
            disabled: false,
        },
    );
    let stick = layout.move_stick;
    let center = Vector2::new(stick.0, stick.1);
    draw_pad(d, center, stick.2, Color::new(76, 161, 226, 176));
    let delta = if game.mobile_joystick_active {
        let pointer = game.mobile_joystick_position.unwrap_or((stick.0, stick.1));
        let dx = pointer.0 - stick.0;
        let dy = pointer.1 - stick.1;
        let len = dx.hypot(dy).max(1.0);
        let max = stick.2 * 0.66;
        let scale = (max / len).min(1.0);
        (dx * scale, dy * scale)
    } else {
        (0.0, 0.0)
    };
    let knob = Vector2::new(center.x + delta.0, center.y + delta.1);
    d.draw_circle_v(knob, stick.2 * 0.34, Color::new(26, 99, 164, 225));
    d.draw_circle_lines_v(knob, stick.2 * 0.34, Color::new(153, 211, 248, 250));
    d.draw_circle_lines_v(knob, stick.2 * 0.29, Color::new(75, 156, 213, 210));
    d.draw_text_ex(
        &fonts.ui,
        "MOVE",
        Vector2::new(center.x - 18.0, center.y + stick.2 * 0.73),
        13.0,
        1.0,
        Color::new(189, 219, 242, 220),
    );

    let action_center = Vector2::new(
        layout.laser.0 + layout.laser.2 * 0.5,
        layout.laser.1 + layout.laser.3 * 0.5,
    );
    let action_radius = stick.2 * 0.60;
    draw_pad(
        d,
        action_center,
        action_radius,
        Color::new(231, 126, 69, 185),
    );
    let selected = game.selected.is_some();
    button(d, fonts, layout.target, "TGT", selected, false);
    button(d, fonts, layout.laser, "FIRE", true, true);
    button(d, fonts, layout.rocket, "RKT", false, false);
    button(d, fonts, layout.jump, "JUMP", false, false);
    button(d, fonts, layout.pet, "PET", false, false);
    draw_laser_ammo_icon(d, game, world, tex, layout.laser);

    draw_vitals(d, fonts, world, vw);
    draw_radar(d, game, world, layout.radar);
}

fn draw_pad(d: &mut RaylibDrawHandle, c: Vector2, r: f32, edge: Color) {
    d.draw_circle_v(c, r, Color::new(5, 18, 34, 72));
    d.draw_circle_lines_v(c, r, Color::new(edge.r, edge.g, edge.b, 95));
    d.draw_circle_lines_v(c, r * 0.72, Color::new(edge.r, edge.g, edge.b, 62));
    d.draw_circle_lines_v(c, r * 0.42, Color::new(edge.r, edge.g, edge.b, 42));
    for i in 0..12 {
        let a = i as f32 * std::f32::consts::TAU / 12.0;
        let (s, c0) = a.sin_cos();
        let inner = r * 0.90;
        let outer = if i % 3 == 0 { r * 0.99 } else { r * 0.94 };
        d.draw_line_ex(
            Vector2::new(c.x + c0 * inner, c.y + s * inner),
            Vector2::new(c.x + c0 * outer, c.y + s * outer),
            if i % 3 == 0 { 2.0 } else { 1.0 },
            Color::new(edge.r, edge.g, edge.b, 115),
        );
    }
}

fn draw_laser_ammo_icon(
    d: &mut RaylibDrawHandle,
    game: &Game,
    world: &SharedWorld,
    tex: &TextureCache,
    r: input::Rect,
) {
    let Some(item) = world
        .menu
        .items
        .values()
        .find(|item| item.selected && item.id.starts_with("ammunition_laser_"))
    else {
        return;
    };
    let key = crate::render::hud::slotbar::item_icon_key(item);
    let inset = (r.2 * 0.64).max(28.0);
    let icon = (r.0 + (r.2 - inset) * 0.5, r.1 - inset * 0.5, inset, inset);
    let _ =
        crate::render::hud::slotbar::draw_icon(d, &game.item_icons, tex, key, icon, Color::WHITE);
}

fn draw_vitals(d: &mut RaylibDrawHandle, fonts: &Fonts, world: &SharedWorld, vw: f32) {
    let Some(hero) = world.world.hero.as_ref() else {
        return;
    };
    let x = 18.0;
    let y = 82.0;
    let w = (vw * 0.56).clamp(220.0, 620.0);
    let h = 60.0;
    d.draw_rectangle_rounded(
        Rectangle::new(x, y, w, h),
        0.12,
        6,
        Color::new(4, 15, 28, 192),
    );
    d.draw_rectangle_lines_ex(
        Rectangle::new(x, y, w, h),
        1.0,
        Color::new(68, 122, 165, 185),
    );
    let values = [
        (
            "HP",
            hero.hitpoints as f32,
            hero.hitpoints_max as f32,
            Color::new(54, 190, 86, 230),
        ),
        (
            "SH",
            hero.shield as f32,
            hero.shield_max as f32,
            Color::new(54, 142, 224, 230),
        ),
        (
            "NH",
            hero.nano_hull as f32,
            hero.nano_hull_max as f32,
            Color::new(56, 175, 194, 230),
        ),
    ];
    let gap = 8.0;
    let cell_w = (w - 16.0 - gap * 2.0) / 3.0;
    for (i, (name, value, max, color)) in values.into_iter().enumerate() {
        let cx = x + 8.0 + i as f32 * (cell_w + gap);
        let frac = if max > 0.0 {
            (value / max).clamp(0.0, 1.0)
        } else {
            0.0
        };
        d.draw_rectangle(
            cx as i32,
            (y + 6.0) as i32,
            cell_w as i32,
            25,
            Color::new(13, 26, 39, 235),
        );
        d.draw_rectangle(
            cx as i32,
            (y + 6.0) as i32,
            (cell_w * frac) as i32,
            25,
            color,
        );
        d.draw_rectangle_lines(
            cx as i32,
            (y + 6.0) as i32,
            cell_w as i32,
            25,
            Color::new(89, 129, 157, 230),
        );
        let label = format!(
            "{name}  {} / {}",
            compact(value as f64),
            compact(max as f64)
        );
        let size = 13.0;
        let text = fonts.ui.measure_text(&label, size, 1.0);
        d.draw_text_ex(
            &fonts.ui,
            &label,
            Vector2::new(cx + (cell_w - text.x) * 0.5, y + 12.0),
            size,
            1.0,
            Color::WHITE,
        );
    }
    let config = if world.extras.config > 0 {
        format!("CFG {}", world.extras.config)
    } else {
        "CFG —".to_owned()
    };
    let details = [
        config,
        format!("SPD {}", hero.speed),
        format!(
            "CARGO {} / {}",
            compact(hero.cargo_space as f64),
            compact(hero.cargo_space_max as f64)
        ),
    ];
    let detail_w = (w - 16.0) / details.len() as f32;
    for (i, label) in details.iter().enumerate() {
        let tx = x + 8.0 + i as f32 * detail_w;
        let size = 11.0;
        d.draw_text_ex(
            &fonts.ui,
            label,
            Vector2::new(tx, y + 39.0),
            size,
            1.0,
            Color::new(187, 211, 231, 245),
        );
    }
}

fn compact(n: f64) -> String {
    if n >= 1_000_000.0 {
        format!("{:.1}M", n / 1_000_000.0)
    } else if n >= 1000.0 {
        format!("{:.0}K", n / 1000.0)
    } else {
        format!("{:.0}", n)
    }
}

fn draw_radar(d: &mut RaylibDrawHandle, game: &Game, world: &SharedWorld, radar: (f32, f32, f32)) {
    let (rx, ry, radius) = radar;
    let c = Vector2::new(rx, ry);
    d.draw_circle_v(c, radius + 5.0, Color::new(4, 12, 24, 196));
    for factor in [1.0, 0.68, 0.36] {
        d.draw_circle_lines_v(c, radius * factor, Color::new(74, 127, 173, 210));
    }
    d.draw_line_ex(
        Vector2::new(c.x - radius, c.y),
        Vector2::new(c.x + radius, c.y),
        1.0,
        Color::new(74, 127, 173, 65),
    );
    d.draw_line_ex(
        Vector2::new(c.x, c.y - radius),
        Vector2::new(c.x, c.y + radius),
        1.0,
        Color::new(74, 127, 173, 65),
    );
    let Some(hero) = world.hero_pos() else {
        return;
    };
    let range = (game.bounds.0.max(game.bounds.1) * 0.13).max(1.0);
    for portal in world.portals.values().filter(|p| p.on_minimap) {
        let dx = portal.x as f32 - hero.0;
        let dy = portal.y as f32 - hero.1;
        let dist = dx.hypot(dy);
        if dist <= range {
            let r = radius * 0.88 * (dist / range).min(1.0);
            let p = Vector2::new(c.x + dx / dist.max(1.0) * r, c.y + dy / dist.max(1.0) * r);
            d.draw_circle_v(p, 4.0, Color::new(238, 188, 79, 245));
            d.draw_circle_lines_v(p, 6.0, Color::new(255, 221, 130, 220));
        }
    }
    for e in world.world.entities.values().filter(|e| !e.destroyed) {
        let dx = e.x as f32 - hero.0;
        let dy = e.y as f32 - hero.1;
        let dist = dx.hypot(dy);
        if dist > range {
            continue;
        }
        let r = radius * 0.88 * (dist / range).min(1.0);
        let p = Vector2::new(c.x + dx / dist.max(1.0) * r, c.y + dy / dist.max(1.0) * r);
        let col = if e.npc {
            Color::new(238, 76, 65, 235)
        } else {
            Color::new(74, 162, 232, 235)
        };
        d.draw_circle_v(p, 2.5, col);
    }
    let heading = world
        .world
        .hero
        .as_ref()
        .and_then(|h| game.headings.get(&h.user_id))
        .map(|degrees| degrees.to_radians())
        .or_else(|| {
            world
                .hero_beacon
                .map(|(x, y)| (y - hero.1).atan2(x - hero.0))
        })
        .unwrap_or(-std::f32::consts::FRAC_PI_2);
    let tip = Vector2::new(c.x + heading.cos() * 8.0, c.y + heading.sin() * 8.0);
    let left = Vector2::new(
        c.x + (heading + 2.45).cos() * 7.0,
        c.y + (heading + 2.45).sin() * 7.0,
    );
    let right = Vector2::new(
        c.x + (heading - 2.45).cos() * 7.0,
        c.y + (heading - 2.45).sin() * 7.0,
    );
    d.draw_triangle(tip, left, right, Color::new(235, 248, 255, 245));
    let label = format!("{} / {}", hero.0.round() as i32, hero.1.round() as i32);
    d.draw_text(
        &label,
        (c.x - 32.0) as i32,
        (c.y + radius + 9.0) as i32,
        14,
        Color::new(225, 238, 249, 235),
    );
}
