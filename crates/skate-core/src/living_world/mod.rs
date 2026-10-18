//! Living world population core (doc `docs/hails-additions/26b-living-world-population.md`, milestone 2).
//!
//! One census engine for every ambient kind: NPC skaters, pedestrians and vehicles (props and
//! dynamic objects later). Pure and engine-independent: no ECS, no I/O, no rendering. The game
//! feeds observer positions, the census map and the session state once per console tick and gets
//! spawn / despawn decisions back.
//!
//! Retail reference (TU3, addresses are evidence only; values are re-implemented, not copied):
//! - census tick `sub_826B71F0`: one living-world type per tick in rotation (peds, vehicles, DMOs,
//!   props), each a cull pass then a spawn pass (`sub_826B7530` peds, `sub_826B7760` vehicles);
//! - ambient skater manager `sub_8245BA28`: a 60-tick phase cycle (cull, pool, spawn, checks).
//!
//! Multiplayer seams (no networking here): a decision is a pure function of
//! (config, seed, tick, observers, slot budget). It runs in one place (standalone or a future
//! host); everything downstream consumes [`Decision`] records, never the RNG, so a client can
//! replay the host's records. Every entity has a stable [`LivingWorldId`]; records carry what is
//! needed to recreate the entity on another machine, including the seed of its own sub-RNG.
//! Decisions never depend on hash-map iteration order or frame time.

pub mod ai_record;
pub mod ai_controller;
pub mod controller_b;
pub mod ai_signals;
pub mod avoid;
pub mod census;
pub mod clock;
pub mod config;
pub mod dmo;
pub mod draw_distance;
pub mod leave_fade;
pub mod npc_reactions;
pub mod npc_tricks;
pub mod air_steer;
pub mod ground_steer;
pub mod path_follow;
pub mod peds;
pub mod population;
pub mod replay;
pub mod rng;
pub mod skaters;
pub mod stance;

pub use census::{CensusCircle, CensusGrid, CensusMap, CensusRange, CensusRecord, VehicleCatalog, VehicleEntity};
pub use clock::ConsoleClock;
pub use config::{CensusKindConfig, FreePlay, PopulationConfig, SkaterConfig};
pub use draw_distance::DrawDistance;
pub use population::{LaneState, LivingWorld, TickInputs};
pub use rng::Rng;
pub use skaters::{SkaterCharacter, SkaterLine, SkaterWorld};

/// World position, metres (engine axes: y up; census grids use x / z).
pub type Vec3 = [f32; 3];

/// The ambient entity kinds the census manages. The discriminants are stable (they go into
/// ids, seeds and network records).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Kind {
    Skater = 0,
    Pedestrian = 1,
    Vehicle = 2,
}

impl Kind {
    pub const ALL: [Kind; 3] = [Kind::Skater, Kind::Pedestrian, Kind::Vehicle];
    pub fn name(self) -> &'static str {
        match self {
            Kind::Skater => "skater",
            Kind::Pedestrian => "pedestrian",
            Kind::Vehicle => "vehicle",
        }
    }
    pub fn from_name(name: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|k| k.name() == name)
    }
}

/// Stable identity of one spawned ambient entity: kind + a per-kind serial that never repeats
/// within a session (serials start at 1). Network and mod safe.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LivingWorldId {
    pub kind: Kind,
    pub serial: u32,
}

impl LivingWorldId {
    /// One u64 for logs, mod handles and wire formats: kind in the top byte, serial below.
    pub fn to_u64(self) -> u64 {
        ((self.kind as u64) << 56) | self.serial as u64
    }
    pub fn from_u64(raw: u64) -> Option<Self> {
        let kind = match raw >> 56 {
            0 => Kind::Skater,
            1 => Kind::Pedestrian,
            2 => Kind::Vehicle,
            _ => return None,
        };
        u32::try_from(raw & 0x00FF_FFFF_FFFF_FFFF).ok().map(|serial| Self { kind, serial })
    }
}

/// A point the population is built around: the local skater now, remote players later.
/// Retail uses the local skater's position (`sub_8245E400`, census circle centre) and its
/// speed (`|velocity| * 3.6` km/h, `sub_826B7D60`).
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Observer {
    pub position: Vec3,
    pub velocity: Vec3,
}

/// What a spawn decided, kind-specific, with stable names (retail record keys) so another
/// machine or a mod can recreate the entity.
#[derive(Clone, Debug, PartialEq)]
pub enum SpawnChoice {
    /// Census kinds (peds, vehicles): the `livingworld_census` record painted at the spawn
    /// point and the `livingworld_entitycategories` category the weight roll picked. The
    /// concrete entity / model inside the category is picked from `seed` by the body milestone.
    Census { record: String, category: String },
    /// Census vehicles (milestone V2): the census record and category, the entity the category
    /// roll picked and its `livingworld_models` record, the palette indices (`vehicles.json`
    /// `palette_ids`: `<model>/chassis/<i>`, `<model>/secondary/<i>`), and the lane the factory
    /// placed it on: retail segment id, lane, distance along the segment (m). Speed at spawn 0.
    Vehicle {
        record: String,
        category: String,
        entity: String,
        model: String,
        chassis: u32,
        secondary: u32,
        segment: u64,
        lane: u8,
        distance: f32,
    },
    /// NPC skaters: the recorded line (retail 16-byte id) and the `characters_marquee` key.
    Skater { line: [u8; 16], character: String, slot: u8 },
}

#[derive(Clone, Debug, PartialEq)]
pub struct SpawnRecord {
    pub id: LivingWorldId,
    /// Console tick of the decision.
    pub tick: u64,
    pub position: Vec3,
    /// Yaw in radians about +y.
    pub heading: f32,
    /// Seed of the entity's own sub-RNG (looks, behaviour); derived from the session seed and id.
    pub seed: u64,
    /// True for the initial-populate pass (retail spawn record byte 0).
    pub initial: bool,
    pub choice: SpawnChoice,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DespawnReason {
    /// Beyond the cull radius (census: 3-D distance to the circle centre; skaters: 3-D distance
    /// or height difference to the reference).
    Distance,
    /// More ambient skaters than desired (the cull pass despawns the excess).
    Excess,
    /// Free Play density / option dropped to zero: everything of the kind goes at once.
    FreePlayOff,
    /// Kill switch (zombie mode for skaters, kind disabled by settings or a mod).
    Disabled,
    /// The game asked (map change, mod cleanup, entity finished).
    External,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DespawnRecord {
    pub id: LivingWorldId,
    pub tick: u64,
    pub reason: DespawnReason,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Decision {
    Spawn(SpawnRecord),
    Despawn(DespawnRecord),
}

pub(crate) fn dist2(a: Vec3, b: Vec3) -> f32 {
    let (x, y, z) = (a[0] - b[0], a[1] - b[1], a[2] - b[2]);
    x * x + y * y + z * z
}

#[cfg(test)]
mod tests;

pub mod traffic;
