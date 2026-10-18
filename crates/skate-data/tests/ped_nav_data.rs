//! Data-gated checks of the ped navmesh (peds milestone M3) on the user's own export. Skips (passes
//! with a note) without data. `SKATE3_ASSET_ROOT`: one or more asset roots joined like PATH; reads
//! `private/living_world/navmesh.bin` (+ `roads.bin` and `tables.json` for the road and
//! crosswalk checks).
//!
//! Expected values are the shipped DownTown NavPower graphs as decoded for M3 (asserted here, not
//! embedded in the engine): 36443 polygons (24991 area 0x11, 8997 area 0xA1, 2455 area 0xF1).

use skate_core::living_world::peds::anim::{Intent, Locomotion};
use skate_core::living_world::peds::crosswalk::{RoadWalkSignals, signalled_arms};
use skate_core::living_world::peds::nav::{AREA_DEFAULT, AREA_OTHER, AREA_ROAD};
use skate_core::living_world::peds::wander::{NoSignals, constrain_move, constrain_step, crosswalk_ok, forward, separation_ok};
use skate_core::living_world::peds::{CrosswalkRule, NavMesh, NavRules, Neighbour, PedNav, WalkSignals, WanderParams};
use skate_core::living_world::traffic::{RoadNetwork, SignalClock};
use skate_core::living_world::traffic::signals::Light;
use std::path::PathBuf;

fn find(rel: &str) -> Option<PathBuf> {
    let raw = std::env::var_os("SKATE3_ASSET_ROOT")?;
    std::env::split_paths(&raw).map(|r| r.join(rel)).find(|p| p.is_file())
}

fn downtown() -> Option<NavMesh> {
    let bytes = std::fs::read(find(skate_data::ped_nav::NAVMESH)?).ok()?;
    let input = skate_data::ped_nav::district(&bytes, "DownTown").unwrap()?;
    Some(NavMesh::build(&input, NavRules::default()))
}

#[test]
fn downtown_navmesh_decodes_with_the_shipped_areas() {
    let Some(m) = downtown() else {
        eprintln!("skipped: set SKATE3_ASSET_ROOT to an export with living_world/navmesh.bin");
        return;
    };
    let count = |a: u8| m.polys.iter().filter(|p| p.area == a).count();
    assert_eq!((m.polys.len(), count(AREA_DEFAULT), count(AREA_ROAD), count(AREA_OTHER)), (36443, 24991, 8997, 2455));
    assert_eq!(m.agent, [0.12, 0.35, 0.2, 1.6]);
    for p in &m.polys {
        assert!(p.verts.len() >= 3);
        assert!(p.verts.iter().all(|v| v.iter().all(|c| c.is_finite())));
    }
    // Road piece starts lie on road polygons (the area byte's meaning).
    let Some(roads) = find("private/living_world/roads.bin") else { return };
    let graph = skate_data::roads::RoadGraph::parse(&std::fs::read(roads).unwrap()).unwrap();
    let net = RoadNetwork::build(&graph.traffic_input()).unwrap();
    let (mut on_mesh, mut on_road) = (0, 0);
    for s in &net.segments {
        for piece in &s.pieces {
            if let Some(p) = m.locate(piece.centre.start) {
                on_mesh += 1;
                on_road += (m.polys[p.poly as usize].area == AREA_ROAD) as u32;
            }
        }
    }
    eprintln!("road piece starts on the DownTown mesh: {on_mesh}, on road polygons: {on_road}");
    assert!(on_mesh > 1000 && on_road as f32 > 0.95 * on_mesh as f32);
}

struct Body {
    id: u64,
    pos: [f32; 3],
    heading: f32,
    nav: PedNav,
    state: Locomotion,
}

/// Peds on pavement polygons within 60 m of Aletown's recomp ped positions (about -150, 12, 480).
fn spawn(m: &NavMesh, n: usize) -> Vec<Body> {
    let centre = [-150.0f32, 12.0, 470.0];
    let pick: Vec<usize> = (0..m.polys.len())
        .filter(|&k| {
            let p = &m.polys[k];
            p.area == AREA_DEFAULT && ((p.centre[0] - centre[0]).powi(2) + (p.centre[2] - centre[2]).powi(2)).sqrt() < 60.0 && (p.centre[1] - centre[1]).abs() < 10.0
        })
        .collect();
    assert!(pick.len() >= n, "pavement near Aletown: {}", pick.len());
    (0..n).map(|i| {
        let k = pick[i * pick.len() / n];
        Body { id: i as u64 + 1, pos: m.polys[k].centre, heading: i as f32 * 0.7, nav: PedNav::default(), state: Locomotion::Idle }
    }).collect()
}

/// Kinematic walk (the walk clip's 1.325 m/s), the mesh and separation rules as in the game.
fn run(m: &NavMesh, bodies: &mut [Body], ticks: u32, rule: CrosswalkRule, signals: &dyn WalkSignals, mut each: impl FnMut(&Body, [f32; 3])) {
    let p = WanderParams::default();
    let dt = 1.0 / 60.0;
    for _ in 0..ticks {
        for k in 0..bodies.len() {
            let others: Vec<Neighbour> = bodies.iter().map(|b| Neighbour { order: b.id, position: b.pos }).collect();
            let b = &mut bodies[k];
            let before = b.pos;
            let out = b.nav.step(m, &p, rule, signals, b.id, b.pos, b.heading, b.state, &others, dt);
            b.heading += out.turn;
            match out.intent {
                Intent::Walk | Intent::Run => {
                    b.state = Locomotion::Walk;
                    let f = forward(b.heading);
                    let (next, ok) = constrain_step(m, b.pos, [b.pos[0] + f[0] * 1.325 * dt, b.pos[1], b.pos[2] + f[1] * 1.325 * dt]);
                    if ok && separation_ok(b.pos, next, b.id, &others, m.agent[1]) && crosswalk_ok(m, rule, signals, b.pos, next) {
                        b.pos = next;
                    }
                }
                Intent::TurnLeft | Intent::TurnRight => {
                    b.heading += (out.turn_degrees as f32).to_radians() * if out.intent == Intent::TurnLeft { 1.0 } else { -1.0 };
                    b.state = Locomotion::Idle;
                }
                Intent::Idle => b.state = Locomotion::Idle,
            }
            each(b, before);
        }
    }
}

#[test]
fn downtown_peds_wander_on_walkable_ground() {
    let Some(m) = downtown() else {
        eprintln!("skipped: needs living_world/navmesh.bin under SKATE3_ASSET_ROOT");
        return;
    };
    let mut a = spawn(&m, 15);
    let (mut samples, mut road, mut off) = (0u32, 0u32, 0u32);
    run(&m, &mut a, 60 * 90, CrosswalkRule::Off, &NoSignals, |b, _| {
        samples += 1;
        match m.locate(b.pos) {
            Some(p) => {
                assert_ne!(m.polys[p.poly as usize].area, AREA_OTHER, "ped {} on a 0xF1 polygon", b.id);
                road += (m.polys[p.poly as usize].area == AREA_ROAD) as u32;
            }
            None => off += 1,
        }
    });
    let moved: Vec<f32> = a.iter().zip(spawn(&m, 15)).map(|(x, y)| ((x.pos[0] - y.pos[0]).powi(2) + (x.pos[2] - y.pos[2]).powi(2)).sqrt()).collect();
    let targets: u32 = a.iter().map(|b| b.nav.targets_chosen).sum();
    eprintln!("90 s x 15 peds: on road polygons {:.1} % (recomp 3-4 %), off mesh {off}, targets {targets}, moved {moved:?}", 100.0 * road as f32 / samples as f32);
    assert_eq!(off, 0, "every sample on walkable ground");
    assert!(moved.iter().filter(|d| **d > 5.0).count() >= 10, "most peds walked away from their spawn");
    // Deterministic: a second run lands every ped on the same spot.
    let mut b = spawn(&m, 15);
    run(&m, &mut b, 60 * 90, CrosswalkRule::Off, &NoSignals, |_, _| {});
    assert!(a.iter().zip(&b).all(|(x, y)| x.pos == y.pos && x.heading == y.heading && x.nav == y.nav));
}

#[test]
fn downtown_crosswalk_rule_crosses_only_on_walk() {
    let (Some(m), Some(roads), Some(tables)) = (downtown(), find("private/living_world/roads.bin"), find("private/living_world/tables.json")) else {
        eprintln!("skipped: needs navmesh.bin, roads.bin and tables.json under SKATE3_ASSET_ROOT");
        return;
    };
    let graph = skate_data::roads::RoadGraph::parse(&std::fs::read(roads).unwrap()).unwrap();
    let net = RoadNetwork::build(&graph.traffic_input()).unwrap();
    let tables: serde_json::Value = serde_json::from_slice(&std::fs::read(tables).unwrap()).unwrap();
    let timings = skate_data::roads::signal_timings(&tables).expect("trafficlights record");
    let arms = signalled_arms(&net);
    let mut clock = SignalClock::new(timings);
    // Peds on pavement next to signalled junction arms of this district.
    let near_arm = |c: [f32; 3]| arms.iter().any(|a| ((a.2[0] - c[0]).powi(2) + (a.2[2] - c[2]).powi(2)).sqrt() < 14.0 && (a.2[1] - c[1]).abs() < 3.0);
    let pick: Vec<usize> = (0..m.polys.len()).filter(|&k| m.polys[k].area == AREA_DEFAULT && near_arm(m.polys[k].centre)).collect();
    assert!(pick.len() >= 30, "pavement next to signalled arms: {}", pick.len());
    let mut bodies: Vec<Body> = (0..30)
        .map(|i| {
            let k = pick[i * pick.len() / 30];
            Body { id: i as u64 + 1, pos: m.polys[k].centre, heading: i as f32 * 1.3, nav: PedNav::default(), state: Locomotion::Idle }
        })
        .collect();
    let (mut entries, mut signalled, mut waits) = (0u32, 0u32, 0u32);
    for _ in 0..(60 * 120) {
        let signals = RoadWalkSignals { arms: &arms, clock: &clock, radius: 20.0 };
        run(&m, &mut bodies, 1, CrosswalkRule::WalkSignal, &signals, |b, before| {
            let area = |p: [f32; 3]| m.locate(p).map(|x| m.polys[x.poly as usize].area);
            if b.nav.waiting == skate_core::living_world::peds::NavWait::Crosswalk {
                waits += 1;
            }
            if area(before) != Some(AREA_ROAD) && area(b.pos) == Some(AREA_ROAD) {
                entries += 1;
                let light = signals.walk_light(b.pos);
                signalled += light.is_some() as u32;
                assert!(light.is_none() || light == Some(Light::Green), "ped {} stepped onto the road at {:?} on {light:?}", b.id, b.pos);
            }
        });
        clock.advance(1.0 / 60.0);
    }
    eprintln!("120 s x 30 peds with the crosswalk rule: {entries} road entries ({signalled} at signalled arms), {waits} wait ticks");
    assert!(signalled > 0 && waits > 0, "the rule was exercised");
}

/// fix 17 (user: peds "get lined up and start walking in place in specific spots", one standing
/// on a wall top): every tile seam of the shipped DownTown mesh (NavPower tiles stop short of
/// their shared border; setup links the two sides) is walked across at the walk clip's speed.
/// The old step rule (locate the step's end on its own) snaps a ped in the gap back to the edge
/// it left, so it walks in place there; the surface move crosses and never changes component
/// (no step onto an unconnected wall-top layer).
#[test]
fn downtown_tile_seams_are_crossed_without_layer_jumps() {
    let Some(m) = downtown() else {
        eprintln!("skipped: needs living_world/navmesh.bin under SKATE3_ASSET_ROOT");
        return;
    };
    let step = 1.325 / 30.0; // one console tick of the walk clip
    let (mut seams, mut crossed, mut old_crossed, mut jumps, mut old_jumps) = (0u32, 0u32, 0u32, 0u32, 0u32);
    for (k, p) in m.polys.iter().enumerate() {
        let n = p.verts.len();
        for e in 0..n {
            let Some(q) = p.neighbours[e] else { continue };
            if !m.rules.walkable(p.area) || !m.rules.walkable(m.polys[q as usize].area) {
                continue;
            }
            let (a, b) = (p.verts[e], p.verts[(e + 1) % n]);
            let mid = [(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5, (a[2] + b[2]) * 0.5];
            let len = ((b[0] - a[0]).powi(2) + (b[2] - a[2]).powi(2)).sqrt();
            if len < 0.5 {
                continue;
            }
            // Outward normal (away from the polygon's centre).
            let mut nrm = [(b[2] - a[2]) / len, -(b[0] - a[0]) / len];
            if (mid[0] - p.centre[0]) * nrm[0] + (mid[2] - p.centre[2]) * nrm[1] < 0.0 {
                nrm = [-nrm[0], -nrm[1]];
            }
            // A seam: just past the edge is not inside the neighbour (a gap between tiles).
            let probe = [mid[0] + nrm[0] * 0.05, mid[1], mid[2] + nrm[1] * 0.05];
            let inside_q = m.point_on(q, probe).is_some_and(|x| x.position[0] == probe[0] && x.position[2] == probe[2]);
            if inside_q {
                continue;
            }
            let start = [mid[0] - nrm[0] * 0.2, mid[1], mid[2] - nrm[1] * 0.2];
            let Some(s) = m.point_on(k as u32, start).filter(|x| x.position[0] == start[0] && x.position[2] == start[2]) else { continue };
            seams += 1;
            let along = |x: [f32; 3]| (x[0] - mid[0]) * nrm[0] + (x[2] - mid[2]) * nrm[1];
            let (mut pos, mut old) = (s.position, s.position);
            let (mut jumped, mut old_jumped) = (false, false);
            let mut poly = Some(s.poly);
            for _ in 0..20 {
                let to = [pos[0] + nrm[0] * step, pos[1], pos[2] + nrm[1] * step];
                // As the game steps: from the tracked polygon (`PedNav::poly`).
                let (next, at, ok) = constrain_move(&m, poly, pos, to);
                poly = at;
                assert!(ok);
                jumped |= poly.is_none_or(|x| !m.reachable(x, s.poly)) || (next[1] - pos[1]).abs() > 1.0;
                pos = next;
                if let Some(x) = m.locate([old[0] + nrm[0] * step, old[1], old[2] + nrm[1] * step]) {
                    old_jumped |= !m.reachable(x.poly, s.poly) || (x.position[1] - old[1]).abs() > 1.0;
                    old = x.position;
                }
            }
            crossed += (along(pos) > 0.45) as u32;
            old_crossed += (along(old) > 0.45) as u32;
            jumps += jumped as u32;
            old_jumps += old_jumped as u32;
        }
    }
    eprintln!("DownTown seams {seams}: crossed {crossed} (old rule {old_crossed}), layer jumps {jumps} (old rule {old_jumps})");
    assert!(seams > 1000, "tile seams found: {seams}");
    assert!(old_crossed * 2 < seams, "control: the old rule stalls at most seams");
    assert_eq!(jumps, 0, "no step leaves the connected surface");
    // The rest are seams into slivers a few centimetres wide, walked straight on into their
    // boundary (a body slides there).
    assert!(crossed as f32 >= 0.93 * seams as f32, "seams crossed: {crossed} of {seams}");
}

/// fix 17 end to end at the user's spot (the DownTown ramp with the long blue rail, player near
/// [-254, 41, 100], on a tile border): 30 peds wander 120 s with the game's step rules (surface
/// moves, separation, re-plan after 3 s of refused or no-progress steps). No ped may walk in
/// place (walking 4 s without getting 0.3 m anywhere) or leave its connected surface.
#[test]
fn downtown_peds_at_the_ramp_never_walk_in_place() {
    let Some(m) = downtown() else {
        eprintln!("skipped: needs living_world/navmesh.bin under SKATE3_ASSET_ROOT");
        return;
    };
    let centre = [-254.0f32, 40.7, 100.0];
    let home = m.locate(centre).expect("the ramp plaza is on the mesh").poly;
    let pick: Vec<usize> = (0..m.polys.len())
        .filter(|&k| {
            let p = &m.polys[k];
            p.area == AREA_DEFAULT
                && ((p.centre[0] - centre[0]).powi(2) + (p.centre[2] - centre[2]).powi(2)).sqrt() < 40.0
                && (p.centre[1] - centre[1]).abs() < 12.0
                && m.reachable(k as u32, home)
        })
        .collect();
    assert!(pick.len() >= 30, "pavement round the ramp: {}", pick.len());
    let params = WanderParams::default();
    let dt = 1.0 / 30.0;
    struct Walker {
        b: Body,
        blocked: f32,
        anchor: [f32; 3],
        walking: f32,
    }
    let mut w: Vec<Walker> = (0..30)
        .map(|i| {
            let k = pick[i * pick.len() / 30];
            let pos = m.polys[k].centre;
            Walker { b: Body { id: i as u64 + 1, pos, heading: i as f32 * 0.9, nav: PedNav::default(), state: Locomotion::Idle }, blocked: 0.0, anchor: pos, walking: 0.0 }
        })
        .collect();
    let (mut in_place, mut worst) = (0u32, 0.0f32);
    for _ in 0..(30 * 120) {
        for k in 0..w.len() {
            let others: Vec<Neighbour> = w.iter().map(|x| Neighbour { order: x.b.id, position: x.b.pos }).collect();
            let x = &mut w[k];
            let out = x.b.nav.step(&m, &params, CrosswalkRule::Off, &NoSignals, x.b.id, x.b.pos, x.b.heading, x.b.state, &others, dt);
            x.b.heading += out.turn;
            match out.intent {
                Intent::Walk | Intent::Run => {
                    x.b.state = Locomotion::Walk;
                    let f = forward(x.b.heading);
                    let to = [x.b.pos[0] + f[0] * 1.325 * dt, x.b.pos[1], x.b.pos[2] + f[1] * 1.325 * dt];
                    let (next, poly, ok) = constrain_move(&m, x.b.nav.poly, x.b.pos, to);
                    let progressed = ((next[0] - x.b.pos[0]).powi(2) + (next[2] - x.b.pos[2]).powi(2)).sqrt() >= 0.25 * 1.325 * dt;
                    if ok && progressed && separation_ok(x.b.pos, next, x.b.id, &others, m.agent[1]) {
                        x.b.pos = next;
                        x.b.nav.poly = poly;
                        x.blocked = 0.0;
                    } else {
                        x.blocked += dt;
                        if x.blocked > params.yield_patience {
                            x.blocked = 0.0;
                            x.b.nav.skip_long = true;
                            x.b.nav.corners.clear();
                        }
                    }
                    x.walking += dt;
                }
                Intent::TurnLeft | Intent::TurnRight => {
                    x.b.heading += (out.turn_degrees as f32).to_radians() * if out.intent == Intent::TurnLeft { 1.0 } else { -1.0 };
                    x.b.state = Locomotion::Idle;
                    x.walking = 0.0;
                    x.anchor = x.b.pos;
                }
                Intent::Idle => {
                    x.b.state = Locomotion::Idle;
                    x.walking = 0.0;
                    x.anchor = x.b.pos;
                }
            }
            let from_anchor = ((x.b.pos[0] - x.anchor[0]).powi(2) + (x.b.pos[2] - x.anchor[2]).powi(2)).sqrt();
            if from_anchor > 0.3 {
                x.anchor = x.b.pos;
                x.walking = 0.0;
            } else if x.walking > 4.0 {
                in_place += 1;
                worst = worst.max(x.walking);
                eprintln!("ped {} walking in place at {:?}", x.b.id, x.b.pos);
                x.walking = 0.0;
            }
            let here = x.b.nav.poly.and_then(|k| m.point_on(k, x.b.pos)).or_else(|| m.locate(x.b.pos)).expect("on the mesh");
            assert!(m.reachable(here.poly, home), "ped {} left its connected surface at {:?}", x.b.id, x.b.pos);
        }
    }
    eprintln!("120 s x 30 peds round the ramp: walking-in-place episodes {in_place}");
    assert_eq!(in_place, 0, "walking in place (worst {worst:.1} s)");
}
