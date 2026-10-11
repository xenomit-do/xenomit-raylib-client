//! Lazy visual recipe resolution for map entities, collectables, portals, backgrounds.

use crate::assets::gamexml::BackgroundPattern;
use crate::assets::mapxml::MapBackground;
use crate::assets::tilemap;
use crate::assets::{
    AssetManager, Display2D, Display3D, MaterialDesc, SpriteId, TextureQuality, TextureSlot,
};
use crate::net::SharedWorld;
use xenomit_packet_client::model::{CollectableKind, Entity, EntityKind};

use super::input;
use super::Game;
use std::time::Instant;

/// `MiniMapBackground` bookkeeping (`map/model/minimap/MiniMapBackground.as`).
#[derive(Clone, Copy, Debug)]
pub struct MinimapBg {
    /// Map this entry was resolved for — a change re-resolves the chain.
    pub map_id: i32,
    /// 300 or 700 — the size class that resolved `file_id`.
    pub size: i32,
    /// `assets.request(file_id, "")` of the chosen `minimap-*` image.
    pub sprite: SpriteId,
    /// First frame landed (`handleFileLoadComplete` → 0.3 s alpha tween);
    /// `None` while the file is still downloading/decoding.
    pub loaded_since: Option<Instant>,
}

/// `MineDecorator2D` overlay data on a collectable visual: the looping
/// `minePulse` clip, drawn tinted to `color` at `alpha` (stock applies a
/// `ColorTransform` that *replaces* RGB — raylib has only a multiply
/// tint, equivalent for the white-authored pulse art).
#[derive(Clone, Copy, Debug)]
pub struct MinePulse {
    pub sprite: SpriteId,
    /// `minePulseColor@color` — 0xRRGGBB.
    pub color: u32,
    /// `minePulseColor@alpha` — `ColorTransform.alphaMultiplier`.
    pub alpha: f32,
    /// `minePulseColor@scale` — clip `scaleX`/`scaleY`.
    pub scale: f32,
}

/// `MiniMapBackground.update`'s file-id chain: `minimap-{minimap}` (the
/// maps-config `@minimap` art key, skipped when absent), then
/// `minimap-{minimap}-{size}`, `minimap-{mapId}-{size}`, `minimap-{mapId}`
/// — first id existing in the FileIndex wins; falls through to
/// `minimap-{mapId}` when none do (the request just resolves empty).
/// `width <= 400` picks the 300 px variant, else 700.
pub(crate) fn minimap_bg_key(
    map_id: i32,
    minimap_attr: &str,
    width: f32,
    exists: impl Fn(&str) -> bool,
) -> String {
    let size = if width <= 400.0 { 300 } else { 700 };
    let mut key = if minimap_attr.is_empty() {
        String::new()
    } else {
        format!("minimap-{minimap_attr}")
    };
    if !exists(&key) {
        key = format!("minimap-{minimap_attr}-{size}");
    }
    if !exists(&key) {
        key = format!("minimap-{map_id}-{size}");
    }
    if !exists(&key) {
        key = format!("minimap-{map_id}");
    }
    key
}

/// One authored `mapAssetView` sub-asset, ready to draw.
#[derive(Clone, Debug)]
pub struct PortalLayer {
    pub id: String,
    pub sprite: SpriteId,
    pub offset: (f32, f32),
    pub scale: (f32, f32),
    pub alpha: f32,
    pub visible: bool,
}

/// Cached display recipe + sprite request for one map object.
///
/// Built lazily by `Game::update` (which owns `&mut AssetManager`) and read
/// by the render layer, so recipes/requests are only re-resolved when the
/// lookup key (usually `typeId`) changes.
#[derive(Clone, Debug)]
pub struct Visual {
    /// The key this recipe was resolved for (entity `type_id`, collectable
    /// asset key, ...). Rebuilt when it changes.
    pub key: String,
    /// Authored `<display3D>` geometry/texture recipe when this asset has
    /// one. Kept beside the independent 2D recipe for mode switching.
    pub display_3d: Option<Display3D>,
    /// Sprite requested from `(srcKey,resKey)`; `None` while a recipe is
    /// known to be missing — renderer falls back to a placeholder.
    pub sprite: Option<SpriteId>,
    /// `center="true"` — shift the drawn bitmap by half its size.
    pub center: bool,
    /// `autoPlay="false"` — freeze on `still_frame`.
    pub auto_play: bool,
    /// `frame` attribute (1-based) used when `!auto_play`.
    pub still_frame: i32,
    /// `useBitmapClip` — frames form a rotation strip.
    pub bitmap_clip: bool,
    /// Recipe-local draw offset (in parent space — scaled by zoom only).
    pub offset: (f32, f32),
    /// `scale`/`scaleX`/`scaleY` recipe attrs — content scale applied to
    /// the clip inside the offset container (`UberAsset2D.
    /// applyTransformations`). `(1,1)` when the recipe has none; negative
    /// axes mirror the content.
    pub scale: (f32, f32),
    /// Click radius in screen px (ship pattern / uber `click_area` or
    /// [`input::DEFAULT_CLICK_RADIUS`]). Hit-testing divides by zoom.
    pub click_radius: f32,
    /// `<pivot id="click_area" x y>` — the hit area's center sits at
    /// server pos + this offset (recipe-space units, same space as
    /// `offset`). Stock anchors `UberAsset2D.getClickArea` there.
    pub click_pivot: (f32, f32),
    /// `miniMapIcon resKey` sprite (`minimap` SWF) — `None` when the
    /// uber node declares none.
    pub minimap_icon: Option<SpriteId>,
    /// `miniMapIcon color` — minimap dot color when no icon is declared.
    pub minimap_color: Option<u32>,
    /// `mapAssetView` layers (portals): base, highlight, active, jump,
    /// spawn, animated_base — each its own atlas clip.
    pub map_layers: Vec<PortalLayer>,
    /// `mapAssetView` `animated_base` overlay clip (portals): a
    /// prefix-matched atlas animation drawn over the base region.
    pub overlay: Option<SpriteId>,
    /// Overlay draw offset (recipe `x`/`y`) and `scaleX`.
    pub overlay_offset: (f32, f32),
    pub overlay_scale: f32,
    /// Additional `<displayObject>` layers of a multi-object recipe
    /// (`pod_heal`, `battleStation`, —), drawn over the main sprite in
    /// document order — each carries its own offset/scale.
    pub layers: Vec<Visual>,
    /// `<label x y>` — name-plate offset from the plate's default anchor
    /// (recipe px, zoom-scaled when drawn).
    pub label_offset: (f32, f32),
    /// `<hp>` present — the asset shows an hp bar without being locked.
    pub has_hp_bar: bool,
    /// `Pod2D` decorator clip (`podType != 0` ships) — drawn over hull
    /// and drones only while the ship carries `PodCarrierTrait`
    /// (factory `aegis`/`citadel` classes or modifier
    /// `pods::HEALING_POD_MODIFIER`); the gate lives in the renderer.
    pub pod: Option<Box<Visual>>,
    /// `MineDecorator2D` pulse overlay (collectable mines with
    /// `pulse_color_id > 0`).
    pub pulse: Option<MinePulse>,
}

impl Visual {
    /// One `Visual` per extra layer of `d` (sprites requested here).
    pub(super) fn build_layers(key: &str, d: &Display2D, assets: &mut AssetManager) -> Vec<Visual> {
        d.layers
            .iter()
            .enumerate()
            .map(|(i, l)| {
                let sprite =
                    (!l.src_key.is_empty()).then(|| assets.request(&l.src_key, &l.res_key));
                Visual::from_display(format!("{key}#layer{i}"), l, sprite)
            })
            .collect()
    }

    pub(super) fn from_display(key: String, d: &Display2D, sprite: Option<SpriteId>) -> Self {
        Self {
            key,
            display_3d: None,
            sprite,
            center: d.center,
            auto_play: d.auto_play,
            still_frame: d.frame,
            bitmap_clip: d.use_bitmap_clip,
            offset: (d.x, d.y),
            scale: d.scale,
            click_radius: input::DEFAULT_CLICK_RADIUS,
            click_pivot: d.click_pivot,
            minimap_icon: None,
            minimap_color: None,
            map_layers: Vec::new(),
            overlay: None,
            overlay_offset: (0.0, 0.0),
            overlay_scale: 1.0,
            layers: Vec::new(),
            label_offset: (0.0, 0.0),
            has_hp_bar: false,
            pod: None,
            pulse: None,
        }
    }

    pub(super) fn placeholder(key: String) -> Self {
        Self {
            key,
            display_3d: None,
            sprite: None,
            center: true,
            auto_play: false,
            still_frame: 0,
            bitmap_clip: false,
            offset: (0.0, 0.0),
            scale: (1.0, 1.0),
            click_radius: input::DEFAULT_CLICK_RADIUS,
            click_pivot: (0.0, 0.0),
            minimap_icon: None,
            minimap_color: None,
            map_layers: Vec::new(),
            overlay: None,
            overlay_offset: (0.0, 0.0),
            overlay_scale: 1.0,
            layers: Vec::new(),
            label_offset: (0.0, 0.0),
            has_hp_bar: false,
            pod: None,
            pulse: None,
        }
    }
}

/// Collectable asset id — `Ore.as` builds `ore_{typeValue}` directly, so
/// the game.xml `<ores>` resKey is never consulted: it is a trade-window
/// resource id offset by one (`type 1 -> "ore_2"`), not the uberlib asset.
/// `game.collectables` only ever holds those same ore keys, so it must not
/// shadow `beacon_N`/`mine_N` either. `game_xml.boxes` stays — it is empty
/// in the shipped file but remains the authored box override point.
fn collectable_asset_key(
    kind: CollectableKind,
    type_id: i32,
    type_str: &str,
    boxes: &std::collections::HashMap<i32, String>,
) -> String {
    if matches!(kind, CollectableKind::Box | CollectableKind::FireworkBox) {
        if let Some(key) = boxes.get(&type_id) {
            return key.clone();
        }
    }
    match kind {
        CollectableKind::Ore => format!("ore_{type_id}"),
        CollectableKind::Box if !type_str.is_empty() => format!("box_{type_str}"),
        CollectableKind::Box => format!("box_{type_id}"),
        CollectableKind::FireworkBox => "firework_box".into(),
        CollectableKind::Beacon => format!("beacon_{type_id}"),
        CollectableKind::Mine => format!("mine_{type_id}"),
    }
}

/// Queue a mesh's geometry plus every texture layer its material resolves
/// at `quality` — the `{base}_{slot}_{size}` manifest ids
/// `PrefabMaterialManager.assembleTextureResKey` builds. Until the
/// `resources_3d` index lands, only the diffuse id can be requested (other
/// layers' manifest membership isn't knowable yet); afterwards non-manifest
/// ids are skipped so `request_geometry` never marks them failed.
pub(crate) fn request_mesh_resources(
    assets: &mut AssetManager,
    geometry: &str,
    material: &MaterialDesc,
    quality: TextureQuality,
) {
    assets.request_geometry(geometry);
    let index_ready = assets.geometry_index_ready();
    for (slot, key) in crate::render::materials3d::layer_keys(geometry, material, quality) {
        if if index_ready {
            assets.geometry_file_exists(&key)
        } else {
            slot == TextureSlot::Diffuse
        } {
            assets.request_geometry(&key);
        }
    }
}

fn asset_uber_keys(
    type_value: i32,
    design_id: &str,
    faction: i32,
    stage: i32,
    map_id: i32,
) -> Vec<String> {
    // Faction suffixes: hangar/refinery/… use the numeric factionId,
    // headquarters keys use the company name (Headquarters.as).
    let fnum = faction.to_string();
    let fname = match faction {
        1 => "mmo",
        2 => "eic",
        3 => "vru",
        _ => "",
    };
    let mut keys: Vec<String> = Vec::new();
    match type_value {
        // QuestGiver / Hangar / Headquarters / Refinery / RepairStation /
        // HomeBaseLaserTurret → LIB_DEFAULT ids.
        34 => keys.push(format!("questgiver_{design_id}")),
        46 => keys.push(format!("headquarters_home_{fname}")),
        47 => keys.push(format!("headquarters_outpost_{fname}")),
        48 => keys.push(format!("hangar_home_{fnum}")),
        49 => keys.push(format!("hangar_outpost_{fnum}")),
        50 => keys.push(format!("refinery_home_{fnum}")),
        51 => keys.push(format!("refinery_outpost_{fnum}")),
        52 => keys.push(format!("refinery_invisible_{fnum}")),
        53 => keys.push(format!("repairstation_home_{fnum}")),
        54 => keys.push(format!("repairstation_outpost_{fnum}")),
        55..=58 => keys.push(format!("turret_{type_value}_{fnum}")),
        74 => keys.push(format!("expedition_home_{fnum}")),
        // RelayStation / HealingPod / Plutus generators / plague range /
        // nebula / payload node / PET beacons → fixed ids.
        31 => keys.push("relay".into()),
        33 => keys.push("pod_heal".into()),
        59 => keys.push("ranged_explosion".into()),
        62 => keys.push("plutus-generator".into()),
        63 => keys.push("plutus-generator-green".into()),
        64 => keys.push("plutus-generator-red".into()),
        68 => keys.push("payload-box-node".into()),
        71 | 72 => keys.push("pet_beacon".into()),
        73 => keys.push("asteroid".into()),
        // Sector-control / TDM / wargame enter+exit points and the
        // sector beacon — fixed ids, exits carry a faction/map suffix.
        40 => keys.push(format!("sectorControlExit_{fnum}")),
        41 => keys.push("sectorControlBeacon".into()),
        42 => keys.push("sectorControlEnter".into()),
        43 => keys.push("tdm_enter".into()),
        44 => keys.push(format!("tdm_exit_{fnum}")),
        66 => keys.push("wargame_enter".into()),
        67 => keys.push(format!("wargame_exit_{}", map_id.rem_euclid(2))),
        // BattleStation.as → LIB_BATTLESTATION; the bare-id search
        // resolves `asteroid` there (lib_default has none).
        36 => keys.push("battleStation".into()),
        35 => keys.push("asteroid".into()),
        // BattleStationModule.as → TRAIT_SETS[designId] +
        // min(3, expansionStage/5 + 1); wreck carries no suffix.
        37 => {
            let base = match design_id.parse::<i32>().unwrap_or(-1) {
                0 => "wreck",
                3 => "module_rep",
                4 => "module_laserlow",
                5 => "module_lasermid",
                6 => "module_laserhigh",
                7 => "module_rocketlow",
                8 => "module_rocketmid",
                9 => "module_hon",
                10 => "module_dama",
                11 => "module_xp",
                _ => "",
            };
            if base == "wreck" {
                keys.push("wreck".into());
            } else if !base.is_empty() {
                keys.push(format!("{base}{}", (stage / 5 + 1).min(3)));
            }
        }
        // BoosterStation / BuffCapsule / NpcBeacon / BurningTrail /
        // SprayAsset → designId-parameterized ids.
        38 => keys.push(format!("boosterstation_{design_id}")),
        45 => keys.push(format!("buffCapsule_{design_id}")),
        60 => keys.push(format!("npc-beacon-{design_id}")),
        61 => keys.push(format!("burning_trail_entity_{design_id}")),
        69 => keys.push(format!("spray_{design_id}")),
        // PlutusMine / MegaMine pass a fixed mine typeId (18/19).
        65 => keys.push("mine_18".into()),
        70 => keys.push("mine_19".into()),
        _ => {}
    }
    // Generic fallbacks (pre-dispatch behavior).
    keys.push(design_id.to_owned());
    keys.push(format!("portal_{design_id}"));
    keys.push(format!("station_{design_id}"));
    keys
}

impl Game {
    /// clip after all retries, or a recipe without a sprite part — still
    /// renders as the grey placeholder circle. Warn once per visual key.
    pub(super) fn report_dead_sprites(&mut self, world: &SharedWorld, assets: &AssetManager) {
        for e in world.world.entities.values() {
            let Some(v) = self.entity_visuals.get(&e.id) else {
                continue;
            };
            let asset_type = world.entity_asset_type.get(&e.id).copied().unwrap_or(0);
            // Same key the recipe-miss branch inserts — entities already
            // reported for a missing recipe are skipped here.
            let pkey = format!("{:?}:{}:t{}", e.kind, e.type_id, asset_type);
            if self.placeholders_logged.contains(&pkey) {
                continue;
            }
            let dead = match v.sprite {
                Some(id) => assets.get(id).is_some_and(|c| c.frames.is_empty()),
                None => true,
            };
            if dead && self.placeholders_logged.insert(pkey) {
                log::warn!(
                    "placeholder visual (dead sprite): id={} kind={:?} \
                     type_id={} asset_type={asset_type} name={} key={}",
                    e.id,
                    e.kind,
                    e.type_id,
                    e.name,
                    v.key,
                );
            }
        }
    }

    /// Session-scope UI sprite (`ui`/`icons`/`minimap` SWF symbols) —
    /// requested once and cached; NOT cleared on map change.
    pub(super) fn ui_sprite(
        &mut self,
        assets: &mut AssetManager,
        src: &str,
        res: &str,
    ) -> SpriteId {
        *self
            .ui_sprites
            .entry((src.to_owned(), res.to_owned()))
            .or_insert_with(|| assets.request(src, res))
    }

    /// Resolve + cache the display recipe/sprite for one map entity.
    /// `asset_type` is the `AssetCreateCommand.type.typeValue` kept in
    /// `SharedWorld::entity_asset_type` (0 when unknown); `faction` is the
    /// entity's own `factionId` (Ship.as `info.factionID`, 0 when absent);
    /// `entity` carries expansion/name for the key derivation +
    /// diagnostics.
    pub(super) fn ensure_entity_visual(
        &mut self,
        id: i32,
        type_id: &str,
        kind: EntityKind,
        asset_type: i32,
        faction: i32,
        entity: Option<&Entity>,
        pet_level: i32,
        assets: &mut AssetManager,
    ) {
        let key = match kind {
            // The uber key depends on typeValue — re-resolve if the row
            // was cached before AssetCreateCommand had been applied.
            EntityKind::Asset => format!("{kind:?}:{type_id}:t{asset_type}"),
            // Faction keys the `{assetId}_{faction}` step — re-resolve
            // when it arrives/changes (e.g. hero init order). Pets also
            // key on their level (`pet-basic-{level}` fallback).
            _ if pet_level > 0 => format!("{kind:?}:{type_id}:f{faction}:p{pet_level}"),
            _ => format!("{kind:?}:{type_id}:f{faction}"),
        };
        if let Some(v) = self.entity_visuals.get(&id) {
            if v.key == key {
                return;
            }
        }
        let recipe: Option<(Display2D, String)> = match kind {
            EntityKind::Ship | EntityKind::Other => assets
                .ship_display_lvl(type_id, faction, pet_level)
                .map(|mut d| {
                    // `Ship2D` ignores `display2D@scale` — the uber
                    // fallback path keeps it.
                    d.scale = (1.0, 1.0);
                    (d, type_id.to_owned())
                })
                .or_else(|| {
                    assets
                        .uber_display(type_id)
                        .map(|d| (d, type_id.to_owned()))
                }),
            EntityKind::Asset => {
                let stage = entity.map(|e| e.expansion_stage).unwrap_or(0);
                let keys = asset_uber_keys(asset_type, type_id, faction, stage, self.map_id);
                log::info!(
                    "asset {id} type={asset_type} design={type_id} \
                     faction={faction} stage={stage} keys={keys:?}"
                );
                keys.iter()
                    .find_map(|k| assets.uber_display(k).map(|d| (d, k.clone())))
            }
        };
        let mut visual = match recipe {
            Some((d, resolved)) => {
                // Copy the node's facets out first — `request` needs `&mut`.
                let node_data = assets.uber.node(&resolved).map(|n| {
                    (
                        n.click_radius,
                        n.minimap_icon.clone(),
                        n.minimap_color,
                        n.label_offset,
                        n.has_hp_bar,
                    )
                });
                let sprite = if d.src_key.is_empty() {
                    None
                } else {
                    Some(assets.request(&d.src_key, &d.res_key))
                };
                let layers = Visual::build_layers(&key, &d, assets);
                let mut v = Visual::from_display(key, &d, sprite);
                v.layers = layers;
                v.display_3d = if matches!(kind, EntityKind::Ship | EntityKind::Other) {
                    assets.ship_display3d(type_id, faction)
                } else {
                    assets.uber_display3d(&resolved)
                };
                if matches!(kind, EntityKind::Ship | EntityKind::Other) {
                    log::info!(
                        "ship {id} type={type_id} faction={faction} 3D recipe: {:?}",
                        v.display_3d.as_ref().map(|d| d
                            .meshes
                            .iter()
                            .map(|m| format!("{}/{}", m.geometry, m.texture))
                            .collect::<Vec<_>>())
                    );
                }
                if let Some((cr, mm_icon, mm_color, label, hp_bar)) = node_data {
                    v.label_offset = (label.0 as f32, label.1 as f32);
                    v.has_hp_bar = hp_bar;
                    // `<click_icon radius>`/`<click_area>` hit-test radius.
                    if cr > 0 {
                        v.click_radius = cr as f32;
                    }
                    // `<miniMapIcon srcKey="minimap" resKey>` — the
                    // marker on the HUD minimap.
                    if let Some(res) = mm_icon.filter(|r| !r.is_empty()) {
                        v.minimap_icon = Some(assets.request("minimap", &res));
                    }
                    v.minimap_color = mm_color;
                }
                v
            }
            None => {
                // Once per type — a grey circle on the map means this
                // fired; the exemplar id/name identify the recipe gap.
                let pkey = format!("{kind:?}:{type_id}:t{asset_type}");
                if self.placeholders_logged.insert(pkey) {
                    let name = entity.map(|e| e.name.as_str()).unwrap_or("");
                    log::warn!(
                        "placeholder visual (no recipe): id={id} kind={kind:?} \
                         type_id={type_id} asset_type={asset_type} name={name}"
                    );
                }
                Visual::placeholder(key)
            }
        };
        // Pattern-defined click radius (screen px) when known.
        if !matches!(kind, EntityKind::Asset) {
            if let Some(r) = assets
                .game_xml
                .ship_by_type(type_id)
                .map(|p| p.click_radius as f32)
                .filter(|r| *r > 0.0)
            {
                visual.click_radius = r;
            }
        }
        // `Pod2D` — `podType != 0` patterns get the faction's pod clip
        // (`graphics_items`); `Pod2D.loadPod` hardcodes the `"mc"` class
        // and ignores the pattern `resKey`. Drawn only while the ship
        // carries `PodCarrierTrait` (renderer-side modifier gate). The
        // low-quality `replacementShips`/`ship49_pod` branch needs
        // `Settings.qualityShip`, which has no client equivalent — the
        // pattern srcKey is always used.
        if matches!(kind, EntityKind::Ship | EntityKind::Other) {
            let pat = assets.game_xml.ship_by_type(type_id).or_else(|| {
                type_id
                    .parse::<i32>()
                    .ok()
                    .and_then(|id| assets.game_xml.ship_by_npc(id))
            });
            if let Some(pod) = pat
                .filter(|p| p.pod_type != 0)
                .and_then(|p| crate::assets::pods::pod_pattern(&p.pod_res, faction))
            {
                let src_key = pod.src_key.clone();
                let mut pv = Visual::placeholder(format!("{}:pod", visual.key));
                pv.center = false;
                pv.sprite = Some(assets.request(&src_key, "mc"));
                visual.pod = Some(Box::new(pv));
            }
        }
        self.entity_visuals.insert(id, visual);
    }

    /// Resolve + cache the display recipe for a collectable.
    /// `pulse_color_id` is the `AddMineCommand.maxSkill1` field —
    /// `Mine.pulseColorId` (`0` disables `MineDecorator2D`).
    pub(super) fn ensure_collectable_visual(
        &mut self,
        hash: &str,
        kind: CollectableKind,
        type_id: i32,
        type_str: &str,
        pulse_color_id: i32,
        assets: &mut AssetManager,
    ) {
        let key = format!("{kind:?}:{type_id}:{type_str}:p{pulse_color_id}");
        if let Some(v) = self.collectable_visuals.get(hash) {
            if v.key == key {
                return;
            }
        }
        let asset_key = collectable_asset_key(kind, type_id, type_str, &assets.game_xml.boxes);
        let recipe = assets
            .collectable_display(&asset_key)
            .or_else(|| assets.uber_display(&asset_key));
        let mut visual = match recipe {
            Some(d) => {
                let sprite = if d.src_key.is_empty() {
                    None
                } else {
                    Some(assets.request(&d.src_key, &d.res_key))
                };
                let layers = Visual::build_layers(&key, &d, assets);
                let mut v = Visual::from_display(key, &d, sprite);
                v.layers = layers;
                v.display_3d = assets.uber_display3d(&asset_key);
                v
            }
            None => Visual::placeholder(key),
        };
        // `MineDecorator2D`: pulseColorId > 0 loads `(pattern.resKey,"mc")`
        // as a tinted looping child of the mine's view.
        if kind == CollectableKind::Mine && pulse_color_id > 0 {
            // Copy the pattern out first — `request` needs `&mut`.
            let pattern = assets
                .game_xml
                .mine_pulses
                .get(&pulse_color_id)
                .map(|p| {
                    (
                        p.res_key.clone(),
                        p.res_key_3d.clone(),
                        p.color,
                        p.alpha,
                        p.scale,
                    )
                });
            if let Some((res_key, res_key_3d, color, alpha, scale)) = pattern {
                if !res_key.is_empty() {
                    visual.pulse = Some(MinePulse {
                        sprite: assets.request(&res_key, "mc"),
                        color,
                        alpha,
                        scale,
                    });
                }
                // `MineDecorator3D` — `Particle3D(resKey3D + ".zip")` as a
                // child of the mine's 3D view; only exists when the mine
                // has a `display3D` recipe to attach to.
                if !res_key_3d.is_empty() {
                    if let Some(d3) = &mut visual.display_3d {
                        d3.particles.push(crate::assets::uberlib::Particles3D {
                            resource: res_key_3d,
                            scale: (1.0, 1.0, 1.0),
                            ..Default::default()
                        });
                    }
                }
            }
        }
        self.collectable_visuals.insert(hash.to_owned(), visual);
    }

    /// Resolve + cache the display recipe for a portal — LIB_PORTALS
    /// `mapAssetView` recipes carry an atlas (`atlas_key` XML +
    /// `atlas_texture_key` PNG + `atlas_region` name), so the sprite is a
    /// single named `SubTexture` crop, not a whole file.
    pub(super) fn ensure_portal_visual(
        &mut self,
        gate: i32,
        type_id: &str,
        assets: &mut AssetManager,
    ) {
        let key = format!("portal:{type_id}");
        if let Some(v) = self.portal_visuals.get(&gate) {
            if v.key == key {
                return;
            }
        }
        let recipe = assets.uber_display(type_id);
        let mut visual = match recipe {
            Some(d) => {
                let mut v = Visual::from_display(key, &d, None);
                v.display_3d = assets.uber_display3d(type_id);
                if !d.map_layers.is_empty() {
                    // Whole `mapAssetView`: base ring, highlight pulse,
                    // active/jump (hidden until jumpOn), spawn burst and
                    // any `animated_base` movie clip.
                    for layer in &d.map_layers {
                        let frame_ms = if layer.fps > 0.0 {
                            (1000.0 / layer.fps).round().max(1.0) as u32
                        } else {
                            33
                        };
                        let sprite = assets.request_map_layer(
                            &layer.atlas_key,
                            &layer.texture_key,
                            &layer.texture,
                            frame_ms,
                        );
                        if layer.id == "base" || (layer.id.is_empty() && v.sprite.is_none()) {
                            v.sprite = Some(sprite);
                            v.offset = (layer.x, layer.y);
                            v.scale = layer.scale;
                        }
                        if layer.id == "animated_base" {
                            v.overlay = Some(sprite);
                            v.overlay_offset = (layer.x, layer.y);
                            v.overlay_scale = layer.scale.0;
                        }
                        v.map_layers.push(PortalLayer {
                            id: layer.id.clone(),
                            sprite,
                            offset: (layer.x, layer.y),
                            scale: layer.scale,
                            alpha: layer.alpha,
                            visible: layer.visible,
                        });
                    }
                } else {
                    let sprite = if !d.atlas_key.is_empty() && !d.atlas_texture_key.is_empty() {
                        Some(assets.request_region(
                            &d.atlas_key,
                            &d.atlas_texture_key,
                            &d.atlas_region,
                        ))
                    } else if !d.src_key.is_empty() {
                        Some(assets.request(&d.src_key, &d.res_key))
                    } else {
                        None
                    };
                    v.sprite = sprite;
                    if !d.anim_prefix.is_empty() {
                        let atlas = if d.anim_atlas_key.is_empty() {
                            d.atlas_key.as_str()
                        } else {
                            d.anim_atlas_key.as_str()
                        };
                        let texture = if d.anim_texture_key.is_empty() {
                            d.atlas_texture_key.as_str()
                        } else {
                            d.anim_texture_key.as_str()
                        };
                        if !atlas.is_empty() && !texture.is_empty() {
                            let frame_ms = if d.anim_fps > 0.0 {
                                (1000.0 / d.anim_fps).round().max(1.0) as u32
                            } else {
                                33
                            };
                            v.overlay = Some(assets.request_region_anim(
                                atlas,
                                texture,
                                &d.anim_prefix,
                                frame_ms,
                            ));
                            v.overlay_offset = d.anim_offset;
                            v.overlay_scale = if d.anim_scale > 0.0 {
                                d.anim_scale
                            } else {
                                1.0
                            };
                        }
                    }
                }
                v
            }
            None => Visual::placeholder(key),
        };
        let node_data = assets
            .uber
            .node(type_id)
            .map(|n| (n.click_radius, n.minimap_icon.clone(), n.minimap_color));
        if let Some((cr, mm_icon, mm_color)) = node_data {
            if cr > 0 {
                visual.click_radius = cr as f32;
            }
            if let Some(res) = mm_icon.filter(|r| !r.is_empty()) {
                visual.minimap_icon = Some(assets.request("minimap", &res));
            }
            visual.minimap_color = mm_color;
        }
        self.portal_visuals.insert(gate, visual);
    }

    /// Queue map background/planet sprites for the current map descriptor.
    ///
    /// Layer kind follows `BackgroundVO.isBackgroundTiled(gpu)` —
    /// `isTiled && resKeyStarling && atlasXml` gets the `StarlingTiledMap`
    /// path (`request_tilemap` + a precomputed cell grid); other resource
    /// backgrounds load `resKeyStarling` falling back to `resKey` as a
    /// whole-image layer (`BasicElement2D`); `content="custom"` patterns
    /// are solid fills drawn directly by the renderer.
    pub(super) fn ensure_map_visuals(&mut self, assets: &mut AssetManager) {
        // One minimap image: the game.xml `<minimaps>` key wins when the
        // map has one, else `MiniMapBackground.update`'s `minimap-*` file
        // chain (300 px under a 400 px draw width). Map or size-class
        // change re-resolves; the 0.3 s fade stamps on the first frame.
        let mm_width = crate::ui::panels::MINIMAP_MAP.0;
        let mm_size = if mm_width <= 400.0 { 300 } else { 700 };
        let stale = self
            .minimap_bg
            .is_none_or(|b| b.map_id != self.map_id || b.size != mm_size);
        if stale {
            let file_id = assets
                .game_xml
                .minimaps
                .get(&self.map_id)
                .filter(|key| !key.is_empty())
                .cloned()
                .unwrap_or_else(|| {
                    let attr = assets
                        .maps
                        .get(self.map_id)
                        .map(|m| m.minimap.clone())
                        .unwrap_or_default();
                    minimap_bg_key(self.map_id, &attr, mm_width, |id| {
                        assets.files.entry(id).is_some()
                    })
                });
            self.minimap_bg = Some(MinimapBg {
                map_id: self.map_id,
                size: mm_size,
                sprite: assets.request(&file_id, ""),
                loaded_since: None,
            });
        }
        if let Some(bg) = &mut self.minimap_bg {
            if bg.loaded_since.is_none()
                && assets.get(bg.sprite).is_some_and(|c| !c.frames.is_empty())
            {
                bg.loaded_since = Some(Instant::now());
            }
        }

        if self.preferences.render.backgrounds_3d {
            let quality = self.preferences.render.texture_quality;
            if let Some(scene) = assets.map_scene3d_for_map(self.map_id) {
                let meshes: Vec<_> = scene
                    .meshes
                    .iter()
                    .map(|mesh| (mesh.geometry.clone(), mesh.material.clone()))
                    .collect();
                let planes: Vec<_> = scene
                    .planes
                    .iter()
                    .map(|plane| plane.texture.clone())
                    .filter(|id| !id.is_empty())
                    .collect();
                for (geometry, material) in meshes {
                    request_mesh_resources(assets, &geometry, &material, quality);
                }
                // UberPlane3D resolves textures through the main resource
                // FileCollection, not World3D's resources_3d collection.
                // Reuse the normal sprite decoder/download pipeline here.
                for texture in planes {
                    let key = format!("plane3d:{texture}");
                    self.map_visuals
                        .entry(key)
                        .or_insert_with(|| assets.request(&texture, ""));
                }
            }
        }
        let Some(desc) = assets.maps.get(self.map_id) else {
            return;
        };
        // Collect everything `desc` is needed for up front — `request`
        // mutably borrows `assets` below.
        let bgs: Vec<_> = desc.backgrounds.to_vec();
        let planets: Vec<_> = desc.planets.to_vec();
        for (i, bg) in bgs.iter().enumerate() {
            let key = format!("bg:{}:{i}", self.map_id);
            let pat = assets.game_xml.backgrounds.get(&bg.type_id).cloned();
            // Texture file id for a `StarlingTiledMap` layer, resolved
            // against resources.xml before queueing: `resKeyStarling`,
            // then `resKey`, then the atlas's own `{stem}_texture`
            // (e.g. blacklight_map-2_stars uses the shared
            // `blacklight_map-1_stars_atlas`, whose texture file the
            // pattern's ids don't name). None present -> layer skipped
            // (stock's FileCollection load fails the same way).
            let tiled_tex = pat
                .as_ref()
                .filter(|p| p.tiled && !p.atlas_xml.is_empty())
                .and_then(|p| {
                    [
                        p.res_key_starling.as_str(),
                        p.res_key.as_str(),
                        // `{atlasStem}_texture` — the pairing the atlas XML
                        // was actually authored against.
                        &format!(
                            "{}_texture",
                            p.atlas_xml.strip_suffix("_atlas").unwrap_or(&p.atlas_xml)
                        ),
                    ]
                    .into_iter()
                    .find(|id| !id.is_empty() && assets.files.entry(id).is_some())
                    .map(str::to_owned)
                });
            if !self.map_visuals.contains_key(&key) {
                match &pat {
                    // Solid-fill layer — nothing to download.
                    Some(p) if p.is_custom => {}
                    Some(p) if tiled_tex.is_some() => {
                        let id = assets.request_tilemap(&p.atlas_xml, tiled_tex.as_ref().unwrap());
                        self.map_visuals.insert(key.clone(), id);
                    }
                    Some(p) if p.tiled => {
                        // Tiled pattern whose texture is absent entirely —
                        // skip (was a `no resources.xml entry` warn).
                        log::info!(
                            "map {} bg {i} (type {}): no starling texture — layer skipped",
                            self.map_id,
                            p.type_id
                        );
                    }
                    Some(p) => {
                        // `BasicElement2D` probes `{resKey}_01.._04`
                        // quadrant files (big layers split at 2048px);
                        // all four present -> quadrant mode, else the
                        // single image (`resKeyStarling` preferred —
                        // the `_atf` probe is skipped, we can't decode
                        // ATF).
                        let quad = (1..=4)
                            .all(|q| assets.files.entry(&format!("{}_0{q}", p.res_key)).is_some());
                        if quad {
                            for q in 1..=4u8 {
                                let id = assets.request(&format!("{}_0{q}", p.res_key), "");
                                self.map_visuals.insert(format!("{key}q{q}"), id);
                            }
                        } else {
                            let res = if p.res_key_starling.is_empty() {
                                &p.res_key
                            } else {
                                &p.res_key_starling
                            };
                            if !res.is_empty() {
                                let id = assets.request(res, "");
                                self.map_visuals.insert(key.clone(), id);
                            }
                        }
                    }
                    None => {
                        let res_key = format!("background{}", bg.type_id);
                        let id = assets.request(&res_key, "");
                        self.map_visuals.insert(key.clone(), id);
                    }
                }
            }
            if tiled_tex.is_some() && !self.tile_grids.contains_key(&key) {
                self.ensure_tile_grid(i, bg, pat.as_ref().unwrap(), assets, &key);
            }
        }
        for (i, planet) in planets.iter().enumerate() {
            let key = format!("pl:{}:{i}", self.map_id);
            if self.map_visuals.contains_key(&key) {
                continue;
            }
            let type_id = planet.type_id.as_str();
            // `planetPatterns[typeID].resKey` is the file id `Planet2D`
            // loads (jxr/png); uber display recipes only as a fallback.
            let planet_res = assets
                .game_xml
                .planets
                .get(type_id)
                .map(|p| p.res_key.clone())
                .filter(|r| !r.is_empty());
            let id = if let Some(res) = planet_res {
                assets.request(&res, "")
            } else if let Some(d) = assets.uber_display(type_id) {
                assets.request(&d.src_key, &d.res_key)
            } else if type_id.ends_with("_atf") || type_id.ends_with(".png") {
                assets.request(type_id, "")
            } else {
                // planets/meshes resolve to "<id>_atf" image files in
                // resources.xml; try both namings.
                let id = assets.request(&format!("{type_id}_atf"), "");
                let _ = assets.request(type_id, "");
                id
            };
            self.map_visuals.insert(key, id);
        }
    }

    /// Build the `StarlingTiledMap.prepareTileMap` cell->tile-id grid once
    /// the tile clip and the optional `maskID` bitmap have both resolved.
    /// Runs every frame until then (the grid is deterministic — the cache
    /// entry is filled once and reused).
    pub(super) fn ensure_tile_grid(
        &mut self,
        idx: usize,
        bg: &MapBackground,
        pat: &BackgroundPattern,
        assets: &mut AssetManager,
        key: &str,
    ) {
        // `backgroundMasks[maskID].resKey` is a single-frame image file;
        // sample it like `prepareTileMap` does. While it is in flight the
        // grid is not built (stock draws the map only after the mask
        // arrived — `maskBitmapData` null means "no mask", but an
        // outstanding request should still be waited out).
        let mask_key = format!("bgmask:{}:{idx}", self.map_id);
        if bg.mask_id >= 0 && !self.map_visuals.contains_key(&mask_key) {
            if let Some(res) = assets
                .game_xml
                .background_masks
                .get(&bg.mask_id)
                .filter(|r| !r.is_empty())
                .cloned()
            {
                let id = assets.request(&res, "");
                self.map_visuals.insert(mask_key.clone(), id);
            }
        }
        let mask_pending = self
            .map_visuals
            .get(&mask_key)
            .is_some_and(|&mid| !assets.sprites.resolved(mid));
        if mask_pending {
            return;
        }
        let Some(&sprite) = self.map_visuals.get(key) else {
            return;
        };
        let Some(clip) = assets.get(sprite) else {
            return;
        };
        if !clip.tiles || clip.frames.is_empty() {
            return;
        }
        let count = clip.frames.len();
        let mask_img = self
            .map_visuals
            .get(&mask_key)
            .and_then(|&mid| assets.get(mid))
            .and_then(|c| c.frames.first())
            .map(|f| &f.image);
        let seed = tilemap::grid_seed(self.map_id, idx);
        let grid = tilemap::build_grid(self.bounds, bg, pat, seed, mask_img, count);
        self.tile_grids.insert(key.to_string(), grid);
    }
}
#[cfg(test)]
mod tests {
    use super::{asset_uber_keys, collectable_asset_key, minimap_bg_key};
    use xenomit_packet_client::model::CollectableKind;

    /// `MiniMapBackground.update`'s chain, fake FileIndex predicate:
    /// `minimap-{attr}` → `minimap-{attr}-{size}` → `minimap-{id}-{size}`
    /// → `minimap-{id}`; width <= 400 picks 300 px art.
    #[test]
    fn minimap_bg_key_chain() {
        fn only<'a>(ids: &'a [&'a str]) -> impl Fn(&str) -> bool + 'a {
            move |k: &str| ids.contains(&k)
        }
        // First hit wins at each chain position.
        assert_eq!(minimap_bg_key(16, "101", 212.0, only(&["minimap-101"])), "minimap-101");
        assert_eq!(
            minimap_bg_key(16, "101", 212.0, only(&["minimap-101-300"])),
            "minimap-101-300"
        );
        assert_eq!(
            minimap_bg_key(16, "101", 212.0, only(&["minimap-16-300"])),
            "minimap-16-300"
        );
        assert_eq!(
            minimap_bg_key(16, "101", 500.0, only(&["minimap-16-700"])),
            "minimap-16-700"
        );
        assert_eq!(
            minimap_bg_key(16, "101", 212.0, only(&["minimap-16"])),
            "minimap-16"
        );
        // No @minimap attr → straight to the map-id keys.
        assert_eq!(
            minimap_bg_key(42, "", 212.0, only(&["minimap-42-300"])),
            "minimap-42-300"
        );
        // Nothing exists → falls through to the last candidate.
        assert_eq!(minimap_bg_key(7, "9", 212.0, only(&[])), "minimap-7");
    }

    #[test]
    fn collectable_keys_ore_uses_type_value_directly() {
        // `Ore.as`: `super("ore_" + typeValue)` — endurium is type 1, so
        // `ore_1`, even though game.xml `<ores>` maps `type 1 -> ore_2`
        // (a trade-window resource id, offset by one).
        let boxes = Default::default();
        assert_eq!(
            collectable_asset_key(CollectableKind::Ore, 0, "", &boxes),
            "ore_0"
        );
        assert_eq!(
            collectable_asset_key(CollectableKind::Ore, 1, "", &boxes),
            "ore_1"
        );
        assert_eq!(
            collectable_asset_key(CollectableKind::Ore, 2, "", &boxes),
            "ore_2"
        );
        assert_eq!(
            collectable_asset_key(CollectableKind::Ore, 8, "", &boxes),
            "ore_8"
        );
    }

    #[test]
    fn collectable_keys_other_kinds() {
        let mut boxes = std::collections::HashMap::new();
        boxes.insert(9, "box_override".to_string());
        assert_eq!(
            collectable_asset_key(CollectableKind::Box, 9, "BONUS_BOX", &boxes),
            "box_override"
        );
        assert_eq!(
            collectable_asset_key(CollectableKind::Box, 1, "BONUS_BOX", &boxes),
            "box_BONUS_BOX"
        );
        assert_eq!(
            collectable_asset_key(CollectableKind::Box, 1, "", &boxes),
            "box_1"
        );
        assert_eq!(
            collectable_asset_key(CollectableKind::Beacon, 4, "", &boxes),
            "beacon_4"
        );
        assert_eq!(
            collectable_asset_key(CollectableKind::Mine, 3, "", &boxes),
            "mine_3"
        );
        assert_eq!(
            collectable_asset_key(CollectableKind::FireworkBox, 0, "", &boxes),
            "firework_box"
        );
    }

    #[test]
    fn asset_keys_home_base() {
        // EIC home map: hangar + HQ + repair station + turrets.
        assert_eq!(asset_uber_keys(48, "2", 2, 0, 24)[0], "hangar_home_2");
        assert_eq!(
            asset_uber_keys(46, "1", 1, 0, 24)[0],
            "headquarters_home_mmo"
        );
        assert_eq!(
            asset_uber_keys(53, "1", 3, 0, 24)[0],
            "repairstation_home_3"
        );
        assert_eq!(asset_uber_keys(57, "1", 2, 0, 24)[0], "turret_57_2");
        assert_eq!(asset_uber_keys(50, "9", 1, 0, 24)[0], "refinery_home_1");
    }

    #[test]
    fn asset_keys_battlestation_modules() {
        // TRAIT_SETS[designId] + min(3, stage/5 + 1); wreck unsuffixed.
        assert_eq!(asset_uber_keys(37, "3", 0, 0, 0)[0], "module_rep1");
        assert_eq!(asset_uber_keys(37, "3", 0, 14, 0)[0], "module_rep3");
        assert_eq!(asset_uber_keys(37, "6", 0, 5, 0)[0], "module_laserhigh2");
        assert_eq!(asset_uber_keys(37, "0", 0, 9, 0)[0], "wreck");
        assert_eq!(asset_uber_keys(36, "1", 0, 0, 0)[0], "battleStation");
    }

    #[test]
    fn asset_keys_generic_fallback_last() {
        // Unknown typeValues still try the designId fallbacks.
        let keys = asset_uber_keys(30, "7", 0, 0, 0);
        assert_eq!(keys[0], "7");
        assert_eq!(keys[1], "portal_7");
        // DesignId-parameterized ids.
        assert_eq!(asset_uber_keys(60, "3", 0, 0, 0)[0], "npc-beacon-3");
        assert_eq!(asset_uber_keys(45, "33", 0, 0, 0)[0], "buffCapsule_33");
        assert_eq!(asset_uber_keys(65, "0", 0, 0, 0)[0], "mine_18");
        assert_eq!(asset_uber_keys(70, "0", 0, 0, 0)[0], "mine_19");
        assert_eq!(asset_uber_keys(71, "0", 0, 0, 0)[0], "pet_beacon");
        assert_eq!(asset_uber_keys(44, "0", 3, 0, 0)[0], "tdm_exit_3");
        assert_eq!(asset_uber_keys(67, "0", 0, 0, 25)[0], "wargame_exit_1");
    }
}
