//! Data-gated run of the V3 lane follower (`skate_core::living_world::traffic::follow`) on the
//! user's own export: DownTown filled with cars by the retail placement rules, two minutes of
//! traffic at the 60 Hz world tick. Cars follow their lanes, enter signalled junctions only on
//! green (or turning right; retail has no amber go-through, b163), never overlap and never pass
//! the lane limit. Skips (passes with a note) when `SKATE3_ASSET_ROOT` is not set.

use skate_core::living_world::rng::Rng;
use skate_core::living_world::traffic::follow::{self, Car, FollowEvent, FollowParams};
use skate_core::living_world::traffic::spawn::{lane_fits, LaneCar, PlacementRules};
use skate_core::living_world::traffic::{choose_connector, ConnectorChoice, Entry, LaneCursor, Light, Place, RoadNetwork, SignalClock, Turn};
use skate_data::roads::{signal_timings, RoadGraph};
use std::path::PathBuf;

fn find(name: &str) -> Option<PathBuf> {
    let raw = std::env::var_os("SKATE3_ASSET_ROOT")?;
    std::env::split_paths(&raw).flat_map(|root| [format!("private/living_world/{name}"), format!("living_world/{name}")].map(|p| root.join(p))).find(|p| p.is_file())
}

fn dist(a: [f32; 3], b: [f32; 3]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

#[test]
fn downtown_traffic_follows_lanes_obeys_lights_and_never_overlaps() {
    let (Some(roads), Some(tables)) = (find("roads.bin"), find("tables.json")) else {
        eprintln!("skipped: set SKATE3_ASSET_ROOT to an export with living_world/roads.bin and tables.json");
        return;
    };
    let g = RoadGraph::parse(&std::fs::read(roads).unwrap()).unwrap();
    let net = RoadNetwork::build(&g.traffic_input()).unwrap();
    let t: serde_json::Value = serde_json::from_slice(&std::fs::read(tables).unwrap()).unwrap();
    let mut clock = SignalClock::new(signal_timings(&t).unwrap());
    let downtown = g.districts.iter().position(|d| d.name == "DownTown").expect("DownTown district") as u32;
    // Fill DownTown like the census factory would (15 m margins, retail lane test), 30 cars max
    // (the dwntwn cap), sedan length 4.0 m (size_hint z).
    let rules = PlacementRules::default();
    let mut rng = Rng::new(0x5633);
    let mut cars: Vec<Car> = Vec::new();
    'fill: for (si, s) in net.segments.iter().enumerate() {
        if s.district != downtown {
            continue;
        }
        for lane in 0..s.lanes {
            let mut d = rules.end_margin;
            while d < s.length - rules.end_margin {
                let on: Vec<LaneCar> = cars
                    .iter()
                    .filter(|c| c.cursor.place == (Place::Lane { segment: si, lane }))
                    .map(|c| LaneCar { distance: c.cursor.distance, length: c.length, speed: 0.0 })
                    .collect();
                if lane_fits(&net, si, lane, d, &rules, &on) {
                    let key = cars.len() as u32 + 1;
                    let cursor = LaneCursor::on_lane(&net, si, lane, d, &mut |n, s, l| choose_connector(n, s, l, ConnectorChoice::LeastLoaded, &|_, _| 0.0, &mut rng));
                    cars.push(Car::new(key, cursor, 4.0, FollowParams::default()));
                    if cars.len() == 30 {
                        break 'fill;
                    }
                }
                d += 10.0;
            }
        }
    }
    assert!(cars.len() >= 20, "placed {} cars", cars.len());
    let mut step_rng = Rng::new(0x5634);
    let (mut entered, mut lane_changes, mut on_red, mut dead) = (0, 0, 0, 0);
    let mut moved = vec![0.0f32; cars.len() + 1];
    let (mut speeds, mut hard) = (Vec::new(), 0u32);
    for tick in 0..60 * 120 {
        clock.tick(&mut Vec::new());
        let before: Vec<(u32, Option<Entry>)> = cars.iter().map(|c| (c.key, c.entry)).collect();
        let prev: Vec<[f32; 3]> = cars.iter().map(|c| c.cursor.frame(&net).position).collect();
        let events = follow::step(&net, &clock, &mut cars, 1.0 / 60.0, ConnectorChoice::LeastLoaded, &mut step_rng);
        for e in &events {
            match *e {
                FollowEvent::EnteredJunction { key, connector } => {
                    entered += 1;
                    let (_, entry) = before.iter().find(|b| b.0 == key).copied().unwrap();
                    let c = &net.connectors[connector];
                    if let Some(phase) = clock.light_for(&net, connector) {
                        if phase.light == Light::Red && c.turn() != Turn::Right {
                            on_red += 1;
                            eprintln!("tick {tick}: car {key} entered {:?} on red (entry {entry:?})", c.id);
                        }
                    }
                }
                FollowEvent::EnteredLane { .. } => lane_changes += 1,
                FollowEvent::DeadEnd { key } => {
                    dead += 1;
                    cars.retain(|c| c.key != key);
                }
                _ => {}
            }
        }
        for (i, c) in cars.iter().enumerate() {
            let p = c.cursor.frame(&net).position;
            assert!(p.iter().all(|x| x.is_finite()) && c.speed.is_finite(), "car {} NaN", c.key);
            assert!(c.speed <= follow::cap(&net, c.cursor.place) + 0.05, "car {} at {} m/s", c.key, c.speed);
            if c.speed > 0.5 {
                speeds.push(c.speed);
            }
            if c.accel < -7.3 {
                hard += 1;
            }
            if let Some(j) = prev.get(i) {
                moved[c.key as usize] += dist(*j, p);
            }
            for o in &cars[i + 1..] {
                let q = o.cursor.frame(&net).position;
                if dist(p, q) <= 1.5 {
                    let show = |pl: Place| match pl {
                        Place::Connector { connector } => {
                            let k = &net.connectors[connector];
                            format!("con {:?} {}:{} -> {}:{} len {:.1} signalled {}", k.id, k.from_end, k.from_lane, k.to_end, k.to_lane, k.length(), net.junctions[k.junction].signalled)
                        }
                        Place::Lane { segment, lane } => format!("lane {segment}/{lane}"),
                    };
                    panic!("tick {tick}: cars {} ({} at {:.1}, {:.1} m/s) and {} ({} at {:.1}, {:.1} m/s) overlap", c.key, show(c.cursor.place), c.cursor.distance, c.speed, o.key, show(o.cursor.place), o.cursor.distance, o.speed);
                }
            }
        }
    }
    speeds.sort_by(|a, b| a.total_cmp(b));
    let pct = |q: f32| speeds.get(((speeds.len() as f32 - 1.0) * q) as usize).copied().unwrap_or(0.0);
    eprintln!("moving speed median {:.1} m/s, p90 {:.1}; car-ticks braking harder than 7.3 m/s^2: {hard}", pct(0.5), pct(0.9));
    eprintln!("cars left {}, junction entries {entered}, lane entries {lane_changes}, dead ends {dead}, entries on red {on_red}", cars.len());
    assert!(entered >= 20, "traffic moves through junctions ({entered})");
    assert!(lane_changes >= 20);
    assert_eq!(on_red, 0, "no car entered on red without having been given Go");
    let moving = moved.iter().filter(|&&m| m > 50.0).count();
    assert!(moving >= 15, "most cars drove more than 50 m ({moving})");
}
