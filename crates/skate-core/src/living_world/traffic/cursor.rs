//! The lane cursor: where a car is on the road graph (a lane of a segment or a turn connector)
//! and how far along, advanced by distance across pieces, connectors and segments.
//!
//! Retail [code]: the mover interface (vehicle `+144`, vtable `0x82322240`) keeps the segment id
//! (`+72`, `ld +3992`), the distance along it (`+68`, `+3496`), the lane (`+84`), the chosen
//! connector index (`+76`, stored at vehicle `+4388`), the distance along the connector (`+60`,
//! `+3492`) and whether the car is on a lane (1) or inside the junction (0) (`+52`). FollowingLane
//! (`sub_82C376E8`) picks the next connector when it has none or the segment changed (mover
//! `+76 < 0` or `+72` != the lane's segment) and stores it at `+4388` before it looks at the
//! junction; the choice:
//! - with vehicle flag `+4401` bit 0x02 (set by the constructor `sub_82C3B7C8`, 0xCE, on every
//!   car and never cleared): the connector of the lane's list whose exit lane has the smallest
//!   `load / exit segment length` (exit segment `+136`, one vec4 per lane, x used; strict `<` so
//!   the first wins a tie) [code];
//! - without it (unused in retail): list entry `trunc(u32 x 100 / 2^32) mod n` from the world RNG
//!   (`sub_82970628`, `0x822F94B0` = 100 x 2^-32) [code].
//! What the per-lane load holds is a runtime value not read yet (open); the engine passes the
//! number of cars on the exit lane by default, a mod or the V4 driver may pass another measure.

use super::graph::{Frame, RoadNetwork};
use crate::living_world::rng::Rng;

/// Where the cursor is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Place {
    /// On lane `lane` of segment `segment` (indices into the network).
    Lane { segment: usize, lane: u8 },
    /// On a turn connector, inside the junction.
    Connector { connector: usize },
}

/// How a car picks the connector at the end of its lane.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ConnectorChoice {
    /// Retail on every car: least loaded exit lane per metre.
    #[default]
    LeastLoaded,
    /// Retail's other branch (flag clear; no car uses it): uniform from the world RNG.
    Random,
}

/// Pick the connector lane `lane` of `segment` takes at its junction (`sub_82C376E8`).
/// `load(segment, lane)` is the exit lane's load. Returns `None` at a dead end.
pub fn choose_connector(net: &RoadNetwork, segment: usize, lane: u8, choice: ConnectorChoice, load: &dyn Fn(usize, u8) -> f32, rng: &mut Rng) -> Option<usize> {
    let list = net.next_connectors(segment, lane);
    if list.is_empty() {
        return None;
    }
    match choice {
        ConnectorChoice::LeastLoaded => {
            let mut best = f32::MAX;
            let mut chosen = None;
            for &c in list {
                let Some(exit) = net.connector_exit(c) else { continue };
                let per_metre = 1.0f32 / net.segments[exit].length;
                let v = per_metre * load(exit, net.connectors[c].to_lane);
                if best > v {
                    best = v;
                    chosen = Some(c);
                }
            }
            chosen
        }
        ConnectorChoice::Random => {
            // u32 -> double -> single, times 100 / 2^32 in single, truncated (fctiwz).
            let roll = (rng.next_u32() as f64) as f32 * 2.328_306_4e-8_f32;
            let k = (roll as i32 as u32) % list.len() as u32;
            Some(list[k as usize])
        }
    }
}

/// What happened during one [`LaneCursor::advance`].
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Advance {
    /// Places entered, in order (a connector, then the exit lane, ...).
    pub entered: Vec<Place>,
    /// The cursor stopped at the end of a lane with nowhere to go (map edge or no connector).
    pub at_dead_end: bool,
}

/// A car's position on the graph.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LaneCursor {
    pub place: Place,
    /// Metres along the segment (on a lane) or the connector.
    pub distance: f32,
    /// The connector chosen for the end of the current lane (retail `+4388`); `None` at a dead
    /// end or while on a connector.
    pub next: Option<usize>,
    /// Lateral shift in lanes (a lane change blends from 0 to +-1, then [`Self::set_lane`]).
    pub lane_shift: f32,
}

impl LaneCursor {
    /// A cursor on a lane; the next connector is chosen at once, as retail does on entering.
    pub fn on_lane(net: &RoadNetwork, segment: usize, lane: u8, distance: f32, choose: &mut dyn FnMut(&RoadNetwork, usize, u8) -> Option<usize>) -> Self {
        let lane = lane.min(net.segments[segment].lanes - 1);
        let next = choose(net, segment, lane);
        LaneCursor { place: Place::Lane { segment, lane }, distance: distance.clamp(0.0, net.segments[segment].length), next, lane_shift: 0.0 }
    }

    /// The length of the current lane or connector.
    pub fn span(&self, net: &RoadNetwork) -> f32 {
        match self.place {
            Place::Lane { segment, .. } => net.segments[segment].length,
            Place::Connector { connector } => net.connectors[connector].length(),
        }
    }

    /// Metres left to the end of the current lane or connector (the stop line on a lane).
    pub fn remaining(&self, net: &RoadNetwork) -> f32 {
        (self.span(net) - self.distance).max(0.0)
    }

    pub fn frame(&self, net: &RoadNetwork) -> Frame {
        match self.place {
            Place::Lane { segment, lane } => net.lane_frame(segment, lane as f32 + self.lane_shift, self.distance),
            Place::Connector { connector } => net.connector_frame(connector, self.distance),
        }
    }

    /// Switch to another lane of the same segment at the same distance (end of a lane change);
    /// the next connector is chosen again for the new lane.
    pub fn set_lane(&mut self, net: &RoadNetwork, lane: u8, choose: &mut dyn FnMut(&RoadNetwork, usize, u8) -> Option<usize>) -> bool {
        let Place::Lane { segment, .. } = self.place else { return false };
        if lane >= net.segments[segment].lanes {
            return false;
        }
        self.place = Place::Lane { segment, lane };
        self.lane_shift = 0.0;
        self.next = choose(net, segment, lane);
        true
    }

    /// Move `ds` metres forward (negative moves are ignored), crossing pieces, onto the chosen
    /// connector at the end of a lane and onto the connector's exit lane at its end (where the
    /// next connector is chosen). The overshoot carries across every boundary, so the path is
    /// continuous; at a dead end the cursor stops on the lane end.
    pub fn advance(&mut self, net: &RoadNetwork, ds: f32, choose: &mut dyn FnMut(&RoadNetwork, usize, u8) -> Option<usize>) -> Advance {
        let mut out = Advance::default();
        if !(ds > 0.0) {
            return out;
        }
        self.distance += ds;
        // A bound on crossings per call keeps a corrupt graph (zero-length loops) from hanging.
        for _ in 0..64 {
            let span = self.span(net);
            if self.distance < span {
                break;
            }
            match self.place {
                Place::Lane { .. } => {
                    let Some(c) = self.next else {
                        self.distance = span;
                        out.at_dead_end = true;
                        break;
                    };
                    self.distance -= span;
                    self.place = Place::Connector { connector: c };
                    self.next = None;
                    self.lane_shift = 0.0;
                    out.entered.push(self.place);
                }
                Place::Connector { connector } => {
                    let Some(exit) = net.connector_exit(connector) else {
                        self.distance = span;
                        out.at_dead_end = true;
                        break;
                    };
                    self.distance -= span;
                    let lane = net.connectors[connector].to_lane.min(net.segments[exit].lanes - 1);
                    self.place = Place::Lane { segment: exit, lane };
                    self.next = choose(net, exit, lane);
                    out.entered.push(self.place);
                }
            }
        }
        out
    }
}
