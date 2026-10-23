//! Peds milestone M3: navmesh, retail wander target choice, crosswalk rule, avoidance.

use super::anim::{Intent, Locomotion};
use super::crosswalk::RoadWalkSignals;
use super::nav::{AREA_DEFAULT, AREA_OTHER, AREA_ROAD, NavMesh, NavMeshInput, NavPolyInput, NavRules, funnel};
use super::wander::*;
use crate::living_world::traffic::signals::{Light, SignalClock, SignalTimings};

/// A grid of `cell` m squares; `area(i, j)` gives each square's area byte or `None` (a hole).
fn grid(nx: i32, nz: i32, cell: f32, area: impl Fn(i32, i32) -> Option<u8>) -> NavMeshInput {
    let mut index = std::collections::BTreeMap::new();
    let mut polys = Vec::new();
    for j in 0..nz {
        for i in 0..nx {
            if let Some(a) = area(i, j) {
                index.insert((i, j), polys.len() as u32);
                let (x, z) = (i as f32 * cell, j as f32 * cell);
                // Counter-clockwise seen from above (x right, z up the page): (x,z) (x+c,z) (x+c,z+c) (x,z+c).
                polys.push(NavPolyInput { verts: vec![[x, 0.0, z], [x + cell, 0.0, z], [x + cell, 0.0, z + cell], [x, 0.0, z + cell]], neighbours: Vec::new(), area: a });
            }
        }
    }
    for j in 0..nz {
        for i in 0..nx {
            let Some(&k) = index.get(&(i, j)) else { continue };
            let n = [(i, j - 1), (i + 1, j), (i, j + 1), (i - 1, j)];
            polys[k as usize].neighbours = n.iter().map(|c| index.get(c).copied()).collect();
        }
    }
    NavMeshInput { agent: [0.12, 0.35, 0.2, 1.6], polygons: polys }
}

fn mesh(input: &NavMeshInput) -> NavMesh {
    NavMesh::build(input, NavRules::default())
}

#[test]
fn fan_angles_follow_the_retail_probe_order() {
    let deg = |v: Vec<f32>| v.into_iter().map(|a| (a.to_degrees() * 100.0).round() / 100.0).collect::<Vec<_>>();
    let p = WanderParams::default();
    assert_eq!(deg(fan_angles(&p.long)), vec![0.0, -22.5, 22.5, -45.0, 45.0, -67.5]);
    assert_eq!(deg(fan_angles(&p.short)), vec![0.0, -33.75, 33.75, -67.5, 67.5, -101.25, 101.25, -135.0, 135.0, -168.75]);
    assert_eq!((p.long.distance, p.short.distance, p.fallback_min, p.fallback_scale, p.mover_value), (40.0, 10.0, 3.0, 1.1, 2.0));
}

#[test]
fn locate_snaps_reachability_and_blocked_areas() {
    // Two islands: x 0..10 and x 14..20 (hole at 10..14); one blocked square inside the first.
    let input = grid(10, 5, 2.0, |i, j| match (i, j) {
        (5..=6, _) => None,
        (2, 2) => Some(AREA_OTHER),
        _ => Some(AREA_DEFAULT),
    });
    let m = mesh(&input);
    let a = m.locate([1.0, 0.0, 1.0]).unwrap();
    assert_eq!(a.position, [1.0, 0.0, 1.0]);
    assert!(m.locate([5.0, 0.0, 5.0]).is_none(), "blocked area");
    let snapped = m.locate([10.3, 0.0, 3.0]).expect("0.3 m outside snaps (radius 0.5)");
    assert!((snapped.position[0] - 10.0).abs() < 1e-5);
    assert!(m.locate([11.0, 0.0, 3.0]).is_none(), "1 m outside does not");
    let b = m.locate([15.0, 0.0, 1.0]).unwrap();
    assert!(!m.reachable(a.poly, b.poly));
    assert!(m.find_path(a, b).is_none());
    assert!(m.locate([1.0, 9.0, 1.0]).is_none(), "outside the height window");
}

#[test]
fn paths_bend_round_holes_and_stay_on_the_mesh() {
    // U-shape: a wall from z 0 to 14 at x 8..10 (open at the top).
    let input = grid(10, 10, 2.0, |i, j| if i == 4 && j < 7 { None } else { Some(AREA_DEFAULT) });
    let m = mesh(&input);
    let from = m.locate([3.0, 0.0, 1.0]).unwrap();
    let to = m.locate([15.0, 0.0, 1.0]).unwrap();
    let path = m.find_path(from, to).unwrap();
    assert_eq!(*path.last().unwrap(), [15.0, 0.0, 1.0]);
    assert!(path.len() >= 3, "goes round the wall: {path:?}");
    let mut prev = [3.0, 0.0, 1.0];
    for c in &path {
        for s in 0..=40 {
            let t = s as f32 / 40.0;
            let p = [prev[0] + (c[0] - prev[0]) * t, 0.0, prev[2] + (c[2] - prev[2]) * t];
            assert!(m.locate(p).is_some(), "segment leaves the mesh at {p:?} ({path:?})");
        }
        prev = *c;
    }
    // Determinism: the same query gives the same corners.
    assert_eq!(m.find_path(from, to), Some(path));
    // The funnel on a straight corridor is the goal alone.
    let straight = funnel([0.0, 0.0, 0.0], [0.0, 0.0, 10.0], &[([-1.0, 0.0, 5.0], [1.0, 0.0, 5.0])]);
    assert_eq!(straight, vec![[0.0, 0.0, 10.0]]);
}

#[test]
fn target_choice_is_retail_and_deterministic() {
    let p = WanderParams::default();
    // Open 100 x 100 m: straight ahead 40 m.
    let open = mesh(&grid(50, 50, 2.0, |_, _| Some(AREA_DEFAULT)));
    let from = open.locate([50.0, 0.0, 10.0]).unwrap();
    let (t, _) = choose_target(&open, &p, from, forward(0.0), false);
    assert!((t[0] - 50.0).abs() < 1e-3 && (t[2] - 50.0).abs() < 1e-3, "{t:?}");
    // Mesh ends 20 m ahead (a 30 m deep strip): straight, -22.5, +22.5, -45 and +45 degrees
    // land beyond it; the last long probe (-67.5 degrees, the extra odd one) fits.
    let strip = mesh(&grid(50, 15, 2.0, |_, _| Some(AREA_DEFAULT)));
    let from = strip.locate([50.0, 0.0, 10.0]).unwrap();
    let (t, located) = choose_target(&strip, &p, from, forward(0.0), false);
    assert!(located.is_some());
    let a = (-67.5f32).to_radians();
    assert!((t[0] - (50.0 + 40.0 * a.sin())).abs() < 1e-3 && (t[2] - (10.0 + 40.0 * a.cos())).abs() < 1e-3, "{t:?}");
    // A 16 m deep strip: no 40 m probe fits; the short fan's straight probe (10 m) does.
    let shallow = mesh(&grid(50, 8, 2.0, |_, _| Some(AREA_DEFAULT)));
    let from = shallow.locate([50.0, 0.0, 4.0]).unwrap();
    let (t, _) = choose_target(&shallow, &p, from, forward(0.0), false);
    assert!((t[0] - 50.0).abs() < 1e-3 && (t[2] - 14.0).abs() < 1e-3, "short fan straight: {t:?}");
    // Facing +x along the strip: the long fan's straight probe fits.
    let from = strip.locate([50.0, 0.0, 10.0]).unwrap();
    let (t, _) = choose_target(&strip, &p, from, forward(std::f32::consts::FRAC_PI_2), false);
    assert!((t[0] - 90.0).abs() < 1e-3 && (t[2] - 10.0).abs() < 1e-3, "{t:?}");
    // Blocked straight ahead at 40 m but open at -22.5 degrees: the first odd probe wins.
    let wedge = mesh(&grid(60, 60, 2.0, |i, j| if (24..27).contains(&i) && j >= 20 { None } else { Some(AREA_DEFAULT) }));
    let from = wedge.locate([51.0, 0.0, 10.0]).unwrap();
    let (t, _) = choose_target(&wedge, &p, from, forward(0.0), false);
    let a = (-22.5f32).to_radians();
    let expect = [51.0 + 40.0 * a.sin(), 10.0 + 40.0 * a.cos()];
    assert!((t[0] - expect[0]).abs() < 1e-3 && (t[2] - expect[1]).abs() < 1e-3, "{t:?} vs {expect:?}");
    // skip_long (after a failure event): straight at 10 m.
    let (t, _) = choose_target(&wedge, &p, from, forward(0.0), true);
    assert!((t[2] - 20.0).abs() < 1e-3);
    // Nothing fits (a 2 x 2 m island): the fallback step max(3, 2.0 x 1.1) = 3 m.
    let island = mesh(&grid(1, 1, 2.0, |_, _| Some(AREA_DEFAULT)));
    let from = island.locate([1.0, 0.0, 1.0]).unwrap();
    let (t, located) = choose_target(&island, &p, from, forward(0.0), false);
    assert!((t[2] - 4.0).abs() < 1e-5 && located.is_none());
}

/// A kinematic body for the tests: walks 1.325 m/s (the walk clip) when told to walk, turns
/// 180 degrees on a turn intent, keeps to the mesh and to the separation rule.
#[derive(Clone, Debug, PartialEq)]
struct Body {
    id: u64,
    pos: [f32; 3],
    heading: f32,
    nav: PedNav,
    state: Locomotion,
}

fn run(m: &NavMesh, bodies: &mut [Body], ticks: u32, rule: CrosswalkRule, signals: &dyn WalkSignals, mut each: impl FnMut(u32, &[Body])) {
    let p = WanderParams::default();
    let dt = 1.0 / 60.0;
    for t in 0..ticks {
        for k in 0..bodies.len() {
            let others: Vec<Neighbour> = bodies.iter().map(|b| Neighbour { order: b.id, position: b.pos }).collect();
            let b = &mut bodies[k];
            let out = b.nav.step(m, &p, rule, signals, b.id, b.pos, b.heading, b.state, &others, dt);
            b.heading += out.turn;
            match out.intent {
                Intent::Walk | Intent::Run => {
                    b.state = Locomotion::Walk;
                    let f = forward(b.heading);
                    let to = [b.pos[0] + f[0] * 1.325 * dt, b.pos[1], b.pos[2] + f[1] * 1.325 * dt];
                    let (next, ok) = constrain_step(m, b.pos, to);
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
        }
        each(t, bodies);
    }
}

#[test]
fn wandering_stays_on_walkable_ground_and_is_deterministic() {
    // A plaza with a blocked block and a hole.
    let input = grid(30, 30, 2.0, |i, j| match (i, j) {
        (10..=14, 10..=14) => None,
        (20..=22, 4..=8) => Some(AREA_OTHER),
        _ => Some(AREA_DEFAULT),
    });
    let m = mesh(&input);
    let start = || vec![Body { id: 1, pos: [5.0, 0.0, 5.0], heading: 0.3, nav: PedNav::default(), state: Locomotion::Idle }];
    let mut a = start();
    let mut b = start();
    run(&m, &mut a, 60 * 120, CrosswalkRule::Off, &NoSignals, |_, bs| {
        assert!(m.locate(bs[0].pos).is_some(), "off the mesh at {:?}", bs[0].pos);
    });
    run(&m, &mut b, 60 * 120, CrosswalkRule::Off, &NoSignals, |_, _| {});
    assert_eq!(a, b, "same inputs, same walk");
    assert!(a[0].nav.targets_chosen >= 3, "kept choosing targets: {}", a[0].nav.targets_chosen);
    let moved = ((a[0].pos[0] - 5.0).powi(2) + (a[0].pos[2] - 5.0).powi(2)).sqrt();
    assert!(moved > 1.0, "walked somewhere");
}

struct Lamp(std::cell::Cell<Light>);
impl WalkSignals for Lamp {
    fn walk_light(&self, _: [f32; 3]) -> Option<Light> {
        Some(self.0.get())
    }
}

#[test]
fn crosswalk_rule_waits_for_the_walk_light() {
    // Pavement z 0..10, road z 10..20, pavement z 20..30; a route across.
    let input = grid(5, 15, 2.0, |_, j| Some(if (5..10).contains(&j) { AREA_ROAD } else { AREA_DEFAULT }));
    let m = mesh(&input);
    let lamp = Lamp(std::cell::Cell::new(Light::Red));
    let mut nav = PedNav::default();
    nav.set_route(Some(PedRoute { points: vec![[5.0, 0.0, 26.0]], looped: false }));
    let mut bodies = vec![Body { id: 1, pos: [5.0, 0.0, 6.0], heading: 0.0, nav, state: Locomotion::Idle }];
    let mut waited = 0;
    run(&m, &mut bodies, 60 * 10, CrosswalkRule::WalkSignal, &lamp, |_, bs| {
        assert!(bs[0].pos[2] < 10.0, "entered the road on red: {:?}", bs[0].pos);
        if bs[0].nav.waiting == NavWait::Crosswalk {
            waited += 1;
        }
    });
    assert!(waited > 60 * 5, "stood at the kerb: {waited}");
    lamp.0.set(Light::Amber);
    run(&m, &mut bodies, 60, CrosswalkRule::WalkSignal, &lamp, |_, bs| assert!(bs[0].pos[2] < 10.0));
    lamp.0.set(Light::Green);
    run(&m, &mut bodies, 60 * 20, CrosswalkRule::WalkSignal, &lamp, |_, _| {});
    assert!(bodies[0].pos[2] > 20.0, "crossed on green: {:?}", bodies[0].pos);
    // Retail default (Off): crosses without looking.
    let mut nav = PedNav::default();
    nav.set_route(Some(PedRoute { points: vec![[5.0, 0.0, 26.0]], looped: false }));
    let mut bodies = vec![Body { id: 1, pos: [5.0, 0.0, 6.0], heading: 0.0, nav, state: Locomotion::Idle }];
    lamp.0.set(Light::Red);
    run(&m, &mut bodies, 60 * 20, CrosswalkRule::Off, &lamp, |_, _| {});
    assert!(bodies[0].pos[2] > 20.0);
}

#[test]
fn crosswalk_lights_come_from_the_shared_clock() {
    let timings = SignalTimings { green: 7.0, amber: 1.0, all_red: 0.5, walk_split: 0.4 };
    let mut clock = SignalClock::new(timings);
    let arms = vec![(0usize, 0u8, [0.0, 0.0, -10.0]), (0, 1, [10.0, 0.0, 0.0]), (0, 2, [0.0, 0.0, 10.0]), (0, 3, [-10.0, 0.0, 0.0])];
    let mut seen = std::collections::BTreeSet::new();
    for _ in 0..(17 * 60) {
        let s = RoadWalkSignals { arms: &arms, clock: &clock, radius: 20.0 };
        for (_, e, p) in &arms {
            let got = s.walk_light([p[0] * 0.9, 0.0, p[2] * 0.9]).unwrap();
            assert_eq!(got, clock.walk_for_end((e + 1) % 4).light);
            seen.insert((*e, got as u8));
        }
        assert!(s.walk_light([100.0, 0.0, 100.0]).is_none());
        clock.advance(1.0 / 60.0);
    }
    assert!(seen.contains(&(0, Light::Green as u8)) && seen.contains(&(0, Light::Red as u8)) && seen.contains(&(1, Light::Green as u8)));
}

#[test]
fn avoidance_keeps_the_agent_radius() {
    // A 4 m wide corridor; two peds walk at each other, a third stands in the way of a fourth.
    let input = grid(40, 2, 2.0, |_, _| Some(AREA_DEFAULT));
    let m = mesh(&input);
    let route = |x: f32| {
        let mut n = PedNav::default();
        n.set_route(Some(PedRoute { points: vec![[x, 0.0, 2.0]], looped: false }));
        n
    };
    let mut bodies = vec![
        Body { id: 1, pos: [10.0, 0.0, 2.0], heading: std::f32::consts::FRAC_PI_2, nav: route(70.0), state: Locomotion::Idle },
        Body { id: 2, pos: [30.0, 0.0, 2.2], heading: -std::f32::consts::FRAC_PI_2, nav: route(2.0), state: Locomotion::Idle },
    ];
    let r = m.agent[1];
    let mut closest = f32::MAX;
    run(&m, &mut bodies, 60 * 60, CrosswalkRule::Off, &NoSignals, |_, bs| {
        let d = ((bs[0].pos[0] - bs[1].pos[0]).powi(2) + (bs[0].pos[2] - bs[1].pos[2]).powi(2)).sqrt();
        closest = closest.min(d);
        assert!(d >= 2.0 * r - 1e-4, "peds overlap: {d}");
    });
    assert!(closest < 3.0, "they did meet ({closest})");
    assert!(bodies[0].pos[0] > 30.0 && bodies[1].pos[0] < 10.0, "and got past each other: {:?} {:?}", bodies[0].pos, bodies[1].pos);
    // separation_ok lets overlapping peds move apart but not closer.
    let others = [Neighbour { order: 9, position: [0.0, 0.0, 0.0] }];
    assert!(separation_ok([0.2, 0.0, 0.0], [0.3, 0.0, 0.0], 1, &others, r));
    assert!(!separation_ok([0.3, 0.0, 0.0], [0.2, 0.0, 0.0], 1, &others, r));
}

/// fix 17: two NavPower tiles stop short of their shared border (a 0.28 m gap, linked by setup)
/// and a wall-top island 3 m above the second tile. A ped walking across the seam must get over
/// the gap (the old step rule snapped it back to the edge it left: walking in place), and no
/// step may put it on the island (an unconnected layer).
#[test]
fn steps_cross_tile_seams_and_never_jump_layers() {
    let sq = |x0: f32, z0: f32, x1: f32, z1: f32, y: f32| vec![[x0, y, z0], [x1, y, z0], [x1, y, z1], [x0, y, z1]];
    let input = NavMeshInput {
        agent: [0.12, 0.35, 0.2, 1.6],
        polygons: vec![
            // Tile A z 0..9.86, tile B z 10.14..20 (edge 2 of A = top, edge 0 of B = bottom).
            NavPolyInput { verts: sq(0.0, 0.0, 4.0, 9.86, 0.0), neighbours: vec![None, None, Some(1), None], area: AREA_DEFAULT },
            NavPolyInput { verts: sq(0.0, 10.14, 4.0, 20.0, 0.0), neighbours: vec![Some(0), None, None, None], area: AREA_DEFAULT },
            // Wall-top island over the seam, 3 m up, linked to nothing.
            NavPolyInput { verts: sq(1.0, 9.0, 3.0, 11.0, 3.0), neighbours: vec![None; 4], area: AREA_DEFAULT },
        ],
    };
    let m = mesh(&input);
    // Old rule (locate the step's end on its own): stuck at the edge it left.
    let old_walk = |m: &NavMesh| {
        let (mut p, mut top) = ([2.0f32, 0.0, 9.5], 0.0f32);
        for _ in 0..60 {
            if let Some(n) = m.locate([p[0], p[1], p[2] + 0.044]) {
                p = n.position;
                top = top.max(p[1]);
            }
        }
        (p, top)
    };
    let mut ground_only = input.clone();
    ground_only.polygons.truncate(2);
    let (stalled, _) = old_walk(&mesh(&ground_only));
    assert!(stalled[2] < 9.9, "control: the old rule stalls at the seam ({stalled:?})");
    // The point query itself keeps a point in the gap on its own layer (it used to pick the
    // island, 3 m up, because the island's polygon is under the point and the ground's is not).
    let gap = m.locate([2.0, 0.0, 10.0]).unwrap();
    assert!(gap.position[1].abs() < 1e-4 && gap.poly != 2, "{gap:?}");
    let (_, top) = old_walk(&m);
    assert!(top < 1e-4);
    // Surface moves: across the gap onto tile B, never onto the island.
    let mut pos = [2.0f32, 0.0, 9.5];
    for _ in 0..60 {
        let (next, ok) = constrain_step(&m, pos, [pos[0], pos[1], pos[2] + 0.044]);
        assert!(ok);
        assert!(next[1].abs() < 1e-4, "stayed on the ground layer: {next:?}");
        pos = next;
    }
    assert!(pos[2] > 11.5, "crossed the seam: {pos:?}");
    // A ped on the ground under the island stays on the ground (the island contains the point too).
    let (next, _) = constrain_step(&m, [2.0, 0.0, 10.5], [2.0, 2.5, 10.55]);
    assert!(next[1].abs() < 1e-4, "{next:?}");
    // A step into a boundary slides onto the edge (no progress beyond it).
    let (edge, ok) = constrain_step(&m, [3.9, 0.0, 5.0], [4.2, 0.0, 5.0]);
    assert!(ok && (edge[0] - 4.0).abs() < 1e-4);
}

/// b331 (retail mover `82EA5C40` / `82EA4C20`): a ped walking into a straight wall at 80 degrees slides along it every
/// tick and gets where the slide leads; each slide is about 17 % of the wanted step, which the old 25 % progress rule
/// refused (the ped stood pinned, b325). Head-on the slide is nothing: the ped stays on the edge.
#[test]
fn a_ped_walking_into_a_wall_at_an_angle_slides_along_it() {
    // A 10 x 4 m strip: the wall is its edge at z = 4.
    let m = mesh(&grid(10, 4, 1.0, |_, _| Some(AREA_DEFAULT)));
    let (dir, speed) = (80f32.to_radians(), 0.044);
    let step = [dir.cos() * speed, 0.0, dir.sin() * speed];
    let mut pos = [1.0, 0.0, 3.95];
    for _ in 0..300 {
        let to = [pos[0] + step[0], 0.0, pos[2] + step[2]];
        let (next, ok) = constrain_step(&m, pos, to);
        assert!(ok, "the slide is applied: {pos:?} -> {next:?}");
        let (wanted, got) = (speed, ((next[0] - pos[0]).powi(2) + (next[2] - pos[2]).powi(2)).sqrt());
        if pos[2] >= 4.0 - 1e-4 {
            assert!(got < 0.25 * wanted, "against the wall the slide is short ({got} of {wanted}), the case the old rule refused");
        }
        pos = next;
    }
    assert!(pos[0] > 3.0 && (pos[2] - 4.0).abs() < 1e-3, "slid along the wall: {pos:?}");
    // Head-on: no slide, the ped stands on the edge.
    let (still, ok) = constrain_step(&m, [5.0, 0.0, 4.0], [5.0, 0.0, 4.044]);
    assert!(ok && (still[0] - 5.0).abs() < 1e-4 && (still[2] - 4.0).abs() < 1e-4, "{still:?}");
}

/// `82E2A500` (b167): 25 / 70 / 115 / 160 degree edges pick 45 / 90 / 135 / 180; below 25 no turn; both sides.
#[test]
fn turn_bucket_edges_follow_retail() {
    let e = WanderParams::default().turn_edges;
    let b = |deg: f32| turn_bucket(deg.to_radians(), &e);
    assert_eq!([b(24.9), b(25.0), b(69.9), b(70.0), b(114.9), b(115.0), b(159.9), b(160.0), b(180.0)], [None, Some(45), Some(45), Some(90), Some(90), Some(135), Some(135), Some(180), Some(180)]);
    assert_eq!([b(-30.0), b(-100.0), b(-170.0)], [Some(45), Some(90), Some(180)]);
}

/// Ours with the still timer's values (`82E97118`, mode 4 only in retail, b336): a walking ped still within 0.13 m for 0.5 s re-paths ONCE to the same target;
/// moving on clears it. Then our NOT RETAIL give-up (2.0 s) gives the target up and plans away from it.
#[test]
fn a_still_walking_ped_repaths_to_its_target_once_then_gives_up() {
    let m = mesh(&grid(20, 20, 2.0, |_, _| Some(AREA_DEFAULT)));
    let p = WanderParams::default();
    assert_eq!((p.stuck_distance, p.stuck_seconds, p.yield_patience), (0.13, 0.5, 2.0), "the still timer's and mode 0's values");
    let mut nav = PedNav::default();
    let pos = [20.0, 0.0, 20.0];
    let dt = 1.0 / 30.0;
    nav.step(&m, &p, CrosswalkRule::Off, &NoSignals, 1, pos, 0.0, Locomotion::Walk, &[], dt);
    let first = nav.target.unwrap();
    let mut ticks = 0;
    while nav.repaths == 0 && ticks < 60 {
        nav.step(&m, &p, CrosswalkRule::Off, &NoSignals, 1, pos, 0.0, Locomotion::Walk, &[], dt);
        ticks += 1;
    }
    assert_eq!(nav.repaths, 1, "re-pathed after {ticks} ticks");
    assert!((ticks as f32 * dt - p.stuck_seconds).abs() < 0.1, "after {} s", ticks as f32 * dt);
    assert_eq!((nav.target, nav.stuck_count), (Some(first), 0), "same target, not given up");
    // Still longer: no second re-path in the same still spell.
    for _ in 0..20 {
        nav.step(&m, &p, CrosswalkRule::Off, &NoSignals, 1, pos, 0.0, Locomotion::Walk, &[], dt);
    }
    assert_eq!(nav.repaths, 1, "once per still spell");
    // Moving more than 0.13 m starts a new spell; still again for 0.5 s re-paths again.
    let moved = [pos[0], 0.0, pos[2] + 0.2];
    for _ in 0..20 {
        nav.step(&m, &p, CrosswalkRule::Off, &NoSignals, 1, moved, 0.0, Locomotion::Walk, &[], dt);
    }
    assert_eq!((nav.repaths, nav.target), (2, Some(first)));
    // The give-up switched off (a mod): never gives up.
    let off = WanderParams { give_up_seconds: 0.0, ..p };
    for _ in 0..200 {
        nav.step(&m, &off, CrosswalkRule::Off, &NoSignals, 1, moved, 0.0, Locomotion::Walk, &[], dt);
    }
    assert_eq!((nav.stuck_count, nav.target), (0, Some(first)), "no give-up when off");
}

/// Our NOT RETAIL safety net (`give_up_seconds`, 2 s; retail's reverse = goal event 4, b158 item 9): a walking ped
/// still pinned gives up its target and plans the next one away from it, skipping the long fan.
#[test]
fn a_pinned_walking_ped_gives_up_and_plans_away_from_its_target() {
    let m = mesh(&grid(20, 20, 2.0, |_, _| Some(AREA_DEFAULT)));
    let p = WanderParams::default();
    let mut nav = PedNav::default();
    let pos = [20.0, 0.0, 20.0];
    let dt = 1.0 / 30.0;
    // Facing +z: the first target lies ahead.
    nav.step(&m, &p, CrosswalkRule::Off, &NoSignals, 1, pos, 0.0, Locomotion::Walk, &[], dt);
    let first = nav.target.unwrap();
    assert!(first[2] > pos[2], "first target ahead: {first:?}");
    // Walking but pinned: the body never moves.
    let mut ticks = 0;
    while nav.stuck_count == 0 && ticks < 90 {
        nav.step(&m, &p, CrosswalkRule::Off, &NoSignals, 1, pos, 0.0, Locomotion::Walk, &[], dt);
        ticks += 1;
    }
    assert_eq!(nav.stuck_count, 1, "gave up after {ticks} ticks");
    assert!((ticks as f32 * dt - p.give_up_seconds).abs() < 0.1, "after {} s", ticks as f32 * dt);
    let next = nav.target.unwrap();
    assert!(next[2] < pos[2], "planned away from the old target: {next:?}");
}

/// b277: the path goal is the nearest point the start can reach (retail `82933A48`, unbounded), not the peds' 0.5 m
/// point query; B's plan acceptance (2.0 h / 1.0 v) then decides.
#[test]
fn path_goal_snaps_to_the_nearest_reachable_point() {
    use crate::living_world::controller_b::{BInput, ControllerB, ControllerBSettings, PathService};
    // Two strips, x 0..5 and x 6..10 (a hole at 5..6): two components.
    let m = mesh(&grid(10, 1, 1.0, |i, _| (i != 5).then_some(AREA_DEFAULT)));
    let start = m.locate([0.5, 0.0, 0.5]).unwrap();
    // 1.2 m off the mesh: the point query misses it, the goal search snaps to the edge.
    let off = [-1.2, 0.0, 0.5];
    assert!(m.locate(off).is_none());
    let at = m.locate_nearest(off, start.poly, f32::MAX).unwrap();
    assert_eq!((at.poly, at.position), (0, [0.0, 0.0, 0.5]));
    // A mod cap below the distance: no goal.
    assert!(m.locate_nearest(off, start.poly, 1.0).is_none());
    // On the other strip: only the start's component counts, so the near edge of the first strip wins.
    let far = m.locate_nearest([6.2, 0.0, 0.5], start.poly, f32::MAX).unwrap();
    assert_eq!((far.poly, far.position), (4, [5.0, 0.0, 0.5]));
    // Under the mesh: straight up onto the polygon above it (distance 7.86).
    let under = m.locate_nearest([2.5, -7.86, 0.5], start.poly, f32::MAX).unwrap();
    assert_eq!((under.poly, under.position), (2, [2.5, 0.0, 0.5]));
    // The plan: a 1.2 m snap is accepted, a 2.67 m snap is rejected (stall #5 at node 152).
    let s = ControllerBSettings::default();
    let input = BInput { position: start.position, body: [1.0, 0.0, 0.0], can_move: true, ..Default::default() };
    let path = PathService::solve(&m, start.position, [-1.2, 0.0, 0.5], s.goal_locate_radius, s.start_locate_radius).unwrap();
    assert_eq!(path.last().unwrap().entry, [0.0, 0.0, 0.5]);
    assert!(ControllerB::activate(&s, &m, [-1.2, 0.0, 0.5], [1.0, 0.0, 0.0], &input).is_some());
    assert!(ControllerB::activate(&s, &m, [-2.67, 0.0, 0.5], [1.0, 0.0, 0.0], &input).is_none());
}

/// b324: a walk-back start off the mesh (a skater stalled on a grind block, inside the block's navmesh cut, 2-3 m up)
/// snaps to the nearest walkable polygon at any distance (retail `829338B0`), not the peds' 0.5 m; laptop 2026-10-19
/// nodes 69 / 36 / 29 / 23 all failed with "no plan".
#[test]
fn walk_back_start_snaps_from_an_obstacle_cut() {
    use crate::living_world::controller_b::{BInput, ControllerB, ControllerBSettings};
    // 10 x 10 m with a 4 x 4 m cut (x, z 3..7) where the block stands.
    let m = mesh(&grid(10, 10, 1.0, |i, j| (!((3..=6).contains(&i) && (3..=6).contains(&j))).then_some(AREA_DEFAULT)));
    let on_block = [5.0, 2.8, 5.2];
    assert!(m.locate(on_block).is_none(), "2 m inside the cut, 2.8 m up: the peds' query refuses it");
    let start = m.locate_start_nearest(on_block, f32::MAX).unwrap();
    // Nearest: the cut's edge at z 7 (1.8 m away horizontally), straight across at ground height.
    assert_eq!(start.position, [5.0, 0.0, 7.0]);
    assert!(m.locate_start_nearest(on_block, 3.0).is_none(), "3.4 m away: a mod cap of 3 m refuses it");
    // Same answer as a scan of every polygon, from points around and inside the cut and off the mesh.
    let brute = |p: [f32; 3]| {
        (0..m.polys_len() as u32).filter_map(|k| m.closest_point(k, p).map(|q| (q, k))).fold(None::<(f32, u32, [f32; 3])>, |best, (q, k)| {
            let d2 = (0..3).map(|i| (q[i] - p[i]).powi(2)).sum::<f32>();
            if best.is_none_or(|(b, bk, _)| d2 < b || (d2 == b && k < bk)) { Some((d2, k, q)) } else { best }
        })
    };
    for p in [[5.0, 2.8, 5.2], [5.0, 0.0, 5.0], [3.1, 1.0, 6.9], [-4.0, 0.0, 12.5], [20.0, -3.0, 4.0], [1.5, 0.0, 1.5]] {
        let got = m.locate_start_nearest(p, f32::MAX).unwrap();
        let (_, k, q) = brute(p).unwrap();
        assert_eq!((got.poly, got.position), (k, q), "{p:?}");
    }
    // The plan: today's start plans and is accepted; with the peds' 0.5 m (the old behaviour) it does not.
    let s = ControllerBSettings::default();
    let input = BInput { position: on_block, body: [1.0, 0.0, 0.0], can_move: true, ..Default::default() };
    let goal = [9.5, 0.0, 9.5];
    assert!(ControllerB::activate(&s, &m, goal, [1.0, 0.0, 0.0], &input).is_some());
    let old = ControllerBSettings { start_locate_radius: 0.5, ..s };
    assert!(ControllerB::activate(&old, &m, goal, [1.0, 0.0, 0.0], &input).is_none());
}
