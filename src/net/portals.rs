use std::time::{Duration, Instant};

use xenomit_packet_client::modules::id;
use xenomit_packet_client::packet::Packet;

use crate::audio::{self, Sfx};
use crate::net::world::SharedWorld;

/// `JumpgateCreateCommand` — a portal joins the map (`designId` = LIB_PORTALS key).
pub(crate) const JUMPGATE_CREATE: i16 = 3389;
/// `JumpGateRemoveCommand`.
pub(crate) const JUMP_GATE_REMOVE: i16 = 4274;
/// `MapChangedCommand` — the map we sit on changed (same socket).
pub(crate) const MAP_CHANGED: i16 = 138;
/// `JumpInitiatedCommand` — portal jump accepted; the server closes the
/// socket on purpose (`Main.scheduledDisconnect` in the stock client).
pub(crate) const JUMP_INITIATED: i16 = 19;
/// `ActivatePortalCommand` holds `Portal.jump` this long before `jumpOff`.
const JUMP_HOLD_SECS: f32 = 10.0;
/// `jumpOn` / `jumpOff` tween (`abstract_portal` `time="1"`).
const JUMP_FADE_SECS: f32 = 1.0;
/// `PortalDecorator2D` plays `spawn_animation` only once the map has
/// already been up this long.
const SPAWN_AFTER: Duration = Duration::from_secs(10);

/// A jumpgate on the current map (`JumpgateCreateCommand`). `type_id`
/// is the `designId` — the bare numeric `LIB_PORTALS` asset key.
#[derive(Clone, Debug, Default)]
pub struct Portal {
    pub id: i32,
    pub type_id: String,
    pub x: i32,
    pub y: i32,
    pub faction_id: i32,
    pub on_minimap: bool,
    pub show_bubble: bool,
    /// When this gate joined the map. The highlight pulse is measured
    /// from here (`animation autoPlay` on `highlight`).
    pub created_at: Option<Instant>,
    /// Map lifetime was already over 10s — play `spawn_animation`
    /// instead of clearing `spawn_effect` immediately.
    pub spawn_anim: bool,
}

pub fn handle(world: &mut SharedWorld, p: &Packet) {
    match p.id {
        JUMPGATE_CREATE => {
            if let Ok(gate) = p.int("gateId") {
                if world.map_started.is_none() {
                    world.map_started = Some(Instant::now());
                }
                let late = world.map_started.is_some_and(|t| t.elapsed() > SPAWN_AFTER);
                let portal = Portal {
                    id: gate as i32,
                    type_id: p.int("designId").map(|d| d.to_string()).unwrap_or_default(),
                    x: p.int("x").map(|v| v as i32).unwrap_or_default(),
                    y: p.int("y").map(|v| v as i32).unwrap_or_default(),
                    faction_id: p.int("factionId").map(|v| v as i32).unwrap_or_default(),
                    on_minimap: p.boolean("isVisibleOnMinimap").unwrap_or(true),
                    show_bubble: p.boolean("showBubble").unwrap_or(false),
                    created_at: Some(Instant::now()),
                    spawn_anim: late,
                };
                if late {
                    audio::play_sfx_at(Sfx::PortalDone, portal.x as f32, portal.y as f32);
                }
                ::log::info!(
                    "portal {} design {} at ({},{}) minimap={}",
                    portal.id,
                    portal.type_id,
                    portal.x,
                    portal.y,
                    portal.on_minimap
                );
                world.portals.insert(portal.id, portal);
            }
        }
        JUMP_GATE_REMOVE => {
            if let Ok(gate) = p.int("gateId") {
                world.portals.remove(&(gate as i32));
                world.portal_jumps.remove(&(gate as i32));
            }
        }
        MAP_CHANGED => {
            if let Ok(map) = p.int("newMapId") {
                world.clear_map_state();
                if let Some(h) = &mut world.world.hero {
                    h.map_id = map as i32;
                }
                world.push_log(format!("map changed -> {map}"));
            }
        }
        JUMP_INITIATED => {
            world.expect_disconnect = true;
            let target_map = p.int("mapId").ok().map(|m| m as i32);
            world.jump_target = target_map;
            // `ActivatePortalCommand`: `Portal.jump(10)` on `gateId` —
            // fade in `active` + the jump movie clip, then fade them out.
            if let Ok(gate) = p.int("gateId") {
                world.portal_jumps.insert(gate as i32, Instant::now());
                // `Portal.jump` plays at the gate position.
                match world.portals.get(&(gate as i32)) {
                    Some(g) => {
                        audio::play_sfx_at(Sfx::PortalJump, g.x as f32, g.y as f32)
                    }
                    None => audio::play_sfx(Sfx::PortalJump),
                }
            }
            if let Some(m) = world.jump_target {
                world.push_log(format!("jump initiated -> map {m}"));
            } else {
                world.push_log("jump initiated".to_owned());
            }
        }
        id::JUMP_TO_SERVER => {
            world.expect_disconnect = true;
            world.push_log("jump to server".to_owned());
        }
        id::JUMP_DENIED => {
            world.push_log("jump denied".to_owned());
        }
        id::RELOGIN => {
            world.expect_disconnect = true;
            if let Ok(m) = p.int("mapID") {
                world.jump_target = Some(m as i32);
            }
            world.push_log("relogin requested".to_owned());
        }
        _ => {}
    }
}

/// `jumpOn` then hold, then `jumpOff`. 0 before and after the tween.
/// `started` is `JumpInitiatedCommand` time (`Portal.jump(10)`).
pub fn jump_strength(started: Instant, now: Instant) -> f32 {
    let t = now.saturating_duration_since(started).as_secs_f32();
    if t < JUMP_FADE_SECS {
        t / JUMP_FADE_SECS
    } else if t < JUMP_HOLD_SECS {
        1.0
    } else if t < JUMP_HOLD_SECS + JUMP_FADE_SECS {
        (1.0 - (t - JUMP_HOLD_SECS) / JUMP_FADE_SECS).max(0.0)
    } else {
        0.0
    }
}
