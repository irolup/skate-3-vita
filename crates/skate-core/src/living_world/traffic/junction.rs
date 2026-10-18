//! Junction entry: may a car on its approach lane enter its chosen connector now?
//!
//! A port of retail's car junction query `sub_82E11E90` (callers: FollowingLane `sub_82C376E8`
//! and `sub_82C3B500`) with its two conflict scans `sub_82E11C78` (merging into the same exit
//! lane) and `sub_82E11980` (crossing flows), plus the lane leader lookup `sub_82E14EE0` [code].
//! The occupancy lists are retail's: each lane of a segment (`+104`, 16-byte list heads per lane)
//! and each connector (`+8` of its 248-byte record) holds its cars newest first.
//!
//! Results ([`Entry`], retail return 0-4, stored by FollowingLane as the junction state `+4392`
//! together with the stop distance):
//! - 0 [`Entry::Go`];
//! - 1 [`Entry::Signal`]: the approach's light is red or amber, or green without time to clear
//!   the line; a right turn (`to_end == from_end - 1`) ignores the light (turn on red);
//! - 2 [`Entry::Approach`]: faster than the connector's entry speed and still beyond the car's
//!   look-ahead; at an unsignalled junction (with the light check on) the entry speed is 0.1 m/s
//!   (`0x820641A8`), so cars come to a stop first (a stop sign);
//! - 3 [`Entry::Yield`]: another connector from the same lane is in use, or a conflicting flow
//!   holds the junction (merge into the same exit lane, crossing, oncoming for left turns);
//!   at an unsignalled junction cars still on their approach yield by distance to the line
//!   (closer first, equal distance: the lower id);
//! - 4 [`Entry::Blocked`]: no room on the exit lane, the car ahead on the connector is closer
//!   than the minimum gap, or the approach / exit segment is missing.
//! Validation [trace, proof1 + proof2 VEHJUNC, 89 changes]: straight on red / amber always 1,
//! right turns never 1, unsignalled right turns 2, the light skipped when the flag is off.

use std::collections::BTreeMap;

use super::cursor::Place;
use super::graph::{RoadNetwork, ENDS};
use super::signals::{Light, SignalClock};

/// Entry speed at an unsignalled junction when the light check is on [code `0x820641A8` = 0.1].
pub const STOP_SPEED: f32 = 0.1;

/// Stable vehicle identity for the occupancy lists and the tie-break (retail compares the mover
/// ids, `+12`, unsigned: the lower id goes first). The engine uses the living-world serial.
pub type VehicleKey = u32;

/// What the query needs to know about a car (retail mover interface slots).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VehicleSnapshot {
    pub id: VehicleKey,
    /// Mover `+16`: the car's length along its lane (its centre is the cursor point).
    pub length: f32,
    /// Mover `+92`: speed, m/s.
    pub speed: f32,
    pub place: Place,
    /// Mover `+68` (on a lane) or `+60` (on a connector): metres along it.
    pub distance: f32,
    /// Mover `+100` (`+3516`): look-ahead distance; FollowingLane runs the query once the stop
    /// line is within `look_ahead + speed x 1 s`.
    pub look_ahead: f32,
    /// Mover `+104` (`+3520`): the smallest gap to a car ahead on the connector.
    pub min_gap: f32,
    /// Mover `+96`: vehicle `+3424` bit 0x04 or `+4402` bit 0x02 (the skitch speed-cap add). A
    /// yield caused by such a car reports [`EntryInfo::blocker_flagged`] (FollowingLane then
    /// sets junction state 5, which the horn decider answers with horn 3).
    pub flagged: bool,
}

impl VehicleSnapshot {
    fn on_connector(&self) -> bool {
        matches!(self.place, Place::Connector { .. })
    }
}

/// Look up cars by key (the engine's vehicle table).
pub trait Vehicles {
    fn get(&self, id: VehicleKey) -> Option<&VehicleSnapshot>;
}

impl Vehicles for BTreeMap<VehicleKey, VehicleSnapshot> {
    fn get(&self, id: VehicleKey) -> Option<&VehicleSnapshot> {
        BTreeMap::get(self, &id)
    }
}

/// Who is on which lane and connector, newest first (retail list order). Deterministic.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Occupancy {
    lanes: BTreeMap<(usize, u8), Vec<VehicleKey>>,
    connectors: BTreeMap<usize, Vec<VehicleKey>>,
}

impl Occupancy {
    pub fn enter_lane(&mut self, segment: usize, lane: u8, id: VehicleKey) {
        let list = self.lanes.entry((segment, lane)).or_default();
        list.retain(|&v| v != id);
        list.insert(0, id);
    }
    pub fn leave_lane(&mut self, segment: usize, lane: u8, id: VehicleKey) {
        if let Some(list) = self.lanes.get_mut(&(segment, lane)) {
            list.retain(|&v| v != id);
        }
    }
    /// Put a car on a connector's list (retail inserts while the car is inside the junction;
    /// whether cars also reserve before they enter is V4 work, the scans handle both).
    pub fn enter_connector(&mut self, connector: usize, id: VehicleKey) {
        let list = self.connectors.entry(connector).or_default();
        list.retain(|&v| v != id);
        list.insert(0, id);
    }
    pub fn leave_connector(&mut self, connector: usize, id: VehicleKey) {
        if let Some(list) = self.connectors.get_mut(&connector) {
            list.retain(|&v| v != id);
        }
    }
    /// Remove a car everywhere (despawn).
    pub fn remove(&mut self, id: VehicleKey) {
        self.lanes.values_mut().for_each(|l| l.retain(|&v| v != id));
        self.connectors.values_mut().for_each(|l| l.retain(|&v| v != id));
    }
    pub fn lane(&self, segment: usize, lane: u8) -> &[VehicleKey] {
        self.lanes.get(&(segment, lane)).map(|v| v.as_slice()).unwrap_or(&[])
    }
    pub fn connector(&self, connector: usize) -> &[VehicleKey] {
        self.connectors.get(&connector).map(|v| v.as_slice()).unwrap_or(&[])
    }
    /// Cars on a lane (the default per-lane load for the connector choice).
    pub fn lane_load(&self, segment: usize, lane: u8) -> f32 {
        self.lane(segment, lane).len() as f32
    }

    /// `sub_82E14EE0`: the car `me` follows on a lane. On the lane: the next older entry (the
    /// car ahead), none when `me` is the front car. Not on the lane: the newest entry (the
    /// rearmost car, the one a car about to enter would follow).
    pub fn leader(&self, segment: usize, lane: u8, me: VehicleKey) -> Option<VehicleKey> {
        let list = self.lane(segment, lane);
        if list.last() == Some(&me) {
            return None;
        }
        match list.iter().position(|&v| v == me) {
            Some(i) => list.get(i + 1).copied(),
            None => list.first().copied(),
        }
    }
}

/// Retail junction query result (`sub_82E11E90` return value).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Entry {
    Go = 0,
    Signal = 1,
    Approach = 2,
    Yield = 3,
    Blocked = 4,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EntryInfo {
    pub entry: Entry,
    /// The conflict scans' output word (`r6`): set when the car that made us yield is flagged.
    pub blocker_flagged: bool,
}

/// Inputs of one query.
pub struct EntryQuery<'a> {
    pub net: &'a RoadNetwork,
    pub signals: &'a SignalClock,
    pub occupancy: &'a Occupancy,
    pub vehicles: &'a dyn Vehicles,
    /// The asking car, on its approach lane.
    pub me: &'a VehicleSnapshot,
    /// Its chosen connector.
    pub connector: usize,
    /// Consult the lights (retail `r7` = not `sub_82C344D0`: false while the player skitches
    /// the car, `+4403` bit 0x80, or for a car with `+4401` bit 0x10 faster than its driver's
    /// `Hash_F142ABBFBEDA71E2` km/h (taxi 40); bit 0x10 is never set in retail code read so far).
    pub check_lights: bool,
}

/// FollowingLane runs the query once the stop line is within `look_ahead + speed` metres.
pub fn query_due(me: &VehicleSnapshot, to_line: f32) -> bool {
    me.look_ahead + me.speed >= to_line
}

/// `sub_82E11E90`.
pub fn junction_entry(q: &EntryQuery) -> EntryInfo {
    let net = q.net;
    let me = q.me;
    let c = &net.connectors[q.connector];
    let j = &net.junctions[c.junction];
    let go = |entry| EntryInfo { entry, blocker_flagged: false };
    let f = c.from_end;
    let t = c.to_end;
    let end = |x: u8| (f + x) & 3;
    let approach = j.approaches[f as usize].as_ref().and_then(|e| e.segment);
    let exit = j.exits[t as usize].as_ref().and_then(|e| e.segment);
    let (Some(approach), Some(exit)) = (approach, exit) else { return go(Entry::Blocked) };
    // Distance from the car's front to the stop line.
    let remaining = net.segments[approach].length - (me.distance + me.length * 0.5);
    let mut entry_speed = c.entry_speed;
    if q.check_lights {
        if j.signalled {
            let phase = q.signals.controller_for_end(f).car();
            let reach = me.speed * phase.remaining;
            let clears = phase.light == Light::Green && reach >= remaining - me.length * 0.5;
            if !clears && t != end(3) {
                return go(Entry::Signal);
            }
        } else {
            entry_speed = STOP_SPEED;
        }
    }
    if me.speed > entry_speed && remaining >= me.look_ahead {
        return go(Entry::Approach);
    }
    // Room on the exit lane behind its rearmost car (predicted 1 s ahead).
    if let Some(&rear) = q.occupancy.lane(exit, c.to_lane).first() {
        if let Some(v) = q.vehicles.get(rear) {
            let tail = v.speed + v.distance - v.length * 0.5;
            if me.length > tail {
                return go(Entry::Blocked);
            }
        }
    }
    // Another connector from my lane in use.
    let approach_end = j.approaches[f as usize].as_ref().unwrap();
    let siblings = approach_end.lane_connectors.get(c.from_lane as usize).map(|v| v.as_slice()).unwrap_or(&[]);
    if siblings.iter().any(|&k| k != q.connector && !q.occupancy.connector(k).is_empty()) {
        return go(Entry::Yield);
    }
    // The newest car on a connector of my lane (in practice my own) too close ahead.
    for &k in siblings {
        let Some(&first) = q.occupancy.connector(k).first() else { continue };
        if first == me.id {
            continue;
        }
        let Some(v) = q.vehicles.get(first) else { continue };
        if !v.on_connector() {
            continue;
        }
        let gap = (v.distance - v.length * 0.5) + remaining;
        if me.min_gap > gap {
            return go(Entry::Blocked);
        }
    }
    let unsignalled = !j.signalled;
    let mut flagged = false;
    let mut merge = |a: u8, lane: u8| merge_conflict(q, a, t, lane, &mut flagged);
    let lanes_out = j.exits[t as usize].as_ref().map(|e| e.lanes).unwrap_or(0);
    let conflict = match c.turn() {
        super::graph::Turn::Left => {
            (0..=c.to_lane).rev().any(|l| merge(end(3), l) || merge(end(2), l)) || {
                let mut cross = |a, e, by_distance| cross_conflict(q, a, e, by_distance, remaining, &mut flagged);
                cross(end(1), end(3), unsignalled) || cross(end(1), end(2), unsignalled) || cross(end(3), f, unsignalled) || cross(end(2), f, true) || cross(end(2), end(3), true)
            }
        }
        super::graph::Turn::Right => (c.to_lane..lanes_out).any(|l| merge(end(1), l) || merge(end(2), l)),
        _ => {
            (c.to_lane..lanes_out).any(|l| merge(end(1), l)) || (0..=c.to_lane).rev().any(|l| merge(end(3), l)) || {
                let mut cross = |a, e, by_distance| cross_conflict(q, a, e, by_distance, remaining, &mut flagged);
                cross(end(2), end(3), true) || cross(end(1), end(3), unsignalled) || cross(end(3), end(1), unsignalled) || cross(end(3), f, unsignalled)
            }
        }
    };
    if conflict {
        EntryInfo { entry: Entry::Yield, blocker_flagged: flagged }
    } else {
        go(Entry::Go)
    }
}

/// Connectors of approach `a` (any lane) that end at exit `e`, optionally only into lane `lane`.
fn flow<'n>(net: &'n RoadNetwork, j: usize, a: u8, e: u8, lane: Option<u8>) -> impl Iterator<Item = usize> + 'n {
    let junction = &net.junctions[j];
    let ok = (a as usize) < ENDS && (e as usize) < ENDS && junction.exits[e as usize].is_some();
    let lists = if ok { junction.approaches[a as usize].as_ref().map(|x| x.lane_connectors.as_slice()).unwrap_or(&[]) } else { &[] };
    lists.iter().flatten().copied().filter(move |&k| {
        let c = &net.connectors[k];
        c.to_end == e && lane.is_none_or(|l| c.to_lane == l)
    })
}

/// `sub_82E11C78`: a car inside the junction on a connector from approach `a` into exit lane
/// (`e`, `lane`). Cars still on their approach do not count.
fn merge_conflict(q: &EntryQuery, a: u8, e: u8, lane: u8, flagged: &mut bool) -> bool {
    let c = &q.net.connectors[q.connector];
    let j = &q.net.junctions[c.junction];
    let lanes = j.exits.get(e as usize).and_then(|x| x.as_ref()).map(|x| x.lanes).unwrap_or(0);
    if j.approaches.get(a as usize).and_then(|x| x.as_ref()).is_none() || lane >= lanes {
        *flagged = false;
        return false;
    }
    let (mut conflict, mut out) = (false, 0u8);
    for k in flow(q.net, c.junction, a, e, Some(lane)) {
        for &id in q.occupancy.connector(k) {
            let Some(v) = q.vehicles.get(id) else { continue };
            if out == 2 {
                continue;
            }
            if v.on_connector() {
                conflict = true;
                out = 1;
            }
            if conflict && v.flagged {
                out = 2;
            }
        }
    }
    *flagged = out == 2;
    conflict
}

/// `sub_82E11980`: a car of the flow from approach `a` to exit `e` that has the junction. Cars
/// inside always do; cars still approaching only when `by_distance` (unsignalled junctions, and
/// the oncoming flow for left turns and straight on): closer to their line than we are, or as
/// close with a lower id.
fn cross_conflict(q: &EntryQuery, a: u8, e: u8, by_distance: bool, my_remaining: f32, flagged: &mut bool) -> bool {
    let c = &q.net.connectors[q.connector];
    let j = &q.net.junctions[c.junction];
    if j.approaches.get(a as usize).and_then(|x| x.as_ref()).is_none() || j.exits.get(e as usize).and_then(|x| x.as_ref()).is_none() {
        *flagged = false;
        return false;
    }
    let (mut conflict, mut out) = (false, 0u8);
    for k in flow(q.net, c.junction, a, e, None) {
        for &id in q.occupancy.connector(k) {
            let Some(v) = q.vehicles.get(id) else { continue };
            if out == 2 {
                continue;
            }
            let has_it = if v.on_connector() {
                true
            } else if by_distance {
                match v.place {
                    Place::Lane { segment, .. } if q.net.segments[segment].to_junction == Some(c.junction) => {
                        let theirs = q.net.segments[segment].length - v.distance - v.length * 0.5;
                        theirs < my_remaining || (theirs == my_remaining && q.me.id > v.id)
                    }
                    _ => false,
                }
            } else {
                false
            };
            if has_it {
                conflict = true;
                out = 1;
            }
            if conflict && v.flagged {
                out = 2;
            }
        }
    }
    *flagged = out == 2;
    conflict
}
