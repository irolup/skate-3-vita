//! Recorded NPC skater lines: the retail AIPATHDATA object (RenderWare type
//! 0x00EB0014) and the living-world skater path pack the setup export writes.
//!
//! Retail free roam streams these objects with the district `cSim_*.xsf` tiles;
//! `Sk8::AIPath::ThePathManager` keeps every path by its 16-byte id, and the AI
//! skater manager (`sub_8245C548`, spawn) picks lines whose start node lies in
//! its spawn ring. A path that crosses several tiles is stored, byte-identical,
//! in every tile it touches: the disc holds 3,891 path copies of 1,691 unique
//! paths (DownTown, University, Industrial; the park districts have none).
//!
//! Layout (big-endian; every `m_p*` offset is relative to the start of the
//! struct that contains it). Field names follow the Skate 2 symbols as
//! documented by DumbadsSkate3ModdingTools (Ethanw05; credits there to SunJay,
//! Dumbad, RenderWareGavin, Tuukkas); every field was re-checked against the
//! user's disc ([data], `.claude/notes/npc-skaters-re.md` section 3):
//!
//! ```text
//! AIPATHDATA  +0 u32 path count, +4 u32 offset of the path array (0x10), +8 8 pad
//! tAIPath (96 B)
//!   +0x00 vec4 bbox min, +0x10 vec4 bbox max (w = 1)
//!   +0x20 16 B m_ID (byte 0 = 0 for ambient paths, bytes 1..5 district tag)
//!   +0x30 m_pNodes, +0x34 node count, +0x38 m_pExtData (0 = none)
//!   +0x3C m_pBranchGroup, +0x40 group count
//!   +0x44 u32 m_BitFlags, +0x48 u64 m_AllowedSkaters, +0x50 i32 m_SkillLevel,
//!   +0x54 3 x i32 extra (0)
//! tAIPathNode (44 B)
//!   +0x00 vec3 position, +0x0C vec3 direction (displacement since the previous
//!   node), +0x18 board orientation, +0x1C skater orientation (4 x u8, biased
//!   quaternion), +0x20 m_pExtData (node-relative, 0 = none), +0x24 u8 frames
//!   since the previous node (60 Hz), +0x25 u8 width left, +0x26 u8 width right
//!   (255 = unbounded), +0x27 u8 event type, +0x28 u8 flags, +0x29 3 B extra
//! tAIPathNodeExtData (40 B)
//!   +0x00 vec3 trajectory start position, +0x0C vec3 trajectory start
//!   velocity, +0x18 vec3 trajectory offset, +0x24 i16 trick index (-1 none),
//!   +0x26 i8 180-degree air spin count, +0x27 u8 flags
//! tAIPathBranchGroup (12 B): +0 offset of its branches (group-relative),
//!   +4 branch count, +8 node index on this path
//! tAIPathBranch (24 B): +0 16 B target path id, +16 u32 target node index,
//!   +20 f32 position along the target
//! ```
//!
//! Bytes in, no file I/O: the setup export (tools/asset_pipeline/
//! living_world_skaters.py) writes one pack per district, and a mod can author
//! extra paths in the same retail layout.

use std::fmt;

/// RenderWare object type of the AIPATHDATA object [data].
pub const AIPATHDATA_TYPE: u32 = 0x00EB_0014;
pub const PATH_STRIDE: usize = 0x60;
pub const NODE_STRIDE: usize = 44;
pub const EXT_STRIDE: usize = 40;
pub const BRANCH_GROUP_STRIDE: usize = 12;
pub const BRANCH_STRIDE: usize = 24;
/// Recording rate of the node timeline: distance to the previous node divided
/// by `frames_since_last_node` equals |direction| (ratio 1.000 over 389,676
/// nodes) [data].
pub const RECORDING_HZ: f32 = 60.0;

/// Path flag bits. Order from the enum strings at 0x82255D7C (Race, Vert,
/// Street, Shortcut, Slowdown, AllOffBoard, SomeOffBoard, Spectate, Escape);
/// the bit for each name is inferred from that order, not yet read from code.
pub mod path_flags {
    pub const RACE: u32 = 1 << 0;
    pub const VERT: u32 = 1 << 1;
    pub const STREET: u32 = 1 << 2;
    pub const SHORTCUT: u32 = 1 << 3;
    pub const SLOWDOWN: u32 = 1 << 4;
    pub const ALL_OFF_BOARD: u32 = 1 << 5;
    pub const SOME_OFF_BOARD: u32 = 1 << 6;
    pub const SPECTATE: u32 = 1 << 7;
    pub const ESCAPE: u32 = 1 << 8;
}

/// Node flag bits (`m_i8Flags`): order of the loader strings mIsBoardFlipped,
/// mIsCrouched, mIsAirborne, mIsOffBoard (DumbadsSkate3ModdingTools reads the
/// same order from the Skate 2 symbols).
pub mod node_flags {
    pub const BOARD_FLIPPED: u8 = 1 << 0;
    pub const CROUCHED: u8 = 1 << 1;
    pub const AIRBORNE: u8 = 1 << 2;
    pub const OFF_BOARD: u8 = 1 << 3;
}

/// Extended node data flag bits.
pub mod ext_flags {
    pub const HAS_TRAJECTORY: u8 = 1 << 0;
    pub const HAS_FRONT_FLIP: u8 = 1 << 1;
    pub const HAS_BACK_FLIP: u8 = 1 << 2;
}

/// Node event types (`m_uiEventType`); 3 never occurs on the disc [data].
pub mod node_events {
    pub const NONE: u8 = 0;
    pub const START_TRICK: u8 = 1;
    pub const END_TRICK: u8 = 2;
    pub const INCIDENTAL_AIR: u8 = 4;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AiPathError {
    Truncated {
        what: &'static str,
        at: usize,
        len: usize,
    },
    BadHeader(String),
    BadPack(String),
}

impl fmt::Display for AiPathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated { what, at, len } => {
                write!(
                    f,
                    "AIPATH {what} at {at:#x} runs past the end ({len:#x} bytes)"
                )
            }
            Self::BadHeader(message) => write!(f, "AIPATH header: {message}"),
            Self::BadPack(message) => write!(f, "skater path pack: {message}"),
        }
    }
}

impl std::error::Error for AiPathError {}

/// The 16-byte path identity (`m_ID`), the key retail's path manager and the
/// branch records use. Stable across tiles, so it is also the key a mod uses to
/// override or remove a line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AiPathId(pub [u8; 16]);

impl AiPathId {
    /// District tag of ambient paths (`dwtn`, `univ`, `indu`), bytes 1..5.
    pub fn tag(&self) -> [u8; 4] {
        [self.0[1], self.0[2], self.0[3], self.0[4]]
    }

    pub fn tag_str(&self) -> String {
        self.tag()
            .iter()
            .map(|&b| if b.is_ascii_graphic() { b as char } else { '?' })
            .collect()
    }

    /// Byte 0 is 0 on every ambient (free-roam) path [data]; the Skate 2 filter
    /// `AIPathID::CompareAgainstFilter` takes its ambient branch on it.
    pub fn is_ambient(&self) -> bool {
        self.0[0] == 0
    }

    /// 32 lowercase hex digits: the string id used in exports and by mods.
    pub fn to_hex(&self) -> String {
        self.0.iter().map(|b| format!("{b:02x}")).collect()
    }

    pub fn from_hex(text: &str) -> Option<Self> {
        if text.len() != 32 || !text.is_ascii() {
            return None;
        }
        let mut id = [0; 16];
        for (i, byte) in id.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&text[i * 2..i * 2 + 2], 16).ok()?;
        }
        Some(Self(id))
    }
}

impl fmt::Display for AiPathId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AiPathNode {
    pub position: [f32; 3],
    /// Displacement from the previous node (zero on the first node).
    pub direction: [f32; 3],
    pub board_orientation: [u8; 4],
    pub skater_orientation: [u8; 4],
    /// Index into [`AiPath::extended`].
    pub extended: Option<usize>,
    pub frames_since_last_node: u8,
    pub width_left: u8,
    pub width_right: u8,
    pub event: u8,
    pub flags: u8,
    pub extra: [u8; 3],
}

impl AiPathNode {
    pub fn has_flag(&self, bit: u8) -> bool {
        self.flags & bit != 0
    }
}

/// A recorded jump / trick slot (`tAIPathNodeExtData`).
#[derive(Debug, Clone, PartialEq)]
pub struct AiPathNodeExt {
    pub trajectory_start_position: [f32; 3],
    pub trajectory_start_velocity: [f32; 3],
    pub trajectory_offset: [f32; 3],
    /// Trick index, -1 when none.
    pub trick_index: i16,
    pub air_spin_180_count: i8,
    pub flags: u8,
}

/// Where this line continues into another one (`tAIPathBranch`).
#[derive(Debug, Clone, PartialEq)]
pub struct AiPathBranch {
    pub target: AiPathId,
    pub target_node: u32,
    /// f32 in 0..=1 on the disc; meaning (position along the target, or a
    /// choice weight) still open (M5).
    pub weight: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AiPathBranchGroup {
    /// Node index on the owning path where the branches start.
    pub node: u32,
    pub branches: Vec<AiPathBranch>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AiPath {
    pub id: AiPathId,
    pub bbox_min: [f32; 3],
    pub bbox_max: [f32; 3],
    /// `m_BitFlags` ([`path_flags`]); retail compares bits 0..2 with the
    /// profile's capability bytes (`sub_8245B068`, path +68).
    pub flags: u32,
    /// `m_AllowedSkaters`: bit n allows the profile with pro index n
    /// (`sub_8245B068`, path +72).
    pub allowed_skaters: u64,
    pub skill_level: i32,
    pub extra: [i32; 3],
    pub nodes: Vec<AiPathNode>,
    /// Extended records in disc order; nodes refer to them by index.
    pub extended: Vec<AiPathNodeExt>,
    pub branch_groups: Vec<AiPathBranchGroup>,
}

impl AiPath {
    pub fn tag(&self) -> [u8; 4] {
        self.id.tag()
    }

    /// The node retail measures the spawn ring and the 5 m rule against
    /// (path +48 -> node 0, `sub_82459480`).
    pub fn start(&self) -> Option<[f32; 3]> {
        self.nodes.first().map(|n| n.position)
    }

    pub fn allows(&self, pro_index: u32) -> bool {
        pro_index < 64 && self.allowed_skaters & (1u64 << pro_index) != 0
    }

    /// Total recorded duration in 60 Hz frames.
    pub fn frames(&self) -> u64 {
        self.nodes
            .iter()
            .skip(1)
            .map(|n| u64::from(n.frames_since_last_node))
            .sum()
    }

    /// Polyline length in metres.
    pub fn length(&self) -> f32 {
        self.nodes
            .windows(2)
            .map(|w| distance(w[0].position, w[1].position))
            .sum()
    }
}

/// Decoded orientation of a node: 4 bytes, each `(b - 128) / 127`, in disc
/// order, renormalised. [data] The raw norm is 0.997 median (p1 0.989, p99
/// 1.011) over the disc's nodes; the component order (likely x, y, z, w) is not
/// yet confirmed from code.
pub fn decode_orientation(raw: [u8; 4]) -> [f32; 4] {
    let q = raw.map(|b| (f32::from(b) - 128.0) / 127.0);
    let norm = q.iter().map(|c| c * c).sum::<f32>().sqrt();
    if norm > 1e-6 {
        q.map(|c| c / norm)
    } else {
        [0.0, 0.0, 0.0, 1.0]
    }
}

pub fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

struct Reader<'a> {
    data: &'a [u8],
}

impl<'a> Reader<'a> {
    fn bytes(&self, what: &'static str, at: usize, len: usize) -> Result<&'a [u8], AiPathError> {
        at.checked_add(len)
            .and_then(|end| self.data.get(at..end))
            .ok_or(AiPathError::Truncated {
                what,
                at,
                len: self.data.len(),
            })
    }
    fn u32(&self, what: &'static str, at: usize) -> Result<u32, AiPathError> {
        Ok(u32::from_be_bytes(
            self.bytes(what, at, 4)?.try_into().unwrap(),
        ))
    }
    fn i32(&self, what: &'static str, at: usize) -> Result<i32, AiPathError> {
        Ok(self.u32(what, at)? as i32)
    }
    fn f32(&self, what: &'static str, at: usize) -> Result<f32, AiPathError> {
        Ok(f32::from_bits(self.u32(what, at)?))
    }
    fn u64(&self, what: &'static str, at: usize) -> Result<u64, AiPathError> {
        Ok(u64::from_be_bytes(
            self.bytes(what, at, 8)?.try_into().unwrap(),
        ))
    }
    fn vec3(&self, what: &'static str, at: usize) -> Result<[f32; 3], AiPathError> {
        Ok([
            self.f32(what, at)?,
            self.f32(what, at + 4)?,
            self.f32(what, at + 8)?,
        ])
    }
    fn id(&self, what: &'static str, at: usize) -> Result<AiPathId, AiPathError> {
        Ok(AiPathId(self.bytes(what, at, 16)?.try_into().unwrap()))
    }
    /// A self-relative offset: `base + offset`, checked.
    fn relative(&self, what: &'static str, base: usize, offset: u32) -> Result<usize, AiPathError> {
        base.checked_add(offset as usize)
            .filter(|&at| at <= self.data.len())
            .ok_or(AiPathError::Truncated {
                what,
                at: base,
                len: self.data.len(),
            })
    }
}

/// Parse one AIPATHDATA blob (the object bytes as stored in the arena).
pub fn parse(data: &[u8]) -> Result<Vec<AiPath>, AiPathError> {
    let r = Reader { data };
    let count = r.u32("path count", 0)? as usize;
    let array = r.u32("path array offset", 4)? as usize;
    if array < 8 {
        return Err(AiPathError::BadHeader(format!(
            "path array offset {array:#x}"
        )));
    }
    // Every path needs at least its header: reject absurd counts before allocating.
    if count > data.len() / PATH_STRIDE {
        return Err(AiPathError::BadHeader(format!(
            "{count} paths in {} bytes",
            data.len()
        )));
    }
    (0..count)
        .map(|i| parse_path(&r, array + i * PATH_STRIDE))
        .collect()
}

fn parse_path(r: &Reader, at: usize) -> Result<AiPath, AiPathError> {
    r.bytes("path header", at, PATH_STRIDE)?;
    let min = r.vec3("bbox", at)?;
    let max = r.vec3("bbox", at + 0x10)?;
    let id = r.id("path id", at + 0x20)?;
    let nodes_at = r.relative("nodes", at, r.u32("nodes", at + 0x30)?)?;
    let node_count = r.u32("node count", at + 0x34)? as usize;
    let groups_offset = r.u32("branch groups", at + 0x3C)?;
    let group_count = r.u32("group count", at + 0x40)? as usize;
    r.bytes(
        "nodes",
        nodes_at,
        node_count.checked_mul(NODE_STRIDE).unwrap_or(usize::MAX),
    )?;

    let mut extended = Vec::new();
    let mut ext_index = std::collections::HashMap::new();
    let mut nodes = Vec::with_capacity(node_count);
    for k in 0..node_count {
        let o = nodes_at + k * NODE_STRIDE;
        let ext_offset = r.u32("node ext", o + 0x20)?;
        let ext = if ext_offset == 0 {
            None
        } else {
            let e = r.relative("node ext", o, ext_offset)?;
            let index = match ext_index.get(&e) {
                Some(&index) => index,
                None => {
                    extended.push(parse_ext(r, e)?);
                    ext_index.insert(e, extended.len() - 1);
                    extended.len() - 1
                }
            };
            Some(index)
        };
        let tail = r.bytes("node", o + 0x24, 8)?;
        nodes.push(AiPathNode {
            position: r.vec3("node", o)?,
            direction: r.vec3("node", o + 0x0C)?,
            board_orientation: r.bytes("node", o + 0x18, 4)?.try_into().unwrap(),
            skater_orientation: r.bytes("node", o + 0x1C, 4)?.try_into().unwrap(),
            extended: ext,
            frames_since_last_node: tail[0],
            width_left: tail[1],
            width_right: tail[2],
            event: tail[3],
            flags: tail[4],
            extra: [tail[5], tail[6], tail[7]],
        });
    }

    let mut branch_groups = Vec::with_capacity(group_count.min(1024));
    if group_count > 0 {
        let groups_at = r.relative("branch groups", at, groups_offset)?;
        for g in 0..group_count {
            let go = groups_at + g * BRANCH_GROUP_STRIDE;
            let branches_at = r.relative("branches", go, r.u32("branch group", go)?)?;
            let branch_count = r.u32("branch group", go + 4)? as usize;
            let node = r.u32("branch group", go + 8)?;
            r.bytes(
                "branches",
                branches_at,
                branch_count
                    .checked_mul(BRANCH_STRIDE)
                    .unwrap_or(usize::MAX),
            )?;
            let branches = (0..branch_count)
                .map(|b| {
                    let bo = branches_at + b * BRANCH_STRIDE;
                    Ok(AiPathBranch {
                        target: r.id("branch", bo)?,
                        target_node: r.u32("branch", bo + 16)?,
                        weight: r.f32("branch", bo + 20)?,
                    })
                })
                .collect::<Result<_, AiPathError>>()?;
            branch_groups.push(AiPathBranchGroup { node, branches });
        }
    }

    Ok(AiPath {
        id,
        bbox_min: min,
        bbox_max: max,
        flags: r.u32("flags", at + 0x44)?,
        allowed_skaters: r.u64("allowed skaters", at + 0x48)?,
        skill_level: r.i32("skill level", at + 0x50)?,
        extra: [
            r.i32("extra", at + 0x54)?,
            r.i32("extra", at + 0x58)?,
            r.i32("extra", at + 0x5C)?,
        ],
        nodes,
        extended,
        branch_groups,
    })
}

fn parse_ext(r: &Reader, at: usize) -> Result<AiPathNodeExt, AiPathError> {
    let tail = r.bytes("ext", at + 0x24, 4)?;
    Ok(AiPathNodeExt {
        trajectory_start_position: r.vec3("ext", at)?,
        trajectory_start_velocity: r.vec3("ext", at + 0x0C)?,
        trajectory_offset: r.vec3("ext", at + 0x18)?,
        trick_index: i16::from_be_bytes([tail[0], tail[1]]),
        air_spin_180_count: tail[2] as i8,
        flags: tail[3],
    })
}

// ---------------------------------------------------------------------------
// Skater path pack (our container, written by living_world_skaters.py)
// ---------------------------------------------------------------------------

/// Pack magic. Layout (little-endian, our own format; the blobs inside stay the
/// retail big-endian bytes, verbatim):
/// ```text
/// +0  8 B  "LWSKPTH\0"
/// +8  u32  version (1)
/// +12 u32  tile count
/// +16 tile table, 48 B each: u64 asset id, u32 blob offset (from file start),
///     u32 blob length, 32 B tile stream name (NUL padded, e.g. cSim_-150_-150_high)
/// then the AIPATHDATA blobs, each 16-byte aligned
/// ```
pub const PACK_MAGIC: &[u8; 8] = b"LWSKPTH\0";
pub const PACK_VERSION: u32 = 1;
pub const PACK_TILE_STRIDE: usize = 48;
pub const PACK_NAME_LEN: usize = 32;

#[derive(Debug, Clone, PartialEq)]
pub struct PackTile<'a> {
    pub asset_id: u64,
    pub name: String,
    pub blob: &'a [u8],
}

impl PackTile<'_> {
    /// Tile grid coordinates from the stream name `cSim_<x>_<z>_<lod>`.
    pub fn grid(&self) -> Option<(i32, i32)> {
        let mut parts = self.name.strip_prefix("cSim_")?.split('_');
        Some((parts.next()?.parse().ok()?, parts.next()?.parse().ok()?))
    }
}

pub fn parse_pack(data: &[u8]) -> Result<Vec<PackTile<'_>>, AiPathError> {
    let bad = |m: String| AiPathError::BadPack(m);
    if data.len() < 16 || &data[..8] != PACK_MAGIC {
        return Err(bad("missing magic".into()));
    }
    let version = u32::from_le_bytes(data[8..12].try_into().unwrap());
    if version != PACK_VERSION {
        return Err(bad(format!("unsupported version {version}")));
    }
    let count = u32::from_le_bytes(data[12..16].try_into().unwrap()) as usize;
    let table_end = count
        .checked_mul(PACK_TILE_STRIDE)
        .and_then(|n| n.checked_add(16))
        .filter(|&end| end <= data.len())
        .ok_or_else(|| bad(format!("{count} tiles do not fit")))?;
    let mut tiles = Vec::with_capacity(count);
    for t in 0..count {
        let e = &data[16 + t * PACK_TILE_STRIDE..16 + (t + 1) * PACK_TILE_STRIDE];
        let asset_id = u64::from_le_bytes(e[0..8].try_into().unwrap());
        let offset = u32::from_le_bytes(e[8..12].try_into().unwrap()) as usize;
        let length = u32::from_le_bytes(e[12..16].try_into().unwrap()) as usize;
        let name_bytes = &e[16..16 + PACK_NAME_LEN];
        let name_len = name_bytes
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(PACK_NAME_LEN);
        let name = std::str::from_utf8(&name_bytes[..name_len])
            .map_err(|_| bad(format!("tile {t} name is not UTF-8")))?
            .to_owned();
        let blob = offset
            .checked_add(length)
            .filter(|_| offset >= table_end)
            .and_then(|end| data.get(offset..end))
            .ok_or_else(|| {
                bad(format!(
                    "tile {t} blob {offset:#x}+{length:#x} out of range"
                ))
            })?;
        tiles.push(PackTile {
            asset_id,
            name,
            blob,
        });
    }
    Ok(tiles)
}

/// Encode a pack (used by tests and tools; the setup export writes the same
/// bytes from Python).
pub fn write_pack(tiles: &[(u64, &str, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(PACK_MAGIC);
    out.extend_from_slice(&PACK_VERSION.to_le_bytes());
    out.extend_from_slice(&(tiles.len() as u32).to_le_bytes());
    let mut cursor = align16(16 + tiles.len() * PACK_TILE_STRIDE);
    for (asset_id, name, blob) in tiles {
        out.extend_from_slice(&asset_id.to_le_bytes());
        out.extend_from_slice(&(cursor as u32).to_le_bytes());
        out.extend_from_slice(&(blob.len() as u32).to_le_bytes());
        let mut field = [0u8; PACK_NAME_LEN];
        let n = name.len().min(PACK_NAME_LEN - 1);
        field[..n].copy_from_slice(&name.as_bytes()[..n]);
        out.extend_from_slice(&field);
        cursor = align16(cursor + blob.len());
    }
    for (_, _, blob) in tiles {
        out.resize(align16(out.len()), 0);
        out.extend_from_slice(blob);
    }
    out
}

fn align16(n: usize) -> usize {
    (n + 15) & !15
}

/// One unique path of a district and the pack tiles that carry a copy of it.
#[derive(Debug, Clone, PartialEq)]
pub struct DistrictPath {
    pub path: AiPath,
    pub tiles: Vec<usize>,
}

/// All unique paths of a pack, keyed like retail's path manager (by `m_ID`),
/// in first-seen order. Returns the paths and the number of copies whose
/// content differed from the first copy (0 on the stock disc; such a copy is
/// ignored, as a second insert under the same key would be).
pub fn district_paths(tiles: &[PackTile]) -> Result<(Vec<DistrictPath>, usize), AiPathError> {
    let mut order: Vec<DistrictPath> = Vec::new();
    let mut by_id = std::collections::HashMap::new();
    let mut conflicts = 0;
    for (t, tile) in tiles.iter().enumerate() {
        for path in parse(tile.blob)? {
            match by_id.get(&path.id) {
                Some(&i) => {
                    let entry: &mut DistrictPath = &mut order[i];
                    if entry.path != path {
                        conflicts += 1;
                    }
                    entry.tiles.push(t);
                }
                None => {
                    by_id.insert(path.id, order.len());
                    order.push(DistrictPath {
                        path,
                        tiles: vec![t],
                    });
                }
            }
        }
    }
    Ok((order, conflicts))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builder for synthetic blobs in the retail layout.
    struct SynthPath {
        id: [u8; 16],
        nodes: Vec<([f32; 3], [f32; 3], u8, Option<i16>)>,
        groups: Vec<(u32, Vec<([u8; 16], u32, f32)>)>,
        flags: u32,
        mask: u64,
    }

    fn put_u32(out: &mut [u8], at: usize, v: u32) {
        out[at..at + 4].copy_from_slice(&v.to_be_bytes());
    }
    fn put_f32(out: &mut [u8], at: usize, v: f32) {
        put_u32(out, at, v.to_bits());
    }

    fn build(paths: &[SynthPath]) -> Vec<u8> {
        let mut out = vec![0u8; 0x10 + paths.len() * PATH_STRIDE];
        put_u32(&mut out, 0, paths.len() as u32);
        put_u32(&mut out, 4, 0x10);
        out[8..16].fill(0xDE);
        for (i, p) in paths.iter().enumerate() {
            let h = 0x10 + i * PATH_STRIDE;
            let mut lo = [f32::MAX; 3];
            let mut hi = [f32::MIN; 3];
            for (pos, ..) in &p.nodes {
                for a in 0..3 {
                    lo[a] = lo[a].min(pos[a]);
                    hi[a] = hi[a].max(pos[a]);
                }
            }
            let nodes_at = out.len();
            out.resize(nodes_at + p.nodes.len() * NODE_STRIDE, 0);
            out.resize(align16(out.len()), 0xDE);
            let ext_at = out.len();
            let mut ext_cursor = ext_at;
            for (k, (pos, dir, frames, trick)) in p.nodes.iter().enumerate() {
                let o = nodes_at + k * NODE_STRIDE;
                for a in 0..3 {
                    put_f32(&mut out, o + a * 4, pos[a]);
                    put_f32(&mut out, o + 12 + a * 4, dir[a]);
                }
                out[o + 0x18..o + 0x1C].copy_from_slice(&[128, 128, 128, 255]);
                out[o + 0x1C..o + 0x20].copy_from_slice(&[128, 255, 128, 128]);
                out[o + 0x24] = *frames;
                out[o + 0x25] = 0xFF;
                out[o + 0x26] = 0xFF;
                out[o + 0x27] = if trick.is_some() {
                    node_events::START_TRICK
                } else {
                    0
                };
                out[o + 0x28] = node_flags::CROUCHED;
                if let Some(t) = trick {
                    out.resize(ext_cursor + EXT_STRIDE, 0);
                    put_u32(&mut out, o + 0x20, (ext_cursor - o) as u32);
                    put_f32(&mut out, ext_cursor + 0x0C, 7.25);
                    out[ext_cursor + 0x24..ext_cursor + 0x26].copy_from_slice(&t.to_be_bytes());
                    out[ext_cursor + 0x26] = 0xFF; // -1 spins
                    out[ext_cursor + 0x27] = ext_flags::HAS_TRAJECTORY;
                    ext_cursor += EXT_STRIDE;
                }
            }
            out.resize(align16(out.len()), 0xDE);
            let groups_at = out.len();
            if !p.groups.is_empty() {
                out.resize(groups_at + p.groups.len() * BRANCH_GROUP_STRIDE, 0);
                for (g, (node, branches)) in p.groups.iter().enumerate() {
                    let go = groups_at + g * BRANCH_GROUP_STRIDE;
                    let branches_at = out.len();
                    out.resize(branches_at + branches.len() * BRANCH_STRIDE, 0);
                    put_u32(&mut out, go, (branches_at - go) as u32);
                    put_u32(&mut out, go + 4, branches.len() as u32);
                    put_u32(&mut out, go + 8, *node);
                    for (b, (target, target_node, weight)) in branches.iter().enumerate() {
                        let bo = branches_at + b * BRANCH_STRIDE;
                        out[bo..bo + 16].copy_from_slice(target);
                        put_u32(&mut out, bo + 16, *target_node);
                        put_f32(&mut out, bo + 20, *weight);
                    }
                }
                out.resize(align16(out.len()), 0xDE);
            }
            for a in 0..3 {
                put_f32(&mut out, h + a * 4, lo[a]);
                put_f32(&mut out, h + 0x10 + a * 4, hi[a]);
            }
            put_f32(&mut out, h + 0x0C, 1.0);
            put_f32(&mut out, h + 0x1C, 1.0);
            out[h + 0x20..h + 0x30].copy_from_slice(&p.id);
            put_u32(&mut out, h + 0x30, (nodes_at - h) as u32);
            put_u32(&mut out, h + 0x34, p.nodes.len() as u32);
            let has_ext = p.nodes.iter().any(|n| n.3.is_some());
            put_u32(
                &mut out,
                h + 0x38,
                if has_ext { (ext_at - h) as u32 } else { 0 },
            );
            put_u32(
                &mut out,
                h + 0x3C,
                if p.groups.is_empty() {
                    0
                } else {
                    (groups_at - h) as u32
                },
            );
            put_u32(&mut out, h + 0x40, p.groups.len() as u32);
            put_u32(&mut out, h + 0x44, p.flags);
            out[h + 0x48..h + 0x50].copy_from_slice(&p.mask.to_be_bytes());
        }
        out
    }

    fn id(tag: &[u8; 4], n: u8) -> [u8; 16] {
        let mut id = [0u8; 16];
        id[1..5].copy_from_slice(tag);
        id[15] = n;
        id
    }

    fn two_paths() -> Vec<u8> {
        build(&[
            SynthPath {
                id: id(b"indu", 1),
                nodes: vec![
                    ([0.0, 0.0, 0.0], [0.0; 3], 1, None),
                    ([0.3, 0.0, 0.4], [0.3, 0.0, 0.4], 4, Some(128)),
                    ([0.6, 0.1, 0.8], [0.3, 0.1, 0.4], 2, Some(54)),
                ],
                groups: vec![
                    (1, vec![(id(b"indu", 2), 0, 0.5), (id(b"indu", 2), 1, 0.25)]),
                    (2, vec![(id(b"indu", 2), 1, 1.0)]),
                ],
                flags: path_flags::RACE | path_flags::VERT | path_flags::STREET,
                mask: 0x3FFF_FFFF_FFFF_FFFF,
            },
            SynthPath {
                id: id(b"indu", 2),
                nodes: vec![
                    ([5.0, 0.0, 5.0], [0.0; 3], 1, None),
                    ([5.0, 0.0, 6.0], [0.0, 0.0, 1.0], 6, None),
                ],
                groups: vec![],
                flags: path_flags::STREET | path_flags::SOME_OFF_BOARD,
                mask: 1 << 51,
            },
        ])
    }

    #[test]
    fn decodes_header_nodes_ext_and_branches() {
        let paths = parse(&two_paths()).unwrap();
        assert_eq!(paths.len(), 2);
        let a = &paths[0];
        assert_eq!(&a.tag(), b"indu");
        assert!(a.id.is_ambient());
        assert_eq!(a.flags, 7);
        assert_eq!(a.nodes.len(), 3);
        assert_eq!(a.bbox_min, [0.0, 0.0, 0.0]);
        assert_eq!(a.bbox_max, [0.6, 0.1, 0.8]);
        assert_eq!(a.start(), Some([0.0, 0.0, 0.0]));
        assert_eq!(a.nodes[1].frames_since_last_node, 4);
        assert_eq!(a.nodes[1].width_left, 255);
        assert!(a.nodes[1].has_flag(node_flags::CROUCHED));
        assert_eq!(a.nodes[0].extended, None);
        assert_eq!(a.nodes[1].extended, Some(0));
        assert_eq!(a.nodes[2].extended, Some(1));
        assert_eq!(a.extended.len(), 2);
        assert_eq!(a.extended[0].trick_index, 128);
        assert_eq!(a.extended[1].trick_index, 54);
        assert_eq!(a.extended[0].air_spin_180_count, -1);
        assert_eq!(a.extended[0].trajectory_start_velocity, [7.25, 0.0, 0.0]);
        assert_eq!(a.branch_groups.len(), 2);
        assert_eq!(a.branch_groups[0].node, 1);
        assert_eq!(a.branch_groups[0].branches.len(), 2);
        assert_eq!(a.branch_groups[0].branches[1].target, paths[1].id);
        assert_eq!(a.branch_groups[0].branches[1].weight, 0.25);
        assert_eq!(a.branch_groups[1].branches[0].target_node, 1);
        assert_eq!(a.frames(), 6);
        assert!((a.length() - (0.5 + (0.09f32 + 0.01 + 0.16).sqrt())).abs() < 1e-5);
        let b = &paths[1];
        assert!(b.allows(51) && !b.allows(0) && !b.allows(64));
        assert!(b.branch_groups.is_empty() && b.extended.is_empty());
        assert_eq!(
            b.flags & path_flags::SOME_OFF_BOARD,
            path_flags::SOME_OFF_BOARD
        );
    }

    #[test]
    fn orientation_bytes_are_biased_unit_quaternions() {
        assert_eq!(
            decode_orientation([128, 128, 128, 255]),
            [0.0, 0.0, 0.0, 1.0]
        );
        let q = decode_orientation([126, 108, 126, 253]);
        let norm: f32 = q.iter().map(|c| c * c).sum();
        assert!((norm - 1.0).abs() < 1e-5);
        assert!(q[3] > 0.98 && q[1] < -0.15);
    }

    #[test]
    fn rejects_truncated_and_inconsistent_blobs() {
        let blob = two_paths();
        for cut in [0, 3, 8, 0x10 + 0x40, 0x10 + PATH_STRIDE * 2 + NODE_STRIDE] {
            assert!(parse(&blob[..cut]).is_err(), "cut at {cut:#x}");
        }
        let mut huge = blob.clone();
        huge[0..4].copy_from_slice(&0x0100_0000u32.to_be_bytes());
        assert!(matches!(parse(&huge), Err(AiPathError::BadHeader(_))));
        let mut wild = blob.clone();
        wild[0x10 + 0x30..0x10 + 0x34].copy_from_slice(&0xFFFF_FFF0u32.to_be_bytes());
        assert!(parse(&wild).is_err());
        assert!(parse(&[0, 0, 0, 0, 0, 0, 0, 0x10]).unwrap().is_empty());
    }

    #[test]
    fn ids_round_trip_through_hex() {
        let i = AiPathId(id(b"dwtn", 7));
        assert_eq!(i.tag_str(), "dwtn");
        assert_eq!(AiPathId::from_hex(&i.to_hex()), Some(i));
        assert_eq!(AiPathId::from_hex("zz"), None);
    }

    #[test]
    fn pack_round_trip_and_district_dedupe() {
        let blob = two_paths();
        // The second tile repeats path 2 only (a line crossing the tile edge).
        let only_second = build(&[SynthPath {
            id: id(b"indu", 2),
            nodes: vec![
                ([5.0, 0.0, 5.0], [0.0; 3], 1, None),
                ([5.0, 0.0, 6.0], [0.0, 0.0, 1.0], 6, None),
            ],
            groups: vec![],
            flags: path_flags::STREET | path_flags::SOME_OFF_BOARD,
            mask: 1 << 51,
        }]);
        let pack = write_pack(&[
            (0x246d_c82d_654a_0724, "cSim_-150_-150_high", &blob),
            (7, "cSim_-50_-150_high", &only_second),
        ]);
        let tiles = parse_pack(&pack).unwrap();
        assert_eq!(tiles.len(), 2);
        assert_eq!(tiles[0].asset_id, 0x246d_c82d_654a_0724);
        assert_eq!(tiles[0].grid(), Some((-150, -150)));
        assert_eq!(tiles[1].blob, &only_second[..]);
        let (paths, conflicts) = district_paths(&tiles).unwrap();
        assert_eq!(conflicts, 0);
        assert_eq!(paths.len(), 2);
        assert_eq!(paths[0].tiles, vec![0]);
        assert_eq!(paths[1].tiles, vec![0, 1]);
        assert!(parse_pack(&pack[..20]).is_err());
        let mut bad = pack.clone();
        bad[8] = 9;
        assert!(parse_pack(&bad).is_err());
    }
}
