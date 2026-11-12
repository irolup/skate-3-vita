//! Line cursor tests on synthetic lines (no data). The data-gated test on the export lives in
//! `skate-data/tests/living_world_data.rs`.

use super::*;
use crate::living_world::clock::ConsoleClock;

const IDENTITY: [u8; 4] = [128, 128, 128, 255];

fn id(n: u8) -> [u8; 16] {
    let mut i = [0u8; 16];
    i[0] = n;
    i
}

/// A straight line along +Z from `origin`: `count` nodes, `frames` 60 Hz frames apart, `step` m
/// apart.
fn straight(n: u8, origin: Vec3, count: u32, frames: u8, step: f32) -> ReplayLine {
    let nodes = (0..count)
        .map(|i| ReplayNode {
            position: [origin[0], origin[1], origin[2] + i as f32 * step],
            step: [0.0; 3],
            board: IDENTITY,
            skater: IDENTITY,
            frames: if i == 0 { 0 } else { frames },
            event: 0,
            flags: 0,
            jump: None,
            width: [50, 50],
        })
        .collect();
    ReplayLine { id: id(n), flags: 4, skill: 0, nodes, jumps: vec![], groups: vec![] }
}

fn lines(v: Vec<ReplayLine>) -> BTreeMap<[u8; 16], ReplayLine> {
    v.into_iter().map(|l| (l.id, l)).collect()
}

fn ctx<'a>(position: Vec3, players: &'a [Vec3]) -> BranchContext<'a> {
    BranchContext { position, forward: [0.0, 0.0, 1.0], speed: 8.0, players, others: &[], in_use: &[], preferred_skill: -1, online: false, chain: ChainConfig::retail(), tricks: Default::default(), body: None, valid_lines: None }
}

#[test]
fn replay_cursor_follows_the_line_at_60_hz() {
    // 4 frames per node, 0.5 m per node = 7.5 m/s.
    let ls = lines(vec![straight(1, [10.0, 2.0, 0.0], 50, 4, 0.5)]);
    let mut c = LineCursor::spawn(&ls, id(1), 0);
    let mut ev = Vec::new();
    c.advance(6, &ls, &mut Decider::Stay, &mut ev);
    // 6 frames = node 1 + 2 frames into the next segment.
    assert_eq!((c.node, c.frame_in_segment), (1, 2));
    let s = c.sample(&ls, 0.0).unwrap();
    assert!((s.position[2] - 0.75).abs() < 1e-5, "{:?}", s.position);
    assert!((s.velocity[2] - 7.5).abs() < 1e-4);
    assert!(s.heading.abs() < 1e-5);
    // Half a frame later: 1/8 of a node further.
    let h = c.sample(&ls, 0.5).unwrap();
    assert!((h.position[2] - 0.8125).abs() < 1e-5);
    assert_eq!(ev.iter().filter(|e| matches!(e, CursorEvent::Node { .. })).count(), 1);
    assert_eq!(s.phase, ReplayPhase::Rolling);
}

#[test]
fn replay_cursor_is_frame_rate_independent() {
    // The engine converts game time into 60 Hz frames; any engine rate gives the same cursor.
    let ls = lines(vec![straight(1, [0.0; 3], 400, 3, 0.4)]);
    let mut results = Vec::new();
    for engine_hz in [30.0, 60.0, 64.0, 144.0, 240.0] {
        let mut clock = ConsoleClock::new(RECORDING_HZ);
        let mut c = LineCursor::spawn(&ls, id(1), 0);
        let mut ev = Vec::new();
        for _ in 0..(engine_hz * 10.0) as u32 {
            let due = clock.advance(1.0 / engine_hz);
            c.advance(due, &ls, &mut Decider::Stay, &mut ev);
        }
        results.push((engine_hz, c.frames, c.node, c.frame_in_segment));
    }
    for r in &results {
        assert!((599..=600).contains(&r.1), "{r:?}");
    }
    let at600: Vec<_> = results.iter().filter(|r| r.1 == 600).map(|r| (r.2, r.3)).collect();
    assert!(at600.windows(2).all(|w| w[0] == w[1]), "{results:?}");
}

#[test]
fn replay_cursor_phases_follow_flags_and_trick_events() {
    let mut l = straight(1, [0.0; 3], 20, 2, 0.3);
    l.nodes[3].flags = node_flags::CROUCHED;
    l.nodes[5].flags = node_flags::AIRBORNE;
    l.nodes[6].flags = node_flags::AIRBORNE;
    l.nodes[6].event = node_events::START_TRICK;
    l.nodes[7].flags = node_flags::AIRBORNE;
    l.nodes[8].event = node_events::END_TRICK;
    l.nodes[10].event = node_events::START_TRICK;
    l.nodes[12].event = node_events::END_TRICK;
    l.nodes[14].flags = node_flags::OFF_BOARD;
    let ls = lines(vec![l]);
    let mut c = LineCursor::spawn(&ls, id(1), 0);
    let mut seen = Vec::new();
    let mut ev = Vec::new();
    for _ in 0..40 {
        c.step(&ls, &mut Decider::Stay, &mut ev);
        if let Some(s) = c.sample(&ls, 0.0) {
            if seen.last() != Some(&(s.phase)) {
                seen.push(s.phase);
            }
        }
    }
    use ReplayPhase::*;
    assert_eq!(seen, vec![Rolling, Crouched, Rolling, Air, AirTrick, Rolling, GroundTrick, Rolling, OffBoard, Rolling]);
    assert!(ev.iter().any(|e| matches!(e, CursorEvent::Node { event: node_events::START_TRICK, .. })));
}

#[test]
fn replay_cursor_finishes_at_the_line_end() {
    let ls = lines(vec![straight(1, [0.0; 3], 5, 2, 0.3)]);
    let mut c = LineCursor::spawn(&ls, id(1), 0);
    let mut ev = Vec::new();
    c.advance(8, &ls, &mut Decider::Stay, &mut ev);
    assert_eq!(c.node, 4);
    assert!(!c.finished);
    c.step(&ls, &mut Decider::Stay, &mut ev);
    assert!(c.finished);
    assert_eq!(ev.last(), Some(&CursorEvent::Finished));
    // The last sample stays at the end node.
    assert!((c.sample(&ls, 0.0).unwrap().position[2] - 1.2).abs() < 1e-5);
    // Unknown line: finished at once.
    assert!(LineCursor::spawn(&ls, id(9), 0).finished);
}

/// Line 1 runs along +Z; at node 10 a branch to line 2 at node 4. Line 2 runs beside it (x 0.05)
/// and turns 45 deg towards +x after its node 6.
fn branching() -> BTreeMap<[u8; 16], ReplayLine> {
    let mut a = straight(1, [0.0; 3], 40, 2, 0.3);
    a.groups.push(ReplayBranchGroup { node: 10, branches: vec![ReplayBranch { target: id(2), target_node: 4, weight: 0.5 }] });
    let mut b = straight(2, [0.05, 0.0, 1.8], 40, 2, 0.3);
    for (i, n) in b.nodes.iter_mut().enumerate().skip(7) {
        n.position[0] = 0.05 + 0.3 * (i as f32 - 6.0);
    }
    lines(vec![a, b])
}

#[test]
fn replay_branch_score_terms_match_the_code() {
    let ls = branching();
    let l = &ls[&id(1)];
    let players = [[0.0, 0.0, 3.0]];
    // At node 10 (z 3.0), forward +Z, next node straight ahead: angle 0; 400 x 8 = 3200;
    // the player stands on node 10: distance 0.
    let c = ctx([0.0, 0.0, 3.0], &players);
    assert_eq!(branch_score(l, 10, &c), Some(3200));
    // Online: only angle and speed.
    assert_eq!(branch_score(l, 10, &BranchContext { online: true, ..c }), Some(3200));
    // Player 20 m to the side of every node: 30 x 20 = 600.
    let far = [[20.0, 0.0, 3.0]];
    let s = branch_score(l, 10, &ctx([0.0, 0.0, 3.0], &far)).unwrap();
    assert!((3200 + 599..=3200 + 600).contains(&s), "{s}");
    // Beyond 50 m: capped at 1500.
    let very_far = [[500.0, 0.0, 0.0]];
    assert_eq!(branch_score(l, 10, &ctx([0.0, 0.0, 3.0], &very_far)), Some(3200 + 1500));
    // Two other AI skaters within 5 nodes of node 10 on this line: + 2 x 1024.
    let others = [(id(1), 6u32), (id(1), 15), (id(1), 16), (id(2), 10)];
    assert_eq!(branch_score(l, 10, &BranchContext { others: &others, ..c }), Some(3200 + 2048));
    // Flags 7: + 1000. Skill 2 against preferred 0: + 2 x 250 + 100.
    let mut l7 = l.clone();
    l7.flags = 7;
    l7.skill = 2;
    assert_eq!(branch_score(&l7, 10, &BranchContext { preferred_skill: 0, ..c }), Some(3200 + 1000 + 600));
    // 60 deg off the forward: rejected; 30 deg: 300 tenths.
    let side = BranchContext { forward: [1.0, 0.0, 0.0], ..c };
    assert_eq!(branch_score(l, 10, &side), None);
    let f30 = BranchContext { forward: [0.5, 0.0, 0.866_025], ..c };
    let s30 = branch_score(l, 10, &f30).unwrap();
    assert!((3200 + 299..=3200 + 300).contains(&s30), "{s30}");
    // Airborne or event node within one: rejected; at the last node (no next): rejected.
    let mut la = l.clone();
    la.nodes[11].flags = node_flags::AIRBORNE;
    assert_eq!(branch_score(&la, 10, &c), None);
    la.nodes[11].flags = 0;
    la.nodes[9].event = node_events::END_TRICK;
    assert_eq!(branch_score(&la, 10, &c), None);
    assert_eq!(branch_score(l, 39, &c), None);
}

#[test]
fn replay_branch_taken_only_when_it_scores_lower_and_mirrors_on_a_client() {
    let ls = branching();
    // Player on line 2's node 22 (4.85, 8.4): line 2 scores 95 (angle 9.5 deg to its next node)
    // + 0; the stay scores 0 + 30 x 4.85 = 145 -> branch.
    let players = [[4.85, 0.0, 8.4]];
    let run = |players: &[Vec3], in_use: &[[u8; 16]]| {
        let mut c = LineCursor::spawn(&ls, id(1), 0);
        let mut ev = Vec::new();
        for _ in 0..30 {
            let s = c.sample(&ls, 0.0).unwrap();
            let ctx = BranchContext { position: s.position, forward: [0.0, 0.0, 1.0], speed: 9.0, players, others: &[], in_use, preferred_skill: -1, online: false, chain: ChainConfig::retail(), tricks: Default::default(), body: None, valid_lines: None };
            c.step(&ls, &mut Decider::Decide(ctx), &mut ev);
        }
        (c, ev)
    };
    let (c, ev) = run(&players, &[]);
    let branches: Vec<_> = ev.iter().filter_map(|e| if let CursorEvent::Branch(b) = e { Some(b.clone()) } else { None }).collect();
    assert_eq!(branches.len(), 1);
    let b = &branches[0];
    assert_eq!((b.from_line, b.from_node, b.to_line), (id(1), 10, id(2)));
    // Rejoin: nearest of target nodes 1..=4 to (0, 0, 3.0): line 2 node 4 is z 3.0.
    assert_eq!(b.to_node, 4);
    assert_eq!(c.line, id(2));
    // Player beside line 1: the stay wins (ties also keep the stay).
    let (c1, ev1) = run(&[[0.0, 0.0, 9.0]], &[]);
    assert_eq!(c1.line, id(1));
    assert!(!ev1.iter().any(|e| matches!(e, CursorEvent::Branch(_))));
    // Target in use by another skater: skipped.
    let (c2, _) = run(&players, &[id(2)]);
    assert_eq!(c2.line, id(1));
    // A client mirrors the host's records without scoring and ends in the same state.
    let mut m = LineCursor::spawn(&ls, id(1), 0);
    let mut mev = Vec::new();
    m.advance(30, &ls, &mut Decider::Mirror(&branches, &[]), &mut mev);
    assert_eq!(m, c);
    assert_eq!(m.sample(&ls, 0.3), c.sample(&ls, 0.3));
}

#[test]
fn replay_orientation_decodes_x_y_z_w() {
    assert_eq!(decode_orientation(IDENTITY), [0.0, 0.0, 0.0, 1.0]);
    // 90 deg about +Y: (0, sin 45, 0, cos 45) turns +Z onto +X.
    let b = (0.707_107f32 * 127.0 + 128.0).round() as u8;
    let q = decode_orientation([128, b, 128, b]);
    let f = rotate(q, [0.0, 0.0, 1.0]);
    assert!((f[0] - 1.0).abs() < 0.01 && f[2].abs() < 0.01, "{f:?}");
    assert_eq!(decode_orientation([128; 4]), [0.0, 0.0, 0.0, 1.0]);
}

// ---------------------------------------------------------------- leave fade (sub_8246EA90, sub_8245A9B8)

#[test]
fn npc_fades_out_only_at_a_dead_end_and_goes_below_alpha_0_2() {
    use crate::living_world::leave_fade::{LeaveFade, LeaveFadeConfig};
    // 20 segments of 10 frames: the line ends at frame 200 and no other line starts near its end.
    let ls = lines(vec![straight(1, [0.0; 3], 21, 10, 1.0)]);
    let cfg = LeaveFadeConfig::retail();
    let players: [Vec3; 0] = [];
    let mut c = LineCursor::spawn(&ls, id(1), 0);
    let mut fade = LeaveFade::default();
    let mut out = Vec::new();
    let mut started = None;
    let mut removed = None;
    // `t` is the spawn-relative frame the engine keeps counting after the cursor stops.
    for t in 1..400u64 {
        c.step(&ls, &mut Decider::Decide(ctx([0.0; 3], &players)), &mut out);
        if fade.update(&c, &ls, &cfg) {
            started = Some(t);
        }
        if removed.is_none() && fade.should_despawn(t, &cfg) {
            removed = Some(t);
        }
        if started.is_none() {
            assert_eq!(fade.alpha(t, &cfg), cfg.fade_in_alpha(t), "no fade out while the line runs");
        }
    }
    // The fade starts only once the line is over with nowhere to go (never 1 s before the end).
    let s = started.unwrap();
    assert_eq!(c.frames, 200);
    assert!((200..=201).contains(&s), "{s}");
    assert!(out.iter().any(|e| matches!(e, CursorEvent::Finished)));
    // Opacity 1 - t: removed once below 0.2, 0.8 s (48-49 frames, float edge) into the fade.
    // The fade clock starts at the cursor's last frame (200), the line's end.
    let f0 = fade.started.unwrap();
    assert_eq!(f0, 200);
    let r = removed.unwrap() - f0;
    assert!((48..=49).contains(&r), "{r}");
    assert!((fade.alpha(f0 + 30, &cfg) - 0.5).abs() < 1e-6);
    assert_eq!(fade.alpha(199, &cfg), 1.0);
}

/// Line `n` from `from` along `dir` (unit, horizontal): `count` nodes, 10 frames and 1 m apart.
fn along(n: u8, from: Vec3, dir: [f32; 2], count: u32) -> ReplayLine {
    let mut l = straight(n, from, count, 10, 1.0);
    for (i, node) in l.nodes.iter_mut().enumerate() {
        node.position = [from[0] + dir[0] * i as f32, from[1], from[2] + dir[1] * i as f32];
    }
    l
}

#[test]
fn npc_skater_chains_to_a_line_starting_within_4_m_like_sub_8246c7f8() {
    // Line 1 ends at (0, 0, 20). Line 2 starts 1 m further on, line 3 6.7 m away (outside 4 m).
    let ls = lines(vec![along(1, [0.0; 3], [0.0, 1.0], 21), along(2, [0.0, 0.0, 21.0], [0.0, 1.0], 31), along(3, [3.0, 0.0, 26.0], [0.0, 1.0], 31)]);
    let players: [Vec3; 0] = [];
    let run = |chain: ChainConfig, in_use: &[[u8; 16]]| {
        let mut c = LineCursor::spawn(&ls, id(1), 0);
        let mut ev = Vec::new();
        for _ in 0..250 {
            let s = c.sample(&ls, 0.0).unwrap();
            let x = BranchContext { forward: s.velocity, in_use, chain, ..ctx(s.position, &players) };
            c.step(&ls, &mut Decider::Decide(x), &mut ev);
        }
        (c, ev)
    };
    let (c, ev) = run(ChainConfig::retail(), &[]);
    let chained: Vec<_> = ev.iter().filter_map(|e| if let CursorEvent::Branch(b) = e { Some(b.clone()) } else { None }).collect();
    assert_eq!(chained.len(), 1);
    let b = &chained[0];
    // Decided on the frame the last node is reached, onto the new line's start node.
    assert_eq!((b.frame, b.from_line, b.from_node, b.to_line, b.to_node), (200, id(1), 20, id(2), 0));
    assert_eq!(c.line, id(2));
    assert!(!c.finished && !ev.iter().any(|e| matches!(e, CursorEvent::Finished)));
    // A client mirrors the record and ends in the same state.
    let mut m = LineCursor::spawn(&ls, id(1), 0);
    m.advance(250, &ls, &mut Decider::Mirror(&chained, &[]), &mut Vec::new());
    assert_eq!(m, c);
    // A line another skater rides is still taken: retail skips no line for being in use (b715 trace, `82458968`).
    assert_eq!(run(ChainConfig::retail(), &[id(2)]).0.line, id(2));
    // Data-driven: under a mod's 0.5 m radius line 2 (1 m) is out of reach; radius 0 never chains.
    assert!(run(ChainConfig { radius: 0.5, ..ChainConfig::retail() }, &[]).0.finished);
    assert!(run(ChainConfig { radius: 0.0, ..ChainConfig::retail() }, &[]).0.finished);
}

#[test]
fn line_end_choice_scores_like_the_branch_chooser_and_falls_back_to_the_first() {
    let players: [Vec3; 0] = [];
    let end = along(1, [0.0; 3], [0.0, 1.0], 21);
    // Line 2 starts 1 m to the side heading +Z (next node 26.6 deg off), line 4 straight ahead
    // (0 deg): the lower score wins although line 2 comes first in id order.
    let side = along(2, [1.0, 0.0, 21.0], [0.0, 1.0], 10);
    let ahead = along(4, [0.0, 0.0, 21.0], [0.0, 1.0], 10);
    let ls = lines(vec![end.clone(), side.clone(), ahead.clone()]);
    let at = ctx([0.0, 0.0, 20.0], &players);
    assert_eq!(choose_next_line(&ls, &ls[&id(1)], &at), Some((id(4), 0)));
    // Every candidate rejected (both lead 90 deg off the forward): the chooser keeps index 0.
    let ls2 = lines(vec![end.clone(), along(5, [0.5, 0.0, 20.0], [1.0, 0.0], 10), along(6, [-0.5, 0.0, 20.0], [-1.0, 0.0], 10)]);
    assert_eq!(choose_next_line(&ls2, &ls2[&id(1)], &at), Some((id(5), 0)));
    // One candidate is taken without scoring, even a rejected one.
    let ls3 = lines(vec![end.clone(), along(7, [0.5, 0.0, 20.0], [1.0, 0.0], 10)]);
    assert_eq!(choose_next_line(&ls3, &ls3[&id(1)], &at), Some((id(7), 0)));
    // The cap: with max_candidates 1 only the first in id order is seen.
    let ls4 = lines(vec![end, side, ahead]);
    let one = BranchContext { chain: ChainConfig { max_candidates: 1, ..ChainConfig::retail() }, ..at };
    assert_eq!(choose_next_line(&ls4, &ls4[&id(1)], &one), Some((id(2), 0)));
}

#[test]
fn the_physics_tier_falls_back_to_the_nearest_free_start_at_any_distance_like_sub_82458968() {
    let players: [Vec3; 0] = [];
    let end = along(1, [0.0; 3], [0.0, 1.0], 21);
    // No start within 4 m of the end (0, 0, 20): line 3 starts 30 m away, line 2 12 m away.
    let far = along(3, [30.0, 0.0, 20.0], [0.0, 1.0], 10);
    let near = along(2, [12.0, 0.0, 20.0], [0.0, 1.0], 10);
    let ls = lines(vec![end, far, near]);
    let at = ctx([0.0, 0.0, 20.0], &players);
    assert_eq!(choose_next_line(&ls, &ls[&id(1)], &at), None, "off by default (the replay tier ends there)");
    let fb = BranchContext { chain: ChainConfig { fallback: true, ..ChainConfig::retail() }, ..at };
    assert_eq!(choose_next_line(&ls, &ls[&id(1)], &fb), Some((id(2), 0)));
    // No in-use skip and the skater's own line is a candidate (b715: `82458968` / `8246C7F8` exclude neither).
    let busy = [id(2)];
    let fb_busy = BranchContext { in_use: &busy, ..fb };
    assert_eq!(choose_next_line(&ls, &ls[&id(1)], &fb_busy), Some((id(2), 0)));
    let alone = lines(vec![along(1, [0.0; 3], [0.0, 1.0], 21)]);
    assert_eq!(choose_next_line(&alone, &alone[&id(1)], &fb), Some((id(1), 0)), "alone: back to its own line's start");
}

/// b701 R9 / R10: a simulated skater searches from its body point; with no hit the nearest invalid start wins over a
/// valid one at least 6 m away, a valid one nearer than 6 m wins, and with no valid line the nearest invalid.
#[test]
fn the_line_end_search_starts_at_the_body_and_falls_back_like_mode_1() {
    let players: [Vec3; 0] = [];
    let end = along(1, [0.0; 3], [0.0, 1.0], 21);
    // Line 2 starts 3.9 m from the body point (0, 0, 24) but 6 m from the last node (0, 0, 20).
    let ahead = along(2, [0.0, 0.0, 26.0], [0.0, 1.0], 10);
    let ls = lines(vec![end.clone(), ahead]);
    let at = ctx([0.0, 0.0, 20.0], &players);
    assert_eq!(choose_next_line(&ls, &ls[&id(1)], &at), None, "6 m from the last node");
    let body = BranchContext { body: Some(BodyGate { point: [0.0, 0.0, 22.2], may_branch: true, heading: None }), ..at };
    assert_eq!(choose_next_line(&ls, &ls[&id(1)], &body), Some((id(2), 0)), "3.8 m from the body point");
    // Fallback: line 3 (valid) 10 m away, line 5 (invalid) 8 m away.
    let valid = [id(3)];
    let fb = BranchContext { chain: ChainConfig { fallback: true, ..ChainConfig::retail() }, valid_lines: Some(&valid), ..at };
    let far = lines(vec![end.clone(), along(3, [10.0, 0.0, 20.0], [0.0, 1.0], 10), along(5, [8.0, 0.0, 20.0], [0.0, 1.0], 10)]);
    assert_eq!(choose_next_line(&far, &far[&id(1)], &fb), Some((id(5), 0)), "invalid nearer, valid >= 6 m");
    let near = lines(vec![end.clone(), along(3, [5.0, 0.0, 20.0], [0.0, 1.0], 10), along(5, [4.5, 0.0, 20.0], [0.0, 1.0], 10)]);
    assert_eq!(choose_next_line(&near, &near[&id(1)], &fb), Some((id(3), 0)), "valid under 6 m");
    let none_valid = lines(vec![end, along(5, [9.0, 0.0, 20.0], [0.0, 1.0], 10)]);
    assert_eq!(choose_next_line(&none_valid, &none_valid[&id(1)], &fb), Some((id(5), 0)), "no valid line");
}

/// b701 2d: a simulated cursor (body gate set) holds on the last node with nothing to chain to; the replay tier ends.
#[test]
fn a_simulated_cursor_holds_at_a_dead_end_and_the_replay_tier_finishes() {
    let players: [Vec3; 0] = [];
    // Its own start 9 m back, beyond the 4 m radius (its own line is a candidate, b715), and no fallback here.
    let ls = lines(vec![along(1, [0.0; 3], [0.0, 1.0], 10)]);
    let at = ctx([0.0, 0.0, 9.0], &players);
    let body = BranchContext { body: Some(BodyGate { point: [0.0, 0.0, 9.0], may_branch: true, heading: None }), ..at };
    let mut held = LineCursor::new(id(1), 0);
    let mut out = Vec::new();
    for _ in 0..400 {
        held.step(&ls, &mut Decider::Decide(body), &mut out);
    }
    assert!(!held.finished && !out.iter().any(|e| matches!(e, CursorEvent::Finished)));
    let frames = held.frames;
    held.step(&ls, &mut Decider::Decide(body), &mut out);
    assert_eq!(held.frames, frames, "no progress while held");
    let mut replay = LineCursor::new(id(1), 0);
    for _ in 0..400 {
        replay.step(&ls, &mut Decider::Decide(at), &mut out);
    }
    assert!(replay.finished);
}

#[test]
fn npc_skater_keeps_riding_a_seeded_line_network_for_minutes() {
    use crate::living_world::rng::Rng;
    // Seeded network: a square loop of 4 lines whose starts sit up to 1.5 m off the previous end
    // (inside retail's 4 m), plus seeded decoys. The skater rides 5 minutes without ever ending,
    // and two runs with the same seed are identical (host determinism).
    let build = |seed: u64| {
        let mut rng = Rng::new(seed);
        let mut v = Vec::new();
        let corners = [[0.0f32, 0.0], [40.0, 0.0], [40.0, 40.0], [0.0, 40.0]];
        for k in 0..4 {
            let (a, b) = (corners[k], corners[(k + 1) % 4]);
            let jitter = [rng.unit() * 2.0 - 1.0, rng.unit() * 2.0 - 1.0];
            let from = [a[0] + jitter[0], 0.0, a[1] + jitter[1]];
            let d = [b[0] - from[0], b[1] - from[2]];
            let len = (d[0] * d[0] + d[1] * d[1]).sqrt();
            v.push(along(10 + k as u8, from, [d[0] / len, d[1] / len], len as u32 + 1));
        }
        for k in 0..6u8 {
            let at = [5.0 + rng.unit() * 30.0, 0.0, 5.0 + rng.unit() * 30.0];
            v.push(along(40 + k, at, [1.0, 0.0], 8));
        }
        lines(v)
    };
    let players = [[20.0f32, 0.0, 20.0]];
    let ride = |ls: &BTreeMap<[u8; 16], ReplayLine>| {
        let mut c = LineCursor::spawn(ls, id(10), 0);
        let mut ev = Vec::new();
        for _ in 0..(5 * 60 * 60) {
            let s = c.sample(ls, 0.0).unwrap();
            let x = BranchContext { forward: s.velocity, ..ctx(s.position, &players) };
            c.step(ls, &mut Decider::Decide(x), &mut ev);
        }
        (c, ev)
    };
    for seed in [1u64, 7, 1234] {
        let ls = build(seed);
        let (c, ev) = ride(&ls);
        assert!(!c.finished, "seed {seed}");
        let hops = ev.iter().filter(|e| matches!(e, CursorEvent::Branch(_))).count();
        assert!(hops >= 20, "seed {seed}: {hops} hops");
        let (c2, ev2) = ride(&ls);
        assert_eq!((c2, ev2), (c, ev), "seed {seed}");
    }
}

#[test]
fn npc_fades_in_over_its_first_second_like_sub_825926f8() {
    use crate::living_world::leave_fade::{LeaveFade, LeaveFadeConfig};
    // Retail: `+1868 = 0` at spawn, opacity = clamp(t, 0, 1) at dt per tick (`sub_82594488`).
    let cfg = LeaveFadeConfig::retail();
    assert_eq!(cfg.fade_in_frames(), 60);
    let fade = LeaveFade::default();
    assert_eq!(fade.alpha(0, &cfg), 0.0);
    assert!((fade.alpha(15, &cfg) - 0.25).abs() < 1e-6);
    assert!((fade.alpha(30, &cfg) - 0.5).abs() < 1e-6);
    assert!((fade.alpha(59, &cfg) - 59.0 / 60.0).abs() < 1e-6);
    assert_eq!(fade.alpha(60, &cfg), 1.0);
    assert_eq!(fade.alpha(6000, &cfg), 1.0);
    // Monotonic, deterministic (a function of the spawn-relative frame only).
    let curve: Vec<f32> = (0..=60).map(|f| fade.alpha(f, &cfg)).collect();
    assert!(curve.windows(2).all(|w| w[1] > w[0]));
    // The fade in wins over a fade out started early, and no removal happens during it.
    let early = LeaveFade { started: Some(0) };
    assert!((early.alpha(30, &cfg) - 0.5).abs() < 1e-6);
    assert!(!early.should_despawn(5, &cfg));
    assert!(early.should_despawn(60, &cfg));
    // Data-driven: a mod's 0.5 s fade in, or none.
    let half = LeaveFadeConfig { fade_in_seconds: 0.5, ..cfg };
    assert_eq!(fade.alpha(15, &half), 0.5);
    assert_eq!(fade.alpha(30, &half), 1.0);
    let none = LeaveFadeConfig { fade_in_seconds: 0.0, ..cfg };
    assert_eq!(fade.alpha(0, &none), 1.0);
}

/// The puppet crossfade (fix 12) needs the phase it leaves and how long that phase has run; both
/// are a function of the cursor alone, so a client rebuilding the cursor gets the same blend.
#[test]
fn replay_cursor_keeps_the_previous_phase_for_the_puppet_crossfade() {
    let mut l = straight(1, [0.0; 3], 20, 2, 0.3);
    for n in 5..9 {
        l.nodes[n].flags = node_flags::AIRBORNE;
    }
    let ls = lines(vec![l]);
    let mut c = LineCursor::spawn(&ls, id(1), 0);
    let s = c.sample(&ls, 0.0).unwrap();
    assert_eq!((s.phase, s.previous_phase), (ReplayPhase::Rolling, None));
    let mut ev = Vec::new();
    // Node 5 is reached on frame 10: the air phase starts there, rolling ran 10 frames.
    c.advance(10, &ls, &mut Decider::Stay, &mut ev);
    let s = c.sample(&ls, 0.0).unwrap();
    assert_eq!((s.phase, s.phase_frames, s.previous_phase, s.previous_phase_frames), (ReplayPhase::Air, 0, Some(ReplayPhase::Rolling), 10));
    c.advance(3, &ls, &mut Decider::Stay, &mut ev);
    let s = c.sample(&ls, 0.0).unwrap();
    assert_eq!((s.phase_frames, s.previous_phase_frames), (3, 13));
    // Back to rolling at node 9 (frame 18): the outgoing clip is the air one, 8 frames in.
    c.advance(5, &ls, &mut Decider::Stay, &mut ev);
    let s = c.sample(&ls, 0.0).unwrap();
    assert_eq!((s.phase, s.phase_frames, s.previous_phase, s.previous_phase_frames), (ReplayPhase::Rolling, 0, Some(ReplayPhase::Air), 8));
    // Deterministic: a second cursor stepped the same way is identical.
    let mut d = LineCursor::spawn(&ls, id(1), 0);
    d.advance(18, &ls, &mut Decider::Stay, &mut Vec::new());
    assert_eq!(c, d);
}

/// The weight curve the puppet shares with the player's graph transitions (Blend82B96058):
/// linear progress, eased with 3w^2 - 2w^3 from 0.05 s up, linear below.
#[test]
fn npc_puppet_blend_weight_is_the_graph_transition_curve() {
    use crate::animation::playback_transition::transition_weight;
    let close = |a: f32, b: f32| (a - b).abs() < 1e-6;
    assert!(close(transition_weight(0.0, 0.2), 0.0));
    assert!(close(transition_weight(0.05, 0.2), 0.156_25));
    assert!(close(transition_weight(0.1, 0.2), 0.5));
    assert!(close(transition_weight(0.15, 0.2), 0.843_75));
    assert!(close(transition_weight(0.2, 0.2), 1.0));
    assert!(close(transition_weight(0.5, 0.2), 1.0));
    // Shorter than 0x3d4ccccd (0.05 s): linear.
    assert!(close(transition_weight(0.01, 0.04), 0.25));
    // 0 s = cut.
    assert!(close(transition_weight(0.0, 0.0), 1.0));
    // Monotonic, continuous at 60 Hz steps.
    let mut last = 0.0;
    for f in 0..=12 {
        let w = transition_weight(f as f32 / 60.0, 0.2);
        assert!(w >= last && w - last <= 0.16, "frame {f}: {last} -> {w}");
        last = w;
    }
}

/// Render a ride at `render_hz` the way `npc_skaters::present_pose` does (the cursor one tick
/// back, interpolated by the fixed-step fraction with the recorded branches) and return the
/// largest per-render-step root move (m), root turn (rad) and the clip-time steps (s).
fn render_ride(ls: &BTreeMap<[u8; 16], ReplayLine>, start: [u8; 16], records: Option<&[BranchRecord]>, blend: f32, seconds: f32, render_hz: f32) -> (f32, f32, Vec<f32>, Vec<BranchRecord>) {
    let players = [[20.0f32, 0.0, 20.0]];
    let mut c = LineCursor::spawn(ls, start, 0);
    c.switch_blend_seconds = blend;
    let mut prev = c.clone();
    let mut made: Vec<BranchRecord> = records.map(<[_]>::to_vec).unwrap_or_default();
    let (mut max_move, mut max_turn, mut times) = (0.0f32, 0.0f32, Vec::new());
    let mut last: Option<ReplaySample> = None;
    for k in 0..(seconds * render_hz) as u32 {
        let t = f64::from(k) / f64::from(render_hz) * RECORDING_HZ;
        let tick = (t + 1e-9).floor() as u64;
        while c.frames < tick && !c.finished {
            prev = c.clone();
            let mut ev = Vec::new();
            match records {
                Some(r) => c.step(ls, &mut Decider::Mirror(r, &[]), &mut ev),
                None => {
                    let s = c.sample(ls, 0.0).unwrap();
                    let x = BranchContext { forward: s.velocity, ..ctx(s.position, &players) };
                    c.step(ls, &mut Decider::Decide(x), &mut ev);
                }
            }
            made.extend(ev.into_iter().filter_map(|e| if let CursorEvent::Branch(b) = e { Some(b) } else { None }));
        }
        let s = if tick == 0 { c.sample(ls, 0.0) } else { prev.render_sample(ls, &made, &[], (t - tick as f64) as f32) }.unwrap();
        if let Some(l) = &last {
            let d = sub(s.position, l.position);
            max_move = max_move.max((d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt());
            let dot: f32 = (0..4).map(|i| s.skater[i] * l.skater[i]).sum();
            max_turn = max_turn.max(2.0 * dot.abs().min(1.0).acos());
            if s.phase == l.phase && tick >= 2 {
                times.push((s.phase_frames as f32 + s.sub_frame - l.phase_frames as f32 - l.sub_frame) / 60.0);
            }
        }
        last = Some(s);
    }
    (max_move, max_turn, times, made)
}

/// Fix 14 (jitter): branch and chain roots never jump at 144 Hz; a mirroring client draws the same.
#[test]
fn npc_skater_render_is_smooth_across_branches_and_chains() {
    let turned = [128, 128 + 33, 128, 128 + 123]; // about 30 deg about +Y
    let mut a = along(1, [0.0; 3], [0.0, 1.0], 31);
    a.groups.push(ReplayBranchGroup { node: 10, branches: vec![ReplayBranch { target: id(2), target_node: 10, weight: 1.0 }] });
    let mut b = along(2, [0.8, 0.0, 0.0], [0.0, 1.0], 31);
    for n in &mut b.nodes {
        n.skater = turned;
        n.board = turned;
    }
    // Line 3 starts 1.5 m beside line 2's end (inside the 4 m chain radius).
    let c3 = along(3, [2.3, 0.0, 30.0], [0.0, 1.0], 31);
    let ls = lines(vec![a, b, c3]);
    // Branch at node 10 (frame 100), chain at line 2's last node (frame 100 + 20 x 10).
    let all = [
        BranchRecord { frame: 100, from_line: id(1), from_node: 10, to_line: id(2), to_node: 10, respawn: false },
        BranchRecord { frame: 300, from_line: id(2), from_node: 30, to_line: id(3), to_node: 0, respawn: false },
    ];
    let speed_step = 6.0 / 144.0;
    let (moved, turned_by, times, _) = render_ride(&ls, id(1), Some(&all), SWITCH_BLEND_SECONDS, 6.0, 144.0);
    // Smoothstep over 0.2 s peaks at 1.5 x offset / 0.2 s: 1.5 m -> 0.078 m per 144 Hz step.
    assert!(moved < speed_step + 1.5 * 1.5 / 0.2 / 144.0 + 1e-3, "root step {moved} m");
    assert!(turned_by < 1.5 * 30f32.to_radians() / 0.2 / 144.0 + 1e-3, "root turn {turned_by} rad");
    // The clip time moves with the render clock (no 60 Hz stair steps), never backwards.
    assert!(times.iter().all(|d| (d - 1.0 / 144.0).abs() < 1e-4), "clip time steps {:?}", times.iter().fold((f32::MAX, 0.0f32), |m, d| (m.0.min(*d), m.1.max(*d))));
    // Without the switch blend (0 s = cut) the same ride jumps at the branch and the chain.
    let (cut, cut_turn, _, _) = render_ride(&ls, id(1), Some(&all), 0.0, 6.0, 144.0);
    assert!(cut > 0.7 && cut_turn > 0.4, "cut ride jumps: {cut} m {cut_turn} rad");
    // Same at any render rate and identical when mirrored (a pure function of cursor, records
    // and fraction).
    let (m60, ..) = render_ride(&ls, id(1), Some(&all), SWITCH_BLEND_SECONDS, 6.0, 60.0);
    assert!(m60 < 6.0 / 60.0 + 1.5 * 1.5 / 0.2 / 60.0 + 1e-3, "60 Hz step {m60}");
    assert_eq!(render_ride(&ls, id(1), Some(&all), SWITCH_BLEND_SECONDS, 6.0, 144.0).0, moved);
}

/// The seeded line network of fix 9 rendered at 144 Hz with the host's own decisions: every
/// chain (starts up to 1.5 m off the previous end) is blended, so no render step jumps.
#[test]
fn npc_skater_render_is_smooth_on_a_seeded_line_network() {
    use crate::living_world::rng::Rng;
    for seed in [1u64, 7, 1234] {
        let mut rng = Rng::new(seed);
        let mut v = Vec::new();
        let corners = [[0.0f32, 0.0], [40.0, 0.0], [40.0, 40.0], [0.0, 40.0]];
        for k in 0..4 {
            let (a, b) = (corners[k], corners[(k + 1) % 4]);
            let jitter = [rng.unit() * 2.0 - 1.0, rng.unit() * 2.0 - 1.0];
            let from = [a[0] + jitter[0], 0.0, a[1] + jitter[1]];
            let d = [b[0] - from[0], b[1] - from[2]];
            let len = (d[0] * d[0] + d[1] * d[1]).sqrt();
            v.push(along(10 + k as u8, from, [d[0] / len, d[1] / len], len as u32 + 1));
        }
        let ls = lines(v);
        let (moved, _, times, made) = render_ride(&ls, id(10), None, SWITCH_BLEND_SECONDS, 60.0, 144.0);
        assert!(made.len() >= 4, "seed {seed}: {} hops", made.len());
        // Off-end gap at most sqrt(2) + 1 m (jitter + node spacing).
        assert!(moved < 6.0 / 144.0 + 1.5 * 2.5 / 0.2 / 144.0, "seed {seed}: root step {moved} m");
        assert!(times.iter().all(|d| *d >= 0.0), "seed {seed}: clip time went back");
        let (cut, ..) = render_ride(&ls, id(10), Some(&made), 0.0, 60.0, 144.0);
        assert!(cut > moved, "seed {seed}: blend {moved} vs cut {cut}");
    }
}

/// Fix 16 rule as mod option (`keep_facing`, not retail) vs the retail recorded facing: docs/hails-additions/26c-living-world-npc-skaters.md#test-keep-facing-option
#[test]
fn npc_skater_keeps_its_facing_across_a_line_switch() {
    const BACKWARD: [u8; 4] = [128, 255, 128, 128]; // (0, 1, 0, 0): +Z turned to -Z.
    let mut fakie = along(2, [0.0, 0.0, 21.0], [0.0, 1.0], 31);
    for n in &mut fakie.nodes[..15] {
        n.skater = BACKWARD;
        n.board = BACKWARD;
    }
    let ls = lines(vec![along(1, [0.0; 3], [0.0, 1.0], 21), fakie]);
    let players: [Vec3; 0] = [];
    let forward_z = |s: &ReplaySample| rotate(s.skater, [0.0, 0.0, 1.0])[2];
    let board_z = |s: &ReplaySample| rotate(s.board, [0.0, 0.0, 1.0])[2];
    let run = |keep: bool, frames: u32| {
        let chain = ChainConfig { keep_facing: keep, ..ChainConfig::retail() };
        let mut c = LineCursor::spawn(&ls, id(1), 0);
        c.keep_facing = chain.keep_facing;
        let mut ev = Vec::new();
        for _ in 0..frames {
            let s = c.sample(&ls, 0.0).unwrap();
            let x = BranchContext { forward: s.velocity, chain, ..ctx(s.position, &players) };
            c.step(&ls, &mut Decider::Decide(x), &mut ev);
        }
        (c, ev)
    };
    assert!(!ChainConfig::retail().keep_facing && !LineCursor::new(id(1), 0).keep_facing, "the fix 16 option is off by default");
    // Before the chain: forward. Right after it (blend over) on the fakie stretch: still forward.
    let (before, _) = run(true, 150);
    assert!(forward_z(&before.sample(&ls, 0.0).unwrap()) > 0.99);
    let (after, ev) = run(true, 260);
    assert_eq!(after.line, id(2));
    assert!(after.facing_flipped);
    let s = after.sample(&ls, 0.0).unwrap();
    assert!(forward_z(&s) > 0.99 && board_z(&s) > 0.99, "kept facing: {:?}", s.skater);
    // Facing +Z on every frame through the chain and its blend (no spin), mirrored on a client.
    let (mut c, mut last) = (LineCursor::spawn(&ls, id(1), 0), forward_z(&before.sample(&ls, 0.0).unwrap()));
    c.keep_facing = true;
    let records: Vec<BranchRecord> = ev.iter().filter_map(|e| if let CursorEvent::Branch(b) = e { Some(b.clone()) } else { None }).collect();
    for _ in 0..260 {
        c.step(&ls, &mut Decider::Mirror(&records, &[]), &mut Vec::new());
        let z = forward_z(&c.sample(&ls, 0.0).unwrap());
        assert!(z > 0.99, "turned at frame {}: {z} (was {last})", c.frames);
        last = z;
    }
    assert_eq!(c, after, "a client mirroring the chain derives the same facing");
    // The recorded revert on line 2 (node 15) still turns the skater, now into fakie.
    let (late, _) = run(true, 420);
    assert!(late.node > 16 && forward_z(&late.sample(&ls, 0.0).unwrap()) < -0.99);
    // Retail (option off): the recorded facing (turned round at the chain by the root blend).
    let (raw, _) = run(false, 260);
    assert!(!raw.facing_flipped && forward_z(&raw.sample(&ls, 0.0).unwrap()) < -0.99);
    // The flip is retail's board-flip turn: +Z to -Z about the frame's own up, up kept.
    let q = turn_about_up(decode_orientation(IDENTITY));
    assert!((rotate(q, [0.0, 0.0, 1.0])[2] + 1.0).abs() < 1e-5 && (rotate(q, [0.0, 1.0, 0.0])[1] - 1.0).abs() < 1e-5);
}

/// Fix 23 under the retail rule: switches leave the latched flip alone; fix 16 reproduces the bug: docs/hails-additions/26c-living-world-npc-skaters.md#test-retail-facing-over-switches
#[test]
fn npc_skater_never_rides_a_forward_recorded_line_backwards_across_switches() {
    const BACKWARD: [u8; 4] = [128, 255, 128, 128];
    let mut fakie = along(2, [0.0, 0.0, 21.0], [0.0, 1.0], 31);
    for n in &mut fakie.nodes[..15] {
        n.skater = BACKWARD;
        n.board = BACKWARD;
    }
    let ls = lines(vec![along(1, [0.0; 3], [0.0, 1.0], 21), fakie, along(3, [0.0, 0.0, 52.0], [0.0, 1.0], 31)]);
    let players: [Vec3; 0] = [];
    let blend_frames = (SWITCH_BLEND_SECONDS * 60.0).ceil() as u64 + 1;
    // (frame, line, blend settled, facing check) per frame plus the branch records.
    let ride = |keep: bool| {
        let chain = ChainConfig { keep_facing: keep, ..ChainConfig::retail() };
        let mut c = LineCursor::spawn(&ls, id(1), 0);
        c.keep_facing = keep;
        let (mut ev, mut seen) = (Vec::new(), Vec::new());
        for _ in 0..800 {
            let s = c.sample(&ls, 0.0).unwrap();
            let x = BranchContext { forward: s.velocity, chain, ..ctx(s.position, &players) };
            c.step(&ls, &mut Decider::Decide(x), &mut ev);
            if c.finished {
                break;
            }
            let s = c.sample(&ls, 0.0).unwrap();
            seen.push((c.frames, c.line, c.switch.is_none(), facing_check(&ls[&c.line], &s)));
        }
        let records: Vec<BranchRecord> = ev.iter().filter_map(|e| if let CursorEvent::Branch(b) = e { Some(b.clone()) } else { None }).collect();
        (c, seen, records)
    };
    let (host, seen, records) = ride(false);
    assert_eq!(records.iter().map(|r| r.to_line).collect::<Vec<_>>(), vec![id(2), id(3)], "two chains");
    let mut on_line3 = 0;
    for (frame, line, settled, check) in &seen {
        let Some(check) = check else { continue };
        if *settled && !check.recorded_fakie {
            assert!(!check.backwards && check.angle < 0.1, "frame {frame} line {}: rides backwards ({} rad)", line[0], check.angle);
        }
        if *line == id(3) && *settled {
            on_line3 += 1;
            assert!(check.angle < 0.01, "line 3 is ridden forward");
        }
    }
    assert!(on_line3 > 200 && !host.facing_flipped && !host.flip, "line 3 frames {on_line3}");
    // Inside the recorded fakie stretch the line's own frame opposes travel (retail target frame).
    assert!(seen.iter().any(|(_, l, settled, c)| *l == id(2) && *settled && c.is_some_and(|c| c.recorded_fakie && c.backwards)));
    // Every switch blend settles within the blend time.
    for r in &records {
        assert!(seen.iter().any(|(f, _, settled, _)| *f == r.frame + blend_frames && *settled), "blend after frame {}", r.frame);
    }
    // A client mirroring the records draws the same frames.
    let mut client = LineCursor::spawn(&ls, id(1), 0);
    for (frame, line, _, check) in &seen {
        client.step(&ls, &mut Decider::Mirror(&records, &[]), &mut Vec::new());
        assert_eq!((client.frames, client.line), (*frame, *line));
        assert_eq!(facing_check(&ls[&client.line], &client.sample(&ls, 0.0).unwrap()), *check);
    }
    // The fix 16 option (mod only) reproduces the reported bug: backwards on forward-recorded nodes.
    let (_, old, _) = ride(true);
    let bad = old.iter().filter(|(_, _, settled, c)| *settled && c.is_some_and(|c| c.backwards && !c.recorded_fakie)).count();
    assert!(bad > 300, "fix 16 rode {bad} forward-recorded frames backwards");
    assert!(old.iter().filter(|(_, l, settled, c)| *l == id(3) && *settled && c.is_some_and(|c| c.backwards)).count() > 200);
}

/// The retail path frame ([code] `sub_82453A58`): the node's board orientation, turned 180 deg
/// about its up axis (X and Z rows negated) when `m_IsBoardFlipped` (flags bit 0) is set.
#[test]
fn path_frame_turns_the_board_when_the_node_is_board_flipped_like_sub_82453a58() {
    let mut n = straight(1, [0.0; 3], 2, 10, 1.0).nodes[1].clone();
    let yaw90: [u8; 4] = [128, 128 + 90, 128, 128 + 90]; // ~(0, 0.707, 0, 0.707): +Z to +X.
    n.board = yaw90;
    n.skater = IDENTITY;
    let f = rotate(path_frame(&n), [0.0, 0.0, 1.0]);
    assert!((f[0] - 1.0).abs() < 1e-3, "board frame, flag clear: {f:?}");
    n.flags = node_flags::BOARD_FLIPPED | node_flags::CROUCHED;
    let f = rotate(path_frame(&n), [0.0, 0.0, 1.0]);
    let up = rotate(path_frame(&n), [0.0, 1.0, 0.0]);
    assert!((f[0] + 1.0).abs() < 1e-3 && (up[1] - 1.0).abs() < 1e-3, "flipped: {f:?} up {up:?}");
    // Other flag bits do not turn it.
    n.flags = node_flags::CROUCHED | node_flags::AIRBORNE;
    assert!((rotate(path_frame(&n), [0.0, 0.0, 1.0])[0] - 1.0).abs() < 1e-3);
}

/// Fix 23 rule as the NOT RETAIL `PerNode` mod option: per-node facing of the path frame: docs/hails-additions/26c-living-world-npc-skaters.md#test-per-node-facing
#[test]
fn drawn_skater_faces_the_retail_path_frame_direction() {
    const BACKWARD: [u8; 4] = [128, 255, 128, 128];
    let fz = |q: [f32; 4]| rotate(q, [0.0, 0.0, 1.0])[2];
    let mut l = along(1, [0.0; 3], [0.0, 1.0], 40);
    // Nodes 5..15: switch stance (skater frame turned, board forward).
    for n in &mut l.nodes[5..15] {
        n.skater = BACKWARD;
    }
    // Nodes 15..20: board turned round and flagged board-flipped, skater forward.
    for n in &mut l.nodes[15..20] {
        n.board = BACKWARD;
        n.flags = node_flags::BOARD_FLIPPED;
    }
    // Nodes 20..24: airborne, the board spins (shove-it) and the skater is turned round in the air.
    for (k, n) in l.nodes[20..24].iter_mut().enumerate() {
        n.flags = node_flags::AIRBORNE;
        n.skater = BACKWARD;
        n.board = if k % 2 == 0 { BACKWARD } else { IDENTITY };
    }
    // Nodes 30..35: recorded fakie (skater and board frame against travel, no flag).
    for n in &mut l.nodes[30..35] {
        n.skater = BACKWARD;
        n.board = BACKWARD;
    }
    for i in 0..30 {
        if (20..24).contains(&i) {
            continue;
        }
        assert!(fz(drawn_skater(&l, i)) > 0.99, "node {i} drawn forward");
    }
    // Air after a board-flipped grounded node 19 (skater forward there): the switch skater frame
    // in the air is turned like node 19's decision (none), so it is drawn as recorded.
    assert!(fz(drawn_skater(&l, 21)) < -0.99 && fz(drawn_skater(&l, 22)) < -0.99, "air keeps the grounded turn");
    for i in 30..35 {
        assert!(fz(drawn_skater(&l, i)) < -0.99, "node {i} recorded fakie is drawn fakie");
    }
    // Raw skater frame unchanged where it agrees with the path frame.
    assert_eq!(drawn_skater(&l, 2), decode_orientation(IDENTITY));
    let ls = lines(vec![l.clone()]);
    let mut c = LineCursor::spawn(&ls, id(1), 0);
    assert_eq!(c.facing_rule, FacingRule::RidingEntry, "the default (retail)");
    c.facing_rule = FacingRule::PerNode;
    let mut retail = LineCursor::spawn(&ls, id(1), 0);
    retail.facing_rule = FacingRule::RidingEntry;
    let mut retail_backwards = 0;
    while c.node < 19 {
        c.step(&ls, &mut Decider::Stay, &mut Vec::new());
        let s = c.sample(&ls, 0.0).unwrap();
        let check = facing_check(&l, &s).unwrap();
        assert!(!check.backwards && !check.recorded_fakie, "node {} drawn backwards", c.node);
        // The retail rule (spawned forward, flip clear) draws the recorded switch stance as
        // recorded: body against travel (the puppet has no stance mirror, NOT RETAIL YET).
        retail.step(&ls, &mut Decider::Stay, &mut Vec::new());
        retail_backwards += usize::from(facing_check(&l, &retail.sample(&ls, 0.0).unwrap()).unwrap().backwards);
    }
    assert!(!retail.flip && retail_backwards > 50, "{retail_backwards}");
}

/// Retail facing rule: spawn latch (8245C548 -> 82453C58, 8246A700, 8246B358; b354): docs/hails-additions/26c-living-world-npc-skaters.md#test-spawn-latch
#[test]
fn retail_flip_latches_at_spawn_against_the_node_world_frame() {
    const BACKWARD: [u8; 4] = [128, 255, 128, 128];
    let fz = |q: [f32; 4]| rotate(q, [0.0, 0.0, 1.0])[2];
    let mut l = along(1, [0.0; 3], [0.0, 1.0], 30);
    for n in &mut l.nodes[..10] {
        n.skater = BACKWARD;
    }
    let ls = lines(vec![l.clone()]);
    let mut c = LineCursor::spawn(&ls, id(1), 0);
    c.facing_rule = FacingRule::RidingEntry;
    assert!(c.flip && !c.airborne);
    assert!(flip_test(&l.nodes[0], node_world_frame(&l.nodes[0])));
    let s = c.sample(&ls, 0.0).unwrap();
    assert_eq!(s.skater, turn_about_up(target_skater(&l, 0, 1, 0.0)));
    assert!(fz(s.skater) > 0.99, "switch stance drawn on the board's side");
    // Same line spawned on a forward node: no flip.
    assert!(!LineCursor::spawn(&ls, id(1), 12).flip);
    // Spawned off board: nothing latched. In the air: latched against the spawn frame.
    let mut m = l.clone();
    m.nodes[0].flags = node_flags::OFF_BOARD;
    let c = LineCursor::spawn(&lines(vec![m]), id(1), 0);
    assert!(!c.flip && !c.airborne);
    let mut m = l.clone();
    m.nodes[0].flags = node_flags::AIRBORNE;
    let c = LineCursor::spawn(&lines(vec![m.clone()]), id(1), 0);
    assert!(c.airborne);
    assert_eq!(c.flip, flip_test(&m.nodes[0], node_world_frame(&m.nodes[0])));
    // The flip holds while riding: the recorded revert at node 10 turns the drawn body round with
    // it (now against travel), no per-node fold.
    let mut c = LineCursor::spawn(&ls, id(1), 0);
    c.facing_rule = FacingRule::RidingEntry;
    c.advance(12 * 10, &ls, &mut Decider::Stay, &mut Vec::new());
    assert!(c.flip && fz(c.sample(&ls, 0.0).unwrap().skater) < -0.99);
}

/// Retail facing rule: the flip holds across switches and relatches only at takeoff (b354).
#[test]
fn retail_flip_is_held_across_a_switch_and_relatched_at_takeoff() {
    const BACKWARD: [u8; 4] = [128, 255, 128, 128];
    let fz = |q: [f32; 4]| rotate(q, [0.0, 0.0, 1.0])[2];
    let players: [Vec3; 0] = [];
    let ride = |ls: &BTreeMap<[u8; 16], ReplayLine>, frames: u32| {
        let mut c = LineCursor::spawn(ls, id(1), 0);
        c.facing_rule = FacingRule::RidingEntry;
        let (mut ev, mut flips) = (Vec::new(), Vec::new());
        for _ in 0..frames {
            let s = c.sample(ls, 0.0).unwrap();
            let x = BranchContext { forward: s.velocity, ..ctx(s.position, &players) };
            c.step(ls, &mut Decider::Decide(x), &mut ev);
            flips.push((c.frames, c.line, c.node, c.flip));
        }
        let records: Vec<BranchRecord> = ev.iter().filter_map(|e| if let CursorEvent::Branch(b) = e { Some(b.clone()) } else { None }).collect();
        (c, flips, records)
    };
    // Held: line 1 switch stance throughout (flip set at spawn), chain on the ground onto line 2
    // recorded forward. The flip stays; line 2 is drawn turned (against travel), as retail's target.
    let mut a = along(1, [0.0; 3], [0.0, 1.0], 21);
    a.nodes.iter_mut().for_each(|n| n.skater = BACKWARD);
    let ls = lines(vec![a.clone(), along(2, [0.0, 0.0, 21.0], [0.0, 1.0], 31)]);
    let (c, flips, records) = ride(&ls, 260);
    assert_eq!((c.line, records.len()), (id(2), 1));
    assert!(flips.iter().all(|f| f.3), "held across the switch");
    assert_eq!(c.sample(&ls, 0.0).unwrap().skater, turn_about_up(target_skater(&ls[&id(2)], c.node as usize, c.node as usize + 1, c.frame_in_segment as f32 / 10.0)));
    assert!(fz(c.sample(&ls, 0.0).unwrap().skater) < -0.99);
    // Relatched at takeoff: chain onto an airborne fakie start; the flip sets and the landing keeps it.
    let air_line = |frames: u8| {
        let mut l = along(3, [0.0, 0.0, 21.0], [0.0, 1.0], 31);
        for (k, n) in l.nodes.iter_mut().enumerate() {
            n.skater = BACKWARD;
            n.board = BACKWARD;
            if k < 3 {
                n.flags = node_flags::AIRBORNE;
            }
            if (1..=3).contains(&k) {
                n.frames = frames;
            }
        }
        l
    };
    for frames in [1u8, 20u8] {
        let ls = lines(vec![along(1, [0.0; 3], [0.0, 1.0], 21), air_line(frames)]);
        let (c, flips, records) = ride(&ls, 300);
        assert_eq!(c.line, id(3));
        let takeoff = flips.iter().find(|f| f.1 == id(3)).unwrap();
        assert!(takeoff.3, "latched at takeoff ({frames} frames per air node)");
        let landed = flips.iter().find(|f| f.1 == id(3) && f.2 >= 3).unwrap();
        assert!(landed.3 && c.flip, "held after the landing");
        assert!(fz(c.sample(&ls, 0.0).unwrap().skater) > 0.99, "drawn on the body's side");
        // A client mirroring the records derives the same flip on every frame.
        let mut m = LineCursor::spawn(&ls, id(1), 0);
        m.facing_rule = FacingRule::RidingEntry;
        for f in &flips {
            m.step(&ls, &mut Decider::Mirror(&records, &[]), &mut Vec::new());
            assert_eq!((m.frames, m.flip), (f.0, f.3));
        }
    }
}

/// Slerp with shortest-arc sign selection like [code] `sub_82454B28`, and the retail AI steer
/// ramp ([code] `sub_82471188`, `ai_skater` defaults 2 / 10 deg): data for the simulated tier.
#[test]
fn slerp_takes_the_short_arc_and_the_steer_ramp_matches_the_ai_skater_defaults() {
    let id_q = [0.0, 0.0, 0.0, 1.0];
    let y90 = [0.0, std::f32::consts::FRAC_1_SQRT_2, 0.0, std::f32::consts::FRAC_1_SQRT_2];
    let neg = y90.map(|c: f32| -c);
    let h = slerp(id_q, neg, 0.5);
    let f = rotate(h, [0.0, 0.0, 1.0]);
    assert!((f[0].atan2(f[2]).to_degrees() - 45.0).abs() < 1e-3, "{f:?}");
    assert_eq!(slerp(id_q, y90, 0.0), id_q);
    let c = ChainConfig::retail();
    assert_eq!((c.steer_dead_zone_deg, c.steer_full_deg, c.facing_rule), (2.0, 10.0, FacingRule::RidingEntry));
    let st = |deg: f32| steer_input(deg.to_radians(), c.steer_dead_zone_deg, c.steer_full_deg);
    assert_eq!(st(1.5), 0.0);
    assert!((st(6.0) + 0.5).abs() < 1e-5 && (st(-6.0) - 0.5).abs() < 1e-5);
    assert_eq!((st(10.0), st(90.0), st(-170.0)), (-1.0, -1.0, 1.0));
    assert_eq!(FacingRule::NAMES.map(|n| FacingRule::from_name(n).unwrap().name()), FacingRule::NAMES);
}

/// `NPC_SKATER_BACKWARDS` check: heading vs velocity yaw, the diagnostic 135 deg threshold, no
/// verdict below 1 m/s, and whether the line itself was recorded fakie there.
#[test]
fn facing_check_flags_heading_against_velocity() {
    const BACKWARD: [u8; 4] = [128, 255, 128, 128];
    let mut l = along(1, [0.0; 3], [0.0, 1.0], 10);
    let ls = lines(vec![l.clone()]);
    let mut s = LineCursor::spawn(&ls, id(1), 0).sample(&ls, 0.0).unwrap();
    s.velocity = [0.0, 0.0, 6.0];
    let c = facing_check(&l, &s).unwrap();
    assert!(!c.backwards && c.angle.abs() < 1e-5 && !c.recorded_fakie && c.heading_yaw.abs() < 1e-5);
    // Drawn turned round, line recorded forward: backwards, not recorded fakie.
    s.skater = turn_about_up(s.skater);
    let c = facing_check(&l, &s).unwrap();
    assert!(c.backwards && !c.recorded_fakie && (c.angle - std::f32::consts::PI).abs() < 1e-3);
    // Recorded fakie node (board frame opposes travel).
    l.nodes[0].board = BACKWARD;
    assert!(facing_check(&l, &s).unwrap().recorded_fakie);
    // 120 deg off: under the 135 deg threshold.
    s.velocity = [6.0 * 120f32.to_radians().sin(), 0.0, 6.0 * 120f32.to_radians().cos()];
    s.skater = decode_orientation(IDENTITY);
    assert!(!facing_check(&l, &s).unwrap().backwards);
    // Too slow to judge.
    s.velocity = [0.0, 0.0, 0.5];
    assert_eq!(facing_check(&l, &s), None);
}

/// Fix 21: the cursor keeps the recent phases (newest first) with the recorded trick of their
/// span, so the puppet can nest a crossfade inside a running one and play the trick's clips.
#[test]
fn replay_cursor_keeps_a_phase_history_with_the_span_trick() {
    use super::{PhaseEntry, ReplayJump};
    let mut l = straight(1, [0.0; 3], 40, 2, 0.3);
    // Node 5: START_TRICK on the ground with a kickflip slot (EScorableID 96); air from node 7;
    // END_TRICK at node 10; landed at node 14.
    l.jumps.push(ReplayJump { start_position: [0.0; 3], start_velocity: [0.0; 3], offset: [0.0; 3], trick: 96, spins: 0, flags: 0 });
    l.nodes[5].event = node_events::START_TRICK;
    l.nodes[5].jump = Some(0);
    for n in 7..14 {
        l.nodes[n].flags = node_flags::AIRBORNE;
    }
    l.nodes[10].event = node_events::END_TRICK;
    let ls = lines(vec![l]);
    assert_eq!(ls[&id(1)].open_trick_at(6), Some(96));
    assert_eq!(ls[&id(1)].open_trick_at(10), None);
    let mut c = LineCursor::spawn(&ls, id(1), 0);
    c.advance(29, &ls, &mut Decider::Stay, &mut Vec::new());
    let h: Vec<(PhaseEntry, u64)> = c.phase_history().collect();
    // `trick_since` = the frame the span's START_TRICK was crossed (kept after the span).
    let e = |phase, since, trick, trick_since| PhaseEntry { phase, since, trick, trick_since };
    assert_eq!(
        h,
        vec![
            (e(ReplayPhase::Rolling, 28, -1, 10), 1),
            (e(ReplayPhase::Air, 20, -1, 10), 9),
            (e(ReplayPhase::AirTrick, 14, 96, 10), 15),
            (e(ReplayPhase::GroundTrick, 10, 96, 10), 19),
            (e(ReplayPhase::Rolling, 0, -1, 0), 29),
        ]
    );
    // Spawned inside the span: the trick comes from the span's START_TRICK node.
    let s = LineCursor::spawn(&ls, id(1), 8);
    assert_eq!(s.phase_history().next().map(|(e, _)| (e.phase, e.trick)), Some((ReplayPhase::AirTrick, 96)));
    // Deterministic.
    let mut d = LineCursor::spawn(&ls, id(1), 0);
    d.advance(29, &ls, &mut Decider::Stay, &mut Vec::new());
    assert_eq!(c, d);
}

/// Retail riding-fakie bit on the drawn body (82BB2330): set, air clears, landing sets; channel fades 0.3 s.
#[test]
fn retail_fakie_bit_follows_the_drawn_body_against_travel() {
    const BACKWARD: [u8; 4] = [128, 255, 128, 128];
    let mut l = along(1, [0.0; 3], [0.0, 1.0], 80);
    for n in &mut l.nodes[..10] {
        n.skater = BACKWARD;
    }
    for n in &mut l.nodes[40..45] {
        n.flags = node_flags::AIRBORNE;
    }
    let ls = lines(vec![l]);
    let run = |rule: FacingRule, settings: crate::animation::riding_fakie::Settings| {
        let mut c = LineCursor::spawn(&ls, id(1), 0);
        c.facing_rule = rule;
        c.fakie_settings = settings;
        let mut bits = Vec::new();
        for _ in 0..700 {
            c.step(&ls, &mut Decider::Stay, &mut Vec::new());
            let s = c.sample(&ls, 0.0).unwrap();
            assert_eq!(s.fakie, c.fakie);
            bits.push((c.fakie, c.fakie_channel_weight(0.0), s.phase));
        }
        bits
    };
    let retail = run(FacingRule::RidingEntry, retail::FAKIE);
    // Node 10 (frame 100) turns the drawn body against travel, after the 1 s spawn window.
    let first = retail.iter().position(|b| b.0).unwrap();
    assert!((90..=100).contains(&first), "set when the body turns against travel ({first})");
    // With a longer spawn window the bit waits for its end.
    let late = run(FacingRule::RidingEntry, crate::animation::riding_fakie::Settings { after_teleport_seconds: 3.0, ..retail::FAKIE });
    let late_first = late.iter().position(|b| b.0).unwrap();
    assert!((180..=182).contains(&late_first), "no fakie inside the spawn window ({late_first})");
    assert!((retail[first + 9].1 - 0.5).abs() < 0.02 && retail[first + 18].1 == 1.0, "0.3 s fade in");
    let air = retail.iter().position(|b| b.2 == ReplayPhase::Air).unwrap();
    assert!(!retail[air].0, "the air clears the bit");
    assert!(retail[air + 1].1 < 1.0 && retail[air + 18].1 == 0.0, "0.3 s fade out");
    let landed = retail.iter().skip(air).position(|b| b.2 == ReplayPhase::Rolling).unwrap() + air;
    assert!(retail[landed].0, "set again on the ground after the landing (flip relatched against the body)");
    assert_eq!(run(FacingRule::RidingEntry, retail::FAKIE), retail, "deterministic");
    assert!(run(FacingRule::PerNode, retail::FAKIE).iter().all(|b| !b.0), "drawn forward: never fakie");
    let off = crate::animation::riding_fakie::Settings { high_speed: 1000.0, low_speed: 1000.0, ..retail::FAKIE };
    assert!(run(FacingRule::RidingEntry, off).iter().all(|b| !b.0), "a mod can turn the fakie drawing off");
}

#[test]
fn steering_target_at_cursor_b_matches_the_cursor_pose_like_sub_8246de38() {
    // The record target is built from cursor B (node, fraction); the replay cursor at the same place gives the same pose.
    let ls = lines(vec![straight(1, [10.0, 2.0, 0.0], 8, 4, 0.5)]);
    let line = &ls[&id(1)];
    let mut c = LineCursor::spawn(&ls, id(1), 0);
    c.advance(6, &ls, &mut Decider::Stay, &mut Vec::new());
    let from_cursor = c.line_target(&ls).unwrap();
    let at_b = line_target_at(line, (1, 0.5)).unwrap();
    assert_eq!(from_cursor.position, at_b.position);
    assert_eq!(from_cursor.step, at_b.step);
    assert!((at_b.position[2] - 0.75).abs() < 1e-5, "{:?}", at_b.position);
    // B behind the cursor (after a rejoin): the target moves back with it.
    let behind = line_target_at(line, (0, 0.0)).unwrap();
    assert!((behind.position[2] - 0.0).abs() < 1e-5);
    // The last node gives its own pose; past the end there is none.
    let last = line_target_at(line, (7, 0.9)).unwrap();
    assert!((last.position[2] - 3.5).abs() < 1e-5);
    assert!(line_target_at(line, (8, 0.0)).is_none());
}

/// b674: a simulated skater (`body`) takes no branch while retail's gate is shut (airborne, a perform / grind trick),
/// and is scored from its body point when it is open.
#[test]
fn a_simulated_skater_branches_only_through_the_gate() {
    let ls = branching();
    let players = [[4.85, 0.0, 8.4]];
    let run = |gate: Option<bool>| {
        let mut c = LineCursor::spawn(&ls, id(1), 0);
        let mut ev = Vec::new();
        for _ in 0..30 {
            let s = c.sample(&ls, 0.0).unwrap();
            let body = gate.map(|may_branch| BodyGate { point: s.position, may_branch, heading: None });
            let ctx = BranchContext { position: s.position, speed: 9.0, body, ..ctx(s.position, &players) };
            c.step(&ls, &mut Decider::Decide(ctx), &mut ev);
        }
        ev.iter().filter(|e| matches!(e, CursorEvent::Branch(_))).count()
    };
    assert_eq!(run(None), 1, "the replay tier branches as before");
    assert_eq!(run(Some(true)), 1, "an open gate branches from the body");
    assert_eq!(run(Some(false)), 0, "a shut gate keeps the line");
}

/// Stage 3 (b701 2e, b712): the simulated skater's line end chains on the trigger tick from mid-segment, then the
/// 4.5 m lost test runs on the line it is on; a client applies the host's mid-segment record at the top of `step`.
mod line_end_stage_3 {
    use super::*;
    use crate::living_world::path_follow::{PathFollow, PathTuning};

    fn body_ctx<'a>(pos: Vec3, players: &'a [Vec3]) -> BranchContext<'a> {
        let at = ctx(pos, players);
        BranchContext { body: Some(BodyGate { point: pos, may_branch: true, heading: None }), chain: ChainConfig { fallback: true, ..ChainConfig::retail() }, ..at }
    }

    /// Line 1 (nodes 1 m apart on +z, 0..9), the cursor 5 frames into node 8, the follower on the path at A = (8, 0.85).
    fn at_the_trigger(ls: &BTreeMap<[u8; 16], ReplayLine>) -> (LineCursor, PathFollow, Vec3) {
        let mut c = LineCursor::new(id(1), 8);
        for _ in 0..5 {
            c.step(ls, &mut Decider::Stay, &mut Vec::new());
        }
        let t = PathTuning::default();
        let f = PathFollow { a: (8, 0.85), b: (8, 0.85), on_path: true, search_start: 8, ..PathFollow::start(8, &t) };
        (c, f, [0.0, 0.0, 8.85])
    }

    #[test]
    fn a_simulated_line_end_chains_on_the_trigger_tick_from_mid_segment() {
        let players: [Vec3; 0] = [];
        let ls = lines(vec![along(1, [0.0; 3], [0.0, 1.0], 10), along(2, [0.0, 0.0, 11.0], [0.0, 1.0], 10)]);
        let (mut c, mut f, pos) = at_the_trigger(&ls);
        let t = PathTuning::default();
        assert!(f.search(&ls[&id(1)], pos, 0.0, &t), "past 0.8 of the second-last segment");
        let mut out = Vec::new();
        assert_eq!(c.line_end_chain(&ls, &body_ctx(pos, &players), &mut out), Some((id(2), 0)));
        assert_eq!(out, vec![CursorEvent::Branch(BranchRecord { frame: 5, from_line: id(1), from_node: 8, to_line: id(2), to_node: 0, respawn: false })]);
        assert!(c.line == id(2) && c.node == 0 && c.frame_in_segment == 0 && !c.finished);
        let mut f = f.switch_to(0);
        f.line_end_lost(&ls[&id(2)], pos, 0.0, &t);
        assert!(!f.on_path && f.search_start == 0 && !f.lost, "2.15 m from the new line: not lost");
    }

    #[test]
    fn a_far_fallback_chain_raises_the_line_end_lost() {
        let players: [Vec3; 0] = [];
        // Line 2 starts 7.15 m away (beyond 4 m), nearer than the skater's own start (8.85 m back).
        let ls = lines(vec![along(1, [0.0; 3], [0.0, 1.0], 10), along(2, [0.0, 0.0, 16.0], [0.0, 1.0], 10)]);
        let t = PathTuning::default();
        for (speed, lost) in [(0.0, true), (14.1, false)] {
            let (mut c, f, pos) = at_the_trigger(&ls);
            assert_eq!(c.line_end_chain(&ls, &body_ctx(pos, &players), &mut Vec::new()), Some((id(2), 0)), "mode 1 fallback beyond the radius");
            let mut f = f.switch_to(0);
            f.line_end_lost(&ls[&id(2)], pos, speed, &t);
            assert_eq!(f.lost, lost, "7.15 m away at {speed} m/s");
        }
    }

    #[test]
    fn alone_the_fallback_takes_its_own_lines_start_and_tests_lost_there() {
        let players: [Vec3; 0] = [];
        let ls = lines(vec![along(1, [0.0; 3], [0.0, 1.0], 10)]);
        let (mut c, f, _) = at_the_trigger(&ls);
        let mut out = Vec::new();
        // Retail excludes no line from the mode 1 search, its own neither (b715): the start 10 m back.
        assert_eq!(c.line_end_chain(&ls, &body_ctx([5.0, 0.0, 9.0], &players), &mut out), Some((id(1), 0)));
        assert!(c.line == id(1) && c.node == 0 && out.len() == 1);
        let mut f = f.switch_to(0);
        f.line_end_lost(&ls[&id(1)], [5.0, 0.0, 9.0], 0.0, &PathTuning::default());
        assert!(f.lost, "10.3 m from its own start: lost at 4.5 m");
    }

    #[test]
    fn two_skaters_at_one_line_end_take_different_lines_in_id_order() {
        let players: [Vec3; 0] = [];
        let ls = lines(vec![along(1, [0.0; 3], [0.0, 1.0], 10), along(2, [0.0, 0.0, 10.5], [0.0, 1.0], 10), along(3, [1.5, 0.0, 10.0], [0.0, 1.0], 10)]);
        let (mut first, _, pos) = at_the_trigger(&ls);
        let (mut second, ..) = at_the_trigger(&ls);
        let a = first.line_end_chain(&ls, &body_ctx(pos, &players), &mut Vec::new()).expect("a line");
        // The first skater's own entry moved (b620): the second sees its line in use.
        let in_use = [a.0];
        let others = [(a.0, a.1)];
        let b = second.line_end_chain(&ls, &BranchContext { in_use: &in_use, others: &others, ..body_ctx(pos, &players) }, &mut Vec::new()).expect("another line");
        assert_ne!(a.0, b.0, "{a:?} {b:?}");
    }

    #[test]
    fn a_mirrored_cursor_applies_a_switch_recorded_between_nodes_once() {
        let players: [Vec3; 0] = [];
        let ls = lines(vec![along(1, [0.0; 3], [0.0, 1.0], 10), along(2, [0.0, 0.0, 11.0], [0.0, 1.0], 10)]);
        let (mut host, _, pos) = at_the_trigger(&ls);
        let mut records = Vec::new();
        host.line_end_chain(&ls, &body_ctx(pos, &players), &mut records);
        let records: Vec<BranchRecord> = records.into_iter().filter_map(|e| if let CursorEvent::Branch(r) = e { Some(r) } else { None }).collect();
        host.step(&ls, &mut Decider::Stay, &mut Vec::new());
        let mut client = LineCursor::new(id(1), 8);
        let mut out = Vec::new();
        for _ in 0..6 {
            client.step(&ls, &mut Decider::Mirror(&records, &[]), &mut out);
        }
        assert_eq!((client.line, client.node, client.frame_in_segment, client.frames), (host.line, host.node, host.frame_in_segment, host.frames));
        for _ in 0..30 {
            client.step(&ls, &mut Decider::Mirror(&records, &[]), &mut out);
        }
        assert_eq!(out.iter().filter(|e| matches!(e, CursorEvent::Branch(_))).count(), 1, "applied once");
    }
}

/// b706: the branch / line-end angle is taken about the skater frame's up row (`824536C8` projects both vectors onto
/// the plane across the axis): the same as world Y on flat ground, different on a bank.
#[test]
fn the_branch_angle_is_taken_about_the_skaters_up_axis() {
    for (a, b) in [([0.0, 0.0, 1.0], [1.0, 0.0, 1.0]), ([0.3, 0.2, 1.0], [-1.0, 0.5, 0.2]), ([1.0, 0.0, 0.0], [0.0, 0.0, -1.0])] {
        assert_eq!(angle_about(a, b, [0.0, 1.0, 0.0]).to_bits(), yaw_angle(a, b).to_bits(), "{a:?} {b:?}");
    }
    // Banked 45 degrees about the forward (+z): a node ahead and raised along the skater's up is dead ahead about that
    // up, but about 35 degrees off about world Y.
    let s = std::f32::consts::FRAC_1_SQRT_2;
    let up = [-s, s, 0.0];
    let ahead_on_bank = [-s, s, 1.0];
    assert!(angle_about([0.0, 0.0, 1.0], ahead_on_bank, up).abs() < 1e-6);
    assert!((yaw_angle([0.0, 0.0, 1.0], ahead_on_bank).abs() - s.atan()).abs() < 1e-5);
    // The skater heading comes from the body gate when set.
    let players: [Vec3; 0] = [];
    let ls = lines(vec![along(1, [0.0; 3], [0.0, 1.0], 10)]);
    let at = ctx([0.0, 0.0, 2.0], &players);
    let facing_away = BranchContext { forward: [0.0, 0.0, -1.0], ..at };
    assert_eq!(branch_score(&ls[&id(1)], 2, &facing_away), None, "180 degrees off");
    let gated = BranchContext { body: Some(BodyGate { point: [0.0, 0.0, 2.0], may_branch: true, heading: Some(([0.0, 0.0, 1.0], [0.0, 1.0, 0.0])) }), ..facing_away };
    assert!(branch_score(&ls[&id(1)], 2, &gated).is_some(), "the deck's forward from the gate wins");
}

/// b711 O1 / item 2: a client following a simulated NPC from the replicated state (the host's follow clock per tick,
/// its branch / restart records, the hold flag) has the host's cursor every tick: across a mid-segment line end
/// chain, a restart (frames keep running, so records stay unique) and a hold at a dead end.
#[test]
fn a_client_follows_a_simulated_cursor_through_a_chain_a_restart_and_a_hold() {
    let players: [Vec3; 0] = [];
    let ls = lines(vec![along(1, [0.0; 3], [0.0, 1.0], 10), along(2, [0.0, 0.0, 11.0], [0.0, 1.0], 10)]);
    let drive = |c: &mut LineCursor, target: u64, decider: &mut Decider, out: &mut Vec<CursorEvent>| {
        while c.frames < target && !c.finished {
            let before = c.frames;
            c.step(&ls, decider, out);
            if c.frames == before && !c.finished {
                break;
            }
        }
    };
    let mut host = LineCursor::spawn(&ls, id(1), 0);
    let mut client = LineCursor::spawn(&ls, id(1), 0);
    host.hold_at_end = true;
    client.hold_at_end = true;
    let mut records: Vec<BranchRecord> = Vec::new();
    let mut target = 0u64;
    let (mut chained, mut restarted) = (false, false);
    for tick in 0..400u64 {
        // The projection holds the clock back now and then (B not ahead).
        if tick % 7 != 3 {
            target += 1;
        }
        drive(&mut host, target, &mut Decider::Stay, &mut Vec::new());
        client.apply_records(&ls, &Decider::Mirror(&records, &[]), &mut Vec::new());
        drive(&mut client, target, &mut Decider::Mirror(&records, &[]), &mut Vec::new());
        assert_eq!((client.line, client.node, client.frame_in_segment, client.frames, client.finished), (host.line, host.node, host.frame_in_segment, host.frames, host.finished), "tick {tick}");
        assert_eq!(client.phase_history().collect::<Vec<_>>(), host.phase_history().collect::<Vec<_>>(), "tick {tick}");
        // After the step, as `simulate` does: the line end chain mid-segment, later a restart (re-snap) back on line 2.
        let mut out = Vec::new();
        if !chained && host.line == id(1) && host.node == 8 && host.frame_in_segment >= 5 {
            let pos = [0.0, 0.0, 8.0 + host.frame_in_segment as f32 / 10.0];
            let ctx = BranchContext { body: Some(BodyGate { point: pos, may_branch: true, heading: None }), chain: ChainConfig { fallback: true, ..ChainConfig::retail() }, ..ctx(pos, &players) };
            chained = host.line_end_chain(&ls, &ctx, &mut out).is_some();
        } else if chained && !restarted && host.line == id(2) && host.node == 5 {
            host.respawn(&ls, id(2), 3, &mut out);
            restarted = true;
        }
        records.extend(out.into_iter().filter_map(|e| if let CursorEvent::Branch(r) = e { Some(r) } else { None }));
    }
    assert!(chained && restarted && records.len() == 2 && records[1].respawn, "{records:?}");
    assert!(host.line == id(2) && host.node == 9 && !host.finished && !client.finished, "both hold at the dead end");
    // Without the hold flag a mirrored cursor finishes there (the old behaviour).
    let mut free = LineCursor::spawn(&ls, id(1), 0);
    drive(&mut free, target, &mut Decider::Mirror(&records, &[]), &mut Vec::new());
    assert!(free.finished);
}

/// b715 (recomp trace of an ambient bail): solid for 4 s, a 1 s fade out (timer +0x750 at 0.983 one tick before the
/// place at 5.0 s), and a mod can turn it off or move it.
#[test]
fn a_bailed_skater_fades_out_over_the_last_second_before_its_respawn() {
    use crate::living_world::leave_fade::LeaveFadeConfig;
    let c = LeaveFadeConfig::retail();
    assert_eq!(c.bail_alpha(0, 300), 1.0);
    assert_eq!(c.bail_alpha(240, 300), 1.0, "the fade starts 1 s before the respawn");
    assert!((c.bail_alpha(270, 300) - 0.5).abs() < 1e-6);
    assert!((c.bail_alpha(299, 300) - (1.0 - 59.0 / 60.0)).abs() < 1e-6, "retail's timer reads 0.983 at the place");
    assert_eq!(c.bail_alpha(400, 300), 0.0);
    assert_eq!(LeaveFadeConfig { bail_fade: false, ..c }.bail_alpha(299, 300), 1.0);
    assert_eq!(LeaveFadeConfig { bail_fade_lead_seconds: 2.0, ..c }.bail_alpha(180, 300), 1.0);
    assert!(LeaveFadeConfig { bail_fade_lead_seconds: 2.0, ..c }.bail_alpha(200, 300) < 1.0);
}
