//! Ped conversations (doc 26 "Ped conversations"): the conversation object a ped spawns with
//! SpawnConversationArea. Retail (TU3, evidence only; re-implemented;
//! `.local/research/peds/b23-ped-plugins-conversations.md`, `b24-conversation-object.md`, main
//! checked the vtable, the completion state and the spawn constants):
//! - a conversation is a waypoint plugin entity (ctor `82E1CEC0`, interface vtable `0x8232B868`)
//!   with up to 3 member slots (participant, in-position bit) and a state (`D+1172`);
//! - gathering (vf4) while it has room and the state is 0 or 1; a ped may join (vf8) when its
//!   entity type fills a free slot of the chosen row, or of any candidate row before one is chosen;
//! - 3 waypoints (`82E1D620`) on a 1.5 m circle 120 deg apart from a random start in [0, pi);
//!   a ped locks the nearest free one (vfunc 144 `82E1CBC0`);
//! - when every member signalled "in position" (vf44) it starts (`82E1EBC0`): fewer than 2
//!   members ends it, else a candidate row is picked at random, the per-row value list gives one
//!   value, state 2, the first member speaks;
//! - each turn lasts the speaker's 3.0 s ConversationSpeak timer, then vf48 advances (`82E1ECB0`):
//!   state + 1, the next occupied member speaks; state 7 is complete (vf36): 5 turns.
//! Speech and lifetime (`b43-conversation-speech-slots.md`, main spot-checked `82E3DAD8`):
//! - start and every advance store the turn's line id (`82E1EA98`: 2 -> 0 or 1, 3 -> 3, 4 -> 5,
//!   5 -> 6, 6 -> 7; `D+1168`) and re-roll the variant (`D+1156`, uniform [0, 2)) once per turn;
//! - ConversationSpeak Begin (`826A66B8` -> `82E3DAD8`) gives the speaker the speech value
//!   line id + 33 (`ped+2468`) with the variant and the row's value (`ped+2472` / `+2476`), and
//!   every listener the value 41;
//! - the first join starts gathering (state 1) with a 30 s timer (`82E1D380`); when it runs out
//!   the conversation starts with whoever is there (`82E1EB10`);
//! - a member leaving (`82E1D470`) aborts a conversation that is not complete (vf52 `82E1E998`);
//!   the last one leaving resets it, and a spawned area is then removed (`82E1BD08`).

use super::super::Vec3;

/// The speaker's speech value is the line id plus this (`82E3DAD8`).
pub const SPEAKER_SPEECH_BASE: i32 = 33;
/// Every listener's speech value (`82E3DAD8`); no audio event uses it.
pub const LISTENER_SPEECH: i32 = 41;

/// Member slots (`D+1040`, 3 inline).
pub const MAX_MEMBERS: usize = 3;
/// The state at which a conversation is complete (vf36).
pub const COMPLETE: u8 = 7;

/// Retail values (data-driven; world tuning later).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ConversationParams {
    /// Waypoint circle radius, m, and waypoint count (`82E1D620`).
    pub radius: f32,
    pub waypoints: usize,
    /// No new conversation within this distance of another, m (`0x8220E13C`).
    pub exclusion: f32,
    /// The area goes this far ahead of the starter, m (`0x821DBCEC`).
    pub ahead: f32,
    /// A speaker's turn, s (ConversationSpeak timer 30).
    pub turn_seconds: f32,
    /// The gather timer set by the first join, s (`D+1136`, `0x820D4924` 30.0).
    pub gather_seconds: f32,
    /// Each waypoint's random offset in x and z, m (`82E1D620`: U(-0.8, 0.8), `0x8208824C`).
    pub jitter: f32,
    /// A waypoint's navmesh point must lie within this of it in x and in z, m (`0x8209975C`).
    pub locate_tolerance: f32,
    /// Highest area cost a waypoint may stand on (`82926640`: max of the area byte's nibbles; pavement 1).
    pub max_area_cost: f32,
    /// Farthest a waypoint may be from the dropped centre, m (`0x8211F0F8` = 9.0 squared).
    pub max_distance: f32,
    /// The centre is dropped onto the world with a ray this far above and below it, m (`82E166C0`, `0x822F8B84`).
    pub drop_height: f32,
}

impl Default for ConversationParams {
    fn default() -> Self {
        Self { radius: 1.5, waypoints: 3, exclusion: 50.0, ahead: 3.0, turn_seconds: 3.0, gather_seconds: 30.0, jitter: 0.8, locate_tolerance: 0.5, max_area_cost: 1.0, max_distance: 3.0, drop_height: 3.0 }
    }
}

/// One conversation row (`livingworld_conversations`): the entity types of its participants
/// (`7819`) and its value list (`36F1`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ConversationRow {
    pub name: String,
    pub participants: Vec<String>,
    pub values: Vec<i32>,
}

/// A member slot.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Member {
    pub ped: u64,
    pub in_position: bool,
}

/// One conversation (host-owned plain data).
#[derive(Clone, Debug, PartialEq)]
pub struct Conversation {
    pub id: u64,
    pub center: Vec3,
    /// Waypoint positions and who holds each.
    pub waypoints: Vec<(Vec3, Option<u64>)>,
    pub members: Vec<Member>,
    /// Candidate rows (indices into the host's row list) and the chosen one.
    pub candidates: Vec<usize>,
    pub row: Option<usize>,
    pub value: Option<i32>,
    pub state: u8,
    pub speaker: usize,
    /// This turn's line id (`D+1168`) and variant (`D+1156`), rolled once per turn.
    pub line: Option<u8>,
    pub variant: u8,
    /// The gather timer (`D+1136`), s, and its start value.
    pub gather_timer: f32,
    pub gather_seconds: f32,
}

/// What a speaker says this turn (`82E3DAD8`): the speech value (line id + 33), the variant
/// (`ped+2472`) and the row's value (`ped+2476`). The last two have no known audio meaning yet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TurnSpeech {
    pub value: i32,
    pub variant: u8,
    pub list_value: i32,
}

/// What `leave` did (`82E1D470`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Left {
    /// Not a member.
    NotMember,
    /// Members remain (an unfinished conversation was aborted).
    Remaining,
    /// The last member left: the owner resets a placed area, removes a spawned one.
    Empty,
}

/// Uniform [0, 1) from the host's seeded generator.
pub type Rand<'a> = &'a mut dyn FnMut() -> f32;

/// The navmesh point and area byte under a waypoint candidate (retail's NavPower locate, any area), `None` off the mesh.
pub type Probe<'a> = &'a dyn Fn(Vec3) -> Option<(Vec3, u8)>;

/// `82926640`: an area byte's cost, the larger of its two nibbles (pavement 0x11 = 1, road 0xA1 = 10, 0xF1 = 15).
pub fn area_cost(area: u8) -> f32 {
    (area >> 4).max(area & 0xF) as f32
}

impl Conversation {
    /// `82E1D620`: the area at `center` (already dropped onto the world by the host; `None` = the drop missed: no
    /// waypoints) with its waypoints. Each waypoint (on the circle, jittered) must land on the navmesh within the
    /// tolerance in x and z, on an area of cost at most `max_area_cost` (so never a road), and within
    /// `max_distance` of the centre; one failure clears them all (`82E1DAC0`, no retry). Its height is the navmesh's.
    /// `probe` = `None`: a map without a navmesh keeps the plain circle (NOT RETAIL: retail always has one).
    pub fn new(id: u64, center: Option<Vec3>, candidates: Vec<usize>, params: &ConversationParams, probe: Option<Probe>, rand: Rand) -> Self {
        let start = rand() * std::f32::consts::PI;
        let step = std::f32::consts::TAU / params.waypoints.max(1) as f32;
        let mut waypoints = Vec::new();
        if let Some(c) = center {
            for i in 0..params.waypoints {
                let a = start + step * i as f32;
                let mut w = [c[0] + a.sin() * params.radius, c[1], c[2] + a.cos() * params.radius];
                if let Some(probe) = probe {
                    w[0] += (rand() * 2.0 - 1.0) * params.jitter;
                    w[2] += (rand() * 2.0 - 1.0) * params.jitter;
                    let Some((at, area)) = probe(w) else {
                        waypoints.clear();
                        break;
                    };
                    let near = (at[0] - w[0]).abs() <= params.locate_tolerance && (at[2] - w[2]).abs() <= params.locate_tolerance;
                    let d2 = (at[0] - c[0]).powi(2) + (at[1] - c[1]).powi(2) + (at[2] - c[2]).powi(2);
                    if !near || area_cost(area) - params.max_area_cost > 0.001 || d2 > params.max_distance * params.max_distance {
                        waypoints.clear();
                        break;
                    }
                    w = [w[0], at[1], w[2]];
                }
                waypoints.push((w, None));
            }
        }
        let center = center.unwrap_or_default();
        Self { id, center, waypoints, members: Vec::new(), candidates, row: None, value: None, state: 0, speaker: 0, line: None, variant: 0, gather_timer: 0.0, gather_seconds: params.gather_seconds }
    }

    /// vf4.
    pub fn is_gathering(&self) -> bool {
        self.members.len() < MAX_MEMBERS && self.state <= 1
    }

    /// vf8: `entity` fills a free slot of the chosen row, or of a candidate row before one is chosen.
    pub fn allowed_to_join(&self, entity: &str, member_types: &[String], rows: &[ConversationRow]) -> bool {
        let fits = |r: &ConversationRow| {
            let mut free = r.participants.clone();
            for t in member_types {
                if let Some(i) = free.iter().position(|p| p == t) {
                    free.remove(i);
                }
            }
            free.iter().any(|p| p == entity)
        };
        match self.row {
            Some(r) => rows.get(r).is_some_and(fits),
            None => self.candidates.iter().filter_map(|&r| rows.get(r)).any(fits),
        }
    }

    pub fn join(&mut self, ped: u64) -> bool {
        if !self.is_gathering() || self.members.iter().any(|m| m.ped == ped) {
            return false;
        }
        self.members.push(Member { ped, in_position: false });
        if self.state == 0 {
            self.state = 1;
            self.gather_timer = self.gather_seconds;
        }
        true
    }

    /// `82E1EB10`: the gather timer runs down; at 0 a gathering conversation starts with whoever
    /// is there (fewer than 2 members ends it). Returns true when it started or ended here.
    pub fn tick(&mut self, dt: f32, rows: &[ConversationRow], rand: Rand) -> bool {
        self.gather_timer = (self.gather_timer - dt).max(0.0);
        if self.state == 1 && self.gather_timer <= 0.0 {
            self.start(rows, rand);
            return true;
        }
        false
    }

    /// vfunc 144: the nearest free waypoint, locked for `ped`.
    pub fn lock_closest_waypoint(&mut self, ped: u64, at: Vec3) -> Option<Vec3> {
        if let Some(w) = self.waypoints.iter().find(|w| w.1 == Some(ped)) {
            return Some(w.0);
        }
        let d2 = |p: Vec3| (p[0] - at[0]).powi(2) + (p[1] - at[1]).powi(2) + (p[2] - at[2]).powi(2);
        let i = self.waypoints.iter().enumerate().filter(|w| w.1 .1.is_none()).min_by(|a, b| d2(a.1 .0).total_cmp(&d2(b.1 .0))).map(|w| w.0)?;
        self.waypoints[i].1 = Some(ped);
        Some(self.waypoints[i].0)
    }

    pub fn unlock_waypoint(&mut self, ped: u64) {
        for w in &mut self.waypoints {
            if w.1 == Some(ped) {
                w.1 = None;
            }
        }
    }

    pub fn waypoint_of(&self, ped: u64) -> Option<Vec3> {
        self.waypoints.iter().find(|w| w.1 == Some(ped)).map(|w| w.0)
    }

    /// vf44, then `82E1EBC0` when every member is in position.
    pub fn signal_in_position(&mut self, ped: u64, rows: &[ConversationRow], rand: Rand) {
        if let Some(m) = self.members.iter_mut().find(|m| m.ped == ped) {
            m.in_position = true;
        }
        if self.state <= 1 && self.members.iter().all(|m| m.in_position) {
            self.start(rows, rand);
        }
    }

    fn start(&mut self, rows: &[ConversationRow], rand: Rand) {
        if self.members.len() < 2 {
            self.abort();
            return;
        }
        if self.row.is_none() && !self.candidates.is_empty() {
            let i = ((rand() * self.candidates.len() as f32) as usize).min(self.candidates.len() - 1);
            self.row = Some(self.candidates[i]);
        }
        if self.value.is_none() {
            let list = self.row.and_then(|r| rows.get(r)).map(|r| r.values.as_slice()).unwrap_or(&[]);
            self.value = Some(if list.is_empty() { 6 } else { list[((rand() * list.len() as f32) as usize).min(list.len() - 1)] });
        }
        self.state = 2;
        self.speaker = 0;
        self.roll_turn(rand);
    }

    /// vf52 `82E1E998`: complete, no speaker.
    fn abort(&mut self) {
        self.state = COMPLETE;
        self.line = None;
    }

    /// Start / advance: store the line id and re-roll the variant (uniform [0, 2), `82E17478`).
    fn roll_turn(&mut self, rand: Rand) {
        self.line = Self::line_for(self.state, rand);
        self.variant = ((rand() * 2.0) as u8).min(1);
    }

    /// vf12: the speaking member.
    pub fn speaker(&self) -> Option<u64> {
        (self.state >= 2 && self.state < COMPLETE).then(|| self.members.get(self.speaker).map(|m| m.ped)).flatten()
    }

    /// vf48 / `82E1ECB0`: the turn passes.
    pub fn pass_turn(&mut self, rand: Rand) {
        if self.state < 2 || self.state >= COMPLETE {
            return;
        }
        self.state += 1;
        if !self.members.is_empty() {
            self.speaker = (self.speaker + 1) % self.members.len();
        }
        self.roll_turn(rand);
    }

    /// `82E3DAD8`: the speaker's speech this turn.
    pub fn turn_speech(&self) -> Option<TurnSpeech> {
        self.speaker()?;
        Some(TurnSpeech { value: i32::from(self.line?) + SPEAKER_SPEECH_BASE, variant: self.variant, list_value: self.value.unwrap_or(0) })
    }

    /// vf32 `82E1E800`: the members other than the speaker.
    pub fn listeners(&self) -> impl Iterator<Item = u64> + '_ {
        let speaker = self.speaker();
        self.members.iter().map(|m| m.ped).filter(move |&p| Some(p) != speaker)
    }

    /// vf36.
    pub fn is_complete(&self) -> bool {
        self.state >= COMPLETE
    }

    /// `82E1EA98`: the line id for a state (`rand` [0, 2) for state 2's 0 / 1).
    fn line_for(state: u8, rand: Rand) -> Option<u8> {
        Some(match state {
            2 => ((rand() * 2.0) as u8).min(1),
            3 => 3,
            4 => 5,
            5 => 6,
            6 => 7,
            _ => return None,
        })
    }

    /// `82E1D470`. Retail keeps the leaver's slot key (a re-join by the same entity type refills
    /// it); ours drops the member.
    pub fn leave(&mut self, ped: u64) -> Left {
        self.unlock_waypoint(ped);
        let Some(i) = self.members.iter().position(|m| m.ped == ped) else {
            return Left::NotMember;
        };
        self.members.remove(i);
        if self.speaker >= self.members.len() {
            self.speaker = 0;
        }
        if self.members.is_empty() {
            return Left::Empty;
        }
        if !self.is_complete() {
            self.abort();
        }
        Left::Remaining
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_conversation_gathers_starts_and_runs_five_turns() {
        let rows = vec![ConversationRow { name: "r".into(), participants: vec!["adult_male".into(), "teen_male".into()], values: vec![4] }];
        let mut seq = [0.0f32, 0.0, 0.0, 0.0].into_iter().cycle();
        let mut rand = || seq.next().unwrap();
        let mut c = Conversation::new(1, Some([0.0; 3]), vec![0], &ConversationParams::default(), None, &mut rand);
        assert_eq!(c.waypoints.len(), 3);
        assert!((c.waypoints[0].0[2] - 1.5).abs() < 1e-5, "start angle 0 -> +z");
        assert!(c.allowed_to_join("teen_male", &["adult_male".into()], &rows));
        assert!(!c.allowed_to_join("adult_male", &["adult_male".into()], &rows), "no free adult_male slot");
        assert!(c.join(10) && c.join(11));
        let a = c.lock_closest_waypoint(10, [0.0, 0.0, 5.0]).unwrap();
        assert_eq!(a, c.waypoints[0].0);
        assert_ne!(c.lock_closest_waypoint(11, [0.0, 0.0, 5.0]).unwrap(), a, "a locked waypoint is skipped");
        c.signal_in_position(10, &rows, &mut rand);
        assert_eq!(c.state, 1, "gathering: waits for every member");
        c.signal_in_position(11, &rows, &mut rand);
        assert_eq!((c.state, c.row, c.value, c.speaker()), (2, Some(0), Some(4), Some(10)));
        let mut speakers = Vec::new();
        let mut values = Vec::new();
        while !c.is_complete() {
            speakers.push(c.speaker().unwrap());
            let t = c.turn_speech().unwrap();
            assert_eq!(c.listeners().collect::<Vec<_>>(), vec![if speakers.last() == Some(&10) { 11 } else { 10 }]);
            assert_eq!((t.variant, t.list_value), (0, 4));
            values.push(t.value);
            c.pass_turn(&mut rand);
        }
        assert_eq!(speakers, vec![10, 11, 10, 11, 10]);
        assert_eq!(values, vec![33, 36, 38, 39, 40], "line id + 33 per state (rand 0 -> intro short)");
        assert_eq!(c.turn_speech(), None);
        assert!(!c.is_gathering());
    }

    #[test]
    fn the_line_and_variant_are_rolled_once_per_turn() {
        let rows = vec![ConversationRow { name: "r".into(), participants: vec!["a".into(), "b".into()], values: vec![8] }];
        let mut seq = [0.0f32, 0.0, 0.0, 0.9, 0.9].into_iter().chain(std::iter::repeat(0.0));
        let mut rand = || seq.next().unwrap();
        let mut c = Conversation::new(1, Some([0.0; 3]), vec![0], &ConversationParams::default(), None, &mut rand);
        c.join(1);
        c.join(2);
        c.signal_in_position(1, &rows, &mut rand);
        c.signal_in_position(2, &rows, &mut rand);
        let t = c.turn_speech().unwrap();
        assert_eq!((t.value, t.variant, t.list_value), (34, 1, 8), "intro question, variant 1");
        assert_eq!(c.turn_speech(), Some(t), "reading it again does not re-roll");
    }

    #[test]
    fn the_gather_timer_starts_with_whoever_is_there() {
        let rows = vec![ConversationRow { name: "r".into(), participants: vec!["a".into(), "b".into()], values: vec![] }];
        let mut rand = || 0.0f32;
        let mut c = Conversation::new(1, Some([0.0; 3]), vec![0], &ConversationParams::default(), None, &mut rand);
        c.join(1);
        c.join(2);
        assert_eq!((c.state, c.gather_timer), (1, 30.0));
        assert!(!c.tick(29.9, &rows, &mut rand));
        assert!(c.tick(0.2, &rows, &mut rand), "nobody signalled in position: starts at 0");
        assert_eq!((c.state, c.speaker(), c.value), (2, Some(1), Some(6)));
    }

    #[test]
    fn a_member_leaving_aborts_and_the_last_one_empties_it() {
        let rows = vec![ConversationRow { name: "r".into(), participants: vec!["a".into(), "b".into()], values: vec![1] }];
        let mut rand = || 0.0f32;
        let mut c = Conversation::new(1, Some([0.0; 3]), vec![0], &ConversationParams::default(), None, &mut rand);
        c.join(1);
        c.join(2);
        c.signal_in_position(1, &rows, &mut rand);
        c.signal_in_position(2, &rows, &mut rand);
        assert_eq!(c.leave(9), Left::NotMember);
        assert_eq!(c.leave(2), Left::Remaining);
        assert!(c.is_complete() && c.speaker().is_none(), "mid-talk leave aborts");
        assert_eq!(c.leave(1), Left::Empty);
    }

    #[test]
    fn a_lone_member_ends_the_conversation() {
        let mut rand = || 0.5f32;
        let mut c = Conversation::new(1, Some([0.0; 3]), vec![], &ConversationParams::default(), None, &mut rand);
        c.join(10);
        c.signal_in_position(10, &[], &mut rand);
        assert!(c.is_complete());
    }

    #[test]
    fn waypoints_keep_off_roads_all_or_nothing() {
        // `82E1D620` (b134): pavement 0x11 keeps all three at the navmesh height; one road (0xA1) or off-mesh
        // waypoint clears the set; a missed centre drop gives none.
        let p = ConversationParams::default();
        let mut half = || 0.5f32;
        let pavement = |w: Vec3| Some(([w[0], 0.25, w[2]], 0x11u8));
        let c = Conversation::new(1, Some([0.0; 3]), vec![], &p, Some(&pavement), &mut half);
        assert_eq!(c.waypoints.len(), 3);
        assert!(c.waypoints.iter().all(|(w, _)| w[1] == 0.25));
        let road_east = |w: Vec3| Some((w, if w[0] > 0.1 { 0xA1u8 } else { 0x11 }));
        assert!(Conversation::new(1, Some([0.0; 3]), vec![], &p, Some(&road_east), &mut half).waypoints.is_empty());
        let far = |w: Vec3| Some(([w[0] + 0.6, w[1], w[2]], 0x11u8));
        assert!(Conversation::new(1, Some([0.0; 3]), vec![], &p, Some(&far), &mut half).waypoints.is_empty(), "outside the 0.5 m tolerance");
        assert!(Conversation::new(1, None, vec![], &p, Some(&pavement), &mut half).waypoints.is_empty());
        assert_eq!((area_cost(0x11), area_cost(0xA1), area_cost(0xF1)), (1.0, 10.0, 15.0));
    }
}
