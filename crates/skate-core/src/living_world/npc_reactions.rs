//! NPC skater reactions to other skaters' wipeouts (reaction manager 8245A7E8; b166 / b168).
//! Retail rules and our order: docs/hails-additions/26e-living-world-skater-ai.md#npc-wipeout-reactions

use std::collections::BTreeMap;

use super::Vec3;

/// The retail values (mod-tunable).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReactionParams {
    /// The manager's enable (retail: off while `mgr+0x260` is set).
    pub enabled: bool,
    /// Broadcast radius, m (strict: distance < radius).
    pub radius: f32,
    /// A skater's score above this raises trick seen (`Hash_4BEEDCA4AD799796` = 50, strict signed compare in
    /// `8245EAA8`; b168).
    pub trick_threshold: i32,
}

impl Default for ReactionParams {
    fn default() -> Self {
        Self { enabled: true, radius: 15.0, trick_threshold: 50 }
    }
}

/// Retail's "by" when the sender has no model (`sub_8245E740`: 0x61).
pub const NO_MODEL: u32 = 97;

/// One skater this tick, in the host's stable order.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReactionSkater {
    /// Stable key (the order and the previous-state memory).
    pub key: u64,
    pub position: Vec3,
    /// The wipeout byte (Processed `+2468` bit 18).
    pub wiping: bool,
    /// What a listener's "by" names: 0 = the player, the pros 1-29, the AI skaters' models; [`NO_MODEL`] = none.
    pub model: u32,
    /// Speaks reaction lines (NPC skaters).
    pub listens: bool,
    /// The score the manager compares (`[P+0x3C]+0x88`, written by `82DA6630`: the larger of the last trick
    /// sequence's reward and the sequence in progress, truncated). 0 for NPCs.
    pub score: i32,
}

/// What one listener gets this tick.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Raised {
    /// `(by)` of the last slam / slam_b sender in range.
    pub slam: Option<u32>,
    pub slam_b: Option<u32>,
    /// `(by)` of the last sender in range scoring above the threshold (a level: every tick while it stays above).
    pub trick: Option<u32>,
    /// Its own wipeout (a level).
    pub crash: bool,
}

/// The previous tick's wipeout per skater (the per-skater +0x29C byte).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ReactionState {
    previous: BTreeMap<u64, bool>,
}

impl ReactionState {
    /// One tick: the raised reactions per listener key (listeners with nothing raised are left out). Skaters not in
    /// `skaters` are forgotten.
    pub fn step(&mut self, params: &ReactionParams, skaters: &[ReactionSkater]) -> BTreeMap<u64, Raised> {
        let mut out: BTreeMap<u64, Raised> = BTreeMap::new();
        let previous = std::mem::take(&mut self.previous);
        for s in skaters {
            self.previous.insert(s.key, s.wiping);
        }
        if !params.enabled {
            return out;
        }
        let r2 = params.radius * params.radius;
        let in_range = |a: &ReactionSkater, b: &ReactionSkater| (0..3).map(|i| (a.position[i] - b.position[i]).powi(2)).sum::<f32>() < r2;
        for sender in skaters.iter().filter(|s| s.score > params.trick_threshold) {
            for listener in skaters.iter().filter(|l| l.listens && l.key != sender.key && in_range(l, sender)) {
                out.entry(listener.key).or_default().trick = Some(sender.model);
            }
        }
        for sender in skaters {
            let was = previous.get(&sender.key).copied().unwrap_or(sender.wiping);
            let (started, ended) = (!was && sender.wiping, was && !sender.wiping);
            if !started && !ended {
                continue;
            }
            for listener in skaters.iter().filter(|l| l.listens && l.key != sender.key && in_range(l, sender)) {
                let entry = out.entry(listener.key).or_default();
                if started {
                    entry.slam = Some(sender.model);
                }
                if ended {
                    entry.slam_b = Some(sender.model);
                }
            }
        }
        for s in skaters.iter().filter(|s| s.listens && s.wiping) {
            out.entry(s.key).or_default().crash = true;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn skater(key: u64, x: f32, wiping: bool, model: u32, listens: bool) -> ReactionSkater {
        ReactionSkater { key, position: [x, 0.0, 0.0], wiping, model, listens, score: 0 }
    }

    #[test]
    fn a_score_above_50_is_trick_seen_within_15_m_every_tick() {
        let p = ReactionParams::default();
        let mut s = ReactionState::default();
        let player = |score| ReactionSkater { score, ..skater(0, 0.0, false, 0, false) };
        let near = skater(10, 10.0, false, 89, true);
        let far = skater(11, 16.0, false, 90, true);
        assert!(s.step(&p, &[player(50), near, far]).is_empty(), "50 is not above the threshold");
        let r = s.step(&p, &[player(51), near, far]);
        assert_eq!(r.get(&10).and_then(|x| x.trick), Some(0));
        assert_eq!(r.get(&11), None);
        assert_eq!(s.step(&p, &[player(51), near, far]).get(&10).and_then(|x| x.trick), Some(0), "a level");
    }

    #[test]
    fn a_wipeout_start_and_end_reach_skaters_within_15_m_for_one_tick() {
        let p = ReactionParams::default();
        let mut s = ReactionState::default();
        // Tick 1: nobody wipes out (and the first sight of a skater is no edge).
        let quiet = s.step(&p, &[skater(0, 0.0, false, 0, false), skater(10, 10.0, false, 89, true), skater(11, 15.0, false, 90, true)]);
        assert!(quiet.is_empty());
        // Tick 2: the player wipes out: the NPC 10 m away sees a slam by the player; the one at exactly 15 m does not.
        let start = s.step(&p, &[skater(0, 0.0, true, 0, false), skater(10, 10.0, false, 89, true), skater(11, 15.0, false, 90, true)]);
        assert_eq!(start.get(&10), Some(&Raised { slam: Some(0), slam_b: None, trick: None, crash: false }));
        assert_eq!(start.get(&11), None, "15 m is not inside the strict radius");
        // Tick 3: still wiping out: nothing new (the pulse lasted one tick).
        assert!(s.step(&p, &[skater(0, 0.0, true, 0, false), skater(10, 10.0, false, 89, true)]).is_empty());
        // Tick 4: the wipeout ends: slam_b.
        let end = s.step(&p, &[skater(0, 0.0, false, 0, false), skater(10, 10.0, false, 89, true)]);
        assert_eq!(end.get(&10), Some(&Raised { slam: None, slam_b: Some(0), trick: None, crash: false }));
    }

    #[test]
    fn an_npc_crash_is_a_level_and_the_last_sender_names_by() {
        let p = ReactionParams::default();
        let mut s = ReactionState::default();
        let calm = [skater(0, 0.0, false, 0, false), skater(10, 5.0, false, 89, true), skater(11, 8.0, false, 12, true), skater(12, 6.0, false, 91, true)];
        s.step(&p, &calm);
        // NPCs 10 and 11 wipe out on the same tick: 12 hears both, the later one in order (11, the pro 12) names it.
        let both = [skater(0, 0.0, false, 0, false), skater(10, 5.0, true, 89, true), skater(11, 8.0, true, 12, true), skater(12, 6.0, false, 91, true)];
        let r = s.step(&p, &both);
        assert_eq!(r.get(&12).and_then(|x| x.slam), Some(12));
        assert_eq!(r.get(&10), Some(&Raised { slam: Some(12), slam_b: None, trick: None, crash: true }), "own crash + 11's slam");
        // Next tick: still down: the crash level stays, no new slam.
        let r = s.step(&p, &both);
        assert_eq!(r.get(&10), Some(&Raised { slam: None, slam_b: None, trick: None, crash: true }));
        assert_eq!(r.get(&12), None);
        // Disabled (a mod): nothing.
        let off = ReactionParams { enabled: false, ..p };
        assert!(s.step(&off, &calm).is_empty());
    }
}
