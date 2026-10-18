//! Fix 11: dynamic objects (props, mod bodies) as ped navigation obstacles.

use super::anim::{Intent, Locomotion};
use super::nav::{AREA_DEFAULT, NavMesh, NavMeshInput, NavPolyInput, NavRules};
use super::obstacles::*;
use super::wander::*;

fn grid(nx: i32, nz: i32, cell: f32) -> NavMesh {
    let mut index = std::collections::BTreeMap::new();
    let mut polys = Vec::new();
    for j in 0..nz {
        for i in 0..nx {
            index.insert((i, j), polys.len() as u32);
            let (x, z) = (i as f32 * cell, j as f32 * cell);
            polys.push(NavPolyInput { verts: vec![[x, 0.0, z], [x + cell, 0.0, z], [x + cell, 0.0, z + cell], [x, 0.0, z + cell]], neighbours: Vec::new(), area: AREA_DEFAULT });
        }
    }
    for j in 0..nz {
        for i in 0..nx {
            let k = index[&(i, j)];
            let n = [(i, j - 1), (i + 1, j), (i, j + 1), (i - 1, j)];
            polys[k as usize].neighbours = n.iter().map(|c| index.get(c).copied()).collect();
        }
    }
    NavMesh::build(&NavMeshInput { agent: [0.12, 0.35, 0.2, 1.6], polygons: polys }, NavRules::default())
}

const AXES: [[f32; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

fn bin(id: u64, x: f32, z: f32) -> ObstacleInput {
    ObstacleInput { id, center: [x, 0.5, z], axes: AXES, half_extents: [0.35, 0.5, 0.35], velocity: [0.0; 3], inactive: false, held: false }
}

#[test]
fn retail_cut_rules_rest_move_recut_and_carry() {
    let mut o = NavObstacles::new(ObstacleParams::default());
    // Tiny box: half extents raised to 0.2 [code 0x82099280].
    let tiny = ObstacleInput { half_extents: [0.05, 0.05, 0.05], ..bin(7, 0.0, 0.0) };
    assert!(o.update(&[tiny]));
    let f = o.states[&7].cut.unwrap();
    assert!((f.half[0] - 0.2).abs() < 1e-6 && (f.half[1] - 0.2).abs() < 1e-6, "{f:?}");
    let v0 = o.version;
    // Moving faster than 0.4 m/s [code 0x82181B90]: the cut goes.
    let rolling = ObstacleInput { velocity: [0.5, 0.0, 0.0], ..tiny };
    assert!(o.update(&[rolling]));
    assert!(o.states[&7].cut.is_none() && o.states[&7].moving);
    // Slower than 0.4: cut again where it lies.
    let slow = ObstacleInput { center: [1.0, 0.5, 0.0], velocity: [0.3, 0.0, 0.0], ..tiny };
    assert!(o.update(&[slow]));
    assert_eq!(o.states[&7].cut.unwrap().center, [1.0, 0.0]);
    // Re-cut only beyond 0.25 x the smallest half extent (0.25 x 0.2 = 0.05 m) [code 0x820C6D98].
    let v1 = o.version;
    assert!(!o.update(&[ObstacleInput { center: [1.04, 0.5, 0.0], ..slow }]), "0.04 m keeps the cut");
    assert_eq!(o.version, v1);
    assert!(o.update(&[ObstacleInput { center: [1.06, 0.5, 0.0], ..slow }]), "0.06 m re-cuts");
    assert_eq!(o.states[&7].cut.unwrap().center, [1.06, 0.0]);
    assert!(o.version > v1 && v1 > v0);
    // Retail obstacle-off word (+144+4252 == 1, meaning not decoded): no obstacle; gone from the list: removed.
    assert!(o.update(&[ObstacleInput { inactive: true, ..slow }]));
    assert!(o.states[&7].cut.is_none());
    assert!(o.update(&[bin(8, 3.0, 3.0)]));
    assert!(!o.states.contains_key(&7) && o.cut_count() == 1);
    // Unchanged resting props: no new version (nothing rebuilt per tick).
    let v2 = o.version;
    for _ in 0..10 {
        assert!(!o.update(&[bin(8, 3.0, 3.0)]));
    }
    assert_eq!(o.version, v2);
}

#[test]
fn footprint_follows_the_box_orientation() {
    // A bench 2 m long along x, turned 90 degrees about y: long along z.
    let (s, c) = (std::f32::consts::FRAC_PI_2.sin(), std::f32::consts::FRAC_PI_2.cos());
    let turned = ObstacleInput { id: 1, center: [0.0, 0.4, 0.0], axes: [[c, 0.0, -s], [0.0, 1.0, 0.0], [s, 0.0, c]], half_extents: [1.0, 0.4, 0.3], velocity: [0.0; 3], inactive: false, held: false };
    let f = Footprint::of(&turned, 0.2);
    assert!(f.contains([0.0, 0.0, 0.9], 0.0) && !f.contains([0.9, 0.0, 0.0], 0.0), "{f:?}");
    assert!((f.y_min - 0.0).abs() < 1e-5 && (f.y_max - 0.8).abs() < 1e-5);
    // A flat thing below the step height does not block; a ped on a different level is not blocked.
    let mut o = NavObstacles::new(ObstacleParams::default());
    o.update(&[ObstacleInput { id: 2, center: [5.0, 0.05, 5.0], axes: AXES, half_extents: [0.5, 0.05, 0.5], velocity: [0.0; 3], inactive: false, held: false }, turned]);
    assert!(o.blocked([5.0, 0.0, 5.0], 0.35), "retail cuts every box, a flat board too (raised to 0.2 m)");
    assert!(o.blocked([0.0, 0.0, 0.9], 0.35));
    assert!(!o.blocked([0.0, 5.0, 0.9], 0.35), "a ped on a ledge above");
    o.set_params(ObstacleParams { step_height: 0.3, ..ObstacleParams::default() });
    o.update(&[ObstacleInput { id: 2, center: [5.0, 0.05, 5.0], axes: AXES, half_extents: [0.5, 0.05, 0.5], velocity: [0.0; 3], inactive: false, held: false }, turned]);
    assert!(!o.blocked([5.0, 0.0, 5.0], 0.35), "a mod's step height steps over it");
}

#[derive(Clone, Debug, PartialEq)]
struct Body {
    id: u64,
    pos: [f32; 3],
    heading: f32,
    nav: PedNav,
    state: Locomotion,
}

/// Walk kinematic bodies (1.325 m/s) with the obstacles; returns the closest any body came to
/// an obstacle centre in each tick via `each`.
fn run(m: &NavMesh, o: &mut NavObstacles, props: &mut [ObstacleInput], bodies: &mut [Body], ticks: u32, mut each: impl FnMut(u32, &[Body], &NavObstacles, &mut [ObstacleInput])) {
    let p = WanderParams::default();
    let dt = 1.0 / 60.0;
    for t in 0..ticks {
        o.update(props);
        for k in 0..bodies.len() {
            let others: Vec<Neighbour> = bodies.iter().map(|b| Neighbour { order: b.id, position: b.pos }).collect();
            let b = &mut bodies[k];
            let out = b.nav.step_avoiding(m, &p, CrosswalkRule::Off, &NoSignals, b.id, b.pos, b.heading, b.state, &others, Some(o), dt);
            b.heading += out.turn;
            match out.intent {
                Intent::Walk | Intent::Run => {
                    b.state = Locomotion::Walk;
                    let f = forward(b.heading);
                    let to = [b.pos[0] + f[0] * 1.325 * dt, b.pos[1], b.pos[2] + f[1] * 1.325 * dt];
                    let (next, ok) = constrain_step(m, b.pos, to);
                    if let Some(next) = o.resolve_step(b.pos, next, m.agent[1]).filter(|n| ok && separation_ok(b.pos, *n, b.id, &others, m.agent[1]) && m.locate(*n).is_some()) {
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
        each(t, bodies, o, props);
    }
}

fn inside_any(o: &NavObstacles, p: [f32; 3], grow: f32) -> bool {
    o.states.values().any(|s| !s.inactive && s.cut.unwrap_or(s.now).contains(p, grow))
}

fn walker(route: Vec<[f32; 3]>) -> Body {
    let mut nav = PedNav::default();
    nav.set_route(Some(PedRoute { points: route, looped: false }));
    Body { id: 1, pos: [1.0, 0.0, 5.0], heading: std::f32::consts::FRAC_PI_2, nav, state: Locomotion::Idle }
}

#[test]
fn ped_walks_round_a_resting_prop_and_reaches_its_target() {
    let m = grid(20, 10, 1.0);
    let mut o = NavObstacles::new(ObstacleParams::default());
    let mut props = vec![bin(1, 10.0, 5.0)];
    let mut bodies = vec![walker(vec![[19.0, 0.0, 5.0]])];
    let mut closest = f32::INFINITY;
    run(&m, &mut o, &mut props, &mut bodies, 60 * 20, |_, bs, o, _| {
        assert!(!inside_any(o, bs[0].pos, 0.0), "walked into the prop at {:?}", bs[0].pos);
        closest = closest.min((bs[0].pos[0] - 10.0).hypot(bs[0].pos[2] - 5.0));
    });
    assert!((bodies[0].pos[0] - 19.0).abs() < 0.6 && (bodies[0].pos[2] - 5.0).abs() < 0.6, "reached the target: {:?}", bodies[0].pos);
    assert!(closest >= 0.35 + 0.35 - 1e-3, "kept the agent radius from the bin: {closest}");
    // Without obstacles the same walk goes straight through the bin's place (the reported bug).
    let mut free = NavObstacles::new(ObstacleParams { enabled: false, ..ObstacleParams::default() });
    let mut bodies = vec![walker(vec![[19.0, 0.0, 5.0]])];
    let mut through = false;
    run(&m, &mut free, &mut props, &mut bodies, 60 * 20, |_, bs, _, _| through |= (bs[0].pos[0] - 10.0).abs() < 0.3 && (bs[0].pos[2] - 5.0).abs() < 0.3);
    assert!(through, "the control walk crosses the prop");
}

#[test]
fn a_prop_moved_into_the_path_counts_where_it_lies_now() {
    let m = grid(20, 10, 1.0);
    let mut o = NavObstacles::new(ObstacleParams::default());
    // Starts off the path; at 2 s the player kicks it onto the path (rolling, then at rest).
    let mut props = vec![bin(1, 12.0, 8.5)];
    let mut bodies = vec![walker(vec![[19.0, 0.0, 5.0]])];
    run(&m, &mut o, &mut props, &mut bodies, 60 * 20, |t, bs, o, props| {
        if (120..150).contains(&t) {
            props[0].center[2] -= 3.5 / 30.0;
            props[0].velocity = [0.0, 0.0, -3.5];
        } else if t == 150 {
            props[0].velocity = [0.0; 3];
        }
        assert!(!inside_any(o, bs[0].pos, 0.0), "walked into the prop at {:?} (tick {t})", bs[0].pos);
    });
    assert!((props[0].center[2] - 5.0).abs() < 1e-3);
    assert!((bodies[0].pos[0] - 19.0).abs() < 0.6, "reached the target: {:?}", bodies[0].pos);
}

#[test]
fn wandering_crowd_with_props_is_deterministic_and_never_inside_one() {
    let m = grid(30, 30, 1.0);
    let mk = || {
        let props: Vec<ObstacleInput> = (0..12).map(|k| bin(100 + k, 3.0 + (k % 4) as f32 * 7.0, 4.0 + (k / 4) as f32 * 9.0)).collect();
        let bodies: Vec<Body> = (0..6)
            .map(|k| Body { id: k + 1, pos: [1.5 + k as f32 * 4.5, 0.0, 1.0 + (k % 2) as f32 * 27.0], heading: k as f32 * 1.1, nav: PedNav::default(), state: Locomotion::Idle })
            .collect();
        (props, bodies)
    };
    let (mut pa, mut a) = mk();
    let (mut pb, mut b) = mk();
    let mut oa = NavObstacles::new(ObstacleParams::default());
    let mut ob = NavObstacles::new(ObstacleParams::default());
    run(&m, &mut oa, &mut pa, &mut a, 60 * 90, |t, bs, o, _| {
        for x in bs {
            assert!(!inside_any(o, x.pos, 0.0), "ped {} inside a prop at {:?} (tick {t})", x.id, x.pos);
        }
    });
    run(&m, &mut ob, &mut pb, &mut b, 60 * 90, |_, _, _, _| {});
    assert_eq!(a, b, "same seed, same walk");
    assert!(a.iter().map(|x| x.nav.targets_chosen).sum::<u32>() >= 12, "they kept wandering");
}

#[test]
fn step_check_lets_a_ped_out_of_a_prop_pushed_onto_it() {
    let mut o = NavObstacles::new(ObstacleParams::default());
    o.update(&[bin(1, 0.0, 0.0)]);
    // Inside the grown box: a step deeper is refused, a step out is allowed.
    assert!(!o.step_ok([0.6, 0.0, 0.0], [0.55, 0.0, 0.0], 0.35));
    assert!(o.step_ok([0.6, 0.0, 0.0], [0.65, 0.0, 0.0], 0.35));
    assert!(!o.step_ok([1.0, 0.0, 0.0], [0.69, 0.0, 0.0], 0.35));
    assert!(o.step_ok([1.0, 0.0, 0.0], [0.71, 0.0, 0.0], 0.35));
    // Carried props do not block.
    o.update(&[ObstacleInput { inactive: true, ..bin(1, 0.0, 0.0) }]);
    assert!(o.step_ok([1.0, 0.0, 0.0], [0.5, 0.0, 0.0], 0.35));
}

/// A prop held by Move Object stays an obstacle (retail: the hold only sets DMO+4464 bit 0x20, which
/// the obstacle code never reads; the off gate is the separate word +144+4252): cut while held
/// still, solid for a ped's step while dragged faster than 0.4 m/s, and the mod switch restores
/// the earlier "held = ignored" rule.
#[test]
fn held_prop_stays_an_obstacle() {
    let mut o = NavObstacles::new(ObstacleParams::default());
    assert!(ObstacleParams::default().held_is_obstacle, "retail default");
    let held_still = ObstacleInput { held: true, ..bin(1, 0.0, 0.0) };
    assert!(o.update(&[held_still]));
    assert!(o.states[&1].cut.is_some() && o.states[&1].held && !o.states[&1].inactive);
    assert!(o.blocked([0.0, 0.0, 0.0], 0.35));
    // Dragged at 1 m/s: no cut (moving), but a ped cannot step into it.
    let dragged = ObstacleInput { center: [0.5, 0.5, 0.0], velocity: [1.0, 0.0, 0.0], ..held_still };
    assert!(o.update(&[dragged]));
    assert!(o.states[&1].cut.is_none() && o.states[&1].moving);
    assert!(!o.step_ok([-0.5, 0.0, 0.0], [0.3, 0.0, 0.0], 0.35), "a ped walks into the dragged prop");
    assert!(o.step_ok([-1.5, 0.0, 0.0], [-1.4, 0.0, 0.0], 0.35), "clear of it");
    // Stand-in switch (NOT RETAIL YET, retail = NavPower moving avoider): off = not solid.
    o.set_params(ObstacleParams { moving_solid: false, ..ObstacleParams::default() });
    o.update(&[dragged]);
    assert!(o.step_ok([-0.5, 0.0, 0.0], [0.3, 0.0, 0.0], 0.35));
    // Mod switch off: held = ignored (the earlier port's rule).
    o.set_params(ObstacleParams { held_is_obstacle: false, ..ObstacleParams::default() });
    o.update(&[dragged]);
    assert!(o.states[&1].inactive && o.step_ok([-0.5, 0.0, 0.0], [0.3, 0.0, 0.0], 0.35));
    o.update(&[held_still]);
    assert!(o.cut_count() == 0 && !o.blocked([0.0, 0.0, 0.0], 0.35));
}

/// `DisableCollisionsWithBehaviourSource`: a ped that ignores its plugin's own prop steps into it (a seat waypoint
/// inside the bench), while every other obstacle still blocks it.
#[test]
fn a_ped_ignoring_its_plugin_prop_steps_into_it_and_nothing_else() {
    let mut o = NavObstacles::new(ObstacleParams::default());
    let bench = ObstacleInput { half_extents: [1.0, 0.4, 0.3], ..bin(3_905_883_390, 0.0, 0.0) };
    assert!(o.update(&[bench, bin(9, 3.0, 0.0)]));
    let (from, seat) = ([0.0, 0.0, -0.9], [0.0, 0.0, -0.25]);
    assert!(!o.step_ok(from, seat, 0.35), "the bench blocks a ped that does not ignore it");
    assert_eq!(o.resolve_step_ignoring(from, seat, 0.35, Ignore { id: Some(3_905_883_390), skaters: false }), Some(seat));
    let (by_bin, into_bin) = ([3.0, 0.0, -0.9], [3.0, 0.0, -0.2]);
    assert!(o.resolve_step_ignoring(by_bin, into_bin, 0.35, Ignore { id: Some(3_905_883_390), skaters: false }) != Some(into_bin), "another prop still blocks");
}

#[test]
fn a_standing_skater_is_a_moving_avoider_unless_the_ped_ignores_skaters() {
    // b135: the skater's obstacle interface always takes the moving-avoider branch, even at rest.
    let mut o = NavObstacles::new(ObstacleParams::default());
    let h = ObstacleParams::default().skater_half_extents;
    let skater = ObstacleInput { id: SKATER_OBSTACLE_BASE, center: [0.0, h[1], 0.0], axes: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]], half_extents: h, velocity: [0.0; 3], inactive: false, held: false };
    o.update(&[skater]);
    assert!(o.states[&SKATER_OBSTACLE_BASE].moving && o.states[&SKATER_OBSTACLE_BASE].cut.is_none(), "never cut into the mesh");
    assert!(!o.step_ok([-1.0, 0.0, 0.0], [-0.4, 0.0, 0.0], 0.35), "a walking ped does not step into a standing skater");
    // SkaterAvoidance off (a chaser): the skater is no obstacle.
    assert!(o.step_ok_ignoring([-1.0, 0.0, 0.0], [-0.4, 0.0, 0.0], 0.35, Ignore { id: None, skaters: true }));
    // Mod switch.
    o.set_params(ObstacleParams { skaters: false, ..ObstacleParams::default() });
}
