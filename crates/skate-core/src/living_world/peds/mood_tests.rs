use super::*;

const PED: u64 = 1;
const PLAYER: u64 = 99;

fn tables() -> MoodTables {
    let mut t = MoodTables::default();
    t.categories.insert("collision".into(), MoodCategory { magnitude: 0.0, lifetime: 30.0, near: 10.0, far: 10.0 });
    t.categories.insert("presence".into(), MoodCategory { magnitude: 0.5, lifetime: 30.0, near: 10.0, far: 10.0 });
    t.entity_parents.insert("skater".into(), "pedestrians".into());
    t.entity_parents.insert("adult_male".into(), "regular".into());
    t.entity_parents.insert("regular".into(), "pedestrians".into());
    let skater = || Some("skater".to_string());
    t.results.insert(
        "collisionwarn".into(),
        MoodResult { category: Some("collision".into()), instigator_type: skater(), outstanding: Some((0, 0)), suppress: 5.0, wants: vec!["warn".into()], ..Default::default() },
    );
    t.results.insert(
        "collisionchase".into(),
        MoodResult { category: Some("collision".into()), instigator_type: skater(), outstanding: Some((1, 1000)), want_flag: true, wants: vec!["angrychase".into()], ..Default::default() },
    );
    t.results.insert(
        "presencethrow".into(),
        MoodResult { category: Some("presence".into()), instigator_type: skater(), magnitude_at_least: Some(1.0), roll: Some((0.8, 10.0)), wants: vec!["throwhandprop".into()], ..Default::default() },
    );
    t.reactions.insert(
        "adult_male".into(),
        MoodReactions {
            results: vec![("collisionchase".into(), 1), ("collisionwarn".into(), 2), ("presencethrow".into(), 3)],
            prerequisites: vec![Prerequisite { category: "presence".into(), kind: 3, comparator: 3, value: 12.0 }, Prerequisite { category: "collision".into(), kind: 2, comparator: 1, value: 0.0 }],
        },
    );
    t
}

fn ctx<'a>(entity: &'a dyn Fn(u64) -> Option<(String, Vec3)>) -> MoodContext<'a> {
    MoodContext { ped: PED, ped_type: "adult_male", position: [0.0; 3], entity, zombie: false, busy: &|_| false, own_plugin_object: false, chasing: false }
}

fn collision() -> MoodEvent {
    MoodEvent { category: "collision".into(), instigator: Some(PLAYER), second: Some(PED), position: [1.0, 0.0, 0.0] }
}

#[test]
fn a_collision_warns_first_and_chases_once_a_warn_is_outstanding() {
    let t = tables();
    let near = |id: u64| (id == PLAYER).then(|| ("skater".to_string(), [1.0, 0.0, 0.0]));
    let mut store = MoodStore::default();
    let mut rng = Rng::new(7);
    store.post(collision(), 0.0);
    let r = t.produce(&mut store, &ctx(&near), false, &|_| 0, &|_| false, &mut rng).unwrap();
    assert_eq!(r.result, "collisionwarn");
    assert_eq!(r.wants, vec![RaisedWant { want: "warn".into(), target: PLAYER, flag_40: false }]);
    // The record is suppressed for 5 s: nothing more until it runs out.
    assert!(t.produce(&mut store, &ctx(&near), false, &|_| 1, &|_| false, &mut rng).is_none());
    store.tick(5.0, &|_| 30.0);
    store.post(collision(), 0.0);
    let r = t.produce(&mut store, &ctx(&near), false, &|_| 1, &|_| false, &mut rng).unwrap();
    assert_eq!((r.result.as_str(), r.wants[0].flag_40), ("collisionchase", true));
    // A pending chase want blocks the same result again.
    assert!(t.produce(&mut store, &ctx(&near), false, &|_| 1, &|w| w == "angrychase", &mut rng).map(|r| r.result) != Some("collisionchase".into()));
}

#[test]
fn presence_needs_its_magnitude_its_12_m_prerequisite_and_a_roll() {
    let t = tables();
    let mut rng = Rng::new(3);
    let ent = |id: u64| (id == PLAYER).then(|| ("skater".to_string(), [5.0, 0.0, 0.0]));
    let mut store = MoodStore::default();
    let ev = || MoodEvent { category: "presence".into(), instigator: Some(PLAYER), second: Some(PED), position: [5.0, 0.0, 0.0] };
    store.post(ev(), 0.5);
    assert!(t.produce(&mut store, &ctx(&ent), false, &|_| 0, &|_| false, &mut rng).is_none(), "0.5 s of presence is under the 1.0 threshold");
    store.post(ev(), 0.5);
    // Too far (13 m > 12 m prerequisite, measured to where the presence was posted, `sub_82BFEA60` kind 3): nothing.
    let mut far = MoodStore::default();
    for _ in 0..2 {
        far.post(MoodEvent { position: [13.0, 0.0, 0.0], ..ev() }, 0.5);
    }
    assert!(t.produce(&mut far, &ctx(&ent), false, &|_| 0, &|_| false, &mut rng).is_none());
    let r = t.produce(&mut store, &ctx(&ent), false, &|_| 0, &|_| false, &mut rng).unwrap();
    assert_eq!(r.result, "presencethrow");
    let n = r.rolled.expect("rolled");
    assert!((1..=100).contains(&n));
    assert_eq!(r.passed, n <= 80);
    // Every roll starts the cooldown: no second roll for 10 s.
    assert!(t.produce(&mut store, &ctx(&ent), false, &|_| 0, &|_| false, &mut rng).is_none());
}

#[test]
fn records_bump_age_expire_and_a_full_store_drops_targeted_events() {
    let mut s = MoodStore::default();
    assert!(s.post(collision(), 1.0));
    s.tick(5.0, &|_| 30.0);
    assert!(s.post(collision(), 1.0));
    assert_eq!((s.records.len(), s.records[0].magnitude, s.records[0].count, s.records[0].age), (1, 2.0, 2, 0.0));
    s.tick(29.0, &|_| 30.0);
    assert_eq!(s.records.len(), 1);
    s.tick(2.0, &|_| 30.0);
    assert!(s.records.is_empty());
    for i in 0..MAX_RECORDS as u64 {
        assert!(s.post(MoodEvent { instigator: Some(100 + i), ..collision() }, 0.0));
    }
    assert!(!s.post(MoodEvent { instigator: Some(7), ..collision() }, 0.0));
    assert!(s.post(MoodEvent { instigator: None, ..collision() }, 0.0));
    assert_eq!(s.records.len(), MAX_RECORDS);
}

#[test]
fn presence_sees_players_within_35_m() {
    let near = presence(PED, [0.0; 3], &[], &[(7, [30.0, 0.0, 10.0]), (8, [35.0, 0.0, 1.0])]);
    assert_eq!(near.len(), 1);
    assert_eq!((near[0].category.as_str(), near[0].instigator, near[0].second), ("presence", Some(7), Some(PED)));
}

#[test]
fn presence_posts_other_peds_then_players() {
    let peds: Vec<(u64, Vec3)> = (0..40).map(|i| (100 + i as u64, [1.0, 0.0, i as f32 * 0.5])).chain([(PED, [0.0; 3]), (200, [50.0, 0.0, 0.0])]).collect();
    let e = presence(PED, [0.0; 3], &peds, &[(7, [3.0, 0.0, 0.0])]);
    // Itself and the ped 50 m away are skipped; at most 30 peds, then the player.
    assert_eq!(e.len(), 31);
    assert!(e.iter().all(|e| e.instigator != Some(PED) && e.instigator != Some(200)));
    assert_eq!(e.last().unwrap().instigator, Some(7));
}

#[test]
fn the_zombie_broadcast_raises_the_zombie_chase_on_the_skater() {
    let mut t = tables();
    t.categories.insert("zombiemode".into(), MoodCategory { magnitude: 0.5, lifetime: 30.0, near: 10.0, far: 10.0 });
    t.results.insert(
        "skaterzombiechase".into(),
        MoodResult { category: Some("zombiemode".into()), instigator_type: Some("skater".into()), want_flag: true, wants: vec!["angrychase".into()], ..Default::default() },
    );
    t.reactions.get_mut("adult_male").unwrap().results.push(("skaterzombiechase".into(), 1));
    let far = |id: u64| (id == PLAYER).then(|| ("skater".to_string(), [40.0, 0.0, 0.0]));
    let events = zombie_mode(&[(PLAYER, [40.0, 0.0, 0.0])]);
    assert_eq!(events.len(), 1);
    assert_eq!((events[0].category.as_str(), events[0].instigator, events[0].second), (category::ZOMBIE_MODE, Some(PLAYER), None));
    let mut store = MoodStore::default();
    let mut rng = Rng::new(5);
    let c = MoodContext { zombie: true, ..ctx(&far) };
    assert!(t.accepts("adult_male", &events[0], &c), "no stock prerequisite on zombiemode, any distance");
    store.post(events[0].clone(), 0.5);
    let r = t.produce(&mut store, &c, false, &|_| 0, &|_| false, &mut rng).unwrap();
    assert_eq!(r.result, "skaterzombiechase");
    assert_eq!(r.wants, vec![RaisedWant { want: "angrychase".into(), target: PLAYER, flag_40: true }]);
}

/// `sub_82BFEA60` (research b152 M5 to M7): kind 6 refuses a bystander's `collision` (posted again as
/// `nearbycollision`, `sub_82E41060`), kind 4 reads whether the ped is chasing, kind 3 measures from the ped to the
/// record's position (not the instigator's live position), and an unknown comparator fails.
#[test]
fn prerequisites_read_the_peds_own_state_and_the_event_position() {
    let mut t = tables();
    let item = |category: &str, kind, comparator, value| Prerequisite { category: category.into(), kind, comparator, value };
    t.reactions.get_mut("adult_male").unwrap().prerequisites =
        vec![item("collision", 6, 0, 0.0), item("nearbycollision", 4, 1, 0.0), item("presence", 3, 3, 12.0), item("greeted", 9, 7, 0.0)];
    // The instigator stands 1 m away; the event happened 20 m away.
    let near = |id: u64| (id == PLAYER).then(|| ("skater".to_string(), [1.0, 0.0, 0.0]));
    let event = |category: &str, second, x: f32| MoodEvent { category: category.into(), instigator: Some(PLAYER), second: Some(second), position: [x, 0.0, 0.0] };
    let c = ctx(&near);
    assert!(t.accepts("adult_male", &event("collision", PED, 0.0), &c));
    assert!(!t.accepts("adult_male", &event("collision", 5, 0.0), &c), "a bystander's collision is refused");
    assert!(t.accepts("adult_male", &event("nearbycollision", 5, 0.0), &c));
    assert!(!t.accepts("adult_male", &event("nearbycollision", 5, 0.0), &MoodContext { chasing: true, ..ctx(&near) }));
    assert!(t.accepts("adult_male", &event("presence", 5, 11.0), &c));
    assert!(!t.accepts("adult_male", &event("presence", 5, 20.0), &c), "measured to the event, not to the instigator");
    assert!(!t.accepts("adult_male", &event("greeted", 5, 0.0), &c));
}
