//! Offline protocol replay: feeds a real captured session's inbound frames
//! through `packet::decode_body` + `WorldState::apply` and asserts the world
//! model accumulates entities/collectables. Catches schema drift without a
//! live server.
//!
//! Fixture: `tests/fixtures/live_capture.jsonl` — one JSON object per frame
//! with `direction` (`in`/`out`) and `body_hex` (frame body, no 3-byte len).

use std::collections::BTreeMap;
use xenomit_packet_client::model::WorldState;
use xenomit_packet_client::packet::decode_body;

#[derive(serde::Deserialize)]
struct Captured {
    direction: String,
    body_hex: String,
}

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

#[test]
fn replay_capture_builds_world() {
    let raw = include_str!("fixtures/live_capture.jsonl");
    let mut world = WorldState::default();
    let mut decoded: BTreeMap<i16, u32> = BTreeMap::new();
    let mut failures: BTreeMap<i16, u32> = BTreeMap::new();
    let mut total_in = 0u32;

    for line in raw.lines() {
        let msg: Captured = serde_json::from_str(line).unwrap();
        if msg.direction != "in" {
            continue;
        }
        total_in += 1;
        let body = unhex(&msg.body_hex);
        if body.len() < 2 {
            continue;
        }
        match decode_body(&body, false) {
            Ok(p) => {
                *decoded.entry(p.id).or_default() += 1;
                world.apply(&p);
            }
            Err(_) => {
                // Module id is the first i16 of the body.
                let id = i16::from_be_bytes([body[0], body[1]]);
                *failures.entry(id).or_default() += 1;
            }
        }
    }

    let decoded_total: u32 = decoded.values().sum();
    let failed_total: u32 = failures.values().sum();
    eprintln!(
        "decoded {decoded_total}/{total_in} frames across {} modules; \
         {failed_total} undecodable: {failures:?}",
        decoded.len()
    );
    eprintln!(
        "world: {} entities, {} collectables, hero={:?}",
        world.entities.len(),
        world.collectables.len(),
        world.hero.as_ref().map(|h| (h.user_id, h.x, h.y))
    );

    // The 1839-frame capture should be mostly decodable; log what isn't so
    // schema gaps surface instead of silently dropping gameplay packets.
    assert!(
        decoded_total > total_in * 7 / 10,
        "decoded {decoded_total}/{total_in} — decode coverage regressed"
    );
    assert!(
        !world.entities.is_empty() || !world.collectables.is_empty(),
        "replay produced an empty world — apply() wiring broken"
    );
}
