//! HUD window input routing: toolbar, window title bars, window bodies,
//! ammo flyout and wheel scrolling. Everything here takes plain cursor
//! coordinates (no raylib handle) so it is unit-testable; `Game::poll_input`
//! feeds it the mouse state.

use super::input::{self, FlyoutSource, HudArea, HudLayout};
use super::Game;
use crate::net::commands::MENU_SOURCE_SLOT_BAR;
use crate::net::{NetCommand, SharedWorld};
use crate::ui::manager::Press;
use crate::ui::panels::{self, MinimapView, QuestHit, SettingsItem};
use crate::ui::toolbar::{self, ToolbarButton};
use crate::ui::window::{point_in, Rect, WindowId};
use crate::ui::{Flyout, MinimapTab};
use raylib::prelude::{KeyboardKey, RaylibHandle};
use std::path::PathBuf;

impl Game {
    /// Remember where the window layout lives and load it. A loaded layout
    /// is trusted as-is; without one the defaults are fitted to the real
    /// viewport on the first input pass.
    pub fn set_layout_path(&mut self, path: PathBuf) {
        if self.windows.set_path(path) {
            self.ui_ready = true;
        }
    }

    pub fn set_toolbar_path(&mut self, path: PathBuf) {
        self.toolbar.set_path(path);
    }

    pub fn set_ui_preferences_path(&mut self, path: PathBuf) {
        self.preferences.set_path(path);
    }

    /// Keep the window set in step with the viewport (first call fits the
    /// default arrangement to the actual screen size).
    pub(super) fn ensure_ui(&mut self, vw: f32, vh: f32) {
        if vw <= 0.0 || vh <= 0.0 {
            return;
        }
        self.windows.set_viewport(vw, vh);
        if !self.ui_ready {
            self.windows.reset_layout();
            self.ui_ready = true;
        }
    }

    /// Action bar / open flyout rects for this frame.
    pub fn hud_for_frame(&self) -> HudLayout {
        let (vw, vh) = self.camera.viewport;
        let mut hud = input::hud_layout(vw, vh, self.selected.is_some());
        if self.preferences.mobile_hud {
            // The mobile fire/rocket controls replace the desktop action bar.
            hud.action_bar = (0.0, 0.0, 0.0, 0.0);
            hud.laser_slot = (0.0, 0.0, 0.0, 0.0);
            hud.rocket_slot = (0.0, 0.0, 0.0, 0.0);
        }
        if let Some(f) = &self.ui.flyout {
            hud = hud.with_flyout(f.source, &f.items, vw, vh);
        }
        hud
    }

    /// True when `p` is over a window, the toolbar or a HUD panel — a
    /// right-drag pan must not start there.
    pub fn ui_covers(&self, p: (f32, f32), hud: &HudLayout) -> bool {
        self.windows.covers(p)
            || point_in(
                toolbar::toolbar_controls_bounds(
                    &self.toolbar.buttons(self.toolbar_editing),
                    self.toolbar_editing,
                ),
                p,
            )
            || input::hud_hit(hud, p).is_some()
    }

    /// Minimap mapping for the current zoom, centred on the hero.
    pub fn minimap_view(&self, world: &SharedWorld, body: Rect) -> MinimapView {
        let l = panels::minimap_layout(body);
        let focus = world
            .hero_pos()
            .unwrap_or((self.bounds.0 * 0.5, self.bounds.1 * 0.5));
        MinimapView::new(l.map, self.bounds, self.ui.minimap_zoom, focus)
    }

    /// Right click: a slot with ammo-family siblings opens (or toggles) the
    /// ammo flyout. Returns true when the click was consumed by the HUD.
    pub(super) fn ui_right_click(
        &mut self,
        world: &SharedWorld,
        p: (f32, f32),
        hud: &HudLayout,
        pressed: bool,
    ) -> bool {
        if !pressed {
            return false;
        }
        if self.toolbar_editing {
            let buttons = self.toolbar.buttons(true);
            if let Some(b) = toolbar::toolbar_hit_for(p, &buttons) {
                self.toolbar.toggle_hidden(b);
                return true;
            }
            if point_in(toolbar::toolbar_controls_bounds(&buttons, true), p) {
                return true;
            }
        }
        let area = input::hud_hit(hud, p);
        let (src, items) = match area {
            Some(HudArea::LaserSlot) => (
                FlyoutSource::Laser,
                input::flyout_items(&world.menu, "ammunition_laser_"),
            ),
            Some(HudArea::RocketSlot) => (
                FlyoutSource::Rocket,
                input::flyout_items(&world.menu, "ammunition_rocket_"),
            ),
            _ => {
                let on_flyout = hud.flyout.as_ref().is_some_and(|f| point_in(f.panel, p));
                if !on_flyout {
                    self.ui.flyout = None;
                }
                return area.is_some();
            }
        };
        if items.is_empty() || self.ui.flyout.as_ref().is_some_and(|f| f.source == src) {
            self.ui.flyout = None;
        } else {
            self.ui.flyout = Some(Flyout { source: src, items });
        }
        true
    }

    /// Left press routed through flyout → toolbar → windows. Returns true
    /// when something consumed it (the world / static HUD never sees it).
    pub(super) fn ui_press(
        &mut self,
        world: &SharedWorld,
        p: (f32, f32),
        hud: &HudLayout,
        area: Option<HudArea>,
        out: &mut Vec<NetCommand>,
    ) -> bool {
        // Modal refinery dialog swallows every press while open
        // (`activateModalLayer` in the stock popups).
        if self.lab_dialog_press(p, out) {
            return true;
        }
        if let Some(HudArea::FlyoutItem(i)) = area {
            if let Some((id, _)) = hud.flyout.as_ref().and_then(|f| f.rows.get(i)) {
                self.pick_flyout_item(world, id, out);
            }
            self.ui.flyout = None;
            return true;
        }
        let on_flyout = hud.flyout.as_ref().is_some_and(|f| point_in(f.panel, p));
        if !on_flyout {
            self.ui.flyout = None;
        }

        let buttons = self.toolbar.buttons(self.toolbar_editing);
        if point_in(toolbar::toolbar_edit_rect(&buttons), p) {
            self.toolbar_editing = !self.toolbar_editing;
            self.toolbar_drag = None;
            return true;
        }
        if let Some(b) = toolbar::toolbar_hit_for(p, &buttons) {
            if self.toolbar_editing {
                if self.toolbar.config.hidden.contains(&b) {
                    self.toolbar.toggle_hidden(b);
                } else {
                    self.toolbar_drag = Some((b, p));
                }
            } else {
                self.toolbar_click(b, world, out);
            }
            return true;
        }
        if point_in(
            toolbar::toolbar_controls_bounds(&buttons, self.toolbar_editing),
            p,
        ) {
            return true;
        }
        match self.windows.press(p) {
            Press::Miss => false,
            Press::Body(id) => {
                self.window_body_click(id, world, p, out);
                true
            }
            Press::Drag(_) | Press::Collapsed(_) | Press::Closed(_) => true,
        }
    }

    pub(super) fn poll_social_text_input(
        &mut self,
        rl: &mut RaylibHandle,
        out: &mut Vec<NetCommand>,
    ) -> bool {
        let Some(focus) = self.ui.social_text_focus else {
            return false;
        };
        while let Some(code) = rl.get_char_pressed() {
            if let Some(ch) = char::from_u32(code as u32).filter(|c| !c.is_control()) {
                let target = match focus {
                    crate::ui::SocialTextFocus::ContactsName => &mut self.ui.contacts_name,
                    crate::ui::SocialTextFocus::GroupInvite => &mut self.ui.group_invite_name,
                };
                if target.chars().count() < 32 {
                    target.push(ch);
                }
            }
        }
        if rl.is_key_pressed(KeyboardKey::KEY_BACKSPACE)
            || rl.is_key_pressed_repeat(KeyboardKey::KEY_BACKSPACE)
        {
            match focus {
                crate::ui::SocialTextFocus::ContactsName => {
                    self.ui.contacts_name.pop();
                }
                crate::ui::SocialTextFocus::GroupInvite => {
                    self.ui.group_invite_name.pop();
                }
            }
        }
        if rl.is_key_pressed(KeyboardKey::KEY_ESCAPE) {
            self.ui.social_text_focus = None;
        } else if rl.is_key_pressed(KeyboardKey::KEY_ENTER)
            || rl.is_key_pressed(KeyboardKey::KEY_KP_ENTER)
        {
            match focus {
                crate::ui::SocialTextFocus::ContactsName => {
                    let name = self.ui.contacts_name.trim().to_owned();
                    if !name.is_empty() {
                        out.push(NetCommand::ContactRequest {
                            request_type: 0,
                            user_id: 0,
                            user_name: name,
                        });
                        self.ui.contacts_name.clear();
                    }
                }
                crate::ui::SocialTextFocus::GroupInvite => {
                    let name = self.ui.group_invite_name.trim().to_owned();
                    if !name.is_empty() {
                        out.push(NetCommand::InviteToGroup { name });
                        self.ui.group_invite_name.clear();
                    }
                }
            }
            self.ui.social_text_focus = None;
        }
        true
    }

    /// Flyout row picked: select that ammo (`MenuActionRequest`).
    fn pick_flyout_item(&mut self, world: &SharedWorld, id: &str, out: &mut Vec<NetCommand>) {
        if let Some(item) = world.menu.items.get(id) {
            out.push(NetCommand::MenuAction {
                item_id: item.id.clone(),
                action_type: item.action_style.action_type(),
                source_type: MENU_SOURCE_SLOT_BAR,
            });
        }
    }

    fn toolbar_click(&mut self, b: ToolbarButton, world: &SharedWorld, out: &mut Vec<NetCommand>) {
        let Some(id) = b.window() else {
            return;
        };
        let opened = self.windows.toggle(id);
        if opened && id == WindowId::Quests && !self.ui.quests_requested {
            self.ui.quests_requested = true;
            out.push(NetCommand::QuestList);
        }
        if opened && id == WindowId::Refinery {
            // Opening the window pulls the lab item amounts immediately
            // (`createRefinementWindow` → `refreshItemList`); the ~1 Hz
            // refresh while expanded is paced in `Game::update`.
            out.push(NetCommand::LabUpdate);
        }
        if opened && id == WindowId::Contacts && !self.ui.contacts_requested {
            self.ui.contacts_requested = true;
            if !world.contacts.loaded {
                out.push(NetCommand::ContactsRequest { start_index: 0 });
            }
        }
    }

    fn window_body_click(
        &mut self,
        id: WindowId,
        world: &SharedWorld,
        p: (f32, f32),
        out: &mut Vec<NetCommand>,
    ) {
        let Some(body) = self.windows.body(id) else {
            return;
        };
        let scroll = self.windows.get(id).map(|w| w.scroll).unwrap_or(0.0);
        match id {
            WindowId::Minimap => {
                let l = panels::minimap_layout(body);
                if point_in(l.zoom_out, p) {
                    self.ui.minimap_zoom = panels::minimap_zoom_step(self.ui.minimap_zoom, -1);
                } else if point_in(l.zoom_in, p) {
                    self.ui.minimap_zoom = panels::minimap_zoom_step(self.ui.minimap_zoom, 1);
                } else if point_in(l.tab_map, p) {
                    self.ui.minimap_tab = MinimapTab::Map;
                } else if point_in(l.tab_galaxy, p) {
                    self.ui.minimap_tab = MinimapTab::Galaxy;
                } else if point_in(l.map, p) && self.ui.minimap_tab == MinimapTab::Map {
                    let view = self.minimap_view(world, body);
                    if let Some(w) = view.to_world(p, self.bounds) {
                        self.order_move(w, out);
                    }
                }
            }
            WindowId::Ship => {
                let l = panels::ship_layout(body);
                for (i, r) in l.config.iter().enumerate() {
                    if point_in(*r, p) {
                        out.push(NetCommand::ConfigChange {
                            preset_id: i as i16 + 1,
                        });
                    }
                }
            }
            WindowId::Pet => {
                let l = panels::pet_layout(body, world.pets.gears.len(), scroll);
                if point_in(l.activate, p) {
                    out.push(NetCommand::Pet { enabled: true });
                } else if point_in(l.deactivate, p) {
                    out.push(NetCommand::Pet { enabled: false });
                } else if point_in(l.gear_view, p) {
                    for (i, r) in &l.gears {
                        if point_in(*r, p) {
                            if let Some(g) = world.pets.gears.get(*i) {
                                if g.enabled {
                                    // `optParam` 0: the gears we drive from the
                                    // list take no target.
                                    out.push(NetCommand::PetGear {
                                        gear_type: g.gear_type,
                                        target: 0,
                                    });
                                }
                            }
                        }
                    }
                }
            }
            WindowId::Quests => {
                let level = world.world.hero.as_ref().map(|h| h.level).unwrap_or(0);
                let (rows, _) =
                    panels::quest_rows(&world.quests, &world.locale, self.ui.selected_quest, level);
                match panels::quest_hit(body, &rows, scroll, p) {
                    Some(QuestHit::Cancel(id)) => {
                        out.push(NetCommand::QuestCancel { quest_id: id })
                    }
                    Some(QuestHit::Select(id)) => {
                        if self.ui.selected_quest == Some(id) {
                            self.ui.selected_quest = None;
                        } else {
                            self.ui.selected_quest = Some(id);
                            if !world.quests.info.contains_key(&id) {
                                out.push(NetCommand::QuestInfo { quest_id: id });
                            }
                        }
                    }
                    None => {}
                }
            }
            WindowId::Settings => match panels::settings_hit(body, p) {
                Some(SettingsItem::Flag(f)) => self.settings_toggles.push(f),
                Some(SettingsItem::DebugOverlay) => self.show_debug = !self.show_debug,
                Some(SettingsItem::WindowSkin) => {
                    let choice = self.preferences.window_skin.toggle();
                    self.preferences.select_window_skin(choice);
                }
                Some(SettingsItem::MobileHud) => {
                    self.preferences.toggle_mobile_hud();
                    self.set_mobile_hud(self.preferences.mobile_hud);
                }
                Some(SettingsItem::RenderModes) => self.open_render_settings(),
                Some(SettingsItem::MinimapMapImage) => {
                    self.preferences.toggle_minimap_map_image();
                }
                Some(SettingsItem::ToolbarBackground) => {
                    self.preferences.toggle_toolbar_background();
                }
                Some(SettingsItem::AudioToggle(t)) => {
                    let mut a = self.preferences.audio;
                    match t {
                        panels::AudioToggle::Sfx => a.play_sfx = !a.play_sfx,
                        panels::AudioToggle::Music => a.play_music = !a.play_music,
                        panels::AudioToggle::CombatMusic => {
                            a.play_combat_music = !a.play_combat_music
                        }
                        panels::AudioToggle::Voice => a.play_voice = !a.play_voice,
                    }
                    self.preferences.set_audio(a);
                }
                Some(SettingsItem::AudioVolume(ch)) => {
                    let items = panels::settings_items();
                    if let Some(i) = items
                        .iter()
                        .position(|it| *it == SettingsItem::AudioVolume(ch))
                    {
                        let r = panels::settings_row_rect(body, i);
                        let (minus, plus) = panels::settings_vol_buttons(r);
                        let delta = if point_in(plus, p) {
                            10
                        } else if point_in(minus, p) {
                            -10
                        } else {
                            0
                        };
                        if delta != 0 {
                            let mut a = self.preferences.audio;
                            let v = match ch {
                                panels::AudioChannel::Sound => &mut a.sound_volume,
                                panels::AudioChannel::Music => &mut a.music_volume,
                                panels::AudioChannel::Voice => &mut a.voice_volume,
                            };
                            *v = (*v + delta).clamp(0, 100);
                            self.preferences.set_audio(a);
                        }
                    }
                }
                Some(SettingsItem::ResetLayout) => self.windows.reset_layout(),
                None => {}
            },
            WindowId::Group => {
                let count = world.group.as_ref().map(|g| g.members.len()).unwrap_or(0);
                let l = panels::group_layout(body, count, scroll);
                if let Some(group) = &world.group {
                    if let Some((index, _)) = l.rows.iter().find(|(_, r)| point_in(*r, p)) {
                        if let Some(member) = group.members.get(*index) {
                            self.ui.selected_group_member = Some(member.id);
                            out.push(NetCommand::GroupMemberSelect {
                                target_id: member.id,
                            });
                        }
                    }
                }
                if point_in(l.invite_input, p) {
                    self.ui.social_text_focus = Some(crate::ui::SocialTextFocus::GroupInvite);
                } else if point_in(l.invite, p) {
                    if !self.ui.group_invite_name.trim().is_empty() {
                        out.push(NetCommand::InviteToGroup {
                            name: self.ui.group_invite_name.trim().to_owned(),
                        });
                        self.ui.group_invite_name.clear();
                    }
                    self.ui.social_text_focus = None;
                } else if point_in(l.follow, p) {
                    if let Some(id) = self.ui.selected_group_member {
                        out.push(NetCommand::FollowGroupMember { member_id: id });
                    }
                } else if point_in(l.leave, p) && world.group.is_some() {
                    out.push(NetCommand::LeaveGroup);
                } else if !point_in(l.invite_input, p) {
                    self.ui.social_text_focus = None;
                }
            }
            WindowId::Contacts => {
                let l = panels::contacts_layout(body, world.contacts.contacts.len(), scroll);
                if let Some((index, _)) = l.rows.iter().find(|(_, r)| point_in(*r, p)) {
                    if let Some(contact) = world.contacts.contacts.get(*index) {
                        self.ui.selected_contact = Some(contact.user_id);
                    }
                    self.ui.social_text_focus = None;
                } else if point_in(l.name_input, p) {
                    self.ui.social_text_focus = Some(crate::ui::SocialTextFocus::ContactsName);
                } else if point_in(l.add, p) {
                    let selected = self
                        .ui
                        .selected_contact
                        .and_then(|id| world.contacts.get(id));
                    let (user_id, name, request_type) =
                        if let Some(c) = selected.filter(|c| c.entry_type == 2) {
                            (c.user_id, c.user_name.clone(), 1)
                        } else if !self.ui.contacts_name.trim().is_empty() {
                            (0, self.ui.contacts_name.trim().to_owned(), 0)
                        } else {
                            (0, String::new(), -1)
                        };
                    if request_type >= 0 {
                        out.push(NetCommand::ContactRequest {
                            request_type,
                            user_id,
                            user_name: name,
                        });
                        self.ui.contacts_name.clear();
                    }
                    self.ui.social_text_focus = None;
                } else if point_in(l.remove, p) {
                    if let Some(contact) = self
                        .ui
                        .selected_contact
                        .and_then(|id| world.contacts.get(id))
                    {
                        let request_type = if contact.entry_type == 2 { 2 } else { 4 };
                        out.push(NetCommand::ContactRequest {
                            request_type,
                            user_id: contact.user_id,
                            user_name: contact.user_name.clone(),
                        });
                    }
                    self.ui.social_text_focus = None;
                } else {
                    self.ui.social_text_focus = None;
                }
            }
            WindowId::Chat => {}
            WindowId::Refinery => self.refinery_body_click(body, world, p, out),
            WindowId::User | WindowId::Log => {}
        }
    }

    /// Refinery window body: tab switches, "Refine" buttons and the start
    /// of an upgrade-ore drag (`SourceRefinementModule.onMouseDown`).
    fn refinery_body_click(
        &mut self,
        body: Rect,
        world: &SharedWorld,
        p: (f32, f32),
        _out: &mut Vec<NetCommand>,
    ) {
        let Some(data) = self.refinery.clone() else {
            return;
        };
        let w = &data.window;
        let flags: Vec<bool> = w.refine_ores.iter().map(|o| o.button).collect();
        let l = panels::refinery_layout(
            body,
            w.refine_static.len(),
            w.refine_ores.len(),
            w.update_static.len(),
            w.update_ores.len(),
            w.update_items.len(),
        );
        match panels::refinery_hit(&l, self.ui.refinery_tab, &flags, p) {
            Some(panels::RefineryHit::Tab(t)) => {
                // `onRefinementTab`/`onUpdateTab`: pure visibility flip —
                // ore counts are already live (`updateOreCounts` reads
                // the client-side cargo, the lab items come from the
                // ~1 Hz `LabUpdateRequest`).
                self.ui.refinery_tab = t;
            }
            Some(panels::RefineryHit::Refine(i)) => {
                let Some(ore) = w.refine_ores.get(i) else {
                    return;
                };
                let recipe = data
                    .ores
                    .get(&ore.ore_type)
                    .map(|o| o.refiner.as_slice())
                    .unwrap_or(&[]);
                let max = crate::net::refinery::refine_max(recipe, &world.extras.ores);
                let options = crate::net::refinery::combo_options(max);
                let amount = *options.last().unwrap_or(&0);
                self.ui.lab_dialog = Some(crate::ui::LabDialog::Refine {
                    ore_type: ore.ore_type,
                    options,
                    amount,
                });
            }
            Some(panels::RefineryHit::Source(i)) => {
                if let Some(ore) = w.update_ores.get(i) {
                    // `onMouseDown`: an empty source can't be dragged.
                    if world.extras.ores.get(&ore.ore_type).copied().unwrap_or(0.0) > 0.0 {
                        self.ui.refinery_drag = Some(ore.ore_type);
                    }
                }
            }
            None => {}
        }
    }

    /// Released the mouse while dragging a lab source ore — resolve the
    /// drop over the upgrade targets (`SourceRefinementModule.onMouseUp`).
    /// Called from `poll_input` when the button goes up.
    pub(super) fn refinery_drop(
        &mut self,
        p: (f32, f32),
        world: &SharedWorld,
        _out: &mut Vec<NetCommand>,
    ) {
        let Some(ore_type) = self.ui.refinery_drag.take() else {
            return;
        };
        if self.ui.refinery_tab != crate::ui::RefineryTab::Upgrade {
            return;
        }
        let (Some(data), Some(body)) =
            (self.refinery.clone(), self.windows.body(WindowId::Refinery))
        else {
            return;
        };
        let w = &data.window;
        let l = panels::refinery_layout(
            body,
            w.refine_static.len(),
            w.refine_ores.len(),
            w.update_static.len(),
            w.update_ores.len(),
            w.update_items.len(),
        );
        let Some(ti) = panels::refinery_drop_target(&l, p) else {
            return;
        };
        let Some(item) = w.update_items.get(ti) else {
            return;
        };
        let Some(kind) = crate::net::refinery::LabItemKind::of_identifier(&item.identifier) else {
            return;
        };
        let cur = world.lab.items[kind.index()];
        if !crate::net::refinery::valid_upgrade(ore_type, kind, cur.ore_type, cur.count) {
            return;
        }
        let ore_count = world.extras.ores.get(&ore_type).copied().unwrap_or(0.0);
        match cur.ore_type {
            // A different ore is applied → `lab_alert` replace-confirm; the
            // stock count cap here uses the FULL 1e6 headroom, not the
            // remaining one.
            Some(t) if t != ore_type => {
                let max = (crate::net::refinery::LAB_ITEM_MAX * 0.1).min(ore_count) as u64;
                self.ui.lab_dialog = Some(crate::ui::LabDialog::UpgradeConfirm {
                    item: kind.index(),
                    ore_type,
                    max,
                });
            }
            // Empty slot or the same ore → the count dialog straight away
            // (`min(target.maxAmount * 0.1, oreCount)` — 10% of remaining).
            _ => {
                let max = crate::net::refinery::upgrade_max_count(cur, ore_count);
                if max < 1 {
                    return;
                }
                let options = crate::net::refinery::combo_options(max);
                let amount = *options.last().unwrap_or(&max);
                self.ui.lab_dialog = Some(crate::ui::LabDialog::UpgradeAmount {
                    item: kind.index(),
                    ore_type,
                    options,
                    amount,
                });
            }
        }
    }

    /// Click inside the modal lab dialog (amount chips / OK / Cancel).
    /// Returns true while a dialog is open — the press never leaks to the
    /// window beneath (`activateModalLayer`).
    fn lab_dialog_press(&mut self, p: (f32, f32), out: &mut Vec<NetCommand>) -> bool {
        let Some(dlg) = self.ui.lab_dialog.clone() else {
            return false;
        };
        let anchor = self
            .windows
            .get(WindowId::Refinery)
            .map(|w| w.rect)
            .unwrap_or((0.0, 0.0, self.camera.viewport.0, self.camera.viewport.1));
        let opts = match &dlg {
            crate::ui::LabDialog::Refine { options, .. }
            | crate::ui::LabDialog::UpgradeAmount { options, .. } => options.clone(),
            crate::ui::LabDialog::UpgradeConfirm { .. } => Vec::new(),
        };
        let l = panels::lab_dialog_layout(anchor, &opts, dlg.panel_size());
        for (v, r) in &l.options {
            if point_in(*r, p) {
                match &mut self.ui.lab_dialog {
                    Some(crate::ui::LabDialog::Refine { amount, .. })
                    | Some(crate::ui::LabDialog::UpgradeAmount { amount, .. }) => *amount = *v,
                    _ => {}
                }
                return true;
            }
        }
        if point_in(l.ok, p) {
            self.ui.lab_dialog = None;
            match dlg {
                crate::ui::LabDialog::Refine {
                    ore_type, amount, ..
                } if amount > 0 => out.push(NetCommand::LabRefine {
                    ore_type,
                    count: amount,
                }),
                crate::ui::LabDialog::UpgradeAmount {
                    item,
                    ore_type,
                    amount,
                    ..
                } if amount > 0 => out.push(NetCommand::LabUpdateItem {
                    item: item as i16,
                    ore_type,
                    count: amount,
                }),
                crate::ui::LabDialog::UpgradeConfirm {
                    item,
                    ore_type,
                    max,
                } => {
                    // "Update" confirms the replace → the count dialog
                    // opens next (`createUpdateWindow` → `createCountWindow`).
                    let options = crate::net::refinery::combo_options(max);
                    let amount = *options.last().unwrap_or(&max);
                    self.ui.lab_dialog = Some(crate::ui::LabDialog::UpgradeAmount {
                        item,
                        ore_type,
                        options,
                        amount,
                    });
                }
                _ => {}
            }
            return true;
        }
        if point_in(l.cancel, p) {
            self.ui.lab_dialog = None;
        }
        true
    }

    /// Wheel over a window scrolls its content; returns true when the
    /// wheel was over a window (the map must not zoom).
    pub(super) fn ui_wheel(&mut self, world: &SharedWorld, p: (f32, f32), wheel: f32) -> bool {
        let Some(id) = self.windows.topmost_at(p) else {
            return false;
        };
        let Some(body) = self.windows.body(id) else {
            return true;
        };
        let range = match id {
            WindowId::Pet => Some((
                panels::pet_max_scroll(body, world.pets.gears.len()),
                -wheel * panels::PET_GEAR_ROW_H,
            )),
            WindowId::Quests => {
                let level = world.world.hero.as_ref().map(|h| h.level).unwrap_or(0);
                let (_, h) =
                    panels::quest_rows(&world.quests, &world.locale, self.ui.selected_quest, level);
                Some((panels::quest_max_scroll(body, h), -wheel * 28.0))
            }
            // Log scroll is measured from the newest line.
            WindowId::Log => Some((
                panels::log_max_scroll(body, world.log.len()),
                wheel * panels::LOG_LINE_H * 2.0,
            )),
            WindowId::Group => Some((
                panels::group_max_scroll(
                    body,
                    world.group.as_ref().map(|g| g.members.len()).unwrap_or(0),
                ),
                -wheel * panels::SOCIAL_ROW_H,
            )),
            WindowId::Contacts => Some((
                panels::contacts_max_scroll(body, world.contacts.contacts.len()),
                -wheel * panels::SOCIAL_ROW_H,
            )),
            WindowId::Chat => Some((
                (world.chat.len() as f32 * 12.0 - body.3).max(0.0),
                -wheel * 24.0,
            )),
            _ => None,
        };
        if let (Some((max, delta)), Some(w)) = (range, self.windows.get_mut(id)) {
            w.scroll = (w.scroll + delta).clamp(0.0, max);
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::menu::{MenuActionStyle, MenuItem};

    fn game() -> Game {
        let mut g = Game::new(1, (21000.0, 13100.0));
        g.camera.viewport = (1280.0, 720.0);
        g.ensure_ui(1280.0, 720.0);
        g
    }

    fn center(r: Rect) -> (f32, f32) {
        (r.0 + r.2 * 0.5, r.1 + r.3 * 0.5)
    }

    fn press(g: &mut Game, w: &SharedWorld, p: (f32, f32)) -> (bool, Vec<NetCommand>) {
        let hud = g.hud_for_frame();
        let area = input::hud_hit(&hud, p);
        let mut out = Vec::new();
        let used = g.ui_press(w, p, &hud, area, &mut out);
        (used, out)
    }

    fn world_with_ammo() -> SharedWorld {
        let mut w = SharedWorld::default();
        for id in ["ammunition_laser_lcb-10", "ammunition_laser_mcb-25"] {
            w.menu.items.insert(
                id.to_string(),
                MenuItem {
                    id: id.to_string(),
                    visible: true,
                    action_style: MenuActionStyle::Selection,
                    ..Default::default()
                },
            );
        }
        w
    }

    #[test]
    fn toolbar_toggles_windows_and_requests_quests_once() {
        let mut g = game();
        let w = SharedWorld::default();
        let quest_btn = toolbar::toolbar_rects()
            .into_iter()
            .find(|(b, _)| *b == ToolbarButton::Quest)
            .unwrap()
            .1;
        let (used, out) = press(&mut g, &w, center(quest_btn));
        assert!(used);
        assert!(g.windows.is_open(WindowId::Quests));
        assert!(matches!(out.as_slice(), [NetCommand::QuestList]));
        // Toggle off and on again: no second request.
        press(&mut g, &w, center(quest_btn));
        assert!(!g.windows.is_open(WindowId::Quests));
        let (_, out) = press(&mut g, &w, center(quest_btn));
        assert!(g.windows.is_open(WindowId::Quests));
        assert!(out.is_empty());
        // Social windows now open from the toolbar like the other panels.
        let chat = toolbar::toolbar_rects()
            .into_iter()
            .find(|(b, _)| *b == ToolbarButton::Chat)
            .unwrap()
            .1;
        let (used, out) = press(&mut g, &w, center(chat));
        assert!(used && out.is_empty());
        assert!(g.windows.is_open(WindowId::Chat));
    }

    #[test]
    fn window_press_swallows_and_title_press_starts_drag() {
        let mut g = game();
        let w = SharedWorld::default();
        let ship = g.windows.get(WindowId::Ship).unwrap().rect;
        let (used, _) = press(&mut g, &w, (ship.0 + 8.0, ship.1 + 4.0));
        assert!(used);
        assert!(g.windows.is_dragging());
        g.windows.drag((ship.0 + 68.0, ship.1 + 44.0));
        g.windows.release();
        let moved = g.windows.get(WindowId::Ship).unwrap().rect;
        assert_eq!((moved.0, moved.1), (ship.0 + 60.0, ship.1 + 40.0));
        // Empty map space is not consumed.
        let (used, _) = press(&mut g, &w, (640.0, 300.0));
        assert!(!used);
    }

    #[test]
    fn ship_config_buttons_send_config_change() {
        let mut g = game();
        let w = SharedWorld::default();
        let body = g.windows.body(WindowId::Ship).unwrap();
        let l = panels::ship_layout(body);
        let (_, out) = press(&mut g, &w, center(l.config[1]));
        assert!(matches!(
            out.as_slice(),
            [NetCommand::ConfigChange { preset_id: 2 }]
        ));
    }

    #[test]
    fn minimap_click_orders_move_and_zoom_buttons_step() {
        let mut g = game();
        let w = SharedWorld::default();
        let body = g.windows.body(WindowId::Minimap).unwrap();
        let l = panels::minimap_layout(body);
        let (_, out) = press(&mut g, &w, center(l.map));
        match out.as_slice() {
            [NetCommand::Move { x, y }] => {
                assert!(
                    (*x - 10500).abs() < 300 && (*y - 6550).abs() < 300,
                    "{x},{y}"
                );
            }
            other => panic!("expected move, got {other:?}"),
        }
        assert!(g.hero_dest.is_some());
        let (_, out) = press(&mut g, &w, center(l.zoom_in));
        assert!(out.is_empty());
        assert!(g.ui.minimap_zoom > 1.0);
        press(&mut g, &w, center(l.zoom_out));
        assert_eq!(g.ui.minimap_zoom, 1.0);
        // Galaxy tab is a stub: switching must not order a move.
        press(&mut g, &w, center(l.tab_galaxy));
        assert_eq!(g.ui.minimap_tab, MinimapTab::Galaxy);
        let (_, out) = press(&mut g, &w, center(l.map));
        assert!(out.is_empty());
    }

    #[test]
    fn pet_buttons_send_pet_commands() {
        let mut g = game();
        let w = SharedWorld::default();
        g.windows.open(WindowId::Pet);
        let body = g.windows.body(WindowId::Pet).unwrap();
        let l = panels::pet_layout(body, 0, 0.0);
        let (_, out) = press(&mut g, &w, center(l.activate));
        assert!(matches!(
            out.as_slice(),
            [NetCommand::Pet { enabled: true }]
        ));
        let (_, out) = press(&mut g, &w, center(l.deactivate));
        assert!(matches!(
            out.as_slice(),
            [NetCommand::Pet { enabled: false }]
        ));
    }

    #[test]
    fn right_click_laser_slot_opens_flyout_and_pick_sends_menu_action() {
        let mut g = game();
        let w = world_with_ammo();
        let hud = g.hud_for_frame();
        let slot = hud.laser_slot;
        assert!(g.ui_right_click(&w, center(slot), &hud, true));
        let f = g.ui.flyout.clone().expect("flyout open");
        assert_eq!(f.items.len(), 2);
        // Right click again closes it.
        assert!(g.ui_right_click(&w, center(slot), &hud, true));
        assert!(g.ui.flyout.is_none());
        g.ui_right_click(&w, center(slot), &hud, true);
        // Pick the second row.
        let hud = g.hud_for_frame();
        let row = hud.flyout.as_ref().unwrap().rows[1].1;
        let (used, out) = press(&mut g, &w, center(row));
        assert!(used);
        assert!(g.ui.flyout.is_none(), "flyout closes after a pick");
        match out.as_slice() {
            [NetCommand::MenuAction {
                item_id,
                action_type,
                source_type,
            }] => {
                assert_eq!(item_id, "ammunition_laser_mcb-25");
                assert_eq!(*action_type, 1);
                assert_eq!(*source_type, MENU_SOURCE_SLOT_BAR);
            }
            other => panic!("expected MenuAction, got {other:?}"),
        }
    }

    #[test]
    fn click_outside_closes_flyout() {
        let mut g = game();
        let w = world_with_ammo();
        let hud = g.hud_for_frame();
        g.ui_right_click(&w, center(hud.laser_slot), &hud, true);
        assert!(g.ui.flyout.is_some());
        press(&mut g, &w, (640.0, 200.0));
        assert!(g.ui.flyout.is_none());
    }

    #[test]
    fn settings_rows_toggle_flags_and_reset_layout() {
        let mut g = game();
        let mut w = SharedWorld::default();
        g.windows.open(WindowId::Settings);
        let body = g.windows.body(WindowId::Settings).unwrap();
        let before = w.settings.get(crate::net::SettingFlag::ALL[0]);
        let r = panels::settings_row_rect(body, 0);
        press(&mut g, &w, center(r));
        // The flip is queued for `update` (input only has `&SharedWorld`).
        assert_eq!(g.settings_toggles.len(), 1);
        for f in std::mem::take(&mut g.settings_toggles) {
            w.settings.toggle(f);
        }
        assert_ne!(w.settings.get(crate::net::SettingFlag::ALL[0]), before);
        // Reset-layout row.
        g.windows.close(WindowId::Ship);
        let last = panels::settings_items().len() - 1;
        let r = panels::settings_row_rect(g.windows.body(WindowId::Settings).unwrap(), last);
        press(&mut g, &w, center(r));
        assert!(g.windows.is_open(WindowId::Ship));
    }

    #[test]
    fn render_modes_settings_row_opens_component_panel_through_real_ui_press() {
        let mut g = game();
        let w = SharedWorld::default();
        g.windows.open(WindowId::Settings);
        let body = g.windows.body(WindowId::Settings).expect("settings body");
        let index = panels::settings_items()
            .iter()
            .position(|item| *item == SettingsItem::RenderModes)
            .expect("render modes row");
        let point = center(panels::settings_row_rect(body, index));
        assert_eq!(
            g.windows.topmost_at(point),
            Some(WindowId::Settings),
            "settings window must own its rendered row hit area"
        );
        let (consumed, commands) = press(&mut g, &w, point);
        assert!(consumed, "settings row press must be consumed");
        assert!(commands.is_empty());
        assert!(
            g.show_render_settings,
            "the Render Modes row must open the settings panel"
        );
        assert!(g.render_settings_draft.is_some());
    }

    #[test]
    fn wheel_scrolls_windows_and_not_the_map() {
        let mut g = game();
        let w = SharedWorld::default();
        // Over the minimap: swallowed, no scroll.
        let mm = g.windows.get(WindowId::Minimap).unwrap().rect;
        assert!(g.ui_wheel(&w, (mm.0 + 20.0, mm.1 + 40.0), 1.0));
        // Over empty map: not consumed.
        assert!(!g.ui_wheel(&w, (640.0, 300.0), 1.0));
    }

    /// Minimal `<refinementWindow>` + ore patterns matching the fixture.
    fn refinery_game() -> (Game, SharedWorld) {
        use crate::assets::gamexml::{OrePattern, RefinementWindow, RefineryItem, RefineryOre};
        let ore = |ore_type: i32, icon: &str, button: bool| RefineryOre {
            ore_type,
            tooltip_key: String::new(),
            icon_res_key: icon.to_string(),
            button,
        };
        let mut g = game();
        g.refinery = Some(crate::game::RefineryData {
            window: RefinementWindow {
                refine_static: vec![ore(0, "", false), ore(1, "", false), ore(2, "", false)],
                refine_ores: vec![
                    ore(4, "", true),
                    ore(5, "", true),
                    ore(6, "", true),
                    ore(3, "", false),
                ],
                update_static: vec![],
                update_ores: vec![ore(4, "icon_11", false), ore(6, "icon_13", false)],
                update_items: vec![RefineryItem {
                    type_id: 1,
                    identifier: "LASER".to_string(),
                    res_key: "item_1".to_string(),
                    language_key: "lab_laser".to_string(),
                    amount_key: "lab_salvos".to_string(),
                }],
            },
            ores: std::collections::HashMap::from([(
                4,
                OrePattern {
                    res_key: "ore_11".to_string(),
                    language_key: "ore_prometid".to_string(),
                    refiner: vec![(0, 20), (1, 10)],
                },
            )]),
        });
        let mut w = SharedWorld::default();
        w.extras.ores.insert(0, 1000.0); // 1000/20 = 50
        w.extras.ores.insert(1, 500.0); // 500/10 = 50
        w.extras.ores.insert(4, 200.0); // draggable source
        (g, w)
    }

    #[test]
    fn refinery_tabs_refine_dialog_and_upgrade_drag() {
        let (mut g, mut w) = refinery_game();
        g.windows.open(WindowId::Refinery);
        let body = g.windows.body(WindowId::Refinery).unwrap();
        let l = panels::refinery_layout(body, 3, 4, 0, 2, 1);

        // Tab switch is a pure visibility flip — no packet.
        let (_, out) = press(&mut g, &w, center(l.tab_upgrade));
        assert_eq!(g.ui.refinery_tab, crate::ui::RefineryTab::Upgrade);
        assert!(out.is_empty());
        press(&mut g, &w, center(l.tab_refining));
        assert_eq!(g.ui.refinery_tab, crate::ui::RefineryTab::Refining);

        // Refine button 0 → amount dialog; max = min(1000/20, 500/10) = 50.
        let (_, out) = press(&mut g, &w, center(l.refine_buttons[0]));
        assert!(out.is_empty());
        match &g.ui.lab_dialog {
            Some(crate::ui::LabDialog::Refine {
                ore_type,
                options,
                amount,
            }) => {
                assert_eq!(*ore_type, 4);
                assert_eq!(options.as_slice(), &[1, 5, 10, 50]);
                assert_eq!(*amount, 50);
            }
            other => panic!("expected refine dialog, got {other:?}"),
        }
        // The dialog is modal — a press on the map is swallowed.
        let (used, out) = press(&mut g, &w, (640.0, 300.0));
        assert!(used && out.is_empty() && g.ui.lab_dialog.is_some());
        // Pick a different preset, then OK sends LabRefine.
        let anchor = g.windows.get(WindowId::Refinery).unwrap().rect;
        let opts = [1u64, 5, 10, 50];
        let dl = panels::lab_dialog_layout(anchor, &opts, (240.0, 150.0));
        press(&mut g, &w, center(dl.options[0].1));
        match &g.ui.lab_dialog {
            Some(crate::ui::LabDialog::Refine { amount, .. }) => assert_eq!(*amount, 1),
            other => panic!("dialog lost? {other:?}"),
        }
        let dl = panels::lab_dialog_layout(anchor, &opts, (240.0, 150.0));
        let (used, out) = press(&mut g, &w, center(dl.ok));
        assert!(used && g.ui.lab_dialog.is_none());
        assert!(matches!(
            out.as_slice(),
            [NetCommand::LabRefine {
                ore_type: 4,
                count: 1
            }]
        ));

        // Upgrade tab: press a source → drag state; drop over LASER →
        // count dialog; OK → LabUpdateItem.
        press(&mut g, &w, center(l.tab_upgrade));
        let (_, out) = press(&mut g, &w, center(l.upd_sources[0]));
        assert!(out.is_empty());
        assert_eq!(g.ui.refinery_drag, Some(4));
        let mut out = Vec::new();
        g.refinery_drop(center(l.upd_targets[0]), &w, &mut out);
        assert!(out.is_empty());
        // 10% of full headroom = 100k, capped by the 200 held → 200.
        match &g.ui.lab_dialog {
            Some(crate::ui::LabDialog::UpgradeAmount {
                item,
                ore_type,
                amount,
                ..
            }) => {
                assert_eq!((*item, *ore_type, *amount), (0, 4, 200));
            }
            other => panic!("expected count dialog, got {other:?}"),
        }
        let dl = panels::lab_dialog_layout(anchor, &[1, 5, 10, 50, 100, 200], (180.0, 110.0));
        let (_, mut out) = press(&mut g, &w, center(dl.ok));
        assert!(matches!(
            out.as_slice(),
            [NetCommand::LabUpdateItem {
                item: 0,
                ore_type: 4,
                count: 200,
            }]
        ));

        // A different applied ore → replace-confirm first.
        w.lab.items[0].ore_type = Some(6);
        w.lab.items[0].count = 10.0;
        press(&mut g, &w, center(l.upd_sources[0]));
        assert_eq!(g.ui.refinery_drag, Some(4));
        g.refinery_drop(center(l.upd_targets[0]), &w, &mut out);
        assert!(matches!(
            g.ui.lab_dialog,
            Some(crate::ui::LabDialog::UpgradeConfirm { .. })
        ));
        // Cancel leaves the lab untouched.
        let dl = panels::lab_dialog_layout(anchor, &[], (350.0, 110.0));
        press(&mut g, &w, center(dl.cancel));
        assert!(g.ui.lab_dialog.is_none());
    }

    #[test]
    fn refinery_toolbar_opens_and_pulls_lab_state() {
        let (mut g, w) = refinery_game();
        let btn = toolbar::toolbar_rects()
            .into_iter()
            .find(|(b, _)| *b == ToolbarButton::Refinery)
            .unwrap()
            .1;
        let (used, out) = press(&mut g, &w, center(btn));
        assert!(used);
        assert!(g.windows.is_open(WindowId::Refinery));
        assert!(matches!(out.as_slice(), [NetCommand::LabUpdate]));
    }
}
