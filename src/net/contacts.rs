//! Contact list state carried by the game's normal packet connection.

use xenomit_packet_client::packet::{Packet, Value};

use crate::net::world::SharedWorld;

pub const CONTACTS_INIT: i16 = 17529;
pub const CONTACTS_UPDATE: i16 = -22989;
pub const CONTACTS_REMOVE: i16 = -13415;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Contact {
    pub user_id: i32,
    pub user_name: String,
    pub clan_tag: String,
    pub level: i32,
    pub map_id: i32,
    pub online: bool,
    /// 0 friend, 1 clan, 2 incoming request, 3 outgoing pending,
    /// 4 declined, 5 blocked (matches ContactListEntryTypeModule).
    pub entry_type: i32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ContactList {
    pub contacts: Vec<Contact>,
    pub loaded: bool,
}

impl ContactList {
    pub fn get(&self, user_id: i32) -> Option<&Contact> {
        self.contacts.iter().find(|c| c.user_id == user_id)
    }
}

pub fn handle(world: &mut SharedWorld, p: &Packet) {
    match p.id {
        CONTACTS_INIT => {
            world.contacts.contacts.clear();
            if let Ok(command) = p.module("contactsCommand") {
                read_updates(&mut world.contacts, command);
            }
            world.contacts.loaded = true;
        }
        CONTACTS_UPDATE => read_updates(&mut world.contacts, p),
        CONTACTS_REMOVE => {
            for value in p.list("contactUserIds").unwrap_or(&[]) {
                let Value::Int(id) = value else {
                    continue;
                };
                world.contacts.contacts.retain(|c| c.user_id != *id as i32);
            }
        }
        _ => {}
    }
}

fn read_updates(list: &mut ContactList, p: &Packet) {
    for value in p.list("contacts").unwrap_or(&[]) {
        let Value::Module(row) = value else {
            continue;
        };
        let Ok(attrs) = row.module("attributeCommand") else {
            continue;
        };
        let Ok(user_id) = attrs.int("userId") else {
            continue;
        };
        let mut c = Contact {
            user_id: user_id as i32,
            ..Default::default()
        };
        for value in attrs.list("attributes").unwrap_or(&[]) {
            let Value::Module(a) = value else {
                continue;
            };
            match a.id {
                -28574 => c.user_name = a.string("userName").unwrap_or("").to_owned(),
                -23538 => c.clan_tag = a.string("clanTag").unwrap_or("").to_owned(),
                6868 => c.level = a.int("level").unwrap_or(0) as i32,
                5101 => c.map_id = a.int("currentMap").unwrap_or(0) as i32,
                21885 => c.online = a.boolean("online").unwrap_or(false),
                _ => {}
            }
        }
        if let Ok(kind) = row.module("typeCommand") {
            c.entry_type = kind.int("entryType").unwrap_or(0) as i32;
        }
        if let Some(existing) = list
            .contacts
            .iter_mut()
            .find(|old| old.user_id == c.user_id)
        {
            *existing = c;
        } else {
            list.contacts.push(c);
        }
    }
    list.contacts.sort_by(|a, b| {
        a.entry_type
            .cmp(&b.entry_type)
            .then_with(|| b.online.cmp(&a.online))
            .then_with(|| a.user_name.to_lowercase().cmp(&b.user_name.to_lowercase()))
    });
}
