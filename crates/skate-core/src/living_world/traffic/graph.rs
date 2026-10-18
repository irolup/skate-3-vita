//! The road graph the traffic runs on: directed segments with ~4 m lane pieces, junctions with
//! approach / exit ends and turn connectors (retail RW object `0x00EB0013`, decoded in milestone V0,
//! `docs/hails-additions/living-world/vehicles-data.md`).
//!
//! Built from plain input records ([`RoadInput`]) that the data layer fills from the user's export
//! (`skate-data::roads::RoadGraph::traffic_input`) or a mod fills from its own map; no I/O here.
//! Every element keeps its retail identity ([`SegmentId`], [`JunctionId`], [`ConnectorId`],
//! [`LaneId`]) and the network stores them sorted by that identity, so the dense indices are the
//! same on every machine that loads the same roads (multiplayer and replay safe).
//!
//! Lane geometry [data, all 138 shipped connectors]: a piece's centre curve is the middle of the
//! road; lane `k` of `n` sits at `(k + 0.5) / n` of the way from the left edge to the right edge
//! (lanes 4 m apart: -2 / +2 m on the two-lane roads, 0 on one-lane roads), and every connector
//! starts exactly on its approach lane at the last piece's end and ends exactly on its exit lane at
//! the first piece's start. Lane 0 is the left lane: right turns leave from the right lane.

use std::collections::BTreeMap;
use std::fmt;

use crate::living_world::Vec3;

/// Arc-length samples per curve (`roads.bin` v2, retail table of 16 floats) [data].
pub const ARC_SAMPLES: usize = 16;
/// Node ends per junction (retail end records 0-3 approaches, 4-7 exits) [data].
pub const ENDS: usize = 4;

#[derive(Clone, Debug, PartialEq)]
pub struct TrafficError(pub String);

impl fmt::Display for TrafficError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "living-world traffic: {}", self.0)
    }
}
impl std::error::Error for TrafficError {}

/// Retail 64-bit segment id (one per travel direction).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SegmentId(pub u64);

/// Retail 64-bit road node id (one junction per node).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct JunctionId(pub u64);

/// A turn connector: its junction and its retail index within the junction (the lane lists name
/// connectors by that index).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ConnectorId {
    pub junction: JunctionId,
    pub index: u32,
}

/// One lane of a segment (lane 0 = the left lane).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LaneId {
    pub segment: SegmentId,
    pub lane: u8,
}

/// A cubic Hermite curve with its arc-length table (`arc[k]` = length from the start to parameter
/// `k / 15`): a lane piece's centre line or a turn connector [data].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Curve {
    pub start: Vec3,
    pub end: Vec3,
    pub tangent_start: Vec3,
    pub tangent_end: Vec3,
    pub length: f32,
    pub arc: [f32; ARC_SAMPLES],
}

impl Curve {
    /// A straight curve (synthetic roads, mods without curve data): tangents = the chord, arc
    /// table linear.
    pub fn straight(start: Vec3, end: Vec3) -> Self {
        let d: Vec3 = std::array::from_fn(|i| end[i] - start[i]);
        let length = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        Curve { start, end, tangent_start: d, tangent_end: d, length, arc: std::array::from_fn(|k| length * k as f32 / (ARC_SAMPLES - 1) as f32) }
    }

    pub fn point(&self, t: f32) -> Vec3 {
        let (t2, t3) = (t * t, t * t * t);
        let h00 = 2.0 * t3 - 3.0 * t2 + 1.0;
        let h10 = t3 - 2.0 * t2 + t;
        let h01 = -2.0 * t3 + 3.0 * t2;
        let h11 = t3 - t2;
        std::array::from_fn(|i| h00 * self.start[i] + h10 * self.tangent_start[i] + h01 * self.end[i] + h11 * self.tangent_end[i])
    }

    /// d(point)/dt.
    pub fn derivative(&self, t: f32) -> Vec3 {
        let t2 = t * t;
        let d00 = 6.0 * t2 - 6.0 * t;
        let d10 = 3.0 * t2 - 4.0 * t + 1.0;
        let d01 = -6.0 * t2 + 6.0 * t;
        let d11 = 3.0 * t2 - 2.0 * t;
        std::array::from_fn(|i| d00 * self.start[i] + d10 * self.tangent_start[i] + d01 * self.end[i] + d11 * self.tangent_end[i])
    }

    /// Parameter at arc length `s` (clamped), interpolated in the arc table.
    pub fn parameter_at(&self, s: f32) -> f32 {
        let s = s.clamp(0.0, self.length);
        for k in 1..ARC_SAMPLES {
            if s <= self.arc[k] {
                let span = (self.arc[k] - self.arc[k - 1]).max(1e-6);
                return ((k - 1) as f32 + (s - self.arc[k - 1]) / span) / (ARC_SAMPLES - 1) as f32;
            }
        }
        1.0
    }
}

// ---------------------------------------------------------------------------------------------
// Input records (what the data layer or a mod hands in)

#[derive(Clone, Debug, PartialEq)]
pub struct RoadInput {
    pub segments: Vec<SegmentInput>,
    pub junctions: Vec<JunctionInput>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SegmentInput {
    pub id: SegmentId,
    /// Origin node and its end (lanes start here).
    pub from_node: u64,
    pub from_end: u8,
    /// Destination node and its end (lanes run towards it) [data: lanes run node_b -> node_a].
    pub to_node: u64,
    pub to_end: u8,
    pub length: f32,
    /// m/s [data].
    pub speed_limit: f32,
    pub lanes: u8,
    /// Retail road +64 (the disc segment's `word_56`, copied by `82E14158`; b76): bit 0x02 lane changes allowed,
    /// bit 0x01 pull-over allowed (and an extra kerb lane slot).
    pub manoeuvres: u32,
    /// Index into the caller's district list (informational).
    pub district: u32,
    /// In order along the segment.
    pub pieces: Vec<PieceInput>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PieceInput {
    /// Distance along the segment at the piece end (m).
    pub end_distance: f32,
    pub centre: Curve,
    pub left_start: Vec3,
    pub right_start: Vec3,
    pub left_end: Vec3,
    pub right_end: Vec3,
}

#[derive(Clone, Debug, PartialEq)]
pub struct JunctionInput {
    pub id: JunctionId,
    /// Retail `flag_04` (junction header `+0x244`): the junction has traffic lights. The car
    /// junction query consults the signal only when it is set (`[[J+4]+580] != 0` in
    /// `sub_82E11E90`, the manager's priority check in `sub_826B2C18`) [code].
    pub signalled: bool,
    /// m/s, the road speed at the junction [data].
    pub speed: f32,
    /// End records 0-3 (approaches) and 4-7 (exits); `None` = no road at that end.
    pub approaches: [Option<EndInput>; ENDS],
    pub exits: [Option<EndInput>; ENDS],
    pub connectors: Vec<ConnectorInput>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EndInput {
    pub lanes: u8,
    /// Per lane: the retail indices of the connectors that lane may take (approach) or that
    /// arrive in it (exit), in record order.
    pub lane_connectors: Vec<Vec<u32>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ConnectorInput {
    pub index: u32,
    /// Retail connector `+0x50` (`f32_50` in V0): the speed a car must be at or below before it
    /// enters (the query returns "approach" while faster and still beyond its look-ahead,
    /// `sub_82E11E90` compares `+80` with the speed) [code]. The road speed on straight
    /// connectors, 1.4-2.7 m/s on turns [data].
    pub entry_speed: f32,
    pub from_end: u8,
    pub from_lane: u8,
    pub to_end: u8,
    pub to_lane: u8,
    pub curve: Curve,
}

// ---------------------------------------------------------------------------------------------
// The network

#[derive(Clone, Debug, PartialEq)]
pub struct Piece {
    pub start_distance: f32,
    pub end_distance: f32,
    pub centre: Curve,
    pub left_start: Vec3,
    pub right_start: Vec3,
    pub left_end: Vec3,
    pub right_end: Vec3,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Segment {
    pub id: SegmentId,
    pub from_node: u64,
    pub from_end: u8,
    pub to_node: u64,
    pub to_end: u8,
    pub length: f32,
    pub speed_limit: f32,
    pub lanes: u8,
    /// See [`SegmentInput::manoeuvres`].
    pub manoeuvres: u32,
    pub district: u32,
    pub pieces: Vec<Piece>,
    /// Horizontal bounds of the road edges: min x, min z, max x, max z (engine helper for the
    /// road-under-a-point query).
    pub bounds: [f32; 4],
    /// The junction at the destination node (index into [`RoadNetwork::junctions`]), if any.
    pub to_junction: Option<usize>,
    /// The junction at the origin node, if any.
    pub from_junction: Option<usize>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct JunctionEnd {
    pub lanes: u8,
    /// Per lane: connector indices into [`RoadNetwork::connectors`], in retail record order.
    pub lane_connectors: Vec<Vec<usize>>,
    /// The segment arriving (approach) or leaving (exit) at this end, if the data has one.
    pub segment: Option<usize>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Junction {
    pub id: JunctionId,
    pub signalled: bool,
    pub speed: f32,
    pub approaches: [Option<JunctionEnd>; ENDS],
    pub exits: [Option<JunctionEnd>; ENDS],
    /// Indices into [`RoadNetwork::connectors`], sorted by retail index.
    pub connectors: Vec<usize>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Connector {
    pub id: ConnectorId,
    pub junction: usize,
    pub entry_speed: f32,
    pub from_end: u8,
    pub from_lane: u8,
    pub to_end: u8,
    pub to_lane: u8,
    pub curve: Curve,
}

impl Connector {
    pub fn length(&self) -> f32 {
        self.curve.length
    }
    /// How the connector turns, from the node end numbering the junction query uses [code
    /// `sub_82E11E90`: `to_end == from_end + 1` yields to oncoming traffic, `to_end == from_end - 1`
    /// only merges and may go on red].
    pub fn turn(&self) -> Turn {
        Turn::between(self.from_end, self.to_end)
    }
}

/// Turn kinds by node end arithmetic (mod 4). With the shipped data's lane order (lane 0 left)
/// `Right` connectors leave from the right lane and `Left` ones from the left lane [data].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Turn {
    /// `to_end == from_end + 1`: crosses the oncoming flow.
    Left,
    /// `to_end == from_end + 2`.
    Straight,
    /// `to_end == from_end + 3` (= `from_end - 1`).
    Right,
    /// `to_end == from_end`.
    UTurn,
}

impl Turn {
    pub fn between(from_end: u8, to_end: u8) -> Turn {
        match (to_end.wrapping_sub(from_end)) & 3 {
            1 => Turn::Left,
            2 => Turn::Straight,
            3 => Turn::Right,
            _ => Turn::UTurn,
        }
    }
}

/// Where a lane point lies: position and unit forward direction (engine axes, y up).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    pub position: Vec3,
    pub forward: Vec3,
}

impl Frame {
    /// Yaw about +y in radians, 0 = facing +z, positive towards +x.
    pub fn yaw(&self) -> f32 {
        self.forward[0].atan2(self.forward[2])
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct RoadNetwork {
    /// Sorted by id.
    pub segments: Vec<Segment>,
    /// Sorted by node id.
    pub junctions: Vec<Junction>,
    /// Sorted by (junction node id, retail index).
    pub connectors: Vec<Connector>,
    segment_index: BTreeMap<SegmentId, usize>,
    junction_index: BTreeMap<JunctionId, usize>,
    connector_index: BTreeMap<ConnectorId, usize>,
}

fn sub(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn normalise(v: Vec3) -> Vec3 {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if l > 1e-9 {
        [v[0] / l, v[1] / l, v[2] / l]
    } else {
        [0.0, 0.0, 1.0]
    }
}

impl RoadNetwork {
    /// Build and check the network. References that cannot be resolved are errors (a connector
    /// naming a missing end or lane, a lane list naming a missing connector, duplicate ids,
    /// pieces out of order); a junction end without a segment is kept as `segment: None` (the
    /// map edge).
    pub fn build(input: &RoadInput) -> Result<Self, TrafficError> {
        let mut segs: Vec<&SegmentInput> = input.segments.iter().collect();
        segs.sort_by_key(|s| s.id);
        let mut juncs: Vec<&JunctionInput> = input.junctions.iter().collect();
        juncs.sort_by_key(|j| j.id);
        let mut net = RoadNetwork::default();
        for (i, j) in juncs.iter().enumerate() {
            if net.junction_index.insert(j.id, i).is_some() {
                return Err(TrafficError(format!("junction {:016X} twice", j.id.0)));
            }
        }
        for (i, s) in segs.iter().enumerate() {
            if net.segment_index.insert(s.id, i).is_some() {
                return Err(TrafficError(format!("segment {:016X} twice", s.id.0)));
            }
            if s.lanes == 0 || s.pieces.is_empty() || s.from_end as usize >= ENDS || s.to_end as usize >= ENDS {
                return Err(TrafficError(format!("segment {:016X}: no lanes, no pieces or a bad end", s.id.0)));
            }
            let mut pieces = Vec::with_capacity(s.pieces.len());
            let mut start = 0.0f32;
            for (k, p) in s.pieces.iter().enumerate() {
                if !(p.end_distance > start) {
                    return Err(TrafficError(format!("segment {:016X}: piece {k} out of order", s.id.0)));
                }
                pieces.push(Piece {
                    start_distance: start,
                    end_distance: p.end_distance,
                    centre: p.centre,
                    left_start: p.left_start,
                    right_start: p.right_start,
                    left_end: p.left_end,
                    right_end: p.right_end,
                });
                start = p.end_distance;
            }
            net.segments.push(Segment {
                id: s.id,
                from_node: s.from_node,
                from_end: s.from_end,
                to_node: s.to_node,
                to_end: s.to_end,
                length: s.length,
                speed_limit: s.speed_limit,
                lanes: s.lanes,
                manoeuvres: s.manoeuvres,
                district: s.district,
                bounds: pieces.iter().flat_map(|p| [p.left_start, p.right_start, p.left_end, p.right_end]).fold(
                    [f32::MAX, f32::MAX, f32::MIN, f32::MIN],
                    |b, v| [b[0].min(v[0]), b[1].min(v[2]), b[2].max(v[0]), b[3].max(v[2])],
                ),
                pieces,
                to_junction: net.junction_index.get(&JunctionId(s.to_node)).copied(),
                from_junction: net.junction_index.get(&JunctionId(s.from_node)).copied(),
            });
        }
        // Connectors, sorted by (junction, index).
        for (ji, j) in juncs.iter().enumerate() {
            let mut cons: Vec<&ConnectorInput> = j.connectors.iter().collect();
            cons.sort_by_key(|c| c.index);
            for c in cons {
                let id = ConnectorId { junction: j.id, index: c.index };
                if c.from_end as usize >= ENDS || c.to_end as usize >= ENDS {
                    return Err(TrafficError(format!("connector {:016X}#{}: bad end", j.id.0, c.index)));
                }
                let (Some(a), Some(e)) = (&j.approaches[c.from_end as usize], &j.exits[c.to_end as usize]) else {
                    return Err(TrafficError(format!("connector {:016X}#{}: missing approach or exit end", j.id.0, c.index)));
                };
                if c.from_lane >= a.lanes || c.to_lane >= e.lanes {
                    return Err(TrafficError(format!("connector {:016X}#{}: lane past the end's lanes", j.id.0, c.index)));
                }
                if net.connector_index.insert(id, net.connectors.len()).is_some() {
                    return Err(TrafficError(format!("connector {:016X}#{} twice", j.id.0, c.index)));
                }
                net.connectors.push(Connector {
                    id,
                    junction: ji,
                    entry_speed: c.entry_speed,
                    from_end: c.from_end,
                    from_lane: c.from_lane,
                    to_end: c.to_end,
                    to_lane: c.to_lane,
                    curve: c.curve,
                });
            }
        }
        for (ji, j) in juncs.iter().enumerate() {
            let resolve = |end: &Option<EndInput>, e: usize, approach: bool| -> Result<Option<JunctionEnd>, TrafficError> {
                let Some(end) = end else { return Ok(None) };
                let mut lists = Vec::with_capacity(end.lanes as usize);
                for lane in 0..end.lanes as usize {
                    let mut list = Vec::new();
                    for &k in end.lane_connectors.get(lane).map(|v| v.as_slice()).unwrap_or(&[]) {
                        let id = ConnectorId { junction: j.id, index: k };
                        let Some(&ci) = net.connector_index.get(&id) else {
                            return Err(TrafficError(format!("junction {:016X} end {e}: lane {lane} names missing connector {k}", j.id.0)));
                        };
                        list.push(ci);
                    }
                    lists.push(list);
                }
                let segment = net.segments.iter().position(|s| {
                    if approach {
                        s.to_node == j.id.0 && s.to_end as usize == e
                    } else {
                        s.from_node == j.id.0 && s.from_end as usize == e
                    }
                });
                Ok(Some(JunctionEnd { lanes: end.lanes, lane_connectors: lists, segment }))
            };
            let mut approaches: [Option<JunctionEnd>; ENDS] = Default::default();
            let mut exits: [Option<JunctionEnd>; ENDS] = Default::default();
            for e in 0..ENDS {
                approaches[e] = resolve(&j.approaches[e], e, true)?;
                exits[e] = resolve(&j.exits[e], e, false)?;
            }
            let connectors = (0..net.connectors.len()).filter(|&c| net.connectors[c].junction == ji).collect();
            net.junctions.push(Junction { id: j.id, signalled: j.signalled, speed: j.speed, approaches, exits, connectors });
        }
        Ok(net)
    }

    pub fn segment_index(&self, id: SegmentId) -> Option<usize> {
        self.segment_index.get(&id).copied()
    }
    pub fn junction_index(&self, id: JunctionId) -> Option<usize> {
        self.junction_index.get(&id).copied()
    }
    pub fn connector_index(&self, id: ConnectorId) -> Option<usize> {
        self.connector_index.get(&id).copied()
    }

    /// The connectors lane `lane` of `segment` may take at its destination junction (retail
    /// approach lane list order); empty at a dead end.
    pub fn next_connectors(&self, segment: usize, lane: u8) -> &[usize] {
        let s = &self.segments[segment];
        let Some(j) = s.to_junction else { return &[] };
        match &self.junctions[j].approaches[s.to_end as usize] {
            Some(end) => end.lane_connectors.get(lane as usize).map(|v| v.as_slice()).unwrap_or(&[]),
            None => &[],
        }
    }

    /// The segment a connector leads onto (the exit at its `to_end`).
    pub fn connector_exit(&self, connector: usize) -> Option<usize> {
        let c = &self.connectors[connector];
        self.junctions[c.junction].exits[c.to_end as usize].as_ref()?.segment
    }

    /// The segment a connector comes from (the approach at its `from_end`).
    pub fn connector_approach(&self, connector: usize) -> Option<usize> {
        let c = &self.connectors[connector];
        self.junctions[c.junction].approaches[c.from_end as usize].as_ref()?.segment
    }

    /// The lanes next to `lane` (left = lane - 1, right = lane + 1), the targets a lane change can
    /// pick on this segment.
    pub fn adjacent_lanes(&self, segment: usize, lane: u8) -> (Option<u8>, Option<u8>) {
        let n = self.segments[segment].lanes;
        (lane.checked_sub(1), (lane + 1 < n).then_some(lane + 1))
    }

    /// The piece holding `distance` along a segment (clamped to the first / last piece).
    pub fn piece_at(&self, segment: usize, distance: f32) -> usize {
        let pieces = &self.segments[segment].pieces;
        pieces.partition_point(|p| p.end_distance < distance).min(pieces.len() - 1)
    }

    /// Point and direction on a lane at `distance` along the segment. `lane` may be fractional
    /// (a lane change blends between two lanes); 0.0 is the centre of lane 0.
    pub fn lane_frame(&self, segment: usize, lane: f32, distance: f32) -> Frame {
        let s = &self.segments[segment];
        let p = &s.pieces[self.piece_at(segment, distance)];
        let local = (distance - p.start_distance).clamp(0.0, p.centre.length);
        let t = p.centre.parameter_at(local);
        let c = p.centre.point(t);
        let across_s = sub(p.right_start, p.left_start);
        let across_e = sub(p.right_end, p.left_end);
        // Fraction of the way from the centre to the right edge: -1 left edge, +1 right edge.
        let f = 2.0 * (lane + 0.5) / s.lanes as f32 - 1.0;
        let half: Vec3 = std::array::from_fn(|i| 0.5 * (across_s[i] + (across_e[i] - across_s[i]) * t));
        let position = std::array::from_fn(|i| c[i] + half[i] * f);
        Frame { position, forward: normalise(p.centre.derivative(t)) }
    }

    /// Point and direction on a connector at `distance` along it.
    pub fn connector_frame(&self, connector: usize, distance: f32) -> Frame {
        let c = &self.connectors[connector].curve;
        let t = c.parameter_at(distance);
        Frame { position: c.point(t), forward: normalise(c.derivative(t)) }
    }

    /// The closest lane point to `point` (piece chords, horizontal distance): segment, lane,
    /// distance along the segment and the squared horizontal distance. An engine helper for
    /// spawning onto the road; where retail's vehicle factory snaps its spawn point is open (V2).
    pub fn nearest_lane(&self, point: Vec3) -> Option<(usize, u8, f32, f32)> {
        let mut best: Option<(usize, u8, f32, f32)> = None;
        for (si, s) in self.segments.iter().enumerate() {
            for p in &s.pieces {
                for lane in 0..s.lanes {
                    let a = self.lane_frame(si, lane as f32, p.start_distance).position;
                    let b = self.lane_frame(si, lane as f32, p.end_distance).position;
                    let (dx, dz) = (b[0] - a[0], b[2] - a[2]);
                    let len2 = dx * dx + dz * dz;
                    let u = if len2 > 1e-9 { (((point[0] - a[0]) * dx + (point[2] - a[2]) * dz) / len2).clamp(0.0, 1.0) } else { 0.0 };
                    let (qx, qz) = (a[0] + dx * u - point[0], a[2] + dz * u - point[2]);
                    let d2 = qx * qx + qz * qz;
                    if best.is_none_or(|b| d2 < b.3) {
                        best = Some((si, lane, p.start_distance + (p.end_distance - p.start_distance) * u, d2));
                    }
                }
            }
        }
        best
    }

    pub fn lane_id(&self, segment: usize, lane: u8) -> LaneId {
        LaneId { segment: self.segments[segment].id, lane }
    }
}
