//! Ordered set of HUD windows: stacking, press/drag routing and layout
//! persistence (`{assets_dir}/window_layout.json`).

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use super::window::{Rect, Window, WindowId, WindowPart};

/// Screen margin the default layout keeps from the viewport edges.
const MARGIN: f32 = 10.0;
/// Toolbar strip height reserved above the default top-left windows.
pub const TOOLBAR_RESERVE: f32 = 38.0;

/// What a press did to the window set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Press {
    /// No visible window under the cursor.
    Miss,
    /// A title-bar drag started (window raised).
    Drag(WindowId),
    /// `_` clicked — collapse toggled.
    Collapsed(WindowId),
    /// `×` clicked — window closed.
    Closed(WindowId),
    /// Click landed on the window body (window raised); the caller
    /// dispatches it to the content.
    Body(WindowId),
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct WindowLayoutEntry {
    pub id: String,
    pub x: f32,
    pub y: f32,
    #[serde(default)]
    pub collapsed: bool,
    #[serde(default)]
    pub closed: bool,
    #[serde(default)]
    pub z: u32,
}

/// On-disk layout. Window sizes are fixed per id, so only position and
/// state are stored.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct WindowLayout {
    #[serde(default)]
    pub version: u32,
    pub windows: Vec<WindowLayoutEntry>,
}

pub struct WindowManager {
    windows: Vec<Window>,
    next_z: u32,
    viewport: (f32, f32),
    path: Option<PathBuf>,
    dirty: bool,
}

/// Default window set for a `vw`x`vh` viewport: ship/user stacked
/// top-left under the toolbar, minimap bottom-right, log bottom-left,
/// everything else closed until the toolbar opens it.
pub fn default_windows(vw: f32, vh: f32) -> Vec<Window> {
    let top = MARGIN + TOOLBAR_RESERVE;
    let ship_h = WindowId::Ship.default_size().1;
    let mm = WindowId::Minimap.default_size();
    let log = WindowId::Log.default_size();
    let quests = WindowId::Quests.default_size();
    let settings = WindowId::Settings.default_size();
    let mut ws = vec![
        Window::new(WindowId::Ship, (MARGIN, top)),
        Window::new(WindowId::User, (MARGIN, top + ship_h + 6.0)),
        Window::new(WindowId::Minimap, (vw - mm.0 - MARGIN, vh - mm.1 - MARGIN)),
        Window::new(WindowId::Log, (MARGIN, vh - log.1 - MARGIN)),
        Window::new(WindowId::Quests, (vw * 0.5 - quests.0 * 0.5, 80.0)),
        Window::new(WindowId::Pet, (MARGIN + 262.0, top)),
        Window::new(WindowId::Settings, (vw * 0.5 - settings.0 * 0.5, 120.0)),
        Window::new(WindowId::Chat, (vw - 380.0, vh - 300.0)),
        Window::new(WindowId::Contacts, (vw - 280.0, 90.0)),
        Window::new(WindowId::Group, (vw - 270.0, 360.0)),
        // game.xml `refinement` window def: x=50 y=50.
        Window::new(WindowId::Refinery, (50.0, 50.0)),
    ];
    for w in &mut ws {
        w.closed = matches!(
            w.id,
            WindowId::Quests
                | WindowId::Pet
                | WindowId::Settings
                | WindowId::Chat
                | WindowId::Contacts
                | WindowId::Group
                | WindowId::Refinery
        );
        w.z = w.id as u32;
    }
    ws
}

impl WindowManager {
    pub fn with_defaults(vw: f32, vh: f32) -> Self {
        let windows = default_windows(vw, vh);
        let next_z = windows.len() as u32 + 1;
        let mut m = Self {
            windows,
            next_z,
            viewport: (vw, vh),
            path: None,
            dirty: false,
        };
        m.clamp_all();
        m
    }

    pub fn viewport(&self) -> (f32, f32) {
        self.viewport
    }

    pub fn get(&self, id: WindowId) -> Option<&Window> {
        self.windows.iter().find(|w| w.id == id)
    }

    pub fn get_mut(&mut self, id: WindowId) -> Option<&mut Window> {
        self.windows.iter_mut().find(|w| w.id == id)
    }

    pub fn is_open(&self, id: WindowId) -> bool {
        self.get(id).is_some_and(|w| w.visible())
    }

    /// Visible and not collapsed — the body is on screen.
    pub fn body_visible(&self, id: WindowId) -> bool {
        self.get(id).is_some_and(|w| w.visible() && !w.collapsed)
    }

    /// Body rect when the window is open and expanded.
    pub fn body(&self, id: WindowId) -> Option<Rect> {
        self.get(id)
            .filter(|w| w.visible() && !w.collapsed)
            .map(|w| w.body_rect())
    }

    /// Windows bottom → top (draw order).
    pub fn draw_order(&self) -> Vec<WindowId> {
        let mut v: Vec<&Window> = self.windows.iter().filter(|w| w.visible()).collect();
        v.sort_by_key(|w| w.z);
        v.into_iter().map(|w| w.id).collect()
    }

    pub fn bring_to_front(&mut self, id: WindowId) {
        let z = self.next_z;
        if let Some(w) = self.get_mut(id) {
            if w.z + 1 == z {
                return;
            }
            w.z = z;
            self.next_z += 1;
            self.dirty = true;
        }
    }

    pub fn open(&mut self, id: WindowId) {
        if let Some(w) = self.get_mut(id) {
            w.open();
            self.dirty = true;
        }
        self.bring_to_front(id);
        self.clamp_all();
    }

    pub fn close(&mut self, id: WindowId) {
        if let Some(w) = self.get_mut(id) {
            w.close();
            self.dirty = true;
        }
    }

    /// Toolbar toggle: closed → open + raised; open → closed. Returns the
    /// new open state.
    pub fn toggle(&mut self, id: WindowId) -> bool {
        if self.is_open(id) {
            self.close(id);
            false
        } else {
            self.open(id);
            true
        }
    }

    /// Topmost visible window containing `p`.
    pub fn topmost_at(&self, p: (f32, f32)) -> Option<WindowId> {
        self.draw_order()
            .into_iter()
            .rev()
            .find(|id| self.get(*id).is_some_and(|w| w.contains(p)))
    }

    /// True when `p` is over any visible window (HUD swallows the click).
    pub fn covers(&self, p: (f32, f32)) -> bool {
        self.topmost_at(p).is_some()
    }

    /// Mouse press: raise the window under `p`, run title-bar buttons or
    /// start a drag.
    pub fn press(&mut self, p: (f32, f32)) -> Press {
        let Some(id) = self.topmost_at(p) else {
            return Press::Miss;
        };
        self.bring_to_front(id);
        let part = self.get(id).and_then(|w| w.part_at(p));
        match part {
            Some(WindowPart::Close) => {
                self.close(id);
                Press::Closed(id)
            }
            Some(WindowPart::Collapse) => {
                if let Some(w) = self.get_mut(id) {
                    w.toggle_collapse();
                }
                self.dirty = true;
                Press::Collapsed(id)
            }
            Some(WindowPart::Title) => {
                if let Some(w) = self.get_mut(id) {
                    w.begin_drag(p);
                }
                Press::Drag(id)
            }
            _ => Press::Body(id),
        }
    }

    pub fn is_dragging(&self) -> bool {
        self.windows.iter().any(|w| w.is_dragging())
    }

    /// Mouse held: move the dragged window.
    pub fn drag(&mut self, p: (f32, f32)) {
        let vp = self.viewport;
        for w in &mut self.windows {
            if w.drag_to(p, vp) {
                self.dirty = true;
            }
        }
    }

    pub fn release(&mut self) {
        for w in &mut self.windows {
            w.end_drag();
        }
    }

    /// React to a viewport change: keep every title bar reachable.
    pub fn set_viewport(&mut self, vw: f32, vh: f32) {
        if (vw, vh) != self.viewport {
            self.viewport = (vw, vh);
            self.clamp_all();
        }
    }

    fn clamp_all(&mut self) {
        let vp = self.viewport;
        for w in &mut self.windows {
            w.clamp_to(vp);
        }
    }

    /// Back to the stock arrangement.
    pub fn reset_layout(&mut self) {
        let (vw, vh) = self.viewport;
        self.windows = default_windows(vw, vh);
        self.next_z = self.windows.len() as u32 + 1;
        self.dirty = true;
    }

    // ---- persistence -----------------------------------------------------

    pub fn layout(&self) -> WindowLayout {
        WindowLayout {
            version: 1,
            windows: self
                .windows
                .iter()
                .map(|w| WindowLayoutEntry {
                    id: w.id.key().to_owned(),
                    x: w.rect.0,
                    y: w.rect.1,
                    collapsed: w.collapsed,
                    closed: w.closed,
                    z: w.z,
                })
                .collect(),
        }
    }

    /// Apply a stored layout; unknown ids are ignored, missing ones keep
    /// their defaults.
    pub fn apply_layout(&mut self, layout: &WindowLayout) {
        for e in &layout.windows {
            let Some(id) = WindowId::from_key(&e.id) else {
                continue;
            };
            if !(e.x.is_finite() && e.y.is_finite()) {
                continue;
            }
            if let Some(w) = self.get_mut(id) {
                w.rect.0 = e.x;
                w.rect.1 = e.y;
                w.collapsed = e.collapsed;
                w.closed = e.closed;
                w.z = e.z;
            }
        }
        self.next_z = self.windows.iter().map(|w| w.z).max().unwrap_or(0) + 1;
        self.clamp_all();
    }

    /// Remember where the layout file lives and load it if present.
    /// Returns true when a layout was applied.
    pub fn set_path(&mut self, path: PathBuf) -> bool {
        let loaded = Self::read_layout(&path);
        self.path = Some(path);
        match loaded {
            Some(l) => {
                self.apply_layout(&l);
                self.dirty = false;
                true
            }
            None => false,
        }
    }

    pub fn read_layout(path: &Path) -> Option<WindowLayout> {
        let text = std::fs::read_to_string(path).ok()?;
        serde_json::from_str(&text).ok()
    }

    pub fn write_layout(layout: &WindowLayout, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        let text = serde_json::to_string_pretty(layout)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        std::fs::write(path, text)
    }

    /// Write the layout when something changed and no drag is mid-flight
    /// (drags dirty it every frame; the release flushes).
    pub fn flush(&mut self) {
        if !self.dirty || self.is_dragging() {
            return;
        }
        self.dirty = false;
        if let Some(path) = &self.path {
            if let Err(e) = Self::write_layout(&self.layout(), path) {
                log::warn!("could not save window layout {}: {e}", path.display());
            }
        }
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::window::TITLE_H;

    fn mgr() -> WindowManager {
        WindowManager::with_defaults(1280.0, 720.0)
    }

    #[test]
    fn defaults_open_core_windows_only() {
        let m = mgr();
        assert!(m.is_open(WindowId::Ship));
        assert!(m.is_open(WindowId::User));
        assert!(m.is_open(WindowId::Minimap));
        assert!(m.is_open(WindowId::Log));
        assert!(!m.is_open(WindowId::Quests));
        assert!(!m.is_open(WindowId::Pet));
        // Minimap sits in the bottom-right corner, fully on screen.
        let mm = m.get(WindowId::Minimap).unwrap().rect;
        assert!(mm.0 + mm.2 <= 1280.0 && mm.1 + mm.3 <= 720.0);
        assert!(mm.0 > 640.0 && mm.1 > 360.0);
    }

    #[test]
    fn press_raises_and_routes_parts() {
        let mut m = mgr();
        // Open Quests on top of Ship region: overlap, then click both.
        m.open(WindowId::Quests);
        let q = m.get(WindowId::Quests).unwrap().rect;
        let body_pt = (q.0 + 20.0, q.1 + TITLE_H + 20.0);
        assert_eq!(m.press(body_pt), Press::Body(WindowId::Quests));
        // Title press starts a drag and release ends it.
        let title_pt = (q.0 + 8.0, q.1 + 5.0);
        assert_eq!(m.press(title_pt), Press::Drag(WindowId::Quests));
        assert!(m.is_dragging());
        m.drag((title_pt.0 + 50.0, title_pt.1 + 30.0));
        let moved = m.get(WindowId::Quests).unwrap().rect;
        assert_eq!((moved.0, moved.1), (q.0 + 50.0, q.1 + 30.0));
        m.release();
        assert!(!m.is_dragging());
        assert_eq!(m.press((0.0, 700.0)), Press::Miss);
    }

    #[test]
    fn buttons_collapse_and_close() {
        let mut m = mgr();
        let w = m.get(WindowId::Ship).unwrap().clone();
        let k = w.collapse_button();
        assert_eq!(
            m.press((k.0 + 2.0, k.1 + 2.0)),
            Press::Collapsed(WindowId::Ship)
        );
        assert!(m.get(WindowId::Ship).unwrap().collapsed);
        let c = w.close_button();
        assert_eq!(
            m.press((c.0 + 2.0, c.1 + 2.0)),
            Press::Closed(WindowId::Ship)
        );
        assert!(!m.is_open(WindowId::Ship));
        // Toolbar toggle reopens it.
        assert!(m.toggle(WindowId::Ship));
        assert!(m.is_open(WindowId::Ship));
        assert!(!m.toggle(WindowId::Ship));
    }

    #[test]
    fn overlapping_windows_hit_topmost() {
        let mut m = mgr();
        m.open(WindowId::Quests);
        m.open(WindowId::Settings);
        // Put both at the same spot.
        let p = (400.0, 200.0);
        for id in [WindowId::Quests, WindowId::Settings] {
            let w = m.get_mut(id).unwrap();
            w.rect.0 = p.0;
            w.rect.1 = p.1;
        }
        let probe = (p.0 + 20.0, p.1 + 40.0);
        assert_eq!(m.topmost_at(probe), Some(WindowId::Settings));
        m.bring_to_front(WindowId::Quests);
        assert_eq!(m.topmost_at(probe), Some(WindowId::Quests));
        assert_eq!(*m.draw_order().last().unwrap(), WindowId::Quests);
    }

    #[test]
    fn layout_roundtrips_through_json_file() {
        let dir = std::env::temp_dir().join(format!("xenomit-layout-{}", std::process::id()));
        let path = dir.join("window_layout.json");
        let _ = std::fs::remove_dir_all(&dir);

        let mut m = mgr();
        assert!(!m.set_path(path.clone()), "no file yet");
        m.open(WindowId::Pet);
        {
            let w = m.get_mut(WindowId::Ship).unwrap();
            w.rect.0 = 321.0;
            w.rect.1 = 111.0;
            w.collapsed = true;
        }
        m.close(WindowId::Log);
        m.flush();
        assert!(!m.is_dirty());
        assert!(path.exists());

        let mut m2 = mgr();
        assert!(m2.set_path(path.clone()));
        let ship = m2.get(WindowId::Ship).unwrap();
        assert_eq!(
            (ship.rect.0, ship.rect.1, ship.collapsed),
            (321.0, 111.0, true)
        );
        assert!(m2.is_open(WindowId::Pet));
        assert!(!m2.is_open(WindowId::Log));
        // Z order survives: Pet was raised last.
        assert_eq!(*m2.draw_order().last().unwrap(), WindowId::Pet);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn garbage_layout_file_is_ignored() {
        let dir = std::env::temp_dir().join(format!("xenomit-layout-bad-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("window_layout.json");
        std::fs::write(&path, "{ not json").unwrap();
        let mut m = mgr();
        assert!(!m.set_path(path));
        assert!(m.is_open(WindowId::Ship));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn saved_offscreen_position_is_clamped_on_load() {
        let mut m = mgr();
        let layout = WindowLayout {
            version: 1,
            windows: vec![WindowLayoutEntry {
                id: "ship".into(),
                x: 99999.0,
                y: 99999.0,
                collapsed: false,
                closed: false,
                z: 1,
            }],
        };
        m.apply_layout(&layout);
        let r = m.get(WindowId::Ship).unwrap().rect;
        assert!(r.0 < 1280.0 && r.1 < 720.0);
        // Unknown ids are skipped.
        let junk = WindowLayout {
            version: 1,
            windows: vec![WindowLayoutEntry {
                id: "bogus".into(),
                x: 1.0,
                y: 1.0,
                collapsed: false,
                closed: false,
                z: 0,
            }],
        };
        m.apply_layout(&junk);
    }

    #[test]
    fn viewport_shrink_keeps_windows_reachable() {
        let mut m = mgr();
        m.set_viewport(640.0, 360.0);
        for id in WindowId::ALL {
            let r = m.get(id).unwrap().rect;
            assert!(r.0 <= 640.0 - 40.0 && r.1 <= 360.0, "{id:?} {r:?}");
        }
    }

    #[test]
    fn reset_restores_defaults() {
        let mut m = mgr();
        m.close(WindowId::Ship);
        m.get_mut(WindowId::User).unwrap().rect.0 = 500.0;
        m.reset_layout();
        assert!(m.is_open(WindowId::Ship));
        assert_eq!(m.get(WindowId::User).unwrap().rect.0, MARGIN);
    }
}
