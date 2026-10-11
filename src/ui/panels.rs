//! Pure content layouts for the window bodies. Both the draw pass
//! (`render::hud::*`) and `Game::poll_input` call these, so a button's
//! rect can never disagree between what is painted and what is clickable.

use super::window::{point_in, Rect};
use crate::locale::LocaleCatalog;
use crate::net::quests::{quest_title, quest_type_label, ObjectiveRow, Quest, QuestEnd, QuestLog};
use crate::net::world::SettingFlag;

// ---------------------------------------------------------------------
// Minimap
// ---------------------------------------------------------------------

/// Map viewport size in px.
pub const MINIMAP_MAP: (f32, f32) = (212.0, 142.0);
pub const MINIMAP_PAD: f32 = 6.0;
pub const MINIMAP_FOOTER_H: f32 = 16.0;
/// Minimap window body size (map + footer strip).
pub const MINIMAP_BODY: (f32, f32) = (
    MINIMAP_MAP.0 + 2.0 * MINIMAP_PAD,
    MINIMAP_PAD + MINIMAP_MAP.1 + 4.0 + MINIMAP_FOOTER_H + MINIMAP_PAD,
);
pub const MINIMAP_ZOOM_MIN: f32 = 1.0;
pub const MINIMAP_ZOOM_MAX: f32 = 8.0;
const MINIMAP_ZOOM_STEP: f32 = 1.5;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum MinimapTab {
    #[default]
    Map,
    /// Galaxy overview — stub (no galaxy data model yet).
    Galaxy,
}

#[derive(Clone, Debug)]
pub struct MinimapLayout {
    pub map: Rect,
    pub zoom_out: Rect,
    pub zoom_in: Rect,
    pub readout: Rect,
    pub tab_map: Rect,
    pub tab_galaxy: Rect,
}

pub fn minimap_layout(body: Rect) -> MinimapLayout {
    let map = (
        body.0 + MINIMAP_PAD,
        body.1 + MINIMAP_PAD,
        MINIMAP_MAP.0,
        MINIMAP_MAP.1,
    );
    let fy = map.1 + map.3 + 4.0;
    let bs = MINIMAP_FOOTER_H;
    let zoom_out = (map.0, fy, bs, bs);
    let zoom_in = (map.0 + bs + 2.0, fy, bs, bs);
    let tab_galaxy = (map.0 + map.2 - 46.0, fy, 46.0, bs);
    let tab_map = (tab_galaxy.0 - 48.0, fy, 46.0, bs);
    let rx = zoom_in.0 + bs + 6.0;
    let readout = (rx, fy, (tab_map.0 - 4.0 - rx).max(0.0), bs);
    MinimapLayout {
        map,
        zoom_out,
        zoom_in,
        readout,
        tab_map,
        tab_galaxy,
    }
}

/// Next zoom level for a ± click, clamped; snaps back to exactly 1.0.
pub fn minimap_zoom_step(zoom: f32, dir: i32) -> f32 {
    let z = if dir > 0 {
        zoom * MINIMAP_ZOOM_STEP
    } else {
        zoom / MINIMAP_ZOOM_STEP
    };
    let z = z.clamp(MINIMAP_ZOOM_MIN, MINIMAP_ZOOM_MAX);
    if (z - 1.0).abs() < 0.05 {
        1.0
    } else {
        z
    }
}

/// World ↔ minimap-pixel mapping for a zoom level centred on the hero.
#[derive(Clone, Copy, Debug)]
pub struct MinimapView {
    pub map: Rect,
    /// World coordinate at the map rect's top-left.
    pub origin: (f32, f32),
    /// World extent shown `(w, h)`.
    pub span: (f32, f32),
}

impl MinimapView {
    pub fn new(map: Rect, bounds: (f32, f32), zoom: f32, focus: (f32, f32)) -> Self {
        let zoom = zoom.clamp(MINIMAP_ZOOM_MIN, MINIMAP_ZOOM_MAX);
        let span = (bounds.0 / zoom, bounds.1 / zoom);
        let ox = (focus.0 - span.0 * 0.5).clamp(0.0, (bounds.0 - span.0).max(0.0));
        let oy = (focus.1 - span.1 * 0.5).clamp(0.0, (bounds.1 - span.1).max(0.0));
        Self {
            map,
            origin: (ox, oy),
            span,
        }
    }

    pub fn to_screen(&self, wx: f32, wy: f32) -> (f32, f32) {
        (
            self.map.0 + (wx - self.origin.0) / self.span.0 * self.map.2,
            self.map.1 + (wy - self.origin.1) / self.span.1 * self.map.3,
        )
    }

    /// Screen px → world, clamped to the shown area; `None` for a
    /// degenerate view.
    pub fn to_world(&self, p: (f32, f32), bounds: (f32, f32)) -> Option<(f32, f32)> {
        if self.map.2 <= 0.0 || self.map.3 <= 0.0 || self.span.0 <= 0.0 || self.span.1 <= 0.0 {
            return None;
        }
        let wx = self.origin.0 + (p.0 - self.map.0) / self.map.2 * self.span.0;
        let wy = self.origin.1 + (p.1 - self.map.1) / self.map.3 * self.span.1;
        Some((wx.clamp(0.0, bounds.0), wy.clamp(0.0, bounds.1)))
    }

    /// World units per minimap pixel (the "pp" readout).
    pub fn units_per_pixel(&self) -> f32 {
        if self.map.2 <= 0.0 {
            0.0
        } else {
            self.span.0 / self.map.2
        }
    }
}

// ---------------------------------------------------------------------
// Ship
// ---------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct ShipLayout {
    /// C1 / C2 buttons.
    pub config: [Rect; 2],
    /// Top-left of the ore breakdown block.
    pub ore_origin: (f32, f32),
}

pub fn ship_layout(body: Rect) -> ShipLayout {
    let y = body.1 + 66.0;
    ShipLayout {
        config: [
            (body.0 + 40.0, y, 30.0, 16.0),
            (body.0 + 74.0, y, 30.0, 16.0),
        ],
        ore_origin: (body.0 + 8.0, body.1 + 90.0),
    }
}

/// Max ore rows the ship window can show (2 columns × 3 rows).
pub const SHIP_ORE_ROWS: usize = 6;

// ---------------------------------------------------------------------
// Pet
// ---------------------------------------------------------------------

pub const PET_GEAR_ROW_H: f32 = 20.0;

#[derive(Clone, Debug)]
pub struct PetLayout {
    pub activate: Rect,
    pub deactivate: Rect,
    /// Scrollable gear list viewport.
    pub gear_view: Rect,
    /// `(gear index, rect)` for rows at least partly inside `gear_view`.
    pub gears: Vec<(usize, Rect)>,
}

pub fn pet_layout(body: Rect, gear_count: usize, scroll: f32) -> PetLayout {
    let activate = (body.0 + 8.0, body.1 + 70.0, 108.0, 18.0);
    let deactivate = (body.0 + 124.0, body.1 + 70.0, 108.0, 18.0);
    let view = (
        body.0 + 6.0,
        body.1 + 108.0,
        body.2 - 12.0,
        (body.3 - 108.0 - 6.0).max(0.0),
    );
    let mut gears = Vec::new();
    for i in 0..gear_count {
        let y = view.1 + i as f32 * PET_GEAR_ROW_H - scroll;
        if y + PET_GEAR_ROW_H > view.1 && y < view.1 + view.3 {
            gears.push((i, (view.0, y, view.2, PET_GEAR_ROW_H - 2.0)));
        }
    }
    PetLayout {
        activate,
        deactivate,
        gear_view: view,
        gears,
    }
}

pub fn pet_max_scroll(body: Rect, gear_count: usize) -> f32 {
    let view_h = (body.3 - 108.0 - 6.0).max(0.0);
    (gear_count as f32 * PET_GEAR_ROW_H - view_h).max(0.0)
}

// ---------------------------------------------------------------------
// Quests
// ---------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub enum QuestRowKind {
    Section(String),
    /// Accepted quest header (with the cancel button).
    Active {
        id: i32,
        title: String,
        badge: String,
        done: bool,
    },
    Objective(ObjectiveRow),
    /// Quest on offer — clicking asks the server for its definition.
    Available {
        id: i32,
        title: String,
        min_level: i32,
        selected: bool,
        locked: bool,
    },
    Finished {
        title: String,
        end: QuestEnd,
    },
    Note(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct QuestRow {
    pub kind: QuestRowKind,
    /// Offset from the top of the scroll content.
    pub y: f32,
    pub h: f32,
}

const ROW_SECTION: f32 = 18.0;
const ROW_QUEST: f32 = 20.0;
const ROW_OBJ: f32 = 14.0;
const ROW_AVAIL: f32 = 16.0;
const ROW_FIN: f32 = 14.0;
const ROW_NOTE: f32 = 16.0;
/// Padding above the first row.
pub const QUEST_TOP_PAD: f32 = 4.0;

/// Build the flat row list for the quests window and its content height.
pub fn quest_rows(
    log: &QuestLog,
    loc: &LocaleCatalog,
    selected: Option<i32>,
    hero_level: i32,
) -> (Vec<QuestRow>, f32) {
    let mut rows: Vec<QuestRow> = Vec::new();
    let mut y = 0.0f32;
    let push = |kind: QuestRowKind, h: f32, rows: &mut Vec<QuestRow>, y: &mut f32| {
        rows.push(QuestRow { kind, y: *y, h });
        *y += h;
    };

    let active_hdr = if log.max_quests > 0 {
        format!("ACTIVE ({}/{})", log.active.len(), log.max_quests)
    } else {
        format!("ACTIVE ({})", log.active.len())
    };
    push(
        QuestRowKind::Section(active_hdr),
        ROW_SECTION,
        &mut rows,
        &mut y,
    );
    if log.active.is_empty() {
        push(
            QuestRowKind::Note("No accepted quests.".into()),
            ROW_NOTE,
            &mut rows,
            &mut y,
        );
    }
    for q in log.active.values() {
        push(
            QuestRowKind::Active {
                id: q.id,
                title: quest_title(loc, q.id, &q.text_key),
                badge: quest_badge(q),
                done: q.all_done(),
            },
            ROW_QUEST,
            &mut rows,
            &mut y,
        );
        let objs = q.objectives();
        if objs.is_empty() {
            push(
                QuestRowKind::Note("Waiting for objective data...".into()),
                ROW_NOTE,
                &mut rows,
                &mut y,
            );
        }
        for o in objs {
            push(QuestRowKind::Objective(o), ROW_OBJ, &mut rows, &mut y);
        }
    }

    if !log.available.is_empty() {
        push(
            QuestRowKind::Section(format!("AVAILABLE ({})", log.available.len())),
            ROW_SECTION,
            &mut rows,
            &mut y,
        );
        let mut avail: Vec<_> = log.available.values().collect();
        avail.sort_by_key(|s| (s.priority, s.id));
        for s in avail {
            let is_sel = selected == Some(s.id);
            push(
                QuestRowKind::Available {
                    id: s.id,
                    title: quest_title(loc, s.id, &s.text_key),
                    min_level: s.min_level,
                    selected: is_sel,
                    locked: s.min_level > hero_level && hero_level > 0,
                },
                ROW_AVAIL,
                &mut rows,
                &mut y,
            );
            if is_sel {
                match log.info.get(&s.id) {
                    Some(info) => {
                        for o in info.objectives() {
                            push(QuestRowKind::Objective(o), ROW_OBJ, &mut rows, &mut y);
                        }
                    }
                    None => push(
                        QuestRowKind::Note("Loading details...".into()),
                        ROW_NOTE,
                        &mut rows,
                        &mut y,
                    ),
                }
            }
        }
    }

    if !log.finished.is_empty() {
        push(
            QuestRowKind::Section("RECENT".into()),
            ROW_SECTION,
            &mut rows,
            &mut y,
        );
        for f in log.finished.iter().rev() {
            push(
                QuestRowKind::Finished {
                    title: quest_title(loc, f.id, &f.text_key),
                    end: f.end,
                },
                ROW_FIN,
                &mut rows,
                &mut y,
            );
        }
    }
    (rows, y + QUEST_TOP_PAD * 2.0)
}

fn quest_badge(q: &Quest) -> String {
    q.types
        .iter()
        .map(|t| quest_type_label(*t))
        .find(|s| !s.is_empty())
        .unwrap_or("")
        .to_owned()
}

/// Scroll viewport of the quests window.
pub fn quest_view(body: Rect) -> Rect {
    (body.0, body.1, body.2, body.3)
}

/// Cancel button for an `Active` row whose content offset is `row_y`.
pub fn quest_cancel_rect(body: Rect, row_y: f32, scroll: f32) -> Rect {
    (
        body.0 + body.2 - 58.0,
        body.1 + QUEST_TOP_PAD + row_y - scroll + 2.0,
        50.0,
        16.0,
    )
}

/// Absolute rect of a quest row's clickable area.
pub fn quest_row_rect(body: Rect, row: &QuestRow, scroll: f32) -> Rect {
    (
        body.0 + 4.0,
        body.1 + QUEST_TOP_PAD + row.y - scroll,
        body.2 - 8.0,
        row.h,
    )
}

pub fn quest_max_scroll(body: Rect, content_h: f32) -> f32 {
    (content_h - body.3).max(0.0)
}

/// Action under `p` in the quests window: `(Cancel|Select, quest id)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuestHit {
    Cancel(i32),
    Select(i32),
}

pub fn quest_hit(body: Rect, rows: &[QuestRow], scroll: f32, p: (f32, f32)) -> Option<QuestHit> {
    if !point_in(quest_view(body), p) {
        return None;
    }
    for row in rows {
        match &row.kind {
            QuestRowKind::Active { id, .. } => {
                if point_in(quest_cancel_rect(body, row.y, scroll), p) {
                    return Some(QuestHit::Cancel(*id));
                }
            }
            QuestRowKind::Available { id, .. } => {
                if point_in(quest_row_rect(body, row, scroll), p) {
                    return Some(QuestHit::Select(*id));
                }
            }
            _ => {}
        }
    }
    None
}

// ---------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------

/// `Settings.as` audio toggles (`AudioManager` mixes on these).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioToggle {
    /// `playSFX`
    Sfx,
    /// `playMusic`
    Music,
    /// `playVoice`
    Voice,
    /// `playCombatMusic`
    CombatMusic,
}

/// The three `Settings` volume channels (0..100).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioChannel {
    /// `soundVolume`
    Sound,
    /// `musicVolume`
    Music,
    /// `voiceVolume`
    Voice,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsItem {
    /// `DisplaySettings` flag (server-pushed, local toggle).
    Flag(SettingFlag),
    DebugOverlay,
    WindowSkin,
    MobileHud,
    RenderModes,
    MinimapMapImage,
    ToolbarBackground,
    /// Audio on/off row.
    AudioToggle(AudioToggle),
    /// 0-100 stepper row (`-`/`+` buttons at the right edge).
    AudioVolume(AudioChannel),
    ResetLayout,
}

pub fn settings_items() -> Vec<SettingsItem> {
    let mut v: Vec<SettingsItem> = SettingFlag::ALL
        .iter()
        .map(|f| SettingsItem::Flag(*f))
        .collect();
    v.push(SettingsItem::DebugOverlay);
    v.push(SettingsItem::WindowSkin);
    v.push(SettingsItem::MobileHud);
    v.push(SettingsItem::RenderModes);
    v.push(SettingsItem::MinimapMapImage);
    v.push(SettingsItem::ToolbarBackground);
    v.push(SettingsItem::AudioToggle(AudioToggle::Sfx));
    v.push(SettingsItem::AudioToggle(AudioToggle::Music));
    v.push(SettingsItem::AudioToggle(AudioToggle::CombatMusic));
    v.push(SettingsItem::AudioToggle(AudioToggle::Voice));
    v.push(SettingsItem::AudioVolume(AudioChannel::Sound));
    v.push(SettingsItem::AudioVolume(AudioChannel::Music));
    v.push(SettingsItem::AudioVolume(AudioChannel::Voice));
    v.push(SettingsItem::ResetLayout);
    v
}

/// `-`/`+` stepper hit boxes on an `AudioVolume` row (right edge).
pub fn settings_vol_buttons(r: Rect) -> (Rect, Rect) {
    let side = r.3.min(12.0);
    let plus = (r.0 + r.2 - side - 2.0, r.1 + (r.3 - side) * 0.5, side, side);
    let minus = (plus.0 - side - 4.0, plus.1, side, side);
    (minus, plus)
}

pub fn settings_row_rect(body: Rect, index: usize) -> Rect {
    let row_pitch = ((body.3 - 16.0) / settings_items().len().max(1) as f32).max(1.0);
    (
        body.0 + 8.0,
        body.1 + 8.0 + index as f32 * row_pitch,
        body.2 - 16.0,
        row_pitch - 1.0,
    )
}

pub fn settings_hit(body: Rect, p: (f32, f32)) -> Option<SettingsItem> {
    settings_items()
        .into_iter()
        .enumerate()
        .find(|(i, _)| point_in(settings_row_rect(body, *i), p))
        .map(|(_, it)| it)
}

// ---------------------------------------------------------------------
// Log
// ---------------------------------------------------------------------

pub const LOG_LINE_H: f32 = 12.0;

pub fn log_view(body: Rect) -> Rect {
    (
        body.0 + 6.0,
        body.1 + 4.0,
        body.2 - 12.0,
        (body.3 - 8.0).max(0.0),
    )
}

/// Scroll range of the log window in px (`scroll` = distance from the
/// newest line).
pub fn log_max_scroll(body: Rect, lines: usize) -> f32 {
    (lines as f32 * LOG_LINE_H - log_view(body).3).max(0.0)
}

// ---------------------------------------------------------------------
// Group, contacts, and chat
// ---------------------------------------------------------------------

pub const SOCIAL_ROW_H: f32 = 25.0;

#[derive(Clone, Debug)]
pub struct GroupLayout {
    pub rows: Vec<(usize, Rect)>,
    pub invite_input: Rect,
    pub invite: Rect,
    pub follow: Rect,
    pub leave: Rect,
}

pub fn group_layout(body: Rect, members: usize, scroll: f32) -> GroupLayout {
    let list = (
        body.0 + 6.0,
        body.1 + 22.0,
        body.2 - 12.0,
        (body.3 - 78.0).max(0.0),
    );
    let mut rows = Vec::new();
    for i in 0..members {
        let y = list.1 + i as f32 * SOCIAL_ROW_H - scroll;
        if y + SOCIAL_ROW_H > list.1 && y < list.1 + list.3 {
            rows.push((i, (list.0, y, list.2, SOCIAL_ROW_H - 2.0)));
        }
    }
    let fy = body.1 + body.3 - 48.0;
    GroupLayout {
        rows,
        invite_input: (body.0 + 6.0, fy, body.2 - 72.0, 19.0),
        invite: (body.0 + body.2 - 62.0, fy, 56.0, 19.0),
        follow: (body.0 + 6.0, fy + 23.0, (body.2 - 18.0) * 0.5, 19.0),
        leave: (
            body.0 + body.2 * 0.5 + 3.0,
            fy + 23.0,
            (body.2 - 18.0) * 0.5,
            19.0,
        ),
    }
}

pub fn group_max_scroll(body: Rect, members: usize) -> f32 {
    let view_h = (body.3 - 78.0).max(0.0);
    (members as f32 * SOCIAL_ROW_H - view_h).max(0.0)
}

#[derive(Clone, Debug)]
pub struct ContactsLayout {
    pub rows: Vec<(usize, Rect)>,
    pub name_input: Rect,
    pub add: Rect,
    pub remove: Rect,
}

pub fn contacts_layout(body: Rect, contacts: usize, scroll: f32) -> ContactsLayout {
    let list = (
        body.0 + 6.0,
        body.1 + 5.0,
        body.2 - 12.0,
        (body.3 - 40.0).max(0.0),
    );
    let mut rows = Vec::new();
    for i in 0..contacts {
        let y = list.1 + i as f32 * SOCIAL_ROW_H - scroll;
        if y + SOCIAL_ROW_H > list.1 && y < list.1 + list.3 {
            rows.push((i, (list.0, y, list.2, SOCIAL_ROW_H - 2.0)));
        }
    }
    let fy = body.1 + body.3 - 28.0;
    ContactsLayout {
        rows,
        name_input: (body.0 + 6.0, fy, (body.2 - 142.0).max(20.0), 20.0),
        add: (body.0 + body.2 - 130.0, fy, 58.0, 20.0),
        remove: (body.0 + body.2 - 66.0, fy, 60.0, 20.0),
    }
}

pub fn contacts_max_scroll(body: Rect, contacts: usize) -> f32 {
    let view_h = (body.3 - 40.0).max(0.0);
    (contacts as f32 * SOCIAL_ROW_H - view_h).max(0.0)
}

// ---------------------------------------------------------------------
// Refinery ("Refining"/"Upgrade" lab window)
// ---------------------------------------------------------------------

/// `sourceSlot`/`targetSlot` bitmap edge in the `refinement` SWF.
pub const REFINERY_SLOT: f32 = 80.0;
/// Gap between slots (`background.width + 8` in `createRefinementContainers`).
pub const REFINERY_SLOT_GAP: f32 = 8.0;
/// Refine button offset below the module top (`_loc26_.y = 100`).
pub const REFINERY_BTN_Y: f32 = 100.0;
pub const REFINERY_BTN_H: f32 = 18.0;
/// Name/amount label height (`labelSlot` strip).
pub const REFINERY_LABEL_H: f32 = 14.0;

/// Whole-body layout of the refinery window (body = 455x507 —
/// `RefinementManager.createRefinementWindow` + the game.xml container
/// coords).
///
/// Stock `<simpleContainer>` coords are window-relative (containers are
/// added to `_rootContainer`, not the uiContainer); our body starts
/// `TITLE_H` (25px) under the window top, so body y = window y − 25.
///
/// The stock client recomputes the container xs at runtime: refining
/// sources and the tree sit at `260 - maxw/2` (≈88 for the shipped
/// 4-slot row), the static row 47px left of that (41); the upgrade
/// source/target containers sit at `230 - maxw/2` (≈14 for 5 sources /
/// 4 targets). Index 2 of the refining ores (Promerium) is moved onto a
/// second row and index 3 (Xenomit) re-anchored at inner x=220.
#[derive(Clone, Debug)]
pub struct RefineryLayout {
    /// "Refining" / "Upgrade" tab buttons (container 36, y=45 → body 20).
    pub tab_refining: Rect,
    pub tab_upgrade: Rect,
    /// 1px separator under the tabs (inner y+20 → body 40, 434px wide).
    pub separator: Rect,
    /// `ore_tree` bitmap anchor on the Refining tab (container 41,
    /// recentered to the source row x).
    pub tree: Rect,
    /// Refining tab: static ore slots (container 38 recentered to x=41).
    pub refine_static: Vec<Rect>,
    /// Refining tab: producible ore slots (container 39, recentered to
    /// x=88). Index 2 drops to the second row (stock `getElementAt`
    /// repositioning); index 3+ keep flowing right on the first row
    /// from inner x=220.
    pub refine_ores: Vec<Rect>,
    /// Parallel to `refine_ores` — "Refine" button rect for entries
    /// with `button="true"` (child of the module at y=100).
    pub refine_buttons: Vec<Rect>,
    /// Upgrade tab: `lab_title` strip (container 37, y=70 → body 45).
    pub upd_title: Rect,
    /// Upgrade tab: static ore slots (container 37, inner y=40).
    pub upd_static: Vec<Rect>,
    /// Upgrade tab: `labor_intro` text (container 31 inner y=0).
    pub upd_intro: Rect,
    /// Upgrade tab: draggable source ore slots (container 31 inner y=55).
    pub upd_sources: Vec<Rect>,
    /// Upgrade tab: target item slots (container 32, y=410 → body 385).
    pub upd_targets: Vec<Rect>,
}

pub fn refinery_layout(
    body: Rect,
    refine_static_n: usize,
    refine_ores_n: usize,
    upd_static_n: usize,
    upd_sources_n: usize,
    upd_targets_n: usize,
) -> RefineryLayout {
    let (bx, by) = (body.0, body.1);
    let tab_refining = (bx + 20.0, by + 20.0, 52.0, 18.0);
    let tab_upgrade = (bx + 76.0, by + 20.0, 52.0, 18.0);
    let separator = (bx + 20.0, by + 40.0, 434.0f32.min(body.2 - 20.0), 1.0);

    let refine_static = (0..refine_static_n)
        .map(|i| {
            (
                bx + 41.0 + i as f32 * 88.0,
                by + 85.0,
                REFINERY_SLOT,
                REFINERY_SLOT,
            )
        })
        .collect();
    let mut refine_ores = Vec::with_capacity(refine_ores_n);
    let mut refine_buttons = Vec::with_capacity(refine_ores_n);
    for i in 0..refine_ores_n {
        let (sx, sy) = if i == 2 {
            // Promerium: `(42, 215)` inside the container at x=88.
            (bx + 130.0, by + 380.0)
        } else if i < 2 {
            (bx + 88.0 + i as f32 * 88.0, by + 220.0)
        } else {
            // Xenomit re-anchors at inner x=220, then flows right.
            (bx + 88.0 + 220.0 + (i - 3) as f32 * 88.0, by + 220.0)
        };
        let r = (sx, sy, REFINERY_SLOT, REFINERY_SLOT);
        refine_ores.push(r);
        refine_buttons.push((sx, sy + REFINERY_BTN_Y, REFINERY_SLOT, REFINERY_BTN_H));
    }
    let tree = (bx + 88.0, by + 181.0, 0.0, 0.0); // anchored; bitmap native size

    let upd_title = (bx + 20.0, by + 45.0, 340.0, 20.0);
    let upd_static = (0..upd_static_n)
        .map(|i| {
            (
                bx + 20.0 + i as f32 * 88.0,
                by + 85.0,
                REFINERY_SLOT,
                REFINERY_SLOT,
            )
        })
        .collect();
    let upd_intro = (bx + 14.0, by + 205.0, 340.0, 40.0);
    let upd_sources = (0..upd_sources_n)
        .map(|i| {
            (
                bx + 14.0 + i as f32 * 88.0,
                by + 260.0,
                REFINERY_SLOT,
                REFINERY_SLOT,
            )
        })
        .collect();
    let upd_targets = (0..upd_targets_n)
        .map(|i| {
            (
                bx + 14.0 + i as f32 * 88.0,
                by + 385.0,
                REFINERY_SLOT,
                REFINERY_SLOT,
            )
        })
        .collect();

    RefineryLayout {
        tab_refining,
        tab_upgrade,
        separator,
        tree,
        refine_static,
        refine_ores,
        refine_buttons,
        upd_title,
        upd_static,
        upd_intro,
        upd_sources,
        upd_targets,
    }
}

/// Interactive hits inside the refinery body.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefineryHit {
    /// Switch to this tab.
    Tab(crate::ui::RefineryTab),
    /// "Refine" button under `refine_ores[i]` — opens the amount dialog.
    Refine(usize),
    /// Press on `upd_sources[i]` — starts the ore drag.
    Source(usize),
}

pub fn refinery_hit(
    l: &RefineryLayout,
    tab: crate::ui::RefineryTab,
    refine_button_flags: &[bool],
    p: (f32, f32),
) -> Option<RefineryHit> {
    if point_in(l.tab_refining, p) {
        return Some(RefineryHit::Tab(crate::ui::RefineryTab::Refining));
    }
    if point_in(l.tab_upgrade, p) {
        return Some(RefineryHit::Tab(crate::ui::RefineryTab::Upgrade));
    }
    match tab {
        crate::ui::RefineryTab::Refining => l
            .refine_buttons
            .iter()
            .enumerate()
            .find(|(i, r)| {
                refine_button_flags.get(*i).copied().unwrap_or(false) && point_in(**r, p)
            })
            .map(|(i, _)| RefineryHit::Refine(i)),
        crate::ui::RefineryTab::Upgrade => l
            .upd_sources
            .iter()
            .position(|r| point_in(*r, p))
            .map(RefineryHit::Source),
    }
}

/// Target item under `p` on the Upgrade tab (drop destination index
/// into `upd_targets`).
pub fn refinery_drop_target(l: &RefineryLayout, p: (f32, f32)) -> Option<usize> {
    l.upd_targets.iter().position(|r| point_in(*r, p))
}

// ---------------------------------------------------------------------
// Refinery modal dialog (refine amount / upgrade count / ore replace)
// ---------------------------------------------------------------------

/// Centered popup over `anchor` (the refinery window's outer rect) —
/// the stock client's `refinement_refine` (240x150) /
/// `refinement_count` (180x110) / `refinement_update` (350x110) modal
/// windows collapsed into one layout; `size` selects which stock
/// window geometry to use.
pub struct LabDialogLayout {
    /// Outer panel (drawn with the window skin).
    pub panel: Rect,
    /// Title/description strip.
    pub header: Rect,
    /// `(amount, rect)` option chips — the ComboBox presets.
    pub options: Vec<(u64, Rect)>,
    pub ok: Rect,
    pub cancel: Rect,
}

pub fn lab_dialog_layout(anchor: Rect, options: &[u64], size: (f32, f32)) -> LabDialogLayout {
    let (pw, ph) = size;
    let panel = (
        anchor.0 + (anchor.2 - pw) * 0.5,
        anchor.1 + (anchor.3 - ph) * 0.5,
        pw,
        ph,
    );
    let header = (panel.0 + 10.0, panel.1 + 8.0, pw - 20.0, 40.0);
    let mut option_rects = Vec::with_capacity(options.len());
    if !options.is_empty() {
        // Amount presets above the OK/Cancel row — up to 4 per row,
        // fewer when the panel is narrow (the 180px count window).
        let (bw, bh, gap) = (50.0, 18.0, 4.0);
        let fit = (((pw - 20.0) + gap) / (bw + gap)).floor().max(1.0) as usize;
        let cols = options.len().min(4).min(fit).max(1);
        let rows = options.len().div_ceil(cols);
        let row_w = cols as f32 * bw + (cols - 1) as f32 * gap;
        let ox = panel.0 + (pw - row_w) * 0.5;
        let oy = panel.1 + ph - 30.0 - rows as f32 * (bh + gap);
        for (i, v) in options.iter().enumerate() {
            let (col, row) = (i % cols, i / cols);
            option_rects.push((
                *v,
                (
                    ox + col as f32 * (bw + gap),
                    oy + row as f32 * (bh + gap),
                    bw,
                    bh,
                ),
            ));
        }
    }
    let (bw, bh) = (70.0, 18.0);
    let by = panel.1 + ph - bh - 8.0;
    let ok = (panel.0 + pw * 0.5 - bw - 6.0, by, bw, bh);
    let cancel = (panel.0 + pw * 0.5 + 6.0, by, bw, bh);
    LabDialogLayout {
        panel,
        header,
        options: option_rects,
        ok,
        cancel,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::world::SharedWorld;
    use xenomit_packet_client::packet::{Packet, Value};

    #[test]
    fn minimap_body_fits_layout() {
        let body = (0.0, 0.0, MINIMAP_BODY.0, MINIMAP_BODY.1);
        let l = minimap_layout(body);
        assert!(l.map.0 >= 0.0 && l.map.0 + l.map.2 <= body.2);
        assert!(l.tab_galaxy.0 + l.tab_galaxy.2 <= body.2);
        assert!(l.zoom_in.1 + l.zoom_in.3 <= body.3);
        // No overlap between footer widgets.
        assert!(l.zoom_in.0 >= l.zoom_out.0 + l.zoom_out.2);
        assert!(l.readout.0 >= l.zoom_in.0 + l.zoom_in.2);
        assert!(l.tab_map.0 >= l.readout.0 + l.readout.2);
    }

    #[test]
    fn zoom_steps_clamp_and_snap() {
        assert_eq!(minimap_zoom_step(1.0, -1), 1.0);
        let z = minimap_zoom_step(1.0, 1);
        assert!((z - 1.5).abs() < 1e-5);
        assert_eq!(minimap_zoom_step(z, -1), 1.0);
        let mut z = 1.0;
        for _ in 0..20 {
            z = minimap_zoom_step(z, 1);
        }
        assert_eq!(z, MINIMAP_ZOOM_MAX);
    }

    #[test]
    fn view_centres_on_focus_and_clamps_at_edges() {
        let map = (10.0, 20.0, 200.0, 100.0);
        let bounds = (20000.0, 10000.0);
        // Zoom 1 shows everything regardless of focus.
        let v = MinimapView::new(map, bounds, 1.0, (500.0, 500.0));
        assert_eq!(v.origin, (0.0, 0.0));
        assert!((v.units_per_pixel() - 100.0).abs() < 1e-3);
        // Zoom 2 centred on the map middle.
        let v = MinimapView::new(map, bounds, 2.0, (10000.0, 5000.0));
        assert_eq!(v.origin, (5000.0, 2500.0));
        let s = v.to_screen(10000.0, 5000.0);
        assert!((s.0 - 110.0).abs() < 1e-3 && (s.1 - 70.0).abs() < 1e-3);
        // Focus in the corner clamps the window to the map edge.
        let v = MinimapView::new(map, bounds, 4.0, (0.0, 0.0));
        assert_eq!(v.origin, (0.0, 0.0));
        let v = MinimapView::new(map, bounds, 4.0, (20000.0, 10000.0));
        assert_eq!(v.origin, (15000.0, 7500.0));
        // Roundtrip.
        let w = v.to_world(v.to_screen(17000.0, 8000.0), bounds).unwrap();
        assert!((w.0 - 17000.0).abs() < 1.0 && (w.1 - 8000.0).abs() < 1.0);
        // Clicks past the rect clamp to map bounds.
        let w = v
            .to_world((map.0 + 9999.0, map.1 + 9999.0), bounds)
            .unwrap();
        assert_eq!(w, bounds);
    }

    #[test]
    fn pet_layout_scrolls_rows() {
        let body = (0.0, 0.0, 240.0, 214.0);
        let l = pet_layout(body, 12, 0.0);
        assert!(l.gears.len() < 12 && !l.gears.is_empty());
        assert_eq!(l.gears[0].0, 0);
        let max = pet_max_scroll(body, 12);
        assert!(max > 0.0);
        let l2 = pet_layout(body, 12, max);
        assert_eq!(l2.gears.last().unwrap().0, 11);
        assert_eq!(pet_max_scroll(body, 2), 0.0);
        assert!(l.deactivate.0 >= l.activate.0 + l.activate.2);
    }

    fn pkt(id: i16, fields: Vec<(&'static str, Value)>) -> Packet {
        Packet {
            id,
            name: None,
            fields: fields.into_iter().map(|(n, v)| (Some(n), v)).collect(),
        }
    }

    #[test]
    fn quest_rows_hit_cancel_and_select() {
        let mut w = SharedWorld::default();
        // Active quest with one counted condition.
        let cond = Value::Module(Box::new(pkt(
            2002,
            vec![
                ("id", Value::Int(10)),
                ("matches", Value::List(vec![])),
                ("type", Value::Int(6)),
                ("displayType", Value::Int(0)),
                ("targetValue", Value::F64(5.0)),
                ("mandatory", Value::Bool(true)),
                ("subConditions", Value::List(vec![])),
            ],
        )));
        let el = Value::Module(Box::new(pkt(
            2001,
            vec![("questCase", Value::Null), ("condition", cond)],
        )));
        let root = Value::Module(Box::new(pkt(
            2000,
            vec![
                ("id", Value::Int(1)),
                ("active", Value::Bool(true)),
                ("mandatory", Value::Bool(true)),
                ("ordered", Value::Bool(false)),
                ("mandatoryCount", Value::Int(0)),
                ("modifier", Value::List(vec![el])),
            ],
        )));
        let def = Value::Module(Box::new(pkt(
            2099,
            vec![
                ("id", Value::Int(77)),
                ("types", Value::List(vec![])),
                ("rootCase", root),
                ("rewards", Value::List(vec![])),
                ("textKeyFoundation", Value::Str("k".into())),
            ],
        )));
        w.apply_packet(&pkt(2017, vec![("quest", def)]));
        w.quests.available.insert(
            5,
            crate::net::quests::QuestSlim {
                id: 5,
                min_level: 3,
                ..Default::default()
            },
        );
        let (rows, h) = quest_rows(&w.quests, &w.locale, None, 10);
        assert!(h > 0.0);
        let body = (100.0, 100.0, 300.0, 280.0);
        // Cancel button of quest 77.
        let active = rows
            .iter()
            .find(|r| matches!(r.kind, QuestRowKind::Active { id: 77, .. }))
            .unwrap();
        let c = quest_cancel_rect(body, active.y, 0.0);
        assert_eq!(
            quest_hit(body, &rows, 0.0, (c.0 + 3.0, c.1 + 3.0)),
            Some(QuestHit::Cancel(77))
        );
        // Available row selects.
        let avail = rows
            .iter()
            .find(|r| matches!(r.kind, QuestRowKind::Available { id: 5, .. }))
            .unwrap();
        let r = quest_row_rect(body, avail, 0.0);
        assert_eq!(
            quest_hit(body, &rows, 0.0, (r.0 + 5.0, r.1 + 3.0)),
            Some(QuestHit::Select(5))
        );
        // Outside the window body nothing hits.
        assert_eq!(quest_hit(body, &rows, 0.0, (5.0, 5.0)), None);
        // Selected + no info → loading note appears after the row.
        let (rows2, _) = quest_rows(&w.quests, &w.locale, Some(5), 10);
        assert!(rows2
            .iter()
            .any(|r| matches!(&r.kind, QuestRowKind::Note(n) if n.contains("Loading"))));
        // Level-locked flag.
        assert!(rows
            .iter()
            .any(|r| matches!(r.kind, QuestRowKind::Available { locked: false, .. })));
        let (rows3, _) = quest_rows(&w.quests, &w.locale, None, 1);
        assert!(rows3
            .iter()
            .any(|r| matches!(r.kind, QuestRowKind::Available { locked: true, .. })));
    }

    #[test]
    fn settings_rows_hit_test() {
        let body = (0.0, 0.0, 230.0, 210.0);
        let items = settings_items();
        assert!(items.len() >= 3);
        let last = items.len() - 1;
        let r = settings_row_rect(body, last);
        assert!(r.1 + r.3 <= body.3, "last row must fit in the window");
        assert_eq!(
            settings_hit(body, (r.0 + 2.0, r.1 + 2.0)),
            Some(SettingsItem::ResetLayout)
        );
        assert_eq!(settings_hit(body, (-5.0, -5.0)), None);
    }

    #[test]
    fn ship_layout_buttons_inside_body() {
        let body = (0.0, 0.0, 252.0, 124.0);
        let l = ship_layout(body);
        for b in l.config {
            assert!(b.0 + b.2 <= body.2 && b.1 + b.3 <= body.3);
        }
        // Ore block (3 rows of 11px) fits below the config row.
        assert!(l.ore_origin.1 > l.config[0].1 + l.config[0].3);
        assert!(l.ore_origin.1 + 33.0 <= body.3 + 1.0);
    }

    fn refinery() -> RefineryLayout {
        refinery_layout((50.0, 68.0, 455.0, 507.0), 3, 4, 5, 5, 4)
    }

    #[test]
    fn refinery_layout_stays_inside_the_body() {
        let body = (50.0, 68.0, 455.0, 507.0);
        let l = refinery();
        for r in l
            .refine_static
            .iter()
            .chain(l.refine_ores.iter())
            .chain(l.upd_static.iter())
            .chain(l.upd_sources.iter())
            .chain(l.upd_targets.iter())
        {
            assert!(r.0 >= body.0 && r.0 + r.2 <= body.0 + body.2, "{r:?}");
            assert!(r.1 >= body.1 && r.1 + r.3 <= body.1 + body.3, "{r:?}");
        }
        // Refine button under the last (Promerium) row must still fit.
        let b = l.refine_buttons[2];
        assert!(b.1 + b.3 <= body.1 + body.3);
        // Promerium sits on its own second row below the button strip.
        assert!(l.refine_ores[2].1 > l.refine_ores[0].1 + REFINERY_SLOT);
    }

    #[test]
    fn refinery_hit_routes_tabs_buttons_and_drag_sources() {
        let l = refinery();
        use crate::ui::RefineryTab;
        assert_eq!(
            refinery_hit(
                &l,
                RefineryTab::Refining,
                &[true; 4],
                (l.tab_refining.0 + 5.0, l.tab_refining.1 + 5.0)
            ),
            Some(RefineryHit::Tab(RefineryTab::Refining))
        );
        assert_eq!(
            refinery_hit(
                &l,
                RefineryTab::Upgrade,
                &[],
                (l.tab_upgrade.0 + 5.0, l.tab_upgrade.1 + 5.0)
            ),
            Some(RefineryHit::Tab(RefineryTab::Upgrade))
        );
        // Refine button hits only when the ore has `button="true"`.
        let flags = [true, true, true, false];
        let b = l.refine_buttons[0];
        assert_eq!(
            refinery_hit(&l, RefineryTab::Refining, &flags, (b.0 + 4.0, b.1 + 4.0)),
            Some(RefineryHit::Refine(0))
        );
        let b3 = l.refine_buttons[3];
        assert_eq!(
            refinery_hit(&l, RefineryTab::Refining, &flags, (b3.0 + 4.0, b3.1 + 4.0)),
            None
        );
        // Upgrade tab: source slots start a drag, drop resolves a target.
        let s = l.upd_sources[1];
        assert_eq!(
            refinery_hit(&l, RefineryTab::Upgrade, &[], (s.0 + 4.0, s.1 + 4.0)),
            Some(RefineryHit::Source(1))
        );
        let t = l.upd_targets[3];
        assert_eq!(refinery_drop_target(&l, (t.0 + 4.0, t.1 + 4.0)), Some(3));
        assert_eq!(refinery_drop_target(&l, (5.0, 5.0)), None);
    }

    #[test]
    fn lab_dialog_layout_centres_on_the_anchor() {
        let opts = [1u64, 5, 10, 50, 100, 500, 1000, 3500];
        let l = lab_dialog_layout((100.0, 100.0, 455.0, 525.0), &opts, (240.0, 150.0));
        let c = (100.0 + 455.0 * 0.5, 100.0 + 525.0 * 0.5);
        let pc = (l.panel.0 + l.panel.2 * 0.5, l.panel.1 + l.panel.3 * 0.5);
        assert!((pc.0 - c.0).abs() < 1.0 && (pc.1 - c.1).abs() < 1.0);
        assert!(l.options.len() == 8 && l.ok.1 > l.options[0].1 .1);
        assert_eq!(l.options[7].0, 3500);
        let confirm = lab_dialog_layout((0.0, 0.0, 400.0, 400.0), &[], (350.0, 110.0));
        assert!(confirm.options.is_empty() && confirm.panel.3 < l.panel.3);
        assert!(confirm.panel.2 > l.panel.2); // 350px update window
    }
}
