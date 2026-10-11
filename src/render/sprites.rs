//! GPU texture cache: promotes `SpriteClip` CPU frames to `Texture2D`.
//!
//! Upload path: `SpriteFrame.image` (RGBA8) is copied into a
//! `raylib::ffi::MemAlloc` buffer, wrapped in a raw `ffi::Image`
//! (`PIXELFORMAT_UNCOMPRESSED_R8G8B8A8`) and handed to
//! `LoadTextureFromImage`. Dropping the `Image` runs `UnloadImage`, which
//! frees the buffer through the same `MemFree` allocator — a raw memcpy
//! with no PNG encode on the upload path.

use crate::assets::{MaterialDesc, SpriteClip, SpriteId, TextureQuality, TextureSlot};
use raylib::ffi;
use raylib::prelude::*;
use std::collections::{HashMap, HashSet};

/// One GPU frame.
pub struct GpuFrame {
    pub tex: Texture2D,
    /// Frame offset relative to the clip origin, sprite pixels.
    pub offset: (f32, f32),
}

/// Per-clip GPU frames.
pub struct GpuClip {
    pub frames: Vec<GpuFrame>,
    /// True for rotation strips (32 frames indexed by heading).
    pub rotation_strip: bool,
    /// Per-frame durations in ms, index-aligned with `frames`
    /// (only successfully uploaded frames get an entry).
    pub durations: Vec<u32>,
    /// Union of every frame's `[offset, offset + tex_size]` rect, clip px
    /// (`(min_x, min_y, max_x, max_y)`; zeros for empty clips). Used for
    /// bounds-based culling and constant label anchors.
    pub bounds: (f32, f32, f32, f32),
    /// Authored scale9 splitter rect `(x, y, w, h)` in frame-0 px
    /// (window chrome, panels) — `Some` when the source SWF had one.
    pub scale9: Option<(f32, f32, f32, f32)>,
}

/// SpriteId -> uploaded frames. Upload happens lazily during draw.
#[derive(Default)]
pub struct TextureCache {
    clips: HashMap<SpriteId, GpuClip>,
    /// Authored AWD mesh per `materials3d::model_key` — (geometry id,
    /// material fingerprint at the current texture quality).
    models3d: HashMap<(String, String), Model>,
    /// Texture owners must outlive the raylib model material references —
    /// holds every resolved material layer, not just diffuse.
    textures3d: HashMap<String, Texture2D>,
    /// Shared lit/unlit shaders + 1x1 fallback textures for authored mesh
    /// materials (`EntityBasicMaterial`); declared after `models3d`/
    /// `textures3d` so models drop before the resources their materials
    /// weakly reference. `materials3d_broken` latches a compile failure so
    /// it isn't retried every frame.
    materials3d: Option<super::materials3d::Materials3D>,
    materials3d_broken: bool,
    unit_plane3d: Option<Model>,
    plane_models3d: HashMap<String, Model>,
    plane_sprite_ids: HashMap<String, SpriteId>,
    failed_models3d: HashSet<String>,
    model_loader: super::model_loader::ModelLoader,
    /// Particle groups copied from the asset manager; their textures and
    /// meshes live in `textures3d` / `models3d` under a `p:` prefix.
    particle_groups: HashMap<String, std::sync::Arc<crate::assets::particles::ParticleGroup>>,
}

impl TextureCache {
    /// Upload every clip in `store` that lacks GPU frames.
    ///
    /// Must run *before* `begin_drawing` — texture creation needs
    /// `&mut RaylibHandle`, which is mutably borrowed while drawing.
    /// Clips with zero frames are still recorded so failed extractions
    /// are not retried every frame.
    pub fn sync(
        &mut self,
        rl: &mut RaylibHandle,
        thread: &RaylibThread,
        store: &crate::assets::SpriteStore,
    ) {
        for (id, clip) in store.iter() {
            if self.clips.contains_key(&id) {
                continue;
            }
            self.clips.insert(id, upload_clip(rl, thread, clip));
        }
    }

    /// Upload selected authored `display3D` meshes. Geometry decode and OBJ
    /// conversion run only after the separate World3D collection has loaded
    /// bytes; all GPU work stays on the raylib thread before drawing begins.
    ///
    /// Models are keyed by `materials3d::model_key` (geometry + material
    /// fingerprint at the current texture quality). A model is only built
    /// once every resolved layer is uploaded or has definitively failed —
    /// pending optional layers delay the model so a late texture never
    /// bakes in as a permanent fallback.
    pub fn sync_models3d(
        &mut self,
        rl: &mut RaylibHandle,
        thread: &RaylibThread,
        assets: &crate::assets::AssetManager,
        game: &crate::game::Game,
        world: &crate::net::SharedWorld,
    ) {
        self.model_loader.poll();
        let rp = game.preferences.render;
        let quality = rp.texture_quality;
        let mut wanted = HashMap::<(String, String), MaterialDesc>::new();
        let mut collect = |recipe: &crate::assets::Display3D| {
            for mesh in &recipe.meshes {
                wanted
                    .entry(super::materials3d::model_key(
                        &mesh.geometry,
                        &mesh.material,
                        quality,
                    ))
                    .or_insert_with(|| mesh.material.clone());
            }
        };
        for (&id, visual) in &game.entity_visuals {
            let is_npc = world.world.entities.get(&id).is_some_and(|e| e.npc);
            if if is_npc { rp.npcs_3d } else { rp.ships_3d } {
                if let Some(recipe) = &visual.display_3d {
                    collect(recipe);
                }
            }
        }
        if rp.collectables_3d {
            for visual in game.collectable_visuals.values() {
                if let Some(recipe) = &visual.display_3d {
                    collect(recipe);
                }
            }
        }
        if rp.portals_3d {
            for visual in game.portal_visuals.values() {
                if let Some(recipe) = &visual.display_3d {
                    collect(recipe);
                }
            }
        }
        if rp.drones_3d {
            for visual in game.drone_visuals.values() {
                if let Some(recipe) = &visual.display_3d {
                    collect(recipe);
                }
            }
        }
        if rp.backgrounds_3d {
            if let Some(scene) = assets.map_scene3d_for_map(game.map_id) {
                for mesh in &scene.meshes {
                    wanted
                        .entry(super::materials3d::model_key(
                            &mesh.geometry,
                            &mesh.material,
                            quality,
                        ))
                        .or_insert_with(|| mesh.material.clone());
                }
                if !scene.planes.is_empty() {
                    self.sync_unit_plane3d(rl, thread);
                }
                for plane in &scene.planes {
                    if plane.texture.is_empty() {
                        continue;
                    }
                    if let Some(sprite) =
                        game.map_visuals.get(&format!("plane3d:{}", plane.texture))
                    {
                        self.sync_plane_model3d(rl, thread, &plane.texture, *sprite);
                    }
                }
            }
        }
        // Particle groups attached to the recipes in use.
        let mut wanted_particles = HashSet::<String>::new();
        for (&id, visual) in &game.entity_visuals {
            let is_npc = world.world.entities.get(&id).is_some_and(|e| e.npc);
            if if is_npc { rp.npcs_3d } else { rp.ships_3d } {
                if let Some(recipe) = &visual.display_3d {
                    wanted_particles.extend(recipe.particles.iter().map(|p| p.resource.clone()));
                }
            }
        }
        if rp.collectables_3d {
            for visual in game.collectable_visuals.values() {
                if let Some(recipe) = &visual.display_3d {
                    wanted_particles.extend(recipe.particles.iter().map(|p| p.resource.clone()));
                }
            }
        }
        if rp.portals_3d {
            for visual in game.portal_visuals.values() {
                if let Some(recipe) = &visual.display_3d {
                    wanted_particles.extend(recipe.particles.iter().map(|p| p.resource.clone()));
                    wanted_particles
                        .extend(recipe.jump_particles.iter().map(|p| p.resource.clone()));
                }
            }
        }
        if rp.drones_3d {
            for visual in game.drone_visuals.values() {
                if let Some(recipe) = &visual.display_3d {
                    wanted_particles.extend(recipe.particles.iter().map(|p| p.resource.clone()));
                }
            }
        }
        if rp.effects_3d {
            // Live laser volleys pull in their `{resKey}.zip` bolt groups
            // and `muzzleFlashResKey` flashes — otherwise
            // `particle_group_ready` never becomes true for them.
            for effect in &game.effects.active {
                let crate::game::effects::Effect::LaserShot { vfx: Some(v), .. } = effect else {
                    continue;
                };
                if v.kind == crate::game::effects::LaserKind3D::Particles && !v.particle.is_empty()
                {
                    wanted_particles.insert(v.particle.clone());
                }
                if let Some(m) = &v.muzzle {
                    wanted_particles.insert(m.resource.clone());
                }
            }
        }
        self.sync_particles(rl, thread, assets, &wanted_particles);
        // Shared shaders/fallbacks spin up lazily once a mesh is wanted;
        // a compile failure is latched so it isn't retried every frame.
        if !wanted.is_empty() && self.materials3d.is_none() && !self.materials3d_broken {
            self.materials3d = super::materials3d::Materials3D::load(rl, thread);
            self.materials3d_broken = self.materials3d.is_none();
        }
        if let Some(mats) = &mut self.materials3d {
            mats.set_lighting(super::scene3d::camera(game).position, hero_light_pos(world));
        }
        for (key, material) in wanted {
            if self.models3d.contains_key(&key) || self.materials3d.is_none() {
                continue;
            }
            let Some(bytes) = assets.geometry(&key.0) else {
                continue;
            };
            // Resolved layer set: filtered to manifest ids once the index
            // has landed; before that only the diffuse id is knowable.
            let mut layers = super::materials3d::layer_keys(&key.0, &material, quality);
            if assets.geometry_index_ready() {
                layers.retain(|(_, id)| assets.geometry_file_exists(id));
            } else {
                layers.retain(|(slot, _)| *slot == TextureSlot::Diffuse);
            }
            let mesh_key = super::model_loader::ModelLoader::mesh_key(&key.0, &bytes);
            let failure_key = format!("model:{}:{}:{:016x}", key.0, key.1, mesh_key.digest());
            if self.failed_models3d.contains(&failure_key) {
                continue;
            }
            let safe = key
                .0
                .chars()
                .map(|c| {
                    if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                        c
                    } else {
                        '_'
                    }
                })
                .collect::<String>();
            let path = assets
                .cache_dir
                .join("models3d")
                .join(format!("{safe}-{:016x}-v3.obj", mesh_key.digest()));
            if self.model_loader.status(&mesh_key) == super::model_loader::Status::Unknown {
                self.model_loader
                    .request_mesh(mesh_key.clone(), bytes, path);
            }
            // Kick every layer's decode/upload; the model is built only
            // once all resolved layers are uploaded or definitively failed
            // (a pending optional layer must never bake in as a fallback).
            // The diffuse gate keeps today's semantics: a failed diffuse
            // leaves the mesh on its 2D sprite.
            let mut settled = true;
            for (slot, id) in &layers {
                if self.sync_texture3d(rl, thread, assets, id) {
                    continue;
                }
                // Definitive failure = decode/upload failed (`texture:`
                // negative-cache) or the collection's fetch failed. A
                // still-pending optional layer keeps the model unbuilt.
                let failed = self.texture3d_failed(id) || assets.geometry_failure(id).is_some();
                if *slot == TextureSlot::Diffuse || !failed {
                    settled = false;
                }
            }
            if !settled {
                continue;
            }
            let path = match self.model_loader.status(&mesh_key) {
                super::model_loader::Status::Ready => match self.model_loader.take(&mesh_key) {
                    Some(super::model_loader::PreparedAsset::MeshObj(path)) => path,
                    _ => continue,
                },
                super::model_loader::Status::Failed => continue,
                super::model_loader::Status::Pending | super::model_loader::Status::Unknown => {
                    continue
                }
            };
            match rl.load_model(thread, &path.to_string_lossy()) {
                Ok(mut model) => {
                    if let Some(mats) = &self.materials3d {
                        super::materials3d::assign(
                            &mut model,
                            &material,
                            quality,
                            &layers,
                            &self.textures3d,
                            mats,
                        );
                    }
                    self.models3d.insert(key, model);
                }
                Err(error) => {
                    if self.failed_models3d.insert(failure_key) {
                        log::warn!("3D mesh {} could not be loaded: {error:#}", key.0);
                    }
                }
            }
        }
    }

    /// Authored mesh model for `(geometry, material)` at `quality` —
    /// `materials3d::model_key` keyed.
    pub fn model3d(
        &self,
        geometry: &str,
        material: &MaterialDesc,
        quality: TextureQuality,
    ) -> Option<&Model> {
        self.models3d
            .get(&super::materials3d::model_key(geometry, material, quality))
    }

    /// `true` once a layer's decode/upload has definitively failed. Failure
    /// keys are `texture:{id}:{digest}` — the digest varies per content
    /// version, so match by prefix (same pattern as `particle_group_ready`).
    fn texture3d_failed(&self, id: &str) -> bool {
        let prefix = format!("texture:{id}:");
        self.failed_models3d.iter().any(|f| f.starts_with(&prefix))
    }

    /// Prepare a 3D material texture asynchronously, then upload its decoded
    /// pixels on the raylib thread. Returns false until the GPU texture exists.
    pub fn sync_texture3d(
        &mut self,
        rl: &mut RaylibHandle,
        thread: &RaylibThread,
        assets: &crate::assets::AssetManager,
        id: &str,
    ) -> bool {
        self.sync_texture_bytes(rl, thread, id, assets.geometry(id))
    }

    /// [`Self::sync_texture3d`] for bytes from any collection, stored under
    /// `store_key`.
    fn sync_texture_bytes(
        &mut self,
        rl: &mut RaylibHandle,
        thread: &RaylibThread,
        store_key: &str,
        bytes: Option<std::sync::Arc<[u8]>>,
    ) -> bool {
        if self.textures3d.contains_key(store_key) {
            return true;
        }
        let Some(bytes) = bytes else {
            return false;
        };
        let key = super::model_loader::ModelLoader::texture_key(store_key, &bytes);
        let failure_key = format!("texture:{store_key}:{:016x}", key.digest());
        if self.failed_models3d.contains(&failure_key) {
            return false;
        }
        if self.model_loader.status(&key) == super::model_loader::Status::Unknown {
            self.model_loader.request_texture(key.clone(), bytes);
            return false;
        }
        let image = match self.model_loader.status(&key) {
            super::model_loader::Status::Ready => match self.model_loader.take(&key) {
                Some(super::model_loader::PreparedAsset::Texture(image)) => image,
                _ => return false,
            },
            super::model_loader::Status::Pending => return false,
            super::model_loader::Status::Failed => {
                self.failed_models3d.insert(failure_key);
                return false;
            }
            super::model_loader::Status::Unknown => return false,
        };
        let Some(texture) = upload_rgba(rl, thread, &image) else {
            self.failed_models3d.insert(failure_key);
            return false;
        };
        self.textures3d.insert(store_key.to_owned(), texture);
        true
    }

    // --- particles ------------------------------------------------------

    pub fn particle_group(
        &self,
        id: &str,
    ) -> Option<&std::sync::Arc<crate::assets::particles::ParticleGroup>> {
        self.particle_groups.get(id)
    }

    pub fn particle_texture(&self, id: &str) -> Option<&Texture2D> {
        self.textures3d.get(&format!("p:{id}"))
    }

    pub fn particle_model(&self, geometry: &str, texture: &str) -> Option<&Model> {
        self.models3d
            .get(&(format!("p:{geometry}"), format!("p:{texture}")))
    }

    /// A group is ready once every texture and external mesh it uses is
    /// uploaded or has definitively failed.
    pub fn particle_group_ready(&self, id: &str) -> bool {
        use crate::assets::particles::Shape;
        let Some(group) = self.particle_groups.get(id) else {
            return false;
        };
        group.anims.iter().all(|anim| {
            let texture_ok = anim.texture.as_ref().is_none_or(|t| {
                let prefix = format!("texture:p:{t}:");
                self.textures3d.contains_key(&format!("p:{t}"))
                    || self.failed_models3d.iter().any(|f| f.starts_with(&prefix))
            });
            let mesh_ok = match &anim.shape {
                Shape::External(g) => {
                    self.particle_model(g, anim.texture.as_deref().unwrap_or(""))
                        .is_some()
                        || self.failed_models3d.contains(&format!("pmesh:{g}"))
                }
                _ => true,
            };
            texture_ok && mesh_ok
        })
    }

    /// Upload the textures/meshes of the wanted particle groups.
    fn sync_particles(
        &mut self,
        rl: &mut RaylibHandle,
        thread: &RaylibThread,
        assets: &crate::assets::AssetManager,
        wanted: &HashSet<String>,
    ) {
        use crate::assets::particles::Shape;
        for id in wanted {
            if !self.particle_groups.contains_key(id) {
                match assets.particle_group(id) {
                    Some(group) => {
                        self.particle_groups.insert(id.clone(), group);
                    }
                    None => continue,
                }
            }
            let Some(group) = self.particle_groups.get(id).cloned() else {
                continue;
            };
            for anim in &group.anims {
                if let Some(texture) = &anim.texture {
                    let store = format!("p:{texture}");
                    self.sync_texture_bytes(rl, thread, &store, assets.particle_texture(texture));
                }
                if let Shape::External(geometry) = &anim.shape {
                    self.sync_particle_model(rl, thread, assets, geometry, anim.texture.as_deref());
                }
            }
        }
    }

    fn sync_particle_model(
        &mut self,
        rl: &mut RaylibHandle,
        thread: &RaylibThread,
        assets: &crate::assets::AssetManager,
        geometry: &str,
        texture: Option<&str>,
    ) {
        let key = (
            format!("p:{geometry}"),
            format!("p:{}", texture.unwrap_or("")),
        );
        let failure = format!("pmesh:{geometry}");
        if self.models3d.contains_key(&key) || self.failed_models3d.contains(&failure) {
            return;
        }
        let Some(bytes) = assets.particle_geometry(geometry) else {
            return;
        };
        if let Some(texture) = texture {
            let store = format!("p:{texture}");
            if !self.sync_texture_bytes(rl, thread, &store, assets.particle_texture(texture)) {
                return;
            }
        }
        let mesh_key = super::model_loader::ModelLoader::mesh_key(&format!("p:{geometry}"), &bytes);
        let safe: String = geometry
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let path = assets
            .cache_dir
            .join("models3d")
            .join(format!("p-{safe}-{:016x}-v3.obj", mesh_key.digest()));
        if self.model_loader.status(&mesh_key) == super::model_loader::Status::Unknown {
            self.model_loader
                .request_mesh(mesh_key.clone(), bytes, path);
        }
        let path = match self.model_loader.status(&mesh_key) {
            super::model_loader::Status::Ready => match self.model_loader.take(&mesh_key) {
                Some(super::model_loader::PreparedAsset::MeshObj(path)) => path,
                _ => return,
            },
            super::model_loader::Status::Failed => {
                self.failed_models3d.insert(failure);
                return;
            }
            _ => return,
        };
        match rl.load_model(thread, &path.to_string_lossy()) {
            Ok(mut model) => {
                if let Some(tex) = texture.and_then(|t| self.textures3d.get(&format!("p:{t}"))) {
                    for material in model.materials_mut() {
                        material.set_material_texture(
                            raylib::consts::MaterialMapIndex::MATERIAL_MAP_ALBEDO,
                            weak_texture(tex),
                        );
                    }
                }
                self.models3d.insert(key, model);
            }
            Err(error) => {
                log::warn!("particle mesh {geometry} could not be loaded: {error:#}");
                self.failed_models3d.insert(failure);
            }
        }
    }

    pub fn texture3d(&self, id: &str) -> Option<&Texture2D> {
        self.textures3d.get(id).or_else(|| {
            let sprite = self.plane_sprite_ids.get(id)?;
            self.clips
                .get(sprite)?
                .frames
                .first()
                .map(|frame| &frame.tex)
        })
    }

    /// Lazily create the shared XZ plane used by native `display3D` scenes.
    pub fn sync_unit_plane3d(&mut self, rl: &mut RaylibHandle, thread: &RaylibThread) -> bool {
        if self.unit_plane3d.is_some() {
            return true;
        }
        let mesh = Mesh::gen_mesh_plane(thread, 1.0, 1.0, 1, 1);
        self.unit_plane3d = rl
            .load_model_from_mesh(thread, unsafe { mesh.make_weak() })
            .ok();
        self.unit_plane3d.is_some()
    }

    pub fn plane3d(&self) -> Option<&Model> {
        self.unit_plane3d.as_ref()
    }

    pub fn sync_plane_model3d(
        &mut self,
        rl: &mut RaylibHandle,
        thread: &RaylibThread,
        texture_id: &str,
        sprite_id: SpriteId,
    ) -> bool {
        if self.plane_models3d.contains_key(texture_id) {
            return true;
        }
        let Some(texture) = self
            .clips
            .get(&sprite_id)
            .and_then(|clip| clip.frames.first())
            .map(|frame| frame.tex.clone())
        else {
            return false;
        };
        let mesh = Mesh::gen_mesh_plane(thread, 1.0, 1.0, 1, 1);
        let Ok(mut model) = rl.load_model_from_mesh(thread, unsafe { mesh.make_weak() }) else {
            return false;
        };
        // `UberPlane3D` uses `PlaneGeometry(1, 1, 1, 1, yUp = false)`: a quad
        // in the XY plane facing the camera. raylib's plane is XZ facing +y,
        // so stand it up (+90° about X: normal +y -> +z, top edge -> +y).
        model.set_transform(&Matrix::rotate_x(std::f32::consts::FRAC_PI_2));
        for material in model.materials_mut() {
            material.set_material_texture(
                raylib::consts::MaterialMapIndex::MATERIAL_MAP_ALBEDO,
                weak_texture_raw(&texture),
            );
        }
        self.plane_sprite_ids
            .insert(texture_id.to_owned(), sprite_id);
        self.plane_models3d.insert(texture_id.to_owned(), model);
        true
    }

    pub fn plane_model3d(&self, texture_id: &str) -> Option<&Model> {
        self.plane_models3d.get(texture_id)
    }

    /// GPU frames for a sprite, if uploaded by [`Self::sync`].
    pub fn get(&self, id: SpriteId) -> Option<&GpuClip> {
        self.clips.get(&id)
    }

    /// Number of clips uploaded so far (debug overlay).
    pub fn len(&self) -> usize {
        self.clips.len()
    }

    /// True if no clips are cached.
    pub fn is_empty(&self) -> bool {
        self.clips.is_empty()
    }

    /// Upload a CPU clip if absent, then return GPU frames.
    pub fn get_or_upload<'a>(
        &'a mut self,
        rl: &mut RaylibHandle,
        thread: &RaylibThread,
        id: SpriteId,
        clip: &SpriteClip,
    ) -> Option<&'a GpuClip> {
        self.clips
            .entry(id)
            .or_insert_with(|| upload_clip(rl, thread, clip));
        self.clips.get(&id)
    }

    /// Draw a clip frame. `pos` is the clip origin on screen (already
    /// camera-translated) and the rotation pivot; the frame's top-left
    /// lands at `pos + frame.offset * scale` (matching Flash placement,
    /// where the bitmap child sits at its rasterized offset inside the
    /// clip and the clip rotates about its registration point).
    pub fn draw_frame(
        d: &mut RaylibDrawHandle,
        clip: &GpuClip,
        frame: usize,
        pos: (f32, f32),
        rotation: f32,
        scale: f32,
        tint: Color,
    ) {
        Self::draw_frame_offset(
            d,
            clip,
            frame,
            pos,
            rotation,
            (scale, scale),
            tint,
            (0.0, 0.0),
        );
    }

    /// [`Self::draw_frame`] with an extra clip-space offset (in unscaled
    /// sprite pixels, added to the frame offset) and a per-axis `scale`
    /// — a negative axis mirrors the frame about the pivot (`pos`), like
    /// Flash `scaleX/scaleY < 0`. `center="true"` recipes pass
    /// `-tex_size / 2` so the bitmap centres on the clip origin while
    /// rotation still pivots on it.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_frame_offset(
        d: &mut RaylibDrawHandle,
        clip: &GpuClip,
        frame: usize,
        pos: (f32, f32),
        rotation: f32,
        scale: (f32, f32),
        tint: Color,
        extra_offset: (f32, f32),
    ) {
        let Some(f) = clip.frames.get(frame) else {
            return;
        };
        let w = f.tex.width as f32;
        let h = f.tex.height as f32;
        if w <= 0.0 || h <= 0.0 || scale.0.abs() < f32::EPSILON || scale.1.abs() < f32::EPSILON {
            return;
        }
        let off = (f.offset.0 + extra_offset.0, f.offset.1 + extra_offset.1);
        blit(d, &f.tex, off, pos, rotation, scale, tint);
    }
}

/// `DrawTexturePro` plumbing: a sprite point `p` lands at
/// `pos + p * scale` — the frame top-left is drawn at
/// `pos + off * scale` and the image rotates `rotation` degrees around
/// `pos`. A negative scale axis mirrors content about the pivot
/// (negative source rect width/height): content pixel `u` then maps to
/// `pos + (off + w - u) * |scale|` on that axis.
fn blit(
    d: &mut RaylibDrawHandle,
    tex: &Texture2D,
    off: (f32, f32),
    pos: (f32, f32),
    rotation: f32,
    scale: (f32, f32),
    tint: Color,
) {
    let w = tex.width as f32;
    let h = tex.height as f32;
    let (sx, sy) = (scale.0, scale.1);
    let (ax, ay) = (sx.abs(), sy.abs());
    // Mirrored axis: sample the source backwards and anchor the rect so
    // content point `(off + w)` lands at `pos - (off + w) * |s|`.
    let src = Rectangle::new(
        if sx < 0.0 { w } else { 0.0 },
        if sy < 0.0 { h } else { 0.0 },
        if sx < 0.0 { -w } else { w },
        if sy < 0.0 { -h } else { h },
    );
    let ox = if sx < 0.0 { -(off.0 + w) } else { off.0 };
    let oy = if sy < 0.0 { -(off.1 + h) } else { off.1 };
    d.draw_texture_pro(
        tex,
        src,
        Rectangle::new(pos.0, pos.1, w * ax, h * ay),
        Vector2::new(-ox * ax, -oy * ay),
        rotation,
        tint,
    );
}

/// Hero point-light position in raylib space (`HeroDecorator3D` parents
/// the light to the hero view at local origin) — `None` while there is no
/// living hero, which disables the light for the frame.
fn hero_light_pos(world: &crate::net::SharedWorld) -> Option<Vector3> {
    let hero = world.world.hero.as_ref()?;
    if hero.hitpoints <= 0.0 && hero.hitpoints_max > 0.0 {
        return None;
    }
    let (x, y) = world.hero_pos()?;
    Some(super::scene3d::rl_point(x, 0.0, -y))
}

/// Material maps retain a non-owning handle; the source texture remains
/// owned by either the sprite cache or the 3D texture cache.
fn weak_texture(texture: &Texture2D) -> raylib::texture::WeakTexture2D {
    // SAFETY: TextureCache keeps the corresponding strong texture alive as
    // long as models that reference it are stored in the same cache.
    unsafe { raylib::texture::WeakTexture2D::from_raw(*texture.as_ref()) }
}

fn weak_texture_raw(texture: &raylib::ffi::Texture2D) -> raylib::texture::WeakTexture2D {
    // SAFETY: the raw handle is borrowed from the strongly owned sprite clip.
    unsafe { raylib::texture::WeakTexture2D::from_raw(*texture) }
}

/// Upload one CPU clip; frames that fail to upload are skipped.
fn upload_clip(rl: &mut RaylibHandle, thread: &RaylibThread, clip: &SpriteClip) -> GpuClip {
    let mut frames = Vec::with_capacity(clip.frames.len());
    let mut durations = Vec::with_capacity(clip.frames.len());
    let mut bounds = (
        f32::INFINITY,
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::NEG_INFINITY,
    );
    for f in &clip.frames {
        if let Some(tex) = upload_rgba(rl, thread, &f.image) {
            bounds.0 = bounds.0.min(f.offset.0);
            bounds.1 = bounds.1.min(f.offset.1);
            bounds.2 = bounds.2.max(f.offset.0 + tex.width as f32);
            bounds.3 = bounds.3.max(f.offset.1 + tex.height as f32);
            frames.push(GpuFrame {
                tex,
                offset: f.offset,
            });
            durations.push(f.duration_ms);
        }
    }
    if frames.is_empty() {
        bounds = (0.0, 0.0, 0.0, 0.0);
    }
    GpuClip {
        frames,
        rotation_strip: clip.rotation_strip,
        durations,
        bounds,
        scale9: clip.scale9,
    }
}

/// RGBA8 -> `Texture2D` via raylib's own allocator (zero encode cost).
fn upload_rgba(
    rl: &mut RaylibHandle,
    thread: &RaylibThread,
    img: &image::RgbaImage,
) -> Option<Texture2D> {
    let (w, h) = img.dimensions();
    if w == 0 || h == 0 || w > i32::MAX as u32 || h > i32::MAX as u32 {
        return None;
    }
    let raw = img.as_raw();
    unsafe {
        let data = ffi::MemAlloc(raw.len() as u32);
        if data.is_null() {
            log::warn!("texture upload: MemAlloc({}) failed", raw.len());
            return None;
        }
        std::ptr::copy_nonoverlapping(raw.as_ptr(), data as *mut u8, raw.len());
        // `Image` (thin wrapper) frees `data` via `UnloadImage` -> `MemFree`
        // — the same allocator `MemAlloc` uses.
        let image = Image::from_raw(ffi::Image {
            data,
            width: w as i32,
            height: h as i32,
            mipmaps: 1,
            format: ffi::PixelFormat::PIXELFORMAT_UNCOMPRESSED_R8G8B8A8 as i32,
        });
        match rl.load_texture_from_image(thread, &image) {
            Ok(t) => Some(t),
            Err(e) => {
                log::warn!("texture upload failed for {w}x{h}: {e}");
                None
            }
        }
    }
}
