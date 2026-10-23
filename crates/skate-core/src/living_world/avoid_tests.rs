use super::*;

/// A straight path along +z through the origin, 1 m wide each side.
fn straight(p: Vec3) -> Option<PathPoint> {
    let z = p[2].max(0.0);
    Some(PathPoint { point: [0.0, 0.0, z], direction: [0.0, 0.0, 1.0], width_left: 1.0, width_right: 1.0, node: z as u32, t: z.fract() })
}

fn me(speed: f32) -> AvoidSelf {
    AvoidSelf { position: [0.0; 3], velocity: [0.0, 0.0, speed], forward: [0.0, 0.0, 1.0], lateral: 0.0 }
}

fn ped(id: u64, position: Vec3, velocity: Vec3) -> Obstacle {
    Obstacle { id, kind: ObstacleKind::Pedestrian, position, velocity, axes: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]], length: 0.6, width: 0.6, height: 1.8 }
}

/// A car facing `forward` (unit, xz), 4 m long, 2 m wide, 1.5 m tall.
fn car(id: u64, position: Vec3, forward: Vec3, speed: f32) -> Obstacle {
    let right = [forward[2], 0.0, -forward[0]];
    Obstacle { id, kind: ObstacleKind::Vehicle, position, velocity: scale(forward, speed), axes: [right, [0.0, 1.0, 0.0], forward], length: 4.0, width: 2.0, height: 1.5 }
}

#[test]
fn standing_ped_close_ahead_stops_the_skater() {
    let s = AvoidSettings::default();
    // Off-centre (0.5 m right) it blocks [-0.3, 1.3] of the path and one gap stays: retail steers into it
    // (`+6006` = free gap count != 0, b112), no speed cap.
    let one = evaluate(&s, &mut AvoidState::default(), &me(4.0), &[ped(7, [0.5, 0.0, 2.0], [0.0; 3])], &straight);
    assert_eq!(one.mode, AvoidMode::Steer, "{one:?}");
    assert_eq!(one.shape_speed(4.0, false), 4.0);
    // A second ped 0.5 m left closes the gap: the whole width is blocked, the skater stops.
    let out = evaluate(&s, &mut AvoidState::default(), &me(4.0), &[ped(7, [0.5, 0.0, 2.0], [0.0; 3]), ped(8, [-0.5, 0.0, 2.0], [0.0; 3])], &straight);
    assert!(out.gaps.is_empty(), "{:?}", out.gaps);
    let e = &out.entries[0];
    assert!(e.closing && e.blocks && e.close, "{e:?}");
    // A standing obstacle gets no floor or cap of its own; the close blocker stops (cap 0).
    assert_eq!((e.floor, e.cap), (None, None));
    assert_eq!(out.mode, AvoidMode::SlowDown);
    assert_eq!(out.cap, 0.0);
    assert_eq!(out.shape_speed(4.0, false), 0.0);
    assert_eq!(out.shape_speed(4.0, true), 4.0, "controller +6007 bypasses the cap");
}

#[test]
fn obstacles_outside_the_cone_or_radius_are_ignored() {
    let s = AvoidSettings::default();
    // Behind, beside (60 deg, 5 m), and a ped beyond 8 m.
    let obstacles = [ped(1, [0.0, 0.0, -3.0], [0.0; 3]), ped(2, [4.33, 0.0, 2.5], [0.0; 3]), ped(3, [0.0, 0.0, 9.0], [0.0; 3])];
    let out = evaluate(&s, &mut AvoidState::default(), &me(4.0), &obstacles, &straight);
    assert!(out.entries.is_empty(), "{:?}", out.entries);
    assert_eq!(out.mode, AvoidMode::None);
    // The wide cone keeps a near one at 60 deg (closer than 1.8 m plus its reach).
    let near = evaluate(&s, &mut AvoidState::default(), &me(4.0), &[ped(4, [1.3, 0.0, 0.75], [0.0; 3])], &straight);
    assert_eq!(near.entries.len(), 1);
}

#[test]
fn slow_car_crossing_ahead_makes_the_skater_speed_up() {
    // Car 3 m right of the line, 4 m ahead, crossing at 1 m/s; we ride at 5 m/s.
    let s = AvoidSettings::default();
    let out = evaluate(&s, &mut AvoidState::default(), &me(5.0), &[car(9, [3.0, 0.0, 4.0], [-1.0, 0.0, 0.0], 1.0)], &straight);
    let e = &out.entries[0];
    assert!(e.closing && e.blocks && e.side_on, "{e:?}");
    // Crossing point 4 m ahead, the car needs (3 - 2.5) / 1 = 0.5 s to reach our clearance:
    // floor = 4 / 0.5 + margin 1 = 9; cap = max(0, (4 - 1) / 5.5 - 1) = 0.
    assert!((e.floor.unwrap() - 9.0).abs() < 1e-4, "{e:?}");
    assert_eq!(e.cap, Some(0.0));
    assert!(out.floor_valid);
    // The car is still beside the path: a free gap, so retail steers (b112).
    assert_eq!(out.mode, AvoidMode::Steer);
    // With the whole width blocked the floor wins: speed up.
    let mut out = out;
    out.gaps.clear();
    pick_mode(&s, &me(5.0), &mut out);
    assert_eq!(out.mode, AvoidMode::SpeedUp);
    assert!((out.shape_speed(5.0, false) - 9.0).abs() < 1e-4);
}

#[test]
fn floor_above_speed_plus_headroom_falls_back_to_the_cap() {
    // The same car 1 m from our clearance: floor 4 / 0.05... far above 5 + 5 -> slow down.
    let s = AvoidSettings::default();
    let mut out = evaluate(&s, &mut AvoidState::default(), &me(5.0), &[car(9, [2.55, 0.0, 4.0], [-1.0, 0.0, 0.0], 1.0)], &straight);
    assert!(!out.floor_valid);
    // Whole width blocked (a free gap would steer, b112).
    out.gaps.clear();
    pick_mode(&s, &me(5.0), &mut out);
    assert_eq!(out.mode, AvoidMode::SlowDown);
}

#[test]
fn car_ahead_moving_our_way_is_a_skitch_target() {
    let s = AvoidSettings::default();
    let mut state = AvoidState::default();
    let out = evaluate(&s, &mut state, &me(5.0), &[car(3, [0.0, 0.0, 8.0], [0.0, 0.0, 1.0], 4.0)], &straight);
    assert!(out.entries[0].skitch);
    assert_eq!(out.skitch_target, Some(3));
    assert_eq!(out.mode, AvoidMode::Skitch);
    assert!(out.lateral.abs() < 1e-4, "centred car: {}", out.lateral);
    // Losing it starts the 60-tick cooldown.
    let gone = evaluate(&s, &mut state, &me(5.0), &[], &straight);
    assert_eq!(gone.skitch_target, None);
    assert_eq!(state.skitch_cooldown, 60);
}

#[test]
fn free_gaps_subtract_blocked_intervals() {
    assert_eq!(free_gaps(&[]), vec![[-1.0, 1.0]]);
    assert_eq!(free_gaps(&[[-0.2, 0.3]]), vec![[-1.0, -0.2], [0.3, 1.0]]);
    assert_eq!(free_gaps(&[[-2.0, 2.0]]), Vec::<[f32; 2]>::new());
    assert_eq!(free_gaps(&[[0.5, 1.5]]), vec![[-1.0, 0.5]]);
}

#[test]
fn ped_in_the_middle_of_the_path_splits_it_and_the_skater_steers() {
    // A walking ped (moving across, 0.5 m/s) in the middle, 6 m ahead: blocks [-0.8, 0.8] of the
    // path, leaving two gaps.
    let s = AvoidSettings::default();
    let out = evaluate(&s, &mut AvoidState::default(), &me(5.0), &[ped(5, [0.0, 0.0, 6.0], [0.5, 0.0, 0.0])], &straight);
    assert_eq!(out.mode, AvoidMode::Steer, "{out:?}");
    // Two equal gaps, the skater on the line: the first nearest one (left, centre -0.9).
    assert!((out.lateral + 0.9).abs() < 1e-4, "{}", out.lateral);
    let p = out.entries[0].path.unwrap();
    let q = steer_point(&p, out.lateral);
    // Heading +z, right of the path = d x up = -x (b527): the left gap lies at +x.
    assert!((q[0] - 0.9).abs() < 1e-4 && (q[2] - 6.0).abs() < 1e-4, "{q:?}");
    // Already right of the line: the right gap.
    let right = AvoidSelf { lateral: 0.5, ..me(5.0) };
    let out = evaluate(&s, &mut AvoidState::default(), &right, &[ped(5, [0.0, 0.0, 6.0], [0.5, 0.0, 0.0])], &straight);
    assert!((out.lateral - 0.9).abs() < 1e-4, "{}", out.lateral);
}

#[test]
fn switch_off_gathers_nothing() {
    let s = AvoidSettings { enabled: false, ..Default::default() };
    let out = evaluate(&s, &mut AvoidState::default(), &me(4.0), &[ped(7, [0.0, 0.0, 2.0], [0.0; 3])], &straight);
    assert!(out.entries.is_empty());
    assert_eq!(out.mode, AvoidMode::None);
    assert_eq!(out.shape_speed(4.0, false), 4.0);
}

/// b527 (`sub_82464E30` -> `sub_82454798`): a negative lateral uses the left width (+0x25) and moves to the left of the
/// path (heading +z: +x); a positive one uses the right width toward -x.
#[test]
fn steer_point_puts_each_width_on_its_own_side() {
    let p = PathPoint { point: [0.0, 0.0, 6.0], direction: [0.0, 0.0, 1.0], width_left: 1.0, width_right: 2.0, node: 6, t: 0.0 };
    let left = steer_point(&p, -0.5);
    let right = steer_point(&p, 0.5);
    assert!((left[0] - 0.5).abs() < 1e-5 && (left[2] - 6.0).abs() < 1e-5, "{left:?}");
    assert!((right[0] + 1.0).abs() < 1e-5, "{right:?}");
}