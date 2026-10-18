//! Data-gated checks of the living-world population data on the user's own export. Skips (passes
//! with a note) when no data is configured. `SKATE3_ASSET_ROOT`: converted assets (one root, or
//! several joined like PATH); reads `private/living_world/{tables.json, census.json,
//! <District>.census.bin, skater_profiles.json, skater_paths/*.bin}`.
//!
//! The expected values are the shipped Skate 3 data the research measured (labels in
//! `.claude/notes/npc-livingworld-re.md` §1 and doc 26); they are asserted, not embedded in the
//! engine.

use skate_core::living_world::{Kind, LivingWorld, Observer, PopulationConfig, TickInputs};
use skate_data::aipath;
use skate_data::living_world::{self, LivingWorldTables};
use std::path::PathBuf;

fn folders() -> Vec<PathBuf> {
    let Some(raw) = std::env::var_os("SKATE3_ASSET_ROOT") else { return Vec::new() };
    std::env::split_paths(&raw)
        .flat_map(|root| ["private/living_world", "living_world"].map(|p| root.join(p)))
        .filter(|p| p.is_dir())
        .collect()
}

fn find(name: &str) -> Option<PathBuf> {
    folders().into_iter().map(|f| f.join(name)).find(|p| p.exists())
}

fn tables() -> Option<LivingWorldTables> {
    let path = find("tables.json")?;
    Some(LivingWorldTables::parse(&std::fs::read(path).unwrap()).unwrap())
}

const DISTRICTS: [(&str, u32, u32); 3] = [("DownTown", 384, 448), ("Industrial", 640, 288), ("University", 384, 448)];

#[test]
fn census_tables_hold_the_shipped_caps_and_ranges() {
    let Some(t) = tables() else {
        eprintln!("skipped: set SKATE3_ASSET_ROOT to an export with living_world/tables.json");
        return;
    };
    let cap = |k: &str| t.census[k].max_population;
    // Peds [data]: districts 20, sub-areas 15, default 40, reclaimed 10, loadingdocks 8, observatory 6.
    for k in ["downtown", "university", "industrial"] {
        assert_eq!(cap(k), 20, "{k}");
    }
    for k in ["aletown", "business_center", "mall", "memorial", "residential", "campus"] {
        assert_eq!(cap(k), 15, "{k}");
    }
    assert_eq!((cap("pedestrians"), cap("reclaimed"), cap("loadingdocks"), cap("observatory")), (40, 10, 8, 6));
    // Vehicles [data]: dwntwn 30, indust 25, univ 10, default 25.
    assert_eq!((cap("dwntwn"), cap("indust"), cap("univ"), cap("vehicles")), (30, 25, 10, 25));
    // Category weights of the DownTown traffic group sum to 0.875 (dwntwn); the roll misses the rest.
    let w: f32 = t.census["dwntwn"].categories.iter().map(|c| c.weight).sum();
    assert!((w - 0.875).abs() < 1e-4, "{w}");

    let peds = t.ranges["pedestrians"];
    assert_eq!((peds.slow.spawn_inner, peds.slow.spawn_outer, peds.slow.cull, peds.slow.forward_offset, peds.slow.speed_kmh), (50.0, 60.0, 70.0, 0.0, 45.0));
    assert_eq!((peds.fast.spawn_inner, peds.fast.spawn_outer, peds.fast.cull, peds.fast.forward_offset, peds.fast.speed_kmh), (50.0, 80.0, 90.0, 20.0, 80.0));
    let cars = t.ranges["vehicles"].at(50.0);
    assert_eq!((cars.spawn_inner, cars.spawn_outer, cars.cull, cars.forward_offset), (80.0, 100.0, 110.0, 0.0));
    let mut cfg = PopulationConfig::retail();
    t.apply_to(&mut cfg);
    assert_eq!(cfg.pedestrians.range, Some(peds));
}

#[test]
fn census_grids_parse_and_name_known_records() {
    let Some(t) = tables() else {
        eprintln!("skipped: set SKATE3_ASSET_ROOT to an export with living_world census grids");
        return;
    };
    for (district, w, h) in DISTRICTS {
        let Some(path) = find(&format!("{district}.census.bin")) else { panic!("{district} grid missing") };
        let g = living_world::parse_census_grid(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!((g.width, g.height, g.cell), (w, h, 4.0), "{district}");
        for name in &g.names {
            assert!(t.census.contains_key(name), "{district}: {name} is not a census record");
        }
        for layer in ["livingworld_npc_census", "livingworld_vehicle_census"] {
            let painted = g.layers[layer].iter().filter(|&&v| v != 0).count();
            assert!(painted > 1000, "{district} {layer}: {painted} painted cells");
        }
    }
}

/// The census point of a grid with the most painted ring around it (8-80 m, peds layer).
fn busy_point(g: &skate_core::living_world::CensusGrid) -> [f32; 3] {
    let mut best = (0, [0.0f32; 3]);
    for j in (0..g.height).step_by(8) {
        for i in (0..g.width).step_by(8) {
            let x = g.origin[0] + (i as f32 + 0.5) * g.cell;
            let z = g.origin[1] + (j as f32 + 0.5) * g.cell;
            let score = (0..48)
                .filter(|k| {
                    let a = *k as f32 * std::f32::consts::TAU / 16.0;
                    let r = [20.0, 50.0, 60.0][*k as usize % 3];
                    g.record_at("livingworld_npc_census", x + r * a.cos(), z + r * a.sin()).is_some()
                })
                .count();
            if score > best.0 {
                best = (score, [x, 0.0, z]);
            }
        }
    }
    best.1
}

#[test]
fn population_on_the_exported_downtown_census_respects_caps_and_radii() {
    let (Some(t), Some(path)) = (tables(), find("DownTown.census.bin")) else {
        eprintln!("skipped: set SKATE3_ASSET_ROOT to an export with the DownTown census grid");
        return;
    };
    let grid = living_world::parse_census_grid(&std::fs::read(path).unwrap()).unwrap();
    let at = busy_point(&grid);
    let map = t.census_map(vec![grid]);
    let mut cfg = PopulationConfig::retail();
    t.apply_to(&mut cfg);
    let mut world = LivingWorld::new(cfg, 2026);
    let obs = [Observer { position: at, velocity: [0.0; 3] }];
    let mut inputs = TickInputs::offline(&obs);
    inputs.census = Some(&map);
    let mut max_peds = 0;
    for _ in 0..(30 * 60) {
        for d in world.step(&inputs) {
            if let skate_core::living_world::Decision::Spawn(s) = d {
                let r = ((s.position[0] - at[0]).powi(2) + (s.position[2] - at[2]).powi(2)).sqrt();
                let (lo, hi) = if s.initial { (8.0, 80.0) } else if s.id.kind == Kind::Pedestrian { (50.0, 60.0) } else { (80.0, 100.0) };
                assert!(r >= lo - 1e-3 && r <= hi + 1e-3, "{:?} at {r}", s.id);
            }
        }
        max_peds = max_peds.max(world.count(Kind::Pedestrian));
        assert!(world.count(Kind::Vehicle) <= 30);
    }
    // DownTown ped records cap at 15 (sub-areas) and 40 (`pedestrians`, a sliver): with the pool of
    // 31 the live count never passes the largest cap painted around the point.
    assert!(max_peds > 0 && max_peds <= 15, "peds {max_peds}");
    eprintln!("DownTown at {at:?}: {max_peds} peds max, {} vehicles", world.count(Kind::Vehicle));
}

#[test]
fn skater_trick_profiles_hold_ollies_and_flips_for_every_pool_character() {
    let Some(profiles) = find("skater_profiles.json") else {
        eprintln!("skipped: set SKATE3_ASSET_ROOT to an export with skater_profiles.json");
        return;
    };
    let bytes = std::fs::read(profiles).unwrap();
    let t = living_world::skater_trick_profiles(&bytes).unwrap();
    // 193 `ai_skater_profiles` records [data].
    assert_eq!(t.profiles.len(), 193);
    for (name, p) in &t.profiles {
        // Retail's re-pick only reads these tables for ollie / flip slots (category 1 / 2).
        for &(trick, w) in p.regular.iter().chain(&p.nollie) {
            assert!(matches!(skate_core::scoring::catalog::category(trick), Some(1 | 2)), "{name}: trick {trick}");
            assert!(w >= 0.0, "{name}: weight {w}");
        }
    }
    let d = &t.profiles["default"];
    assert!(!d.regular.is_empty() && !d.nollie.is_empty());
    for c in living_world::skater_characters(&bytes, &[]).unwrap() {
        let p = t.for_character(&c.key).unwrap_or_else(|| panic!("{} has no profile", c.key));
        assert!(!p.regular.is_empty(), "{}", c.key);
    }
}

#[test]
fn skater_pool_and_lines_load() {
    let (Some(profiles), Some(paths)) = (find("skater_profiles.json"), find("skater_paths")) else {
        eprintln!("skipped: set SKATE3_ASSET_ROOT to an export with skater_profiles.json and skater_paths");
        return;
    };
    let chars = living_world::skater_characters(&std::fs::read(profiles).unwrap(), &[]).unwrap();
    // 42-character pool [data] minus the 4 teammates nobody recruited yet.
    assert_eq!(chars.len(), 38, "{:?}", chars.iter().map(|c| &c.key).collect::<Vec<_>>());
    let mut total = 0;
    for entry in std::fs::read_dir(paths).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("bin") {
            continue;
        }
        let bytes = std::fs::read(&path).unwrap();
        let tiles = aipath::parse_pack(&bytes).unwrap();
        let (unique, _) = aipath::district_paths(&tiles).unwrap();
        let lines = living_world::skater_lines(unique.iter().map(|d| &d.path));
        assert_eq!(lines.len(), unique.len());
        total += lines.len();
    }
    assert_eq!(total, 1_691);
}

/// Milestone 3: every exported line rides end to end with the replay cursor at the recording
/// rate, with the retail branch choice (a player standing at the line start), deterministically.
#[test]
fn replay_cursor_rides_every_exported_line() {
    use skate_core::living_world::replay::{BranchContext, ChainConfig, CursorEvent, Decider, LineCursor, ReplayLine};
    use std::collections::BTreeMap;
    let Some(paths) = find("skater_paths") else {
        eprintln!("skipped: set SKATE3_ASSET_ROOT to an export with skater_paths");
        return;
    };
    let mut total = (0usize, 0usize, 0usize, 0u64);
    for district in ["DownTown", "Industrial", "University"] {
        let pack = std::fs::read(paths.join(format!("{district}.bin"))).unwrap();
        let tiles = aipath::parse_pack(&pack).unwrap();
        let (unique, _) = aipath::district_paths(&tiles).unwrap();
        let lines: BTreeMap<[u8; 16], ReplayLine> = unique.iter().map(|p| (p.path.id.0, living_world::replay_line(&p.path))).collect();
        // Every branch target resolves to a line of the same district with the node in range.
        for l in lines.values() {
            for g in &l.groups {
                assert!((g.node as usize) < l.nodes.len().saturating_sub(1), "group on the last node");
                for b in &g.branches {
                    assert!(lines.get(&b.target).is_some_and(|t| (b.target_node as usize) < t.nodes.len()));
                }
            }
        }
        let run = |id: [u8; 16]| {
            let start = lines[&id].nodes[0].position;
            let players = [start];
            let mut c = LineCursor::spawn(&lines, id, 0);
            let mut ev = Vec::new();
            let mut frames = 0u64;
            let mut last = c.sample(&lines, 0.0).unwrap().position;
            let mut max_jump = 0.0f32;
            while !c.finished && frames < 60 * 60 * 2 {
                let s = c.sample(&lines, 0.0).unwrap();
                let speed = (s.velocity[0].powi(2) + s.velocity[1].powi(2) + s.velocity[2].powi(2)).sqrt();
                let ctx = BranchContext { position: s.position, forward: s.velocity, speed, players: &players, others: &[], in_use: &[], preferred_skill: -1, online: false, chain: ChainConfig::retail(), tricks: Default::default() };
                c.step(&lines, &mut Decider::Decide(ctx), &mut ev);
                frames += 1;
                let p = c.sample(&lines, 0.0).unwrap().position;
                let branched = matches!(ev.last(), Some(CursorEvent::Branch(_)));
                if !branched {
                    max_jump = max_jump.max(((p[0] - last[0]).powi(2) + (p[1] - last[1]).powi(2) + (p[2] - last[2]).powi(2)).sqrt());
                }
                last = p;
            }
            (c, ev.into_iter().filter(|e| matches!(e, CursorEvent::Branch(_))).count(), frames, max_jump)
        };
        for &id in lines.keys() {
            let (c, branches, frames, max_jump) = run(id);
            // A recorded line moves at most a few metres per 60 Hz frame (p90 speed 16 m/s [data]).
            assert!(max_jump < 3.0, "{district} {:02x?}: {max_jump} m in one frame", &id[..8]);
            total.0 += 1;
            total.1 += branches;
            total.2 += c.finished as usize;
            total.3 += frames;
        }
        // Deterministic: the same line twice gives the same cursor.
        let first = *lines.keys().next().unwrap();
        assert_eq!(run(first).0, run(first).0);
    }
    eprintln!("replay: {} lines, {} branches taken, {} finished, {} frames", total.0, total.1, total.2, total.3);
    assert_eq!(total.0, 1691);
    // Fix 9: at a line end the cursor chains to a line starting within 4 m (sub_8246C7F8), so
    // most rides keep going for the whole 2 minutes instead of ending with their first line.
    assert!(total.2 * 2 < total.0, "most rides keep going for 2 minutes: {} of {} ended", total.2, total.0);
}

/// Fix 14 (NPC skater jitter): exported lines ridden with the retail branch and chain choice and
/// drawn at 144 Hz like `npc_skaters::present_pose` (the cursor one tick back, interpolated by the
/// fixed-step fraction with the recorded decisions). A render step never moves the root more than
/// the line's own fastest motion plus one smoothstep step of a 4 m chain gap, while the same ride
/// cut over (blend 0) jumps; the clip time never steps back. Seeded (the line order) and
/// deterministic.
#[test]
fn npc_skater_render_is_smooth_on_the_exported_lines() {
    use skate_core::living_world::replay::{BranchContext, BranchRecord, ChainConfig, CursorEvent, Decider, LineCursor, ReplayLine, ReplaySample, SWITCH_BLEND_SECONDS};
    use std::collections::BTreeMap;
    let Some(paths) = find("skater_paths") else {
        eprintln!("skipped: set SKATE3_ASSET_ROOT to an export with skater_paths");
        return;
    };
    const RENDER_HZ: f64 = 144.0;
    const SECONDS: f64 = 40.0;
    let dist = |a: [f32; 3], b: [f32; 3]| ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt();
    // (largest drawn step at a render step that crossed a switch, largest step elsewhere, switches,
    // largest raw step, clip time went back)
    let ride = |lines: &BTreeMap<[u8; 16], ReplayLine>, id: [u8; 16], blend: f32| {
        let players = [lines[&id].nodes[0].position];
        let mut c = LineCursor::spawn(lines, id, 0);
        c.switch_blend_seconds = blend;
        let mut prev = c.clone();
        let mut made: Vec<BranchRecord> = Vec::new();
        let (mut at_switch, mut elsewhere, mut back) = (0.0f32, 0.0f32, false);
        let mut last: Option<ReplaySample> = None;
        let mut last_hops = 0;
        let mut gaps: Vec<f32> = Vec::new();
        for k in 0..(SECONDS * RENDER_HZ) as u32 {
            let t = f64::from(k) / RENDER_HZ * 60.0;
            let tick = (t + 1e-9).floor() as u64;
            while c.frames < tick && !c.finished {
                prev = c.clone();
                let s = c.sample(lines, 0.0).unwrap();
                let speed = (s.velocity[0].powi(2) + s.velocity[1].powi(2) + s.velocity[2].powi(2)).sqrt();
                let ctx = BranchContext { position: s.position, forward: s.velocity, speed, players: &players, others: &[], in_use: &[], preferred_skill: -1, online: false, chain: ChainConfig::retail(), tricks: Default::default() };
                let mut ev = Vec::new();
                c.step(lines, &mut Decider::Decide(ctx), &mut ev);
                if ev.iter().any(|e| matches!(e, CursorEvent::Branch(_))) {
                    gaps.push(c.switch.as_ref().map_or(0.0, |w| dist(w.offset, [0.0; 3])));
                }
                made.extend(ev.into_iter().filter_map(|e| if let CursorEvent::Branch(b) = e { Some(b) } else { None }));
            }
            if c.finished {
                break;
            }
            let s = if tick == 0 { c.sample(lines, 0.0) } else { prev.render_sample(lines, &made, &[], (t - tick as f64) as f32) }.unwrap();
            if let Some(l) = &last {
                let d = dist(s.position, l.position);
                // A render step while a switch blends (0.2 s = 12 ticks, plus the tick of render lag)
                // counts as "at the switch".
                let near_switch = made.len() != last_hops || made.last().is_some_and(|b| c.frames.saturating_sub(b.frame) <= 14);
                if near_switch {
                    at_switch = at_switch.max(d);
                } else {
                    elsewhere = elsewhere.max(d);
                }
                if s.phase == l.phase && tick >= 2 && (s.phase_frames as f32 + s.sub_frame) < (l.phase_frames as f32 + l.sub_frame) {
                    back = true;
                }
            }
            last_hops = made.len();
            last = Some(s);
        }
        (at_switch, elsewhere, made.len(), back, c, gaps)
    };
    let mut worst = (0.0f32, 0.0f32, 0.0f32, 0usize, 0usize);
    let mut all_gaps: Vec<f32> = Vec::new();
    for district in ["DownTown", "Industrial", "University"] {
        let pack = std::fs::read(paths.join(format!("{district}.bin"))).unwrap();
        let tiles = aipath::parse_pack(&pack).unwrap();
        let (unique, _) = aipath::district_paths(&tiles).unwrap();
        let lines: BTreeMap<[u8; 16], ReplayLine> = unique.iter().map(|p| (p.path.id.0, living_world::replay_line(&p.path))).collect();
        // Seeded sample of start lines: every 8th in id order.
        for &id in lines.keys().step_by(8) {
            let (sw, other, hops, back, c, gaps) = ride(&lines, id, SWITCH_BLEND_SECONDS);
            let largest_gap = gaps.iter().copied().fold(0.0f32, f32::max);
            all_gaps.extend(gaps);
            let (cut, ..) = ride(&lines, id, 0.0);
            assert!(!back, "{district} {:02x?}: clip time went back", &id[..8]);
            worst.0 = worst.0.max(sw);
            worst.1 = worst.1.max(other);
            worst.2 = worst.2.max(cut);
            worst.3 += hops;
            worst.4 += 1;
            // A gap blended over 0.2 s with smoothstep moves at most 1.5 x gap / 0.2 / 144 m per
            // render step on top of the line's own motion.
            let bound = other.max(0.15) + 1.5 * largest_gap / SWITCH_BLEND_SECONDS / RENDER_HZ as f32 + 0.01;
            assert!(sw <= bound, "{district} {:02x?}: {sw} m at a switch vs bound {bound} m (gap {largest_gap} m)", &id[..8]);
            assert_eq!(ride(&lines, id, SWITCH_BLEND_SECONDS).4, c, "deterministic");
        }
    }
    all_gaps.sort_by(f32::total_cmp);
    let q = |f: f32| all_gaps.get(((all_gaps.len() as f32 - 1.0) * f) as usize).copied().unwrap_or(0.0);
    eprintln!("switch gaps: p50 {:.2} m, p90 {:.2} m, p99 {:.2} m, max {:.2} m, over 4 m: {}", q(0.5), q(0.9), q(0.99), q(1.0), all_gaps.iter().filter(|g| **g > 4.0).count());
    eprintln!("render 144 Hz: {} rides, {} switches, worst step at a switch {:.3} m (cut {:.3} m), elsewhere {:.3} m", worst.4, worst.3, worst.0, worst.2, worst.1);
    assert!(worst.3 > 100 && worst.0 < worst.2, "the blend removes the switch jumps");
}

/// Facing rules on the exported lines (fix 16, fix 23, fix 23 corrected). Every eighth line of the
/// three districts is ridden for 40 s (branches and chains decided by the retail chooser) under
/// three rules: the retail rule (`riding_entry`, mod option: recorded skater frame, turned while
/// the flip latched on entering riding is set, held across switches), the fix 23 rule
/// (`per_node`, the default, NOT RETAIL YET) and the fix 16 option (`keep_facing` on top of `per_node`, not retail).
/// Counted per rule: settled frames drawn more than 135 deg against travel where the line's path
/// frame does not oppose travel (`facing_check`, the `NPC_SKATER_BACKWARDS` diagnostic). The
/// counts are reported, not tuned: the retail rule draws a recorder's switch stance as recorded,
/// and the puppet has no stance mirror (NOT RETAIL YET), so its count measures that gap. Asserted:
/// determinism, the fix 23 rule's own property (it folds every node onto the path frame), and that
/// the fix 16 option rides more forward-recorded frames backwards than the fix 23 rule.
#[test]
fn npc_skater_facing_rules_on_the_exported_lines() {
    use skate_core::living_world::replay::{facing_check, rotate, BranchContext, ChainConfig, CursorEvent, Decider, FacingRule, LineCursor, ReplayLine, ReplayPhase};
    use std::collections::BTreeMap;
    let Some(paths) = find("skater_paths") else {
        eprintln!("skipped: set SKATE3_ASSET_ROOT to an export with skater_paths");
        return;
    };
    let yaw = |q: [f32; 4]| {
        let f = rotate(q, [0.0, 0.0, 1.0]);
        f[0].atan2(f[2])
    };
    let turn = |a: f32, b: f32| ((a - b + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI).abs();
    // (switches, spins: > 90 deg turn of the drawn skater in the 14 ticks after a switch beyond what
    // the rule's own line frame turns there, final cursor, backwards, judged, flip changes)
    let ride = |lines: &BTreeMap<[u8; 16], ReplayLine>, id: [u8; 16], chain: ChainConfig| {
        let players = [lines[&id].nodes[0].position];
        let mut c = LineCursor::spawn(lines, id, 0);
        c.keep_facing = chain.keep_facing;
        c.facing_rule = chain.facing_rule;
        c.fakie_settings = chain.fakie;
        let (mut switches, mut spins, mut backwards, mut judged, mut flip_changes) = (0usize, 0usize, 0usize, 0usize, 0usize);
        // Backwards frames by drawn stance: [drawn fakie (retail's fakie drawing), then not fakie
        // because: airborne, off board, ground trick span (bit held), first 3 s after spawn, other].
        let mut stance = [0usize; 7];
        // The fakie bit on the last ground frame (an air after a fakie takeoff is a fakie air).
        let mut ground_fakie = false;
        let mut watch: Option<(u64, f32, f32)> = None; // (switch frame, drawn yaw before, rule yaw at the new node)
        let spawn_flip = c.flip;
        for _ in 0..(40 * 60) {
            if c.finished {
                break;
            }
            let s = c.sample(lines, 0.0).unwrap();
            let speed = (s.velocity[0].powi(2) + s.velocity[1].powi(2) + s.velocity[2].powi(2)).sqrt();
            let ctx = BranchContext { position: s.position, forward: s.velocity, speed, players: &players, others: &[], in_use: &[], preferred_skill: -1, online: false, chain, tricks: Default::default() };
            let was = c.flip;
            let mut ev = Vec::new();
            c.step(lines, &mut Decider::Decide(ctx), &mut ev);
            flip_changes += usize::from(c.flip != was);
            let node = c.node as usize;
            if ev.iter().any(|e| matches!(e, CursorEvent::Branch(_))) {
                switches += 1;
                watch = Some((c.frames, yaw(s.skater), yaw(c.rule_skater(&lines[&c.line], node, node, 0.0))));
            }
            if c.switch.is_none() && !c.finished {
                if let Some(now) = c.sample(lines, 0.0) {
                    if matches!(now.phase, ReplayPhase::Rolling | ReplayPhase::Crouched) {
                        ground_fakie = now.fakie;
                    }
                    if let Some(f) = facing_check(&lines[&c.line], &now) {
                        judged += 1;
                        if f.backwards && !f.recorded_fakie {
                            backwards += 1;
                            let k = if f.drawn_fakie {
                                0
                            } else if matches!(now.phase, ReplayPhase::Air | ReplayPhase::AirTrick) {
                                if ground_fakie { 6 } else { 1 }
                            } else if now.phase == ReplayPhase::OffBoard {
                                2
                            } else if now.phase == ReplayPhase::GroundTrick {
                                3
                            } else if (c.frames as f32) < chain.fakie.after_teleport_seconds * 60.0 {
                                4
                            } else {
                                5
                            };
                            stance[k] += 1;
                        }
                    }
                }
            }
            if let Some((at, before, raw0)) = watch {
                if c.frames - at > 14 {
                    watch = None;
                } else if let Some(now) = c.sample(lines, 0.0) {
                    let raw_now = yaw(c.rule_skater(&lines[&c.line], node, node, 0.0));
                    if turn(yaw(now.skater), before) > std::f32::consts::FRAC_PI_2 + turn(raw_now, raw0) {
                        spins += 1;
                        watch = None;
                    }
                }
            }
        }
        (switches, spins, c, backwards, judged, flip_changes, spawn_flip, stance)
    };
    let retail = ChainConfig::retail();
    assert_eq!(retail.facing_rule, FacingRule::RidingEntry, "the default");
    let per_node = ChainConfig { facing_rule: FacingRule::PerNode, ..retail };
    let keep = ChainConfig { keep_facing: true, ..per_node };
    // [rides, switches, judged, spawn flips, later flip changes, then (backwards, spins) per rule]
    let mut total = [0usize; 11];
    // Backwards frames by drawn stance per rule (see `stance` in `ride`).
    let mut stances = [[0usize; 7]; 3];
    for district in ["DownTown", "Industrial", "University"] {
        let pack = std::fs::read(paths.join(format!("{district}.bin"))).unwrap();
        let tiles = aipath::parse_pack(&pack).unwrap();
        let (unique, _) = aipath::district_paths(&tiles).unwrap();
        let lines: BTreeMap<[u8; 16], ReplayLine> = unique.iter().map(|p| (p.path.id.0, living_world::replay_line(&p.path))).collect();
        for &id in lines.keys().step_by(8) {
            let r = ride(&lines, id, retail);
            assert_eq!(ride(&lines, id, retail).2, r.2, "deterministic");
            let p = ride(&lines, id, per_node);
            let k = ride(&lines, id, keep);
            assert!(!r.2.facing_flipped && !p.2.facing_flipped);
            total[0] += 1;
            total[1] += r.0;
            total[2] += r.4;
            total[3] += usize::from(r.6);
            total[4] += r.5;
            for (j, x) in [&r, &p, &k].into_iter().enumerate() {
                total[5 + 2 * j] += x.3;
                total[6 + 2 * j] += x.1;
                for (a, b) in stances[j].iter_mut().zip(x.7) {
                    *a += b;
                }
            }
        }
    }
    eprintln!(
        "facing: {} rides, {} switches, {} judged settled frames (retail rule), retail flip set at spawn on {} rides and changed {} times later; backwards on forward-recorded nodes / spins after a switch: retail riding_entry {} / {}, per_node (fix 23) {} / {}, per_node + keep_facing (fix 16) {} / {}",
        total[0], total[1], total[2], total[3], total[4], total[5], total[6], total[7], total[8], total[9], total[10]
    );
    for (name, st) in ["retail riding_entry", "per_node (fix 23)", "per_node + keep_facing (fix 16)"].iter().zip(stances) {
        eprintln!(
            "drawn stance of the backwards frames, {name}: drawn fakie {}, airborne after a fakie takeoff {}, not fakie: airborne {}, off board {}, ground trick span {}, first {} s after spawn {}, other {}",
            st[0], st[6], st[1], st[2], st[3], ChainConfig::retail().fakie.after_teleport_seconds, st[4], st[5]
        );
    }
    assert!(total[1] > 100 && total[2] > 100_000, "the exported lines hold switches");
    assert!(total[7] * 1000 < total[2], "fix 23 rule: forward-recorded frames ridden backwards {} of {}", total[7], total[2]);
    assert!(total[7] < total[9], "the fix 16 option rides more forward-recorded frames backwards ({} vs {})", total[7], total[9]);
}

/// Fix 21 (NPC skater pops and tricks): on the exported lines the trick slots hold `EScorableID`s
/// (-1 or 0..332, [code] `sub_8246A2E0`), most of them jump tricks (ollie 128, kickflip 96), and
/// about half of all phase changes come before a 0.2 s crossfade could finish: the puppet must
/// nest a running blend instead of restarting it.
#[test]
fn npc_skater_trick_slots_and_phase_changes_on_the_exported_lines() {
    use skate_core::living_world::replay::{node_events, ReplayLine, ReplayPhase};
    use std::collections::BTreeMap;
    let Some(paths) = find("skater_paths") else {
        eprintln!("skipped: set SKATE3_ASSET_ROOT to an export with skater_paths");
        return;
    };
    let mut tricks: BTreeMap<i16, usize> = BTreeMap::new();
    let (mut changes, mut short) = (0usize, 0usize);
    for district in ["DownTown", "Industrial", "University"] {
        let pack = std::fs::read(paths.join(format!("{district}.bin"))).unwrap();
        let tiles = aipath::parse_pack(&pack).unwrap();
        let (unique, _) = aipath::district_paths(&tiles).unwrap();
        let lines: Vec<ReplayLine> = unique.iter().map(|p| living_world::replay_line(&p.path)).collect();
        for l in &lines {
            let mut open = false;
            let mut cur: Option<(ReplayPhase, u32)> = None;
            for (i, n) in l.nodes.iter().enumerate() {
                match n.event {
                    node_events::START_TRICK => {
                        open = true;
                        *tricks.entry(l.node_trick(n)).or_default() += 1;
                    }
                    node_events::END_TRICK => open = false,
                    _ => {}
                }
                let f = if i == 0 { 0 } else { u32::from(n.frames) };
                let p = ReplayPhase::of(n.flags, open);
                match &mut cur {
                    Some((q, len)) if *q == p => *len += f,
                    _ => {
                        if let Some((_, len)) = cur.take() {
                            changes += 1;
                            short += usize::from(len < 12);
                        }
                        cur = Some((p, 0));
                    }
                }
            }
        }
    }
    let mut top: Vec<_> = tricks.iter().map(|(t, n)| (*n, *t)).collect();
    top.sort_unstable_by(|a, b| b.cmp(a));
    eprintln!("trick spans by slot id (count, id): {:?}", &top[..top.len().min(10)]);
    eprintln!("phase changes {changes}, phase shorter than 0.2 s: {short}");
    assert!(tricks.keys().all(|t| (-1..332).contains(t)), "every slot is -1 or an EScorableID");
    assert_eq!(top[0].1, 128, "ollie is the most common recorded trick");
    assert!(short * 3 > changes, "many phases are shorter than a crossfade ({short} of {changes})");
}

