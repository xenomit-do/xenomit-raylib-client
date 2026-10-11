//! Game state + rules glue: turns the shared world model into a playable
//! scene — selection, orders, camera, effects — the `map/model` +
//! `map/hud` equivalent.
//!
//! Frame contract (called from the app loop):
//! ```ignore
//! game.poll_input(&mut rl, &world, &mut cmds);       // raylib input pass
//! game.update(dt, &world, &mut assets, &mut cmds);  // world sync pass
//! // ... or the combined form:
//! game.update_with_rl(&mut rl, dt, &world, &mut assets, &mut cmds);
//! ```

use crate::assets::tilemap::TileGrid;
use crate::assets::{AssetManager, SpriteId};
use crate::net::world::WorldFx;
use crate::net::{NetCommand, SharedWorld};
use raylib::prelude::RaylibHandle;
use raylib::prelude::{KeyboardKey, MouseButton};
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};
use xenomit_packet_client::model::{CollectableKind, EntityKind};

pub mod camera;
mod combat_fx;
pub mod drones;
pub mod effects;
pub mod headings;
pub mod input;
pub mod lensflares;
pub mod orders;
pub mod settings_ui;
pub mod ship_fx;
pub mod ships;
pub mod ui_input;
pub mod visuals;

pub use camera::Camera;
pub use visuals::{MinePulse, MinimapBg, PortalLayer, Visual};

use effects::{Effect, EffectQueue};
use headings::{
    attacker_salvo_mounts, attacker_slots3d, ease_headings, laser_class_of, rocket_class_of,
    update_headings,
};
use input::ClickAction;

/// Distance to `hero_dest` at which the retained movement order is cleared.
const DEST_ARRIVE_RADIUS: f32 = 16.0;
/// Hero proximity to a pending collectable that also counts as "arrived"
/// (the server may stop the ship a touch off the exact click point).
/// Must be small: `handleMoveComplete` in the stock client only fires
/// once the move *finishes* on the box, not while still flying over.
const COLLECT_ARRIVE_RADIUS: f32 = 60.0;
/// Logical map size at `scaleFactor == 1.0` (`Map.STD_WIDTH`/`STD_HEIGHT`).
pub const BASE_MAP_SIZE: (f32, f32) = (21000.0, 13100.0);
/// Re-issue interval for hold-to-move (`EventManager` pacing).
const HOLD_MOVE_EVERY: Duration = Duration::from_millis(100);
/// `itemsControlMenu` atlas file ids (resources.xml): 38px icons named by
/// item id (`ammunition_laser_lcb-10`, `ammunition_rocket_plt-2026`, ...).
/// `assetsProxy.getBitmapDataFromAtlas(item.iconLootId, itemsControlMenuBitmapAtlas)`.
pub const ITEM_ICON_ATLAS_XML: &str = "itemsControlMenu_atlasXml";
pub const ITEM_ICON_ATLAS_TEX: &str = "itemsControlMenu_texture";

/// `LockDisplay3D.TYPE_2_URL` — particle-group stems for the 3D lock
/// indicator, indexed `lockType - 1` (`abstract_ui_mark_target_*.zip`).
/// `render::lock3d` maps lock types to the same ids.
const LOCK3D_STEMS: [&str; 4] = [
    "abstract_ui_mark_target_red",
    "abstract_ui_mark_target_gray_light",
    "abstract_ui_mark_target_purple",
    "abstract_ui_mark_target_gray_dark",
];

/// Static refinery view-model cloned out of `assets.game_xml` once —
/// `poll_input`/`window_body_click` run without the asset manager, so
/// the parsed `<refinementWindow>` lists and ore recipe patterns are
/// snapshotted here (immutable for the session).
#[derive(Clone, Debug, Default)]
pub struct RefineryData {
    /// `<refinementWindow>` ore/item lists (`refine_static`, …).
    pub window: crate::assets::gamexml::RefinementWindow,
    /// ore `type` -> pattern (`res_key`, `language_key`, `refiner`).
    pub ores: HashMap<i32, crate::assets::gamexml::OrePattern>,
}

/// A collect order awaiting move completion — `Collect`/`Harvest` goes
/// out when the ordered move finishes, mirroring the stock
/// `moveTrait.goto(..., handleMoveComplete)` flow.
pub(crate) struct PendingCollect {
    hash: String,
    kind: CollectableKind,
    /// Destination of the ordered move (the collectable's position).
    pos: (f32, f32),
    /// The ordered move tween has been observed. `NetCommand::Move` is
    /// applied on the net thread, so the first frames after
    /// `order_collect` still see the previous tween — arming prevents a
    /// stale/redirected tween from cancelling the order early.
    armed: bool,
}

/// Interactive game session state.
pub struct Game {
    pub camera: Camera,
    /// Currently selected entity id (lock target).
    pub selected: Option<i32>,
    /// Hero destination retained for movement prediction and camera tracking.
    pub hero_dest: Option<(f32, f32)>,
    pub effects: EffectQueue,
    /// Per-entity visual-modifier effect state (`EffectManager` +
    /// `EffectsManager2D` equivalent) — driven by `WorldFx::
    /// VisualModifier`/`ShipFx` events, drawn by `render::ship_fx`.
    pub ship_fx: ship_fx::ShipFxState,
    /// Floating HUD windows (ship, user, minimap, quests, pet, log,
    /// settings) — geometry, z-order, persisted layout.
    pub windows: crate::ui::WindowManager,
    /// Persistent top-left shortcut order and visibility.
    pub toolbar: crate::ui::toolbar::ToolbarLayout,
    /// Toolbar edit mode reveals hidden buttons and enables drag ordering.
    pub toolbar_editing: bool,
    /// Drag source plus the original pointer position.
    pub toolbar_drag: Option<(crate::ui::toolbar::ToolbarButton, (f32, f32))>,
    /// Non-geometry UI state (minimap zoom, quest selection, flyout).
    pub ui: crate::ui::UiState,
    /// `<refinementWindow>` + ore patterns snapshot for input handling —
    /// filled from `assets.game_xml` on the first `update`.
    pub refinery: Option<RefineryData>,
    /// Client-local display preferences, separate from server game settings.
    pub preferences: crate::ui::preferences::UiPreferences,
    /// Window defaults were fitted to the real viewport (or a saved
    /// layout was loaded).
    ui_ready: bool,
    /// F2 debug overlay toggle.
    pub show_debug: bool,
    /// Map bounds (scaled by `scaleFactor`).
    pub bounds: (f32, f32),
    pub map_id: i32,
    /// entity id -> resolved sprite recipe (ships, assets, hero).
    pub entity_visuals: HashMap<i32, Visual>,
    /// collectable hash -> resolved sprite recipe.
    pub collectable_visuals: HashMap<String, Visual>,
    /// portal gate id -> resolved sprite recipe (`mapAssetView` atlases).
    pub portal_visuals: HashMap<i32, Visual>,
    /// owner id -> tweened drone formation (`DronesFormation2D`).
    pub drone_rigs: HashMap<i32, drones::DroneRig>,
    /// `(type, level, stats, visual)` -> drone sprite recipe.
    pub drone_visuals: HashMap<drones::DroneKey, Visual>,
    /// Swarm minion id -> world position on its mother's `dock_N` pivot.
    pub docked: HashMap<i32, (f32, f32)>,
    /// `MiniMapBackground` — resolved `minimap-*` art sprite + fade
    /// bookkeeping; re-resolved when the map or size class changes.
    pub minimap_bg: Option<MinimapBg>,
    /// `LensFlaresLayer` — the map's sun flares (maps-config +
    /// `display3D` providers), rebuilt on map change.
    pub lensflares: lensflares::LensFlares,
    /// map background / planet key -> sprite id.
    pub map_visuals: HashMap<String, SpriteId>,
    /// map background key -> precomputed `StarlingTiledMap` cell->tile-id
    /// grid (only for `isTiled && resKeyStarling && atlasXml` layers).
    pub tile_grids: HashMap<String, TileGrid>,
    /// entity id -> immediate ship heading in degrees (for ship rotation strips).
    pub headings: HashMap<i32, f32>,
    /// Eased heading used only by drone rotation and formation placement.
    pub drone_headings: HashMap<i32, f32>,
    /// Menu item icon key (`iconLootId`, else `menuItemId`) -> atlas
    /// region sprite. Session scope — the `itemsControlMenu` atlas is
    /// account data, so this is NOT cleared on map change.
    pub item_icons: HashMap<String, SpriteId>,
    /// `(srcKey, resKey)` -> sprite id for session-scope UI sprites —
    /// rank/faction/formation icons, gg badges, `ship_border`, minimap
    /// markers. Requested lazily per frame; NOT cleared on map change.
    pub ui_sprites: HashMap<(String, String), SpriteId>,
    /// When `selected` last changed — `ship_border` fades in over 300 ms.
    pub selected_at: Option<Instant>,
    /// Previous-frame selection for the `selected_at` edge.
    prev_selected: Option<i32>,
    /// True while the right-mouse free-look drag is active.
    pub panning: bool,
    /// True while the touch/preview movement stick is held.
    pub mobile_joystick_active: bool,
    /// Last captured stick pointer in screen coordinates, for drawing its knob.
    pub mobile_joystick_position: Option<(f32, f32)>,
    pub mobile_toolbar_collapsed: bool,
    /// Stable raylib touch id owning the movement stick, independent of
    /// fingers used for attack/target buttons.
    mobile_joystick_touch: Option<i32>,
    /// Touch id currently steering ordinary UI panels.
    mobile_ui_touch: Option<i32>,
    /// Finger ids observed during the previous input poll.
    mobile_prev_touches: HashSet<i32>,
    /// Collectable positions seen last frame — used to emit `CollectFlash`
    /// when one disappears.
    collectable_pos: HashMap<String, (f32, f32)>,
    /// First-seen time for the game-authored box spawn animation.
    pub collectable_spawned: HashMap<String, Instant>,
    /// Next engine exhaust emission time for each moving ship.
    pub engine_smoke_next: HashMap<i32, Instant>,
    /// Loaded game engine-smoke clips, keyed by resources.xml file id.
    pub engine_smoke_sprites: HashMap<String, SpriteId>,
    /// Entity ids already credited with an `Explosion` effect.
    seen_destroyed: HashSet<i32>,
    /// Collect order awaiting arrival — `Collect`/`Harvest` goes out when
    /// the move finishes, mirroring `moveTrait.goto(handleMoveComplete)`.
    pending_collect: Option<PendingCollect>,
    /// Explicitly requested pickups, retained until the server confirms
    /// their removal so only our own collection gets the collection effect.
    collect_requested: HashMap<String, Instant>,
    /// Next time a held left button may re-issue a `Move` (hold-to-move).
    hold_move_next: Option<Instant>,
    /// True while a left-press that started on a HUD panel is held —
    /// suppresses hold-to-move so dragging on a panel can't steer.
    lmb_on_hud: bool,
    /// Reconnect status line for the HUD banner (`Some` while the app is
    /// re-dialling the game socket after a drop/jump).
    pub reconnect_note: Option<String>,
    /// Last mouse position during a right-drag pan.
    pan_last: Option<(f32, f32)>,
    /// Last entity the `Tab` key cycled to — repeat presses advance to
    /// the next-nearest NPC (`KeyboardMapper` cycling).
    tab_cycle: Option<i32>,
    /// `--shot-on-hit`: the fx drain arms `shot_at` on each inbound
    /// combat event so the app captures a frame ~250 ms later — bolts
    /// are then provably mid-flight in the screenshot.
    pub shot_on_fx: bool,
    /// Screenshot request timestamp set by the fx drain (consumed and
    /// cleared by the app loop).
    pub shot_at: Option<Instant>,
    /// `"{kind}:{type_id}:t{asset_type}"` already warned as a placeholder —
    /// keeps the report once-per-type instead of once-per-entity.
    placeholders_logged: HashSet<String>,
    /// F3 display-settings overlay toggle.
    pub show_settings: bool,
    /// F4 native 2D/3D component settings panel.
    pub show_render_settings: bool,
    /// Unsaved component rendering choices shown in the settings panel.
    pub render_settings_draft: Option<settings_ui::RenderSettingsDraft>,
    /// Flags flipped by the overlay since the last `update` (input runs
    /// on `&SharedWorld`, the flip needs `&mut`).
    settings_toggles: Vec<crate::net::SettingFlag>,
    /// Last `laserColor` (ammo type) seen per attacker — picks the
    /// `laserDamage<N>` impact clip for the next `AttackHit`.
    laser_types: HashMap<i32, i32>,
    /// Ships flagged by `BOOOM` — their destruction plays the
    /// "soundword" pyro class instead of the regular explosion.
    special_explosions: HashSet<i32>,
}

impl Game {
    /// Open the draft-based component rendering settings page.
    pub fn open_render_settings(&mut self) {
        self.show_render_settings = true;
        self.render_settings_draft = Some(settings_ui::RenderSettingsDraft::new(
            self.preferences.render,
        ));
    }

    /// Route a pointer press through the transactional 3D settings controls.
    /// Returns true while the modal owns the pointer, including empty panel
    /// space, so mobile touch controls never receive the same press.
    pub fn handle_render_setting_click(&mut self, p: (f32, f32)) -> bool {
        if !self.show_render_settings {
            return false;
        }
        let (vw, vh) = self.camera.viewport;
        if !input::point_in(settings_ui::render_layout(vw, vh).0, p) {
            return true;
        }
        if let Some((field, use_3d)) = settings_ui::render_component_hit(vw, vh, p) {
            if let Some(draft) = self.render_settings_draft.as_mut() {
                field.set(&mut draft.0, use_3d);
            }
            return true;
        }
        if let Some(quality) = settings_ui::render_quality_hit(vw, vh, p) {
            if let Some(draft) = self.render_settings_draft.as_mut() {
                draft.0.texture_quality = quality;
            }
            return true;
        }
        match settings_ui::render_hit(vw, vh, p) {
            Some(settings_ui::RenderSetting::Reset) => {
                if let Some(draft) = self.render_settings_draft.as_mut() {
                    draft.reset();
                }
            }
            Some(settings_ui::RenderSetting::Save) => {
                if let Some(draft) = self.render_settings_draft.take() {
                    self.preferences.set_render_preferences(draft.save());
                    self.preferences.flush();
                }
                self.show_render_settings = false;
            }
            Some(settings_ui::RenderSetting::Cancel) => {
                self.render_settings_draft = None;
                self.show_render_settings = false;
            }
            Some(settings_ui::RenderSetting::General) => {
                self.render_settings_draft = None;
                self.show_render_settings = false;
                self.windows.open(crate::ui::WindowId::Settings);
            }
            None => {}
            Some(_) => {}
        }
        true
    }

    pub fn new(map_id: i32, bounds: (f32, f32)) -> Self {
        Self {
            camera: Camera::default(),
            selected: None,
            hero_dest: None,
            effects: Default::default(),
            ship_fx: Default::default(),
            windows: crate::ui::WindowManager::with_defaults(1280.0, 720.0),
            toolbar: crate::ui::toolbar::ToolbarLayout::default(),
            toolbar_editing: false,
            toolbar_drag: None,
            ui: crate::ui::UiState::default(),
            refinery: None,
            preferences: crate::ui::preferences::UiPreferences::default(),
            ui_ready: false,
            show_debug: false,
            bounds,
            map_id,
            entity_visuals: HashMap::new(),
            collectable_visuals: HashMap::new(),
            portal_visuals: HashMap::new(),
            drone_rigs: HashMap::new(),
            drone_visuals: HashMap::new(),
            docked: HashMap::new(),
            minimap_bg: None,
            lensflares: Default::default(),
            map_visuals: HashMap::new(),
            tile_grids: HashMap::new(),
            headings: HashMap::new(),
            drone_headings: HashMap::new(),
            item_icons: HashMap::new(),
            ui_sprites: HashMap::new(),
            selected_at: None,
            prev_selected: None,
            panning: false,
            mobile_joystick_active: false,
            mobile_joystick_position: None,
            mobile_toolbar_collapsed: false,
            mobile_joystick_touch: None,
            mobile_ui_touch: None,
            mobile_prev_touches: HashSet::new(),
            collectable_pos: HashMap::new(),
            collectable_spawned: HashMap::new(),
            engine_smoke_next: HashMap::new(),
            engine_smoke_sprites: HashMap::new(),
            seen_destroyed: HashSet::new(),
            pending_collect: None,
            collect_requested: HashMap::new(),
            hold_move_next: None,
            lmb_on_hud: false,
            reconnect_note: None,
            pan_last: None,
            tab_cycle: None,
            shot_on_fx: false,
            shot_at: None,
            placeholders_logged: HashSet::new(),
            show_settings: false,
            show_render_settings: false,
            render_settings_draft: None,
            settings_toggles: Vec::new(),
            laser_types: HashMap::new(),
            special_explosions: HashSet::new(),
        }
    }

    /// Apply touch-first HUD mode. Public for startup flags such as
    /// `--mobile-preview`; it intentionally does not dirty saved settings.
    pub fn set_mobile_hud(&mut self, enabled: bool) {
        self.preferences.mobile_hud = enabled;
        if enabled {
            self.ui.flyout = None;
            for id in [
                crate::ui::WindowId::Ship,
                crate::ui::WindowId::User,
                crate::ui::WindowId::Minimap,
                crate::ui::WindowId::Log,
            ] {
                self.windows.close(id);
            }
        }
    }

    /// Convenience: input pass + world sync in one call (app loop entry).
    pub fn update_with_rl(
        &mut self,
        rl: &mut RaylibHandle,
        dt: f32,
        world: &mut SharedWorld,
        assets: &mut AssetManager,
        out: &mut Vec<NetCommand>,
    ) {
        self.poll_input(rl, world, out);
        self.update(dt, world, assets, out);
    }

    /// Consume input + world state, emit `NetCommand`s, update tweens/FX.
    /// Called once per frame before drawing. `world` is mutable so the
    /// `fx` queue (`WorldFx` combat events) can be drained here.
    pub fn update(
        &mut self,
        dt: f32,
        world: &mut SharedWorld,
        assets: &mut AssetManager,
        out: &mut Vec<NetCommand>,
    ) {
        self.effects.gc();
        let now = Instant::now();
        for flag in std::mem::take(&mut self.settings_toggles) {
            world.settings.toggle(flag);
        }

        let hero_id = world.world.hero.as_ref().map(|h| h.user_id);
        let hero_pos = hero_id.map(|id| world.pos(id));

        // --- map transition -------------------------------------------------
        if let Some(h) = &world.world.hero {
            if h.map_id != 0 && h.map_id != self.map_id {
                self.map_id = h.map_id;
                let scale = assets
                    .maps
                    .get(self.map_id)
                    .map(|m| m.scale_factor)
                    .filter(|s| *s > 0.0)
                    .unwrap_or(1.0);
                self.bounds = (BASE_MAP_SIZE.0 * scale, BASE_MAP_SIZE.1 * scale);
                self.map_visuals.clear();
                self.tile_grids.clear();
                self.entity_visuals.clear();
                self.collectable_visuals.clear();
                self.portal_visuals.clear();
                self.drone_rigs.clear();
                self.docked.clear();
                self.collectable_pos.clear();
                self.collectable_spawned.clear();
                self.engine_smoke_next.clear();
                self.collect_requested.clear();
                self.seen_destroyed.clear();
                self.laser_types.clear();
                self.special_explosions.clear();
                self.headings.clear();
                self.drone_headings.clear();
                self.ship_fx.clear();
                // Orders and locks die with the map.
                self.hero_dest = None;
                self.selected = None;
                self.pending_collect = None;
                self.hold_move_next = None;
            }
        }

        // --- camera ---------------------------------------------------------
        if let Some(hp) = hero_pos {
            if !self.panning {
                self.camera.follow(hp, dt);
            }
            // Clear the retained movement order once we arrive.
            if let Some(d) = self.hero_dest {
                let dx = hp.0 - d.0;
                let dy = hp.1 - d.1;
                if dx * dx + dy * dy < DEST_ARRIVE_RADIUS * DEST_ARRIVE_RADIUS {
                    self.hero_dest = None;
                }
            }
        }

        // --- sprites: resolve recipes lazily --------------------------------
        if let Some(h) = &world.world.hero {
            let ov = world
                .ship_info
                .get(&h.user_id)
                .and_then(|i| i.design_override.as_deref());
            let type_id = ships::effective_type_id(&h.type_id, ov, assets).to_owned();
            self.ensure_entity_visual(
                h.user_id,
                &type_id,
                EntityKind::Ship,
                0,
                h.faction_id,
                None,
                0,
                assets,
            );
        }
        for e in world.world.entities.values() {
            // The hero's entity row has an empty type_id — resolving it
            // here would clobber the visual resolved from `hero.type_id`.
            if Some(e.id) == hero_id {
                continue;
            }
            let asset_type = world.entity_asset_type.get(&e.id).copied().unwrap_or(0);
            let ov = world
                .ship_info
                .get(&e.id)
                .and_then(|i| i.design_override.as_deref());
            let type_id = ships::effective_type_id(&e.type_id, ov, assets).to_owned();
            let pet_level = Self::pet_level(world, e.id);
            self.ensure_entity_visual(
                e.id,
                &type_id,
                e.kind,
                asset_type,
                e.faction_id,
                Some(e),
                pet_level,
                assets,
            );
        }
        self.update_drones(world, assets);
        self.update_docks(world, assets);
        for c in world.world.collectables.values() {
            self.ensure_collectable_visual(
                &c.hash,
                c.kind,
                c.type_id,
                &c.type_str,
                c.pulse_color_id,
                assets,
            );
        }
        for p in world.portals.values() {
            self.ensure_portal_visual(p.id, &p.type_id, assets);
        }
        self.ensure_map_visuals(assets);
        self.report_dead_sprites(world, assets);
        // Drop cached visuals for entities that left the map.
        self.entity_visuals
            .retain(|id, _| Some(*id) == hero_id || world.world.entities.contains_key(id));
        self.collectable_visuals
            .retain(|h, _| world.world.collectables.contains_key(h));
        self.portal_visuals
            .retain(|id, _| world.portals.contains_key(id));

        // 3D resources are independent CDN assets, so request only recipes
        // selected for 3D. `AssetManager::pump` completes these on its worker;
        // the render cache then uploads them on the next frame.
        let rp = self.preferences.render;
        let quality = rp.texture_quality;
        for (&id, visual) in &self.entity_visuals {
            let is_npc = world.world.entities.get(&id).is_some_and(|e| e.npc);
            if !(if is_npc { rp.npcs_3d } else { rp.ships_3d }) {
                continue;
            }
            if let Some(recipe) = &visual.display_3d {
                for mesh in &recipe.meshes {
                    visuals::request_mesh_resources(
                        assets,
                        &mesh.geometry,
                        &mesh.material,
                        quality,
                    );
                }
                for particles in &recipe.particles {
                    assets.update_particle_group(&particles.resource);
                }
            }
        }
        if rp.collectables_3d {
            for visual in self.collectable_visuals.values() {
                if let Some(recipe) = &visual.display_3d {
                    for mesh in &recipe.meshes {
                        visuals::request_mesh_resources(
                            assets,
                            &mesh.geometry,
                            &mesh.material,
                            quality,
                        );
                    }
                    for particles in &recipe.particles {
                        assets.update_particle_group(&particles.resource);
                    }
                }
            }
        }
        if rp.portals_3d {
            for visual in self.portal_visuals.values() {
                if let Some(recipe) = &visual.display_3d {
                    for mesh in &recipe.meshes {
                        visuals::request_mesh_resources(
                            assets,
                            &mesh.geometry,
                            &mesh.material,
                            quality,
                        );
                    }
                    for particles in &recipe.particles {
                        assets.update_particle_group(&particles.resource);
                    }
                }
            }
        }
        if rp.drones_3d {
            for visual in self.drone_visuals.values() {
                if let Some(recipe) = &visual.display_3d {
                    for particles in &recipe.particles {
                        assets.update_particle_group(&particles.resource);
                    }
                }
            }
        }
        // `LockDisplay3D` target marker — the four lockType groups fetch
        // once while the 3D ship scene can show them (the same gate
        // `scene3d::draw_ship_models` uses).
        if rp.camera_3d || rp.npcs_3d || rp.ships_3d {
            for stem in LOCK3D_STEMS {
                assets.update_particle_group(stem);
            }
        }

        // --- item icons (session-scope `itemsControlMenu` atlas) ------------
        for item in world.menu.items.values() {
            let key = if item.icon_loot_id.is_empty() {
                item.id.as_str()
            } else {
                item.icon_loot_id.as_str()
            };
            if key.is_empty() {
                continue;
            }
            self.item_icons.entry(key.to_owned()).or_insert_with(|| {
                assets.request_region(ITEM_ICON_ATLAS_XML, ITEM_ICON_ATLAS_TEX, key)
            });
        }

        // `SimpleWindowIcon` uses these feature-menu atlas variants. Cache
        // all states once so hover/open feedback does not fall back to glyphs.
        for button in crate::ui::toolbar::ToolbarButton::ALL {
            for state in ["normal", "hover", "select"] {
                let region = button.icon_region(state);
                self.ui_sprites
                    .entry(("featuresMenu_atlasXml".to_owned(), region.clone()))
                    .or_insert_with(|| {
                        assets.request_region(
                            "featuresMenu_atlasXml",
                            "featuresMenu_texture",
                            &region,
                        )
                    });
            }
        }

        // `GroupUI.getShipIconBitmap` — each member's `nettyShipIcons`
        // bitmap from `groupSystemShipIcons`.
        if let Some(group) = &world.group {
            for m in &group.members {
                self.ui_sprite(
                    assets,
                    "groupSystemShipIcons",
                    crate::net::group::ship_icon_res(m.ship_icon),
                );
            }
        }

        // --- refinery / laboratory ---------------------------------------
        // Snapshot the immutable `<refinementWindow>` + ore patterns once —
        // `window_body_click` runs without the asset manager.
        if self.refinery.is_none() {
            self.refinery = Some(RefineryData {
                window: assets.game_xml.refinement.clone(),
                ores: assets.game_xml.ores.clone(),
            });
        }
        if self.windows.body_visible(crate::ui::WindowId::Refinery) {
            // Slot/ore/icon bitmaps ship as symbols in the `refinement`
            // SWF; `ore_tree` is part of `ui` (`createRefinementContainers`).
            for res in ["sourceSlot", "targetSlot", "targetIcon", "labelSlot"] {
                self.ui_sprite(assets, "refinement", res);
            }
            self.ui_sprite(assets, "ui", "ore_tree");
            let res_keys: Vec<String> = self
                .refinery
                .as_ref()
                .map(|d| {
                    let mut v: Vec<String> = Vec::new();
                    for ore in d
                        .window
                        .refine_static
                        .iter()
                        .chain(d.window.refine_ores.iter())
                        .chain(d.window.update_static.iter())
                        .chain(d.window.update_ores.iter())
                    {
                        if let Some(p) = d.ores.get(&ore.ore_type) {
                            v.push(p.res_key.clone());
                        }
                        if !ore.icon_res_key.is_empty() {
                            v.push(ore.icon_res_key.clone());
                        }
                    }
                    v.extend(d.window.update_items.iter().map(|i| i.res_key.clone()));
                    v.retain(|s| !s.is_empty());
                    v.sort_unstable();
                    v.dedup();
                    v
                })
                .unwrap_or_default();
            for res in res_keys {
                self.ui_sprite(assets, "refinement", &res);
            }
            // `RefinementManager.refreshItemList` — ~1 Hz `LabUpdateRequest`
            // while maximized; after 600 ticks the stock client minimizes.
            if self.ui.lab_refresh.is_none_or(|t| now >= t) {
                out.push(NetCommand::LabUpdate);
                self.ui.lab_refresh = Some(now + Duration::from_millis(999));
                self.ui.lab_ticks = self.ui.lab_ticks.saturating_add(1);
                if self.ui.lab_ticks > 599 {
                    if let Some(w) = self.windows.get_mut(crate::ui::WindowId::Refinery) {
                        if !w.collapsed {
                            w.toggle_collapse();
                        }
                    }
                    self.ui.lab_ticks = 0;
                }
            }
        } else {
            self.ui.lab_refresh = None;
            self.ui.lab_ticks = 0;
            self.ui.refinery_drag = None;
            self.ui.lab_dialog = None;
        }

        // --- ui sprites (rank/faction/formation/gg/lock icons) -----------
        // Session-scope `ui`/`icons` SWF symbols, requested on demand;
        // the renderer skips any that are still missing.
        self.ui_sprite(assets, "ui", "ship_border");
        self.ui_sprite(assets, "ui", "bar_background");
        if self.preferences.toolbar_background {
            self.ui_sprite(assets, "ui", "bg_standard.png");
            self.ui_sprite(assets, "ui", "bg_active.png");
        }
        for info in world.ship_info.values() {
            if info.daily_rank != 0 {
                let res = format!("rank{}", info.daily_rank);
                self.ui_sprite(assets, "icons", &res);
            }
            if info.drone_count > 0 {
                let res = format!("drone_formation{}_simple.png", info.drone_formation);
                self.ui_sprite(assets, "icons", &res);
            }
            if info.gg_done > 0 {
                for res in ["ggIconBg", "alphaToDelta_gg", "epsilonZeta_gg", "kronos_gg"] {
                    self.ui_sprite(assets, "ui", res);
                }
            }
        }
        for faction in world
            .world
            .entities
            .values()
            .map(|e| e.faction_id)
            .chain(world.world.hero.as_ref().map(|h| h.faction_id))
            .filter(|f| *f != 0)
        {
            let res = format!("fraction{faction}");
            self.ui_sprite(assets, "icons", &res);
        }

        // `ship_border` fades in when the lock target changes.
        if self.selected != self.prev_selected {
            self.selected_at = Some(Instant::now());
            self.prev_selected = self.selected;
        }

        // --- headings (rotation strips) -------------------------------------
        // Hero fallback: laser active but the link hasn't registered yet
        // -> face the locked selection.
        let hero_link = hero_id
            .filter(|h| {
                world.world.laser_active.contains(h) && !world.world.laser_links.contains_key(h)
            })
            .and_then(|h| self.selected.map(|t| (h, t)));
        update_headings(
            &mut self.headings,
            &world.tweens,
            &world.world.laser_links,
            hero_link,
            |id| world.pos(id),
        );
        // Ship facing snaps to the game-authored movement/aim heading.
        // Drone orientation and orbit positions ease independently so a
        // turning ship does not drag its ship sprite through a custom turn.
        ease_headings(&mut self.drone_headings, &self.headings, dt);
        self.update_engine_smoke(world, assets, now);

        // --- effects ----------------------------------------------------------
        // Combat events the net layer queued since last frame — each
        // `AttackLaserRunCommand` is one volley (server pace ≈ 1/s while a
        // link runs; stopping it stops the shots, as does the abort).
        let fx_events = std::mem::take(&mut world.fx);
        for fx in fx_events {
            // `--shot-on-hit`: an inbound volley means bolts will be
            // mid-flight ~250 ms out — ask the app for a capture.
            if self.shot_on_fx
                && self.shot_at.is_none()
                && matches!(
                    fx,
                    WorldFx::LaserRun { .. }
                        | WorldFx::Rocket { .. }
                        | WorldFx::Hit { .. }
                        | WorldFx::Miss { .. }
                )
            {
                self.shot_at = Some(now);
            }
            self.spawn_world_fx(fx, world, assets, now);
        }
        // `EffectManager` housekeeping: drop effects whose entity left
        // the map, expire timed/one-shot clips, request missing sprites.
        self.ship_fx
            .cleanup(|id| world.world.entities.contains_key(&id));
        self.ship_fx.update(assets);
        self.update_combat_fx(world, assets, now);
        self.lensflares
            .update(self.map_id, self.preferences.render.camera_3d, dt, assets);
        // In-flight rockets drop smoke puffs and fire their impact.
        let mut deferred: Vec<Effect> = Vec::new();
        for e in self.effects.active.iter_mut() {
            let Effect::Rocket {
                victim,
                duration,
                started,
                ..
            } = *e
            else {
                continue;
            };
            let target = world.pos(victim);
            let pos = e.rocket_pos(target, now);
            let arrived = now.saturating_duration_since(started).as_secs_f32() >= duration;
            let Effect::Rocket {
                smoke_sprite,
                smoke_interval,
                smoke_life,
                last_smoke,
                finished,
                impact_sprite,
                impact_scale,
                hit,
                hero_involved,
                ..
            } = e
            else {
                unreachable!()
            };
            if *smoke_interval > 0.0 && effects::smoke_moved(*last_smoke, pos) > *smoke_interval {
                *last_smoke = (pos.0 as i32, pos.1 as i32);
                deferred.push(Effect::SpriteBurst {
                    pos,
                    sprite: *smoke_sprite,
                    life: *smoke_life,
                    scale: 1.0,
                    fade: true,
                    rotation: 0.0,
                    ring_fallback: false,
                    started: now,
                });
            }
            if arrived {
                *finished = true;
                deferred.push(Effect::SpriteBurst {
                    pos,
                    sprite: *impact_sprite,
                    life: 0.6,
                    scale: *impact_scale,
                    fade: false,
                    rotation: 0.0,
                    ring_fallback: true,
                    started: now,
                });
                if !*hit && *hero_involved {
                    deferred.push(Effect::Text {
                        pos: target,
                        text: "MISS".to_owned(),
                        color: effects::COLOR_MISS,
                        started: now,
                    });
                }
            }
        }
        for e in deferred {
            self.effects.push(e);
        }
        // Explosions on the destroyed-flag edge (`pyroEffects[0][id]`).
        for e in world.world.entities.values() {
            if e.destroyed && !self.seen_destroyed.contains(&e.id) {
                self.seen_destroyed.insert(e.id);
                let pos = world.pos(e.id);
                let exp_id = assets
                    .game_xml
                    .ship_by_type(&e.type_id)
                    .map(|p| p.explosion_id)
                    .unwrap_or(0);
                // `BOOOM` flagged ships blow up as "soundword" pyro (class 6).
                let special = if self.special_explosions.remove(&e.id) {
                    let pick = 1 + (self.effects.rand() * 3.0) as i32;
                    assets.game_xml.pyro(6, pick.min(3))
                } else {
                    None
                };
                let pyro = special
                    .or_else(|| assets.game_xml.pyro(0, exp_id))
                    .or_else(|| assets.game_xml.pyro(0, 0));
                // `MapObjectEffect` plays the pyro pattern's `soundID`
                // at the explosion position.
                let pyro_sound = pyro.map(|p| p.sound_id).filter(|s| *s > 0);
                let pyro = pyro.map(|p| (p.res_key.clone(), p.scale, p.rotatable));
                match pyro_sound {
                    Some(s) => crate::audio::play_sound(s, Some(pos)),
                    None => crate::audio::play_sfx_at(crate::audio::Sfx::Explosion, pos.0, pos.1),
                }
                let (sprite, scale) = match &pyro {
                    Some((res, scale, _)) if !res.is_empty() => {
                        (Some(assets.request(res, "mc")), *scale)
                    }
                    _ => (None, 1.0 + 0.08 * exp_id.clamp(0, 10) as f32),
                };
                let rot = if pyro.map(|p| p.2).unwrap_or(false) {
                    self.effects.rand() * 360.0
                } else {
                    0.0
                };
                self.effects.push(Effect::SpriteBurst {
                    pos,
                    sprite,
                    life: 1.4,
                    scale,
                    fade: false,
                    rotation: rot,
                    ring_fallback: true,
                    started: now,
                });
            }
        }
        // Collectable pickups flash where the box disappeared.
        let mut gone = Vec::new();
        for (hash, pos) in &self.collectable_pos {
            if !world.world.collectables.contains_key(hash) {
                gone.push((hash.clone(), *pos));
            }
        }
        for (hash, pos) in gone {
            self.collectable_pos.remove(&hash);
            self.collectable_spawned.remove(&hash);
            // A mine/firework that went off already got its own burst.
            if world.silent_removals.remove(&hash) {
                self.collect_requested.remove(&hash);
                continue;
            }
            // Proximity alone caused effects for boxes collected by other
            // ships. Tie this to our explicit collect order.
            if self.collect_requested.remove(&hash).is_some() {
                if let Some(hid) = hero_id {
                    if let Some(res_key) = assets
                        .game_xml
                        .effect_by_name("loopingCollectorBeam")
                        .map(|effect| effect.res_key.clone())
                        .filter(|res_key| !res_key.is_empty())
                    {
                        let sprite = assets.request(&res_key, "mc");
                        self.effects.push(Effect::CollectorBeam {
                            from: effects::Anchor::Pos(pos),
                            to: effects::Anchor::Entity(hid),
                            sprite: Some(sprite),
                            life: effects::COLLECTOR_BEAM_LIFE,
                            started: now,
                        });
                    }
                    crate::audio::play_sfx_at(crate::audio::Sfx::Collect, pos.0, pos.1);
                }
            }
            self.effects
                .push(Effect::CollectFlash { pos, started: now });
        }
        for c in world.world.collectables.values() {
            self.collectable_pos
                .insert(c.hash.clone(), (c.x as f32, c.y as f32));
            self.collectable_spawned
                .entry(c.hash.clone())
                .or_insert(now);
        }
        self.collect_requested.retain(|hash, at| {
            world.world.collectables.contains_key(hash)
                && now.saturating_duration_since(*at) < Duration::from_secs(10)
        });
        // Selected target vanished -> drop the lock.
        if let Some(sel) = self.selected {
            if !world.world.entities.contains_key(&sel) {
                self.selected = None;
            }
        }

        // --- pending collect --------------------------------------------------
        // `MoveTrait.goto`'s onComplete is a TweenLite finish — *time*-based
        // (`dist/speed`), not a radius check: the request goes out wherever
        // the ship actually stands when the move ends, even if a beacon
        // parked it short of the box. Any new `goto` silently replaces the
        // pending callback, so a redirected move drops the collect order.
        if let Some(mut pending) = self.pending_collect.take() {
            if !world.world.collectables.contains_key(&pending.hash) {
                // Collected/despawned meanwhile — nothing to send.
            } else {
                let on_target = |to: (f32, f32)| {
                    let dx = to.0 - pending.pos.0;
                    let dy = to.1 - pending.pos.1;
                    dx * dx + dy * dy <= 4.0
                };
                let tween = hero_id.and_then(|id| world.tweens.get(&id));
                if !pending.armed {
                    pending.armed = tween.is_some_and(|t| on_target(t.to));
                }
                let arrived = hero_pos.is_some_and(|hp| {
                    let dx = hp.0 - pending.pos.0;
                    let dy = hp.1 - pending.pos.1;
                    dx * dx + dy * dy <= COLLECT_ARRIVE_RADIUS * COLLECT_ARRIVE_RADIUS
                });
                // Redirected: an armed order whose tween now points
                // elsewhere (server move, new order mid-flight) — the
                // ordered move no longer exists, done or not. A tween
                // that vanished after arming means the move is over.
                let redirected = pending.armed && tween.is_some_and(|t| !on_target(t.to));
                let move_over =
                    pending.armed && tween.map_or(true, |t| t.done() && on_target(t.to));
                if redirected {
                    // Order superseded — the callback was replaced.
                } else if arrived || move_over || self.hero_dest.is_none() {
                    // `handleMoveComplete`: Ore -> Harvest, Box/Beacon ->
                    // CollectBoxRequest; mines/firework boxes are clicked
                    // for movement only (no request branch in stock).
                    match pending.kind {
                        CollectableKind::Ore => {
                            self.collect_requested.insert(pending.hash.clone(), now);
                            out.push(NetCommand::Harvest { hash: pending.hash });
                        }
                        CollectableKind::Box | CollectableKind::Beacon => {
                            self.collect_requested.insert(pending.hash.clone(), now);
                            out.push(NetCommand::Collect { hash: pending.hash });
                        }
                        CollectableKind::Mine | CollectableKind::FireworkBox => {}
                    }
                } else {
                    self.pending_collect = Some(pending);
                }
            }
        }
    }
    /// `NetCommand`s. Kept separate from `update` so the world sync also
    /// runs headless in tests.
    pub fn poll_input(
        &mut self,
        rl: &mut RaylibHandle,
        world: &SharedWorld,
        out: &mut Vec<NetCommand>,
    ) {
        self.camera.viewport = (rl.get_screen_width() as f32, rl.get_screen_height() as f32);
        self.camera.perspective = self.preferences.render.camera_3d;

        let mouse = rl.get_mouse_position();
        let raw_mouse_p = (mouse.x, mouse.y);
        self.ensure_ui(self.camera.viewport.0, self.camera.viewport.1);
        let touch_points: Vec<(i32, (f32, f32))> = (0..rl.get_touch_point_count())
            .map(|i| {
                let p = rl.get_touch_position(i);
                (rl.get_touch_point_id(i), (p.x, p.y))
            })
            .collect();
        let touch_ids: HashSet<i32> = touch_points.iter().map(|(id, _)| *id).collect();
        let touch_mode = self.preferences.mobile_hud && !touch_points.is_empty();
        let mobile_layout =
            input::mobile_hud_layout(self.camera.viewport.0, self.camera.viewport.1);
        let active_touch_ids: Vec<i32> = touch_points.iter().map(|(id, _)| *id).collect();
        if input::retain_touch_id(self.mobile_joystick_touch, &active_touch_ids).is_none() {
            self.mobile_joystick_touch = None;
            self.mobile_joystick_active = false;
        }
        if self
            .mobile_ui_touch
            .is_some_and(|id| !touch_ids.contains(&id))
        {
            self.mobile_ui_touch = None;
        }
        let fresh_touch = touch_points
            .iter()
            .find(|(id, _)| !self.mobile_prev_touches.contains(id));
        if self.preferences.mobile_hud && self.mobile_joystick_touch.is_none() {
            if let Some((id, p)) = touch_points.iter().find(|(id, p)| {
                !self.mobile_prev_touches.contains(id)
                    && input::mobile_hud_hit(&mobile_layout, *p)
                        == Some(input::MobileControl::MoveStick)
            }) {
                self.mobile_joystick_touch = Some(*id);
                self.mobile_joystick_active = true;
                let _ = p;
            }
        }
        let mobile_touch_press = if self.preferences.mobile_hud {
            touch_points
                .iter()
                .find(|(id, p)| {
                    !self.mobile_prev_touches.contains(id)
                        && input::mobile_hud_hit(&mobile_layout, *p)
                            .is_some_and(|c| c != input::MobileControl::MoveStick)
                })
                .map(|(_, p)| *p)
        } else {
            None
        };
        if self.preferences.mobile_hud && self.mobile_ui_touch.is_none() {
            if let Some((id, _)) = touch_points.iter().find(|(id, p)| {
                !self.mobile_prev_touches.contains(id)
                    && Some(*id) != self.mobile_joystick_touch
                    && input::mobile_hud_hit(&mobile_layout, *p).is_none()
            }) {
                self.mobile_ui_touch = Some(*id);
            }
        }
        let ui_touch_position = self.mobile_ui_touch.and_then(|id| {
            touch_points
                .iter()
                .find(|(touch_id, _)| *touch_id == id)
                .map(|(_, p)| *p)
        });
        let mobile_joystick_position = self.mobile_joystick_touch.and_then(|id| {
            touch_points
                .iter()
                .find(|(touch_id, _)| *touch_id == id)
                .map(|(_, p)| *p)
        });
        self.mobile_joystick_position = mobile_joystick_position;
        let pointer_p = mobile_touch_press
            .or_else(|| {
                fresh_touch
                    .filter(|(id, _)| Some(*id) == self.mobile_joystick_touch)
                    .map(|(_, p)| *p)
            })
            .or(ui_touch_position)
            .unwrap_or(raw_mouse_p);
        let pointer_down = if touch_mode {
            mobile_touch_press.is_some()
                || ui_touch_position.is_some()
                || mobile_joystick_position.is_some()
        } else {
            rl.is_mouse_button_down(MouseButton::MOUSE_BUTTON_LEFT)
        };
        let mouse_p = pointer_p;

        // Wheel: scrolls the window under the cursor, else zooms the map.
        let wheel = rl.get_mouse_wheel_move();
        if wheel.abs() > 0.0 && !self.ui_wheel(world, mouse_p, wheel) {
            self.camera.zoom_by(wheel);
        }

        // HUD rects for this frame (action bar, ammo flyout).
        let hud = self.hud_for_frame();

        // Right click: slot -> ammo flyout (never starts a pan).
        let rmb_over_hud = self.ui_right_click(
            world,
            mouse_p,
            &hud,
            rl.is_mouse_button_pressed(MouseButton::MOUSE_BUTTON_RIGHT),
        );

        // Right-drag free-look pan: pan while held, resume follow on
        // release. A press that starts over a window/HUD never pans.
        if rl.is_mouse_button_down(MouseButton::MOUSE_BUTTON_RIGHT)
            && (self.panning || !rmb_over_hud && !self.ui_covers(mouse_p, &hud))
        {
            if let Some(last) = self.pan_last {
                let delta = (mouse.x - last.0, mouse.y - last.1);
                if delta.0 != 0.0 || delta.1 != 0.0 {
                    self.camera.pan_pixels(delta);
                }
            }
            self.pan_last = Some(mouse_p);
            self.panning = true;
        } else {
            self.pan_last = None;
            self.panning = false;
        }

        // Left click -> world orders. HUD rects are consulted first:
        // clicks inside a panel act on the HUD (or are swallowed) and
        // never become world orders.
        let hud_area = input::hud_hit(&hud, mouse_p);
        let lmb_pressed = if touch_mode {
            mobile_touch_press.is_some()
                || fresh_touch.is_some_and(|(id, _)| Some(*id) == self.mobile_joystick_touch)
                || self
                    .mobile_ui_touch
                    .is_some_and(|id| fresh_touch.is_some_and(|(fresh_id, _)| *fresh_id == id))
        } else {
            rl.is_mouse_button_pressed(MouseButton::MOUSE_BUTTON_LEFT)
        };
        let mobile_toolbar_toggle = self.preferences.mobile_hud
            && lmb_pressed
            && input::point_in(
                input::mobile_toolbar_toggle_rect(
                    self.camera.viewport.0,
                    self.mobile_toolbar_collapsed,
                ),
                mouse_p,
            );
        if let Some((from, start)) = self.toolbar_drag {
            if pointer_down {
                self.lmb_on_hud = true;
            } else {
                let moved = (mouse_p.0 - start.0).hypot(mouse_p.1 - start.1) >= 5.0;
                let buttons = self.toolbar.buttons(true);
                if moved {
                    if let Some(to) = crate::ui::toolbar::toolbar_hit_for(mouse_p, &buttons) {
                        self.toolbar.move_button(from, to);
                    }
                }
                self.toolbar_drag = None;
                self.lmb_on_hud = true;
            }
        }
        // Window drags follow the held button wherever the cursor goes.
        if self.windows.is_dragging() {
            if pointer_down {
                self.windows.drag(mouse_p);
            } else {
                self.windows.release();
            }
            self.lmb_on_hud = true;
        }
        // Refinery ore drag: release resolves the drop target
        // (`SourceRefinementModule.onMouseUp`).
        if self.ui.refinery_drag.is_some() && !pointer_down {
            self.refinery_drop(mouse_p, world, out);
        }
        // F3 settings overlay: swallows clicks inside its panel.
        let (vw, vh) = self.camera.viewport;
        let over_render_settings = self.show_render_settings;
        let over_settings = self.show_settings;
        // Which repair the kill screen / R / B should send — the offered
        // free basic repair, else the cheapest affordable option.
        let repair_type = world
            .kill_screen
            .as_ref()
            .map(|ks| ks.best_repair_type())
            .unwrap_or(crate::net::world::KillScreen::BASIC_REPAIR)
            as i16;
        if world.kill_screen.is_some() {
            // Modal: the only live click is the Repair button; every
            // other press is swallowed so the map can't be steered.
            self.lmb_on_hud = true;
            if lmb_pressed {
                let options = world
                    .kill_screen
                    .as_ref()
                    .map(|ks| ks.options.as_slice())
                    .unwrap_or(&[]);
                let layout = input::kill_screen_layout_n(
                    self.camera.viewport.0,
                    self.camera.viewport.1,
                    options.len(),
                );
                if input::point_in(layout.button, mouse_p)
                    || self.preferences.mobile_hud
                        && matches!(
                            input::mobile_hud_hit(
                                &input::mobile_hud_layout(
                                    self.camera.viewport.0,
                                    self.camera.viewport.1
                                ),
                                mouse_p,
                            ),
                            Some(input::MobileControl::Jump)
                        )
                {
                    out.push(NetCommand::Repair { repair_type });
                } else if let Some(i) = layout
                    .rows
                    .iter()
                    .position(|r| input::point_in(*r, mouse_p))
                {
                    // A specific option row: only usable ones send.
                    let o = &options[i];
                    if o.affordable && o.cooldown_ms <= 0 {
                        out.push(NetCommand::Repair {
                            repair_type: o.repair_type as i16,
                        });
                    }
                }
            }
        } else if over_render_settings {
            self.lmb_on_hud = true;
            if lmb_pressed {
                self.handle_render_setting_click(mouse_p);
            }
        } else if over_settings {
            self.lmb_on_hud = true;
            if lmb_pressed {
                if let Some(flag) = settings_ui::hit(vw, vh, mouse_p) {
                    self.settings_toggles.push(flag);
                }
            }
        } else if mobile_toolbar_toggle {
            self.mobile_toolbar_collapsed = !self.mobile_toolbar_collapsed;
            self.lmb_on_hud = true;
        } else if self.preferences.mobile_hud
            && lmb_pressed
            && input::mobile_hud_hit(&input::mobile_hud_layout(vw, vh), mouse_p).is_some()
        {
            self.lmb_on_hud = true;
            match input::mobile_hud_hit(&input::mobile_hud_layout(vw, vh), mouse_p) {
                Some(input::MobileControl::MoveStick) => self.mobile_joystick_active = true,
                Some(input::MobileControl::Target) => {
                    if let Some(id) = self.next_npc_target(world) {
                        self.selected = Some(id);
                        self.tab_cycle = Some(id);
                        out.push(NetCommand::Select { target_id: id });
                    }
                }
                Some(input::MobileControl::Laser) => self.toggle_laser(world, out),
                Some(input::MobileControl::Rocket) => {
                    if let Some(id) = self.selected {
                        out.push(NetCommand::AttackRocket { target_id: id });
                    }
                }
                Some(input::MobileControl::Jump) => {
                    if let Some(ks) = &world.kill_screen {
                        out.push(NetCommand::Repair {
                            repair_type: ks.best_repair_type() as i16,
                        });
                    } else {
                        out.push(NetCommand::Jump);
                    }
                }
                Some(input::MobileControl::Pet) => out.push(NetCommand::Pet {
                    enabled: world.pets.hero.is_none(),
                }),
                Some(input::MobileControl::Radar) => {
                    self.windows.toggle(crate::ui::WindowId::Minimap);
                }
                None => {}
            }
        } else if self.preferences.mobile_hud && self.mobile_joystick_active && pointer_down {
            self.lmb_on_hud = true;
            let stick = input::mobile_hud_layout(vw, vh).move_stick;
            let joystick_p = mobile_joystick_position.unwrap_or(mouse_p);
            let delta = (joystick_p.0 - stick.0, joystick_p.1 - stick.1);
            let len = delta.0.hypot(delta.1);
            let now = Instant::now();
            if len > stick.2 * 0.12 && self.hold_move_next.is_none_or(|t| now >= t) {
                if let Some(hero) = world.hero_pos() {
                    let strength = (len / (stick.2 * 0.66)).clamp(0.0, 1.0);
                    let distance = 1800.0 * strength;
                    let map_direction = self.camera.screen_direction_to_world(hero, delta);
                    let to = (
                        (hero.0 + map_direction.0 * distance).clamp(0.0, self.bounds.0),
                        (hero.1 + map_direction.1 * distance).clamp(0.0, self.bounds.1),
                    );
                    self.order_held_move(to, out);
                    self.hold_move_next = Some(now + HOLD_MOVE_EVERY);
                }
            }
        } else if self.mobile_joystick_active && !pointer_down {
            self.mobile_joystick_active = false;
            self.hold_move_next = None;
            self.lmb_on_hud = false;
        } else if lmb_pressed && self.ui_press(world, mouse_p, &hud, hud_area, out) {
            // Window / toolbar / flyout consumed the press.
            self.lmb_on_hud = true;
        } else if lmb_pressed {
            self.lmb_on_hud = hud_area.is_some();
            match hud_area {
                Some(input::HudArea::LaserSlot) => self.toggle_laser(world, out),
                Some(input::HudArea::RocketSlot) => {
                    if let Some(id) = self.selected {
                        out.push(NetCommand::AttackRocket { target_id: id });
                    }
                }
                Some(input::HudArea::FlyoutItem(_)) | Some(input::HudArea::Panel) => {}
                None => {
                    let w = self.camera.to_world(mouse_p);
                    let hit = self.hit_test(world, w);
                    let action = input::decide_click(hit.clone(), w, self.bounds, self.selected);
                    match action {
                        ClickAction::MoveTo(x, y) => {
                            self.order_move((x, y), out);
                            self.hold_move_next = Some(Instant::now() + HOLD_MOVE_EVERY);
                        }
                        ClickAction::SelectEntity(id) => {
                            self.selected = Some(id);
                            out.push(NetCommand::Select { target_id: id });
                        }
                        // Clicking the locked target again toggles the
                        // laser off (stock behaviour).
                        ClickAction::AttackEntity(id) => {
                            self.selected = Some(id);
                            self.toggle_laser(world, out);
                        }
                        ClickAction::CollectEntity(hash) => {
                            self.order_collect(&hash, world, out);
                        }
                        ClickAction::JumpPortal => out.push(NetCommand::Jump),
                        ClickAction::Nothing => {}
                    }
                }
            }
        } else if pointer_down {
            // Hold-to-move: LMB held on empty map space keeps re-issuing
            // the order toward the cursor (stock mouse-down flight). A
            // press that started on a HUD panel never steers the ship.
            let now = Instant::now();
            if !self.lmb_on_hud && self.hold_move_next.is_none_or(|t| now >= t) {
                let w = self.camera.to_world(mouse_p);
                let action =
                    input::decide_click(self.hit_test(world, w), w, self.bounds, self.selected);
                match action {
                    ClickAction::MoveTo(x, y) => self.order_held_move((x, y), out),
                    _ => {}
                }
                self.hold_move_next = Some(now + HOLD_MOVE_EVERY);
            }
        } else {
            self.hold_move_next = None;
            self.lmb_on_hud = false;
        }

        let social_typing = self.poll_social_text_input(rl, out);
        if !social_typing {
            // Hotkeys (stock bindings): Ctrl toggles the laser on the locked
            // target, Space fires a rocket, J jumps, Esc deselects.
            if rl.is_key_pressed(KeyboardKey::KEY_LEFT_CONTROL)
                || rl.is_key_pressed(KeyboardKey::KEY_RIGHT_CONTROL)
            {
                self.toggle_laser(world, out);
            }
            let hero_dead = world.kill_screen.is_some()
                || world
                    .world
                    .hero
                    .as_ref()
                    .is_some_and(|h| h.hitpoints <= 0.0 && h.hitpoints_max > 0.0);
            if rl.is_key_pressed(KeyboardKey::KEY_R) {
                if hero_dead {
                    out.push(NetCommand::Repair { repair_type });
                } else if let Some(id) = self.selected {
                    out.push(NetCommand::AttackRocket { target_id: id });
                }
            }
            if rl.is_key_pressed(KeyboardKey::KEY_B) && hero_dead {
                out.push(NetCommand::Repair { repair_type });
            }
            // Tab cycles nearest NPCs (stock KeyboardMapper target cycling).
            if rl.is_key_pressed(KeyboardKey::KEY_TAB) {
                let id = self.next_npc_target(world);
                log::debug!(
                "tab -> npc target {id:?} hero={:?} npcpos={:?} (entities={} npcs={} destroyed={})",
                world.hero_pos().map(|p| (p.0 as i32, p.1 as i32)),
                id.map(|i| world.pos(i)),
                world.world.entities.len(),
                world.world.entities.values().filter(|e| e.npc).count(),
                world.world.entities.values().filter(|e| e.destroyed).count(),
            );
                if let Some(id) = id {
                    self.selected = Some(id);
                    self.tab_cycle = Some(id);
                    out.push(NetCommand::Select { target_id: id });
                }
            }
            if rl.is_key_pressed(KeyboardKey::KEY_ESCAPE) {
                if self.selected.is_some() {
                    out.push(NetCommand::AbortLaser);
                }
                self.selected = None;
            }
            // Digit hotkeys 1..9,0 → standardSlotBar slots 1..10
            // (`ActivateSlotCommand`); suppressed while the kill-screen
            // modal is up.
            if world.kill_screen.is_none() {
                const DIGIT_KEYS: [(KeyboardKey, usize); 10] = [
                    (KeyboardKey::KEY_ONE, 1),
                    (KeyboardKey::KEY_TWO, 2),
                    (KeyboardKey::KEY_THREE, 3),
                    (KeyboardKey::KEY_FOUR, 4),
                    (KeyboardKey::KEY_FIVE, 5),
                    (KeyboardKey::KEY_SIX, 6),
                    (KeyboardKey::KEY_SEVEN, 7),
                    (KeyboardKey::KEY_EIGHT, 8),
                    (KeyboardKey::KEY_NINE, 9),
                    (KeyboardKey::KEY_ZERO, 10),
                ];
                for (key, slot) in DIGIT_KEYS {
                    if rl.is_key_pressed(key) {
                        if self.show_settings {
                            // Overlay open: digits flip its rows instead.
                            if let Some(flag) = crate::net::SettingFlag::ALL.get(slot - 1) {
                                self.settings_toggles.push(*flag);
                            }
                        } else {
                            self.activate_slot(world, slot, out);
                        }
                    }
                }
            }
            if rl.is_key_pressed(KeyboardKey::KEY_SPACE) {
                // Rocket at the locked target (stock Space binding).
                if let Some(id) = self.selected {
                    out.push(NetCommand::AttackRocket { target_id: id });
                }
            }
            if rl.is_key_pressed(KeyboardKey::KEY_J) {
                // Stock sends unconditionally; the server denies out-of-range.
                out.push(NetCommand::Jump);
            }
            if rl.is_key_pressed(KeyboardKey::KEY_P) {
                // Summon / dismiss the P.E.T. (`SetPetRequest`).
                out.push(NetCommand::Pet {
                    enabled: world.pets.hero.is_none(),
                });
            }
            if rl.is_key_pressed(KeyboardKey::KEY_M) {
                self.windows.toggle(crate::ui::WindowId::Minimap);
            }
            if rl.is_key_pressed(KeyboardKey::KEY_L) {
                self.windows.toggle(crate::ui::WindowId::Log);
            }
            if rl.is_key_pressed(KeyboardKey::KEY_F2) {
                self.show_debug = !self.show_debug;
            }
            if rl.is_key_pressed(KeyboardKey::KEY_F3) {
                self.show_settings = !self.show_settings;
            }
            if rl.is_key_pressed(KeyboardKey::KEY_F4) {
                if self.show_render_settings {
                    self.render_settings_draft = None;
                    self.show_render_settings = false;
                } else {
                    self.open_render_settings();
                }
            }
        }
        if rl.is_key_pressed(KeyboardKey::KEY_ESCAPE) {
            self.ui.flyout = None;
            self.ui.lab_dialog = None;
            self.ui.refinery_drag = None;
        }
        // Persist window moves/collapses (no-op when unchanged).
        self.windows.flush();
        self.toolbar.flush();
        self.preferences.flush();
        self.mobile_prev_touches = touch_ids;
    }
    /// Damage-number anchor: above the ship sprite (stock floats the
    /// text over the hit clip), jittered a little per hit so consecutive
    /// numbers in one volley don't print on top of each other.
    fn damage_text_pos(&mut self, pos: (f32, f32)) -> (f32, f32) {
        let rx = self.effects.rand();
        let ry = self.effects.rand();
        (pos.0 + (rx - 0.5) * 52.0, pos.1 - 48.0 - ry * 22.0)
    }

    /// the `LaserSalvo2D`/`Rocket2D`/`DamageReceivedEffect` spawn points.
    fn spawn_world_fx(
        &mut self,
        fx: WorldFx,
        world: &mut SharedWorld,
        assets: &mut AssetManager,
        now: Instant,
    ) {
        let hero_id = world.world.hero.as_ref().map(|h| h.user_id);
        match fx {
            WorldFx::LaserRun {
                attacker,
                victim,
                laser_type,
                skilled,
                shield_damage,
            } => {
                self.laser_types.insert(attacker, laser_type);
                // `displayAllLasers` off: only volleys the hero is part of.
                if !world.settings.display_all_lasers
                    && Some(attacker) != hero_id
                    && Some(victim) != hero_id
                {
                    return;
                }
                let class = laser_class_of(attacker, world, assets);
                let (from, to) = (world.pos(attacker), world.pos(victim));
                let fallback = effects::laser_color_u32(class);
                let defs: Vec<crate::assets::gamexml::LaserDefinition> = assets
                    .game_xml
                    .laser_salvo(class, laser_type)
                    .map(|s| s.salvos.clone())
                    .unwrap_or_default();
                // `LaserSalvo` plays the laser pattern's `soundID` at
                // the shooter position.
                match defs.iter().map(|d| d.sound_id).find(|s| *s > 0) {
                    Some(id) => crate::audio::play_sound(id, Some(from)),
                    None => {
                        crate::audio::play_sfx_at(crate::audio::Sfx::Laser, from.0, from.1)
                    }
                }
                if defs.is_empty() {
                    // No pattern — thin bolt fallback, center to center.
                    let def = crate::assets::gamexml::LaserDefinition::default();
                    if let Some(e) = effects::laser_shot(from, to, &def, None, fallback, 0.0, None)
                    {
                        self.effects.push(e);
                    }
                    return;
                }
                // `LaserSalvo2D`: one bolt per salvo group x laser mount,
                // mounts from the ship's expansion positionsList at the
                // attacker's heading; groups fire 200 ms apart.
                let heading = self
                    .headings
                    .get(&attacker)
                    .copied()
                    .unwrap_or_else(|| (to.1 - from.1).atan2(to.0 - from.0).to_degrees());
                // `Lasers3D`: authored `laserpoint_*` slot table + the
                // `mesh3D` transform chain (only for expansion ships with
                // a parsed 3D recipe).
                let slots3d = attacker_slots3d(
                    attacker,
                    self.entity_visuals
                        .get(&attacker)
                        .and_then(|v| v.display_3d.as_ref()),
                    heading,
                    world,
                    assets,
                );
                // `LaserSalvo3D`: the shielded endpoint is pulled back by
                // the victim's `display3D@visualSize` * 50 (default 1).
                let visual_size = self
                    .entity_visuals
                    .get(&victim)
                    .and_then(|v| v.display_3d.as_ref())
                    .map(|d| d.visual_size)
                    .filter(|s| *s != 0.0)
                    .unwrap_or(1.0);
                let victim_shielded = if Some(victim) == hero_id {
                    world.world.hero.as_ref().is_some_and(|h| h.shield > 0)
                } else {
                    world
                        .world
                        .entities
                        .get(&victim)
                        .and_then(|e| e.shield)
                        .unwrap_or(0)
                        > 0
                };
                let shield_pull = if victim_shielded {
                    visual_size * 50.0
                } else {
                    0.0
                };
                // `getSalvos` mount groups keep their authored names so the
                // 3D path can resolve `laserpoint_{name}` slots.
                let mut groups = attacker_salvo_mounts(attacker, heading, world, assets);
                if groups.is_empty() {
                    // `DEFAULT_SALVOS` — single bolt from the ship center.
                    groups.push(vec![("mainCannon".to_owned(), (0.0, 0.0))]);
                }
                for (g, mounts) in groups.iter().enumerate() {
                    let delay = g as f32 * 0.2;
                    for def in &defs {
                        let res = if skilled
                            && !def.skill_res_key.is_empty()
                            && def.skill_res_key != "-1"
                        {
                            def.skill_res_key.as_str()
                        } else {
                            def.res_key.as_str()
                        };
                        let sprite = if res.is_empty() {
                            None
                        } else {
                            Some(assets.request(res, "mc"))
                        };
                        // Stock pulls the endpoint back by the CLIP's
                        // pixel width when the pattern has no mcWidth/
                        // laserLength (`_loc15_ = _loc11_.width`).
                        let mut d = def.clone();
                        if d.mc_width <= 0 && d.laser_length <= 0 {
                            if let Some(w) = sprite
                                .and_then(|id| assets.get(id))
                                .and_then(|c| c.frames.first())
                                .map(|f| f.image.width())
                            {
                                d.mc_width = w as i32;
                            }
                        }
                        let color = if def.light_color != 0 {
                            def.light_color
                        } else {
                            fallback
                        };
                        // `LaserBullet3D.bulletLength`: `laserLength` or
                        // the clip width (`mc_width` already holds the
                        // clip-width fallback) — pattern scale excluded.
                        let bullet_len = (if d.laser_length > 0 {
                            d.laser_length
                        } else {
                            d.mc_width
                        })
                        .max(0) as f32;
                        // `startAttack` structure: absorbers fire once
                        // center->center (skilled: 0/60/440/500 ms) and
                        // `ignoreLaserPoints` once attacker->victim; only
                        // ordinary defs fan out across the salvo mounts.
                        if (def.absorber || def.ignore_laser_points) && g > 0 {
                            continue;
                        }
                        let shots: Vec<(f32, (String, (f32, f32)))> = if def.absorber {
                            const SKILLED: [f32; 4] = [0.0, 0.06, 0.44, 0.5];
                            let times: &[f32] = if skilled { &SKILLED } else { &[0.0] };
                            times
                                .iter()
                                .map(|t| (*t, (String::new(), (0.0, 0.0))))
                                .collect()
                        } else if def.ignore_laser_points {
                            vec![(0.0, (String::new(), (0.0, 0.0)))]
                        } else {
                            mounts.iter().map(|m| (0.0, m.clone())).collect()
                        };
                        for (extra, (name, m)) in shots {
                            let f = (from.0 + m.0, from.1 + m.1);
                            // The 3D payload mirrors `LaserSalvo3D.shoot`:
                            // `_loc4_` = attacker position + slot offset —
                            // `draw_effects_3d` does the shield pullback,
                            // short-shot drop and absorber swap itself.
                            let vfx = if res.is_empty() {
                                None
                            } else {
                                let slot = if name.is_empty() {
                                    None
                                } else {
                                    slots3d.as_ref().and_then(|s| s.offset(&name))
                                };
                                let spawn = match slot {
                                    // `sceneTransform` slot -> world
                                    // offset (map.y is Away3D `-z`).
                                    Some(o) => (from.0 + o[0], o[1], -from.1 + o[2]),
                                    None => (f.0, 0.0, -f.1),
                                };
                                let muzzle = if slot.is_some() && !def.muzzle_flash.is_empty() {
                                    slots3d.as_ref().map(|s| effects::LaserMuzzle3D {
                                        resource: def.muzzle_flash.clone(),
                                        rot: s.mesh_world_rot(),
                                    })
                                } else {
                                    None
                                };
                                Some(effects::LaserShot3D {
                                    kind: effects::LaserKind3D::of(&def.laser_type),
                                    scale: if def.scale > 0.0 { def.scale } else { 1.0 },
                                    spawn,
                                    end: (to.0, 0.0, -to.1),
                                    shield_pull,
                                    bullet_len,
                                    muzzle,
                                    particle: res.to_owned(),
                                })
                            };
                            if let Some(e) =
                                effects::laser_shot(f, to, &d, sprite, color, delay + extra, vfx)
                            {
                                self.effects.push(e);
                                // `isDiminishedBySkillShield` salvo ->
                                // `showShieldDamageClip` on each landed
                                // bolt, angled back at the attacker.
                                if shield_damage {
                                    let angle = (to.1 - f.1)
                                        .atan2(to.0 - f.0)
                                        .to_degrees()
                                        + 180.0;
                                    self.shield_damage_hit(
                                        assets,
                                        victim,
                                        angle,
                                        delay + extra + d.speed.max(0.0),
                                    );
                                }
                            }
                        }
                    }
                }
            }
            WorldFx::Rocket {
                attacker,
                victim,
                rocket_type,
                smoke_id,
                hit,
                heat_seeking: _,
            } => {
                let class = rocket_class_of(attacker, world, assets);
                let from = world.pos(attacker);
                let to = world.pos(victim);
                let (duration, miss_offset) =
                    effects::rocket_flight(from, to, hit, self.effects.rand());
                let rocket_pat = assets
                    .game_xml
                    .rocket_pattern(class, rocket_type)
                    .or_else(|| assets.game_xml.rocket_pattern(class, 0));
                // `soundID` on the rocket pattern (stock plays it at the
                // shooter).
                match rocket_pat.map(|p| p.sound_id).filter(|s| *s > 0) {
                    Some(id) => crate::audio::play_sound(id, Some(from)),
                    None => {
                        crate::audio::play_sfx_at(crate::audio::Sfx::Rocket, from.0, from.1)
                    }
                }
                let rocket_res = rocket_pat
                    .map(|p| p.res_key.clone())
                    .filter(|r| !r.is_empty());
                let sprite = rocket_res.map(|r| assets.request(&r, "mc"));
                let smoke = assets
                    .game_xml
                    .rocket_smokes
                    .get(&smoke_id)
                    .or_else(|| assets.game_xml.rocket_smokes.values().next())
                    .map(|s| {
                        (
                            s.res_key.clone(),
                            s.particle_interval as f32,
                            s.particle_duration,
                        )
                    });
                let smoke_sprite = smoke
                    .as_ref()
                    .filter(|(r, ..)| !r.is_empty())
                    .map(|(r, ..)| assets.request(r, "mc"));
                let (interval, life) = smoke
                    .map(|(_, i, d)| {
                        (
                            i,
                            if d > 0.0 {
                                d
                            } else {
                                effects::SMOKE_LIFE_DEFAULT
                            },
                        )
                    })
                    .unwrap_or((0.0, effects::SMOKE_LIFE_DEFAULT));
                // Hit -> rocketDamage (pyro class 3); miss -> laserDamage
                // (class 2), per Rocket2D::handleTweenComplete.
                let impact = assets
                    .game_xml
                    .pyro(if hit { 3 } else { 2 }, 1)
                    .or_else(|| assets.game_xml.pyro(if hit { 3 } else { 2 }, 0))
                    .map(|p| (p.res_key.clone(), p.scale));
                let impact_sprite = impact
                    .as_ref()
                    .filter(|(r, _)| !r.is_empty())
                    .map(|(r, _)| assets.request(r, "mc"));
                self.effects.push(Effect::Rocket {
                    from,
                    victim,
                    to,
                    miss_offset,
                    duration,
                    hit,
                    hero_involved: Some(attacker) == hero_id || Some(victim) == hero_id,
                    sprite,
                    smoke_sprite,
                    smoke_interval: interval,
                    smoke_life: life,
                    last_smoke: (i32::MIN, i32::MIN),
                    finished: false,
                    impact_sprite,
                    impact_scale: impact.map(|(_, s)| s).unwrap_or(1.0),
                    started: now,
                });
            }
            WorldFx::Hit {
                victim,
                attacker,
                damage,
                attack_type,
                strike,
            } => {
                let on_hero = Some(victim) == hero_id;
                // Laser impact clip at the victim (`laserDamage<N>`,
                // pyro class 2 — N follows the attacker's ammo type).
                const ATTACK_TYPE_LASER: i32 = 1;
                if attack_type == ATTACK_TYPE_LASER
                    && (world.settings.display_all_lasers || on_hero || Some(attacker) == hero_id)
                {
                    let id = self
                        .laser_types
                        .get(&attacker)
                        .copied()
                        .unwrap_or(0)
                        .clamp(0, 2);
                    let at = world.pos(victim);
                    if let Some(e) = self.pyro_burst(assets, 2, &[id, 0], at, 0.5, false, now) {
                        self.effects.push(e);
                    }
                    crate::audio::play_sfx_at(crate::audio::Sfx::Impact, at.0, at.1);
                }
                if !world.settings.display_hitpoint_bubbles {
                    return;
                }
                let pos = self.damage_text_pos(world.pos(victim));
                if let Some(e) =
                    effects::hit_text(pos, damage, attack_type, on_hero, strike, &assets.locale)
                {
                    self.effects.push(e);
                }
            }
            WorldFx::Miss { victim } => {
                if !world.settings.display_hitpoint_bubbles {
                    return;
                }
                // `DamageReceivedEffect(owner, 0)` over the victim.
                let pos = self.damage_text_pos(world.pos(victim));
                self.effects.push(Effect::Text {
                    pos,
                    text: "MISS".to_owned(),
                    color: effects::COLOR_MISS,
                    started: now,
                });
            }
            WorldFx::SabShot { attacker, victim } => {
                if !world.settings.display_all_lasers
                    && Some(attacker) != hero_id
                    && Some(victim) != hero_id
                {
                    return;
                }
                // `ChasingShotEffect("sab_shot")` — a violet bolt that
                // homes on the victim; no bundled clip, so a bolt line.
                self.effects.push(Effect::LaserShot {
                    from: world.pos(attacker),
                    to: world.pos(victim),
                    sprite: None,
                    duration: 0.45,
                    delay: 0.0,
                    absorber: false,
                    play_loop: false,
                    color: 0xB060FF,
                    vfx: None,
                    started: now,
                });
            }
            WorldFx::HealRay { healer, target } => {
                self.effects.push(Effect::Beam {
                    from: effects::Anchor::Entity(healer),
                    to: effects::Anchor::Entity(target),
                    kind: effects::BeamKind::Heal,
                    life: effects::HEAL_BEAM_LIFE,
                    started: now,
                });
            }
            WorldFx::MineExplode { pos } => {
                // Mine pyro (`pyroEffects[1][0]`) carries its `soundID`.
                match self.pyro_sound_id(assets, 1, &[0]) {
                    s if s > 0 => crate::audio::play_sound(s, Some(pos)),
                    _ => crate::audio::play_sfx_at(crate::audio::Sfx::Mine, pos.0, pos.1),
                }
                // `pyroEffects[1][0]` (mine explosion).
                if let Some(e) = self.pyro_burst(assets, 1, &[0], pos, 1.2, true, now) {
                    self.effects.push(e);
                }
            }
            WorldFx::FireworksIgnite { pos } => {
                // The firework's own show is the box clip's `fire()`; a
                // golden flash marks the spot.
                self.effects.push(Effect::Pulse {
                    at: effects::Anchor::Pos(pos),
                    kind: effects::PulseKind::MegaMine,
                    started: now,
                });
            }
            WorldFx::Smartbomb { owner } => {
                let pos = world.pos(owner);
                crate::audio::play_sfx_at(crate::audio::Sfx::Smartbomb, pos.0, pos.1);
                // `getPyroPattern(TYPE_SMARTBOMB_EXPLOSION, 0)`.
                match self.pyro_burst(assets, 4, &[0], pos, 1.2, false, now) {
                    Some(e) => self.effects.push(e),
                    None => self.effects.push(Effect::Pulse {
                        at: effects::Anchor::Entity(owner),
                        kind: effects::PulseKind::Smartbomb,
                        started: now,
                    }),
                }
            }
            WorldFx::InstaShield { owner } => {
                // `InstaShieldEffect` — the `instaShield` clip (the
                // procedural `Pulse` stand-in is retired).
                self.ship_fx.add_clip(
                    owner,
                    world.pos(owner),
                    ship_fx::modifiers::fx::INSTA_SHIELD,
                    ship_fx::modifiers::Life::Once,
                );
            }
            WorldFx::VisualModifier { entity, row } => {
                self.apply_visual_modifier(world, assets, entity, &row, now);
            }
            WorldFx::ShipFx {
                entity,
                effect,
                activated,
            } => {
                self.ship_fx
                    .toggle(entity, world.pos(entity), effect, activated);
            }
            WorldFx::RepairBot {
                active,
                loot_id,
                skilled,
            } => {
                if let Some(h) = &world.world.hero {
                    let uid = h.user_id;
                    if active && !loot_id.is_empty() {
                        self.ship_fx
                            .add_repair_bot(uid, world.pos(uid), &loot_id, skilled);
                    } else {
                        self.ship_fx.remove(uid, ship_fx::modifiers::fx::REP_BOT_EFFECT);
                    }
                }
            }
            WorldFx::Emp { target, kind } => {
                self.spawn_emp(world, assets, target, kind, now);
            }
            WorldFx::Emote { entity, loot_id } => {
                // `EmoteEffect2D` — `emotes` SWF clip named by `lootId`.
                let sprite = Some(assets.request("emotes", &loot_id));
                self.effects.push(Effect::Emote {
                    entity,
                    sprite,
                    started: now,
                });
            }
            WorldFx::MegaMine { target } => {
                self.effects.push(Effect::Pulse {
                    at: effects::Anchor::Entity(target),
                    kind: effects::PulseKind::MegaMine,
                    started: now,
                });
            }
            WorldFx::Spawn { pos, despawn } => {
                crate::audio::play_sfx_at(crate::audio::Sfx::Spawn, pos.0, pos.1);
                self.effects.push(Effect::Pulse {
                    at: effects::Anchor::Pos(pos),
                    kind: if despawn {
                        effects::PulseKind::SpawnOut
                    } else {
                        effects::PulseKind::SpawnIn
                    },
                    started: now,
                });
            }
            WorldFx::SpecialExplosion { ship } => {
                self.special_explosions.insert(ship);
            }
        }
    }

    /// Resolve `pyroEffects[class][id]` (first id that exists) into a
    /// one-shot clip burst at `pos`. `None` when game.xml has no such
    /// pattern; `ring_fallback` keeps the placeholder ring while the
    /// clip streams in.
    #[allow(clippy::too_many_arguments)]
    fn pyro_burst(
        &mut self,
        assets: &mut AssetManager,
        class: i32,
        ids: &[i32],
        pos: (f32, f32),
        life: f32,
        ring_fallback: bool,
        now: Instant,
    ) -> Option<Effect> {
        let (res, scale, rotatable) = ids
            .iter()
            .find_map(|id| assets.game_xml.pyro(class, *id))
            .map(|p| (p.res_key.clone(), p.scale, p.rotatable))?;
        let sprite = (!res.is_empty()).then(|| assets.request(&res, "mc"));
        let rotation = if rotatable {
            self.effects.rand() * 360.0
        } else {
            0.0
        };
        Some(Effect::SpriteBurst {
            pos,
            sprite,
            life,
            scale,
            fade: false,
            rotation,
            ring_fallback,
            started: now,
        })
    }
}
