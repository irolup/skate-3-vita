//! Population core tests against the retail code constants (labels and addresses in
//! `config::retail`). The census ranges and caps used here are the shipped data values, set up
//! by hand as test fixtures (the data-gated tests in `skate-data` read them from the export).

use super::census::{cap_at, pick_category, ring_point, CensusCategory};
use super::config::retail;
use super::skaters::{near_skater_penalty, SkaterData};
use super::*;
use std::collections::BTreeMap;

const PEDS: CensusRange = CensusRange {
    slow: CensusCircle { spawn_inner: 50.0, spawn_outer: 60.0, cull: 70.0, forward_offset: 0.0, speed_kmh: 45.0 },
    fast: CensusCircle { spawn_inner: 50.0, spawn_outer: 80.0, cull: 90.0, forward_offset: 20.0, speed_kmh: 80.0 },
};
const VEHICLES: CensusRange = CensusRange {
    slow: CensusCircle { spawn_inner: 80.0, spawn_outer: 100.0, cull: 110.0, forward_offset: 0.0, speed_kmh: 0.0 },
    fast: CensusCircle { spawn_inner: 80.0, spawn_outer: 100.0, cull: 110.0, forward_offset: 0.0, speed_kmh: 0.0 },
};

fn record(max: u32, cats: &[(&str, f32)]) -> CensusRecord {
    CensusRecord {
        max_population: max,
        categories: cats.iter().map(|(n, w)| CensusCategory { name: n.to_string(), weight: *w }).collect(),
    }
}

/// 2 km x 2 km grid centred on the origin, 4 m cells. Peds: `aletown` (15) everywhere except the
/// strip x > 600 (unpainted); vehicles: `dwntwn` (30) everywhere.
fn map() -> CensusMap {
    let (w, h) = (500u32, 500u32);
    let mut peds = vec![1u16; (w * h) as usize];
    for j in 0..h {
        for i in 0..w {
            if -1000.0 + i as f32 * 4.0 >= 600.0 {
                peds[(j * w + i) as usize] = 0;
            }
        }
    }
    let mut layers = BTreeMap::new();
    layers.insert("livingworld_npc_census".to_string(), peds);
    layers.insert("livingworld_vehicle_census".to_string(), vec![2u16; (w * h) as usize]);
    let grid = CensusGrid { cell: 4.0, origin: [-1000.0, -1000.0], width: w, height: h, names: vec!["aletown".into(), "dwntwn".into()], layers };
    let mut records = BTreeMap::new();
    records.insert("aletown".into(), record(15, &[("adult", 0.2), ("jock", 0.1), ("tourist", 0.1), ("skater", 0.1), ("teen", 0.05), ("business", 0.3), ("bum", 0.15)]));
    records.insert("dwntwn".into(), record(30, &[("hatchbacks", 0.1), ("minivans", 0.1), ("muscles", 0.075), ("sedans", 0.2), ("sports", 0.175), ("suvs", 0.125), ("taxis", 0.1)]));
    CensusMap { grids: vec![grid], records }
}

/// Synthetic roads over the test map: east-west roads every 100 m from z = -1000 to 1000, each a
/// pair of directed 2-lane segments (eastbound 5 m north of the line, westbound 5 m south), 2 km
/// long in 4 m pieces, 8 m wide. No junctions.
pub(super) fn roads() -> &'static traffic::RoadNetwork {
    use traffic::{Curve, PieceInput, RoadInput, SegmentId, SegmentInput};
    static NET: std::sync::OnceLock<traffic::RoadNetwork> = std::sync::OnceLock::new();
    NET.get_or_init(|| {
        let mut segments = Vec::new();
        for k in 0..21u64 {
            let z = -1000.0 + k as f32 * 100.0;
            for (dir, zc) in [(1.0f32, z + 5.0), (-1.0f32, z - 5.0)] {
                let x0 = -1000.0 * dir;
                let pieces = (0..500)
                    .map(|i| {
                        let a = [x0 + dir * 4.0 * i as f32, 0.0, zc];
                        let b = [x0 + dir * 4.0 * (i + 1) as f32, 0.0, zc];
                        // Facing +x the right side is +z (y up): left = -z.
                        let side = |p: [f32; 3], s: f32| [p[0], 0.0, p[2] + s * dir * 4.0];
                        PieceInput { end_distance: 4.0 * (i + 1) as f32, centre: Curve::straight(a, b), left_start: side(a, -1.0), right_start: side(a, 1.0), left_end: side(b, -1.0), right_end: side(b, 1.0) }
                    })
                    .collect();
                let id = 1000 + k * 2 + (dir < 0.0) as u64;
                segments.push(SegmentInput { id: SegmentId(id), from_node: id * 10, from_end: 0, to_node: id * 10 + 1, to_end: 0, length: 2000.0, speed_limit: 14.167, lanes: 2, manoeuvres: 2, district: 0, pieces });
            }
        }
        traffic::RoadNetwork::build(&RoadInput { segments, junctions: Vec::new() }).unwrap()
    })
}

/// The dwntwn vehicle categories with one entity each (4.5 x 1.9 m, 10 chassis colours).
pub(super) fn catalog() -> &'static VehicleCatalog {
    static CAT: std::sync::OnceLock<VehicleCatalog> = std::sync::OnceLock::new();
    CAT.get_or_init(|| {
        let mut c = VehicleCatalog::default();
        for cat in ["hatchbacks", "minivans", "muscles", "sedans", "sports", "suvs", "taxis"] {
            let entity = format!("{cat}01");
            c.categories.insert(cat.to_string(), vec![entity.clone()]);
            c.entities.insert(entity, VehicleEntity { model: format!("vehicle_{cat}01"), chassis_colours: 10, secondary_colours: 1, length: 4.5, width: 1.9 });
        }
        c
    })
}

/// Offline inputs with the test census, roads and vehicle catalog.
pub(super) fn offline<'a>(obs: &'a [Observer], map: &'a CensusMap) -> TickInputs<'a> {
    let mut inputs = TickInputs::offline(obs);
    inputs.census = Some(map);
    inputs.roads = Some(roads());
    inputs.vehicles = Some(catalog());
    inputs
}

fn config() -> PopulationConfig {
    let mut c = PopulationConfig::retail();
    c.pedestrians.range = Some(PEDS);
    c.vehicles.range = Some(VEHICLES);
    c
}

fn still(x: f32, z: f32) -> Observer {
    Observer { position: [x, 0.0, z], velocity: [0.0; 3] }
}

fn spawns(d: &[Decision], kind: Kind) -> Vec<&SpawnRecord> {
    d.iter().filter_map(|d| match d { Decision::Spawn(s) if s.id.kind == kind => Some(s), _ => None }).collect()
}
fn despawns(d: &[Decision], kind: Kind) -> Vec<&DespawnRecord> {
    d.iter().filter_map(|d| match d { Decision::Despawn(s) if s.id.kind == kind => Some(s), _ => None }).collect()
}
fn h(a: Vec3, b: Vec3) -> f32 {
    dist2(a, b).sqrt()
}

/// Skater lines on circles around the origin, every 10 degrees at radii 40, 75, 110 m.
fn skater_data() -> SkaterData {
    let mut lines = Vec::new();
    for (r_i, r) in [40.0f32, 75.0, 110.0].into_iter().enumerate() {
        for a in 0..36 {
            let t = a as f32 * std::f32::consts::TAU / 36.0;
            let mut id = [0u8; 16];
            id[0] = r_i as u8;
            id[1] = a as u8;
            lines.push(SkaterLine { id, start: [r * t.cos(), 0.0, r * t.sin()], heading: t, valid: true, allowed_skaters: u64::MAX, flags: 0 });
        }
    }
    let characters = (0..8)
        .map(|i| SkaterCharacter { key: format!("pro_{i}"), pro_index: Some(i), capabilities: [false; 3], community: false })
        .collect();
    SkaterData { lines, characters }
}

// ---------------------------------------------------------------- census circle

#[test]
fn ped_circle_lerps_by_speed_like_sub_826b7d60() {
    let slow = PEDS.at(30.0);
    assert_eq!((slow.spawn_inner, slow.spawn_outer, slow.cull, slow.forward_offset), (50.0, 60.0, 70.0, 0.0));
    let fast = PEDS.at(100.0);
    assert_eq!((fast.spawn_inner, fast.spawn_outer, fast.cull, fast.forward_offset), (50.0, 80.0, 90.0, 20.0));
    let mid = PEDS.at(62.5);
    assert_eq!((mid.spawn_outer, mid.cull, mid.forward_offset), (70.0, 80.0, 10.0));
    // Vehicles: equal sets (A.key >= B.key) → set A as is.
    let v = VEHICLES.at(120.0);
    assert_eq!((v.spawn_inner, v.spawn_outer, v.cull), (80.0, 100.0, 110.0));
    // Speed key from |velocity| x 3.6: 22.2 m/s = 80 km/h; the centre moves 20 m forward.
    let (c, centre) = PEDS.around(&Observer { position: [0.0; 3], velocity: [0.0, 0.0, 80.0 / 3.6] });
    assert!((c.spawn_outer - 80.0).abs() < 1e-3);
    assert!((centre[2] - 20.0).abs() < 1e-3 && centre[0].abs() < 1e-6);
}

#[test]
fn census_centre_follows_the_3d_velocity_like_sub_826b7530() {
    // Down a 3-4-5 slope at 100 km/h: retail normalises the 3-D velocity, so the 20 m offset
    // splits 12 m forward and 16 m down (the old horizontal direction put all 20 m forward).
    let s = 100.0 / 3.6;
    let (c, centre) = PEDS.around(&Observer { position: [1.0, 50.0, 2.0], velocity: [0.0, -0.8 * s, 0.6 * s] });
    assert_eq!(c.forward_offset, 20.0);
    assert!((centre[0] - 1.0).abs() < 1e-4, "{centre:?}");
    assert!((centre[1] - 34.0).abs() < 1e-3, "{centre:?}");
    assert!((centre[2] - 14.0).abs() < 1e-3, "{centre:?}");
    // Falling straight down still moves the centre (horizontal speed 0).
    let (_, centre) = PEDS.around(&Observer { position: [0.0; 3], velocity: [0.0, -s, 0.0] });
    assert!((centre[1] + 20.0).abs() < 1e-3, "{centre:?}");
}

#[test]
fn ring_points_stay_in_the_ring() {
    let mut rng = Rng::new(3);
    for _ in 0..5000 {
        let p = ring_point(&mut rng, [10.0, 2.0, -5.0], 50.0, 60.0);
        let d = h(p, [10.0, 2.0, -5.0]);
        assert!((50.0 - 1e-3..=60.0 + 1e-3).contains(&d), "{d}");
        assert_eq!(p[1], 2.0);
    }
}

#[test]
fn cap_is_record_max_times_density_truncated_and_zero_when_unpainted() {
    let r = record(30, &[("a", 1.0)]);
    assert_eq!(cap_at(Some(&r), 1.0, false), 30);
    assert_eq!(cap_at(Some(&r), 0.55, false), 16); // 16.5 truncated
    assert_eq!(cap_at(Some(&r), 0.0, true), 30); // zombie: no density scaling
    assert_eq!(cap_at(None, 1.0, true), 0); // unpainted: no record, cap 0 (sub_826B8A28)
}

#[test]
fn category_roll_misses_when_weights_sum_below_one() {
    let r = record(10, &[("only", 0.5)]);
    let mut rng = Rng::new(9);
    let hits = (0..10_000).filter(|_| pick_category(&mut rng, &r).is_some()).count();
    assert!((4_700..5_300).contains(&hits), "{hits}");
    let full = record(10, &[("a", 0.25), ("b", 0.75)]);
    assert!((0..1000).all(|_| pick_category(&mut rng, &full).is_some()));
}

// ---------------------------------------------------------------- census passes

#[test]
fn peds_initial_populate_then_one_spawn_per_pass_in_the_ring_up_to_the_cap() {
    let map = map();
    let obs = [still(0.0, 0.0)];
    let mut inputs = offline(&obs, &map);
    let mut world = LivingWorld::new(config(), 42);
    // Tick 0 is the peds slot: initial populate (8-80 m), up to the cap of 15.
    let d = world.step(&inputs);
    let first = spawns(&d, Kind::Pedestrian);
    assert_eq!(first.len(), 15);
    assert!(first.iter().all(|s| s.initial && (8.0 - 1e-3..=80.0 + 1e-3).contains(&h(s.position, obs[0].position))));
    assert!(first.iter().all(|s| (0.0..std::f32::consts::TAU + 1e-4).contains(&s.heading)));
    // Remove the far ones (beyond 70 m) the way the cull would, then the regular passes refill
    // one per pass, only on ped ticks (tick % 4 == 0), always 50-60 m away.
    for _ in 1..2000 {
        let t = world.tick();
        let d = world.step(&inputs);
        let s = spawns(&d, Kind::Pedestrian);
        assert!(s.len() <= retail::SPAWNS_PER_PASS as usize);
        if !s.is_empty() {
            assert_eq!(t % 4, 0, "ped spawns run in rotation slot 0");
            let r = h(s[0].position, obs[0].position);
            assert!((50.0 - 1e-3..=60.0 + 1e-3).contains(&r), "spawn at {r}");
            assert!(!s[0].initial);
        }
        assert!(world.count(Kind::Pedestrian) <= 15);
        for x in despawns(&d, Kind::Pedestrian) {
            assert_eq!(x.reason, DespawnReason::Distance);
        }
    }
}

#[test]
fn census_cull_at_the_cull_radius() {
    let map = map();
    let mut world = LivingWorld::new(config(), 1);
    let obs = [still(0.0, 0.0)];
    let mut inputs = offline(&obs, &map);
    world.step(&inputs);
    let ids: Vec<_> = world.live(Kind::Pedestrian).map(|l| l.id).take(2).collect();
    world.update_position(ids[0], [69.9, 0.0, 0.0]);
    world.update_position(ids[1], [70.1, 0.0, 0.0]);
    let others: Vec<_> = world.live(Kind::Pedestrian).map(|l| l.id).skip(2).collect();
    for id in others {
        world.update_position(id, [0.0, 0.0, 10.0]);
    }
    for _ in 0..3 {
        world.step(&inputs); // vehicles, DMOs, props slots
    }
    let d = world.step(&inputs); // next ped pass: cull first
    let gone = despawns(&d, Kind::Pedestrian);
    assert_eq!(gone.len(), 1);
    assert_eq!(gone[0].id, ids[1]);
    // Height counts too (3-D distance): 70.1 m straight up goes.
    world.update_position(ids[0], [0.0, 70.1, 0.0]);
    for _ in 0..3 {
        world.step(&inputs);
    }
    assert!(despawns(&world.step(&inputs), Kind::Pedestrian).iter().any(|x| x.id == ids[0]));
}

#[test]
fn unpainted_ground_spawns_no_peds() {
    let map = map();
    let obs = [still(800.0, 0.0)]; // the ring 8-80 m stays in x > 600: unpainted for peds
    let mut inputs = offline(&obs, &map);
    let mut world = LivingWorld::new(config(), 5);
    for _ in 0..400 {
        world.step(&inputs);
    }
    assert_eq!(world.count(Kind::Pedestrian), 0);
    assert!(world.count(Kind::Vehicle) > 0, "the vehicle layer is painted there");
}

#[test]
fn vehicles_ring_80_100_cull_110_cap_30_rotation_slot_1() {
    let map = map();
    let obs = [still(0.0, 0.0)];
    let mut inputs = offline(&obs, &map);
    let mut world = LivingWorld::new(config(), 77);
    let mut regular = 0;
    for _ in 0..4000 {
        let t = world.tick();
        let d = world.step(&inputs);
        for s in spawns(&d, Kind::Vehicle) {
            assert_eq!(t % 4, 1);
            // The car sits on the lane at the start of the piece under the ring point: up to
            // 4 m back along the road and 6 m across from the point.
            let r = h(s.position, obs[0].position);
            if s.initial {
                assert!(r <= 80.0 + 7.3, "{r}");
            } else {
                regular += 1;
                assert!((80.0 - 7.3..=100.0 + 7.3).contains(&r), "{r}");
            }
        }
        assert!(world.count(Kind::Vehicle) <= 15, "vehicle limit 15 under the cap of 30");
    }
    assert_eq!(world.count(Kind::Vehicle), 15);
    assert_eq!(regular, 0, "nothing culls a still observer's cars, so the cap stays full");
    let id = world.live(Kind::Vehicle).next().unwrap().id;
    world.update_position(id, [0.0, 0.0, 110.5]);
    while world.tick() % 4 != 1 {
        world.step(&inputs);
    }
    assert_eq!(despawns(&world.step(&inputs), Kind::Vehicle)[0].id, id);
}

#[test]
fn free_play_scales_caps_and_zero_removes_all_at_once() {
    let map = map();
    let obs = [still(0.0, 0.0)];
    let mut inputs = offline(&obs, &map);
    inputs.free_play = Some(FreePlay { traffic: 0.5, pedestrians: 1.0, ai_skaters: true });
    let mut world = LivingWorld::new(config(), 8);
    for _ in 0..400 {
        world.step(&inputs);
    }
    assert_eq!(world.count(Kind::Vehicle), 15); // trunc(30 x 0.5) = 15 = the vehicle limit
    assert_eq!(world.count(Kind::Pedestrian), 15);
    inputs.free_play = Some(FreePlay { traffic: 0.0, pedestrians: 0.0, ai_skaters: true });
    let mut gone = Vec::new();
    for _ in 0..4 {
        gone.extend(world.step(&inputs));
    }
    assert_eq!(despawns(&gone, Kind::Pedestrian).len(), 15);
    assert_eq!(despawns(&gone, Kind::Vehicle).len(), 15);
    assert!(despawns(&gone, Kind::Vehicle).iter().all(|d| d.reason == DespawnReason::FreePlayOff));
    for _ in 0..200 {
        assert!(spawns(&world.step(&inputs), Kind::Pedestrian).is_empty());
    }
    // Outside Free Play the option values do nothing (scale 1.0); the vehicle limit holds.
    inputs.free_play = None;
    for _ in 0..400 {
        world.step(&inputs);
    }
    assert_eq!(world.count(Kind::Vehicle), 15);
}

#[test]
fn online_spawns_nothing_but_still_culls() {
    let map = map();
    let mut world = LivingWorld::new(config(), 11);
    let data = skater_data();
    let obs = [still(0.0, 0.0)];
    let mut inputs = offline(&obs, &map);
    inputs.skater_world = Some(&data);
    for _ in 0..240 {
        world.step(&inputs);
    }
    assert!(world.count(Kind::Pedestrian) > 0 && world.count(Kind::Vehicle) > 0 && world.count(Kind::Skater) > 0);
    inputs.online = true;
    let far = [still(5000.0, 5000.0)];
    let mut online = inputs;
    online.observers = &far;
    let mut all = Vec::new();
    for _ in 0..240 {
        all.extend(world.step(&online));
    }
    assert!(all.iter().all(|d| matches!(d, Decision::Despawn(_))), "online: no spawns of any kind");
    assert_eq!(world.count(Kind::Pedestrian), 0);
    assert_eq!(world.count(Kind::Vehicle), 0);
    assert_eq!(world.count(Kind::Skater), 0);
}

#[test]
fn zombie_mode_peds_ignore_the_cap_no_traffic_no_skaters() {
    let map = map();
    let data = skater_data();
    let obs = [still(0.0, 0.0)];
    let mut inputs = offline(&obs, &map);
    inputs.skater_world = Some(&data);
    inputs.zombie = true;
    let mut world = LivingWorld::new(config(), 13);
    for _ in 0..400 {
        world.step(&inputs);
    }
    assert_eq!(world.count(Kind::Pedestrian), 31, "no census cap in zombie mode; the pool of 31 limits");
    assert_eq!(world.count(Kind::Vehicle), 0);
    assert_eq!(world.count(Kind::Skater), 0);
}

/// Turning zombie mode on removes the traffic already driving (`sub_826BAAB8`: cull beyond the radius OR zombie) and
/// keeps the peds.
#[test]
fn zombie_mode_culls_the_live_traffic() {
    let map = map();
    let obs = [still(0.0, 0.0)];
    let mut inputs = offline(&obs, &map);
    let mut world = LivingWorld::new(config(), 13);
    for _ in 0..400 {
        world.step(&inputs);
    }
    let (peds, cars) = (world.count(Kind::Pedestrian), world.count(Kind::Vehicle));
    assert!(cars > 0 && peds > 0, "traffic and peds before: {cars} / {peds}");
    inputs.zombie = true;
    // The census kinds take turns (rotation slots): one round of passes.
    let decisions: Vec<_> = (0..8).flat_map(|_| world.step(&inputs)).collect();
    assert_eq!(world.count(Kind::Vehicle), 0);
    assert!(world.count(Kind::Pedestrian) >= peds, "peds stay");
    assert!(decisions.iter().any(|d| matches!(d, Decision::Despawn(r) if r.reason == DespawnReason::Disabled)));
}

#[test]
fn disabled_kind_despawns_and_density_setting_scales() {
    let map = map();
    let obs = [still(0.0, 0.0)];
    let mut inputs = offline(&obs, &map);
    let mut cfg = config();
    cfg.vehicles.density = 0.2;
    let mut world = LivingWorld::new(cfg, 2);
    for _ in 0..400 {
        world.step(&inputs);
    }
    assert_eq!(world.count(Kind::Vehicle), 6);
    world.config.pedestrians.enabled = false;
    let d: Vec<_> = (0..4).flat_map(|_| world.step(&inputs)).collect();
    assert!(despawns(&d, Kind::Pedestrian).iter().all(|x| x.reason == DespawnReason::Disabled));
    assert_eq!(world.count(Kind::Pedestrian), 0);
}

#[test]
fn no_export_means_no_census_population() {
    let map = map();
    let obs = [still(0.0, 0.0)];
    let mut inputs = offline(&obs, &map);
    let mut world = LivingWorld::new(PopulationConfig::retail(), 2);
    for _ in 0..100 {
        world.step(&inputs);
    }
    assert_eq!(world.count(Kind::Pedestrian) + world.count(Kind::Vehicle), 0);
}

// ---------------------------------------------------------------- skaters

#[test]
fn skaters_three_ambient_spawn_on_phase_30_at_60_to_90_m() {
    let data = skater_data();
    let obs = [still(0.0, 0.0)];
    let mut inputs = TickInputs::offline(&obs);
    inputs.skater_world = Some(&data);
    let mut world = LivingWorld::new(config(), 21);
    for _ in 0..(60 * 20) {
        let t = world.tick();
        let d = world.step(&inputs);
        let s = spawns(&d, Kind::Skater);
        assert!(s.len() <= 1);
        if let Some(s) = s.first() {
            assert_eq!(t % 60, 30);
            let r = h(s.position, obs[0].position);
            assert!((60.0 - 1e-3..=90.0 + 1e-3).contains(&r), "{r}");
            let SpawnChoice::Skater { slot, .. } = &s.choice else { panic!() };
            assert!((1..7).contains(slot));
        }
        assert!(world.count(Kind::Skater) <= 3);
    }
    assert_eq!(world.count(Kind::Skater), 3);
    assert_eq!(world.skater_pool().len(), 5);
}

#[test]
fn skaters_never_spawn_within_5_m_of_another_skater() {
    // Two lines only: one 3 m from a standing player 75 m out, one free.
    let mut data = skater_data();
    data.lines.truncate(0);
    let mk = |b: u8, p: Vec3| SkaterLine { id: [b; 16], start: p, heading: 0.0, valid: true, allowed_skaters: u64::MAX, flags: 0 };
    data.lines.push(mk(1, [75.0, 0.0, 0.0]));
    data.lines.push(mk(2, [0.0, 0.0, 75.0]));
    let obs = [still(0.0, 0.0), still(77.0, 2.0)]; // a second skater (remote player later) near line 1
    let mut inputs = TickInputs::offline(&obs);
    inputs.skater_world = Some(&data);
    inputs.player_slots = 2;
    let mut world = LivingWorld::new(config(), 4);
    let mut lines = Vec::new();
    for _ in 0..(60 * 10) {
        for s in spawns(&world.step(&inputs), Kind::Skater) {
            let SpawnChoice::Skater { line, slot, .. } = &s.choice else { panic!() };
            lines.push(line[0]);
            assert!(*slot >= 2, "slots 0 and 1 belong to the players");
        }
    }
    assert_eq!(lines, vec![2], "line 1 starts 2.8 m from a skater: rejected (d^2 < 25)");
}

#[test]
fn skater_near_term_and_cull() {
    let cfg = SkaterConfig::retail();
    assert_eq!(near_skater_penalty(100.0, &cfg), 0); // 10 m: no term
    assert_eq!(near_skater_penalty(0.1, &cfg), 600);
    assert_eq!(near_skater_penalty(64.0, &cfg), ((10.0f32 - 8.0) * retail::SKATER_NEAR_K * 600.0) as i64);
    let data = skater_data();
    let obs = [still(0.0, 0.0)];
    let mut inputs = TickInputs::offline(&obs);
    inputs.skater_world = Some(&data);
    let mut world = LivingWorld::new(config(), 6);
    while world.count(Kind::Skater) < 1 {
        world.step(&inputs);
    }
    let id = world.live(Kind::Skater).next().unwrap().id;
    world.update_position(id, [119.0, 0.0, 0.0]);
    let mut d = Vec::new();
    for _ in 0..60 {
        d.extend(world.step(&inputs));
    }
    assert!(despawns(&d, Kind::Skater).is_empty());
    world.update_position(id, [121.0, 0.0, 0.0]);
    let mut d = Vec::new();
    for _ in 0..60 {
        d.extend(world.step(&inputs));
    }
    let gone = despawns(&d, Kind::Skater);
    assert_eq!(gone.len(), 1);
    assert_eq!((gone[0].id, gone[0].reason), (id, DespawnReason::Distance));
    assert_eq!(gone[0].tick % 60, 0, "the cull runs on phase 0");
}

#[test]
fn skaters_free_play_off_and_online_excess() {
    let data = skater_data();
    let obs = [still(0.0, 0.0)];
    let mut inputs = TickInputs::offline(&obs);
    inputs.skater_world = Some(&data);
    let mut world = LivingWorld::new(config(), 31);
    for _ in 0..(60 * 10) {
        world.step(&inputs);
    }
    assert_eq!(world.count(Kind::Skater), 3);
    inputs.free_play = Some(FreePlay { ai_skaters: false, ..FreePlay::default() });
    while world.tick() % 60 != 0 {
        world.step(&inputs);
    }
    world.step(&inputs); // phase 0, 1, 2: no check yet
    world.step(&inputs);
    world.step(&inputs);
    assert_eq!(world.count(Kind::Skater), 3);
    let d = world.step(&inputs); // phase 3: per-skater checks despawn all
    assert_eq!(despawns(&d, Kind::Skater).len(), 3);
    for _ in 0..600 {
        assert!(spawns(&world.step(&inputs), Kind::Skater).is_empty());
    }
    // Online: desired 0, the phase-0 cull removes the excess even nearby.
    inputs.free_play = None;
    for _ in 0..600 {
        world.step(&inputs);
    }
    assert_eq!(world.count(Kind::Skater), 3);
    inputs.online = true;
    let mut d = Vec::new();
    for _ in 0..60 {
        d.extend(world.step(&inputs));
    }
    assert!(despawns(&d, Kind::Skater).iter().all(|x| x.reason == DespawnReason::Excess));
    assert_eq!(world.count(Kind::Skater), 0);
}

#[test]
fn skater_slots_are_shared_with_players() {
    let data = skater_data();
    let obs = [still(0.0, 0.0)];
    let mut inputs = TickInputs::offline(&obs);
    inputs.skater_world = Some(&data);
    inputs.player_slots = 5; // 5 players: slots 5 and 6 are left
    let mut world = LivingWorld::new(config(), 3);
    for _ in 0..(60 * 10) {
        world.step(&inputs);
    }
    assert_eq!(world.count(Kind::Skater), 2);
}

// ---------------------------------------------------------------- determinism and ids

fn run(seed: u64) -> Vec<Decision> {
    run_world(seed).1
}

fn run_world(seed: u64) -> (LivingWorld, Vec<Decision>) {
    let map = map();
    let data = skater_data();
    let mut world = LivingWorld::new(config(), seed);
    let mut out = Vec::new();
    for step in 0..6000u32 {
        // A player moving in a circle of 150 m at 8 m/s (100 s of 60 Hz world ticks).
        let t = (step as f64 / super::clock::RETAIL_TICK_HZ) as f32;
        let a = t * 8.0 / 150.0;
        let obs = [Observer { position: [150.0 * a.cos(), 0.0, 150.0 * a.sin()], velocity: [-8.0 * a.sin(), 0.0, 8.0 * a.cos()] }];
        let mut inputs = offline(&obs, &map);
        inputs.skater_world = Some(&data);
        out.extend(world.step(&inputs));
    }
    (world, out)
}

#[test]
fn same_inputs_same_decision_stream() {
    let a = run(1234);
    let b = run(1234);
    assert!(a.len() > 50);
    assert_eq!(a, b);
    assert_ne!(a, run(1235));
}

#[test]
fn ids_are_stable_and_unique_and_a_client_can_mirror_the_records() {
    let (host, a) = run_world(99);
    let mut seen = std::collections::BTreeSet::new();
    let mut client = LivingWorld::new(config(), 0);
    for d in &a {
        if let Decision::Spawn(s) = d {
            assert!(seen.insert(s.id), "id reused: {:?}", s.id);
            assert_eq!(LivingWorldId::from_u64(s.id.to_u64()), Some(s.id));
        }
        client.apply(d);
    }
    // The client's roster equals the host's after replaying the records (no rules, no RNG).
    for k in Kind::ALL {
        let x: Vec<_> = client.live(k).map(|l| l.id).collect();
        let y: Vec<_> = host.live(k).map(|l| l.id).collect();
        assert_eq!(x, y);
    }
}

// ---------------------------------------------------------------- vehicles (milestone V2)

use super::traffic::spawn::{candidate_lanes, lane_fits, overlaps, pick_lane, road_under};
use super::traffic::{LaneCar, PlacementRules};

#[test]
fn vehicle_code_constants() {
    // Census +148 = 15 (sub_826B6D58) and the literal 15 of the initial populate (sub_826B83C8).
    assert_eq!((retail::VEHICLE_LIMIT, retail::VEHICLE_INITIAL_LIMIT), (15, 15));
    let v = config::CensusKindConfig::retail_vehicles();
    assert_eq!((v.pool, v.initial_pool), (Some(15), Some(15)));
    assert_eq!((v.attempts_per_pass, v.spawns_per_pass, v.initial_attempts, v.initial_spawns), (2, 1, 6000, 600));
    assert_eq!(v.initial_ring, (8.0, 80.0));
    assert!(!v.spawn_in_zombie && v.cull_in_zombie);
    assert_eq!(v.rotation_slot, 1);
    // Factory placement (sub_82C36300 / sub_82E14928): 15 m margin, extra 0, half 0.5, 20 m
    // overlap query, lane roll 100.
    let r = PlacementRules::default();
    assert_eq!((r.end_margin, r.extra_ahead, r.half, r.behind_seconds, r.clear_radius, r.lane_roll), (15.0, 0.0, 0.5, 1.0, 20.0, 100));
    assert!(config::CensusKindConfig::retail_pedestrians().placement.is_none());
}

#[test]
fn entity_and_lane_rolls_follow_the_code_formulas() {
    // trunc(u32 x 100 / 2^32) mod n.
    let lanes = [0u8, 1];
    assert_eq!(pick_lane(&lanes, 0, &PlacementRules::default()), 0);
    assert_eq!(pick_lane(&lanes, (4_294_967_296.0 * 0.015) as u32, &PlacementRules::default()), 1); // roll 1
    assert_eq!(pick_lane(&lanes, (4_294_967_296.0 * 0.025) as u32, &PlacementRules::default()), 0); // roll 2
    let mut rng = Rng::new(9);
    let list: Vec<String> = ["a", "b", "c"].iter().map(|s| s.to_string()).collect();
    let mut seen = std::collections::BTreeSet::new();
    for _ in 0..200 {
        seen.insert(census::pick_entity(&mut rng, &list).unwrap().clone());
    }
    assert_eq!(seen.len(), 3);
    assert!(census::pick_entity(&mut rng, &[]).is_none());
}

#[test]
fn road_under_needs_the_road_surface() {
    let net = roads();
    // Eastbound road of the z = 0 line: centre z = 5, 8 m wide.
    let (seg, d) = road_under(net, [10.0, 0.0, 6.0]).unwrap();
    assert_eq!(net.segments[seg].id.0, 1000 + 10 * 2);
    assert_eq!(d, 1008.0, "the start of the piece under the point");
    assert!(road_under(net, [10.0, 0.0, 9.5]).is_none(), "beside the road");
    assert!(road_under(net, [10.0, 0.0, 50.0]).is_none(), "between roads");
}

#[test]
fn lane_fit_rules() {
    let net = roads();
    let seg = net.segment_index(traffic::SegmentId(1020)).unwrap();
    let r = PlacementRules::default();
    // 15 m from both segment ends.
    assert!(!lane_fits(net, seg, 0, 14.9, &r, &[]));
    assert!(lane_fits(net, seg, 0, 15.0, &r, &[]));
    assert!(lane_fits(net, seg, 0, 1985.0, &r, &[]));
    assert!(!lane_fits(net, seg, 0, 1985.1, &r, &[]));
    assert!(!lane_fits(net, seg, 2, 500.0, &r, &[]), "no third lane");
    // Ahead: its rear (distance - 0.5 x length) at least 15 m past the point.
    let ahead = |d: f32| [LaneCar { distance: d, length: 4.0, speed: 0.0 }];
    assert!(lane_fits(net, seg, 0, 500.0, &r, &ahead(517.0)));
    assert!(!lane_fits(net, seg, 0, 500.0, &r, &ahead(516.9)));
    // Behind: distance + length + speed x 1 s at most 7.5 m (half the margin) before the point.
    let behind = |d: f32, v: f32| [LaneCar { distance: d, length: 4.0, speed: v }];
    assert!(lane_fits(net, seg, 0, 500.0, &r, &behind(488.5, 0.0)));
    assert!(!lane_fits(net, seg, 0, 500.0, &r, &behind(488.6, 0.0)));
    assert!(!lane_fits(net, seg, 0, 500.0, &r, &behind(480.0, 10.0)), "a fast follower needs room");
    // The other lane is independent.
    let cars_on = |_: usize, l: u8| if l == 0 { ahead(510.0).to_vec() } else { Vec::new() };
    let point = net.lane_frame(seg, 0.0, 500.0).position;
    let (_, _, lanes) = candidate_lanes(net, point, &r, &cars_on).unwrap();
    assert_eq!(lanes, vec![1]);
    // Overlap check: radius sum, within the 20 m query.
    assert!(overlaps([0.0; 3], 2.25, &[([4.0, 0.0, 0.0], 2.25)], &r));
    assert!(!overlaps([0.0; 3], 2.25, &[([4.6, 0.0, 0.0], 2.25)], &r));
}

/// Live cars: every one on a real lane at a valid distance, facing its lane's direction, the
/// record matching the lane point, and the lane gaps kept.
fn check_cars(world: &LivingWorld, net: &traffic::RoadNetwork) {
    let r = PlacementRules::default();
    let mut by_lane: BTreeMap<(u64, u8), Vec<f32>> = BTreeMap::new();
    for l in world.live(Kind::Vehicle) {
        let SpawnChoice::Vehicle { segment, lane, distance, model, chassis, .. } = &l.choice else { panic!("vehicle record") };
        let si = net.segment_index(traffic::SegmentId(*segment)).expect("a real segment");
        let s = &net.segments[si];
        assert!(*lane < s.lanes);
        assert!(*distance >= r.end_margin && *distance <= s.length - r.end_margin);
        let frame = net.lane_frame(si, *lane as f32, *distance);
        assert!(h(frame.position, l.position) < 1e-3);
        assert!(model.starts_with("vehicle_") && *chassis < 10);
        by_lane.entry((*segment, *lane)).or_default().push(*distance);
    }
    for v in by_lane.values_mut() {
        v.sort_by(f32::total_cmp);
        for w in v.windows(2) {
            // A new car behind keeps half the margin plus its length (the car-ahead rule is
            // stricter, 15 m + half a length): the smaller one bounds every gap.
            assert!(w[1] - w[0] >= 0.5 * r.end_margin + 4.5 - 1e-3, "gap {w:?}");
        }
    }
}

#[test]
fn spawned_cars_sit_on_lanes_in_their_direction_with_gaps() {
    let map = map();
    let net = roads();
    let obs = [still(0.0, 0.0)];
    let inputs = offline(&obs, &map);
    let mut world = LivingWorld::new(config(), 21);
    let mut all = Vec::new();
    for _ in 0..2000 {
        all.extend(world.step(&inputs));
        check_cars(&world, net);
    }
    assert_eq!(world.count(Kind::Vehicle), 15);
    for s in spawns(&all, Kind::Vehicle) {
        let SpawnChoice::Vehicle { segment, lane, distance, .. } = &s.choice else { panic!() };
        let si = net.segment_index(traffic::SegmentId(*segment)).unwrap();
        let frame = net.lane_frame(si, *lane as f32, *distance);
        assert!((s.heading - frame.yaw()).abs() < 1e-5, "heading = the lane direction");
        let along = net.segments[si].pieces[0].centre.derivative(0.0);
        let east = along[0] > 0.0;
        assert_eq!(east, *segment % 2 == 0, "eastbound segments carry eastbound cars");
    }
}

#[test]
fn no_roads_or_no_catalog_means_no_cars() {
    let map = map();
    let obs = [still(0.0, 0.0)];
    for (roads_on, cat_on) in [(false, true), (true, false)] {
        let mut inputs = offline(&obs, &map);
        if !roads_on {
            inputs.roads = None;
        }
        if !cat_on {
            inputs.vehicles = None;
        }
        let mut world = LivingWorld::new(config(), 4);
        for _ in 0..400 {
            world.step(&inputs);
        }
        assert_eq!(world.count(Kind::Vehicle), 0);
        assert!(world.count(Kind::Pedestrian) > 0);
    }
}

#[test]
fn a_car_update_moves_its_lane_gap() {
    let map = map();
    let obs = [still(0.0, 0.0)];
    let inputs = offline(&obs, &map);
    let mut world = LivingWorld::new(config(), 31);
    world.step(&inputs);
    world.step(&inputs); // tick 1: the vehicle initial populate
    let first = world.live(Kind::Vehicle).next().unwrap().clone();
    let lane = first.lane.unwrap();
    world.update_lane(first.id, lane.segment, lane.lane, lane.distance + 40.0, 12.0);
    let now = world.live(Kind::Vehicle).find(|l| l.id == first.id).unwrap().lane.unwrap();
    assert_eq!((now.distance, now.speed), (lane.distance + 40.0, 12.0));
}

// ---------------------------------------------------------------- guard: a broken focus

/// Doc 26 "Cars flying off": an observer with a non-finite position (an infinite deck position
/// put every NPC beyond the cull) or speed never empties the population; the pass neither culls
/// nor spawns until the focus is usable again, then the rules run as before.
#[test]
fn a_non_finite_focus_never_empties_the_population() {
    let m = map();
    let mut w = LivingWorld::new(config(), 7);
    let here = [still(0.0, 0.0)];
    for _ in 0..600 {
        w.step(&offline(&here, &m));
    }
    let (peds, cars) = (w.count(Kind::Pedestrian), w.count(Kind::Vehicle));
    assert!(peds > 0 && cars > 0, "setup: peds {peds} cars {cars}");
    for bad in [
        Observer { position: [f32::INFINITY, 0.0, 0.0], velocity: [0.0; 3] },
        Observer { position: [f32::NAN, 0.0, 0.0], velocity: [0.0; 3] },
        Observer { position: [0.0; 3], velocity: [f32::INFINITY, 0.0, 0.0] },
        Observer { position: [0.0; 3], velocity: [f32::NAN, 0.0, 0.0] },
    ] {
        let obs = [bad];
        let mut out = Vec::new();
        for _ in 0..120 {
            out.extend(w.step(&offline(&obs, &m)));
        }
        assert!(out.is_empty(), "{bad:?}: {} decisions", out.len());
        assert_eq!((w.count(Kind::Pedestrian), w.count(Kind::Vehicle)), (peds, cars), "{bad:?}");
    }
    // A real jump (teleport) still culls by the retail rules.
    let far = [still(5000.0, 5000.0)];
    let mut out = Vec::new();
    for _ in 0..120 {
        out.extend(w.step(&offline(&far, &m)));
    }
    assert_eq!(despawns(&out, Kind::Pedestrian).len(), peds);
}


// ---------------------------------------------------------------- NPC draw distance (QoL, not retail)

fn run_world_with(seed: u64, config: PopulationConfig) -> (LivingWorld, Vec<Decision>) {
    let map = map();
    let data = skater_data();
    let mut world = LivingWorld::new(config, seed);
    let mut out = Vec::new();
    for step in 0..6000u32 {
        let t = (step as f64 / super::clock::RETAIL_TICK_HZ) as f32;
        let a = t * 8.0 / 150.0;
        let obs = [Observer { position: [150.0 * a.cos(), 0.0, 150.0 * a.sin()], velocity: [-8.0 * a.sin(), 0.0, 8.0 * a.cos()] }];
        let mut inputs = offline(&obs, &map);
        inputs.skater_world = Some(&data);
        out.extend(world.step(&inputs));
    }
    (world, out)
}

#[test]
fn draw_distance_retail_runs_the_unchanged_config() {
    // 1x builds no scaled copy at all: the population runs the retail config itself.
    assert!(DrawDistance::new(1.0).scaled(&config()).is_none());
    // Bad values from a settings file or a mod fall back to retail or are clamped.
    for bad in [f32::NAN, f32::INFINITY, 0.0, -2.0] {
        assert!(DrawDistance::new(bad).is_retail(), "{bad}");
    }
    assert_eq!(DrawDistance::new(100.0).multiplier(), DrawDistance::MAX);
    // Same seeded run (players, peds, cars, 100 s) with the field set explicitly to 1: identical
    // decision stream (the pre-change stream was also diffed byte for byte, doc 26).
    for seed in [1234u64, 99] {
        let mut explicit = config();
        explicit.draw_distance = 1.0;
        assert_eq!(run_world_with(seed, explicit).1, run_world(seed).1);
    }
}

#[test]
fn draw_distance_2x_doubles_every_range_and_quadruples_the_caps() {
    let base = config();
    let s = DrawDistance::new(2.0).scaled(&base).expect("2x scales");
    for (b, x) in [(&base.pedestrians, &s.pedestrians), (&base.vehicles, &s.vehicles)] {
        let (br, xr) = (b.range.unwrap(), x.range.unwrap());
        for (bc, xc) in [(br.slow, xr.slow), (br.fast, xr.fast)] {
            assert_eq!(xc.spawn_inner, 2.0 * bc.spawn_inner);
            assert_eq!(xc.spawn_outer, 2.0 * bc.spawn_outer);
            assert_eq!(xc.cull, 2.0 * bc.cull);
            assert_eq!(xc.forward_offset, 2.0 * bc.forward_offset);
            assert_eq!(xc.speed_kmh, bc.speed_kmh, "speed keys stay");
        }
        assert_eq!(x.density, 4.0 * b.density);
        assert_eq!(x.pool, b.pool.map(|p| 4 * p));
        assert_eq!(x.initial_ring, (2.0 * b.initial_ring.0, 2.0 * b.initial_ring.1));
        assert_eq!((x.attempts_per_pass, x.spawns_per_pass), (4 * b.attempts_per_pass, 4 * b.spawns_per_pass));
    }
    assert_eq!((s.skaters.spawn_inner, s.skaters.spawn_outer, s.skaters.cull), (120.0, 180.0, 240.0));
    assert_eq!((s.skaters.desired, s.skaters.ai_cap, s.skaters.pool_size), (12, 20, 20));
    assert_eq!(s.skaters.slots, 1 + 4 * (retail::SKATER_SLOTS - 1), "player slot 0 stays one slot");
    // The retail config itself is untouched (the multiplier sits on top).
    assert_eq!(base, config());

    // Simulation: a still player in `aletown` (cap 15). Retail fills to 15 within 80 m; 2x fills
    // to 4 x 15 = 60 within 160 m, the near area as busy as retail.
    let map = map();
    let obs = [still(0.0, 0.0)];
    let inputs = offline(&obs, &map);
    let mut counts = Vec::new();
    for m in [1.0f32, 2.0] {
        let mut c = config();
        c.draw_distance = m;
        let mut world = LivingWorld::new(c, 42);
        let mut all = Vec::new();
        for _ in 0..600 {
            all.extend(world.step(&inputs));
        }
        let peds = spawns(&all, Kind::Pedestrian);
        assert!(peds.iter().all(|s| h(s.position, obs[0].position) <= 80.0 * m + 1e-3));
        let near = world.live(Kind::Pedestrian).filter(|l| h(l.position, obs[0].position) <= 80.0).count();
        counts.push((world.count(Kind::Pedestrian), near));
        // Cars: ring 80-100 m (x m), cap 30 x m^2 but the vehicle limit 15 x m^2.
        let cars = spawns(&all, Kind::Vehicle);
        assert!(cars.iter().filter(|s| !s.initial).all(|s| h(s.position, obs[0].position) <= 100.0 * m + 6.0));
        assert!(world.count(Kind::Vehicle) as u32 <= retail::VEHICLE_LIMIT * (m * m) as u32);
    }
    assert_eq!(counts[0].0, 15);
    assert_eq!(counts[1].0, 60, "2x range = 4x peds");
    assert!(counts[1].1 >= 8, "the retail area keeps a retail-like share: {:?}", counts);
}

#[test]
fn draw_distance_keeps_the_fade_before_the_cull_at_every_step() {
    use super::peds::{draw_alpha, PedFadeConfig};
    let fade = PedFadeConfig::default();
    for m in DrawDistance::MENU_STEPS {
        let dd = DrawDistance::new(m);
        let mut c = config();
        c.draw_distance = m;
        let s = dd.scaled(&c).unwrap_or(c.clone());
        let r = s.pedestrians.range.unwrap();
        // The pair the engine draws with (model pair x m) against the near edge of either cull
        // circle (cull - forward offset), with the camera up to 10 m nearer than the skater.
        let pair = [dd.distance(45.0), dd.distance(55.0)];
        for circle in [r.slow, r.fast] {
            let edge = circle.cull - circle.forward_offset;
            assert!(pair[1] < edge, "{m}x: fade ends at {} m, cull edge {edge} m", pair[1]);
            for slack in [0.0f32, 5.0, 10.0] {
                assert_eq!(draw_alpha(&fade, Some(pair), edge - slack, 10.0), 0.0, "{m}x slack {slack}");
            }
            assert!(circle.spawn_outer < circle.cull, "{m}x peds spawn inside the cull");
        }
        let v = s.vehicles.range.unwrap();
        assert!(v.slow.spawn_outer < v.slow.cull && v.fast.spawn_outer < v.fast.cull, "{m}x cars");
        assert!(s.skaters.spawn_outer < s.skaters.cull, "{m}x skaters spawn inside the cull");
    }
}
