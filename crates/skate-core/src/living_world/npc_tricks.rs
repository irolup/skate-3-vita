//! Which trick an NPC skater does at a recorded trick slot (doc 26, "NPC skater trick choice").
//!
//! Retail (TU3, evidence only; re-implemented, nothing copied): the PathController's dispatcher
//! `sub_8246A2E0` runs on every start-trick node (event 1) the skater passes. Ollie and flip slots
//! (table820862A8 category 1 / 2) go to `sub_8246A080`; every other category (grinds, grabs,
//! manuals, slides, reverts, plants) does the recorded trick.
//! - A higher chain level (heelflip2 and up) takes no action: the base trick already running
//!   covers it.
//! - Behaviour mode (behaviour context +60): 0 = the recorded trick (`sub_82469FA8` walks the
//!   chain within 120 frames but only ever keeps the recorded id: it assigns each matched node's
//!   previous level, which is the id it already has); 1 = the default (`sub_824733A0` creates the
//!   ambient behaviour with it): when the gate passes, a weighted pick from the character's
//!   profile table, else the recorded trick; 2 = a scripted list (not ported yet); 3 and up = no
//!   trick.
//! - Gate `sub_82469C60`: walk forward from the slot summing the nodes' frames until 300; a start
//!   node first passes only when it is an ollie / flip whose previous level is the recorded trick;
//!   a landing (airborne seen, then a node without it) passes only when the next node is not a
//!   start node and more than 50 frames passed (the landing node's frames included); the end of
//!   the window or the line passes.
//! - Pick `sub_82469A28` -> `sub_8245FE58`: the nollie table (profile +8) when the recorded
//!   trick's name starts with `n` / `N` (`sub_82469048`), else the regular table (profile +0);
//!   `u = rand32 / 2^32`; walk the weights (normalised to sum 1 at load, `sub_824720C8`) and take
//!   the first entry with `u < sum`, else entry 0.
//!
//! Not retail yet / determinism choice: retail draws `rand32` from the skater's random source
//! (skater vfunc +36) in tick order. Here the draw is `derive(seed, [line, node, frame])`: the same
//! distribution, but a pure function of the NPC's seed and the slot, so the choice does not depend
//! on how many other draws came first and a client could check it. `sub_82469A28` also keeps the
//! recorded trick when its trick attribute record has no family entries; those records are not
//! decoded, every recorded ollie / flip is taken to have them [inferred].

use crate::scoring::catalog;

use super::replay::{node_events, node_flags, ReplayLine};
use super::rng;

/// Behaviour mode of the trick choice (retail behaviour context +60).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TrickMode {
    /// Mode 0: always the recorded trick.
    Recorded,
    /// Mode 1 (retail ambient default): re-pick ollie / flip slots from the profile when the
    /// gate passes.
    #[default]
    Profile,
    /// Mode 3 and up: no ollie / flip (other slots still do their recorded trick).
    None,
}

impl TrickMode {
    /// Stable id for mods and logs.
    pub fn name(self) -> &'static str {
        match self {
            Self::Recorded => "recorded",
            Self::Profile => "profile",
            Self::None => "none",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        [Self::Recorded, Self::Profile, Self::None].into_iter().find(|m| m.name() == name)
    }
}

/// The gate's frame windows ([code] `sub_82469C60`; a mod may change them).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TrickParams {
    /// Recorded 60 Hz frames after which the look-ahead stops and the gate passes (retail 300).
    pub gate_window: u32,
    /// A landing passes only after more than this many frames (retail 50).
    pub min_air_frames: u32,
}

impl TrickParams {
    pub const RETAIL: Self = Self { gate_window: 300, min_air_frames: 50 };
}

impl Default for TrickParams {
    fn default() -> Self {
        Self::RETAIL
    }
}

/// A character's trick tables (`ai_skater_profiles`: regular flips + ollie, nollie flips +
/// nollie), raw weights as on the disc; [`pick_weighted`] normalises.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TrickProfile {
    pub regular: Vec<(i16, f32)>,
    pub nollie: Vec<(i16, f32)>,
}

impl TrickProfile {
    /// The table for a recorded trick: nollie when its name starts with `n` / `N`.
    pub fn table_for(&self, recorded: i16) -> &[(i16, f32)] {
        if is_nollie(recorded) {
            &self.nollie
        } else {
            &self.regular
        }
    }
}

/// [code] `sub_82469048`: the trick's scorable name starts with `n` or `N`.
pub fn is_nollie(trick: i16) -> bool {
    usize::try_from(trick).ok().and_then(|i| catalog::IDENTIFIERS.get(i)).is_some_and(|t| t.0.starts_with(['n', 'N']))
}

/// What a trick slot's choice reads.
#[derive(Clone, Copy, Debug, Default)]
pub struct TrickContext<'a> {
    pub mode: TrickMode,
    pub params: TrickParams,
    /// The NPC's profile; `None` keeps recorded tricks.
    pub profile: Option<&'a TrickProfile>,
    /// The NPC's own seed (from its spawn record).
    pub seed: u64,
    /// The profile's slot-trick flag (`Hash_6DBAF7AD6CDE6A16`, profile byte +0x33): without it the AI never starts a
    /// grab, fingerflip or boneless (categories 3, 4, 12; `sub_8246A2E0`).
    pub slot_tricks: bool,
}

/// The outcome at a start-trick node.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrickChoice {
    /// Start this trick (`-1` = no trick).
    Start(i16),
    /// A higher chain level: keep the trick already running.
    Continue,
}

fn is_ollie_or_flip(trick: i16) -> bool {
    matches!(catalog::category(trick), Some(1 | 2))
}

/// [code] `sub_82469C60`: may the slot at `slot` (recorded trick `recorded`) be re-picked?
pub fn gate(line: &ReplayLine, slot: u32, recorded: i16, params: TrickParams) -> bool {
    let mut frames = 0u32;
    let mut airborne_seen = false;
    let mut i = slot as usize + 1;
    while i < line.nodes.len() && frames < params.gate_window {
        let n = &line.nodes[i];
        frames += u32::from(n.frames);
        let airborne = n.flags & node_flags::AIRBORNE != 0;
        if n.event == node_events::START_TRICK {
            let next = line.node_trick(n);
            return is_ollie_or_flip(next) && catalog::previous_level(next) == recorded;
        }
        if airborne_seen && !airborne {
            let next_starts = line.nodes.get(i + 1).is_some_and(|m| m.event == node_events::START_TRICK);
            return !next_starts && frames > params.min_air_frames;
        }
        airborne_seen |= airborne;
        i += 1;
    }
    true
}

/// [code] `sub_8245FE58` over a table normalised like `sub_824720C8`: the first entry with
/// `u < running sum`, else entry 0; `None` for an empty table or one without positive weight.
pub fn pick_weighted(table: &[(i16, f32)], u: f32) -> Option<i16> {
    let total: f32 = table.iter().map(|e| e.1.max(0.0)).sum();
    if total.is_nan() || total <= 0.0 {
        return None;
    }
    let mut sum = 0.0f32;
    for &(trick, w) in table {
        sum += w.max(0.0) / total;
        if u < sum {
            return Some(trick);
        }
    }
    Some(table[0].0)
}

/// The draw for a slot: `[0, 1)` from the NPC's seed, the line and node of the slot and the
/// cursor frame (see the module note).
pub fn slot_draw(seed: u64, line: &[u8; 16], node: u32, frame: u64) -> f32 {
    let id = u64::from_le_bytes(line[..8].try_into().unwrap_or_default()) ^ u64::from_le_bytes(line[8..].try_into().unwrap_or_default());
    rng::Rng::new(rng::derive(seed, &[0x5452_4943_4B, id, u64::from(node), frame])).unit()
}

/// The choice at the start-trick node `slot` of `line` ([code] `sub_8246A2E0` / `sub_8246A080`).
pub fn choose(line: &ReplayLine, slot: u32, frame: u64, ctx: &TrickContext) -> TrickChoice {
    let Some(node) = line.nodes.get(slot as usize) else { return TrickChoice::Start(-1) };
    let recorded = line.node_trick(node);
    if !is_ollie_or_flip(recorded) {
        // `sub_8246A2E0`: grabs, fingerflips and bonelesses (3, 4, 12) only with the profile's slot-trick flag.
        if !ctx.slot_tricks && matches!(catalog::category(recorded), Some(3 | 4 | 12)) {
            return TrickChoice::Start(-1);
        }
        return TrickChoice::Start(recorded);
    }
    if catalog::is_chain_level(recorded) {
        return TrickChoice::Continue;
    }
    if ctx.mode == TrickMode::None {
        return TrickChoice::Start(-1);
    }
    match (ctx.mode, ctx.profile) {
        (TrickMode::Profile, Some(profile)) if gate(line, slot, recorded, ctx.params) => {
            let u = slot_draw(ctx.seed, &line.id, slot, frame);
            TrickChoice::Start(pick_weighted(profile.table_for(recorded), u).unwrap_or(recorded))
        }
        _ => TrickChoice::Start(recorded),
    }
}

#[cfg(test)]
#[path = "npc_tricks_tests.rs"]
mod tests;
