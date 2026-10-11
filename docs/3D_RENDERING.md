# DarkOrbit 3D rendering

The Render Modes preferences are client-local and independent for camera,
collectables, NPCs, ships, drones, backgrounds, portals, effects, and
explosions. Each component uses its authored `display3D` recipe when that
recipe and its assets are ready; the renderer retains the 2D path as a
per-object fallback. The options start disabled so the existing 2D client
keeps its original startup behavior.

The implementation follows the ActionScript client rather than synthesizing
replacement ship geometry:

- `MapView3D.as` creates a 30 degree perspective lens. `Observer3D.as` supplies
  the orbit camera position, and `CameraManager3D.as` maps `(x,y)` to
  `(x,0,-y)`, uses Y up, near 10, and far 80000. The raylib camera and 2D
  projection/picking conversion share those conventions.
- `World3D.as` loads `resources_3d.xml` and its particle collection separately
  from the 2D resource manifest. `GeometryCollection` therefore fetches 3D
  geometry and textures lazily in its own namespace.
- Uber recipes come from `lib_ships.xml`, `lib_portals.xml`, other Uber
  libraries, and map-scene recipes in `lib_maps.xml`. Parent and child mesh
  transforms compose with the same Z*Y*X quaternion convention used by the
  map-scene parser.
- `PrefabMaterialManager.as` resolves each material layer to
  `<texture-or-geometry>_<map-type>_<size>`; `MaterialDesc::texture_key`
  (`src/assets/material.rs`) does the same per `TextureSlot`
  (diffuse/alpha/gal/normal/specular/ao/glow). A mesh node's `texture`,
  `diffuse` and per-slot attributes name the bases, `tex_settings` picks a
  `Settings3D` size column, and the client-local "Texture quality"
  preference (`RenderPreferences.texture_quality`, Low/Medium/High) picks
  the row — mirroring `displaySetting3DsizeTextures` +
  `displaySetting3DqualityTextures`, which also gate the glow and
  normal/specular/ao layers. Layer ids are requested only when present in
  the `resources_3d` manifest (`geometry_file_exists`), so meshes without
  authored layers never fetch 404s. The renderer decodes ATF and JXR
  source images off-thread.
- `EntityBasicMaterial.as`/`EntityXRayMaterial.as` map to the two shared
  GLSL 330 programs in `src/render/materials3d.rs` (lit + `light="false"`
  unlit). Each `Model` material binds all seven resolved layers into the
  raylib map slots (specular→METALNESS, gal→ROUGHNESS, ao→OCCLUSION,
  glow→EMISSION, alpha→HEIGHT) with 1x1 identity fallbacks for absent
  layers, so no per-material shader state is needed. Per-mesh
  `specularity`/`gloss`/`alphaBlending` ride inside `colDiffuse`; sun +
  ambient + the hero point light are pushed per frame from
  `TextureCache::sync_models3d`. Known deviations (UV1-only lightmaps,
  linear point-light falloff, no per-material blend/cull switching) are
  listed in the `materials3d.rs` header.
- `AWD2Parser.as` is the source for the defensive static-geometry reader in
  `src/assets/awd.rs`. The checked-in Aegis and Palladium fixtures are genuine
  resources from the game's 3D manifest.
- `DronesFormation3D.as`/`Drone3D.as` drive drone placement
  (`src/render/scene3d.rs::draw_drone_models`). Each owner's
  `_formationRotation` eases toward the ship heading over 0.3 s
  (Quad ease-out, short-angle) and each drone's ship-local position chases
  its slot with the same 0.3 s ease. `game.xml`
  `<formation><display3D>` (`PatternManager.parseDroneFormationPatterns3D`)
  supplies the 3D slot table (with height) and `<rotationGroup droneIds>`
  containers whose `<tween>` attributes — `rotationX/Y/Z`, `x/y/z`,
  `scale`, `delay`, `repeat`, `yoyo`, `ease` — animate the group's
  transform (`src/assets/formations.rs::DroneFormationPattern::target3d`).
  The slot vector passes through the spinning container first, then
  `userScale`, formation yaw and the group's static `rotationX/Y/Z` base
  euler apply in that order. The drone model itself only yaws
  (`rotationY = _formationRotation - 90`), so ring/dome/drill formations
  tilt and orbit via drone *positions*, not hull banking. Formations
  without `display3D` reuse the 2D table, matching the
  `standardFormations` fallback.
- `LaserSalvo3D.as`/`LaserBullet3D.as`/`Lasers3D.as` drive combat lasers
  (`src/game/mod.rs` `WorldFx::LaserRun` + `src/render/effects3d.rs`).
  Bolts spawn per salvo group (200 ms stagger) per named mount; each mount
  name resolves to the ship AWD's `laserpoint_<name>` slot
  (`awd::parse_aw2_slots` = `ShipView3D.processMesh` + `Slot3D.fromMesh`,
  bounds center through the instance matrix) pushed through the
  `meshes[0]` node transform and the eased ship rotation — falling back to
  the expansion `positionsList` offset or ship center when no authored
  slot exists. Absorber and `ignoreLaserPoints` defs fire once
  center-to-center like stock. The projectile body follows `laserType`:
  `doublePlane` (default) draws the laser clip on two crossed
  flight-aligned planes, `singlePlane` draws one 2x flight-perpendicular
  quad, and `particle` queues the `{resKey}.zip` particle group `lookAt`-ed
  along the flight path. `bolt_tween` reproduces `LaserSalvo3D.shoot`
  exactly: both `checkDistance` drops (before and after the
  `visualSize*50` shield pullback), the absorber endpoint swap, and the
  one-bolt-length overshoot of the tween target. `muzzleFlashResKey`
  groups fire once at the authored slot with the ship+mesh world
  rotation. Effects whose projectile assets are still streaming render
  through the 2D billboard path until ready.
- `LockDisplay3D.as` supplies the 3D target marker
  (`src/render/lock3d.rs`): the four `abstract_ui_mark_target_*.zip`
  particle groups keyed by `lockType` (1 red, 2 gray-light, 3 purple,
  4 gray-dark), queued before `particles3d::flush`, following the
  target's drawn position and scaling from `1.5x visualSize3D` to
  `visualSize3D` over 0.3 s, resetting on target/type change.

The currently supported AWD subset is version 2.1 with no compression or zlib
compression, top-level geometry blocks, float position/UV attributes, and
16-bit triangle indices. The parser enforces a 128 MiB decoded-body limit and
rejects malformed or unsupported blocks. It bakes every top-level mesh instance with
its own matrix (skipping the `engine_`/`laserpoint_`/... attachment helpers, as
`ShipView3D` does) into one mesh. It does not yet handle LZMA, nested instance
hierarchies, float64 AWD, bones or animated
vertices, all Away3D material/shader maps, particles, or animated map scene
objects. Some recipes that reference these features continue through the
existing 2D renderer.

The native model path converts parsed static geometry to OBJ and uploads it
through raylib. CPU decoding (AWD→OBJ, ATF/JXR→RGBA) runs on the
`ModelLoader` worker thread while raylib model and texture creation stays on
the render thread. Models are cached under `materials3d::model_key`
(geometry + a fingerprint of the resolved layer ids, scalars and quality),
and a model is only built once every resolved layer has either uploaded or
definitively failed — a late texture can never bake in as a permanent
fallback. The render path has not yet had final visual QA against a live
session with each 3D preference enabled.
