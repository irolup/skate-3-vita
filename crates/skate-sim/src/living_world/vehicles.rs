//! Traffic cars: skate-game `living_world/vehicles.rs` (upstream PR #52 `618e824`) without Bevy
//! (L1c). The population's vehicle spawn / despawn records become cars that drive their lanes,
//! collide with the skater as vehicle solids, carry skitch grab splines and grind rails.
//!
//! skate_vita: the functions are copied verbatim; the Bevy plumbing becomes plain data. A car
//! entity (`TrafficCar` + `CarMotion`) is a [`CarEntity`] in [`TrafficState::index`]; the systems
//! are functions the session calls in upstream's order ([`super::LivingWorldState::step`]);
//! messages are returned or kept in [`TrafficState::events`]. Not taken: the look (Bevy scenes and
//! the retail car material; the Vita draws the baked car, `vita-render`), the engine / horn / alarm
//! audio (#32's traffic engine host), the mod horn patches (`skate_mods`), the peds and NPC skaters
//! in the look-ahead and holders (L1d / L1e).
//!
//! Upstream's module docs follow.
//!
//! - **Entity** per spawn record: [`TrafficCar`] (stable `LivingWorldId`, entity / model keys,
//!   palette ids and colours, engine record). [`TrafficState`] keeps the follower cars
//!   (`skate_core::living_world::traffic::follow`) and the id -> entity map.
//! - **Motion** (`FixedUpdate`, after the population step): one follower step per 60 Hz world
//!   tick, the signal clock ticked once per world tick (retail ticks its 4 controllers by 1/60 s
//!   per world tick, `sub_826B2C18`); a car spawned at tick `s` steps from tick `s + 1`. Lane state
//!   goes back to the population (`LivingWorld::update_lane`, `update_position`) so the census
//!   cull and the placement test see the real cars. A car at a dead end leaves (despawn, reason
//!   External).
//! - **Collision**: one kinematic body per car with the car's retail chassis mesh (else a box
//!   from the GLB bounds), infinite mass, the car's velocity, joined to the skater solve through
//!   `physics::network::Proxies` like the NPC skaters.
//! - **Events** ([`TrafficEvent`]): spawned, despawned, junction answer changed, entered a
//!   junction, entered a lane.
//!
//! Multiplayer (no networking): a car's motion is a function of its spawn record, the world tick,
//! the signal clock's tick count (= world ticks since the world loaded) and the other cars (which
//! are themselves spawn records); the connector choice reads the occupancy, so a client must run
//! the whole set of cars, not one.
//!
//! Moddability: [`VehicleOverrides`] (model per entity, GLB per model, colour per palette id,
//! follower numbers per entity, connector choice); restoring `VehicleOverrides::default()` undoes
//! a mod; the spec / palette defaults come from the export (`tables.json`, `vehicles.json`).

use super::PopulationState;
use crate::transform::Transform;
use glam::{Mat3, Quat, Vec3};
use skate_core::living_world::rng::Rng;
use skate_core::living_world::traffic::follow::{self, Car, FollowEvent, FollowParams};
use skate_core::living_world::traffic::{ConnectorChoice, Entry, LaneCursor, Place, RoadNetwork, SegmentId, SignalClock, SignalTimings};
use skate_core::living_world::{Decision, DespawnReason, Kind, LivingWorldId, SpawnChoice};
use std::collections::BTreeMap;
use tracing::{info, warn};

/// The six wheel bones of the car rig [data, V0].
pub(crate) const WHEEL_BONES: [&str; 6] = ["LeftFront_wheel", "RightFront_wheel", "LeftRear_wheel_1", "RightRear_wheel_1", "LeftRear_wheel_2", "RightRear_wheel_2"];
/// Default scale on the tint colours: 1, retail (`vehicle_defaultPS` multiplies the colours as
/// they are). A mod overrides it ([`VehicleOverrides::paint_gain`]).
pub(crate) const PAINT_GAIN: f32 = 1.0;
/// Solid ids of car proxies: a tag in the top bits keeps them apart from NPCs and mod bodies.
pub(crate) const PROXY_ID_TAG: u64 = 0x5643_0000_0000_0000;

/// One car model (`vehicles.json` `models.<record>`).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct VehicleModel {
    /// Asset path relative to the asset root.
    pub glb: String,
    pub chassis: Vec<[f32; 4]>,
    pub secondary: Vec<[f32; 4]>,
    pub chassis_ids: Vec<String>,
    pub secondary_ids: Vec<String>,
    /// m (`wheel_hint`).
    pub wheel_radius: f32,
    /// Mesh bounds, model space (min, max).
    pub bounds: [[f32; 3]; 2],
    /// The skitch grab splines (RW4 GRABDATA of the first part arena that has one, b35; one rear-edge spline per
    /// stock car, none on `reda_car`). Empty = the car cannot be skitched (retail).
    pub grab_splines: Vec<CarGrabSpline>,
    /// The grind rails, model space (`grind_splines`: the EB0004 rails of the grab splines' part arena, car setup
    /// 82C3CCD8; none on `reda_car`). Empty = the car cannot be grinded.
    pub grind_rails: std::sync::Arc<[crate::grind_world::props::PropRail]>,
    /// The car body's retail physics collision (`vehicles.json` `collision`, setup
    /// `living_world_vehicles.recipe_collision`: the `Accessory` arena's ClusteredMesh, model space). The skater and
    /// board meet this instead of a box from the model bounds, which sat above the roof / hood / bumper rails.
    pub collision: Option<CarCollision>,
}

/// A car body's collision mesh, built once into a shared parry shape.
#[derive(Clone)]
pub(crate) struct CarCollision {
    pub triangles: std::sync::Arc<[[[f32; 3]; 3]]>,
    pub shape: crate::dynamics::rapier3d::prelude::SharedShape,
}

impl PartialEq for CarCollision {
    fn eq(&self, other: &Self) -> bool {
        self.triangles == other.triangles
    }
}

impl std::fmt::Debug for CarCollision {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "CarCollision({} triangles)", self.triangles.len())
    }
}

impl CarCollision {
    /// From the exported `collision.triangles` (model space); `None` when absent or not a valid mesh.
    pub(crate) fn parse(value: &serde_json::Value) -> Option<Self> {
        use crate::dynamics::rapier3d::prelude::{SharedShape, Vector};
        let point = |p: &serde_json::Value| -> Option<[f32; 3]> {
            let a = p.as_array()?;
            Some([a.first()?.as_f64()? as f32, a.get(1)?.as_f64()? as f32, a.get(2)?.as_f64()? as f32])
        };
        let triangles: Vec<[[f32; 3]; 3]> = value["triangles"].as_array()?.iter().filter_map(|t| {
            let t = t.as_array()?;
            Some([point(t.first()?)?, point(t.get(1)?)?, point(t.get(2)?)?])
        }).collect();
        if triangles.is_empty() {
            return None;
        }
        let vertices: Vec<Vector> = triangles.iter().flatten().map(|p| Vector::new(p[0], p[1], p[2])).collect();
        let indices: Vec<[u32; 3]> = (0..triangles.len() as u32).map(|i| [i * 3, i * 3 + 1, i * 3 + 2]).collect();
        let shape = SharedShape::trimesh(vertices, indices).ok()?;
        Some(Self { triangles: triangles.into(), shape })
    }
}

/// The exported `grab_splines` list (cars and props): a spline that is not whole Bezier segments is dropped.
pub(crate) fn parse_grab_splines(list: &serde_json::Value) -> Vec<CarGrabSpline> {
    list.as_array()
        .into_iter()
        .flatten()
        .filter_map(|g| {
            let points: Vec<[f32; 3]> = g["points"].as_array()?.iter().map(vec3).collect::<Option<_>>()?;
            (!points.is_empty() && points.len() % 4 == 0).then_some(CarGrabSpline { points, direction: vec3(&g["direction"])?, flags: g["flags"].as_u64().unwrap_or(0) as u32 })
        })
        .collect()
}

/// One authored grab spline (cars and props), model space: chains of cubic Bezier segments (4 control points each) and the grab
/// direction (stock cars: (0, 0, -1), backwards).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct CarGrabSpline {
    pub points: Vec<[f32; 3]>,
    pub direction: [f32; 3],
    /// Entry +60 (0x3E4 on stock cars; meaning open), the record's word 60 [inferred].
    pub flags: u32,
}

/// What a car entity drives with (`livingworld_entities` -> spec record).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct VehicleSpec {
    /// `aud_traffic_engine` record (spec `engine_audio`).
    pub engine: String,
    pub params: FollowParams,
    /// The entity's driver record (`livingworld_vehicle_drivers`; the horn values are in `params.horn`).
    pub driver: String,
}

/// Car data of the loaded world.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct VehicleData {
    pub models: BTreeMap<String, VehicleModel>,
    /// Entity name -> spec.
    pub specs: BTreeMap<String, VehicleSpec>,
    pub timings: Option<SignalTimings>,
    /// Car material rows (`vehicles.json` `materials`): `AttribulatorMaterialName` -> `m_params`.
    pub materials: BTreeMap<String, Vec<[f32; 4]>>,
    /// Material kind (`vehicle_chassis`, `vehicle_glass`) -> the name its retail parts use, for a
    /// GLB material without `attrib_material` extras.
    pub material_kinds: BTreeMap<String, String>,
    /// Environment cube id (GLB material extras `environment`) -> strip PNG, relative to
    /// `private/living_world/` (`vehicles.json` `environments`).
    pub environments: BTreeMap<String, String>,
}

fn vec3(v: &serde_json::Value) -> Option<[f32; 3]> {
    Some([v.get(0)?.as_f64()? as f32, v.get(1)?.as_f64()? as f32, v.get(2)?.as_f64()? as f32])
}

/// Read `vehicles.json` and `tables.json` (the follower numbers and engine record from each
/// entity's spec record; missing fields keep [`FollowParams::default`]).
pub(crate) fn parse_vehicle_data(vehicles: &[u8], tables: Option<&[u8]>) -> Result<VehicleData, String> {
    let doc: serde_json::Value = serde_json::from_slice(vehicles).map_err(|e| format!("vehicles.json: {e}"))?;
    let tables: Option<serde_json::Value> = tables.and_then(|t| serde_json::from_slice(t).ok());
    let mut data = VehicleData { timings: tables.as_ref().and_then(skate_data::roads::signal_timings), ..Default::default() };
    for (key, m) in doc["models"].as_object().into_iter().flatten() {
        let Some(glb) = m["glb"].as_str() else { continue };
        let colours = |k: &str| -> Vec<[f32; 4]> {
            m[k].as_array().into_iter().flatten().filter_map(|c| Some([c.get(0)?.as_f64()? as f32, c.get(1)?.as_f64()? as f32, c.get(2)?.as_f64()? as f32, c.get(3).and_then(|x| x.as_f64()).unwrap_or(1.0) as f32])).collect()
        };
        let ids = |k: &str| -> Vec<String> { m["palette_ids"][k].as_array().into_iter().flatten().filter_map(|s| s.as_str().map(str::to_string)).collect() };
        let bounds = (|| Some([vec3(&m["mesh_bounds"][0])?, vec3(&m["mesh_bounds"][1])?]))().unwrap_or([[-0.9, 0.0, -2.2], [0.9, 1.5, 2.2]]);
        let wheel_radius = m["wheel_hint"].as_f64().map(|x| x as f32).filter(|r| *r > 0.05).unwrap_or(0.32);
        data.models.insert(
            key.clone(),
            VehicleModel {
                glb: format!("private/living_world/{glb}"),
                chassis: colours("chassis_colours"),
                secondary: colours("secondary_colours"),
                chassis_ids: ids("chassis"),
                secondary_ids: ids("secondary"),
                wheel_radius,
                bounds,
                grab_splines: parse_grab_splines(&m["grab_splines"]),
                grind_rails: if m["grind_splines"].is_null() {
                    Default::default()
                } else {
                    crate::grind_world::props::parse_rails(&m["grind_splines"]).map(Into::into).unwrap_or_else(|e| {
                        warn!("LIVING_WORLD traffic: {key} grind rails ignored: {e}");
                        Default::default()
                    })
                },
                // The model's retail chassis mesh; installs exported before 2026-10-16 carry it only on the recipe
                // (`recipes/<recipe>/glb/collision`), so read it there too: without it the car collided as its bounds
                // box, whose rear wall sat between a skitcher's hands and shoulders (arm hits the whole tow).
                collision: CarCollision::parse(&m["collision"])
                    .or_else(|| m["recipe"].as_str().and_then(|r| CarCollision::parse(&doc["recipes"][r]["glb"]["collision"]))),
            },
        );
    }
    let row = |v: &serde_json::Value| Some([v.get(0)?.as_f64()? as f32, v.get(1)?.as_f64()? as f32, v.get(2)?.as_f64()? as f32, v.get(3)?.as_f64()? as f32]);
    for (name, m) in doc["materials"].as_object().into_iter().flatten() {
        data.materials.insert(name.clone(), m["params"].as_array().into_iter().flatten().filter_map(row).collect());
    }
    for (id, path) in doc["environments"].as_object().into_iter().flatten() {
        if let Some(path) = path.as_str() {
            data.environments.insert(id.clone(), format!("private/living_world/{path}"));
        }
    }
    for (kind, name) in doc["material_kinds"].as_object().into_iter().flatten() {
        if let Some(name) = name.as_str() {
            data.material_kinds.insert(kind.clone(), name.to_string());
        }
    }
    let class = |c: &str, r: &str| tables.as_ref().and_then(|t| t.pointer(&format!("/classes/{c}/{r}/fields")).cloned());
    for (name, e) in doc["entities"].as_object().into_iter().flatten() {
        let spec_name = e["spec"].as_str().unwrap_or("default");
        let mut params = FollowParams::default();
        let mut engine = "default".to_string();
        let driver = e["driver"].as_str().unwrap_or("default").to_string();
        if let Some(f) = class("livingworld_vehicle_drivers", &driver) {
            let num = |k: &str| f.get(k).and_then(|v| v.as_f64()).map(|v| v as f32);
            let h = &mut params.horn;
            h.blocked_time = num("honk_blocked_time").unwrap_or(h.blocked_time);
            h.obstacle_time = num("honk_obstacle_time").unwrap_or(h.obstacle_time);
            h.approach_speed = num("honk_approach_speed_kmh").map_or(h.approach_speed, |v| v / 3.6);
            // Driver block +36 / +20 (`82C42348`, b74).
            h.enabled_chance = num("Hash_50E084076390A573").unwrap_or(h.enabled_chance);
            h.blocked_long_chance = num("Hash_20E9C6487FDDBDE8").unwrap_or(h.blocked_long_chance);
            // The manoeuvre decider's driver values (b74: driver block +28 go, +24 least loaded; hashed fields).
            let m = &mut params.manoeuvre;
            m.lane_change_chance = num("Hash_52CF2CF346699CA1").unwrap_or(m.lane_change_chance);
            m.least_loaded_chance = num("Hash_7C6B48BD9ADF8E6E").unwrap_or(m.least_loaded_chance);
            m.overtake_chance = num("Hash_9366C67755A24D89").unwrap_or(m.overtake_chance);
            m.pull_over_chance = num("pull_over_chance").or_else(|| num("Hash_559BA807F95FF93E")).unwrap_or(m.pull_over_chance);
            m.held_pull_over_chance = num("Hash_99083122A1B7116A").unwrap_or(m.held_pull_over_chance);
            params.parked_time = num("parked_time").or_else(|| num("Hash_988BB0F6F043EB3D")).unwrap_or(params.parked_time);
            // The speed style chances, normal / slow / fast (`82C43330`, b78.2), and the held-bonus chance (b78.3).
            for (i, hash) in ["Hash_90FC5AB44797501B", "Hash_85C11600C2914F96", "Hash_3C32BAC5E516710F"].into_iter().enumerate() {
                if let Some(v) = num(hash) {
                    params.speed_style[i] = v;
                }
            }
            params.held_bonus_chance = num("Hash_D0BA2E02DE46A57A").unwrap_or(params.held_bonus_chance);
        }
        if let Some(f) = class("livingworld_vehicle_characteristics", spec_name) {
            let num = |k: &str| f.get(k).and_then(|v| v.as_f64()).map(|v| v as f32);
            // The speed control's spec values (`82C3CDD0`, `sub_82C3FF38`, `sub_82C3FA08`; research b75 / b78): the max
            // accel range, the slow / fast biases, the held bonus and the hit-brake latch.
            for (hash, field) in [
                ("Hash_5EFF87155A4EABE1", &mut params.accel_lo),
                ("Hash_CA49D85FA629F7E7", &mut params.accel_hi),
                ("Hash_3DF4F2D4CA631369", &mut params.bias_slow),
                ("Hash_E0C564403296C5F8", &mut params.bias_fast),
                ("Hash_0070396243C5622D", &mut params.held_bonus),
                ("Hash_DD49E3C2B833A723", &mut params.latch_decel),
                // The junction and lead standoffs (`+3660` / `+3664` = length x these, `82C3CDD0`; b163).
                ("Hash_2ACE3FEE16D6F7A2", &mut params.stop_standoff_factor),
                ("Hash_73A1989BB90C93BA", &mut params.lead_standoff_factor),
            ] {
                if let Some(v) = num(hash) {
                    *field = v;
                }
            }
            // Retail reads spec+40 (`328B9F46`) as the lane-change passage factor and spec+48 (`90AB56A5`) as the lane
            // timer (b72 / b74).
            if let Some(v) = num("Hash_328B9F4685A14018") {
                params.passage_factor = v;
            }
            // spec+36: the pull-over approach factor (`+3672` = ext x it, b71 / b77).
            if let Some(v) = num("Hash_758229215579C6D1") {
                params.approach_factor = v;
            }
            if let Some(v) = num("Hash_90AB56A5DDCF2A3A") {
                params.manoeuvre.lane_timer = v;
            }
            // The skater-behind rule (b63). Older exports name 3AB7FC7C `follow_speed_margin_kmh` (it is the FAR
            // braking distance) and keep the others as raw hashes.
            let either = |a: &str, b: &str| num(a).or_else(|| num(b));
            let s = &mut params.skater;
            if let Some(v) = num("follow_min_speed_kmh") {
                s.min_speed = v / 3.6;
            }
            if let Some(v) = either("skater_follow_margin_kmh", "Hash_256A412E350A2659") {
                s.margin = v / 3.6;
            }
            if let Some(v) = either("skater_far_distance", "follow_speed_margin_kmh") {
                s.far_distance = v;
            }
            if let Some(v) = either("skater_scan_range", "Hash_33466832D8178EAF") {
                s.range = v;
            }
            if let Some(v) = either("skater_near_range", "Hash_D49FC49019181EE5") {
                s.near_range = v;
            }
            if let Some(v) = either("skater_near_distance", "Hash_F682D359CDBC4D12") {
                s.near_distance = v;
            }
            if let Some(v) = either("release_grace", "Hash_4727CF785EF735C8") {
                s.release_grace = v;
            }
            if let Some(k) = f.pointer("/engine_audio/key").and_then(|v| v.as_str()) {
                engine = k.to_string();
            }
        }
        data.specs.insert(name.clone(), VehicleSpec { engine, params, driver });
    }
    Ok(data)
}

/// Mod and engine overrides (all keyed by the export's stable names). Empty = retail.
#[derive(Default, Clone, Debug, PartialEq)]
pub(crate) struct VehicleOverrides {
    /// Entity name -> `livingworld_models` record.
    pub models: BTreeMap<String, String>,
    /// Model record -> GLB asset path.
    pub glbs: BTreeMap<String, String>,
    /// Palette id (`<model>/chassis/<i>`, `<model>/secondary/<i>`) -> RGBA.
    pub colours: BTreeMap<String, [f32; 4]>,
    /// Entity name -> follower numbers.
    pub params: BTreeMap<String, FollowParams>,
    /// Connector choice (retail: least loaded on every car).
    pub connector_choice: ConnectorChoice,
    /// Scale on both tint colours; `None` = [`PAINT_GAIN`] (1, retail).
    pub paint_gain: Option<f32>,
}

/// One traffic car.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TrafficCar {
    pub id: LivingWorldId,
    pub entity: String,
    pub model: String,
    pub glb: String,
    pub chassis_id: String,
    pub secondary_id: String,
    pub chassis: [f32; 4],
    pub secondary: [f32; 4],
    pub engine: String,
    /// Tint gain this car is drawn with ([`PAINT_GAIN`] unless overridden).
    pub paint_gain: f32,
    pub wheel_radius: f32,
    pub bounds: [[f32; 3]; 2],
    pub spawn_tick: u64,
}

/// Fixed-step pose of a car. The skater's solve of tick N meets `curr` of tick N-1 (traffic moves after Physics), so
/// the car is drawn between `drawn_prev` and `prev`, one tick behind `curr`, the ticks the skater and camera are
/// drawn between (b285: drawn `prev` -> `curr` it was a tick ahead of the skater holding it, 0.5 m at 30 m/s).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct CarMotion {
    pub drawn_prev: Transform,
    pub prev: Transform,
    pub curr: Transform,
    pub wheel_drawn_prev: f32,
    pub wheel_prev: f32,
    pub wheel: f32,
    pub velocity: Vec3,
}

/// skate_vita: a car entity of upstream's ECS (its `TrafficCar` and `CarMotion` components).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CarEntity {
    pub meta: TrafficCar,
    pub motion: CarMotion,
}

/// What happened to a car (engine systems, the planned mod events).
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum TrafficEvent {
    Spawned { id: LivingWorldId, entity: String, model: String, chassis: String },
    Despawned { id: LivingWorldId, reason: DespawnReason },
    /// The junction answer for the chosen connector changed (retail junction state).
    Junction { id: LivingWorldId, junction: u64, connector: u32, entry: Entry },
    EnteredJunction { id: LivingWorldId, junction: u64, connector: u32 },
    EnteredLane { id: LivingWorldId, segment: u64, lane: u8 },
    /// A lane change started on `segment` from lane `from` to `to` (`82C3A9E0`; b69 / b74).
    LaneChange { id: LivingWorldId, segment: u64, from: u8, to: u8 },
    /// The car pulls over to its spot / pulls out again (b73).
    PullingOver { id: LivingWorldId, segment: u64, spot: f32 },
    PullingOut { id: LivingWorldId, segment: u64 },
    /// Horn kind 2 at a ped (`sub_82C40660` -> vt+100 `sub_82E3C3D0`): the ped's honker is this car. Sent every
    /// frame while it lasts, as in retail. (L1e stage 2: the peds' RunFromHonker reads it.)
    HonkedAt { id: LivingWorldId, ped: u64 },
}

/// The traffic of the loaded world.
pub(crate) struct TrafficState {
    /// Follower cars, sorted by key (= the id serial).
    pub cars: Vec<Car>,
    /// Serial -> (entity, spawn tick).
    pub index: BTreeMap<u32, (CarEntity, u64)>,
    pub clock: Option<SignalClock>,
    /// World tick the cars were stepped to.
    pub last_tick: Option<u64>,
    pub rng: Rng,
    pub data: VehicleData,
    /// skate_vita: the messages of the last step (upstream's `TrafficEvent` writer).
    pub events: Vec<TrafficEvent>,
}

impl Default for TrafficState {
    fn default() -> Self {
        Self { cars: Vec::new(), index: BTreeMap::new(), clock: None, last_tick: None, rng: Rng::new(0x5452_4146), data: VehicleData::default(), events: Vec::new() }
    }
}

/// Car rotation from the lane direction (+Z forward, y up).
pub(crate) fn car_rotation(forward: [f32; 3]) -> Quat {
    let f = Vec3::from_array(forward).normalize_or(Vec3::Z);
    let x = Vec3::Y.cross(f).normalize_or(Vec3::X);
    let y = f.cross(x);
    Quat::from_mat3(&Mat3::from_cols(x, y, f))
}

/// The car's pose: on its lane-change curve while one runs (`Car::pose`), else its cursor frame.
fn car_transform(net: &RoadNetwork, car: &Car) -> Transform {
    let frame = car.pose(net);
    Transform::from_translation(Vec3::from_array(frame.position)).with_rotation(car_rotation(frame.forward))
}

impl TrafficState {
    /// `load_traffic_data` for a new world: the car data from `vehicles.json` and `tables.json`
    /// (through `skate_data::vfs`, the packs).
    pub(crate) fn load(asset_root: &std::path::Path) -> Self {
        let dir = asset_root.join("private/living_world");
        let tables = skate_data::vfs::read(&dir.join("tables.json")).ok();
        let mut traffic = Self::default();
        traffic.data = skate_data::vfs::read(&dir.join("vehicles.json"))
            .ok()
            .and_then(|b| parse_vehicle_data(&b, tables.as_deref()).map_err(|e| warn!("LIVING_WORLD traffic: {e}")).ok())
            .unwrap_or_default();
        traffic.clock = traffic.data.timings.map(SignalClock::new);
        traffic.last_tick = None;
        info!("LIVING_WORLD traffic models {} specs {} lights {}", traffic.data.models.len(), traffic.data.specs.len(), traffic.clock.is_some());
        traffic
    }
}

/// Build a car from a spawn record (pure; the engine and the tests use it). skate_vita: no mod
/// horn patches (`skate_mods`), no engine audio component.
pub(crate) fn car_from_record(
    net: &RoadNetwork,
    data: &VehicleData,
    overrides: &VehicleOverrides,
    record: &skate_core::living_world::SpawnRecord,
    length: f32,
) -> Option<(TrafficCar, Car)> {
    let SpawnChoice::Vehicle { entity, model, chassis, secondary, segment, lane, distance, .. } = &record.choice else { return None };
    let model_key = overrides.models.get(entity).unwrap_or(model);
    let m = data.models.get(model_key).cloned().unwrap_or_default();
    let pick = |ids: &[String], colours: &[[f32; 4]], i: u32, base: [f32; 4]| -> (String, [f32; 4]) {
        let id = ids.get(i as usize).cloned().unwrap_or_else(|| format!("{model_key}/{i}"));
        let c = overrides.colours.get(&id).copied().or_else(|| colours.get(i as usize).copied()).unwrap_or(base);
        (id, c)
    };
    let (chassis_id, chassis_c) = pick(&m.chassis_ids, &m.chassis, *chassis, [0.0, 0.0, 1.0, 1.0]);
    let (secondary_id, secondary_c) = pick(&m.secondary_ids, &m.secondary, *secondary, [1.0, 0.0, 0.0, 1.0]);
    let spec = data.specs.get(entity).cloned().unwrap_or(VehicleSpec { engine: "default".into(), params: FollowParams::default(), driver: "default".into() });
    let params = overrides.params.get(entity).copied().unwrap_or(spec.params);
    let si = net.segment_index(SegmentId(*segment))?;
    // The connector is chosen on the first step from the live occupancy (cursor next = None here
    // is replaced at once by the engine with the loads of that tick).
    let cursor = LaneCursor { place: Place::Lane { segment: si, lane: (*lane).min(net.segments[si].lanes - 1) }, distance: distance.clamp(0.0, net.segments[si].length), next: None, lane_shift: 0.0 };
    let length = if length > 0.5 { length } else { (m.bounds[1][2] - m.bounds[0][2]).max(3.0) };
    let mut car = Car::new(record.id.serial, cursor, length, params);
    // The spawn draws (`82C3CDD0`): max acceleration, speed style and held bonus, from their own seeded stream.
    let mut speed_rng = Rng::new(skate_core::living_world::rng::derive(record.seed, &[0x5350_4443]));
    (car.accel_cap, car.speed_bias, car.held_bonus) = skate_core::living_world::traffic::follow::spawn_draws(&params, &mut || speed_rng.next_u32());
    let mut horn_rng = Rng::new(skate_core::living_world::rng::derive(record.seed, &[0x484f_524e]));
    car.driver = skate_core::living_world::traffic::horn::DriverBits::roll(&params.horn, &mut || horn_rng.unit());
    // `+4402` bit 0x80 (`82C42348`, the last roll): pulls over even while a skater holds the car.
    car.params.manoeuvre.pulls_over_while_held = skate_core::living_world::traffic::horn::percent_roll(params.manoeuvre.held_pull_over_chance, horn_rng.unit());
    let glb = overrides.glbs.get(model_key).cloned().unwrap_or(m.glb.clone());
    Some((
        TrafficCar {
            id: record.id,
            entity: entity.clone(),
            model: model_key.clone(),
            glb,
            chassis_id,
            secondary_id,
            chassis: chassis_c,
            secondary: secondary_c,
            engine: spec.engine,
            paint_gain: overrides.paint_gain.filter(|g| g.is_finite() && *g > 0.0).unwrap_or(PAINT_GAIN),
            wheel_radius: m.wheel_radius.max(0.05),
            bounds: m.bounds,
            spawn_tick: record.tick,
        },
        car,
    ))
}

/// Pick the first connector of a freshly spawned car with the live loads (retail chooses on
/// entering the lane, `sub_82C376E8`).
fn choose_first(net: &RoadNetwork, cars: &[Car], car: &mut Car, choice: ConnectorChoice, rng: &mut Rng) {
    if let Place::Lane { segment, lane } = car.cursor.place {
        let occ = follow::occupancy(cars);
        car.cursor.next = skate_core::living_world::traffic::choose_connector(net, segment, lane, choice, &|s, l| occ.lane_load(s, l), rng);
    }
}

/// Spawn and despawn car entities from the population's records (`apply_vehicle_records`).
pub(crate) fn apply_vehicle_records(decisions: &[Decision], state: &PopulationState, overrides: &VehicleOverrides, traffic: &mut TrafficState) {
    for d in decisions {
        let Decision::Despawn(r) = d else { continue };
        if r.id.kind != Kind::Vehicle {
            continue;
        }
        if traffic.index.remove(&r.id.serial).is_some() {
            traffic.cars.retain(|c| c.key != r.id.serial);
            traffic.events.push(TrafficEvent::Despawned { id: r.id, reason: r.reason });
        }
    }
    let Some(net) = state.roads.as_ref() else { return };
    for d in decisions {
        let Decision::Spawn(s) = d else { continue };
        if s.id.kind != Kind::Vehicle || traffic.index.contains_key(&s.id.serial) {
            continue;
        }
        let length = state.world.live(Kind::Vehicle).find(|l| l.id == s.id).and_then(|l| l.lane).map_or(0.0, |l| l.length);
        let Some((meta, mut car)) = car_from_record(net, &traffic.data, overrides, s, length) else {
            warn!("LIVING_WORLD traffic: car #{} has no lane on this road network", s.id.serial);
            continue;
        };
        choose_first(net, &traffic.cars, &mut car, overrides.connector_choice, &mut traffic.rng);
        let t = car_transform(net, &car);
        traffic.events.push(TrafficEvent::Spawned { id: s.id, entity: meta.entity.clone(), model: meta.model.clone(), chassis: meta.chassis_id.clone() });
        let motion = CarMotion { drawn_prev: t, prev: t, curr: t, wheel_drawn_prev: 0.0, wheel_prev: 0.0, wheel: 0.0, velocity: Vec3::ZERO };
        traffic.index.insert(s.id.serial, (CarEntity { meta, motion }, s.tick));
        let at = traffic.cars.partition_point(|c| c.key < car.key);
        traffic.cars.insert(at, car);
    }
}

/// Step the cars to the population's tick, publish pose and lane state (`drive_traffic`).
/// `held`: the car the local player is skitching on. Returns the despawn records of cars that
/// left (dead ends, broken states). skate_vita: no alarms, peds or simulated NPC skaters yet.
pub(crate) fn drive_traffic(
    overrides: &VehicleOverrides,
    st: &mut PopulationState,
    traffic: &mut TrafficState,
    observers: &[skate_core::living_world::Observer],
    peds: &[(u64, [f32; 3])],
    held: Option<u32>,
    props: Option<&skate_core::living_world::peds::NavObstacles>,
) -> Vec<Decision> {
    let _span = skate_core::profile::scope("z_traffic");
    let mut despawns = Vec::new();
    let Some(net) = st.roads.as_ref() else { return despawns };
    // The held bits are cleared and set again every tick (82C34CD0 / 82C361E8; b57): the local player's state 104.
    for car in &mut traffic.cars {
        car.player_held = Some(car.key) == held;
        car.held = car.player_held;
    }
    let look = skate_core::profile::scope("z_tr_look");
    look_ahead(traffic, observers, peds, props, held.is_some());
    drop(look);
    let now = st.world.tick();
    let from = traffic.last_tick.unwrap_or(now);
    traffic.last_tick = Some(now);
    let mut travelled: BTreeMap<u32, f32> = BTreeMap::new();
    let mut prev_pose: BTreeMap<u32, Transform> = BTreeMap::new();
    for c in &traffic.cars {
        prev_pose.insert(c.key, car_transform(net, c));
    }
    let mut dead = Vec::new();
    let dt = (1.0 / skate_core::living_world::clock::RETAIL_TICK_HZ) as f32;
    for tick in from + 1..=now {
        let changes = traffic.clock.as_mut().map(|c| {
            let mut v = Vec::new();
            c.tick(&mut v);
            v
        });
        drop(changes);
        let Some(clock) = traffic.clock.as_ref() else { break };
        // Cars decided before this tick drive in it.
        let active: Vec<usize> = (0..traffic.cars.len()).filter(|&i| traffic.index.get(&traffic.cars[i].key).is_some_and(|(_, s)| *s < tick)).collect();
        let mut batch: Vec<Car> = active.iter().map(|&i| traffic.cars[i]).collect();
        let before: Vec<f32> = batch.iter().map(|c| c.cursor.distance).collect();
        let before_place: Vec<Place> = batch.iter().map(|c| c.cursor.place).collect();
        let step = skate_core::profile::scope("z_tr_step");
        let out = follow::step(net, clock, &mut batch, dt, overrides.connector_choice, &mut traffic.rng);
        drop(step);
        for (k, c) in batch.iter().enumerate() {
            let ds = if c.cursor.place == before_place[k] { (c.cursor.distance - before[k]).max(0.0) } else { c.speed * dt };
            *travelled.entry(c.key).or_default() += ds;
        }
        for (k, &i) in active.iter().enumerate() {
            traffic.cars[i] = batch[k];
        }
        for e in out {
            let id = |key: u32| LivingWorldId { kind: Kind::Vehicle, serial: key };
            match e {
                FollowEvent::Junction { key, connector, entry } => {
                    let c = &net.connectors[connector];
                    traffic.events.push(TrafficEvent::Junction { id: id(key), junction: c.id.junction.0, connector: c.id.index, entry });
                }
                FollowEvent::EnteredJunction { key, connector } => {
                    let c = &net.connectors[connector];
                    traffic.events.push(TrafficEvent::EnteredJunction { id: id(key), junction: c.id.junction.0, connector: c.id.index });
                }
                FollowEvent::EnteredLane { key, segment, lane } => {
                    traffic.events.push(TrafficEvent::EnteredLane { id: id(key), segment: net.segments[segment].id.0, lane });
                }
                FollowEvent::LaneChange { key, segment, from, to } => {
                    traffic.events.push(TrafficEvent::LaneChange { id: id(key), segment: net.segments[segment].id.0, from, to });
                }
                FollowEvent::PullingOver { key, segment, spot } => {
                    traffic.events.push(TrafficEvent::PullingOver { id: id(key), segment: net.segments[segment].id.0, spot });
                }
                FollowEvent::PullingOut { key, segment } => {
                    traffic.events.push(TrafficEvent::PullingOut { id: id(key), segment: net.segments[segment].id.0 });
                }
                FollowEvent::DeadEnd { key } => dead.push(key),
            }
        }
    }
    // Lane state and position back to the population; pose.
    let _publish = skate_core::profile::scope("z_tr_publish");
    for c in &traffic.cars {
        let id = LivingWorldId { kind: Kind::Vehicle, serial: c.key };
        let frame = c.cursor.frame(net);
        if frame.position.iter().chain(&frame.forward).chain([&c.speed, &c.cursor.distance]).any(|x| !x.is_finite()) {
            // Guard (engine): one broken car leaves on its own; it never reaches the census.
            warn!("LIVING_WORLD traffic: car #{} has a non-finite state (position {:?}, speed {}, distance {}); removed", c.key, frame.position, c.speed, c.cursor.distance);
            dead.push(c.key);
            continue;
        }
        st.world.update_position(id, frame.position);
        let (segment, lane, distance) = match c.cursor.place {
            Place::Lane { segment, lane } => (segment, lane, c.cursor.distance),
            Place::Connector { connector } => {
                let k = &net.connectors[connector];
                (net.connector_exit(connector).unwrap_or(0), k.to_lane, 0.0)
            }
        };
        st.world.update_lane(id, net.segments[segment].id, lane, distance, c.speed);
        let Some((entity, _)) = traffic.index.get_mut(&c.key) else { continue };
        let (meta, motion) = (&entity.meta, &mut entity.motion);
        let t = car_transform(net, c);
        let prev = prev_pose.get(&c.key).copied().unwrap_or(t);
        // A car new to this list (spawned or teleported this tick) starts its drawn pair here too.
        motion.drawn_prev = if prev_pose.contains_key(&c.key) { motion.prev } else { t };
        motion.prev = prev;
        motion.curr = t;
        motion.wheel_drawn_prev = motion.wheel_prev;
        motion.wheel_prev = motion.wheel;
        motion.wheel = (motion.wheel + travelled.get(&c.key).copied().unwrap_or(0.0) / meta.wheel_radius) % std::f32::consts::TAU;
        motion.velocity = Vec3::from_array(frame.forward) * c.speed;
        // L1e stage 2 (upstream, after the horn state; the horn sound is not the Vita's).
        if let Some(ped) = c.honk_target {
            traffic.events.push(TrafficEvent::HonkedAt { id, ped });
        }
    }
    // Dead ends: the car leaves (single player: this host decides).
    for key in dead {
        let id = LivingWorldId { kind: Kind::Vehicle, serial: key };
        if let Some(d @ Decision::Despawn(_)) = st.world.despawn(id, DespawnReason::External) {
            st.despawned += 1;
            despawns.push(d);
        }
        if traffic.index.remove(&key).is_some() {
            traffic.cars.retain(|c| c.key != key);
            traffic.events.push(TrafficEvent::Despawned { id, reason: DespawnReason::External });
        }
    }
    despawns
}

/// The kinematic box of one car, world space.
pub(crate) fn proxy(car: &TrafficCar, pose: &Transform, velocity: Vec3) -> crate::dynamics::SolidBody {
    proxy_with(car, pose, velocity, None)
}

/// [`proxy`] with the car's retail body mesh when it has one (else the box from the model bounds).
pub(crate) fn proxy_with(car: &TrafficCar, pose: &Transform, velocity: Vec3, collision: Option<&CarCollision>) -> crate::dynamics::SolidBody {
    use crate::dynamics::rapier3d::prelude::{Pose, Rotation, SharedShape, Vector};
    let [lo, hi] = car.bounds;
    let half = [(hi[0] - lo[0]) * 0.5, (hi[1] - lo[1]) * 0.5, (hi[2] - lo[2]) * 0.5];
    let centre = pose.translation + pose.rotation * Vec3::new((hi[0] + lo[0]) * 0.5, (hi[1] + lo[1]) * 0.5, (hi[2] + lo[2]) * 0.5);
    let q = pose.rotation;
    let rotation = Rotation::from_xyzw(q.x, q.y, q.z, q.w).normalize();
    let p = |v: Vec3| Vector::new(v.x, v.y, v.z);
    crate::dynamics::SolidBody {
        id: PROXY_ID_TAG | car.id.to_u64(),
        pose: Pose::from_parts(p(pose.translation), rotation),
        center_of_mass: p(centre),
        inertia_rotation: rotation,
        inverse_mass: 0.0,
        inverse_inertia: Vector::new(0.0, 0.0, 0.0),
        linvel: p(velocity),
        angvel: Vector::new(0.0, 0.0, 0.0),
        // Retail's vehicle contact group (8; the skeleton contact pass keeps the largest
        // relative normal speed against it for the car-hit bail, `sub_82BD4A30` / `sub_82D90C98`).
        contact_group: crate::physics::VEHICLE_GROUP,
        colliders: vec![match collision {
            Some(mesh) => crate::dynamics::SolidCollider { shape: mesh.shape.clone(), pose: Pose::from_parts(p(pose.translation), rotation), friction: 0.5 },
            None => crate::dynamics::SolidCollider {
                shape: SharedShape::cuboid(half[0].max(0.1), half[1].max(0.1), half[2].max(0.1)),
                pose: Pose::from_parts(p(centre), rotation),
                friction: 0.5,
            },
        }],
    }
}

/// The skater's body hit a car (`sub_82C3C150`, actor branch): a contact ahead of the car (ours:
/// along its velocity; retail's axis is the car's vtable +24, inferred forward) latches the hit
/// brake on the follower car. Logs `VEHICLE_HIT`. skate_vita: no car alarm rule (audio).
pub(crate) fn apply_vehicle_hits(skater: &mut crate::physics::SkaterRuntime, traffic: &mut TrafficState) {
    for (id, point, _speed) in std::mem::take(&mut skater.vehicle_hits) {
        if id & PROXY_ID_TAG != PROXY_ID_TAG {
            continue;
        }
        let Some((car, motion)) = traffic.index.values().map(|(e, _)| (&e.meta, &e.motion)).find(|(c, _)| (PROXY_ID_TAG | c.id.to_u64()) == id) else { continue };
        let at = motion.curr.translation;
        let v = motion.velocity;
        let ahead = (Vec3::from_array(point) - at).dot(v) > 0.0;
        let serial = car.id.serial;
        if let Some(f) = traffic.cars.iter_mut().find(|c| c.key == serial) {
            if ahead && !f.hit_brake && f.speed > 0.0 {
                f.hit_brake = true;
                info!("VEHICLE_HIT car=#{serial} speed={:.1} at=[{:.1}, {:.1}, {:.1}]", f.speed, point[0], point[1], point[2]);
            }
        }
    }
}

/// V4 look-ahead (`skate_core::living_world::traffic::obstacles`): the obstacle lists (`sub_826B2EE0`: skaters
/// radius 0 and peds, soft; movable props (DMOs) radius half their smallest extent, hard; traffic cars add nothing,
/// `b42`) against each car's look-ahead quad from last frame's pose; the nearest free distance goes to the follower
/// car. Ours: the ped radius is the fallback ped radius (retail 0.5 x the body extents' z, the per-ped value is
/// collision data), props carry no corner points, a bailing skater's four extra points are not added, and the
/// turn widening is off (the turn side `+3748` is open; threshold 0.025). skate_vita: `peds` = the
/// peds' (id, position) in id order.
fn look_ahead(traffic: &mut TrafficState, observers: &[skate_core::living_world::Observer], peds: &[(u64, [f32; 3])], props: Option<&skate_core::living_world::peds::NavObstacles>, local_skitching: bool) {
    use skate_core::living_world::traffic::horn::ObstacleHit;
    use skate_core::living_world::traffic::obstacles::{look_ahead_quad, nearest, CarFrame, LookAheadParams, Obstacle};
    use skate_core::living_world::traffic::skater_scan::{rear_zone_quad, scan, ScanActor};
    let params = LookAheadParams::default();
    let mut list: Vec<Obstacle> = observers.iter().map(|o| Obstacle { position: o.position, radius: 0.0, soft: true, id: None }).collect();
    list.extend(peds.iter().map(|(id, p)| Obstacle { position: *p, radius: super::vehicle_contacts::FALLBACK_PED_RADIUS, soft: true, id: Some(*id) }));
    // The skater scan's list (82C414A8 walks the skater manager's actors, b63): our observers (index 0 = the local
    // player, skipped while skitching). NOT RETAIL YET: the facing is the velocity heading (retail: the actor
    // matrix row +32), and the NEAR flag byte `[[state+52]+55]` is not identified (false: no NEAR rule).
    let actors: Vec<ScanActor> = observers
        .iter()
        .enumerate()
        .map(|(i, o)| {
            let l = (o.velocity[0] * o.velocity[0] + o.velocity[2] * o.velocity[2]).sqrt();
            let forward = if l > 1e-6 { [o.velocity[0] / l, o.velocity[2] / l] } else { [0.0, 0.0] };
            ScanActor { position: o.position, velocity: o.velocity, forward, skitching: i == 0 && local_skitching, near_flag: false }
        })
        .collect();
    let cars: Vec<(u32, CarFrame, [f32; 3])> = traffic
        .index
        .values()
        .map(|(e, _)| {
            let (car, motion) = (&e.meta, &e.motion);
            let r = motion.curr.rotation;
            let f = r * Vec3::Z;
            let s = r * Vec3::X;
            let flat = |v: Vec3| {
                let l = (v.x * v.x + v.z * v.z).sqrt().max(1e-6);
                [v.x / l, v.z / l]
            };
            let [lo, hi] = car.bounds;
            let frame = CarFrame {
                position: motion.curr.translation.to_array(),
                forward: flat(f),
                side: flat(s),
                half_width: (hi[0] - lo[0]) * 0.5,
                half_length: (hi[2] - lo[2]) * 0.5,
            };
            (car.id.serial, frame, [hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]])
        })
        .collect();
    // Props: active (not obstacle-off) DMOs; mod bodies live above `MOD_BODY_OBSTACLE_BASE` and are not DMOs.
    list.extend(props.into_iter().flat_map(|p| p.states.iter()).filter(|(id, st)| **id < super::peds::MOD_BODY_OBSTACLE_BASE && !st.inactive).map(|(_, st)| Obstacle {
        position: [st.now.center[0], st.now.y_min, st.now.center[1]],
        radius: st.now.half[0].min(st.now.half[1]),
        soft: false,
        id: None,
    }));
    for (serial, frame, size) in &cars {
        let Some(car) = traffic.cars.iter_mut().find(|c| c.key == *serial) else { continue };
        let quad = look_ahead_quad(frame, car.speed, car.params.min_gap, 0.0, None, &params);
        // 82C344D0: a car the player holds skips the slow-speed soft records (b57).
        car.obstacle = nearest(frame, &quad, &list, car.speed, !car.player_held, &params).map(|(i, d)| ObstacleHit { distance: d, soft: list[i].soft, id: list[i].id });
        // 82C414A8 after the look-ahead: quad B with the same widths as quad A (speed ratio 0, see above).
        let w = frame.half_width * (1.0 + params.base_widen);
        let zone = rear_zone_quad(frame, w, w, car.params.skater.range);
        car.skater = scan(frame, *size, car.held, &zone, &actors, &car.params.skater);
    }
}

/// Add the car proxies to the skater solve (after the network proxies were rebuilt).
pub(crate) fn push_vehicle_proxies(traffic: &TrafficState, physics: &mut crate::physics::GamePhysics, skater: &crate::physics::SkaterRuntime) {
    if traffic.index.is_empty() {
        return;
    }
    let mut proxies = std::mem::take(&mut physics.network_proxies);
    let mut list: Vec<_> = traffic.index.values().map(|(e, _)| (&e.meta, &e.motion)).collect();
    list.sort_by_key(|(c, _)| c.id);
    for (car, motion) in list {
        let collision = traffic.data.models.get(&car.model).and_then(|m| m.collision.as_ref());
        proxies.append_solid(proxy_with(car, &motion.curr, motion.velocity, collision), physics, skater, false);
    }
    physics.network_proxies = proxies;
}

/// Grab-scene ids of the cars: object `CAR_GRAB_TAG | serial`, spline / geometry `CAR_GRAB_TAG | serial << 3 | index`
/// (ours: stable per car; retail numbers splines from a global counter, b34).
pub(crate) const CAR_GRAB_TAG: u32 = 0x8000_0000;

/// The cars' authored grab splines into the grab scene each tick (retail: the vehicle provider `82C35B98`, scene
/// slot `+4088`, answering the riding skitch query's mode 255; b49). The car's world transform is the record frame,
/// its velocity the record vector. NOT RETAIL YET: the assembly is a stand-in with the car's id (retail reads it
/// from the car's `+172` interface, object open).
pub(crate) fn push_vehicle_grab_splines(traffic: &TrafficState, physics: &mut crate::physics::GamePhysics) {
    let mut list: Vec<_> = traffic.index.values().map(|(e, _)| (&e.meta, &e.motion)).collect();
    list.sort_by_key(|(c, _)| c.id);
    physics.set_car_rails(car_rails(&list, &traffic.data));
    let objects: Vec<_> = list.into_iter().filter_map(|(car, motion)| car_grab_object(car, motion, &traffic.data)).collect();
    if let Err(e) = physics.set_grab_cars(objects) {
        warn!("LIVING_WORLD traffic: car grab splines rejected: {e}");
    }
}

/// The cars' grind rails at this tick's car transform, id order (the second rail provider of 82C1EAD8, after the
/// props; 82C4C4E0 re-places them with the car matrix on every query, no moving / parked test).
pub(crate) fn car_rails(cars: &[(&TrafficCar, &CarMotion)], data: &VehicleData) -> crate::grind_world::props::PropRailPoses {
    crate::grind_world::props::PropRailPoses(
        cars.iter()
            .filter_map(|(car, motion)| {
                let rails = data.models.get(&car.model)?.grind_rails.clone();
                if rails.is_empty() {
                    return None;
                }
                let t = &motion.curr;
                let axis = |v: Vec3| (t.rotation * v).to_array();
                Some(crate::grind_world::props::PosedRails {
                    id: car.id.serial,
                    tag: crate::grind_world::props::CAR_OWNER_TAG,
                    origin: t.translation.to_array(),
                    basis: [axis(Vec3::X), axis(Vec3::Y), axis(Vec3::Z)],
                    scale: [1.0; 3],
                    rails,
                })
            })
            .collect(),
    )
}

/// One car as a grab-scene object (pure; the engine and the tests use it).
pub(crate) fn car_grab_object(car: &TrafficCar, motion: &CarMotion, data: &VehicleData) -> Option<skate_core::player::offboard::grab_scene::Object> {
    let splines = &data.models.get(&car.model)?.grab_splines;
    Some(grab_object_at(car.id.serial, splines, &motion.curr, motion.velocity))
}

/// A car's grab-scene object from its model's grab splines at transform `t` moving at `velocity` (serial = the car's
/// stable id; the spline keys are `CAR_GRAB_TAG | serial << 3 | index`).
pub(crate) fn grab_object_at(serial: u32, splines: &[CarGrabSpline], t: &Transform, velocity: Vec3) -> skate_core::player::offboard::grab_scene::Object {
    use skate_core::player::offboard::grab_scene::{AssemblyData, Descriptor, Geometry, Object, Provider, Spline};
    let serial = serial & 0x0FFF_FFFF;
    let id = CAR_GRAB_TAG | serial;
    let motion_velocity = velocity;
    let axis = |v: Vec3| {
        let w = t.rotation * v;
        [w.x, w.y, w.z, 0.0]
    };
    Object {
        id,
        provider: Provider::Vehicle,
        disabled: false,
        assembly_ready: true,
        assembly: Some(AssemblyData { identity: id, first_part: None }),
        frame: [axis(Vec3::X), axis(Vec3::Y), axis(Vec3::Z), [t.translation.x, t.translation.y, t.translation.z, 1.0]],
        object_vector_128: [motion_velocity.x, motion_velocity.y, motion_velocity.z, 0.0],
        splines: splines
            .iter()
            .take(8)
            .enumerate()
            .map(|(i, s)| {
                let key = CAR_GRAB_TAG | serial << 3 | i as u32;
                Spline {
                    descriptor: Descriptor { kind: 1, id: key },
                    geometry: std::sync::Arc::new(Geometry {
                        id: key,
                        points: s.points.iter().map(|p| [p[0], p[1], p[2], 1.0]).collect(),
                        approach_vectors: vec![[s.direction[0], s.direction[1], s.direction[2], 0.0]],
                        word_60: s.flags,
                    }),
                    word_272: 0,
                }
            })
            .collect(),
    }
}

/// One-line traffic summary for the debug readout.
pub(crate) fn traffic_readout(traffic: &TrafficState) -> String {
    let moving = traffic.cars.iter().filter(|c| c.speed > 0.5).count();
    let waiting = traffic.cars.iter().filter(|c| matches!(c.entry, Some(Entry::Signal | Entry::Yield | Entry::Blocked)) && c.speed < 0.5).count();
    let mean = if traffic.cars.is_empty() { 0.0 } else { traffic.cars.iter().map(|c| c.speed).sum::<f32>() / traffic.cars.len() as f32 };
    format!("traffic cars {} moving {} waiting {} mean speed {:.1} m/s signal ticks {}", traffic.cars.len(), moving, waiting, mean, traffic.clock.as_ref().map_or(0, |c| c.ticks()))
}
