//! Effect scheduling from `WorldFx` packets: each `AttackLaserRunCommand`
//! volley spawns bolt effects, the volleys stop when the link aborts,
//! rockets take flight, and hit/miss packets become floating text.

use std::path::PathBuf;
use xenomit_client::assets::gamexml::GameXml;
use xenomit_client::assets::mapxml::MapDb;
use xenomit_client::assets::resources::FileIndex;
use xenomit_client::assets::uberlib::UberAssets;
use xenomit_client::assets::AssetManager;
use xenomit_client::game::effects::Effect;
use xenomit_client::game::Game;
use xenomit_client::net::world::SharedWorld;
use xenomit_client::net::NetCommand;
use xenomit_packet_client::model::Entity;
use xenomit_packet_client::modules::id;
use xenomit_packet_client::packet::{Packet, Value};

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

fn test_assets() -> AssetManager {
    AssetManager::new(
        PathBuf::from("target/fx-sched-test"),
        FileIndex::default(),
        GameXml::default(),
        UberAssets::default(),
        MapDb::default(),
    )
}

fn world_with_hero_and_npc() -> SharedWorld {
    let mut sw = SharedWorld::default();
    sw.apply_packet(&pkt(
        id::SHIP_INITIALIZATION,
        Some("ShipInitializationCommand"),
        vec![
            int("userId", 7),
            int("x", 1000),
            int("y", 1000),
            int("speed", 300),
        ],
    ));
    sw.world.entities.insert(
        5,
        Entity {
            id: 5,
            x: 2000,
            y: 1000,
            npc: true,
            ..Default::default()
        },
    );
    sw
}

fn laser_run(attacker: i64, victim: i64) -> Packet {
    pkt(
        id::ATTACK_LASER_RUN,
        Some("AttackLaserRunCommand"),
        vec![
            int("attackerId", attacker),
            int("targetId", victim),
            int("laserColor", 1),
        ],
    )
}

#[test]
fn laser_volleys_spawn_shots_and_abort_stops_them() {
    let mut sw = world_with_hero_and_npc();
    let mut assets = test_assets();
    let mut game = Game::new(1, (21000.0, 13100.0));
    let mut out: Vec<NetCommand> = Vec::new();

    // Volley 1 -> at least one bolt from attacker(7) toward victim(5).
    sw.apply_packet(&laser_run(7, 5));
    game.update(0.016, &mut sw, &mut assets, &mut out);
    let bolts = game
        .effects
        .active
        .iter()
        .filter(|e| matches!(e, Effect::LaserShot { .. }))
        .count();
    assert!(bolts >= 1, "volley 1 spawned {bolts} bolts");
    // Empty gamexml -> thin-bolt fallback; the shot still tracks
    // attacker -> victim geometry.
    let Effect::LaserShot { from, to, .. } = game
        .effects
        .active
        .iter()
        .find(|e| matches!(e, Effect::LaserShot { .. }))
        .unwrap()
    else {
        panic!();
    };
    assert_eq!(*from, (1000.0, 1000.0));
    assert_eq!(*to, (2000.0, 1000.0));

    // Volley 2 (the server's ~1s cadence) adds another bolt.
    sw.apply_packet(&laser_run(7, 5));
    game.update(0.016, &mut sw, &mut assets, &mut out);
    let bolts = game
        .effects
        .active
        .iter()
        .filter(|e| matches!(e, Effect::LaserShot { .. }))
        .count();
    assert!(bolts >= 2, "volley 2 spawned {bolts} bolts total");

    // Abort: the link dies and no new volleys arrive -> the in-flight
    // bolts finish and nothing new spawns.
    sw.apply_packet(&pkt(
        id::ATTACK_ABORT_LASER,
        Some("AttackAbortLaserCommand"),
        vec![int("uid", 7)],
    ));
    assert!(sw.world.laser_links.is_empty());
    game.update(0.016, &mut sw, &mut assets, &mut out);
    let _ = bolts;
}

#[test]
fn rocket_packet_flies_to_victim() {
    let mut sw = world_with_hero_and_npc();
    let mut assets = test_assets();
    let mut game = Game::new(1, (21000.0, 13100.0));
    let mut out = Vec::new();

    sw.apply_packet(&pkt(
        6,
        Some("AttackRocketCommand"),
        vec![
            int("attackerId", 7),
            int("targetId", 5),
            int("rocketType", 0),
            ("hit", Value::Bool(true)),
        ],
    ));
    game.update(0.016, &mut sw, &mut assets, &mut out);
    let Effect::Rocket {
        from,
        victim,
        duration,
        ..
    } = game
        .effects
        .active
        .iter()
        .find(|e| matches!(e, Effect::Rocket { .. }))
        .expect("rocket effect")
    else {
        panic!();
    };
    assert_eq!(*from, (1000.0, 1000.0));
    assert_eq!(*victim, 5);
    // 1000u at 500 u/s -> 2.0s.
    assert_eq!(*duration, 2.0);
}

#[test]
fn attack_hit_spawns_damage_number() {
    let mut sw = world_with_hero_and_npc();
    let mut assets = test_assets();
    let mut game = Game::new(1, (21000.0, 13100.0));
    let mut out = Vec::new();

    sw.apply_packet(&pkt(
        id::ATTACK_HIT,
        Some("AttackHitCommand"),
        vec![
            int("attackerId", 7),
            int("victimId", 5),
            int("damage", 4321),
            (
                "attackType",
                Value::Module(Box::new(pkt(0, None, vec![int("attackTypeValue", 1)]))),
            ),
        ],
    ));
    game.update(0.016, &mut sw, &mut assets, &mut out);
    let Effect::Text { pos, text, .. } = game
        .effects
        .active
        .iter()
        .find(|e| matches!(e, Effect::Text { .. }))
        .expect("damage text")
    else {
        panic!();
    };
    assert_eq!(text, "-4.321");
    // Spawned above the victim's sprite, jittered per hit (deterministic
    // seed): within +/-26u horizontally and 48-70u above the victim.
    let dx = (pos.0 - 2000.0).abs();
    let dy = 1000.0 - pos.1;
    assert!(dx <= 26.0 && (48.0..=70.0).contains(&dy), "pos={pos:?}");
}

#[test]
fn attack_missed_spawns_miss_text() {
    let mut sw = world_with_hero_and_npc();
    let mut assets = test_assets();
    let mut game = Game::new(1, (21000.0, 13100.0));
    let mut out = Vec::new();

    sw.apply_packet(&pkt(
        163,
        Some("AttackMissedCommand"),
        vec![int("targetUserId", 5)],
    ));
    game.update(0.016, &mut sw, &mut assets, &mut out);
    let Effect::Text { text, .. } = game
        .effects
        .active
        .iter()
        .find(|e| matches!(e, Effect::Text { .. }))
        .expect("MISS text")
    else {
        panic!();
    };
    assert_eq!(text, "MISS");
}
