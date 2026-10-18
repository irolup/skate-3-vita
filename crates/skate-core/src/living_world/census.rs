//! Census kinds (pedestrians, vehicles): the retail living-world census.
//!
//! Per pass (`sub_826B7530` peds / `sub_826B7760` vehicles) [code]:
//! 1. the census circle: two `livingworld_census_ranges` sets lerped by the observer's speed in
//!    km/h (`sub_826B7D60`): spawn ring inner / outer, cull radius, forward offset of the
//!    centre along the velocity;
//! 2. cull: entities whose 3-D distance to the centre exceeds the cull radius go
//!    (`sub_826BA8B0` / `sub_826BAAB8`, squared-distance test);
//! 3. spawn (only offline, world ready, not zombie for vehicles): 2 attempts, at most 1 spawn
//!    (initial populate: 6000 / 600 in an 8-80 m ring): a random point in the ring
//!    (`sub_82E17508`), a random heading, the census record painted at that point and its cap
//!    times the density (`sub_826B8A28`), the budget and category roll (`sub_826B8B88`), then the
//!    factory (pool).

use super::config::{retail, CensusKindConfig};
use super::rng::Rng;
use super::{dist2, Observer, Vec3};
use std::collections::BTreeMap;

/// `tLWCensusCircle` [data layout: 5 floats].
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct CensusCircle {
    pub spawn_inner: f32,
    pub spawn_outer: f32,
    pub cull: f32,
    pub forward_offset: f32,
    pub speed_kmh: f32,
}

/// A `livingworld_census_ranges` record: set A (`circle_slow`) and set B (`circle_fast`).
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct CensusRange {
    pub slow: CensusCircle,
    pub fast: CensusCircle,
}

impl CensusRange {
    /// The circle at a speed (km/h): `t = clamp((v - A.key) / (B.key - A.key), 0, 1)`; when
    /// `A.key >= B.key` set A is used as is (`sub_826B7D60`) [code].
    pub fn at(&self, speed_kmh: f32) -> CensusCircle {
        let (a, b) = (self.slow, self.fast);
        if a.speed_kmh >= b.speed_kmh {
            return a;
        }
        let t = ((speed_kmh - a.speed_kmh) / (b.speed_kmh - a.speed_kmh)).clamp(0.0, 1.0);
        let l = |x: f32, y: f32| x + (y - x) * t;
        CensusCircle {
            spawn_inner: l(a.spawn_inner, b.spawn_inner),
            spawn_outer: l(a.spawn_outer, b.spawn_outer),
            cull: l(a.cull, b.cull),
            forward_offset: l(a.forward_offset, b.forward_offset),
            speed_kmh,
        }
    }

    /// Circle and centre for an observer: speed = |velocity| x 3.6 [code `0x822F8628`]; the centre
    /// moves `forward_offset` along the 3-D velocity direction (`normalize3(velocity)`,
    /// `sub_826B7530`) [code], so on a slope the centre also moves up or down the hill.
    pub fn around(&self, observer: &Observer) -> (CensusCircle, Vec3) {
        let v = observer.velocity;
        let speed = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        let circle = self.at(speed * retail::KMH_PER_MS);
        let mut centre = observer.position;
        if speed > 1e-6 && circle.forward_offset != 0.0 {
            for (c, d) in centre.iter_mut().zip(v) {
                *c += d / speed * circle.forward_offset;
            }
        }
        (circle, centre)
    }
}

/// One category of a census record's group: `livingworld_entitycategories` key + weight.
#[derive(Clone, Debug, PartialEq)]
pub struct CensusCategory {
    pub name: String,
    pub weight: f32,
}

/// A `livingworld_census` record resolved: max population and the group's categories.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct CensusRecord {
    pub max_population: u32,
    pub categories: Vec<CensusCategory>,
}

/// A district census grid (`<District>.census.bin`, parsed by `skate-data::living_world`):
/// cell (i, j) covers [ox + i*cell, ox + (i+1)*cell) x [oz + j*cell, ...); per layer one u16 per
/// cell, 0 = unpainted, k = `names[k-1]`.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct CensusGrid {
    pub cell: f32,
    pub origin: [f32; 2],
    pub width: u32,
    pub height: u32,
    pub names: Vec<String>,
    pub layers: BTreeMap<String, Vec<u16>>,
}

impl CensusGrid {
    pub fn record_at(&self, layer: &str, x: f32, z: f32) -> Option<&str> {
        if !(self.cell > 0.0) {
            return None;
        }
        let i = ((x - self.origin[0]) / self.cell).floor();
        let j = ((z - self.origin[1]) / self.cell).floor();
        if i < 0.0 || j < 0.0 || i >= self.width as f32 || j >= self.height as f32 {
            return None;
        }
        let cells = self.layers.get(layer)?;
        let value = *cells.get(j as usize * self.width as usize + i as usize)?;
        if value == 0 {
            None
        } else {
            self.names.get(value as usize - 1).map(String::as_str)
        }
    }
}

/// Census grids of the loaded districts plus the resolved records.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct CensusMap {
    pub grids: Vec<CensusGrid>,
    pub records: BTreeMap<String, CensusRecord>,
}

impl CensusMap {
    /// The record painted at (x, z) in a layer. Unpainted = `None`: for peds and vehicles the
    /// retail lookup (`sub_826B8A28`, types 2 / 3) only resolves a record when the layer query
    /// hits, so an unpainted point has cap 0 (no spawn) [code].
    pub fn record_at(&self, layer: &str, x: f32, z: f32) -> Option<(&str, &CensusRecord)> {
        let name = self.grids.iter().find_map(|g| g.record_at(layer, x, z))?;
        self.records.get_key_value(name).map(|(k, r)| (k.as_str(), r))
    }
}

/// Random point in the ring like `sub_82E17508` [code]: a direction from two uniform draws in
/// [-0.5, 0.5) (x, z; y = 0) normalised, a radius uniform in [inner, outer] (linear, not by
/// area), at the centre's height.
pub fn ring_point(rng: &mut Rng, centre: Vec3, inner: f32, outer: f32) -> Vec3 {
    let x = rng.unit() - 0.5;
    let z = rng.unit() - 0.5;
    let r = inner + rng.unit() * (outer - inner);
    let len = (x * x + z * z).sqrt();
    let (dx, dz) = if len > 1e-12 { (x / len, z / len) } else { (1.0, 0.0) };
    [centre[0] + dx * r, centre[1], centre[2] + dz * r]
}

/// Random heading: a u32 times 2pi / 2^32 (`0x822F9598`) [code].
pub fn random_heading(rng: &mut Rng) -> f32 {
    (rng.next_u32() as f64 * (std::f64::consts::TAU / 4_294_967_296.0)) as f32
}

/// The cap at a point (`sub_826B8A28`) [code]: the record's max population, times the density
/// unless the zombie cheat is on, truncated; 0 when unpainted.
pub fn cap_at(record: Option<&CensusRecord>, density: f32, zombie: bool) -> u32 {
    let Some(record) = record else { return 0 };
    if zombie {
        record.max_population
    } else {
        (record.max_population as f32 * density.max(0.0)) as u32
    }
}

/// The category roll of `sub_826B8B88` [code]: `roll = rand() % 100 + 1`; the first category
/// whose cumulative weight x 100 is not below the roll wins; none (weights summing below the
/// roll) = no spawn this attempt.
pub fn pick_category<'a>(rng: &mut Rng, record: &'a CensusRecord) -> Option<&'a CensusCategory> {
    let roll = (rng.modulo(retail::CATEGORY_ROLL) + 1) as f32;
    let mut cumulative = 0.0f32;
    for category in &record.categories {
        cumulative += category.weight;
        if !(roll > cumulative * retail::CATEGORY_ROLL as f32) {
            return Some(category);
        }
    }
    None
}

/// Whether a census entity at `position` is beyond the cull radius of every observer's circle.
pub fn beyond_cull(position: Vec3, circles: &[(CensusCircle, Vec3)]) -> bool {
    circles.iter().all(|(c, centre)| dist2(position, *centre) > c.cull * c.cull)
}

/// The spawn ring and attempt budget of one pass.
pub fn pass_budget(cfg: &CensusKindConfig, circle: &CensusCircle, initial: bool) -> (f32, f32, u32, u32) {
    if initial {
        (cfg.initial_ring.0, cfg.initial_ring.1, cfg.initial_attempts, cfg.initial_spawns)
    } else {
        (circle.spawn_inner, circle.spawn_outer, cfg.attempts_per_pass, cfg.spawns_per_pass)
    }
}

/// One vehicle entity (`livingworld_entities`, vehicle category) as the census needs it, from the
/// export (`vehicles.json`, stable names; a mod adds or overrides entries by the same keys).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct VehicleEntity {
    /// `livingworld_models` record.
    pub model: String,
    /// Colours in the model's chassis / secondary palettes (`palette_ids` in `vehicles.json`).
    pub chassis_colours: u32,
    pub secondary_colours: u32,
    /// Length (m): z of the model's size vector (`size_hint`) [data].
    pub length: f32,
    /// Width (m): x of the size vector [data].
    pub width: f32,
}

/// The vehicle entities per census category (`vehicles.json` `census.<record>.categories`) and
/// per entity. Iteration is sorted (BTreeMap), so the picks are machine independent.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct VehicleCatalog {
    /// Category name -> entity names in retail list order.
    pub categories: BTreeMap<String, Vec<String>>,
    pub entities: BTreeMap<String, VehicleEntity>,
}

/// The entity pick inside a category (`sub_826B8B88` via `sub_826BB058`) [code]:
/// `trunc(u32 x 2^-32 x 100) mod n`.
pub fn pick_entity<'a>(rng: &mut Rng, entities: &'a [String]) -> Option<&'a String> {
    if entities.is_empty() {
        return None;
    }
    let roll = (rng.next_u32() as f64 / 4_294_967_296.0 * retail::ENTITY_ROLL as f64) as u32;
    entities.get((roll % entities.len() as u32) as usize)
}
