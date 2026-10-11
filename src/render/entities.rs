use crate::assets::AssetManager;
use crate::game::{input, Camera, Game};
use crate::net::SharedWorld;
use crate::render::draw_util::{
    draw_visual, draw_visual_layers, entity_label, faction_color, npc_color, pick_frame, ratio,
    ratio_i, ship_pattern, visual_world_rect, CULL_MARGIN,
};
use crate::render::drones::draw_owner_drones;
use crate::render::effects_draw::{draw_selection, draw_wreck};
use crate::render::fonts::Fonts;
use crate::render::nameplate::{
    draw_criminal_marker, draw_name_plate, draw_top_bars, lock_type, name_color, plate_visible,
    tag_color, MICRO_BARS_H,
};
use crate::render::ship_fx;
use crate::render::sprites::TextureCache;
use raylib::prelude::*;
use xenomit_packet_client::model::EntityKind;

/// Cloaked ships render dimly (stock alpha ~0.35).
const CLOAK_ALPHA: f32 = 0.35;

#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_entities(
    d: &mut RaylibDrawHandle,
    game: &Game,
    world: &SharedWorld,
    assets: &AssetManager,
    tex: &TextureCache,
    fonts: &Fonts,
    camera: &Camera,
    time_ms: u64,
    rendered_3d: &std::collections::HashSet<i32>,
    rendered_drones_3d: &std::collections::HashSet<(i32, i32)>,
) {
    let hero_id = world.world.hero.as_ref().map(|h| h.user_id);
    let hero_faction = world.world.hero.as_ref().map(|h| h.faction_id).unwrap_or(0);
    let hero_clan = world.world.hero.as_ref().map(|h| h.clan_id).unwrap_or(0);
    for e in world.world.entities.values() {
        if Some(e.id) == hero_id {
            continue;
        }
        // A ship whose `type_id` has not arrived yet still gets the
        // faction-dot fallback plus nameplate/drones — never skip it.
        // Idle pets are parked — not drawn.
        if world.pets.is_hidden(e.id) {
            continue;
        }
        let pos = game.draw_pos(world, e.id);
        let heading = game.headings.get(&e.id).copied().unwrap_or(0.0);
        let drone_heading = game.drone_headings.get(&e.id).copied().unwrap_or(heading);
        let pat = ship_pattern(assets, &e.type_id);
        let rotatable = pat.map(|p| p.rotatable).unwrap_or(false);
        let loop_play = pat.map(|p| p.loop_anim).unwrap_or(false);
        // `0|n|LSH`: a target fading to gray (lost lock / neutralized).
        let base_tint = if world.gray_targets.contains(&e.id) {
            Color::new(140, 140, 140, 255)
        } else {
            Color::WHITE
        };
        let tint = if e.cloaked {
            base_tint.alpha(CLOAK_ALPHA)
        } else {
            base_tint
        };
        // `Kind::HullAlpha` fx (ghost/scramble) dim the whole ship view.
        let hull_alpha = game.ship_fx.hull_alpha(e.id);
        let tint = if hull_alpha < 1.0 {
            tint.alpha(hull_alpha)
        } else {
            tint
        };
        // `simpleOpponents`: other players' ships collapse to a dot.
        let simple = world.settings.simple_opponents
            && e.kind == EntityKind::Ship
            && !e.npc
            && world.pets.get(e.id).is_none();
        let v = game.entity_visuals.get(&e.id);
        let clip = v
            .and_then(|v| v.sprite)
            .and_then(|id| tex.get(id))
            .filter(|c| !c.frames.is_empty())
            .filter(|_| !simple);
        let ship = matches!(e.kind, EntityKind::Ship | EntityKind::Other);
        let picked = match (v, clip) {
            (Some(v), Some(clip)) => {
                pick_frame(v, clip, heading, time_ms, rotatable, loop_play, ship)
            }
            _ => None,
        };
        let anchor = match (v, clip, picked) {
            (Some(v), Some(clip), Some((_, rot))) => {
                Some(visual_world_rect(v, clip, pos, rot != 0.0))
            }
            _ => None,
        };
        let on_screen = match anchor {
            Some((x0, y0, x1, y1)) => camera.rect_in_view((x0, y0), (x1, y1), CULL_MARGIN),
            None => camera.in_view(pos, CULL_MARGIN),
        };
        if !on_screen {
            continue;
        }
        let screen = camera.to_screen(pos);
        let sc = camera.scale_at(pos);
        let ring_r = match anchor {
            Some((x0, y0, x1, y1)) => (x1 - x0).max(y1 - y0) * 0.5 * sc,
            None => 18.0 * sc,
        };
        // `ShipViewShake`-style fx jitter displaces only the ship view
        // (hull/pod/fx), never plates or selection rings.
        let (jx, jy) = game.ship_fx.shake(e.id, time_ms as f32 / 1000.0);
        let ship_screen = (screen.0 + jx, screen.1 + jy);
        let ship_pos = (pos.0 + jx / sc.max(0.01), pos.1 + jy / sc.max(0.01));
        let moving = world.tweens.contains_key(&e.id);
        // `addChildAt(0)` ship-view fx — below the hull.
        if !e.destroyed && !simple {
            ship_fx::draw_entity_fx(
                d,
                game,
                assets,
                tex,
                camera,
                e.id,
                ship_pos,
                heading,
                ring_r / sc,
                moving,
                ship_fx::Layer::BelowHull,
            );
        }
        let mut drew = false;
        if rendered_3d.contains(&e.id) {
            drew = true;
        } else if let (Some(v), Some(clip), Some((fi, rot))) = (v, clip, picked) {
            draw_visual(d, clip, v, fi, rot, ship_screen, sc, tint);
            draw_visual_layers(
                d, tex, v, heading, time_ms, rotatable, loop_play, ship, ship_screen, sc, tint,
            );
            drew = true;
        }
        if !drew {
            if e.destroyed {
                draw_wreck(d, ship_screen, sc);
            } else {
                let col = if e.npc {
                    npc_color()
                } else {
                    faction_color(e.faction_id)
                };
                let r = (13.0 * sc).clamp(5.0, 30.0);
                d.draw_circle_v(
                    Vector2::new(ship_screen.0, ship_screen.1),
                    r,
                    col.alpha(0.85),
                );
                d.draw_circle_lines_v(
                    Vector2::new(ship_screen.0, ship_screen.1),
                    r + 1.0,
                    Color::new(10, 14, 20, 200),
                );
            }
        }

        if !e.destroyed && !simple {
            draw_owner_drones(
                d,
                game,
                tex,
                camera,
                &world.settings,
                false,
                e.id,
                pos,
                drone_heading,
                time_ms,
                tint,
                rendered_drones_3d,
            );
        }

        let info = world.ship_info.get(&e.id).cloned().unwrap_or_default();
        draw_ship_pod(
            d,
            tex,
            v,
            pat,
            &info,
            heading,
            time_ms,
            ship,
            ship_screen,
            sc,
            tint,
        );
        // `addChild` ship-view fx — above hull + decorators.
        if !e.destroyed && !simple {
            ship_fx::draw_entity_fx(
                d,
                game,
                assets,
                tex,
                camera,
                e.id,
                ship_pos,
                heading,
                ring_r / sc,
                moving,
                ship_fx::Layer::AboveHull,
            );
        }

        if game.selected == Some(e.id) {
            let border = game
                .ui_sprites
                .get(&("ui".to_owned(), "ship_border".to_owned()))
                .and_then(|id| tex.get(*id))
                .filter(|c| !c.frames.is_empty());
            match border {
                Some(clip) => {
                    let fi = lock_type(e.npc, &info)
                        .saturating_sub(1)
                        .min(clip.frames.len() - 1);
                    let fade = game
                        .selected_at
                        .map(|at| (at.elapsed().as_secs_f32() / 0.3).min(1.0))
                        .unwrap_or(1.0);
                    let b = clip.bounds;
                    TextureCache::draw_frame_offset(
                        d,
                        clip,
                        fi,
                        screen,
                        0.0,
                        (1.0, 1.0),
                        Color::WHITE.alpha(fade),
                        (-(b.0 + b.2) * 0.5, -(b.1 + b.3) * 0.5),
                    );
                }
                None => draw_selection(d, screen, ring_r + 10.0),
            }
        }

        if !e.cloaked && !e.destroyed {
            // `click_radius` is screen px at zoom 1 — scale like the
            // hero path so plates/bars sit at the ship's edge.
            let click_r = v
                .map(|v| v.click_radius)
                .unwrap_or(input::DEFAULT_CLICK_RADIUS)
                * sc;
            // Uber `<label x y>` shifts the plate anchor (zoom-scaled).
            let (lx, ly) = v.map(|v| v.label_offset).unwrap_or((0.0, 0.0));
            // Pets read `P.E.T {name} [{level}]`.
            let name = match world.pets.get(e.id) {
                Some(p) => p.plate_name(),
                None => entity_label(&e.name, "", e.npc),
            };
            let is_player = e.kind == EntityKind::Ship && !e.npc && world.pets.get(e.id).is_none();
            if world.criminals.contains(&e.id) {
                draw_criminal_marker(d, fonts, screen.0, screen.1 - click_r - 22.0 - MICRO_BARS_H);
            }
            if (!name.is_empty() || !e.clan_tag.is_empty())
                && plate_visible(&world.settings, is_player)
            {
                draw_name_plate(
                    d,
                    fonts,
                    tex,
                    game,
                    &info,
                    screen.0 + lx * sc,
                    screen.1 + click_r + ly * sc,
                    &name,
                    &e.clan_tag,
                    name_color(false, e.clan_id, e.faction_id, hero_clan, hero_faction),
                    tag_color(info.clan_diplomacy),
                    e.faction_id,
                );
            }
            let under_attack = world.world.laser_links.values().any(|&v| v == e.id);
            let damaged = e
                .hitpoints
                .zip(e.hitpoints_max)
                .is_some_and(|(hp, max)| max > 0.0 && hp < max);
            // Uber `<hp>` assets (turrets, stations) keep their bar up.
            let always_bar = v.is_some_and(|v| v.has_hp_bar);
            let locked = game.selected == Some(e.id) || under_attack || damaged || always_bar;
            if locked && (e.hitpoints_max.is_some() || e.shield_max.is_some()) {
                let frac_hp = ratio(e.hitpoints, e.hitpoints_max);
                let frac_sh = ratio_i(e.shield, e.shield_max);
                draw_top_bars(
                    d,
                    tex,
                    game,
                    &info,
                    screen.0,
                    screen.1 - click_r - 5.0 - MICRO_BARS_H,
                    frac_hp,
                    frac_sh,
                );
            }
        }
    }

    if let Some(h) = &world.world.hero {
        let pos = world.pos(h.user_id);
        let screen = camera.to_screen(pos);
        let sc = camera.scale_at(pos);
        let heading = game.headings.get(&h.user_id).copied().unwrap_or(0.0);
        let drone_heading = game
            .drone_headings
            .get(&h.user_id)
            .copied()
            .unwrap_or(heading);
        let pat = ship_pattern(assets, &h.type_id);
        let rotatable = pat.map(|p| p.rotatable).unwrap_or(false);
        let loop_play = pat.map(|p| p.loop_anim).unwrap_or(false);
        let tint = if h.cloaked {
            Color::WHITE.alpha(CLOAK_ALPHA)
        } else {
            Color::WHITE
        };
        let hull_alpha = game.ship_fx.hull_alpha(h.user_id);
        let tint = if hull_alpha < 1.0 {
            tint.alpha(hull_alpha)
        } else {
            tint
        };
        let v = game.entity_visuals.get(&h.user_id);
        let clip = v
            .and_then(|v| v.sprite)
            .and_then(|id| tex.get(id))
            .filter(|c| !c.frames.is_empty());
        let picked = match (v, clip) {
            (Some(v), Some(clip)) => {
                pick_frame(v, clip, heading, time_ms, rotatable, loop_play, true)
            }
            _ => None,
        };
        let hero_ring_r = match (v, clip, picked) {
            (Some(v), Some(clip), Some((_, rot))) => {
                let (x0, y0, x1, y1) = visual_world_rect(v, clip, pos, rot != 0.0);
                (x1 - x0).max(y1 - y0) * 0.5
            }
            _ => 18.0,
        };
        let (jx, jy) = game.ship_fx.shake(h.user_id, time_ms as f32 / 1000.0);
        let ship_screen = (screen.0 + jx, screen.1 + jy);
        let ship_pos = (pos.0 + jx / sc.max(0.01), pos.1 + jy / sc.max(0.01));
        let moving = world.tweens.contains_key(&h.user_id);
        ship_fx::draw_entity_fx(
            d,
            game,
            assets,
            tex,
            camera,
            h.user_id,
            ship_pos,
            heading,
            hero_ring_r,
            moving,
            ship_fx::Layer::BelowHull,
        );
        let mut drew = rendered_3d.contains(&h.user_id);
        if drew {
            // Drawn as an authored 3D mesh in `scene3d::draw_ship_models`.
        } else if let (Some(v), Some(clip), Some((fi, rot))) = (v, clip, picked) {
            draw_visual(d, clip, v, fi, rot, ship_screen, sc, tint);
            draw_visual_layers(
                d, tex, v, heading, time_ms, rotatable, loop_play, true, ship_screen, sc, tint,
            );
            drew = true;
        }
        if !(h.hitpoints <= 0.0 && h.hitpoints_max > 0.0) {
            draw_owner_drones(
                d,
                game,
                tex,
                camera,
                &world.settings,
                true,
                h.user_id,
                pos,
                drone_heading,
                time_ms,
                tint,
                rendered_drones_3d,
            );
        }
        if !drew {
            let r = (13.0 * sc).clamp(5.0, 30.0);
            d.draw_circle_v(
                Vector2::new(ship_screen.0, ship_screen.1),
                r,
                Color::new(80, 220, 140, 220),
            );
            d.draw_circle_lines_v(
                Vector2::new(ship_screen.0, ship_screen.1),
                r + 1.0,
                Color::new(10, 14, 20, 200),
            );
        }
        let info = world.ship_info.get(&h.user_id).cloned().unwrap_or_default();
        draw_ship_pod(
            d,
            tex,
            v,
            pat,
            &info,
            heading,
            time_ms,
            true,
            ship_screen,
            sc,
            tint,
        );
        ship_fx::draw_entity_fx(
            d,
            game,
            assets,
            tex,
            camera,
            h.user_id,
            ship_pos,
            heading,
            hero_ring_r,
            moving,
            ship_fx::Layer::AboveHull,
        );
        let click_r = v
            .map(|v| v.click_radius)
            .unwrap_or(input::DEFAULT_CLICK_RADIUS)
            * sc;
        let name = entity_label(&h.user_name, "", false);
        if (!name.is_empty() || !h.clan_tag.is_empty()) && plate_visible(&world.settings, true) {
            draw_name_plate(
                d,
                fonts,
                tex,
                game,
                &info,
                screen.0,
                screen.1 + click_r,
                &name,
                &h.clan_tag,
                name_color(true, h.clan_id, h.faction_id, h.clan_id, h.faction_id),
                tag_color(info.clan_diplomacy),
                h.faction_id,
            );
        }
        let frac_hp = if h.hitpoints_max > 0.0 {
            (h.hitpoints / h.hitpoints_max) as f32
        } else {
            0.0
        };
        let frac_sh = if h.shield_max > 0 {
            h.shield as f32 / h.shield_max as f32
        } else {
            0.0
        };
        draw_top_bars(
            d,
            tex,
            game,
            &info,
            screen.0,
            screen.1 - click_r - 5.0 - MICRO_BARS_H,
            frac_hp,
            frac_sh,
        );
    }
}

/// `Pod2D` — pod clip over hull and drones while the owner carries
/// `PodCarrierTrait`: `aegis`/`citadel` pattern classes get it from
/// their ship factory, everyone else only with
/// `VISUALMODIFIER_HEALING_POD` (7) active.
#[allow(clippy::too_many_arguments)]
fn draw_ship_pod(
    d: &mut RaylibDrawHandle,
    tex: &TextureCache,
    v: Option<&crate::game::Visual>,
    pat: Option<&crate::assets::gamexml::ShipPattern>,
    info: &crate::net::ships::ShipInfo,
    heading: f32,
    time_ms: u64,
    ship: bool,
    screen: (f32, f32),
    sc: f32,
    tint: Color,
) {
    let Some(pv) = v.and_then(|v| v.pod.as_deref()) else {
        return;
    };
    let carrier = info
        .modifiers
        .contains(&crate::assets::pods::HEALING_POD_MODIFIER)
        || matches!(pat.map(|p| p.class_id.as_str()), Some("aegis" | "citadel"));
    if !carrier {
        return;
    }
    let Some(clip) = pv
        .sprite
        .and_then(|id| tex.get(id))
        .filter(|c| !c.frames.is_empty())
    else {
        return;
    };
    // `load(srcKey, "mc")` defaults: rotatable, no loop.
    if let Some((fi, rot)) = pick_frame(pv, clip, heading, time_ms, true, false, ship) {
        draw_visual(d, clip, pv, fi, rot, screen, sc, tint);
    }
}
