use super::*;

fn frame() -> Frame {
    Frame { dt: 1.0 / 30.0, current: None, last: None, state_times: Vec::new() }
}

fn ops(names: &[(&str, &[(&str, &str)])]) -> Vec<PedOp> {
    names
        .iter()
        .map(|(name, attrs)| {
            let text = |k: &str| attrs.iter().find(|a| a.0 == k).map(|a| a.1.to_string());
            let float = |k: &str| attrs.iter().find(|a| a.0 == k).and_then(|a| a.1.parse().ok());
            PedOp::parse(name, &text, &float)
        })
        .collect()
}

#[test]
fn operations_parse_their_attributes_and_unknown_names_are_pending() {
    let v = ops(&[("DistanceToWantTarget", &[("want", "Flee"), ("greater", "15.0")]), ("ChannelWarnWantTarget", &[("want", "warn")]), ("Taze", &[])]);
    assert_eq!(v[0], PedOp::DistanceToWantTarget { want: "flee".into(), greater: Some(15.0), less: None });
    assert_eq!(v[1], PedOp::ChannelWarnWantTarget { want: "warn".into(), deactivate: true });
    assert!(v[2].is_pending());
}

#[test]
fn want_conditions_read_the_brain_and_the_target_distance() {
    let conditions = ops(&[("HasSpecificWantThatNeedsToBeAddressed", &[("want", "flee")]), ("DistanceToWantTarget", &[("want", "flee"), ("greater", "15.0")])]);
    let mut brain = PedBrain::default();
    let settings = BrainSettings::default();
    let at = std::cell::Cell::new([10.0f32, 0.0, 0.0]);
    let target = |_: u64| Some(at.get());
    let mut h = BrainHost { behaviors: &[], conditions: &conditions, brain: &mut brain, settings: &settings, position: [0.0; 3], heading: 0.0, skater: None, target_position: &target, chase: Default::default() };
    assert_eq!(h.condition_activation(0, &frame()), 0);
    h.brain.set_want("Flee", 7);
    assert_eq!(h.condition_activation(0, &frame()), 1);
    // 10 m is not farther than 15 m; 16 m is (retail compares squared distances).
    assert_eq!(h.condition_activation(1, &frame()), 0);
    at.set([16.0, 0.0, 0.0]);
    assert_eq!(h.condition_activation(1, &frame()), 1);
}

#[test]
fn warn_holds_the_want_for_retails_three_and_a_half_seconds() {
    let behaviors = ops(&[("ChannelWarnWantTarget", &[("want", "warn")])]);
    let mut brain = PedBrain::default();
    brain.set_want("warn", 1);
    let settings = BrainSettings::default();
    let none = |_: u64| None;
    let f = frame();
    {
        let mut h = BrainHost { behaviors: &behaviors, conditions: &[], brain: &mut brain, settings: &settings, position: [0.0; 3], heading: 0.0, skater: None, target_position: &none, chase: Default::default() };
        h.begin(0, [0; 6], &f);
        h.update(0, [0; 6], &f);
    }
    assert!(brain.wants.contains_key("warn"));
    brain.tick_timers(3.4);
    {
        let mut h = BrainHost { behaviors: &behaviors, conditions: &[], brain: &mut brain, settings: &settings, position: [0.0; 3], heading: 0.0, skater: None, target_position: &none, chase: Default::default() };
        h.update(0, [0; 6], &f);
    }
    assert!(brain.wants.contains_key("warn"));
    brain.tick_timers(0.2);
    let mut h = BrainHost { behaviors: &behaviors, conditions: &[], brain: &mut brain, settings: &settings, position: [0.0; 3], heading: 0.0, skater: None, target_position: &none, chase: Default::default() };
    h.update(0, [0; 6], &f);
    assert!(!brain.wants.contains_key("warn"));
}

#[test]
fn speech_ops_store_the_value_on_the_ped() {
    // `SendSpeechEvent` truncates its float attribute (`fctiwz`); the warn sends 53 from the code.
    let behaviors = ops(&[("SendSpeechEvent", &[("speechevent", "PedestrianFlee"), ("speechvalue", "20.9")]), ("ChannelWarnWantTarget", &[("want", "warn")]), ("SendSpeechEvent", &[])]);
    assert_eq!(behaviors[0], PedOp::SendSpeech { value: 20 });
    assert_eq!(behaviors[2], PedOp::SendSpeech { value: 0 });
    let mut brain = PedBrain::default();
    // A fresh ped holds the constructor's 68, which speaks nothing.
    assert_eq!(brain.speech, None);
    let settings = BrainSettings::default();
    let none = |_: u64| None;
    let f = frame();
    let mut h = BrainHost { behaviors: &behaviors, conditions: &[], brain: &mut brain, settings: &settings, position: [0.0; 3], heading: 0.0, skater: None, target_position: &none, chase: Default::default() };
    h.begin(0, [0; 6], &f);
    assert_eq!(h.brain.speech, Some(20));
    // Begin only: Update and End leave the value.
    h.update(0, [0; 6], &f);
    h.end(0, [0; 6], &f);
    assert_eq!(h.brain.speech, Some(20));
    h.begin(1, [0; 6], &f);
    assert_eq!(h.brain.speech, Some(53));
}

#[test]
fn wander_and_flee_set_the_body_outputs() {
    let behaviors = ops(&[("Wander", &[]), ("Flee", &[]), ("SuggestVelocity", &[("linear", "3.0")])]);
    let mut brain = PedBrain { honker: Some(4), ..Default::default() };
    brain.set_want("flee", 9);
    let settings = BrainSettings::default();
    let none = |_: u64| None;
    let f = frame();
    let mut h = BrainHost { behaviors: &behaviors, conditions: &[], brain: &mut brain, settings: &settings, position: [0.0; 3], heading: 0.0, skater: None, target_position: &none, chase: Default::default() };
    h.begin(0, [0; 6], &f);
    h.update(0, [0; 6], &f);
    assert_eq!((h.brain.honker, h.brain.motion_intent, h.brain.speed_suggestion), (None, Some(motion::WANDER), Some(2.0)));
    h.begin(1, [0; 6], &f);
    assert_eq!((h.brain.motion_intent, h.brain.flee_from), (Some(motion::FLEE), Some(9)));
    h.end(1, [0; 6], &f);
    assert_eq!(h.brain.flee_from, None);
    h.begin(2, [0; 6], &f);
    assert_eq!(h.brain.speed_suggestion, Some(3.0));
    h.end(2, [0; 6], &f);
    assert_eq!(h.brain.speed_suggestion, None);
}

#[test]
fn stop_and_face_and_watch_follow_retails_rules() {
    let behaviors = ops(&[("StopAndFaceWantTarget", &[("want", "warn")]), ("WatchWantTargetWithoutInterruptingLocomotion", &[("want", "warn")]), ("StandAndWatchSkater", &[])]);
    let mut brain = PedBrain { speed_suggestion: Some(1.4), ..Default::default() };
    brain.set_want("warn", 5);
    let settings = BrainSettings::default();
    let skater = std::cell::Cell::new([0.0f32, 0.0, 10.0]);
    let target = |_: u64| Some(skater.get());
    let f = frame();
    // Stop and face: speed 0 and the target, then the saved speed back.
    let mut h = BrainHost { behaviors: &behaviors, conditions: &[], brain: &mut brain, settings: &settings, position: [0.0; 3], heading: 0.0, skater: None, target_position: &target, chase: Default::default() };
    h.begin(0, [0; 6], &f);
    h.update(0, [0; 6], &f);
    assert_eq!((h.brain.speed_suggestion, h.brain.face), (Some(0.0), Some([0.0, 0.0, 10.0])));
    h.end(0, [0; 6], &f);
    assert_eq!((h.brain.speed_suggestion, h.brain.face), (Some(1.4), None));
    // Watch: ahead (inside 55 deg) keeps walking; behind latches into stop and face.
    h.begin(1, [0; 6], &f);
    h.update(1, [0; 6], &f);
    assert_eq!((h.brain.speed_suggestion, h.brain.face), (Some(1.4), None));
    skater.set([0.0, 0.0, -10.0]);
    h.update(1, [0; 6], &f);
    assert_eq!(h.brain.speed_suggestion, Some(0.0));
    skater.set([0.0, 0.0, 10.0]);
    h.update(1, [0; 6], &f);
    assert_eq!(h.brain.speed_suggestion, Some(0.0), "latched until End");
    h.end(1, [0; 6], &f);
    assert_eq!(h.brain.speed_suggestion, Some(1.4));
    // Stand and watch: 4 m ahead first, then the skater's predicted position once it is off to the side.
    h.skater = Some(([10.0, 0.0, 0.0], [0.0, 0.0, 1.0]));
    h.begin(2, [0; 6], &f);
    assert_eq!(h.brain.watch_point, Some([0.0, 0.0, 4.0]));
    h.update(2, [0; 6], &f);
    assert_eq!(h.brain.face, Some([10.0, 0.0, 2.0]));
}

#[test]
fn simple_timers_follow_the_retail_timer_map() {
    let behaviors = ops(&[("SetSimpleTimer", &[("timerName", "StartChase"), ("length", "1.25")]), ("SetSimpleTimer", &[("timerName", "StartChase")])]);
    let conditions = ops(&[("SimpleTimerExpired", &[("timerName", "StartChase")]), ("SimpleTimerExpired", &[])]);
    assert_eq!(behaviors[0], PedOp::SetSimpleTimer { timer: 21, length: 1.25 });
    // No name: "TimedRangeRandom" (8); no length: 0.0.
    assert_eq!(conditions[1], PedOp::SimpleTimerExpired { timer: 8 });
    let mut brain = PedBrain::default();
    let settings = BrainSettings::default();
    let none = |_: u64| None;
    let f = frame();
    {
        let mut h = BrainHost { behaviors: &behaviors, conditions: &conditions, brain: &mut brain, settings: &settings, position: [0.0; 3], heading: 0.0, skater: None, target_position: &none, chase: Default::default() };
        // A timer that is not running reads 0: expired.
        assert_eq!(h.condition_activation(0, &f), 1);
        h.begin(0, [0; 6], &f);
        assert_eq!(h.condition_activation(0, &f), 0);
    }
    brain.tick_timers(1.25);
    assert_eq!(brain.timer(21), 0.0);
    // Length 0 stops the timer.
    brain.set_timer(21, 1.0);
    let mut h = BrainHost { behaviors: &behaviors, conditions: &conditions, brain: &mut brain, settings: &settings, position: [0.0; 3], heading: 0.0, skater: None, target_position: &none, chase: Default::default() };
    h.begin(1, [0; 6], &f);
    assert!(!h.brain.timers.contains_key(&21));
    // At most 14 running timers; a running one can still be restarted.
    for t in 0..20 {
        brain.set_timer(t, 5.0);
    }
    assert_eq!(brain.timers.len(), timers::CAPACITY);
    brain.set_timer(0, 9.0);
    assert_eq!(brain.timer(0), 9.0);
    assert_eq!(timers::index("TargetUnreachableTimer"), 42);
    assert_eq!(timers::index("nope"), 47);
}

#[test]
fn chase_conditions_read_the_chasee_and_the_record() {
    let conditions = ops(&[("IsChasing", &[]), ("ChaseeEscaped", &[]), ("CanNewChaseStart", &[])]);
    let behaviors = ops(&[("StartChase", &[]), ("InterceptChasee", &[])]);
    let mut record = super::super::chase::ChaseRecord::default();
    record.fields.insert("escape_distance".into(), 65.0);
    let mut brain = PedBrain::default();
    let mut settings = BrainSettings::default();
    let at = |_: u64| Some([0.0f32, 0.0, 70.0]);
    let f = frame();
    {
        let mut h = BrainHost { behaviors: &behaviors, conditions: &conditions, brain: &mut brain, settings: &settings, position: [0.0; 3], heading: 0.0, skater: None, target_position: &at, chase: ChaseView { record: Some(&record), ..Default::default() } };
        assert_eq!((h.condition_activation(0, &f), h.condition_activation(1, &f), h.condition_activation(2, &f)), (0, 0, 1));
        h.brain.chasee = Some(9);
        // 70 m is past the record's 65 m.
        assert_eq!((h.condition_activation(0, &f), h.condition_activation(1, &f)), (1, 1));
        h.update(0, [0; 6], &f);
        assert_eq!(h.brain.speech, Some(55));
        h.begin(1, [0; 6], &f);
        assert_eq!(h.brain.motion_intent, Some(motion::INTERCEPT));
    }
    settings.chases_enabled = false;
    let mut h = BrainHost { behaviors: &behaviors, conditions: &conditions, brain: &mut brain, settings: &settings, position: [0.0, 0.0, 10.0], heading: 0.0, skater: None, target_position: &at, chase: ChaseView { record: Some(&record), ..Default::default() } };
    assert_eq!((h.condition_activation(1, &f), h.condition_activation(2, &f)), (0, 0));
}

#[test]
fn chase_group_ops_ask_the_host_and_read_the_group() {
    use super::super::chase::ChaseGroupInfo;
    let conditions = ops(&[("CanChaseeAddNewChaser", &[]), ("IsPrimaryChaser", &[]), ("IsChaserEndingChase", &[]), ("ChaserShouldGiveUpDueToTakedowns", &[])]);
    let behaviors = ops(&[("NewChasee", &[]), ("GiveUpBeingPrimaryChaser", &[("timeout", "5.0")]), ("ChaserEndChase", &[("reason", "lostinterest")]), ("ChaserGroupEndChase", &[("reason", "aggressivecapture")]), ("EndChase", &[])]);
    assert_eq!(behaviors[2], PedOp::ChaserEndChase { reason: 3 });
    let mut record = super::super::chase::ChaseRecord::default();
    record.fields.insert("give_up_after_takedowns".into(), 2.0);
    let info = std::cell::Cell::new(None::<ChaseGroupInfo>);
    let groups = |_: u64| info.get();
    let mut brain = PedBrain::default();
    brain.set_want("angrychase", 100);
    let settings = BrainSettings::default();
    let none = |_: u64| None;
    let f = frame();
    let view = ChaseView { me: 7, record: Some(&record), groups: Some(&groups), ..Default::default() };
    let mut h = BrainHost { behaviors: &behaviors, conditions: &conditions, brain: &mut brain, settings: &settings, position: [0.0; 3], heading: 0.0, skater: None, target_position: &none, chase: view };
    // No group yet: the target takes chasers.
    assert_eq!(h.condition_activation(0, &f), 1);
    h.begin(0, [0; 6], &f);
    assert_eq!((h.brain.chasee, h.brain.speech), (Some(100), Some(15)));
    assert_eq!(h.brain.chase_requests, vec![ChaseRequest::Join { chasee: 100 }]);
    info.set(Some(ChaseGroupInfo { count: 2, primary: Some(7), end_reason: None, can_add: false }));
    assert_eq!((h.condition_activation(0, &f), h.condition_activation(1, &f), h.condition_activation(2, &f)), (0, 1, 0));
    h.begin(1, [0; 6], &f);
    assert_eq!(h.brain.timer(PRIMARY_TIMER), 5.0);
    h.begin(2, [0; 6], &f);
    assert_eq!(h.condition_activation(2, &f), 1);
    h.brain.takedowns = 2;
    assert_eq!(h.condition_activation(3, &f), 1);
    h.begin(3, [0; 6], &f);
    h.begin(4, [0; 6], &f);
    assert_eq!(h.brain.chasee, None);
    assert_eq!(
        h.brain.chase_requests[1..],
        [ChaseRequest::GiveUpPrimary { chasee: 100 }, ChaseRequest::GroupEnd { chasee: 100, reason: 1 }, ChaseRequest::Leave { chasee: 100 }]
    );
}

#[test]
fn related_wants_and_nav_modifiers_follow_retail() {
    let behaviors = ops(&[
        ("ActivateRelatedWant", &[("originalWant", "JoinChase"), ("relatedWant", "AngryChase")]),
        ("ChangeNavModifierSetting", &[("modifier", "Pedestrian"), ("set_to", "false")]),
    ]);
    assert_eq!(behaviors[1], PedOp::ChangeNavModifierSetting { modifier: nav_modifier::PEDESTRIAN, set_to: false });
    let mut brain = PedBrain::default();
    brain.set_want("joinchase", 42);
    let settings = BrainSettings::default();
    let none = |_: u64| None;
    let f = frame();
    let mut h = BrainHost { behaviors: &behaviors, conditions: &[], brain: &mut brain, settings: &settings, position: [0.0; 3], heading: 0.0, skater: None, target_position: &none, chase: Default::default() };
    h.begin(0, [0; 6], &f);
    assert_eq!(h.brain.wants["angrychase"], Want { target: 42, needs_addressing: true, forget_on_unset: false });
    assert!(!h.brain.wants["joinchase"].needs_addressing);
    assert!(h.brain.nav_modifier(nav_modifier::PEDESTRIAN));
    h.begin(1, [0; 6], &f);
    assert!(!h.brain.nav_modifier(nav_modifier::PEDESTRIAN));
    h.end(1, [0; 6], &f);
    assert!(h.brain.nav_modifier(nav_modifier::PEDESTRIAN));
}

#[test]
fn chase_state_messages_go_to_the_chased_or_warned_target() {
    let behaviors = ops(&[("SendChaseStateMessage", &[("stateName", "warn")]), ("SendChaseStateMessage", &[("stateName", "tired")])]);
    assert_eq!(behaviors[1], PedOp::SendChaseStateMessage { state: 2 });
    let mut brain = PedBrain::default();
    brain.set_want("warn", 5);
    let settings = BrainSettings::default();
    let none = |_: u64| None;
    let f = frame();
    let mut h = BrainHost { behaviors: &behaviors, conditions: &[], brain: &mut brain, settings: &settings, position: [0.0; 3], heading: 0.0, skater: None, target_position: &none, chase: Default::default() };
    // No chasee: only `warn` falls back to the warn want's target.
    h.begin(0, [0; 6], &f);
    h.begin(1, [0; 6], &f);
    assert_eq!(h.brain.chase_requests, vec![ChaseRequest::StateMessage { target: 5, state: 0 }]);
    h.brain.chasee = Some(9);
    h.brain.zombie = true;
    h.begin(1, [0; 6], &f);
    assert_eq!(h.brain.chase_requests.len(), 1, "zombie mode sends nothing");
}

#[test]
fn a_takedown_attempt_succeeds_on_contact_with_the_target() {
    use super::super::takedown::TakedownChoice;
    let conditions = ops(&[("HasTakeDownTargetable", &[]), ("CanAttemptTakeDownTargetable", &[]), ("HasMonitoredIntent", &[("intentName", "ActiveTakedown")]), ("TakeDownAttemptSuccessful", &[])]);
    let behaviors = ops(&[("ChaserEvaluateWhoToTakeDown", &[]), ("TakeDownTargetablePredictions", &[]), ("AttemptTakeDownTargetable", &[]), ("TakeDownTargetableSuccess", &[]), ("TakedownTargetableClearOnEnd", &[])]);
    let choice = TakedownChoice { entry: 2, mirrored: false, predicted: [0.0, 0.0, 1.0] };
    let choose = |_: u64| Some(choice);
    let at = |_: u64| Some([0.0f32, 0.0, 1.0]);
    let mut brain = PedBrain { chasee: Some(9), ..Default::default() };
    let settings = BrainSettings::default();
    let f = frame();
    let mut h = BrainHost { behaviors: &behaviors, conditions: &conditions, brain: &mut brain, settings: &settings, position: [0.0; 3], heading: 0.0, skater: None, target_position: &at, chase: ChaseView { takedowns: Some(&choose), ..Default::default() } };
    h.update(0, [0; 6], &f);
    h.update(1, [0; 6], &f);
    assert_eq!((h.condition_activation(0, &f), h.condition_activation(1, &f)), (1, 1));
    h.begin(2, [0; 6], &f);
    assert_eq!(h.condition_activation(2, &f), 1);
    h.update(2, [0; 6], &f);
    assert_eq!(h.condition_activation(3, &f), 0, "no contact yet");
    h.brain.takedown_contact = true;
    h.update(2, [0; 6], &f);
    assert_eq!(h.condition_activation(3, &f), 1);
    h.begin(3, [0; 6], &f);
    assert_eq!(h.brain.chase_requests, vec![ChaseRequest::Takedown { target: 9 }]);
    assert!(!h.brain.takedown_active);
    h.end(4, [0; 6], &f);
    assert_eq!((h.brain.takedown_target, h.brain.takedown_choice), (None, None));
}

#[test]
fn chasing_exhausts_and_resting_recovers() {
    let conditions = ops(&[("NeedToRest", &[]), ("InvestigateTimeExceeded", &[])]);
    let behaviors = ops(&[("InterceptChasee", &[]), ("RestFromChase", &[]), ("StartInvestigateTimer", &[]), ("EndInvestigateTimer", &[])]);
    let mut record = super::super::chase::ChaseRecord::default();
    for (k, v) in [("exhaustion_limit", 30.0), ("Hash_59604F998608081E", 5.0), ("Hash_1AE29113978BE5AA", 10.0)] {
        record.fields.insert(k.into(), v);
    }
    let mut brain = PedBrain::default();
    let settings = BrainSettings::default();
    let none = |_: u64| None;
    let f = Frame { dt: 1.0, ..frame() };
    {
        let mut h = BrainHost { behaviors: &behaviors, conditions: &conditions, brain: &mut brain, settings: &settings, position: [0.0; 3], heading: 0.0, skater: None, target_position: &none, chase: ChaseView { record: Some(&record), ..Default::default() } };
        for _ in 0..30 {
            h.update(0, [0; 6], &f);
        }
        assert_eq!(h.condition_activation(0, &f), 0, "30 s is not past the 30 s limit");
        h.update(0, [0; 6], &f);
        assert_eq!(h.condition_activation(0, &f), 1);
        h.begin(1, [0; 6], &f);
        // Resting: needs rest until timer 4 (5 s) runs out.
        assert_eq!(h.condition_activation(0, &f), 1);
        h.begin(2, [0; 6], &f);
        assert_eq!(h.condition_activation(1, &f), 0);
    }
    brain.tick_timers(5.0);
    let mut h = BrainHost { behaviors: &behaviors, conditions: &conditions, brain: &mut brain, settings: &settings, position: [0.0; 3], heading: 0.0, skater: None, target_position: &none, chase: ChaseView { record: Some(&record), ..Default::default() } };
    assert_eq!(h.condition_activation(0, &f), 0);
    h.end(1, [0; 6], &f);
    assert!(!h.brain.chase_resting);
    assert_eq!(h.condition_activation(1, &f), 0, "investigate: 10 s, 5 s left");
    h.end(3, [0; 6], &f);
    assert_eq!(h.condition_activation(1, &f), 1);
}

#[test]
fn a_tazer_is_drawn_then_hits_its_target_once() {
    let conditions = ops(&[("HasDrawnTazer", &[]), ("IsFacingWantTarget", &[("want", "taze"), ("angle", "20.0"), ("distance", "10.0")])]);
    let behaviors = ops(&[("DrawTazer", &[("want", "taze")]), ("TazeWantTarget", &[("want", "taze")])]);
    let mut brain = PedBrain::default();
    brain.set_want("taze", 3);
    let settings = BrainSettings::default();
    let ahead = |_: u64| Some([0.5f32, 0.0, 8.0]);
    let f = frame();
    let mut h = BrainHost { behaviors: &behaviors, conditions: &conditions, brain: &mut brain, settings: &settings, position: [0.0; 3], heading: 0.0, skater: None, target_position: &ahead, chase: Default::default() };
    // 8 m ahead, 3.6 deg off: inside 10 m and the 20 deg cone.
    assert_eq!(h.condition_activation(1, &f), 1);
    h.begin(0, [0; 6], &f);
    h.update(0, [0; 6], &f);
    assert_eq!(h.condition_activation(0, &f), 0, "still drawing");
    h.brain.tick_timers(0.2);
    h.update(0, [0; 6], &f);
    assert_eq!(h.condition_activation(0, &f), 1);
    h.begin(1, [0; 6], &f);
    assert_eq!(h.brain.speech, Some(66));
    h.update(1, [0; 6], &f);
    assert!(h.brain.chase_requests.is_empty(), "0.3 s before the hit");
    h.brain.tick_timers(0.3);
    h.update(1, [0; 6], &f);
    h.update(1, [0; 6], &f);
    assert_eq!(h.brain.chase_requests, vec![ChaseRequest::Taze { target: 3 }]);
}

#[test]
fn a_greet_posts_greeted_once_and_speaks_until_the_timer_runs_out() {
    let behaviors = ops(&[
        ("ChannelGreetWantTarget", &[("want", "greet"), ("unsetWant", "false")]),
        ("ChannelGreetWantTarget", &[("want", "returngreet"), ("returnGreet", "true")]),
        ("ChannelWarnWantTarget", &[("want", "angrychase"), ("deactivateWant", "false")]),
    ]);
    // `unsetWant` is not read: deactivateWant stays true.
    assert_eq!(behaviors[0], PedOp::ChannelGreetWantTarget { want: "greet".into(), deactivate: true, return_greet: false });
    let mut brain = PedBrain::default();
    brain.set_want("greet", 12);
    brain.set_want("returngreet", 12);
    brain.set_want("angrychase", 12);
    let settings = BrainSettings::default();
    let at = |_: u64| Some([1.0f32, 0.0, 1.0]);
    let f = frame();
    let mut h = BrainHost { behaviors: &behaviors, conditions: &[], brain: &mut brain, settings: &settings, position: [0.0; 3], heading: 0.0, skater: None, target_position: &at, chase: Default::default() };
    h.begin(0, [0; 6], &f);
    h.update(0, [0; 6], &f);
    assert_eq!((h.brain.speech, h.brain.chase_requests.clone()), (Some(56), vec![ChaseRequest::Greeted { target: 12 }]));
    h.begin(1, [0; 6], &f);
    h.update(1, [0; 6], &f);
    assert_eq!((h.brain.speech, h.brain.chase_requests.len()), (Some(63), 1), "a return greet posts nothing");
    h.brain.tick_timers(3.5);
    h.update(0, [0; 6], &f);
    assert!(!h.brain.wants.contains_key("greet"));
    // The unreachable chase's warn keeps its want (deactivateWant false).
    h.begin(2, [0; 6], &f);
    h.brain.tick_timers(3.5);
    h.update(2, [0; 6], &f);
    assert!(h.brain.wants.contains_key("angrychase"));
}

#[test]
fn run_from_honker_posts_its_intent_and_keeps_the_honker() {
    let behaviors = ops(&[("RunFromHonker", &[("timeout", "30")])]);
    let mut brain = PedBrain { honker: Some(4), ..Default::default() };
    let settings = BrainSettings::default();
    let none = |_: u64| None;
    let f = frame();
    let mut h = BrainHost { behaviors: &behaviors, conditions: &[], brain: &mut brain, settings: &settings, position: [0.0; 3], heading: 0.0, skater: None, target_position: &none, chase: Default::default() };
    h.begin(0, [0; 6], &f);
    h.update(0, [0; 6], &f);
    h.end(0, [0; 6], &f);
    assert_eq!((h.brain.motion_intent, h.brain.honker), (Some(motion::RUN_FROM_HONKER), Some(4)), "only Wander clears the honker");
}

#[test]
fn the_taunt_faces_the_victim_holds_the_intent_and_unsets_the_want() {
    let behaviors = ops(&[("TakedownTauntVictim", &[])]);
    let mut brain = PedBrain::default();
    brain.set_want("taunt", 7);
    let settings = BrainSettings::default();
    let at = |id: u64| (id == 7).then_some([1.0, 0.0, 2.0]);
    let f = frame();
    let mut h = BrainHost { behaviors: &behaviors, conditions: &[], brain: &mut brain, settings: &settings, position: [0.0; 3], heading: 0.0, skater: None, target_position: &at, chase: Default::default() };
    h.begin(0, [0; 6], &f);
    assert_eq!(h.brain.face, Some([1.0, 0.0, 2.0]));
    assert!(h.brain.monitored.contains_key("SGIntent"));
    assert_eq!(h.brain.chase_requests, vec![ChaseRequest::Taunt { target: 7 }]);
    h.end(0, [0; 6], &f);
    assert!(!h.brain.wants.contains_key("taunt") && h.brain.face.is_none() && !h.brain.monitored.contains_key("SGIntent"));
}

/// ThrowHandPropAtWantTarget (`826A7A58` / `826A7AA8`): Begin aims at the want's target with its velocity and plays the
/// attack clip; Update keeps the want while the prop is held and through the flight (timer 35), then unsets it.
#[test]
fn the_attack_throw_aims_at_the_want_target_and_unsets_the_want_after_the_flight() {
    use crate::living_world::peds::hand_prop::THROW_REACTION_TIMER;
    let behaviors = ops(&[("ThrowHandPropAtWantTarget", &[("want", "throwhandprop")])]);
    assert_eq!(behaviors[0], PedOp::ThrowHandPropAtWantTarget { want: "throwhandprop".into() });
    let mut brain = PedBrain::default();
    brain.set_want("throwhandprop", 7);
    brain.hand_prop.holding = true;
    let settings = BrainSettings { hand_prop: crate::living_world::peds::hand_prop::HandPropSettings { jitter_max: 0.0, ..Default::default() }, ..Default::default() };
    let at = |id: u64| (id == 7).then_some([0.0, 0.0, 8.0]);
    let velocity = |id: u64| (id == 7).then_some([0.0, 0.0, 3.0]);
    let f = frame();
    let chase = ChaseView { velocity: Some(&velocity), ..Default::default() };
    let mut h = BrainHost { behaviors: &behaviors, conditions: &[], brain: &mut brain, settings: &settings, position: [0.0; 3], heading: 0.0, skater: None, target_position: &at, chase };
    h.begin(0, [0; 6], &f);
    assert_eq!(h.brain.chase_requests, vec![ChaseRequest::HandPropClip { clip: "HandPropAttackThrow" }]);
    // Straight ahead and running away: the target 2.5 m further after 0.8333 s, then the 10 m/s prop's catch-up time.
    let target = h.brain.hand_prop.throw.expect("throw started").target;
    let t = 10.5 / 7.0;
    assert!(target[0] == 0.0 && (target[2] - (10.5 + 3.0 * t)).abs() < 1e-3 && target[1] == 1.0, "{target:?}");
    h.update(0, [0; 6], &f);
    assert!(h.brain.wants.contains_key("throwhandprop"), "still held");
    h.brain.tick_timers(0.84);
    assert!(h.brain.update_hand_prop_release(&settings.hand_prop, [0.2, 1.2, 0.3]).is_some());
    h.update(0, [0; 6], &f);
    assert!(h.brain.timer(THROW_REACTION_TIMER) > 0.0 && h.brain.wants.contains_key("throwhandprop"), "in flight");
    h.brain.tick_timers(5.0);
    h.update(0, [0; 6], &f);
    assert!(!h.brain.wants.contains_key("throwhandprop"));
}

/// The starting hand prop (`82E33198`): the chance roll first (`8269A588`: chance x 100 >= roll), then the first entry
/// whose running total x 100 reaches the pick roll; weights summing below 1 can give nothing.
#[test]
fn the_starting_hand_prop_follows_the_chance_and_the_running_total() {
    let list = vec![("grocerybag".to_string(), 0.2), ("purse".to_string(), 0.2), ("coffee".to_string(), 0.1)];
    assert_eq!(HandProp::starting_pick(0.65, &list, 66, 1), None, "66 > 65: no prop");
    assert_eq!(HandProp::starting_pick(0.65, &list, 65, 1), Some("grocerybag"));
    assert_eq!(HandProp::starting_pick(0.65, &list, 1, 20), Some("grocerybag"));
    assert_eq!(HandProp::starting_pick(0.65, &list, 1, 21), Some("purse"));
    assert_eq!(HandProp::starting_pick(0.65, &list, 1, 50), Some("coffee"));
    assert_eq!(HandProp::starting_pick(0.65, &list, 1, 51), None, "the list sums to 0.5");
    assert_eq!(HandProp::starting_pick(0.0, &list, 1, 1), None, "pros never carry");
}

/// ZombieFollow (`826A91A8` / `826A9250` / `826A9518`): the goal is the player beyond 8 m, a point 1..8 m round them once
/// the goal is reached inside 8 m; 8 m/s beyond 15 m, else 3 m/s. OverrideAnimData sets the anim set for the state's
/// life; ped speech is not sent in zombie mode (`826A77A8`).
#[test]
fn zombies_follow_the_player_and_mill_round_them() {
    let behaviors = ops(&[("ZombieFollow", &[]), ("OverrideAnimData", &[("overrideEntityName", "zombie")]), ("SendSpeechEvent", &[("speechvalue", "20")])]);
    assert_eq!(behaviors[1], PedOp::OverrideAnimData { entity: "zombie".into() });
    let mut brain = PedBrain { rng: Some(crate::living_world::Rng::new(3)), zombie: true, ..Default::default() };
    let settings = BrainSettings::default();
    let none = |_: u64| None;
    let f = frame();
    let player = [20.0, 0.0, 0.0];
    let mut h = BrainHost { behaviors: &behaviors, conditions: &[], brain: &mut brain, settings: &settings, position: [0.0; 3], heading: 0.0, skater: Some((player, [0.0; 3])), target_position: &none, chase: Default::default() };
    h.begin(0, [0; 6], &f);
    assert_eq!((h.brain.motion_intent, h.brain.zombie_goal), (Some(motion::ZOMBIE_FOLLOW), Some(player)));
    h.update(0, [0; 6], &f);
    assert_eq!((h.brain.zombie_goal, h.brain.speed_suggestion), (Some(player), Some(8.0)), "20 m away: run at the player");
    h.position = [10.0, 0.0, 0.0];
    h.update(0, [0; 6], &f);
    assert_eq!((h.brain.zombie_goal, h.brain.speed_suggestion), (Some(player), Some(3.0)), "10 m: walk, still at the player");
    // Inside 8 m with the goal reached: a ring point 1..8 m from the player.
    h.position = [19.8, 0.0, 0.0];
    h.update(0, [0; 6], &f);
    let g = h.brain.zombie_goal.unwrap();
    let r = ((g[0] - player[0]).powi(2) + (g[2] - player[2]).powi(2)).sqrt();
    assert!((1.0..8.0).contains(&r), "ring point {g:?} at {r}");
    // Goal reached = 3D distance within the mover's 2.0 m arrival radius (`82E2BC70`).
    h.position = [g[0], g[1] + 2.1, g[2]];
    h.update(0, [0; 6], &f);
    assert_eq!(h.brain.zombie_goal, Some(g), "2.1 m above the point: not reached yet, keep it");
    h.position = [g[0] + 1.2, g[1] + 1.5, g[2]];
    h.update(0, [0; 6], &f);
    assert_ne!(h.brain.zombie_goal, Some(g), "1.92 m away in 3D: reached, a new ring point");
    h.end(0, [0; 6], &f);
    assert_eq!(h.brain.zombie_goal, None);
    h.begin(1, [0; 6], &f);
    assert_eq!(h.brain.anim_override.as_deref(), Some("zombie"));
    h.end(1, [0; 6], &f);
    assert_eq!(h.brain.anim_override, None);
    h.brain.speech = None;
    h.begin(2, [0; 6], &f);
    h.update(2, [0; 6], &f);
    assert_eq!(h.brain.speech, None, "no ped speech in zombie mode");
}

/// The sit plugin's ops (`826A2898` SetSitTimer, `826AD1F0` GoingToStandBackUp): the sit time lies between the ped type's
/// min and max, the stand-up roll follows its chance, and timer 24 is retail's SitTimer.
#[test]
fn the_sit_timer_and_the_stand_up_roll_follow_the_ped_type() {
    assert_eq!(timers::NAMES[timers::SIT as usize], "SitTimer");
    let settings = BrainSettings::default();
    let none = |_: u64| None;
    for seed in 0..20u64 {
        let mut brain = PedBrain { rng: Some(crate::living_world::Rng::new(seed)), ..Default::default() };
        let behaviors = [PedOp::SetSitTimer];
        let mut h = BrainHost { behaviors: &behaviors, conditions: &[], brain: &mut brain, settings: &settings, position: [0.0; 3], heading: 0.0, skater: None, target_position: &none, chase: Default::default() };
        h.begin(0, [0; 6], &frame());
        let t = brain.timers[&timers::SIT];
        assert!((30.0..60.0).contains(&t), "{t}");
    }
    let rate = |chance: f32| {
        let mut brain = PedBrain { rng: Some(crate::living_world::Rng::new(3)), sit: SitValues { stand_up_chance: chance, ..Default::default() }, ..Default::default() };
        (0..1000).filter(|_| brain.roll_stand_up()).count()
    };
    assert_eq!(rate(0.0), 0);
    assert_eq!(rate(1.0), 1000);
    let half = rate(0.5);
    assert!((430..570).contains(&half), "{half}");
}

/// The plugin flags (`826A7498`, `826A3108`, `826A2988` and their Ends): set while the behaviour runs, cleared at its
/// end; the explicit turn takes the locked waypoint's orientation, and nothing without a waypoint.
#[test]
fn plugin_flags_hold_while_their_behaviours_run() {
    let settings = BrainSettings::default();
    let none = |_: u64| None;
    let behaviors = [PedOp::OwnPluginObject, PedOp::IgnoreStandingCollisions, PedOp::SetExplicitTurnDirectionToWaypointOrientation];
    let mut brain = PedBrain::default();
    {
        let mut h = BrainHost { behaviors: &behaviors, conditions: &[], brain: &mut brain, settings: &settings, position: [0.0; 3], heading: 0.0, skater: None, target_position: &none, chase: Default::default() };
        for i in 0..3 {
            h.begin(i, [0; 6], &frame());
        }
    }
    assert!(brain.own_plugin_object && brain.ignore_standing_collisions);
    assert_eq!(brain.explicit_turn, None, "no waypoint locked");
    brain.waypoint = Some([1.0, 0.0, 0.0]);
    brain.waypoint_facing = Some([0.0, 0.0, -1.0]);
    {
        let mut h = BrainHost { behaviors: &behaviors, conditions: &[], brain: &mut brain, settings: &settings, position: [0.0; 3], heading: 0.0, skater: None, target_position: &none, chase: Default::default() };
        h.begin(2, [0; 6], &frame());
    }
    assert_eq!(brain.explicit_turn, Some([0.0, 0.0, -1.0]));
    {
        let mut h = BrainHost { behaviors: &behaviors, conditions: &[], brain: &mut brain, settings: &settings, position: [0.0; 3], heading: 0.0, skater: None, target_position: &none, chase: Default::default() };
        for i in 0..3 {
            h.end(i, [0; 6], &frame());
        }
    }
    assert!(!brain.own_plugin_object && !brain.ignore_standing_collisions && brain.explicit_turn.is_none());
}

/// IsOnRoad (the stub, always false), SimpleRandom (percentage x 0.01 against a roll), IsAtWaypoint (3 m horizontal
/// by default) and InSkaterRadius (3D).
#[test]
fn plugin_conditions_follow_retail() {
    let settings = BrainSettings::default();
    let none = |_: u64| None;
    let conditions = [PedOp::IsOnRoad, PedOp::SimpleRandom { chance: 0.2 }, PedOp::IsAtWaypoint { radius: 3.0 }, PedOp::InSkaterRadius { radius: 25.0 }];
    let mut brain = PedBrain { rng: Some(crate::living_world::Rng::new(5)), waypoint: Some([2.9, 10.0, 0.0]), ..Default::default() };
    let mut h = BrainHost { behaviors: &[], conditions: &conditions, brain: &mut brain, settings: &settings, position: [0.0; 3], heading: 0.0, skater: Some(([0.0, 24.0, 5.0], [0.0; 3])), target_position: &none, chase: Default::default() };
    let f = frame();
    assert_eq!(h.condition_activation(0, &f), 0);
    let hits: u32 = (0..1000).map(|_| h.condition_activation(1, &f)).sum();
    assert!((150..250).contains(&hits), "{hits}");
    assert_eq!(h.condition_activation(2, &f), 1, "10 m above but 2.9 m away horizontally");
    assert_eq!(h.condition_activation(3, &f), 1, "24.5 m");
    h.skater = Some(([0.0, 25.0, 5.0], [0.0; 3]));
    assert_eq!(h.condition_activation(3, &f), 0, "25.5 m");
    h.brain.waypoint = None;
    assert_eq!(h.condition_activation(2, &f), 0);
}

/// The collision / mood switches hold while their behaviours run; DisableAllMoods restores what it saved.
#[test]
fn collision_and_mood_switches_restore_at_end() {
    let settings = BrainSettings::default();
    let none = |_: u64| None;
    let behaviors = [PedOp::DisableHeavyCollision, PedOp::DisableCollisionSliding, PedOp::DisableAllMoods, PedOp::OverridePluginCollision, PedOp::OverridePluginAvoid];
    let conditions = [PedOp::IsPluginCollisionOverride, PedOp::IsPluginAvoidOverride];
    let mut brain = PedBrain { moods_disabled: true, ..Default::default() };
    let mut h = BrainHost { behaviors: &behaviors, conditions: &conditions, brain: &mut brain, settings: &settings, position: [0.0; 3], heading: 0.0, skater: None, target_position: &none, chase: Default::default() };
    let f = frame();
    for i in 0..5 {
        h.begin(i, [0; 6], &f);
    }
    assert_eq!((h.condition_activation(0, &f), h.condition_activation(1, &f)), (1, 1));
    for i in 0..5 {
        h.end(i, [0; 6], &f);
    }
    assert_eq!((h.condition_activation(0, &f), h.condition_activation(1, &f)), (0, 0));
    assert!(!brain.heavy_collision_disabled && !brain.collision_sliding_disabled);
    assert!(brain.moods_disabled, "restored to the value before the behaviour");
}

/// Monitored packets as retail (`826A2600` / `826A27D0` / `826ACF90` / `826A2770`, b84): stage names from the XML,
/// Increment only moves the stage, HasMonitoredIntent reads the active flag, the Create End removes the packet.
#[test]
fn monitored_packets_follow_retail() {
    let behaviors = ops(&[("CreateSimpleMonitoredIntent", &[("intentName", "Sit"), ("numberOfStages", "2"), ("stage2Name", "StandUp")]), ("IncrementMonitoredPacketStage", &[("intentName", "Sit")])]);
    let conditions = ops(&[("HasMonitoredIntent", &[("intentName", "Sit")])]);
    let settings = BrainSettings::default();
    let none = |_: u64| None;
    let f = frame();
    let mut brain = PedBrain::default();
    let mut h = BrainHost { behaviors: &behaviors, conditions: &conditions, brain: &mut brain, settings: &settings, position: [0.0; 3], heading: 0.0, skater: None, target_position: &none, chase: Default::default() };
    h.begin(0, [0; 6], &f);
    assert_eq!(h.brain.monitored["Sit"].current(), Some("Sit"));
    h.begin(1, [0; 6], &f);
    assert_eq!(h.brain.monitored["Sit"].current(), Some("StandUp"));
    assert_eq!(h.condition_activation(0, &f), 1, "still active: the motion side ends it");
    // A ported motion state keeps it until its clips end; an unported one ends at the last stage (fallback).
    h.brain.settle_packets(&|name| name == "Sit");
    assert_eq!(h.condition_activation(0, &f), 1);
    h.brain.settle_packets(&|_| false);
    assert_eq!(h.condition_activation(0, &f), 0);
    h.end(0, [0; 6], &f);
    assert!(!h.brain.monitored.contains_key("Sit"));
}

#[test]
fn hand_prop_conditions_and_the_weighted_pick() {
    let list = vec![("pop".to_string(), 0.5), ("waterbottle".to_string(), 0.5)];
    assert_eq!(HandProp::pick(&list, 0.0), Some("pop"));
    assert_eq!(HandProp::pick(&list, 0.49), Some("pop"));
    assert_eq!(HandProp::pick(&list, 0.51), Some("waterbottle"));
    assert_eq!(HandProp::pick(&list, 1.0), Some("waterbottle"));
    assert_eq!(HandProp::pick(&[("newspaper".to_string(), 1.0)], 0.7), Some("newspaper"));
    assert_eq!(HandProp::pick(&[], 0.3), None);
    let conditions = ops(&[("HasHandProp", &[]), ("IsHoldingSpecificHandProp", &[("handprop", "Newspaper")]), ("IsHoldingSpecificHandProp", &[("handprop", "tazr")])]);
    let behaviors = ops(&[]);
    let mut brain = PedBrain::default();
    let settings = BrainSettings::default();
    let at = |_: u64| None;
    let f = frame();
    let mut h = BrainHost { behaviors: &behaviors, conditions: &conditions, brain: &mut brain, settings: &settings, position: [0.0; 3], heading: 0.0, skater: None, target_position: &at, chase: ChaseView::default() };
    assert_eq!((h.condition_activation(0, &f), h.condition_activation(1, &f), h.condition_activation(2, &f)), (0, 0, 0));
    // Requested (82E3DDA0) already counts as HasHandProp (826AD1A8); the key test is case-blind like the hash.
    h.brain.hand_prop.request("newspaper");
    assert_eq!((h.condition_activation(0, &f), h.condition_activation(1, &f), h.condition_activation(2, &f)), (1, 1, 0));
    h.brain.hand_prop.clear();
    assert_eq!(h.condition_activation(0, &f), 0);
}

/// After a takedown the chase ends through EndChase's aggressivecapture branch like retail
/// (`pedestrian_endchase.xml`: IgnoreCollisionsWithWantTarget angrychase, UnsetWant angrychase, SuppressMoodAboutChasee
/// 5.5): the chase want (flag 0x40 from its mood result) forgets the chasee's mood records and perception entry on
/// unset, and the chasee's contacts are ignored. Without this a ped chased again the moment its taunt ended, without a
/// new collision (play test, 2026-10-12).
#[test]
fn a_captured_chasee_is_forgotten_and_its_contacts_ignored() {
    use super::super::chase::ChaseGroupInfo;
    let conditions = ops(&[("IsEndChaseReason", &[("reason", "aggressivecapture")]), ("IsEndChaseReason", &[("reason", "lostinterest")])]);
    let behaviors = ops(&[("IgnoreCollisionsWithWantTarget", &[("want", "AngryChase")]), ("UnsetWant", &[("want", "AngryChase")]), ("NewChasee", &[])]);
    assert_eq!(conditions[0], PedOp::IsEndChaseReason { reason: 1 });
    assert_eq!(behaviors[0], PedOp::IgnoreCollisionsWithWantTarget { want: "angrychase".into() });
    let info = std::cell::Cell::new(None::<ChaseGroupInfo>);
    let groups = |_: u64| info.get();
    let mut brain = PedBrain::default();
    // Raised by the collisionchase mood result (want flag set).
    brain.raise_want("angrychase", 100, true);
    brain.perceptions.know(100, 10.0);
    let settings = BrainSettings::default();
    let none = |_: u64| None;
    let f = frame();
    let view = ChaseView { me: 7, groups: Some(&groups), ..Default::default() };
    let mut h = BrainHost { behaviors: &behaviors, conditions: &conditions, brain: &mut brain, settings: &settings, position: [0.0; 3], heading: 0.0, skater: None, target_position: &none, chase: view };
    h.begin(2, [0; 6], &f);
    h.brain.chase_requests.clear();
    // The takedown ended the group with aggressivecapture: EndChase takes that branch, not lostinterest's.
    info.set(Some(ChaseGroupInfo { count: 1, primary: Some(7), end_reason: Some(1), can_add: false }));
    assert_eq!((h.condition_activation(0, &f), h.condition_activation(1, &f)), (1, 0));
    h.begin(0, [0; 6], &f);
    assert_eq!(h.brain.ignore_collisions_with, Some(100));
    h.begin(1, [0; 6], &f);
    assert!(!h.brain.wants.contains_key("angrychase"));
    assert_eq!(h.brain.chase_requests, vec![ChaseRequest::MoodReset { target: 100 }]);
    assert!(h.brain.perceptions.get(100).is_none());
    // A want without the flag (warn, greet, ...) keeps the memory when it is unset.
    h.brain.chase_requests.clear();
    h.brain.set_want("angrychase", 100);
    h.begin(1, [0; 6], &f);
    assert!(h.brain.chase_requests.is_empty());
}

/// `826ACB70` / `826AC7B0` / `826ACA08` (b106): a wiping-out chasee cannot be taken down, and the chase ends by
/// aggressive capture once the chaser is within 2 m of it while it is down.
#[test]
fn a_wiping_chasee_is_not_taken_down_and_ends_the_chase_within_2_m() {
    let conditions = ops(&[("ChaseeIsWipedOut", &[]), ("ChaseeCloseEnoughToSatisfyAggressiveChase", &[]), ("CanAttemptTakeDownTargetable", &[])]);
    let wiping = std::cell::Cell::new(false);
    let wiping_of = |_: u64| wiping.get();
    let groups = |_: u64| Some(super::super::chase::ChaseGroupInfo { count: 1, primary: Some(7), end_reason: None, can_add: true });
    let at = std::cell::Cell::new([1.9f32, 0.0, 0.0]);
    let target = |_: u64| Some(at.get());
    let mut brain = PedBrain::default();
    brain.chasee = Some(100);
    brain.takedown_target = Some(100);
    brain.takedown_choice = Some(super::super::takedown::TakedownChoice { entry: 0, mirrored: false, predicted: [0.0; 3] });
    let settings = BrainSettings::default();
    let f = frame();
    let view = ChaseView { me: 7, groups: Some(&groups), wiping: Some(&wiping_of), ..Default::default() };
    let mut h = BrainHost { behaviors: &[], conditions: &conditions, brain: &mut brain, settings: &settings, position: [0.0; 3], heading: 0.0, skater: None, target_position: &target, chase: view };
    assert_eq!((h.condition_activation(0, &f), h.condition_activation(1, &f), h.condition_activation(2, &f)), (0, 1, 1));
    wiping.set(true);
    assert_eq!((h.condition_activation(0, &f), h.condition_activation(2, &f)), (1, 0), "down: wiped out, no takedown");
    at.set([2.0, 0.0, 0.0]);
    assert_eq!(h.condition_activation(1, &f), 0, "2 m is not under 2 m");
}

/// `826AD640` / `826A8810` (b108): CannotReachChaseTarget reads the host's bit; SetTargetUnreachableTimer starts
/// `TargetUnreachableTimer` at the chase record's `unreachable_timer`, so the graph gives up after it.
#[test]
fn an_unreachable_chasee_starts_the_record_wait() {
    let conditions = ops(&[("CannotReachChaseTarget", &[]), ("SimpleTimerExpired", &[("timerName", "TargetUnreachableTimer")]), ("CanTazeUnreachableTargets", &[])]);
    let behaviors = ops(&[("SetTargetUnreachableTimer", &[])]);
    let mut record = super::super::chase::ChaseRecord::default();
    record.fields.insert("unreachable_timer".into(), 5.0);
    let mut brain = PedBrain::default();
    let settings = BrainSettings::default();
    let none = |_: u64| None;
    let f = frame();
    let view = ChaseView { me: 7, record: Some(&record), ..Default::default() };
    let mut h = BrainHost { behaviors: &behaviors, conditions: &conditions, brain: &mut brain, settings: &settings, position: [0.0; 3], heading: 0.0, skater: None, target_position: &none, chase: view };
    assert_eq!(h.condition_activation(0, &f), 0);
    h.brain.target_unreachable = true;
    assert_eq!(h.condition_activation(0, &f), 1);
    h.begin(0, [0; 6], &f);
    assert_eq!(h.condition_activation(1, &f), 0, "the 5 s wait runs");
    h.brain.tick_timers(5.0);
    assert_eq!(h.condition_activation(1, &f), 1);
    assert_eq!(h.condition_activation(2, &f), 0, "no tazer without the per-ped flag");
}

#[test]
fn is_facing_skater_takes_the_cosine_of_its_half_angle_and_a_flat_distance() {
    // `826A9E60`: dot(forward, flat dir) > cos(FOVAngle); `Distance` 0 = no range test.
    let conditions = ops(&[("IsFacingSkater", &[]), ("IsFacingSkater", &[("FOVAngle", "1.4"), ("Distance", "6.0")])]);
    assert_eq!(conditions[0], PedOp::IsFacingSkater { fov: 0.8, distance: 0.0 });
    let mut brain = PedBrain::default();
    let settings = BrainSettings::default();
    let none = |_: u64| None;
    let f = frame();
    let at = |angle: f32, range: f32| [angle.sin() * range, 3.0, angle.cos() * range];
    let mut check = |angle: f32, range: f32, op: usize| {
        let mut h = BrainHost { behaviors: &[], conditions: &conditions, brain: &mut brain, settings: &settings, position: [0.0; 3], heading: 0.0, skater: Some((at(angle, range), [0.0; 3])), target_position: &none, chase: Default::default() };
        h.condition_activation(op, &f)
    };
    assert_eq!(check(0.7, 20.0, 0), 1, "0.7 rad is inside the stock 0.8 half angle at any distance");
    assert_eq!(check(0.9, 20.0, 0), 0);
    assert_eq!(check(1.3, 5.0, 1), 1, "the marquee's 1.4 rad inside 6 m");
    assert_eq!(check(1.3, 7.0, 1), 0, "outside 6 m");
}

#[test]
fn disable_navigation_holds_while_its_state_runs() {
    // `826A3170` Update clears the navigation bit every frame, `826A3188` End sets it again.
    let behaviors = ops(&[("DisableNavigation", &[])]);
    assert_eq!(behaviors[0], PedOp::DisableNavigation);
    let mut brain = PedBrain::default();
    let settings = BrainSettings::default();
    let none = |_: u64| None;
    let f = frame();
    let mut h = BrainHost { behaviors: &behaviors, conditions: &[], brain: &mut brain, settings: &settings, position: [0.0; 3], heading: 0.0, skater: None, target_position: &none, chase: Default::default() };
    h.begin(0, [0; 6], &f);
    h.update(0, [0; 6], &f);
    assert!(h.brain.navigation_disabled);
    h.end(0, [0; 6], &f);
    assert!(!h.brain.navigation_disabled);
}

/// A finished reaction counts on the target's perception entry (`82E42A58`, `+68`), which the mood records' `7FF7`
/// test reads: a collisionwarn's want, once unset, lets the next collision escalate. A deactivated want counts
/// nothing, and a flag 0x40 want forgets its target instead (research b152 M1 / M2).
#[test]
fn unset_mood_wants_count_finished_reactions_on_the_perception_entry() {
    let memory = BrainSettings::default().mood_want_memory_seconds;
    let mut brain = PedBrain::default();
    brain.raise_mood_want("warn", 100, false, memory);
    assert_eq!(brain.perceptions.get(100).map(|e| (e.reacted, e.memory)), Some((0, 30.0)));
    brain.unset_want("warn");
    assert_eq!(brain.perceptions.get(100).map(|e| e.reacted), Some(1));
    // `82E42868` keeps the longer memory.
    brain.perceptions.know(100, 5.0);
    assert_eq!(brain.perceptions.get(100).map(|e| e.memory), Some(30.0));
    brain.raise_mood_want("warn", 100, false, memory);
    brain.wants.get_mut("warn").expect("raised").needs_addressing = false;
    brain.unset_want("warn");
    assert_eq!(brain.perceptions.get(100).map(|e| e.reacted), Some(1), "a deactivated want is not a finished reaction");
    brain.raise_mood_want("angrychase", 100, true, memory);
    brain.unset_want("angrychase");
    assert!(brain.perceptions.get(100).is_none());
    assert_eq!(brain.chase_requests, vec![ChaseRequest::MoodReset { target: 100 }]);
}

/// TazeWantTarget (research b153 M4 / M5): no speech or hit timer when the want's target does not resolve
/// (`826A8398`); after the hit, a target with chasers has its chase group ended with aggressivecapture (`826A8420`).
#[test]
fn a_taze_needs_a_resolved_target_and_ends_the_targets_chase_with_a_capture() {
    let behaviors = ops(&[("TazeWantTarget", &[("want", "taze")])]);
    let settings = BrainSettings::default();
    let f = frame();
    let mut brain = PedBrain::default();
    brain.set_want("taze", 3);
    let gone = |_: u64| None;
    let mut h = BrainHost { behaviors: &behaviors, conditions: &[], brain: &mut brain, settings: &settings, position: [0.0; 3], heading: 0.0, skater: None, target_position: &gone, chase: Default::default() };
    h.begin(0, [0; 6], &f);
    assert_eq!(h.brain.speech, None);
    let mut brain = PedBrain::default();
    brain.set_want("taze", 3);
    let here = |_: u64| Some([0.0f32, 0.0, 2.0]);
    let groups = |_: u64| Some(super::super::chase::ChaseGroupInfo { count: 2, primary: Some(8), end_reason: None, can_add: true });
    let view = ChaseView { me: 7, groups: Some(&groups), ..Default::default() };
    let mut h = BrainHost { behaviors: &behaviors, conditions: &[], brain: &mut brain, settings: &settings, position: [0.0; 3], heading: 0.0, skater: None, target_position: &here, chase: view };
    h.begin(0, [0; 6], &f);
    assert_eq!(h.brain.speech, Some(66));
    h.brain.tick_timers(0.3);
    h.update(0, [0; 6], &f);
    let capture = super::super::chase::reason::AGGRESSIVE_CAPTURE;
    assert_eq!(h.brain.chase_requests, vec![ChaseRequest::Taze { target: 3 }, ChaseRequest::GroupEnd { chasee: 3, reason: capture }]);
}

/// EndChase is SetChasee(none) (`82E3CFF8`, research b154 M4): the end reason resets and the angrychase want (slot 1) is
/// unset, which forgets a flag 0x40 target as the want's unset does.
#[test]
fn end_chase_resets_the_end_reason_and_unsets_angrychase() {
    let behaviors = ops(&[("EndChase", &[])]);
    let settings = BrainSettings::default();
    let f = frame();
    let mut brain = PedBrain::default();
    brain.raise_mood_want("angrychase", 100, true, settings.mood_want_memory_seconds);
    brain.chasee = Some(100);
    brain.end_reason = Some(super::super::chase::reason::AGGRESSIVE_CAPTURE);
    let none = |_: u64| None;
    let mut h = BrainHost { behaviors: &behaviors, conditions: &[], brain: &mut brain, settings: &settings, position: [0.0; 3], heading: 0.0, skater: None, target_position: &none, chase: Default::default() };
    h.begin(0, [0; 6], &f);
    assert_eq!((h.brain.chasee, h.brain.end_reason), (None, None));
    assert!(!h.brain.wants.contains_key("angrychase"));
    assert_eq!(h.brain.chase_requests, vec![ChaseRequest::Leave { chasee: 100 }, ChaseRequest::MoodReset { target: 100 }]);
}
