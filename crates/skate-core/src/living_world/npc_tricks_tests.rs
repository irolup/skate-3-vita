//! Trick choice tests on synthetic lines (no data). The data-gated profile checks live in
//! `skate-data/tests/living_world_data.rs`.

use super::*;
use crate::living_world::replay::{BranchContext, ChainConfig, CursorEvent, Decider, LineCursor, ReplayJump, ReplayNode, TrickRecord};
use std::collections::BTreeMap;

const OLLIE: i16 = 128;
const NOLLIE: i16 = 127;
const KICKFLIP: i16 = 96;
const KICKFLIP2: i16 = 97;
const HEELFLIP: i16 = 92;
const N_KICKFLIP: i16 = 117;
const BSGRAB: i16 = 54;
const NOSEMANUAL: i16 = 2;

const S: u8 = node_events::START_TRICK;
const E: u8 = node_events::END_TRICK;

/// One node per entry: (frames since the previous node, event, airborne, trick slot).
fn line(nodes: &[(u8, u8, bool, i16)]) -> ReplayLine {
    let mut jumps = Vec::new();
    let nodes = nodes
        .iter()
        .enumerate()
        .map(|(i, &(frames, event, air, trick))| {
            let jump = (trick >= 0).then(|| {
                jumps.push(ReplayJump { start_position: [0.0; 3], start_velocity: [0.0; 3], offset: [0.0; 3], trick, spins: 0, flags: 0 });
                jumps.len() as u32 - 1
            });
            ReplayNode { position: [0.0, 0.0, i as f32], step: [0.0, 0.0, 0.0], board: [128, 128, 128, 255], skater: [128, 128, 128, 255], frames, event, flags: if air { node_flags::AIRBORNE } else { 0 }, jump, width: [50, 50] }
        })
        .collect();
    ReplayLine { id: [7; 16], flags: 4, skill: 0, nodes, jumps, groups: vec![] }
}

/// A slot at node 1 with the trick `t`, then `air / 10` airborne nodes of 10 frames, a landing
/// node of 10 frames and ground after it.
fn air_line(t: i16, air: u8) -> ReplayLine {
    let mut v = vec![(0, 0, false, -1), (10, S, true, t)];
    for _ in 0..air / 10 {
        v.push((10, 0, true, -1));
    }
    v.push((10, E, false, -1));
    v.extend([(10, 0, false, -1), (10, 0, false, -1)]);
    line(&v)
}

fn profile() -> TrickProfile {
    TrickProfile { regular: vec![(KICKFLIP, 1.0), (HEELFLIP, 3.0)], nollie: vec![(N_KICKFLIP, 2.0)] }
}

fn ctx(p: &TrickProfile, seed: u64) -> TrickContext<'_> {
    TrickContext { mode: TrickMode::Profile, params: TrickParams::RETAIL, profile: Some(p), seed, slot_tricks: true }
}

#[test]
fn weighted_pick_takes_the_first_entry_below_the_running_sum() {
    let t = [(1, 1.0), (2, 3.0)]; // normalised 0.25 / 1.0
    assert_eq!(pick_weighted(&t, 0.0), Some(1));
    assert_eq!(pick_weighted(&t, 0.249), Some(1));
    assert_eq!(pick_weighted(&t, 0.25), Some(2), "u < sum is strict");
    assert_eq!(pick_weighted(&t, 0.999_999), Some(2));
    // Rounding past the last sum falls back to entry 0 like retail.
    assert_eq!(pick_weighted(&[(5, 0.1), (6, 0.1), (7, 0.1)], 1.0), Some(5));
    assert_eq!(pick_weighted(&[], 0.5), None);
    assert_eq!(pick_weighted(&[(1, 0.0)], 0.5), None);
}

#[test]
fn gate_needs_more_than_50_frames_before_the_landing() {
    // After the slot: 3 air nodes (30) + the landing node (10) = 40 frames.
    assert!(!gate(&air_line(KICKFLIP, 30), 1, KICKFLIP, TrickParams::RETAIL));
    // 4 air nodes + landing = 50: not more than 50.
    assert!(!gate(&air_line(KICKFLIP, 40), 1, KICKFLIP, TrickParams::RETAIL));
    // 5 air nodes + landing = 60.
    assert!(gate(&air_line(KICKFLIP, 50), 1, KICKFLIP, TrickParams::RETAIL));
    // A mod's window applies.
    assert!(gate(&air_line(KICKFLIP, 40), 1, KICKFLIP, TrickParams { min_air_frames: 49, ..TrickParams::RETAIL }));
}

#[test]
fn gate_fails_when_a_trick_follows_the_landing_and_passes_at_the_window_end() {
    let v = [(0, 0, false, -1), (10, S, true, KICKFLIP), (30, 0, true, -1), (30, 0, false, -1), (10, S, false, NOSEMANUAL)];
    assert!(!gate(&line(&v), 1, KICKFLIP, TrickParams::RETAIL));
    // Airborne past 300 frames: the window ends and the gate passes.
    let mut v = vec![(0, 0, false, -1), (10, S, true, KICKFLIP)];
    v.extend(std::iter::repeat_n((100, 0, true, -1), 4));
    v.push((10, 0, false, -1));
    assert!(gate(&line(&v), 1, KICKFLIP, TrickParams::RETAIL));
    // The end of the line passes too.
    assert!(gate(&line(&[(0, 0, false, -1), (10, S, true, KICKFLIP), (10, 0, true, -1)]), 1, KICKFLIP, TrickParams::RETAIL));
}

#[test]
fn gate_passes_a_following_slot_only_when_it_continues_the_chain() {
    let chain = line(&[(0, 0, false, -1), (10, S, true, KICKFLIP), (10, S, true, KICKFLIP2), (60, E, false, -1)]);
    assert!(gate(&chain, 1, KICKFLIP, TrickParams::RETAIL));
    let other = line(&[(0, 0, false, -1), (10, S, true, KICKFLIP), (10, S, true, BSGRAB), (60, E, false, -1)]);
    assert!(!gate(&other, 1, KICKFLIP, TrickParams::RETAIL));
    let not_mine = line(&[(0, 0, false, -1), (10, S, true, HEELFLIP), (10, S, true, KICKFLIP2), (60, E, false, -1)]);
    assert!(!gate(&not_mine, 1, HEELFLIP, TrickParams::RETAIL));
}

fn started(c: TrickChoice) -> Option<i16> {
    if let TrickChoice::Start(t) = c {
        Some(t)
    } else {
        None
    }
}

#[test]
fn ollie_and_flip_slots_are_picked_from_the_profile_table() {
    let p = profile();
    let l = air_line(OLLIE, 60);
    let picks: std::collections::BTreeSet<i16> = (0..64).filter_map(|seed| started(choose(&l, 1, 10, &ctx(&p, seed)))).collect();
    assert_eq!(picks, [HEELFLIP, KICKFLIP].into(), "the regular table");
    // Nollie slots use the nollie table.
    assert_eq!(choose(&air_line(NOLLIE, 60), 1, 10, &ctx(&p, 3)), TrickChoice::Start(N_KICKFLIP));
    assert!(is_nollie(N_KICKFLIP) && is_nollie(NOLLIE) && !is_nollie(OLLIE) && !is_nollie(KICKFLIP));
    // Short air: the gate fails and the recorded trick stays.
    assert_eq!(choose(&air_line(KICKFLIP, 20), 1, 10, &ctx(&p, 3)), TrickChoice::Start(KICKFLIP));
}

#[test]
fn other_categories_chain_levels_and_modes() {
    let p = profile();
    for t in [BSGRAB, NOSEMANUAL] {
        let l = air_line(t, 60);
        assert_eq!(choose(&l, 1, 10, &ctx(&p, 1)), TrickChoice::Start(t), "category {:?} keeps the recorded trick", catalog::category(t));
        assert_eq!(choose(&l, 1, 10, &TrickContext { mode: TrickMode::None, ..ctx(&p, 1) }), TrickChoice::Start(t), "mode none only stops ollies / flips");
    }
    assert_eq!(choose(&air_line(KICKFLIP2, 60), 1, 10, &ctx(&p, 1)), TrickChoice::Continue);
    let l = air_line(KICKFLIP, 60);
    assert_eq!(choose(&l, 1, 10, &TrickContext { mode: TrickMode::Recorded, ..ctx(&p, 1) }), TrickChoice::Start(KICKFLIP));
    assert_eq!(choose(&l, 1, 10, &TrickContext { mode: TrickMode::None, ..ctx(&p, 1) }), TrickChoice::Start(-1));
    assert_eq!(choose(&l, 1, 10, &TrickContext { profile: None, ..ctx(&p, 1) }), TrickChoice::Start(KICKFLIP));
    let empty = TrickProfile::default();
    assert_eq!(choose(&l, 1, 10, &ctx(&empty, 1)), TrickChoice::Start(KICKFLIP), "an empty table keeps the recorded trick");
    assert!(catalog::is_chain_level(KICKFLIP2) && !catalog::is_chain_level(KICKFLIP));
    assert!(!catalog::is_chain_level(104), "late flips have the ollie as base and are no chain level");
    assert_eq!(TrickMode::from_name("profile"), Some(TrickMode::Profile));
    assert_eq!(TrickMode::from_name("scripted"), None);
}

#[test]
fn catalog_links_are_consistent() {
    for &(id, prev, base) in &catalog::LINKS {
        let id = id as i16;
        assert!(matches!(catalog::category(id), Some(1 | 2)), "{id}");
        assert!(matches!(catalog::category(base), Some(1 | 2)), "{id}");
        if prev >= 0 {
            // A level steps down to the level before it, which shares its base (or is it).
            let prev_base = catalog::base(prev);
            assert_eq!(if prev_base < 0 { prev } else { prev_base }, base, "{id}");
        }
    }
}

fn run(l: &ReplayLine, mode: TrickMode, seed: u64, p: &TrickProfile) -> (Vec<TrickRecord>, Vec<i16>) {
    let ls: BTreeMap<[u8; 16], ReplayLine> = [(l.id, l.clone())].into();
    let mut c = LineCursor::new(l.id, 0);
    let mut records = Vec::new();
    let mut seen = Vec::new();
    let tricks = TrickContext { mode, ..ctx(p, seed) };
    let b = BranchContext { position: [0.0; 3], forward: [0.0, 0.0, 1.0], speed: 5.0, players: &[], others: &[], in_use: &[], preferred_skill: -1, online: false, chain: ChainConfig { radius: 0.0, ..ChainConfig::retail() }, tricks, body: None, valid_lines: None };
    for _ in 0..200 {
        let mut ev = Vec::new();
        c.step(&ls, &mut Decider::Decide(b), &mut ev);
        records.extend(ev.into_iter().filter_map(|e| if let CursorEvent::Trick(r) = e { Some(r) } else { None }));
        seen.push(c.current_trick());
    }
    (records, seen)
}

#[test]
fn the_cursor_starts_the_chosen_trick_and_a_client_mirrors_it() {
    let p = profile();
    let l = line(&[(0, 0, false, -1), (10, S, true, KICKFLIP), (10, S, true, KICKFLIP2), (60, 0, true, -1), (10, E, false, -1), (10, 0, false, -1), (10, 0, false, -1)]);
    let (records, seen) = run(&l, TrickMode::Profile, 9, &p);
    assert_eq!(records.len(), 1, "the chain level makes no record: {records:?}");
    let chosen = records[0].chosen;
    assert_eq!(records[0].recorded, KICKFLIP);
    assert!(chosen == KICKFLIP || chosen == HEELFLIP);
    assert!(seen.contains(&chosen) && !seen.contains(&KICKFLIP2), "the chain level keeps the running trick");
    // Same seed, same choice; a client mirroring the records sees the same tricks.
    assert_eq!(run(&l, TrickMode::Profile, 9, &p).0, records);
    let ls: BTreeMap<[u8; 16], ReplayLine> = [(l.id, l.clone())].into();
    let mut m = LineCursor::new(l.id, 0);
    let mut mirrored = Vec::new();
    for _ in 0..200 {
        m.step(&ls, &mut Decider::Mirror(&[], &records), &mut Vec::new());
        mirrored.push(m.current_trick());
    }
    assert_eq!(mirrored, seen);
    // Recorded mode: the line's trick (the chain level keeps the kickflip running).
    let (records, seen) = run(&l, TrickMode::Recorded, 9, &p);
    assert_eq!(records.iter().map(|r| r.chosen).collect::<Vec<_>>(), [KICKFLIP]);
    assert!(!seen.contains(&KICKFLIP2));
}

#[test]
fn seeds_spread_over_the_table_weights() {
    let p = TrickProfile { regular: vec![(KICKFLIP, 1.0), (HEELFLIP, 3.0)], nollie: vec![] };
    let l = air_line(OLLIE, 60);
    let heel = (0..4000u64).filter(|&s| choose(&l, 1, 10, &ctx(&p, s)) == TrickChoice::Start(HEELFLIP)).count();
    assert!((2800..3200).contains(&heel), "about 3 in 4: {heel}");
}

/// `sub_8246A2E0` (research b159 item 1): without the profile's slot-trick flag the AI starts no grab (categories 3, 4, 12);
/// other categories keep the recorded trick.
#[test]
fn a_profile_without_slot_tricks_skips_grabs() {
    let p = profile();
    assert!(matches!(catalog::category(BSGRAB), Some(3 | 4 | 12)) && !matches!(catalog::category(NOSEMANUAL), Some(3 | 4 | 12)));
    let off = |t: i16| choose(&air_line(t, 60), 1, 10, &TrickContext { slot_tricks: false, ..ctx(&p, 1) });
    assert_eq!(off(BSGRAB), TrickChoice::Start(-1));
    assert_eq!(off(NOSEMANUAL), TrickChoice::Start(NOSEMANUAL));
    assert_eq!(choose(&air_line(BSGRAB, 60), 1, 10, &ctx(&p, 1)), TrickChoice::Start(BSGRAB));
}

/// A dropped slot (choice -1) starts no trick: the record still says so, but the cursor enters no trick phase, so the
/// puppet shows no trick pose for it (research b187; before, the span opened and the puppet fell back to a grind pose).
#[test]
fn a_dropped_slot_opens_no_trick_span() {
    use crate::living_world::replay::ReplayPhase;
    let p = profile();
    let l = air_line(BSGRAB, 60);
    let ls: BTreeMap<[u8; 16], ReplayLine> = [(l.id, l.clone())].into();
    let tricks = TrickContext { slot_tricks: false, ..ctx(&p, 1) };
    let b = BranchContext { position: [0.0; 3], forward: [0.0, 0.0, 1.0], speed: 5.0, players: &[], others: &[], in_use: &[], preferred_skill: -1, online: false, chain: ChainConfig { radius: 0.0, ..ChainConfig::retail() }, tricks, body: None, valid_lines: None };
    let mut c = LineCursor::new(l.id, 0);
    let mut records = Vec::new();
    let mut phases = Vec::new();
    for _ in 0..200 {
        let mut ev = Vec::new();
        c.step(&ls, &mut Decider::Decide(b), &mut ev);
        records.extend(ev.into_iter().filter_map(|e| if let CursorEvent::Trick(r) = e { Some(r) } else { None }));
        assert_eq!(c.current_trick(), -1);
        phases.extend(c.phase_history().map(|(e, _)| e.phase));
    }
    assert_eq!(records.iter().map(|r| (r.recorded, r.chosen)).collect::<Vec<_>>(), [(BSGRAB, -1)]);
    assert!(phases.contains(&ReplayPhase::Air), "{phases:?}");
    assert!(!phases.iter().any(|ph| matches!(ph, ReplayPhase::AirTrick | ReplayPhase::GroundTrick)), "{phases:?}");
}
