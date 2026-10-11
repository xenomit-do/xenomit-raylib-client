use super::{fmt_num_i64, panel, TXT_DIM};
use crate::game::input;
use crate::locale::LocaleCatalog;
use crate::net::SharedWorld;
use crate::render::entity_label;
use crate::render::fonts::Fonts;
use crate::render::plate_color;
use raylib::prelude::*;
use xenomit_packet_client::model::{Entity, EntityKind};

pub(crate) fn draw_target_panel(
    d: &mut RaylibDrawHandle,
    e: &Entity,
    e_pos: (f32, f32),
    hero_pos: Option<(f32, f32)>,
    fonts: &Fonts,
    r: input::Rect,
    world: &SharedWorld,
) {
    let (px, py, pw, _) = r;
    panel(d, fonts, "", r);

    let hero_faction = world.world.hero.as_ref().map(|h| h.faction_id).unwrap_or(0);
    let name = entity_label(&e.name, &e.clan_tag, e.npc);
    let name = if name.is_empty() {
        format!("{} #{}", kind_name(e.kind), e.id)
    } else {
        name
    };
    let m = fonts.ui.measure_text(&name, 11.0, 1.0);
    let name_col = plate_color(input::plate_kind(e.npc, e.faction_id, hero_faction));
    d.draw_text_ex(
        &fonts.ui,
        &name,
        Vector2::new(px + pw * 0.5 - m.x * 0.5, py + 5.0),
        11.0,
        1.0,
        name_col,
    );

    let dist = hero_pos
        .map(|hp| {
            let dx = e_pos.0 - hp.0;
            let dy = e_pos.1 - hp.1;
            (dx * dx + dy * dy).sqrt() as i32
        })
        .unwrap_or(0);
    let sub = format!("{} {}   {}m", kind_name(e.kind), e.type_id, dist);
    let m = fonts.ui.measure_text(&sub, 10.0, 1.0);
    d.draw_text_ex(
        &fonts.ui,
        &sub,
        Vector2::new(px + pw * 0.5 - m.x * 0.5, py + 19.0),
        10.0,
        1.0,
        TXT_DIM,
    );

    let bx = px + 10.0;
    let bw = pw - 20.0;
    let mut by = py + 32.0;
    target_bar(
        d,
        fonts,
        bx,
        by,
        bw,
        e.hitpoints.map(|v| v as i64),
        e.hitpoints_max.map(|v| v as i64),
        Color::new(70, 220, 100, 235),
        false,
        &world.locale,
    );
    by += 9.0;
    target_bar(
        d,
        fonts,
        bx,
        by,
        bw,
        e.shield.map(|v| v as i64),
        e.shield_max.map(|v| v as i64),
        Color::new(80, 160, 255, 235),
        true,
        &world.locale,
    );
}

#[allow(clippy::too_many_arguments)]
fn target_bar(
    d: &mut RaylibDrawHandle,
    fonts: &Fonts,
    x: f32,
    y: f32,
    w: f32,
    cur: Option<i64>,
    max: Option<i64>,
    col: Color,
    grey_unknown: bool,
    loc: &LocaleCatalog,
) {
    let known = cur.is_some() && max.is_some();
    let frac = match (cur, max) {
        (Some(c), Some(m)) if m > 0 => (c as f32 / m as f32).clamp(0.0, 1.0),
        _ => 0.0,
    };
    let label = match (cur, max) {
        (Some(c), Some(m)) => format!("{}/{}", fmt_num_i64(c, loc), fmt_num_i64(m, loc)),
        _ if grey_unknown => "—".to_owned(),
        _ => String::new(),
    };
    d.draw_rectangle(x as i32, y as i32, w as i32, 6, Color::new(6, 8, 14, 220));
    if frac > 0.0 {
        d.draw_rectangle(
            x as i32,
            y as i32,
            (w * frac) as i32,
            6,
            if known {
                col
            } else {
                Color::new(110, 115, 125, 200)
            },
        );
    }
    if !label.is_empty() {
        let text_col = if known {
            Color::new(235, 240, 248, 230)
        } else {
            Color::new(120, 126, 138, 200)
        };
        let m = fonts.ui.measure_text(&label, 7.0, 1.0);
        d.draw_text_ex(
            &fonts.ui,
            &label,
            Vector2::new(x + w * 0.5 - m.x * 0.5, y - 0.5),
            7.0,
            1.0,
            text_col,
        );
    }
}

fn kind_name(kind: EntityKind) -> &'static str {
    match kind {
        EntityKind::Ship => "Ship",
        EntityKind::Asset => "Asset",
        EntityKind::Other => "Entity",
    }
}
