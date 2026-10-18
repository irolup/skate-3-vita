//! A pedestrian's mood: what happened around it (the skater nearby, a collision, a trick) and the
//! want producer that turns it into wants for the ped AI graph (doc 26 "Ped behaviour runtime").
//! All rules come from the stock mood tables (`livingworld_moodeventcategories`,
//! `livingworld_entities_moodreactions`, `livingworld_entities_moodresults`, exported by setup);
//! this module holds no values of its own.
//!
//! Retail (TU3, evidence only; re-implemented; `.local/research/peds/b4-ped-want-producer.md`,
//! `b5-ped-mood-events.md`, `b6-ped-mood-fields.md`; main checked the key addresses):
//! - posting `sub_82E41060`: an existing record with the same (instigator, second entity,
//!   category) is bumped (`sub_82E41750`: magnitude += the category's per-event magnitude, count
//!   += 1, age = 0, position = the event's), else a new one is pushed (`sub_82E414D8`: at most 10;
//!   when full only an event without an instigator or second entity may evict one). The
//!   per-event magnitude is the category field `B1DD` (the presence period, 0.5 s stock; discrete
//!   categories have none: the attribute default `0x830D0850`, 0.0);
//! - per tick (`sub_82E41600`): cooldowns run down and go at 0, ages run up, suppress timers run
//!   down to 0; magnitude and count never decay; a record older than its category's lifetime
//!   (`A379`, 30 s stock) goes (`sub_82E440E0`);
//! - presence (`sub_82E3CA20`): every presence period each player within 35 m (`0x8206D148`)
//!   posts `presence` (instigator = player, second = the ped);
//! - the producer `sub_82E41B98` / evaluator `sub_82E41EB0`: for each record not suppressed,
//!   the ped's reaction results by priority (lower wins), each checked in retail's order (see
//!   [`MoodTables::evaluate`]); the winner is rolled (`sub_8269A588`: `rand() % 100 + 1`, pass
//!   when p x 100 >= n; p 0 always passes), every roll starts the result's cooldown; on a pass
//!   the result's wants are raised on its target (instigator, second entity or the ped itself)
//!   and the record is suppressed for the result's suppress time.
//! Not decoded (answer false, documented): the target checks `vfunc +28` / `+32` (`D9F3` /
//! `D44E`), the hand-prop bit (`brain+3278` 0x02, `072D`), the ped-state prerequisites (kinds 1,
//! 2, 4, 7, 8), the sight test (`sub_82E27240`; every post is seen). The count range (`7FF7`)
//! reads the instigator's perception entry `+68`, raised wants about it (inferred).

use super::super::rng::Rng;
use super::super::Vec3;
use std::collections::BTreeMap;

/// Stock category names used by the game's posters.
pub mod category {
    pub const PRESENCE: &str = "presence";
    /// Posted by a greeting ped into the greeted ped's store (ChannelGreetWantTarget, `8269FAC0`).
    pub const GREETED: &str = "greeted";
    pub const COLLISION: &str = "collision";
    pub const NEARBY_COLLISION: &str = "nearbycollision";
    /// Posted for every skater while the zombie cheat is on (`sub_82E24B60`, category 19); raises
    /// `skaterzombiechase` in the stock sets.
    pub const ZOMBIE_MODE: &str = "zombiemode";
}

/// Console frame the skater broadcasts are posted at (`sub_826BE020` runs once per living-world update, ~30 fps on the
/// 360); ours posts at this period whatever the engine fps.
pub const BROADCAST_PERIOD: f32 = 1.0 / 30.0;

/// Presence scan radius, metres (`0x8206D148`).
pub const PRESENCE_RADIUS: f32 = 35.0;
/// Retail store size (`sub_82E414D8`).
pub const MAX_RECORDS: usize = 10;

/// One mood event category (`livingworld_moodeventcategories`).
#[derive(Clone, Debug, PartialEq)]
pub struct MoodCategory {
    /// `B1DD`: per-event magnitude and the continuous post period (0 for discrete categories).
    pub magnitude: f32,
    /// `A379`: record lifetime, s.
    pub lifetime: f32,
    /// The post's vision test ranges (`82E27240` via `sub_82E41060`: near = `D16B`, far = `A7EB`, times the ped type's
    /// perception scales) [code, b18 / b5]; zombiemode 5 / 5, presence 10 / 20, collision 20 / 20.
    pub near: f32,
    pub far: f32,
}

/// One reaction prerequisite {category, kind a, comparator b, value c}.
#[derive(Clone, Debug, PartialEq)]
pub struct Prerequisite {
    pub category: String,
    pub kind: u32,
    pub comparator: u32,
    pub value: f32,
}

/// One mood result (`livingworld_entities_moodresults`, 144-byte class), gates resolved.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MoodResult {
    /// `B45E`.
    pub category: Option<String>,
    /// `470C` + `1C72`: the instigator's entity type (or an ancestor).
    pub instigator_type: Option<String>,
    /// `FD19` + `1770`.
    pub second_type: Option<String>,
    /// `C404` + `8239`: record magnitude >= this.
    pub magnitude_at_least: Option<f32>,
    /// `B1A9` + `0982`: record count >= this.
    pub count_at_least: Option<u32>,
    /// `072D` + `ADAC`: the hand-prop bit must equal this.
    pub hand_prop: Option<bool>,
    /// `D9F3` + `AA82`, `D44E` + `9B44`: target checks (not decoded; answer false).
    pub target_check_a: Option<bool>,
    pub target_check_b: Option<bool>,
    /// `7FF7` + `9F09` / `C41A`: outstanding reactions about the instigator within [min, max].
    pub outstanding: Option<(u32, u32)>,
    /// `9ABC` + `093E` / `441C`: probability (0..1) and cooldown, s.
    pub roll: Option<(f32, f32)>,
    /// `C69F`: 0 instigator, 1 second entity, 2 the ped itself.
    pub target_source: u32,
    /// `CE65`: want flag bit 0x40.
    pub want_flag: bool,
    /// `4EC2`: record suppress time after a reaction, s.
    pub suppress: f32,
    /// `4694`: wants raised (names, lower case).
    pub wants: Vec<String>,
}

/// A ped type's reactions (`livingworld_entities_moodreactions`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MoodReactions {
    /// (result name, priority).
    pub results: Vec<(String, u32)>,
    pub prerequisites: Vec<Prerequisite>,
}

/// The stock mood tables (from setup data; a mod may replace any entry).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MoodTables {
    pub categories: BTreeMap<String, MoodCategory>,
    pub results: BTreeMap<String, MoodResult>,
    pub reactions: BTreeMap<String, MoodReactions>,
    /// Entity type -> parent type (`livingworld_entities`), for the type gates.
    pub entity_parents: BTreeMap<String, String>,
}

/// One mood event.
#[derive(Clone, Debug, PartialEq)]
pub struct MoodEvent {
    pub category: String,
    pub instigator: Option<u64>,
    pub second: Option<u64>,
    pub position: Vec3,
}

/// One mood record (112 bytes at `brain+1040`).
#[derive(Clone, Debug, PartialEq)]
pub struct MoodRecord {
    pub category: String,
    pub instigator: Option<u64>,
    pub second: Option<u64>,
    pub position: Vec3,
    /// `+84`, `+88`, `+92`, `+96`.
    pub magnitude: f32,
    pub count: u32,
    pub suppress: f32,
    pub age: f32,
    /// Result name -> seconds left.
    pub cooldowns: BTreeMap<String, f32>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct MoodStore {
    pub records: Vec<MoodRecord>,
}

impl MoodStore {
    /// Post an event with its category's magnitude. Returns false when it was dropped.
    /// `82E40FF0`: erase every record about `entity` (instigator or second).
    pub fn forget(&mut self, entity: u64) {
        self.records.retain(|r| r.instigator != Some(entity) && r.second != Some(entity));
    }
    pub fn post(&mut self, event: MoodEvent, magnitude: f32) -> bool {
        if let Some(r) = self.records.iter_mut().find(|r| r.category == event.category && r.instigator == event.instigator && r.second == event.second) {
            r.magnitude += magnitude;
            r.count += 1;
            r.age = 0.0;
            r.position = event.position;
            return true;
        }
        if self.records.len() >= MAX_RECORDS {
            if event.instigator.is_some() && event.second.is_some() {
                return false;
            }
            // Which record retail evicts (`sub_82E441C8`) is open: the oldest goes here.
            if let Some(i) = self.records.iter().enumerate().max_by(|a, b| a.1.age.total_cmp(&b.1.age)).map(|x| x.0) {
                self.records.remove(i);
            }
        }
        self.records.push(MoodRecord {
            category: event.category,
            instigator: event.instigator,
            second: event.second,
            position: event.position,
            magnitude,
            count: 1,
            suppress: 0.0,
            age: 0.0,
            cooldowns: BTreeMap::new(),
        });
        true
    }

    /// One tick: cooldowns and suppress timers run down, ages run up, expired records go.
    pub fn tick(&mut self, dt: f32, lifetime: &dyn Fn(&str) -> f32) {
        for r in &mut self.records {
            for c in r.cooldowns.values_mut() {
                *c -= dt;
            }
            r.cooldowns.retain(|_, c| *c > 0.0);
            r.age += dt;
            r.suppress = (r.suppress - dt).max(0.0);
        }
        self.records.retain(|r| r.age <= lifetime(&r.category));
    }
}

/// The presence query's result cap (`sub_82E3CA20`: 30 entities).
pub const PRESENCE_MAX_PEDS: usize = 30;

/// The presence events of one scan (`sub_82E3CA20`): the other live peds within
/// [`PRESENCE_RADIUS`] (at most [`PRESENCE_MAX_PEDS`], in the given order; retail's world query
/// order is open), then each player within it.
pub fn presence(ped: u64, at: Vec3, peds: &[(u64, Vec3)], players: &[(u64, Vec3)]) -> Vec<MoodEvent> {
    let near = |p: &Vec3| dist_sq(*p, at) < PRESENCE_RADIUS * PRESENCE_RADIUS;
    let event = |&(id, p): &(u64, Vec3)| MoodEvent { category: category::PRESENCE.into(), instigator: Some(id), second: Some(ped), position: p };
    peds.iter()
        .filter(|(id, p)| *id != ped && near(p))
        .take(PRESENCE_MAX_PEDS)
        .map(event)
        .chain(players.iter().filter(|(_, p)| near(p)).map(event))
        .collect()
}

/// The zombie broadcast (`sub_82E24B60` tail, gated by the living-world manager's vfunc +156 = zombie mode) [code]: one
/// event per skater at the skater's position, instigator = the skater, no second entity (`-1`), 1.0 / 1.0 sight
/// factors. Radius unlimited (category radius 0); each ped's post gate decides.
pub fn zombie_mode(players: &[(u64, Vec3)]) -> Vec<MoodEvent> {
    players.iter().map(|&(id, p)| MoodEvent { category: category::ZOMBIE_MODE.into(), instigator: Some(id), second: None, position: p }).collect()
}

fn dist_sq(a: Vec3, b: Vec3) -> f32 {
    (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)
}

/// What the producer needs to know about the world this tick.
pub struct MoodContext<'a> {
    /// The ped's id, entity type and position.
    pub ped: u64,
    pub ped_type: &'a str,
    pub position: Vec3,
    /// Entity type and position of an id (players are `skater`).
    pub entity: &'a dyn Fn(u64) -> Option<(String, Vec3)>,
    /// Zombie mode (prerequisite kind 9, inferred).
    pub zombie: bool,
    /// Whether an entity is busy (the ped component's vfunc +32, `sub_82E3BF70`; check 8 `D44E`).
    pub busy: &'a dyn Fn(u64) -> bool,
    /// The ped's own state for the prerequisites (`sub_82BFEA60`): kind 2, OwnPluginObject (`brain+3277` bit 0x01);
    /// kind 4, the ped has a chasee (chaser component `0x8269A940` vfunc +12).
    pub own_plugin_object: bool,
    pub chasing: bool,
}

/// A raised want.
#[derive(Clone, Debug, PartialEq)]
pub struct RaisedWant {
    pub want: String,
    pub target: u64,
    pub flag_40: bool,
}

/// The producer's decision this tick (for logs and mod events).
#[derive(Clone, Debug, PartialEq)]
pub struct MoodReaction {
    pub result: String,
    pub category: String,
    pub rolled: Option<u32>,
    pub passed: bool,
    pub wants: Vec<RaisedWant>,
}

impl MoodTables {
    fn is_a(&self, ty: &str, wanted: &str) -> bool {
        let mut t = Some(ty);
        let mut guard = 0;
        while let Some(x) = t {
            if x == wanted {
                return true;
            }
            guard += 1;
            if guard > 32 {
                return false;
            }
            t = self.entity_parents.get(x).map(String::as_str);
        }
        false
    }

    /// `sub_82E41060`: an event is posted only when the ped's reaction set's prerequisites for its
    /// category pass (e.g. presence: the instigator within 12 m), so a record never builds up
    /// from events the ped could not react to.
    pub fn accepts(&self, set: &str, event: &MoodEvent, ctx: &MoodContext) -> bool {
        let Some(reactions) = self.reactions.get(set) else { return true };
        let record = MoodRecord {
            category: event.category.clone(),
            instigator: event.instigator,
            second: event.second,
            position: event.position,
            magnitude: 0.0,
            count: 0,
            suppress: 0.0,
            age: 0.0,
            cooldowns: BTreeMap::new(),
        };
        self.prerequisites_pass(reactions, &record, ctx)
    }

    /// Prerequisites of a record's category (`sub_82BFED98`, AND of the items; each item `sub_82BFEA60` about the
    /// ped itself) [code]: kind 2 OwnPluginObject, 3 the squared distance from the ped to the record's position
    /// (`+48`), 4 the ped is chasing, 6 the second entity is the ped itself (`sub_82E41060`'s input), 9 zombie mode.
    /// Comparators: 0 the value, 1 its negation, 2 farther than the item value, 3 nearer; any other is false. Only
    /// kind 3 has a distance (0.0 otherwise) and only kind 3 squares the item value.
    /// NOT RETAIL YET: kinds 1 (`0x8269A888` vfunc +16), 5 (`0x8269A950` vfunc +8 with the position), 7 (the chasee's
    /// `0x8258E778` vfunc +12) and 8 (`brain+540` bit 0x80, the pending flag of want slot 10) are false (research b152).
    pub fn prerequisites_pass(&self, reactions: &MoodReactions, record: &MoodRecord, ctx: &MoodContext) -> bool {
        reactions.prerequisites.iter().filter(|p| p.category == record.category).all(|p| {
            let (value, item, distance) = match p.kind {
                2 => (ctx.own_plugin_object, p.value, 0.0),
                3 => (false, p.value * p.value, dist_sq(record.position, ctx.position)),
                4 => (ctx.chasing, p.value, 0.0),
                6 => (record.second == Some(ctx.ped), p.value, 0.0),
                9 => (ctx.zombie, p.value, 0.0),
                _ => (false, p.value, 0.0),
            };
            match p.comparator {
                0 => value,
                1 => !value,
                2 => item < distance,
                3 => distance < item,
                _ => false,
            }
        })
    }

    /// `sub_82E41B98`: pick, roll and raise. `outstanding(id)` counts the wants already raised
    /// about `id`; `pending(want)` says whether that want is still waiting to be addressed.
    pub fn produce(&self, store: &mut MoodStore, ctx: &MoodContext, hand_prop: bool, outstanding: &dyn Fn(u64) -> u32, pending: &dyn Fn(&str) -> bool, rng: &mut Rng) -> Option<MoodReaction> {
        let reactions = self.reactions.get(ctx.ped_type)?;
        let (record_index, name, result, target) = self.evaluate(store, reactions, ctx, hand_prop, outstanding, pending)?;
        let record = &mut store.records[record_index];
        let mut reaction = MoodReaction { result: name.clone(), category: record.category.clone(), rolled: None, passed: true, wants: Vec::new() };
        if let Some((p, cooldown)) = result.roll {
            // The cooldown starts only with a real roll (`|p|` above `0x82063A40`; a 0.0 chance passes unrolled).
            if p.abs() > 2.938_736e-39 {
                let n = rng.modulo(100) + 1;
                reaction.rolled = Some(n);
                reaction.passed = p * 100.0 >= n as f32;
                record.cooldowns.insert(name.clone(), cooldown);
            }
        }
        if !reaction.passed {
            return Some(reaction);
        }
        record.suppress = result.suppress;
        reaction.wants = result.wants.iter().map(|w| RaisedWant { want: w.clone(), target, flag_40: result.want_flag }).collect();
        Some(reaction)
    }

    /// `sub_82E41EB0`: the best (lowest priority) result over all records, checked in retail's
    /// order. Returns (record index, result name, result, target id).
    #[allow(clippy::type_complexity)]
    pub fn evaluate<'a>(
        &'a self,
        store: &MoodStore,
        reactions: &MoodReactions,
        ctx: &MoodContext,
        hand_prop: bool,
        outstanding: &dyn Fn(u64) -> u32,
        pending: &dyn Fn(&str) -> bool,
    ) -> Option<(usize, String, &'a MoodResult, u64)> {
        let mut best: Option<(u32, usize, String, &MoodResult, u64)> = None;
        for (ri, r) in store.records.iter().enumerate() {
            if r.suppress > 0.0 {
                continue;
            }
            for (name, priority) in &reactions.results {
                if best.as_ref().is_some_and(|b| *priority >= b.0) {
                    continue;
                }
                let Some(res) = self.results.get(name) else { continue };
                // 1: the target must exist.
                let target = match res.target_source {
                    0 => r.instigator,
                    1 => r.second,
                    _ => Some(ctx.ped),
                };
                let Some(target) = target else { continue };
                // 2: the category.
                if res.category.as_deref() != Some(r.category.as_str()) {
                    continue;
                }
                // 3: the instigator's type.
                if let Some(t) = &res.instigator_type {
                    let ok = r.instigator.and_then(|i| (ctx.entity)(i)).is_some_and(|(ty, _)| self.is_a(&ty, t));
                    if !ok {
                        continue;
                    }
                }
                // 4: magnitude; 5: Nth bump.
                if res.magnitude_at_least.is_some_and(|m| r.magnitude < m) || res.count_at_least.is_some_and(|n| r.count < n) {
                    continue;
                }
                // 6: the second entity's type.
                if let Some(t) = &res.second_type {
                    let ok = r.second.and_then(|i| (ctx.entity)(i)).is_some_and(|(ty, _)| self.is_a(&ty, t));
                    if !ok {
                        continue;
                    }
                }
                // 7: hand prop; 8: the target's busy state must equal the flag (`D44E` -> `9B44`;
                // the other target check is not decoded: false).
                if res.hand_prop.is_some_and(|h| h != hand_prop) || res.target_check_a.is_some_and(|v| v) || res.target_check_b.is_some_and(|v| v != (ctx.busy)(target)) {
                    continue;
                }
                // 9: outstanding reactions about the instigator.
                if let Some((lo, hi)) = res.outstanding {
                    let n = r.instigator.map_or(0, outstanding);
                    if n < lo || n > hi {
                        continue;
                    }
                }
                // 10: cooldown.
                if res.roll.is_some() && r.cooldowns.contains_key(name) {
                    continue;
                }
                // 11: prerequisites.
                if !self.prerequisites_pass(reactions, r, ctx) {
                    continue;
                }
                // 12: none of its wants still pending.
                if res.wants.iter().any(|w| pending(w)) {
                    continue;
                }
                best = Some((*priority, ri, name.clone(), res, target));
            }
        }
        best.map(|b| (b.1, b.2, b.3, b.4))
    }
}

#[cfg(test)]
#[path = "mood_tests.rs"]
mod tests;
