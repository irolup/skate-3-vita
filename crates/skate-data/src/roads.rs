//! The living-world road graph, `roads.bin` v2 (setup group `livingworld`, written by
//! `tools/asset_pipeline/living_world_roads.py` `write_graph`; format notes in
//! `docs/hails-additions/living-world/vehicles-data.md`). Bytes in, graph out; no file I/O.
//!
//! The graph is the retail road network (RW object `0x00EB0013`) of every district merged across its
//! tiles: directed segments (retail 64-bit ids; `to_node` is the destination, `from_node` the origin),
//! their lane geometry as ~4 m pieces (cubic Hermite centre line, road edges, arc-length tables),
//! junctions (one per road node: inner / outer quads, 8 end records = the approaches and exits of
//! node ends 0-3 with the connectors each lane may take) and the turn connectors (cubic Hermite
//! curves from an approach lane to an exit lane). Unknown retail fields keep raw names (`flag_04`,
//! `f32_50`, `word_56`). Every element keeps its retail id or index so a mod or a network peer can
//! name it; [`RoadGraph::to_bytes`] writes the same format (synthetic tests, per-map roads of mods).

use std::collections::BTreeMap;
use std::fmt;

pub const MAGIC: &[u8; 8] = b"LWROADS2";
pub const VERSION: u32 = 2;
pub const ARC_SAMPLES: usize = 16;
/// End records per junction: approaches of node ends 0-3, then exits of node ends 0-3 [data].
pub const JUNCTION_ENDS: usize = 8;
/// Segment flag: the pieces cover the whole segment (index 0..n, last piece ends at its length).
pub const SEGMENT_PIECES_COMPLETE: u32 = 1;

const HEADER: usize = 36;
const DISTRICT: usize = 32;
const SEGMENT: usize = 72;
const PIECE: usize = 224;
const JUNCTION: usize = 132;
const END: usize = 40;
const CONNECTOR: usize = 144;

#[derive(Debug, Clone, PartialEq)]
pub struct RoadError(pub String);

impl fmt::Display for RoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "living-world road graph: {}", self.0)
    }
}
impl std::error::Error for RoadError {}

/// A cubic Hermite curve (centre line of a piece, or a turn connector) with its arc-length table:
/// `arc[k]` = length from the start to parameter `k / 15`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Curve {
    pub start: [f32; 3],
    pub end: [f32; 3],
    pub tangent_start: [f32; 3],
    pub tangent_end: [f32; 3],
    pub length: f32,
    pub arc: [f32; ARC_SAMPLES],
}

impl Curve {
    /// Point at parameter `t` in [0, 1] (Hermite basis with the stored tangents).
    pub fn point(&self, t: f32) -> [f32; 3] {
        let (t2, t3) = (t * t, t * t * t);
        let h00 = 2.0 * t3 - 3.0 * t2 + 1.0;
        let h10 = t3 - 2.0 * t2 + t;
        let h01 = -2.0 * t3 + 3.0 * t2;
        let h11 = t3 - t2;
        std::array::from_fn(|i| h00 * self.start[i] + h10 * self.tangent_start[i] + h01 * self.end[i] + h11 * self.tangent_end[i])
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

#[derive(Debug, Clone, PartialEq)]
pub struct District {
    pub name: String,
    pub segments: std::ops::Range<usize>,
    pub junctions: std::ops::Range<usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Segment {
    pub id: u64,
    pub to_node: u64,
    pub from_node: u64,
    pub to_end: u32,
    pub from_end: u32,
    pub length: f32,
    pub width_a: f32,
    pub width_b: f32,
    /// m/s (14.167 = 51 km/h, 13.889 = 50 km/h) [data].
    pub speed_limit: f32,
    pub lanes: u32,
    pub word_56: u32,
    pub pieces: std::ops::Range<usize>,
    pub flags: u32,
    pub district: u32,
}

impl Segment {
    pub fn pieces_complete(&self) -> bool {
        self.flags & SEGMENT_PIECES_COMPLETE != 0
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Piece {
    pub segment: u32,
    pub index: u32,
    /// Distance along the segment at the piece end (m).
    pub distance: f32,
    pub centre: Curve,
    pub left_start: [f32; 3],
    pub right_start: [f32; 3],
    pub left_end: [f32; 3],
    pub right_end: [f32; 3],
    pub edge_tangents: [[f32; 3]; 4],
}

#[derive(Debug, Clone, PartialEq)]
pub struct JunctionEnd {
    pub id: u64,
    /// 1 = a road, 2 = none [data].
    pub kind: u32,
    /// 1 = approach, 0 = exit, `u32::MAX` = none [data].
    pub side: u32,
    pub lanes: u32,
    pub lane_counts: [u32; 4],
    pub first_entry: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Junction {
    pub node: u64,
    pub district: u32,
    /// m/s, the road speed at the junction [data].
    pub speed: f32,
    /// 0 / 1; meaning open (set on 24 of the 33 shipped junctions) [data].
    pub flag_04: u32,
    pub quad_tags: [u32; 2],
    pub inner: [[f32; 3]; 4],
    pub outer: [[f32; 3]; 4],
    pub connectors: std::ops::Range<usize>,
    pub ends: [JunctionEnd; JUNCTION_ENDS],
}

#[derive(Debug, Clone, PartialEq)]
pub struct Connector {
    pub junction: u32,
    /// Retail index within its junction (the lane lists name connectors by it).
    pub index: u32,
    pub f32_50: f32,
    pub from_end: u32,
    pub from_lane: u32,
    pub to_end: u32,
    pub to_lane: u32,
    pub curve: Curve,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct RoadGraph {
    pub districts: Vec<District>,
    pub segments: Vec<Segment>,
    pub pieces: Vec<Piece>,
    pub junctions: Vec<Junction>,
    pub connectors: Vec<Connector>,
    pub lane_entries: Vec<u32>,
    segment_index: BTreeMap<u64, usize>,
    junction_index: BTreeMap<u64, usize>,
}

struct Reader<'a> {
    data: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn u32(&mut self) -> Result<u32, RoadError> {
        let b = self.data.get(self.at..self.at + 4).ok_or_else(|| RoadError(format!("truncated at {}", self.at)))?;
        self.at += 4;
        Ok(u32::from_le_bytes(b.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<u64, RoadError> {
        Ok(self.u32()? as u64 | (self.u32()? as u64) << 32)
    }
    fn f32(&mut self) -> Result<f32, RoadError> {
        self.u32().map(f32::from_bits)
    }
    fn vec3(&mut self) -> Result<[f32; 3], RoadError> {
        Ok([self.f32()?, self.f32()?, self.f32()?])
    }
    fn arc(&mut self) -> Result<[f32; ARC_SAMPLES], RoadError> {
        let mut out = [0.0; ARC_SAMPLES];
        for v in &mut out {
            *v = self.f32()?;
        }
        Ok(out)
    }
}

fn range(first: u32, count: u32, len: usize, what: &str) -> Result<std::ops::Range<usize>, RoadError> {
    let (first, count) = (first as usize, count as usize);
    if first + count > len {
        return Err(RoadError(format!("{what} {first}+{count} past {len}")));
    }
    Ok(first..first + count)
}

impl RoadGraph {
    /// Parse `roads.bin` v2.
    pub fn parse(data: &[u8]) -> Result<Self, RoadError> {
        if data.get(..8) != Some(&MAGIC[..]) {
            return Err(RoadError("not a roads.bin v2 graph".into()));
        }
        let mut r = Reader { data, at: 8 };
        let version = r.u32()?;
        if version != VERSION {
            return Err(RoadError(format!("unsupported version {version}")));
        }
        let counts: Vec<usize> = (0..6).map(|_| r.u32().map(|v| v as usize)).collect::<Result<_, _>>()?;
        let [nd, ns, np, nj, nc, nl] = counts[..] else { unreachable!() };
        let expected = HEADER as u128
            + nd as u128 * DISTRICT as u128
            + ns as u128 * SEGMENT as u128
            + np as u128 * PIECE as u128
            + nj as u128 * (JUNCTION + JUNCTION_ENDS * END) as u128
            + nc as u128 * CONNECTOR as u128
            + nl as u128 * 4;
        if expected != data.len() as u128 {
            return Err(RoadError(format!("size {} does not match the counts ({expected})", data.len())));
        }
        let mut g = RoadGraph::default();
        for _ in 0..nd {
            let raw = &data[r.at..r.at + 16];
            r.at += 16;
            let name = String::from_utf8(raw.split(|&b| b == 0).next().unwrap_or(&[]).to_vec()).map_err(|_| RoadError("district name is not text".into()))?;
            let (sf, sc, jf, jc) = (r.u32()?, r.u32()?, r.u32()?, r.u32()?);
            g.districts.push(District { name, segments: range(sf, sc, ns, "district segments")?, junctions: range(jf, jc, nj, "district junctions")? });
        }
        for _ in 0..ns {
            let (id, to_node, from_node) = (r.u64()?, r.u64()?, r.u64()?);
            let (to_end, from_end) = (r.u32()?, r.u32()?);
            let (length, width_a, width_b, speed_limit) = (r.f32()?, r.f32()?, r.f32()?, r.f32()?);
            let (lanes, word_56, pf, pc, flags, district) = (r.u32()?, r.u32()?, r.u32()?, r.u32()?, r.u32()?, r.u32()?);
            if district as usize >= nd {
                return Err(RoadError(format!("segment {id:016X}: district {district}")));
            }
            let pieces = range(pf, pc, np, "segment pieces")?;
            g.segments.push(Segment { id, to_node, from_node, to_end, from_end, length, width_a, width_b, speed_limit, lanes, word_56, pieces, flags, district });
        }
        for _ in 0..np {
            let (segment, index, distance, length) = (r.u32()?, r.u32()?, r.f32()?, r.f32()?);
            if segment as usize >= ns {
                return Err(RoadError(format!("piece of segment {segment} past {ns}")));
            }
            let (start, end, tangent_start, tangent_end) = (r.vec3()?, r.vec3()?, r.vec3()?, r.vec3()?);
            let (left_start, right_start, left_end, right_end) = (r.vec3()?, r.vec3()?, r.vec3()?, r.vec3()?);
            let edge_tangents = [r.vec3()?, r.vec3()?, r.vec3()?, r.vec3()?];
            let arc = r.arc()?;
            g.pieces.push(Piece {
                segment,
                index,
                distance,
                centre: Curve { start, end, tangent_start, tangent_end, length, arc },
                left_start,
                right_start,
                left_end,
                right_end,
                edge_tangents,
            });
        }
        for _ in 0..nj {
            let node = r.u64()?;
            let (district, speed, flag_04) = (r.u32()?, r.f32()?, r.u32()?);
            let quad_tags = [r.u32()?, r.u32()?];
            let inner = [r.vec3()?, r.vec3()?, r.vec3()?, r.vec3()?];
            let outer = [r.vec3()?, r.vec3()?, r.vec3()?, r.vec3()?];
            let connectors = range(r.u32()?, r.u32()?, nc, "junction connectors")?;
            let mut ends = Vec::with_capacity(JUNCTION_ENDS);
            for _ in 0..JUNCTION_ENDS {
                let id = r.u64()?;
                let (kind, side, lanes) = (r.u32()?, r.u32()?, r.u32()?);
                let lane_counts = [r.u32()?, r.u32()?, r.u32()?, r.u32()?];
                let first_entry = r.u32()? as usize;
                let total: usize = lane_counts.iter().map(|&c| c as usize).sum();
                if first_entry + total > nl {
                    return Err(RoadError(format!("junction {node:016X}: lane list past {nl}")));
                }
                ends.push(JunctionEnd { id, kind, side, lanes, lane_counts, first_entry });
            }
            if district as usize >= nd {
                return Err(RoadError(format!("junction {node:016X}: district {district}")));
            }
            g.junctions.push(Junction { node, district, speed, flag_04, quad_tags, inner, outer, connectors, ends: ends.try_into().unwrap() });
        }
        for _ in 0..nc {
            let (junction, index, length, f32_50) = (r.u32()?, r.u32()?, r.f32()?, r.f32()?);
            let (from_end, from_lane, to_end, to_lane) = (r.u32()?, r.u32()?, r.u32()?, r.u32()?);
            if junction as usize >= nj || from_end >= 4 || to_end >= 4 {
                return Err(RoadError(format!("connector {index} of junction {junction}: bad references")));
            }
            let (start, end, tangent_start, tangent_end) = (r.vec3()?, r.vec3()?, r.vec3()?, r.vec3()?);
            let arc = r.arc()?;
            g.connectors.push(Connector { junction, index, f32_50, from_end, from_lane, to_end, to_lane, curve: Curve { start, end, tangent_start, tangent_end, length, arc } });
        }
        for _ in 0..nl {
            g.lane_entries.push(r.u32()?);
        }
        g.reindex()?;
        Ok(g)
    }

    fn reindex(&mut self) -> Result<(), RoadError> {
        self.segment_index.clear();
        self.junction_index.clear();
        for (i, s) in self.segments.iter().enumerate() {
            if self.segment_index.insert(s.id, i).is_some() {
                return Err(RoadError(format!("segment {:016X} twice", s.id)));
            }
        }
        for (i, j) in self.junctions.iter().enumerate() {
            if self.junction_index.insert(j.node, i).is_some() {
                return Err(RoadError(format!("junction {:016X} twice", j.node)));
            }
        }
        Ok(())
    }

    pub fn segment(&self, id: u64) -> Option<&Segment> {
        self.segment_index.get(&id).map(|&i| &self.segments[i])
    }

    pub fn junction(&self, node: u64) -> Option<&Junction> {
        self.junction_index.get(&node).map(|&i| &self.junctions[i])
    }

    pub fn district(&self, name: &str) -> Option<&District> {
        self.districts.iter().find(|d| d.name == name)
    }

    pub fn segment_pieces(&self, segment: &Segment) -> &[Piece] {
        &self.pieces[segment.pieces.clone()]
    }

    pub fn junction_connectors(&self, junction: &Junction) -> &[Connector] {
        &self.connectors[junction.connectors.clone()]
    }

    /// Retail connector indices lane `lane` of end record `end` (0-3 approaches, 4-7 exits) lists.
    pub fn lane_connectors(&self, junction: &Junction, end: usize, lane: usize) -> &[u32] {
        let Some(e) = junction.ends.get(end) else { return &[] };
        if lane >= 4 {
            return &[];
        }
        let skip: usize = e.lane_counts[..lane].iter().map(|&c| c as usize).sum();
        let first = e.first_entry + skip;
        &self.lane_entries[first..first + e.lane_counts[lane] as usize]
    }

    /// The connectors a car on `segment`, lane `lane`, may take at its destination junction.
    pub fn next_connectors(&self, segment: &Segment, lane: usize) -> Vec<&Connector> {
        let Some(j) = self.junction(segment.to_node) else { return Vec::new() };
        let own = self.junction_connectors(j);
        self.lane_connectors(j, segment.to_end as usize, lane).iter().filter_map(|&k| own.iter().find(|c| c.index == k)).collect()
    }

    /// The segment a connector leads onto: the one leaving its junction's node at the exit end.
    pub fn connector_exit(&self, connector: &Connector) -> Option<&Segment> {
        let j = self.junctions.get(connector.junction as usize)?;
        self.segments.iter().find(|s| s.from_node == j.node && s.from_end == connector.to_end)
    }

    /// The traffic core's input (`skate_core::living_world::traffic::RoadInput`): segments with
    /// their pieces, junctions with their approach / exit ends (end records of kind 1) and
    /// connectors. `flag_04` becomes `signalled`, `f32_50` the connector's entry speed.
    pub fn traffic_input(&self) -> skate_core::living_world::traffic::RoadInput {
        self.traffic_input_where(&|_| true)
    }

    /// The traffic input of one district only (its segments and junctions; a junction end whose
    /// segment belongs to another district becomes a map edge). The districts are separate
    /// worlds that overlap in x / z [data: University roads lie 16-18 m above the DownTown walk
    /// mesh, Industrial roads 47-59 m below it], so a world must only see its own roads; with the
    /// whole graph a car placed or routed onto another district's road floats in the air or
    /// drives under the ground. `None` when the graph has no district of that name.
    pub fn district_traffic_input(&self, name: &str) -> Option<skate_core::living_world::traffic::RoadInput> {
        let d = self.districts.iter().position(|d| d.name == name)? as u32;
        Some(self.traffic_input_where(&|district| district == d))
    }

    fn traffic_input_where(&self, keep: &dyn Fn(u32) -> bool) -> skate_core::living_world::traffic::RoadInput {
        use skate_core::living_world::traffic as t;
        let curve = |c: &Curve| t::Curve { start: c.start, end: c.end, tangent_start: c.tangent_start, tangent_end: c.tangent_end, length: c.length, arc: c.arc };
        let segments = self
            .segments
            .iter()
            .filter(|s| keep(s.district))
            .map(|s| t::SegmentInput {
                id: t::SegmentId(s.id),
                from_node: s.from_node,
                from_end: s.from_end as u8,
                to_node: s.to_node,
                to_end: s.to_end as u8,
                length: s.length,
                speed_limit: s.speed_limit,
                lanes: s.lanes as u8,
                manoeuvres: s.word_56,
                district: s.district,
                pieces: self
                    .segment_pieces(s)
                    .iter()
                    .map(|p| t::PieceInput { end_distance: p.distance, centre: curve(&p.centre), left_start: p.left_start, right_start: p.right_start, left_end: p.left_end, right_end: p.right_end })
                    .collect(),
            })
            .collect();
        let junctions = self
            .junctions
            .iter()
            .filter(|j| keep(j.district))
            .map(|j| {
                let end = |k: usize| -> Option<t::EndInput> {
                    let e = &j.ends[k];
                    (e.kind == 1).then(|| t::EndInput {
                        lanes: e.lanes as u8,
                        lane_connectors: (0..(e.lanes as usize).min(4)).map(|lane| self.lane_connectors(j, k, lane).to_vec()).collect(),
                    })
                };
                t::JunctionInput {
                    id: t::JunctionId(j.node),
                    signalled: j.flag_04 != 0,
                    speed: j.speed,
                    approaches: std::array::from_fn(end),
                    exits: std::array::from_fn(|k| end(4 + k)),
                    connectors: self
                        .junction_connectors(j)
                        .iter()
                        .map(|c| t::ConnectorInput {
                            index: c.index,
                            entry_speed: c.f32_50,
                            from_end: c.from_end as u8,
                            from_lane: c.from_lane as u8,
                            to_end: c.to_end as u8,
                            to_lane: c.to_lane as u8,
                            curve: curve(&c.curve),
                        })
                        .collect(),
                }
            })
            .collect();
        skate_core::living_world::traffic::RoadInput { segments, junctions }
    }

    /// Write the same format (`roads.bin` v2).
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        let u32s = |out: &mut Vec<u8>, v: &[u32]| v.iter().for_each(|x| out.extend_from_slice(&x.to_le_bytes()));
        let f32s = |out: &mut Vec<u8>, v: &[f32]| v.iter().for_each(|x| out.extend_from_slice(&x.to_le_bytes()));
        let u64s = |out: &mut Vec<u8>, v: &[u64]| v.iter().for_each(|x| out.extend_from_slice(&x.to_le_bytes()));
        let curve = |out: &mut Vec<u8>, c: &Curve| {
            f32s(out, &c.start);
            f32s(out, &c.end);
            f32s(out, &c.tangent_start);
            f32s(out, &c.tangent_end);
        };
        out.extend_from_slice(MAGIC);
        let n = |v: usize| v as u32;
        u32s(&mut out, &[VERSION, n(self.districts.len()), n(self.segments.len()), n(self.pieces.len()), n(self.junctions.len()), n(self.connectors.len()), n(self.lane_entries.len())]);
        for d in &self.districts {
            let mut name = [0u8; 16];
            let raw = d.name.as_bytes();
            name[..raw.len().min(16)].copy_from_slice(&raw[..raw.len().min(16)]);
            out.extend_from_slice(&name);
            u32s(&mut out, &[n(d.segments.start), n(d.segments.len()), n(d.junctions.start), n(d.junctions.len())]);
        }
        for s in &self.segments {
            u64s(&mut out, &[s.id, s.to_node, s.from_node]);
            u32s(&mut out, &[s.to_end, s.from_end]);
            f32s(&mut out, &[s.length, s.width_a, s.width_b, s.speed_limit]);
            u32s(&mut out, &[s.lanes, s.word_56, n(s.pieces.start), n(s.pieces.len()), s.flags, s.district]);
        }
        for p in &self.pieces {
            u32s(&mut out, &[p.segment, p.index]);
            f32s(&mut out, &[p.distance, p.centre.length]);
            curve(&mut out, &p.centre);
            for v in [p.left_start, p.right_start, p.left_end, p.right_end].iter().chain(p.edge_tangents.iter()) {
                f32s(&mut out, v);
            }
            f32s(&mut out, &p.centre.arc);
        }
        for j in &self.junctions {
            u64s(&mut out, &[j.node]);
            u32s(&mut out, &[j.district]);
            f32s(&mut out, &[j.speed]);
            u32s(&mut out, &[j.flag_04, j.quad_tags[0], j.quad_tags[1]]);
            for v in j.inner.iter().chain(j.outer.iter()) {
                f32s(&mut out, v);
            }
            u32s(&mut out, &[n(j.connectors.start), n(j.connectors.len())]);
            for e in &j.ends {
                u64s(&mut out, &[e.id]);
                u32s(&mut out, &[e.kind, e.side, e.lanes]);
                u32s(&mut out, &e.lane_counts);
                u32s(&mut out, &[n(e.first_entry)]);
            }
        }
        for c in &self.connectors {
            u32s(&mut out, &[c.junction, c.index]);
            f32s(&mut out, &[c.curve.length, c.f32_50]);
            u32s(&mut out, &[c.from_end, c.from_lane, c.to_end, c.to_lane]);
            curve(&mut out, &c.curve);
            f32s(&mut out, &c.curve.arc);
        }
        u32s(&mut out, &self.lane_entries);
        out
    }
}

/// Signal durations from the exported `tables.json` (`classes.livingworld.trafficlights`:
/// `signal_green`, `signal_amber`, `signal_all_red`, `Hash_5E41C959D17527CC` = the walk split,
/// read by `sub_826B1540` into controller `+300`). A content overlay that patches those fields
/// changes the city's lights. `None` when a field is missing.
pub fn signal_timings(tables: &serde_json::Value) -> Option<skate_core::living_world::traffic::SignalTimings> {
    let f = tables.pointer("/classes/livingworld/trafficlights/fields")?;
    let num = |k: &str| f.get(k).and_then(|v| v.as_f64()).map(|v| v as f32);
    Some(skate_core::living_world::traffic::SignalTimings {
        green: num("signal_green")?,
        amber: num("signal_amber")?,
        all_red: num("signal_all_red")?,
        walk_split: num("Hash_5E41C959D17527CC")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn straight(start: [f32; 3], end: [f32; 3]) -> Curve {
        let d: [f32; 3] = std::array::from_fn(|i| end[i] - start[i]);
        let length = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        Curve { start, end, tangent_start: d, tangent_end: d, length, arc: std::array::from_fn(|k| length * k as f32 / 15.0) }
    }

    fn end(id: u64, kind: u32, side: u32, lanes: u32, counts: [u32; 4], first: usize) -> JunctionEnd {
        JunctionEnd { id, kind, side, lanes, lane_counts: counts, first_entry: first }
    }

    /// One district: segment 1 runs west -> into junction node 0xA at its end 0; connector 0 takes
    /// lane 0 straight through to end 2, where segment 2 leaves the junction.
    fn synthetic() -> RoadGraph {
        let none = end(0, 2, u32::MAX, 2, [0; 4], 0);
        let mut ends: [JunctionEnd; JUNCTION_ENDS] = std::array::from_fn(|_| none.clone());
        ends[0] = end(0x10, 1, 1, 1, [1, 0, 0, 0], 0); // approach at end 0, lane 0 -> connector 0
        ends[4 + 2] = end(0x11, 1, 0, 1, [1, 0, 0, 0], 1); // exit at end 2, lane 0 <- connector 0
        let mut g = RoadGraph {
            districts: vec![District { name: "Test".into(), segments: 0..2, junctions: 0..1 }],
            segments: vec![
                Segment { id: 1, to_node: 0xA, from_node: 0xB, to_end: 0, from_end: 1, length: 8.0, width_a: 4.0, width_b: 4.0, speed_limit: 14.166667, lanes: 1, word_56: 3, pieces: 0..2, flags: SEGMENT_PIECES_COMPLETE, district: 0 },
                Segment { id: 2, to_node: 0xC, from_node: 0xA, to_end: 3, from_end: 2, length: 4.0, width_a: 4.0, width_b: 4.0, speed_limit: 13.888889, lanes: 1, word_56: 3, pieces: 2..3, flags: SEGMENT_PIECES_COMPLETE, district: 0 },
            ],
            pieces: vec![],
            junctions: vec![Junction { node: 0xA, district: 0, speed: 13.888889, flag_04: 1, quad_tags: [7, 9], inner: [[0.0; 3]; 4], outer: [[1.0; 3]; 4], connectors: 0..1, ends }],
            connectors: vec![Connector { junction: 0, index: 0, f32_50: 13.888889, from_end: 0, from_lane: 0, to_end: 2, to_lane: 0, curve: straight([8.0, 0.0, 0.0], [20.0, 0.0, 0.0]) }],
            lane_entries: vec![0, 0],
            ..Default::default()
        };
        for (k, (s, a, b)) in [(0u32, 0.0f32, 4.0f32), (0, 4.0, 8.0), (1, 20.0, 24.0)].into_iter().enumerate() {
            g.pieces.push(Piece {
                segment: s,
                index: if k == 2 { 0 } else { k as u32 },
                distance: if k == 2 { 4.0 } else { b },
                centre: straight([a, 0.0, 0.0], [b, 0.0, 0.0]),
                left_start: [a, 0.0, 2.0],
                right_start: [a, 0.0, -2.0],
                left_end: [b, 0.0, 2.0],
                right_end: [b, 0.0, -2.0],
                edge_tangents: [[4.0, 0.0, 0.0]; 4],
            });
        }
        g.reindex().unwrap();
        g
    }

    #[test]
    fn round_trip_keeps_every_record() {
        let g = synthetic();
        let bytes = g.to_bytes();
        assert_eq!(bytes.len(), HEADER + DISTRICT + 2 * SEGMENT + 3 * PIECE + JUNCTION + 8 * END + CONNECTOR + 2 * 4);
        let back = RoadGraph::parse(&bytes).unwrap();
        assert_eq!(back, g);
        assert_eq!(back.to_bytes(), bytes);
    }

    #[test]
    fn lookups_follow_lanes_through_the_junction() {
        let g = RoadGraph::parse(&synthetic().to_bytes()).unwrap();
        let s1 = g.segment(1).unwrap();
        assert_eq!(g.segment_pieces(s1).len(), 2);
        assert!(s1.pieces_complete());
        let j = g.junction(0xA).unwrap();
        assert_eq!(g.lane_connectors(j, 0, 0), &[0]);
        assert_eq!(g.lane_connectors(j, 6, 0), &[0]);
        assert!(g.lane_connectors(j, 1, 0).is_empty());
        let next = g.next_connectors(s1, 0);
        assert_eq!(next.len(), 1);
        assert_eq!(g.connector_exit(next[0]).map(|s| s.id), Some(2));
        assert_eq!(g.district("Test").unwrap().segments, 0..2);
    }

    #[test]
    fn traffic_input_builds_a_network() {
        use skate_core::living_world::traffic::{RoadNetwork, SegmentId};
        let g = synthetic();
        let net = RoadNetwork::build(&g.traffic_input()).unwrap();
        let s1 = net.segment_index(SegmentId(1)).unwrap();
        assert_eq!(net.segments[s1].pieces.len(), 2);
        assert!(net.junctions[0].signalled);
        let next = net.next_connectors(s1, 0);
        assert_eq!(next.len(), 1);
        assert_eq!(net.segments[net.connector_exit(next[0]).unwrap()].id, SegmentId(2));
        assert_eq!(net.connectors[next[0]].entry_speed, 13.888889);
    }

    /// Doc 26 "Cars flying off": the districts are separate worlds overlapping in x / z. A world
    /// gets only its own district's roads; another district's segment (here 17 m up, like the
    /// University roads over DownTown) and the junction ends leading to it are left out, so the
    /// census can neither place a car on it nor route one onto it.
    #[test]
    fn district_traffic_input_keeps_only_that_districts_roads() {
        use skate_core::living_world::traffic::{RoadNetwork, SegmentId};
        let mut g = synthetic();
        g.segments[1].district = 1;
        let p = &mut g.pieces[2];
        for v in [&mut p.left_start, &mut p.right_start, &mut p.left_end, &mut p.right_end] {
            v[1] = 17.0;
        }
        g.districts = vec![District { name: "Test".into(), segments: 0..1, junctions: 0..1 }, District { name: "Other".into(), segments: 1..2, junctions: 1..1 }];
        let whole = RoadNetwork::build(&g.traffic_input()).unwrap();
        assert!(whole.segment_index(SegmentId(2)).is_some(), "the whole file still holds both");
        let net = RoadNetwork::build(&g.district_traffic_input("Test").unwrap()).unwrap();
        assert!(net.segment_index(SegmentId(2)).is_none(), "another district's road joined the world");
        let s1 = net.segment_index(SegmentId(1)).unwrap();
        let next = net.next_connectors(s1, 0);
        assert_eq!(next.len(), 1);
        assert_eq!(net.connector_exit(next[0]), None, "the connector into the other district is a map edge");
        let other = RoadNetwork::build(&g.district_traffic_input("Other").unwrap()).unwrap();
        assert_eq!(other.segments.len(), 1);
        assert!(other.junctions.is_empty());
        assert!(g.district_traffic_input("NoSuchDistrict").is_none());
    }

    #[test]
    fn curve_points_and_arc_parameters() {
        let c = straight([0.0, 0.0, 0.0], [15.0, 0.0, 0.0]);
        assert_eq!(c.point(0.0), [0.0, 0.0, 0.0]);
        assert_eq!(c.point(1.0), [15.0, 0.0, 0.0]);
        assert!((c.point(0.5)[0] - 7.5).abs() < 1e-5);
        assert!((c.parameter_at(7.5) - 0.5).abs() < 1e-5);
        assert_eq!(c.parameter_at(99.0), 1.0);
    }

    #[test]
    fn rejects_bad_magic_size_and_references() {
        let bytes = synthetic().to_bytes();
        assert!(RoadGraph::parse(b"LWROADS\0\x01\0\0\0").is_err());
        assert!(RoadGraph::parse(&bytes[..bytes.len() - 4]).is_err());
        let mut wrong = bytes.clone();
        wrong[8] = 3; // version
        assert!(RoadGraph::parse(&wrong).is_err());
        let mut g = synthetic();
        g.connectors[0].junction = 5;
        assert!(RoadGraph::parse(&g.to_bytes()).is_err());
        let mut g = synthetic();
        g.segments[1].pieces = 2..9;
        assert!(RoadGraph::parse(&g.to_bytes()).is_err());
        let mut g = synthetic();
        g.segments[1].id = 1;
        assert!(RoadGraph::parse(&g.to_bytes()).is_err(), "duplicate ids");
    }
}
