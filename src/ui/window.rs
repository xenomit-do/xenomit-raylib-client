//! One floating HUD window: frame geometry, drag, collapse, close.
//!
//! Pure data + hit testing (no raylib) so the whole interaction model is
//! unit-testable; `ui::chrome` draws it, `ui::manager::WindowManager`
//! orders/persists a set of them.

/// Screen-space rectangle `(x, y, w, h)` in pixels.
pub type Rect = (f32, f32, f32, f32);

/// `(x,y) in (rx,ry,rw,rh)`.
pub fn point_in(r: Rect, p: (f32, f32)) -> bool {
    p.0 >= r.0 && p.0 < r.0 + r.2 && p.1 >= r.1 && p.1 < r.1 + r.3
}

/// Title bar height in px — `SimpleWindow.WINDOW_TITLEBAR_HEIGHT` /
/// `UICONTAINER_Y` in the stock client (content begins at y = 25).
pub const TITLE_H: f32 = 25.0;
/// `closeBtn`/`minimizeBtn` authored sizes (21x13 / 21x12) and their
/// right-edge distance (`_closeBtnInitDistance`/`MINIMIZE_BTN_OFFSET`).
pub const TITLE_BTN: (f32, f32) = (21.0, 13.0);
const CLOSE_BTN_OFFSET: f32 = 26.0;
const MINIMIZE_BTN_GAP: f32 = 22.0;
/// Margin that must stay on screen when a window is dragged away.
const KEEP_VISIBLE: f32 = 40.0;

/// Every window the HUD can show.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum WindowId {
    Ship,
    User,
    Minimap,
    Quests,
    Pet,
    Log,
    Settings,
    Chat,
    Contacts,
    Group,
    /// `WindowIDModule.WINDOWID_REFINEMENT` — Refining/Upgrade lab.
    Refinery,
}

impl WindowId {
    pub const ALL: [WindowId; 11] = [
        WindowId::Ship,
        WindowId::User,
        WindowId::Minimap,
        WindowId::Quests,
        WindowId::Pet,
        WindowId::Log,
        WindowId::Settings,
        WindowId::Chat,
        WindowId::Contacts,
        WindowId::Group,
        WindowId::Refinery,
    ];

    /// Stable key used in `window_layout.json`.
    pub fn key(self) -> &'static str {
        match self {
            WindowId::Ship => "ship",
            WindowId::User => "user",
            WindowId::Minimap => "minimap",
            WindowId::Quests => "quests",
            WindowId::Pet => "pet",
            WindowId::Log => "log",
            WindowId::Settings => "settings",
            WindowId::Chat => "chat",
            WindowId::Contacts => "contacts",
            WindowId::Group => "group",
            WindowId::Refinery => "refinery",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|w| w.key() == key)
    }

    /// Default title text.
    pub fn title(self) -> &'static str {
        match self {
            WindowId::Ship => "SHIP",
            WindowId::User => "USER",
            WindowId::Minimap => "MINIMAP",
            WindowId::Quests => "QUESTS",
            WindowId::Pet => "P.E.T.",
            WindowId::Log => "LOG",
            WindowId::Settings => "SETTINGS",
            WindowId::Chat => "CHAT",
            WindowId::Contacts => "CONTACTS",
            WindowId::Group => "GROUP",
            // Resolved to `title_refinement` ("Refining") at draw time.
            WindowId::Refinery => "REFINING",
        }
    }

    /// Full (expanded) outer size `(w, h)` including the title bar.
    pub fn default_size(self) -> (f32, f32) {
        match self {
            WindowId::Ship => (252.0, TITLE_H + 124.0),
            WindowId::User => (252.0, TITLE_H + 112.0),
            WindowId::Minimap => (
                crate::ui::panels::MINIMAP_BODY.0,
                TITLE_H + crate::ui::panels::MINIMAP_BODY.1,
            ),
            WindowId::Quests => (300.0, 300.0),
            WindowId::Pet => (240.0, TITLE_H + 214.0),
            WindowId::Log => (340.0, 150.0),
            WindowId::Settings => (230.0, TITLE_H + 250.0),
            WindowId::Chat => (360.0, TITLE_H + 250.0),
            WindowId::Contacts => (260.0, TITLE_H + 250.0),
            WindowId::Group => (250.0, TITLE_H + 230.0),
            // game.xml `refinement` window def: 455x525 incl. the title.
            // Stock `<window id="refinement">` is 455x525 total —
            // TITLE_H is inside it.
            WindowId::Refinery => (455.0, 525.0),
        }
    }
}

/// Which part of a window a point landed on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowPart {
    Title,
    Collapse,
    Close,
    Body,
}

#[derive(Clone, Debug)]
pub struct Window {
    pub id: WindowId,
    pub title: String,
    /// Expanded outer rect `(x, y, w, h)`; `h` includes the title bar.
    pub rect: Rect,
    pub collapsed: bool,
    pub closed: bool,
    /// Stacking order — higher draws on top.
    pub z: u32,
    /// Grab point inside the window while a title-bar drag is active.
    pub drag_offset: Option<(f32, f32)>,
    /// Whether the title bar shows a close button.
    pub closable: bool,
    /// Body scroll offset in px (lists); 0 = top (for the log: bottom).
    pub scroll: f32,
}

impl Window {
    pub fn new(id: WindowId, pos: (f32, f32)) -> Self {
        let (w, h) = id.default_size();
        Self {
            id,
            title: id.title().to_owned(),
            rect: (pos.0, pos.1, w, h),
            collapsed: false,
            closed: false,
            z: 0,
            drag_offset: None,
            closable: true,
            scroll: 0.0,
        }
    }

    pub fn visible(&self) -> bool {
        !self.closed
    }

    /// The rect that is actually on screen (title bar only when collapsed).
    pub fn outer_rect(&self) -> Rect {
        if self.collapsed {
            self.title_bar()
        } else {
            self.rect
        }
    }

    pub fn title_bar(&self) -> Rect {
        (self.rect.0, self.rect.1, self.rect.2, TITLE_H)
    }

    /// Content area below the title bar (valid even when collapsed).
    pub fn body_rect(&self) -> Rect {
        (
            self.rect.0,
            self.rect.1 + TITLE_H,
            self.rect.2,
            (self.rect.3 - TITLE_H).max(0.0),
        )
    }

    pub fn close_button(&self) -> Rect {
        (
            self.rect.0 + self.rect.2 - CLOSE_BTN_OFFSET,
            self.rect.1 + 1.0,
            TITLE_BTN.0,
            TITLE_BTN.1,
        )
    }

    /// `minimizeBtn` — one button-width left of `closeBtn`.
    pub fn collapse_button(&self) -> Rect {
        let c = self.close_button();
        let x = if self.closable {
            c.0 - MINIMIZE_BTN_GAP
        } else {
            c.0
        };
        (x, c.1, TITLE_BTN.0, TITLE_BTN.1 - 1.0)
    }

    pub fn contains(&self, p: (f32, f32)) -> bool {
        self.visible() && point_in(self.outer_rect(), p)
    }

    pub fn hit_collapse(&self, p: (f32, f32)) -> bool {
        self.visible() && point_in(self.collapse_button(), p)
    }

    pub fn hit_close(&self, p: (f32, f32)) -> bool {
        self.visible() && self.closable && point_in(self.close_button(), p)
    }

    /// Title-bar drag handle: the bar minus its buttons.
    pub fn hit_title(&self, p: (f32, f32)) -> bool {
        self.visible()
            && point_in(self.title_bar(), p)
            && !self.hit_collapse(p)
            && !self.hit_close(p)
    }

    pub fn part_at(&self, p: (f32, f32)) -> Option<WindowPart> {
        if !self.contains(p) {
            return None;
        }
        Some(if self.hit_close(p) {
            WindowPart::Close
        } else if self.hit_collapse(p) {
            WindowPart::Collapse
        } else if point_in(self.title_bar(), p) {
            WindowPart::Title
        } else {
            WindowPart::Body
        })
    }

    pub fn is_dragging(&self) -> bool {
        self.drag_offset.is_some()
    }

    pub fn begin_drag(&mut self, p: (f32, f32)) {
        self.drag_offset = Some((p.0 - self.rect.0, p.1 - self.rect.1));
    }

    /// Move to follow the cursor; returns true when the position changed.
    pub fn drag_to(&mut self, p: (f32, f32), viewport: (f32, f32)) -> bool {
        let Some(off) = self.drag_offset else {
            return false;
        };
        let before = (self.rect.0, self.rect.1);
        self.rect.0 = p.0 - off.0;
        self.rect.1 = p.1 - off.1;
        self.clamp_to(viewport);
        before != (self.rect.0, self.rect.1)
    }

    pub fn end_drag(&mut self) {
        self.drag_offset = None;
    }

    pub fn toggle_collapse(&mut self) {
        self.collapsed = !self.collapsed;
        self.drag_offset = None;
    }

    pub fn close(&mut self) {
        self.closed = true;
        self.drag_offset = None;
    }

    pub fn open(&mut self) {
        self.closed = false;
    }

    /// Keep the title bar reachable inside the viewport.
    pub fn clamp_to(&mut self, viewport: (f32, f32)) {
        if viewport.0 <= 0.0 || viewport.1 <= 0.0 {
            return;
        }
        let w = self.rect.2;
        let min_x = KEEP_VISIBLE - w;
        let max_x = (viewport.0 - KEEP_VISIBLE).max(min_x);
        self.rect.0 = self.rect.0.clamp(min_x, max_x);
        self.rect.1 = self.rect.1.clamp(0.0, (viewport.1 - TITLE_H).max(0.0));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn win() -> Window {
        Window::new(WindowId::Ship, (100.0, 100.0))
    }

    #[test]
    fn title_bar_and_buttons_hit_separately() {
        let w = win();
        let t = w.title_bar();
        // Left part of the bar is the drag handle.
        assert!(w.hit_title((t.0 + 6.0, t.1 + 5.0)));
        // Buttons are not title-drag.
        let c = w.close_button();
        let p = (c.0 + 2.0, c.1 + 2.0);
        assert!(w.hit_close(p) && !w.hit_title(p));
        assert_eq!(w.part_at(p), Some(WindowPart::Close));
        let k = w.collapse_button();
        let p = (k.0 + 2.0, k.1 + 2.0);
        assert!(w.hit_collapse(p) && !w.hit_title(p) && !w.hit_close(p));
        assert_eq!(w.part_at(p), Some(WindowPart::Collapse));
        // Body below the bar.
        assert_eq!(
            w.part_at((t.0 + 10.0, t.1 + TITLE_H + 10.0)),
            Some(WindowPart::Body)
        );
        assert_eq!(w.part_at((0.0, 0.0)), None);
    }

    #[test]
    fn drag_follows_cursor_and_keeps_title_visible() {
        let mut w = win();
        w.begin_drag((110.0, 105.0));
        assert!(w.is_dragging());
        assert!(w.drag_to((310.0, 205.0), (1280.0, 720.0)));
        assert_eq!((w.rect.0, w.rect.1), (300.0, 200.0));
        // Way off-screen: clamped so the title bar is still grabbable.
        w.drag_to((9999.0, 9999.0), (1280.0, 720.0));
        assert!(w.rect.0 <= 1280.0 - KEEP_VISIBLE);
        assert!(w.rect.1 <= 720.0 - TITLE_H);
        w.drag_to((-9999.0, -9999.0), (1280.0, 720.0));
        assert!(w.rect.0 + w.rect.2 >= KEEP_VISIBLE);
        assert_eq!(w.rect.1, 0.0);
        w.end_drag();
        assert!(!w.drag_to((0.0, 0.0), (1280.0, 720.0)));
    }

    #[test]
    fn collapse_shrinks_hit_area_to_title_bar() {
        let mut w = win();
        let inside_body = (w.rect.0 + 10.0, w.rect.1 + TITLE_H + 20.0);
        assert!(w.contains(inside_body));
        w.toggle_collapse();
        assert!(w.collapsed);
        assert!(!w.contains(inside_body));
        assert!(w.contains((w.rect.0 + 10.0, w.rect.1 + 4.0)));
        w.toggle_collapse();
        assert!(w.contains(inside_body));
    }

    #[test]
    fn closed_windows_ignore_input() {
        let mut w = win();
        w.close();
        assert!(!w.visible());
        assert_eq!(w.part_at((w.rect.0 + 5.0, w.rect.1 + 5.0)), None);
        w.open();
        assert!(w.visible());
    }

    #[test]
    fn non_closable_window_moves_collapse_button_right() {
        let mut w = win();
        let closable = w.collapse_button();
        w.closable = false;
        assert!(w.collapse_button().0 > closable.0);
        let c = w.close_button();
        assert!(!w.hit_close((c.0 + 1.0, c.1 + 1.0)));
    }

    #[test]
    fn keys_roundtrip() {
        for id in WindowId::ALL {
            assert_eq!(WindowId::from_key(id.key()), Some(id));
        }
        assert_eq!(WindowId::from_key("nope"), None);
    }
}
