//! Synthetic-graph tests of the V3 lane follower. Data-gated runs on the user's export live in
//! `skate-data/tests/traffic_follow_data.rs`.

use super::*;
use crate::living_world::traffic::graph::{ConnectorInput, Curve, EndInput, JunctionId, JunctionInput, PieceInput, RoadInput, SegmentId, SegmentInput};
use crate::living_world::traffic::signals::{Light, SignalTimings};
use crate::living_world::Vec3;

const NODE: u64 = 0xA;
const DT: f32 = 1.0 / 60.0;

fn dir(end: u8) -> Vec3 {
    match end & 3 {
        0 => [0.0, 0.0, 1.0],
        1 => [-1.0, 0.0, 0.0],
        2 => [0.0, 0.0, -1.0],
        _ => [1.0, 0.0, 0.0],
    }
}
fn right_of(f: Vec3) -> Vec3 {
    [-f[2], 0.0, f[0]]
}
fn add(a: Vec3, b: Vec3, s: f32) -> Vec3 {
    [a[0] + b[0] * s, a[1] + b[1] * s, a[2] + b[2] * s]
}
fn dist(a: Vec3, b: Vec3) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

fn segment(id: u64, from: (u64, u8), to: (u64, u8), a: Vec3, b: Vec3) -> SegmentInput {
    let length = dist(a, b);
    let f: Vec3 = std::array::from_fn(|i| (b[i] - a[i]) / length);
    let r = right_of(f);
    let mid = add(a, f, length * 0.5);
    let piece = |s: Vec3, e: Vec3, end_distance: f32| PieceInput {
        end_distance,
        centre: Curve::straight(s, e),
        left_start: add(s, r, -2.0),
        right_start: add(s, r, 2.0),
        left_end: add(e, r, -2.0),
        right_end: add(e, r, 2.0),
    };
    SegmentInput {
        id: SegmentId(id),
        from_node: from.0,
        from_end: from.1,
        to_node: to.0,
        to_end: to.1,
        length,
        speed_limit: 14.166_667,
        lanes: 1,
        manoeuvres: 2,
        district: 0,
        pieces: vec![piece(a, mid, length * 0.5), piece(mid, b, length)],
    }
}

/// A signalled (or not) 4-way junction at the origin, one-lane 92 m approaches (0x100 + k
/// arriving at end k) and exits (0x200 + k), connectors left / straight / right (index 3 k +
/// turn). The outer ends are dead ends.
fn four_way(signalled: bool) -> RoadInput {
    let mut segments = Vec::new();
    let mut connectors = Vec::new();
    let mut approaches: [Option<EndInput>; 4] = Default::default();
    let mut exits: [Option<EndInput>; 4] = Default::default();
    let mut into_exit: [Vec<u32>; 4] = Default::default();
    let start = |k: u8| {
        let d = dir(k);
        add(add([0.0; 3], d, 8.0), right_of([-d[0], -d[1], -d[2]]), 2.0)
    };
    let finish = |k: u8| {
        let d = dir(k);
        add(add([0.0; 3], d, 8.0), right_of(d), 2.0)
    };
    for k in 0..4u8 {
        let d = dir(k);
        let ra = right_of([-d[0], -d[1], -d[2]]);
        segments.push(segment(0x100 + k as u64, (0x10 + k as u64, 0), (NODE, k), add(add([0.0; 3], d, 100.0), ra, 2.0), start(k)));
        let rx = right_of(d);
        segments.push(segment(0x200 + k as u64, (NODE, k), (0x20 + k as u64, 0), finish(k), add(add([0.0; 3], d, 100.0), rx, 2.0)));
    }
    for f in 0..4u8 {
        let mut list = Vec::new();
        for (turn, delta) in [(0u32, 1u8), (1, 2), (2, 3)] {
            let t = (f + delta) & 3;
            let index = 3 * f as u32 + turn;
            connectors.push(ConnectorInput { index, entry_speed: if delta == 2 { 14.166_667 } else { 2.0 }, from_end: f, from_lane: 0, to_end: t, to_lane: 0, curve: Curve::straight(start(f), finish(t)) });
            list.push(index);
            into_exit[t as usize].push(index);
        }
        approaches[f as usize] = Some(EndInput { lanes: 1, lane_connectors: vec![list] });
    }
    for t in 0..4 {
        exits[t] = Some(EndInput { lanes: 1, lane_connectors: vec![into_exit[t].clone()] });
    }
    RoadInput { segments, junctions: vec![JunctionInput { id: JunctionId(NODE), signalled, speed: 14.166_667, approaches, exits, connectors }] }
}

fn timings() -> SignalTimings {
    SignalTimings { green: 7.0, amber: 1.0, all_red: 0.5, walk_split: 0.4 }
}

/// Only the straight connectors exist in the choice (other lists emptied), so a car goes straight.
fn net_straight(signalled: bool) -> RoadNetwork {
    let mut input = four_way(signalled);
    for j in &mut input.junctions {
        for (f, a) in j.approaches.iter_mut().enumerate() {
            if let Some(a) = a {
                a.lane_connectors = vec![vec![3 * f as u32 + 1]];
            }
        }
    }
    RoadNetwork::build(&input).unwrap()
}

fn seg(net: &RoadNetwork, id: u64) -> usize {
    net.segment_index(SegmentId(id)).unwrap()
}

fn car(net: &RoadNetwork, key: u32, segment: usize, distance: f32) -> Car {
    let mut rng = Rng::new(1);
    let cursor = LaneCursor::on_lane(net, segment, 0, distance, &mut |n, s, l| choose_connector(n, s, l, ConnectorChoice::LeastLoaded, &|_, _| 0.0, &mut rng));
    Car::new(key, cursor, 4.5, FollowParams::default())
}

fn run(net: &RoadNetwork, clock: &mut SignalClock, cars: &mut [Car], ticks: u32) -> Vec<FollowEvent> {
    let mut rng = Rng::new(7);
    let mut out = Vec::new();
    for _ in 0..ticks {
        clock.tick(&mut Vec::new());
        out.extend(step(net, clock, cars, DT, ConnectorChoice::LeastLoaded, &mut rng));
        assert_no_overlap(net, cars);
    }
    out
}

fn assert_no_overlap(net: &RoadNetwork, cars: &[Car]) {
    for a in cars {
        for b in cars {
            if a.key < b.key {
                let (pa, pb) = (a.cursor.frame(net).position, b.cursor.frame(net).position);
                assert!(dist(pa, pb) > 2.0, "cars {} and {} overlap at {pa:?} / {pb:?}", a.key, b.key);
                if a.cursor.place == b.cursor.place {
                    assert!((a.cursor.distance - b.cursor.distance).abs() >= (a.length + b.length) * 0.5, "cars {} / {} overlap on a lane", a.key, b.key);
                }
            }
        }
    }
}

#[test]
fn integrator_matches_retail_rules() {
    // speed += accel x dt
    assert_eq!(integrate(5.0, 2.0, 14.0, 0.5), (6.0, 2.0));
    // accel forced to 0 above the cap, braking kept
    assert_eq!(integrate(15.0, 2.0, 14.0, 0.5), (15.0, 0.0));
    assert_eq!(integrate(15.0, -2.0, 14.0, 0.5), (14.0, -2.0));
    // zeroed when tiny, never negative
    assert_eq!(integrate(0.0005, 0.0, 14.0, 0.5), (0.0, 0.0));
    assert_eq!(integrate(1.0, -10.0, 14.0, 0.5).0, 0.0);
    // stop line: -v^2 / (2 (d - f2) + 0.001)
    let a = stop_accel(10.0, 20.5, 0.5);
    assert!((a + 100.0 / 40.001).abs() < 1e-5);
    assert_eq!(stop_accel(10.0, 0.4, 0.5), f32::NEG_INFINITY);
}

#[test]
fn pulls_away_with_the_ramp_and_holds_the_lane_limit() {
    let net = net_straight(false);
    let mut clock = SignalClock::new(timings());
    let s = seg(&net, 0x100);
    let mut cars = vec![car(&net, 1, s, 0.0)];
    run(&net, &mut clock, &mut cars, 15);
    // 0.25 s: accel ramped by jerk x t (0.8 x 0.25 = 0.2 m/s^2).
    assert!((cars[0].accel - 0.2).abs() < 0.02, "accel {}", cars[0].accel);
    run(&net, &mut clock, &mut cars, 60 * 3);
    assert!(cars[0].accel <= FollowParams::default().accel_max + 1e-4);
    // Peak speed: the lane limit, never more (the car then slows for the stop sign at the end of the road).
    let mut cars2 = vec![car(&net, 1, s, 0.0)];
    let mut peak = 0.0f32;
    for _ in 0..60 * 12 {
        run(&net, &mut clock, &mut cars2, 1);
        peak = peak.max(cars2[0].speed);
    }
    assert!(peak <= 14.166_667 + 1e-3);
    assert!(peak > 13.5, "peak speed {peak}");
}

#[test]
fn follows_lanes_across_a_connector() {
    let net = net_straight(false);
    let mut clock = SignalClock::new(timings());
    let s = seg(&net, 0x100);
    let mut cars = vec![car(&net, 1, s, 50.0)];
    let events = run(&net, &mut clock, &mut cars, 60 * 20);
    let junction = events.iter().position(|e| matches!(e, FollowEvent::EnteredJunction { .. })).expect("entered the junction");
    let lane = events.iter().position(|e| matches!(e, FollowEvent::EnteredLane { segment, .. } if *segment == seg(&net, 0x202))).expect("entered the exit lane");
    assert!(junction < lane);
    // An unsignalled junction is a stop sign: the car stopped (Approach to 0.1 m/s) before entering.
    assert!(events.iter().any(|e| matches!(e, FollowEvent::Junction { entry: Entry::Approach, .. })));
}

#[test]
fn stops_at_red_and_goes_on_green() {
    let net = net_straight(true);
    let mut clock = SignalClock::new(timings());
    // Controller 0 (end 0) starts all-red then red 8 s: the car arrives on red.
    let s = seg(&net, 0x100);
    let mut cars = vec![car(&net, 1, s, 30.0)];
    let mut stopped_on_red = false;
    let mut rng = Rng::new(3);
    let mut entered_light = None;
    // Hold the lights on red for 20 s (a mod's freeze), then let them run.
    clock.frozen = true;
    for t in 0..60 * 50 {
        if t == 60 * 20 {
            clock.frozen = false;
        }
        clock.tick(&mut Vec::new());
        let light = clock.controller_for_end(0).car().light;
        for e in step(&net, &clock, &mut cars, DT, ConnectorChoice::LeastLoaded, &mut rng) {
            if matches!(e, FollowEvent::EnteredJunction { .. }) {
                entered_light = Some(light);
            }
        }
        let c = &cars[0];
        if let Place::Lane { segment, .. } = c.cursor.place {
            if segment == s && light == Light::Red && c.speed < 0.05 && t > 60 * 15 {
                // The front bumper stops the junction standoff before the line (`+3660` = 4.5 x 0.5).
                let to_line = net.segments[s].length - c.cursor.distance - c.length * 0.5;
                assert!((to_line - c.junction_standoff()).abs() < 0.1, "held {to_line} m before the line");
                stopped_on_red = true;
            }
        }
        if entered_light.is_some() {
            break;
        }
    }
    assert!(stopped_on_red, "the car waited at the red light");
    assert_eq!(entered_light, Some(Light::Green), "it entered on green");
}

#[test]
fn keeps_a_gap_and_queues_behind_a_stopped_car() {
    let net = net_straight(true);
    let mut clock = SignalClock::new(timings());
    let s = seg(&net, 0x100);
    let mut cars = vec![car(&net, 1, s, 70.0), car(&net, 2, s, 40.0), car(&net, 3, s, 10.0)];
    // Lights held on red (end 0 starts red): everyone queues.
    clock.frozen = true;
    run(&net, &mut clock, &mut cars, 60 * 25);
    let mut d: Vec<f32> = cars.iter().map(|c| c.cursor.distance).collect();
    d.sort_by(|a, b| b.total_cmp(a));
    for w in d.windows(2) {
        // Bumper to bumper at the lead standoff (half of 4.5 x 0.5 = 1.125 m, `82C38390`).
        let gap = w[0] - w[1] - 4.5;
        assert!((gap - 1.125).abs() < 0.1, "queued at the lead standoff: gap {gap}");
    }
    assert!(cars.iter().all(|c| c.speed < 0.5));
    // On green the queue drains in step.
    clock.frozen = false;
    let events = run(&net, &mut clock, &mut cars, 60 * 20);
    assert!(events.iter().filter(|e| matches!(e, FollowEvent::EnteredJunction { .. })).count() >= 2);
}

#[test]
fn least_loaded_connector_choice_on_lane_entry() {
    let net = RoadNetwork::build(&four_way(false)).unwrap();
    let s = seg(&net, 0x100);
    // Exit lanes: 0x203 (left of end 0 is end 1? use the list order) loaded with cars; the
    // choice avoids the loaded exits.
    let list = net.next_connectors(s, 0).to_vec();
    let exits: Vec<usize> = list.iter().map(|&c| net.connector_exit(c).unwrap()).collect();
    let mut cars = vec![car(&net, 9, exits[0], 50.0), car(&net, 8, exits[1], 50.0)];
    cars.sort_by_key(|c| c.key);
    let occ = occupancy(&cars);
    let mut rng = Rng::new(1);
    let c = choose_connector(&net, s, 0, ConnectorChoice::LeastLoaded, &|seg, lane| occ.lane_load(seg, lane), &mut rng).unwrap();
    assert_eq!(net.connector_exit(c), Some(exits[2]));
}

#[test]
fn motion_is_reproducible_from_the_spawn_state() {
    let net = RoadNetwork::build(&four_way(true)).unwrap();
    let mk = || {
        let mut v: Vec<Car> = (0..4u8).map(|k| car(&net, k as u32 + 1, seg(&net, 0x100 + k as u64), 30.0 + 5.0 * k as f32)).collect();
        v.sort_by_key(|c| c.key);
        v
    };
    let (mut a, mut b) = (mk(), mk());
    let (mut ca, mut cb) = (SignalClock::new(timings()), SignalClock::new(timings()));
    let ea = run(&net, &mut ca, &mut a, 60 * 25);
    let eb = run(&net, &mut cb, &mut b, 60 * 25);
    assert_eq!(a, b);
    assert_eq!(ea, eb);
}

#[test]
fn a_car_hit_by_the_skater_ahead_brakes_to_a_stop_then_drives_on() {
    let net = net_straight(false);
    let mut clock = SignalClock::new(timings());
    let mut cars = vec![car(&net, 1, seg(&net, 0x100), 0.0)];
    run(&net, &mut clock, &mut cars, 180);
    let cruising = cars[0].speed;
    assert!(cruising > 1.0, "{cruising}");
    cars[0].hit_brake = true;
    // accel = -speed: the speed decays and the latch clears when the car stands.
    let mut stopped_at = None;
    for t in 0..600 {
        run(&net, &mut clock, &mut cars, 1);
        if cars[0].speed <= 0.0 {
            stopped_at = Some(t);
            break;
        }
    }
    assert!(stopped_at.is_some(), "the hit brake stops the car");
    assert!(!cars[0].hit_brake);
    run(&net, &mut clock, &mut cars, 60);
    assert!(cars[0].speed > 0.0, "it drives on afterwards");
}

#[test]
fn a_car_stuck_behind_a_standing_car_honks_but_a_red_light_queue_does_not() {
    let net = net_straight(true);
    let mut clock = SignalClock::new(timings());
    let s = seg(&net, 0x100);
    // A lead that never drives (cap 0) in mid-lane: the car behind is blocked (kind 3).
    let mut lead = car(&net, 1, s, 40.0);
    lead.params.cap_scale = 0.0;
    let mut cars = vec![lead, car(&net, 2, s, 10.0)];
    clock.frozen = true;
    run(&net, &mut clock, &mut cars, 60 * 20);
    assert_eq!(cars[1].limiter, super::super::horn::limiter::BEHIND_LEAD);
    assert_eq!(cars[1].horn, 4, "blocked over 4 s: horn kind 4 (driver bit 0x02); gap {} timers {:?} speed {}", cars[0].cursor.distance - cars[1].cursor.distance - 4.5, cars[1].horn_timers, cars[1].speed);
    assert_eq!(cars[0].horn, 0);
    // The red-light queue: the first car waits at the light (1), the ones behind wait with it.
    let mut cars = vec![car(&net, 1, s, 70.0), car(&net, 2, s, 40.0), car(&net, 3, s, 10.0)];
    run(&net, &mut clock, &mut cars, 60 * 25);
    assert!(cars.iter().all(|c| c.speed < 0.5));
    assert_eq!(cars[0].limiter, Entry::Signal as u8);
    assert!(cars[1..].iter().all(|c| c.limiter == super::super::horn::limiter::BEHIND_WAITING_LEAD), "{:?}", cars.iter().map(|c| c.limiter).collect::<Vec<_>>());
    assert!(cars.iter().all(|c| c.horn == 0));
}

#[test]
fn a_held_car_drives_up_to_twelve_tenths_of_the_cap_and_the_player_skips_the_light() {
    let net = net_straight(true);
    let s = seg(&net, 0x100);
    // Top speed over the run (the junction ahead slows both later).
    let top = |held: bool| {
        let mut clock = SignalClock::new(timings());
        let mut cars = vec![car(&net, 1, s, 0.0)];
        cars[0].held = held;
        let mut best = 0.0f32;
        for _ in 0..60 * 15 {
            run(&net, &mut clock, &mut cars, 1);
            best = best.max(cars[0].speed);
        }
        best
    };
    let (free, held) = (top(false), top(true));
    assert!(held > free * 1.15, "{held} vs {free}");
    // A player-held car ignores the red light (82C344D0); an NPC-held one stops.
    let mut clock = SignalClock::new(timings());
    clock.frozen = true;
    let mut player = vec![car(&net, 1, s, 40.0)];
    player[0].held = true;
    player[0].player_held = true;
    let events = run(&net, &mut clock, &mut player, 60 * 20);
    assert!(events.iter().any(|e| matches!(e, FollowEvent::EnteredJunction { .. })), "the player-held car crossed on red");
    assert!(player[0].snapshot().flagged);
}

/// Manoeuvres (b69 / b74, `82C39F78` read by main): with the go roll certain and a short lane timer, a car on a
/// two-lane road changes lane along its passage and ends on the other lane, moving sideways on the way.
#[test]
fn a_car_changes_lane_along_its_passage() {
    let mut input = RoadInput { segments: vec![segment(0x300, (0x1, 0), (0x2, 0), [0.0; 3], [0.0, 0.0, 400.0])], junctions: vec![] };
    input.segments[0].lanes = 2;
    let net = RoadNetwork::build(&input).unwrap();
    let mut clock = SignalClock::new(timings());
    let s = seg(&net, 0x300);
    let mut c = car(&net, 1, s, 30.0);
    c.speed = 12.0;
    c.params.manoeuvre = crate::living_world::traffic::manoeuvre::DeciderParams { lane_timer: 0.5, lane_change_chance: 1.0, least_loaded_chance: 0.0, ..Default::default() };
    let mut cars = vec![c];
    let start_x = cars[0].pose(&net).position[0];
    let mut mid_x = start_x;
    let mut rng = Rng::new(7);
    let mut events = Vec::new();
    for _ in 0..60 * 8 {
        clock.tick(&mut Vec::new());
        events.extend(step(&net, &clock, &mut cars, DT, ConnectorChoice::LeastLoaded, &mut rng));
        if cars[0].passage.is_some() {
            mid_x = cars[0].pose(&net).position[0];
        }
        if events.iter().any(|e| matches!(e, FollowEvent::EnteredLane { .. })) {
            break;
        }
    }
    assert!(events.iter().any(|e| matches!(e, FollowEvent::LaneChange { from: 0, to: 1, .. })), "{events:?}");
    assert!(matches!(cars[0].cursor.place, Place::Lane { lane: 1, .. }), "{:?}", cars[0].cursor.place);
    assert!(cars[0].passage.is_none());
    let end_x = cars[0].pose(&net).position[0];
    assert!((end_x - start_x).abs() > 1.5, "{start_x} -> {end_x}");
    assert!((mid_x - start_x).abs() > 0.1 && (mid_x - start_x).abs() < (end_x - start_x).abs(), "{start_x} {mid_x} {end_x}");
}

/// Pull-over (b73 / b77): on a one-lane road that allows it, the car reserves the road's middle, brakes to a crawl
/// short of it, rides to the kerb slot, parks for the parked time, pulls out and drives on along the road.
#[test]
fn a_car_pulls_over_parks_and_pulls_out() {
    let mut input = RoadInput { segments: vec![segment(0x300, (0x1, 0), (0x2, 0), [0.0; 3], [0.0, 0.0, 400.0])], junctions: vec![] };
    input.segments[0].manoeuvres = 3;
    let net = RoadNetwork::build(&input).unwrap();
    let mut clock = SignalClock::new(timings());
    let s = seg(&net, 0x300);
    let mut c = car(&net, 1, s, 20.0);
    c.speed = 10.0;
    c.params.manoeuvre = crate::living_world::traffic::manoeuvre::DeciderParams { lane_timer: 0.5, lane_change_chance: 0.0, pull_over_chance: 1.0, ..Default::default() };
    c.params.parked_time = 2.0;
    let mut cars = vec![c];
    let lane_x = cars[0].pose(&net).position[0];
    let mut rng = Rng::new(7);
    let mut events = Vec::new();
    let mut parked_x = None;
    for _ in 0..60 * 120 {
        clock.tick(&mut Vec::new());
        events.extend(step(&net, &clock, &mut cars, DT, ConnectorChoice::LeastLoaded, &mut rng));
        if let crate::living_world::traffic::manoeuvre::Manoeuvre::Parked { .. } = cars[0].manoeuvre {
            parked_x.get_or_insert(cars[0].pose(&net).position[0]);
            assert_eq!(cars[0].speed, 0.0);
        }
        if events.iter().any(|e| matches!(e, FollowEvent::PullingOut { .. })) && cars[0].manoeuvre == crate::living_world::traffic::manoeuvre::Manoeuvre::Following {
            break;
        }
    }
    let over = events.iter().find_map(|e| if let FollowEvent::PullingOver { spot, .. } = e { Some(*spot) } else { None }).expect("pulled over");
    assert!((over - 200.0).abs() < 1e-3, "{over}");
    let px = parked_x.expect("parked");
    // The kerb slot is one lane width (4 m on this one-lane road) beside the lane.
    assert!((px - lane_x).abs() > 3.0, "{lane_x} -> {px}");
    assert_eq!(cars[0].manoeuvre, crate::living_world::traffic::manoeuvre::Manoeuvre::Following);
    assert!((cars[0].pose(&net).position[0] - lane_x).abs() < 1e-3);
    assert!(cars[0].cursor.distance > over);
}

/// A sounding car alarm (`+3424` bit 0x10): StayingParked (`82C39138`) holds the parked time at 0 and the car never
/// pulls out (`82C3A3A8`); once the alarm stops, the parked time runs again from 0.
#[test]
fn an_alarming_parked_car_holds_its_parked_time_and_stays() {
    use crate::living_world::traffic::manoeuvre::Manoeuvre;
    let input = RoadInput { segments: vec![segment(0x300, (0x1, 0), (0x2, 0), [0.0; 3], [0.0, 0.0, 400.0])], junctions: vec![] };
    let net = RoadNetwork::build(&input).unwrap();
    let mut clock = SignalClock::new(timings());
    let s = seg(&net, 0x300);
    let mut c = car(&net, 1, s, 200.0);
    c.params.parked_time = 2.0;
    c.manoeuvre = Manoeuvre::Parked { spot: 200.0, time: 1.5 };
    c.alarming = true;
    let mut cars = vec![c];
    let mut rng = Rng::new(7);
    let mut events = Vec::new();
    for _ in 0..60 * 10 {
        clock.tick(&mut Vec::new());
        events.extend(step(&net, &clock, &mut cars, DT, ConnectorChoice::LeastLoaded, &mut rng));
    }
    assert_eq!(cars[0].manoeuvre, Manoeuvre::Parked { spot: 200.0, time: 0.0 }, "{events:?}");
    assert!(!events.iter().any(|e| matches!(e, FollowEvent::PullingOut { .. })));
    cars[0].alarming = false;
    let mut ticks = 0;
    while !events.iter().any(|e| matches!(e, FollowEvent::PullingOut { .. })) {
        clock.tick(&mut Vec::new());
        events.extend(step(&net, &clock, &mut cars, DT, ConnectorChoice::LeastLoaded, &mut rng));
        ticks += 1;
        assert!(ticks < 60 * 5, "no pull-out after the alarm");
    }
    // The full parked time again (2 s), not the 0.5 s left before the alarm.
    assert!((ticks as f32 * DT - 2.0).abs() < 2.0 * DT, "{ticks}");
}

/// The hit brake latches above 5 m/s^2 (`sub_82C3FA08`, `DD49E3C2`): the braking stays at the speed it had when the
/// latch set, so the car stops in about one second instead of a fading decay; the latch clears at the stop.
#[test]
fn a_fast_hit_brake_latches_and_stops_in_about_a_second() {
    let net = net_straight(false);
    let mut clock = SignalClock::new(timings());
    let mut cars = vec![car(&net, 1, seg(&net, 0x100), 0.0)];
    cars[0].speed = 8.0;
    cars[0].hit_brake = true;
    run(&net, &mut clock, &mut cars, 1);
    assert!(cars[0].brake_latched, "8 m/s brakes harder than 5 m/s^2");
    let held = cars[0].accel;
    let mut steps = 1;
    while cars[0].speed > 0.0 && steps < 600 {
        run(&net, &mut clock, &mut cars, 1);
        if cars[0].speed > 0.0 {
            assert_eq!(cars[0].accel, held, "the latched braking does not fade");
        }
        steps += 1;
    }
    assert!((steps as f32 * DT) < 1.2, "stopped after {} s", steps as f32 * DT);
    assert!(!cars[0].brake_latched && !cars[0].hit_brake);
}

/// The spawn draws (`82C3CDD0`, research b75 / b78): the max acceleration in 1 % steps of the spec range, the speed
/// style from the driver's chances (normal / slow / fast) and the held bonus behind its roll.
#[test]
fn spawn_draws_follow_the_spec_and_driver_values() {
    let p = FollowParams { bias_slow: 0.3, bias_fast: 0.5, ..FollowParams::default() };
    let draws = |values: [u32; 3]| {
        let mut it = values.into_iter();
        spawn_draws(&p, &mut || it.next().unwrap())
    };
    assert_eq!(draws([0, 0, 0]), (2.0, 0.0, 0.5), "rand 0: low end, roll 1 is normal, the bonus passes");
    assert_eq!(draws([100, 80, 0]).0, 5.0);
    assert_eq!(draws([50, 84, 0]).1, -0.3, "roll 85 lands in the slow 10 %");
    assert_eq!(draws([50, 95, 0]).1, 0.5, "roll 96 is fast");
    let never = FollowParams { held_bonus_chance: 0.0, ..p };
    let mut it = [0u32, 0, 0].into_iter();
    assert_eq!(spawn_draws(&never, &mut || it.next().unwrap()).2, 0.0);
}

/// The standoff rule (`82C38390` -> `82C41120` -> `82C3FA08`, b162 M1 / b163): beyond `speed + standoff` a standing
/// car ahead does not limit; inside it the brake is `-v^2 / (2 (gap - standoff) + 0.001)`; inside the standoff the
/// car stops dead. A spec override (a mod) of the lead factor moves the stop gap.
#[test]
fn a_car_brakes_by_the_standoff_rule_and_a_mod_factor_moves_the_gap() {
    let net = net_straight(true);
    let mut clock = SignalClock::new(timings());
    clock.frozen = true;
    let s = seg(&net, 0x100);
    // 10 m/s, the standing car's rear 30 m ahead: outside the 10 + 1.125 m window, free road.
    let mut cars = vec![car(&net, 1, s, 64.5), car(&net, 2, s, 30.0)];
    cars[1].speed = 10.0;
    run(&net, &mut clock, &mut cars, 1);
    assert!(cars[1].accel >= 0.0, "outside the window: {}", cars[1].accel);
    // 8 m ahead at 10 m/s: inside the window, the geometric brake.
    let mut cars = vec![car(&net, 1, s, 42.5), car(&net, 2, s, 30.0)];
    cars[1].speed = 10.0;
    let gap: f32 = 42.5 - 30.0 - 4.5;
    run(&net, &mut clock, &mut cars, 1);
    let want = -100.0 / (2.0 * (gap - 1.125) + 0.001);
    assert!((cars[1].accel - want).abs() < 0.05, "accel {} vs {want}", cars[1].accel);
    // Inside the standoff: a dead stop at once.
    let mut cars = vec![car(&net, 1, s, 35.5), car(&net, 2, s, 30.0)];
    cars[1].speed = 3.0;
    run(&net, &mut clock, &mut cars, 1);
    assert_eq!(cars[1].speed, 0.0, "1.0 m gap < 1.125 m standoff");
    // A family-car factor (0.6) queues 4.5 x 0.6 / 2 = 1.35 m behind.
    let mut cars = vec![car(&net, 1, s, 60.0), car(&net, 2, s, 20.0)];
    cars[1].params.lead_standoff_factor = 0.6;
    cars[1].speed = 8.0;
    run(&net, &mut clock, &mut cars, 60 * 10);
    let gap = cars[0].cursor.distance - cars[1].cursor.distance - 4.5;
    assert!((gap - 1.35).abs() < 0.1, "gap {gap}");
}
