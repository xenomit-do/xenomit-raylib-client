//! Geometry for the F3 display-settings overlay (stub window until the
//! windowing layer grows a real settings window). Pure — the raylib
//! drawing is in `render::hud::settings`, input in `Game::poll_input`.

use crate::assets::uberlib::TextureQuality;
use crate::game::input::Rect;
use crate::net::SettingFlag;
use crate::ui::preferences::RenderPreferences;

pub const PANEL_W: f32 = 250.0;
pub const TITLE_H: f32 = 26.0;
pub const ROW_H: f32 = 22.0;
pub const FOOTER_H: f32 = 20.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenderSetting {
    Camera,
    Collectables,
    Npcs,
    Ships,
    Drones,
    Backgrounds,
    Portals,
    Effects,
    Explosions,
    /// Low/Medium/High texture-quality row (not a 2D/3D component toggle —
    /// kept out of `COMPONENTS`; hit-tested via `render_quality_hit`).
    TextureQuality,
    General,
    Save,
    Cancel,
    Reset,
}

impl RenderSetting {
    pub const COMPONENTS: [Self; 9] = [
        Self::Camera,
        Self::Collectables,
        Self::Npcs,
        Self::Ships,
        Self::Drones,
        Self::Backgrounds,
        Self::Portals,
        Self::Effects,
        Self::Explosions,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Camera => "Camera",
            Self::Collectables => "Boxes & loot",
            Self::Npcs => "NPCs",
            Self::Ships => "Ships",
            Self::Drones => "Drones",
            Self::Backgrounds => "Backgrounds",
            Self::Portals => "Portals",
            Self::Effects => "Effects",
            Self::Explosions => "Explosions",
            Self::TextureQuality => "Texture quality",
            Self::General => "General",
            Self::Save => "Save",
            Self::Cancel => "Cancel",
            Self::Reset => "Reset",
        }
    }

    pub fn get(self, p: RenderPreferences) -> Option<bool> {
        Some(match self {
            Self::Camera => p.camera_3d,
            Self::Collectables => p.collectables_3d,
            Self::Npcs => p.npcs_3d,
            Self::Ships => p.ships_3d,
            Self::Drones => p.drones_3d,
            Self::Backgrounds => p.backgrounds_3d,
            Self::Portals => p.portals_3d,
            Self::Effects => p.effects_3d,
            Self::Explosions => p.explosions_3d,
            _ => return None,
        })
    }

    pub fn set(self, p: &mut RenderPreferences, use_3d: bool) {
        let field = match self {
            Self::Camera => &mut p.camera_3d,
            Self::Collectables => &mut p.collectables_3d,
            Self::Npcs => &mut p.npcs_3d,
            Self::Ships => &mut p.ships_3d,
            Self::Drones => &mut p.drones_3d,
            Self::Backgrounds => &mut p.backgrounds_3d,
            Self::Portals => &mut p.portals_3d,
            Self::Effects => &mut p.effects_3d,
            Self::Explosions => &mut p.explosions_3d,
            _ => return,
        };
        *field = use_3d;
    }

    pub fn toggle(self, p: &mut RenderPreferences) {
        if let Some(current) = self.get(*p) {
            self.set(p, !current);
        }
    }
}

/// Uncommitted component rendering settings. Save returns a value for the
/// preferences store; Cancel simply drops this draft.
#[derive(Clone, Debug)]
pub struct RenderSettingsDraft(pub RenderPreferences);

impl RenderSettingsDraft {
    pub fn new(saved: RenderPreferences) -> Self {
        Self(saved)
    }
    pub fn reset(&mut self) {
        self.0 = RenderPreferences::default();
    }
    pub fn save(self) -> RenderPreferences {
        self.0
    }
}

pub const RENDER_PANEL_W: f32 = 570.0;
pub const RENDER_HEADER_H: f32 = 83.0;
const RENDER_FOOTER_H: f32 = 62.0;
const MODE_BTN_W: f32 = 54.0;
const MODE_BTN_GAP: f32 = 6.0;

/// `(2D button, 3D button)` aligned to the right edge of a component row.
pub fn component_mode_buttons(row: Rect) -> (Rect, Rect) {
    let h = (row.3 * 0.72).clamp(18.0, 30.0);
    let total = MODE_BTN_W * 2.0 + MODE_BTN_GAP;
    let x0 = row.0 + row.2 - total - 10.0;
    let y0 = row.1 + (row.3 - h) * 0.5;
    (
        (x0, y0, MODE_BTN_W, h),
        (x0 + MODE_BTN_W + MODE_BTN_GAP, y0, MODE_BTN_W, h),
    )
}

/// `(Low, Medium, High)` buttons aligned to the right edge of the
/// texture-quality row — same metrics as `component_mode_buttons`.
pub fn quality_mode_buttons(row: Rect) -> [Rect; 3] {
    let h = (row.3 * 0.72).clamp(18.0, 30.0);
    let total = MODE_BTN_W * 3.0 + MODE_BTN_GAP * 2.0;
    let x0 = row.0 + row.2 - total - 10.0;
    let y0 = row.1 + (row.3 - h) * 0.5;
    [
        (x0, y0, MODE_BTN_W, h),
        (x0 + MODE_BTN_W + MODE_BTN_GAP, y0, MODE_BTN_W, h),
        (x0 + (MODE_BTN_W + MODE_BTN_GAP) * 2.0, y0, MODE_BTN_W, h),
    ]
}

pub fn render_layout(vw: f32, vh: f32) -> (Rect, Vec<(RenderSetting, Rect)>) {
    // Component rows plus the texture-quality row below them.
    let row_count = (RenderSetting::COMPONENTS.len() + 1) as f32;
    let width = (vw - 24.0).clamp(240.0, RENDER_PANEL_W);
    let row_h = ((vh - 32.0 - RENDER_HEADER_H - RENDER_FOOTER_H) / row_count).clamp(16.0, 42.0);
    let h = RENDER_HEADER_H + row_count * row_h + RENDER_FOOTER_H;
    let panel = ((vw - width) * 0.5, (vh - h) * 0.5, width, h);
    let mut all = RenderSetting::COMPONENTS
        .iter()
        .enumerate()
        .map(|(i, c)| {
            (
                *c,
                (
                    panel.0 + 23.0,
                    panel.1 + RENDER_HEADER_H + i as f32 * row_h,
                    width - 46.0,
                    row_h,
                ),
            )
        })
        .collect::<Vec<_>>();
    all.push((
        RenderSetting::TextureQuality,
        (
            panel.0 + 23.0,
            panel.1 + RENDER_HEADER_H + RenderSetting::COMPONENTS.len() as f32 * row_h,
            width - 46.0,
            row_h,
        ),
    ));
    let y = panel.1 + h - 49.0;
    let button_w = (width - 48.0) / 3.0;
    all.push((RenderSetting::Save, (panel.0 + 12.0, y, button_w, 37.0)));
    all.push((
        RenderSetting::Cancel,
        (panel.0 + 24.0 + button_w, y, button_w, 37.0),
    ));
    all.push((
        RenderSetting::Reset,
        (panel.0 + 36.0 + button_w * 2.0, y, button_w, 37.0),
    ));
    all.push((
        RenderSetting::General,
        (panel.0 + 23.0, panel.1 + 43.0, (width - 46.0) * 0.45, 28.0),
    ));
    (panel, all)
}

/// Component row hit: `(setting, use_3d)`.
pub fn render_component_hit(vw: f32, vh: f32, p: (f32, f32)) -> Option<(RenderSetting, bool)> {
    for (field, row) in render_layout(vw, vh).1 {
        if !RenderSetting::COMPONENTS.contains(&field) {
            continue;
        }
        let (two_d, three_d) = component_mode_buttons(row);
        if crate::game::input::point_in(two_d, p) {
            return Some((field, false));
        }
        if crate::game::input::point_in(three_d, p) {
            return Some((field, true));
        }
    }
    None
}

/// Texture-quality row hit: which of the three quality buttons holds `p`.
pub fn render_quality_hit(vw: f32, vh: f32, p: (f32, f32)) -> Option<TextureQuality> {
    let (_, row) = render_layout(vw, vh)
        .1
        .into_iter()
        .find(|(f, _)| *f == RenderSetting::TextureQuality)?;
    for (button, quality) in quality_mode_buttons(row).iter().zip(TextureQuality::ALL) {
        if crate::game::input::point_in(*button, p) {
            return Some(quality);
        }
    }
    None
}

pub fn render_hit(vw: f32, vh: f32, p: (f32, f32)) -> Option<RenderSetting> {
    render_layout(vw, vh)
        .1
        .into_iter()
        .find(|(field, r)| {
            !RenderSetting::COMPONENTS.contains(field)
                && *field != RenderSetting::TextureQuality
                && crate::game::input::point_in(*r, p)
        })
        .map(|(v, _)| v)
}

/// `(panel, rows)` — rows in `SettingFlag::ALL` order.
pub fn layout(vw: f32, vh: f32) -> (Rect, Vec<(SettingFlag, Rect)>) {
    let n = SettingFlag::ALL.len() as f32;
    let h = TITLE_H + n * ROW_H + FOOTER_H;
    let panel = (vw * 0.5 - PANEL_W * 0.5, vh * 0.5 - h * 0.5, PANEL_W, h);
    let rows = SettingFlag::ALL
        .iter()
        .enumerate()
        .map(|(i, f)| {
            (
                *f,
                (
                    panel.0 + 6.0,
                    panel.1 + TITLE_H + i as f32 * ROW_H,
                    PANEL_W - 12.0,
                    ROW_H,
                ),
            )
        })
        .collect();
    (panel, rows)
}

/// Row under `p`, if any.
pub fn hit(vw: f32, vh: f32, p: (f32, f32)) -> Option<SettingFlag> {
    layout(vw, vh)
        .1
        .into_iter()
        .find(|(_, r)| crate::game::input::point_in(*r, p))
        .map(|(f, _)| f)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::input::point_in;

    #[test]
    fn rows_fit_in_panel_and_hit_resolves() {
        let (panel, rows) = layout(1280.0, 720.0);
        assert_eq!(rows.len(), SettingFlag::ALL.len());
        for (_, r) in &rows {
            assert!(point_in(panel, (r.0 + 1.0, r.1 + 1.0)));
        }
        let (f, r) = rows[2];
        assert_eq!(hit(1280.0, 720.0, (r.0 + 3.0, r.1 + 3.0)), Some(f));
        assert_eq!(hit(1280.0, 720.0, (0.0, 0.0)), None);
    }

    #[test]
    fn render_preferences_support_save_cancel_and_reset_drafts() {
        let saved = RenderPreferences::default();
        let mut draft = RenderSettingsDraft::new(saved);
        RenderSetting::Ships.toggle(&mut draft.0);
        // Cancel is dropping the draft; committed preferences remain intact.
        assert!(!saved.ships_3d);
        let mut reset = RenderSettingsDraft::new(draft.0);
        reset.reset();
        assert_eq!(reset.save(), saved);
        let mut commit = RenderSettingsDraft::new(saved);
        RenderSetting::Portals.toggle(&mut commit.0);
        let committed = commit.save();
        assert!(committed.portals_3d);
        assert!(!committed.ships_3d);
    }

    #[test]
    fn render_controls_fit_and_hit_each_action() {
        let (panel, controls) = render_layout(1280.0, 720.0);
        // 9 component rows + texture-quality row + Save/Cancel/Reset + General tab.
        assert_eq!(controls.len(), RenderSetting::COMPONENTS.len() + 5);
        for (_, r) in &controls {
            assert!(crate::game::input::point_in(panel, (r.0 + 1.0, r.1 + 1.0)));
        }
        let save = controls
            .iter()
            .find(|(c, _)| *c == RenderSetting::Save)
            .unwrap()
            .1;
        assert_eq!(
            render_hit(1280.0, 720.0, (save.0 + 2.0, save.1 + 2.0)),
            Some(RenderSetting::Save)
        );
    }

    #[test]
    fn component_settings_fit_landscape_phone_and_desktop() {
        for (vw, vh) in [
            (640.0, 360.0),
            (960.0, 540.0),
            (1280.0, 576.0),
            (2400.0, 1080.0),
        ] {
            let (panel, rows) = render_layout(vw, vh);
            assert!(panel.0 >= 0.0 && panel.1 >= 0.0);
            assert!(panel.0 + panel.2 <= vw && panel.1 + panel.3 <= vh);
            for (action, r) in rows {
                assert!(r.0 >= panel.0 && r.1 >= panel.1);
                assert!(r.0 + r.2 <= panel.0 + panel.2 && r.1 + r.3 <= panel.1 + panel.3);
                if RenderSetting::COMPONENTS.contains(&action) {
                    let (_, three_d) = component_mode_buttons(r);
                    assert_eq!(
                        render_component_hit(vw, vh, (three_d.0 + 2.0, three_d.1 + 2.0)),
                        Some((action, true))
                    );
                } else if action == RenderSetting::TextureQuality {
                    // Not a footer button — render_hit must skip it; the
                    // three quality buttons resolve via render_quality_hit.
                    assert_eq!(render_hit(vw, vh, (r.0 + r.2 * 0.5, r.1 + r.3 * 0.5)), None);
                    for (b, q) in quality_mode_buttons(r).iter().zip(TextureQuality::ALL) {
                        assert_eq!(render_quality_hit(vw, vh, (b.0 + 2.0, b.1 + 2.0)), Some(q));
                    }
                } else {
                    assert_eq!(
                        render_hit(vw, vh, (r.0 + r.2 * 0.5, r.1 + r.3 * 0.5)),
                        Some(action)
                    );
                }
            }
        }
    }

    #[test]
    fn texture_quality_row_hits_each_button_and_is_draft_only() {
        let (_, controls) = render_layout(1280.0, 720.0);
        let row = controls
            .iter()
            .find(|(f, _)| *f == RenderSetting::TextureQuality)
            .unwrap()
            .1;
        let buttons = quality_mode_buttons(row);
        for (b, q) in buttons.iter().zip(TextureQuality::ALL) {
            assert_eq!(
                render_quality_hit(1280.0, 720.0, (b.0 + 2.0, b.1 + 2.0)),
                Some(q)
            );
        }
        // Row label area and empty space resolve to nothing.
        assert_eq!(
            render_quality_hit(1280.0, 720.0, (row.0 + 4.0, row.1 + row.3 * 0.5)),
            None
        );
        assert_eq!(render_quality_hit(1280.0, 720.0, (0.0, 0.0)), None);
        // The quality row is neither a 2D/3D component nor a footer action.
        assert_eq!(
            render_hit(1280.0, 720.0, (row.0 + row.2 * 0.5, row.1 + row.3 * 0.5)),
            None
        );
        assert_eq!(
            RenderSetting::TextureQuality.get(RenderPreferences::default()),
            None
        );
        // Draft semantics: mutating the draft leaves saved prefs untouched.
        let saved = RenderPreferences::default();
        let mut draft = RenderSettingsDraft::new(saved);
        draft.0.texture_quality = TextureQuality::Medium;
        assert_eq!(saved.texture_quality, TextureQuality::High);
        assert_eq!(draft.save().texture_quality, TextureQuality::Medium);
    }

    #[test]
    fn component_mode_buttons_set_independent_2d_and_3d() {
        let mut prefs = RenderPreferences::default();
        RenderSetting::Ships.set(&mut prefs, true);
        RenderSetting::Portals.set(&mut prefs, false);
        assert!(prefs.ships_3d);
        assert!(!prefs.portals_3d);
        let (_, row) = render_layout(1280.0, 720.0)
            .1
            .into_iter()
            .find(|(f, _)| *f == RenderSetting::Ships)
            .unwrap();
        let (two_d, _) = component_mode_buttons(row);
        assert_eq!(
            render_component_hit(1280.0, 720.0, (two_d.0 + 2.0, two_d.1 + 2.0)),
            Some((RenderSetting::Ships, false))
        );
    }
}
