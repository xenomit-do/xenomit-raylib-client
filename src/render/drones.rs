//! Drone sprites — the `DronesFormation2D` view. Drawn right after their
//! owner's ship sprite (stock stacks drones above the ship).

use crate::game::{Camera, Game};
use crate::net::DisplaySettings;
use crate::render::draw_util::{draw_visual, draw_visual_layers, pick_frame, CULL_MARGIN};
use crate::render::sprites::TextureCache;
use raylib::prelude::*;

/// Whether an owner's drones are drawn under the display settings.
pub(crate) fn drones_visible(settings: &DisplaySettings, owner_is_hero: bool) -> bool {
    if owner_is_hero {
        settings.display_my_drones && settings.display_drones
    } else {
        settings.display_drones
    }
}

/// Draw every drone of `owner_id` (ship at `owner_pos`, facing `heading`
/// degrees). Drone clips are rotation strips indexed by the *owner's*
/// heading, so they turn with the ship; drones whose recipe is missing
/// or still loading are skipped.
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_owner_drones(
    d: &mut RaylibDrawHandle,
    game: &Game,
    tex: &TextureCache,
    camera: &Camera,
    settings: &DisplaySettings,
    owner_is_hero: bool,
    owner_id: i32,
    owner_pos: (f32, f32),
    heading: f32,
    time_ms: u64,
    tint: Color,
    rendered_3d: &std::collections::HashSet<(i32, i32)>,
) {
    // `Settings.displayDrones` (others) / `displayMyDrones` (hero).
    if !drones_visible(settings, owner_is_hero) {
        return;
    }
    for (entry, pos) in game.drone_positions(owner_id, owner_pos, heading) {
        if rendered_3d.contains(&(owner_id, entry.index)) {
            continue;
        }
        let key = (
            entry.type_id,
            entry.level,
            entry.stats_design,
            entry.visual_design,
        );
        let Some(v) = game.drone_visuals.get(&key) else {
            continue;
        };
        let Some(clip) = v
            .sprite
            .and_then(|id| tex.get(id))
            .filter(|c| !c.frames.is_empty())
        else {
            continue;
        };
        if !camera.in_view(pos, CULL_MARGIN) {
            continue;
        }
        let screen = camera.to_screen(pos);
        let sc = camera.scale_at(pos);
        if let Some((fi, rot)) = pick_frame(v, clip, heading, time_ms, true, false, true) {
            draw_visual(d, clip, v, fi, rot, screen, sc, tint);
        }
        draw_visual_layers(
            d, tex, v, heading, time_ms, true, false, true, screen, sc, tint,
        );
    }
}
