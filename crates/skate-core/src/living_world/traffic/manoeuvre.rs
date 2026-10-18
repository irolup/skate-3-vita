//! The traffic manoeuvre decider (retail TU3, evidence only; re-implemented;
//! `.local/research/npc/b69-traffic-manoeuvres.md` with the b71 to b74 extensions; main checked the overtake hash
//! before the first roll, the driver block at `[car+4116]` (schema offsets) and the bit roller):
//! - `sub_82C3D830` first: a pending pull-over is cancelled while a skater holds the car (unless the car has the
//!   "pulls over while held" trait), outside junction state 2: the spot is released, the timer reset;
//! - `sub_82C41CD0`, only while no manoeuvre is pending: (1) the overtake roll (`Hash_9366C67755A24D89`, 0 in every
//!   stock record, one draw every call); (2) the lane timer (`+3632`, + dt per tick) past the spec's period
//!   (`[spec+48]` = `Hash_90AB56A5DDCF2A3A`, 10 s, sports 5, truck 8) resets and rolls "go" (driver +28
//!   `Hash_52CF2CF3...`: 0.1, taxi 0.4, others 0) and "least loaded" (driver +24 `Hash_7C6B48BD...`); on a road that
//!   allows lane changes the adjacent lane (random, one draw, or the least loaded) must pass the gap check
//!   (`82E14928`); (3) when the timer fired and no lane change started: the pull-over roll (`Hash_559BA807...`, 0.02,
//!   taxi 0.04; 0 while held without the trait) on a road that allows it, on the outer lane, with a spot at least
//!   the approach length ahead (`82E14CF8`), which is reserved.
//!
//! The roll is retail's percent test (`8269A588`: `r % 100 + 1 <= chance x 100`, [`super::horn::percent_roll`]).
//! Multiplayer: [`DeciderState`] is plain per-car data; the host rolls with its seeded RNG.

/// Per-car values (spec and driver records; retail defaults from the setup data, a mod may override them).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DeciderParams {
    pub overtake_chance: f32,
    /// `[spec+48]`, s.
    pub lane_timer: f32,
    /// Driver +28 / +24.
    pub lane_change_chance: f32,
    pub least_loaded_chance: f32,
    pub pull_over_chance: f32,
    /// `+4402` bit 0x80, rolled at spawn (`82C42348`) with `held_pull_over_chance` (`Hash_99083122A1B7116A`, 0, taxi
    /// 0.2).
    pub pulls_over_while_held: bool,
    pub held_pull_over_chance: f32,
}

impl Default for DeciderParams {
    fn default() -> Self {
        Self { overtake_chance: 0.0, lane_timer: 10.0, lane_change_chance: 0.1, least_loaded_chance: 1.0, pull_over_chance: 0.02, pulls_over_while_held: false, held_pull_over_chance: 0.0 }
    }
}

/// What is pending (`+4396`: 1 lane change, 2 overtake, 3 pull-over).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Pending {
    #[default]
    None,
    LaneChange { lane: u8 },
    Overtake,
    PullOver { spot: f32 },
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DeciderState {
    pub timer: f32,
    pub pending: Pending,
}

/// The car's surroundings this tick, as the host sees them.
pub struct DeciderInput<'a> {
    pub dt: f32,
    pub lane: u8,
    pub lane_count: u8,
    /// Road flags (+64 bits 0x02 / 0x01; which segment field they are is open).
    pub lane_changes_allowed: bool,
    pub pull_over_allowed: bool,
    pub held: bool,
    /// Junction state 2 (`+4403` bit 0x20).
    pub at_junction: bool,
    /// Per-lane load for the least-loaded pick (retail `road+136` .x / road length; what it measures is open).
    pub lane_load: &'a dyn Fn(u8) -> f32,
    /// The gap check on the target lane (`82E14928`).
    pub gap_free: &'a dyn Fn(u8) -> bool,
    /// The stop spot search (`82E14CF8`) and the minimum distance ahead it must have (`d + [+3672]`).
    pub find_spot: &'a dyn Fn() -> Option<f32>,
    pub spot_min: f32,
}

/// What the host must do with the road's reservations.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SpotAction {
    Reserve(f32),
    Release,
}

/// `sub_82C3D830` + `sub_82C41CD0` for one tick. `rand` gives the next draw as a uniform [0, 1) value (the host's
/// seeded generator, like the horn rolls).
pub fn decide(state: &mut DeciderState, p: &DeciderParams, i: &DeciderInput, rand: &mut dyn FnMut() -> f32) -> Option<SpotAction> {
    use super::horn::percent_roll;
    if matches!(state.pending, Pending::PullOver { .. }) && !i.at_junction && i.held && !p.pulls_over_while_held {
        state.pending = Pending::None;
        state.timer = 0.0;
        return Some(SpotAction::Release);
    }
    if state.pending != Pending::None {
        return None;
    }
    if percent_roll(p.overtake_chance, rand()) {
        state.pending = Pending::Overtake;
        return None;
    }
    state.timer += i.dt;
    if !(state.timer > p.lane_timer) {
        return None;
    }
    state.timer = 0.0;
    let go = percent_roll(p.lane_change_chance, rand());
    let least = percent_roll(p.least_loaded_chance, rand());
    if go && i.lane_changes_allowed {
        let mut candidates = Vec::with_capacity(2);
        if i.lane + 1 < i.lane_count {
            candidates.push(i.lane + 1);
        }
        if i.lane > 0 {
            candidates.push(i.lane - 1);
        }
        let target = if candidates.is_empty() {
            i.lane
        } else if !least {
            candidates[((rand() * candidates.len() as f32) as usize).min(candidates.len() - 1)]
        } else {
            let own = (i.lane_load)(i.lane);
            candidates.iter().copied().filter(|c| (i.lane_load)(*c) < own).min_by(|a, b| (i.lane_load)(*a).total_cmp(&(i.lane_load)(*b))).unwrap_or(i.lane)
        };
        if target != i.lane && (i.gap_free)(target) {
            state.pending = Pending::LaneChange { lane: target };
            return None;
        }
    }
    let scale = if i.held && !p.pulls_over_while_held { 0.0 } else { 1.0 };
    if !percent_roll(scale * p.pull_over_chance, rand()) {
        return None;
    }
    if !i.pull_over_allowed || i.lane + 1 != i.lane_count {
        return None;
    }
    let spot = (i.find_spot)().filter(|s| *s >= i.spot_min)?;
    state.pending = Pending::PullOver { spot };
    Some(SpotAction::Reserve(spot))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input<'a>(load: &'a dyn Fn(u8) -> f32, gap: &'a dyn Fn(u8) -> bool, spot: &'a dyn Fn() -> Option<f32>) -> DeciderInput<'a> {
        DeciderInput { dt: 1.0, lane: 0, lane_count: 2, lane_changes_allowed: true, pull_over_allowed: true, held: false, at_junction: false, lane_load: load, gap_free: gap, find_spot: spot, spot_min: 10.0 }
    }

    #[test]
    fn the_lane_timer_gates_the_rolls_and_a_free_gap_starts_a_lane_change() {
        let (load, gap, spot) = (|_: u8| 0.0, |_: u8| true, || None);
        let i = input(&load, &gap, &spot);
        let p = DeciderParams { lane_change_chance: 1.0, least_loaded_chance: 0.0, ..Default::default() };
        let mut s = DeciderState::default();
        let draws = std::cell::Cell::new(0);
        let mut rand = || {
            draws.set(draws.get() + 1);
            0.0
        };
        for _ in 0..10 {
            decide(&mut s, &p, &i, &mut rand);
        }
        // Ten ticks of 1 s: the timer is at 10, not past it; one overtake draw per tick.
        assert_eq!((s.pending, draws.get()), (Pending::None, 10));
        decide(&mut s, &p, &i, &mut rand);
        assert_eq!(s.pending, Pending::LaneChange { lane: 1 });
        assert_eq!(s.timer, 0.0);
        // overtake + go + least + the random adjacent pick.
        assert_eq!(draws.get(), 14);
    }

    #[test]
    fn the_outer_lane_pulls_over_into_a_reserved_spot_and_a_grab_cancels_it() {
        let (load, gap, spot) = (|_: u8| 0.0, |_: u8| false, || Some(40.0));
        let mut i = input(&load, &gap, &spot);
        i.lane = 1;
        i.dt = 11.0;
        let p = DeciderParams { lane_change_chance: 0.0, pull_over_chance: 1.0, ..Default::default() };
        let mut s = DeciderState::default();
        let mut rand = || 0.0;
        assert_eq!(decide(&mut s, &p, &i, &mut rand), Some(SpotAction::Reserve(40.0)));
        assert_eq!(s.pending, Pending::PullOver { spot: 40.0 });
        i.held = true;
        assert_eq!(decide(&mut s, &p, &i, &mut rand), Some(SpotAction::Release));
        assert_eq!(s, DeciderState::default());
        // Held without the trait: the roll is 0, no pull-over.
        assert_eq!(decide(&mut s, &p, &i, &mut rand), None);
        // The inner lane never pulls over.
        i.held = false;
        i.lane = 0;
        assert_eq!(decide(&mut s, &p, &i, &mut rand), None);
    }

    #[test]
    fn stock_overtake_never_fires_but_draws() {
        let (load, gap, spot) = (|_: u8| 0.0, |_: u8| true, || None);
        let mut i = input(&load, &gap, &spot);
        i.dt = 0.0;
        let mut s = DeciderState::default();
        let draws = std::cell::Cell::new(0);
        let mut rand = || {
            draws.set(draws.get() + 1);
            0.0
        };
        decide(&mut s, &DeciderParams::default(), &i, &mut rand);
        assert_eq!((s.pending, draws.get()), (Pending::None, 1));
    }
}

/// The car's manoeuvre state (Vehicle.xml states, b74): following its lane (lane changes included, see
/// `Car::passage`), pulling over to its reserved spot, parked there, pulling out again.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Manoeuvre {
    #[default]
    Following,
    PullingOver { spot: f32 },
    Parked { spot: f32, time: f32 },
    PullingOut,
}

impl Manoeuvre {
    /// The reserved spot (from the decider's reservation until the pull-out starts, `82E15368`).
    pub fn spot(&self) -> Option<f32> {
        match *self {
            Manoeuvre::PullingOver { spot } | Manoeuvre::Parked { spot, .. } => Some(spot),
            _ => None,
        }
    }
}

/// FollowingLane's spot term (`82C376E8`, b77): the stop distance x = spot - d + slack (0.1, `0x820641A8`) when the
/// spot is within speed + the approach length; the planner then brakes to stop `approach` short of x (kind 2).
pub fn pull_over_stop(spot: f32, d: f32, speed: f32, approach: f32, slack: f32) -> Option<f32> {
    let x = spot - d + slack;
    (x > 0.0 && x <= speed + approach).then_some(x)
}

/// `IsRequiredToPullOver` (`82C3A270`, b77): the spot ahead within the approach length and the car below
/// `max_speed` (1.0 m/s, `0x8231A844`).
pub fn is_required_to_pull_over(spot: f32, d: f32, speed: f32, approach: f32, max_speed: f32) -> bool {
    spot >= 0.0 && spot > d && spot - d <= approach && speed < max_speed
}

#[cfg(test)]
mod pull_over_tests {
    use super::*;

    #[test]
    fn the_spot_becomes_a_stop_target_and_the_pull_over_needs_a_crawl() {
        assert_eq!(pull_over_stop(50.0, 30.0, 10.0, 12.0, 0.1), Some(20.1));
        assert_eq!(pull_over_stop(50.0, 20.0, 10.0, 12.0, 0.1), None);
        assert!(is_required_to_pull_over(50.0, 40.0, 0.5, 12.0, 1.0));
        assert!(!is_required_to_pull_over(50.0, 40.0, 1.5, 12.0, 1.0));
        assert!(!is_required_to_pull_over(50.0, 30.0, 0.5, 12.0, 1.0));
    }
}
