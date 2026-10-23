//! Grind rails carried by dynamic props (DMOs), retail provider 82C4C4E0.
//! Each DMO part keeps its rails in its own model space (setup 82C535C0 fills
//! the rail map at DMO+1796 from the part's EB001B list); every query
//! re-places them with the prop's current matrix, so a moved or knocked prop
//! takes its rails along and no grind state is attached to the prop.
use serde_json::Value;

/// One cubic segment in template space: the contact chord D -> (A+B)+(C+D)
/// (four lanes, like the static decoder) and the authored segment bounds (+80/+96).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PropSegment {
    pub start: [f32; 4],
    pub end: [f32; 4],
    pub min: [f32; 3],
    pub max: [f32; 3],
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PropRail {
    /// Header +0 / +8 (`spline_id`, `type_signature`).
    pub guids: [u64; 2],
    pub segments: Vec<PropSegment>,
}

/// Owner tag of prop rail records. Retail's owner is the rail block; ours is
/// (prop id, rail index), stable across ticks and peers, and never equal to a
/// static owner (1-based rail index).
pub(crate) const PROP_OWNER_TAG: u64 = 1 << 62;
/// Owner tag of living-world car rail records: (car serial, rail index).
pub(crate) const CAR_OWNER_TAG: u64 = 1 << 63;

/// Owner tag of skate.Park piece rail records: (piece id, rail index). Below [`PROP_OWNER_TAG`]: the pieces are fixed,
/// and owners from that tag up count as moving rails (air_trajectory/grind.rs, player_input/grind/pre.rs).
pub(crate) const PIECE_OWNER_TAG: u64 = 1 << 61;

pub(crate) fn piece_owner(piece: u32, rail: usize) -> u64 {
    PIECE_OWNER_TAG | (u64::from(piece) << 16) | (rail as u64 & 0xFFFF)
}

pub(crate) fn prop_owner(prop: u32, rail: usize) -> u64 {
    PROP_OWNER_TAG | (u64::from(prop) << 16) | (rail as u64 & 0xFFFF)
}

pub(crate) fn car_owner(serial: u32, rail: usize) -> u64 {
    CAR_OWNER_TAG | (u64::from(serial) << 16) | (rail as u64 & 0xFFFF)
}

/// The prop body id of a prop rail owner.
pub(crate) fn prop_of_owner(owner: u64) -> Option<u32> {
    (owner & PROP_OWNER_TAG != 0 && owner & CAR_OWNER_TAG == 0).then_some((owner >> 16) as u32)
}

/// `map rail=N`, `prop=ID rail=N` or `car=SERIAL rail=N` for logs.
pub(crate) fn describe_owner(owner: u64) -> String {
    let (id, rail) = ((owner >> 16) & 0x3FFF_FFFF_FFFF, owner & 0xFFFF);
    if owner & (PIECE_OWNER_TAG | PROP_OWNER_TAG | CAR_OWNER_TAG) == PIECE_OWNER_TAG {
        format!("park piece={id} rail={rail}")
    } else if owner & CAR_OWNER_TAG != 0 {
        format!("car={id} rail={rail}")
    } else if owner & PROP_OWNER_TAG != 0 {
        format!("prop={id} rail={rail}")
    } else {
        format!("map rail={owner}")
    }
}

/// One prop's rails at one pose: origin, basis columns and the body's axis scale (the grab-spline frame:
/// world = origin + basis * (point * scale)), from the last completed physics step.
#[derive(Clone, Debug)]
pub(crate) struct PosedRails {
    pub id: u32,
    /// [`PROP_OWNER_TAG`] or [`CAR_OWNER_TAG`].
    pub tag: u64,
    pub origin: [f32; 3],
    pub basis: [[f32; 3]; 3],
    pub scale: [f32; 3],
    pub rails: std::sync::Arc<[PropRail]>,
}

impl PosedRails {
    fn place(&self, p: [f32; 3]) -> [f32; 3] {
        let q = [p[0] * self.scale[0], p[1] * self.scale[1], p[2] * self.scale[2]];
        std::array::from_fn(|i| self.origin[i] + self.basis[0][i] * q[0] + self.basis[1][i] * q[1] + self.basis[2][i] * q[2])
    }

    /// 82C4C4E0 for one prop: skipped past `prop_radius` from the box centre; each segment's bounds placed by the
    /// pose (8258C1E0) and tested against the box; chord endpoints through the same pose. Returns false once the
    /// shared cap is reached.
    pub(crate) fn append(&self, min: [f32; 3], max: [f32; 3], rules: &super::RailRules, out: &mut Vec<super::RailHit>) -> bool {
        let d: [f32; 3] = std::array::from_fn(|i| self.origin[i] - (min[i] + max[i]) * 0.5);
        if !(d[0] * d[0] + d[1] * d[1] + d[2] * d[2] < rules.prop_radius * rules.prop_radius) {
            return true;
        }
        for (index, rail) in self.rails.iter().enumerate() {
            for (segment_index, segment) in rail.segments.iter().enumerate() {
                let mut lo = [f32::INFINITY; 3];
                let mut hi = [f32::NEG_INFINITY; 3];
                for corner in 0..8 {
                    let p = self.place(std::array::from_fn(|i| if corner >> i & 1 == 0 { segment.min[i] } else { segment.max[i] }));
                    for i in 0..3 {
                        lo[i] = lo[i].min(p[i]);
                        hi[i] = hi[i].max(p[i]);
                    }
                }
                if (0..3).any(|i| hi[i] < min[i] || lo[i] > max[i]) {
                    continue;
                }
                let [start, end] = [segment.start, segment.end].map(|p| {
                    let w = self.place([p[0], p[1], p[2]]);
                    [w[0], w[1], w[2], p[3]]
                });
                let primitive = skate_core::physics::grind_contact::Primitive { start, end, owner: self.tag | (u64::from(self.id) << 16) | (index as u64 & 0xFFFF) };
                out.push(super::RailHit::prop(primitive, rail.guids, segment_index as u32));
                if out.len() >= rules.cap {
                    return false;
                }
            }
        }
        true
    }
}

/// Every prop's (or car's) rails at the last stepped pose, id order: the cars every tick, and the props for
/// consumers that do not reach the prop store (air trajectory). Same records as querying the store directly.
#[derive(Clone, Debug, Default)]
pub(crate) struct PropRailPoses(pub Vec<PosedRails>);

impl super::RailSource for PropRailPoses {
    fn append_rails(&self, min: [f32; 3], max: [f32; 3], skip: Option<u32>, rules: &super::RailRules, out: &mut Vec<super::RailHit>) {
        for posed in self.0.iter().filter(|p| Some(p.id) != skip) {
            if !posed.append(min, max, rules, out) {
                return;
            }
        }
    }
}

/// The exported `grind_splines` list of one template (`native-props/<map>.json`,
/// same record shape as map rails: `spline_id`, `type_signature`,
/// `native_segment_payloads` as 120-byte big-endian segments).
pub(crate) fn parse_rails(list: &Value) -> Result<Vec<PropRail>, String> {
    let list = list.as_array().ok_or("Prop grind_splines must be a list")?;
    list.iter().map(|rail| {
        let id = |key: &str| -> Result<u64, String> {
            let text = rail[key].as_str().ok_or_else(|| format!("Prop rail missing {key}"))?;
            u64::from_str_radix(text.strip_prefix("0x").unwrap_or(text), 16).map_err(|_| format!("Invalid prop rail {key}"))
        };
        let payloads = rail["native_segment_payloads"].as_array().ok_or("Prop rail missing native_segment_payloads")?;
        let segments = payloads.iter().map(|p| segment(p.as_str().ok_or("Prop rail payload must be hex")?)).collect::<Result<Vec<_>, _>>()?;
        if segments.is_empty() {
            return Err("Prop rail has no segments".into());
        }
        Ok(PropRail { guids: [id("spline_id")?, id("type_signature")?], segments })
    }).collect()
}

fn segment(hex: &str) -> Result<PropSegment, String> {
    if hex.len() != 240 || !hex.is_ascii() {
        return Err("Prop rail segment must be 120 bytes".into());
    }
    let word = |i: usize| -> Result<f32, String> {
        let value = u32::from_str_radix(&hex[i * 8..i * 8 + 8], 16).map_err(|_| "Invalid prop rail hex".to_string())?;
        let value = f32::from_bits(value);
        value.is_finite().then_some(value).ok_or_else(|| "Non-finite prop rail data".into())
    };
    let lanes = |at: usize| -> Result<[f32; 4], String> { Ok([word(at)?, word(at + 1)?, word(at + 2)?, word(at + 3)?]) };
    let vector = |at: usize| -> Result<[f32; 3], String> { Ok([word(at)?, word(at + 1)?, word(at + 2)?]) };
    let [a, b, c, d] = [lanes(0)?, lanes(4)?, lanes(8)?, lanes(12)?];
    let (min, max) = (vector(20)?, vector(24)?);
    if (0..3).any(|i| min[i] > max[i]) {
        return Err("Inverted prop rail bounds".into());
    }
    Ok(PropSegment { start: d, end: std::array::from_fn(|i| (a[i] + b[i]) + (c[i] + d[i])), min, max })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload(words: &[(usize, f32)]) -> String {
        let mut all = [0f32; 30];
        for &(i, v) in words { all[i] = v; }
        all.iter().map(|v| format!("{:08X}", v.to_bits())).collect()
    }

    #[test]
    fn exported_rails_parse_to_chords_and_bounds() {
        // Straight segment (1,2,3) -> (2,2,3): D = start, C = delta, A = B = 0.
        let hex = payload(&[(8, 1.0), (12, 1.0), (13, 2.0), (14, 3.0), (15, 1.0), (20, 1.0), (21, 2.0), (22, 3.0), (24, 2.0), (25, 2.0), (26, 3.0)]);
        let list = serde_json::json!([{ "spline_id": "0x00000000000000AB", "type_signature": "0x2C701707000F00ED", "native_segment_payloads": [hex] }]);
        let rails = parse_rails(&list).unwrap();
        assert_eq!(rails[0].guids, [0xAB, 0x2C70_1707_000F_00ED]);
        assert_eq!(rails[0].segments, vec![PropSegment { start: [1.0, 2.0, 3.0, 1.0], end: [2.0, 2.0, 3.0, 1.0], min: [1.0, 2.0, 3.0], max: [2.0, 2.0, 3.0] }]);
        let inverted = payload(&[(20, 1.0), (24, 0.0)]);
        assert!(parse_rails(&serde_json::json!([{ "spline_id": "0x1", "type_signature": "0x2", "native_segment_payloads": [inverted] }])).is_err());
        assert!(parse_rails(&serde_json::json!([{ "spline_id": "0x1", "type_signature": "0x2", "native_segment_payloads": [] }])).is_err());
        assert!(parse_rails(&serde_json::json!([{ "spline_id": "0x1", "type_signature": "0x2", "native_segment_payloads": ["00"] }])).is_err());
    }
}
