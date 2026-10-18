//! Data-gated checks of the traffic core (`skate_core::living_world::traffic`) on the user's own
//! export. Skips (passes with a note) when no data is configured.
//! - `SKATE3_ASSET_ROOT`: converted assets (one root, or several joined like PATH); reads
//!   `private/living_world/roads.bin` and `tables.json`.
//! - `SKATE3_TRAFFIC_TRACE`: recomp trace files (`.tsv`, joined like PATH) with TRAFLIGHT2 lines
//!   (recomp-research hooks, category `traffic`); their signal timeline must be reproduced.
//!
//! Counts are the shipped Skate 3 road networks (milestone V0); they are asserted, not embedded.

use skate_core::living_world::rng::Rng;
use skate_core::living_world::traffic::{choose_connector, ConnectorChoice, LaneCursor, Place, RoadNetwork, SignalClock, SignalTimings};
use skate_data::roads::{signal_timings, RoadGraph};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

fn find(name: &str) -> Option<PathBuf> {
    let raw = std::env::var_os("SKATE3_ASSET_ROOT")?;
    std::env::split_paths(&raw).flat_map(|root| [format!("private/living_world/{name}"), format!("living_world/{name}")].map(|p| root.join(p))).find(|p| p.is_file())
}

fn network() -> Option<RoadNetwork> {
    let g = RoadGraph::parse(&std::fs::read(find("roads.bin")?).unwrap()).unwrap();
    Some(RoadNetwork::build(&g.traffic_input()).unwrap())
}

fn timings() -> Option<SignalTimings> {
    let v: serde_json::Value = serde_json::from_slice(&std::fs::read(find("tables.json")?).unwrap()).unwrap();
    Some(signal_timings(&v).expect("trafficlights record"))
}

fn dist(a: [f32; 3], b: [f32; 3]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

#[test]
fn network_builds_with_the_shipped_counts() {
    let Some(net) = network() else {
        eprintln!("skipped: set SKATE3_ASSET_ROOT to an export with living_world/roads.bin");
        return;
    };
    assert_eq!((net.segments.len(), net.junctions.len(), net.connectors.len()), (76, 33, 138));
    assert_eq!(net.junctions.iter().filter(|j| j.signalled).count(), 24, "flag_04 set on 14 DownTown + 10 Industrial junctions");
    // Every connector names a real approach and exit segment.
    for c in 0..net.connectors.len() {
        assert!(net.connector_approach(c).is_some() && net.connector_exit(c).is_some(), "{:?}", net.connectors[c].id);
    }
}

#[test]
fn connector_endpoints_sit_on_their_lanes() {
    let Some(net) = network() else {
        eprintln!("skipped: set SKATE3_ASSET_ROOT to an export with living_world/roads.bin");
        return;
    };
    let mut worst: f32 = 0.0;
    for c in 0..net.connectors.len() {
        let k = &net.connectors[c];
        let a = net.connector_approach(c).unwrap();
        let x = net.connector_exit(c).unwrap();
        let lane_end = net.lane_frame(a, k.from_lane as f32, net.segments[a].length).position;
        let lane_start = net.lane_frame(x, k.to_lane as f32, 0.0).position;
        let (d0, d1) = (dist(lane_end, k.curve.start), dist(lane_start, k.curve.end));
        worst = worst.max(d0).max(d1);
        assert!(d0 < 0.15 && d1 < 0.15, "{:?}: start off by {d0}, end off by {d1}", k.id);
    }
    eprintln!("worst connector endpoint offset {worst:.4} m");
}

#[test]
fn every_lane_is_reachable_and_has_a_way_on() {
    let Some(net) = network() else {
        eprintln!("skipped: set SKATE3_ASSET_ROOT to an export with living_world/roads.bin");
        return;
    };
    let lanes: Vec<(usize, u8)> = (0..net.segments.len()).flat_map(|s| (0..net.segments[s].lanes).map(move |l| (s, l))).collect();
    let mut into: BTreeMap<(usize, u8), usize> = BTreeMap::new();
    let mut edges: BTreeMap<(usize, u8), BTreeSet<(usize, u8)>> = BTreeMap::new();
    for c in 0..net.connectors.len() {
        let k = &net.connectors[c];
        let from = (net.connector_approach(c).unwrap(), k.from_lane);
        let to = (net.connector_exit(c).unwrap(), k.to_lane);
        *into.entry(to).or_default() += 1;
        edges.entry(from).or_default().insert(to);
    }
    let mut lane_change_only = Vec::new();
    for &(s, l) in &lanes {
        let seg = &net.segments[s];
        if seg.to_junction.is_some() {
            assert!(!net.next_connectors(s, l).is_empty(), "{:016X} lane {l}: no connector at its junction", seg.id.0);
        }
        if seg.from_junction.is_some() && into.get(&(s, l)).copied().unwrap_or(0) == 0 {
            // Five shipped lanes (DownTown, one lane of a two-lane exit) get no connector; cars
            // reach them by a lane change from the neighbour lane, which has one [data].
            let (left, right) = net.adjacent_lanes(s, l);
            assert!(
                [left, right].into_iter().flatten().any(|n| into.get(&(s, n)).copied().unwrap_or(0) > 0),
                "{:016X} lane {l}: no connector into it or its neighbours",
                seg.id.0
            );
            lane_change_only.push(format!("{:016X}/{l}", seg.id.0));
        }
    }
    eprintln!("lanes entered only by a lane change: {lane_change_only:?}");
    assert_eq!(lane_change_only.len(), 5);
    // Lane changes are edges too (both ways between neighbours).
    for &(s, l) in &lanes {
        let (left, right) = net.adjacent_lanes(s, l);
        for n in [left, right].into_iter().flatten() {
            edges.entry((s, l)).or_default().insert((s, n));
        }
    }
    // Lanes reachable from each other: report the strongly connected groups per district.
    let reach = |start: (usize, u8)| {
        let mut seen = BTreeSet::from([start]);
        let mut stack = vec![start];
        while let Some(n) = stack.pop() {
            for &m in edges.get(&n).into_iter().flatten() {
                if seen.insert(m) {
                    stack.push(m);
                }
            }
        }
        seen
    };
    let mut unreachable = Vec::new();
    for district in 0..3u32 {
        let own: Vec<(usize, u8)> = lanes.iter().copied().filter(|&(s, _)| net.segments[s].district == district).collect();
        let reached: BTreeSet<(usize, u8)> = own.iter().flat_map(|&l| reach(l).into_iter().filter(move |&m| m != l)).collect();
        for l in &own {
            if !reached.contains(l) {
                unreachable.push(format!("{:016X}/{}", net.segments[l.0].id.0, l.1));
            }
        }
    }
    assert!(unreachable.is_empty(), "lanes no other lane leads to: {unreachable:?}");
}

#[test]
fn cursors_drive_the_whole_network_without_jumps() {
    let Some(net) = network() else {
        eprintln!("skipped: set SKATE3_ASSET_ROOT to an export with living_world/roads.bin");
        return;
    };
    let mut rng = Rng::new(52);
    let mut used = BTreeSet::new();
    for s in 0..net.segments.len() {
        for lane in 0..net.segments[s].lanes {
            let mut choose = |n: &RoadNetwork, seg: usize, l: u8| choose_connector(n, seg, l, ConnectorChoice::Random, &|_, _| 0.0, &mut rng);
            let mut cur = LaneCursor::on_lane(&net, s, lane, 0.0, &mut choose);
            let mut last = cur.frame(&net).position;
            for _ in 0..4000 {
                let step = cur.advance(&net, 0.5, &mut choose);
                for p in &step.entered {
                    if let Place::Connector { connector } = p {
                        used.insert(*connector);
                    }
                }
                let p = cur.frame(&net).position;
                assert!(dist(p, last) < 0.5 + 0.2, "{:016X} lane {lane}: jump {} at {:?}", net.segments[s].id.0, dist(p, last), cur.place);
                last = p;
                if step.at_dead_end {
                    break;
                }
            }
        }
    }
    eprintln!("connectors driven: {} of {}", used.len(), net.connectors.len());
}

#[test]
fn the_city_has_four_controllers_with_the_shipped_timings() {
    let Some(t) = timings() else {
        eprintln!("skipped: set SKATE3_ASSET_ROOT to an export with living_world/tables.json");
        return;
    };
    assert_eq!(t, SignalTimings { green: 7.0, amber: 1.0, all_red: 0.5, walk_split: 0.4 });
    let clock = SignalClock::new(t);
    assert_eq!(clock.controllers.len(), 4);
    if let Some(net) = network() {
        // Every signalled approach maps to one of the 4 (its node end).
        for j in net.junctions.iter().filter(|j| j.signalled) {
            for &c in &j.connectors {
                assert!((net.connectors[c].from_end as usize) < clock.controllers.len());
            }
        }
    }
}

/// TRAFLIGHT2 fields: ms, index, C, car phase, n, kind, length, remaining, walk phase, n2, kind2,
/// length2, C+300, ticks since the last line, ms since the last line.
fn trace_timelines(path: &std::path::Path) -> BTreeMap<u8, Vec<(u64, usize, usize)>> {
    let text = std::fs::read_to_string(path).unwrap();
    let mut out: BTreeMap<u8, Vec<(u64, usize, usize)>> = BTreeMap::new();
    let mut at: BTreeMap<u8, u64> = BTreeMap::new();
    for line in text.lines() {
        let f: Vec<&str> = line.split('\t').collect();
        if f.first() != Some(&"TRAFLIGHT2") || f.len() < 16 {
            continue;
        }
        let index: u8 = f[2].parse().unwrap();
        let ticks: u64 = f[14].parse().unwrap();
        let tick = at.entry(index).or_insert(0);
        *tick += ticks;
        out.entry(index).or_default().push((*tick, f[4].parse().unwrap(), f[9].parse().unwrap()));
    }
    out
}

#[test]
fn recorded_signal_timeline_is_reproduced() {
    let (Some(raw), Some(t)) = (std::env::var_os("SKATE3_TRAFFIC_TRACE"), timings().or(Some(SignalTimings { green: 7.0, amber: 1.0, all_red: 0.5, walk_split: 0.4 }))) else {
        eprintln!("skipped: set SKATE3_TRAFFIC_TRACE to recomp traces with TRAFLIGHT2 lines");
        return;
    };
    for path in std::env::split_paths(&raw).filter(|p| p.is_file()) {
        let recorded = trace_timelines(&path);
        assert_eq!(recorded.len(), 4, "{}: four controllers", path.display());
        let horizon = recorded.values().flat_map(|v| v.last()).map(|e| e.0).max().unwrap();
        let mut clock = SignalClock::new(t);
        let mut ours: BTreeMap<u8, Vec<(u64, usize, usize)>> = BTreeMap::new();
        let mut changes = Vec::new();
        for _ in 0..horizon {
            clock.tick(&mut changes);
        }
        for ch in changes {
            ours.entry(ch.controller).or_default().push((ch.tick, ch.car_phase, ch.walk_phase));
        }
        for (index, rec) in &recorded {
            // The first line is the hook's first call (tick 1, nothing changed yet).
            assert_eq!(rec[0], (1, 0, 0), "{} controller {index}", path.display());
            let mine = &ours[index];
            let n = rec.len() - 1;
            assert_eq!(&mine[..n], &rec[1..], "{} controller {index}: timeline", path.display());
        }
        eprintln!("{}: {} phase changes reproduced", path.display(), recorded.values().map(|v| v.len() - 1).sum::<usize>());
    }
}
