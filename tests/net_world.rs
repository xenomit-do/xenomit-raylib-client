//! Unit tests for `net::world` — MoveTween interpolation, SharedWorld::pos
//! fallbacks, packet-driven tween bookkeeping, and the `game_log` console
//! line parser.

use std::time::{Duration, Instant};
use xenomit_client::locale::LocaleCatalog;
use xenomit_client::net::world::{game_log, MoveTween, SharedWorld};
use xenomit_packet_client::modules::id;
use xenomit_packet_client::packet::{Packet, Value};

fn empty_locale() -> LocaleCatalog {
    LocaleCatalog::default()
}

fn pkt(id: i16, name: Option<&'static str>, fields: Vec<(&'static str, Value)>) -> Packet {
    Packet {
        id,
        name,
        fields: fields.into_iter().map(|(n, v)| (Some(n), v)).collect(),
    }
}

fn int(name: &'static str, v: i64) -> (&'static str, Value) {
    (name, Value::Int(v))
}

fn string(name: &'static str, v: &str) -> (&'static str, Value) {
    (name, Value::Str(v.to_owned()))
}

fn bool_(name: &'static str, v: bool) -> (&'static str, Value) {
    (name, Value::Bool(v))
}

fn f64_(name: &'static str, v: f64) -> (&'static str, Value) {
    (name, Value::F64(v))
}

fn module(p: Packet) -> Value {
    Value::Module(Box::new(p))
}

fn list(name: &'static str, v: Vec<Value>) -> (&'static str, Value) {
    (name, Value::List(v))
}

fn move_command(uid: i32, x: i32, y: i32, ms: i32) -> Packet {
    pkt(
        id::MOVE_COMMAND,
        Some("MoveCommand"),
        vec![
            int("userId", uid as i64),
            int("x", x as i64),
            int("y", y as i64),
            int("timeToTarget", ms as i64),
        ],
    )
}

fn hero_init(uid: i32, x: i32, y: i32, speed: i32) -> Packet {
    pkt(
        id::SHIP_INITIALIZATION,
        Some("ShipInitializationCommand"),
        vec![
            int("userId", uid as i64),
            int("x", x as i64),
            int("y", y as i64),
            int("speed", speed as i64),
        ],
    )
}

fn beacon(x: i32, y: i32) -> Packet {
    pkt(
        id::BEACON_COMMAND,
        Some("BeaconCommand"),
        vec![int("positionX", x as i64), int("positionY", y as i64)],
    )
}

// ---------------------------------------------------------------- tween lerp

#[test]
fn tween_lerps_midpoint() {
    let now = Instant::now();
    let t = MoveTween {
        from: (0.0, 0.0),
        to: (100.0, 50.0),
        duration_ms: 1000,
        started: now - Duration::from_millis(500),
    };
    let (x, y) = t.pos_at(now);
    assert!((x - 50.0).abs() < 1e-3, "x={x}");
    assert!((y - 25.0).abs() < 1e-3, "y={y}");
}

#[test]
fn tween_clamps_at_end() {
    let now = Instant::now();
    let t = MoveTween {
        from: (0.0, 0.0),
        to: (100.0, 50.0),
        duration_ms: 1000,
        started: now - Duration::from_secs(2),
    };
    assert_eq!(t.pos_at(now), (100.0, 50.0));
    assert!(t.done());
}

#[test]
fn tween_clamps_at_start() {
    let now = Instant::now();
    let t = MoveTween {
        from: (10.0, 20.0),
        to: (100.0, 50.0),
        duration_ms: 1000,
        started: now,
    };
    // Sampling before `started` must not extrapolate backwards.
    assert_eq!(t.pos_at(now - Duration::from_millis(500)), (10.0, 20.0));
    assert!(!t.done());
}

#[test]
fn tween_zero_duration_snaps() {
    let t = MoveTween {
        from: (0.0, 0.0),
        to: (7.0, 9.0),
        duration_ms: 0,
        started: Instant::now(),
    };
    assert_eq!(t.pos(), (7.0, 9.0));
    assert!(t.done());
}

// ------------------------------------------------------------- SharedWorld

#[test]
fn pos_falls_back_to_model() {
    let mut sw = SharedWorld::default();
    sw.world.entities.insert(
        5,
        xenomit_packet_client::model::Entity {
            id: 5,
            x: 300,
            y: 400,
            ..Default::default()
        },
    );
    assert_eq!(sw.pos(5), (300.0, 400.0));
}

#[test]
fn pos_uses_hero_not_entity_row() {
    // The hero's entity row stays at (0,0) — pos must resolve via hero.
    let mut sw = SharedWorld::default();
    sw.apply_packet(&hero_init(7, 111, 222, 400));
    assert_eq!(sw.pos(7), (111.0, 222.0));
    assert_eq!(sw.hero_pos(), Some((111.0, 222.0)));
}

#[test]
fn pos_unknown_id_is_origin() {
    let sw = SharedWorld::default();
    assert_eq!(sw.pos(999), (0.0, 0.0));
}

#[test]
fn pos_prefers_active_tween() {
    let mut sw = SharedWorld::default();
    sw.world.entities.insert(
        5,
        xenomit_packet_client::model::Entity {
            id: 5,
            x: 300,
            y: 400,
            ..Default::default()
        },
    );
    sw.tweens.insert(
        5,
        MoveTween {
            from: (0.0, 0.0),
            to: (100.0, 100.0),
            duration_ms: 1000,
            started: Instant::now() - Duration::from_millis(500),
        },
    );
    let (x, _) = sw.pos(5);
    assert!((x - 50.0).abs() < 5.0, "x={x}");
}

// ------------------------------------------------------- apply_packet tweens

#[test]
fn move_command_creates_tween() {
    let mut sw = SharedWorld::default();
    sw.apply_packet(&move_command(5, 1000, 2000, 3000));
    let t = sw.tweens.get(&5).expect("tween for uid 5");
    assert_eq!(t.to, (1000.0, 2000.0));
    assert_eq!(t.duration_ms, 3000);
    // Entity was unknown -> from is the model fallback (0,0).
    assert_eq!(t.from, (0.0, 0.0));
    // move_target is folded into the model too.
    assert_eq!(
        sw.world.entities.get(&5).unwrap().move_target,
        Some((1000, 2000))
    );
}

#[test]
fn move_command_chains_from_interpolated() {
    let mut sw = SharedWorld::default();
    sw.apply_packet(&move_command(5, 1000, 2000, 60_000));
    std::thread::sleep(Duration::from_millis(15));
    sw.apply_packet(&move_command(5, 500, 500, 1000));
    let t = sw.tweens.get(&5).unwrap();
    assert_eq!(t.to, (500.0, 500.0));
    // `from` continues from the interrupted tween's interpolated pos.
    assert!(t.from.0 > 0.0, "from={:?}", t.from);
    assert!(t.from.0 < 1000.0);
}

#[test]
fn hero_move_duration_from_speed() {
    let mut sw = SharedWorld::default();
    sw.apply_packet(&hero_init(7, 0, 0, 400));
    // dist(0,0 -> 800,600) = 1000; speed 400 u/s -> 2500 ms.
    sw.apply_packet(&pkt(
        id::HERO_MOVE,
        Some("HeroMoveCommand"),
        vec![int("x", 800), int("y", 600)],
    ));
    let t = sw.tweens.get(&7).expect("hero tween");
    assert_eq!(t.duration_ms, 2500);
    assert_eq!(t.to, (800.0, 600.0));
}

#[test]
fn hero_move_zero_speed_uses_default() {
    let mut sw = SharedWorld::default();
    sw.apply_packet(&hero_init(7, 0, 0, 0));
    sw.apply_packet(&pkt(
        id::HERO_MOVE,
        Some("HeroMoveCommand"),
        vec![int("x", 800), int("y", 600)],
    ));
    assert_eq!(sw.tweens.get(&7).unwrap().duration_ms, 400);
}

#[test]
fn ship_remove_drops_tween() {
    let mut sw = SharedWorld::default();
    sw.apply_packet(&move_command(5, 1000, 2000, 3000));
    assert!(sw.tweens.contains_key(&5));
    sw.apply_packet(&pkt(
        id::SHIP_REMOVE,
        Some("ShipRemoveCommand"),
        vec![int("userId", 5)],
    ));
    assert!(!sw.tweens.contains_key(&5));
    assert!(!sw.world.entities.contains_key(&5));
}

// ------------------------------------------------------ hero prediction

#[test]
fn start_hero_move_builds_predicted_tween() {
    let mut sw = SharedWorld::default();
    sw.apply_packet(&hero_init(7, 1000, 1000, 300));
    sw.start_hero_move((1300, 1000));
    let t = sw.tweens.get(&7).expect("hero tween");
    // dist 300 @ 300 u/s -> 1000 ms.
    assert_eq!(t.duration_ms, 1000);
    assert_eq!(t.from, (1000.0, 1000.0));
    assert_eq!(t.to, (1300.0, 1000.0));
}

#[test]
fn sync_hero_model_writes_interpolated_pos() {
    let mut sw = SharedWorld::default();
    sw.apply_packet(&hero_init(7, 0, 0, 400));
    sw.tweens.insert(
        7,
        MoveTween {
            from: (0.0, 0.0),
            to: (700.0, 900.0),
            duration_ms: 0, // already arrived
            started: Instant::now(),
        },
    );
    sw.sync_hero_model();
    let h = sw.world.hero.as_ref().unwrap();
    assert_eq!((h.x, h.y), (700, 900));
}

#[test]
fn beacon_snaps_idle_hero() {
    let mut sw = SharedWorld::default();
    sw.apply_packet(&hero_init(7, 0, 0, 400));
    sw.apply_packet(&beacon(500, 600));
    let h = sw.world.hero.as_ref().unwrap();
    assert_eq!((h.x, h.y), (500, 600));
    assert_eq!(sw.hero_pos(), Some((500.0, 600.0)));
}

#[test]
fn beacon_far_retargets_active_tween() {
    let mut sw = SharedWorld::default();
    sw.apply_packet(&hero_init(7, 0, 0, 300));
    sw.start_hero_move((3000, 0));
    // Predicted pos is ~(0,0); the beacon at (0,1000) is 1000 > 500 away.
    sw.apply_packet(&beacon(0, 1000));
    let t = sw.tweens.get(&7).expect("retargeted tween");
    assert_eq!(t.from, (0.0, 1000.0));
    assert_eq!(t.to, (3000.0, 0.0));
    let h = sw.world.hero.as_ref().unwrap();
    assert_eq!((h.x, h.y), (0, 1000));
}

#[test]
fn beacon_near_leaves_tween_alone() {
    let mut sw = SharedWorld::default();
    sw.apply_packet(&hero_init(7, 0, 0, 300));
    sw.start_hero_move((3000, 0));
    let before = sw.tweens.get(&7).unwrap().to;
    // Beacon 100 units from the predicted pos — inside resync range.
    sw.apply_packet(&beacon(100, 0));
    let t = sw.tweens.get(&7).expect("tween still there");
    assert_eq!(t.to, before);
    // Close enough: model keeps the predicted pos, not the beacon's.
    let h = sw.world.hero.as_ref().unwrap();
    assert_eq!((h.x, h.y), (0, 0));
}

#[test]
fn beacon_idle_small_drift_sets_pos() {
    let mut sw = SharedWorld::default();
    sw.apply_packet(&hero_init(7, 0, 0, 300));
    // < 5 units: settle the model, no tween.
    sw.apply_packet(&beacon(3, 0));
    let h = sw.world.hero.as_ref().unwrap();
    assert_eq!((h.x, h.y), (3, 0));
    assert!(!sw.tweens.contains_key(&7));
}

#[test]
fn beacon_idle_medium_drift_smooths() {
    let mut sw = SharedWorld::default();
    sw.apply_packet(&hero_init(7, 0, 0, 300));
    // <= ~300 units: glide onto the beacon pos over a short tween.
    sw.apply_packet(&beacon(200, 0));
    let t = sw.tweens.get(&7).expect("smoothing tween");
    assert_eq!(t.to, (200.0, 0.0));
    assert_eq!(t.duration_ms, 250);
    assert_eq!(t.from, (0.0, 0.0));
    let h = sw.world.hero.as_ref().unwrap();
    assert_eq!((h.x, h.y), (200, 0));
}

#[test]
fn beacon_lagging_does_not_pull_back() {
    let mut sw = SharedWorld::default();
    sw.apply_packet(&hero_init(7, 0, 0, 300));
    // A finished prediction sitting at `to`.
    sw.tweens.insert(
        7,
        MoveTween {
            from: (0.0, 0.0),
            to: (3000.0, 0.0),
            duration_ms: 1,
            started: Instant::now() - Duration::from_secs(10),
        },
    );
    // The beacon is still 500 behind on the same course — the ship must
    // stay at the target while the server catches up.
    sw.apply_packet(&beacon(2500, 0));
    assert_eq!(sw.hero_pos(), Some((3000.0, 0.0)));
    assert_eq!(sw.tweens.get(&7).unwrap().to, (3000.0, 0.0));
    // Once the beacon arrives, the model settles.
    sw.apply_packet(&beacon(3000, 0));
    assert_eq!(sw.hero_pos(), Some((3000.0, 0.0)));
    assert!(!sw.tweens.contains_key(&7));
}

#[test]
fn beacon_stalled_far_pulls_back() {
    let mut sw = SharedWorld::default();
    sw.apply_packet(&hero_init(7, 0, 0, 300));
    sw.tweens.insert(
        7,
        MoveTween {
            from: (0.0, 0.0),
            to: (3000.0, 0.0),
            duration_ms: 1,
            started: Instant::now() - Duration::from_secs(10),
        },
    );
    // First lagging beacon: no baseline yet — treated as converging.
    sw.apply_packet(&beacon(2100, 0));
    assert_eq!(sw.hero_pos(), Some((3000.0, 0.0)));
    // The server stopped 900 units short (denied move) — the repeated
    // beacon is not converging, so it must snap the ship back.
    sw.apply_packet(&beacon(2100, 0));
    assert_eq!(sw.hero_pos(), Some((2100.0, 0.0)));
    assert!(!sw.tweens.contains_key(&7));
}

#[test]
fn ship_init_clears_hero_tween() {
    let mut sw = SharedWorld::default();
    sw.apply_packet(&hero_init(7, 0, 0, 300));
    sw.start_hero_move((3000, 0));
    assert!(sw.tweens.contains_key(&7));
    // Respawn / map jump — fresh hero row, fresh position.
    sw.apply_packet(&hero_init(7, 42, 43, 300));
    assert!(!sw.tweens.contains_key(&7));
    assert_eq!(sw.hero_pos(), Some((42.0, 43.0)));
}

// ------------------------------------------------------------------ game_log

#[test]
fn game_log_legacy_reward_lines() {
    let locale = empty_locale();
    let p = pkt(
        id::LEGACY,
        Some("LegacyModule"),
        vec![string("message", "0|y|CRE|10|5000")],
    );
    // Empty catalog keeps the raw reward line.
    assert_eq!(game_log(&p, &locale).as_deref(), Some("0|y|CRE|10|5000"));

    // Chat `MSG` lines are handled by `net::chat`, not `game_log`.
    let chat = pkt(
        id::LEGACY,
        Some("LegacyModule"),
        vec![string("message", "L|n|MSG|hi there")],
    );
    assert_eq!(game_log(&chat, &locale), None);
}

#[test]
fn game_log_legacy_filters_noise() {
    let p = pkt(
        id::LEGACY,
        Some("LegacyModule"),
        vec![string("message", "w")],
    );
    assert_eq!(game_log(&p, &empty_locale()), None);
}

#[test]
fn game_log_message_command_replaces_wildcards() {
    let replacement = |k: &str, v: &str| {
        Value::Module(Box::new(pkt(
            0,
            None,
            vec![string("wildcard", k), string("replacement", v)],
        )))
    };
    let message = Value::Module(Box::new(pkt(
        0,
        None,
        vec![
            string("baseKey", "loot %LOOT%"),
            (
                "wildCardReplacements",
                Value::List(vec![replacement("%LOOT%", "10 credits")]),
            ),
        ],
    )));
    let p = pkt(0, Some("LogMessageCommand"), vec![("message", message)]);
    assert_eq!(
        game_log(&p, &empty_locale()).as_deref(),
        Some("loot 10 credits")
    );
}

#[test]
fn game_log_message_command_appends_unmatched_wildcards() {
    let message = Value::Module(Box::new(pkt(
        0,
        None,
        vec![
            string("baseKey", "plain"),
            (
                "wildCardReplacements",
                Value::List(vec![Value::Module(Box::new(pkt(
                    0,
                    None,
                    vec![string("wildcard", "%X%"), string("replacement", "b")],
                )))]),
            ),
        ],
    )));
    let p = pkt(0, Some("LogMessageCommand"), vec![("message", message)]);
    assert_eq!(
        game_log(&p, &empty_locale()).as_deref(),
        Some("plain %X%=b")
    );
}

#[test]
fn game_log_ignores_other_modules() {
    let p = pkt(id::PING, Some("PingCommand"), vec![]);
    assert_eq!(game_log(&p, &empty_locale()), None);
}

#[test]
fn apply_packet_appends_log_lines() {
    let mut sw = SharedWorld::default();
    sw.apply_packet(&pkt(
        id::LEGACY,
        Some("LegacyModule"),
        vec![string("message", "0|y|CRE|10|5000")],
    ));
    assert_eq!(sw.log.len(), 1);
    assert_eq!(sw.log[0].text, "0|y|CRE|10|5000");
}

// ------------------------------------------------------------------ portals

fn jumpgate_create(gate: i32, design: i32, x: i32, y: i32) -> Packet {
    pkt(
        3389,
        Some("JumpgateCreateCommand"),
        vec![
            int("gateId", gate as i64),
            int("designId", design as i64),
            int("x", x as i64),
            int("y", y as i64),
            int("factionId", 2),
            ("isVisibleOnMinimap", Value::Bool(true)),
            ("showBubble", Value::Bool(true)),
        ],
    )
}

#[test]
fn jumpgate_create_adds_portal() {
    let mut sw = SharedWorld::default();
    sw.apply_packet(&jumpgate_create(9, 1, 4000, 3000));
    let p = sw.portals.get(&9).expect("portal added");
    assert_eq!(p.type_id, "1");
    assert_eq!((p.x, p.y), (4000, 3000));
    assert_eq!(p.faction_id, 2);
    assert!(p.on_minimap);
    // An update with the same gate id overwrites, not duplicates.
    sw.apply_packet(&jumpgate_create(9, 3, 1, 2));
    assert_eq!(sw.portals.len(), 1);
    assert_eq!(sw.portals[&9].type_id, "3");
}

#[test]
fn jumpgate_remove_drops_portal() {
    let mut sw = SharedWorld::default();
    sw.apply_packet(&jumpgate_create(9, 1, 4000, 3000));
    sw.apply_packet(&pkt(
        4274,
        Some("JumpGateRemoveCommand"),
        vec![int("gateId", 9)],
    ));
    assert!(sw.portals.is_empty());
}

#[test]
fn ship_selection_fills_entity_vitals() {
    use xenomit_packet_client::model::Entity;
    let mut sw = SharedWorld::default();
    sw.world.entities.insert(
        7,
        Entity {
            id: 7,
            type_id: "ship_goliath".into(),
            ..Default::default()
        },
    );
    sw.apply_packet(&pkt(
        50,
        Some("ShipSelectionCommand"),
        vec![
            int("userId", 7),
            ("hitpoints", Value::F64(4321.0)),
            ("hitpointsMax", Value::F64(5000.0)),
            int("shield", 900),
            int("shieldMax", 1200),
            int("nanoHull", 77),
            int("maxNanoHull", 100),
        ],
    ));
    let e = &sw.world.entities[&7];
    assert_eq!(e.hitpoints, Some(4321.0));
    assert_eq!(e.hitpoints_max, Some(5000.0));
    assert_eq!(e.shield, Some(900));
    assert_eq!(e.shield_max, Some(1200));
    assert_eq!(sw.entity_nano.get(&7), Some(&(77, 100)));
}

// ----------------------------------------------------------- map transition

#[test]
fn map_changed_clears_per_map_state() {
    use xenomit_packet_client::model::{Collectable, CollectableKind, Entity, Poi};
    let mut sw = SharedWorld::default();
    sw.apply_packet(&hero_init(7, 100, 200, 300));
    sw.world.entities.insert(
        5,
        Entity {
            id: 5,
            ..Default::default()
        },
    );
    sw.world.collectables.insert(
        "h1".into(),
        Collectable {
            hash: "h1".into(),
            kind: CollectableKind::Box,
            x: 1,
            y: 2,
            type_id: 0,
            type_str: String::new(),
            pulse_color_id: 0,
            colour_code: 0,
            event_modifier: String::new(),
        },
    );
    sw.world.pois.insert(
        "poi".into(),
        Poi {
            poi_id: "poi".into(),
            poi_type: None,
            active: true,
            coordinates: vec![10, 20],
        },
    );
    sw.world.laser_links.insert(1, 5);
    sw.world.laser_active.insert(1);
    sw.tweens.insert(
        5,
        MoveTween {
            from: (0.0, 0.0),
            to: (50.0, 50.0),
            duration_ms: 1000,
            started: Instant::now(),
        },
    );
    sw.apply_packet(&jumpgate_create(9, 1, 4000, 3000));
    sw.entity_nano.insert(5, (10, 10));
    sw.apply_packet(&pkt(
        138,
        Some("MapChangedCommand"),
        vec![int("newMapId", 13)],
    ));
    // Every per-map table starts over; the hero row is re-seeded.
    assert_eq!(sw.world.entities.len(), 1);
    assert!(sw.world.entities.contains_key(&7));
    assert!(sw.world.collectables.is_empty());
    assert!(sw.world.pois.is_empty());
    assert!(sw.world.laser_links.is_empty());
    assert!(sw.world.laser_active.is_empty());
    assert!(sw.portals.is_empty());
    assert!(sw.tweens.is_empty());
    assert!(sw.entity_nano.is_empty());
    assert_eq!(sw.world.hero.as_ref().unwrap().map_id, 13);
}

#[test]
fn jump_initiated_arms_reconnect() {
    let mut sw = SharedWorld::default();
    assert!(!sw.expect_disconnect);
    sw.apply_packet(&pkt(
        19,
        Some("JumpInitiatedCommand"),
        vec![int("mapId", 16), int("gateId", 9)],
    ));
    assert!(sw.expect_disconnect);
    assert_eq!(sw.jump_target, Some(16));
    assert!(sw.portal_jumps.contains_key(&9));
    // JumpToServer (id 20) also flags the expected close.
    let mut sw = SharedWorld::default();
    sw.apply_packet(&pkt(
        id::JUMP_TO_SERVER,
        Some("JumpToServerCommand"),
        vec![],
    ));
    assert!(sw.expect_disconnect);
}

// --------------------------------------------------------------- kill screen

fn kill_option(repair_type: i64, cost: i64, affordable: bool) -> Value {
    Value::Module(Box::new(pkt(
        253,
        Some("KillScreenOptionModule"),
        vec![
            (
                "repairType",
                Value::Module(Box::new(pkt(
                    0,
                    None,
                    vec![int("repairTypeValue", repair_type)],
                ))),
            ),
            (
                "price",
                Value::Module(Box::new(pkt(
                    0,
                    None,
                    vec![int("amount", cost), int("type", 0)],
                ))),
            ),
            ("affordableForPlayer", Value::Bool(affordable)),
            int("cooldownTime", 0),
            (
                "buttonKey",
                Value::Module(Box::new(pkt(
                    0,
                    None,
                    vec![string("baseKey", "killscreen_option_basic")],
                ))),
            ),
        ],
    )))
}

#[test]
fn hero_destroyed_opens_kill_screen_and_post_fills_options() {
    use xenomit_client::net::world::KillScreen;
    let mut sw = SharedWorld::default();
    sw.apply_packet(&hero_init(7, 100, 200, 300));
    // In-game death (not the jump teardown — `expect_disconnect` unset).
    sw.apply_packet(&pkt(
        30,
        Some("ShipDestroyedCommand"),
        vec![int("destroyedUserId", 7)],
    ));
    assert!(sw.kill_screen.is_some());
    // The post command carries the killer + the repair option list.
    sw.apply_packet(&pkt(
        252,
        Some("KillScreenPostCommand"),
        vec![
            string("killerName", "-=[ Impulse II ]=-"),
            (
                "options",
                Value::List(vec![kill_option(1, 0, true), kill_option(4, 10, false)]),
            ),
        ],
    ));
    let ks = sw.kill_screen.as_ref().unwrap();
    assert_eq!(ks.killer_name, "-=[ Impulse II ]=-");
    assert_eq!(ks.options.len(), 2);
    assert_eq!(ks.options[0].repair_type, 1);
    assert_eq!(ks.options[0].cost, 0);
    assert!(ks.options[0].affordable);
    // Basic repair (free + affordable) is what R/B/the button send.
    assert_eq!(ks.best_repair_type(), KillScreen::BASIC_REPAIR);
}

#[test]
fn kill_screen_update_refreshes_options() {
    let mut sw = SharedWorld::default();
    sw.apply_packet(&pkt(
        6777,
        Some("KillScreenUpdateCommand"),
        vec![("options", Value::List(vec![kill_option(4, 10, true)]))],
    ));
    let ks = sw.kill_screen.as_ref().unwrap();
    assert_eq!(ks.options.len(), 1);
    // Only affordable option wins when no basic repair is offered.
    assert_eq!(ks.best_repair_type(), 4);
}

#[test]
fn kill_screen_best_repair_fallbacks() {
    use xenomit_client::net::world::{KillScreen, KillScreenOption};
    let opt = |t, c, a| KillScreenOption {
        repair_type: t,
        cost: c,
        affordable: a,
        ..Default::default()
    };
    // No options at all -> implied basic repair.
    assert_eq!(KillScreen::default().best_repair_type(), 1);
    // Basic on cooldown -> cheapest affordable alternative.
    let ks = KillScreen {
        killer_name: String::new(),
        options: vec![
            KillScreenOption {
                cooldown_ms: 60_000,
                ..opt(1, 0, true)
            },
            opt(4, 50, true),
            opt(5, 5, false),
        ],
    };
    assert_eq!(ks.best_repair_type(), 4);
}

// --------------------------------------------------------------- combat fx

#[test]
fn laser_run_queues_world_fx() {
    use xenomit_client::net::world::WorldFx;
    let mut sw = SharedWorld::default();
    sw.apply_packet(&pkt(
        id::ATTACK_LASER_RUN,
        Some("AttackLaserRunCommand"),
        vec![
            int("attackerId", 7),
            int("targetId", 5),
            int("laserColor", 2),
            ("skilledLaser", Value::Bool(false)),
        ],
    ));
    assert!(matches!(
        sw.fx.as_slice(),
        [WorldFx::LaserRun {
            attacker: 7,
            victim: 5,
            laser_type: 2,
            skilled: false,
            shield_damage: false
        }]
    ));
}

#[test]
fn emote_activation_queues_world_fx() {
    use xenomit_client::net::world::WorldFx;
    let mut sw = SharedWorld::default();
    sw.apply_packet(&pkt(
        id::EMOTE_ACTIVATION,
        Some("EmoteActivationCommand"),
        vec![
            int("userId", 42),
            string("lootId", "ammunition_specialammo_emote_alien-angry"),
        ],
    ));
    assert!(matches!(
        sw.fx.as_slice(),
        [WorldFx::Emote {
            entity: 42,
            ref loot_id
        }] if loot_id == "ammunition_specialammo_emote_alien-angry"
    ));
}

#[test]
fn jamx_activation_queues_ultimate_emp() {
    use xenomit_client::net::world::{EmpKind, WorldFx};
    let mut sw = SharedWorld::default();
    sw.apply_packet(&pkt(
        id::ABILITY_EFFECT_ACTIVATION,
        Some("AbilityEffectActivationCommand"),
        vec![int("selectedAbilityId", 110), int("activatorId", 42)],
    ));
    assert!(matches!(
        sw.fx.as_slice(),
        [WorldFx::Emp {
            target: 42,
            kind: EmpKind::Ultimate
        }]
    ));
    // A non-JAMX ability activation queues nothing.
    let mut sw = SharedWorld::default();
    sw.apply_packet(&pkt(
        id::ABILITY_EFFECT_ACTIVATION,
        Some("AbilityEffectActivationCommand"),
        vec![int("selectedAbilityId", 1), int("activatorId", 42)],
    ));
    assert!(sw.fx.is_empty());
}

#[test]
fn rocket_and_hit_and_miss_queue_world_fx() {
    use xenomit_client::net::world::WorldFx;
    let mut sw = SharedWorld::default();
    sw.apply_packet(&pkt(
        6,
        Some("AttackRocketCommand"),
        vec![
            int("attackerId", 7),
            int("targetId", 5),
            int("rocketType", 3),
            int("smokeId", 1),
            ("hit", Value::Bool(true)),
        ],
    ));
    sw.apply_packet(&pkt(
        id::ATTACK_HIT,
        Some("AttackHitCommand"),
        vec![
            int("attackerId", 7),
            int("victimId", 5),
            int("damage", 1234),
            (
                "attackType",
                Value::Module(Box::new(pkt(0, None, vec![int("attackTypeValue", 1)]))),
            ),
        ],
    ));
    sw.apply_packet(&pkt(
        163,
        Some("AttackMissedCommand"),
        vec![int("targetUserId", 5)],
    ));
    assert!(matches!(
        sw.fx[0],
        WorldFx::Rocket {
            attacker: 7,
            victim: 5,
            rocket_type: 3,
            hit: true,
            ..
        }
    ));
    assert!(matches!(
        sw.fx[1],
        WorldFx::Hit {
            victim: 5,
            damage: 1234,
            attack_type: 1,
            ..
        }
    ));
    assert!(matches!(sw.fx[2], WorldFx::Miss { victim: 5 }));
}

// ---------------------------------------------------------------- slot bar

/// `MenuInitializationCommand` — standard bar slot 1 = a real category
/// item (`ammunition_laser_lcb-10`, actionStyle 2, selected, counter
/// 1500) and slot 2 = an id the categories never list (→ empty).
fn menu_init_packet() -> Packet {
    let status = pkt(
        24049,
        Some("MenuItemStatusCommand"),
        vec![
            string("menuItemId", "ammunition_laser_lcb-10"),
            bool_("selected", true),
            bool_("available", true),
            bool_("activatable", true),
            bool_("visible", true),
            f64_("counterValue", 1500.0),
            f64_("maxCounterValue", 2000.0),
            string("iconLootId", "ammo_laser_lcb-10"),
        ],
    );
    let item = pkt(
        23633,
        Some("MenuItemInitializationCommand"),
        // Wire shape: no `menuItemId` here — only the nested status has it.
        vec![int("actionStyle", 2), ("status", module(status))],
    );
    let cat = pkt(
        -3461,
        Some("MenuCategoryInitializationCommand"),
        vec![
            string("categoryId", "laser"),
            list("menuItems", vec![module(item)]),
        ],
    );
    let bar = pkt(
        -620,
        Some("SlotBarInitializationCommand"),
        vec![
            string("slotBarId", "standardSlotBar"),
            bool_("visible", true),
            list(
                "slots",
                vec![
                    module(pkt(
                        -20408,
                        Some("SlotInfoCommand"),
                        vec![
                            int("slotId", 1),
                            string("menuItemId", "ammunition_laser_lcb-10"),
                        ],
                    )),
                    module(pkt(
                        -20408,
                        Some("SlotInfoCommand"),
                        vec![
                            int("slotId", 2),
                            string("menuItemId", "not_a_category_item"),
                        ],
                    )),
                ],
            ),
        ],
    );
    pkt(
        -8140,
        Some("MenuInitializationCommand"),
        vec![
            string("itemBarPosition", "bottom"),
            list("categories", vec![module(cat)]),
            list("slotBars", vec![module(bar)]),
        ],
    )
}

#[test]
fn menu_init_builds_items_and_slots() {
    use xenomit_client::net::world::{MenuActionStyle, STANDARD_SLOT_BAR};
    let mut sw = SharedWorld::default();
    sw.apply_packet(&menu_init_packet());
    let it = sw
        .menu
        .slot_item(STANDARD_SLOT_BAR, 1)
        .expect("slot 1 item");
    assert_eq!(it.id, "ammunition_laser_lcb-10");
    assert_eq!(it.category, "laser");
    assert_eq!(it.action_style, MenuActionStyle::Selection);
    assert!(it.selected);
    assert_eq!(it.counter_value, 1500.0);
    // Slot 2 references an id the categories never listed -> empty.
    assert!(sw.menu.slot_item(STANDARD_SLOT_BAR, 2).is_none());
    // Out-of-range slots.
    assert!(sw.menu.slot_item(STANDARD_SLOT_BAR, 0).is_none());
    assert!(sw.menu.slot_item(STANDARD_SLOT_BAR, 11).is_none());
}

#[test]
fn menu_status_updates_item_and_inserts_unknown() {
    use xenomit_client::net::world::{MenuActionStyle, STANDARD_SLOT_BAR};
    let mut sw = SharedWorld::default();
    sw.apply_packet(&menu_init_packet());
    // Standalone update replaces the status but keeps style/category.
    sw.apply_packet(&pkt(
        24049,
        Some("MenuItemStatusCommand"),
        vec![
            string("menuItemId", "ammunition_laser_lcb-10"),
            bool_("selected", false),
            bool_("available", true),
            bool_("blocked", false),
            f64_("counterValue", 42.0),
        ],
    ));
    let it = &sw.menu.items["ammunition_laser_lcb-10"];
    assert!(!it.selected);
    assert_eq!(it.counter_value, 42.0);
    assert_eq!(it.category, "laser");
    assert_eq!(it.action_style, MenuActionStyle::Selection);
    // An unknown id is inserted with default category/style.
    sw.apply_packet(&pkt(
        24049,
        Some("MenuItemStatusCommand"),
        vec![
            string("menuItemId", "skill_repair_boost"),
            bool_("selected", true),
        ],
    ));
    let it = &sw.menu.items["skill_repair_boost"];
    assert_eq!(it.category, "");
    assert_eq!(it.action_style, MenuActionStyle::OneShot);
    assert!(it.selected);
    // …but it does not claim a slot by itself.
    assert!(sw.menu.slot_item(STANDARD_SLOT_BAR, 2).is_none());
}

#[test]
fn clear_map_state_keeps_menu() {
    use xenomit_client::net::world::STANDARD_SLOT_BAR;
    let mut sw = SharedWorld::default();
    sw.apply_packet(&menu_init_packet());
    sw.clear_map_state();
    // Menu is session scope, not map scope — survives the reset.
    assert!(sw.menu.slot_item(STANDARD_SLOT_BAR, 1).is_some());
}

#[test]
fn menu_action_request_packet_encodes() {
    use xenomit_client::net::commands::{NetCommand, MENU_SOURCE_SLOT_BAR};
    use xenomit_packet_client::model::WorldState;
    let cmd = NetCommand::MenuAction {
        item_id: "ammunition_laser_lcb-10".to_owned(),
        action_type: 1,
        source_type: MENU_SOURCE_SLOT_BAR,
    };
    let p = cmd.to_packet(&WorldState::default()).expect("packet");
    assert_eq!(p.id, 8766);
    assert_eq!(p.int("actionType").unwrap(), 1);
    assert_eq!(p.int("sourceType").unwrap(), 1);
    assert_eq!(p.string("menuItemId").unwrap(), "ammunition_laser_lcb-10");
    xenomit_packet_client::packet::encode_body(&p).expect("encodes");
}

#[test]
fn menu_action_style_maps_wire_and_action_type() {
    use xenomit_client::net::world::MenuActionStyle;
    // Wire values (actionStyle 0..3; unknown -> one-shot).
    assert_eq!(MenuActionStyle::from_wire(0), MenuActionStyle::OneShot);
    assert_eq!(MenuActionStyle::from_wire(1), MenuActionStyle::Toggle);
    assert_eq!(MenuActionStyle::from_wire(2), MenuActionStyle::Selection);
    assert_eq!(MenuActionStyle::from_wire(3), MenuActionStyle::Timer);
    assert_eq!(MenuActionStyle::from_wire(99), MenuActionStyle::OneShot);
    // actionType: oneShot/selection -> ACTIVATE(1), toggle/timer -> SELECT(0).
    assert_eq!(MenuActionStyle::OneShot.action_type(), 1);
    assert_eq!(MenuActionStyle::Toggle.action_type(), 0);
    assert_eq!(MenuActionStyle::Selection.action_type(), 1);
    assert_eq!(MenuActionStyle::Timer.action_type(), 0);
}
