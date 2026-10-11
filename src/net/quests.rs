//! Quest log (`questfm` / `QuestProxy`): accepted quests with their
//! condition trees, the list of available quests, and finished quests.
//!
//! Wire modules (ids from `xenomit-packet-client` `modules.json`):
//!
//! | id    | module                       | effect                                   |
//! |-------|------------------------------|------------------------------------------|
//! | 2017  | `QuestInitializationCommand` | accepted quest, full definition + state  |
//! | 2075  | `QuestListCommand`           | available quests (`QuestSlimInfoModule`) |
//! | 12122 | `QuestListUpdateCommand`     | upsert available rows                    |
//! | 12121 | `QuestListRemoveCommand`     | drop available rows                      |
//! | 2090  | `QuestInfoCommand`           | definition of any quest (`info`)         |
//! | 2014  | `QuestConditionUpdateCommand`| `{questConditionId, state}` progress     |
//! | 2006  | `QuestCancelledCommand`      | accepted quest cancelled                 |
//! | 2004  | `QuestCompletedCommand`      | accepted quest completed                 |
//! | 2005  | `QuestFailedCommand`         | accepted quest failed                    |
//!
//! A `QuestDefinitionModule.rootCase` is a tree: `QuestCaseModule.modifier`
//! holds `QuestElementModule`s, each either a nested case or a
//! `QuestConditionModule` (which can nest `subConditions`).

use std::collections::{BTreeMap, HashMap};
use xenomit_packet_client::packet::{Packet, Value};

use crate::locale::LocaleCatalog;
use crate::net::world::SharedWorld;

pub(crate) const QUEST_INITIALIZATION: i16 = 2017;
pub(crate) const QUEST_LIST: i16 = 2075;
pub(crate) const QUEST_LIST_UPDATE: i16 = 12122;
pub(crate) const QUEST_LIST_REMOVE: i16 = 12121;
pub(crate) const QUEST_INFO: i16 = 2090;
pub(crate) const QUEST_CONDITION_UPDATE: i16 = 2014;
pub(crate) const QUEST_CANCELLED: i16 = 2006;
pub(crate) const QUEST_COMPLETED: i16 = 2004;
pub(crate) const QUEST_FAILED: i16 = 2005;

/// Finished quests remembered for the window's history section.
const FINISHED_CAP: usize = 12;

/// `QuestConditionStateModule` + the static condition fields.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct QuestCondition {
    pub id: i32,
    /// `QuestConditionModule.type` — `CONDITIONTYPE_*`.
    pub kind: i16,
    pub display_type: i16,
    pub target: f64,
    pub current: f64,
    pub mandatory: bool,
    pub active: bool,
    pub completed: bool,
    /// `matches` — npc / map / item ids the condition is about.
    pub matches: Vec<String>,
    pub children: Vec<QuestNode>,
}

/// `QuestCaseModule` — group of elements.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct QuestCase {
    pub id: i32,
    pub active: bool,
    pub mandatory: bool,
    pub ordered: bool,
    /// How many mandatory children must be met (0 = all).
    pub mandatory_count: i32,
    pub children: Vec<QuestNode>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum QuestNode {
    Case(QuestCase),
    Condition(QuestCondition),
}

/// One accepted (or inspected) quest definition.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Quest {
    pub id: i32,
    pub text_key: String,
    pub portrait: String,
    pub star: i32,
    /// `QuestTypeModule.type` values (`QUESTTYPE_*`).
    pub types: Vec<i16>,
    pub root: QuestCase,
    /// Number of reward rows (`rewards` loot modules).
    pub reward_count: usize,
}

/// Available-quest row (`QuestSlimInfoModule`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct QuestSlim {
    pub id: i32,
    pub text_key: String,
    pub min_level: i32,
    pub priority: i32,
    pub star: i32,
    pub types: Vec<i16>,
    /// `acceptabilityStatus.type` — 0 when unknown.
    pub acceptability: i16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuestEnd {
    Completed,
    Failed,
    Cancelled,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FinishedQuest {
    pub id: i32,
    pub text_key: String,
    pub end: QuestEnd,
}

/// Session-scope quest model.
#[derive(Clone, Debug, Default)]
pub struct QuestLog {
    /// Accepted quests by id.
    pub active: BTreeMap<i32, Quest>,
    /// Quests on offer (`QuestListCommand`).
    pub available: BTreeMap<i32, QuestSlim>,
    /// `QuestInfoCommand` definitions of non-accepted quests.
    pub info: HashMap<i32, Quest>,
    pub finished: Vec<FinishedQuest>,
    pub max_quests: i32,
    pub max_event_quests: i32,
    pub only_starter: bool,
    /// Bumps on every change — lets the UI cache layouts.
    pub revision: u32,
}

/// Flattened objective row for display.
#[derive(Clone, Debug, PartialEq)]
pub struct ObjectiveRow {
    pub depth: usize,
    pub text: String,
    /// `Some((current, target))` for counted conditions.
    pub progress: Option<(f64, f64)>,
    pub done: bool,
    pub active: bool,
    pub mandatory: bool,
}

impl QuestCondition {
    fn is_done(&self) -> bool {
        self.completed || (self.target > 0.0 && self.current >= self.target)
    }
}

impl Quest {
    /// Depth-first objective rows (case headers only where they carry a
    /// "N of M" rule).
    pub fn objectives(&self) -> Vec<ObjectiveRow> {
        let mut rows = Vec::new();
        walk_case(&self.root, 0, &mut rows);
        rows
    }

    /// True once every mandatory top-level objective is done.
    pub fn all_done(&self) -> bool {
        let rows = self.objectives();
        !rows.is_empty() && rows.iter().filter(|r| r.mandatory).all(|r| r.done)
    }

    fn find_condition_mut(&mut self, id: i32) -> Option<&mut QuestCondition> {
        find_in_case(&mut self.root, id)
    }
}

fn find_in_case(case: &mut QuestCase, id: i32) -> Option<&mut QuestCondition> {
    for n in &mut case.children {
        match n {
            QuestNode::Case(c) => {
                if let Some(f) = find_in_case(c, id) {
                    return Some(f);
                }
            }
            QuestNode::Condition(c) => {
                if let Some(f) = find_in_condition(c, id) {
                    return Some(f);
                }
            }
        }
    }
    None
}

fn find_in_condition(cond: &mut QuestCondition, id: i32) -> Option<&mut QuestCondition> {
    if cond.id == id {
        return Some(cond);
    }
    for n in &mut cond.children {
        match n {
            QuestNode::Case(c) => {
                if let Some(f) = find_in_case(c, id) {
                    return Some(f);
                }
            }
            QuestNode::Condition(c) => {
                if let Some(f) = find_in_condition(c, id) {
                    return Some(f);
                }
            }
        }
    }
    None
}

fn walk_case(case: &QuestCase, depth: usize, out: &mut Vec<ObjectiveRow>) {
    let n = case.children.len() as i32;
    let nested_depth = if case.mandatory_count > 0 && case.mandatory_count < n && depth == 0 {
        out.push(ObjectiveRow {
            depth,
            text: format!("Complete {} of {}", case.mandatory_count, n),
            progress: None,
            done: false,
            active: case.active,
            mandatory: case.mandatory,
        });
        depth + 1
    } else {
        depth
    };
    for child in &case.children {
        match child {
            QuestNode::Case(c) => walk_case(c, nested_depth + 1, out),
            QuestNode::Condition(c) => walk_condition(c, nested_depth, out),
        }
    }
}

fn walk_condition(c: &QuestCondition, depth: usize, out: &mut Vec<ObjectiveRow>) {
    out.push(ObjectiveRow {
        depth,
        text: condition_label(c.kind, &c.matches),
        progress: (c.target > 0.0).then_some((c.current, c.target)),
        done: c.is_done(),
        active: c.active,
        mandatory: c.mandatory,
    });
    for child in &c.children {
        match child {
            QuestNode::Case(cs) => walk_case(cs, depth + 1, out),
            QuestNode::Condition(cc) => walk_condition(cc, depth + 1, out),
        }
    }
}

/// Human label for a `CONDITIONTYPE_*` plus its `matches`.
pub fn condition_label(kind: i16, matches: &[String]) -> String {
    let base = match kind {
        0 => "Complete objectives",
        1 => "Timer",
        2 => "Haste",
        3 => "Endurance",
        4 => "Countdown",
        5 => "Collect",
        6 => "Destroy",
        7 => "Deal damage",
        8 => "Avoid damage",
        9 => "Take damage",
        10 => "Avoid death",
        11 => "Reach coordinates",
        12 => "Travel distance",
        13 => "Travel",
        14 => "Fuel shortage",
        15 => "Stay near",
        16 => "Visit map",
        17 => "Visit different maps",
        _ => "Objective",
    };
    if matches.is_empty() {
        base.to_owned()
    } else {
        let shown: Vec<&str> = matches.iter().take(3).map(String::as_str).collect();
        format!("{base}: {}", shown.join(", "))
    }
}

/// Quest title: `quests.quest_title_{id}`, then `…_{textKey}`, then the
/// raw key / `Quest #id` (`QuestLocale.getTitleEx`, minus faction suffixes).
pub fn quest_title(loc: &LocaleCatalog, id: i32, text_key: &str) -> String {
    let by_id = format!("quests.quest_title_{id}");
    if let Some(t) = loc.lookup(&by_id).filter(|t| !t.starts_with('{')) {
        return t.to_owned();
    }
    if !text_key.is_empty() {
        let by_key = format!("quests.quest_title_{text_key}");
        if let Some(t) = loc.lookup(&by_key).filter(|t| !t.starts_with('{')) {
            return t.to_owned();
        }
        return text_key.to_owned();
    }
    format!("Quest #{id}")
}

/// `QuestTypeModule` label for badges.
pub fn quest_type_label(t: i16) -> &'static str {
    match t {
        1 => "Starter",
        2 => "Mission",
        3 => "Daily",
        4 => "Challenge",
        5 => "Event",
        6 => "Weekly",
        7 => "Battle pass",
        8 => "Diminishing",
        9 => "Season pass",
        10 => "Free period",
        _ => "",
    }
}

// ---------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------

fn parse_types(p: &Packet) -> Vec<i16> {
    p.list("types")
        .unwrap_or(&[])
        .iter()
        .filter_map(|v| match v {
            Value::Module(m) => m.int("type").ok().map(|t| t as i16),
            _ => None,
        })
        .collect()
}

fn parse_condition(p: &Packet) -> QuestCondition {
    let mut c = QuestCondition {
        id: p.int("id").unwrap_or(0) as i32,
        kind: p.int("type").unwrap_or(0) as i16,
        display_type: p.int("displayType").unwrap_or(0) as i16,
        target: p.f64("targetValue").unwrap_or(0.0),
        mandatory: p.boolean("mandatory").unwrap_or(false),
        matches: p
            .list("matches")
            .unwrap_or(&[])
            .iter()
            .filter_map(|v| match v {
                Value::Str(s) => Some(s.clone()),
                _ => None,
            })
            .collect(),
        ..Default::default()
    };
    if let Ok(st) = p.module("state") {
        apply_state(&mut c, st);
    }
    for v in p.list("subConditions").unwrap_or(&[]) {
        if let Value::Module(m) = v {
            c.children.push(QuestNode::Condition(parse_condition(m)));
        }
    }
    c
}

fn apply_state(c: &mut QuestCondition, st: &Packet) {
    c.current = st.f64("currentValue").unwrap_or(0.0);
    c.active = st.boolean("active").unwrap_or(false);
    c.completed = st.boolean("completed").unwrap_or(false);
}

fn parse_case(p: &Packet) -> QuestCase {
    let mut case = QuestCase {
        id: p.int("id").unwrap_or(0) as i32,
        active: p.boolean("active").unwrap_or(false),
        mandatory: p.boolean("mandatory").unwrap_or(false),
        ordered: p.boolean("ordered").unwrap_or(false),
        mandatory_count: p.int("mandatoryCount").unwrap_or(0) as i32,
        children: Vec::new(),
    };
    for v in p.list("modifier").unwrap_or(&[]) {
        let Value::Module(el) = v else {
            continue;
        };
        // `QuestElementModule` wraps a case or a condition; tolerate a bare
        // case/condition module in the list too.
        if let Ok(c) = el.module("questCase") {
            case.children.push(QuestNode::Case(parse_case(c)));
        } else if let Ok(c) = el.module("condition") {
            case.children.push(QuestNode::Condition(parse_condition(c)));
        } else if el.get("modifier").is_some() {
            case.children.push(QuestNode::Case(parse_case(el)));
        } else if el.get("targetValue").is_some() {
            case.children
                .push(QuestNode::Condition(parse_condition(el)));
        }
    }
    case
}

fn parse_definition(p: &Packet) -> Quest {
    Quest {
        id: p.int("id").unwrap_or(0) as i32,
        text_key: p.string("textKeyFoundation").unwrap_or("").to_owned(),
        portrait: p.string("portraitId").unwrap_or("").to_owned(),
        star: p.int("star").unwrap_or(0) as i32,
        types: parse_types(p),
        root: p.module("rootCase").map(parse_case).unwrap_or_default(),
        reward_count: p.list("rewards").map(|l| l.len()).unwrap_or(0),
    }
}

fn parse_slim(p: &Packet) -> QuestSlim {
    QuestSlim {
        id: p.int("questId").unwrap_or(0) as i32,
        text_key: p.string("textKeyFoundation").unwrap_or("").to_owned(),
        min_level: p.int("minLevel").unwrap_or(0) as i32,
        priority: p.int("priority").unwrap_or(0) as i32,
        star: p.int("star").unwrap_or(0) as i32,
        types: parse_types(p),
        acceptability: p
            .module("acceptabilityStatus")
            .and_then(|m| m.int("type"))
            .unwrap_or(0) as i16,
    }
}

// ---------------------------------------------------------------------
// Handler
// ---------------------------------------------------------------------

pub fn handle(world: &mut SharedWorld, p: &Packet) {
    match p.id {
        QUEST_INITIALIZATION => {
            let Ok(def) = p.module("quest") else {
                return;
            };
            let quest = parse_definition(def);
            if quest.id == 0 {
                return;
            }
            world.quests.available.remove(&quest.id);
            world.quests.active.insert(quest.id, quest);
            world.quests.revision += 1;
        }
        QUEST_LIST => {
            world.quests.available = p
                .list("list")
                .unwrap_or(&[])
                .iter()
                .filter_map(|v| match v {
                    Value::Module(m) => Some(parse_slim(m)),
                    _ => None,
                })
                .filter(|s| s.id != 0)
                .map(|s| (s.id, s))
                .collect();
            world.quests.only_starter = p.boolean("onlyStarter").unwrap_or(false);
            world.quests.max_quests = p.int("maxQuests").unwrap_or(0) as i32;
            world.quests.max_event_quests = p.int("maxEventQuests").unwrap_or(0) as i32;
            world.quests.revision += 1;
        }
        QUEST_LIST_UPDATE => {
            for v in p.list("list").unwrap_or(&[]) {
                if let Value::Module(m) = v {
                    let s = parse_slim(m);
                    if s.id != 0 && !world.quests.active.contains_key(&s.id) {
                        world.quests.available.insert(s.id, s);
                    }
                }
            }
            world.quests.revision += 1;
        }
        QUEST_LIST_REMOVE => {
            for v in p.list("list").unwrap_or(&[]) {
                if let Value::Int(id) = v {
                    world.quests.available.remove(&(*id as i32));
                }
            }
            world.quests.revision += 1;
        }
        QUEST_INFO => {
            let Ok(def) = p.module("definition") else {
                return;
            };
            let quest = parse_definition(def);
            if quest.id == 0 {
                return;
            }
            if world.quests.active.contains_key(&quest.id) {
                world.quests.active.insert(quest.id, quest);
            } else {
                world.quests.info.insert(quest.id, quest);
            }
            world.quests.revision += 1;
        }
        QUEST_CONDITION_UPDATE => {
            let Ok(cid) = p.int("questConditionId") else {
                return;
            };
            let Ok(state) = p.module("state") else {
                return;
            };
            for q in world.quests.active.values_mut() {
                if let Some(c) = q.find_condition_mut(cid as i32) {
                    apply_state(c, state);
                }
            }
            world.quests.revision += 1;
        }
        QUEST_CANCELLED | QUEST_COMPLETED | QUEST_FAILED => {
            let Ok(id) = p.int("id") else {
                return;
            };
            let id = id as i32;
            let end = match p.id {
                QUEST_COMPLETED => QuestEnd::Completed,
                QUEST_FAILED => QuestEnd::Failed,
                _ => QuestEnd::Cancelled,
            };
            let text_key = world
                .quests
                .active
                .remove(&id)
                .map(|q| q.text_key)
                .unwrap_or_default();
            let title = quest_title(&world.locale, id, &text_key);
            world.push_log(match end {
                QuestEnd::Completed => format!("Quest completed: {title}"),
                QuestEnd::Failed => format!("Quest failed: {title}"),
                QuestEnd::Cancelled => format!("Quest cancelled: {title}"),
            });
            world
                .quests
                .finished
                .push(FinishedQuest { id, text_key, end });
            if world.quests.finished.len() > FINISHED_CAP {
                world.quests.finished.remove(0);
            }
            world.quests.revision += 1;
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pkt(id: i16, fields: Vec<(&'static str, Value)>) -> Packet {
        Packet {
            id,
            name: None,
            fields: fields.into_iter().map(|(n, v)| (Some(n), v)).collect(),
        }
    }

    fn m(p: Packet) -> Value {
        Value::Module(Box::new(p))
    }

    fn state(cur: f64, active: bool, done: bool) -> Value {
        m(pkt(
            2037,
            vec![
                ("currentValue", Value::F64(cur)),
                ("active", Value::Bool(active)),
                ("completed", Value::Bool(done)),
            ],
        ))
    }

    fn condition(id: i64, kind: i64, target: f64, cur: f64) -> Packet {
        pkt(
            2002,
            vec![
                ("id", Value::Int(id)),
                (
                    "matches",
                    Value::List(vec![Value::Str("npc_streuner".into())]),
                ),
                ("type", Value::Int(kind)),
                ("displayType", Value::Int(0)),
                ("targetValue", Value::F64(target)),
                ("mandatory", Value::Bool(true)),
                ("state", state(cur, true, false)),
                ("subConditions", Value::List(vec![])),
            ],
        )
    }

    fn element_cond(c: Packet) -> Value {
        m(pkt(
            2001,
            vec![("questCase", Value::Null), ("condition", m(c))],
        ))
    }

    fn definition(id: i64) -> Packet {
        let root = pkt(
            2000,
            vec![
                ("id", Value::Int(1)),
                ("active", Value::Bool(true)),
                ("mandatory", Value::Bool(true)),
                ("ordered", Value::Bool(false)),
                ("mandatoryCount", Value::Int(0)),
                (
                    "modifier",
                    Value::List(vec![
                        element_cond(condition(10, 6, 5.0, 2.0)),
                        element_cond(condition(11, 5, 3.0, 0.0)),
                    ]),
                ),
            ],
        );
        pkt(
            2099,
            vec![
                ("id", Value::Int(id)),
                (
                    "types",
                    Value::List(vec![m(pkt(4141, vec![("type", Value::Int(3))]))]),
                ),
                ("rootCase", m(root)),
                ("rewards", Value::List(vec![])),
                ("icons", Value::List(vec![])),
                ("textKeyFoundation", Value::Str("daily_kill".into())),
                ("portraitId", Value::Str("".into())),
                ("star", Value::Int(0)),
            ],
        )
    }

    fn init(id: i64) -> Packet {
        pkt(2017, vec![("quest", m(definition(id)))])
    }

    #[test]
    fn initialization_builds_condition_tree() {
        let mut w = SharedWorld::default();
        w.apply_packet(&init(77));
        let q = &w.quests.active[&77];
        assert_eq!(q.types, vec![3]);
        assert_eq!(q.text_key, "daily_kill");
        let rows = q.objectives();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].progress, Some((2.0, 5.0)));
        assert!(!rows[0].done);
        assert!(rows[0].text.starts_with("Destroy"));
        assert!(!q.all_done());
    }

    #[test]
    fn condition_update_ticks_progress() {
        let mut w = SharedWorld::default();
        w.apply_packet(&init(77));
        w.apply_packet(&pkt(
            2014,
            vec![
                ("questConditionId", Value::Int(10)),
                ("state", state(5.0, true, true)),
            ],
        ));
        w.apply_packet(&pkt(
            2014,
            vec![
                ("questConditionId", Value::Int(11)),
                ("state", state(3.0, true, false)),
            ],
        ));
        let q = &w.quests.active[&77];
        let rows = q.objectives();
        assert!(rows.iter().all(|r| r.done), "{rows:?}");
        assert!(q.all_done());
        // Unknown condition id is a no-op.
        w.apply_packet(&pkt(
            2014,
            vec![
                ("questConditionId", Value::Int(999)),
                ("state", state(1.0, true, false)),
            ],
        ));
        assert_eq!(w.quests.active.len(), 1);
    }

    #[test]
    fn list_update_remove_and_accept() {
        let mut w = SharedWorld::default();
        let slim = |id: i64| {
            m(pkt(
                2078,
                vec![
                    ("questId", Value::Int(id)),
                    ("rootCaseId", Value::Int(0)),
                    ("minLevel", Value::Int(4)),
                    ("priority", Value::Int(1)),
                    ("types", Value::List(vec![])),
                    ("icon", Value::Null),
                    (
                        "acceptabilityStatus",
                        m(pkt(10101, vec![("type", Value::Int(2))])),
                    ),
                    ("missingAcceptRequirements", Value::List(vec![])),
                    ("textKeyFoundation", Value::Str("k".into())),
                    ("portraitId", Value::Str("".into())),
                    ("star", Value::Int(0)),
                ],
            ))
        };
        w.apply_packet(&pkt(
            2075,
            vec![
                ("list", Value::List(vec![slim(1), slim(2), slim(77)])),
                ("onlyStarter", Value::Bool(false)),
                ("maxQuests", Value::Int(10)),
                ("maxEventQuests", Value::Int(3)),
            ],
        ));
        assert_eq!(w.quests.available.len(), 3);
        assert_eq!(w.quests.available[&2].min_level, 4);
        assert_eq!(w.quests.available[&2].acceptability, 2);
        assert_eq!(w.quests.max_quests, 10);
        w.apply_packet(&pkt(
            12121,
            vec![("list", Value::List(vec![Value::Int(1)]))],
        ));
        assert!(!w.quests.available.contains_key(&1));
        // Accepting a listed quest moves it from available to active.
        w.apply_packet(&init(77));
        assert!(!w.quests.available.contains_key(&77));
        assert!(w.quests.active.contains_key(&77));
        // A later list update does not resurrect an accepted quest.
        w.apply_packet(&pkt(
            12122,
            vec![("list", Value::List(vec![slim(77), slim(5)]))],
        ));
        assert!(!w.quests.available.contains_key(&77));
        assert!(w.quests.available.contains_key(&5));
    }

    #[test]
    fn end_states_move_to_finished() {
        let mut w = SharedWorld::default();
        w.apply_packet(&init(77));
        w.apply_packet(&init(78));
        w.apply_packet(&init(79));
        w.apply_packet(&pkt(
            2004,
            vec![("id", Value::Int(77)), ("rating", Value::Int(1))],
        ));
        w.apply_packet(&pkt(2005, vec![("id", Value::Int(78))]));
        w.apply_packet(&pkt(2006, vec![("id", Value::Int(79))]));
        assert!(w.quests.active.is_empty());
        let ends: Vec<_> = w.quests.finished.iter().map(|f| (f.id, f.end)).collect();
        assert_eq!(
            ends,
            vec![
                (77, QuestEnd::Completed),
                (78, QuestEnd::Failed),
                (79, QuestEnd::Cancelled)
            ]
        );
        assert!(w.log.iter().any(|l| l.text.contains("Quest completed")));
    }

    #[test]
    fn info_for_unaccepted_goes_to_info_map() {
        let mut w = SharedWorld::default();
        w.apply_packet(&pkt(
            2090,
            vec![
                ("definition", m(definition(5))),
                ("ratings", Value::List(vec![])),
                ("playersRating", Value::Null),
            ],
        ));
        assert!(w.quests.info.contains_key(&5));
        assert!(w.quests.active.is_empty());
    }

    #[test]
    fn titles_resolve_through_locale() {
        let mut loc = LocaleCatalog::new("en");
        loc.set_item("quests", "quest_title_77", "Hunt Streuners", 0, false);
        loc.set_item("quests", "quest_title_daily_kill", "Daily hunt", 0, false);
        assert_eq!(quest_title(&loc, 77, "daily_kill"), "Hunt Streuners");
        assert_eq!(quest_title(&loc, 5, "daily_kill"), "Daily hunt");
        assert_eq!(quest_title(&loc, 5, "other"), "other");
        assert_eq!(quest_title(&loc, 5, ""), "Quest #5");
    }

    #[test]
    fn case_with_partial_requirement_gets_header() {
        let case = QuestCase {
            mandatory_count: 1,
            mandatory: true,
            children: vec![
                QuestNode::Condition(QuestCondition {
                    id: 1,
                    kind: 6,
                    target: 2.0,
                    ..Default::default()
                }),
                QuestNode::Condition(QuestCondition {
                    id: 2,
                    kind: 5,
                    target: 2.0,
                    ..Default::default()
                }),
            ],
            ..Default::default()
        };
        let q = Quest {
            root: case,
            ..Default::default()
        };
        let rows = q.objectives();
        assert_eq!(rows[0].text, "Complete 1 of 2");
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[1].depth, 1);
    }
}
