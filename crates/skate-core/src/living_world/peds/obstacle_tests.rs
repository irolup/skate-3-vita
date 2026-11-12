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
                Intent::Walk | Intent::Run | Intent::Shuffle => {
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

/// A newspaper box (half extents about 0.25 m) whose use waypoint stands 0.42 m in front of its centre, inside the
/// box grown by the agent radius (DownTown `EB4A16533EACEC04`, laptop session 2026-10-19: ped #573 pinned 55 s
/// against the back of the box, 1.045 m from the waypoint, just outside TargetWaypoint's 1.0 m slide).
fn newspaper_box() -> ObstacleInput {
    ObstacleInput { id: 7, center: [10.0, 0.5, 5.0], axes: AXES, half_extents: [0.25, 0.5, 0.25], velocity: [0.0; 3], inactive: false, held: false }
}

#[test]
fn a_ped_reaches_a_waypoint_in_front_of_its_prop_from_behind() {
    let m = grid(20, 10, 1.0);
    let w = [9.58, 0.0, 5.0];
    // From behind the box, and from pressed against its back (outside the box grown by the agent radius, inside the
    // detour margin).
    for start in [[12.0, 0.0, 5.1], [10.61, 0.0, 5.17]] {
        let mut o = NavObstacles::new(ObstacleParams::default());
        let mut props = vec![newspaper_box()];
        let mut nav = PedNav::default();
        nav.set_route(Some(PedRoute { points: vec![w], looped: false }));
        let mut bodies = vec![Body { id: 1, pos: start, heading: -std::f32::consts::FRAC_PI_2, nav, state: Locomotion::Idle }];
        run(&m, &mut o, &mut props, &mut bodies, 60 * 20, |_, bs, o, _| assert!(!inside_any(o, bs[0].pos, 0.0), "walked into the box at {:?}", bs[0].pos));
        let d = (bodies[0].pos[0] - w[0]).hypot(bodies[0].pos[2] - w[2]);
        // The nearest free point to the waypoint is 0.19 m from it; the slide (1.0 m) takes the ped the rest.
        assert!(d < 0.3, "from {start:?} the ped ends {d:.2} m from the waypoint at {:?}", bodies[0].pos);
    }
}

#[test]
fn a_target_inside_a_cut_snaps_out_only_within_the_snap_radius() {
    let mut o = NavObstacles::new(ObstacleParams::default());
    o.update(&[newspaper_box()]);
    // 0.42 m in front of the centre: 0.19 m inside the box grown by 0.35, snapped onto the grown edge.
    let s = o.snap_out([9.58, 0.0, 5.0], 0.35).expect("within the snap radius");
    assert!((s[0] - 9.39).abs() < 0.02 && (s[2] - 5.0).abs() < 1e-4, "snapped to {s:?}");
    // The centre is 0.6 m from the grown edge: farther than the 0.5 m snap, no point.
    assert!(o.snap_out([10.0, 0.0, 5.0], 0.35).is_none());
    // A free point stays where it is.
    assert_eq!(o.snap_out([8.0, 0.0, 5.0], 0.35), Some([8.0, 0.0, 5.0]));
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

/// A held prop stays an obstacle (hold sets only DMO+4464 0x20): cut when still, solid when dragged; mod switch.
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

/// Retail's predictive side-step (`82E9CE90` / `82E9CBD0`, b699): left round a standing skater dead ahead, behind a
/// crossing one, nothing beyond 3 s or once touching.
#[test]
fn a_ped_side_steps_a_skater_up_to_three_seconds_ahead() {
    let skater = |x: f32, z: f32, v: [f32; 3]| ObstacleInput { id: SKATER_OBSTACLE_BASE, center: [x, 0.9, z], axes: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]], half_extents: [0.3, 0.9, 0.3], velocity: v, inactive: false, held: false };
    let mut o = NavObstacles::new(ObstacleParams::default());
    let (p, v, r) = ([0.0, 0.0, 0.0], [0.0, 1.4], 0.35);
    o.update(&[skater(0.0, 3.5, [0.0; 3])]);
    let a = o.side_step(p, v, r, Ignore::default());
    assert!((a[0] - 3.9).abs() < 1e-4 && a[1].abs() < 1e-6, "dead ahead -> left (+x when facing +z): {a:?}");
    assert_eq!(o.side_step(p, v, r, Ignore { id: None, skaters: true }), [0.0; 2], "SkaterAvoidance off");
    o.update(&[skater(0.0, 5.5, [0.0; 3])]);
    assert_eq!(o.side_step(p, v, r, Ignore::default()), [0.0; 2], "contact later than 3 s");
    o.update(&[skater(0.0, 0.6, [0.0; 3])]);
    assert_eq!(o.side_step(p, v, r, Ignore::default()), [0.0; 2], "already touching: the contact slide's job");
    // Crossing from the ped's right to its left (+x): the ped steers right, behind it.
    o.update(&[skater(-2.0, 2.0, [1.0, 0.0, 0.0])]);
    let a = o.side_step(p, v, r, Ignore::default());
    assert!(a[0] < 0.0, "behind the crossing skater: {a:?}");
    o.set_params(ObstacleParams { avoid_horizon: 0.0, ..ObstacleParams::default() });
    o.update(&[skater(0.0, 3.5, [0.0; 3])]);
    assert_eq!(o.side_step(p, v, r, Ignore::default()), [0.0; 2], "mod switch off");
}

/// Retail's contact slide (`82E9D0C0`, b698): a ped walking head-on into a standing skater slides round it instead of
/// standing pressed against it (DownTown 2026-10-29: 45 s); a cut prop keeps the face clamp.
#[test]
fn a_ped_running_head_on_into_a_skater_slides_round_it() {
    let mut o = NavObstacles::new(ObstacleParams::default());
    let h = ObstacleParams::default().skater_half_extents;
    let skater = ObstacleInput { id: SKATER_OBSTACLE_BASE, center: [0.0, h[1], 0.0], axes: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]], half_extents: h, velocity: [0.0; 3], inactive: false, held: false };
    o.update(&[skater]);
    let radius = 0.35;
    let mut p = [-0.66, 0.0, 0.0];
    for _ in 0..200 {
        let to = [p[0] + 0.05, 0.0, p[2]];
        p = o.resolve_step(p, to, radius).expect("never refused head-on");
        assert!(!o.blocked(p, radius), "never inside the skater");
    }
    assert!(p[2].abs() > 0.65 && p[0] > 1.0, "slid round the side: {p:?}");
    // Grazing (dot >= -0.3): the face clamp only, no extra slide.
    let g = o.resolve_step([-0.66, 0.0, 0.0], [-0.6, 0.0, 0.3], radius).unwrap();
    assert!((g[2] - 0.3).abs() < 1e-4 && g[0] <= -0.65 + 1e-3);
    // Mod switch off: the old stand against the face.
    o.set_params(ObstacleParams { contact_slide: 0.0, ..ObstacleParams::default() });
    o.update(&[skater]);
    let q = o.resolve_step([-0.66, 0.0, 0.0], [-0.61, 0.0, 0.0], radius).unwrap();
    assert!(q[2].abs() < 1e-6);
}

/// A prop that comes to rest with a ped inside its grown footprint (its own dropped hand prop, laptop 2026-10-19): a step
/// toward the nearest free point (`snap_out`) is allowed, a step on through the middle is not (the walker then walks
/// the ped out first, `peds.rs`, b349).
#[test]
fn a_ped_inside_a_fresh_cut_can_walk_out_toward_the_nearest_free_point() {
    let mut o = NavObstacles::new(ObstacleParams::default());
    // A dropped can: tiny box raised to the 0.2 m minimum half extent, resting at the origin.
    let can = ObstacleInput { half_extents: [0.04, 0.06, 0.04], ..bin(9, 0.0, 0.0) };
    assert!(o.update(&[can]));
    let radius = 0.35;
    let ped = [0.3, 0.0, 0.1];
    assert!(o.blocked(ped, radius), "the ped stands inside the grown footprint");
    let out = o.snap_out(ped, radius + 0.02).expect("a free point within the snap distance");
    assert!(!o.blocked(out, radius));
    let dir = [out[0] - ped[0], out[2] - ped[2]];
    let len = dir[0].hypot(dir[1]);
    let step = [ped[0] + dir[0] / len * 0.05, 0.0, ped[2] + dir[1] / len * 0.05];
    assert!(o.step_ok(ped, step, radius), "walking out is allowed");
    assert!(!o.step_ok(ped, [0.25, 0.0, 0.08], radius), "walking further in is not");
}

/// Research b716 (`82E9DF70`, recomp trace 2026-10-30): the path turn term pulls the side-step's sideways velocity back
/// every tick, so at walking speed (2 m/s, the centripetal limit above the side-step's 3.9) it settles near
/// `a x controller_step` instead of growing, and it drains within a few ticks once nothing pushes.
#[test]
fn the_path_turn_holds_a_side_step_near_one_controller_step_of_push() {
    let o = NavObstacles::new(ObstacleParams::default());
    let (base, speed, dt, a) = ([0.0, 1.0], 2.0, 1.0 / 60.0, 3.9);
    let mut sv = [0.0f32; 2];
    for _ in 0..180 {
        sv = [sv[0] + a * dt, sv[1]];
        sv = o.path_turn(sv, base, speed, dt);
    }
    assert!(sv[0] > 0.05 && sv[0] < 0.2, "settles near 3.9 / 30 = 0.13 m/s: {sv:?}");
    // Retail's 1 / 30 s controller step removes it in one step; our 60 Hz tick halves it each tick (gone in ~0.1 s).
    for _ in 0..6 {
        sv = o.path_turn(sv, base, speed, dt);
    }
    assert!(sv[0].abs() < 3e-3, "gone once nothing pushes: {sv:?}");
    // Standing still: no turn (the centripetal limit is 0), the old fade's job is gone.
    assert_eq!(o.path_turn([0.1, 0.0], base, 0.0, dt), [0.1, 0.0]);
    // A mod can switch the pull-back off.
    let off = NavObstacles::new(ObstacleParams { path_turn_accel: 0.0, ..ObstacleParams::default() });
    assert_eq!(off.path_turn([0.1, 0.0], base, speed, dt), [0.1, 0.0]);
}

/// Research b717 / b718 (`82E9D0C0`, the recomp's compare re-read by main): the contact slide's speed tests. A body
/// moving away along the slide direction at half the path speed (2 m/s) or more gets push-out only; a ped at or below
/// half its path speed takes its direction from its steer target, a faster one from its velocity.
#[test]
fn the_contact_slide_tests_the_avoiders_speed_and_a_slow_peds_steer_target() {
    use super::obstacles::Contact;
    let h = ObstacleParams::default().skater_half_extents;
    let skater = |v: [f32; 3]| ObstacleInput { id: SKATER_OBSTACLE_BASE, center: [0.0, h[1], 0.0], axes: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]], half_extents: h, velocity: v, inactive: false, held: false };
    let (from, to, r) = ([-0.66, 0.0, 0.0], [-0.6, 0.0, 0.0], 0.35);
    let fast = Contact { velocity: Some([1.4, 0.0]), target: Some([5.0, 0.0]) };
    let mut o = NavObstacles::new(ObstacleParams::default());
    o.update(&[skater([0.0; 3])]);
    let slid = o.resolve_step_contact(from, to, r, Ignore::default(), fast).unwrap();
    assert!(slid[2].abs() > 1e-3, "a standing skater: slide round it {slid:?}");
    o.update(&[skater([1.5, 0.0, 0.0])]);
    o.update(&[skater([1.5, 0.0, 0.0])]);
    let pushed = o.resolve_step_contact(from, to, r, Ignore::default(), fast).unwrap_or(from);
    assert!(pushed[2].abs() < 1e-4, "moving away at 1.5 >= 1.0 m/s: push-out only {pushed:?}");
    // Slow (0.5 <= 1.0 m/s) with its steer target off to the side: not heading into the skater, no slide.
    o.update(&[skater([0.0; 3])]);
    o.update(&[skater([0.0; 3])]);
    let slow = Contact { velocity: Some([0.5, 0.0]), target: Some([-0.66, 5.0]) };
    let held = o.resolve_step_contact(from, to, r, Ignore::default(), slow).unwrap_or(from);
    assert!(held[2].abs() < 1e-4, "steer target across: no slide {held:?}");
}

/// Research b716 (`82E9FC20` / `82E9F9F0`): within the goal radius a moving body ahead inside the stopping distance
/// (v^2 / 160 at 80 m/s^2, the ray radius 1.225 + the avoider's 0.455) brakes the ped; farther, beyond the goal radius,
/// or moving away at a quarter of the ped's speed or more, it does not.
#[test]
fn a_ped_brakes_only_for_a_body_inside_its_stopping_distance_near_its_goal() {
    let skater = |z: f32, vz: f32| ObstacleInput { id: SKATER_OBSTACLE_BASE, center: [0.0, 0.9, z], axes: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]], half_extents: [0.3, 0.9, 0.3], velocity: [0.0, 0.0, vz], inactive: false, held: false };
    let mut o = NavObstacles::new(ObstacleParams::default());
    let (p, v, near) = ([0.0, 0.0, 0.0], [0.0, 2.0], [0.0, 1.0]);
    let brakes = |o: &mut NavObstacles, z: f32, vz: f32, target: [f32; 2]| {
        o.update(&[skater(z, vz)]);
        o.update(&[skater(z, vz)]);
        o.brakes(p, v, target, Ignore::default())
    };
    assert!(brakes(&mut o, 1.69, 0.0, near), "inside 1.68 + 0.025");
    assert!(!brakes(&mut o, 1.75, 0.0, near), "beyond the stopping distance");
    assert!(!brakes(&mut o, 1.69, 0.0, [0.0, 5.0]), "goal 5 m away, beyond the 2 m goal radius");
    assert!(!brakes(&mut o, 1.69, 0.6, near), "moving away at 0.6 >= 0.5 m/s");
    o.set_params(ObstacleParams { brake_decel: 0.0, ..ObstacleParams::default() });
    assert!(!brakes(&mut o, 1.69, 0.0, near), "mod switch off");
}
