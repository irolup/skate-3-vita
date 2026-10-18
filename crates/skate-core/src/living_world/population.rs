//! The generic population engine: per kind a roster of live entities with stable ids, a seeded
//! decision RNG, the cull test, the spawn pass budget; the console-tick driver that runs the
//! census rotation (peds, vehicles) and the ambient skater cycle; and the scorer trait the kind
//! rules use to rank candidates.

use super::census::{self, CensusMap, VehicleCatalog};
use super::traffic::{spawn as placement, LaneCar, RoadNetwork, SegmentId};
use super::clock::ConsoleClock;
use super::config::{retail, CensusKindConfig, FreePlay, PopulationConfig};
use super::draw_distance::DrawDistance;
use super::rng::{derive, Rng};
use super::skaters::{self, SkaterWorld};
use super::{Decision, DespawnReason, DespawnRecord, Kind, LivingWorldId, Observer, SpawnChoice, SpawnRecord, Vec3};
use std::collections::BTreeMap;

/// Ranks a candidate (lower is better unless the caller says otherwise); `None` rejects it.
/// The kinds' retail scorers implement this: the skater line scorer `sub_8245C018` (lowest
/// wins) and the character scorer `sub_8245B068` (highest wins).
pub trait Scorer<C> {
    fn score(&mut self, candidate: &C, rng: &mut Rng) -> Option<i64>;
}

/// First candidate with the lowest score (strict `<` keeps the earlier one on ties).
pub fn pick_lowest<C, S: Scorer<C>>(candidates: &[C], scorer: &mut S, rng: &mut Rng) -> Option<usize> {
    let mut best: Option<(usize, i64)> = None;
    for (i, c) in candidates.iter().enumerate() {
        if let Some(s) = scorer.score(c, rng) {
            if best.is_none_or(|(_, b)| s < b) {
                best = Some((i, s));
            }
        }
    }
    best.map(|(i, _)| i)
}

/// First candidate with the highest score.
pub fn pick_highest<C, S: Scorer<C>>(candidates: &[C], scorer: &mut S, rng: &mut Rng) -> Option<usize> {
    let mut best: Option<(usize, i64)> = None;
    for (i, c) in candidates.iter().enumerate() {
        if let Some(s) = scorer.score(c, rng) {
            if best.is_none_or(|(_, b)| s > b) {
                best = Some((i, s));
            }
        }
    }
    best.map(|(i, _)| i)
}

/// Where a live car is on the road now (spawn: the factory's lane, speed 0; the engine updates
/// it as the car drives, [`LivingWorld::update_lane`]). The census placement test reads it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LaneState {
    pub segment: SegmentId,
    pub lane: u8,
    pub distance: f32,
    pub speed: f32,
    pub length: f32,
    pub width: f32,
}

/// One live ambient entity as the core sees it.
#[derive(Clone, Debug, PartialEq)]
pub struct Live {
    pub id: LivingWorldId,
    pub position: Vec3,
    pub spawned_tick: u64,
    pub choice: SpawnChoice,
    /// Cars only.
    pub lane: Option<LaneState>,
}

/// Per-kind state: the roster (ordered by serial, so iteration is deterministic), the serial
/// counter and the decision RNG.
#[derive(Clone, Debug)]
pub struct KindState {
    pub kind: Kind,
    pub(crate) next_serial: u32,
    pub(crate) live: BTreeMap<u32, Live>,
    pub(crate) rng: Rng,
    /// Retail initial-populate flag (census `+169 & 0x40`): the next spawn pass uses the
    /// initial budget.
    pub(crate) initial_pending: bool,
    pub(crate) passes: u64,
}

impl KindState {
    fn new(kind: Kind, seed: u64) -> Self {
        Self {
            kind,
            next_serial: 1,
            live: BTreeMap::new(),
            rng: Rng::new(derive(seed, &[0x4C57_5249_4E47, kind as u64])),
            initial_pending: true,
            passes: 0,
        }
    }

    pub(crate) fn spawn(&mut self, session_seed: u64, tick: u64, position: Vec3, heading: f32, initial: bool, choice: SpawnChoice) -> SpawnRecord {
        let id = LivingWorldId { kind: self.kind, serial: self.next_serial };
        self.next_serial += 1;
        let seed = derive(session_seed, &[0x454E_5449_5459, self.kind as u64, id.serial as u64]);
        self.live.insert(id.serial, Live { id, position, spawned_tick: tick, choice: choice.clone(), lane: None });
        SpawnRecord { id, tick, position, heading, seed, initial, choice }
    }

    pub(crate) fn despawn(&mut self, serial: u32, tick: u64, reason: DespawnReason) -> Option<DespawnRecord> {
        self.live.remove(&serial).map(|l| DespawnRecord { id: l.id, tick, reason })
    }

    pub(crate) fn despawn_all(&mut self, tick: u64, reason: DespawnReason, out: &mut Vec<Decision>) {
        let serials: Vec<u32> = self.live.keys().copied().collect();
        for s in serials {
            if let Some(r) = self.despawn(s, tick, reason) {
                out.push(Decision::Despawn(r));
            }
        }
    }

    pub fn len(&self) -> usize {
        self.live.len()
    }
    pub fn is_empty(&self) -> bool {
        self.live.is_empty()
    }
}

/// Everything the core reads in one console tick.
#[derive(Clone, Copy)]
pub struct TickInputs<'a> {
    /// The local player first; remote players after it (later milestones). Empty = no
    /// population work this tick (entities are kept).
    pub observers: &'a [Observer],
    pub census: Option<&'a CensusMap>,
    pub skater_world: Option<&'a dyn SkaterWorld>,
    /// The road network of the loaded world (cars are placed on its lanes; none = no cars).
    pub roads: Option<&'a RoadNetwork>,
    /// Vehicle entities per category (`vehicles.json`; none = no cars).
    pub vehicles: Option<&'a VehicleCatalog>,
    /// Online session (retail `0x830B7C2B` / `0x83082929`): no ambient spawns of any kind,
    /// culling runs, desired ambient skaters 0 [code].
    pub online: bool,
    /// The zombie cheat (`*(0x830CFD94)+212` vfunc 156) [code].
    pub zombie: bool,
    /// `Some` only in Free Play (mode 3).
    pub free_play: Option<FreePlay>,
    /// Skater slots taken by players (local + remote). Retail slot 0 is the local player.
    pub player_slots: u32,
    /// Scripted / challenge AI skaters alive (retail kinds 3; they count toward the AI cap and a
    /// kind-3 slot turns the ambient skaters off, `sub_8245C4A8`).
    pub scripted_skaters: u32,
    /// `EnterAmbientSkaterChallenge` style override of the desired ambient count (mgr+607/+592).
    pub ambient_skater_override: Option<u32>,
    /// The world is streamed in and the census may spawn (retail census `+168 & 0x40`, the
    /// world-state word `0x830670CC == 5`, the manager byte `+121`).
    pub world_ready: bool,
}

impl<'a> TickInputs<'a> {
    pub fn offline(observers: &'a [Observer]) -> Self {
        Self {
            observers,
            census: None,
            skater_world: None,
            roads: None,
            vehicles: None,
            online: false,
            zombie: false,
            free_play: None,
            player_slots: 1,
            scripted_skaters: 0,
            ambient_skater_override: None,
            world_ready: true,
        }
    }
}

/// The population of one session: config, seed, console clock and the three kinds.
#[derive(Clone, Debug)]
pub struct LivingWorld {
    pub config: PopulationConfig,
    seed: u64,
    tick: u64,
    clock: ConsoleClock,
    pub(crate) skaters: KindState,
    pub(crate) skater_pool: Vec<String>,
    pub(crate) peds: KindState,
    pub(crate) vehicles: KindState,
}

impl LivingWorld {
    pub fn new(config: PopulationConfig, seed: u64) -> Self {
        Self {
            config,
            seed,
            tick: 0,
            clock: ConsoleClock::default(),
            skaters: KindState::new(Kind::Skater, seed),
            skater_pool: Vec::new(),
            peds: KindState::new(Kind::Pedestrian, seed),
            vehicles: KindState::new(Kind::Vehicle, seed),
        }
    }

    pub fn seed(&self) -> u64 {
        self.seed
    }
    /// Console ticks run so far.
    pub fn tick(&self) -> u64 {
        self.tick
    }
    pub fn clock(&self) -> &ConsoleClock {
        &self.clock
    }
    pub fn clock_mut(&mut self) -> &mut ConsoleClock {
        &mut self.clock
    }

    pub fn state(&self, kind: Kind) -> &KindState {
        match kind {
            Kind::Skater => &self.skaters,
            Kind::Pedestrian => &self.peds,
            Kind::Vehicle => &self.vehicles,
        }
    }
    fn state_mut(&mut self, kind: Kind) -> &mut KindState {
        match kind {
            Kind::Skater => &mut self.skaters,
            Kind::Pedestrian => &mut self.peds,
            Kind::Vehicle => &mut self.vehicles,
        }
    }
    pub fn count(&self, kind: Kind) -> usize {
        self.state(kind).len()
    }
    pub fn live(&self, kind: Kind) -> impl Iterator<Item = &Live> {
        self.state(kind).live.values()
    }
    /// The NPC skater character pool (retail 5 entries at mgr+64).
    pub fn skater_pool(&self) -> &[String] {
        &self.skater_pool
    }

    /// Engine-side movement of an entity (NPC riding its line, ped walking): culls and the
    /// 5 m rule use the latest position.
    pub fn update_position(&mut self, id: LivingWorldId, position: Vec3) {
        if let Some(l) = self.state_mut(id.kind).live.get_mut(&id.serial) {
            l.position = position;
        }
    }

    /// Engine-side movement of a car along the road (V3 driving): the placement test of later
    /// spawns keeps its gaps to the car's current lane, distance and speed.
    pub fn update_lane(&mut self, id: LivingWorldId, segment: SegmentId, lane: u8, distance: f32, speed: f32) {
        if let Some(l) = self.state_mut(id.kind).live.get_mut(&id.serial) {
            if let Some(s) = l.lane.as_mut() {
                s.segment = segment;
                s.lane = lane;
                s.distance = distance;
                s.speed = speed;
            }
        }
    }

    /// Remove one entity at the game's request (finished, mod cleanup).
    pub fn despawn(&mut self, id: LivingWorldId, reason: DespawnReason) -> Option<Decision> {
        let tick = self.tick;
        self.state_mut(id.kind).despawn(id.serial, tick, reason).map(Decision::Despawn)
    }

    /// Remove everything (of one kind or all), e.g. on map change or when a mod is disabled.
    pub fn despawn_all(&mut self, kind: Option<Kind>, reason: DespawnReason) -> Vec<Decision> {
        let mut out = Vec::new();
        let tick = self.tick;
        for k in Kind::ALL {
            if kind.is_none_or(|x| x == k) {
                self.state_mut(k).despawn_all(tick, reason, &mut out);
            }
        }
        out
    }

    /// A new world (map change): empty rosters, initial populate armed again. Serials keep
    /// counting so an id never repeats within a session.
    pub fn reset_world(&mut self) -> Vec<Decision> {
        let out = self.despawn_all(None, DespawnReason::External);
        for k in Kind::ALL {
            self.state_mut(k).initial_pending = true;
        }
        self.skater_pool.clear();
        out
    }

    /// Client side of a future host: mirror a host decision without running any rule or RNG.
    pub fn apply(&mut self, decision: &Decision) {
        match decision {
            Decision::Spawn(r) => {
                let st = self.state_mut(r.id.kind);
                st.next_serial = st.next_serial.max(r.id.serial + 1);
                st.live.insert(r.id.serial, Live { id: r.id, position: r.position, spawned_tick: r.tick, choice: r.choice.clone(), lane: None });
            }
            Decision::Despawn(r) => {
                self.state_mut(r.id.kind).live.remove(&r.id.serial);
            }
        }
        let (Decision::Spawn(SpawnRecord { tick, .. }) | Decision::Despawn(DespawnRecord { tick, .. })) = decision;
        self.tick = self.tick.max(*tick);
    }

    /// Advance by game time; runs every console tick that became due.
    pub fn advance(&mut self, seconds: f64, inputs: &TickInputs) -> Vec<Decision> {
        let due = self.clock.advance(seconds);
        let mut out = Vec::new();
        for _ in 0..due {
            out.extend(self.step(inputs));
        }
        out
    }

    /// One console tick.
    pub fn step(&mut self, inputs: &TickInputs) -> Vec<Decision> {
        let mut out = Vec::new();
        let tick = self.tick;
        if !inputs.observers.is_empty() {
            let slot = (tick % retail::CENSUS_ROTATION as u64) as u32;
            let seed = self.seed;
            // NPC draw distance (QoL, not retail): a scaled copy on top of the retail config, built
            // only when the multiplier is not 1; retail runs `self.config` itself.
            let scaled = DrawDistance::new(self.config.draw_distance).scaled(&self.config);
            let config = scaled.as_ref().unwrap_or(&self.config);
            if slot == config.pedestrians.rotation_slot {
                let density = density(&config.pedestrians, inputs.free_play.map(|f| f.pedestrians));
                census_pass(&config.pedestrians, &mut self.peds, inputs, tick, seed, density, &mut out);
            }
            if slot == config.vehicles.rotation_slot {
                let density = density(&config.vehicles, inputs.free_play.map(|f| f.traffic));
                census_pass(&config.vehicles, &mut self.vehicles, inputs, tick, seed, density, &mut out);
            }
            skaters::tick(&config.skaters, &mut self.skaters, &mut self.skater_pool, inputs, tick, seed, &mut out);
        }
        self.tick += 1;
        out
    }
}

/// Settings density x the Free Play scale (clamped 0..1 like `sub_826B7010`; 1.0 outside mode 3).
fn density(cfg: &CensusKindConfig, free_play: Option<f32>) -> f32 {
    cfg.density.max(0.0) * free_play.map_or(1.0, |f| f.clamp(0.0, 1.0))
}

/// One census pass of a kind: mass cull (density 0), distance cull, then the gated spawn pass.
pub(crate) fn census_pass(cfg: &CensusKindConfig, st: &mut KindState, inputs: &TickInputs, tick: u64, seed: u64, density: f32, out: &mut Vec<Decision>) {
    if !cfg.enabled {
        st.despawn_all(tick, DespawnReason::Disabled, out);
        return;
    }
    let Some(range) = cfg.range else { return };
    // Mass cull at density ~0 (not in zombie mode) [code sub_826B7010].
    if !inputs.zombie && density < retail::DENSITY_EPSILON {
        st.despawn_all(tick, DespawnReason::FreePlayOff, out);
        return;
    }
    // Guard (engine, not retail; doc 26 "Cars flying off"): a focus with a non-finite position or
    // speed gives no circle. An infinite centre would put every NPC beyond the cull and empty the
    // whole population in one pass; without a usable circle this pass neither culls nor spawns.
    let circles: Vec<_> = inputs.observers.iter().map(|o| range.around(o)).filter(|(c, centre)| c.cull.is_finite() && c.spawn_outer.is_finite() && centre.iter().all(|v| v.is_finite())).collect();
    if circles.is_empty() {
        return;
    }
    let zombie_cull = inputs.zombie && cfg.cull_in_zombie;
    let serials: Vec<(u32, DespawnReason)> = st
        .live
        .iter()
        .filter_map(|(s, l)| match () {
            _ if census::beyond_cull(l.position, &circles) => Some((*s, DespawnReason::Distance)),
            _ if zombie_cull => Some((*s, DespawnReason::Disabled)),
            _ => None,
        })
        .collect();
    for (s, reason) in serials {
        if let Some(r) = st.despawn(s, tick, reason) {
            out.push(Decision::Despawn(r));
        }
    }
    let gate = !inputs.online && inputs.world_ready && (!inputs.zombie || cfg.spawn_in_zombie);
    let Some(map) = inputs.census.filter(|_| gate) else { return };
    if let Some(rules) = cfg.placement {
        vehicle_spawns(cfg, &rules, st, inputs, map, &circles, tick, seed, density, out);
        return;
    }
    // Several observers: the spawn pass rotates through them (one observer = retail).
    let (circle, centre) = circles[(st.passes % circles.len() as u64) as usize];
    st.passes += 1;
    let initial = std::mem::take(&mut st.initial_pending);
    let (inner, outer, attempts, spawns) = census::pass_budget(cfg, &circle, initial);
    let mut left = spawns;
    for _ in 0..attempts {
        if left == 0 {
            break;
        }
        let point = census::ring_point(&mut st.rng, centre, inner, outer);
        let heading = census::random_heading(&mut st.rng);
        let found = map.record_at(&cfg.layer, point[0], point[2]);
        let cap = census::cap_at(found.map(|(_, r)| r), density, inputs.zombie);
        if cap == 0 {
            continue;
        }
        // Budget: the live count against the cap at the point; the zombie cheat ignores it.
        // (Retail passes a count read once per pass; re-reading it per spawn only differs in the
        // initial populate, see doc 26 open questions.)
        let count = st.live.len() as u32;
        if count >= cap && !inputs.zombie {
            continue;
        }
        let (record_name, record) = found.expect("cap > 0 has a record");
        let Some(category) = census::pick_category(&mut st.rng, record) else { continue };
        if cfg.pool.is_some_and(|p| count >= p) {
            continue; // the factory has no free entity
        }
        let choice = SpawnChoice::Census { record: record_name.to_string(), category: category.name.clone() };
        out.push(Decision::Spawn(st.spawn(seed, tick, point, heading, initial, choice)));
        left -= 1;
    }
}

/// The vehicle spawn pass (`sub_826B9B90` -> `sub_826B8B88` -> census slot +8 `sub_826B83C8` ->
/// factory `sub_82C36300`) [code]: per attempt a ring point (no heading draw), the cap at the
/// point and the category and entity rolls, the vehicle limit (15), then the road placement
/// (`traffic::spawn`): road under the point, 15 m from the segment ends, fitting lanes, a lane
/// roll, the overlap check. Needs the roads and the vehicle catalog; without them no car spawns.
#[allow(clippy::too_many_arguments)]
fn vehicle_spawns(
    cfg: &CensusKindConfig,
    rules: &placement::PlacementRules,
    st: &mut KindState,
    inputs: &TickInputs,
    map: &CensusMap,
    circles: &[(census::CensusCircle, Vec3)],
    tick: u64,
    seed: u64,
    density: f32,
    out: &mut Vec<Decision>,
) {
    let (Some(net), Some(catalog)) = (inputs.roads, inputs.vehicles) else { return };
    let (circle, centre) = circles[(st.passes % circles.len() as u64) as usize];
    st.passes += 1;
    let initial = std::mem::take(&mut st.initial_pending);
    let (inner, outer, attempts, spawns) = census::pass_budget(cfg, &circle, initial);
    let limit = if initial { cfg.initial_pool.or(cfg.pool) } else { cfg.pool };
    let mut left = spawns;
    for _ in 0..attempts {
        if left == 0 {
            break;
        }
        let point = census::ring_point(&mut st.rng, centre, inner, outer);
        let found = map.record_at(&cfg.layer, point[0], point[2]);
        let cap = census::cap_at(found.map(|(_, r)| r), density, inputs.zombie);
        if cap == 0 {
            continue;
        }
        let count = st.live.len() as u32;
        if count >= cap {
            continue;
        }
        let (record_name, record) = found.expect("cap > 0 has a record");
        let Some(category) = census::pick_category(&mut st.rng, record) else { continue };
        let Some(entity_name) = catalog.categories.get(&category.name).and_then(|list| census::pick_entity(&mut st.rng, list)) else { continue };
        let Some(entity) = catalog.entities.get(entity_name) else { continue };
        if limit.is_some_and(|p| count >= p) {
            continue; // census slot +8: the vehicle limit
        }
        // Factory: the road under the point and the lanes a car fits on.
        let cars_on = |segment: usize, lane: u8| -> Vec<LaneCar> {
            let id = net.segments[segment].id;
            st.live
                .values()
                .filter_map(|l| l.lane)
                .filter(|s| s.segment == id && s.lane == lane)
                .map(|s| LaneCar { distance: s.distance, length: s.length, speed: s.speed })
                .collect()
        };
        let Some((segment, distance, lanes)) = placement::candidate_lanes(net, point, rules, &cars_on) else { continue };
        let lane = placement::pick_lane(&lanes, st.rng.next_u32(), rules);
        let frame = net.lane_frame(segment, lane as f32, distance);
        // Overlap with the cars and the players near the new car.
        let radius = 0.5 * entity.length.max(entity.width);
        let mut obstacles: Vec<(Vec3, f32)> = st.live.values().map(|l| (l.position, l.lane.map_or(0.0, |s| 0.5 * s.length.max(s.width)))).collect();
        obstacles.extend(inputs.observers.iter().map(|o| (o.position, 0.0)));
        if placement::overlaps(frame.position, radius, &obstacles, rules) {
            continue;
        }
        // Palette: the code read so far does not show the pick (open); the engine derives it from
        // the session seed and the id, so the record alone recreates the car.
        let palette_seed = derive(seed, &[0x5041_4C45_5454_45, Kind::Vehicle as u64, st.next_serial as u64]);
        let choice = SpawnChoice::Vehicle {
            record: record_name.to_string(),
            category: category.name.clone(),
            entity: entity_name.clone(),
            model: entity.model.clone(),
            chassis: (palette_seed % entity.chassis_colours.max(1) as u64) as u32,
            secondary: ((palette_seed >> 32) % entity.secondary_colours.max(1) as u64) as u32,
            segment: net.segments[segment].id.0,
            lane,
            distance,
        };
        let record = st.spawn(seed, tick, frame.position, frame.yaw(), initial, choice);
        if let Some(l) = st.live.get_mut(&record.id.serial) {
            l.lane = Some(LaneState { segment: net.segments[segment].id, lane, distance, speed: 0.0, length: entity.length, width: entity.width });
        }
        out.push(Decision::Spawn(record));
        left -= 1;
    }
}
