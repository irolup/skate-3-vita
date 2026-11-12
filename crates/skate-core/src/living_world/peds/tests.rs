use super::anim::{ClipWindow, Intent, RemapClip, TestPath, IDENTITY};
use super::choice::{entity_index, tints, unit_f32};
use super::*;
use crate::animation::output::Sqt;
use std::collections::BTreeMap;

fn s(names: &[&str]) -> Vec<String> {
    names.iter().map(|n| n.to_string()).collect()
}

fn model(parent: Option<&str>, recipe: &str, tints_a: usize) -> PedModel {
    PedModel {
        parent: parent.map(Into::into),
        recipe: recipe.into(),
        voice: Some(50),
        tints_a: (0..tints_a).map(|i| [i as f32, 0.0, 0.0, 1.0]).collect(),
        tints_b: vec![[0.0, 0.0, 1.0, 1.0], [0.0, 1.0, 0.0, 1.0]],
        ..Default::default()
    }
}

fn catalog() -> PedCatalog {
    let mut c = PedCatalog::default();
    c.categories.insert("aletown".into(), s(&["jock02", "skater_female03", "bum02"]));
    c.categories.insert("empty".into(), vec![]);
    let e = |m: &str, a: &str| PedEntity { model: Some(m.into()), anim_set: Some(a.into()) };
    c.entities.insert("jock02".into(), e("jock02", "jock"));
    c.entities.insert("skater_female03".into(), e("skater_female", "female"));
    c.entities.insert("bum02".into(), e("bum02", "bum"));
    c.models.insert("jock02".into(), model(Some("jock"), "male_jock_2", 1));
    c.models.insert("bum02".into(), model(Some("bum"), "male_bum_2", 3));
    c.models.insert("skater_female".into(), model(Some("census_spawned"), "", 1));
    for i in 1..=3 {
        c.models.insert(format!("skater_female0{i}"), model(Some("skater_female"), &format!("female_skater_{i}"), 5));
    }
    c
}

#[test]
fn entity_index_is_the_retail_formula() {
    // sub_826BB058: u32 x 2^-32 as f32; sub_826B8B88: trunc(x 100) % count.
    assert_eq!(unit_f32(0), 0.0);
    assert_eq!(unit_f32(u32::MAX), 1.0); // rounds up in f32, as on the console
    assert_eq!(entity_index(0, 3), Some(0));
    assert_eq!(entity_index(1 << 31, 3), Some(50 % 3));
    assert_eq!(entity_index(u32::MAX, 3), Some(100 % 3));
    // u32::MAX / 4 rounds up to 2^30 in f32: exactly 0.25 -> 25.
    assert_eq!(entity_index(u32::MAX / 4, 7), Some(25 % 7));
    assert_eq!(entity_index(12345, 0), None);
    // The x100 truncation biases lists that do not divide 100: index 0 of 3 wins 34 of 100.
    let mut counts = [0u32; 3];
    for k in 0..100u32 {
        counts[entity_index(((k as u64 * (1u64 << 32)) / 100 + 4096) as u32, 3).unwrap()] += 1;
    }
    assert_eq!(counts, [34, 33, 33]);
}

#[test]
fn tints_share_one_rand() {
    let m = model(None, "x", 5);
    let (a, b) = tints(&m, 7);
    assert_eq!(a, [2.0, 0.0, 0.0, 1.0]); // 7 % 5
    assert_eq!(b, [0.0, 1.0, 0.0, 1.0]); // 7 % 2
    let none = PedModel::default();
    assert_eq!(tints(&none, 3), (choice::DEFAULT_TINT, choice::DEFAULT_TINT));
}

#[test]
fn a_mod_palette_replaces_the_model_tints_with_the_same_rule() {
    let c = catalog();
    let mut o = PedOverrides::default();
    let base = c.choose("aletown", 42, &o).unwrap();
    o.model_tints.insert(base.model.clone(), (vec![[0.9, 0.1, 0.1, 1.0]], vec![[0.1, 0.1, 0.9, 1.0]]));
    let modded = c.choose("aletown", 42, &o).unwrap();
    assert_eq!((modded.entity.as_str(), modded.model.as_str()), (base.entity.as_str(), base.model.as_str()));
    assert_eq!((modded.tint_a, modded.tint_b), ([0.9, 0.1, 0.1, 1.0], [0.1, 0.1, 0.9, 1.0]));
    // restoring the defaults (mod disabled) gives the retail look again
    assert_eq!(c.choose("aletown", 42, &PedOverrides::default()).unwrap(), base);
}

#[test]
fn the_look_follows_from_category_and_seed() {
    let c = catalog();
    let o = PedOverrides::default();
    let a = c.choose("aletown", 42, &o).unwrap();
    assert_eq!(a, c.choose("aletown", 42, &o).unwrap());
    let mut seen = BTreeMap::new();
    for seed in 0..600u64 {
        let l = c.choose("aletown", seed, &o).unwrap();
        assert!(!l.recipe.is_empty());
        *seen.entry(l.recipe.clone()).or_insert(0) += 1;
        if l.entity == "skater_female03" {
            assert!(l.model.starts_with("skater_female0"), "group record resolves to a child: {}", l.model);
            assert_eq!(l.anim_set, "female");
        }
    }
    // Every entity and every group child appears.
    for r in ["male_jock_2", "male_bum_2", "female_skater_1", "female_skater_2", "female_skater_3"] {
        assert!(seen.contains_key(r), "{r} never chosen: {seen:?}");
    }
    assert!(c.choose("empty", 1, &o).is_none());
    assert!(c.choose("nope", 1, &o).is_none());
}

#[test]
fn overrides_replace_lists_models_and_sets() {
    let c = catalog();
    let mut o = PedOverrides::default();
    o.category_entities.insert("aletown".into(), s(&["bum02"]));
    o.entity_model.insert("bum02".into(), "jock02".into());
    o.entity_anim_set.insert("bum02".into(), "zombie".into());
    for seed in 0..20 {
        let l = c.choose("aletown", seed, &o).unwrap();
        assert_eq!((l.entity.as_str(), l.recipe.as_str(), l.anim_set.as_str()), ("bum02", "male_jock_2", "zombie"));
    }
}

#[test]
fn bones_match_by_name() {
    let (map, unused) = match_bones(&s(&["Hips", "LeftFoot", "Tail"]), &s(&["TRAJECTORY", "HIPS", "LEFTFOOT"]));
    assert_eq!(map, vec![Some(1), Some(2), None]);
    assert_eq!(unused, s(&["TRAJECTORY"]));
}

// ---------------------------------------------------------------- animation player

const FPS: f32 = 30.0;

fn sqt(t: [f32; 3], yaw: f32) -> Sqt {
    Sqt { scale: [1.0; 4], rotation: [0.0, (yaw * 0.5).sin(), 0.0, (yaw * 0.5).cos()], translation: [t[0], t[1], t[2], 1.0] }
}

/// A 3-bone clip: trajectory moving at `speed` m/s (z) and turning to `turn` rad by the end,
/// hips bent by `bend` so poses differ.
fn clip(name: &str, frames: usize, speed: f32, turn: f32, looping: bool, bend: f32, windows: Vec<ClipWindow>) -> PedClip {
    let len = (frames - 1) as f32 / FPS;
    PedClip { channel_weights: None,
        name: name.into(),
        fps: FPS,
        frames: (0..frames)
            .map(|f| {
                let t = f as f32 / FPS;
                vec![sqt([0.0, 0.0, speed * t], turn * t / len.max(1e-6)), sqt([0.0, 0.0, 0.0], bend), IDENTITY]
            })
            .collect(),
        looping,
        loop_rotation: [0.0, 0.0, 0.0, 1.0],
        loop_translation: [0.0, 0.0, speed * len],
        windows,
    }
}

fn w(channel: &str, begin: f32, end: f32) -> ClipWindow {
    ClipWindow { channel: channel.into(), begin, end, value: 1.0 }
}

fn fixture() -> (PedRig, PedAnimSet, BTreeMap<String, PedClip>) {
    let rig = PedRig {
        names: s(&["TRAJECTORY", "HIPS", "HEADEND"]),
        parents: vec![-1, 0, 1],
        mirrors: vec![0, 1, -1],
        reference: vec![IDENTITY, sqt([0.0, 0.92, 0.0], 0.0), sqt([0.0, 0.5, 0.0], 0.0)],
        animated: vec![true, true, false],
    };
    let mut clips = BTreeMap::new();
    let walk_windows = vec![w("LEFTTOEDOWN", 0.25, 0.75), w("RIGHTTOEDOWN", 0.0, 0.3), w("RIGHTTOEDOWN", 0.76, 1.0)];
    for c in [
        clip("IDLE_A", 31, 0.0, 0.0, true, 0.1, vec![w("LEFTTOEDOWN", 0.0, 1.0), w("RIGHTTOEDOWN", 0.0, 1.0)]),
        clip("IDLE_B", 31, 0.0, 0.0, true, 0.2, vec![]),
        clip("WALK", 34, 1.3, 0.0, true, 0.3, walk_windows),
        clip("START", 28, 0.8, 0.0, false, 0.25, vec![]),
        clip("STOP", 30, 0.5, 0.0, false, 0.15, vec![]),
        clip("TURN", 43, 0.0, -std::f32::consts::PI, false, 0.05, vec![]),
    ] {
        clips.insert(c.name.clone(), c);
    }
    let r = |c: &str, windows: Vec<(f32, f32, i32)>| RemapClip { clip: c.into(), windows };
    let mut set = PedAnimSet::default();
    set.entries.insert(anim::names::IDLE.into(), vec![r("IDLE_A", vec![]), r("IDLE_B", vec![])]);
    set.entries.insert(anim::names::WALK.into(), vec![r("WALK", vec![(0.0, 0.051, 1), (0.4166, 0.584, -1), (1.0, 1.1, 1)])]);
    set.entries.insert(anim::names::START.into(), vec![r("START", vec![])]);
    set.entries.insert(anim::names::STOP.into(), vec![r("STOP", vec![])]);
    set.entries.insert(anim::names::TURN_180.into(), vec![r("TURN", vec![])]);
    (rig, set, clips)
}

/// Runs a player with an intent schedule at `hz`, returns (states entered, distance, yaw, feet log).
fn run(seed: u64, hz: f32, seconds: f32, schedule: &dyn Fn(f32) -> Intent) -> (Vec<Locomotion>, f32, f32, Vec<[bool; 2]>, PedAnimPlayer) {
    let (_, set, clips) = fixture();
    let mut p = PedAnimPlayer::new(&set, seed).unwrap();
    let (mut states, mut dist, mut yaw, mut feet) = (vec![p.state], 0.0, 0.0, Vec::new());
    let dt = 1.0 / hz;
    let mut t = 0.0;
    while t < seconds {
        p.intent = schedule(t);
        let o = p.step(dt, &set, &clips);
        dist += o.root.translation[2];
        yaw += o.root.yaw;
        if let Some(e) = o.entered {
            if states.last() != Some(&e) {
                states.push(e);
            }
        }
        feet.push(o.feet_down);
        t += dt;
    }
    (states, dist, yaw, feet, p)
}

#[test]
fn idle_start_walk_stop_follow_the_motion_graph() {
    let schedule = |t: f32| if (1.0..5.0).contains(&t) { Intent::Walk } else { Intent::Idle };
    let (states, dist, _, feet, p) = run(7, 30.0, 8.0, &schedule);
    assert_eq!(&states[..5], &[Locomotion::Idle, Locomotion::Start, Locomotion::Walk, Locomotion::Stop, Locomotion::Idle], "{states:?}");
    assert_eq!(p.state, Locomotion::Idle);
    // Root motion: about 0.9 s start at 0.8 m/s + about 3 s walk at 1.3 m/s + stop.
    assert!((4.0..6.5).contains(&dist), "distance {dist}");
    // Walk foot plants alternate (both feet down at times, each foot up at times).
    assert!(feet.iter().any(|f| *f == [true, false]) && feet.iter().any(|f| *f == [false, true]));
}

#[test]
fn walk_speed_is_the_clip_trajectory_at_any_step() {
    for hz in [30.0, 60.0, 144.0] {
        let (_, set, clips) = fixture();
        let mut p = PedAnimPlayer::new(&set, 1).unwrap();
        p.intent = Intent::Walk;
        let dt = 1.0 / hz;
        let mut t = 0.0;
        while p.state != Locomotion::Walk || p.blend_weight() < 1.0 {
            p.step(dt, &set, &clips);
            t += dt;
            assert!(t < 3.0);
        }
        let mut dist = 0.0;
        let steps = (3.0 * hz) as usize;
        for _ in 0..steps {
            dist += p.step(dt, &set, &clips).root.translation[2];
        }
        let speed = dist / (steps as f32 * dt);
        assert!((speed - 1.3).abs() < 0.02, "{hz} Hz: {speed} m/s (loop transform carries the wraps)");
    }
}

#[test]
fn turns_rotate_the_root_and_the_left_turn_is_mirrored() {
    let right = |t: f32| if t < 0.05 { Intent::TurnRight } else { Intent::Idle };
    let left = |t: f32| if t < 0.05 { Intent::TurnLeft } else { Intent::Idle };
    let (sr, _, yr, ..) = run(3, 30.0, 3.0, &right);
    let (sl, _, yl, ..) = run(3, 30.0, 3.0, &left);
    assert!(sr.contains(&Locomotion::TurnRight) && sl.contains(&Locomotion::TurnLeft));
    // About 180 degrees (the exit 0.1 s before the end and the blend take a little off).
    assert!((yr + std::f32::consts::PI).abs() < 0.5, "right turn yaw {yr}");
    assert!((yl - std::f32::consts::PI).abs() < 0.5, "left turn yaw {yl}");
}

#[test]
fn same_seed_same_animation_and_pose_has_reference_added() {
    let schedule = |t: f32| if (0.5..3.0).contains(&t) { Intent::Walk } else { Intent::Idle };
    let a = run(11, 30.0, 6.0, &schedule);
    let b = run(11, 30.0, 6.0, &schedule);
    assert_eq!(a.4, b.4);
    assert_eq!((a.1, a.2), (b.1, b.2));
    let (rig, _, clips) = fixture();
    let pose = a.4.pose(&rig, &clips, 0.0).unwrap();
    assert_eq!(pose[0], IDENTITY, "trajectory goes to the entity transform");
    assert!((pose[1].translation[1] - 0.92).abs() < 1e-5, "hips get the reference offset");
    assert_eq!(pose[2], rig.reference[2], "bones without clip data keep the reference");
    let g = PedEvaluator::globals(&rig, &pose);
    assert!((g[2][3][1] - 1.42).abs() < 0.05, "child global = local x parent: {:?}", g[2][3]);
}

#[test]
fn idle_cycles_swap_between_list_clips() {
    let (_, _, _, _, _) = run(5, 30.0, 0.1, &|_| Intent::Idle);
    let (_, set, clips) = fixture();
    let mut p = PedAnimPlayer::new(&set, 5).unwrap();
    let mut seen = std::collections::BTreeSet::new();
    for _ in 0..(60 * 30) {
        p.step(1.0 / 30.0, &set, &clips);
        seen.insert(p.current_clip().to_string());
    }
    assert_eq!(seen.len(), 2, "both idle clips play over a minute: {seen:?}");
}

#[test]
fn test_path_walks_turns_and_repeats() {
    let (_, set, clips) = fixture();
    let mut p = PedAnimPlayer::new(&set, 9).unwrap();
    let mut path = TestPath::new(9);
    let mut states = std::collections::BTreeSet::new();
    for _ in 0..(40 * 30) {
        p.intent = path.intent(1.0 / 30.0, p.state);
        p.step(1.0 / 30.0, &set, &clips);
        states.insert(p.state);
    }
    for s in [Locomotion::Idle, Locomotion::Start, Locomotion::Walk, Locomotion::Stop, Locomotion::TurnRight, Locomotion::TurnLeft] {
        assert!(states.contains(&s), "{s:?} never reached");
    }
}

/// A rig like the export's: the hips reference carries the 180 degree literal that trajectory
/// mode 1 mirroring multiplies in, so the reference is symmetric only as a FULL pose.
fn mirror_fixture() -> (PedRig, PedAnimSet, BTreeMap<String, PedClip>) {
    let leg = |z: f32| Sqt { scale: [1.0; 4], rotation: [0.0, 0.0, 0.0, 1.0], translation: [0.0, -0.4, z, 1.0] };
    let rig = PedRig {
        names: s(&["TRAJECTORY", "HIPS", "LEFTUPLEG", "RIGHTUPLEG"]),
        parents: vec![-1, 0, 1, 1],
        mirrors: vec![0, 1, 3, 2],
        reference: vec![IDENTITY, Sqt { scale: [1.0; 4], rotation: [0.5, 0.5, -0.5, -0.5], translation: [0.0, 0.92, 0.0, 1.0] }, leg(0.1), leg(-0.1)],
        animated: vec![true, true, true, true],
    };
    let bend = |a: f32| Sqt { scale: [1.0; 4], rotation: [(a * 0.5).sin(), 0.0, 0.0, (a * 0.5).cos()], translation: [0.0, 0.0, 0.0, 1.0] };
    let still = |name: &str, frames: usize, turn: f32, looping: bool| PedClip { channel_weights: None,
        name: name.into(),
        fps: FPS,
        frames: (0..frames)
            .map(|f| {
                let t = f as f32 / (frames - 1) as f32;
                // Hips delta zero; only the left leg moves (an asymmetric clip).
                vec![sqt([0.0, 0.0, 0.0], turn * t), IDENTITY, bend(0.6 * t), IDENTITY]
            })
            .collect(),
        looping,
        loop_rotation: [0.0, 0.0, 0.0, 1.0],
        loop_translation: [0.0; 3],
        windows: vec![],
    };
    let mut clips = BTreeMap::new();
    clips.insert("IDLE".into(), still("IDLE", 31, 0.0, true));
    clips.insert("TURN".into(), still("TURN", 43, -std::f32::consts::PI, false));
    let r = |c: &str| vec![RemapClip { clip: c.into(), windows: vec![] }];
    let mut set = PedAnimSet::default();
    set.entries.insert(anim::names::IDLE.into(), r("IDLE"));
    set.entries.insert(anim::names::TURN_180.into(), r("TURN"));
    (rig, set, clips)
}

fn angle(a: [f32; 4], b: [f32; 4]) -> f32 {
    let d: f32 = (0..4).map(|k| a[k] * b[k]).sum();
    2.0 * d.abs().min(1.0).acos()
}

#[test]
fn the_mirrored_turn_mirrors_the_full_pose_and_never_flips_the_hips() {
    let (rig, set, clips) = mirror_fixture();
    // Precondition: the reference is symmetric under the full-pose mirror.
    let mut m = rig.reference.clone();
    crate::animation::pose_mirror::mirror(&mut m, &rig.parents, &rig.mirrors, 1).unwrap();
    for (a, b) in m.iter().zip(&rig.reference).skip(1) {
        assert!(angle(a.rotation, b.rotation) < 1e-4);
    }
    let dt = 1.0 / 60.0;
    let poses = |intent: Intent| {
        let mut p = PedAnimPlayer::new(&set, 1).unwrap();
        p.intent = intent;
        let mut out = Vec::new();
        for _ in 0..40 {
            p.step(dt, &set, &clips);
            p.intent = Intent::Idle;
            out.push((p.blend_weight(), p.pose(&rig, &clips, 0.0).unwrap()));
        }
        out
    };
    let right = poses(Intent::TurnRight);
    let left = poses(Intent::TurnLeft);
    for (k, ((wl, l), (wr, r))) in left.iter().zip(&right).enumerate() {
        assert_eq!(wl, wr);
        // The hips delta is zero in every clip: mid-blend or not, the hips stay at the reference
        // (the old delta mirror turned them 180 degrees: the ped stood on its head).
        assert!(angle(l[1].rotation, rig.reference[1].rotation) < 1e-3, "tick {k} weight {wl}: hips {:?}", l[1].rotation);
        // Out of the blend, the left turn is the right turn's full pose mirrored (in the blend the
        // outgoing idle layer is not mirrored).
        if *wl < 1.0 {
            continue;
        }
        let mut mr = r.clone();
        crate::animation::pose_mirror::mirror(&mut mr, &rig.parents, &rig.mirrors, 1).unwrap();
        for b in 1..4 {
            assert!(angle(l[b].rotation, mr[b].rotation) < 1e-3, "tick {k} bone {b}: {:?} vs {:?}", l[b].rotation, mr[b].rotation);
            assert!((0..3).all(|i| (l[b].translation[i] - mr[b].translation[i]).abs() < 1e-5), "tick {k} bone {b} translation");
        }
    }
    // And the clip really is asymmetric: the left leg moves in the right turn, the right leg in the left.
    let (_, r) = &right[39];
    let (_, l) = &left[39];
    assert!(angle(r[2].rotation, rig.reference[2].rotation) > 0.1 && angle(r[3].rotation, rig.reference[3].rotation) < 1e-4);
    assert!(angle(l[3].rotation, rig.reference[3].rotation) > 0.1 && angle(l[2].rotation, rig.reference[2].rotation) < 1e-4);
}

#[test]
fn a_knock_down_falls_lies_for_the_ground_time_gets_up_and_walks_on() {
    use super::skater_contact::{reaction_steps, ReactionDirection, ReactionKind};
    let (_, mut set, mut clips) = fixture();
    let r = |c: &str| vec![RemapClip { clip: c.into(), windows: vec![] }];
    for (name, frames, looping) in [("FALL", 31, false), ("GROUND", 21, true), ("GETUP", 31, false)] {
        clips.insert(name.into(), clip(name, frames, 0.0, 0.0, looping, 0.0, vec![]));
    }
    set.entries.insert("WipeoutBackFall".into(), r("FALL"));
    set.entries.insert("WipeoutBackGroundCyc".into(), r("GROUND"));
    set.entries.insert("WipeoutBackGetUp".into(), r("GETUP"));
    let mut p = PedAnimPlayer::new(&set, 3).unwrap();
    assert!(p.react(&set, reaction_steps(ReactionKind::Knockdown, ReactionDirection::FromFront), 1.5));
    assert!(!p.react(&set, reaction_steps(ReactionKind::Standing, ReactionDirection::FromFront), 1.5), "no second reaction while one runs");
    let mut log = Vec::new();
    let dt = 1.0 / 60.0;
    for _ in 0..(6.0 / dt) as usize {
        p.intent = Intent::Walk;
        p.step(dt, &set, &clips);
        log.push((p.state, p.current_clip().to_owned()));
    }
    let first = |c: &str| log.iter().position(|(_, x)| x == c).unwrap_or_else(|| panic!("{c} never played"));
    let (fall, ground, getup) = (first("FALL"), first("GROUND"), first("GETUP"));
    assert!(fall < ground && ground < getup);
    // The fall clip is 1 s; retail's ground time (1.5 s) counts from the hit (b372), so the get-up starts
    // 1.5 s after it and the ground cycle lasts about 0.5 s.
    assert!((89..=91).contains(&(getup - fall)), "get-up {} frames after the hit", getup - fall);
    let ground_frames = getup - ground;
    assert!((28..=32).contains(&ground_frames), "{ground_frames} frames on the ground");
    assert!(log[..getup].iter().all(|(s, _)| *s == Locomotion::Reaction), "no walking while reacting");
    assert!(log.iter().skip(getup).any(|(s, _)| *s != Locomotion::Reaction), "back to locomotion after the get-up");
    assert_eq!(p.reaction_anim(), None);
}

/// Retail's knock-down Colliding state machine (b372): a contact while down re-arms the ground timer at its expiry
/// (the get-up comes one ground time later); a contact while getting up re-hits from the posture the get-up clip has
/// reached; more than `block_after` expiries block contacts.
#[test]
fn a_knocked_down_ped_stays_down_when_hit_again_and_is_rehit_while_getting_up() {
    use super::anim::KnockdownContact;
    use super::skater_contact::{reaction_steps, Posture, ReactionDirection, ReactionKind};
    let (_, mut set, mut clips) = fixture();
    let r = |c: &str| vec![RemapClip { clip: c.into(), windows: vec![] }];
    for (name, frames, looping) in [("FALL", 31, false), ("GROUND", 21, true), ("GETUP", 121, false), ("CROUCH", 31, false)] {
        clips.insert(name.into(), clip(name, frames, 0.0, 0.0, looping, 0.0, vec![]));
    }
    set.entries.insert("WipeoutBackFall".into(), r("FALL"));
    set.entries.insert("WipeoutBackGroundCyc".into(), r("GROUND"));
    set.entries.insert("WipeoutBackGetUp".into(), r("GETUP"));
    set.entries.insert("WipeoutBackCrouch".into(), r("CROUCH"));
    set.entries.insert("WipeoutLeftCrouch".into(), r("CROUCH"));
    let dt = 1.0 / 60.0;
    let knock = |p: &mut PedAnimPlayer| {
        assert!(p.react(&set, reaction_steps(ReactionKind::Knockdown, ReactionDirection::FromFront), 1.5));
        p.begin_knockdown(ReactionDirection::FromFront);
    };
    let run_until = |p: &mut PedAnimPlayer, clip: &str, limit: usize| -> usize {
        (0..limit).find(|_| {
            p.intent = Intent::Walk;
            p.step(dt, &set, &clips);
            p.current_clip() == clip
        }).unwrap_or(limit)
    };

    // Posture choice (`sub_8269E460`) by the stored direction's intervals.
    let rules = set.collision;
    assert_eq!(rules.posture(1.9, ReactionDirection::FromFront), Posture::LyingDown);
    assert_eq!(rules.posture(2.5, ReactionDirection::FromFront), Posture::Crouched);
    assert_eq!(rules.posture(3.3, ReactionDirection::FromBack), Posture::Standing);
    assert_eq!(rules.posture(1.7, ReactionDirection::FromLeft), Posture::Crouched);

    // A contact while down waits for the expiry and re-arms the timer: the get-up comes 3.0 s after the hit.
    let mut p = PedAnimPlayer::new(&set, 3).unwrap();
    knock(&mut p);
    let before = run_until(&mut p, "GROUND", 600);
    assert_eq!(p.knockdown_contact(&set, ReactionDirection::FromBack), KnockdownContact::Pending);
    let getup = before + 1 + run_until(&mut p, "GETUP", 600);
    assert!((179..=181).contains(&getup), "get-up {getup} ticks after the hit");

    // A contact while getting up re-hits: early in the get-up lying down (fall restarted, stored direction), later
    // crouched (the crouch clip, the new direction).
    let mut p = PedAnimPlayer::new(&set, 3).unwrap();
    knock(&mut p);
    run_until(&mut p, "GETUP", 600);
    assert_eq!(p.knockdown_contact(&set, ReactionDirection::FromBack), KnockdownContact::ReHit(Posture::LyingDown, ReactionDirection::FromFront));
    assert_eq!(p.current_clip(), "FALL");
    assert!((p.current_time() - rules.ground_restart[0]).abs() < 1e-4, "fall restarted at {}", p.current_time());
    run_until(&mut p, "GETUP", 600);
    for _ in 0..(2.2 / dt) as usize {
        p.step(dt, &set, &clips);
    }
    assert_eq!(p.knockdown_contact(&set, ReactionDirection::FromLeft), KnockdownContact::ReHit(Posture::Crouched, ReactionDirection::FromLeft));
    assert_eq!(p.reaction_anim(), Some("WipeoutLeftCrouch"));

    // Contacts at every expiry keep the ped down until more than `block_after` expiries, then are dropped.
    let mut p = PedAnimPlayer::new(&set, 3).unwrap();
    knock(&mut p);
    let mut results = Vec::new();
    for _ in 0..(12.0 / dt) as usize {
        if p.knocked_down() && p.reaction_anim() != Some("WipeoutBackGetUp") {
            let r = p.knockdown_contact(&set, ReactionDirection::FromFront);
            if results.last() != Some(&r) {
                results.push(r);
            }
        }
        p.step(dt, &set, &clips);
    }
    assert_eq!(results, vec![KnockdownContact::Pending, KnockdownContact::Ignored], "blocked after {} expiries", rules.block_after);
}

/// The sit plugin state (`motiongraph_sit.xml` via `plugin_motion`): Stand2Sit, then SitIdleCyc held for as long as the
/// packet stays on its first stage, then (released on StandUp) Sit2Stand and back to locomotion.
#[test]
fn a_sitting_ped_holds_the_idle_until_released_then_stands_up() {
    use super::plugin_motion::{motion_for, BLEND};
    use super::skater_contact::ReactionStep;
    let (_, mut set, mut clips) = fixture();
    let r = |c: &str| vec![RemapClip { clip: c.into(), windows: vec![] }];
    for (name, frames, looping) in [("SITDOWN", 31, false), ("SITIDLE", 21, true), ("STANDUP", 31, false)] {
        clips.insert(name.into(), clip(name, frames, 0.0, 0.0, looping, 0.0, vec![]));
    }
    set.entries.insert("Stand2Sit".into(), r("SITDOWN"));
    set.entries.insert("SitIdleCyc".into(), r("SITIDLE"));
    set.entries.insert("Sit2Stand".into(), r("STANDUP"));
    let m = motion_for("Sit").unwrap();
    let steps = m.steps.iter().map(|s| ReactionStep { anim: s.anim, mirror: false, blend: BLEND, cycle: s.hold_until.is_some() }).collect();
    let mut p = PedAnimPlayer::new(&set, 3).unwrap();
    assert!(p.react(&set, steps, f32::INFINITY));
    let dt = 1.0 / 60.0;
    for _ in 0..600 {
        p.step(dt, &set, &clips);
    }
    assert_eq!(p.current_clip(), "SITIDLE", "still seated after 10 s");
    p.release_hold();
    let mut log = Vec::new();
    for _ in 0..120 {
        p.step(dt, &set, &clips);
        log.push(p.current_clip().to_owned());
    }
    assert!(log.iter().any(|c| c == "STANDUP"));
    assert!(!p.reacting(), "the run ended after the stand-up");
}

/// The Sit intent ends while the ped is still in Stand2Sit (its seat was carried away). Retail's sit state is
/// non-interruptable: Stand2Sit plays out, SittingCyc leaves on "not HasIntent Sit" to Sit2Stand. A single release
/// during Stand2Sit is lost (the ped sat forever, session 2026-10-13 11:23); the game keeps releasing until the cycle.
#[test]
fn a_sit_ended_during_stand2sit_stands_up_once_seated() {
    use super::plugin_motion::{motion_for, BLEND};
    use super::skater_contact::ReactionStep;
    let (_, mut set, mut clips) = fixture();
    let r = |c: &str| vec![RemapClip { clip: c.into(), windows: vec![] }];
    for (name, frames, looping) in [("SITDOWN", 31, false), ("SITIDLE", 21, true), ("STANDUP", 31, false)] {
        clips.insert(name.into(), clip(name, frames, 0.0, 0.0, looping, 0.0, vec![]));
    }
    set.entries.insert("Stand2Sit".into(), r("SITDOWN"));
    set.entries.insert("SitIdleCyc".into(), r("SITIDLE"));
    set.entries.insert("Sit2Stand".into(), r("STANDUP"));
    let m = motion_for("Sit").unwrap();
    let steps: Vec<ReactionStep> = m.steps.iter().map(|s| ReactionStep { anim: s.anim, mirror: false, blend: BLEND, cycle: s.hold_until.is_some() }).collect();
    let dt = 1.0 / 60.0;
    for keep_releasing in [false, true] {
        let mut p = PedAnimPlayer::new(&set, 3).unwrap();
        assert!(p.react(&set, steps.clone(), f32::INFINITY));
        p.step(dt, &set, &clips);
        assert_eq!(p.reaction_anim(), Some("Stand2Sit"));
        p.release_hold();
        let mut log = Vec::new();
        for _ in 0..600 {
            if keep_releasing {
                p.release_hold();
            }
            p.step(dt, &set, &clips);
            log.push(p.current_clip().to_owned());
        }
        if keep_releasing {
            assert!(log.iter().any(|c| c == "STANDUP") && !p.reacting(), "sat down, stood up, run ended");
        } else {
            assert_eq!(p.current_clip(), "SITIDLE", "one early release is lost: still seated");
        }
    }
}

/// The ped channel (b99, `82E32878` / `82E3A5A0`): a carry clip fades in over 0.2 s and takes over only the bones its
/// channel weights pick (here the hips), the rest keeps locomotion; a stop fades it out and frees the slot.
#[test]
fn the_channel_overrides_only_its_weighted_bones_and_fades() {
    let (rig, mut set, mut clips) = fixture();
    let mut carry = clip("CARRY", 3, 0.0, 0.0, true, 1.0, vec![]);
    carry.channel_weights = Some(vec![0.0, 1.0, 0.0]);
    clips.insert("CARRY".into(), carry);
    set.entries.insert("CarrySmallRHChannel".into(), vec![RemapClip { clip: "CARRY".into(), windows: vec![] }]);
    let mut p = PedAnimPlayer::new(&set, 1).unwrap();
    let base = p.pose(&rig, &clips, 0.0).unwrap();
    assert!(p.channel_idle());
    assert!(p.channel_request(&set, "CarrySmallRHChannel", 0.2, 0.2));
    assert!(!p.channel_request(&set, "NoSuchChannel", 0.2, 0.2) && p.channel_clip() == Some("CARRY"), "unknown name: nothing replaced");
    p.step(0.1, &set, &clips);
    let half = p.pose(&rig, &clips, 0.0).unwrap();
    for _ in 0..3 {
        p.step(0.1, &set, &clips);
    }
    let full = p.pose(&rig, &clips, 0.0).unwrap();
    let yaw = |q: [f32; 4]| 2.0 * q[1].atan2(q[3]);
    assert!((yaw(full[1].rotation) - 1.0).abs() < 1e-3, "hips take the carry pose: {:?}", full[1].rotation);
    let h = yaw(half[1].rotation);
    assert!(h > yaw(base[1].rotation) + 0.1 && h < 0.95, "half way through the fade: {h}");
    assert_eq!(full[2], base[2], "a zero-weight bone keeps locomotion");
    assert_eq!(full[1].translation[3], 1.0, "the weight lane does not leak into the pose");
    p.channel_stop(0.2);
    for _ in 0..3 {
        p.step(0.1, &set, &clips);
    }
    assert!(p.channel_idle(), "the stop fade freed the slot");
}

/// Turn buckets (`82E2A500`, b167): a standing ped plays the bucket's stand turn, a walking ped the bucket's walk turn
/// in its branch window and then walks on (`blendedturn`); left turns are the mirrored right clip.
#[test]
fn turn_buckets_pick_the_stand_and_walk_turn_clips() {
    let (_, mut set, mut clips) = fixture();
    let r = |c: &str| vec![RemapClip { clip: c.into(), windows: vec![] }];
    for c in [clip("TURN90", 30, 0.0, -std::f32::consts::FRAC_PI_2, false, 0.05, vec![]), clip("WTURN45", 30, 1.3, -std::f32::consts::FRAC_PI_4, false, 0.05, vec![])] {
        clips.insert(c.name.clone(), c);
    }
    set.entries.insert("StandTurnR90".into(), r("TURN90"));
    set.entries.insert("WalkTurnR45".into(), r("WTURN45"));
    let dt = 1.0 / 30.0;
    // Standing, 90 degrees right.
    let mut p = PedAnimPlayer::new(&set, 3).unwrap();
    p.turn_degrees = 90;
    p.intent = Intent::TurnRight;
    let mut yaw = 0.0;
    p.step(dt, &set, &clips);
    assert_eq!((p.state, p.current_clip()), (Locomotion::TurnRight, "TURN90"));
    p.intent = Intent::Idle;
    for _ in 0..60 {
        yaw += p.step(dt, &set, &clips).root.yaw;
    }
    assert_eq!(p.state, Locomotion::Idle);
    assert!((yaw + std::f32::consts::FRAC_PI_2).abs() < 0.3, "right 90 yaw {yaw}");
    // Walking, 45 degrees left: the mirrored walk turn, then the walk cycle again.
    let mut p = PedAnimPlayer::new(&set, 3).unwrap();
    p.intent = Intent::Walk;
    for _ in 0..90 {
        p.step(dt, &set, &clips);
    }
    assert_eq!(p.state, Locomotion::Walk);
    p.turn_degrees = 45;
    p.intent = Intent::TurnLeft;
    let mut entered = false;
    for _ in 0..60 {
        p.step(dt, &set, &clips);
        if p.state == Locomotion::WalkTurnLeft {
            entered = true;
            assert!(p.mirrored() && p.current_clip() == "WTURN45");
            p.intent = Intent::Walk;
        }
    }
    assert!(entered, "the walk turn played in the branch window");
    assert_eq!(p.state, Locomotion::Walk, "back on the walk cycle");
}

#[test]
fn reachable_recipes_hold_every_look_a_census_spawn_can_get() {
    let c = catalog();
    let mut o = PedOverrides::default();
    let reachable = c.reachable_recipes(&o);
    // The group record's three children, the two plain models; the empty category adds none.
    let want: std::collections::BTreeSet<String> = ["male_jock_2", "male_bum_2", "female_skater_1", "female_skater_2", "female_skater_3"].map(String::from).into();
    assert_eq!(reachable, want);
    for seed in 0..2000 {
        let look = c.choose("aletown", seed, &o).unwrap();
        assert!(reachable.contains(&look.recipe), "seed {seed}: {} missing", look.recipe);
    }
    // A mod's category list replaces the stock one, and a mod's entity model is followed.
    o.category_entities.insert("aletown".into(), s(&["bum02"]));
    o.entity_model.insert("bum02".into(), "jock02".into());
    assert_eq!(c.reachable_recipes(&o), ["male_jock_2".to_string()].into());
    for seed in 0..200 {
        assert!(c.reachable_recipes(&o).contains(&c.choose("aletown", seed, &o).unwrap().recipe));
    }
}

/// A seated avoid that keeps the seat (SitSmall / ReturnToSit, `avoid::seated`): the clip plays out of the held sit
/// cycle and the sit sequence goes on from there (held again until released, then the stand-up).
#[test]
fn a_seated_step_avoid_returns_to_the_held_sit_cycle() {
    use super::skater_contact::ReactionStep;
    let (_, mut set, mut clips) = fixture();
    let r = |c: &str| vec![RemapClip { clip: c.into(), windows: vec![] }];
    for (name, frames, looping) in [("TOSIT", 31, false), ("SITCYC", 31, true), ("TOSTAND", 31, false), ("FLINCH", 31, false)] {
        clips.insert(name.into(), clip(name, frames, 0.0, 0.0, looping, 0.0, vec![]));
    }
    set.entries.insert("Stand2Sit".into(), r("TOSIT"));
    set.entries.insert("SitIdleCyc".into(), r("SITCYC"));
    set.entries.insert("Sit2Stand".into(), r("TOSTAND"));
    set.entries.insert("SitAvoidSmallFront".into(), r("FLINCH"));
    let step = |anim, cycle| ReactionStep { anim, mirror: false, blend: 0.1, cycle };
    let mut p = PedAnimPlayer::new(&set, 3).unwrap();
    assert!(p.react(&set, vec![step("Stand2Sit", false), step("SitIdleCyc", true), step("Sit2Stand", false)], f32::INFINITY));
    let dt = 1.0 / 60.0;
    let run = |p: &mut PedAnimPlayer, seconds: f32| {
        for _ in 0..(seconds / dt) as usize {
            p.step(dt, &set, &clips);
        }
    };
    run(&mut p, 2.0);
    assert_eq!(p.reaction_anim(), Some("SitIdleCyc"));
    let mut steps = vec![step("SitAvoidSmallFront", false)];
    steps.extend(p.remaining_steps());
    assert!(p.interrupt(&set, steps, f32::INFINITY));
    assert_eq!(p.reaction_anim(), Some("SitAvoidSmallFront"));
    run(&mut p, 1.5);
    assert_eq!(p.reaction_anim(), Some("SitIdleCyc"), "back in the held sit cycle");
    run(&mut p, 3.0);
    assert_eq!(p.reaction_anim(), Some("SitIdleCyc"), "still held");
    p.release_hold();
    run(&mut p, 0.2);
    assert_eq!(p.reaction_anim(), Some("Sit2Stand"));
}

fn rows(spec: &[(f32, &str, anim::Tier)]) -> Vec<anim::SpeedRow> {
    spec.iter().map(|&(threshold, state, tier)| anim::SpeedRow { threshold, state: state.into(), tier, turn_from: Vec::new() }).collect()
}

/// The stock default and female rows (b682): a speed above 1.0 gets +0.25, then the highest row at or below it.
#[test]
fn the_speed_rows_pick_state_and_tier_after_the_bump() {
    use anim::Tier::*;
    let default = rows(&[(0.0, "Idle", Default), (1.0, "Shuffle", Default), (2.0, "Walk", Default), (4.0, "Walk", Brisk), (6.0, "ChaseRun", Slow), (8.0, "ChaseRun", Default), (10.0, "ChaseRun", Fast)]);
    let bump = anim::SpeedBump::default();
    let pick = |r: &[anim::SpeedRow], s| anim::speed_row(r, s, bump).map(|r| (r.state.clone(), r.tier));
    assert_eq!(pick(&default, 0.5), Some(("Idle".into(), Default)));
    assert_eq!(pick(&default, 1.0), Some(("Shuffle".into(), Default)), "no bump at 1.0");
    assert_eq!(pick(&default, 1.7), Some(("Shuffle".into(), Default)));
    assert_eq!(pick(&default, 1.75), Some(("Walk".into(), Default)));
    assert_eq!(pick(&default, 3.8), Some(("Walk".into(), Brisk)));
    assert_eq!(pick(&default, 5.8), Some(("ChaseRun".into(), Slow)));
    assert_eq!(pick(&default, 7.8), Some(("ChaseRun".into(), Default)));
    assert_eq!(pick(&default, 9.8), Some(("ChaseRun".into(), Fast)));
    let female = rows(&[(0.0, "Idle", Default), (1.0, "Shuffle", Default), (2.0, "Walk", Default), (4.0, "Walk", Brisk), (8.0, "ChaseRun", Slow)]);
    assert_eq!(pick(&female, 6.0), Some(("Walk".into(), Brisk)));
    assert_eq!(pick(&female, 10.0), Some(("ChaseRun".into(), Slow)));
    // Intents: a speed decides; a flee without one runs the untiered run row, else the first one.
    assert_eq!(anim::moving_intent(Intent::Walk, true, Some(3.0), &default, bump), (Intent::Walk, Default));
    assert_eq!(anim::moving_intent(Intent::Walk, false, Some(9.8), &default, bump), (Intent::Run, Fast));
    assert_eq!(anim::moving_intent(Intent::Walk, true, None, &default, bump), (Intent::Run, Default));
    assert_eq!(anim::moving_intent(Intent::Walk, true, None, &female, bump), (Intent::Run, Slow));
    assert_eq!(anim::moving_intent(Intent::Walk, true, Some(9.0), &[], bump), (Intent::Run, Default));
    assert_eq!(anim::moving_intent(Intent::TurnLeft, true, Some(9.0), &default, bump), (Intent::TurnLeft, Default));
}

/// `WalkCyc.Brisk` / `Default`: a tier change waits for the walk's branch window, then swaps the cycle.
#[test]
fn a_tier_change_swaps_the_walk_cycle_at_the_branch_window() {
    let (_, mut set, mut clips) = fixture();
    let walk = clips["WALK"].clone();
    clips.insert("BRISK".into(), PedClip { name: "BRISK".into(), ..walk });
    set.entries.insert(anim::names::BRISK_WALK.into(), vec![RemapClip { clip: "BRISK".into(), windows: vec![(0.0, 0.051, 1), (0.4166, 0.584, -1), (1.0, 1.1, 1)] }]);
    let mut p = PedAnimPlayer::new(&set, 3).unwrap();
    let dt = 1.0 / 30.0;
    p.intent = Intent::Walk;
    for _ in 0..60 {
        p.step(dt, &set, &clips);
    }
    assert_eq!((p.state, p.current_clip()), (Locomotion::Walk, "WALK"));
    p.tier = anim::Tier::Brisk;
    for _ in 0..60 {
        p.step(dt, &set, &clips);
    }
    assert_eq!((p.state, p.current_clip()), (Locomotion::Walk, "BRISK"));
    p.tier = anim::Tier::Slow;
    for _ in 0..60 {
        p.step(dt, &set, &clips);
    }
    assert_eq!(p.current_clip(), "WALK", "Slow is a run tier: the walk plays its default cycle");
}

/// `motiongraph_chaserun.xml` / `motiongraph_idle.xml` (b684): Idle -> `Stand2ChaseRun` -> the run cycle, a turn
/// intent plays the run turn (left mirrored) back into the cycle, Idle stops through `ChaseRun2Stand`, Walk goes
/// back through `Stand2Walk`.
#[test]
fn a_run_starts_turns_and_stops_through_its_own_clips() {
    let (_, mut set, mut clips) = fixture();
    let r = |c: &str| vec![RemapClip { clip: c.into(), windows: vec![] }];
    for (logical, clip_name, frames, speed, looping) in [
        (anim::names::RUN_START, "RUNSTART", 20, 2.0, false),
        (anim::names::RUN, "RUN", 24, 6.0, true),
        (anim::names::run_turn(90), "RUNTURN", 20, 4.0, false),
        (anim::names::RUN_STOP, "RUNSTOP", 20, 1.0, false),
    ] {
        clips.insert(clip_name.into(), clip(clip_name, frames, speed, 0.0, looping, 0.1, vec![]));
        set.entries.insert(logical.into(), r(clip_name));
    }
    let mut p = PedAnimPlayer::new(&set, 5).unwrap();
    let dt = 1.0 / 30.0;
    let mut states = vec![p.state];
    let mut step = |p: &mut PedAnimPlayer, intent: Intent, n: usize, states: &mut Vec<Locomotion>| {
        p.intent = intent;
        for _ in 0..n {
            if let Some(e) = p.step(dt, &set, &clips).entered {
                if states.last() != Some(&e) {
                    states.push(e);
                }
            }
        }
    };
    step(&mut p, Intent::Run, 40, &mut states);
    assert_eq!(p.current_clip(), "RUN");
    p.turn_degrees = 90;
    step(&mut p, Intent::TurnLeft, 1, &mut states);
    assert_eq!((p.state, p.current_clip()), (Locomotion::RunTurnLeft, "RUNTURN"));
    step(&mut p, Intent::Run, 30, &mut states);
    step(&mut p, Intent::Idle, 40, &mut states);
    assert_eq!(
        states,
        vec![Locomotion::Idle, Locomotion::RunStart, Locomotion::Run, Locomotion::RunTurnLeft, Locomotion::Run, Locomotion::Stop, Locomotion::Idle],
    );
    step(&mut p, Intent::Run, 40, &mut states);
    step(&mut p, Intent::Walk, 2, &mut states);
    assert_eq!(p.state, Locomotion::Start, "run -> walk plays Stand2Walk");
}

/// `motiongraph_tired.xml` (b684): resting from the run plays ChaseTiredInto at the run cycle's branch window, then the
/// tired cycle until the rest ends (StopTired at a loop end), ChaseTiredOut, and a walk starts its cycle directly.
#[test]
fn a_rest_after_a_run_plays_the_tired_clips_then_walks_on() {
    let (_, mut set, mut clips) = fixture();
    let r = |c: &str| vec![RemapClip { clip: c.into(), windows: vec![] }];
    for (logical, name, frames, looping) in [(anim::names::RUN, "RUN", 24, true), (anim::names::TIRED_INTO, "TINTO", 20, false), (anim::names::TIRED_CYCLE, "TCYC", 30, true), (anim::names::TIRED_OUT, "TOUT", 20, false)] {
        clips.insert(name.into(), clip(name, frames, 0.0, 0.0, looping, 0.1, vec![]));
        set.entries.insert(logical.into(), r(name));
    }
    let mut p = PedAnimPlayer::new(&set, 5).unwrap();
    let dt = 1.0 / 30.0;
    let mut states = vec![p.state];
    let mut step = |p: &mut PedAnimPlayer, n: usize, states: &mut Vec<Locomotion>| {
        for _ in 0..n {
            if let Some(e) = p.step(dt, &set, &clips).entered {
                if states.last() != Some(&e) {
                    states.push(e);
                }
            }
        }
    };
    p.intent = Intent::Run;
    step(&mut p, 20, &mut states);
    p.resting = true;
    p.intent = Intent::Idle;
    step(&mut p, 120, &mut states);
    assert_eq!(p.state, Locomotion::TiredCycle, "tired until rested: {states:?}");
    p.resting = false;
    p.intent = Intent::Walk;
    step(&mut p, 60, &mut states);
    assert_eq!(&states[..], &[Locomotion::Idle, Locomotion::Run, Locomotion::TiredInto, Locomotion::TiredCycle, Locomotion::TiredOut, Locomotion::Walk]);
}

/// `motiongraph_shuffle.xml` (b686): a speed in the Shuffle row shuffles: `Stand2Shuffle`, the shuffle cycle, a turn
/// plays the shuffle turn, Idle stops through `Shuffle2Stand`; TargetWaypoint's 1.0 picks the row (no bump at 1.0).
#[test]
fn a_shuffle_speed_shuffles_turns_and_stops_through_its_clips() {
    use anim::Tier::Default as D;
    let default = rows(&[(0.0, "Idle", D), (1.0, "Shuffle", D), (2.0, "Walk", D)]);
    assert_eq!(anim::moving_intent(Intent::Walk, false, Some(1.0), &default, anim::SpeedBump::default()), (Intent::Shuffle, D));
    let (_, mut set, mut clips) = fixture();
    let r = |c: &str| vec![RemapClip { clip: c.into(), windows: vec![] }];
    for (logical, name, looping) in [(anim::names::SHUFFLE_START, "SHSTART", false), (anim::names::SHUFFLE, "SHUFFLE", true), (anim::names::shuffle_turn(45), "SHTURN", false), (anim::names::SHUFFLE_STOP, "SHSTOP", false)] {
        clips.insert(name.into(), clip(name, 20, 0.5, 0.0, looping, 0.1, vec![]));
        set.entries.insert(logical.into(), r(name));
    }
    let mut p = PedAnimPlayer::new(&set, 5).unwrap();
    let dt = 1.0 / 30.0;
    let mut states = vec![p.state];
    let mut step = |p: &mut PedAnimPlayer, intent: Intent, n: usize, states: &mut Vec<Locomotion>| {
        p.intent = intent;
        for _ in 0..n {
            if let Some(e) = p.step(dt, &set, &clips).entered {
                if states.last() != Some(&e) {
                    states.push(e);
                }
            }
        }
    };
    step(&mut p, Intent::Shuffle, 30, &mut states);
    p.turn_degrees = 45;
    step(&mut p, Intent::TurnRight, 1, &mut states);
    step(&mut p, Intent::Shuffle, 30, &mut states);
    step(&mut p, Intent::Idle, 40, &mut states);
    assert_eq!(states, vec![Locomotion::Idle, Locomotion::ShuffleStart, Locomotion::Shuffle, Locomotion::ShuffleTurnRight, Locomotion::Shuffle, Locomotion::Stop, Locomotion::Idle]);
}

/// `sub_82E2ADE0` / `sub_82E2A500` mask 0xCC / `sub_82E2AFC0` (b686): a target 25 degrees or more off the facing turns
/// (90 from 70 up, nothing larger); facing it, the step is picked on the 25 / 70 / 115 / 160 edges; no speed, no step.
#[test]
fn the_strafe_turns_to_the_target_then_steps_by_angle() {
    use anim::{strafe_input, StrafeMove, StrafeRules};
    let r = StrafeRules::default();
    // Facing +z (heading 0); +x is the ped's left (positive error = left; retail: cross(facing, dir).y > 0).
    assert_eq!(strafe_input(0.0, [-1.0, 0.0, 0.0], None, 2.0, &r).turn, Some((90, false)));
    assert_eq!(strafe_input(0.0, [1.0, 0.0, -1.0], None, 2.0, &r).turn, Some((90, true)), "135 off falls to 90");
    assert_eq!(strafe_input(0.0, [0.5, 0.0, 1.0], None, 2.0, &r).turn, Some((45, true)));
    // `sub_82E2AB18` is 3D: a chasee 0.5 m up 1 m ahead is 26.6 degrees off, a 45 turn (b695).
    assert_eq!(strafe_input(0.0, [0.0, 0.5, 1.0], None, 2.0, &r).turn, Some((45, false)));
    let facing = |step: [f32; 2]| strafe_input(0.0, [0.0, 0.0, 1.0], Some(step), 2.0, &r);
    assert_eq!(facing([0.0, 1.0]).step, Some(StrafeMove::Forwards));
    assert_eq!(facing([0.0, -1.0]).step, Some(StrafeMove::Backwards));
    assert_eq!(facing([1.0, 0.0]).step, Some(StrafeMove::Side { degrees: 90, left: true }));
    assert_eq!(facing([-1.0, 1.0]).step, Some(StrafeMove::Side { degrees: 45, left: false }));
    assert_eq!(facing([-1.0, -1.0]).step, Some(StrafeMove::Side { degrees: 135, left: false }));
    assert_eq!(strafe_input(0.0, [0.0, 0.0, 1.0], Some([1.0, 0.0]), 0.0, &r).step, None);
}

/// `motiongraph_strafe.xml` (b686): Idle -> `Stand2Strafe` -> `StrafeIdle`, a step plays its clip (left mirrored), a
/// turn plays the strafe turn; the flag cleared goes straight back to the idle cycle (no Strafe2Stand).
#[test]
fn a_strafing_ped_steps_turns_and_leaves_without_a_stand_clip() {
    use anim::{StrafeInput, StrafeMove};
    let (_, mut set, mut clips) = fixture();
    let r = |c: &str| vec![RemapClip { clip: c.into(), windows: vec![] }];
    for (logical, name, looping) in [(anim::names::STRAFE_INTO, "STIN", false), (anim::names::STRAFE_IDLE, "STIDLE", true), ("StrafeR90", "STR90", true), ("StrafeTurnR45", "STTURN", false)] {
        clips.insert(name.into(), clip(name, 20, 0.3, 0.0, looping, 0.1, vec![]));
        set.entries.insert(logical.into(), r(name));
    }
    let mut p = PedAnimPlayer::new(&set, 5).unwrap();
    let dt = 1.0 / 30.0;
    let mut states = vec![p.state];
    let mut step = |p: &mut PedAnimPlayer, input: Option<StrafeInput>, n: usize, states: &mut Vec<Locomotion>| {
        p.strafe = input;
        for _ in 0..n {
            if let Some(e) = p.step(dt, &set, &clips).entered {
                if states.last() != Some(&e) {
                    states.push(e);
                }
            }
        }
    };
    let stand = StrafeInput::default();
    step(&mut p, Some(stand), 30, &mut states);
    step(&mut p, Some(StrafeInput { turn: None, step: Some(StrafeMove::Side { degrees: 90, left: true }) }), 1, &mut states);
    assert_eq!((p.state, p.current_clip()), (Locomotion::StrafeMove, "STR90"));
    step(&mut p, Some(StrafeInput { turn: Some((45, false)), step: None }), 1, &mut states);
    step(&mut p, Some(stand), 30, &mut states);
    step(&mut p, None, 2, &mut states);
    assert_eq!(states, vec![Locomotion::Idle, Locomotion::StrafeInto, Locomotion::StrafeIdle, Locomotion::StrafeMove, Locomotion::StrafeTurnRight, Locomotion::StrafeIdle, Locomotion::Idle]);
}
