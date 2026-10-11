//! `game.xml` — the `PatternManager` static database embedded in main.swf.
//!
//! Relevant sections (all under `<game><patterns>` unless noted):
//!
//! ```xml
//! <ships><ship type="ship_goliath" shipId="10" name="Goliath" icon="ship_10"
//!              expansionClassID="ship_goliath" enginePositionClassID="ship_goliath"
//!              clickRadius="70" rotatable="true" loopPlay="false" ...>
//!     <designs><design type="ship_g-bastion" shipId="59" .../></designs>
//! </ship></ships>
//! <engines><engine type="0" resKey="engine0" soundID="4" loop="true"/></engines>
//! <engineSmokes><engineSmoke id="0" resKey="engineSmoke0" poolSize="2500"/></engineSmokes>
//! <rocketSmokes><rocketSmoke id="0" resKey="rocketSmoke0" particleInterval="15" .../></rocketSmokes>
//! <enginePositions><enginePosition class="ship_goliath"><positionsList .../></enginePosition></enginePositions>
//! <expansions><expansion class="ship_goliath"><stage id="1">...</stage></expansion></expansions>
//! <backgrounds><background type="1" resKey="background1" offsetX="26" isTiled="true"/></backgrounds>
//! <backgroundMasks><backgroundMask type="1" resKey="mask91-1"/></backgroundMasks>
//! <minimaps><minimap type="16" resKey="minimap16"/></minimaps>
//! <planets><planet id="1" resKey="planet1" radius="250"/></planets>
//! <drones><designs><design id="0" assetId="standard" color="0xFFFFFF"/></designs></drones>
//! <ores><ore type="0" resKey="ore_1" languageKey="ore_prometium"/></ores>
//! <preloadedResources><resource resKey="ui"/></preloadedResources>
//! <hudIcons><hudIcon id="drawFire" srcKey="shockwaves" resKey="drawFireIcon"/></hudIcons>
//! ```
//!
//! Semantics follow `PatternManager.as`: `parseShipPatterns` /
//! `setShipPatternAttributes` / `createShipDesignPatterns` (designs clone the
//! base ship pattern, overriding only present attributes),
//! `parseEnginePatterns`, `parseEngineSmokePatterns`,
//! `parseRocketSmokePatterns`, `parseEnginePositionPatterns`,
//! `parseExpansionPatterns`, `parseBackgroundPatterns`,
//! `parseBackgroundMaskPatterns`, `parseMinimapPatterns`? (minimap resKeys),
//! `parsePlanetPatterns`, `parseDroneDesigns`?, `parseOrePatterns`,
//! `parsePreloaderPatterns`, `parseIconPattern`.

use super::formations::DroneFormationPattern;
use anyhow::{Context, Result};
use roxmltree::Node;
use std::collections::HashMap;

/// Ship visual/identity data from `<ship>` (or `<design>` inside `<designs>`).
///
/// `PatternManager.parseShipPatterns` iterates `patterns.ships.ship`; each
/// `<designs>` child spawns clones of the base pattern with attribute
/// overrides (`createShipDesignPatterns`). `asset_id` defaults to `type_id`
/// (`param2.assetId = param2.typeId`) and is the key into `lib_ships` for
/// `Ship.getUberDescriptor`.
#[derive(Clone, Debug, Default)]
pub struct ShipPattern {
    /// `type` attribute — the pattern id (e.g. `ship_goliath`, `alien_streuner`,
    /// or numeric `11` for old NPC ships).
    pub type_id: String,
    /// Uber-lib asset id — `shipPattern.assetId` (== `type_id` in AS3;
    /// an explicit `assetId` attr overrides if present).
    pub asset_id: String,
    /// `shipId` — player ship design id (0 when absent).
    pub ship_id: i32,
    /// `npcId` — NPC design id (0 when absent; old-style NPC ships carry it
    /// directly on `<ship>`, wild-PET designs via `<design npcId=..>`).
    pub npc_id: i32,
    /// `name` (display name; `""` when absent).
    pub name: String,
    /// `icon` — ship icon id (e.g. `ship_10`); resKey in the `icons` file.
    pub icon_id: String,
    /// `laserClassID` — laser class for laser visual lookup (0 default).
    pub laser_id: i32,
    /// `rocketClassID` — rocket class id.
    pub rocket_id: i32,
    /// `expansionClassID` — STRING key into `expansions` (e.g. `ship_goliath`,
    /// `pet_phoenix`), NOT an int.
    pub expansion_id: String,
    /// `explodeTypeID` — explosion pattern id.
    pub explosion_id: i32,
    /// `engineTypeID` — engine sound/fx class id (`engines` section).
    pub engine_id: i32,
    /// `engineSmokeID` — smoke pattern id (`engineSmokes` section).
    pub smoke_id: i32,
    /// `enginePositionClassID` — STRING key into `engine_positions`
    /// (e.g. `ship_goliath`; `-1` = no engine positions per the file's own
    /// doc comment).
    pub engine_position_id: String,
    /// `clickRadius` — hit-test radius in px (ShipPattern default: 45).
    pub click_radius: i32,
    /// `clickOffsetX`/`clickOffsetY`.
    pub click_offset: (i32, i32),
    /// `rotatable` — ShipPattern default: true.
    pub rotatable: bool,
    /// `loopPlay` — animated clip loops (else single-frame rotation strip
    /// semantics).
    pub loop_anim: bool,
    /// `minimapIcon` — resKey for the minimap marker (e.g. `mapIcon_npc_115`).
    pub minimap_icon_id: String,
    /// `defaultCharacter` — `"player"` / `"npc"` / `"pet"` (tri-state string).
    pub default_character: String,
    /// `class` — `standart`/`pet`/`premium`... (informational).
    pub class_id: String,
    /// `moveRadius` — selection ring radius (0 when absent).
    pub move_radius: i32,
    /// `labelYOffset` — name-plate vertical offset.
    pub label_y_offset: i32,
    /// `podType` — `InGameCatalog.POD_TYPE_*` (`0`/`NONE` = no pod).
    pub pod_type: i32,
    /// `<resources><podOverlay>` — faction-keyed pod art
    /// (`podResPatterns`; a design's own list replaces the base's).
    pub pod_res: Vec<super::pods::PodPattern>,
    /// `engine3DEffect` — `ShipPattern.engine3DRes`: particle group file
    /// (`engine_hyperion.zip`) replacing the default `thruster.zip`.
    pub engine_3d: String,
    /// `shockwaveID` — `ShipManager` fires `ShockwaveEffect(id)` beside
    /// the ship's explosion (-1 = none).
    pub shockwave_id: i32,
}

/// `<engines><engine>` — `AudibleResourcePattern(type, resKey)`.
#[derive(Clone, Debug, Default)]
pub struct EnginePattern {
    /// `type` attr (int) — `ShipPattern.engine_id` lookup key.
    pub type_id: i32,
    /// `resKey` — engine gfx file id.
    pub res_key: String,
    /// `soundID` — `<sounds>` entry for the loop (-1/none when absent).
    pub sound_id: i32,
    /// `loop="true"` — engine loop animation.
    pub loop_anim: bool,
}

/// `<engineSmokes><engineSmoke>` — `EngineSmokePattern(id, resKey)`.
#[derive(Clone, Debug, Default)]
pub struct SmokePattern {
    /// `id` attr — `ShipPattern.smoke_id` lookup key.
    pub type_id: i32,
    /// `resKey` — smoke gfx file id ("" disables smoke).
    pub res_key: String,
    /// `poolSize` — particle pool size.
    pub pool_size: i32,
    /// `rotation="true"`.
    pub rotation: bool,
    /// `alpha` attr (default 1).
    pub alpha: f32,
}

/// `<rocketSmokes><rocketSmoke>` — `RocketSmokePattern(id, resKey, interval, duration, fx3D)`.
#[derive(Clone, Debug, Default)]
pub struct RocketSmokePattern {
    /// `id` — `ShipPattern.rocket_id`-adjacent lookup key.
    pub type_id: i32,
    /// `resKey` — smoke gfx file id.
    pub res_key: String,
    /// `particleInterval` (ms between particles).
    pub particle_interval: i32,
    /// `particleDuration` (seconds).
    pub particle_duration: f32,
    /// `poolSize`.
    pub pool_size: i32,
    /// `fx3D` — `Rocket3D`'s trail group (`rocket_trail_1.zip`; empty
    /// falls back to 2D smoke puffs).
    pub fx3d: String,
}

/// `<enginePositions><enginePosition class="...">` — per-rotation engine
/// mount offsets. `positionsList name="center" data="x1,y1,x2,y2,..."`
/// gives 32 comma-separated x,y pairs (indexed by rotation frame); each
/// `<positionsList>` is ONE engine mount — a multi-engine ship has several
/// lists (`EnginePositionPattern.enginePositions: Vector<Vector<Point>>`).
#[derive(Clone, Debug, Default)]
pub struct EnginePositionPattern {
    /// `class` — string key matched by `ShipPattern.engine_position_id`.
    pub class_id: String,
    /// Parsed `positionsList` entries in document order: each inner Vec is
    /// one engine's 32 (x,y) offsets by rotation frame.
    pub positions: Vec<Vec<(f32, f32)>>,
    /// Parsed `lowPositionsList` entries (low-quality variants), parallel
    /// to `positions` when present.
    pub low_positions: Vec<Vec<(f32, f32)>>,
}

/// `<expansions><expansion class="..."><stage id>...` — hangar/drone/PET
/// formation data (`ExpansionPattern`); `expansionClasses[class][stageId]`
/// with `[0]` aliased to `[1]` (AS3 sets `classes[class][0] = [1]`).
#[derive(Clone, Debug, Default)]
pub struct ExpansionPattern {
    /// `stage.@id`.
    pub stage_id: i32,
    /// `stage.@resKey` — visual file id for the stage (often absent → stage
    /// is positions-only).
    pub res_key: String,
    /// `positionsList name=data` dictionary — formation slot names to
    /// per-rotation (x,y) offsets.
    pub positions: HashMap<String, Vec<(f32, f32)>>,
    /// `<salvo laser="left,right"/>` — laser mount groups per salvo.
    pub salvos: Vec<Vec<String>>,
}

/// All stages of one `<expansion class="...">`.
#[derive(Clone, Debug, Default)]
pub struct ExpansionClass {
    /// `stage id` -> stage. Stage 0 mirrors stage 1 (AS3
    /// `classes[class][0] = classes[class][1]`).
    pub stages: HashMap<i32, ExpansionPattern>,
    /// `positionsList` shared across the expansion's stages (AS3 parses them
    /// at `<expansion>` level: `_loc5_[name] = parseCommaCoordinatesList`).
    pub positions: HashMap<String, Vec<(f32, f32)>>,
}

/// `<backgrounds><background>` — `BackgroundPattern` for `content="resource"`.
///
/// Mirrors `PatternManager.parseBackgroundPatterns`
/// (PatternManager.as:1279-1388): `BackgroundPattern(type, resKey,
/// resKeyStarling||null, isTiled, atlasXml||null, isReloadable, tileWidth,
/// tileHeight, order, width, height, showInStarlingMode(!="false"),
/// useOutOfMapBorder, poi3dVO)` then `offsetX`/`offsetY`. `null` strings are
/// stored as `""`; `order` keeps "" for absent (AS3 leaves `_order` null
/// when the attr is empty).
#[derive(Clone, Debug, Default)]
pub struct BackgroundPattern {
    /// `type` attr — `MapBackground.type_id` lookup key.
    pub type_id: i32,
    /// `resKey` — resources.xml file id; `game/mod.rs` does
    /// `assets.request(&res_key, "")`, so this MUST be a file id, never an
    /// uber symbol name.
    pub res_key: String,
    /// `resKeyStarling` — texture file id for the Starling (GPU) paths:
    /// `StarlingTiledMap` loads this for tiled layers, and it is preferred
    /// over `res_key` for whole-image layers.
    pub res_key_starling: String,
    /// `atlasXml` — resources.xml file id of the Starling `TextureAtlas`
    /// document for tiled layers ("" when the pattern is not atlas-backed).
    pub atlas_xml: String,
    /// `offsetX` — tile-grid x offset (`BackgroundPattern.offsetX`).
    pub offset_x: i32,
    /// `offsetY`.
    pub offset_y: i32,
    /// `isTiled="true"` — the layer is a tile map. GPU path additionally
    /// requires `resKeyStarling` and `atlasXml`
    /// (`BackgroundVO.isBackgroundTiled`).
    pub tiled: bool,
    /// `isReloadable` — stock re-loads the texture on GPU context loss
    /// (informational here — raylib does not lose the context).
    pub is_reloadable: bool,
    /// `tileWidth` — tile grid cell size in px (`pattern.tileWidth`).
    pub tile_width: i32,
    /// `tileHeight`.
    pub tile_height: i32,
    /// `order` — `""` (null) | `"ASC"` | `"DESC"`: sequential tile ids
    /// instead of random (`prepareTileMap`).
    pub order: String,
    /// `width` — explicit tile-map width in px (0 = derive from map size).
    pub width: i32,
    /// `height`.
    pub height: i32,
    /// `useOutOfMapBorder` — drawn on the out-of-map border instead
    /// (informational; border rendering is not implemented).
    pub use_out_of_map_border: bool,
    /// `tiles="a,b,c"` — explicit split-image tile resKeys (the AS3 2D path
    /// instead probes `{resKey}_01.._04`; this attr documents the same
    /// split). Kept for completeness.
    pub tiles: Vec<String>,
    /// `content="custom"` backgrounds are solid colours — `color` carries
    /// the RGB value (0 when a resource background).
    pub color: u32,
    /// True when `content="custom"` (solid fill, no resource).
    pub is_custom: bool,
}

/// `<planets><planet>` — `PlanetPattern(id, resKey, radius)`; the
/// maps-config `planet typeID` resolves through this table.
#[derive(Clone, Debug, Default)]
pub struct PlanetPattern {
    /// `id` (string — `planetPatterns` is indexed by `String(@id)`).
    pub id: String,
    /// `resKey` — resources.xml file id loaded by `Planet2D`.
    pub res_key: String,
    /// `radius` — hit/occluder radius in world units.
    pub radius: i32,
}

/// `<drones><designs><design id assetId color>` — drone visual design.
#[derive(Clone, Debug, Default)]
pub struct DroneDesign {
    /// `id` — drone design index (sent by the server per drone).
    pub id: i32,
    /// `assetId` — `lib_drones` asset id (e.g. `havok`, `iris`).
    pub asset_id: String,
    /// `color` — `0xRRGGBB` tint.
    pub color: u32,
}

/// `<hudIcons><hudIcon>` — `FullResourcePattern(-1, srcKey, resKey)`.
#[derive(Clone, Debug)]
pub struct IconPattern {
    /// `id` — lookup key (`drawFire`, `ubaChampion`, ...).
    pub id: String,
    /// `srcKey` — resources.xml file id.
    pub src_key: String,
    /// `resKey` — symbol inside `srcKey`.
    pub res_key: String,
}

#[derive(Default)]
pub struct GameXml {
    /// `<music>`/`<sounds>` pattern tables (`PatternManager.musicPatterns`
    /// / `soundPatterns`).
    pub audio: super::audioxml::AudioPatterns,
    /// `type` -> ship pattern (base ships AND designs; design entries
    /// override nothing in this map — keyed by their own `type`).
    pub ships_by_type: HashMap<String, ShipPattern>,
    /// `shipId` -> `type` (later wins — designs without their own
    /// `shipId` inherit the base's, matching AS3 overwrite semantics).
    pub ships_by_ship_id: HashMap<i32, String>,
    /// `npcId` -> `type`.
    pub ships_by_npc_id: HashMap<i32, String>,
    /// `expansionClassID` (string) -> stages.
    pub expansions: HashMap<String, ExpansionClass>,
    /// `enginePositionClassID` (string) -> mount data.
    pub engine_positions: HashMap<String, EnginePositionPattern>,
    /// `engineTypeID` (int) -> engine pattern.
    pub engines: HashMap<i32, EnginePattern>,
    /// `engineSmokeID` (int) -> smoke pattern.
    pub engine_smokes: HashMap<i32, SmokePattern>,
    /// `laserClassID` -> `type` -> salvo group (`laserClasses[class][type]`;
    /// `getLaserPattern(class,type)` falls back to `[0]`/`[class][0]`).
    pub lasers: HashMap<i32, HashMap<i32, LaserSalvoPattern>>,
    /// `laserDefinitions` keyed by `salvoID` — the actual laser gfx
    /// patterns shared by class/type salvos.
    pub laser_definitions: HashMap<i32, LaserDefinition>,
    /// `rocketClasses[class][id]` — `rocketClassID` -> rocket id ->
    /// pattern (resKey + soundID).
    pub rockets: HashMap<i32, HashMap<i32, RocketPattern>>,
    /// `shieldClasses` keyed by `type` attr (`"default"`, `"12"`, ...).
    /// NOTE: AS3 `parseShieldPatterns` actually keys by `resKey` (likely a
    /// decompiler-faithful quirk); `type` is the useful key — deviation
    /// documented.
    pub shields: HashMap<String, ShieldPattern>,
    /// `rocketSmokeID` (int) -> rocket smoke pattern.
    pub rocket_smokes: HashMap<i32, RocketSmokePattern>,
    /// Generic `<effects>` entries keyed by effect id.
    pub effects: HashMap<i32, EffectPattern>,
    /// `<bossShield>` — level -> clip symbol inside `boss_shield`.
    pub boss_shields: HashMap<i32, String>,
    /// `<repairRobots>` — `loot_id` -> robot pattern.
    pub repair_robots: HashMap<String, RepairRobotPattern>,
    /// `<repairRobots><sparks>` — `repairSkillLevel` -> sparks pattern.
    pub repair_sparks: HashMap<i32, RepairRobotPattern>,
    /// `pyroClasses[class][id]` — `PatternManager.pyroClasses`:
    /// class 0 = ship explosions, 2 = laser damage, 3 = rocket damage,
    /// 6 = "soundword" hit-word clips.
    pub pyro: HashMap<i32, HashMap<i32, PyroPattern>>,
    /// background `type` -> pattern (`content="resource"` and `"custom"`).
    pub backgrounds: HashMap<i32, BackgroundPattern>,
    /// backgroundMask `type` -> resKey (`maskID` in maps-config resolves
    /// through this).
    pub background_masks: HashMap<i32, String>,
    /// minimap `type` -> resKey.
    pub minimaps: HashMap<i32, String>,
    /// planet `id` (string) -> pattern.
    pub planets: HashMap<String, PlanetPattern>,
    /// drone `design id` -> design.
    pub drone_designs: HashMap<i32, DroneDesign>,
    /// drone formation `id` -> position tables
    /// (`DroneFormation.standardFormations`).
    pub drone_formations: HashMap<i32, DroneFormationPattern>,
    /// collectable `type` -> resKey (generic collectables section; absent
    /// in the shipped file — today it only ever holds the `<ores>` rows,
    /// which are *resource* keys, not uberlib asset ids — `Ore.as` builds
    /// `ore_{typeValue}` directly).
    pub collectables: HashMap<i32, String>,
    /// ore `type` -> pattern (`OrePattern.resKey` — trade/loot icon +
    /// refinery bitmap id, offset by one vs the `ore_N` collectable
    /// asset; `refiner` is the production recipe).
    pub ores: HashMap<i32, OrePattern>,
    /// box `type` -> uber asset id (no `<boxes>` section exists in the
    /// shipped file — `Box.as` builds `box_{type_str}` directly; kept empty).
    pub boxes: HashMap<i32, String>,
    /// `preloadedResources/resource@resKey` — high-priority file ids.
    pub preload: Vec<String>,
    /// `hudIcons/hudIcon@id` -> pattern.
    pub icons: HashMap<String, IconPattern>,
    /// `portalAssets/portalAsset@id` -> resKey (faction-scrap portal assets;
    /// `patterns.portalAssets.portalAsset`, mis-indented in the file).
    pub portal_assets: HashMap<i32, String>,
    /// `<refinementWindow>` — the lab window content definition
    /// (`createRefinementWindow` in `RefinementManager.as`).
    pub refinement: RefinementWindow,
    /// `minePulseColor id` -> pattern (`PatternManager.minePulsePatterns`).
    pub mine_pulses: HashMap<i32, super::minepulse::MinePulsePattern>,
    /// `<shockwaves><shockwave id>` -> ring pattern (`ShockwaveEffect` —
    /// fired beside explosions via `PyroPattern.shockwave_id`).
    pub shockwaves: HashMap<i32, ShockwavePattern>,
    /// `<fireworks><type id=class><firework id=size><colorN>name</colorN>`
    /// — class -> size -> colour -> particle stem
    /// (`PatternManager.fireworkPatterns[type][size][colour]`).
    pub fireworks: HashMap<String, HashMap<String, HashMap<i32, String>>>,
    /// `<fireworkDescriptors><fireworkDescriptor firework=typeId
    /// order="1,2,3">` — box typeId -> ordered firework size ids.
    pub firework_descs: HashMap<i32, Vec<String>>,
}

/// `<ores><ore type resKey languageKey refiner>` — `OrePattern` in AS3.
/// The `refiner` attribute is a flat `"{oreType},{count},..."` list of
/// recipe inputs (`OrePattern.refiner` — produced ores only).
#[derive(Clone, Debug, Default)]
pub struct OrePattern {
    /// `resKey` — bitmap id inside the `refinement`/`tradeWindow` SWF
    /// (e.g. `ore_1`, `ore_11`, `osmium`).
    pub res_key: String,
    /// `languageKey` — BPLocale name key (e.g. `ore_prometium`).
    pub language_key: String,
    /// `(input ore type, required count)` recipe pairs.
    pub refiner: Vec<(i32, i32)>,
}

/// `<refinementWindow>` — layout data for the lab/refinery window.
#[derive(Clone, Debug, Default)]
pub struct RefinementWindow {
    /// `<refinement><staticOres>` — non-refinable ingredient ores shown
    /// on the top row of the Refining tab.
    pub refine_static: Vec<RefineryOre>,
    /// `<refinement><ores>` — producible/display ores; `button` adds a
    /// "Refine" action under the slot.
    pub refine_ores: Vec<RefineryOre>,
    /// `<update><staticOres>` — display-only ores on the Upgrade tab.
    pub update_static: Vec<RefineryOre>,
    /// `<update><ores>` — draggable source ores with a corner
    /// `iconResKey` effect icon.
    pub update_ores: Vec<RefineryOre>,
    /// `<update><items>` — lab target items (LASER/ROCKET/DRIVING/SHIELD).
    pub update_items: Vec<RefineryItem>,
}

/// A `<refinementWindow>//<ore>` row — display slot or draggable source.
#[derive(Clone, Debug)]
pub struct RefineryOre {
    /// `type` — `OreTypeModule.typeValue`.
    pub ore_type: i32,
    /// comma-separated tooltip key list (BPLocale).
    pub tooltip_key: String,
    /// `<update><ores>` only — small corner icon (`icon_11`…`osmium_icon`).
    pub icon_res_key: String,
    /// `<refinement><ores>` only — shows the "Refine" button.
    pub button: bool,
}

/// A `<refinementWindow><update><item>` lab target (LASER/ROCKET/…).
#[derive(Clone, Debug)]
pub struct RefineryItem {
    /// XML `type` (1..4 — display order only; the wire `itemValue` is
    /// `getItemTypeUint(identifier)`).
    pub type_id: i32,
    /// `LASER`/`ROCKET`/`DRIVING`/`SHIELD`.
    pub identifier: String,
    /// `item_N` bitmap in the `refinement` SWF.
    pub res_key: String,
    /// label `languageKey` (e.g. `lab_laser`).
    pub language_key: String,
    /// `ammountKey` — `lab_salvos`/`lab_pieces`/`lab_minutes`
    /// (`%COUNT%` template for the applied-amount label).
    pub amount_key: String,
}

/// `<laserDefinitions><laser salvoID=...>` — `LaserPattern` in AS3.
#[derive(Clone, Debug, Default)]
pub struct LaserDefinition {
    /// `salvoID`.
    pub salvo_id: i32,
    /// `resKey` — laser gfx file id.
    pub res_key: String,
    /// `skillResKey` — skilled variant gfx.
    pub skill_res_key: String,
    /// `soundID`.
    pub sound_id: i32,
    /// `fireRate` (ms between shots).
    pub fire_rate: i32,
    /// `absorber="true"` — SAB absorb beam.
    pub absorber: bool,
    /// `speed` attr.
    pub speed: f32,
    /// `playLoop`/`playLoopRotated`.
    pub play_loop: bool,
    /// `muzzleFlashResKey`.
    pub muzzle_flash: String,
    /// `lightColor` hex string → RGB.
    pub light_color: u32,
    /// `laserType` (`""` beam, `particle`, ...).
    pub laser_type: String,
    /// `laserLength` (0 default).
    pub laser_length: i32,
    /// `mcWidth` (0 when absent; stock defaults -1, both mean "absent"
    /// to consumers) — overrides the clip width for the endpoint
    /// pull-back in `LaserSalvo2D.shoot`.
    pub mc_width: i32,
    /// `scale` attr — the 3D bolt container's uniform scale
    /// (`LaserPattern.scale`, default 1).
    pub scale: f32,
    /// `ignoreLaserPoints="true"` — the 3D salvo fires one shot from the
    /// ship center instead of resolving `laserpoint_*` mounts
    /// (`LaserSalvo3D.startAttack`).
    pub ignore_laser_points: bool,
}

/// `<lasers><laser class type><salvo salvoID/>` — `LaserSalvoPattern`:
/// the resolved list of laser definitions for one `class`+`type`.
#[derive(Clone, Debug, Default)]
pub struct LaserSalvoPattern {
    pub type_id: i32,
    /// Salvo definitions in `<salvo>` order (resolved via
    /// `laser_definitions[salvoID]` at parse time).
    pub salvos: Vec<LaserDefinition>,
    /// Raw salvo ids (kept for debugging).
    pub salvo_ids: Vec<i32>,
}

/// `<rockets><rocket class id resKey soundID>` — `RocketPattern`.
#[derive(Clone, Debug, Default)]
pub struct RocketPattern {
    /// `id` — server-sent rocket type id.
    pub id: i32,
    /// `resKey` — rocket gfx file id.
    pub res_key: String,
    /// `soundID` (-1/none when absent).
    pub sound_id: i32,
}

/// `<pyroEffects><pyroEffect class id resKey>` — `ExplosionPattern` in
/// AS3: a once-through clip played at a fixed position (explosions and
/// weapon-impact flashes).
#[derive(Clone, Debug, Default)]
pub struct PyroPattern {
    /// `id` attr — `ShipPattern.explosion_id` / damage-type id.
    pub id: i32,
    /// `resKey` — gfx file id (`explosion0`, `laserDamage1`, ...).
    pub res_key: String,
    /// `scale` attr (default 1).
    pub scale: f32,
    /// `rotatable` — stock defaults TRUE when the attr is absent; the
    /// whole clip gets a random rotation, not a rotation strip.
    pub rotatable: bool,
    /// `useBitmapClip` — frames are independent bitmaps (play once).
    pub use_bitmap_clip: bool,
    /// `has3D` — `PyroEffect3D` only builds when the pattern allows it
    /// (`@has3D != "false"`, default true).
    pub has_3d: bool,
    /// `shockwaveId` — `ShipManager` fires `ShockwaveEffect(shockwaveId)`
    /// beside the explosion (-1 = none).
    pub shockwave_id: i32,
    /// `soundID` (-1/none when absent).
    pub sound_id: i32,
}

/// `<shockwaves><shockwave id resKey>` — `ShockwavePattern`: N copies of
/// the `shockwaves` bitmap flying outward (`ShockwaveEffect2D`); in 3D a
/// `{resKey}.zip` particle group (`ShockwaveEffect3D`).
#[derive(Clone, Debug)]
pub struct ShockwavePattern {
    /// `id` — `PyroPattern.shockwave_id` lookup key.
    pub id: i32,
    /// `resKey` — `shockwaves` symbol / `{resKey}.zip` group stem.
    pub res_key: String,
    /// `soundID` (-1/none when absent).
    pub sound_id: i32,
    /// `radius` — how far the ring flies (300 default).
    pub radius: f32,
    /// `duration` — ring expansion seconds (2 default).
    pub duration: f32,
    /// `beginScale`/`endScale` — bitmap scale over the flight (1 -> 0.25).
    pub begin_scale: f32,
    pub end_scale: f32,
    /// `maxShockwaves` — ring density (40 default).
    pub max_shockwaves: i32,
    /// `shakeScreen`.
    pub shake_screen: bool,
}

/// `<effects><effect id effect resKey>` — generic game effect resources.
#[derive(Clone, Debug, Default)]
pub struct EffectPattern {
    pub id: i32,
    pub effect: String,
    pub res_key: String,
    /// `radius` — world-unit radius the 2D view reads (shield0/1/2, …).
    pub radius: f32,
    /// `scale` — authored uniform scale (`PatternManager.effectPatterns`).
    pub scale: f32,
    /// `scale2D` — 2D-view scale multiplier (drive-area clips, …).
    pub scale2d: f32,
    /// `alpha` — initial clip alpha (`ShieldEffect2D` `initAlpha`).
    pub init_alpha: f32,
    /// `loop="true"` — the clip loops instead of playing once.
    pub is_loop: bool,
    /// `fps` — authored playback rate (0 => view default).
    pub fps: f32,
    /// `soundID` (-1/none when absent).
    pub sound_id: i32,
}

/// `<repairRobots><robot loot_id robotsNum resKeyPrefix offsetX offsetY>`.
#[derive(Clone, Debug, Default)]
pub struct RepairRobotPattern {
    /// `robotsNum` — how many robot clips orbit.
    pub robots: i32,
    /// `resKeyPrefix` — resources.xml file id (`repair_robot1`, …).
    pub res_key: String,
    /// `offsetX`/`offsetY` — clip origin inside the robot view.
    pub offset: (f32, f32),
}

/// `<shields><shield type resKey resKeySkilledLaser>` — `ShieldPattern`.
#[derive(Clone, Debug, Default)]
pub struct ShieldPattern {
    /// `type` attr (`"default"`, numeric ids).
    pub type_id: String,
    /// `resKey` — shield gfx file id (`impact_shield.zip`).
    pub res_key: String,
    /// `resKeySkilledLaser` — skilled-hit variant (falls back to `resKey`).
    pub res_key_skilled: String,
    /// `scale` — `ShieldImpactEffect.fixedScale` (1 default).
    pub scale: f32,
}

// --- attribute helpers (AS3 `String(node.@x)` -> absent == "") -----------

fn attr<'a>(node: &Node<'a, 'a>, name: &str) -> &'a str {
    node.attribute(name).unwrap_or_default()
}

fn attr_i32(node: &Node, name: &str) -> i32 {
    node.attribute(name)
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or_default()
}

fn attr_f32(node: &Node, name: &str, default: f32) -> f32 {
    node.attribute(name)
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(default)
}

/// `ParserUtility.parseBooleanFromString` — only `"true"` counts.
fn attr_bool(node: &Node, name: &str) -> bool {
    node.attribute(name).is_some_and(|v| v == "true")
}

/// `0xRRGGBB`, bare-hex (`ff0000` — `parseInt("0x"+v)` in AS3) or decimal
/// color attr.
fn attr_color(node: &Node, name: &str) -> u32 {
    let v = attr(node, name).trim();
    if let Some(hex) = v.strip_prefix("0x").or_else(|| v.strip_prefix("0X")) {
        u32::from_str_radix(hex, 16).unwrap_or_default()
    } else if let Ok(n) = u32::from_str_radix(v, 16) {
        n // bare hex (lightColor, color attrs)
    } else {
        v.parse().unwrap_or_default()
    }
}

/// `ParserUtility.parseCommaCoordinatesList` — flat `x1,y1,x2,y2,...`.
fn parse_xy_list(data: &str) -> Vec<(f32, f32)> {
    let nums: Vec<f32> = data
        .split(',')
        .filter_map(|s| s.trim().parse().ok())
        .collect();
    nums.as_chunks::<2>()
        .0
        .iter()
        .map(|c| (c[0], c[1]))
        .collect()
}

/// `setShipPatternAttributes`: assign only PRESENT attributes — designs
/// inherit everything else from the cloned base pattern.
fn apply_ship_attrs(node: &Node, pat: &mut ShipPattern) {
    let has = |n: &str| node.attribute(n).is_some();
    if has("shipId") {
        pat.ship_id = attr_i32(node, "shipId");
    }
    if has("npcId") {
        pat.npc_id = attr_i32(node, "npcId");
    }
    if has("name") {
        pat.name = attr(node, "name").to_string();
    }
    if has("icon") {
        pat.icon_id = attr(node, "icon").to_string();
    }
    if has("type") {
        pat.type_id = attr(node, "type").to_string();
        pat.asset_id = pat.type_id.clone(); // `assetId = typeId`
    }
    if has("assetId") {
        pat.asset_id = attr(node, "assetId").to_string();
    }
    if has("class") {
        pat.class_id = attr(node, "class").to_string();
    }
    if has("loopPlay") {
        pat.loop_anim = attr_bool(node, "loopPlay");
    }
    if has("laserClassID") {
        pat.laser_id = attr_i32(node, "laserClassID");
    }
    if has("rocketClassID") {
        pat.rocket_id = attr_i32(node, "rocketClassID");
    }
    if has("expansionClassID") {
        pat.expansion_id = attr(node, "expansionClassID").to_string();
    }
    if has("explodeTypeID") {
        pat.explosion_id = attr_i32(node, "explodeTypeID");
    }
    if has("engineTypeID") {
        pat.engine_id = attr_i32(node, "engineTypeID");
    }
    if has("engineSmokeID") {
        pat.smoke_id = attr_i32(node, "engineSmokeID");
    }
    if has("engine3DEffect") {
        pat.engine_3d = attr(node, "engine3DEffect").to_string();
    }
    if has("shockwaveID") {
        pat.shockwave_id = attr_i32(node, "shockwaveID");
    }
    if has("enginePositionClassID") {
        pat.engine_position_id = attr(node, "enginePositionClassID").to_string();
    }
    if has("clickRadius") {
        pat.click_radius = attr_i32(node, "clickRadius");
    }
    if has("clickOffsetX") {
        pat.click_offset.0 = attr_i32(node, "clickOffsetX");
    }
    if has("clickOffsetY") {
        pat.click_offset.1 = attr_i32(node, "clickOffsetY");
    }
    if has("rotatable") {
        pat.rotatable = attr_bool(node, "rotatable");
    }
    if has("minimapIcon") {
        pat.minimap_icon_id = attr(node, "minimapIcon").to_string();
    }
    if has("defaultCharacter") {
        pat.default_character = attr(node, "defaultCharacter").to_string();
    }
    if has("moveRadius") {
        pat.move_radius = attr_i32(node, "moveRadius");
    }
    if has("labelYOffset") {
        pat.label_y_offset = attr_i32(node, "labelYOffset");
    }
    if has("podType") {
        pat.pod_type = attr_i32(node, "podType");
    }
    // `setShipPatternAttributes` lines 647-659: the node's own
    // <podOverlay> list replaces the (possibly inherited) vector when
    // podType != POD_TYPE_NONE.
    if pat.pod_type != 0 {
        if let Some(overlays) = super::pods::parse_pod_overlays(node) {
            pat.pod_res = overlays;
        }
    }
}

/// Direct element children of `node` named `tag`.
fn children<'a>(node: &Node<'a, 'a>, tag: &'a str) -> impl Iterator<Item = Node<'a, 'a>> {
    node.children()
        .filter(move |d| d.is_element() && d.has_tag_name(tag))
}

impl GameXml {
    /// Parse the embedded `game.xml` document.
    ///
    /// The shipped file contains a malformed comment (a nested `<!--` inside
    /// a comment around the `<effects>` section) that strict parsers reject
    /// — comment regions are stripped up-front (they're dead data anyway).
    pub fn parse(xml: &str) -> Result<Self> {
        let cleaned = strip_comments(xml);
        let doc = roxmltree::Document::parse(&cleaned).context("invalid game.xml")?;
        let mut game = GameXml::default();
        let root = doc.root_element();
        let patterns = if root.has_tag_name("patterns") {
            root
        } else {
            match children(&root, "patterns").next() {
                Some(p) => p,
                None => anyhow::bail!("game.xml has no <patterns> root"),
            }
        };

        // -- <music>/<sounds> (PatternManager.parseMusicPatterns/parseSoundPatterns)
        game.audio = super::audioxml::AudioPatterns::parse(&patterns);

        // -- <minePulseColors> (PatternManager.parseMinePulsePatterns) --
        super::minepulse::parse_mine_pulses(&patterns, &mut game.mine_pulses);

        // -- <ships><ship> + <ship><designs><design> -------------------------
        // `parseShipPatterns` iterates `patterns.ships.ship`; designs clone
        // the base pattern (`createShipDesignPatterns`). `<designs>` may
        // also contain `<ship>` elements — same treatment (children()).
        for section in children(&patterns, "ships") {
            for ship in children(&section, "ship") {
                // Base pattern: ShipPattern ctor takes (type, shipId, npcId,
                // name) — `int()`/`String()` of absent attrs give 0/"".
                let mut base = ShipPattern {
                    type_id: attr(&ship, "type").to_string(),
                    ship_id: attr_i32(&ship, "shipId"),
                    npc_id: attr_i32(&ship, "npcId"),
                    name: attr(&ship, "name").to_string(),
                    // ShipPattern.as defaults
                    click_radius: 45,
                    rotatable: true,
                    shockwave_id: -1,
                    ..Default::default()
                };
                base.asset_id = base.type_id.clone();
                apply_ship_attrs(&ship, &mut base);
                insert_ship(&mut game, base.clone());
                for designs in children(&ship, "designs") {
                    for design in designs.children().filter(|n| n.is_element()) {
                        let mut pat = base.clone();
                        apply_ship_attrs(&design, &mut pat);
                        insert_ship(&mut game, pat);
                    }
                }
            }
        }

        // -- <engines><engine type resKey soundID loop> ---------------------
        for section in children(&patterns, "engines") {
            for eng in children(&section, "engine") {
                let pat = EnginePattern {
                    type_id: attr_i32(&eng, "type"),
                    res_key: attr(&eng, "resKey").to_string(),
                    sound_id: attr_i32(&eng, "soundID"),
                    loop_anim: attr_bool(&eng, "loop"),
                };
                game.engines.insert(pat.type_id, pat);
            }
        }

        // -- <engineSmokes><engineSmoke id resKey poolSize rotation alpha> --
        for section in children(&patterns, "engineSmokes") {
            for smoke in children(&section, "engineSmoke") {
                let pat = SmokePattern {
                    type_id: attr_i32(&smoke, "id"),
                    res_key: attr(&smoke, "resKey").to_string(),
                    pool_size: attr_i32(&smoke, "poolSize"),
                    rotation: attr_bool(&smoke, "rotation"),
                    alpha: attr_f32(&smoke, "alpha", 1.0),
                };
                game.engine_smokes.insert(pat.type_id, pat);
            }
        }

        // -- <effects><effect id effect resKey> -----------------------------
        for section in children(&patterns, "effects") {
            for effect in children(&section, "effect") {
                let pat = EffectPattern {
                    id: attr_i32(&effect, "id"),
                    effect: attr(&effect, "effect").to_string(),
                    res_key: attr(&effect, "resKey").to_string(),
                    radius: attr_f32(&effect, "radius", 0.0),
                    scale: attr_f32(&effect, "scale", 0.0),
                    scale2d: attr_f32(&effect, "scale2D", 0.0),
                    init_alpha: attr_f32(&effect, "alpha", 1.0),
                    is_loop: attr(&effect, "loop") == "true",
                    fps: attr_f32(&effect, "fps", 0.0),
                    sound_id: attr_i32(&effect, "soundID"),
                };
                game.effects.insert(pat.id, pat);
            }
        }

        // -- <bossShield><shield level resKey> ------------------------------
        // BossShieldEffect clip symbol inside `boss_shield` per level.
        for section in children(&patterns, "bossShield") {
            for shield in children(&section, "shield") {
                game.boss_shields.insert(
                    attr_i32(&shield, "level"),
                    attr(&shield, "resKey").to_string(),
                );
            }
        }

        // -- <repairRobots><robot loot_id robotsNum resKeyPrefix …> ---------
        for section in children(&patterns, "repairRobots") {
            for robot in children(&section, "robot") {
                let pat = RepairRobotPattern {
                    robots: attr_i32(&robot, "robotsNum").max(1),
                    res_key: attr(&robot, "resKeyPrefix").to_string(),
                    offset: (
                        attr_f32(&robot, "offsetX", 0.0),
                        attr_f32(&robot, "offsetY", 0.0),
                    ),
                };
                game.repair_robots
                    .insert(attr(&robot, "loot_id").to_string(), pat);
            }
            for sparks in children(&section, "sparks") {
                let pat = RepairRobotPattern {
                    res_key: attr(&sparks, "resKeyPrefix").to_string(),
                    offset: (
                        attr_f32(&sparks, "offsetX", 0.0),
                        attr_f32(&sparks, "offsetY", 0.0),
                    ),
                    ..RepairRobotPattern::default()
                };
                game.repair_sparks
                    .insert(attr_i32(&sparks, "repairSkillLevel"), pat);
            }
        }

        // -- <laserDefinitions><laser salvoID resKey ...> -------------------
        for section in children(&patterns, "laserDefinitions") {
            for laser in children(&section, "laser") {
                let def = LaserDefinition {
                    salvo_id: attr_i32(&laser, "salvoID"),
                    res_key: attr(&laser, "resKey").to_string(),
                    skill_res_key: attr(&laser, "skillResKey").to_string(),
                    sound_id: attr_i32(&laser, "soundID"),
                    fire_rate: attr_i32(&laser, "fireRate"),
                    absorber: attr_bool(&laser, "absorber"),
                    speed: attr_f32(&laser, "speed", 0.0),
                    play_loop: attr_bool(&laser, "playLoop")
                        || attr_bool(&laser, "playLoopRotated"),
                    muzzle_flash: attr(&laser, "muzzleFlashResKey").to_string(),
                    light_color: attr_color(&laser, "lightColor"),
                    laser_type: attr(&laser, "laserType").to_string(),
                    laser_length: attr_i32(&laser, "laserLength"),
                    mc_width: attr_i32(&laser, "mcWidth"),
                    scale: attr_f32(&laser, "scale", 1.0),
                    ignore_laser_points: attr_bool(&laser, "ignoreLaserPoints"),
                };
                game.laser_definitions.insert(def.salvo_id, def);
            }
        }

        // -- <lasers><laser class type><salvo salvoID/> ----------------------
        for section in children(&patterns, "lasers") {
            for laser in children(&section, "laser") {
                let class_id = attr_i32(&laser, "class");
                let type_id = attr_i32(&laser, "type");
                let salvo_ids: Vec<i32> = children(&laser, "salvo")
                    .map(|s| attr_i32(&s, "salvoID"))
                    .collect();
                let pat = LaserSalvoPattern {
                    type_id,
                    salvos: salvo_ids
                        .iter()
                        .filter_map(|id| game.laser_definitions.get(id).cloned())
                        .collect(),
                    salvo_ids,
                };
                game.lasers
                    .entry(class_id)
                    .or_default()
                    .insert(type_id, pat);
            }
        }

        // -- <rockets><rocket class id resKey soundID> -----------------------
        for section in children(&patterns, "rockets") {
            for rocket in children(&section, "rocket") {
                let class_id = attr_i32(&rocket, "class");
                let pat = RocketPattern {
                    id: attr_i32(&rocket, "id"),
                    res_key: attr(&rocket, "resKey").to_string(),
                    sound_id: attr_i32(&rocket, "soundID"),
                };
                game.rockets
                    .entry(class_id)
                    .or_default()
                    .insert(pat.id, pat);
            }
        }

        // -- <shields><shield type resKey resKeySkilledLaser> ----------------
        for section in children(&patterns, "shields") {
            for shield in children(&section, "shield") {
                let pat = ShieldPattern {
                    type_id: attr(&shield, "type").to_string(),
                    res_key: attr(&shield, "resKey").to_string(),
                    res_key_skilled: attr(&shield, "resKeySkilledLaser").to_string(),
                    scale: shield
                        .attribute("scale")
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(1.0),
                };
                game.shields.insert(pat.type_id.clone(), pat);
            }
        }

        // -- <rocketSmokes><rocketSmoke id resKey particleInterval ...> -----
        for section in children(&patterns, "rocketSmokes") {
            for smoke in children(&section, "rocketSmoke") {
                let pat = RocketSmokePattern {
                    type_id: attr_i32(&smoke, "id"),
                    res_key: attr(&smoke, "resKey").to_string(),
                    particle_interval: attr_i32(&smoke, "particleInterval"),
                    particle_duration: attr_f32(&smoke, "particleDuration", 0.0),
                    pool_size: attr_i32(&smoke, "poolSize"),
                    fx3d: attr(&smoke, "fx3D").to_string(),
                };
                game.rocket_smokes.insert(pat.type_id, pat);
            }
        }

        // -- <pyroEffects><pyroEffect class id resKey ...> ------------------
        for section in children(&patterns, "pyroEffects") {
            for pyro in children(&section, "pyroEffect") {
                let class_id = attr_i32(&pyro, "class");
                let pat = PyroPattern {
                    id: attr_i32(&pyro, "id"),
                    res_key: attr(&pyro, "resKey").to_string(),
                    scale: attr_f32(&pyro, "scale", 1.0),
                    // AS3: `rotatable` absent -> true (see
                    // `parsePyroEffectPatterns` else-branch).
                    rotatable: pyro.attribute("rotatable").is_none_or(|v| v == "true"),
                    use_bitmap_clip: attr_bool(&pyro, "useBitmapClip"),
                    // `has3D` absent -> true (`parsePyroEffectPatterns`
                    // else-branch, same as `rotatable`).
                    has_3d: pyro.attribute("has3D").is_none_or(|v| v == "true"),
                    shockwave_id: pyro
                        .attribute("shockwaveId")
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(-1),
                    sound_id: attr_i32(&pyro, "soundID"),
                };
                game.pyro.entry(class_id).or_default().insert(pat.id, pat);
            }
        }

        // -- <shockwaves><shockwave id resKey ...> --------------------------
        for section in children(&patterns, "shockwaves") {
            for sw in children(&section, "shockwave") {
                let pat = ShockwavePattern {
                    id: attr_i32(&sw, "id"),
                    res_key: attr(&sw, "resKey").to_string(),
                    sound_id: sw
                        .attribute("soundID")
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(-1),
                    radius: sw
                        .attribute("radius")
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(300.0),
                    duration: sw
                        .attribute("duration")
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(2.0),
                    begin_scale: sw
                        .attribute("beginScale")
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(1.0),
                    end_scale: sw
                        .attribute("endScale")
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(0.25),
                    max_shockwaves: sw
                        .attribute("maxShockwaves")
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(40),
                    shake_screen: attr_bool(&sw, "shakeScreen"),
                };
                game.shockwaves.insert(pat.id, pat);
            }
        }

        // -- <fireworks><type id><firework id><colorN>name</colorN> -------
        for section in children(&patterns, "fireworks") {
            for ty in children(&section, "type") {
                let class = attr(&ty, "id").to_string();
                for fw in children(&ty, "firework") {
                    let colours = fw
                        .children()
                        .filter(|n| {
                            n.is_element() && n.tag_name().name().starts_with("color")
                        })
                        .map(|n| {
                            (
                                n.tag_name().name()[5..].parse().unwrap_or(0),
                                n.text().unwrap_or("").trim().to_string(),
                            )
                        })
                        .collect();
                    game.fireworks
                        .entry(class.clone())
                        .or_default()
                        .insert(attr(&fw, "id").to_string(), colours);
                }
            }
        }
        // -- <fireworkDescriptors><fireworkDescriptor firework order> ----
        for section in children(&patterns, "fireworkDescriptors") {
            for fd in children(&section, "fireworkDescriptor") {
                game.firework_descs.insert(
                    attr_i32(&fd, "firework"),
                    attr(&fd, "order")
                        .split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect(),
                );
            }
        }

        // -- <enginePositions><enginePosition class> -----------------------
        for section in children(&patterns, "enginePositions") {
            for ep in children(&section, "enginePosition") {
                let pat = EnginePositionPattern {
                    class_id: attr(&ep, "class").to_string(),
                    positions: children(&ep, "positionsList")
                        .map(|pl| parse_xy_list(attr(&pl, "data")))
                        .collect(),
                    low_positions: children(&ep, "lowPositionsList")
                        .map(|pl| parse_xy_list(attr(&pl, "data")))
                        .collect(),
                };
                game.engine_positions.insert(pat.class_id.clone(), pat);
            }
        }

        // -- <expansions><expansion class> ----------------------------------
        for section in children(&patterns, "expansions") {
            for exp in children(&section, "expansion") {
                let class_id = attr(&exp, "class").to_string();
                // positionsLists are parsed at <expansion> level
                // (`_loc5_[name] = parseCommaCoordinatesList`).
                let positions: HashMap<String, Vec<(f32, f32)>> = children(&exp, "positionsList")
                    .map(|pl| {
                        (
                            attr(&pl, "name").to_string(),
                            parse_xy_list(attr(&pl, "data")),
                        )
                    })
                    .collect();
                let mut class = ExpansionClass {
                    positions,
                    ..Default::default()
                };
                for stage in children(&exp, "stage") {
                    let pat = ExpansionPattern {
                        stage_id: attr_i32(&stage, "id"),
                        res_key: attr(&stage, "resKey").to_string(),
                        positions: class.positions.clone(),
                        salvos: children(&stage, "salvo")
                            .map(|s| {
                                attr(&s, "laser")
                                    .split(',')
                                    .map(|n| n.trim().to_string())
                                    .collect()
                            })
                            .collect(),
                    };
                    class.stages.insert(pat.stage_id, pat);
                }
                // AS3: `classes[class][0] = classes[class][1]`.
                if let Some(first) = class.stages.get(&1).cloned() {
                    class.stages.entry(0).or_insert(first);
                }
                game.expansions.insert(class_id, class);
            }
        }

        // -- <backgrounds><background type content resKey offsetX isTiled> --
        for section in children(&patterns, "backgrounds") {
            for bg in children(&section, "background") {
                let content = attr(&bg, "content");
                let pat = BackgroundPattern {
                    type_id: attr_i32(&bg, "type"),
                    res_key: attr(&bg, "resKey").to_string(),
                    // `@resKeyStarling || null` / `@atlasXml || null` — ""
                    // stands in for null.
                    res_key_starling: attr(&bg, "resKeyStarling").to_string(),
                    atlas_xml: attr(&bg, "atlasXml").to_string(),
                    offset_x: attr_i32(&bg, "offsetX"),
                    offset_y: attr_i32(&bg, "offsetY"),
                    tiled: attr_bool(&bg, "isTiled") || bg.attribute("tiles").is_some(),
                    is_reloadable: attr_bool(&bg, "isReloadable"),
                    tile_width: attr_i32(&bg, "tileWidth"),
                    tile_height: attr_i32(&bg, "tileHeight"),
                    order: attr(&bg, "order").to_string(),
                    width: attr_i32(&bg, "width"),
                    height: attr_i32(&bg, "height"),
                    use_out_of_map_border: attr_bool(&bg, "useOutOfMapBorder"),
                    tiles: attr(&bg, "tiles")
                        .split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect(),
                    color: attr_color(&bg, "color"),
                    is_custom: content == "custom",
                };
                game.backgrounds.insert(pat.type_id, pat);
            }
        }

        // -- <backgroundMasks><backgroundMask type resKey> ------------------
        for section in children(&patterns, "backgroundMasks") {
            for mask in children(&section, "backgroundMask") {
                game.background_masks
                    .insert(attr_i32(&mask, "type"), attr(&mask, "resKey").to_string());
            }
        }

        // -- <minimaps><minimap type resKey> --------------------------------
        for section in children(&patterns, "minimaps") {
            for mm in children(&section, "minimap") {
                game.minimaps
                    .insert(attr_i32(&mm, "type"), attr(&mm, "resKey").to_string());
            }
        }

        // -- <planets><planet id resKey radius> ------------------------------
        for section in children(&patterns, "planets") {
            for pl in children(&section, "planet") {
                let pat = PlanetPattern {
                    id: attr(&pl, "id").to_string(),
                    res_key: attr(&pl, "resKey").to_string(),
                    radius: attr_i32(&pl, "radius"),
                };
                game.planets.insert(pat.id.clone(), pat);
            }
        }

        // -- <drones><designs><design id assetId color> ----------------------
        for section in children(&patterns, "drones") {
            for designs in children(&section, "designs") {
                for d in children(&designs, "design") {
                    let pat = DroneDesign {
                        id: attr_i32(&d, "id"),
                        asset_id: attr(&d, "assetId").to_string(),
                        color: attr_color(&d, "color"),
                    };
                    game.drone_designs.insert(pat.id, pat);
                }
            }
            // -- <drones><formations><formation> ----------------------------
            for formations in children(&section, "formations") {
                for f in children(&formations, "formation") {
                    let pat = DroneFormationPattern::parse(&f);
                    game.drone_formations.insert(pat.id, pat);
                }
            }
        }

        // -- <ores><ore type resKey languageKey refiner> ---------------------
        for section in children(&patterns, "ores") {
            for ore in children(&section, "ore") {
                let type_id = attr_i32(&ore, "type");
                // `refiner="4,10,5,10,3,1"` — flat type,count pairs.
                let refiner: Vec<(i32, i32)> = attr(&ore, "refiner")
                    .split(',')
                    .filter_map(|s| s.trim().parse::<i32>().ok())
                    .collect::<Vec<_>>()
                    .chunks_exact(2)
                    .map(|c| (c[0], c[1]))
                    .collect();
                let pat = OrePattern {
                    res_key: attr(&ore, "resKey").to_string(),
                    language_key: attr(&ore, "languageKey").to_string(),
                    refiner,
                };
                game.collectables.insert(type_id, pat.res_key.clone());
                game.ores.insert(type_id, pat);
            }
        }

        // -- <collectables><*> (no such section in the shipped file — kept
        //    for forward-compat; ores already feed `collectables` above).
        for section in children(&patterns, "collectables") {
            for item in section.descendants().filter(|n| n.is_element()) {
                let id = attr_i32(&item, "typeID").max(attr_i32(&item, "type"));
                let res = {
                    let s = attr(&item, "srcKey");
                    if s.is_empty() {
                        attr(&item, "resKey").to_string()
                    } else {
                        s.to_string()
                    }
                };
                if !res.is_empty() {
                    game.collectables.insert(id, res);
                }
            }
        }

        // -- <preloadedResources><resource resKey> ---------------------------
        for section in children(&patterns, "preloadedResources") {
            for res in children(&section, "resource") {
                let id = attr(&res, "resKey");
                if !id.is_empty() {
                    game.preload.push(id.to_string());
                }
            }
        }

        // -- <portalAssets><portalAsset id resKey> -----------------------------
        for section in children(&patterns, "portalAssets") {
            for pa in children(&section, "portalAsset") {
                game.portal_assets
                    .insert(attr_i32(&pa, "id"), attr(&pa, "resKey").to_string());
            }
        }

        // -- <hudIcons><hudIcon id srcKey resKey> -----------------------------
        for section in children(&patterns, "hudIcons") {
            for icon in children(&section, "hudIcon") {
                let id = attr(&icon, "id");
                if id.is_empty() {
                    continue;
                }
                game.icons.insert(
                    id.to_string(),
                    IconPattern {
                        id: id.to_string(),
                        src_key: attr(&icon, "srcKey").to_string(),
                        res_key: attr(&icon, "resKey").to_string(),
                    },
                );
            }
        }

        // -- <refinementWindow> (sibling of <patterns> on the root) ---------
        // `RefinementManager.createRefinementWindow` reads these lists to
        // lay out the Refining and Upgrade tabs.
        for win in children(&root, "refinementWindow") {
            let ore_row = |n: roxmltree::Node<'_, '_>| RefineryOre {
                ore_type: attr_i32(&n, "type"),
                tooltip_key: attr(&n, "tooltipKey").to_string(),
                icon_res_key: attr(&n, "iconResKey").to_string(),
                button: attr_bool(&n, "button"),
            };
            for refine in children(&win, "refinement") {
                for section in children(&refine, "staticOres") {
                    game.refinement.refine_static =
                        children(&section, "ore").map(ore_row).collect();
                }
                for section in children(&refine, "ores") {
                    game.refinement.refine_ores = children(&section, "ore").map(ore_row).collect();
                }
            }
            for update in children(&win, "update") {
                for section in children(&update, "staticOres") {
                    game.refinement.update_static =
                        children(&section, "ore").map(ore_row).collect();
                }
                for section in children(&update, "ores") {
                    game.refinement.update_ores = children(&section, "ore").map(ore_row).collect();
                }
                for section in children(&update, "items") {
                    game.refinement.update_items = children(&section, "item")
                        .map(|n| RefineryItem {
                            type_id: attr_i32(&n, "type"),
                            identifier: attr(&n, "identifier").to_string(),
                            res_key: attr(&n, "resKey").to_string(),
                            language_key: attr(&n, "languageKey").to_string(),
                            amount_key: attr(&n, "ammountKey").to_string(),
                        })
                        .collect();
                }
            }
        }
        Ok(game)
    }

    /// `patterns` root — also tolerates files whose root IS `<patterns>`.
    pub fn ship(&self, type_id: &str) -> Option<&ShipPattern> {
        self.ships_by_type.get(type_id)
    }

    /// Ship pattern by numeric `shipId` (server-sent hero/design id).
    pub fn ship_by_id(&self, ship_id: i32) -> Option<&ShipPattern> {
        self.ships_by_ship_id
            .get(&ship_id)
            .and_then(|t| self.ships_by_type.get(t))
    }

    /// Ship pattern by `npcId` (server-sent NPC design id).
    pub fn ship_by_npc(&self, npc_id: i32) -> Option<&ShipPattern> {
        self.ships_by_npc_id
            .get(&npc_id)
            .and_then(|t| self.ships_by_type.get(t))
    }

    /// `render/mod.rs` compat alias — pattern by `type` string.
    pub fn ship_by_type(&self, type_id: &str) -> Option<&ShipPattern> {
        self.ship(type_id)
    }

    /// `PatternManager.getLaserPattern(class,type)` —
    /// `laserClasses[c] || laserClasses[0]` then `[t] || [0] ||
    /// laserClasses[0][t]`.
    pub fn laser_salvo(&self, class_id: i32, type_id: i32) -> Option<&LaserSalvoPattern> {
        let by_class = self.lasers.get(&class_id).or_else(|| self.lasers.get(&0))?;
        by_class
            .get(&type_id)
            .or_else(|| by_class.get(&0))
            .or_else(|| self.lasers.get(&0).and_then(|z| z.get(&type_id)))
    }

    /// `Lasers2D.getSalvos`/`getLaserCannonLocalPosition` for a ship's
    /// `expansionClassID`: mount-name groups from the stage with the
    /// highest id < 1001 (the AS3 stage pick), resolved to ship-local
    /// (x,y) offsets at `heading_deg` via `positionsList[frame]` where
    /// `frame = int(32*deg/360 + 0.5) % 32 % len`. Mounts are Flash
    /// y-down, same as world coords — no axis flip.
    ///
    /// Empty `Vec` = no expansion/mounts: the caller fires one bolt from
    /// the ship center (stock `DEFAULT_SALVOS`/`mainCannon`).
    pub fn expansion_salvos(&self, expansion_id: &str, heading_deg: f32) -> Vec<Vec<(f32, f32)>> {
        self.expansion_salvo_mounts(expansion_id, heading_deg)
            .into_iter()
            .map(|group| group.into_iter().map(|(_, offset)| offset).collect())
            .collect()
    }

    /// [`Self::expansion_salvos`] keeping the salvo mount names alongside
    /// their resolved `(x,y)` offsets — the 3D path resolves the same
    /// names against authored `laserpoint_{name}` mesh slots
    /// (`Lasers3D.getLaserCannonSlot`).
    pub fn expansion_salvo_mounts(
        &self,
        expansion_id: &str,
        heading_deg: f32,
    ) -> Vec<Vec<(String, (f32, f32))>> {
        let Some(class) = self.expansions.get(expansion_id) else {
            return Vec::new();
        };
        // Lasers2D.expansionPattern: highest stage id below 1001.
        let Some(stage) = class
            .stages
            .keys()
            .filter(|id| **id < 1001)
            .max()
            .and_then(|id| class.stages.get(id))
        else {
            return Vec::new();
        };
        let mut deg = heading_deg % 360.0;
        if deg < 0.0 {
            deg += 360.0;
        }
        stage
            .salvos
            .iter()
            .map(|group| {
                group
                    .iter()
                    .filter_map(|name| {
                        let list = stage.positions.get(name)?;
                        let idx = ((32.0 * deg / 360.0 + 0.5) as usize % 32) % list.len();
                        list.get(idx).map(|offset| (name.clone(), *offset))
                    })
                    .collect()
            })
            .filter(|g: &Vec<(String, (f32, f32))>| !g.is_empty())
            .collect()
    }

    /// `PatternManager.getRocketPattern(class,id)` — callers fall back to
    /// `(0,1)` (`RocketEffect2D` `|| getRocketPattern(0,1)`).
    pub fn rocket_pattern(&self, class_id: i32, id: i32) -> Option<&RocketPattern> {
        self.rockets
            .get(&class_id)
            .and_then(|m| m.get(&id))
            .or_else(|| self.rockets.get(&0).and_then(|m| m.get(&1)))
    }

    /// `PatternManager.getPyroPattern(class,id)` — `pyroClasses`.
    pub fn pyro(&self, class_id: i32, id: i32) -> Option<&PyroPattern> {
        self.pyro.get(&class_id).and_then(|m| m.get(&id))
    }

    /// `PatternManager.getShieldPattern(id)` —
    /// `shieldClasses[id] || shieldClasses["default"]`.
    pub fn shield_pattern(&self, type_id: i32) -> Option<&ShieldPattern> {
        self.shields
            .get(&type_id.to_string())
            .or_else(|| self.shields.get("default"))
    }

    /// Generic effect resource by its game XML `effect` name.
    pub fn effect_by_name(&self, name: &str) -> Option<&EffectPattern> {
        self.effects.values().find(|effect| effect.effect == name)
    }
}

/// `shipPatterns[type] = pattern; byShipId[shipId]; byNpcId[npcId]` —
/// unconditional inserts (AS3 semantics: later entries overwrite, and
/// designs that lack `shipId`/`npcId` inherit the base's ids).
fn insert_ship(game: &mut GameXml, pat: ShipPattern) {
    if pat.type_id.is_empty() {
        return;
    }
    game.ships_by_ship_id
        .insert(pat.ship_id, pat.type_id.clone());
    game.ships_by_npc_id.insert(pat.npc_id, pat.type_id.clone());
    game.ships_by_type.insert(pat.type_id.clone(), pat);
}

/// Remove `<!--` .. `-->` regions (first close wins). The shipped game.xml
/// has a nested `<!--` inside a comment — invalid XML that Flash tolerated;
/// comments are dead data anyway.
fn strip_comments(xml: &str) -> String {
    let mut out = String::with_capacity(xml.len());
    let mut rest = xml;
    while let Some(open) = rest.find("<!--") {
        out.push_str(&rest[..open]);
        match rest[open + 4..].find("-->") {
            Some(close) => rest = &rest[open + 4 + close + 3..],
            None => return out, // unterminated — drop the tail
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shipped fixture carries real `StarlingTiledMap` patterns —
    /// `asteroid_desert`/`scrap_pirates` (poizone tiles) and the
    /// `stars2_50x50` family.
    #[test]
    fn background_patterns_parse_tiled_fields() {
        let xml = std::fs::read_to_string("tests/fixtures/game.xml").expect("fixture missing");
        let game = GameXml::parse(&xml).expect("game.xml parses");

        let p = &game.backgrounds[&2001];
        assert!(p.tiled);
        assert_eq!(p.res_key, "stars2_50x50");
        assert_eq!(p.res_key_starling, "stars2_50x50_texture");
        assert_eq!(p.atlas_xml, "stars2_50x50_atlasXml");
        assert_eq!(p.tile_width, 50);
        assert_eq!(p.tile_height, 50);

        // `isTiled` without Starling keys -> the BasicElement2D image path.
        let p = &game.backgrounds[&2000];
        assert!(p.tiled);
        assert_eq!(p.res_key, "stars1-50x50");
        assert!(p.res_key_starling.is_empty());
        assert!(p.atlas_xml.is_empty());
        assert_eq!(p.tile_width, 50);

        // Poizone tile patterns also parse through the same struct.
        let p = &game.backgrounds[&3000];
        assert!(p.tiled);
        assert_eq!(p.res_key, "asteroid_desert");
        assert_eq!(p.res_key_starling, "asteroid_desert_texture");
        assert_eq!(p.atlas_xml, "asteroid_desert_atlasXml");
        assert_eq!(p.tile_width, 256);
        assert!(p.order.is_empty(), "order defaults to empty");

        // `content="custom"` — solid colour, no resource keys.
        let p = &game.backgrounds[&1999];
        assert!(p.is_custom);
        assert!(!p.tiled);
    }

    /// Real `<drones><formations>` tables: STANDARD (id 0) with the
    /// back/right/left groups, and the other formations are present too.
    #[test]
    fn drone_formations_parse_from_fixture() {
        let xml = std::fs::read_to_string("tests/fixtures/game.xml").expect("fixture missing");
        let game = GameXml::parse(&xml).expect("game.xml parses");
        for id in [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 42] {
            assert!(game.drone_formations.contains_key(&id), "formation {id}");
        }
        let std = &game.drone_formations[&0];
        assert_eq!(std.name, "standard");
        assert_eq!(std.locations.len(), 12);
        // 1 drone -> position 1 (back), 2 drones -> 3,4 (back pair).
        assert_eq!(std.position(1, 0), Some((150.0, 0.0)));
        assert_eq!(std.position(2, 0), Some((187.5, 37.5)));
        assert_eq!(std.position(2, 1), Some((187.5, -37.5)));
        // 12 drones fill every slot, in order.
        assert_eq!(std.position(12, 11), Some((0.0, 112.5)));
        // Turtle (scale 1.3) takes the list order verbatim.
        let turtle = &game.drone_formations[&1];
        assert_eq!(turtle.scale, 1.3);
        assert_eq!(turtle.position(2, 1), Some((-100.0, 50.0)));
    }
}
