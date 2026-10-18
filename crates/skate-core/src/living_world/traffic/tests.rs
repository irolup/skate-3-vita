//! Synthetic-graph tests of the traffic core. Data-gated tests on the user's export live in
//! `skate-data/tests/traffic_data.rs`.

use super::*;
use crate::living_world::rng::Rng;
use crate::living_world::Vec3;
use std::collections::BTreeMap;

const NODE: u64 = 0xA;

fn dir(end: u8) -> Vec3 {
    match end & 3 {
        0 => [0.0, 0.0, 1.0],
        1 => [-1.0, 0.0, 0.0],
        2 => [0.0, 0.0, -1.0],
        _ => [1.0, 0.0, 0.0],
    }
}

/// Right of travel direction `f` (y up): (-f.z, 0, f.x) rotated so lanes keep to one side.
fn right_of(f: Vec3) -> Vec3 {
    [-f[2], 0.0, f[0]]
}

fn add(a: Vec3, b: Vec3, s: f32) -> Vec3 {
    [a[0] + b[0] * s, a[1] + b[1] * s, a[2] + b[2] * s]
}

fn dist(a: Vec3, b: Vec3) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

/// One straight segment from `a` to `b` (lane centre line for `lanes` lanes, 4 m each), two
/// pieces.
fn segment(id: u64, from: (u64, u8), to: (u64, u8), a: Vec3, b: Vec3, lanes: u8) -> SegmentInput {
    let length = dist(a, b);
    let f: Vec3 = std::array::from_fn(|i| (b[i] - a[i]) / length);
    let r = right_of(f);
    let half = 2.0 * lanes as f32;
    let mid = add(a, f, length * 0.5);
    let piece = |s: Vec3, e: Vec3, end_distance: f32| PieceInput {
        end_distance,
        centre: Curve::straight(s, e),
        left_start: add(s, r, -half),
        right_start: add(s, r, half),
        left_end: add(e, r, -half),
        right_end: add(e, r, half),
    };
    SegmentInput {
        id: SegmentId(id),
        from_node: from.0,
        from_end: from.1,
        to_node: to.0,
        to_end: to.1,
        length,
        speed_limit: 14.166_667,
        lanes,
        manoeuvres: 2,
        district: 0,
        pieces: vec![piece(a, mid, length * 0.5), piece(mid, b, length)],
    }
}

/// A 4-way junction at the origin with one-lane roads: approaches 0x100 + k arrive at end k,
/// exits 0x200 + k leave from end k; connectors left / straight / right from every end
/// (index = 3 k + turn). Both outer ends are dead ends.
fn four_way(signalled: bool) -> RoadInput {
    let mut segments = Vec::new();
    let mut connectors = Vec::new();
    let mut approaches: [Option<EndInput>; 4] = Default::default();
    let mut exits: [Option<EndInput>; 4] = Default::default();
    let mut into_exit: [Vec<u32>; 4] = Default::default();
    for k in 0..4u8 {
        let d = dir(k);
        // Approach travels towards the centre (-d); its lane sits to its right.
        let ra = right_of([-d[0], -d[1], -d[2]]);
        let a0 = add(add([0.0; 3], d, 40.0), ra, 2.0);
        let a1 = add(add([0.0; 3], d, 8.0), ra, 2.0);
        segments.push(segment(0x100 + k as u64, (0x10 + k as u64, 0), (NODE, k), a0, a1, 1));
        let rx = right_of(d);
        let x0 = add(add([0.0; 3], d, 8.0), rx, 2.0);
        let x1 = add(add([0.0; 3], d, 40.0), rx, 2.0);
        segments.push(segment(0x200 + k as u64, (NODE, k), (0x20 + k as u64, 0), x0, x1, 1));
    }
    let start = |k: u8| {
        let d = dir(k);
        add(add([0.0; 3], d, 8.0), right_of([-d[0], -d[1], -d[2]]), 2.0)
    };
    let finish = |k: u8| {
        let d = dir(k);
        add(add([0.0; 3], d, 8.0), right_of(d), 2.0)
    };
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
    // The shipped `trafficlights` record values (asserted against the export in skate-data).
    SignalTimings { green: 7.0, amber: 1.0, all_red: 0.5, walk_split: 0.4 }
}

fn con(net: &RoadNetwork, from: u8, to: u8) -> usize {
    (0..net.connectors.len()).find(|&c| net.connectors[c].from_end == from && net.connectors[c].to_end == to).unwrap()
}

#[test]
fn graph_resolves_ends_connectors_and_turns() {
    let net = RoadNetwork::build(&four_way(true)).unwrap();
    assert_eq!(net.segments.len(), 8);
    assert!(net.segments.windows(2).all(|w| w[0].id < w[1].id), "sorted by id");
    assert_eq!(net.connectors.len(), 12);
    let a0 = net.segment_index(SegmentId(0x100)).unwrap();
    assert_eq!(net.segments[a0].to_junction, Some(0));
    assert_eq!(net.segments[a0].from_junction, None);
    assert_eq!(net.next_connectors(a0, 0).len(), 3);
    for c in 0..12 {
        let k = &net.connectors[c];
        let exit = net.connector_exit(c).unwrap();
        assert_eq!(net.segments[exit].id, SegmentId(0x200 + k.to_end as u64));
        assert_eq!(net.segments[net.connector_approach(c).unwrap()].id, SegmentId(0x100 + k.from_end as u64));
    }
    assert_eq!(net.connectors[con(&net, 0, 1)].turn(), Turn::Left);
    assert_eq!(net.connectors[con(&net, 0, 2)].turn(), Turn::Straight);
    assert_eq!(net.connectors[con(&net, 0, 3)].turn(), Turn::Right);
    assert_eq!(Turn::between(2, 1), Turn::Right);
    assert_eq!(net.connector_index(ConnectorId { junction: JunctionId(NODE), index: 5 }), Some(5));
    // Input order does not change the network (stable dense indices).
    let mut shuffled = four_way(true);
    shuffled.segments.reverse();
    shuffled.junctions[0].connectors.reverse();
    assert_eq!(RoadNetwork::build(&shuffled).unwrap(), net);
}

#[test]
fn graph_rejects_bad_references() {
    let mut bad = four_way(true);
    bad.junctions[0].connectors[0].to_lane = 3;
    assert!(RoadNetwork::build(&bad).is_err());
    let mut bad = four_way(true);
    bad.junctions[0].approaches[0].as_mut().unwrap().lane_connectors[0].push(99);
    assert!(RoadNetwork::build(&bad).is_err());
    let mut bad = four_way(true);
    bad.segments[1].id = bad.segments[0].id;
    assert!(RoadNetwork::build(&bad).is_err());
    let mut bad = four_way(true);
    bad.segments[0].pieces[1].end_distance = 1.0;
    assert!(RoadNetwork::build(&bad).is_err());
}

#[test]
fn lane_frames_sit_4m_apart_and_follow_the_road() {
    let input = RoadInput { segments: vec![segment(1, (1, 0), (2, 0), [0.0, 0.0, 0.0], [0.0, 0.0, 20.0], 2)], junctions: vec![] };
    let net = RoadNetwork::build(&input).unwrap();
    let l0 = net.lane_frame(0, 0.0, 5.0);
    let l1 = net.lane_frame(0, 1.0, 5.0);
    assert!((dist(l0.position, l1.position) - 4.0).abs() < 1e-4);
    assert!((l0.position[2] - 5.0).abs() < 1e-4 && (l0.position[0] - 2.0).abs() < 1e-4, "lane 0 at the left edge side: {:?}", l0.position);
    assert!((l1.position[0] + 2.0).abs() < 1e-4);
    assert!((l0.yaw()).abs() < 1e-5, "facing +z");
    let mid = net.lane_frame(0, 0.5, 5.0);
    assert!(mid.position[0].abs() < 1e-4, "half a lane shift is the centre line");
    assert_eq!(net.adjacent_lanes(0, 0), (None, Some(1)));
    assert_eq!(net.adjacent_lanes(0, 1), (Some(0), None));
    let (seg, lane, d, d2) = net.nearest_lane([-1.9, 0.0, 12.0]).unwrap();
    assert_eq!((seg, lane), (0, 1));
    assert!((d - 12.0).abs() < 1e-3 && d2 < 0.02);
}

#[test]
fn cursor_is_continuous_across_pieces_connectors_and_segments() {
    let net = RoadNetwork::build(&four_way(true)).unwrap();
    let a0 = net.segment_index(SegmentId(0x100)).unwrap();
    let mut choose = |n: &RoadNetwork, s: usize, l: u8| n.next_connectors(s, l).get(1).copied(); // straight on
    let mut cur = LaneCursor::on_lane(&net, a0, 0, 0.0, &mut choose);
    assert_eq!(cur.next, Some(con(&net, 0, 2)));
    let mut last = cur.frame(&net).position;
    let mut entered = Vec::new();
    let mut travelled = 0.0;
    for _ in 0..400 {
        let step = cur.advance(&net, 0.25, &mut choose);
        entered.extend(step.entered.iter().copied());
        let p = cur.frame(&net).position;
        assert!(dist(p, last) <= 0.25 + 1e-3, "jump of {} at {:?}", dist(p, last), cur.place);
        last = p;
        travelled += 0.25;
        if step.at_dead_end {
            break;
        }
    }
    let c = con(&net, 0, 2);
    let x2 = net.segment_index(SegmentId(0x202)).unwrap();
    assert_eq!(entered, vec![Place::Connector { connector: c }, Place::Lane { segment: x2, lane: 0 }]);
    assert_eq!(cur.place, Place::Lane { segment: x2, lane: 0 });
    assert_eq!(cur.next, None, "the exit ends at a dead end");
    assert!((cur.distance - net.segments[x2].length).abs() < 1e-4);
    let total = net.segments[a0].length + net.connectors[c].length() + net.segments[x2].length;
    assert!(travelled >= total - 0.25 && travelled <= total + 0.25);
    // One big step lands at the same place as many small ones.
    let mut big = LaneCursor::on_lane(&net, a0, 0, 0.0, &mut choose);
    let r = big.advance(&net, net.segments[a0].length + 3.0, &mut choose);
    assert_eq!(r.entered, vec![Place::Connector { connector: c }]);
    assert!((big.distance - 3.0).abs() < 1e-4);
    // Heading on the connector of a right turn turns towards the exit.
    let right = con(&net, 0, 3);
    let f = net.connector_frame(right, 1.0).forward;
    assert!((f[0] * f[0] + f[1] * f[1] + f[2] * f[2] - 1.0).abs() < 1e-4);
}

#[test]
fn connector_choice_least_loaded_and_random() {
    let net = RoadNetwork::build(&four_way(true)).unwrap();
    let a0 = net.segment_index(SegmentId(0x100)).unwrap();
    let mut rng = Rng::new(1);
    let zero = |_: usize, _: u8| 0.0f32;
    // All empty: the first of the lane list wins (strict <).
    assert_eq!(choose_connector(&net, a0, 0, ConnectorChoice::LeastLoaded, &zero, &mut rng), Some(net.next_connectors(a0, 0)[0]));
    // Load on the left exit (end 1) and the straight exit (end 2): the right turn wins.
    let x1 = net.segment_index(SegmentId(0x201)).unwrap();
    let x2 = net.segment_index(SegmentId(0x202)).unwrap();
    let load = move |s: usize, _: u8| if s == x1 || s == x2 { 2.0 } else { 0.5 };
    assert_eq!(choose_connector(&net, a0, 0, ConnectorChoice::LeastLoaded, &load, &mut rng), Some(con(&net, 0, 3)));
    // Random: deterministic per seed, always from the list.
    let picks = |seed| {
        let mut r = Rng::new(seed);
        (0..32).map(|_| choose_connector(&net, a0, 0, ConnectorChoice::Random, &zero, &mut r).unwrap()).collect::<Vec<_>>()
    };
    assert_eq!(picks(9), picks(9));
    assert!(picks(9).iter().all(|c| net.next_connectors(a0, 0).contains(c)));
    assert!(picks(9).iter().collect::<std::collections::BTreeSet<_>>().len() > 1);
    // Dead end.
    assert_eq!(choose_connector(&net, x1, 0, ConnectorChoice::LeastLoaded, &zero, &mut rng), None);
}

#[test]
fn programmes_match_the_retail_layout() {
    let clock = SignalClock::new(timings());
    let shape = |p: &[Phase]| p.iter().map(|p| (p.light as u8, (p.length * 1000.0).round() as u32, p.word)).collect::<Vec<_>>();
    // The recomp's TRAFPROG lines for controllers 0 and 1 [trace].
    assert_eq!(shape(&clock.controllers[0].car), vec![(0, 500, 0), (0, 8000, 0), (0, 500, 0), (2, 7000, 2), (1, 1000, 1)]);
    assert_eq!(shape(&clock.controllers[0].walk), vec![(0, 500, 0), (0, 8000, 0), (0, 500, 0), (2, 4200, 1), (1, 2800, 1), (0, 1000, 0)]);
    assert_eq!(shape(&clock.controllers[1].car), vec![(0, 500, 0), (2, 7000, 2), (1, 1000, 1), (0, 500, 0), (0, 8000, 0)]);
    assert_eq!(shape(&clock.controllers[1].walk), vec![(0, 500, 0), (2, 4200, 1), (1, 2800, 1), (0, 1000, 0), (0, 500, 0), (0, 8000, 0)]);
    assert_eq!(clock.controllers[2].car, clock.controllers[0].car);
    assert_eq!(clock.controllers[3].car, clock.controllers[1].car);
    assert_eq!((clock.controllers[0].to_red, clock.controllers[0].to_green), ((0, 0), (2, 2)));
    assert_eq!((clock.controllers[1].to_red, clock.controllers[1].to_green), ((3, 4), (0, 0)));
}

/// Ticks between phase changes of one controller, from a fresh clock.
fn change_gaps(controller: u8, ticks: u64) -> Vec<(u64, usize, usize)> {
    let mut clock = SignalClock::new(timings());
    let mut out = Vec::new();
    let mut last = 0;
    let mut changes = Vec::new();
    for _ in 0..ticks {
        changes.clear();
        clock.tick(&mut changes);
        for ch in changes.iter().filter(|c| c.controller == controller) {
            out.push((ch.tick - last, ch.car_phase, ch.walk_phase));
            last = ch.tick;
        }
    }
    out
}

#[test]
fn phase_timing_matches_the_recorded_ticks() {
    // proof1.tsv TRAFLIGHT2, controller 0: the first line (one tick in, 0.483 left) then 29
    // ticks to phase 1, 480 to phase 2, 30 to the green, 252 to the walk amber, 168 to the car
    // amber, 60 to phase 0, 30 to phase 1, 480 ... [trace]. Controller 1 (f32 carries make its
    // walk light lead the car light by one tick): 30 to the green, 253 to the walk amber, 167 to
    // the car amber, 1 to the walk red, 59 to the all-red, 1, 29 to the red, 1, 479 [trace].
    let c0 = change_gaps(0, 2100);
    let gaps: Vec<u64> = c0.iter().map(|g| g.0).take(10).collect();
    assert_eq!(gaps, vec![30, 480, 30, 252, 168, 60, 30, 480, 30, 252]);
    assert_eq!(c0[3], (252, 3, 4), "walk amber while the car light stays green");
    let c1: Vec<u64> = change_gaps(1, 2100).iter().map(|g| g.0).take(10).collect();
    assert_eq!(c1, vec![30, 253, 167, 1, 59, 1, 29, 1, 479, 30]);
    // One cycle = 1020 ticks = 17 s.
    let first_cycle: u64 = change_gaps(0, 1100).iter().take(6).map(|g| g.0).sum();
    assert_eq!(first_cycle, 30 + 480 + 30 + 420 + 60);
}

#[test]
fn signal_clock_is_frame_rate_independent() {
    let run = |hz: f64| {
        let mut clock = SignalClock::new(timings());
        let frames = (hz * 60.0).round() as u32;
        let mut changes = Vec::new();
        for _ in 0..frames {
            changes.extend(clock.advance(1.0 / hz));
        }
        (clock.ticks(), changes, clock.controllers.clone())
    };
    let base = run(60.0);
    assert!((3599..=3600).contains(&base.0));
    for hz in [30.0, 144.0, 240.0, 29.97] {
        let other = run(hz);
        let n = base.1.len().min(other.1.len());
        assert_eq!(&base.1[..n], &other.1[..n], "{hz} Hz changes differ");
        assert!(base.0.abs_diff(other.0) <= 1, "{hz} Hz: {} vs {} ticks", other.0, base.0);
    }
    // Exact elapsed times give identical states.
    for hz in [30.0, 120.0, 144.0] {
        let other = run(hz);
        if other.0 == base.0 {
            assert_eq!(other.2, base.2, "{hz} Hz state");
        }
    }
}

#[test]
fn opposite_ends_share_a_phase_and_crossing_ends_never_go_together() {
    let mut clock = SignalClock::new(timings());
    let mut changes = Vec::new();
    for _ in 0..3000 {
        clock.tick(&mut changes);
        let lit = |c: u8| clock.controller_for_end(c).car().light != Light::Red;
        assert!(!(lit(0) && lit(1)), "ends 0 and 1 both moving at tick {}", clock.ticks());
        assert_eq!(clock.controllers[0].car_phase, clock.controllers[2].car_phase);
        assert_eq!(clock.controllers[1].car_phase, clock.controllers[3].car_phase);
    }
}

#[test]
fn approach_binding_uses_the_from_end_controller() {
    let net = RoadNetwork::build(&four_way(true)).unwrap();
    let clock = SignalClock::new(timings());
    for c in 0..net.connectors.len() {
        let from = net.connectors[c].from_end as usize;
        assert_eq!(clock.light_for(&net, c), Some(clock.controllers[from].car()));
    }
    let unsignalled = RoadNetwork::build(&four_way(false)).unwrap();
    assert_eq!(clock.light_for(&unsignalled, 0), None);
}

#[test]
fn green_wave_holds_the_lights_until_cleared() {
    let mut clock = SignalClock::new(timings());
    let mut changes = Vec::new();
    // Controller 0 starts red; a priority car from end 2 asks for green.
    clock.request_green(2);
    assert_eq!(clock.pending_green, Some(0));
    assert_eq!(clock.controllers[0].car_phase, 2, "jumped to the all-red before green");
    assert_eq!(clock.controllers[1].car_phase, 3, "the crossing ends jumped to the all-red before red");
    for _ in 0..31 {
        clock.tick(&mut changes);
    }
    assert_eq!(clock.controllers[2].car().light, Light::Green);
    assert!(clock.frozen);
    let held = clock.controllers.clone();
    for _ in 0..2000 {
        clock.tick(&mut changes);
    }
    assert_eq!(clock.controllers, held, "frozen");
    clock.request_green(0); // already green: stays held
    assert!(clock.frozen);
    clock.clear_priority();
    for _ in 0..600 {
        clock.tick(&mut changes);
    }
    assert_eq!(clock.controllers[0].car().light, Light::Red, "running again");
}

fn snapshot(id: u32, place: Place, distance: f32, speed: f32) -> VehicleSnapshot {
    VehicleSnapshot { id, length: 4.0, speed, place, distance, look_ahead: 10.0, min_gap: 2.0, flagged: false }
}

struct World {
    net: RoadNetwork,
    clock: SignalClock,
    occ: Occupancy,
    cars: BTreeMap<u32, VehicleSnapshot>,
}

impl World {
    fn new(signalled: bool) -> Self {
        World { net: RoadNetwork::build(&four_way(signalled)).unwrap(), clock: SignalClock::new(timings()), occ: Occupancy::default(), cars: BTreeMap::new() }
    }
    fn seg(&self, id: u64) -> usize {
        self.net.segment_index(SegmentId(id)).unwrap()
    }
    /// A car on the approach from `end`, `to_line` metres from its stop line (front).
    fn approaching(&mut self, id: u32, end: u8, to_line: f32, speed: f32) -> VehicleSnapshot {
        let s = self.seg(0x100 + end as u64);
        let d = self.net.segments[s].length - to_line - 2.0;
        let v = snapshot(id, Place::Lane { segment: s, lane: 0 }, d, speed);
        self.occ.enter_lane(s, 0, id);
        self.cars.insert(id, v);
        v
    }
    fn inside(&mut self, id: u32, from: u8, to: u8, distance: f32) {
        let c = con(&self.net, from, to);
        self.occ.enter_connector(c, id);
        self.cars.insert(id, snapshot(id, Place::Connector { connector: c }, distance, 5.0));
    }
    fn ask(&self, me: &VehicleSnapshot, from: u8, to: u8, check_lights: bool) -> EntryInfo {
        junction_entry(&EntryQuery { net: &self.net, signals: &self.clock, occupancy: &self.occ, vehicles: &self.cars, me, connector: con(&self.net, from, to), check_lights })
    }
    fn ticks(&mut self, n: u32) {
        let mut v = Vec::new();
        for _ in 0..n {
            self.clock.tick(&mut v);
        }
    }
}

#[test]
fn junction_entry_obeys_the_light() {
    let mut w = World::new(true);
    // Controller 0 starts in its all-red / red: straight on from end 0 must stop.
    let me = w.approaching(1, 0, 3.0, 0.0);
    assert_eq!(w.ask(&me, 0, 2, true).entry, Entry::Signal);
    assert_eq!(w.ask(&me, 0, 1, true).entry, Entry::Signal, "left on red");
    assert_eq!(w.ask(&me, 0, 3, true).entry, Entry::Go, "right on red");
    assert_eq!(w.ask(&me, 0, 2, false).entry, Entry::Go, "lights not consulted (skitched car)");
    // Controller 1 (end 1) is green after 30 ticks with 7 s to go.
    w.ticks(31);
    let me1 = w.approaching(2, 1, 3.0, 5.0);
    assert_eq!(w.clock.controller_for_end(1).car().light, Light::Green);
    assert_eq!(w.ask(&me1, 1, 3, true).entry, Entry::Go);
    // Near the end of the green a far, slow car cannot clear the line: stop.
    w.ticks(400);
    let far = snapshot(3, Place::Lane { segment: w.seg(0x101), lane: 0 }, 0.0, 0.5);
    assert_eq!(w.clock.controller_for_end(1).car().light, Light::Green);
    assert_eq!(w.ask(&far, 1, 3, true).entry, Entry::Signal);
    // Amber: stop.
    w.ticks(30);
    assert_eq!(w.clock.controller_for_end(1).car().light, Light::Amber);
    assert_eq!(w.ask(&me1, 1, 3, true).entry, Entry::Signal);
}

#[test]
fn junction_entry_speed_and_stop_sign() {
    let mut w = World::new(false);
    // Unsignalled: a moving car beyond its look-ahead keeps approaching (must stop first).
    let me = w.approaching(1, 0, 20.0, 5.0);
    assert_eq!(w.ask(&me, 0, 2, true).entry, Entry::Approach);
    let near = w.approaching(2, 0, 3.0, 5.0);
    assert_eq!(w.ask(&near, 0, 2, true).entry, Entry::Go, "within the look-ahead");
    // Light check off: the connector's entry speed applies (2 m/s on a turn).
    assert_eq!(w.ask(&me, 0, 3, false).entry, Entry::Approach);
    assert_eq!(w.ask(&me, 0, 2, false).entry, Entry::Go, "straight entry speed is the road speed");
}

#[test]
fn junction_entry_yields_and_blocks() {
    let mut w = World::new(false);
    let me = w.approaching(10, 0, 3.0, 0.0);
    // A car crossing (from end 1 straight to end 3) is inside: yield.
    w.inside(20, 1, 3, 3.0);
    assert_eq!(w.ask(&me, 0, 2, true).entry, Entry::Yield);
    w.occ.remove(20);
    w.cars.remove(&20);
    assert_eq!(w.ask(&me, 0, 2, true).entry, Entry::Go);
    // Another connector from my lane in use: yield.
    w.inside(21, 0, 3, 1.0);
    assert_eq!(w.ask(&me, 0, 2, true).entry, Entry::Yield);
    w.occ.remove(21);
    w.cars.remove(&21);
    // My own connector: the car ahead too close for my minimum gap: blocked.
    w.inside(22, 0, 2, 0.5);
    let mut tight = me;
    tight.min_gap = 10.0;
    w.cars.insert(10, tight);
    assert_eq!(w.ask(&tight, 0, 2, true).entry, Entry::Blocked);
    w.occ.remove(22);
    w.cars.remove(&22);
    // No room on the exit lane: its rearmost car is stopped right at the start.
    let x2 = w.seg(0x202);
    w.occ.enter_lane(x2, 0, 30);
    w.cars.insert(30, snapshot(30, Place::Lane { segment: x2, lane: 0 }, 2.5, 0.0));
    assert_eq!(w.ask(&me, 0, 2, true).entry, Entry::Blocked);
    // It moves on: room again.
    w.cars.insert(30, snapshot(30, Place::Lane { segment: x2, lane: 0 }, 10.0, 0.0));
    assert_eq!(w.ask(&me, 0, 2, true).entry, Entry::Go);
}

#[test]
fn unsignalled_cross_traffic_goes_by_distance_then_id() {
    let mut w = World::new(false);
    let me = w.approaching(10, 0, 3.0, 0.0);
    // A crossing car (end 1 -> end 3) that has reserved its connector but is still on its lane.
    let c = con(&w.net, 1, 3);
    let s1 = w.seg(0x101);
    let place = Place::Lane { segment: s1, lane: 0 };
    let len = w.net.segments[s1].length;
    let at = |to_line: f32| len - to_line - 2.0;
    w.occ.enter_connector(c, 20);
    w.cars.insert(20, snapshot(20, place, at(2.0), 0.0));
    assert_eq!(w.ask(&me, 0, 2, true).entry, Entry::Yield, "it is closer");
    w.cars.insert(20, snapshot(20, place, at(5.0), 0.0));
    assert_eq!(w.ask(&me, 0, 2, true).entry, Entry::Go, "we are closer");
    w.cars.insert(20, snapshot(20, place, at(3.0), 0.0));
    assert_eq!(w.ask(&me, 0, 2, true).entry, Entry::Go, "tie: our id 10 is lower");
    let mut higher = me;
    higher.id = 30;
    assert_eq!(w.ask(&higher, 0, 2, true).entry, Entry::Yield, "tie: id 30 waits for 20");
}

#[test]
fn signalled_left_turn_yields_to_oncoming_by_distance() {
    let mut w = World::new(true);
    w.ticks(541);
    assert_eq!(w.clock.controller_for_end(0).car().light, Light::Green);
    assert_eq!(w.clock.controller_for_end(2).car().light, Light::Green);
    // Stopped at the line: the green "clears" only when the front is within half the car's
    // length of the line (speed x remaining green >= distance - length / 2) [code].
    let waiting = w.approaching(9, 0, 3.0, 0.0);
    assert_eq!(w.ask(&waiting, 0, 2, true).entry, Entry::Signal);
    let me = w.approaching(10, 0, 1.5, 0.0);
    assert_eq!(w.ask(&me, 0, 2, true).entry, Entry::Go);
    w.occ.remove(9);
    w.cars.remove(&9);
    // Oncoming straight on (end 2 -> end 0), reserved, still on its lane.
    let c = con(&w.net, 2, 0);
    let s2 = w.seg(0x102);
    let len = w.net.segments[s2].length;
    w.occ.enter_connector(c, 20);
    w.cars.insert(20, snapshot(20, Place::Lane { segment: s2, lane: 0 }, len - 1.0 - 2.0, 0.0));
    assert_eq!(w.ask(&me, 0, 1, true).entry, Entry::Yield, "left turn waits for closer oncoming");
    // A crossing car still on its lane does not matter at a signalled junction.
    w.occ.remove(20);
    w.cars.remove(&20);
    let c = con(&w.net, 1, 3);
    let s1 = w.seg(0x101);
    w.occ.enter_connector(c, 21);
    w.cars.insert(21, snapshot(21, Place::Lane { segment: s1, lane: 0 }, len - 1.0 - 2.0, 0.0));
    assert_eq!(w.ask(&me, 0, 2, true).entry, Entry::Go);
    // Flagged blocker inside the junction: yield with the flag (junction state 5).
    w.occ.remove(21);
    w.cars.remove(&21);
    w.inside(22, 2, 0, 2.0);
    w.cars.get_mut(&22).unwrap().flagged = true;
    let info = w.ask(&me, 0, 1, true);
    assert_eq!(info, EntryInfo { entry: Entry::Yield, blocker_flagged: true });
}

#[test]
fn leader_follows_retail_list_order() {
    let mut occ = Occupancy::default();
    occ.enter_lane(0, 0, 1); // entered first: front
    occ.enter_lane(0, 0, 2);
    occ.enter_lane(0, 0, 3); // newest: rear
    assert_eq!(occ.lane(0, 0), &[3, 2, 1]);
    assert_eq!(occ.leader(0, 0, 3), Some(2));
    assert_eq!(occ.leader(0, 0, 2), Some(1));
    assert_eq!(occ.leader(0, 0, 1), None, "front car");
    assert_eq!(occ.leader(0, 0, 9), Some(3), "not on the lane: the rearmost");
    assert_eq!(occ.leader(0, 1, 9), None);
    occ.remove(2);
    assert_eq!(occ.leader(0, 0, 3), Some(1));
    assert_eq!(occ.lane_load(0, 0), 2.0);
}

#[test]
fn priority_end_within_50m_or_inside() {
    let net = RoadNetwork::build(&four_way(true)).unwrap();
    let a1 = net.segment_index(SegmentId(0x101)).unwrap();
    let c = con(&net, 1, 3);
    let lane = Place::Lane { segment: a1, lane: 0 };
    assert_eq!(priority_end(&net, &lane, 0.0, Some(c)), Some(1), "32 m road: within 50 m");
    assert_eq!(priority_end(&net, &Place::Connector { connector: c }, 1.0, None), Some(1));
    assert_eq!(priority_end(&net, &lane, 0.0, None), None);
    let plain = RoadNetwork::build(&four_way(false)).unwrap();
    assert_eq!(priority_end(&plain, &lane, 0.0, Some(c)), None);
}
