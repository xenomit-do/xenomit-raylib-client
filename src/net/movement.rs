use std::time::Instant;
use xenomit_packet_client::modules::{self, id};
use xenomit_packet_client::packet::Packet;

use crate::net::util::log_once;
use crate::net::world::SharedWorld;

/// Interpolation anchor for a moving entity.
#[derive(Clone, Debug)]
pub struct MoveTween {
    pub from: (f32, f32),
    pub to: (f32, f32),
    pub duration_ms: i64,
    pub started: Instant,
}

impl MoveTween {
    /// Linearly interpolated position at `now`, clamped to `[from, to]`.
    /// A non-positive duration snaps straight to `to`.
    pub fn pos_at(&self, now: Instant) -> (f32, f32) {
        let t = if self.duration_ms <= 0 {
            1.0
        } else {
            let elapsed = now.saturating_duration_since(self.started).as_secs_f64() * 1000.0;
            (elapsed / self.duration_ms as f64).clamp(0.0, 1.0)
        } as f32;
        (
            self.from.0 + (self.to.0 - self.from.0) * t,
            self.from.1 + (self.to.1 - self.from.1) * t,
        )
    }

    /// Position right now.
    pub fn pos(&self) -> (f32, f32) {
        self.pos_at(Instant::now())
    }

    /// True once the tween has fully arrived at `to`.
    pub fn done(&self) -> bool {
        self.duration_ms <= 0
            || self.started.elapsed().as_secs_f64() * 1000.0 >= self.duration_ms as f64
    }
}

/// Fallback tween duration for hero moves when speed is unknown/invalid.
const HERO_MOVE_DEFAULT_MS: i64 = 400;
/// While flying, tolerate ~`speed * (beacon interval + RTT)` units of
/// predicted-vs-beacon drift before retargeting (`HERO_RESYNC_DIST` is
/// the floor for slow/zero-speed heroes).
const HERO_RESYNC_DIST: f32 = 300.0;
/// Pull-back protection cap: a beacon *behind* the predicted pos on the
/// current course is a lagging server update, not a correction, even
/// when the raw drift exceeds the resync threshold.
const HERO_LAG_FACTOR: f32 = 3.0;
/// Idle beacon corrections: below this, settle the model silently…
const BEACON_SETTLE_DIST: f32 = 5.0;
/// …and up to this, glide to the beacon pos over a short tween.
const BEACON_SMOOTH_DIST: f32 = 300.0;
/// Duration of the idle smoothing tween.
const BEACON_SMOOTH_MS: i64 = 250;

/// Resync threshold for an active hero tween — beacons arrive ~once a
/// second, so expected lag is roughly `speed * (interval + RTT)`.
fn hero_resync_dist(speed: i32) -> f32 {
    (speed as f32 * 1.2).max(HERO_RESYNC_DIST)
}

/// Milliseconds a hero tween should take for `from`→`to` at `speed`
/// (world units/s); `HERO_MOVE_DEFAULT_MS` when the speed is unknown or
/// non-positive. Minimum 1 so `done()` can't trip on arrival frames.
pub fn hero_move_ms(speed: i32, from: (f32, f32), to: (f32, f32)) -> i64 {
    let dist = ((to.0 - from.0) as f64).powi(2) + ((to.1 - from.1) as f64).powi(2);
    let ms = if speed > 0 {
        (dist.sqrt() / speed as f64 * 1000.0) as i64
    } else {
        HERO_MOVE_DEFAULT_MS
    };
    ms.max(1)
}

/// Deferred tween/snap work from movement packets — applied after
/// `world.apply` in `SharedWorld::apply_packet`.
pub struct MovementEffect {
    pub new_tween: Option<(i32, MoveTween)>,
    pub hero_snap: Option<(i32, i32)>,
}

/// Sample movement tweens *before* `world.apply` mutates positions.
pub fn handle_pre(world: &mut SharedWorld, p: &Packet, now: Instant) -> Option<MovementEffect> {
    let mut effect = MovementEffect {
        new_tween: None,
        hero_snap: None,
    };
    match p.id {
        id::MOVE_COMMAND => {
            match modules::parse_move_command(p) {
                Some((uid, (x, y), time_ms)) => {
                    let from = world.pos(uid);
                    effect.new_tween = Some((
                        uid,
                        MoveTween {
                            from,
                            to: (x as f32, y as f32),
                            duration_ms: time_ms as i64,
                            started: now,
                        },
                    ));
                }
                None => log_once("MoveCommand parse failed", format!("{p:?}")),
            }
            Some(effect)
        }
        id::HERO_MOVE => {
            match modules::parse_hero_move(p) {
                Some((x, y)) => {
                    if let Some(h) = &world.world.hero {
                        let uid = h.user_id;
                        let from = world.pos(uid);
                        effect.new_tween = Some((
                            uid,
                            MoveTween {
                                from,
                                to: (x as f32, y as f32),
                                duration_ms: hero_move_ms(h.speed, from, (x as f32, y as f32)),
                                started: now,
                            },
                        ));
                    }
                }
                None => log_once("HeroMoveCommand parse failed", format!("{p:?}")),
            }
            Some(effect)
        }
        id::BEACON_COMMAND => {
            if let (Ok(bx), Ok(by)) = (p.int("positionX"), p.int("positionY")) {
                if let Some(h) = &world.world.hero {
                    let uid = h.user_id;
                    let speed = h.speed;
                    let beacon = (bx as f32, by as f32);
                    let predicted = world.pos(uid);
                    let drift = ((predicted.0 - beacon.0).powi(2)
                        + (predicted.1 - beacon.1).powi(2))
                    .sqrt();
                    let resync = hero_resync_dist(speed);
                    let tween = world.tweens.get(&uid);
                    let active = tween.is_some_and(|t| !t.done());
                    let lagging = tween.is_some_and(|t| {
                        let dx = t.to.0 - t.from.0;
                        let dy = t.to.1 - t.from.1;
                        let len = (dx * dx + dy * dy).sqrt();
                        if len <= 1e-3 {
                            return false;
                        }
                        let (ux, uy) = (dx / len, dy / len);
                        let bs = (beacon.0 - t.from.0) * ux + (beacon.1 - t.from.1) * uy;
                        let ps = (predicted.0 - t.from.0) * ux + (predicted.1 - t.from.1) * uy;
                        let lateral =
                            ((beacon.0 - t.from.0) * uy - (beacon.1 - t.from.1) * ux).abs();
                        bs < ps && lateral <= resync
                    });
                    let prev = world.hero_beacon;
                    world.hero_beacon = Some(beacon);
                    let converging = prev.is_none_or(|pb| {
                        let dx = beacon.0 - pb.0;
                        let dy = beacon.1 - pb.1;
                        dx * dx + dy * dy > 25.0
                    });
                    let action;
                    if lagging && converging && drift <= resync * HERO_LAG_FACTOR {
                        action = "lag";
                    } else if active && drift <= resync {
                        action = "leave";
                    } else if active {
                        let to = tween.map(|t| t.to).unwrap_or(beacon);
                        action = "retarget";
                        effect.new_tween = Some((
                            uid,
                            MoveTween {
                                from: beacon,
                                to,
                                duration_ms: hero_move_ms(speed, beacon, to),
                                started: now,
                            },
                        ));
                        effect.hero_snap = Some((bx as i32, by as i32));
                    } else if drift < BEACON_SETTLE_DIST {
                        action = "set";
                        world.tweens.remove(&uid);
                        effect.hero_snap = Some((bx as i32, by as i32));
                    } else if drift <= BEACON_SMOOTH_DIST {
                        action = "smooth";
                        effect.new_tween = Some((
                            uid,
                            MoveTween {
                                from: predicted,
                                to: beacon,
                                duration_ms: BEACON_SMOOTH_MS,
                                started: now,
                            },
                        ));
                        effect.hero_snap = Some((bx as i32, by as i32));
                    } else {
                        action = "snap";
                        world.tweens.remove(&uid);
                        effect.hero_snap = Some((bx as i32, by as i32));
                    }
                    let near_gate = p.boolean("nearJumpGate").unwrap_or(false);
                    // `BeaconHandler` — the hero's `RepairRobotEffect`
                    // follows the beacon flags (`addMapObjectEffectOnce`
                    // dedupes; empty loot id removes it).
                    world.fx.push(crate::net::combat::WorldFx::RepairBot {
                        active: p.boolean("repairRobotActive").unwrap_or(false),
                        loot_id: p
                            .string("repairRobotLootId")
                            .unwrap_or_default()
                            .to_owned(),
                        skilled: p.boolean("repairBotSkilled").unwrap_or(false),
                    });
                    ::log::debug!(
                        "beacon pos=({bx},{by}) predicted=({:.0},{:.0}) \
                         drift={drift:.0} active={active} action={action} \
                         gate={near_gate}",
                        predicted.0,
                        predicted.1
                    );
                }
            }
            Some(effect)
        }
        _ => None,
    }
}
