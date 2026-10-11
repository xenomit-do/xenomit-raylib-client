//! raylib draw layer: textures, sprites, map, HUD — the `view2D` equivalent.
//!
//! `draw_scene` renders one frame in stock-client order: space backdrop
//! (parallax layers + planets over a procedural starfield), collectables,
//! entities + hero (rotation strips, name plates, hp/shield bars,
//! selection ring, placeholders while sprites stream in), transient
//! effects (lasers/explosions/pickups/floating text)
//! and finally the HUD.

use crate::assets::AssetManager;
use crate::game::Game;
use crate::net::SharedWorld;
use raylib::prelude::*;

pub(crate) mod backgrounds3d;
pub mod collectables;
pub mod draw_util;
pub mod drones;
pub mod effects3d;
pub mod effects_draw;
pub mod entities;
pub mod fonts;
pub mod hud;
pub(crate) mod lensflares;
pub(crate) mod lock3d;
pub mod map;
pub(crate) mod materials3d;
pub(crate) mod model_loader;
pub mod nameplate;
pub mod particles3d;
pub mod portals;
pub mod scene3d;
pub(crate) mod ship_fx;
pub mod sprites;

pub use fonts::{draw_text_glow, load_fonts, Fonts};
pub use sprites::{GpuClip, TextureCache};

pub(crate) use draw_util::{entity_label, npc_color, plate_color};

/// Draw one full frame: map backdrop, map objects (collectables, assets,
/// ships incl. hero), effects, then HUD (minimap, vitals, log, target
/// info, debug overlay).
///
/// Call between `begin_drawing`/`end`. Texture uploads happen earlier via
/// [`TextureCache::sync`] (GPU uploads need `&mut RaylibHandle`, which the
/// draw handle mutably borrows).
pub fn draw_scene(
    d: &mut RaylibDrawHandle<'_>,
    game: &mut Game,
    world: &SharedWorld,
    assets: &mut AssetManager,
    tex: &mut TextureCache,
    fonts: &Fonts,
) {
    game.camera.viewport = (d.get_screen_width() as f32, d.get_screen_height() as f32);
    game.camera.perspective = game.preferences.render.camera_3d;
    let camera = game.camera.clone();
    let time_ms = (d.get_time() * 1000.0) as u64;
    let assets = &*assets;

    map::draw_backdrop(d, &camera, game, assets, tex);
    let rendered_3d = scene3d::draw_ship_models(d, game, world, tex);
    let rendered_drones_3d =
        scene3d::draw_drone_models(d, game, world, &assets.game_xml.drone_formations, tex);
    let rendered_3d_effects = effects3d::draw_effects_3d(d, game, tex);
    let rendered_collectables_3d = scene3d::draw_collectable_models(d, game, world, tex);
    let rendered_portals_3d = scene3d::draw_portal_models(d, game, world, tex);
    lock3d::queue_lock_indicator(game, world);
    particles3d::flush(d, game, tex);
    collectables::draw_collectables(
        d,
        game,
        world,
        tex,
        &camera,
        time_ms,
        &rendered_collectables_3d,
    );
    portals::draw_portals(d, game, world, tex, &camera, time_ms, &rendered_portals_3d);
    effects_draw::draw_engine_smoke(d, game, tex, &camera);
    entities::draw_entities(
        d,
        game,
        world,
        assets,
        tex,
        fonts,
        &camera,
        time_ms,
        &rendered_3d,
        &rendered_drones_3d,
    );
    ship_fx::draw_fx_layer(d, game, world, assets, tex, &camera);
    effects_draw::draw_effects(d, game, world, tex, fonts, &camera, &rendered_3d_effects);
    lensflares::draw(d, game, world, tex, &camera);
    hud::draw_hud(d, game, world, assets, tex, fonts, &camera);
}
