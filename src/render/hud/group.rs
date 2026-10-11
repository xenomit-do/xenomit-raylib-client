//! Group HUD: off-screen edge indicators for party members (stub for the
//! full group window — name, hp/shield, direction only).

use super::{TXT, TXT_DIM};
use crate::game::{Camera, Game};
use crate::net::SharedWorld;
use crate::render::fonts::{draw_text_fallback, measure_text_fallback, Fonts};
use raylib::prelude::*;

/// Distance kept from the viewport border.
const EDGE_MARGIN: f32 = 30.0;

/// Where the ray from the viewport centre toward `target` leaves the
/// viewport inset by `margin`; `None` if `target` is already inside it.
pub(crate) fn edge_point(
    target: (f32, f32),
    vw: f32,
    vh: f32,
    margin: f32,
) -> Option<((f32, f32), (f32, f32))> {
    let (cx, cy) = (vw * 0.5, vh * 0.5);
    let (dx, dy) = (target.0 - cx, target.1 - cy);
    let (hx, hy) = ((cx - margin).max(1.0), (cy - margin).max(1.0));
    if dx.abs() <= hx && dy.abs() <= hy {
        return None;
    }
    let len = dx.hypot(dy).max(0.001);
    let s = (hx / dx.abs().max(0.001)).min(hy / dy.abs().max(0.001));
    Some(((cx + dx * s, cy + dy * s), (dx / len, dy / len)))
}

pub(crate) fn draw_group_indicators(
    d: &mut RaylibDrawHandle,
    game: &Game,
    world: &SharedWorld,
    tex: &crate::render::sprites::TextureCache,
    fonts: &Fonts,
    camera: &Camera,
    vw: f32,
    vh: f32,
) {
    let Some(group) = &world.group else {
        return;
    };
    let hero_id = world.world.hero.as_ref().map(|h| h.user_id);
    for m in &group.members {
        if Some(m.id) == hero_id || m.logged_out {
            continue;
        }
        if m.map_id != 0 && game.map_id != 0 && m.map_id != game.map_id {
            continue;
        }
        // Live entity position when the member is on our screen's map
        // section, else the last location the group channel reported.
        let pos = if world.world.entities.contains_key(&m.id) {
            world.pos(m.id)
        } else {
            (m.x as f32, m.y as f32)
        };
        let Some(((px, py), (ux, uy))) = edge_point(camera.to_screen(pos), vw, vh, EDGE_MARGIN)
        else {
            continue;
        };
        let col = if m.in_fight {
            Color::new(255, 120, 80, 235)
        } else {
            Color::new(110, 200, 255, 235)
        };
        // Arrow: tip toward the member.
        let (nx, ny) = (-uy, ux);
        let tip = Vector2::new(px + ux * 10.0, py + uy * 10.0);
        let a = Vector2::new(px - ux * 5.0 + nx * 7.0, py - uy * 5.0 + ny * 7.0);
        let b = Vector2::new(px - ux * 5.0 - nx * 7.0, py - uy * 5.0 - ny * 7.0);
        let cross = (a.x - tip.x) * (b.y - tip.y) - (a.y - tip.y) * (b.x - tip.x);
        if cross > 0.0 {
            d.draw_triangle(tip, b, a, col);
        } else {
            d.draw_triangle(tip, a, b, col);
        }

        // Label sits on the inward side so it stays on screen.
        let label = if m.name.is_empty() {
            format!("#{}", m.id)
        } else {
            m.name.clone()
        };
        let tw = measure_text_fallback(&fonts.ui, fonts.symbols.as_ref(), &label, 10.0, 1.0).x;
        let lx = (px - ux * 16.0 - tw * 0.5).clamp(4.0, vw - tw - 4.0);
        let ly = (py - uy * 16.0 - 12.0).clamp(4.0, vh - 28.0);
        // `GroupUI.getShipIconBitmap` — the member's `nettyShipIcons`
        // bitmap left of the name (scaled to a 14 px box).
        if let Some(clip) = game
            .ui_sprites
            .get(&(
                "groupSystemShipIcons".to_owned(),
                crate::net::group::ship_icon_res(m.ship_icon).to_owned(),
            ))
            .and_then(|id| tex.get(*id))
            .filter(|c| !c.frames.is_empty())
        {
            let (w, h) = (
                clip.frames[0].tex.width as f32,
                clip.frames[0].tex.height as f32,
            );
            let s = (14.0 / w.max(h)).min(1.0);
            crate::render::sprites::TextureCache::draw_frame_offset(
                d,
                clip,
                0,
                ((lx - 10.0).max(9.0), ly + 5.0),
                0.0,
                (s, s),
                Color::WHITE,
                (-w * 0.5, -h * 0.5),
            );
        }
        draw_text_fallback(
            d,
            &fonts.ui,
            fonts.symbols.as_ref(),
            &label,
            Vector2::new(lx, ly),
            10.0,
            1.0,
            TXT,
        );
        let bw = 40.0;
        let bx = (lx + tw * 0.5 - bw * 0.5).clamp(4.0, vw - bw - 4.0);
        let by = ly + 12.0;
        d.draw_rectangle(
            bx as i32,
            by as i32,
            bw as i32,
            3,
            Color::new(10, 12, 18, 200),
        );
        d.draw_rectangle(
            bx as i32,
            by as i32,
            (bw * m.shield_frac()) as i32,
            3,
            Color::new(80, 160, 255, 230),
        );
        d.draw_rectangle(
            bx as i32,
            by as i32 + 3,
            bw as i32,
            3,
            Color::new(10, 12, 18, 200),
        );
        d.draw_rectangle(
            bx as i32,
            by as i32 + 3,
            (bw * m.hp_frac()) as i32,
            3,
            Color::new(70, 220, 100, 230),
        );
        if m.id == group.leader_id {
            d.draw_text_ex(
                &fonts.ui,
                "*",
                Vector2::new(lx - 8.0, ly),
                10.0,
                1.0,
                TXT_DIM,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inside_is_none_outside_clamps_to_edge() {
        assert!(edge_point((400.0, 300.0), 800.0, 600.0, 30.0).is_none());
        let ((x, y), (ux, uy)) = edge_point((2000.0, 300.0), 800.0, 600.0, 30.0).unwrap();
        assert_eq!((x, y), (770.0, 300.0));
        assert!((ux - 1.0).abs() < 1e-5 && uy.abs() < 1e-5);
        let ((x, y), _) = edge_point((1200.0, 1200.0), 800.0, 600.0, 30.0).unwrap();
        // Diagonal hits the nearer bound first.
        assert!(x <= 770.0 + 0.01 && y <= 570.0 + 0.01);
        assert!((x - 770.0).abs() < 0.01 || (y - 570.0).abs() < 0.01);
    }
}
