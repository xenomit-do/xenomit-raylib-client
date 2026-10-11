//! Floating HUD windows: framework (`window`, `manager`), look
//! (`chrome`, `toolbar`) and the pure per-window content layouts
//! (`panels`). The window *bodies* are painted by `render::hud::*`;
//! clicks are dispatched by `game::ui_input`.

pub mod chrome;
pub mod manager;
pub mod panels;
pub mod preferences;
pub mod toolbar;
pub mod window;

pub use manager::{Press, WindowManager};
pub use panels::MinimapTab;
pub use window::{Window, WindowId};

use crate::game::input::FlyoutSource;

/// Open ammo flyout: where it hangs and the item ids it lists.
#[derive(Clone, Debug)]
pub struct Flyout {
    pub source: FlyoutSource,
    pub items: Vec<String>,
}

/// Per-session UI state that is not window geometry.
#[derive(Clone, Debug)]
pub struct UiState {
    pub minimap_zoom: f32,
    pub minimap_tab: MinimapTab,
    /// Quest row expanded in the Available list.
    pub selected_quest: Option<i32>,
    pub flyout: Option<Flyout>,
    /// Quest list/definitions were requested once this session.
    pub quests_requested: bool,
    pub contacts_requested: bool,
    pub selected_contact: Option<i32>,
    pub selected_group_member: Option<i32>,
    pub contacts_name: String,
    pub group_invite_name: String,
    pub social_text_focus: Option<SocialTextFocus>,
    /// Active refinery tab (`lab_btn_refinement`/`lab_btn_update`).
    pub refinery_tab: RefineryTab,
    /// Modal refinery dialog (refine amount / upgrade amount /
    /// replace-ore confirm) — swallows clicks while open.
    pub lab_dialog: Option<LabDialog>,
    /// Upgrade-tab source ore being dragged (`typeValue`); set on slot
    /// press, resolved on release (drop over a target item = upgrade).
    pub refinery_drag: Option<i32>,
    /// Next due `LabUpdateRequest` while the refinery window is expanded
    /// (stock client refreshes the lab item amounts once per second).
    pub lab_refresh: Option<std::time::Instant>,
    /// Refresh ticks since the window was opened — the stock client
    /// auto-minimizes the refinery after ~600 refreshes (10 min idle).
    pub lab_ticks: u16,
}

/// Refinery window tab (`RefinementManager.showRefinement`/`showUpdate`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefineryTab {
    /// `<refinement>` — static ores + producible ores with Refine buttons.
    Refining,
    /// `<update>` — drag sources onto lab items to apply ore upgrades.
    Upgrade,
}

/// Modal dialog shown over the refinery window — the stock client's
/// `REFINEMENT_REFINE_WINDOW`/`REFINEMENT_COUNT_WINDOW`/
/// `REFINEMENT_UPDATE_WINDOW` popups collapsed into one enum.
#[derive(Clone, Debug)]
pub enum LabDialog {
    /// Refine-amount picker for a producible ore (`LabRefinementRequest`).
    Refine {
        ore_type: i32,
        /// Preset amounts (`combo_options`); last entry is the max.
        options: Vec<u64>,
        amount: u64,
    },
    /// "Replace the already-applied ore?" — Update → Count dialog.
    UpgradeConfirm {
        /// `LabItemKind::index`.
        item: usize,
        ore_type: i32,
        /// `upgrade_max_count` for the item/dragged ore.
        max: u64,
    },
    /// Upgrade-amount picker (`LabUpdateItemRequest`).
    UpgradeAmount {
        /// `LabItemKind::index`.
        item: usize,
        ore_type: i32,
        options: Vec<u64>,
        amount: u64,
    },
}

impl LabDialog {
    /// Stock modal window size — the `<window>` defs in game.xml:
    /// `refinement_refine` 240x150, `refinement_count` 180x110,
    /// `refinement_update` 350x110.
    pub fn panel_size(&self) -> (f32, f32) {
        match self {
            LabDialog::Refine { .. } => (240.0, 150.0),
            LabDialog::UpgradeAmount { .. } => (180.0, 110.0),
            LabDialog::UpgradeConfirm { .. } => (350.0, 110.0),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SocialTextFocus {
    ContactsName,
    GroupInvite,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            minimap_zoom: 1.0,
            minimap_tab: MinimapTab::Map,
            selected_quest: None,
            flyout: None,
            quests_requested: false,
            contacts_requested: false,
            selected_contact: None,
            selected_group_member: None,
            contacts_name: String::new(),
            group_invite_name: String::new(),
            social_text_focus: None,
            refinery_tab: RefineryTab::Refining,
            lab_dialog: None,
            refinery_drag: None,
            lab_refresh: None,
            lab_ticks: 0,
        }
    }
}
