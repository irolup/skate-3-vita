//! NPC skater reactions to other skaters' wipeouts (the reaction manager, research b166 / b168) [code]:
//! - each simulation tick (`sub_8245A7E8`, tick mask bit 0) every skater's reaction bits are cleared first
//!   (R+0x24 `clrlwi 7`, main-checked), then set from this tick's events, so a pulse lasts one tick;
//! - a skater's wipeout start (`sub_82772748`: H+0xD1 = !prev && cur of the wipeout byte, Processed `+2468` bit 18) is
//!   passed to every other skater within a strict 15.0 (`0x822F8F68` = 225.0 compared with the squared distance,
//!   `sub_8245E950`, main-checked) as the slam bit with "by" = the sender's model (`sub_8245E6C8`, `sub_8245E740`;
//!   0x61 = 97 when the sender has none); its wipeout end (H+0xD2) is passed the same way as slam_b (`sub_8245E848`);
//! - the skaters are visited in index order, so the last sender in range names "by";
//! - a skater's own crash bit is its wipeout byte itself, a level (`sub_827A1B78`, entry +148 bit 5).
//! Trick seen (score > 50, `Hash_4BEEDCA4AD799796`) waits for the score's writer (b168 open).
//!
//! Ours: the skater order is the host's stable key order (players first, then NPC skaters by id), and players
//! listen to nothing (only NPC skaters speak these lines). Multiplayer: a pure function of the skater list and the
//! previous tick's wipeout states; the host runs it and sends the raised reactions.

use std::collections::BTreeMap;

use super::Vec3;

/// The retail values (mod-tunable).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReactionParams {
    /// The manager's enable (retail: off while `mgr+0x260` is set).
    pub enabled: bool,
    /// Broadcast radius, m (strict: distance < radius).
    pub radius: f32,
}

impl Default for ReactionParams {
    fn default() -> Self {
        Self { enabled: true, radius: 15.0 }
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
}

/// What one listener gets this tick.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Raised {
    /// `(by)` of the last slam / slam_b sender in range.
    pub slam: Option<u32>,
    pub slam_b: Option<u32>,
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
        for sender in skaters {
            let was = previous.get(&sender.key).copied().unwrap_or(sender.wiping);
            let (started, ended) = (!was && sender.wiping, was && !sender.wiping);
            if !started && !ended {
                continue;
            }
            for listener in skaters.iter().filter(|l| l.listens && l.key != sender.key) {
                let d2: f32 = (0..3).map(|i| (listener.position[i] - sender.position[i]).powi(2)).sum();
                if d2 < r2 {
                    let entry = out.entry(listener.key).or_default();
                    if started {
                        entry.slam = Some(sender.model);
                    }
                    if ended {
                        entry.slam_b = Some(sender.model);
                    }
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
        ReactionSkater { key, position: [x, 0.0, 0.0], wiping, model, listens }
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
        assert_eq!(start.get(&10), Some(&Raised { slam: Some(0), slam_b: None, crash: false }));
        assert_eq!(start.get(&11), None, "15 m is not inside the strict radius");
        // Tick 3: still wiping out: nothing new (the pulse lasted one tick).
        assert!(s.step(&p, &[skater(0, 0.0, true, 0, false), skater(10, 10.0, false, 89, true)]).is_empty());
        // Tick 4: the wipeout ends: slam_b.
        let end = s.step(&p, &[skater(0, 0.0, false, 0, false), skater(10, 10.0, false, 89, true)]);
        assert_eq!(end.get(&10), Some(&Raised { slam: None, slam_b: Some(0), crash: false }));
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
        assert_eq!(r.get(&10), Some(&Raised { slam: Some(12), slam_b: None, crash: true }), "own crash + 11's slam");
        // Next tick: still down: the crash level stays, no new slam.
        let r = s.step(&p, &both);
        assert_eq!(r.get(&10), Some(&Raised { slam: None, slam_b: None, crash: true }));
        assert_eq!(r.get(&12), None);
        // Disabled (a mod): nothing.
        let off = ReactionParams { enabled: false, ..p };
        assert!(s.step(&off, &calm).is_empty());
    }
}
