//! Packet-backed group/contact panels and the map-session chat log.

use super::{TXT, TXT_DIM};
use crate::net::SharedWorld;
use crate::render::fonts::Fonts;
use crate::ui::window::{self, Rect};
use crate::ui::{chrome, panels};
use raylib::prelude::*;

pub(crate) fn draw_group(
    d: &mut RaylibDrawHandle,
    world: &SharedWorld,
    game: &crate::game::Game,
    fonts: &Fonts,
    body: Rect,
    mouse: (f32, f32),
) {
    let members = world.group.as_ref().map(|g| g.members.len()).unwrap_or(0);
    let scroll = game
        .windows
        .get(crate::ui::WindowId::Group)
        .map(|w| w.scroll)
        .unwrap_or(0.0);
    let l = panels::group_layout(body, members, scroll);
    match &world.group {
        Some(group) => chrome::label(
            d,
            fonts,
            &format!("Group {}  ·  {}/{}", group.id, members, group.max_size),
            body.0 + 8.0,
            body.1 + 4.0,
            9.0,
            TXT_DIM,
        ),
        None => chrome::label(
            d,
            fonts,
            "No active group",
            body.0 + 8.0,
            body.1 + 4.0,
            9.0,
            TXT_DIM,
        ),
    }
    if let Some(group) = &world.group {
        for (index, r) in &l.rows {
            let Some(m) = group.members.get(*index) else {
                continue;
            };
            let selected = game.ui.selected_group_member == Some(m.id);
            let hover = window::point_in(*r, mouse);
            d.draw_rectangle(
                r.0 as i32,
                r.1 as i32,
                r.2 as i32,
                r.3 as i32,
                if selected {
                    Color::new(24, 76, 112, 225)
                } else if hover {
                    Color::new(25, 48, 76, 200)
                } else {
                    Color::new(8, 13, 21, 160)
                },
            );
            let status = if m.logged_out {
                "offline"
            } else if m.in_fight {
                "combat"
            } else if m.map_id == game.map_id {
                "here"
            } else {
                "away"
            };
            let clan = if m.clan_tag.is_empty() {
                String::new()
            } else {
                format!("[{}] ", m.clan_tag)
            };
            let name = if m.name.is_empty() {
                format!("Player {}", m.id)
            } else {
                m.name.clone()
            };
            let label = chrome::fit_text(
                fonts,
                &format!("{}{}  L{}", clan, name, m.level),
                9.0,
                r.2 - 48.0,
            );
            chrome::label(
                d,
                fonts,
                &label,
                r.0 + 5.0,
                r.1 + 4.0,
                9.0,
                if selected { Color::WHITE } else { TXT },
            );
            chrome::label_right(
                d,
                fonts,
                status,
                r.0 + r.2 - 5.0,
                r.1 + 4.0,
                8.0,
                if m.logged_out {
                    TXT_DIM
                } else if m.in_fight {
                    Color::new(255, 145, 105, 255)
                } else {
                    Color::new(115, 205, 245, 255)
                },
            );
            let bx = r.0 + 5.0;
            let by = r.1 + r.3 - 5.0;
            let bw = (r.2 - 10.0) * m.hp_frac();
            d.draw_rectangle(
                bx as i32,
                by as i32,
                (r.2 - 10.0) as i32,
                2,
                Color::new(18, 24, 32, 255),
            );
            d.draw_rectangle(
                bx as i32,
                by as i32,
                bw as i32,
                2,
                Color::new(90, 205, 112, 235),
            );
        }
    } else {
        chrome::label(
            d,
            fonts,
            "Invite a player by nickname to start a group.",
            body.0 + 8.0,
            body.1 + 29.0,
            8.0,
            TXT_DIM,
        );
    }
    chrome::draw_inset(d, l.invite_input);
    let input_label = if game.ui.group_invite_name.is_empty() {
        "Player nickname"
    } else {
        &game.ui.group_invite_name
    };
    chrome::label(
        d,
        fonts,
        input_label,
        l.invite_input.0 + 5.0,
        l.invite_input.1 + 5.0,
        9.0,
        if game.ui.social_text_focus == Some(crate::ui::SocialTextFocus::GroupInvite) {
            TXT
        } else {
            TXT_DIM
        },
    );
    chrome::draw_button(
        d,
        fonts,
        l.invite,
        "Invite",
        chrome::ButtonState {
            hover: window::point_in(l.invite, mouse),
            active: false,
            disabled: game.ui.group_invite_name.trim().is_empty(),
        },
    );
    chrome::draw_button(
        d,
        fonts,
        l.follow,
        "Follow selected",
        chrome::ButtonState {
            hover: window::point_in(l.follow, mouse),
            active: false,
            disabled: game.ui.selected_group_member.is_none(),
        },
    );
    chrome::draw_button(
        d,
        fonts,
        l.leave,
        "Leave group",
        chrome::ButtonState {
            hover: window::point_in(l.leave, mouse),
            active: false,
            disabled: world.group.is_none(),
        },
    );
}

pub(crate) fn draw_contacts(
    d: &mut RaylibDrawHandle,
    world: &SharedWorld,
    game: &crate::game::Game,
    fonts: &Fonts,
    body: Rect,
    mouse: (f32, f32),
) {
    let scroll = game
        .windows
        .get(crate::ui::WindowId::Contacts)
        .map(|w| w.scroll)
        .unwrap_or(0.0);
    let l = panels::contacts_layout(body, world.contacts.contacts.len(), scroll);
    for (index, r) in &l.rows {
        let Some(c) = world.contacts.contacts.get(*index) else {
            continue;
        };
        let selected = game.ui.selected_contact == Some(c.user_id);
        let hover = window::point_in(*r, mouse);
        d.draw_rectangle(
            r.0 as i32,
            r.1 as i32,
            r.2 as i32,
            r.3 as i32,
            if selected {
                Color::new(24, 76, 112, 225)
            } else if hover {
                Color::new(25, 48, 76, 200)
            } else {
                Color::new(8, 13, 21, 160)
            },
        );
        let dot = if c.online {
            Color::new(95, 215, 125, 255)
        } else {
            Color::new(95, 102, 114, 255)
        };
        d.draw_circle((r.0 + 8.0) as i32, (r.1 + r.3 * 0.5) as i32, 3.0, dot);
        let name = if c.user_name.is_empty() {
            format!("Player {}", c.user_id)
        } else {
            c.user_name.clone()
        };
        let clan = if c.clan_tag.is_empty() {
            String::new()
        } else {
            format!("[{}] ", c.clan_tag)
        };
        let text = chrome::fit_text(fonts, &format!("{clan}{name}"), 9.0, r.2 - 42.0);
        chrome::label(
            d,
            fonts,
            &text,
            r.0 + 17.0,
            r.1 + 4.0,
            9.0,
            if selected { Color::WHITE } else { TXT },
        );
        let kind = match c.entry_type {
            2 => "request",
            3 => "pending",
            4 => "declined",
            5 => "blocked",
            _ => {
                if c.online {
                    "online"
                } else {
                    "offline"
                }
            }
        };
        chrome::label_right(d, fonts, kind, r.0 + r.2 - 4.0, r.1 + 4.0, 7.0, TXT_DIM);
    }
    if world.contacts.contacts.is_empty() {
        let message = if world.contacts.loaded {
            "No contacts yet"
        } else {
            "Loading contacts..."
        };
        chrome::label(d, fonts, message, body.0 + 8.0, body.1 + 10.0, 9.0, TXT_DIM);
    }
    chrome::draw_inset(d, l.name_input);
    let entry = if game.ui.contacts_name.is_empty() {
        "Nickname"
    } else {
        &game.ui.contacts_name
    };
    chrome::label(
        d,
        fonts,
        entry,
        l.name_input.0 + 5.0,
        l.name_input.1 + 5.0,
        9.0,
        if game.ui.social_text_focus == Some(crate::ui::SocialTextFocus::ContactsName) {
            TXT
        } else {
            TXT_DIM
        },
    );
    let incoming = game
        .ui
        .selected_contact
        .and_then(|id| world.contacts.get(id))
        .is_some_and(|c| c.entry_type == 2);
    chrome::draw_button(
        d,
        fonts,
        l.add,
        if incoming { "Accept" } else { "Add" },
        chrome::ButtonState {
            hover: window::point_in(l.add, mouse),
            active: false,
            disabled: game.ui.contacts_name.trim().is_empty() && !incoming,
        },
    );
    chrome::draw_button(
        d,
        fonts,
        l.remove,
        if incoming { "Decline" } else { "Remove" },
        chrome::ButtonState {
            hover: window::point_in(l.remove, mouse),
            active: false,
            disabled: game.ui.selected_contact.is_none(),
        },
    );
}

pub(crate) fn draw_chat(
    d: &mut RaylibDrawHandle,
    world: &SharedWorld,
    fonts: &Fonts,
    body: Rect,
    scroll: f32,
) {
    let view = (body.0 + 6.0, body.1 + 5.0, body.2 - 12.0, body.3 - 28.0);
    let line_h = 13.0;
    let visible = (view.3 / line_h).floor().max(1.0) as usize;
    let skip_from_end = (scroll / line_h).floor() as usize;
    let end = world.chat.len().saturating_sub(skip_from_end);
    let start = end.saturating_sub(visible);
    for (i, line) in world.chat[start..end].iter().enumerate() {
        let text = chrome::fit_text(fonts, &line.display(), 9.0, view.2 - 4.0);
        let y = view.1 + i as f32 * line_h;
        let color = match line.kind {
            crate::net::ChatKind::Player => TXT,
            crate::net::ChatKind::System => TXT_DIM,
        };
        chrome::label(d, fonts, &text, view.0 + 3.0, y, 9.0, color);
    }
    if world.chat.is_empty() {
        chrome::label(
            d,
            fonts,
            "No map-session messages yet.",
            view.0 + 4.0,
            view.1 + 5.0,
            9.0,
            TXT_DIM,
        );
    }
    chrome::label(
        d,
        fonts,
        "Map messages only · chat rooms are not connected",
        body.0 + 7.0,
        body.1 + body.3 - 17.0,
        8.0,
        TXT_DIM,
    );
}
