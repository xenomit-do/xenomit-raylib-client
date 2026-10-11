//! Top-left window toolbar — the `SimpleWindowIcon` strip and its saved
//! order/visibility settings.

use super::chrome::{self, ButtonState};
use super::manager::WindowManager;
use super::window::{point_in, Rect, WindowId};
use crate::render::fonts::Fonts;
use raylib::prelude::*;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const BTN: f32 = 28.0;
pub const GAP: f32 = 3.0;
pub const ORIGIN: (f32, f32) = (10.0, 10.0);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolbarButton {
    Quest,
    Pet,
    User,
    Ship,
    Minimap,
    Log,
    Settings,
    Chat,
    Contacts,
    Group,
    Refinery,
}

impl ToolbarButton {
    pub const ALL: [ToolbarButton; 11] = [
        ToolbarButton::Quest,
        ToolbarButton::Pet,
        ToolbarButton::User,
        ToolbarButton::Ship,
        ToolbarButton::Minimap,
        ToolbarButton::Log,
        ToolbarButton::Settings,
        ToolbarButton::Chat,
        ToolbarButton::Contacts,
        ToolbarButton::Group,
        ToolbarButton::Refinery,
    ];

    /// The window this button toggles.
    pub fn window(self) -> Option<WindowId> {
        match self {
            ToolbarButton::Quest => Some(WindowId::Quests),
            ToolbarButton::Pet => Some(WindowId::Pet),
            ToolbarButton::User => Some(WindowId::User),
            ToolbarButton::Ship => Some(WindowId::Ship),
            ToolbarButton::Minimap => Some(WindowId::Minimap),
            ToolbarButton::Log => Some(WindowId::Log),
            ToolbarButton::Settings => Some(WindowId::Settings),
            ToolbarButton::Chat => Some(WindowId::Chat),
            ToolbarButton::Contacts => Some(WindowId::Contacts),
            ToolbarButton::Group => Some(WindowId::Group),
            ToolbarButton::Refinery => Some(WindowId::Refinery),
        }
    }

    pub fn tooltip(self) -> &'static str {
        match self {
            ToolbarButton::Quest => "Quests",
            ToolbarButton::Pet => "P.E.T. (P)",
            ToolbarButton::User => "User",
            ToolbarButton::Ship => "Ship",
            ToolbarButton::Minimap => "Minimap (M)",
            ToolbarButton::Log => "Log (L)",
            ToolbarButton::Settings => "Settings",
            ToolbarButton::Chat => "Chat",
            ToolbarButton::Contacts => "Contacts",
            ToolbarButton::Group => "Group",
            ToolbarButton::Refinery => "Refining",
        }
    }

    /// `FeatureWindowVO.id` used by the live `featuresMenu` icon atlas.
    pub fn icon_id(self) -> &'static str {
        match self {
            ToolbarButton::Quest => "quests",
            ToolbarButton::Pet => "pet",
            ToolbarButton::User => "user",
            ToolbarButton::Ship => "ship",
            ToolbarButton::Minimap => "minimap",
            ToolbarButton::Log => "log",
            ToolbarButton::Settings => "settings",
            ToolbarButton::Chat => "chat",
            ToolbarButton::Contacts => "contacts",
            ToolbarButton::Group => "group",
            // `SimpleWindow.id` == `GuiConstants.REFINEMENT_WINDOW`.
            ToolbarButton::Refinery => "refinement",
        }
    }

    pub fn icon_region(self, state: &str) -> String {
        format!("{}_{}", self.icon_id(), state)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolbarConfig {
    #[serde(default = "config_version")]
    pub version: u32,
    #[serde(default = "default_order")]
    pub order: Vec<ToolbarButton>,
    #[serde(default)]
    pub hidden: Vec<ToolbarButton>,
}

fn config_version() -> u32 {
    1
}
fn default_order() -> Vec<ToolbarButton> {
    ToolbarButton::ALL.to_vec()
}

impl Default for ToolbarConfig {
    fn default() -> Self {
        Self {
            version: 1,
            order: default_order(),
            hidden: Vec::new(),
        }
    }
}

/// Validated persistent toolbar settings.
pub struct ToolbarLayout {
    pub config: ToolbarConfig,
    path: Option<PathBuf>,
    dirty: bool,
}

impl Default for ToolbarLayout {
    fn default() -> Self {
        Self {
            config: ToolbarConfig::default(),
            path: None,
            dirty: false,
        }
    }
}

impl ToolbarLayout {
    pub fn set_path(&mut self, path: PathBuf) -> bool {
        let loaded = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str::<ToolbarConfig>(&s).ok());
        self.path = Some(path);
        if let Some(mut config) = loaded {
            let mut normalized = Vec::with_capacity(ToolbarButton::ALL.len());
            for b in config.order.drain(..).chain(ToolbarButton::ALL) {
                if ToolbarButton::ALL.contains(&b) && !normalized.contains(&b) {
                    normalized.push(b);
                }
            }
            config.order = normalized;
            config.hidden.retain(|b| ToolbarButton::ALL.contains(b));
            config.hidden.sort_by_key(|b| {
                ToolbarButton::ALL
                    .iter()
                    .position(|x| x == b)
                    .unwrap_or(usize::MAX)
            });
            config.hidden.dedup();
            self.config = config;
            self.dirty = false;
            true
        } else {
            false
        }
    }

    pub fn buttons(&self, editing: bool) -> Vec<ToolbarButton> {
        self.config
            .order
            .iter()
            .copied()
            .filter(|b| editing || !self.config.hidden.contains(b))
            .collect()
    }

    pub fn move_button(&mut self, from: ToolbarButton, to: ToolbarButton) {
        if from == to {
            return;
        }
        let Some(i) = self.config.order.iter().position(|b| *b == from) else {
            return;
        };
        if !self.config.order.iter().any(|b| *b == to) {
            return;
        }
        let b = self.config.order.remove(i);
        let Some(target) = self.config.order.iter().position(|button| *button == to) else {
            self.config.order.insert(i, b);
            return;
        };
        self.config.order.insert(target, b);
        self.dirty = true;
    }

    pub fn toggle_hidden(&mut self, b: ToolbarButton) {
        if self.config.hidden.contains(&b) {
            self.config.hidden.retain(|v| *v != b);
        } else if self
            .config
            .order
            .iter()
            .any(|v| *v != b && !self.config.hidden.contains(v))
        {
            self.config.hidden.push(b);
        }
        self.dirty = true;
    }

    pub fn flush(&mut self) {
        if !self.dirty {
            return;
        }
        self.dirty = false;
        if let Some(path) = &self.path {
            if let Err(e) = write_config(&self.config, path) {
                log::warn!("could not save toolbar layout {}: {e}", path.display());
            }
        }
    }
}

fn write_config(config: &ToolbarConfig, path: &Path) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let text = serde_json::to_string_pretty(config)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    let mut temp = path.as_os_str().to_owned();
    temp.push(".tmp");
    let temp = PathBuf::from(temp);
    std::fs::write(&temp, text)?;
    if let Err(e) = std::fs::rename(&temp, path) {
        let _ = std::fs::remove_file(&temp);
        return Err(e);
    }
    Ok(())
}

/// Button rects in toolbar order.
pub fn toolbar_rects() -> Vec<(ToolbarButton, Rect)> {
    toolbar_rects_for(&ToolbarButton::ALL)
}

pub fn toolbar_rects_for(buttons: &[ToolbarButton]) -> Vec<(ToolbarButton, Rect)> {
    buttons
        .iter()
        .enumerate()
        .map(|(i, b)| (*b, (ORIGIN.0 + i as f32 * (BTN + GAP), ORIGIN.1, BTN, BTN)))
        .collect()
}

/// Bounding rect of the whole strip (swallows clicks between buttons).
pub fn toolbar_bounds() -> Rect {
    toolbar_bounds_for(&ToolbarButton::ALL)
}

pub fn toolbar_bounds_for(buttons: &[ToolbarButton]) -> Rect {
    let n = buttons.len() as f32;
    if n == 0.0 {
        return (ORIGIN.0, ORIGIN.1, BTN, BTN);
    }
    (ORIGIN.0, ORIGIN.1, n * BTN + (n - 1.0) * GAP, BTN)
}

pub fn toolbar_edit_rect(_buttons: &[ToolbarButton]) -> Rect {
    // Keep the edit hit target at a fixed x-coordinate. Hidden icons are
    // revealed while editing, and must not move the control under the mouse.
    let bounds = toolbar_bounds_for(&ToolbarButton::ALL);
    (bounds.0 + bounds.2 + GAP, bounds.1, 36.0, BTN)
}

pub fn toolbar_controls_bounds(_buttons: &[ToolbarButton], _editing: bool) -> Rect {
    let base = toolbar_bounds_for(&ToolbarButton::ALL);
    let edit = toolbar_edit_rect(&ToolbarButton::ALL);
    let w = edit.0 + edit.2 - base.0;
    (base.0, base.1, w, base.3)
}

pub fn toolbar_hit(p: (f32, f32)) -> Option<ToolbarButton> {
    toolbar_hit_for(p, &ToolbarButton::ALL)
}

pub fn toolbar_hit_for(p: (f32, f32), buttons: &[ToolbarButton]) -> Option<ToolbarButton> {
    toolbar_rects_for(buttons)
        .into_iter()
        .find(|(_, r)| point_in(*r, p))
        .map(|(b, _)| b)
}

pub fn draw_toolbar(
    d: &mut RaylibDrawHandle,
    fonts: &Fonts,
    windows: &WindowManager,
    mouse: (f32, f32),
) {
    draw_toolbar_config(
        d,
        fonts,
        windows,
        &ToolbarLayout::default(),
        false,
        mouse,
        &std::collections::HashMap::new(),
        &crate::render::sprites::TextureCache::default(),
        false,
    );
}

pub fn draw_toolbar_config(
    d: &mut RaylibDrawHandle,
    fonts: &Fonts,
    windows: &WindowManager,
    layout: &ToolbarLayout,
    editing: bool,
    mouse: (f32, f32),
    icons: &std::collections::HashMap<(String, String), crate::assets::SpriteId>,
    textures: &crate::render::sprites::TextureCache,
    use_game_background: bool,
) {
    let buttons = layout.buttons(editing);
    for (b, r) in toolbar_rects_for(&buttons) {
        let open = b.window().is_some_and(|w| windows.is_open(w));
        let hidden = layout.config.hidden.contains(&b);
        let hover = point_in(r, mouse);
        let bg_key = if open || hover {
            "bg_active.png"
        } else {
            "bg_standard.png"
        };
        let game_button = use_game_background
            && icons
                .get(&("ui".to_owned(), bg_key.to_owned()))
                .and_then(|id| textures.get(*id))
                .is_some_and(|clip| {
                    draw_toolbar_button_bg(d, clip, r, if hidden { 125 } else { 255 })
                });
        if !game_button {
            chrome::draw_button(
                d,
                fonts,
                r,
                "",
                ButtonState {
                    hover,
                    active: open,
                    disabled: hidden,
                },
            );
        }
        let col = if hidden {
            Color::new(96, 108, 126, 255)
        } else if open {
            Color::WHITE
        } else {
            Color::new(190, 208, 230, 255)
        };
        let icon_state = if hover {
            "hover"
        } else if open {
            "select"
        } else {
            "normal"
        };
        let icon_key = (
            "featuresMenu_atlasXml".to_owned(),
            b.icon_region(icon_state),
        );
        let drawn = icons
            .get(&icon_key)
            .and_then(|id| textures.get(*id))
            .is_some_and(|clip| draw_atlas_icon(d, clip, r, col));
        if !drawn {
            draw_icon(d, b, r, col);
        }
        if editing && hidden {
            d.draw_line(
                r.0 as i32 + 5,
                r.1 as i32 + 5,
                (r.0 + r.2 - 5.0) as i32,
                (r.1 + r.3 - 5.0) as i32,
                Color::new(240, 100, 85, 220),
            );
            d.draw_line(
                (r.0 + r.2 - 5.0) as i32,
                r.1 as i32 + 5,
                r.0 as i32 + 5,
                (r.1 + r.3 - 5.0) as i32,
                Color::new(240, 100, 85, 220),
            );
        }
    }
    let edit = toolbar_edit_rect(&buttons);
    chrome::draw_button(
        d,
        fonts,
        edit,
        if editing { "Done" } else { "Edit" },
        ButtonState {
            hover: point_in(edit, mouse),
            active: editing,
            disabled: false,
        },
    );
}

fn draw_toolbar_button_bg(
    d: &mut RaylibDrawHandle,
    clip: &crate::render::sprites::GpuClip,
    r: Rect,
    alpha: u8,
) -> bool {
    if clip.frames.is_empty() {
        return false;
    }
    let (x0, y0, x1, y1) = clip.bounds;
    let (w, h) = (x1 - x0, y1 - y0);
    if w <= 0.0 || h <= 0.0 {
        return false;
    }
    let scale = ((r.2 - 2.0) / w).min((r.3 - 2.0) / h);
    let pos = (r.0 + (r.2 - w * scale) * 0.5, r.1 + (r.3 - h * scale) * 0.5);
    crate::render::sprites::TextureCache::draw_frame_offset(
        d,
        clip,
        0,
        pos,
        0.0,
        (scale, scale),
        Color::new(255, 255, 255, alpha),
        (-x0, -y0),
    );
    true
}

fn draw_atlas_icon(
    d: &mut RaylibDrawHandle,
    clip: &crate::render::sprites::GpuClip,
    r: Rect,
    tint: Color,
) -> bool {
    let Some(frame) = clip.frames.first() else {
        return false;
    };
    let (x0, y0, x1, y1) = clip.bounds;
    let (w, h) = (x1 - x0, y1 - y0);
    if w <= 0.0 || h <= 0.0 {
        return false;
    }
    let scale = ((r.2 - 4.0) / w).min((r.3 - 4.0) / h);
    let pos = (r.0 + (r.2 - w * scale) * 0.5, r.1 + (r.3 - h * scale) * 0.5);
    crate::render::sprites::TextureCache::draw_frame_offset(
        d,
        clip,
        0,
        pos,
        0.0,
        (scale, scale),
        tint,
        (-x0, -y0),
    );
    // Keep the frame borrow live until after draw_frame_offset.
    let _ = frame;
    true
}

/// Little vector glyphs, drawn inside a `BTN`² cell.
fn draw_icon(d: &mut RaylibDrawHandle, b: ToolbarButton, r: Rect, col: Color) {
    let (cx, cy) = (r.0 + r.2 * 0.5, r.1 + r.3 * 0.5);
    let c = Vector2::new(cx, cy);
    match b {
        ToolbarButton::Quest => {
            // Scroll with text lines.
            d.draw_rectangle_lines(cx as i32 - 6, cy as i32 - 8, 12, 16, col);
            for i in 0..3 {
                d.draw_line(
                    cx as i32 - 3,
                    cy as i32 - 4 + i * 4,
                    cx as i32 + 3,
                    cy as i32 - 4 + i * 4,
                    col,
                );
            }
        }
        ToolbarButton::Pet => {
            // Paw: pad + three toes.
            d.draw_circle_v(Vector2::new(cx, cy + 3.0), 4.5, col);
            for dx in [-5.5f32, 0.0, 5.5] {
                let lift = if dx == 0.0 { 1.5 } else { 0.0 };
                d.draw_circle_v(Vector2::new(cx + dx, cy - 4.0 - lift), 2.2, col);
            }
        }
        ToolbarButton::User => {
            d.draw_circle_v(Vector2::new(cx, cy - 3.5), 3.8, col);
            d.draw_circle_sector(Vector2::new(cx, cy + 9.0), 8.5, 180.0, 360.0, 14, col);
        }
        ToolbarButton::Ship => {
            d.draw_triangle(
                Vector2::new(cx, cy - 9.0),
                Vector2::new(cx - 7.0, cy + 7.0),
                Vector2::new(cx + 7.0, cy + 7.0),
                col,
            );
            d.draw_triangle(
                Vector2::new(cx, cy + 2.0),
                Vector2::new(cx + 3.0, cy + 7.0),
                Vector2::new(cx - 3.0, cy + 7.0),
                Color::new(16, 28, 49, 255),
            );
        }
        ToolbarButton::Minimap => {
            d.draw_rectangle_lines(cx as i32 - 8, cy as i32 - 6, 16, 12, col);
            d.draw_line(cx as i32 - 8, cy as i32, cx as i32 + 8, cy as i32, col);
            d.draw_line(cx as i32, cy as i32 - 6, cx as i32, cy as i32 + 6, col);
            d.draw_circle_v(c, 1.8, col);
        }
        ToolbarButton::Log => {
            for i in 0..4 {
                let w = if i == 3 { 6 } else { 12 };
                d.draw_rectangle(cx as i32 - 7, cy as i32 - 7 + i * 4, w, 2, col);
            }
        }
        ToolbarButton::Settings => {
            d.draw_ring(c, 3.5, 6.5, 0.0, 360.0, 18, col);
            for k in 0..8 {
                let a = (k as f32) * std::f32::consts::FRAC_PI_4;
                let (s, co) = a.sin_cos();
                d.draw_line_ex(
                    Vector2::new(cx + co * 6.5, cy + s * 6.5),
                    Vector2::new(cx + co * 9.0, cy + s * 9.0),
                    2.0,
                    col,
                );
            }
        }
        ToolbarButton::Chat => {
            d.draw_rectangle_lines(cx as i32 - 8, cy as i32 - 7, 16, 11, col);
            d.draw_triangle(
                Vector2::new(cx - 4.0, cy + 4.0),
                Vector2::new(cx - 6.0, cy + 9.0),
                Vector2::new(cx + 1.0, cy + 4.0),
                col,
            );
        }
        ToolbarButton::Contacts => {
            d.draw_circle_v(Vector2::new(cx - 4.0, cy - 3.0), 3.0, col);
            d.draw_circle_v(Vector2::new(cx + 4.5, cy - 1.0), 2.5, col);
            d.draw_circle_sector(Vector2::new(cx - 4.0, cy + 9.0), 6.5, 180.0, 360.0, 12, col);
        }
        ToolbarButton::Group => {
            for (dx, dy) in [(0.0f32, -5.0f32), (-5.5, 3.0), (5.5, 3.0)] {
                d.draw_circle_v(Vector2::new(cx + dx, cy + dy), 3.2, col);
            }
        }
        ToolbarButton::Refinery => {
            // Flask: neck + sloped body.
            d.draw_rectangle(cx as i32 - 2, cy as i32 - 9, 4, 5, col);
            d.draw_triangle(
                Vector2::new(cx - 2.0, cy - 4.0),
                Vector2::new(cx + 2.0, cy - 4.0),
                Vector2::new(cx + 8.0, cy + 8.0),
                col,
            );
            d.draw_triangle(
                Vector2::new(cx + 2.0, cy - 4.0),
                Vector2::new(cx - 2.0, cy - 4.0),
                Vector2::new(cx - 8.0, cy + 8.0),
                col,
            );
            d.draw_rectangle(cx as i32 - 8, cy as i32 + 7, 16, 2, col);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buttons_do_not_overlap_and_hit_individually() {
        let rects = toolbar_rects();
        assert_eq!(rects.len(), ToolbarButton::ALL.len());
        for w in rects.windows(2) {
            assert!(w[0].1 .0 + w[0].1 .2 < w[1].1 .0);
        }
        for (b, r) in &rects {
            assert_eq!(toolbar_hit((r.0 + 2.0, r.1 + 2.0)), Some(*b));
        }
        // Gap between buttons belongs to no button but is inside the strip.
        let g = (rects[0].1 .0 + BTN + 1.0, rects[0].1 .1 + 5.0);
        assert_eq!(toolbar_hit(g), None);
        assert!(point_in(toolbar_bounds(), g));
        assert_eq!(toolbar_hit((600.0, 600.0)), None);
    }

    #[test]
    fn window_buttons_map_to_windows() {
        assert_eq!(ToolbarButton::Quest.window(), Some(WindowId::Quests));
        assert_eq!(ToolbarButton::Minimap.window(), Some(WindowId::Minimap));
        // Every window except none is reachable from the toolbar.
        for id in WindowId::ALL {
            assert!(
                ToolbarButton::ALL.iter().any(|b| b.window() == Some(id)),
                "{id:?} has no toolbar button"
            );
        }
    }

    #[test]
    fn edit_controls_keep_their_position_when_icons_are_hidden() {
        let all = ToolbarButton::ALL.to_vec();
        let visible = vec![ToolbarButton::Quest, ToolbarButton::Ship];
        assert_eq!(toolbar_edit_rect(&all), toolbar_edit_rect(&visible));
        assert_eq!(
            toolbar_controls_bounds(&all, false),
            toolbar_controls_bounds(&visible, true)
        );
    }

    #[test]
    fn reordered_and_hidden_toolbar_settings_survive_a_save() {
        let root = std::env::temp_dir().join(format!("xenomit-toolbar-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let path = root.join("toolbar_layout.json");
        let mut layout = ToolbarLayout::default();
        layout.set_path(path.clone());
        layout.move_button(ToolbarButton::Quest, ToolbarButton::Ship);
        layout.toggle_hidden(ToolbarButton::User);
        layout.flush();

        let mut loaded = ToolbarLayout::default();
        assert!(loaded.set_path(path));
        assert_eq!(loaded.config.order[2], ToolbarButton::Quest);
        assert!(loaded.config.hidden.contains(&ToolbarButton::User));
        let _ = std::fs::remove_dir_all(root);
    }
}
