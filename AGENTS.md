# xenomit-raylib-client notes

Native DarkOrbit client in Rust + raylib — a from-scratch reimplementation
of the Flash client (`../game_resources/main.swf`, decompiled to
`../game_resources/scripts/`, 7.4k AS3 files). Goal: log into the real
server and play with the real assets, no Flash runtime.

## Big picture

- Wire protocol: `xenomit-packet-client` (sibling crate, path dep) — the
  whole frame/schema/handshake stack is done and capture-verified. We only
  add `type_str`/`beaconId`/`OreTypeModule` extraction on collectables.
- Auth/bootstrap: `xenomit_backpage` (sibling crate) — `dosid` cookie →
  `launch::load` → authenticated flashvars (`userID`, `sessionID`,
  `factionID`, `pid`→instanceId, `mapID`, `cdn`, `basePath`, `dynamicHost`,
  `resourcesXmlHash`, `gameXmlHash`, `platform`).
- Endpoint resolution: `{dynamicHost}{basePath}/xml/maps.php` →
  `<map id="N"><gameserverIP>` (default port 5001) — same as
  `xenomit-bot/src/bot/packet_client.rs`.
- Asset index: `{cdn}{basePath}/xml/resources.xml?__cv={hash}` →
  `<location id=… path=…>` + `<file id location name type version hash>`.
  `FileCollection`/`FileVO`/`FileCollectionFinisher` semantics.
- Config embedded IN main.swf (`DefineBinaryData`, extracted via
  `swf_extract::SwfLibrary::binary_blobs()`; dumps in `tests/fixtures/`):
  - `…InitPatternManagerCommand_GAME_XML_FILE` → game.xml
    (ship/expansion/engine/laser/background/ore patterns; contains a
    malformed comment ~line 11615 — strip `<!--…-->` before strict parse)
  - `…UberAssetsLib_LIB_{SHIPS,DRONES,PORTALS,COLLECTABLES,MAPS,DEFAULT,
    BATTLESTATION}_XML` → assetId → `<display2D>` recipes with
    `template`/`reference`/`replace`/`append` resolution
    (`XMLUtils.resolveReferences`).
- Static map descriptors: resources.xml file id `maps-config` (XML) →
  `<map id scaleFactor><backgrounds/><planets/>`.
- Runtime sprites: `(srcKey = file id in resources.xml) → SWF download →
  `resKey` symbol → rasterized frames. `resKey` defaults to `"mc"`;
  `useBitmapClip="true"` on `display2D` means rasterize. Ship clips:
  MovieClip w/ 32 frames = rotation strip, frame
  `int((32*deg/360 + 0.5) % 32 + 1) % totalFrames` (ShipClip.as:130);
  single-frame rotatable clips rotate natively; loop clips play ~30fps.
- Portals: `display2D>mapAssetView` → `textureAtlas` entries —
  `{atlasKey,textureKey}` pairs are resources.xml files (atlas XML + PNG),
  sub-assets reference `atlas`+`texture` region names.

## Layout

- `src/swf_extract/` — SWF→RGBA rasterizer (`swf` crate parse +
  tiny-skia composite); `src/bin/extract.rs` = `do-extract` CLI.
- `src/assets/` — FlashVars→BootConfig, FileIndex, GameXml, UberAssets,
  MapDb, Collection (CDN cache), SpriteStore, AssetManager (lazy
  `(src,res)→SpriteClip` pipeline).
- `src/net/` — Connection (worker thread owns `Session`), NetCommand,
  SharedWorld (WorldState + movement tweens + log).
- `src/game/` — Game (camera, selection, orders, effects), input mapping.
- `src/render/` — TextureCache (GPU frames), map backdrop, HUD/minimap.
- `src/app/` — App state machine SignIn→Loading→Playing→Failed.
- `src/ui/` — floating HUD windows: `window`/`manager` (drag, collapse,
  close, z-order, layout persisted to `{assets_dir}/window_layout.json`),
  `chrome` (window skin: default `Stock` = the real `window` SWF
  composite — `pattern` texture masked `(4,0,w-8,h-4)` under the
  scale9 `window` child of `backgroundWithTopControls` /
  `backgroundNormal`, authored `DefineScalingGrid` (41,36,~105,69),
  real `closeBtn`/`minimizeBtn` `background`/`rollOverState` art;
  `ModernBlue` = flat `questWin` scale9, `Legacy` = `main.swf`
  `DOWindowSkin` export; procedural bevel fallback. `>` child paths
  resolve via `SwfLibrary::resolve_path`, scale9 via
  `scaling_grid()` — both cached in `SpriteClip::scale9`, sprite
  cache v5). TITLE_H=25 (`UICONTAINER_Y`); window `<simpleContainer>`
  coords are window-relative, so body y = xml y − 25,
  `toolbar` (top-left icon toggles, `featuresMenu_atlasXml` regions
  `<feature>_{normal,hover,select}` with glyph fallback), `panels`
  (pure body layouts shared by draw + input). Bodies are painted in
  `render/hud/{ship,user,minimap,quests,pet,log,prefs,refinery}.rs`;
  clicks route in `game/ui_input.rs` (flyout → toolbar → windows →
  static HUD → world). Static HUD (action bar, ammo flyout via
  right-click, target panel) lives in `game/input.rs`
  (`hud_layout`/`hud_hit`). Quests: `net/quests.rs`; C1/C2 + ore breakdown
  + boosters: `net/attributes.rs`; refinery/lab: `net/refinery.rs`
  (`LabUpdateItemCommand` 135 inbound; `LabRefinement` 132 /
  `LabUpdateItem` 133 / `LabUpdate` 137 outbound via `NetCommand`,
  ~1 Hz refresh while open), layout parsed from game.xml
  `<refinementWindow>`, bitmaps = `refinement` SWF symbols
  (`sourceSlot`/`targetSlot`/`targetIcon`/`labelSlot`/`ore_*`/`icon_*`/
  `item_*`) + `ore_tree` from `ui`.
- `src/bin/client.rs` — CLI: `xenomit-client --server <srv> --dosid <sid>
  [--main-swf <path>] [--assets-dir <dir>] [--width px] [--height px] [-v]`.
  Both credentials given → skips sign-in; otherwise a form collects them.
  `--main-swf` default search: `../game_resources/main.swf`,
  `tests/fixtures/main.swf`, `main.swf`.

## Conventions / gotchas

- `cargo check` uses MSVC; libclang needed for `raylib-sys` bindgen:
  `LIBCLANG_PATH=C:\Users\Yagiz\AppData\Roaming\Python\Python313\site-packages\clang\native`
  (already set system-wide or pass per-build).
- Keep-alive: `modules::keep_alive(false)` every ~20s on the pump thread.
- Coordinates: world units, 21000×13100×scaleFactor, y grows downward,
  heading deg = atan2(dy,dx) (Flash convention: 0 = east, CW positive).
- Collectable asset ids (uberlib `<asset id=…>` ids are comma-separated
  lists — split on `,` when indexing):
  `Box` → `"box_"+type_str` (LIB_COLLECTABLES, e.g. `box_BONUS_BOX`);
  `Ore` → `"ore_"+type_id` (LIB_COLLECTABLES);
  `Beacon` → `"beacon_"+type_id` (LIB_COLLECTABLES);
  `Mine` → `"mine_"+type_id` (LIB_DEFAULT);
  `FireworkBox` → fixed `"firework_box"` (LIB_COLLECTABLES).
- Portal asset ids are bare numeric strings (`"1"`,`"2"`,…) in LIB_PORTALS,
  keyed by portal `typeId`.
- Map background parallax = `1/pFactor` of camera scroll.
- `ReadyRequest` uses view `"2D"` + `MAP_LOADED` + `UI_READY`, then `w`.
- Portal jump = reconnect path (observed live, `target/jump*.log`):
  `JumpRequest(10)` when `BeaconCommand.nearJumpGate` →
  `JumpInitiatedCommand(19) {mapId, gateId}` → teardown stream
  (`ShipRemove`/`ShipDestroyed`/`AssetRemove`) → `JumpToServerCommand(20)`
  → server closes the socket. Then re-run `launch::load`, resolve the new
  map's endpoint and re-handshake (assets stay cached). `ReloginCommand`
  (-30607) may also appear — the stock client turns it into
  `LegacyModule("0|i|<mapID>")`; our full re-bootstrap is equivalent.
  Same path handles unexpected drops: first retry immediate, then waits
  of 5/15/30s and every 60s after that within a 360s budget (the server
  ghosts a dead session for ~5 min), with an attempt-count/countdown
  banner; Esc aborts an unexpected drop.
- Login `status 7` = `STATUS_ShipIsDestroyed`: server keeps the socket,
  sends no KillScreenPostCommand on a *fresh* login (kill-screen options
  only flow for in-game deaths). `examples/repair.rs` sends
  `KillScreenRepairRequest(255)` (`selection`=KillScreenOptionTypeModule
  `repairTypeValue=1` BASIC_REPAIR + `requestModule`=fresh LoginRequest);
  server replies `ReloginCommand` and the next plain login succeeds.
  In-game the `R`/`B` keys send the same packet via `NetCommand::Repair`.
- Item slot bar: `MenuInitializationCommand(-8140)` seeds the session
  menu — `categories` → `MenuItemInitializationCommand(23633)` rows
  (`actionStyle` 0=oneShot/1=toggle/2=selection/3=timer + nested status
  24049 — the item id is only `status.menuItemId`) and `slotBars` → `SlotBarInitializationCommand(-620)` →
  `SlotInfoCommand(-20408)` (`slotId` 1-based, 1..=10; a slot whose
  `menuItemId` isn't a category item is EMPTY). Standalone
  `MenuItemStatusCommand(24049)` updates an item in place. The stock
  `itemsControlMenu` widget (toolbar + category row + drag-items row +
  `premiumSlotBar`/`proActionBar`) is NOT drawn — only the state is kept:
  digit keys 1..9,0 → `standardSlotBar` slots 1..10 still send
  `MenuActionRequest(8766)` (`actionType`: selection/oneShot→ACTIVATE=1,
  toggle/timer→SELECT=0; `sourceType`=SLOT_BAR=1). `SharedWorld::menu`
  is session scope — `clear_map_state` keeps it.
- Slot/ammo icons: resources.xml `itemsControlMenu_atlasXml` +
  `itemsControlMenu_texture` (graphics/ui/, png), 38px regions named by
  item id / `iconLootId` (`ammunition_laser_lcb-10`) → `request_region`.
- Uber `scale`/`scaleX`/`scaleY` on `display2D`/`displayObject` scale the
  content (`UberAsset2D.applyTransformations`); `x`/`y` stay parent-space.
  Ship recipes (`Ship2D`) ignore `display2D@scale`. `center` shifts by
  frame 0's half size once (not per frame).
- Boxes/ores/explosions (`box1`, `box2`, `oreYellow`, `explosion0`…) are
  `DefineVideoStream` VP6A clips; `PlaceObject.ratio` = video frame
  (`swf_extract/video.rs`, nihav pinned like Ruffle).
- Ship rotation strips are perspective renders: frame sizes vary a lot by
  heading (aegis frame 8 = 70x75 vs frame 24 = 74x153) — that's the art.

- Drones: legacy `0|n|d|uid|[formation]|(type|level|stats|visual)*` →
  `SharedWorld::drones` (`DroneLoadout`). Remote ships' loadouts arrive
  wrapped in `MapEventDroneInfoCommand` (module 180,
  `legacyDroneCommandString` → same parser; `simple` flag ignored).
  Formation tables come from
  game.xml `<drones><formations>` (`assets/formations.rs`, offsets
  east-facing, rotated by owner heading, `userScale` 1.0, 0.5s quad
  ease-out tween in `game/drones.rs`; ring fallback). Sprites via
  `AssetManager::drone_display` (`{type}_{asset}_{level}` chain in
  lib_drones). Drawn right after the owner's ship (`render/drones.rs`).
  3D drones (`scene3d::draw_drone_models`) use the `<display3D>`
  formation tables + `<rotationGroup>` tweens (`target3d`): eased
  formation yaw + eased slot chase (0.3s), tilt lives in orbit
  positions — drone hulls only yaw.
- Pets are ship-like `Entity` rows mirrored from `net/pet.rs`
  (`world.pets`); idle pets hidden, plate `P.E.T {name} [{level}]`,
  `P` key sends `SetPetRequest`. Swarm minions render on the mother's
  `dock_N` pivot (`game/ships.rs`, unrotated like MotherShipTrait).
- Multi-`displayObject` recipes: `Display2D.layers` → `Visual.layers`.
- Localization (`src/locale/`): NOT in resources.xml. `InitBPLocaleCommand`
  layout: manifest `{cdn}spacemap/templates/language_{lang}.xml?__cv=
  {languageXmlHash}` (`<x name hash>`), then
  `{cdn}spacemap/templates/{lang}/{stem}.xml?__cv={hash}` for `flashres`→
  category `""`, `resource_quest`→`quests`, `resource_items`→`items`, … (table
  in `locale/source.rs`). Keys: `category.name` split at the last `.`.
  Disk cache `{assets_dir}/lang/{lang}/{stem}.{hash}.xml`; optional override
  `{assets_dir}/lang/{lang}.xml` (BPLocale shape) wins over the CDN.
  `AssetManager::locale` (Arc) is mirrored into `SharedWorld::locale`
  (`set_locale`, also after reconnects); log lines/titles/damage/HUD numbers
  resolve through it, reward lines also feed `SharedWorld::toasts`.

## Audio

- `src/audio/mod.rs` — thread-safe request queue: `play_sound(id, at)` /
  `play_sfx` / `play_sfx_at` push `SoundRequest`s the render thread drains
  (`Sfx` variants map to game.xml `<sounds>` ids). `AudioSettings`
  (persisted in prefs) carries the stock `Settings.as` defaults.
- `src/audio/device.rs` — render-thread raylib owner. `init_audio_device`
  fails once the device is ready, so the token + decoded buffers +
  `Sound`/`SoundAlias` caches + music stream + resume positions live in a
  `thread_local` `SHARED`; `AudioDevice`'s `Session` (map-music machine,
  pending plays, map id) resets per `PlayState`. SFX: `SoundAlias` pools
  cap ~10 concurrent, `interval` cooldown, 800-unit radius with linear
  falloff + `(x-hero.x)/500` pan (raylib 0..1).
- `src/audio/loader.rs` — decode worker sharing the sprite `file_source`;
  `swf_extract/sound.rs` `DefineSound` → MP3 passthrough (strip
  `SeekSamples`), PCM→WAV, Flash ADPCM→WAV. All cached audio SWFs are
  44.1 kHz mono MP3.
- `src/audio/music.rs` — pure `MapMusic` = `MapAudioManager`: maps-config
  `music="id[,delay,id…]"`, `loopLastTrack`, `disableCombatMusic`; combat
  track `1000001` while `prefs.play_combat_music &&
  world.combat_music_active` (`CombatMusicCommand` -12650); fades +
  per-resKey resume positions.
- TODO: ambient/engine loops (ship `soundIDs`/`loopPlay`, engine
  `soundID loop`) — patterns parsed, no channel tracking yet.

## W2 asset families

- Minimap backgrounds (`graphics_minimaps`): maps-config `<map>@minimap` →
  `MapDescriptor.minimap`; `game::visuals::minimap_bg_key` chains
  `minimap-{attr}` → `minimap-{attr}-{300,700}` → `minimap-{id}-{300,700}`
  → `minimap-{id}` (300 px when the map draw width <= 400) against the
  FileIndex; drawn under the minimap content, scaled to its width, 0.3 s
  fade-in on first load, re-resolved on map change. The
  `display_minimap_background` setting gates the art only — the solid
  backdrop stays (stock: 0x090909 fill).
- Ship pods (`graphics_items`): `<ship podType>` +
  `<resources><podOverlay factionId srcKey resKey>` in `assets/pods.rs`
  (a design's own rows replace the cloned base list); the `Pod2D`
  equivalent draws `(srcKey,"mc")` over hull + drones while the ship has
  `PodCarrierTrait` — factory-added for `aegis`/`citadel` pattern
  classes, else `ShipInfo.modifiers` must contain
  `VISUALMODIFIER_HEALING_POD` (7). Low-quality
  `replacementShips`/`ship49_pod` branch skipped (no ship-quality
  setting); no 3D pod — `PodDecorator3D` toggles a `pod` node baked into
  the merged ship OBJ with no per-instance visibility hook.
- Mine pulse (`graphics_mines`): `<minePulseColors><minePulseColor id
  resKey color alpha scale resKey3D>` → `assets/minepulse.rs`;
  `AddMineCommand.maxSkill1` → `Collectable.pulse_color_id` (0/1 per
  `AddMineHandler`). 2D: looping `minePulse` clip over the mine at pattern
  scale, RGB tint + alpha (ColorTransform ≈ multiply on the
  white-authored art). 3D: `resKey3D` (`glow_fast_flickering_*.zip`) is
  appended to the mine's `display3D` particles — only when the mine
  already has a 3D recipe. `maxSkill5b` (shockwave colour) not carried —
  no shockwave visual exists.

## Ship FX (visual modifiers)

- `VisualModifierCommand` rows (live + activated initial rows on
  ship/asset create) → `WorldFx::VisualModifier` (`net/ships.rs` keeps
  the `modifiers` id set); legacy `0|n|fx|start/end|NAME` auras and
  `0|A|LUP` → `WorldFx::ShipFx`; `InstaShield` → clip 84. All drained
  into `Game::ship_fx` (`game/ship_fx/`), the `EffectManager` +
  `EffectsManager2D` equivalent: persistent/timed dedupe + reactivate,
  one-shots stack, per-entity cleanup on remove/map clear.
- `game/ship_fx/modifiers.rs` = the `VisualModifierAssembly` switch
  (103 ported cases → `FxAction::Add/Remove/Pyro`); the `// not ported`
  comment at the bottom lists the skipped ids (beams, camera shake,
  HUD/countdown UI, traits/model-only, `no 2D view`).
- `game/ship_fx/catalog.rs` = per-effect-id view descriptors distilled
  from the `*Effect2D` classes: `Res` (pattern `resKey`/`Fixed`/
  `PatternOr`) + symbol, `Layer::{BelowHull,AboveHull,Fx}` (the
  `AbstractMapObjectEffect2D` param4/param5 flags), `Anim::{Loop,Once}`,
  `Rot`, `Kind` (Clip/Glow/Ring/Shockring/StickyBomb/SphereShield/
  RepairRobots/Turbo/HullAlpha/HullFitClip/ChainLink).
- `render/ship_fx.rs` draws Below/Above inside `entities.rs` around the
  hull + `draw_fx_layer` after entities; `hull_alpha` fades the ship
  tint, `shake` jitters the hull position. 3D-rendered ships keep the
  2D sprite fx (no 3D effect assets).
- Chain bolts (`ChainBoltEffect2D`): modifiers 74/83/86/111/140 add a
  `Kind::ChainLink` fx whose `a` is the chained target id; `catalog.rs`
  `Bolt{steps,thickness,speed,amplitude,color}` carries the per-link
  `Lightning` params, drawn as a jittered polyline in `draw_fx_layer`
  (0.5 s fade-in, non-persistent links fade out at `boltFadeTime` 3 s).

## Combat/world FX parity

- Lens flares: `game/lensflares.rs` (state/maths) +
  `render/lensflares.rs`, drawn above world fx under the HUD. 2D rows
  from maps-config `<lensflares>` (`MapDescriptor.lensflares`), 3D rows
  from authored `<lensFlare>` scene items (win in the perspective view);
  `lensflare{typeID}` `lens0..N` children at
  `pos + i * -(pos - center) * 3 / N`, rotating `star`, once-per-show
  `lensFlash` (0.2 s → 0.75, 3 s out), `hideLensflare` honoured.
  Occlusion ≈ drawn-hull radii, not stock hit tests.
- Repair robot: `BeaconCommand.{repairBotActive,repairRobotLootId,
  repairBotSkilled}` → `WorldFx::RepairBot` → hero `RepairRobots` ship
  fx; `<repairRobots>` row by loot id (`_default` fallback), `<sparks>`
  by skill level.
- Shield damage: `AttackLaserRun.isDiminishedBySkillShield` → per-bolt
  `("shieldDamage","mc")` on the victim at `clickRadius·cos/sin(angle)`
  (`angle = atan2 + 180`), scale `clickRadius/100`, once at 30 fps,
  cap 9/target.
- EMP: `Effect::Emp` plays `lensFlash` + `blitz`/`blitzUltimate` +
  `shockring0*` clips from `shockwaves` — rings 5/1/4/7/5
  (normal/ultimate/emperor/nuke/burning), alpha 0.3/0.7/0.3/0.5/0.5,
  reach 455/430/250 (emperor fixed scale 9, stagger 0.03 s, first two
  rings `shockring0Red`). `EmpKind::Ultimate` ← module 275
  (`AbilityEffectActivation`, JAMX 110/146); `EmpKind::Emperor` ←
  `POLARITY_POWERUP` one-shot expiry (`ship_fx.pending_emps`).
- Emotes: `EmoteActivationCommand` (4739) → `WorldFx::Emote` →
  `emotes` clip by `lootId`, 5 s, 0.2 s pop-in/squash-out above the
  ship.
- Group ship icons: `GroupMember.ship_icon` (netty `shipIcon` module)
  → `net::group::ship_icon_res` = `nettyShipIcons` table →
  `groupSystemShipIcons` bitmap next to member markers on the map.

## 3D scene conventions (Away3D → raylib)

- Away3D is **left-handed** (Y up, +Z into screen, CW front faces); raylib is
  right-handed. Map `(x, y)` is Away `(x, 0, -y)` (`MapEntity.x3D/y3D/z3D`) and
  raylib `(x, 0, y)`: use `scene3d::rl_point` (z flip) for every world point and
  `scene3d::euler_axis_angle` for authored Euler rotations (Rz kept, Rx/Ry
  negated). `AwdMesh::to_obj` bakes the same z flip + reversed winding.
- Camera (`Observer3D`/`CameraManager3D`): pan 0, tilt 135°, distance
  `1740/zoom`, fov 30°, near 10, far 80000; `game/camera.rs` perspective math is
  unmirrored (+x map = screen right) and tested against a raylib camera.
- AWD meshes: render every top-level (`parent == 0`) mesh instance with its own
  matrix (row-vector 3x4), except `engine_`/`laserpoint_`/`light_laser`/
  `light_position`/`rocketlauncher`/`dockingPoint_` helpers (`ShipView3D`).
- `UberPlane3D` quads are XY planes facing the camera (`yUp=false`); the shared
  plane model is stood up with a +90° X transform.
- raylib's OBJ loader dereferences `vt`/`vn` unconditionally — always write both.
- OBJ cache files are `*-v3.obj`; bump the suffix when `to_obj` output changes.
- Materials: every `Mesh3D`/`SceneMesh3D` carries a `MaterialDesc`
  (`src/assets/material.rs`) parsed like `PrefabMaterialManager` — per-slot
  texture bases (`texture`/`diffuse`/`alpha`/`gal`/`normal`/`specular`/`ao`/
  `glow` attrs, `"none"` suppresses), `tex_settings` size column, and scalar
  params. `texture_key(slot, geometry, quality, size_mult)` builds
  `{base}_{slot}_{size}` manifest ids; `supported_slots(quality)` mirrors
  `EntityBasicMaterial` (Low: diffuse/alpha/gal, +glow at Medium,
  +normal/specular/ao at High). Request layer ids only when
  `AssetManager::geometry_file_exists(id)` (index must be loaded first) —
  use `game::visuals::request_mesh_resources`. Models are cached per
  `materials3d::model_key` (geometry + material fingerprint at the
  `render.texture_quality` preference) and built only once every resolved
  layer uploaded or definitively failed. The lit/unlit shared shaders,
  layer→map-slot binding and per-frame sun/hero lighting live in
  `src/render/materials3d.rs`; its header lists the known deviations
  (UV1-only lightmaps, linear falloff, no per-material blend/cull).

## Particles (`resources_3d_particles`)

- Effects are Away3D particle-editor JSON (`<id>.zip` holding one `.awp`) in the
  separate particle manifest; textures (`.atf`) and external shapes (`.awd`)
  are in the same manifest. `assets/particles.rs` parses and evaluates them on
  the CPU (Time/Scale/Velocity/Billboard/SegmentedColor/Orbit/UV/SpriteSheet/…
  nodes; Follow, Bezier and primitive shapes are ignored).
- The particle manifest is keyed `id.type` (`FileCollection`
  `_useFilesExtensions=true`) — `box_2.zip`/`box_2.atf`/`box_2.awd` are three
  different files under one stem, so `FileIndex` keeps a `files_typed` map and
  every particle request uses typed ids (`.png→.atf`, `.obj→.awd` remapped per
  `getRemappedUrl`). Bare-id lookups still hit `files` for `resources_3d`.
- Recipes carry `<particles url="x.zip" x y z scale…>` children
  (`Display3D::particles`); boxes (`box_N.zip`), NPC/ship trails and station
  effects are all particles. `scene3d` queues them per owner, `particles3d::flush`
  draws them (additive/alpha, no depth writes) after the 3D mesh passes.
- `cargo run --example particles -- <dir of zip/atf/awd> out.png box_2 …` renders
  groups offline; `XENOMIT_PARTICLE_DIR=<dir> cargo test real_particle_corpus`
  parses every real file.
- Ship 3D rotation is eased like `Ship3D.updateRotation` (quad ease-out, 0.2 s,
  20° bank); the 2D sprite keeps snapping to the heading.
- Asset entities (stations, relays, turrets, mines) with a `display3D` mesh are
  drawn in `scene3d::draw_ship_models` with yaw 0.
