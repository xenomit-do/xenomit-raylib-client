//! Parser tests over the real embedded fixtures (`tests/fixtures/*.xml`)
//! plus synthetic `resources.xml` / `maps-config` documents.

use std::path::Path;
use xenomit_client::assets::bootstrap::BootConfig;
use xenomit_client::assets::gamexml::GameXml;
use xenomit_client::assets::mapxml::MapDb;
use xenomit_client::assets::resources::FileIndex;
use xenomit_client::assets::uberlib::UberAssets;

fn fixture(name: &str) -> String {
    std::fs::read_to_string(Path::new("tests/fixtures").join(name))
        .unwrap_or_else(|e| panic!("cannot read {name}: {e}"))
}

#[test]
fn game_xml_parses_real_fixture() {
    let xml = fixture("game.xml");
    let game = GameXml::parse(&xml).expect("game.xml parses");
    assert!(!game.ships_by_type.is_empty(), "no ships parsed");

    let goliath = game
        .ship("ship_goliath")
        .expect("ship_goliath pattern exists");
    assert_eq!(goliath.ship_id, 10);
    assert_eq!(goliath.name, "Goliath");
    assert_eq!(goliath.icon_id, "ship_10");
    assert_eq!(goliath.expansion_id, "ship_goliath");
    assert_eq!(goliath.engine_position_id, "ship_goliath");
    assert_eq!(goliath.asset_id, "ship_goliath");
    assert_eq!(goliath.click_radius, 70);
    assert_eq!(goliath.default_character, "player");

    // shipId / npcId secondary maps
    let by_id = game.ship_by_id(10).expect("shipId=10 resolves");
    assert!(!by_id.type_id.is_empty());

    // Designs clone the base pattern: a goliath design inherits
    // expansionClassID/enginePositionClassID from `ship_goliath`.
    let designs: Vec<_> = game
        .ships_by_type
        .values()
        .filter(|p| p.type_id.starts_with("ship_g"))
        .collect();
    assert!(designs.len() > 10, "expected many goliath designs");
    let exalted = game.ship("ship_g-exalted").expect("design present");
    assert_eq!(exalted.ship_id, 62);
    assert_eq!(exalted.expansion_id, "ship_goliath"); // inherited

    // NPC ship patterns keyed by npcId
    let demaner = game.ship_by_npc(39).expect("npcId=39 resolves");
    assert_eq!(demaner.name, "Demaner");

    // Sections
    assert!(game.backgrounds.contains_key(&1));
    let bg1 = &game.backgrounds[&1];
    assert_eq!(bg1.res_key, "background1");
    assert!(game.ores.contains_key(&0), "ore type 0");
    assert_eq!(game.ores[&0].res_key, "ore_1");
    assert_eq!(game.ores[&8].res_key, "ore_15");
    // `refiner` recipes — `OrePattern.refiners` in the stock client.
    assert_eq!(game.ores[&4].refiner, [(0, 20), (1, 10)]);
    assert_eq!(game.ores[&5].refiner, [(1, 10), (2, 20)]);
    assert_eq!(game.ores[&6].refiner, [(4, 10), (5, 10), (3, 1)]);
    // `<refinementWindow>` — lab window layout lists.
    let rw = &game.refinement;
    assert_eq!(
        rw.refine_static
            .iter()
            .map(|o| o.ore_type)
            .collect::<Vec<_>>(),
        [0, 1, 2]
    );
    assert_eq!(
        rw.refine_ores
            .iter()
            .map(|o| o.ore_type)
            .collect::<Vec<_>>(),
        [4, 5, 6, 3]
    );
    assert_eq!(
        rw.refine_ores.iter().map(|o| o.button).collect::<Vec<_>>(),
        [true, true, true, false]
    );
    assert_eq!(
        rw.update_static
            .iter()
            .map(|o| o.ore_type)
            .collect::<Vec<_>>(),
        [0, 1, 2, 3, 8]
    );
    assert_eq!(
        rw.update_ores
            .iter()
            .map(|o| o.ore_type)
            .collect::<Vec<_>>(),
        [4, 5, 6, 7, 28]
    );
    assert_eq!(rw.update_ores[4].icon_res_key, "osmium_icon");
    assert_eq!(
        rw.update_items
            .iter()
            .map(|i| i.identifier.as_str())
            .collect::<Vec<_>>(),
        ["LASER", "ROCKET", "DRIVING", "SHIELD"]
    );
    assert_eq!(rw.update_items[0].res_key, "item_1");
    assert_eq!(rw.update_items[0].amount_key, "lab_salvos");
    assert!(game.engines.contains_key(&0));
    assert_eq!(game.engines[&0].res_key, "engine0");
    assert!(game.engine_smokes.contains_key(&0));
    assert!(game.rocket_smokes.contains_key(&0));
    assert!(game
        .engine_positions
        .get("ship_goliath")
        .is_some_and(|p| !p.positions.is_empty()));
    assert!(game
        .expansions
        .get("ship_goliath")
        .is_some_and(|c| c.stages.contains_key(&1)));
    // Stage 0 mirrors stage 1 (AS3 alias).
    assert!(game.expansions["ship_goliath"].stages.contains_key(&0));
    assert!(game.minimaps.contains_key(&16));
    assert!(game.background_masks.contains_key(&1));
    assert!(game
        .planets
        .get("1")
        .is_some_and(|p| p.res_key == "planet1"));
    assert!(game
        .drone_designs
        .get(&1)
        .is_some_and(|d| d.asset_id == "havok"));
    assert!(!game.preload.is_empty(), "preload list");
    assert!(game.icons.contains_key("drawFire"));
    assert!(
        game.ships_by_type.len() > 150,
        "183+ ships/designs expected"
    );
}

#[test]
fn game_xml_tolerates_malformed_comments() {
    // Nested `<!--` inside a comment — the real file ships one.
    let xml = r#"<game><patterns><ships>
        <!-- <ship type="x<!-- _broken" />
        <ship type="y" /> -->
        <ship type="ok_ship" shipId="7"/>
        </ships></patterns></game>"#;
    let game = GameXml::parse(xml).expect("malformed comment tolerated");
    assert!(game.ship("ok_ship").is_some());
}

#[test]
fn uber_assets_parse_all_fixtures() {
    // Canonical lib ids (`UberAssetsLib.LIB_*`): the fixture files follow
    // `lib_*.xml` naming but `lib_default.xml` is the `lib_assets` library.
    let libs: Vec<(String, String)> = [
        ("lib_assets", "lib_default.xml"),
        ("lib_ships", "lib_ships.xml"),
        ("lib_drones", "lib_drones.xml"),
        ("lib_portals", "lib_portals.xml"),
        ("lib_collectables", "lib_collectables.xml"),
        ("lib_battlestation", "lib_battlestation.xml"),
        ("lib_maps", "lib_maps.xml"),
        ("lib_poizones", "lib_poizones.xml"),
    ]
    .iter()
    .map(|(lib, file)| (lib.to_string(), fixture(file)))
    .collect();
    let refs: Vec<(&str, &str)> = libs.iter().map(|(n, t)| (n.as_str(), t.as_str())).collect();
    let uber = UberAssets::parse(&refs).expect("uber libs parse");
    assert!(uber.len() > 100, "hundreds of assets expected");

    // Ships: display2D carries srcKey on the element itself.
    let goliath = uber.display_2d("ship_goliath").expect("ship_goliath");
    assert_eq!(goliath.src_key, "ship_goliath");
    assert_eq!(goliath.res_key, "mc"); // default
    assert_eq!(goliath.replacement, "ship_goliath");

    // Designs resolve through template references.
    let d = uber.display_2d("ship_g-exalted").expect("design resolves");
    assert!(!d.src_key.is_empty(), "design src_key");

    // Portals: bare numeric ids, atlas recipes.
    let portal = uber.display_2d("1").expect("portal 1");
    assert!(!portal.src_key.is_empty(), "portal src_key (atlas texture)");
    assert_eq!(portal.atlas_key, "portal_standard_atlas");
    assert!(!portal.atlas_region.is_empty(), "portal region");

    // Collectables: comma-separated ids + template srcKey replace.
    let ore = uber.display_2d("ore_0").expect("ore_0");
    assert_eq!(ore.src_key, "oreRed");
    assert_eq!(ore.res_key, "mc");
    assert!(ore.use_bitmap_clip);
    let bonus = uber.display_2d("box_BONUS_BOX").expect("box_BONUS_BOX");
    assert_eq!(bonus.src_key, "box2");
    // Comma ids: box_RAZER_BOX aliases the same asset.
    assert_eq!(uber.display_2d("box_RAZER_BOX").unwrap().src_key, "box2");
    let fw = uber.display_2d("firework_box").expect("firework_box");
    assert_eq!(fw.src_key, "fireworks_box");

    // Mines live in lib_default.
    let mine = uber.display_2d("mine_2").expect("mine_2");
    assert_eq!(mine.src_key, "emp_mine");

    // Composite `lib:id` form also works.
    assert!(uber.display_2d("lib_portals:1").is_some());

    // Traits via node().
    let node = uber.node("ore_0").expect("ore_0 traits");
    assert_eq!(node.click_radius, 25);
    let pnode = uber.node("1").expect("portal traits");
    assert!(pnode.minimap_icon.is_some());
}

#[test]
fn ship_display_resolves_hero_aegis() {
    // Live capture: `ShipInitializationCommand.typeId` for the hero is
    // `ship_aegis` — it must resolve to a recipe with a real srcKey or
    // the hero renders as a placeholder dot.
    let game_xml = GameXml::parse(&fixture("game.xml")).expect("game.xml parses");
    let libs: Vec<(String, String)> = [("lib_ships", "lib_ships.xml")]
        .iter()
        .map(|(l, f)| (l.to_string(), fixture(f)))
        .collect();
    let refs: Vec<(&str, &str)> = libs.iter().map(|(n, t)| (n.as_str(), t.as_str())).collect();
    let uber = UberAssets::parse(&refs).expect("lib_ships parses");

    // The faction-keyed step the live client (factionID=2) takes first.
    let eic = uber
        .display_2d("lib_ships:ship_aegis_2")
        .expect("ship_aegis_2 in lib_ships");
    assert!(!eic.src_key.is_empty(), "ship_aegis_2 src_key empty");

    let files = FileIndex::parse("<resources/>").expect("empty index parses");
    let assets = xenomit_client::assets::AssetManager::new(
        std::path::PathBuf::new(),
        files,
        game_xml,
        uber,
        MapDb::default(),
    );
    let d = assets
        .ship_display("ship_aegis", 0)
        .expect("ship_aegis resolves");
    assert!(!d.src_key.is_empty(), "ship_aegis src_key empty");
}

#[test]
fn file_index_url_scheme() {
    let xml = r#"<resources>
        <location id="do_img" path="do_img/"/>
        <location id="root" path=""/>
        <file id="ship_goliath" location="do_img" name="ships/goliath" type="swf" version="1" hash="abc123"/>
        <file id="maps-config" location="root" name="maps-config" type="xml" version="2"/>
    </resources>"#;
    let index = FileIndex::parse(xml).expect("resources.xml parses");
    assert_eq!(
        index.url("https://cdn.example.com/spacemap/", "ship_goliath"),
        Some("https://cdn.example.com/spacemap/do_img/ships/goliath.swf?__cv=abc123".to_string())
    );
    // No `hash` attribute → no __cv suffix.
    assert_eq!(
        index.url("https://cdn.example.com/spacemap/", "maps-config"),
        Some("https://cdn.example.com/spacemap/maps-config.xml".to_string())
    );
    assert!(index.url("https://cdn/", "missing").is_none());
}

#[test]
fn map_db_parse() {
    let xml = r#"<maps>
        <map id="1" name="MMO home" scaleFactor="1.2" poiColor="16777215">
            <backgrounds>
                <background typeID="1" pFactor="10" layer="0"/>
            </backgrounds>
            <planets>
                <planet typeID="1" x="100" y="200" rotation="30" pFactor="2.5"
                        layer="1" scale="0" quality_settings="high"/>
            </planets>
        </map>
        <map id="42">
            <backgrounds>
                <background typeID="16" isMain="false" region="0,0,2048,2048"
                            pFactor="4" layer="2" shiftX="26" shiftY="-5"
                            scale="0.5" maskID="3"/>
                <background typeID="2" isMain="true" region="4096,2048"/>
            </backgrounds>
        </map>
    </maps>"#;
    let db = MapDb::parse(xml).expect("maps-config parses");
    let m1 = db.get(1).expect("map 1");
    assert_eq!(m1.scale_factor, 1.2);
    assert_eq!(m1.poi_color, 0xFFFFFF);
    assert_eq!(m1.backgrounds.len(), 1);
    // Single background => isMain defaults true.
    assert!(m1.backgrounds[0].is_main);
    assert_eq!(m1.planets.len(), 1);
    let p = &m1.planets[0];
    assert_eq!(p.type_id, "1");
    assert_eq!(p.scale, 1.0); // scale="0" falls back to 1
    assert_eq!(p.parallax_factor, 2.5);

    let m42 = db.get(42).expect("map 42");
    assert_eq!(m42.scale_factor, 1.0); // default
    assert_eq!(m42.backgrounds[0].region, Some((0, 0, 2048, 2048)));
    assert_eq!(m42.backgrounds[1].region, Some((0, 0, 4096, 2048)));
    assert_eq!(m42.backgrounds[0].mask_id, 3);
    assert_eq!(m42.backgrounds[1].mask_id, -1);
    assert_eq!(m42.backgrounds[0].parallax_factor, 4);
    assert_eq!(m42.backgrounds[1].parallax_factor, 10); // default
}

#[test]
fn boot_config_from_flashvars() {
    let mut vars = xenomit_client::assets::bootstrap::FlashVars::new();
    vars.insert("cdn".into(), "https://cdn.example.com/".into());
    vars.insert("basePath".into(), "spacemap".into());
    vars.insert("resourcesXmlHash".into(), "deadbeef".into());
    vars.insert("userID".into(), "123".into());
    vars.insert("factionID".into(), "2".into());
    vars.insert("mapID".into(), "16".into());
    vars.insert("pid".into(), "42".into());
    vars.insert("lang".into(), "en_US".into());
    vars.insert("dynamicHost".into(), "https://game.example.com/".into());
    let cfg = BootConfig::from_flashvars(&vars).expect("boot config");
    assert_eq!(cfg.resource_base(), "https://cdn.example.com/spacemap/");
    assert_eq!(
        cfg.resources_xml_url(),
        "https://cdn.example.com/spacemap/xml/resources.xml?__cv=deadbeef"
    );
    assert_eq!(
        cfg.dynamic_base("https://instance.example.com/"),
        "https://game.example.com/spacemap/"
    );
    assert_eq!(cfg.language, "en");
    assert_eq!(cfg.faction_id, 2);
    assert_eq!(cfg.map_id, 16);
}
