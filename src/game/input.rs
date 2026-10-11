//! Mouse/keyboard -> `NetCommand` translation (`keyboardManager`,
//! `MouseHitAreaTrait` equivalents).
//!
//! This module is the *pure* decision layer — no raylib types. The raylib
//! polling lives in `crate::game::Game::poll_input`; everything here is
//! deterministic and unit-testable.

/// Result of a world-space click.
#[derive(Clone, Debug, PartialEq)]
pub enum ClickAction {
    MoveTo(f32, f32),
    SelectEntity(i32),
    AttackEntity(i32),
    /// `Collect{hash}` — caller decides `Harvest` for ores.
    CollectEntity(String),
    JumpPortal,
    Nothing,
}

/// What sits under a world-space click, resolved by the caller.
#[derive(Clone, Debug, PartialEq)]
pub enum Hit {
    /// Empty space — becomes a `Move` order.
    Empty,
    /// A ship/asset entity by id.
    Entity(i32),
    /// A collectable by its server hash.
    Collectable(String),
    /// A jumpgate (`SharedWorld::portals` gateId).
    Portal(i32),
}

/// One candidate for entity picking: `(id, x, y, click_radius)`.
pub type PickItem = (i32, f32, f32, f32);

/// Default click radius (world units) when a pattern doesn't define one.
pub const DEFAULT_CLICK_RADIUS: f32 = 80.0;

/// Decide a left-click action.
///
/// - `hit`: what the hit-test found under the cursor.
/// - `w`: world coordinates of the click.
/// - `bounds`: map bounds used to clamp `MoveTo`.
/// - `selected`: currently selected entity id — clicking it again attacks.
pub fn decide_click(
    hit: Hit,
    w: (f32, f32),
    bounds: (f32, f32),
    selected: Option<i32>,
) -> ClickAction {
    match hit {
        Hit::Entity(id) => {
            if selected == Some(id) {
                ClickAction::AttackEntity(id)
            } else {
                ClickAction::SelectEntity(id)
            }
        }
        Hit::Collectable(hash) => ClickAction::CollectEntity(hash),
        // A portal hit only tells us which kind of map object was under
        // the cursor. Preserve the actual click position as the movement
        // target instead of snapping to the portal's center.
        Hit::Portal(_) | Hit::Empty => ClickAction::MoveTo(
            w.0.clamp(0.0, bounds.0.max(0.0)),
            w.1.clamp(0.0, bounds.1.max(0.0)),
        ),
    }
}

/// Pick the nearest entity whose click radius covers `w`.
/// `items` is a flat list of `(id, x, y, radius)` — ships and assets alike.
pub fn pick_entity(items: &[PickItem], w: (f32, f32)) -> Option<i32> {
    let mut best: Option<(i32, f32)> = None;
    for &(id, x, y, r) in items {
        let dx = w.0 - x;
        let dy = w.1 - y;
        let d2 = dx * dx + dy * dy;
        if d2 <= r * r && best.map(|(_, bd)| d2 < bd).unwrap_or(true) {
            best = Some((id, d2));
        }
    }
    best.map(|(id, _)| id)
}

/// Pick the nearest portal within `radius` world units.
/// `items`: `(gate_id, x, y)`.
pub fn pick_portal(items: &[(i32, f32, f32)], w: (f32, f32), radius: f32) -> Option<i32> {
    let mut best: Option<(i32, f32)> = None;
    for &(id, x, y) in items {
        let dx = w.0 - x;
        let dy = w.1 - y;
        let d2 = dx * dx + dy * dy;
        if d2 <= radius * radius && best.map(|(_, bd)| d2 < bd).unwrap_or(true) {
            best = Some((id, d2));
        }
    }
    best.map(|(id, _)| id)
}

/// Name-plate color class — the stock client's red/blue scheme:
/// NPCs always read as hostiles, own company is blue, everyone else red.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlateKind {
    /// NPC — `-=[ name ]=-`, red.
    Npc,
    /// Same faction as the hero — blue.
    Ally,
    /// Player of another faction — red.
    Enemy,
    /// No faction information (e.g. neutral assets) — dim grey.
    Neutral,
}

/// Classify an entity's name plate from its faction vs the hero's.
/// `faction_id == 0` is "no faction" and lands on [`PlateKind::Neutral`]
/// rather than pretending it is an enemy.
pub fn plate_kind(npc: bool, faction_id: i32, hero_faction: i32) -> PlateKind {
    if npc {
        PlateKind::Npc
    } else if faction_id == 0 {
        PlateKind::Neutral
    } else if hero_faction != 0 && faction_id == hero_faction {
        PlateKind::Ally
    } else {
        PlateKind::Enemy
    }
}

// ---------------------------------------------------------------------
// HUD geometry — shared between `poll_input` (click swallowing) and
// `render::hud` (drawing) so the two never disagree on a panel's rect.
// ---------------------------------------------------------------------

/// Screen-space rectangle `(x, y, w, h)` in pixels.
pub type Rect = (f32, f32, f32, f32);

/// Touch-first controls shown over the game when the mobile HUD preference
/// is enabled. The geometry is relative to the viewport, so it also works
/// as a desktop preview at arbitrary window sizes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MobileControl {
    MoveStick,
    Target,
    Laser,
    Rocket,
    Jump,
    Pet,
    Radar,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MobileHudLayout {
    pub move_stick: (f32, f32, f32),
    pub target: Rect,
    pub laser: Rect,
    pub rocket: Rect,
    pub jump: Rect,
    pub pet: Rect,
    pub radar: (f32, f32, f32),
}

pub fn mobile_hud_layout(vw: f32, vh: f32) -> MobileHudLayout {
    let unit = vw.min(vh);
    let stick_r = (unit * 0.20).clamp(82.0, 196.0);
    let button = (unit * 0.078).clamp(44.0, 74.0);
    let left = (vw * 0.22).clamp(stick_r + 12.0, (vw - stick_r - 12.0).max(stick_r + 12.0));
    let right = vw * 0.88;
    let cy = vh * 0.74;
    let rect = |x: f32, y: f32| (x - button * 0.5, y - button * 0.5, button, button);
    let action_r = stick_r;
    let radar_r = (unit * 0.10).clamp(46.0, 96.0);
    MobileHudLayout {
        move_stick: (left, cy, stick_r),
        target: rect(right - action_r * 0.70, cy - action_r * 0.55),
        laser: rect(right, cy),
        rocket: rect(right - action_r * 0.78, cy + action_r * 0.20),
        jump: rect(right - action_r * 0.48, cy + action_r * 0.82),
        pet: rect(right + action_r * 0.42, cy + action_r * 0.62),
        radar: (vw - radar_r - 20.0, radar_r + 20.0, radar_r),
    }
}

pub fn mobile_hud_hit(layout: &MobileHudLayout, p: (f32, f32)) -> Option<MobileControl> {
    let (rx, ry, rr) = layout.radar;
    if (p.0 - rx).hypot(p.1 - ry) <= rr {
        return Some(MobileControl::Radar);
    }
    let (sx, sy, sr) = layout.move_stick;
    if (p.0 - sx).hypot(p.1 - sy) <= sr * 1.15 {
        return Some(MobileControl::MoveStick);
    }
    [
        (MobileControl::Target, layout.target),
        (MobileControl::Laser, layout.laser),
        (MobileControl::Rocket, layout.rocket),
        (MobileControl::Jump, layout.jump),
        (MobileControl::Pet, layout.pet),
    ]
    .into_iter()
    .find_map(|(control, rect)| point_in(rect, p).then_some(control))
}

/// Keep a captured finger stable while additional touches arrive. Raylib
/// may reorder active touch points between frames, so callers retain the id
/// instead of relying on index zero.
pub fn retain_touch_id(captured: Option<i32>, active_ids: &[i32]) -> Option<i32> {
    captured.filter(|id| active_ids.contains(id))
}

pub fn mobile_toolbar_toggle_rect(vw: f32, collapsed: bool) -> Rect {
    if collapsed {
        (10.0, 12.0, 38.0, 38.0)
    } else {
        let buttons = crate::ui::toolbar::toolbar_bounds();
        (
            (buttons.0 + buttons.2 + 8.0).min((vw - 44.0).max(10.0)),
            buttons.1 + 2.0,
            34.0,
            34.0,
        )
    }
}

/// Where an ammo flyout hangs from.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FlyoutSource {
    /// The action bar's laser slot.
    Laser,
    /// The action bar's rocket slot.
    Rocket,
}

/// What a HUD click lands on — used to route/swallow mouse input.
/// (Windows — ship, user, minimap, … — are routed by `ui::WindowManager`
/// *before* this.)
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HudArea {
    /// Decorative panel — swallows the click, does nothing.
    Panel,
    /// Action-bar laser slot — toggles AttackLaser/AbortLaser.
    LaserSlot,
    /// Action-bar rocket slot — fires AttackRocket.
    RocketSlot,
    /// Row `i` of the open ammo flyout.
    FlyoutItem(usize),
}

/// Ammo flyout: a column of `(menuItemId, rect)` rows over a panel.
#[derive(Clone, Debug, Default)]
pub struct FlyoutLayout {
    pub panel: Rect,
    pub rows: Vec<(String, Rect)>,
}

/// Flyout row size in px.
pub const FLYOUT_ROW: (f32, f32) = (160.0, 26.0);

/// Panel rects for one frame: action bar bottom-center, target panel
/// top-center. Ship/user/minimap/quests/... live in `ui::WindowManager`.
#[derive(Clone, Debug, Default)]
pub struct HudLayout {
    /// `None` when nothing is selected.
    pub target_panel: Option<Rect>,
    pub action_bar: Rect,
    pub laser_slot: Rect,
    pub rocket_slot: Rect,
    /// Open ammo flyout, if any.
    pub flyout: Option<FlyoutLayout>,
}

impl HudLayout {
    /// Rect a flyout hangs from.
    pub fn source_rect(&self, src: FlyoutSource) -> Option<Rect> {
        match src {
            FlyoutSource::Laser => Some(self.laser_slot),
            FlyoutSource::Rocket => Some(self.rocket_slot),
        }
    }

    /// Attach a flyout listing `items` above/below `src`.
    pub fn with_flyout(mut self, src: FlyoutSource, items: &[String], vw: f32, vh: f32) -> Self {
        if let Some(anchor) = self.source_rect(src) {
            if !items.is_empty() {
                self.flyout = Some(flyout_layout(anchor, items, vw, vh));
            }
        }
        self
    }
}

/// Screen margin for the corner-anchored panels.
pub const HUD_MARGIN: f32 = 10.0;
/// Action-bar slot edge in px.
pub const SLOT: f32 = 40.0;

/// Flyout panel for `items` hanging above `anchor` (below when there is
/// no room), kept inside the viewport.
pub fn flyout_layout(anchor: Rect, items: &[String], vw: f32, vh: f32) -> FlyoutLayout {
    let n = items.len() as f32;
    let (rw, rh) = FLYOUT_ROW;
    let pw = rw + 8.0;
    let ph = n * rh + 8.0;
    let x = (anchor.0 + anchor.2 * 0.5 - pw * 0.5).clamp(4.0, (vw - pw - 4.0).max(4.0));
    let y = if anchor.1 - ph - 4.0 >= 4.0 {
        anchor.1 - ph - 4.0
    } else {
        (anchor.1 + anchor.3 + 4.0).min((vh - ph - 4.0).max(4.0))
    };
    let panel = (x, y, pw, ph);
    let rows = items
        .iter()
        .enumerate()
        .map(|(i, id)| (id.clone(), (x + 4.0, y + 4.0 + i as f32 * rh, rw, rh)))
        .collect();
    FlyoutLayout { panel, rows }
}

/// Full HUD geometry for a `vw`x`vh` viewport.
pub fn hud_layout(vw: f32, vh: f32, has_target: bool) -> HudLayout {
    let m = HUD_MARGIN;
    let target_panel = has_target.then_some((vw * 0.5 - 125.0, m, 250.0, 58.0));

    let bar_w = SLOT * 2.0 + 18.0;
    let action_bar = (vw * 0.5 - bar_w * 0.5, vh - SLOT - 14.0, bar_w, SLOT + 8.0);
    let laser_slot = (action_bar.0 + 4.0, action_bar.1 + 4.0, SLOT, SLOT);
    let rocket_slot = (action_bar.0 + SLOT + 14.0, action_bar.1 + 4.0, SLOT, SLOT);

    HudLayout {
        target_panel,
        action_bar,
        laser_slot,
        rocket_slot,
        flyout: None,
    }
}

/// `(x,y) in (rx,ry,rw,rh)` — pixels.
pub fn point_in(r: Rect, p: (f32, f32)) -> bool {
    p.0 >= r.0 && p.0 < r.0 + r.2 && p.1 >= r.1 && p.1 < r.1 + r.3
}

/// Kill-screen modal geometry: `(panel, repair_button)` — the panel is
/// centered; while it is up every click is swallowed except the button
/// (`poll_input` treats it as modal).
pub fn kill_screen_layout(vw: f32, vh: f32) -> (Rect, Rect) {
    let (w, h) = (360.0, 240.0);
    let panel = (vw * 0.5 - w * 0.5, vh * 0.5 - h * 0.5, w, h);
    let btn = (panel.0 + w * 0.5 - 70.0, panel.1 + h - 48.0, 140.0, 30.0);
    (panel, btn)
}

/// Max repair-option rows the kill screen lists.
pub const KILL_MAX_ROWS: usize = 6;
/// Height of one option row / gap between rows.
pub const KILL_ROW_H: f32 = 30.0;
pub const KILL_ROW_GAP: f32 = 4.0;
/// Title bar + killer line above the first row.
pub const KILL_HEADER_H: f32 = 70.0;
/// Footer below the rows (repair button + hint).
pub const KILL_FOOTER_H: f32 = 56.0;

/// Kill-screen geometry for `n` option rows: the panel grows with the
/// option list (at least one row's worth of room for the "basic repair"
/// note shown when the list never arrived).
#[derive(Clone, Debug, PartialEq)]
pub struct KillLayout {
    pub panel: Rect,
    /// One rect per shown option (`min(n, KILL_MAX_ROWS)`), top to bottom.
    pub rows: Vec<Rect>,
    /// The big "repair" button (best option).
    pub button: Rect,
}

pub fn kill_screen_layout_n(vw: f32, vh: f32, n: usize) -> KillLayout {
    let shown = n.min(KILL_MAX_ROWS);
    let slots = shown.max(1) as f32;
    let w = 400.0;
    let h = KILL_HEADER_H + slots * (KILL_ROW_H + KILL_ROW_GAP) + KILL_FOOTER_H;
    let panel = (vw * 0.5 - w * 0.5, vh * 0.5 - h * 0.5, w, h);
    let rows = (0..shown)
        .map(|i| {
            (
                panel.0 + 14.0,
                panel.1 + KILL_HEADER_H + i as f32 * (KILL_ROW_H + KILL_ROW_GAP),
                w - 28.0,
                KILL_ROW_H,
            )
        })
        .collect();
    let button = (
        panel.0 + w * 0.5 - 80.0,
        panel.1 + h - KILL_FOOTER_H + 10.0,
        160.0,
        32.0,
    );
    KillLayout {
        panel,
        rows,
        button,
    }
}

/// First HUD area covering `p`, if any — checked in hit priority order
/// (flyout, interactive slots, then plain panels).
pub fn hud_hit(layout: &HudLayout, p: (f32, f32)) -> Option<HudArea> {
    if let Some(f) = &layout.flyout {
        for (i, (_, r)) in f.rows.iter().enumerate() {
            if point_in(*r, p) {
                return Some(HudArea::FlyoutItem(i));
            }
        }
        if point_in(f.panel, p) {
            return Some(HudArea::Panel);
        }
    }
    if point_in(layout.laser_slot, p) {
        return Some(HudArea::LaserSlot);
    }
    if point_in(layout.rocket_slot, p) {
        return Some(HudArea::RocketSlot);
    }
    if point_in(layout.action_bar, p) || layout.target_panel.is_some_and(|r| point_in(r, p)) {
        return Some(HudArea::Panel);
    }
    None
}

/// Items an ammo flyout opened from `item_id` lists: every visible menu
/// item sharing its family prefix (`ammunition_laser_` for
/// `ammunition_laser_lcb-10`), sorted by id. Empty when `item_id` has no
/// family (no `_`) or isn't ammunition.
pub fn flyout_items(menu: &crate::net::world::MenuState, item_id: &str) -> Vec<String> {
    let Some(cut) = item_id.rfind('_') else {
        return Vec::new();
    };
    let prefix = &item_id[..=cut];
    if !prefix.starts_with("ammunition_") {
        return Vec::new();
    }
    let mut ids: Vec<String> = menu
        .items
        .values()
        .filter(|it| it.id.starts_with(prefix) && it.visible)
        .map(|it| it.id.clone())
        .collect();
    ids.sort();
    ids
}

/// Short label for a `menuItemId`/`lootId` — text after the last `_`,
/// uppercased, truncated to 7 chars (`"ammunition_laser_ucb-100"` →
/// `"UCB-100"`; no `_` → the whole id).
pub fn short_item_label(id: &str) -> String {
    let tail = id.rsplit('_').next().unwrap_or(id);
    tail.to_uppercase().chars().take(7).collect()
}

/// Compact counter text for a slot (`counterValue`): `999`, `1.2K`
/// (>=1000), `3.4M` (>=1e6); negative/NaN → `"0"`.
pub fn fmt_compact(v: f64) -> String {
    if v.is_nan() || v <= 0.0 {
        return "0".into();
    }
    if v >= 1e6 {
        format!("{:.1}M", v / 1e6)
    } else if v >= 1e3 {
        format!("{:.1}K", v / 1e3)
    } else {
        format!("{}", v as i64)
    }
}

/// Pick the nearest collectable within `radius` world units.
/// `items`: `(hash, x, y)`.
pub fn pick_collectable(
    items: &[(String, f32, f32)],
    w: (f32, f32),
    radius: f32,
) -> Option<String> {
    let mut best: Option<(&str, f32)> = None;
    for (hash, x, y) in items {
        let dx = w.0 - *x;
        let dy = w.1 - *y;
        let d2 = dx * dx + dy * dy;
        if d2 <= radius * radius && best.map(|(_, bd)| d2 < bd).unwrap_or(true) {
            best = Some((hash.as_str(), d2));
        }
    }
    best.map(|(h, _)| h.to_owned())
}

/// Ship heading in degrees matching the stock client's `MoveTrait`:
/// `atan2(dy, dx) + PI` normalized to `0..360` (screen-space y down).
pub fn heading_deg(from: (f32, f32), to: (f32, f32)) -> f32 {
    let dy = to.1 - from.1;
    let dx = to.0 - from.0;
    if dx.abs() < f32::EPSILON && dy.abs() < f32::EPSILON {
        return 0.0;
    }
    let deg = dy.atan2(dx).to_degrees() + 180.0;
    deg.rem_euclid(360.0)
}

/// Rotation-strip frame index for a heading — `ShipClip` equivalent:
/// `int((32*deg/360 + 0.5) % 32)` on a 32-frame strip; generalized to
/// `frames` so any strip length works.
pub fn rotation_frame(deg: f32, frames: usize) -> usize {
    if frames == 0 {
        return 0;
    }
    let deg = deg.rem_euclid(360.0);
    (((frames as f32 * deg / 360.0) + 0.5) as i64).rem_euclid(frames as i64) as usize
}

/// Looping-clip frame index at `time_ms` — honors per-frame `durations_ms`,
/// falling back to ~30fps (33ms) per frame when durations are absent/zero.
pub fn anim_frame(durations_ms: &[u32], time_ms: u64) -> usize {
    if durations_ms.is_empty() {
        return 0;
    }
    // All-zero durations -> fixed ~30fps cadence.
    if durations_ms.iter().all(|&d| d == 0) {
        return ((time_ms / 33) % durations_ms.len() as u64) as usize;
    }
    let total: u64 = durations_ms.iter().map(|&d| d.max(1) as u64).sum();
    let mut t = (time_ms % total.max(1)) as i64;
    for (i, &d) in durations_ms.iter().enumerate() {
        t -= d.max(1) as i64;
        if t < 0 {
            return i;
        }
    }
    durations_ms.len() - 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn click_entity_selects_then_attacks() {
        let h = Hit::Entity(42);
        assert_eq!(
            decide_click(h.clone(), (0.0, 0.0), (100.0, 100.0), None),
            ClickAction::SelectEntity(42)
        );
        assert_eq!(
            decide_click(h.clone(), (0.0, 0.0), (100.0, 100.0), Some(7)),
            ClickAction::SelectEntity(42)
        );
        assert_eq!(
            decide_click(h, (0.0, 0.0), (100.0, 100.0), Some(42)),
            ClickAction::AttackEntity(42)
        );
    }

    #[test]
    fn click_empty_moves_clamped() {
        assert_eq!(
            decide_click(Hit::Empty, (-50.0, 99999.0), (20480.0, 12800.0), None),
            ClickAction::MoveTo(0.0, 12800.0)
        );
    }

    #[test]
    fn mobile_hud_layout_stays_touchable_at_landscape_preview_sizes() {
        for (vw, vh) in [(960.0, 540.0), (1280.0, 720.0), (2400.0, 1080.0)] {
            let l = mobile_hud_layout(vw, vh);
            assert_eq!(
                mobile_hud_hit(&l, (l.move_stick.0, l.move_stick.1)),
                Some(MobileControl::MoveStick)
            );
            assert_eq!(
                mobile_hud_hit(
                    &l,
                    (l.target.0 + l.target.2 * 0.5, l.target.1 + l.target.3 * 0.5)
                ),
                Some(MobileControl::Target)
            );
            assert_eq!(
                mobile_hud_hit(
                    &l,
                    (l.laser.0 + l.laser.2 * 0.5, l.laser.1 + l.laser.3 * 0.5)
                ),
                Some(MobileControl::Laser)
            );
            assert_eq!(
                mobile_hud_hit(
                    &l,
                    (l.rocket.0 + l.rocket.2 * 0.5, l.rocket.1 + l.rocket.3 * 0.5)
                ),
                Some(MobileControl::Rocket)
            );
            for r in [l.target, l.laser, l.rocket, l.jump, l.pet] {
                assert!(
                    r.0 >= 0.0 && r.1 >= 0.0 && r.0 + r.2 <= vw && r.1 + r.3 <= vh,
                    "{r:?} outside {vw}x{vh}"
                );
                assert!(r.2 >= 34.0 && r.3 >= 34.0, "touch target too small: {r:?}");
            }
        }
    }

    #[test]
    fn captured_movement_finger_survives_an_attack_finger_and_releases_cleanly() {
        assert_eq!(retain_touch_id(Some(17), &[42, 17]), Some(17));
        assert_eq!(retain_touch_id(Some(17), &[42]), None);
    }

    #[test]
    fn pick_nearest_in_radius() {
        let items = [
            (1, 0.0, 0.0, 50.0),
            (2, 100.0, 0.0, 50.0),
            (3, 300.0, 0.0, 10.0),
        ];
        assert_eq!(pick_entity(&items, (110.0, 10.0)), Some(2));
        assert_eq!(pick_entity(&items, (240.0, 0.0)), None);
        // overlapping radii -> nearest wins
        assert_eq!(pick_entity(&items, (60.0, 0.0)), Some(2));
    }

    #[test]
    fn rotation_frame_wraps() {
        assert_eq!(rotation_frame(0.0, 32), 0);
        assert_eq!(rotation_frame(90.0, 32), 8);
        assert_eq!(rotation_frame(180.0, 32), 16);
        assert_eq!(rotation_frame(359.9, 32), 0);
        assert_eq!(rotation_frame(-45.0, 32), 28);
        assert_eq!(rotation_frame(0.0, 0), 0);
    }

    #[test]
    fn heading_points_at_target() {
        // Moving right -> atan2(0,+)=0 -> +180 -> 180 (frame 16 on a strip).
        assert!((heading_deg((0.0, 0.0), (1.0, 0.0)) - 180.0).abs() < 1e-3);
        assert!((heading_deg((0.0, 0.0), (0.0, 1.0)) - 270.0).abs() < 1e-3);
        assert_eq!(heading_deg((5.0, 5.0), (5.0, 5.0)), 0.0);
    }

    #[test]
    fn anim_frame_uses_durations() {
        let durs = [100u32, 100, 100];
        assert_eq!(anim_frame(&durs, 0), 0);
        assert_eq!(anim_frame(&durs, 150), 1);
        assert_eq!(anim_frame(&durs, 299), 2);
        assert_eq!(anim_frame(&durs, 300), 0);
        // zero durations -> ~30fps cadence
        assert_eq!(anim_frame(&[0, 0, 0], 66), 2);
    }

    #[test]
    fn plate_kind_classifies_faction() {
        // NPCs are always hostiles, even same-faction ids.
        assert_eq!(plate_kind(true, 2, 2), PlateKind::Npc);
        // Same company -> ally blue; other companies -> enemy red.
        assert_eq!(plate_kind(false, 2, 2), PlateKind::Ally);
        assert_eq!(plate_kind(false, 1, 2), PlateKind::Enemy);
        assert_eq!(plate_kind(false, 3, 1), PlateKind::Enemy);
        // faction 0 means "no faction" -> neutral, not enemy.
        assert_eq!(plate_kind(false, 0, 2), PlateKind::Neutral);
        // An unknown hero faction doesn't pretend everyone is an ally.
        assert_eq!(plate_kind(false, 0, 0), PlateKind::Neutral);
        assert_eq!(plate_kind(false, 3, 0), PlateKind::Enemy);
    }

    #[test]
    fn hud_hit_swallows_panels_not_world() {
        let l = hud_layout(1280.0, 720.0, true);
        let tp = l.target_panel.unwrap();
        assert_eq!(
            hud_hit(&l, (tp.0 + tp.2 * 0.5, tp.1 + tp.3 * 0.5)),
            Some(HudArea::Panel)
        );
        // Action-bar slots win over the bar's panel.
        assert_eq!(
            hud_hit(&l, (l.laser_slot.0 + 2.0, l.laser_slot.1 + 2.0)),
            Some(HudArea::LaserSlot)
        );
        assert_eq!(
            hud_hit(&l, (l.rocket_slot.0 + 2.0, l.rocket_slot.1 + 2.0)),
            Some(HudArea::RocketSlot)
        );
        // Bare bar area (outside the slots) swallows without acting.
        assert_eq!(
            hud_hit(
                &l,
                (l.action_bar.0 + l.action_bar.2 - 2.0, l.action_bar.1 + 2.0)
            ),
            Some(HudArea::Panel)
        );
        // The world between the panels stays clickable.
        assert_eq!(hud_hit(&l, (640.0, 300.0)), None);
        // No target -> no target panel swallowing.
        let l2 = hud_layout(1280.0, 720.0, false);
        assert_eq!(hud_hit(&l2, (tp.0 + 5.0, tp.1 + 5.0)), None);
    }

    #[test]
    fn flyout_hangs_above_slot_and_hits_rows() {
        let items: Vec<String> = ["ammunition_laser_lcb-10", "ammunition_laser_mcb-25"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let l = hud_layout(1280.0, 720.0, false).with_flyout(
            FlyoutSource::Laser,
            &items,
            1280.0,
            720.0,
        );
        let f = l.flyout.as_ref().expect("flyout");
        assert_eq!(f.rows.len(), 2);
        let anchor = l.laser_slot;
        assert!(f.panel.1 + f.panel.3 <= anchor.1, "above the slot");
        let r1 = f.rows[1].1;
        assert_eq!(
            hud_hit(&l, (r1.0 + 3.0, r1.1 + 3.0)),
            Some(HudArea::FlyoutItem(1))
        );
        // Clamped inside the viewport at the screen edge.
        let edge = flyout_layout((0.0, 300.0, 34.0, 34.0), &items, 1280.0, 720.0);
        assert!(edge.panel.0 >= 4.0);
        // No items -> no flyout.
        let none =
            hud_layout(1280.0, 720.0, false).with_flyout(FlyoutSource::Laser, &[], 1280.0, 720.0);
        assert!(none.flyout.is_none());
    }

    #[test]
    fn flyout_items_lists_family_sorted() {
        use crate::net::world::{MenuItem, MenuState};
        let mut menu = MenuState::default();
        for id in [
            "ammunition_laser_mcb-25",
            "ammunition_laser_lcb-10",
            "ammunition_rocket_r-310",
        ] {
            let it = MenuItem {
                id: id.to_string(),
                visible: true,
                ..Default::default()
            };
            menu.items.insert(id.to_string(), it);
        }
        assert_eq!(
            flyout_items(&menu, "ammunition_laser_lcb-10"),
            vec!["ammunition_laser_lcb-10", "ammunition_laser_mcb-25"]
        );
        assert!(flyout_items(&menu, "tech_shield").is_empty());
    }

    #[test]
    fn short_item_label_tails() {
        assert_eq!(short_item_label("ammunition_laser_ucb-100"), "UCB-100");
        assert_eq!(short_item_label("ammunition_rocket_r-310"), "R-310");
        assert_eq!(short_item_label("ammunition_laser_lcb-10"), "LCB-10");
        // No underscore -> whole id, uppercased/truncated to 7.
        assert_eq!(short_item_label("repairbot"), "REPAIRB");
        assert_eq!(
            short_item_label("ammunition_specialitem_longname"),
            "LONGNAM"
        );
    }

    #[test]
    fn fmt_compact_scales() {
        assert_eq!(fmt_compact(999.0), "999");
        assert_eq!(fmt_compact(1200.0), "1.2K");
        assert_eq!(fmt_compact(1500.0), "1.5K");
        assert_eq!(fmt_compact(3_400_000.0), "3.4M");
        assert_eq!(fmt_compact(-5.0), "0");
        assert_eq!(fmt_compact(f64::NAN), "0");
    }
}
