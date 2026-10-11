//! P.E.T. companions: `PetAssembly` / `net/netty/handlers/pet/*`.
//!
//! A pet is a ship-like map entity (`ShipManager.createPet`): its id is a
//! normal entity id, so `MoveCommand`s tween it through the regular
//! `SharedWorld::tweens` and the renderer draws the `Entity` row we keep
//! in `world.world.entities`. [`PetState`] holds what an `Entity` can't:
//! level, vitals (hp / shield / fuel / xp), loot id, idle flag.
//!
//! The hero's own pet receives the id-less vitals updates
//! (`PetHitpointsUpdate`, `PetShieldUpdate`, `PetFuelUpdate`, …); other
//! players' pets only ever show up through `PetActivationCommand`.

use std::collections::HashMap;
use xenomit_packet_client::model::{Entity, EntityKind};
use xenomit_packet_client::modules::{self, id};
use xenomit_packet_client::packet::Packet;

use crate::net::world::SharedWorld;

pub(crate) const PET_HERO_ACTIVATION: i16 = 155;
pub(crate) const PET_DEACTIVATION: i16 = 156;
pub(crate) const PET_STATUS: i16 = 157;
pub(crate) const PET_HITPOINTS_UPDATE: i16 = 158;
pub(crate) const PET_EXPERIENCE_UPDATE: i16 = 159;
pub(crate) const PET_LEVEL_UPDATE: i16 = 160;
pub(crate) const PET_SHIELD_UPDATE: i16 = 161;
pub(crate) const PET_FUEL_UPDATE: i16 = 162;
pub(crate) const PET_IS_DESTROYED: i16 = 227;
pub(crate) const PET_ACTIVATION: i16 = 232;
pub(crate) const PET_HARVEST_EFFECT: i16 = 241;
pub(crate) const PET_IDLE_MODE: i16 = 242;
pub(crate) const PET_INITIALIZATION: i16 = 229;
pub(crate) const PET_GEAR_ADD: i16 = 234;
pub(crate) const PET_GEAR_REMOVE: i16 = 233;
pub(crate) const PET_GEAR_RESET: i16 = 235;
pub(crate) const PET_GEAR_SELECT: i16 = 236;
pub(crate) const PET_GEAR_ACTIVATION_CHANGE: i16 = 4943;

/// One P.E.T. gear the hero owns (`PetGearAddCommand`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PetGear {
    /// `PetGearTypeModule.typeValue` (`PETGEARTYPE_*`).
    pub gear_type: i32,
    pub level: i32,
    pub amount: i32,
    /// Selectable right now (`enabled`).
    pub enabled: bool,
    /// Currently running (`PetGearActivationChangeCommand.active`).
    pub active: bool,
}

/// Display name of a `PETGEARTYPE_*` value.
pub fn gear_name(gear_type: i32) -> &'static str {
    match gear_type {
        0 => "Behavior",
        1 => "Passive mode",
        2 => "Guard mode",
        3 => "Gear",
        4 => "Auto-loot",
        5 => "Resource collection",
        6 => "Enemy locator",
        7 => "Resource locator",
        8 => "Trade pod",
        9 => "Repair pet",
        10 => "Kamikaze",
        11 => "Combo ship repair",
        12 => "Combo guard",
        13 => "Admin",
        14 => "Friendly sacrifice",
        _ => "Gear",
    }
}

/// Everything known about one pet.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PetState {
    pub id: i32,
    pub owner_id: i32,
    /// Ship pattern id the pet is skinned as (`createPet(designId)` — the
    /// entity's `type_id`); changes with `PetLevelUpdateCommand`.
    pub design_id: i32,
    pub name: String,
    pub clan_tag: String,
    pub faction_id: i32,
    pub clan_id: i32,
    pub level: i32,
    /// `petLootId` (`PetStatusCommand`) — the pet's item/design key.
    pub loot_id: String,
    pub hitpoints: i32,
    pub hitpoints_max: i32,
    pub shield: i32,
    pub shield_max: i32,
    pub fuel: i32,
    pub fuel_max: i32,
    pub experience: f64,
    /// `petExperiencePointsUntilNextLevel`.
    pub experience_to_next: f64,
    pub speed: i32,
    pub expansion_stage: i32,
    /// `isInIdleMode` — the pet is parked (not drawn).
    pub idle: bool,
    /// `PetIsDestroyedCommand` — the wreck is showing.
    pub destroyed: bool,
    /// `!isVisible` — cloaked.
    pub cloaked: bool,
    /// Owned by the local player (`PetHeroActivationCommand`).
    pub is_hero_pet: bool,
}

impl PetState {
    /// Plate text: `P.E.T {name} [{level}]`.
    pub fn plate_name(&self) -> String {
        format!("P.E.T {} [{}]", self.name, self.level)
    }
}

/// Pets on the current map plus the hero's pet pointer
/// (`Hero.pet`).
#[derive(Clone, Debug, Default)]
pub struct PetTable {
    /// The hero's pet id (`Some` between hero-activation and
    /// deactivation).
    pub hero: Option<i32>,
    /// Every known pet, hero's included, by entity id.
    pub by_id: HashMap<i32, PetState>,
    /// The hero's gears (`PetGearAddCommand`). Not cleared on map change —
    /// adds are upserts, so a re-send is harmless.
    pub gears: Vec<PetGear>,
    /// `PetGearSelectCommand` — the gear the server selected last.
    pub selected_gear: Option<i32>,
    /// `PetInitializationCommand`: `(hasPet, hasFuel, petIsAlive)`.
    pub init: Option<(bool, bool, bool)>,
}

impl PetTable {
    pub fn get(&self, id: i32) -> Option<&PetState> {
        self.by_id.get(&id)
    }

    /// The hero's pet.
    pub fn hero_pet(&self) -> Option<&PetState> {
        self.hero.and_then(|id| self.by_id.get(&id))
    }

    /// Idle pets are parked and not drawn.
    pub fn is_hidden(&self, id: i32) -> bool {
        self.by_id.get(&id).is_some_and(|p| p.idle)
    }

    /// Drop every pet (map change — the server re-sends them).
    pub fn clear(&mut self) {
        self.hero = None;
        self.by_id.clear();
    }
}

/// Mirror a pet into its entity row so ship rendering (`draw_entities`,
/// minimap, hit-test, tweens) treats it like any other ship.
fn sync_entity(world: &mut SharedWorld, pet: &PetState, pos: Option<(i32, i32)>) {
    let e: &mut Entity = world.world.entities.entry(pet.id).or_default();
    e.id = pet.id;
    e.kind = EntityKind::Ship;
    e.name = pet.name.clone();
    e.type_id = pet.design_id.to_string();
    e.clan_tag = pet.clan_tag.clone();
    e.faction_id = pet.faction_id;
    e.clan_id = pet.clan_id;
    e.npc = false;
    e.cloaked = pet.cloaked;
    e.destroyed = pet.destroyed;
    e.expansion_stage = pet.expansion_stage;
    if let Some((x, y)) = pos {
        e.x = x;
        e.y = y;
    }
    if pet.hitpoints_max > 0 {
        e.hitpoints = Some(pet.hitpoints as f64);
        e.hitpoints_max = Some(pet.hitpoints_max as f64);
    }
    if pet.shield_max > 0 {
        e.shield = Some(pet.shield);
        e.shield_max = Some(pet.shield_max);
    }
}

/// Insert/refresh a pet from an activation packet. Existing vitals
/// (a `PetStatusCommand` can precede the activation) are preserved.
fn spawn(world: &mut SharedWorld, mut pet: PetState, pos: (i32, i32)) {
    if let Some(prev) = world.pets.by_id.get(&pet.id) {
        pet.loot_id = prev.loot_id.clone();
        pet.hitpoints = prev.hitpoints;
        pet.hitpoints_max = prev.hitpoints_max;
        pet.shield = prev.shield;
        pet.shield_max = prev.shield_max;
        pet.fuel = prev.fuel;
        pet.fuel_max = prev.fuel_max;
        pet.experience = prev.experience;
        pet.experience_to_next = prev.experience_to_next;
    }
    if pet.is_hero_pet {
        world.pets.hero = Some(pet.id);
    }
    sync_entity(world, &pet, Some(pos));
    let id = pet.id;
    world.pets.by_id.insert(pet.id, pet);
    // `PetActivationHandler`/`PetHeroActivationHandler` —
    // `addMapObjectEffectOnce(new PetSpawnEffect(pet))`.
    world.fx.push(crate::net::WorldFx::PetSpawn { entity: id });
}

/// Run `f` on the hero's pet (id-less vitals updates) and re-sync its
/// entity row.
fn with_hero_pet(world: &mut SharedWorld, f: impl FnOnce(&mut PetState)) {
    let Some(pid) = world.pets.hero else {
        return;
    };
    let Some(mut pet) = world.pets.by_id.remove(&pid) else {
        return;
    };
    f(&mut pet);
    sync_entity(world, &pet, None);
    world.pets.by_id.insert(pid, pet);
}

fn remove_pet(world: &mut SharedWorld, pid: i32) {
    world.pets.by_id.remove(&pid);
    if world.pets.hero == Some(pid) {
        world.pets.hero = None;
    }
    world.world.entities.remove(&pid);
    world.tweens.remove(&pid);
    world.ship_info.remove(&pid);
    world.drones.remove(&pid);
}

/// `gearType` (`PetGearTypeModule.typeValue`) of a gear packet.
fn gear_type_of(p: &Packet) -> Option<i32> {
    p.module("gearType")
        .and_then(|m| m.int("typeValue"))
        .ok()
        .map(|v| v as i32)
}

pub fn handle(world: &mut SharedWorld, p: &Packet) {
    let int = |name: &str| p.int(name).unwrap_or(0) as i32;
    match p.id {
        PET_HERO_ACTIVATION => {
            let pet = PetState {
                id: int("petId"),
                owner_id: int("ownerId"),
                design_id: int("shipType"),
                name: p.string("petName").unwrap_or("").to_owned(),
                clan_tag: p.string("clanTag").unwrap_or("").to_owned(),
                faction_id: int("factionId"),
                clan_id: int("clanId"),
                level: int("petLevel"),
                speed: int("petSpeed"),
                expansion_stage: int("expansionStage"),
                is_hero_pet: true,
                ..Default::default()
            };
            spawn(world, pet, (int("x"), int("y")));
        }
        PET_ACTIVATION => {
            let owner_id = int("ownerId");
            // Stock skips a pet that already exists.
            if world.pets.by_id.contains_key(&int("petId")) {
                return;
            }
            let is_hero_pet = world
                .world
                .hero
                .as_ref()
                .is_some_and(|h| h.user_id == owner_id);
            let pet = PetState {
                id: int("petId"),
                owner_id,
                design_id: int("petDesignId"),
                name: p.string("petName").unwrap_or("").to_owned(),
                clan_tag: p.string("clanTag").unwrap_or("").to_owned(),
                faction_id: int("petFactionId"),
                clan_id: int("petClanID"),
                level: int("petLevel"),
                speed: int("petSpeed"),
                expansion_stage: int("expansionStage"),
                idle: p.boolean("isInIdleMode").unwrap_or(false),
                cloaked: !p.boolean("isVisible").unwrap_or(true),
                is_hero_pet,
                ..Default::default()
            };
            spawn(world, pet, (int("x"), int("y")));
        }
        PET_DEACTIVATION => {
            // `assemblePetDeactivation` ignores the id: it is the hero's
            // pet that goes away.
            let pid = p
                .int("petId")
                .ok()
                .map(|v| v as i32)
                .filter(|v| world.pets.by_id.contains_key(v))
                .or(world.pets.hero);
            if let Some(pid) = pid {
                remove_pet(world, pid);
            }
        }
        PET_STATUS => {
            let pid = int("petId");
            let loot = p.string("petLootId").unwrap_or("").to_owned();
            let name = p.string("petName").unwrap_or("").to_owned();
            let is_hero_pet = world.pets.hero == Some(pid);
            let pet = world.pets.by_id.entry(pid).or_insert_with(|| PetState {
                id: pid,
                is_hero_pet,
                ..Default::default()
            });
            pet.level = int("petLevel");
            pet.experience = p.f64("petExperiencePoints").unwrap_or(0.0);
            pet.experience_to_next = p.f64("petExperiencePointsUntilNextLevel").unwrap_or(0.0);
            pet.hitpoints = int("petHitPoints");
            pet.hitpoints_max = int("petHitPointsMax");
            pet.shield = int("petShieldEnergyNow");
            pet.shield_max = int("petShieldEnergyMax");
            pet.fuel = int("petCurrentFuel");
            pet.fuel_max = int("petMaxFuel");
            pet.speed = int("petSpeed");
            pet.loot_id = loot;
            if !name.is_empty() {
                pet.name = name;
            }
            // Only a pet that already has an entity row is mirrored.
            if world.world.entities.contains_key(&pid) {
                let pet = world.pets.by_id[&pid].clone();
                sync_entity(world, &pet, None);
            }
        }
        PET_HITPOINTS_UPDATE => with_hero_pet(world, |pet| {
            pet.hitpoints = int("hitpointsNow");
            pet.hitpoints_max = int("hitpointsMax");
        }),
        PET_SHIELD_UPDATE => with_hero_pet(world, |pet| {
            pet.shield = int("petShieldNow");
            pet.shield_max = int("petShieldMax");
        }),
        PET_FUEL_UPDATE => with_hero_pet(world, |pet| {
            pet.fuel = int("petFuelNow");
            pet.fuel_max = int("petFuelMax");
        }),
        PET_EXPERIENCE_UPDATE => with_hero_pet(world, |pet| {
            pet.experience = p.f64("currentExperiencePoints").unwrap_or(0.0);
            pet.experience_to_next = p.f64("maxExperiencePoints").unwrap_or(0.0);
        }),
        PET_LEVEL_UPDATE => with_hero_pet(world, |pet| {
            // `assembleLevelUp(level, xpUntilNext, designId, stage)` —
            // levelling up can swap the pet's design.
            pet.level = int("petLevel");
            pet.experience_to_next = p.f64("petExperiencePointsUntilNextLevel").unwrap_or(0.0);
            let design = int("designId");
            if design != 0 {
                pet.design_id = design;
            }
            pet.expansion_stage = int("expansionStage");
        }),
        PET_IDLE_MODE => {
            let pid = int("petId");
            let idle = p.boolean("isIdle").unwrap_or(false);
            if let Some(pet) = world.pets.by_id.get_mut(&pid) {
                pet.idle = idle;
            }
        }
        PET_IS_DESTROYED => with_hero_pet(world, |pet| pet.destroyed = true),
        PET_INITIALIZATION => {
            world.pets.init = Some((
                p.boolean("hasPet").unwrap_or(false),
                p.boolean("hasFuel").unwrap_or(false),
                p.boolean("petIsAlive").unwrap_or(false),
            ));
        }
        PET_GEAR_ADD => {
            if let Some(gear_type) = gear_type_of(p) {
                let gear = PetGear {
                    gear_type,
                    level: int("level"),
                    amount: int("amount"),
                    enabled: p.boolean("enabled").unwrap_or(true),
                    active: false,
                };
                match world
                    .pets
                    .gears
                    .iter_mut()
                    .find(|g| g.gear_type == gear_type)
                {
                    Some(g) => {
                        let active = g.active;
                        *g = gear;
                        g.active = active;
                    }
                    None => world.pets.gears.push(gear),
                }
            }
        }
        PET_GEAR_REMOVE => {
            if let Some(gear_type) = gear_type_of(p) {
                world.pets.gears.retain(|g| g.gear_type != gear_type);
            }
        }
        PET_GEAR_RESET => {
            world.pets.gears.clear();
            world.pets.selected_gear = None;
        }
        PET_GEAR_SELECT => {
            world.pets.selected_gear = gear_type_of(p);
        }
        PET_GEAR_ACTIVATION_CHANGE => {
            if let Some(gear_type) = gear_type_of(p) {
                let active = p.boolean("active").unwrap_or(false);
                if let Some(g) = world
                    .pets
                    .gears
                    .iter_mut()
                    .find(|g| g.gear_type == gear_type)
                {
                    g.active = active;
                }
            }
        }
        PET_HARVEST_EFFECT => {
            crate::net::util::log_once("pet-harvest-effect", format!("{p:?}"));
        }
        id::SHIP_REMOVE | crate::net::ships::ASSET_REMOVE_COMMAND => {
            if let Some(uid) =
                modules::parse_ship_remove(p).or_else(|| p.int("uid").ok().map(|v| v as i32))
            {
                if world.pets.by_id.contains_key(&uid) {
                    world.pets.by_id.remove(&uid);
                    if world.pets.hero == Some(uid) {
                        world.pets.hero = None;
                    }
                }
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xenomit_packet_client::packet::Value;

    fn packet(id: i16, name: &'static str, fields: Vec<(&'static str, Value)>) -> Packet {
        Packet {
            id,
            name: Some(name),
            fields: fields.into_iter().map(|(n, v)| (Some(n), v)).collect(),
        }
    }

    fn s(v: &str) -> Value {
        Value::Str(v.to_owned())
    }

    fn hero_activation() -> Packet {
        packet(
            PET_HERO_ACTIVATION,
            "PetHeroActivationCommand",
            vec![
                ("ownerId", Value::Int(1)),
                ("petId", Value::Int(900)),
                ("shipType", Value::Int(13)),
                ("expansionStage", Value::Int(2)),
                ("petName", s("Rex")),
                ("factionId", Value::Int(3)),
                ("clanId", Value::Int(5)),
                ("petLevel", Value::Int(4)),
                ("clanTag", s("TAG")),
                ("x", Value::Int(1000)),
                ("y", Value::Int(2000)),
                ("petSpeed", Value::Int(300)),
            ],
        )
    }

    #[test]
    fn hero_activation_spawns_entity_row() {
        let mut w = SharedWorld::default();
        w.apply_packet(&hero_activation());
        let pet = w.pets.hero_pet().expect("hero pet");
        assert_eq!((pet.id, pet.level, pet.design_id), (900, 4, 13));
        assert!(pet.is_hero_pet);
        assert_eq!(pet.plate_name(), "P.E.T Rex [4]");
        let e = &w.world.entities[&900];
        assert_eq!(e.kind, EntityKind::Ship);
        assert_eq!(e.type_id, "13");
        assert_eq!((e.x, e.y), (1000, 2000));
        assert_eq!(e.name, "Rex");
        assert_eq!(e.clan_tag, "TAG");
        assert!(!e.npc);
    }

    #[test]
    fn map_pet_activation_and_idle() {
        let mut w = SharedWorld::default();
        let p = packet(
            PET_ACTIVATION,
            "PetActivationCommand",
            vec![
                ("ownerId", Value::Int(77)),
                ("petId", Value::Int(501)),
                ("petDesignId", Value::Int(12)),
                ("expansionStage", Value::Int(1)),
                ("petName", s("Fido")),
                ("petFactionId", Value::Int(2)),
                ("petClanID", Value::Int(0)),
                ("petLevel", Value::Int(2)),
                ("clanTag", s("")),
                ("x", Value::Int(10)),
                ("y", Value::Int(20)),
                ("petSpeed", Value::Int(200)),
                ("isInIdleMode", Value::Bool(true)),
                ("isVisible", Value::Bool(false)),
            ],
        );
        w.apply_packet(&p);
        assert!(w.pets.hero.is_none(), "someone else's pet");
        let pet = w.pets.get(501).unwrap();
        assert!(pet.idle && pet.cloaked && !pet.is_hero_pet);
        assert!(w.pets.is_hidden(501));
        assert_eq!(w.world.entities[&501].type_id, "12");

        w.apply_packet(&packet(
            PET_IDLE_MODE,
            "PetIdleModeCommand",
            vec![("petId", Value::Int(501)), ("isIdle", Value::Bool(false))],
        ));
        assert!(!w.pets.is_hidden(501));
    }

    #[test]
    fn status_and_updates_drive_hero_vitals() {
        let mut w = SharedWorld::default();
        w.apply_packet(&hero_activation());
        w.apply_packet(&packet(
            PET_STATUS,
            "PetStatusCommand",
            vec![
                ("petId", Value::Int(900)),
                ("petLevel", Value::Int(5)),
                ("petExperiencePoints", Value::F64(120.0)),
                ("petExperiencePointsUntilNextLevel", Value::F64(880.0)),
                ("petHitPoints", Value::Int(4000)),
                ("petHitPointsMax", Value::Int(5000)),
                ("petShieldEnergyNow", Value::Int(100)),
                ("petShieldEnergyMax", Value::Int(200)),
                ("petCurrentFuel", Value::Int(30)),
                ("petMaxFuel", Value::Int(60)),
                ("petSpeed", Value::Int(310)),
                ("petName", s("Rex")),
                ("petLootId", s("pet_basic")),
            ],
        ));
        let pet = w.pets.hero_pet().unwrap();
        assert_eq!(
            (pet.level, pet.hitpoints, pet.hitpoints_max),
            (5, 4000, 5000)
        );
        assert_eq!(
            (pet.fuel, pet.fuel_max, pet.loot_id.as_str()),
            (30, 60, "pet_basic")
        );
        let e = &w.world.entities[&900];
        assert_eq!((e.hitpoints, e.hitpoints_max), (Some(4000.0), Some(5000.0)));
        assert_eq!((e.shield, e.shield_max), (Some(100), Some(200)));

        w.apply_packet(&packet(
            PET_HITPOINTS_UPDATE,
            "PetHitpointsUpdateCommand",
            vec![
                ("hitpointsNow", Value::Int(2500)),
                ("hitpointsMax", Value::Int(5000)),
                ("useRepairGear", Value::Bool(false)),
            ],
        ));
        w.apply_packet(&packet(
            PET_FUEL_UPDATE,
            "PetFuelUpdateCommand",
            vec![
                ("petFuelNow", Value::Int(10)),
                ("petFuelMax", Value::Int(60)),
            ],
        ));
        w.apply_packet(&packet(
            PET_SHIELD_UPDATE,
            "PetShieldUpdateCommand",
            vec![
                ("petShieldNow", Value::Int(0)),
                ("petShieldMax", Value::Int(200)),
            ],
        ));
        let pet = w.pets.hero_pet().unwrap();
        assert_eq!((pet.hitpoints, pet.fuel, pet.shield), (2500, 10, 0));
        assert_eq!(w.world.entities[&900].hitpoints, Some(2500.0));

        // Level-up swaps the design.
        w.apply_packet(&packet(
            PET_LEVEL_UPDATE,
            "PetLevelUpdateCommand",
            vec![
                ("petLevel", Value::Int(7)),
                ("petExperiencePointsUntilNextLevel", Value::F64(500.0)),
                ("designId", Value::Int(14)),
                ("expansionStage", Value::Int(3)),
            ],
        ));
        assert_eq!(w.pets.hero_pet().unwrap().level, 7);
        assert_eq!(w.world.entities[&900].type_id, "14");
    }

    #[test]
    fn status_before_activation_is_kept() {
        let mut w = SharedWorld::default();
        w.apply_packet(&packet(
            PET_STATUS,
            "PetStatusCommand",
            vec![
                ("petId", Value::Int(900)),
                ("petLevel", Value::Int(5)),
                ("petHitPoints", Value::Int(10)),
                ("petHitPointsMax", Value::Int(20)),
                ("petLootId", s("x")),
            ],
        ));
        assert!(!w.world.entities.contains_key(&900), "no entity yet");
        w.apply_packet(&hero_activation());
        let pet = w.pets.hero_pet().unwrap();
        assert_eq!((pet.hitpoints, pet.hitpoints_max), (10, 20));
        assert_eq!(pet.loot_id, "x");
    }

    #[test]
    fn deactivation_and_destruction() {
        let mut w = SharedWorld::default();
        w.apply_packet(&hero_activation());
        w.apply_packet(&packet(
            PET_IS_DESTROYED,
            "PetIsDestroyedCommand",
            vec![("destroyedType", Value::Int(0))],
        ));
        assert!(w.pets.hero_pet().unwrap().destroyed);
        assert!(w.world.entities[&900].destroyed);

        w.apply_packet(&packet(
            PET_DEACTIVATION,
            "PetDeactivationCommand",
            vec![("petId", Value::Int(900))],
        ));
        assert!(w.pets.hero.is_none() && w.pets.by_id.is_empty());
        assert!(!w.world.entities.contains_key(&900));
    }

    fn gear_packet(
        id: i16,
        name: &'static str,
        ty: i64,
        extra: Vec<(&'static str, Value)>,
    ) -> Packet {
        let mut fields = vec![(
            "gearType",
            Value::Module(Box::new(packet(
                164,
                "PetGearTypeModule",
                vec![("typeValue", Value::Int(ty))],
            ))),
        )];
        fields.extend(extra);
        packet(id, name, fields)
    }

    #[test]
    fn gear_list_tracks_add_remove_activation() {
        let mut w = SharedWorld::default();
        w.apply_packet(&gear_packet(
            PET_GEAR_ADD,
            "PetGearAddCommand",
            4,
            vec![
                ("level", Value::Int(2)),
                ("amount", Value::Int(1)),
                ("enabled", Value::Bool(true)),
            ],
        ));
        w.apply_packet(&gear_packet(
            PET_GEAR_ADD,
            "PetGearAddCommand",
            2,
            vec![
                ("level", Value::Int(1)),
                ("amount", Value::Int(1)),
                ("enabled", Value::Bool(false)),
            ],
        ));
        assert_eq!(w.pets.gears.len(), 2);
        w.apply_packet(&gear_packet(
            PET_GEAR_ACTIVATION_CHANGE,
            "PetGearActivationChangeCommand",
            4,
            vec![("active", Value::Bool(true))],
        ));
        assert!(
            w.pets
                .gears
                .iter()
                .find(|g| g.gear_type == 4)
                .unwrap()
                .active
        );
        // Re-add keeps the active flag (upsert).
        w.apply_packet(&gear_packet(
            PET_GEAR_ADD,
            "PetGearAddCommand",
            4,
            vec![
                ("level", Value::Int(3)),
                ("amount", Value::Int(1)),
                ("enabled", Value::Bool(true)),
            ],
        ));
        let g = w.pets.gears.iter().find(|g| g.gear_type == 4).unwrap();
        assert!(g.active && g.level == 3);
        w.apply_packet(&gear_packet(
            PET_GEAR_SELECT,
            "PetGearSelectCommand",
            2,
            vec![],
        ));
        assert_eq!(w.pets.selected_gear, Some(2));
        w.apply_packet(&gear_packet(
            PET_GEAR_REMOVE,
            "PetGearRemoveCommand",
            2,
            vec![("level", Value::Int(1)), ("amount", Value::Int(1))],
        ));
        assert_eq!(w.pets.gears.len(), 1);
        w.apply_packet(&packet(PET_GEAR_RESET, "PetGearResetCommand", vec![]));
        assert!(w.pets.gears.is_empty());
        assert_eq!(gear_name(4), "Auto-loot");
    }

    #[test]
    fn ship_remove_drops_map_pet() {
        let mut w = SharedWorld::default();
        w.apply_packet(&hero_activation());
        w.apply_packet(&packet(
            id::SHIP_REMOVE,
            "ShipRemoveCommand",
            vec![("userId", Value::Int(900))],
        ));
        assert!(w.pets.by_id.is_empty());
        assert!(w.pets.hero.is_none());
    }
}
