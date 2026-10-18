use super::*;

fn placement(id: u32, x: f32, priority: Option<u32>) -> DmoPlacement {
    DmoPlacement { id, position: [x, 0.0, 0.0], radius: 0.5, priority, keep: false, streamable: true }
}

fn at(x: f32) -> (Observer, DmoView) {
    (Observer { position: [x, 0.0, 0.0], velocity: [0.0; 3] }, DmoView { camera: [x, 0.0, 0.0], forward: [1.0, 0.0, 0.0], reference: [x, 0.0, 0.0] })
}

const NONE: fn(u32) -> bool = |_| false;

/// Spawn within 90 m, hold up to 100 m, cull beyond it (`dynamicobjects` 0 / 90 / 100); the cull is horizontal.
#[test]
fn placements_spawn_inside_90_and_cull_beyond_100_horizontally() {
    let s = DmoStreamSettings::default();
    let mut high = placement(3, 50.0, Some(100));
    high.position[1] = 80.0;
    let mut d = DmoStream::new(vec![placement(1, 50.0, Some(100)), placement(2, 95.0, Some(100)), high], &s);
    let out = d.step(&[at(0.0)], &s, false, &NONE);
    assert_eq!(out, vec![DmoDecision::Spawn(1), DmoDecision::Spawn(3)], "95 m is outside the spawn ring; 80 m up still spawns");
    // Walk away: at 45 m the 50 m prop is 95 m behind (kept), at 60 m it is 110 m away (culled).
    assert!(d.step(&[at(-45.0)], &s, false, &NONE).is_empty());
    let out = d.step(&[at(-60.0)], &s, false, &NONE);
    assert_eq!(out, vec![DmoDecision::Cull(1), DmoDecision::Cull(3)]);
    assert!(d.live.is_empty());
}

/// Touching placements form a group: they spawn together and cull together.
#[test]
fn touching_props_spawn_and_cull_as_a_group() {
    let s = DmoStreamSettings::default();
    // 1 and 2 touch (0.8 m apart, radii 0.5), 2 is beyond the spawn ring on its own but inside the cull circle.
    let mut d = DmoStream::new(vec![placement(1, 89.6, Some(100)), placement(2, 90.4, Some(100)), placement(3, 60.0, Some(100))], &s);
    let out = d.step(&[at(0.0)], &s, false, &NONE);
    assert_eq!(out, vec![DmoDecision::Spawn(3), DmoDecision::Spawn(1), DmoDecision::Spawn(2)]);
    // Move so that 2 is beyond 100 m but 1 is not: the group goes.
    let out = d.step(&[at(-9.8)], &s, false, &NONE);
    assert_eq!(out, vec![DmoDecision::Cull(1), DmoDecision::Cull(2)]);
}

/// The score (`sub_82C4A130`): keepalways beats everything, priority x 250, closer is better, in front of the
/// camera multiplies by 2.5.
#[test]
fn the_score_follows_the_retail_weights() {
    let w = DmoWeights::default();
    let v = at(0.0).1;
    let s = |p: &DmoPlacement| score_value(score(p, &v, &w));
    let mut near = placement(1, 10.0, Some(200));
    assert_eq!(score(&near, &v, &w), [250.0 * 200.0, -300.0, 450.0 * -10.0 / 6.25, 2.5]);
    let far = placement(2, 40.0, Some(200));
    let low = placement(3, 10.0, Some(1));
    let always = placement(4, 80.0, None);
    assert!(s(&always) > s(&near) && s(&near) > s(&far) && s(&near) > s(&low));
    let behind = placement(5, -10.0, Some(200));
    assert!(s(&near) > s(&behind), "in front x2.5");
    near.keep = true;
    assert!(s(&near) >= 2f32.powi(30), "the keep flag scores like keepalways");
}

/// The pool cap: with 49 live objects one is evicted per pass, and a new placement only spawns when it outscores
/// the lowest live one; held props are never culled or evicted.
#[test]
fn the_pool_cap_evicts_the_lowest_score_once_per_pass() {
    let s = DmoStreamSettings { cap: 3, ..Default::default() };
    let mut d = DmoStream::new(vec![placement(1, 10.0, Some(1)), placement(2, 20.0, Some(200)), placement(3, 30.0, Some(200)), placement(4, 40.0, Some(450))], &s);
    let held = |id: u32| id == 1;
    let out = d.step(&[at(0.0)], &s, false, &held);
    // 4 (priority 450) outscores the farther of the two priority-200 props, which makes room.
    assert_eq!(out, vec![DmoDecision::Spawn(1), DmoDecision::Spawn(2), DmoDecision::Spawn(3), DmoDecision::Evict(3), DmoDecision::Spawn(4)]);
    // Next pass, still at the cap: one eviction, the lowest live one that is not held (2, priority 200).
    let out = d.step(&[at(0.0)], &s, false, &held);
    assert_eq!(out[0], DmoDecision::Evict(2));
    assert!(d.live.contains(&1), "held: never evicted");
}

/// The per-pass spawn budget: 49, 100 in fill mode.
#[test]
fn the_spawn_budget_is_49_per_pass_and_100_when_filling() {
    let s = DmoStreamSettings { cap: 500, ..Default::default() };
    let many: Vec<_> = (0..150).map(|i| placement(i, 1.0 + i as f32 * 0.5, Some(100))).collect();
    let mut d = DmoStream::new(many.clone(), &s);
    assert_eq!(d.step(&[at(0.0)], &s, false, &NONE).len(), 49);
    let mut f = DmoStream::new(many, &s);
    assert_eq!(f.step(&[at(0.0)], &s, true, &NONE).len(), 100);
}

/// Same inputs, same decisions.
#[test]
fn streaming_is_deterministic() {
    let s = DmoStreamSettings { cap: 10, ..Default::default() };
    let many: Vec<_> = (0..40).map(|i| placement(i, (i as f32 * 7.3) % 95.0, Some(i % 3 * 200))).collect();
    let run = || {
        let mut d = DmoStream::new(many.clone(), &s);
        (0..5).flat_map(|k| d.step(&[at(k as f32 * 10.0)], &s, false, &NONE)).collect::<Vec<_>>()
    };
    assert_eq!(run(), run());
}
