//! Traffic signals: retail's 4 signal controllers, each with a car light programme and a walk
//! light programme, ticked by a fixed step.
//!
//! Retail [code, TU3; addresses are evidence only]:
//! - `sub_826B1540` (living-world init) builds 4 controllers (loop to 4 at `0x826B1860`), each a
//!   car phase list at `+4` and a walk phase list at `+148` (16-byte phases: light, length,
//!   remaining, word), filled by `sub_82E156D8`. Even controllers: all-red, red (= green +
//!   amber), all-red, green, amber; odd controllers: all-red, green, amber, all-red, red. So
//!   controllers 0 / 2 and 1 / 3 alternate. The walk list mirrors the car list, except that a
//!   green phase splits into walk green `(1 - split) x green` and walk amber `split x green`
//!   (split = `+300`, the `trafficlights` record's `Hash_5E41C959D17527CC`, 0.4) and every
//!   other phase is walk red. Jump targets: `+312` / `+316` = the index of the all-red before
//!   green, `+304` / `+308` = the all-red before red.
//! - `sub_82E158D8` ticks one controller by a fixed `1/60` s (`0x820849C8`): the current phase's
//!   remaining time drops; below 0 the next phase starts and inherits the overshoot, except
//!   when the car list wraps to phase 0 (then the overshoot is dropped and the walk list is
//!   forced back to its phase 0 too).
//! - `sub_826B2C18` (from the living-world update `sub_826BDB50`, world tick `sub_82859E70`)
//!   ticks all 4 every world tick unless the lights are frozen (`+13382`). The world tick is a
//!   fixed 60 Hz step [trace: 60 ticks per second in the recomp while it renders at ~345 fps],
//!   so the cycle is 17 s of game time at any frame rate.
//! - Which controller an approach obeys: the connector's `from_end` (connector `+84`) indexes
//!   the controllers (road network vtable `+36`, `sub_82E11E90`). Every signalled junction in a
//!   district shares the 4 controllers: approaches from node ends 0 and 2 run together, 1 and 3
//!   together, and the whole city switches in step.
//! - A priority vehicle (`sub_8269B328` sets it, `sub_8269B338` clears it and unfreezes; caller
//!   not found) gets a green wave: within 50 m (`0x8220E13C`) of a signalled junction, or inside
//!   one, the manager calls `sub_826B3C88(end)`: the controllers with the end's parity jump to
//!   their all-red-before-green, the others to their all-red-before-red, and once the end's
//!   controller shows green the lights freeze until the priority is cleared.
//!
//! Durations are data (`livingworld` record `trafficlights`: `signal_green` 7, `signal_amber`
//! 1, `signal_all_red` 0.5, split 0.4 [data]); [`SignalTimings`] takes them from the export or a
//! mod. The phase arithmetic stays in f32 like retail, so the phase changes land on the same
//! ticks.

use super::graph::{RoadNetwork, ENDS};

/// Controllers per district [code `sub_826B1540`, loop to 4].
pub const CONTROLLERS: usize = 4;
/// Seconds per signal tick [code `0x820849C8` = 0x3C888889].
pub const TICK_SECONDS: f32 = 1.0 / 60.0;
/// Signal ticks per second of game time: one per 60 Hz world step [trace].
pub const TICKS_PER_SECOND: f64 = 60.0;
/// Distance to a signalled junction at which a priority vehicle requests green [code
/// `0x8220E13C`, `sub_826B2C18`].
pub const PRIORITY_DISTANCE: f32 = 50.0;

/// Light shown by a phase: retail phase kinds 0 / 1 / 2.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Light {
    Red = 0,
    Amber = 1,
    Green = 2,
}

/// One phase (retail 16 bytes: kind, length, remaining, word).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Phase {
    pub light: Light,
    pub length: f32,
    pub remaining: f32,
    /// Retail word 3: the car light kind on car phases; 1 on the two walk phases cut from a
    /// green, 0 otherwise [code `sub_82E156D8`]; use open.
    pub word: u32,
}

/// Signal durations in seconds. Data values: built from the `trafficlights` record (or a mod's
/// override); no defaults are duplicated here.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SignalTimings {
    pub green: f32,
    pub amber: f32,
    pub all_red: f32,
    /// Share of a green that the walk light spends amber (`+300`).
    pub walk_split: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Controller {
    pub index: u8,
    pub car: Vec<Phase>,
    pub walk: Vec<Phase>,
    /// Current car / walk phase (`+292` / `+296`).
    pub car_phase: usize,
    pub walk_phase: usize,
    /// All-red before red (`+304` car, `+308` walk) and before green (`+312`, `+316`).
    pub to_red: (usize, usize),
    pub to_green: (usize, usize),
}

impl Controller {
    /// Build controller `index` the way `sub_826B1540` does.
    pub fn new(index: u8, t: &SignalTimings) -> Self {
        let mut c = Controller { index, car: Vec::new(), walk: Vec::new(), car_phase: 0, walk_phase: 0, to_red: (0, 0), to_green: (0, 0) };
        let red = t.green + t.amber; // vaddfp of the two record fields
        if index & 1 == 1 {
            c.to_green = (c.car.len(), c.walk.len());
            c.push(Light::Red, t.all_red, t.walk_split);
            c.push(Light::Green, t.green, t.walk_split);
            c.push(Light::Amber, t.amber, t.walk_split);
            c.to_red = (c.car.len(), c.walk.len());
            c.push(Light::Red, t.all_red, t.walk_split);
            c.push(Light::Red, red, t.walk_split);
        } else {
            c.to_red = (c.car.len(), c.walk.len());
            c.push(Light::Red, t.all_red, t.walk_split);
            c.push(Light::Red, red, t.walk_split);
            c.to_green = (c.car.len(), c.walk.len());
            c.push(Light::Red, t.all_red, t.walk_split);
            c.push(Light::Green, t.green, t.walk_split);
            c.push(Light::Amber, t.amber, t.walk_split);
        }
        c
    }

    /// `sub_82E156D8`: append a car phase and its walk phase(s).
    fn push(&mut self, light: Light, seconds: f32, split: f32) {
        self.car.push(Phase { light, length: seconds, remaining: seconds, word: light as u32 });
        if light == Light::Green {
            let green = (1.0f32 - split) * seconds;
            self.walk.push(Phase { light: Light::Green, length: green, remaining: green, word: 1 });
            let amber = seconds * split;
            self.walk.push(Phase { light: Light::Amber, length: amber, remaining: amber, word: 1 });
        } else {
            self.walk.push(Phase { light: Light::Red, length: seconds, remaining: seconds, word: 0 });
        }
    }

    /// `sub_82E158D8`: one fixed step. Returns true when a car or walk phase changed.
    pub fn tick(&mut self) -> bool {
        let mut wrapped = false;
        let mut changed = false;
        if !self.car.is_empty() {
            let old = self.car_phase;
            self.car[old].remaining -= TICK_SECONDS;
            if self.car[old].remaining < 0.0 {
                self.car_phase = (old + 1) % self.car.len();
                if self.car_phase == 0 {
                    self.car[old].remaining = 0.0;
                    wrapped = true;
                }
                let carry = self.car[old].remaining;
                self.car[self.car_phase].remaining += carry;
                self.car[old].remaining = self.car[old].length;
                changed = true;
            }
        }
        if !self.walk.is_empty() {
            let old = self.walk_phase;
            self.walk[old].remaining -= TICK_SECONDS;
            if self.walk[old].remaining < 0.0 || wrapped {
                if wrapped {
                    self.walk_phase = 0;
                    self.walk[old].remaining = 0.0;
                } else {
                    self.walk_phase = (old + 1) % self.walk.len();
                }
                let carry = self.walk[old].remaining;
                self.walk[self.walk_phase].remaining += carry;
                self.walk[old].remaining = self.walk[old].length;
                changed = true;
            }
        }
        changed
    }

    pub fn car(&self) -> &Phase {
        &self.car[self.car_phase]
    }
    pub fn walk(&self) -> &Phase {
        &self.walk[self.walk_phase]
    }

    /// `sub_826B3C88`'s jump: restart the current phase and go to a target.
    fn jump(&mut self, car: usize, walk: usize) {
        let cur = self.car_phase;
        self.car[cur].remaining = self.car[cur].length;
        let cur = self.walk_phase;
        self.walk[cur].remaining = self.walk[cur].length;
        self.car_phase = car.min(self.car.len().saturating_sub(1));
        self.walk_phase = walk.min(self.walk.len().saturating_sub(1));
    }
}

/// A phase change, for logs, audio or a network peer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SignalChange {
    /// Signal tick count after the change.
    pub tick: u64,
    pub controller: u8,
    pub car_phase: usize,
    pub car: Light,
    pub walk_phase: usize,
    pub walk: Light,
}

/// The 4 controllers plus the manager state (`sub_826B2C18`), stepped at a fixed 60 Hz from
/// any engine frame rate.
#[derive(Clone, Debug, PartialEq)]
pub struct SignalClock {
    pub timings: SignalTimings,
    pub controllers: [Controller; CONTROLLERS],
    /// Lights held (`+13382`).
    pub frozen: bool,
    /// A green request for an end parity is waiting for its controller to turn green
    /// (`+13380` even / `+13381` odd).
    pub pending_green: Option<u8>,
    /// Signal ticks per second of game time (default 60). A mod may slow or speed the city's
    /// lights; retail is 60.
    pub ticks_per_second: f64,
    /// At most this many ticks per `advance` call (a long hitch does not replay minutes of
    /// lights in one frame). Large by default: the lights are a clock, not a simulation.
    pub max_ticks_per_advance: u32,
    accumulator: f64,
    ticks: u64,
}

impl SignalClock {
    pub fn new(timings: SignalTimings) -> Self {
        Self {
            timings,
            controllers: std::array::from_fn(|i| Controller::new(i as u8, &timings)),
            frozen: false,
            pending_green: None,
            ticks_per_second: TICKS_PER_SECOND,
            max_ticks_per_advance: 600,
            accumulator: 0.0,
            ticks: 0,
        }
    }

    /// Signal ticks run so far.
    pub fn ticks(&self) -> u64 {
        self.ticks
    }

    /// One manager pass (`sub_826B2C18`, lights part): tick the 4 controllers unless frozen,
    /// then freeze when a pending green request's controller shows green.
    pub fn tick(&mut self, changes: &mut Vec<SignalChange>) {
        if self.frozen {
            return;
        }
        self.ticks += 1;
        for c in &mut self.controllers {
            if c.tick() {
                changes.push(SignalChange {
                    tick: self.ticks,
                    controller: c.index,
                    car_phase: c.car_phase,
                    car: c.car().light,
                    walk_phase: c.walk_phase,
                    walk: c.walk().light,
                });
            }
        }
        if let Some(parity) = self.pending_green {
            if self.controllers[parity as usize & 1].car().light == Light::Green {
                self.frozen = true;
            }
        }
    }

    /// Add `seconds` of game time; runs the whole ticks that are due and returns the phase
    /// changes. The same elapsed time gives the same ticks at any engine frame rate.
    pub fn advance(&mut self, seconds: f64) -> Vec<SignalChange> {
        let mut changes = Vec::new();
        if !seconds.is_finite() || seconds <= 0.0 {
            return changes;
        }
        let period = 1.0 / self.ticks_per_second.max(1.0);
        self.accumulator += seconds;
        let mut due = 0;
        // The epsilon keeps 2 x (1/120) == 1/60 from losing a tick to rounding.
        while self.accumulator + 1e-9 >= period && due < self.max_ticks_per_advance {
            self.accumulator -= period;
            due += 1;
            self.tick(&mut changes);
        }
        if due == self.max_ticks_per_advance && self.accumulator >= period {
            self.accumulator %= period;
        }
        self.accumulator = self.accumulator.max(0.0);
        changes
    }

    /// The controller an approach from node end `from_end` obeys [code `sub_82E11E90`].
    pub fn controller_for_end(&self, from_end: u8) -> &Controller {
        &self.controllers[from_end as usize % CONTROLLERS]
    }

    /// The car light a car on connector `connector` sees (None at an unsignalled junction).
    pub fn light_for(&self, net: &RoadNetwork, connector: usize) -> Option<&Phase> {
        let c = &net.connectors[connector];
        net.junctions[c.junction].signalled.then(|| self.controller_for_end(c.from_end).car())
    }

    /// The walk light for pedestrians crossing at node end `end` (peds' `WalkSignSaysGo`; the
    /// mapping of crossings to ends is ped work, open).
    pub fn walk_for_end(&self, end: u8) -> &Phase {
        self.controller_for_end(end).walk()
    }

    /// `sub_826B3C88(end)`: green wave for a priority vehicle approaching from node end `end`.
    pub fn request_green(&mut self, end: u8) {
        let parity = end & 1;
        let blocked = if self.frozen {
            // Already holding: keep holding only while this end's controller is green.
            self.controller_for_end(end).car().light == Light::Green
        } else {
            self.pending_green == Some(parity)
        };
        if blocked {
            return;
        }
        self.frozen = false;
        self.pending_green = None;
        if self.controller_for_end(end).car().light == Light::Green {
            self.frozen = true;
            return;
        }
        self.pending_green = Some(parity);
        for c in &mut self.controllers {
            if c.index & 1 == parity {
                let g = c.to_green.0;
                // Retail uses +312 for both lists here (+312 == +316 in both programmes).
                c.jump(g, g);
            } else {
                let (car, walk) = c.to_red;
                c.jump(car, walk);
            }
        }
    }

    /// `sub_8269B338`: clear the priority vehicle and release the lights.
    pub fn clear_priority(&mut self) {
        self.frozen = false;
        self.pending_green = None;
    }

    /// Rebuild the programmes (a mod changed the timings); keeps the tick count.
    pub fn set_timings(&mut self, timings: SignalTimings) {
        self.timings = timings;
        self.controllers = std::array::from_fn(|i| Controller::new(i as u8, &timings));
        self.frozen = false;
        self.pending_green = None;
    }
}

/// The manager's priority test (`sub_826B2C18`): the end whose controller a priority vehicle
/// should turn green, if it is on a lane within [`PRIORITY_DISTANCE`] of a signalled junction
/// (`segment length - distance <= 50`) or on a connector of one.
pub fn priority_end(net: &RoadNetwork, place: &super::cursor::Place, distance: f32, chosen_connector: Option<usize>) -> Option<u8> {
    use super::cursor::Place;
    let c = match *place {
        Place::Lane { segment, .. } => {
            let s = &net.segments[segment];
            if s.length - distance > PRIORITY_DISTANCE {
                return None;
            }
            chosen_connector?
        }
        Place::Connector { connector } => connector,
    };
    let con = &net.connectors[c];
    (net.junctions[con.junction].signalled && (con.from_end as usize) < ENDS).then_some(con.from_end)
}
