//! Data-gated checks of the vehicle census (milestone V2) on the user's own export: a seeded run
//! over the DownTown road network with the shipped census, ranges and vehicles. Skips (passes
//! with a note) when no data is configured. `SKATE3_ASSET_ROOT`: converted assets (one root, or
//! several joined like PATH); reads `private/living_world/{tables.json, DownTown.census.bin,
//! roads.bin, vehicles.json}`.
//!
//! Asserted (shipped data and code rules, nothing embedded in the engine): the DownTown cap 30
//! and the vehicle limit 15 hold, every car sits on a real lane at least 15 m from the segment
//! ends facing its lane's direction, cars on one lane keep the factory gaps, and the same inputs
//! give the same decision stream.

use skate_core::living_world::traffic::{PlacementRules, RoadNetwork, SegmentId};
use skate_core::living_world::{Decision, Kind, LivingWorld, Observer, PopulationConfig, SpawnChoice, TickInputs, VehicleCatalog};
use skate_data::living_world::{self, LivingWorldTables};
use skate_data::roads::RoadGraph;
use std::collections::BTreeMap;
use std::path::PathBuf;

fn find(name: &str) -> Option<PathBuf> {
    let raw = std::env::var_os("SKATE3_ASSET_ROOT")?;
    std::env::split_paths(&raw).flat_map(|root| [format!("private/living_world/{name}"), format!("living_world/{name}")].map(|p| root.join(p))).find(|p| p.is_file())
}

struct Data {
    config: PopulationConfig,
    census: skate_core::living_world::CensusMap,
    roads: RoadNetwork,
    catalog: VehicleCatalog,
}

fn data() -> Option<Data> {
    let tables = LivingWorldTables::parse(&std::fs::read(find("tables.json")?).unwrap()).unwrap();
    let grid = living_world::parse_census_grid(&std::fs::read(find("DownTown.census.bin")?).unwrap()).unwrap();
    let roads = RoadNetwork::build(&RoadGraph::parse(&std::fs::read(find("roads.bin")?).unwrap()).unwrap().traffic_input()).unwrap();
    let catalog = living_world::vehicle_catalog(&std::fs::read(find("vehicles.json")?).unwrap()).unwrap();
    let mut config = PopulationConfig::retail();
    tables.apply_to(&mut config);
    Some(Data { config, census: tables.census_map(vec![grid]), roads, catalog })
}

/// The middle of a DownTown road: the observer stands on the first two-lane segment's midpoint.
fn observer(net: &RoadNetwork, district: u32) -> Observer {
    let (i, s) = net.segments.iter().enumerate().find(|(_, s)| s.district == district && s.lanes == 2 && s.length > 60.0).expect("a DownTown road");
    Observer { position: net.lane_frame(i, 0.0, s.length * 0.5).position, velocity: [0.0; 3] }
}

fn run(d: &Data, seed: u64, ticks: u32, check: bool) -> Vec<Decision> {
    let downtown = d.roads.segments.iter().map(|s| s.district).min().unwrap_or(0);
    let obs = [observer(&d.roads, downtown)];
    let mut inputs = TickInputs::offline(&obs);
    inputs.census = Some(&d.census);
    inputs.roads = Some(&d.roads);
    inputs.vehicles = Some(&d.catalog);
    let mut world = LivingWorld::new(d.config.clone(), seed);
    let rules = PlacementRules::default();
    let mut out = Vec::new();
    let mut max = 0;
    for _ in 0..ticks {
        out.extend(world.step(&inputs));
        if !check {
            continue;
        }
        let n = world.count(Kind::Vehicle);
        max = max.max(n);
        assert!(n <= 15, "vehicle limit 15 (cap 30)");
        let mut by_lane: BTreeMap<(u64, u8), Vec<(f32, f32)>> = BTreeMap::new();
        for l in world.live(Kind::Vehicle) {
            let SpawnChoice::Vehicle { segment, lane, distance, entity, .. } = &l.choice else { panic!("vehicle record") };
            let si = d.roads.segment_index(SegmentId(*segment)).expect("a real segment");
            let s = &d.roads.segments[si];
            assert!(*lane < s.lanes, "lane {lane} of {}", s.lanes);
            assert!(*distance >= rules.end_margin && *distance <= s.length - rules.end_margin);
            let f = d.roads.lane_frame(si, *lane as f32, *distance);
            let gap = ((f.position[0] - l.position[0]).powi(2) + (f.position[2] - l.position[2]).powi(2)).sqrt();
            assert!(gap < 1e-3, "on the lane point");
            by_lane.entry((*segment, *lane)).or_default().push((*distance, d.catalog.entities[entity].length));
        }
        for v in by_lane.values_mut() {
            v.sort_by(|a, b| a.0.total_cmp(&b.0));
            for w in v.windows(2) {
                assert!(w[1].0 - w[0].0 >= 0.5 * rules.end_margin + w[0].1 - 1e-3, "lane gap {w:?}");
            }
        }
    }
    if check {
        eprintln!("DownTown: {max} cars at most over {ticks} ticks");
        assert!(max > 0, "DownTown roads get traffic");
    }
    for dcs in &out {
        if let Decision::Spawn(s) = dcs {
            if let SpawnChoice::Vehicle { segment, lane, distance, .. } = &s.choice {
                let si = d.roads.segment_index(SegmentId(*segment)).unwrap();
                let f = d.roads.lane_frame(si, *lane as f32, *distance);
                assert!((s.heading - f.yaw()).abs() < 1e-5, "facing the lane direction");
            }
        }
    }
    out
}

#[test]
fn downtown_vehicle_census_on_the_shipped_roads() {
    let Some(d) = data() else {
        eprintln!("skipped: set SKATE3_ASSET_ROOT to an export with living_world/roads.bin and vehicles.json");
        return;
    };
    assert!(d.catalog.categories.contains_key("sedans") && d.catalog.entities.values().all(|e| e.length > 3.0 && e.chassis_colours >= 1));
    run(&d, 7, 60 * 120, true);
}

#[test]
fn downtown_same_inputs_same_stream() {
    let Some(d) = data() else {
        eprintln!("skipped: set SKATE3_ASSET_ROOT to an export with living_world/roads.bin and vehicles.json");
        return;
    };
    assert_eq!(run(&d, 99, 60 * 30, false), run(&d, 99, 60 * 30, false));
}
