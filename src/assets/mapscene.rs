//! Authored `lib_maps` display3D scene recipes.
//!
//! These are separate from `Display3D` on ships and portals: map recipes also
//! contain parallax planes, tilemaps, camera zones, and transform containers.

use super::material::MaterialDesc;
use super::uberlib::Xml;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct MapScene3D {
    pub asset_id: String,
    pub scale: f32,
    pub camera_zone: Option<CameraZone3D>,
    pub planes: Vec<ScenePlane3D>,
    pub meshes: Vec<SceneMesh3D>,
    pub tilemaps: Vec<SceneTilemap3D>,
    /// `display3D>lensFlare` rows (`Background3D._lensFlares`).
    pub lensflares: Vec<SceneLensFlare3D>,
}

/// `<lensFlare id typeID x y z star blendMode>` — `Background3D` wraps it
/// in a `LensFlareVO` with the position `(x, -z, -y)` (Away3D point).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SceneLensFlare3D {
    pub id: i32,
    pub type_id: i32,
    /// Away3D world position after parent transforms.
    pub position: (f32, f32, f32),
    pub star: bool,
    pub blend: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CameraZone3D {
    pub center: (f32, f32),
    pub scale: f32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ScenePlane3D {
    pub id: String,
    pub texture: String,
    pub quality: String,
    pub blend_mode: String,
    pub position: (f32, f32, f32),
    pub spherical: Option<(f32, f32, f32)>,
    pub rotation: (f32, f32, f32),
    pub scale: f32,
    pub billboard: bool,
    pub alpha_blending: bool,
    pub both_sides: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SceneMesh3D {
    pub id: String,
    pub geometry: String,
    /// Mirror of `material.texture` (`@diffuse || @texture`).
    pub texture: String,
    /// Mirror of `material.texture_settings` (`@tex_settings`).
    pub texture_settings: String,
    /// Full Away3D material descriptor (per-slot texture sources).
    pub material: MaterialDesc,
    pub quality: String,
    pub position: (f32, f32, f32),
    pub rotation: (f32, f32, f32),
    pub scale: f32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SceneTilemap3D {
    pub type_id: i32,
    pub layer: i32,
    pub mask_id: i32,
    pub map_scale: f32,
    pub tile_scale: f32,
    pub hide_lensflare: bool,
    pub quality: String,
}

impl MapScene3D {
    /// `hideLensflare` tilemaps occlude the scene's flares — with no
    /// per-pixel mask hit test we treat them as hiding the whole scene's
    /// flares (documented approximation).
    pub fn hides_lensflares(&self) -> bool {
        self.tilemaps.iter().any(|t| t.hide_lensflare)
    }
}

#[derive(Clone, Copy, Default)]
struct Transform {
    pos: (f32, f32, f32),
    rot: (f32, f32, f32),
    scale: f32,
}

pub(crate) fn parse(asset_id: &str, asset: &Xml) -> Option<MapScene3D> {
    let display = asset.children.iter().find(|n| n.tag == "display3D")?;
    let mut scene = MapScene3D {
        asset_id: asset_id.to_owned(),
        scale: 1.0,
        ..MapScene3D::default()
    };
    if let Some(zone) = display.children.iter().find(|n| n.tag == "cameraZone") {
        scene.camera_zone = Some(CameraZone3D {
            center: (num(zone, "x"), num(zone, "y")),
            scale: number(zone, "scale").unwrap_or(1.0),
        });
    }
    visit_children(display, transform(display), &mut scene);
    (!scene.planes.is_empty()
        || !scene.meshes.is_empty()
        || !scene.tilemaps.is_empty()
        || !scene.lensflares.is_empty())
    .then_some(scene)
}

fn visit_children(node: &Xml, parent: Transform, scene: &mut MapScene3D) {
    for child in &node.children {
        match child.tag.as_str() {
            "mesh" => {
                let local = transform(child);
                let combined = combine(parent, local);
                let geometry = attr(child, "geometry");
                if geometry.is_empty() || geometry.contains('%') {
                    continue;
                }
                let material = MaterialDesc::parse(child);
                scene.meshes.push(SceneMesh3D {
                    id: attr(child, "id"),
                    geometry,
                    texture: material.texture.clone(),
                    texture_settings: material.texture_settings.clone(),
                    material,
                    quality: attr(child, "quality_settings"),
                    position: combined.pos,
                    rotation: combined.rot,
                    scale: combined.scale,
                });
            }
            "plane" => {
                let local = transform(child);
                let combined = combine(parent, local);
                let phi = number(child, "sphericalPhi");
                let theta = number(child, "sphericalTheta");
                let radius = number(child, "sphericalR");
                scene.planes.push(ScenePlane3D {
                    id: attr(child, "id"),
                    texture: attr(child, "resKey"),
                    quality: attr(child, "quality_settings"),
                    blend_mode: attr(child, "blendMode"),
                    position: combined.pos,
                    spherical: match (phi, theta, radius) {
                        (Some(phi), Some(theta), Some(radius)) => Some((phi, theta, radius)),
                        _ => None,
                    },
                    rotation: combined.rot,
                    scale: combined.scale,
                    billboard: boolean(child, "billboard"),
                    alpha_blending: boolean(child, "alphaBlending"),
                    both_sides: boolean(child, "bothSides"),
                });
            }
            "tilemap" => scene.tilemaps.push(SceneTilemap3D {
                type_id: integer(child, "typeID"),
                layer: integer(child, "layer"),
                mask_id: integer(child, "maskID"),
                map_scale: number(child, "mapScale").unwrap_or(1.0),
                tile_scale: number(child, "tileScale").unwrap_or(1.0),
                hide_lensflare: boolean(child, "hideLensflare"),
                quality: attr(child, "quality_settings"),
            }),
            // `Background3D` stores the authored point as
            // `LensFlareVO(x, -z, -y)` — kept in that Away3D space here.
            "lensFlare" => {
                let combined = combine(parent, transform(child));
                scene.lensflares.push(SceneLensFlare3D {
                    id: integer(child, "id"),
                    type_id: integer(child, "typeID"),
                    position: (
                        combined.pos.0,
                        -combined.pos.2,
                        -combined.pos.1,
                    ),
                    star: boolean(child, "star"),
                    blend: attr(child, "blendMode"),
                });
            }
            // Transforms on containers are applied to their child meshes and
            // planes. Other nodes (animation, particles) are not rendered by
            // the current map-scene pass.
            "container" => visit_children(child, combine(parent, transform(child)), scene),
            _ => {}
        }
    }
}

fn transform(node: &Xml) -> Transform {
    Transform {
        pos: (num(node, "x"), num(node, "y"), num(node, "z")),
        rot: (
            num(node, "rotationX"),
            num(node, "rotationY"),
            num(node, "rotationZ"),
        ),
        scale: number(node, "scale").unwrap_or(1.0),
    }
}

fn combine(parent: Transform, child: Transform) -> Transform {
    let (pos, rot, scale) = combine_transform(
        parent.pos,
        parent.rot,
        parent.scale,
        child.pos,
        child.rot,
        child.scale,
    );
    Transform { pos, rot, scale }
}

/// Compose a local Away3D position/Euler rotation/scale into its parent space.
pub(crate) fn combine_transform(
    parent_pos: (f32, f32, f32),
    parent_rot: (f32, f32, f32),
    parent_scale: f32,
    child_pos: (f32, f32, f32),
    child_rot: (f32, f32, f32),
    child_scale: f32,
) -> ((f32, f32, f32), (f32, f32, f32), f32) {
    let parent = Transform {
        pos: parent_pos,
        rot: parent_rot,
        scale: parent_scale,
    };
    let child = Transform {
        pos: child_pos,
        rot: child_rot,
        scale: child_scale,
    };
    let offset = rotate(
        quaternion(parent.rot),
        (
            child.pos.0 * parent.scale,
            child.pos.1 * parent.scale,
            child.pos.2 * parent.scale,
        ),
    );
    let rotation = multiply(quaternion(parent.rot), quaternion(child.rot));
    (
        (
            parent.pos.0 + offset.0,
            parent.pos.1 + offset.1,
            parent.pos.2 + offset.2,
        ),
        euler(rotation),
        parent.scale * child.scale,
    )
}

/// Convert Away3D's XYZ Euler rotation to an equivalent axis and angle.
pub(crate) fn rotation_axis_angle(rot: (f32, f32, f32)) -> ((f32, f32, f32), f32) {
    let q = quaternion(rot);
    let angle = (2.0 * q.0.clamp(-1.0, 1.0).acos()).to_degrees();
    let sin = (1.0 - q.0 * q.0).max(0.0).sqrt();
    if sin < 0.0001 {
        ((0.0, 1.0, 0.0), 0.0)
    } else {
        ((q.1 / sin, q.2 / sin, q.3 / sin), angle)
    }
}

type Quat = (f32, f32, f32, f32); // (w, x, y, z)

fn quaternion(rot: (f32, f32, f32)) -> Quat {
    let (sx, cx) = (rot.0.to_radians() * 0.5).sin_cos();
    let (sy, cy) = (rot.1.to_radians() * 0.5).sin_cos();
    let (sz, cz) = (rot.2.to_radians() * 0.5).sin_cos();
    // Z * Y * X, matching the source scene's Euler transform order.
    (
        cz * cy * cx + sz * sy * sx,
        cz * cy * sx - sz * sy * cx,
        cz * sy * cx + sz * cy * sx,
        sz * cy * cx - cz * sy * sx,
    )
}

fn multiply(a: Quat, b: Quat) -> Quat {
    (
        a.0 * b.0 - a.1 * b.1 - a.2 * b.2 - a.3 * b.3,
        a.0 * b.1 + a.1 * b.0 + a.2 * b.3 - a.3 * b.2,
        a.0 * b.2 - a.1 * b.3 + a.2 * b.0 + a.3 * b.1,
        a.0 * b.3 + a.1 * b.2 - a.2 * b.1 + a.3 * b.0,
    )
}

fn rotate(q: Quat, v: (f32, f32, f32)) -> (f32, f32, f32) {
    let u = (q.1, q.2, q.3);
    let dot = u.0 * v.0 + u.1 * v.1 + u.2 * v.2;
    let cross = (
        u.1 * v.2 - u.2 * v.1,
        u.2 * v.0 - u.0 * v.2,
        u.0 * v.1 - u.1 * v.0,
    );
    let factor = 2.0 * dot;
    let wfactor = 2.0 * q.0;
    (
        v.0 * (q.0 * q.0 - u.0 * u.0 - u.1 * u.1 - u.2 * u.2) + u.0 * factor + cross.0 * wfactor,
        v.1 * (q.0 * q.0 - u.0 * u.0 - u.1 * u.1 - u.2 * u.2) + u.1 * factor + cross.1 * wfactor,
        v.2 * (q.0 * q.0 - u.0 * u.0 - u.1 * u.1 - u.2 * u.2) + u.2 * factor + cross.2 * wfactor,
    )
}

fn euler(q: Quat) -> (f32, f32, f32) {
    let (w, x, y, z) = q;
    let roll = (2.0 * (w * x + y * z)).atan2(1.0 - 2.0 * (x * x + y * y));
    let sin_pitch = (2.0 * (w * y - z * x)).clamp(-1.0, 1.0);
    // asin magnifies round-off near its ±1 endpoints: a nominal 90° rotation
    // can round-trip as 89.98°, moving distant children by many world units.
    let pitch = if sin_pitch.abs() >= 0.999_999 {
        std::f32::consts::FRAC_PI_2.copysign(sin_pitch)
    } else {
        sin_pitch.asin()
    };
    let yaw = (2.0 * (w * z + x * y)).atan2(1.0 - 2.0 * (y * y + z * z));
    (roll.to_degrees(), pitch.to_degrees(), yaw.to_degrees())
}

fn attr(node: &Xml, key: &str) -> String {
    node.attrs
        .iter()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value.trim().to_owned())
        .unwrap_or_default()
}

fn number(node: &Xml, key: &str) -> Option<f32> {
    attr(node, key)
        .parse::<f32>()
        .ok()
        .filter(|n| n.is_finite())
}

fn num(node: &Xml, key: &str) -> f32 {
    number(node, key).unwrap_or(0.0)
}
fn integer(node: &Xml, key: &str) -> i32 {
    attr(node, key).parse().unwrap_or_default()
}
fn boolean(node: &Xml, key: &str) -> bool {
    matches!(attr(node, key).as_str(), "true" | "1")
}

#[cfg(test)]
mod tests {
    use crate::assets::uberlib::UberAssets;

    #[test]
    fn nested_container_rotation_and_scale_apply_to_child_position() {
        let xml = r#"<assets><asset id="probe"><display3D><container x="10" rotationY="90" scale="2"><mesh id="nested" geometry="probe" x="0" z="3"/></container></display3D></asset></assets>"#;
        let assets = UberAssets::parse(&[("lib_maps", xml)]).unwrap();
        let scene = assets.map_scene3d("probe").unwrap();
        let p = scene.meshes[0].position;
        assert!((p.0 - 16.0).abs() < 0.001, "x = {p:?}");
        assert!(p.1.abs() < 0.001 && p.2.abs() < 0.001, "y/z = {p:?}");
        assert!((scene.meshes[0].scale - 2.0).abs() < 0.001);
    }

    #[test]
    fn parses_authored_mmo_home_scene_layers() {
        let assets = UberAssets::parse(&[(
            "lib_maps",
            include_str!("../../tests/fixtures/lib_maps.xml"),
        )])
        .unwrap();
        let scene = assets.map_scene3d("lib_maps:map_1-1").unwrap();
        assert_eq!(scene.planes.len(), 2);
        assert_eq!(scene.meshes.len(), 6);
        assert_eq!(scene.tilemaps.len(), 1);
        assert!(scene.planes.iter().any(|p| p.texture == "background1_atf"));
        assert!(scene
            .meshes
            .iter()
            .any(|m| m.geometry == "asset-mmo-phobos"));
    }

    /// `Background3D` — `display3D>lensFlare` rows land in Away3D
    /// `(x, -z, -y)` VO space and `hideLensflare` tilemaps are flagged.
    #[test]
    fn lensflare_rows_and_hide_flag_parse() {
        let xml = r#"<assets><asset id="m"><display3D>
            <container x="100"><lensFlare id="1" typeID="6" x="34000" y="48000" z="26000" star="false" blendMode="add"/></container>
            <tilemap typeID="1" hideLensflare="true"/>
        </display3D></asset></assets>"#;
        let assets = UberAssets::parse(&[("lib_maps", xml)]).unwrap();
        let scene = assets.map_scene3d("m").unwrap();
        assert_eq!(scene.lensflares.len(), 1);
        let f = &scene.lensflares[0];
        assert_eq!(f.type_id, 6);
        assert_eq!(f.position, (34100.0, -26000.0, -48000.0));
        assert!(!f.star && f.blend == "add");
        assert!(scene.hides_lensflares());
    }
}
