//! Player orders: move, collect, combat toggles, hit-testing.

use super::input::{self, Hit};
use super::Game;
use crate::net::{NetCommand, SharedWorld};

/// Click radius for collectables (screen px, divided by zoom to world units).
const COLLECT_CLICK_RADIUS: f32 = 90.0;
/// Click radius for portals (screen px, divided by zoom) — gates are big.
const PORTAL_CLICK_RADIUS: f32 = 160.0;

impl Game {
    /// Hit-test a world point: entities first (nearest inside its click
    /// radius), then collectables. Radii are stored in screen px and
    /// converted to world units via the current zoom.
    pub(crate) fn hit_test(&self, world: &SharedWorld, w: (f32, f32)) -> Hit {
        let hero_id = world.world.hero.as_ref().map(|h| h.user_id);
        let zoom = self.camera.zoom.clamp(0.4, 2.5);
        let mut items: Vec<input::PickItem> = Vec::with_capacity(world.world.entities.len());
        for e in world.world.entities.values() {
            if Some(e.id) == hero_id || e.destroyed {
                continue;
            }
            let p = world.pos(e.id);
            let radius_px = self
                .entity_visuals
                .get(&e.id)
                .map(|v| v.click_radius)
                .unwrap_or(input::DEFAULT_CLICK_RADIUS);
            items.push((e.id, p.0, p.1, radius_px / zoom));
        }
        if let Some(id) = input::pick_entity(&items, w) {
            return Hit::Entity(id);
        }
        let r = COLLECT_CLICK_RADIUS / zoom;
        let mut cols: Vec<(String, f32, f32)> = Vec::with_capacity(world.world.collectables.len());
        for c in world.world.collectables.values() {
            // The drawn box sits at the recipe's `click_area` pivot below
            // the server pos — hit-test the pivot (`UberAsset2D.
            // getClickArea`), not the model position.
            let cp = self
                .collectable_visuals
                .get(&c.hash)
                .map(|v| v.click_pivot)
                .unwrap_or((0.0, 0.0));
            cols.push((c.hash.clone(), c.x as f32 + cp.0, c.y as f32 + cp.1));
        }
        if let Some(hash) = input::pick_collectable(&cols, w, r) {
            return Hit::Collectable(hash);
        }
        let r = PORTAL_CLICK_RADIUS / zoom;
        let mut gates: Vec<(i32, f32, f32)> = Vec::with_capacity(world.portals.len());
        for p in world.portals.values() {
            gates.push((p.id, p.x as f32, p.y as f32));
        }
        if let Some(id) = input::pick_portal(&gates, w, r) {
            return Hit::Portal(id);
        }
        Hit::Empty
    }

    /// Move order toward `target`: sets the destination marker, drops any
    /// outstanding collect order and queues the `NetCommand::Move`.
    pub(crate) fn order_move(&mut self, target: (f32, f32), out: &mut Vec<NetCommand>) {
        self.hero_dest = Some(target);
        self.pending_collect = None;
        out.push(NetCommand::Move {
            x: target.0.round() as i32,
            y: target.1.round() as i32,
        });
    }

    /// Reissue a movement destination while the mouse is held. The network
    /// layer gives these updates a shorter pacing interval than one-shot moves.
    pub(crate) fn order_held_move(&mut self, target: (f32, f32), out: &mut Vec<NetCommand>) {
        self.hero_dest = Some(target);
        self.pending_collect = None;
        out.push(NetCommand::MoveHeld {
            x: target.0.round() as i32,
            y: target.1.round() as i32,
        });
    }

    /// Toggle the laser on the selected target (stock `Ctrl`): abort when
    /// our laser is already running on it, attack otherwise.
    pub(crate) fn toggle_laser(&mut self, world: &SharedWorld, out: &mut Vec<NetCommand>) {
        let Some(id) = self.selected else {
            return;
        };
        let hero_id = world.world.hero.as_ref().map(|h| h.user_id);
        let running = hero_id.is_some_and(|h| world.world.laser_links.get(&h) == Some(&id));
        if running {
            out.push(NetCommand::AbortLaser);
        } else {
            out.push(NetCommand::AttackLaser { target_id: id });
        }
    }

    /// Activate a `standardSlotBar` slot (`ActivateSlotCommand` /
    /// `SendMenuActionRequestCommand`): `MenuActionRequest` with the
    /// item's `actionType` from its `actionStyle`, `sourceType` =
    /// SLOT_BAR(1). `slot` is 1-based; empty/unknown slots do nothing.
    pub(crate) fn activate_slot(
        &self,
        world: &SharedWorld,
        slot: usize,
        out: &mut Vec<NetCommand>,
    ) {
        let Some(item) = world
            .menu
            .slot_item(crate::net::world::STANDARD_SLOT_BAR, slot)
        else {
            return;
        };
        out.push(NetCommand::MenuAction {
            item_id: item.id.clone(),
            action_type: item.action_style.action_type(),
            source_type: crate::net::commands::MENU_SOURCE_SLOT_BAR,
        });
    }

    /// Issue the walk-then-collect order for a box/ore click: move to the
    /// collectable now, send `Collect`/`Harvest` once the move finishes
    /// (stock `handleMoveComplete`), or when we're already in range.
    pub(crate) fn order_collect(
        &mut self,
        hash: &str,
        world: &SharedWorld,
        out: &mut Vec<NetCommand>,
    ) {
        let Some(c) = world.world.collectables.get(hash) else {
            return;
        };
        let pos = (c.x as f32, c.y as f32);
        self.hero_dest = Some(pos);
        out.push(NetCommand::Move { x: c.x, y: c.y });
        self.pending_collect = Some(crate::game::PendingCollect {
            hash: hash.to_owned(),
            kind: c.kind,
            pos,
            armed: false,
        });
    }

    /// `Tab` target: the nearest NPC after `tab_cycle` — entities are
    /// sorted by distance and the cycle pointer walks the list, so
    /// repeat presses iterate nearest-first. Never selects players.
    pub(crate) fn next_npc_target(&self, world: &SharedWorld) -> Option<i32> {
        let hero_id = world.world.hero.as_ref().map(|h| h.user_id);
        let hero_pos = world.hero_pos().unwrap_or((0.0, 0.0));
        let mut npcs: Vec<(i64, i32)> = world
            .world
            .entities
            .values()
            .filter(|e| e.npc && !e.destroyed && Some(e.id) != hero_id)
            .map(|e| {
                let p = world.pos(e.id);
                let dx = (p.0 - hero_pos.0) as i64;
                let dy = (p.1 - hero_pos.1) as i64;
                (dx * dx + dy * dy, e.id)
            })
            .collect();
        npcs.sort();
        npcs.dedup_by_key(|(_, id)| *id);
        if npcs.is_empty() {
            return None;
        }
        let after = self.tab_cycle.or(self.selected);
        let pos = after
            .and_then(|last| npcs.iter().position(|(_, id)| *id == last))
            .map(|i| (i + 1) % npcs.len())
            .unwrap_or(0);
        Some(npcs[pos].1)
    }
}
