//! Asset pipeline: resource index, CDN downloads, pattern/asset databases.
//!
//! Mirrors `ResourceManager`/`FileCollection` + `PatternManager` +
//! `UberAssetsLib` from the Flash client:
//!
//! - `resources.xml` (fetched with `__cv=<resourcesXmlHash>`) maps a file id
//!   (`srcKey`) to a CDN path; `FileIndex` owns that table.
//! - The uber-asset XMLs embedded in main.swf (`lib_ships`, `lib_portals`,
//!   `lib_collectables`, `lib_maps`, ...) map a game `assetId`/`typeId`
//!   to a `<display2D srcKey resKey .../>` recipe.
//! - `game.xml` (embedded in main.swf) supplies ship patterns
//!   (expansion/engine/laser classes, click radii, minimap icons),
//!   background patterns and the preload list.
//! - `maps-config` (a `resources.xml` XML file) gives static per-map
//!   descriptors: `<backgrounds>`, `<planets>`, `scaleFactor`.
//!
//! Sprites are produced on demand: `request` queues a CDN download, then
//! `swf_extract` converts the wanted symbol into CPU frames that the
//! render layer uploads to the GPU. Downloads run on a small worker pool
//! (4 threads, each with its own current-thread tokio runtime) sharing one
//! `reqwest::Client` — `pump` drains completions on the UI thread.

use anyhow::{anyhow, Context, Result};
use std::collections::{hash_map::DefaultHasher, HashSet};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

pub mod atf;
pub mod audioxml;
pub mod awd;
pub mod bootstrap;
pub mod collection;
pub mod drones;
pub mod formations;
pub mod fx3d;
pub mod gamexml;
pub mod mapscene;
pub mod mapxml;
pub mod material;
pub mod minepulse;
pub mod particles;
pub mod pods;
pub mod resources;
pub mod resources3d;
pub mod sprites;
pub mod tilemap;
pub mod uberlib;

pub use bootstrap::FlashVars;
pub use formations::DroneFormationPattern;
pub use gamexml::GameXml;
pub use mapscene::{
    CameraZone3D, MapScene3D, SceneLensFlare3D, SceneMesh3D, ScenePlane3D, SceneTilemap3D,
};
pub use mapxml::MapDescriptor;
pub use resources::FileIndex;
pub use sprites::{SpriteClip, SpriteFrame, SpriteId, SpriteStore};
pub use tilemap::TileGrid;
pub use uberlib::{
    Display2D, Display3D, MaterialDesc, Mesh3D, SlotSource, TextureQuality, TextureSlot, UberAssets,
};

/// Decode a cached standalone image file for the offline asset catalogue.
///
/// Game assets use JPEG-XR and Adobe Texture Format in addition to the usual
/// browser image formats. The catalogue's Python renderer cannot decode
/// those formats, so this shared decoder feeds it small PNG previews.
pub fn decode_catalog_image(bytes: &[u8]) -> Result<image::RgbaImage> {
    if bytes.starts_with(b"ATF") {
        atf::decode(bytes)
    } else if bytes.starts_with(b"II\xbc\x01") || bytes.starts_with(b"MM\x01\xbc") {
        tilemap::raster_image("jxr", bytes)
    } else {
        anyhow::bail!("unsupported catalogue image format")
    }
}

use bootstrap::BootConfig;
use collection::Collection;

/// Number of download/decode worker threads.
const WORKERS: usize = 4;

/// What a [`Job`] produces.
enum JobKind {
    /// `(src,res)` — image file or SWF symbol.
    Sprite,
    /// `src` = atlas XML file id, `res` = texture file id — the tile set
    /// of a `StarlingTiledMap` layer (`tiles == true` clip).
    Tilemap,
    /// `src` = atlas XML file id, `res` = `{texture_id}#{region}` — one
    /// named `SubTexture` of a `mapAssetView` atlas (portals).
    AtlasRegion,
    /// Like [`JobKind::AtlasRegion`] but keeps every `SubTexture` whose
    /// name starts with `res`'s region prefix — the `animated_base`
    /// overlay of a `mapAssetView` (e.g. portal 306's idle rotation).
    AtlasRegionAnim,
}

/// One pending `(src,res) -> SpriteClip` job.
struct Job {
    id: SpriteId,
    src: String,
    res: String,
    kind: JobKind,
    /// Per-frame duration for [`JobKind::AtlasRegionAnim`] clips (the
    /// `animated_base` `fps` attr, 0 = use the clip default).
    frame_ms: u32,
    /// Completed attempts — transient CDN failures are requeued until
    /// this reaches [`MAX_JOB_ATTEMPTS`].
    attempts: u8,
}

/// Times a failed job is tried before its clip resolves empty.
const MAX_JOB_ATTEMPTS: u8 = 3;
/// Bump whenever SWF rasterization, atlas extraction, or frame metadata
/// changes. Existing frame caches then become cold without deleting them.
const SPRITE_CACHE_VERSION: u32 = 5;
const WINDOW_SKIN_SYMBOL: &str =
    "net.bigpoint.darkorbit.mvc.common.view.skins.DOWindowSkin_fuckingCunt";
const WINDOW_SKIN_SOURCE: &str = "__main_swf";

/// Sprite ids for the stock `SimpleWindow` chrome (`window` SWF).
/// Each field stays `None` until the resource resolves on the worker —
/// the renderer falls back to the procedural bevel meanwhile.
#[derive(Clone, Copy, Debug, Default)]
pub struct StockWindowSkin {
    /// `backgroundWithTopControls>window` — scale9 frame w/ title bar.
    pub chrome: Option<SpriteId>,
    /// `backgroundNormal>window` — same frame without window controls
    /// (borderless panels: amount dialogs, flyouts).
    pub chrome_plain: Option<SpriteId>,
    /// `backgroundWithTopControls>pattern` — masked body texture.
    pub pattern: Option<SpriteId>,
    /// `closeBtn`/`minimizeBtn` child sprites, up + hover states.
    pub close: Option<SpriteId>,
    pub close_hover: Option<SpriteId>,
    pub minimize: Option<SpriteId>,
    pub minimize_hover: Option<SpriteId>,
}

/// Worker completion: the [`Job`] itself + its result, so transient
/// failures can be requeued by [`AssetManager::pump`].
type Done = (Job, Result<SpriteClip>);

/// Shared job queue + result channel + per-worker state.
struct Pipeline {
    jobs_tx: Sender<Job>,
    /// Workers share the single std-mpsc receiver through a mutex —
    /// `Receiver` is `!Sync`, so the lock serializes `recv()` calls
    /// (one job per worker at a time, FIFO order preserved).
    job_rx: Arc<Mutex<Receiver<Job>>>,
    done_tx: Sender<Done>,
    done_rx: Receiver<Done>,
    /// `(src,res)` already handed to the queue (id in `SpriteStore`).
    queued: HashSet<SpriteId>,
    workers: Vec<JoinHandle<()>>,
}

impl Pipeline {
    fn new() -> Self {
        let (jobs_tx, job_rx) = channel();
        let (done_tx, done_rx) = channel();
        Self {
            jobs_tx,
            job_rx: Arc::new(Mutex::new(job_rx)),
            done_tx,
            done_rx,
            queued: HashSet::new(),
            workers: Vec::new(),
        }
    }
}

/// Central asset facade. CPU-side only — the render layer owns textures.
pub struct AssetManager {
    /// Lazy World3D geometry/texture collection; absent in offline fixtures.
    pub geometries: Option<resources3d::GeometryCollection>,
    /// `resources_3d_particles` — fetched by `"{id}.{type}"` key
    /// (`_useFilesExtensions`), see [`Self::update_particle_group`].
    pub particles_3d: Option<resources3d::GeometryCollection>,
    /// Parsed particle groups by collection id; `None` = unavailable.
    particle_groups: std::collections::HashMap<String, Option<Arc<particles::ParticleGroup>>>,
    pub cache_dir: PathBuf,
    pub files: FileIndex,
    pub game_xml: GameXml,
    pub uber: UberAssets,
    pub maps: mapxml::MapDb,
    pub sprites: SpriteStore,
    /// Older window chrome rasterized from the main SWF.
    pub legacy_window_skin: Option<SpriteId>,
    /// Current blue Spark window background from the quest window SWF.
    pub modern_window_skin: Option<SpriteId>,
    /// The stock `SimpleWindow` chrome — sprites inside the `window` SWF
    /// finisher (`initializeSimpleWindow` loads them via
    /// `backgroundWithTopControls`/`backgroundNormal` containers).
    pub stock_window: StockWindowSkin,
    /// Language table (`BPLocale`) — loaded with the rest of the
    /// bootstrap; empty (identity) when the language is unknown or the
    /// CDN is unreachable. Mirrored into `SharedWorld::locale`.
    pub locale: Arc<crate::locale::LocaleCatalog>,
    /// `resource_base` — `{cdn}{basePath}` prefix for file URLs
    /// (`FileCollection` `m_sBasePath`). Empty until [`Self::load`] fills
    /// it; downloads issued while empty fail their job (permanent empty
    /// clip, same as a missing file).
    base: String,
    /// Player faction id (`factionID` flashvar) — used for the
    /// `{assetId}_{faction}` step of `Ship.getUberDescriptor`.
    faction_id: i16,
    pipeline: Pipeline,
    /// The file index shared with workers (cloned once at first request —
    /// `files` is immutable after `load`).
    worker_files: Option<Arc<FileIndex>>,
}

/// Cloneable off-thread file fetch handle — the same disk-cache/CDN
/// path (`fetch_file`) the sprite workers use, handed to other lazily
/// loaded families (audio loader).
#[derive(Clone)]
pub(crate) struct FileSource {
    pub(crate) http: reqwest::Client,
    pub(crate) base: String,
    pub(crate) files: Arc<FileIndex>,
    pub(crate) collection: Arc<Collection>,
}

impl FileSource {
    /// [`fetch_file`] on a caller-owned tokio runtime.
    pub(crate) fn fetch(
        &self,
        rt: &tokio::runtime::Runtime,
        file_id: &str,
    ) -> Result<Vec<u8>> {
        fetch_file(rt, &self.http, &self.base, &self.files, &self.collection, file_id)
    }
}

/// HTTP fetch helper shared by bootstrap + workers.
pub(crate) async fn fetch_bytes(http: &reqwest::Client, url: &str) -> Result<Vec<u8>> {
    let resp = http
        .get(url)
        .send()
        .await
        .with_context(|| format!("GET {url}"))?
        .error_for_status()
        .with_context(|| format!("GET {url}"))?;
    Ok(resp.bytes().await?.to_vec())
}

/// Classify an embedded binary blob name
/// (`..._InitPatternManagerCommand_GAME_XML_FILE`,
/// `..._UberAssetsLib_LIB_SHIPS_XML`) into the canonical uber lib id
/// (`UberAssetsLib.LIB_*` constants: `LIB_DEFAULT` maps to `lib_assets`,
/// `LIB_BATTLE_STATION` to `lib_battlestation`); `None` when the blob is
/// not a `LIB_*_XML` library.
fn lib_name(symbol: &str) -> Option<String> {
    let start = symbol.find("LIB_")?;
    let rest = &symbol[start..]; // e.g. LIB_SHIPS_XML
    let rest = rest.strip_suffix("_XML")?;
    Some(match rest {
        "LIB_DEFAULT" => "lib_assets".into(),
        "LIB_BATTLE_STATION" => "lib_battlestation".into(),
        other => other.to_ascii_lowercase(),
    })
}

/// `UberAssetsLib.addEmbededLib` registration order — the bare-id search
/// order for `display_2d`.
fn lib_sort_key(lib: &str) -> usize {
    [
        "lib_assets",
        "lib_ships",
        "lib_drones",
        "lib_portals",
        "lib_collectables",
        "lib_battlestation",
        "lib_maps",
        "lib_test",
        "lib_poizones",
    ]
    .iter()
    .position(|l| l == &lib)
    .unwrap_or(usize::MAX)
}

impl AssetManager {
    /// Authored `lib_maps` scene for the current static map descriptor.
    pub fn map_scene3d_for_map(&self, map_id: i32) -> Option<&MapScene3D> {
        let desc = self.maps.get(map_id)?;
        let name = desc.name.trim();
        let prefixed = if name.starts_with("map_") {
            name.to_owned()
        } else {
            format!("map_{name}")
        };
        [name.to_owned(), prefixed, format!("map_{map_id}")]
            .iter()
            .find_map(|key| self.uber.map_scene3d(key))
    }

    pub fn request_geometry(&mut self, id: &str) {
        if let Some(collection) = &mut self.geometries {
            collection.request(id);
        }
    }

    /// Request a particle group (once), parse it when it arrives and queue
    /// its textures and external meshes from the particle collection.
    ///
    /// `id` is the group stem (`box_2`); `resources_3d_particles` keys
    /// files `id.type` (`FileCollection` `_useFilesExtensions`,
    /// `World3D.as:86`), so the archive fetches as `{id}.zip` and anim
    /// dependencies remap `.png -> .atf` / `.obj -> .awd`
    /// (`ParticlesAssetLoaderContext.getRemappedUrl`). `particle_groups`
    /// stays keyed by the bare stem.
    pub fn update_particle_group(&mut self, id: &str) {
        if id.is_empty() || self.particle_groups.contains_key(id) {
            return;
        }
        let Some(collection) = self.particles_3d.as_mut() else {
            return;
        };
        let zip = format!("{id}.zip");
        collection.request(&zip);
        let Some(bytes) = collection.get(&zip) else {
            if collection.failure(&zip).is_some() {
                self.particle_groups.insert(id.to_owned(), None);
            }
            return;
        };
        match particles::ParticleGroup::from_bytes(&bytes) {
            Ok(group) => {
                for anim in &group.anims {
                    if let Some(texture) = &anim.texture {
                        collection.request(&format!("{texture}.atf"));
                    }
                    if let particles::Shape::External(mesh) = &anim.shape {
                        collection.request(&format!("{mesh}.awd"));
                    }
                }
                self.particle_groups
                    .insert(id.to_owned(), Some(Arc::new(group)));
            }
            Err(error) => {
                log::warn!("particle group {id}: {error:#}");
                self.particle_groups.insert(id.to_owned(), None);
            }
        }
    }

    pub fn particle_group(&self, id: &str) -> Option<Arc<particles::ParticleGroup>> {
        self.particle_groups.get(id).cloned().flatten()
    }

    /// ATF texture bytes for a particle anim (`{stem}.atf` in the
    /// particle manifest — `.png` recipe urls remap to `.atf`).
    pub fn particle_texture(&self, stem: &str) -> Option<Arc<[u8]>> {
        self.particles_3d
            .as_ref()
            .and_then(|collection| collection.get(&format!("{stem}.atf")))
    }

    /// AWD mesh bytes for a particle `Shape::External` (`{stem}.awd` —
    /// `.obj` recipe urls remap to `.awd`).
    pub fn particle_geometry(&self, stem: &str) -> Option<Arc<[u8]>> {
        self.particles_3d
            .as_ref()
            .and_then(|collection| collection.get(&format!("{stem}.awd")))
    }

    /// Raw AWD/OBJ geometry or texture bytes from World3D's manifest.
    pub fn geometry(&self, id: &str) -> Option<Arc<[u8]>> {
        self.geometries
            .as_ref()
            .and_then(|collection| collection.get(id))
    }

    /// The `resources_3d` manifest index finished loading — texture-layer
    /// ids can be presence-checked via [`Self::geometry_file_exists`]
    /// instead of queueing a download that will 404.
    pub fn geometry_index_ready(&self) -> bool {
        self.geometries
            .as_ref()
            .is_some_and(|collection| collection.index_ready())
    }

    /// `id` is listed in the `resources_3d` manifest (false until loaded).
    pub fn geometry_file_exists(&self, id: &str) -> bool {
        self.geometries
            .as_ref()
            .is_some_and(|collection| collection.has_file(id))
    }

    /// `id` was requested from `resources_3d` and its fetch definitively
    /// failed (404, oversized, corrupt …). CPU decode failures happen later
    /// in the render-side `ModelLoader` and are not reported here.
    pub fn geometry_failure(&self, id: &str) -> Option<&str> {
        self.geometries
            .as_ref()
            .and_then(|collection| collection.failure(id))
    }

    /// Build from already-loaded local state (used by bootstrap after
    /// fetching/parsing every config document).
    pub fn new(
        cache_dir: PathBuf,
        files: FileIndex,
        game_xml: GameXml,
        uber: UberAssets,
        maps: mapxml::MapDb,
    ) -> Self {
        Self {
            geometries: None,
            particles_3d: None,
            particle_groups: std::collections::HashMap::new(),
            cache_dir,
            files,
            game_xml,
            uber,
            maps,
            sprites: SpriteStore::default(),
            legacy_window_skin: None,
            modern_window_skin: None,
            stock_window: StockWindowSkin::default(),
            locale: Arc::new(crate::locale::LocaleCatalog::default()),
            base: String::new(),
            faction_id: 0,
            pipeline: Pipeline::new(),
            worker_files: None,
        }
    }

    /// Blocking bootstrap: parse the embedded XMLs out of `main_swf`, fetch
    /// `resources.xml` + the `maps-config` document from the CDN described by
    /// `vars`, and build every asset database.
    ///
    /// Called once from the loading worker (off the UI thread). The returned
    /// manager spawns its download pool lazily on the first
    /// [`Self::request`].
    pub fn load(cache_dir: &Path, main_swf: &Path, vars: &FlashVars) -> Result<Self> {
        let swf = crate::swf_extract::SwfLibrary::open(main_swf)
            .with_context(|| format!("cannot open {}", main_swf.display()))?;
        Self::load_with_library(cache_dir, &swf, vars)
    }

    /// [`Self::load`] taking an already-opened SWF — bootstrap downloads the
    /// *live* build (`BootConfig::main_swf_url`) so `VersionHash` and the
    /// embedded XMLs always match the deployed client, and only falls back
    /// to a local file when the download fails.
    pub fn load_with_library(
        cache_dir: &Path,
        swf: &crate::swf_extract::SwfLibrary,
        vars: &FlashVars,
    ) -> Result<Self> {
        let cfg = BootConfig::from_flashvars(vars)?;
        let base = cfg.resource_base();
        let http = crate::net::bootstrap::default_http_client()?;
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .context("tokio runtime for asset bootstrap")?;

        // 1. `{base}xml/resources.xml?__cv={resourcesXmlHash}` — the
        //    `FileCollection` index (`loadResourceFile`).
        let resources_text = rt
            .block_on(fetch_bytes(&http, &cfg.resources_xml_url()))
            .context("could not fetch resources.xml")?;
        let resources_text = String::from_utf8_lossy(&resources_text).into_owned();
        if let Err(e) =
            persist_resource_index(cache_dir, &base, &cfg.resources_xml_url(), &resources_text)
        {
            log::warn!("could not save resources.xml catalogue snapshot: {e:#}");
        }
        let files = FileIndex::parse(&resources_text)?;

        // 2. `maps-config` — static map descriptors (`MapManagerProxy`
        //    loads it by the literal file id "maps-config").
        let mut maps = mapxml::MapDb::default();
        match files.url(&base, "maps-config") {
            Some(url) => match rt.block_on(fetch_bytes(&http, &url)) {
                Ok(bytes) => match mapxml::MapDb::parse(&String::from_utf8_lossy(&bytes)) {
                    Ok(db) => maps = db,
                    Err(e) => log::warn!("maps-config parse failed: {e}"),
                },
                Err(e) => log::warn!("maps-config fetch failed: {e}"),
            },
            None => log::warn!("resources.xml has no maps-config entry"),
        }

        // 3. Embedded XMLs from main.swf DefineBinaryData:
        //    `*GAME_XML_FILE*` -> GameXml, `*LIB_*_XML` -> uber libs.
        let mut game_xml = GameXml::default();
        let mut libs: Vec<(String, String)> = Vec::new();
        for (name, bytes) in swf.binary_blobs() {
            if name.contains("GAME_XML_FILE") {
                game_xml = GameXml::parse(&String::from_utf8_lossy(&bytes))
                    .context("embedded game.xml parse failed")?;
            } else if let Some(short) = lib_name(&name) {
                libs.push((short, String::from_utf8_lossy(&bytes).into_owned()));
            }
        }
        libs.sort_by_key(|a| lib_sort_key(&a.0));
        let lib_refs: Vec<(&str, &str)> =
            libs.iter().map(|(n, t)| (n.as_str(), t.as_str())).collect();
        let uber = UberAssets::parse(&lib_refs)?;

        // 4. Language tables (`InitBPLocaleCommand`) — failures degrade to
        //    an empty catalog, never abort the bootstrap.
        let locale = crate::locale::source::load_catalog(&rt, &http, &cfg, cache_dir);

        let mut mgr = Self::new(cache_dir.to_owned(), files, game_xml, uber, maps);
        mgr.locale = Arc::new(locale);
        mgr.base = base;
        let static_host = if cfg.cdn_host.is_empty() {
            cfg.dynamic_host.clone()
        } else {
            cfg.cdn_host.clone()
        };
        mgr.geometries = Some(resources3d::GeometryCollection::new(
            static_host.clone(),
            cfg.base_path.clone(),
            cfg.resources_3d_xml_hash.clone(),
            cache_dir.to_owned(),
            http.clone(),
        ));
        mgr.particles_3d = Some(resources3d::GeometryCollection::particles(
            static_host,
            cfg.base_path.clone(),
            cfg.resources_3d_particles_xml_hash.clone(),
            cache_dir.to_owned(),
            http.clone(),
        ));
        mgr.faction_id = cfg.faction_id;
        mgr.legacy_window_skin =
            load_window_skin(&mut mgr.sprites, cache_dir, swf, &cfg.gameclient_hash);
        // The modern Flex/Spark window skin uses the live questWin SWF's
        // scale9 bitmap, rather than the dated main.swf export.
        mgr.modern_window_skin = Some(mgr.request("questWin", "newWinBg_x10y37"));
        // The classic window chrome (`SimpleWindow.as`): the `window`
        // finisher's `window` sprite carries a scale9 grid, `pattern` is
        // the masked body texture, `closeBtn`/`minimizeBtn` hold the real
        // `background`/`rollOverState`/`mouseDownState` child sprites.
        mgr.stock_window = StockWindowSkin {
            chrome: Some(mgr.request("window", "backgroundWithTopControls>window")),
            chrome_plain: Some(mgr.request("window", "backgroundNormal>window")),
            pattern: Some(mgr.request("window", "backgroundWithTopControls>pattern")),
            close: Some(mgr.request("window", "backgroundWithTopControls>closeBtn>background")),
            close_hover: Some(
                mgr.request("window", "backgroundWithTopControls>closeBtn>rollOverState"),
            ),
            minimize: Some(
                mgr.request("window", "backgroundWithTopControls>minimizeBtn>background"),
            ),
            minimize_hover: Some(mgr.request(
                "window",
                "backgroundWithTopControls>minimizeBtn>rollOverState",
            )),
        };
        Ok(mgr)
    }

    /// Queue a `(srcKey,resKey)` sprite for download+extraction; returns the
    /// id used to query `get`. Cheap to call every frame — deduped inside.
    /// `res_key == ""` means "whole file": decode image files directly or
    /// use the default `"mc"` symbol for SWFs.
    pub fn request(&mut self, src_key: &str, res_key: &str) -> SpriteId {
        let id = self.sprites.ensure(src_key, res_key);
        if self.sprites.resolved(id) || self.pipeline.queued.contains(&id) {
            return id;
        }
        self.pipeline.queued.insert(id);
        self.spawn_workers();
        let job = Job {
            id,
            src: src_key.to_string(),
            res: res_key.to_string(),
            kind: JobKind::Sprite,
            frame_ms: 0,
            attempts: 0,
        };
        if self.pipeline.jobs_tx.send(job).is_err() {
            log::warn!("asset request dropped (pipeline down): {src_key}");
            self.pipeline.queued.remove(&id);
            self.sprites.insert(id, SpriteClip::default());
        }
        id
    }

    /// Queue a `StarlingTiledMap` tile map: `atlas_xml_id` is the
    /// `TextureAtlas` XML file id, `texture_id` the tile-sheet image file
    /// id (png/jpg/jxr). The clip resolves `tiles == true` with one frame
    /// per `SubTexture` in `getTextures()` name order. Deduped under the
    /// key `(atlas_xml_id, "tile:{texture_id}")`.
    pub fn request_tilemap(&mut self, atlas_xml_id: &str, texture_id: &str) -> SpriteId {
        let id = self
            .sprites
            .ensure(atlas_xml_id, &format!("tile:{texture_id}"));
        if self.sprites.resolved(id) || self.pipeline.queued.contains(&id) {
            return id;
        }
        self.pipeline.queued.insert(id);
        self.spawn_workers();
        let job = Job {
            id,
            src: atlas_xml_id.to_string(),
            res: texture_id.to_string(),
            kind: JobKind::Tilemap,
            frame_ms: 0,
            attempts: 0,
        };
        if self.pipeline.jobs_tx.send(job).is_err() {
            log::warn!("tilemap request dropped (pipeline down): {atlas_xml_id}");
            self.pipeline.queued.remove(&id);
            self.sprites.insert(id, SpriteClip::default());
        }
        id
    }

    /// Queue one named `SubTexture` of a `mapAssetView` atlas (portal
    /// recipes: `atlasKey` XML + `textureKey` PNG + `texture` region).
    /// Deduped under `(atlas_xml_id, "{texture_id}#{region}")`.
    pub fn request_region(
        &mut self,
        atlas_xml_id: &str,
        texture_id: &str,
        region: &str,
    ) -> SpriteId {
        let res = format!("{texture_id}#{region}");
        let id = self.sprites.ensure(atlas_xml_id, &res);
        if self.sprites.resolved(id) || self.pipeline.queued.contains(&id) {
            return id;
        }
        self.pipeline.queued.insert(id);
        self.spawn_workers();
        let job = Job {
            id,
            src: atlas_xml_id.to_string(),
            res,
            kind: JobKind::AtlasRegion,
            frame_ms: 0,
            attempts: 0,
        };
        if self.pipeline.jobs_tx.send(job).is_err() {
            log::warn!("region request dropped (pipeline down): {atlas_xml_id}");
            self.pipeline.queued.remove(&id);
            self.sprites.insert(id, SpriteClip::default());
        }
        id
    }

    /// Queue a `mapAssetView` layer. `prefix` empty plays the whole atlas
    /// (`portal_jump`); an exact region name is one still; a frame stem
    /// (`portal_305_000`) is the numbered movie clip. `frame_ms` is the
    /// recipe `fps` (0 → 30fps once more than one region matches).
    pub fn request_map_layer(
        &mut self,
        atlas_xml_id: &str,
        texture_id: &str,
        prefix: &str,
        frame_ms: u32,
    ) -> SpriteId {
        let res = format!("{texture_id}#layer:{prefix}:{frame_ms}");
        let id = self.sprites.ensure(atlas_xml_id, &res);
        if self.sprites.resolved(id) || self.pipeline.queued.contains(&id) {
            return id;
        }
        self.pipeline.queued.insert(id);
        self.spawn_workers();
        let job = Job {
            id,
            src: atlas_xml_id.to_string(),
            res,
            kind: JobKind::AtlasRegionAnim,
            frame_ms,
            attempts: 0,
        };
        if self.pipeline.jobs_tx.send(job).is_err() {
            log::warn!("map-layer request dropped (pipeline down): {atlas_xml_id}");
            self.pipeline.queued.remove(&id);
            self.sprites.insert(id, SpriteClip::default());
        }
        id
    }

    /// Queue a `mapAssetView` `animated_base` clip: every `SubTexture`
    /// whose name starts with `prefix` becomes one frame (name-sorted,
    /// `frame_ms` apart — the recipe's `fps`). Deduped under
    /// `(atlas_xml_id, "{texture_id}#anim:{prefix}:{frame_ms}")`.
    pub fn request_region_anim(
        &mut self,
        atlas_xml_id: &str,
        texture_id: &str,
        prefix: &str,
        frame_ms: u32,
    ) -> SpriteId {
        let res = format!("{texture_id}#anim:{prefix}:{frame_ms}");
        let id = self.sprites.ensure(atlas_xml_id, &res);
        if self.sprites.resolved(id) || self.pipeline.queued.contains(&id) {
            return id;
        }
        self.pipeline.queued.insert(id);
        self.spawn_workers();
        let job = Job {
            id,
            src: atlas_xml_id.to_string(),
            res,
            kind: JobKind::AtlasRegionAnim,
            frame_ms,
            attempts: 0,
        };
        if self.pipeline.jobs_tx.send(job).is_err() {
            log::warn!("region-anim request dropped (pipeline down): {atlas_xml_id}");
            self.pipeline.queued.remove(&id);
            self.sprites.insert(id, SpriteClip::default());
        }
        id
    }

    /// File fetch handle for non-sprite loaders (audio): shares the file
    /// index, CDN base and disk cache with the sprite workers.
    pub(crate) fn file_source(&mut self) -> FileSource {
        FileSource {
            http: crate::net::bootstrap::default_http_client()
                .unwrap_or_else(|_| reqwest::Client::new()),
            base: self.base.clone(),
            files: self
                .worker_files
                .get_or_insert_with(|| Arc::new(self.files.clone()))
                .clone(),
            collection: Arc::new(Collection::new(self.cache_dir.clone())),
        }
    }

    /// Lazily start the worker pool (first `request`). Each worker owns a
    /// current-thread tokio runtime for `reqwest` and drains the shared
    /// job queue.
    fn spawn_workers(&mut self) {
        if !self.pipeline.workers.is_empty() {
            return;
        }
        let files = self
            .worker_files
            .get_or_insert_with(|| Arc::new(self.files.clone()))
            .clone();
        let collection = Arc::new(Collection::new(self.cache_dir.clone()));
        let http =
            crate::net::bootstrap::default_http_client().unwrap_or_else(|_| reqwest::Client::new());
        for i in 0..WORKERS {
            let rx = self.pipeline.job_rx.clone();
            let done = self.pipeline.done_tx.clone();
            let http = http.clone();
            let files = files.clone();
            let collection = collection.clone();
            let base = self.base.clone();
            let handle = std::thread::Builder::new()
                .name(format!("asset-worker-{i}"))
                .spawn(move || worker_loop(rx, done, http, base, files, collection))
                .expect("spawn asset worker");
            self.pipeline.workers.push(handle);
        }
    }

    /// Pump pending downloads/extractions (non-blocking). Transient
    /// failures (CDN timeouts etc.) are requeued up to
    /// [`MAX_JOB_ATTEMPTS`]; only a job that keeps failing resolves to an
    /// empty clip permanently (logged once per attempt).
    pub fn pump(&mut self) {
        if let Some(collection) = &mut self.geometries {
            collection.pump();
        }
        if let Some(collection) = &mut self.particles_3d {
            collection.pump();
        }
        while let Ok((mut job, result)) = self.pipeline.done_rx.try_recv() {
            match result {
                Ok(clip) => {
                    self.pipeline.queued.remove(&job.id);
                    self.sprites.insert(job.id, clip);
                }
                Err(e) => {
                    job.attempts += 1;
                    let id = job.id;
                    let failed_src = job.src.clone();
                    let failed_res = job.res.clone();
                    let failed_attempt = job.attempts;
                    if job.attempts < MAX_JOB_ATTEMPTS && self.pipeline.jobs_tx.send(job).is_ok() {
                        log::warn!(
                            "asset {}/{} decode/download failed (attempt {}/{}; retrying): {e:#}",
                            failed_src,
                            failed_res,
                            failed_attempt,
                            MAX_JOB_ATTEMPTS,
                        );
                        continue;
                    }
                    self.pipeline.queued.remove(&id);
                    log::warn!(
                        "asset {failed_src}/{failed_res} failed after {failed_attempt} attempts: {e:#}"
                    );
                    self.sprites.insert(id, SpriteClip::default());
                }
            }
        }
    }

    /// Look up a ready sprite, if extraction finished.
    pub fn get(&self, id: SpriteId) -> Option<&SpriteClip> {
        self.sprites.get(id)
    }

    /// `lib`-scoped uber display lookup (`getDescriptor(id, lib)`).
    fn uber_lib_display(&self, lib: &str, id: &str) -> Option<Display2D> {
        self.uber.display_2d(&format!("{lib}:{id}"))
    }

    /// Resolve a ship entity `typeId`/`assetId` to its display recipe —
    /// `Ship.getUberDescriptor` (Ship.as:234-236):
    ///
    /// ```text
    /// {assetId}_{factionId}_{assetIdExtension}
    /// {assetId}_{assetIdExtension}
    /// {assetId}_{factionId}
    /// {assetId}
    /// pet fallback: {typeId}, "pet-basic-" + max(1, petLevel)
    /// ```
    ///
    /// `assetIdExtension` is a runtime ship-state suffix (cloaked/damaged
    /// variants) we can't know here — the two `*_extension` candidates are
    /// skipped. `{assetId}_{faction}` uses `faction`, the entity's own
    /// `factionID` (Ship.as:237 `info.factionID`); when it is 0/absent the
    /// player faction (`factionID` flashvar, itself often missing) fills
    /// in. Everything resolves within `lib_ships`.
    pub fn ship_display(&self, type_id: &str, faction: i32) -> Option<Display2D> {
        self.ship_display_lvl(type_id, faction, 0)
    }

    /// Resolve a ship's native 3D mesh recipe using the same ship pattern
    /// and faction-specific design key as its 2D recipe.
    pub fn ship_display3d(&self, type_id: &str, faction: i32) -> Option<Display3D> {
        let pat = self.game_xml.ship(type_id).or_else(|| {
            type_id
                .parse::<i32>()
                .ok()
                .and_then(|id| self.game_xml.ship_by_npc(id))
        });
        let asset_id = pat.map(|p| p.asset_id.as_str()).unwrap_or(type_id);
        let faction = if faction != 0 {
            faction
        } else {
            i32::from(self.faction_id)
        };
        if faction != 0 {
            if let Some(d) = self
                .uber
                .display_3d(&format!("lib_ships:{asset_id}_{faction}"))
            {
                return Some(d);
            }
        }
        self.uber
            .display_3d(&format!("lib_ships:{asset_id}"))
            .or_else(|| {
                (asset_id != type_id)
                    .then(|| self.uber.display_3d(&format!("lib_ships:{type_id}")))
                    .flatten()
            })
    }

    /// [`Self::ship_display`] with the pet level known (`PetTrait.level`):
    /// the `pet-basic-{max(1, level)}` fallback tries that exact level
    /// first. `pet_level == 0` = unknown / not a pet.
    pub fn ship_display_lvl(
        &self,
        type_id: &str,
        faction: i32,
        pet_level: i32,
    ) -> Option<Display2D> {
        // Pattern lookup by `type` then numeric `npcId` (mirrors
        // `render::ship_pattern`).
        let pat = self.game_xml.ship(type_id).or_else(|| {
            type_id
                .parse::<i32>()
                .ok()
                .and_then(|id| self.game_xml.ship_by_npc(id))
        });
        let asset_id = pat.map(|p| p.asset_id.as_str()).unwrap_or(type_id);
        // {assetId}_{faction} then {assetId} (extension steps unavailable).
        let faction = if faction != 0 {
            faction
        } else {
            i32::from(self.faction_id)
        };
        if faction != 0 {
            let keyed = format!("{asset_id}_{faction}");
            if let Some(d) = self.uber_lib_display("lib_ships", &keyed) {
                return Some(d);
            }
        }
        if let Some(d) = self.uber_lib_display("lib_ships", asset_id) {
            return Some(d);
        }
        // Pet fallback (`getPetDescriptor`, Ship.as:240) — pets only
        // (`PetTrait` exists exclusively on `defaultCharacter="pet"`
        // ships): candidates are the pattern's own `typeId` then
        // `pet-basic-{level}`. Pet level isn't in the world model, so
        // probe the shipped range (`pet-basic-0` doesn't exist).
        if pat.is_some_and(|p| p.default_character == "pet") {
            let pattern_type = pat.map(|p| p.type_id.as_str()).unwrap_or(type_id);
            if let Some(d) = self.uber_lib_display("lib_ships", pattern_type) {
                return Some(d);
            }
            if pet_level > 0 {
                let pet = format!("pet-basic-{}", pet_level.clamp(1, 20));
                if let Some(d) = self.uber_lib_display("lib_ships", &pet) {
                    return Some(d);
                }
            }
            for level in 1..=20 {
                let pet = format!("pet-basic-{level}");
                if let Some(d) = self.uber_lib_display("lib_ships", &pet) {
                    return Some(d);
                }
            }
        }
        // `getDebugDescription` — nothing sensible; miss.
        None
    }

    /// A named `<pivot>` on a ship's `lib_ships` `display2D` (same asset
    /// candidates as [`Self::ship_display`]) — the `dock_N` mounts a
    /// mothership places its minions on.
    pub fn ship_pivot(&self, type_id: &str, faction: i32, pivot_id: &str) -> Option<(f32, f32)> {
        let pat = self.game_xml.ship(type_id).or_else(|| {
            type_id
                .parse::<i32>()
                .ok()
                .and_then(|id| self.game_xml.ship_by_npc(id))
        });
        let asset_id = pat.map(|p| p.asset_id.as_str()).unwrap_or(type_id);
        let faction = if faction != 0 {
            faction
        } else {
            i32::from(self.faction_id)
        };
        if faction != 0 {
            if let Some(p) = self
                .uber
                .display_pivot(&format!("lib_ships:{asset_id}_{faction}"), pivot_id)
            {
                return Some(p);
            }
        }
        self.uber
            .display_pivot(&format!("lib_ships:{asset_id}"), pivot_id)
    }

    /// Resolve a collectable asset id (`ore_N`, `box_*`, `beacon_N`,
    /// `mine_N`, `firework_box`) — `lib_collectables` then `lib_assets`
    /// (mines live there), then any lib.
    pub fn collectable_display(&self, asset_id: &str) -> Option<Display2D> {
        self.uber_lib_display("lib_collectables", asset_id)
            .or_else(|| self.uber_lib_display("lib_assets", asset_id))
            .or_else(|| self.uber.display_2d(asset_id))
    }

    /// Resolve any uber-asset id (portal, station, beacon, ...) — searches
    /// every library (portals carry bare numeric ids like `"1"`).
    pub fn uber_display(&self, asset_id: &str) -> Option<Display2D> {
        self.uber.display_2d(asset_id)
    }

    /// Resolve the authentic Away3D mesh recipe for an asset. A `None`
    /// result means the source library has no usable `<display3D>` mesh.
    pub fn uber_display3d(&self, asset_id: &str) -> Option<Display3D> {
        self.uber.display_3d(asset_id)
    }
}

fn load_window_skin(
    sprites: &mut SpriteStore,
    cache_dir: &Path,
    swf: &crate::swf_extract::SwfLibrary,
    gameclient_hash: &str,
) -> Option<SpriteId> {
    let res = WINDOW_SKIN_SYMBOL;
    let id = sprites.ensure(WINDOW_SKIN_SOURCE, res);
    let build = if gameclient_hash.is_empty() {
        swf.version_hash().unwrap_or_else(|| "local".to_owned())
    } else {
        gameclient_hash.to_owned()
    };
    let key = format!("main-{build}-extractor-{SPRITE_CACHE_VERSION}");
    let collection = Collection::new(cache_dir.to_owned());
    let dir = collection.versioned_sprite_dir(WINDOW_SKIN_SOURCE, res, &key);
    let clip = read_cached_clip(&dir).or_else(|| {
        let frames = match swf.render_symbol(res) {
            Ok(frames) => frames,
            Err(e) => {
                log::warn!("embedded window skin unavailable ({e})");
                return None;
            }
        };
        let duration_ms = if swf.frame_rate > 0.0 {
            (1000.0 / swf.frame_rate) as u32
        } else {
            33
        };
        let clip = SpriteClip {
            frames: frames
                .into_iter()
                .map(|f| SpriteFrame {
                    image: f.image,
                    offset: f.offset,
                    duration_ms,
                })
                .collect(),
            rotation_strip: false,
            tiles: false,
            scale9: swf.scaling_grid(res),
        };
        if let Err(e) = write_cached_clip(&dir, &clip) {
            log::warn!("embedded window skin cache write failed: {e}");
        }
        Some(clip)
    })?;
    sprites.insert(id, clip);
    Some(id)
}

// ---------------------------------------------------------------------------
// Worker pipeline
// ---------------------------------------------------------------------------

fn worker_loop(
    rx: Arc<Mutex<Receiver<Job>>>,
    done: Sender<Done>,
    http: reqwest::Client,
    base: String,
    files: Arc<FileIndex>,
    collection: Arc<Collection>,
) {
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            log::error!("asset worker: no tokio runtime: {e}");
            return;
        }
    };
    loop {
        // Lock only for the recv: one worker dequeues at a time.
        let job = {
            let guard = match rx.lock() {
                Ok(g) => g,
                Err(p) => p.into_inner(),
            };
            guard.recv()
        };
        let Ok(job) = job else {
            return; // all senders dropped — shutdown
        };
        let result = run_job(&rt, &http, &base, &files, &collection, &job);
        if done.send((job, result)).is_err() {
            return;
        }
    }
}

/// Fetch (or cache-hit) the raw file then decode to a `SpriteClip`.
fn run_job(
    rt: &tokio::runtime::Runtime,
    http: &reqwest::Client,
    base: &str,
    files: &FileIndex,
    collection: &Collection,
    job: &Job,
) -> Result<SpriteClip> {
    match job.kind {
        // Tile map: `src` = atlas XML, `res` = tile-sheet texture.
        JobKind::Tilemap => {
            let atlas = fetch_file(rt, http, base, files, collection, &job.src)?;
            let texture = fetch_file(rt, http, base, files, collection, &job.res)?;
            let key = clip_cache_key(files, job, &[(&job.src, &atlas), (&job.res, &texture)]);
            cached_or_build(collection, &job.src, &job.res, &key, || {
                tilemap::tilemap_clip(&atlas, &file_kind(files, &job.res, &texture), &texture)
            })
        }
        // `res` encodes `{texture_id}#{region}` (see `request_region`);
        // `AtlasRegionAnim` uses `{texture_id}#anim:{prefix}:{ms}`.
        JobKind::AtlasRegion | JobKind::AtlasRegionAnim => {
            let (texture_id, region) = job
                .res
                .split_once('#')
                .context("region job: malformed res key")?;
            let atlas = fetch_file(rt, http, base, files, collection, &job.src)?;
            let texture = fetch_file(rt, http, base, files, collection, texture_id)?;
            let kind = file_kind(files, texture_id, &texture);
            let key = clip_cache_key(files, job, &[(&job.src, &atlas), (texture_id, &texture)]);
            cached_or_build(collection, &job.src, &job.res, &key, || match job.kind {
                JobKind::AtlasRegionAnim => {
                    if let Some(rest) = region.strip_prefix("layer:") {
                        let prefix = rest
                            .rsplit_once(':')
                            .map(|(p, _)| p)
                            .context("map-layer job: malformed res key")?;
                        tilemap::map_asset_clip(&atlas, &kind, &texture, prefix, job.frame_ms)
                    } else {
                        let prefix = region
                            .strip_prefix("anim:")
                            .and_then(|s| s.rsplit_once(':').map(|(p, _)| p))
                            .context("region-anim job: malformed res key")?;
                        tilemap::region_clip_prefix(&atlas, &kind, &texture, prefix, job.frame_ms)
                    }
                }
                _ => tilemap::region_clip(&atlas, &kind, &texture, region),
            })
        }
        JobKind::Sprite => {
            let bytes = fetch_file(rt, http, base, files, collection, &job.src)?;
            let key = clip_cache_key(files, job, &[(&job.src, &bytes)]);
            cached_or_build(collection, &job.src, &job.res, &key, || {
                build_clip(files, &bytes, job)
            })
        }
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct CachedFrame {
    offset: (f32, f32),
    duration_ms: u32,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct CachedClipMeta {
    version: u32,
    rotation_strip: bool,
    tiles: bool,
    #[serde(default)]
    scale9: Option<(f32, f32, f32, f32)>,
    frames: Vec<CachedFrame>,
}

/// Look up decoded frames before parsing/rasterizing the source. A miss is
/// built on the asset worker and saved for the next launch.
fn cached_or_build(
    collection: &Collection,
    src: &str,
    res: &str,
    key: &str,
    build: impl FnOnce() -> Result<SpriteClip>,
) -> Result<SpriteClip> {
    let dir = collection.versioned_sprite_dir(src, res, key);
    if let Some(clip) = read_cached_clip(&dir) {
        return Ok(clip);
    }
    let clip = build()?;
    if let Err(e) = write_cached_clip(&dir, &clip) {
        log::warn!("sprite cache write failed ({src}/{res}): {e}");
    }
    Ok(clip)
}

fn read_cached_clip(dir: &Path) -> Option<SpriteClip> {
    let meta: CachedClipMeta =
        serde_json::from_slice(&std::fs::read(dir.join("meta.json")).ok()?).ok()?;
    if meta.version != SPRITE_CACHE_VERSION {
        return None;
    }
    let mut frames = Vec::with_capacity(meta.frames.len());
    for (i, frame) in meta.frames.iter().enumerate() {
        let image = image::open(dir.join(format!("f{i:03}.png")))
            .ok()?
            .into_rgba8();
        frames.push(SpriteFrame {
            image,
            offset: frame.offset,
            duration_ms: frame.duration_ms,
        });
    }
    Some(SpriteClip {
        frames,
        rotation_strip: meta.rotation_strip,
        tiles: meta.tiles,
        scale9: meta.scale9,
    })
}

fn write_cached_clip(dir: &Path, clip: &SpriteClip) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    let mut frames = Vec::with_capacity(clip.frames.len());
    for (i, frame) in clip.frames.iter().enumerate() {
        frame.image.save(dir.join(format!("f{i:03}.png")))?;
        frames.push(CachedFrame {
            offset: frame.offset,
            duration_ms: frame.duration_ms,
        });
    }
    let meta = CachedClipMeta {
        version: SPRITE_CACHE_VERSION,
        rotation_strip: clip.rotation_strip,
        tiles: clip.tiles,
        scale9: clip.scale9,
        frames,
    };
    let bytes = serde_json::to_vec(&meta)?;
    let tmp = dir.join("meta.json.tmp");
    std::fs::write(&tmp, bytes)?;
    let final_path = dir.join("meta.json");
    if final_path.exists() {
        std::fs::remove_file(&final_path)?;
    }
    std::fs::rename(tmp, final_path)?;
    Ok(())
}

/// Cache key includes the source resource hash, symbol/atlas request,
/// animation timing and extractor version. If a resource has no CDN hash,
/// its downloaded bytes supply the identity.
fn clip_cache_key(files: &FileIndex, job: &Job, inputs: &[(&str, &[u8])]) -> String {
    let mut hasher = DefaultHasher::new();
    SPRITE_CACHE_VERSION.hash(&mut hasher);
    job.res.hash(&mut hasher);
    job.frame_ms.hash(&mut hasher);
    for (id, bytes) in inputs {
        id.hash(&mut hasher);
        if let Some(entry) = files.entry(id) {
            entry.hash.hash(&mut hasher);
            entry.version.hash(&mut hasher);
            entry.kind.hash(&mut hasher);
        }
        if files.entry(id).is_none_or(|e| e.hash.is_empty()) {
            bytes.hash(&mut hasher);
        }
    }
    format!("v{SPRITE_CACHE_VERSION}-{:016x}", hasher.finish())
}

/// Keep the live CDN index and its public URL base beside the downloaded
/// assets so offline tools can enumerate resources that the renderer has not
/// requested yet. No account/session credentials are persisted here.
fn persist_resource_index(cache_dir: &Path, base: &str, index_url: &str, xml: &str) -> Result<()> {
    std::fs::create_dir_all(cache_dir)?;
    let index_path = cache_dir.join("resources.xml");
    let metadata_path = cache_dir.join("resource-catalog.json");
    let mut index_tmp = index_path.as_os_str().to_owned();
    index_tmp.push(".tmp");
    std::fs::write(&index_tmp, xml)?;
    std::fs::rename(&index_tmp, &index_path)?;

    let metadata = serde_json::json!({
        "schema": 1,
        "resource_base": base,
        "resources_xml_url": index_url,
        "resources_xml": "resources.xml",
        "retrieved_unix_seconds": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
    });
    let mut metadata_tmp = metadata_path.as_os_str().to_owned();
    metadata_tmp.push(".tmp");
    std::fs::write(&metadata_tmp, serde_json::to_vec_pretty(&metadata)?)?;
    std::fs::rename(&metadata_tmp, &metadata_path)?;
    Ok(())
}

/// Disk-cache lookup, then CDN download + store (`FileCollection.load`).
fn fetch_file(
    rt: &tokio::runtime::Runtime,
    http: &reqwest::Client,
    base: &str,
    files: &FileIndex,
    collection: &Collection,
    file_id: &str,
) -> Result<Vec<u8>> {
    let version = files
        .entry(file_id)
        .map(|entry| {
            if entry.hash.is_empty() {
                format!("v{}", entry.version)
            } else {
                format!("h{}", entry.hash)
            }
        })
        .unwrap_or_else(|| "unknown".to_owned());
    if let Some(path) = collection.cached_file_version(file_id, &version) {
        return std::fs::read(&path).with_context(|| format!("read cached {}", path.display()));
    }
    let url = files
        .url(base, file_id)
        .ok_or_else(|| anyhow!("no resources.xml entry for {file_id}"))?;
    let bytes = rt.block_on(fetch_bytes(http, &url))?;
    if let Err(e) = collection.store_file_version(file_id, &version, &bytes) {
        log::warn!("cache write failed for {file_id}: {e}");
    }
    Ok(bytes)
}

/// resources.xml `type` attr, falling back to magic-byte sniffing.
fn file_kind(files: &FileIndex, id: &str, bytes: &[u8]) -> String {
    files
        .entry(id)
        .map(|e| e.kind.to_ascii_lowercase())
        .unwrap_or_else(|| sniff_kind(bytes))
}

/// Decode downloaded bytes into a `SpriteClip`.
///
/// `png/jpg/jxr/wdp/atf` -> single frame; `swf/swc` ->
/// `SwfLibrary::render_symbol(res || "mc")`; non-image config/media files
/// resolve empty with a warning.
fn build_clip(files: &FileIndex, bytes: &[u8], job: &Job) -> Result<SpriteClip> {
    let kind = file_kind(files, &job.src, bytes);
    match kind.as_str() {
        "png" | "jpg" | "jpeg" | "bmp" | "gif" | "jxr" | "wdp" => raster_clip(&kind, bytes),
        "swf" | "swc" => {
            let res = if job.res.is_empty() {
                "mc"
            } else {
                job.res.as_str()
            };
            swf_clip(bytes, res)
        }
        "atf" => atf_clip(bytes),
        "xml" | "txt" | "js" | "json" | "css" | "flv" | "mp3" | "zip" => {
            log::warn!("{}: {kind} files carry no sprites (empty clip)", job.src);
            Ok(SpriteClip::default())
        }
        // Unknown kind: sniff once more, then try the image decoder.
        _ => match sniff_kind(bytes).as_str() {
            "png" | "jpg" | "jxr" => raster_clip(&kind, bytes),
            "swf" => swf_clip(bytes, if job.res.is_empty() { "mc" } else { &job.res }),
            _ => raster_clip(&kind, bytes).or_else(|_| {
                log::warn!("{}: undecodable file (empty clip)", job.src);
                Ok(SpriteClip::default())
            }),
        },
    }
}

fn atf_clip(bytes: &[u8]) -> Result<SpriteClip> {
    let image = atf::decode(bytes).context("ATF texture decode")?;
    Ok(SpriteClip {
        frames: vec![SpriteFrame {
            image,
            offset: (0.0, 0.0),
            duration_ms: 0,
        }],
        rotation_strip: false,
        tiles: false,
        scale9: None,
    })
}

/// Magic-byte sniffing for ids missing from the index.
fn sniff_kind(bytes: &[u8]) -> String {
    if bytes.starts_with(b"\x89PNG") {
        "png".into()
    } else if bytes.starts_with(b"\xFF\xD8") {
        "jpg".into()
    } else if bytes.starts_with(b"FWS") || bytes.starts_with(b"CWS") || bytes.starts_with(b"ZWS") {
        "swf".into()
    } else if bytes.starts_with(b"ATF") {
        "atf".into()
    } else if bytes.starts_with(b"\x49\x49\xBC\x01") {
        "jxr".into()
    } else {
        String::new()
    }
}

/// Whole-image decode (png/jpg/jxr) → single frame (offset 0, duration 0).
fn raster_clip(kind: &str, bytes: &[u8]) -> Result<SpriteClip> {
    let img = tilemap::raster_image(kind, bytes)?;
    Ok(SpriteClip {
        frames: vec![SpriteFrame {
            image: img,
            offset: (0.0, 0.0),
            duration_ms: 0,
        }],
        rotation_strip: false,
        tiles: false,
        scale9: None,
    })
}

/// Rasterize one SWF symbol (default `mc`) — every timeline frame is kept;
/// `frames.len() > 1` marks a rotation strip (`ShipClip.as` semantics).
/// `res` may be an export name or a `parent>child>…` display-list path.
fn swf_clip(bytes: &[u8], res: &str) -> Result<SpriteClip> {
    let lib = crate::swf_extract::SwfLibrary::from_bytes(bytes)?;
    // `mc` is the default game symbol; when it's missing fall back to the
    // file's sole export (e.g. `mask` SWFs) before giving up.
    let frames = lib
        .render_symbol(res)
        .or_else(|e| {
            if lib.symbols.len() == 1 {
                let only = lib.symbols.keys().next().unwrap().clone();
                lib.render_symbol(&only)
            } else {
                Err(e)
            }
        })
        .with_context(|| format!("symbol {res}"))?;
    let duration_ms = if lib.frame_rate > 0.0 {
        (1000.0 / lib.frame_rate) as u32
    } else {
        33
    };
    let rotation_strip = frames.len() > 1;
    Ok(SpriteClip {
        frames: frames
            .into_iter()
            .map(|f| SpriteFrame {
                image: f.image,
                offset: f.offset,
                duration_ms,
            })
            .collect(),
        rotation_strip,
        tiles: false,
        scale9: lib.scaling_grid(res),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> String {
        std::fs::read_to_string(Path::new("tests/fixtures").join(name))
            .unwrap_or_else(|e| panic!("cannot read {name}: {e}"))
    }

    /// Manager over the real `game.xml` + `lib_ships.xml` fixtures;
    /// `faction_id` (the flashvar fallback) stays 0.
    fn ships_manager() -> AssetManager {
        let game_xml = GameXml::parse(&fixture("game.xml")).expect("game.xml parses");
        let ships = fixture("lib_ships.xml");
        let uber = UberAssets::parse(&[("lib_ships", ships.as_str())]).expect("lib_ships parses");
        AssetManager::new(
            PathBuf::from("target/asset-test-cache"),
            FileIndex::default(),
            game_xml,
            uber,
            mapxml::MapDb::default(),
        )
    }

    /// `Ship.getUberDescriptor` `{assetId}_{faction}` step uses the
    /// entity's own factionId (lib_ships `ship_aegis_{1,2,3}` → mmo/eic/vru).
    #[test]
    fn ship_display_uses_entity_faction() {
        let mgr = ships_manager();
        // `ship_aegis` has no `assetId` attr -> asset_id == type_id.
        let d = mgr.ship_display("ship_aegis", 2).expect("eic variant");
        assert_eq!(d.src_key, "ship_aegis-eic");
        let d = mgr.ship_display("ship_aegis", 1).expect("mmo variant");
        assert_eq!(d.src_key, "ship_aegis-mmo");
        let d = mgr.ship_display("ship_aegis", 3).expect("vru variant");
        assert_eq!(d.src_key, "ship_aegis-vru");
        // Factionless entity + no flashvar faction -> bare assetId.
        let d = mgr.ship_display("ship_aegis", 0).expect("neutral variant");
        assert_eq!(d.src_key, "ship_aegis");
    }
}
