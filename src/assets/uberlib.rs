//! `UberAssetsLib` equivalent: `lib_*.xml` databases with template
//! `reference`/`replace`/`append` resolution (`XMLUtils.resolveReferences`).
//!
//! Each library embeds `<template id="x">` blocks and `<asset id="a,b,c">`
//! entries whose `<reference selector="template WHERE id==x">` children
//! splice template content (optionally attribute-patched via `<replace>` /
//! `<append>`) into the asset. After resolution the asset index is
//! `"lib:id"` -> resolved `<asset>` node; bare `id` also resolves
//! (first library wins).
//!
//! `Display2D` extraction understands the three recipe shapes found in the
//! libs:
//! - `<display2D srcKey="ship_goliath" .../>` — attrs on the element itself
//!   (ships, mines; `resKey` defaults to `"mc"`, see `Ship2D.as:364`).
//! - `<display2D><displayObject srcKey="oreRed" resKey="mc" .../></display2D>`
//!   — first usable `<displayObject>` child (collectables, modules).
//! - `<display2D><mapAssetView><textureAtlas .../>...</mapAssetView></display2D>`
//!   — atlas-based views (portals): `atlas_key`/`atlas_texture` carry the
//!   atlas XML file id + region name; `src_key`/`res_key` get the texture
//!   PNG id so callers still have a loadable file.

use anyhow::Result;
use std::collections::HashMap;

pub use super::material::{MaterialDesc, SlotSource, TextureSlot};

/// A resolved `<display2D>` recipe — what to draw for an asset id.
#[derive(Clone, Debug)]
pub struct Display2D {
    /// resources.xml file id (`srcKey` on `display2D` or its `displayObject`s).
    pub src_key: String,
    /// Embedded symbol inside the SWF (`resKey`; default `"mc"`).
    pub res_key: String,
    /// `center="true"` — offset the bitmap by half its size.
    pub center: bool,
    /// `frame` attribute for static movie clips (1-based).
    pub frame: i32,
    /// `autoPlay="false"` freezes on `frame`.
    pub auto_play: bool,
    /// `useBitmapClip="true"` — treat clip frames as independent bitmaps
    /// (rotation strip semantics).
    pub use_bitmap_clip: bool,
    /// y offset.
    pub y: f32,
    pub x: f32,
    /// Content scale `(scaleX, scaleY)` — the `scale` attr sets both axes,
    /// `scaleX`/`scaleY` then override per axis (`UberAsset2D.
    /// applyTransformations` / `UberAnimationBase.parseProps`). Defaults
    /// to `(1, 1)`; any finite value (incl. 0/negative) is literal.
    pub scale: (f32, f32),
    /// `replacement` — low-quality stand-in srcKey (`Ship2D` loads it on
    /// `QUALITY_LOW` instead of `srcKey`).
    pub replacement: String,
    /// `mapAssetView` assets: atlas XML file id (`atlasKey`).
    pub atlas_key: String,
    /// `mapAssetView` assets: texture PNG file id (`textureKey`).
    pub atlas_texture_key: String,
    /// `mapAssetView` base sub-asset region name inside the atlas
    /// (`<asset atlas="portal" texture="normal">`).
    pub atlas_region: String,
    /// `mapAssetView` `animated_base` sub-asset — an atlas region-name
    /// *prefix* animation layered on the base (e.g. portal 306's idle
    /// rotation `portal_306_000` → `portal_306_00000..29` in a second
    /// atlas). "" when the recipe has no animated overlay.
    pub anim_atlas_key: String,
    /// `animated_base`'s atlas `textureKey`.
    pub anim_texture_key: String,
    /// `animated_base`'s `texture` prefix.
    pub anim_prefix: String,
    /// `animated_base`'s `fps`.
    pub anim_fps: f32,
    /// `animated_base`'s x/y.
    pub anim_offset: (f32, f32),
    /// `animated_base`'s `scaleX` (default 1).
    pub anim_scale: f32,
    /// Every `<asset>` of a `mapAssetView`, in document order: `base`,
    /// `highlight`, `active`, `jump`, `spawn_effect`, `animated_base`.
    /// The flat atlas fields above still describe `base` / `animated_base`
    /// for older callers; drawing uses this list so the jump clip and the
    /// other authored layers are not dropped.
    pub map_layers: Vec<MapAssetLayer>,
    /// `<pivot id="click_area" x y>` — parent-space offset of the click
    /// hit area (0,0 when absent). Stock anchors `getClickArea` there.
    pub click_pivot: (f32, f32),
    /// The *other* usable `<displayObject>` children of a multi-object
    /// recipe (`pod_heal`, `sectorControlBeacon`, `battleStation`, …),
    /// in document order — the flat fields above describe the first one.
    /// Each layer carries its own `x`/`y` offset and (container-
    /// multiplied) `scale`; stock stacks them as siblings in the
    /// `display2D` container. Empty for single-object recipes.
    pub layers: Vec<Display2D>,
}

/// A DarkOrbit `display3D` mesh recipe. Geometry ids are Away3D `.awd`
/// collection keys; textures are separate texture collection keys. Keeping
/// the authored recipe intact lets the renderer distinguish a real 3D asset
/// from a 2D sprite fallback.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Display3D {
    pub visual_size: f32,
    pub scale: f32,
    /// `display3D@engineScale` — `Engines3D._engineScale`; `1` = the
    /// thruster views are scaled by ship speed instead (0 = absent).
    pub engine_scale: f32,
    pub meshes: Vec<Mesh3D>,
    pub particles: Vec<Particles3D>,
    /// `<animation id="jump">` particle groups (`gate_jump.zip`,
    /// `portal_305_jump.zip`, …). Played when `Portal.jump` runs; not
    /// part of the idle recipe.
    pub jump_particles: Vec<Particles3D>,
}

/// A `<particles url="x.zip" .../>` child (`UberParticles3D`): a particle group
/// from the separate particle collection placed relative to the asset.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Particles3D {
    pub id: String,
    /// Collection id (`box_2.zip` -> `box_2`).
    pub resource: String,
    pub position: (f32, f32, f32),
    pub rotation: (f32, f32, f32),
    /// Per-axis scale (`scale` then `scaleX/Y/Z` overrides, `applyTransformations`).
    pub scale: (f32, f32, f32),
}

/// One `<asset>` inside a `mapAssetView` (`CreateMapAssetViewCommand`).
///
/// `texture` is a `SubTexture` name or a frame-stem prefix. An empty
/// texture selects every region in the atlas — that is how `portal_jump`
/// plays `portalAnim0001..` (`DOAtlas.getTextures("")`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MapAssetLayer {
    pub id: String,
    pub atlas_key: String,
    pub texture_key: String,
    pub texture: String,
    pub x: f32,
    pub y: f32,
    pub scale: (f32, f32),
    /// Authored `alpha` (1 when absent). `highlight` starts at 0 and is
    /// driven by the looping fade tween.
    pub alpha: f32,
    /// Authored `visible`. `false` / `"0"` hides `active` and `jump`
    /// until `jumpOn`.
    pub visible: bool,
    /// Authored `fps`. 0 means the movie-clip default (30) when the
    /// texture prefix matches more than one region.
    pub fps: f32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Mesh3D {
    pub id: String,
    pub geometry: String,
    /// Mirror of `material.texture` (`@diffuse || @texture`).
    pub texture: String,
    /// Mirror of `material.texture_settings` (`@tex_settings`).
    pub texture_settings: String,
    /// Full Away3D material descriptor — per-slot texture sources and
    /// scalar params (`PrefabMaterialManager`/`setShaderParams` input).
    pub material: MaterialDesc,
    pub position: (f32, f32, f32),
    pub rotation: (f32, f32, f32),
    pub scale: f32,
    pub visual_size: f32,
}

/// Client-local 3D texture quality tier (stock `displaySetting3DsizeTextures`
/// + `displaySetting3DqualityTextures`, folded into one Low/Medium/High
/// setting). Drives both the `{base}_{slot}_{size}` file variant and which
/// material layers are loaded.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum TextureQuality {
    /// Smallest size tier; diffuse/alpha/gal layers only.
    Low,
    /// Medium size tier; adds the `glow` emissive layer.
    Medium,
    /// Largest size tier; adds `normal`/`specular`/`ao` layers.
    #[default]
    High,
}

impl TextureQuality {
    pub const ALL: [Self; 3] = [Self::Low, Self::Medium, Self::High];

    pub fn label(self) -> &'static str {
        match self {
            Self::Low => "Low",
            Self::Medium => "Medium",
            Self::High => "High",
        }
    }
}

/// Diffuse-layer-at-High helper: the `resources_3d` id
/// `PrefabMaterialManager.assembleTextureResKey` builds for the diffuse
/// slot at `TextureQuality::High` with `sizeMultiplier = 1`. Kept for
/// callers that pre-request only the diffuse file; new code should use
/// `MaterialDesc::supported_slots` + `MaterialDesc::texture_key`.
pub fn resolve_3d_texture_id(texture: &str, geometry: &str, settings: &str) -> Option<String> {
    MaterialDesc {
        texture: texture.to_string(),
        texture_settings: settings.to_string(),
        texture_base: texture.to_string(),
        ..MaterialDesc::default()
    }
    .texture_key(TextureSlot::Diffuse, geometry, TextureQuality::High, 1)
}

impl Default for Display2D {
    fn default() -> Self {
        Self {
            src_key: String::new(),
            res_key: String::new(),
            center: false,
            frame: 0,
            auto_play: false,
            use_bitmap_clip: false,
            y: 0.0,
            x: 0.0,
            scale: (1.0, 1.0),
            replacement: String::new(),
            atlas_key: String::new(),
            atlas_texture_key: String::new(),
            atlas_region: String::new(),
            anim_atlas_key: String::new(),
            anim_texture_key: String::new(),
            anim_prefix: String::new(),
            anim_fps: 0.0,
            anim_offset: (0.0, 0.0),
            anim_scale: 1.0,
            map_layers: Vec::new(),
            click_pivot: (0.0, 0.0),
            layers: Vec::new(),
        }
    }
}

/// Parsed non-display facets of an asset node.
#[derive(Clone, Debug, Default)]
pub struct UberNode {
    /// `<click_area radius>` — hit-test radius (0 when absent).
    pub click_radius: i32,
    /// `<miniMapIcon resKey>` — minimap marker symbol.
    pub minimap_icon: Option<String>,
    /// `<miniMapIcon color>` — decimal RGB value.
    pub minimap_color: Option<u32>,
    /// `<label x y>` offset (0,0 when absent).
    pub label_offset: (i32, i32),
    /// `<hp>` element present.
    pub has_hp_bar: bool,
    /// `<hp width>`.
    pub hp_bar_width: i32,
}

// ---------------------------------------------------------------------------
// Minimal owned XML tree (roxmltree nodes borrow; we need stored, mutable
// trees for template resolution).
// ---------------------------------------------------------------------------

/// Owned XML element tree — mutable for `resolveReferences`.
#[derive(Clone, Debug)]
pub struct Xml {
    pub tag: String,
    pub attrs: Vec<(String, String)>,
    pub children: Vec<Xml>,
    /// Concatenated text content (unused for attrs but kept for `text()`).
    pub text: String,
}

impl Xml {
    fn from_roxml(node: roxmltree::Node) -> Xml {
        let mut children = Vec::new();
        let mut text = String::new();
        for c in node.children() {
            if c.is_element() {
                children.push(Xml::from_roxml(c));
            } else if c.is_text() {
                text.push_str(c.text().unwrap_or_default());
            }
        }
        Xml {
            tag: node.tag_name().name().to_string(),
            attrs: node
                .attributes()
                .map(|a| (a.name().to_string(), a.value().to_string()))
                .collect(),
            children,
            text,
        }
    }

    pub(crate) fn attr(&self, name: &str) -> &str {
        self.attrs
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
            .unwrap_or("")
    }

    fn set_attr(&mut self, name: &str, value: &str) {
        if let Some(e) = self.attrs.iter_mut().find(|(n, _)| n == name) {
            e.1 = value.to_string();
        } else {
            self.attrs.push((name.to_string(), value.to_string()));
        }
    }

    fn children_named<'a, 'b>(&'a self, tag: &'b str) -> impl Iterator<Item = &'a Xml> + 'b
    where
        'a: 'b,
    {
        self.children.iter().filter(move |c| c.tag.as_str() == tag)
    }

    fn child(&self, tag: &str) -> Option<&Xml> {
        self.children_named(tag).next()
    }
}

// ---------------------------------------------------------------------------
// `XMLUtils.selectNode` — `a>b>c WHERE attr==value` selector, "this" special.
// ---------------------------------------------------------------------------

fn parse_selector(selector: &str) -> (Vec<String>, Option<(String, String)>) {
    let (path, cond) = match selector.split_once("WHERE") {
        Some((p, c)) => (p, Some(c)),
        None => (selector, None),
    };
    let steps: Vec<String> = path
        .split('>')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    let cond = cond.and_then(|c| {
        let c = c.trim();
        if c == "*" || c.is_empty() {
            return None;
        }
        c.split_once("==")
            .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
    });
    (steps, cond)
}

/// `selectNode(root, selector, false)` — child-chain select + WHERE filter.
fn select<'a>(root: &'a Xml, selector: &str) -> Vec<&'a Xml> {
    let selector = selector.trim();
    if selector == "this" {
        return vec![root];
    }
    let (steps, cond) = parse_selector(selector);
    let mut current: Vec<&Xml> = vec![root];
    for step in &steps {
        let mut next = Vec::new();
        for node in &current {
            next.extend(node.children_named(step));
        }
        current = next;
        if current.is_empty() {
            break;
        }
    }
    current
        .into_iter()
        .filter(|n| match &cond {
            Some((k, v)) => n.attr(k) == v,
            None => true,
        })
        .collect()
}

/// `processReference` — apply `<replace>`/`<append>` ops of `reference`
/// onto a copy of `selected`.
fn process_reference(selected: &Xml, reference: &Xml) -> Xml {
    let mut out = selected.clone();
    for rep in reference.children_named("replace") {
        let selector = rep.attr("selector").to_string();
        let attrs: Vec<String> = rep
            .attr("attributes")
            .split(',')
            .map(|s| s.trim().to_string())
            .collect();
        let values: Vec<String> = rep
            .attr("values")
            .split(',')
            .map(|s| s.trim().to_string())
            .collect();
        for target in select_mut(&mut out, &selector) {
            for (a, v) in attrs.iter().zip(values.iter()) {
                target.set_attr(a, v);
            }
        }
    }
    for app in reference.children_named("append") {
        let selector = app.attr("selector").to_string();
        let additions: Vec<Xml> = app.children.clone();
        for target in select_mut(&mut out, &selector) {
            target.children.extend(additions.iter().cloned());
        }
    }
    out
}

/// Mutable variant of `select` — same traversal, yields `&mut Xml`.
fn select_mut<'a>(root: &'a mut Xml, selector: &str) -> Vec<&'a mut Xml> {
    let selector = selector.trim();
    if selector == "this" {
        return vec![root];
    }
    let (steps, cond) = parse_selector(selector);
    let mut current: Vec<&mut Xml> = vec![root];
    for step in &steps {
        let mut next = Vec::new();
        for node in current {
            next.extend(node.children.iter_mut().filter(|c| &c.tag == step));
        }
        current = next;
        if current.is_empty() {
            break;
        }
    }
    current
        .into_iter()
        .filter(|n| match &cond {
            Some((k, v)) => n.attr(k) == v,
            None => true,
        })
        .collect()
}

/// Snapshot all `<reference>` element paths in document order.
fn reference_paths(root: &Xml) -> Vec<Vec<usize>> {
    fn walk(node: &Xml, path: &mut Vec<usize>, out: &mut Vec<Vec<usize>>) {
        for (i, c) in node.children.iter().enumerate() {
            path.push(i);
            if c.tag == "reference" {
                out.push(path.clone());
            } else {
                walk(c, path, out);
            }
            path.pop();
        }
    }
    let mut out = Vec::new();
    walk(root, &mut Vec::new(), &mut out);
    out
}

fn node_at_mut<'a>(root: &'a mut Xml, path: &[usize]) -> Option<&'a mut Xml> {
    let mut cur = root;
    for &i in path {
        cur = cur.children.get_mut(i)?;
    }
    Some(cur)
}

fn node_at<'a>(root: &'a Xml, path: &[usize]) -> Option<&'a Xml> {
    let mut cur = root;
    for &i in path {
        cur = cur.children.get(i)?;
    }
    Some(cur)
}

/// `XMLUtils.resolveReferences` — process `descendants("reference")` in
/// REVERSE document order (paths stay valid: deletions only touch indices
/// after the processed node, splices append at parent end):
/// for each `<reference>`, select nodes within the whole doc, copy + patch
/// them, splice into the reference's parent (`children="false"` splices the
/// copy itself, default splices the copy's children), then delete the ref.
fn resolve_references(root: &mut Xml) {
    let mut paths = reference_paths(root);
    paths.reverse();
    for path in paths {
        let (parent_path, _) = path.split_at(path.len() - 1);
        // Read the reference element (immutable borrow first).
        let Some(reference) = node_at(root, &path).cloned() else {
            continue;
        };
        let selector = reference.attr("selector").to_string();
        let children_mode = reference.attr("children") != "false";
        // Selected nodes are copied, so immutable borrows end before splice.
        let copies: Vec<Xml> = select(root, &selector)
            .iter()
            .map(|n| process_reference(n, &reference))
            .collect();
        if copies.is_empty() {
            continue; // selector matched nothing — reference stays (AS3 too)
        }
        // Re-fetch paths: the doc may have mutated since the snapshot — the
        // path for THIS reference is still valid (nothing before it moved;
        // earlier-visited deletions only affected later siblings).
        let Some(parent) = node_at_mut(root, parent_path) else {
            continue;
        };
        for copy in copies {
            if children_mode {
                parent.children.extend(copy.children);
            } else {
                parent.children.push(copy);
            }
        }
        // Delete the reference element itself.
        if let Some(&idx) = path.last() {
            if idx < parent.children.len() {
                parent.children.remove(idx);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// UberAssets
// ---------------------------------------------------------------------------

/// The whole uber-asset database: `lib:assetId` -> resolved asset node +
/// extracted trait metadata; bare `assetId` -> composite key (first lib in
/// `libs` order wins — AS3 `getDescriptor(id)` searches per-library, our
/// `display_2d` searches all).
#[derive(Default)]
pub struct UberAssets {
    /// "lib:id" -> resolved `<asset>` node.
    assets: HashMap<String, Xml>,
    /// bare id -> "lib:id" composite (first-seen wins).
    by_id: HashMap<String, String>,
    /// "lib:id" -> trait summary for [`Self::node`].
    nodes: HashMap<String, UberNode>,
    /// Authored map-scene compositions from `lib_maps`.
    map_scenes: HashMap<String, super::mapscene::MapScene3D>,
}

impl UberAssets {
    /// Build from the embedded library documents (`lib_ships`,
    /// `lib_portals`, `lib_collectables`, `lib_maps`, `lib_drones`,
    /// `lib_default`, `lib_battlestation`). `libs` is `(lib_name, xml_text)`
    /// pairs; resolution (`resolveReferences`) + comma-id splitting happen
    /// here (`UberAssetsLib.addDefinitions`).
    pub fn parse(libs: &[(&str, &str)]) -> Result<Self> {
        let mut uber = UberAssets::default();
        for (lib_name, xml_text) in libs {
            let doc = match roxmltree::Document::parse(xml_text) {
                Ok(d) => d,
                Err(e) => {
                    log::warn!("uberlib: {lib_name} parse failed: {e}");
                    continue;
                }
            };
            let mut root = Xml::from_roxml(doc.root_element());
            resolve_references(&mut root);
            for asset in &root.children {
                if asset.tag != "asset" {
                    continue;
                }
                let id_attr = asset.attr("id");
                if id_attr.is_empty() {
                    continue;
                }
                for id in id_attr.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                    let key = format!("{lib_name}:{id}");
                    uber.assets.insert(key.clone(), asset.clone());
                    uber.nodes.insert(key.clone(), extract_traits(asset));
                    if *lib_name == "lib_maps" {
                        if let Some(scene) = super::mapscene::parse(id, asset) {
                            uber.map_scenes.insert(key.clone(), scene);
                        }
                    }
                    uber.by_id.entry(id.to_string()).or_insert(key);
                }
            }
        }
        Ok(uber)
    }

    /// Composite key for `asset_id` — accepts `id` (bare, first lib wins)
    /// or `lib:id`. `UberAsset2D`/`UberAsset3D` disagree on the colon
    /// order (`id:lib` vs `lib:id`), so the swapped form is tried too.
    fn composite(&self, asset_id: &str) -> Option<String> {
        if let Some((a, b)) = asset_id.split_once(':') {
            if self.assets.contains_key(asset_id) {
                return Some(asset_id.to_string());
            }
            let swapped = format!("{b}:{a}");
            if self.assets.contains_key(&swapped) {
                return Some(swapped);
            }
            return None;
        }
        self.by_id.get(asset_id).cloned()
    }

    /// Resolved asset XML node (raw, for trait consumers beyond `UberNode`).
    pub fn asset_node(&self, asset_id: &str) -> Option<&Xml> {
        let key = self.composite(asset_id)?;
        self.assets.get(&key)
    }

    /// Resolved `display2D` recipe for `assetId` (searches all libraries).
    pub fn display_2d(&self, asset_id: &str) -> Option<Display2D> {
        self.display_2d_inner(asset_id, 0)
    }

    /// Resolved native Away3D recipe. Geometry ids normally resolve to an
    /// `.awd` file through the game's file collection; unlike `display2D`,
    /// this is not a raster clip and must be rendered by a 3D mesh backend.
    pub fn display_3d(&self, asset_id: &str) -> Option<Display3D> {
        self.display_3d_inner(asset_id, 0)
    }

    /// Authored map background scene (`lib_maps:map_1-1`, etc.), including
    /// planes, geometry, and the tilemap layers the renderer may not support.
    pub fn map_scene3d(&self, asset_id: &str) -> Option<&super::mapscene::MapScene3D> {
        let key = self.composite(asset_id)?;
        self.map_scenes.get(&key)
    }

    fn display_3d_inner(&self, asset_id: &str, depth: u8) -> Option<Display3D> {
        if depth > 4 {
            return None;
        }
        let asset = self.asset_node(asset_id)?;
        let d3d = asset.child("display3D").or_else(|| {
            asset
                .children
                .iter()
                .flat_map(|c| c.children.iter())
                .find(|c| c.tag == "display3D")
        })?;
        let template = d3d.attr("templateId");
        if !template.is_empty() {
            if let Some(d) = self.display_3d_inner(template, depth + 1) {
                return Some(d);
            }
        }
        let mut out = Display3D {
            visual_size: opt_f_attr(d3d, "visualSize").unwrap_or(1.0),
            scale: opt_f_attr(d3d, "scale").unwrap_or(1.0),
            engine_scale: opt_f_attr(d3d, "engineScale")
                .filter(|v| *v != 0.0)
                .unwrap_or(1.0),
            meshes: Vec::new(),
            particles: Vec::new(),
            jump_particles: Vec::new(),
        };
        let parent_pos = (f_attr(d3d, "x"), f_attr(d3d, "y"), f_attr(d3d, "z"));
        let parent_rot = (
            f_attr(d3d, "rotationX"),
            f_attr(d3d, "rotationY"),
            f_attr(d3d, "rotationZ"),
        );
        // `UberAsset3D.parseDisplayTree`: a `<display3D geometry="...">` is
        // itself the mesh (e.g. `saimon-boss`); its own transform and scale
        // are the recipe's, with no child `<mesh>` nodes.
        let root_geometry = clean_attr(d3d.attr("geometry"));
        if !root_geometry.is_empty() {
            // The material descriptor lives on the `<display3D>` itself.
            let material = MaterialDesc::parse(d3d);
            out.meshes.push(Mesh3D {
                id: clean_attr(d3d.attr("id")),
                geometry: root_geometry,
                texture: material.texture.clone(),
                texture_settings: material.texture_settings.clone(),
                material,
                position: parent_pos,
                rotation: parent_rot,
                scale: 1.0,
                visual_size: out.visual_size,
            });
        }
        for mesh in d3d.children.iter().filter(|n| n.tag == "mesh") {
            let geometry = clean_attr(mesh.attr("geometry"));
            if geometry.is_empty() || geometry.contains('%') {
                continue;
            }
            let visual_size = opt_f_attr(mesh, "visualSize").unwrap_or(out.visual_size);
            if out.meshes.is_empty() && opt_f_attr(d3d, "visualSize").is_none() {
                out.visual_size = visual_size;
            }
            let mesh_pos = (f_attr(mesh, "x"), f_attr(mesh, "y"), f_attr(mesh, "z"));
            let mesh_rot = (
                f_attr(mesh, "rotationX"),
                f_attr(mesh, "rotationY"),
                f_attr(mesh, "rotationZ"),
            );
            let mesh_scale = opt_f_attr(mesh, "scale").unwrap_or(1.0);
            let (position, rotation, _) = super::mapscene::combine_transform(
                parent_pos, parent_rot, out.scale, mesh_pos, mesh_rot, mesh_scale,
            );
            // `PrefabMaterialManager` reads the whole descriptor off the
            // mesh node; `material.texture` still folds `diffuse` in so the
            // legacy `texture` field keys cached models per variant.
            let material = MaterialDesc::parse(mesh);
            out.meshes.push(Mesh3D {
                id: clean_attr(mesh.attr("id")),
                geometry,
                texture: material.texture.clone(),
                texture_settings: material.texture_settings.clone(),
                material,
                position,
                rotation,
                scale: mesh_scale,
                visual_size,
            });
        }
        // Named `<container>`s (`jumpEffect`, `idleEffect`) are the parents
        // `UberAnimation3D` adds particle groups under.
        let mut containers: HashMap<String, ((f32, f32, f32), (f32, f32, f32), f32)> =
            HashMap::new();
        for node in d3d.children.iter().filter(|n| n.tag == "container") {
            let id = clean_attr(node.attr("id"));
            if id.is_empty() {
                continue;
            }
            let local_pos = (f_attr(node, "x"), f_attr(node, "y"), f_attr(node, "z"));
            let local_rot = (
                f_attr(node, "rotationX"),
                f_attr(node, "rotationY"),
                f_attr(node, "rotationZ"),
            );
            let child_scale = opt_f_attr(node, "scale").unwrap_or(1.0);
            let (position, rotation, scale) = super::mapscene::combine_transform(
                parent_pos,
                parent_rot,
                out.scale,
                local_pos,
                local_rot,
                child_scale,
            );
            containers.insert(id, (position, rotation, scale));
        }
        for node in d3d.children.iter().filter(|n| n.tag == "particles") {
            if let Some(p) = bake_particle(node, parent_pos, parent_rot, out.scale) {
                out.particles.push(p);
            }
        }
        // `<animation>` particle children. autoPlay groups (idle light,
        // kronos ambience) stay on; `id="jump"` plays with `Portal.jump`.
        for anim in d3d.children.iter().filter(|n| n.tag == "animation") {
            let jump = anim.attr("id") == "jump";
            let auto = anim.attr("autoPlay") == "true";
            if !jump && !auto {
                continue;
            }
            for node in anim.children.iter().filter(|n| n.tag == "particles") {
                let parent_id = clean_attr(node.attr("parent"));
                let (pos, rot, scale) = containers
                    .get(&parent_id)
                    .copied()
                    .unwrap_or((parent_pos, parent_rot, out.scale));
                let Some(p) = bake_particle(node, pos, rot, scale) else {
                    continue;
                };
                if jump {
                    out.jump_particles.push(p);
                } else {
                    out.particles.push(p);
                }
            }
        }
        (!out.meshes.is_empty() || !out.particles.is_empty() || !out.jump_particles.is_empty())
            .then_some(out)
    }

    fn display_2d_inner(&self, asset_id: &str, depth: u8) -> Option<Display2D> {
        if depth > 4 {
            return None;
        }
        let asset = self.asset_node(asset_id)?;
        let d2d = asset.child("display2D").or_else(|| {
            asset
                .children
                .iter()
                .flat_map(|c| c.children.iter())
                .find(|c| c.tag == "display2D")
        })?;
        // `<display2D templateId="lib:id"/>` — nested template expansion
        // (`UberAsset2D.parseDisplay2D` handles `id:lib`/`lib:id` forms).
        if let Some(t) = d2d
            .attrs
            .iter()
            .find(|(n, _)| n == "templateId")
            .map(|(_, v)| v.clone())
        {
            if let Some(d) = self.display_2d_inner(&t, depth + 1) {
                return Some(d);
            }
        }
        Some(extract_display2d(d2d))
    }

    /// Raw resolved trait summary — `click_area`, `miniMapIcon`, `label`,
    /// `hp`, `attack_target` facets.
    pub fn node(&self, asset_id: &str) -> Option<&UberNode> {
        let key = self.composite(asset_id)?;
        self.nodes.get(&key)
    }

    /// `<pivot id=… x y>` child of the asset's `<display2D>` — e.g. the
    /// `dock_N` mounts of a mothership (`MotherShipTrait.updateMinionsPositions`
    /// = `owner.pos + getPivot("dock_N")`). `None` when absent.
    pub fn display_pivot(&self, asset_id: &str, pivot_id: &str) -> Option<(f32, f32)> {
        let asset = self.asset_node(asset_id)?;
        let d2d = asset.child("display2D")?;
        let p = d2d
            .children_named("pivot")
            .find(|p| p.attr("id") == pivot_id)?;
        Some((f_attr(p, "x"), f_attr(p, "y")))
    }

    /// Number of indexed asset ids (all libraries).
    pub fn len(&self) -> usize {
        self.assets.len()
    }

    pub fn is_empty(&self) -> bool {
        self.assets.is_empty()
    }
}

/// `%placeholder%` attributes that survived resolution count as empty.
pub(crate) fn clean_attr(s: &str) -> String {
    if s.starts_with('%') && s.ends_with('%') {
        String::new()
    } else {
        s.to_string()
    }
}

fn bool_attr(node: &Xml, name: &str, default: bool) -> bool {
    match node.attr(name) {
        "true" => true,
        "false" => false,
        _ => default,
    }
}

/// `visible` on map-asset views: absent stays at `default`; `"0"` is the
/// same as `false` (`jump` / `active` are authored that way).
fn vis_attr(node: &Xml, name: &str, default: bool) -> bool {
    match node.attr(name).trim() {
        "" => default,
        "true" | "1" => true,
        "false" | "0" => false,
        _ => default,
    }
}

/// Place one `<particles>` node in `parent_*` space. `includeIn` still
/// marks a state we do not track, so those nodes are skipped.
fn bake_particle(
    node: &Xml,
    parent_pos: (f32, f32, f32),
    parent_rot: (f32, f32, f32),
    parent_scale: f32,
) -> Option<Particles3D> {
    let url = clean_attr(node.attr("url"));
    if url.is_empty() || !node.attr("includeIn").is_empty() {
        return None;
    }
    let local_pos = (f_attr(node, "x"), f_attr(node, "y"), f_attr(node, "z"));
    let local_rot = (
        f_attr(node, "rotationX"),
        f_attr(node, "rotationY"),
        f_attr(node, "rotationZ"),
    );
    let uniform = opt_f_attr(node, "scale").unwrap_or(1.0);
    let (position, rotation, _) = super::mapscene::combine_transform(
        parent_pos,
        parent_rot,
        parent_scale,
        local_pos,
        local_rot,
        1.0,
    );
    Some(Particles3D {
        id: clean_attr(node.attr("id")),
        resource: super::particles::resource_id(&url),
        position,
        rotation,
        scale: (
            opt_f_attr(node, "scaleX").unwrap_or(uniform) * parent_scale,
            opt_f_attr(node, "scaleY").unwrap_or(uniform) * parent_scale,
            opt_f_attr(node, "scaleZ").unwrap_or(uniform) * parent_scale,
        ),
    })
}

fn i_attr(node: &Xml, name: &str) -> i32 {
    node.attr(name).trim().parse().unwrap_or_default()
}

fn f_attr(node: &Xml, name: &str) -> f32 {
    node.attr(name).trim().parse().unwrap_or_default()
}

/// Optional finite float attr — `Some(v)` only when present and numeric
/// (any finite value is literal, negative/0 included). Absent/NaN → `None`.
pub(crate) fn opt_f_attr(node: &Xml, name: &str) -> Option<f32> {
    let v = node.attr(name);
    if v.is_empty() {
        return None;
    }
    v.trim().parse::<f32>().ok().filter(|f| f.is_finite())
}

/// Fill `Display2D` scalar attrs from one element (`display2D` or
/// `displayObject`). `resKey` defaults to `"mc"` (`Ship2D.as:364`).
fn attrs_to_display(node: &Xml, d: &mut Display2D) {
    d.src_key = clean_attr(node.attr("srcKey"));
    d.res_key = {
        let r = clean_attr(node.attr("resKey"));
        if r.is_empty() {
            "mc".to_string()
        } else {
            r
        }
    };
    d.replacement = clean_attr(node.attr("replacement"));
    d.center = bool_attr(node, "center", d.center);
    d.frame = i_attr(node, "frame");
    d.auto_play = bool_attr(node, "autoPlay", true);
    d.use_bitmap_clip = bool_attr(node, "useBitmapClip", false);
    d.x = f_attr(node, "x");
    d.y = f_attr(node, "y");
    // `scale` sets both axes, then `scaleX`/`scaleY` override per axis
    // (`UberAnimationBase.parseProps`). Absent -> untouched.
    if let Some(s) = opt_f_attr(node, "scale") {
        d.scale = (s, s);
    }
    if let Some(s) = opt_f_attr(node, "scaleX") {
        d.scale.0 = s;
    }
    if let Some(s) = opt_f_attr(node, "scaleY") {
        d.scale.1 = s;
    }
}

/// Pull a usable recipe out of one `<display2D>` element.
fn extract_display2d(d2d: &Xml) -> Display2D {
    let mut d = Display2D::default();
    attrs_to_display(d2d, &mut d);
    if d.src_key.is_empty() {
        // `<displayObject>` children carry the recipe (collectables,
        // modules, ...). The first usable non-empty srcKey is the main
        // object; further usable ones become `layers`.
        let objs: Vec<&Xml> = d2d
            .children
            .iter()
            .filter(|c| c.tag == "displayObject")
            .collect();
        let usable = |o: &Xml| !clean_attr(o.attr("srcKey")).is_empty();
        let main_idx =
            objs.iter()
                .position(|o| usable(o))
                .or(if objs.is_empty() { None } else { Some(0) });
        if let Some(mi) = main_idx {
            // Container chain: the child sits inside the display2D
            // container, so its scale multiplies the element's (a child
            // without scale attrs contributes (1,1)).
            let parent_scale = d.scale;
            d.scale = (1.0, 1.0);
            attrs_to_display(objs[mi], &mut d);
            d.scale = (d.scale.0 * parent_scale.0, d.scale.1 * parent_scale.1);
            for (i, obj) in objs.iter().enumerate() {
                if i == mi || !usable(obj) {
                    continue;
                }
                let mut layer = Display2D::default();
                attrs_to_display(obj, &mut layer);
                layer.scale = (
                    layer.scale.0 * parent_scale.0,
                    layer.scale.1 * parent_scale.1,
                );
                d.layers.push(layer);
            }
        }
    }
    if d.src_key.is_empty() {
        // `<mapAssetView>` atlas recipes (portals): atlas XML + texture PNG
        // + `<asset atlas= texture=>` regions. `src_key` gets the texture
        // PNG file id — whole-atlas decode until the renderer learns
        // regions; `atlas_*` keeps the real metadata.
        if let Some(view) = d2d.children.iter().find(|c| c.tag == "mapAssetView") {
            // Atlases are keyed by `id`; sub-assets reference them via
            // `atlas` and carry the `texture` region name. The "base"
            // sub-asset (or the first with a non-empty texture) supplies
            // the display region.
            let atlases: Vec<&Xml> = view
                .children
                .iter()
                .filter(|c| c.tag == "textureAtlas")
                .collect();
            let base = view
                .children
                .iter()
                .find(|c| c.tag == "assets")
                .and_then(|assets| {
                    assets
                        .children
                        .iter()
                        .find(|c| c.tag == "asset" && c.attr("id") == "base")
                        .or_else(|| {
                            assets
                                .children
                                .iter()
                                .find(|c| c.tag == "asset" && !c.attr("texture").is_empty())
                        })
                });
            if let Some(sub) = base {
                d.atlas_region = clean_attr(sub.attr("texture"));
                d.x = f_attr(sub, "x");
                d.y = f_attr(sub, "y");
                let atlas = atlases
                    .iter()
                    .find(|a| a.attr("id") == sub.attr("atlas"))
                    .or_else(|| atlases.first());
                if let Some(atlas) = atlas {
                    d.atlas_key = clean_attr(atlas.attr("atlasKey"));
                    d.atlas_texture_key = clean_attr(atlas.attr("textureKey"));
                }
            } else if let Some(atlas) = atlases.first() {
                d.atlas_key = clean_attr(atlas.attr("atlasKey"));
                d.atlas_texture_key = clean_attr(atlas.attr("textureKey"));
            }
            // `animated_base` — prefix-matched atlas animation over the
            // base (may use a different atlas, e.g. `idleEffect`).
            if let Some(anim) =
                view.children
                    .iter()
                    .find(|c| c.tag == "assets")
                    .and_then(|assets| {
                        assets.children.iter().find(|c| {
                            c.tag == "asset"
                                && c.attr("id") == "animated_base"
                                && !c.attr("texture").is_empty()
                        })
                    })
            {
                d.anim_prefix = clean_attr(anim.attr("texture"));
                d.anim_fps = f_attr(anim, "fps");
                d.anim_offset = (f_attr(anim, "x"), f_attr(anim, "y"));
                let sx = f_attr(anim, "scaleX");
                d.anim_scale = if sx > 0.0 { sx } else { 1.0 };
                let atlas = atlases
                    .iter()
                    .find(|a| a.attr("id") == anim.attr("atlas"))
                    .or_else(|| atlases.first());
                if let Some(atlas) = atlas {
                    d.anim_atlas_key = clean_attr(atlas.attr("atlasKey"));
                    d.anim_texture_key = clean_attr(atlas.attr("textureKey"));
                }
            }
            if !d.atlas_texture_key.is_empty() {
                d.src_key = d.atlas_texture_key.clone();
                d.res_key = d.atlas_region.clone();
            }
            // Every authored sub-asset, not just `base`. `jump` with an
            // empty texture is the shared portal-jump movie clip.
            if let Some(assets) = view.children.iter().find(|c| c.tag == "assets") {
                for sub in assets.children.iter().filter(|c| c.tag == "asset") {
                    let atlas = atlases.iter().find(|a| a.attr("id") == sub.attr("atlas"));
                    let Some(atlas) = atlas else {
                        continue;
                    };
                    let atlas_key = clean_attr(atlas.attr("atlasKey"));
                    let texture_key = clean_attr(atlas.attr("textureKey"));
                    if atlas_key.is_empty() || texture_key.is_empty() {
                        continue;
                    }
                    let uniform = opt_f_attr(sub, "scale");
                    d.map_layers.push(MapAssetLayer {
                        id: clean_attr(sub.attr("id")),
                        atlas_key,
                        texture_key,
                        texture: clean_attr(sub.attr("texture")),
                        x: f_attr(sub, "x"),
                        y: f_attr(sub, "y"),
                        scale: (
                            opt_f_attr(sub, "scaleX").or(uniform).unwrap_or(1.0),
                            opt_f_attr(sub, "scaleY").or(uniform).unwrap_or(1.0),
                        ),
                        alpha: opt_f_attr(sub, "alpha").unwrap_or(1.0),
                        visible: vis_attr(sub, "visible", true),
                        fps: f_attr(sub, "fps"),
                    });
                }
            }
            // `mapAssetView` recipes don't take `scale` — the atlas
            // regions draw at native size (`scaleX` on `animated_base`
            // is tracked separately in `anim_scale`).
            d.scale = (1.0, 1.0);
        }
    }
    // `<pivot id="click_area" x y>` — stock anchors the click hit area
    // on that pivot (`UberAsset2D`), offset from the anchor point.
    if let Some(p) = d2d
        .children
        .iter()
        .find(|c| c.tag == "pivot" && c.attr("id") == "click_area")
    {
        d.click_pivot = (f_attr(p, "x"), f_attr(p, "y"));
    }
    d
}

/// `getTraits`-adjacent facet extraction for [`UberAssets::node`].
fn extract_traits(asset: &Xml) -> UberNode {
    let find = |tag: &str| -> Option<&Xml> {
        let mut stack: Vec<&Xml> = asset.children.iter().collect();
        while let Some(n) = stack.pop() {
            if n.tag == tag {
                return Some(n);
            }
            stack.extend(n.children.iter());
        }
        None
    };
    let click = find("click_area");
    let minimap = find("miniMapIcon");
    let hp = find("hp");
    let label = find("label").or_else(|| find("pivot").filter(|p| p.attr("id") == "label"));
    UberNode {
        click_radius: click.map(|c| i_attr(c, "radius")).unwrap_or_default(),
        minimap_icon: minimap.and_then(|m| {
            let r = m.attr("resKey");
            if r.is_empty() {
                None
            } else {
                Some(r.to_string())
            }
        }),
        minimap_color: minimap.and_then(|m| {
            let v = m.attr("color");
            if v.is_empty() {
                None
            } else if let Some(hex) = v.strip_prefix("0x") {
                u32::from_str_radix(hex, 16).ok()
            } else {
                v.parse().ok()
            }
        }),
        label_offset: label
            .map(|l| (i_attr(l, "x"), i_attr(l, "y")))
            .unwrap_or_default(),
        has_hp_bar: hp.is_some(),
        hp_bar_width: hp.map(|h| i_attr(h, "width")).unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn fixture(name: &str) -> String {
        std::fs::read_to_string(Path::new("tests/fixtures").join(name))
            .unwrap_or_else(|e| panic!("cannot read {name}: {e}"))
    }

    fn uber_with(libs: &[(&str, &str)]) -> UberAssets {
        let owned: Vec<(String, String)> = libs
            .iter()
            .map(|(lib, file)| (lib.to_string(), fixture(file)))
            .collect();
        let refs: Vec<(&str, &str)> = owned
            .iter()
            .map(|(n, t)| (n.as_str(), t.as_str()))
            .collect();
        UberAssets::parse(&refs).expect("uber libs parse")
    }

    #[test]
    fn scale_attr_sets_both_axes() {
        let uber = uber_with(&[("lib_collectables", "lib_collectables.xml")]);
        // `replace attributes="srcKey,scale" values="box_booty_astral,0.2"`
        // — the displayObject carries scale="0.2".
        let d = uber
            .display_2d("box_ASTRAL_BOOTY_BOX")
            .expect("box_ASTRAL_BOOTY_BOX");
        assert_eq!(d.src_key, "box_booty_astral");
        assert_eq!(d.scale, (0.2, 0.2));
        // No scale attr anywhere -> (1, 1).
        let d = uber.display_2d("box_BONUS_BOX").expect("box_BONUS_BOX");
        assert_eq!(d.scale, (1.0, 1.0));
    }

    #[test]
    fn extracts_authored_away3d_ship_mesh_recipe() {
        let uber = uber_with(&[("lib_ships", "lib_ships.xml")]);
        let d = uber.display_3d("ship_phoenix").expect("Phoenix 3D recipe");
        assert!((d.visual_size - 0.6).abs() < 0.001);
        assert_eq!(d.meshes.len(), 1);
        assert_eq!(d.meshes[0].geometry, "phoenix");
        assert_eq!(d.meshes[0].texture, "phoenix");
        assert_eq!(d.meshes[0].texture_settings, "ship_small");
    }

    #[test]
    fn faction_diffuse_variant_and_root_geometry_recipes_resolve() {
        let uber = uber_with(&[("lib_ships", "lib_ships.xml")]);
        let eic = uber
            .display_3d("lib_ships:ship_aegis_2")
            .expect("Aegis EIC");
        assert_eq!(eic.meshes[0].geometry, "aegis");
        assert_eq!(eic.meshes[0].texture, "aegis-eic");
        // `<display3D geometry="saimon-boss" scale="1.12"/>` is the mesh itself.
        let saimon = uber.display_3d("25").expect("Saimon boss");
        assert_eq!(saimon.meshes.len(), 1);
        assert_eq!(saimon.meshes[0].geometry, "saimon-boss");
        assert!((saimon.scale - 1.12).abs() < 1e-4);
    }

    #[test]
    fn particle_children_resolve_for_boxes_and_ship_effects() {
        let uber = uber_with(&[
            ("lib_collectables", "lib_collectables.xml"),
            ("lib_ships", "lib_ships.xml"),
        ]);
        // abstract_3d_box: `<particles url="">` replaced by the per-box zip,
        // lifted by the recipe's y=-100.
        let boxed = uber
            .display_3d("box_BONUS_BOX")
            .expect("bonus box 3D recipe");
        assert!(boxed.meshes.is_empty());
        assert_eq!(boxed.particles.len(), 1);
        assert_eq!(boxed.particles[0].resource, "box_2");
        assert!((boxed.particles[0].position.1 + 100.0).abs() < 1e-3);
        // Mordon boss: a mesh plus `<particles url="mordon_boss.zip" scale="0.8">`.
        let mordon = uber.display_3d("31").expect("Mordon boss 3D recipe");
        assert_eq!(mordon.meshes.len(), 1);
        assert_eq!(mordon.particles[0].resource, "mordon_boss");
        assert!((mordon.particles[0].scale.0 - 0.8).abs() < 1e-3);
    }

    #[test]
    fn composes_authored_portal_parent_and_mesh_transforms() {
        let uber = uber_with(&[("lib_portals", "lib_portals.xml")]);
        let recipe = uber.display_3d("1").expect("regular gate 3D recipe");
        let mesh = &recipe.meshes[0];
        assert_eq!(mesh.geometry, "jumpgate-regular");
        let expected = super::super::mapscene::combine_transform(
            (0.0, 0.0, 0.0),
            (20.0, 35.0, 10.0),
            1.0,
            (0.0, 0.0, 0.0),
            (0.0, 90.0, 0.0),
            0.6,
        );
        assert_eq!(mesh.position, expected.0);
        assert_eq!(mesh.rotation, expected.1);
        assert_eq!(mesh.scale, 0.6);
    }

    #[test]
    fn portal_recipes_keep_jump_layers_and_jump_particles() {
        let uber = uber_with(&[("lib_portals", "lib_portals.xml")]);
        let d = uber.display_2d("1").expect("standard portal");
        let ids: Vec<_> = d.map_layers.iter().map(|l| l.id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["base", "highlight", "active", "jump", "spawn_effect"]
        );
        let jump = d.map_layers.iter().find(|l| l.id == "jump").unwrap();
        assert_eq!(jump.atlas_key, "portal_jump_atlas"); // gitleaks:allow -- texture atlas identifier
        assert_eq!(jump.texture_key, "portal_jump_texture");
        assert!(jump.texture.is_empty(), "empty texture plays every frame");
        assert!(jump.alpha.abs() < 1e-4);
        let gg = uber.display_2d("2").expect("galaxy gate");
        let gg_jump = gg.map_layers.iter().find(|l| l.id == "jump").unwrap();
        assert!(!gg_jump.visible, "visible=\"0\" stays hidden until jumpOn");
        let spawn = d
            .map_layers
            .iter()
            .find(|l| l.id == "spawn_effect")
            .unwrap();
        assert_eq!(spawn.atlas_key, "spawn_animation_atlas");
        assert!((spawn.scale.0 - 1.5).abs() < 1e-4);

        let recipe = uber.display_3d("1").expect("standard portal 3d");
        assert!(recipe.particles.is_empty(), "jump vfx is not idle");
        assert_eq!(recipe.jump_particles.len(), 1);
        assert_eq!(recipe.jump_particles[0].resource, "gate_jump");
        assert!((recipe.jump_particles[0].scale.0 - 0.6).abs() < 1e-3);

        // Expedition gate: idle particles autoplay, entry plays on jump.
        let expedition = uber.display_3d("300").expect("expedition portal");
        assert!(expedition
            .particles
            .iter()
            .any(|p| p.resource == "portal_300_idle"));
        assert!(expedition
            .jump_particles
            .iter()
            .any(|p| p.resource == "portal_300_entry"));

        let drive = uber.display_2d("30005").expect("drive portal");
        let light = drive
            .map_layers
            .iter()
            .find(|l| l.id == "animated_base")
            .unwrap();
        assert_eq!(light.atlas_key, "light_305_atlas");
        assert_eq!(light.texture, "portal_305_light_000");
        let jump = drive.map_layers.iter().find(|l| l.id == "jump").unwrap();
        assert_eq!(jump.atlas_key, "jump_305_atlas"); // gitleaks:allow -- texture atlas identifier
        assert_eq!(jump.texture, "portal_305_000");
    }

    #[test]
    fn resolves_material_diffuse_texture_size_from_authored_settings() {
        assert_eq!(
            resolve_3d_texture_id("aegis", "aegis", "ship_small").as_deref(),
            Some("aegis_diffuse_128")
        );
        assert_eq!(
            resolve_3d_texture_id("ore-palladium", "ore-palladium", "building").as_deref(),
            Some("ore-palladium_diffuse_512")
        );
        assert_eq!(
            resolve_3d_texture_id("none", "aegis", "ship").as_deref(),
            None
        );
        assert_eq!(
            resolve_3d_texture_id("", "phoenix", "ship_small").as_deref(),
            Some("phoenix_diffuse_128")
        );
    }

    fn bare_material(texture: &str, settings: &str) -> MaterialDesc {
        MaterialDesc {
            texture: texture.to_string(),
            texture_settings: settings.to_string(),
            texture_base: texture.to_string(),
            ..MaterialDesc::default()
        }
    }

    fn key(
        m: &MaterialDesc,
        slot: TextureSlot,
        geometry: &str,
        quality: TextureQuality,
    ) -> Option<String> {
        m.texture_key(slot, geometry, quality, 1)
    }

    #[test]
    fn texture_key_size_tables_per_quality() {
        // Settings3D.updateTexturesSize: ship column is 128/128/256.
        let ship = bare_material("aegis", "ship");
        assert_eq!(
            key(&ship, TextureSlot::Diffuse, "aegis", TextureQuality::Low).as_deref(),
            Some("aegis_diffuse_128")
        );
        assert_eq!(
            key(&ship, TextureSlot::Diffuse, "aegis", TextureQuality::Medium).as_deref(),
            Some("aegis_diffuse_128")
        );
        assert_eq!(
            key(&ship, TextureSlot::Diffuse, "aegis", TextureQuality::High).as_deref(),
            Some("aegis_diffuse_256")
        );

        let big = bare_material("station", "building_big");
        assert_eq!(
            key(&big, TextureSlot::Diffuse, "station", TextureQuality::Low).as_deref(),
            Some("station_diffuse_256")
        );
        assert_eq!(
            key(
                &big,
                TextureSlot::Diffuse,
                "station",
                TextureQuality::Medium
            )
            .as_deref(),
            Some("station_diffuse_512")
        );
        assert_eq!(
            key(&big, TextureSlot::Diffuse, "station", TextureQuality::High).as_deref(),
            Some("station_diffuse_1024")
        );

        let ore = bare_material("ore-palladium", "building");
        assert_eq!(
            key(
                &ore,
                TextureSlot::Diffuse,
                "ore-palladium",
                TextureQuality::Low
            )
            .as_deref(),
            Some("ore-palladium_diffuse_256")
        );
        assert_eq!(
            key(
                &ore,
                TextureSlot::Diffuse,
                "ore-palladium",
                TextureQuality::Medium
            )
            .as_deref(),
            Some("ore-palladium_diffuse_512")
        );
        assert_eq!(
            key(
                &ore,
                TextureSlot::Diffuse,
                "ore-palladium",
                TextureQuality::High
            )
            .as_deref(),
            Some("ore-palladium_diffuse_512")
        );

        // Unknown/absent tex_settings fall back to the ship column
        // (`getTextureSize` returns `tex_ships` last); sub-128 sizes clamp up.
        let small = bare_material("pet", "ship_very_small");
        assert_eq!(
            key(&small, TextureSlot::Diffuse, "pet", TextureQuality::Low).as_deref(),
            Some("pet_diffuse_128")
        );
        let unknown = bare_material("x", "bogus");
        assert_eq!(
            key(&unknown, TextureSlot::Diffuse, "x", TextureQuality::High).as_deref(),
            Some("x_diffuse_256")
        );
        let absent = bare_material("x", "");
        assert_eq!(
            key(&absent, TextureSlot::Diffuse, "x", TextureQuality::High).as_deref(),
            Some("x_diffuse_256")
        );
    }

    #[test]
    fn texture_key_base_chain_and_suppression() {
        // texture="none" suppresses every Auto slot (getTextureName chain).
        let none = bare_material("none", "ship");
        assert_eq!(
            key(&none, TextureSlot::Diffuse, "geo", TextureQuality::High),
            None
        );
        assert_eq!(
            key(&none, TextureSlot::Normal, "geo", TextureQuality::High),
            None
        );
        // Empty texture falls back to the geometry id.
        let auto = bare_material("", "ship");
        assert_eq!(
            key(&auto, TextureSlot::Diffuse, "geo", TextureQuality::High).as_deref(),
            Some("geo_diffuse_256")
        );
        assert_eq!(
            key(&auto, TextureSlot::Diffuse, "", TextureQuality::High),
            None
        );
        // Numeric attribute values stay verbatim base ids — they resolve to
        // absent manifest keys, never to scalar params.
        let xml = r#"<assets><asset id="n"><display3D><mesh geometry="g" texture="t" alpha="0.2"/></display3D></asset></assets>"#;
        let uber = UberAssets::parse(&[("lib_test", xml)]).unwrap();
        let m = &uber.display_3d("n").unwrap().meshes[0].material;
        assert_eq!(
            m.slot_source(TextureSlot::Alpha),
            &SlotSource::Named("0.2".into())
        );
        assert_eq!(
            key(m, TextureSlot::Alpha, "g", TextureQuality::High).as_deref(),
            Some("0.2_alpha_256")
        );
    }

    #[test]
    fn size_multiplier_applies_only_below_512() {
        // assembleTextureResKey: `size < 512 ? sizeMultiplier : 1`.
        let ship = bare_material("x", "ship");
        assert_eq!(
            ship.texture_key(TextureSlot::Diffuse, "x", TextureQuality::Medium, 2)
                .as_deref(),
            Some("x_diffuse_256")
        );
        let building = bare_material("x", "building");
        assert_eq!(
            building
                .texture_key(TextureSlot::Diffuse, "x", TextureQuality::Medium, 2)
                .as_deref(),
            Some("x_diffuse_512")
        );
        assert_eq!(
            building
                .texture_key(TextureSlot::Diffuse, "x", TextureQuality::Low, 2)
                .as_deref(),
            Some("x_diffuse_512")
        );
        // Clamp: 1024 column cannot exceed 1024 regardless of multiplier.
        let planet = bare_material("x", "planet");
        assert_eq!(
            planet
                .texture_key(TextureSlot::Diffuse, "x", TextureQuality::High, 4)
                .as_deref(),
            Some("x_diffuse_1024")
        );
    }

    #[test]
    fn supported_slots_match_entity_basic_material() {
        use TextureSlot::*;
        assert_eq!(
            MaterialDesc::supported_slots(TextureQuality::Low),
            &[Diffuse, Alpha, Gal]
        );
        assert_eq!(
            MaterialDesc::supported_slots(TextureQuality::Medium),
            &[Diffuse, Alpha, Gal, Glow]
        );
        assert_eq!(
            MaterialDesc::supported_slots(TextureQuality::High),
            &[Diffuse, Alpha, Gal, Normal, Specular, Ao, Glow]
        );
    }

    #[test]
    fn material_desc_parses_scalar_params_and_flags() {
        let xml = r#"<assets><asset id="m"><display3D><mesh geometry="g" texture="t" tex_settings="ship" shader="uber_ship" specularity="0.5" specularityLow="0.1" gloss="25" glossLow="100" blendMode="add" alphaBlending="true" materialBothSides="true" light="false" secondaryUVForLightMap="true"/></display3D></asset></assets>"#;
        let uber = UberAssets::parse(&[("lib_test", xml)]).unwrap();
        let mesh = &uber.display_3d("m").unwrap().meshes[0];
        let m = &mesh.material;
        // Legacy flat fields mirror the descriptor.
        assert_eq!(mesh.texture, "t");
        assert_eq!(mesh.texture_settings, "ship");
        assert_eq!(m.shader, "uber_ship");
        assert_eq!(m.specular_strength(TextureQuality::High), 0.5);
        assert_eq!(m.specular_strength(TextureQuality::Low), 0.1);
        assert_eq!(m.gloss_value(TextureQuality::High), 25.0);
        assert_eq!(m.gloss_value(TextureQuality::Medium), 100.0);
        assert!(m.blend_add && m.alpha_blending && m.both_sides && m.secondary_uv_lightmap);
        assert!(!m.lit);

        // Defaults: lit, specular 1 / gloss 50, low tiers reuse the main value.
        let xml = r#"<assets><asset id="d"><display3D><mesh geometry="g" specularity="0.4"/></display3D></asset></assets>"#;
        let uber = UberAssets::parse(&[("lib_test", xml)]).unwrap();
        let m = &uber.display_3d("d").unwrap().meshes[0].material;
        assert!(m.lit);
        assert_eq!(m.specular_strength(TextureQuality::High), 0.4);
        assert_eq!(m.specular_strength(TextureQuality::Low), 0.4);
        assert_eq!(m.gloss_value(TextureQuality::High), 50.0);
    }

    #[test]
    fn material_slots_from_real_recipes() {
        let uber = uber_with(&[("lib_default", "lib_default.xml")]);
        // Root-geometry `<display3D>` recipe: `texture="repair-robot"
        // diffuse="repair-robot-lvl01" glow="repair-robot-lvl01" normal="none"`.
        let d = uber.display_3d("repair-robot-1").expect("repair-robot-1");
        let mesh = &d.meshes[0];
        let m = &mesh.material;
        assert_eq!(mesh.texture, "repair-robot-lvl01");
        assert_eq!(
            m.slot_source(TextureSlot::Diffuse),
            &SlotSource::Named("repair-robot-lvl01".into())
        );
        assert_eq!(m.slot_source(TextureSlot::Normal), &SlotSource::Off);
        assert_eq!(
            key(
                m,
                TextureSlot::Diffuse,
                "repair-robot",
                TextureQuality::High
            )
            .as_deref(),
            Some("repair-robot-lvl01_diffuse_128")
        );
        assert_eq!(
            key(m, TextureSlot::Glow, "repair-robot", TextureQuality::High).as_deref(),
            Some("repair-robot-lvl01_glow_128")
        );
        assert_eq!(
            key(m, TextureSlot::Normal, "repair-robot", TextureQuality::High),
            None
        );
        // Auto slots fall back to raw `@texture` — NOT the @diffuse value.
        assert_eq!(
            key(
                m,
                TextureSlot::Specular,
                "repair-robot",
                TextureQuality::High
            )
            .as_deref(),
            Some("repair-robot_specular_128")
        );
        assert_eq!(
            key(m, TextureSlot::Ao, "repair-robot", TextureQuality::High).as_deref(),
            Some("repair-robot_ao_128")
        );

        // `glow="beacon-exit-eic"` on a `<mesh>` child (building column).
        let d = uber.display_3d("tdm_exit_2").expect("tdm_exit_2");
        let m = &d.meshes[0].material;
        assert_eq!(
            key(m, TextureSlot::Glow, "beacon-static", TextureQuality::High).as_deref(),
            Some("beacon-exit-eic_glow_512")
        );
        assert_eq!(
            key(
                m,
                TextureSlot::Diffuse,
                "beacon-static",
                TextureQuality::High
            )
            .as_deref(),
            Some("beacon-exit-eic_diffuse_512")
        );
        assert_eq!(
            key(m, TextureSlot::Alpha, "beacon-static", TextureQuality::High).as_deref(),
            Some("beacon_alpha_512")
        );
        assert_eq!(
            key(m, TextureSlot::Alpha, "beacon-static", TextureQuality::Low).as_deref(),
            Some("beacon_alpha_256")
        );
    }

    #[test]
    fn scene_meshes_carry_material_descriptors() {
        let uber = uber_with(&[("lib_maps", "lib_maps.xml")]);
        let scene = uber.map_scene3d("lib_maps:map_1-1").unwrap();
        // `satellite`: building_small, materialBothSides, glossLow=100,
        // no texture attr -> geometry id is the Auto base.
        let sat = scene
            .meshes
            .iter()
            .find(|m| m.id == "satellite")
            .expect("satellite mesh");
        assert_eq!(sat.texture, "");
        assert_eq!(sat.material.texture_settings, "building_small");
        assert!(sat.material.both_sides);
        assert_eq!(sat.material.gloss_value(TextureQuality::Low), 100.0);
        assert_eq!(
            key(
                &sat.material,
                TextureSlot::Diffuse,
                &sat.geometry,
                TextureQuality::High
            )
            .as_deref(),
            Some("asset-mmo-satellite_diffuse_512")
        );
    }

    #[test]
    fn scale_attr_on_display2d_element() {
        // lib_default mine_26: `<display2D srcKey="agatusLure" scale="0.6">`.
        let uber = uber_with(&[("lib_assets", "lib_default.xml")]);
        let d = uber.display_2d("mine_26").expect("mine_26");
        assert_eq!(d.src_key, "agatusLure");
        assert_eq!(d.scale, (0.6, 0.6));
    }

    #[test]
    fn scale_container_chain_multiplies() {
        // display2D scale * displayObject scale.
        let xml = r#"<assets>
            <asset id="chained">
                <display2D scale="0.5">
                    <displayObject srcKey="s" scale="0.2"/>
                </display2D>
            </asset>
            <asset id="child_only">
                <display2D scale="0.5">
                    <displayObject srcKey="s"/>
                </display2D>
            </asset>
            <asset id="per_axis">
                <display2D>
                    <displayObject srcKey="s" scale="0.2" scaleX="-0.4" scaleY="0.7"/>
                </display2D>
            </asset>
            <asset id="noscale">
                <display2D><displayObject srcKey="s"/></display2D>
            </asset>
        </assets>"#;
        let uber = UberAssets::parse(&[("lib_test", xml)]).expect("parse");
        assert_eq!(uber.display_2d("chained").unwrap().scale, (0.1, 0.1));
        assert_eq!(uber.display_2d("child_only").unwrap().scale, (0.5, 0.5));
        assert_eq!(
            uber.display_2d("per_axis").unwrap().scale,
            (-0.4, 0.7),
            "scaleX/scaleY override the shared scale per axis"
        );
        assert_eq!(uber.display_2d("noscale").unwrap().scale, (1.0, 1.0));
    }

    #[test]
    fn scale_non_numeric_is_ignored() {
        let xml = r#"<assets>
            <asset id="bad"><display2D srcKey="s" scale="abc"/></asset>
        </assets>"#;
        let uber = UberAssets::parse(&[("lib_test", xml)]).expect("parse");
        assert_eq!(uber.display_2d("bad").unwrap().scale, (1.0, 1.0));
    }

    #[test]
    fn multiple_display_objects_become_layers() {
        let xml = r#"<assets>
            <asset id="multi">
                <display2D scale="0.5">
                    <displayObject srcKey="a" resKey="r1" x="10" y="-4"/>
                    <displayObject srcKey="" resKey="skipped"/>
                    <displayObject srcKey="b" resKey="r2" y="-15" scale="0.4"/>
                    <displayObject srcKey="c" resKey="r3" x="3"/>
                </display2D>
            </asset>
            <asset id="single">
                <display2D><displayObject srcKey="s" resKey="only"/></display2D>
            </asset>
        </assets>"#;
        let uber = UberAssets::parse(&[("lib_test", xml)]).expect("parse");
        let d = uber.display_2d("multi").unwrap();
        // First usable object is the main recipe (unchanged single path).
        assert_eq!((d.src_key.as_str(), d.res_key.as_str()), ("a", "r1"));
        assert_eq!((d.x, d.y), (10.0, -4.0));
        assert_eq!(d.scale, (0.5, 0.5));
        // The rest, in document order, each with own offset + container scale.
        assert_eq!(d.layers.len(), 2);
        assert_eq!(d.layers[0].src_key, "b");
        assert_eq!((d.layers[0].x, d.layers[0].y), (0.0, -15.0));
        assert_eq!(d.layers[0].scale, (0.2, 0.2));
        assert_eq!(d.layers[1].res_key, "r3");
        assert_eq!((d.layers[1].x, d.layers[1].y), (3.0, 0.0));
        assert_eq!(d.layers[1].scale, (0.5, 0.5));
        assert!(uber.display_2d("single").unwrap().layers.is_empty());
    }

    #[test]
    fn fixture_multi_object_recipes() {
        let uber = uber_with(&[("lib_assets", "lib_default.xml")]);
        let d = uber.display_2d("pod_heal").expect("pod_heal");
        assert_eq!(d.layers.len(), 1);
        assert_eq!((d.x, d.y), (50.0, 50.0));
        assert_eq!((d.layers[0].x, d.layers[0].y), (0.0, -15.0));
    }

    #[test]
    fn display_pivot_reads_dock_mounts() {
        let uber = uber_with(&[("lib_ships", "lib_ships.xml")]);
        assert_eq!(uber.display_pivot("116", "dock_1"), Some((-178.0, 79.0)));
        assert_eq!(uber.display_pivot("116", "dock_6"), Some((41.0, 161.0)));
        assert_eq!(uber.display_pivot("116", "dock_9"), None);
        assert_eq!(uber.display_pivot("nope", "dock_1"), None);
    }

    #[test]
    fn click_area_pivot_is_parsed() {
        let uber = uber_with(&[("lib_collectables", "lib_collectables.xml")]);
        // `abstract_3d_box` template: `<pivot id="click_area" y="100"/>`.
        let d = uber.display_2d("box_BONUS_BOX").expect("box_BONUS_BOX");
        assert_eq!(d.click_pivot, (0.0, 100.0));
    }
}
