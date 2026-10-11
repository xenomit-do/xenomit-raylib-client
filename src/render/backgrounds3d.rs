//! Authored `lib_maps` Away3D backdrop pass.
//!
//! Planes and meshes use the live map scene recipes. Tilemap layers remain on
//! the established 2D tile renderer until their mask/material path is ported.

use crate::assets::{AssetManager, MapScene3D, ScenePlane3D};
use crate::game::Game;
use crate::render::{scene3d, sprites::TextureCache};
use raylib::prelude::*;
use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};

fn unsupported_once() -> &'static Mutex<HashSet<String>> {
    static SEEN: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    SEEN.get_or_init(|| Mutex::new(HashSet::new()))
}

/// Whether every drawable object in the map's scene is uploaded and ready.
/// Returns false during streaming so the normal 2D backdrop remains visible.
pub(crate) fn ready(game: &Game, assets: &AssetManager, tex: &TextureCache) -> bool {
    if !game.preferences.render.backgrounds_3d {
        return false;
    }
    let Some(scene) = assets.map_scene3d_for_map(game.map_id) else {
        return false;
    };
    if scene.planes.is_empty() && scene.meshes.is_empty() {
        return false;
    }
    let quality = game.preferences.render.texture_quality;
    let planes_ready = scene.planes.iter().all(|p| {
        p.texture.is_empty()
            || tex.texture3d(&p.texture).is_some()
                && (p.billboard || tex.plane_model3d(&p.texture).is_some())
    });
    let meshes_ready = scene
        .meshes
        .iter()
        .all(|m| tex.model3d(&m.geometry, &m.material, quality).is_some());
    if planes_ready && meshes_ready {
        report_unsupported_once(scene);
    }
    planes_ready && meshes_ready
}

pub(crate) fn draw(
    d: &mut RaylibDrawHandle<'_>,
    game: &Game,
    assets: &AssetManager,
    tex: &TextureCache,
) {
    let Some(scene) = assets.map_scene3d_for_map(game.map_id) else {
        return;
    };
    let quality = game.preferences.render.texture_quality;
    let camera = scene3d::camera(game);
    let mut mode = d.begin_mode3D(camera);
    for plane in &scene.planes {
        draw_plane(&mut mode, camera, scene, plane, tex);
    }
    for mesh in &scene.meshes {
        let Some(model) = tex.model3d(&mesh.geometry, &mesh.material, quality) else {
            continue;
        };
        let pos = scene3d::rl_point(mesh.position.0, mesh.position.1, mesh.position.2);
        let scale = scene.scale * if mesh.scale == 0.0 { 1.0 } else { mesh.scale };
        let (axis, angle) = scene3d::euler_axis_angle(mesh.rotation);
        mode.draw_model_ex(
            model,
            pos,
            axis,
            angle,
            Vector3::new(scale, scale, scale),
            Color::WHITE,
        );
    }
}

fn draw_plane(
    d: &mut RaylibMode3D<'_, RaylibDrawHandle<'_>>,
    camera: Camera3D,
    scene: &MapScene3D,
    plane: &ScenePlane3D,
    tex: &TextureCache,
) {
    let Some(texture) = tex.texture3d(&plane.texture) else {
        return;
    };
    if plane.billboard {
        let position = plane_position(plane);
        let size = Vector2::new(
            plane.scale.abs().max(1.0) * scene.scale,
            plane.scale.abs().max(1.0) * scene.scale,
        );
        let source = Rectangle::new(0.0, 0.0, texture.width as f32, texture.height as f32);
        d.draw_billboard_pro(
            camera,
            **texture,
            source,
            position,
            Vector3::new(0.0, 1.0, 0.0),
            size,
            Vector2::new(size.x * 0.5, size.y * 0.5),
            plane.rotation.2,
            Color::WHITE,
        );
        return;
    }
    let Some(model) = tex.plane_model3d(&plane.texture) else {
        return;
    };
    let position = plane_position(plane);
    let scale = plane.scale.abs().max(1.0) * scene.scale;
    let (axis, angle) = scene3d::euler_axis_angle(plane.rotation);
    d.draw_model_ex(
        model,
        position,
        axis,
        angle,
        Vector3::new(scale, scale, scale),
        Color::WHITE,
    );
}

/// Plane position in Away3D scene coordinates (left-handed).
fn plane_away_position(plane: &ScenePlane3D) -> Vector3 {
    let mut position = Vector3::new(plane.position.0, plane.position.1, plane.position.2);
    if let Some((phi, theta, radius)) = plane.spherical {
        let (phi, theta) = (phi.to_radians(), theta.to_radians());
        // UberAsset3D.as:435-439 converts the authored spherical point as:
        // x += R*sin(theta)*cos(phi); y -= R*cos(theta);
        // z -= R*sin(theta)*sin(phi).
        position.x += radius * theta.sin() * phi.cos();
        position.y -= radius * theta.cos();
        position.z -= radius * theta.sin() * phi.sin();
    }
    position
}

/// Plane position in raylib world coordinates.
fn plane_position(plane: &ScenePlane3D) -> Vector3 {
    let p = plane_away_position(plane);
    scene3d::rl_point(p.x, p.y, p.z)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::uberlib::UberAssets;

    #[test]
    fn authored_mmo_plane_spherical_position_matches_away3d_conversion() {
        let assets = UberAssets::parse(&[(
            "lib_maps",
            include_str!("../../tests/fixtures/lib_maps.xml"),
        )])
        .unwrap();
        let scene = assets.map_scene3d("lib_maps:map_1-1").unwrap();
        let plane = scene
            .planes
            .iter()
            .find(|p| p.texture == "background1_atf")
            .unwrap();
        let p = plane_away_position(plane);
        assert!((p.x - 10_000.0).abs() < 0.1, "x = {}", p.x);
        assert!((p.y + 43_309.93).abs() < 0.1, "y = {}", p.y);
        assert!((p.z - 25_005.0).abs() < 0.1, "z = {}", p.z);
        assert!((plane_position(plane).z + 25_005.0).abs() < 0.1);
    }
}

fn report_unsupported_once(scene: &MapScene3D) {
    if scene.tilemaps.is_empty() {
        return;
    }
    let mut seen = unsupported_once().lock().unwrap_or_else(|e| e.into_inner());
    if seen.insert(scene.asset_id.clone()) {
        log::info!(
            "map scene {}: rendering authored 3D planes/meshes; {} tilemap layer(s) still use the 2D renderer",
            scene.asset_id,
            scene.tilemaps.len(),
        );
    }
}
