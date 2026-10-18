//! Population configuration. Code constants of the retail build are the defaults here (labelled
//! [code] with their address); data values (census caps, category weights, ranges) are not
//! duplicated: they come from the user's exported tables (`skate-data::living_world`) and a world
//! without that export stays empty. Every field is public so settings and mods can override it;
//! restoring `PopulationConfig::retail()` (plus the data) undoes a mod.

use super::census::CensusRange;
use super::traffic::PlacementRules;

/// Retail constants, one place. Tags: [code] read from the TU3 code / image at the address.
pub mod retail {
    /// Census: spawn attempts per pass and spawns per pass (`sub_826B9940`, `sub_826B9B90`:
    /// loop counter 2, spawn budget 1) [code].
    pub const ATTEMPTS_PER_PASS: u32 = 2;
    pub const SPAWNS_PER_PASS: u32 = 1;
    /// Initial populate: 6000 attempts, 600 spawns, ring 8 to 80 m (`0x82099250` = 8,
    /// `0x820E5748` = 80, same function) [code].
    pub const INITIAL_ATTEMPTS: u32 = 6000;
    pub const INITIAL_SPAWNS: u32 = 600;
    pub const INITIAL_RING: (f32, f32) = (8.0, 80.0);
    /// km/h per m/s for the range lerp key (`0x822F8628` = 3.6) [code].
    pub const KMH_PER_MS: f32 = 3.6;
    /// Census type rotation: the census tick runs one type per tick, peds 0, vehicles 1,
    /// DMOs 2, props 3 (`sub_826B71F0`, census `+164`, `(+164 + 1) % 4` at `0x826B7420`) [code].
    /// At the 60 Hz world tick each type gets a cull + spawn pass 15 times per second. The
    /// initial populate runs every type in the same tick (`0x826B7444` path) [code].
    pub const CENSUS_ROTATION: u32 = 4;
    pub const ROTATION_PEDS: u32 = 0;
    pub const ROTATION_VEHICLES: u32 = 1;
    /// Vehicle limit: the census vehicle spawn (`sub_826B83C8`, census vtable `0x8230C638` slot
    /// +8) refuses while the live vehicle count is not below census `+148` = 15 (constructor
    /// `sub_826B6D58`); the initial populate compares with a literal 15 [code]. The recomp never
    /// showed more than 15 cars at once in DownTown (cap 30) [trace proof1 / proof2, VEHSTATE].
    pub const VEHICLE_LIMIT: u32 = 15;
    pub const VEHICLE_INITIAL_LIMIT: u32 = 15;
    /// Entity inside the picked category: `trunc(u32 x 2^-32 x 100) mod n` from the world RNG
    /// (`sub_826B8B88` via `sub_826BB058`, `0x822F88F4` = 2^-32, `0x820ED57C` = 100) [code].
    pub const ENTITY_ROLL: u32 = 100;
    /// Pedestrian memory stores sized for 31 (`sub_82E22DD8` → `sub_82970810`, count 31) [code].
    pub const PED_POOL: u32 = 31;
    /// Category roll: `rand() % 100 + 1` against the cumulative weight x 100 (`sub_826B8B88`,
    /// `0x820ED57C` = 100) [code].
    pub const CATEGORY_ROLL: u32 = 100;
    /// Free Play density below this culls the whole kind at once (`sub_826B7010`, 1.19e-7) [code].
    pub const DENSITY_EPSILON: f32 = 1.192_092_9e-7;

    /// Ambient skaters (`sub_8245BA28` and callees) [code].
    // tick mod 60 (0x88888889 multiply on mgr+584, bumped once per world tick by sub_8245A7E8):
    // at the 60 Hz world tick one cycle is 1 s.
    pub const SKATER_CYCLE: u32 = 60;
    pub const SKATER_PHASE_CULL: u32 = 0; // sub_8245D520
    pub const SKATER_PHASE_POOL: u32 = 15; // sub_8245B400
    pub const SKATER_PHASE_SPAWN: u32 = 30; // sub_8245C548
    pub const SKATER_DESIRED: u32 = 3; // mgr+596
    pub const SKATER_AI_CAP: u32 = 5; // AI controllers of kind != 4
    pub const SKATER_SLOTS: u32 = 7; // *(0x83067060+8), 168-byte slots, slot 0 = local player
    pub const SKATER_POOL: u32 = 5; // character pool entries at mgr+64
    pub const SKATER_SPAWN_INNER: f32 = 60.0; // 0x821FF080
    pub const SKATER_SPAWN_OUTER: f32 = 90.0; // sub_82459480
    pub const SKATER_MAX_CANDIDATE_LINES: usize = 32; // sub_82459480
    pub const SKATER_CULL: f32 = 120.0; // 0x82256FE0
    pub const SKATER_CULL_HEIGHT: f32 = 1000.0; // 0x82256FE8
    pub const SKATER_REJECT_RADIUS2: f32 = 25.0; // 0x8209994C: any skater within 5 m of the start
    pub const SKATER_NEAR_RADIUS2: f32 = 100.0; // 0x820ED57C: nearest skater closer than 10 m
    pub const SKATER_NEAR_TOUCH: f32 = 0.5; // 0x8209975C
    pub const SKATER_NEAR_TOUCH_SCORE: i64 = 600; // li r11,600
    pub const SKATER_NEAR_SPAN: f32 = 10.0; // 0x821963E4
    pub const SKATER_NEAR_K: f32 = 299.999_97; // 0x822F92C0 (0x4395FFFF)
    pub const SKATER_NEAR_SCALE: f32 = 600.0; // 0x820994A8
    pub const SKATER_SCORE_RAND: u32 = 400; // rand() % 400 in sub_8245C018
    pub const CHARACTER_FIT: i64 = 500; // sub_8245B068
    pub const CHARACTER_PER_LINE: i64 = 5;
    pub const CHARACTER_RAND: u32 = 160;
}

/// Free Play (mode 3, `0x830B7AE8`): the pause-menu world options. Only applied while the game
/// is in Free Play; career free roam has no switch (scales are 1.0 there) [code].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FreePlay {
    /// `+332`, clamped 0..1 into census `+140` (vehicles) by `sub_826B7010`.
    pub traffic: f32,
    /// `+336`, clamped 0..1 into census `+136` (peds).
    pub pedestrians: f32,
    /// `+340`: A.I. skaters on / off (`sub_8245C548` spawn gate, `sub_8245A9B8` despawn).
    pub ai_skaters: bool,
}

impl Default for FreePlay {
    /// The mode block reset `sub_824FAB90` writes 1.0 / 1.0 / on [code].
    fn default() -> Self {
        Self { traffic: 1.0, pedestrians: 1.0, ai_skaters: true }
    }
}

/// One census kind (peds or vehicles).
#[derive(Clone, Debug, PartialEq)]
pub struct CensusKindConfig {
    /// Settings / mod switch (retail: always on in free roam).
    pub enabled: bool,
    /// Settings / mod density, multiplies the census cap like the Free Play scale (1.0 = retail).
    pub density: f32,
    /// The `livingworld_census_ranges` record (data). `None` = no export: nothing spawns.
    pub range: Option<CensusRange>,
    /// Census layer name in the grids (`livingworld_npc_census` / `livingworld_vehicle_census`).
    pub layer: String,
    /// Position of this kind in the census rotation.
    pub rotation_slot: u32,
    pub attempts_per_pass: u32,
    pub spawns_per_pass: u32,
    pub initial_attempts: u32,
    pub initial_spawns: u32,
    pub initial_ring: (f32, f32),
    /// Entity limit (the factory refuses beyond it). Peds 31 [code]; vehicles 15 (census `+148`,
    /// `retail::VEHICLE_LIMIT`) [code]. `None` = no limit beyond the census cap.
    pub pool: Option<u32>,
    /// The limit the initial populate checks (vehicles: a literal 15 in `sub_826B83C8`); `None` =
    /// `pool`.
    pub initial_pool: Option<u32>,
    /// Vehicles: the factory places the car on a road lane (`sub_82C36300`); the census pass draws
    /// no heading for them (`sub_826B9B90`) [code]. `None` = peds (heading drawn, point used).
    pub placement: Option<PlacementRules>,
    /// Spawns while the zombie cheat is on. Peds: yes and without the cap (`sub_826B8B88`);
    /// vehicles: no (`sub_826B7760`) [code].
    pub spawn_in_zombie: bool,
    /// Culled whatever the distance while the zombie cheat is on: vehicles yes (the cull `sub_826BAAB8` despawns a
    /// car beyond the cull radius OR when the zombie query is true) [code]; peds no.
    pub cull_in_zombie: bool,
}

impl CensusKindConfig {
    pub fn retail_pedestrians() -> Self {
        Self {
            enabled: true,
            density: 1.0,
            range: None,
            layer: "livingworld_npc_census".into(),
            rotation_slot: retail::ROTATION_PEDS,
            attempts_per_pass: retail::ATTEMPTS_PER_PASS,
            spawns_per_pass: retail::SPAWNS_PER_PASS,
            initial_attempts: retail::INITIAL_ATTEMPTS,
            initial_spawns: retail::INITIAL_SPAWNS,
            initial_ring: retail::INITIAL_RING,
            pool: Some(retail::PED_POOL),
            initial_pool: None,
            placement: None,
            spawn_in_zombie: true,
            cull_in_zombie: false,
        }
    }
    pub fn retail_vehicles() -> Self {
        Self {
            layer: "livingworld_vehicle_census".into(),
            rotation_slot: retail::ROTATION_VEHICLES,
            pool: Some(retail::VEHICLE_LIMIT),
            initial_pool: Some(retail::VEHICLE_INITIAL_LIMIT),
            placement: Some(PlacementRules::default()),
            spawn_in_zombie: false,
            cull_in_zombie: true,
            ..Self::retail_pedestrians()
        }
    }
}

/// Ambient NPC skaters.
#[derive(Clone, Debug, PartialEq)]
pub struct SkaterConfig {
    /// Settings / mod switch (retail manager byte `+610`, set at init).
    pub enabled: bool,
    /// Desired ambient count offline (mgr+596 = 3; online 0).
    pub desired: u32,
    /// All AI skaters (ambient + scripted, kind != 4).
    pub ai_cap: u32,
    /// Skater slots shared with players (slot 0 = local player, online players take more).
    pub slots: u32,
    pub pool_size: u32,
    pub cycle: u32,
    pub phase_cull: u32,
    pub phase_pool: u32,
    pub phase_spawn: u32,
    pub spawn_inner: f32,
    pub spawn_outer: f32,
    pub max_candidate_lines: usize,
    pub cull: f32,
    pub cull_height: f32,
    pub reject_radius2: f32,
    pub near_radius2: f32,
    pub score_rand: u32,
    /// Community skaters may be offered (retail: only offline, `sub_8245B7A0`).
    pub allow_community_offline: bool,
    /// How a skater appears and leaves: 1 s fade in after the spawn (`sub_825926F8`); 1 s fade out,
    /// removed below opacity 0.2 (`sub_8246EA90`, `sub_8245A9B8`), only at a dead end of the
    /// replay tier (fix 9: retail chains lines, see `line_chain`).
    pub leave_fade: super::leave_fade::LeaveFadeConfig,
    /// What a skater does at the end of its line: continue on a line starting within 4 m
    /// (`sub_8246C7F8`, [data] `ai_skater` tunable), so it leaves only by the 120 m cull.
    pub line_chain: super::replay::ChainConfig,
}

impl SkaterConfig {
    pub fn retail() -> Self {
        use retail::*;
        Self {
            enabled: true,
            desired: SKATER_DESIRED,
            ai_cap: SKATER_AI_CAP,
            slots: SKATER_SLOTS,
            pool_size: SKATER_POOL,
            cycle: SKATER_CYCLE,
            phase_cull: SKATER_PHASE_CULL,
            phase_pool: SKATER_PHASE_POOL,
            phase_spawn: SKATER_PHASE_SPAWN,
            spawn_inner: SKATER_SPAWN_INNER,
            spawn_outer: SKATER_SPAWN_OUTER,
            max_candidate_lines: SKATER_MAX_CANDIDATE_LINES,
            cull: SKATER_CULL,
            cull_height: SKATER_CULL_HEIGHT,
            reject_radius2: SKATER_REJECT_RADIUS2,
            near_radius2: SKATER_NEAR_RADIUS2,
            score_rand: SKATER_SCORE_RAND,
            allow_community_offline: true,
            leave_fade: super::leave_fade::LeaveFadeConfig::retail(),
            line_chain: super::replay::ChainConfig::retail(),
        }
    }

    /// Per-skater check phases: 3-12, 18-27, 33-57 of the cycle (`sub_8245A9B8`) [code].
    pub fn is_check_phase(&self, phase: u32) -> bool {
        (3..=12).contains(&phase) || (18..=27).contains(&phase) || (33..=57).contains(&phase)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PopulationConfig {
    pub skaters: SkaterConfig,
    pub pedestrians: CensusKindConfig,
    pub vehicles: CensusKindConfig,
    /// NPC draw distance (QoL, not retail; `draw_distance` module): every population distance x
    /// this, caps x its square. 1.0 = retail (the config above runs unchanged). Owned by the
    /// population authority; settings and mods set it through the engine's settings resource.
    pub draw_distance: f32,
}

impl PopulationConfig {
    /// Code defaults; census ranges stay empty until the exported tables fill them.
    pub fn retail() -> Self {
        Self {
            skaters: SkaterConfig::retail(),
            pedestrians: CensusKindConfig::retail_pedestrians(),
            vehicles: CensusKindConfig::retail_vehicles(),
            draw_distance: super::draw_distance::DrawDistance::RETAIL,
        }
    }
}

impl Default for PopulationConfig {
    fn default() -> Self {
        Self::retail()
    }
}
