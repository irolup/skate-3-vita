//! Static stock assets only. Registration follows package source occurrence
//! order; native octree order is retained within each registered section.
use super::{octree::{Bounds, Octree}, spline};
use skate_core::physics::grind_contact::Primitive;
use skate_data::skate_map::SkateMap;
use serde_json::Value;
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct SourceIdentity {
    pub stream_file: String,
    pub asset_id: String,
    pub section_index: u64,
    pub section_offset: u64,
}
struct Asset {
    #[cfg(test)]
    source: SourceIdentity,
    bounds: Bounds,
    indices: Vec<usize>,
    tree: Octree,
}
pub(crate) struct StaticProvider {
    primitives: Vec<Primitive>,
    metadata: Vec<spline::PrimitiveMetadata>,
    #[cfg(test)]
    rail_guids: Vec<[u64; 2]>,
    assets: Vec<Asset>,
    #[cfg(test)]
    source_for_primitive: Vec<usize>,
    #[cfg(test)]
    source_rail_indices: Vec<u64>,
}

impl StaticProvider {
    /// WMET preserves source section identities even when spline IDs repeat.
    /// Missing provenance is an explicit conversion prerequisite, not a ray miss.
    pub fn new(map: Option<&SkateMap>) -> Result<Self, String> {
        let Some(map) = map else {
            return Ok(Self { primitives: vec![], metadata: vec![], #[cfg(test)] rail_guids: vec![], assets: vec![], #[cfg(test)] source_for_primitive: vec![], #[cfg(test)] source_rail_indices: vec![] });
        };
        if map.rails.is_empty() {
            return Ok(Self { primitives: vec![], metadata: vec![], #[cfg(test)] rail_guids: vec![], assets: vec![], #[cfg(test)] source_for_primitive: vec![], #[cfg(test)] source_rail_indices: vec![] });
        }
        if map.rails.iter().all(|rail| rail.native.is_none()) { return Self::authored(&map.rails); }
        let mut metadata = map.extensions.iter().filter(|e| e.tag == *b"WMET");
        let extension = metadata.next().ok_or("Stock grind provider requires WMET source identity metadata")?;
        if extension.schema != 1 || metadata.next().is_some() { return Err("Expected exactly one WMET schema 1 extension".into()); }
        let manifest: Value = serde_json::from_slice(&extension.payload).map_err(|e| format!("Invalid WMET: {e}"))?;
        if manifest["grind_coordinate_policy"]["mode"].as_str() != Some("world_space") {
            return Err("Static grind provider requires verified world_space spline payloads".into());
        }
        let records = manifest["grind_splines"].as_array().ok_or("WMET grind_splines array missing")?;
        if records.len() != map.rails.len() { return Err("WMET/package rail count mismatch".into()); }
        let bytes = spline::build(Some(map))?;
        let (primitives, metadata) = spline::decoded_from_blob(&bytes)?;
        let word = |at: usize| u32::from_be_bytes(bytes[at..at+4].try_into().unwrap());
        let mut grouped: Vec<(SourceIdentity, Vec<usize>)> = Vec::new();
        let mut source_index = HashMap::new();
        let mut spatial_bounds = Vec::with_capacity(primitives.len());
        #[cfg(test)]
        let mut source_for_primitive = Vec::with_capacity(primitives.len());
        #[cfg(test)]
        let mut source_rail_indices = Vec::with_capacity(primitives.len());
        #[cfg(test)]
        let mut rail_guids = Vec::with_capacity(map.rails.len());
        let mut ordinal = 0;
        for (rail, (record, package_rail)) in records.iter().zip(&map.rails).enumerate() {
            if package_rail.native.is_none() { return Err(format!("Stock rail {} lacks native cubic payload", package_rail.name)); }
            let source = SourceIdentity {
                stream_file: string(record, "stream_file")?, asset_id: string(record, "asset_id")?,
                section_index: number(record, "section_index")?, section_offset: number(record, "section_offset")?,
            };
            let source_rail = number(record, "rail_index")?;
            let expected_name = format!("{}_{}_{}", source.asset_id, source.section_index, source_rail);
            if package_rail.name != expected_name { return Err(format!("Stock rail provenance/name mismatch: {}", package_rail.name)); }
            let header = 16+rail*32;
            let first = word(header+20) as usize;
            let last = word(header+24) as usize;
            let count = (last-first)/144+1;
            let spline_id = ((word(header) as u64)<<32)|word(header+4) as u64;
            let type_signature = ((word(header+8) as u64)<<32)|word(header+12) as u64;
            #[cfg(test)]
            rail_guids.push([spline_id, type_signature]);
            if parse_id(record, "spline_id")? != spline_id || number(record, "segment_count")? != count as u64
                || parse_id(record, "type_signature")? != type_signature
                || number(record, "flags")? != word(header+16) as u64
                || number(record, "trailing_word")? != word(header+28) as u64
                || record["closed"].as_bool() != Some(package_rail.closed) {
                return Err(format!("Stock rail native identity/count mismatch: {}", package_rail.name));
            }
            let asset = *source_index.entry(source.clone()).or_insert_with(|| {
                let index = grouped.len(); grouped.push((source, Vec::new())); index
            });
            for segment in 0..count {
                let at = first+144*segment;
                let bounds = Bounds {
                    min: std::array::from_fn(|i| f32::from_bits(word(at+80+i*4))),
                    max: std::array::from_fn(|i| f32::from_bits(word(at+96+i*4))),
                };
                spatial_bounds.push(bounds.identity_transformed());
                grouped[asset].1.push(ordinal);
                #[cfg(test)]
                source_for_primitive.push(asset);
                #[cfg(test)]
                source_rail_indices.push(source_rail);
                ordinal+=1;
            }
        }
        let assets = grouped.into_iter().map(|(_source, indices)| {
            let bounds = indices.iter().map(|&i| spatial_bounds[i]).reduce(Bounds::union)
                .ok_or("Empty stock grind source section")?.padded();
            let tree = Octree::new(bounds, indices.iter().map(|&i| spatial_bounds[i]).collect())?;
            Ok(Asset { #[cfg(test)] source: _source, bounds, indices, tree })
        }).collect::<Result<_, String>>()?;
        Ok(Self { primitives, metadata, #[cfg(test)] rail_guids, assets, #[cfg(test)] source_for_primitive, #[cfg(test)] source_rail_indices })
    }

    /// Explicit host-authored polylines have no retail source section identity.
    pub fn authored(rails: &[skate_data::skate_map::Rail]) -> Result<Self, String> {
        let bytes = spline::build_rails(rails)?;
        let (primitives, metadata) = spline::decoded_from_blob(&bytes)?;
        #[cfg(test)]
        let rail_guids = (0..rails.len()).map(|i| {
            let at = 16 + i * 32;
            [u64::from_be_bytes(bytes[at..at+8].try_into().unwrap()),
             u64::from_be_bytes(bytes[at+8..at+16].try_into().unwrap())]
        }).collect();
        let authored_bounds: Vec<_> = primitives.iter().map(|p| Bounds {
            min: std::array::from_fn(|i| p.start[i].min(p.end[i])),
            max: std::array::from_fn(|i| p.start[i].max(p.end[i])),
        }).collect();
        let assets = if let Some(bounds) = authored_bounds.iter().copied().reduce(Bounds::union) {
            let bounds = bounds.padded();
            vec![Asset {
                #[cfg(test)]
                source: SourceIdentity { stream_file: String::new(), asset_id: "host-authored".into(), section_index: 0, section_offset: 0 },
                bounds, indices: (0..primitives.len()).collect(),
                tree: Octree::new(bounds, authored_bounds.clone())?,
            }]
        } else { vec![] };
        #[cfg(test)]
        let source_rail_indices = primitives.iter().map(|p| p.owner - 1).collect();
        Ok(Self { #[cfg(test)] source_for_primitive: vec![0; primitives.len()], primitives, metadata, #[cfg(test)] rail_guids, assets, #[cfg(test)] source_rail_indices })
    }

    pub fn primitives(&self) -> &[Primitive] { &self.primitives }

    pub fn metadata(&self, primitive: usize) -> Option<&spline::PrimitiveMetadata> {
        self.metadata.get(primitive)
    }

    /// Resolve contact's map-local header handle without confusing it with GUID.
    #[cfg(test)]
    pub fn spline_guids(&self, owner: u64) -> Option<[u64; 2]> {
        let rail = usize::try_from(owner.checked_sub(1)?).ok()?;
        self.rail_guids.get(rail).copied()
    }

    #[cfg(test)]
    pub fn source_rail_index(&self, primitive: usize) -> Option<u64> {
        self.source_rail_indices.get(primitive).copied()
    }

    #[cfg(test)]
    pub fn source(&self, primitive: usize) -> Option<&SourceIdentity> {
        self.source_for_primitive.get(primitive).map(|&asset| &self.assets[asset].source)
    }

    /// S3 82C1EAD8 static pass only: query gate, ordered asset overlap and
    /// native per-asset octree traversal, at most forty original vector indices.
    /// Does not implement or pretend to query unavailable moving providers.
    pub fn query(&self, min: [f32; 3], max: [f32; 3]) -> Result<Vec<usize>, String> {
        self.query_capped(min, max, 40)
    }

    fn query_capped(&self, min: [f32; 3], max: [f32; 3], cap: usize) -> Result<Vec<usize>, String> {
        if !admitted(min, max)? { return Ok(vec![]); }
        let query = Bounds { min, max };
        let mut result = Vec::new();
        for asset in &self.assets {
            if result.len() >= cap { break; }
            if !asset.bounds.overlaps(query) { continue; }
            for local in asset.tree.query(query, cap-result.len()) { result.push(asset.indices[local]); }
        }
        Ok(result)
    }
}

/// 82C1EAD8 query gate: finite ordered box, squared diagonal above 2^-16.
fn admitted(min: [f32; 3], max: [f32; 3]) -> Result<bool, String> {
    if (0..3).any(|i| !min[i].is_finite() || !max[i].is_finite() || min[i] > max[i]) {
        return Err("Invalid grind query bounds".into());
    }
    let delta: [f32; 3] = std::array::from_fn(|i| min[i]-max[i]);
    let square = (delta[0]*delta[0]+delta[1]*delta[1])+delta[2]*delta[2];
    Ok(square > f32::from_bits(0x3780_0000))
}

/// One rail record of the shared query: contact primitive plus side metadata.
#[derive(Clone, Copy, Debug)]
pub(crate) struct RailHit {
    pub primitive: Primitive,
    pub metadata: spline::PrimitiveMetadata,
}

impl RailHit {
    pub(crate) fn prop(primitive: Primitive, guids: [u64; 2], segment_index: u32) -> Self {
        // 82C4C4E0 merges an unset stack word into bit 31; flags 0 is the port's value (research "Open items").
        Self { primitive, metadata: spline::PrimitiveMetadata { spline_guids: guids, segment_index, flags: 0 } }
    }
}

/// Named rules of the shared rail query; retail values by default, mod-tunable.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct RailRules {
    /// 82C4C4E0 skips a prop whose origin is this far from the query box centre (0x822F8F68 = 225, squared).
    pub prop_radius: f32,
    /// Records per query, static and providers together (82C1EAD8).
    pub cap: usize,
    /// Whether the prop provider runs at all.
    pub props: bool,
    /// Whether the living-world car provider runs at all (retail: the second provider, LWVehicleMan).
    pub cars: bool,
}

impl Default for RailRules {
    fn default() -> Self { Self { prop_radius: 15.0, cap: 40, props: true, cars: true } }
}

/// A moving rail provider (props; later cars): appends records in its own order up to `cap`.
pub(crate) trait RailSource {
    fn append_rails(&self, min: [f32; 3], max: [f32; 3], skip: Option<u32>, rules: &RailRules, out: &mut Vec<RailHit>);
    /// The body's model name for logs ("DMO_DT_BenchFlat_1001"), when the source knows it.
    fn describe(&self, _id: u32) -> Option<String> {
        None
    }
}

/// 82C1EAD8 (via 826C0630): static rails first, then the prop provider, then the car provider, one shared cap.
#[derive(Clone, Copy)]
pub(crate) struct RailView<'a> {
    pub rails: &'a StaticProvider,
    pub props: Option<&'a dyn RailSource>,
    pub cars: Option<&'a dyn RailSource>,
    /// The querying skater's held prop: its rails are left out (the carrier
    /// runs no rail query in retail Object Move; carried props cannot be grinded).
    pub skip: Option<u32>,
    pub rules: RailRules,
}

impl<'a> RailView<'a> {
    /// A rail owner for the log: `describe_owner` plus the prop's model name when the prop source knows it.
    pub(crate) fn describe(&self, owner: u64) -> String {
        let text = super::props::describe_owner(owner);
        if owner & super::props::CAR_OWNER_TAG == 0 && owner & super::props::PROP_OWNER_TAG != 0 {
            let id = ((owner >> 16) & 0xFFFF_FFFF) as u32;
            if let Some(name) = self.props.and_then(|p| p.describe(id)) {
                return format!("{text} ({name})");
            }
        }
        text
    }

    pub(crate) fn statics(rails: &'a StaticProvider) -> Self {
        Self { rails, props: None, cars: None, skip: None, rules: RailRules::default() }
    }

    pub(crate) fn query(&self, min: [f32; 3], max: [f32; 3]) -> Result<Vec<RailHit>, String> {
        if !admitted(min, max)? { return Ok(vec![]); }
        let mut hits = self.rails.query_capped(min, max, self.rules.cap)?.into_iter().map(|i| RailHit {
            primitive: self.rails.primitives[i],
            metadata: self.rails.metadata[i],
        }).collect::<Vec<_>>();
        if let Some(props) = self.props.filter(|_| self.rules.props && hits.len() < self.rules.cap) {
            props.append_rails(min, max, self.skip, &self.rules, &mut hits);
        }
        if let Some(cars) = self.cars.filter(|_| self.rules.cars && hits.len() < self.rules.cap) {
            cars.append_rails(min, max, None, &self.rules, &mut hits);
        }
        hits.truncate(self.rules.cap);
        Ok(hits)
    }
}

fn string(value: &Value, key: &str) -> Result<String, String> {
    value[key].as_str().map(str::to_owned).ok_or_else(|| format!("WMET rail missing string {key}"))
}
fn number(value: &Value, key: &str) -> Result<u64, String> {
    value[key].as_u64().ok_or_else(|| format!("WMET rail missing unsigned {key}"))
}
fn parse_id(value: &Value, key: &str) -> Result<u64, String> {
    let value = string(value, key)?;
    u64::from_str_radix(value.strip_prefix("0x").unwrap_or(&value), 16)
        .map_err(|_| format!("Invalid WMET {key}"))
}
