use super::*;

fn frame(tick: u32, category: FilteredCategory, descriptor: Option<AttributeName>) -> Frame {
    Frame {
        tick,
        dt: 1. / 60.,
        category,
        state: if category == FilteredCategory::Air {
            200
        } else {
            100
        },
        descriptor,
        grind_id: -1,
        flags: 0,
        position: [0., 1., tick as f32 / 60.],
        velocity: [0., 0., 1.],
        forward: [0., 0., 1.],
        switch: false,
        fakie: false,
        regular: true,
        player_basis: [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
        board_basis: [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
        reckoning_up: [0., 1., 0.],
        body_flip: false,
        front_flip: false,
        suspend_air: false,
        landing: Default::default(),
        teleported: false,
        reverting: false,
    }
}

#[test]
#[ignore = "requires private authored scoring data via SKATE3_ASSET_ROOT"]
fn native_scoring_observations_confirm_only_banked_landings() {
    let root = std::path::PathBuf::from(std::env::var_os("SKATE3_ASSET_ROOT").expect("asset root"));
    let data = Collections::load(&root).unwrap();
    let mut runtime = Runtime::load(&data).unwrap();
    let trick = runtime
        .data
        .definitions
        .iter()
        .find(|d| d.identifier == "kickflip")
        .expect("kickflip tuning")
        .encoded_name;
    let mut tick = 0;
    for outcome in [
        FilteredCategory::Ground,
        FilteredCategory::Wipeout,
        FilteredCategory::Teleport,
        FilteredCategory::Ground,
    ] {
        for _ in 0..15 {
            tick += 1;
            runtime
                .advance(frame(tick, FilteredCategory::Ground, None))
                .unwrap();
        }
        let before = runtime.landing_seq;
        let announcements = runtime.trick_seq;
        for _ in 0..120 {
            tick += 1;
            runtime
                .advance(frame(tick, FilteredCategory::Air, Some(trick)))
                .unwrap();
        }
        assert!(
            runtime.trick_seq > announcements,
            "air trick must be announced"
        );
        assert_eq!(runtime.landing_seq, before, "announcement is not a landing");
        for _ in 0..15 {
            tick += 1;
            let mut f = frame(tick, outcome, None);
            f.teleported = outcome == FilteredCategory::Teleport;
            runtime.advance(f).unwrap();
        }
        let expected = before + u32::from(outcome == FilteredCategory::Ground);
        assert_eq!(runtime.landing_seq, expected, "outcome: {outcome:?}");
        for _ in 0..15 {
            tick += 1;
            runtime
                .advance(frame(tick, FilteredCategory::Ground, None))
                .unwrap();
        }
        assert_eq!(
            runtime.landing_seq, expected,
            "old announcements must not be credited after recovery"
        );
    }
    assert_eq!(runtime.landing_seq, 2);
    assert_eq!(runtime.bail_seq, 1);
    assert!(!runtime.landed_trick.is_empty());
}

fn yaw(degrees: f32) -> rotation::Basis {
    let (s, c) = degrees.to_radians().sin_cos();
    [[c, 0., -s], [0., 1., 0.], [s, 0., c]]
}
fn authored() -> Collections {
    Collections::load(&std::path::PathBuf::from(
        std::env::var_os("SKATE3_ASSET_ROOT").expect("asset root"),
    ))
    .unwrap()
}

#[test]
#[ignore = "requires private authored scoring data via SKATE3_ASSET_ROOT"]
fn skate3_spin_flip_stance_and_frontlip_publication() {
    let data = authored();
    for trick in [Some("kickflip"), None] {
        let mut r = Runtime::load(&data).unwrap();
        let name = trick.map(|name| {
            r.data
                .definitions
                .iter()
                .find(|d| d.identifier == name)
                .unwrap()
                .encoded_name
        });
        for tick in 0..90 {
            let mut f = frame(tick, FilteredCategory::Air, name);
            f.player_basis = yaw(tick as f32 * 2.);
            f.board_basis = yaw(tick as f32 * 2.);
            r.advance(f).unwrap();
        }
        assert_eq!(r.spin_turns, 1);
        assert!(r.trick_name.ends_with("180"), "{}", r.trick_name);
        if trick.is_some() {
            assert!(r.trick_name.contains("KICKFLIP"), "{}", r.trick_name);
        }
        assert!(r.trick_seq > 0, "standalone spins must announce a trick");
        let displayed = r.trick_name.clone();
        for tick in 90..105 {
            r.advance(frame(tick, FilteredCategory::Ground, None))
                .unwrap();
        }
        assert_eq!(r.landed_trick, displayed);
    }
    let mut r = Runtime::load(&data).unwrap();
    let nollie = r
        .data
        .definitions
        .iter()
        .find(|d| d.identifier == "n_kickflip")
        .unwrap()
        .metadata
        .id;
    let kickflip = r
        .data
        .definitions
        .iter()
        .find(|d| d.identifier == "kickflip")
        .unwrap()
        .metadata
        .id;
    assert_eq!(
        display::compose(&r.data, Some(nollie), 0, 0, false, false, true).1,
        [false, false, true, true]
    );
    assert_eq!(
        display::compose(&r.data, Some(kickflip), 0, 0, false, false, true).1,
        [false, false, true, false]
    );
    let (cab, stance) = display::compose(&r.data, Some(kickflip), 1, 0, false, true, true);
    assert!(cab.contains("HALFCAB"));
    assert!(!stance[1]);
    assert!(!cab.ends_with("180"));
    for tick in 0..120 {
        let mut f = frame(tick, FilteredCategory::Grind, None);
        f.grind_id = 28;
        r.advance(f).unwrap();
    }
    assert_eq!(r.display_id, Some(28));
    assert_eq!(r.trick_name, r.data.by_id(28).unwrap().label);
    assert!(
        !r.trick_name.ends_with("180"),
        "a previous air spin must not decorate a grind"
    );
}

#[test]
#[ignore = "requires private authored scoring data via SKATE3_ASSET_ROOT"]
fn landing_preserves_multiplier_even_when_line_timer_is_nearly_empty() {
    let data = authored();
    let mut results = Vec::new();
    for multiplier in [1., 3.] {
        let mut r = Runtime::load(&data).unwrap();
        let name = r
            .data
            .definitions
            .iter()
            .find(|d| d.identifier == "kickflip")
            .unwrap()
            .encoded_name;
        for tick in 0..60 {
            r.advance(frame(tick, FilteredCategory::Air, Some(name)))
                .unwrap();
        }
        r.session.combo.multiplier = multiplier;
        r.session.combo.timer.points = r.data.combo_capacity;
        // The next grounded tick tries to drain past zero. Native holds the
        // ongoing sequence through settlement instead of resetting to x1.
        r.session.line.points = 1.1;
        for tick in 60..66 {
            r.advance(frame(tick, FilteredCategory::Ground, None))
                .unwrap();
        }
        assert_eq!(r.landing_seq, 1);
        assert!(r.sequence_score > 0.);
        assert_eq!(r.sequence_score, r.session.holder.snapshot.last_reward);
        results.push(r.sequence_score);
    }
    assert!(
        (results[1] - results[0] * 3.).abs() < 0.01,
        "banked rewards: {results:?}"
    );
}


#[test]
#[ignore = "requires private authored scoring data via SKATE3_ASSET_ROOT"]
fn body_flips_with_grabs_and_spins_keep_names_rewards_and_landing() {
    let data = authored();
    for front in [false, true] {
        for degrees in [-360_f32, 0., 360.] {
            let mut r = Runtime::load(&data).unwrap();
            let grab = r
                .data
                .definitions
                .iter()
                .find(|d| d.identifier == "melon")
                .or_else(|| {
                    r.data
                        .definitions
                        .iter()
                        .find(|d| d.metadata.class == 2 && d.metadata.id != 252)
                })
                .unwrap();
            let name = grab.encoded_name;
            let label = grab.label.clone();
            let mut flip = skate_core::air::body_flip::BodyFlipState {
                angle: 0.,
                speed: 0.,
                requested_speed: 0.,
                spin_transform: skate_core::physics::skeleton_animation_record::IDENTITY,
                combined_transform: skate_core::physics::skeleton_animation_record::IDENTITY,
            };
            let settings = skate_core::air::body_flip::BodyFlipSettings {
                smoothing: Some(1.),
                maximum_speed: Some(20.),
                spin_scale: Some(1.),
                missing_attribute_value: 0.,
            };
            for tick in 0..=180 {
                let mut f = frame(tick, FilteredCategory::Air, Some(name));
                // Use the actual Skate 3 body-flip transform producer, not just yaw.
                skate_core::air::body_flip::update(
                    &mut flip,
                    &settings,
                    &skate_core::air::body_flip::BodyFlipInput {
                        requested_speed: if tick == 0 {
                            0.
                        } else {
                            std::f32::consts::TAU / 3.
                        },
                        spin_angle: (degrees * tick as f32 / 180.).to_radians(),
                        normal: [0., 1., 0., 0.],
                        flip_axis: [if front { -1. } else { 1. }, 0., 0., 0.],
                        timestep: 1. / 60.,
                        perfect_body_flips: false,
                    },
                );
                f.player_basis =
                    std::array::from_fn(|i| std::array::from_fn(|j| flip.combined_transform[i][j]));
                f.board_basis = f.player_basis;
                f.body_flip = tick > 30;
                f.front_flip = front;
                r.advance(f).unwrap();
            }
            let suffix = if front { "FRONTFLIP" } else { "BACKFLIP" };
            assert_eq!(r.body_flip_count, if front { 1 } else { -1 });
            assert!(r.trick_name.starts_with(&label), "{}", r.trick_name);
            assert!(r.trick_name.ends_with(suffix), "{}", r.trick_name);
            assert_eq!(
                r.spin_turns.abs(),
                (degrees.abs() / 180.) as i32,
                "spin radians {}",
                r.spin
            );
            assert!(r.air_metrics[4] > 0.);
            if degrees != 0. {
                assert!(r.trick_name.contains("360"), "{}", r.trick_name);
            }
            let displayed = r.trick_name.clone();
            for tick in 181..196 {
                r.advance(frame(tick, FilteredCategory::Ground, None))
                    .unwrap();
            }
            assert_eq!(r.landing_seq, 1);
            assert_eq!(r.landed_trick, displayed);
            assert_eq!(r.landed_spin_degrees.abs(), degrees.abs() as i32);
            assert!(!r.landed_base.is_empty());
            assert!(r.sequence_score > 0.);
        }
    }
    let r = Runtime::load(&data).unwrap();
    for flip in [-1, 1] {
        let (name, _) = display::compose(&r.data, Some(252), 2, flip, false, false, true);
        assert_eq!(name, "ID_TRICK_GRAB_MIRACLE_WHIP 360");
    }
}


#[test]
#[ignore = "requires private authored scoring data"]
fn landing_banks_air_metrics_without_vault_descriptors() {
    let data = authored();
    for multiplier in [1.0, 1.5, 2.0, 3.0] {
        let mut r = Runtime::load(&data).unwrap();
        let name = r.data.definitions.iter().find(|d| d.identifier == "kickflip").unwrap().encoded_name;
        r.session.combo.multiplier = multiplier;
        r.session.combo.timer.points = r.data.combo_capacity;
        r.session.line.points = r.data.line_capacity;
        for tick in 0..60 {
            let mut f = frame(tick, FilteredCategory::Air, Some(name));
            f.position = [0.0, 1.0 + (tick as f32 / 59.0 * std::f32::consts::PI).sin() * 5.0, tick as f32 * 0.4];
            f.velocity = [0.0, 0.0, 24.0];
            r.advance(f).unwrap();
        }
        assert!(r.air_metrics.iter().sum::<f32>() > 0.0, "fixture must earn air bonuses");
        let preview = r.sequence_score;
        for tick in 60..66 {r.advance(frame(tick, FilteredCategory::Ground, None)).unwrap();}
        assert_eq!(r.landing_seq, 1);
        assert!((r.sequence_score - preview).abs() < 0.01,
            "multiplier={multiplier} air={preview} landed={} missing metric definitions={:?}",
            r.sequence_score, (129..134).map(|id|r.data.by_id(id).is_none()).collect::<Vec<_>>());
        assert_eq!(r.sequence_score, r.session.holder.snapshot.last_reward);
    }
}

#[test]
#[ignore = "requires private authored scoring data"]
fn descent_keeps_live_points_and_banks_height_gain_only_at_landing() {
    let data = authored();
    for multiplier in [1.0, 3.0] {
        let mut r = Runtime::load(&data).unwrap();
        let name = r.data.definitions.iter().find(|d| d.identifier == "kickflip").unwrap().encoded_name;
        r.session.combo.multiplier = multiplier;
        r.session.combo.timer.points = r.data.combo_capacity;
        r.session.line.points = r.data.line_capacity;
        let mut previous_score = 0.0;
        for tick in 0..60 {
            let mut f = frame(tick, FilteredCategory::Air, Some(name));
            // Rise ten metres, then descend to a platform five metres above takeoff.
            let height = if tick <= 30 { tick as f32 / 3.0 } else { 10.0 - (tick - 30) as f32 * 5.0 / 29.0 };
            f.position = [0.0, 1.0 + height, 0.0];
            f.velocity = [0.0, if tick <= 30 { 20.0 } else { -10.0 }, 0.0];
            r.advance(f).unwrap();
            assert!(r.sequence_score + 0.01 >= previous_score,
                "descent lost points at tick {tick}: {previous_score} -> {} (multiplier {multiplier})", r.sequence_score);
            previous_score = r.sequence_score;
        }
        let landing_height_reward = r.air_metrics[2];
        assert!(landing_height_reward > 0.0, "fixture must earn a landing height bonus");
        for tick in 60..66 {
            let mut f = frame(tick, FilteredCategory::Ground, None);
            f.position = [0.0, 6.0, 0.0];
            r.advance(f).unwrap();
        }
        assert_eq!(r.landing_seq, 1);
        let expected = previous_score + landing_height_reward * multiplier;
        assert!((r.sequence_score - expected).abs() < 0.01,
            "landing must add the deferred height bonus: expected {expected}, got {}", r.sequence_score);
    }
}

#[test]
#[ignore = "requires private authored scoring data"]
fn manuals_accumulate_and_preserve_the_sequence() {
    let data = authored();
    for augmentation in [4, 5] {
        let mut r = Runtime::load(&data).unwrap();
        r.session.combo.multiplier = 3.0;
        r.session.combo.timer.points = r.data.combo_capacity;
        r.session.line.points = r.data.line_capacity;
        let mut packet = crate::graph_host::motion_native::ScorePacket::default();
        packet.set(augmentation);
        for tick in 0..480 {
            let mut f = frame(tick, FilteredCategory::Ground, None);
            f.flags = packet.flags;
            f.position = [0.0, 1.0, tick as f32 * 0.2];
            f.velocity = [0.0, 0.0, 12.0];
            r.advance(f).unwrap();
            if tick == 120 || tick == 479 {
                assert!(r.sequence_score > 0.0,
                    "manual {augmentation} tick={tick} score={} metric={:?} carrier={:?} expired={}",
                    r.sequence_score, r.metric_rewards, r.carriers[1], r.session.line.expired);
            }
        }
        let preview = r.sequence_score;
        for tick in 480..486 { r.advance(frame(tick, FilteredCategory::Ground, None)).unwrap(); }
        assert!((r.sequence_score - preview).abs() < 0.01, "manual lost score on release: {preview} -> {}", r.sequence_score);
    }
}

#[test]
#[ignore = "requires private authored scoring data"]
fn manual_pop_keeps_previous_tricks_during_ground_takeoff() {
    let data = authored();
    for fakie in [false, true] {
        let mut r = Runtime::load(&data).unwrap();
        let name = r.data.by_id(96).unwrap().encoded_name;
        for tick in 0..60 { r.advance(frame(tick, FilteredCategory::Air, Some(name))).unwrap(); }
        for tick in 60..150 {
            let mut f = frame(tick, FilteredCategory::Ground, None);
            f.flags = 0x0400_0000;
            f.fakie = fakie;
            r.advance(f).unwrap();
        }
        let prior = r.sequence_score;
        assert!(prior > 0.0);
        // ScoringTrick publishes bit 24 during the grounded takeoff animation.
        // The air collector has not started yet, but the sequence must continue.
        for tick in 150..165 {
            let mut f = frame(tick, FilteredCategory::Ground, Some(name));
            f.state = 103;
            f.flags = 0x0100_0000;
            f.fakie = fakie;
            r.advance(f).unwrap();
            assert!(r.sequence_active, "manual pop banked before takeoff at tick {tick}");
        }
        for tick in 165..225 { r.advance(frame(tick, FilteredCategory::Air, Some(name))).unwrap(); }
        assert!(r.sequence_score > prior, "manual chain lost earlier rewards: {prior} -> {}", r.sequence_score);
    }
}

#[test]
#[ignore = "requires private authored scoring data via SKATE3_ASSET_ROOT"]
fn tailwalk_flips_with_540_keep_names_rewards_and_landing() {
    let data = authored();
    for front in [false, true] {
        for degrees in [-540_f32, 540.] {
            let mut r = Runtime::load(&data).unwrap();
            let grab = r
                .data
                .definitions
                .iter()
                .find(|d| d.identifier == "tailgrab_airwalk")
                .or_else(|| {
                    r.data
                        .definitions
                        .iter()
                        .find(|d| d.metadata.class == 2 && d.metadata.id != 252)
                })
                .unwrap();
            let name = grab.encoded_name;
            let label = grab.label.clone();
            let mut flip = skate_core::air::body_flip::BodyFlipState {
                angle: 0.,
                speed: 0.,
                requested_speed: 0.,
                spin_transform: skate_core::physics::skeleton_animation_record::IDENTITY,
                combined_transform: skate_core::physics::skeleton_animation_record::IDENTITY,
            };
            let settings = skate_core::air::body_flip::BodyFlipSettings {
                smoothing: Some(1.),
                maximum_speed: Some(20.),
                spin_scale: Some(1.),
                missing_attribute_value: 0.,
            };
            for tick in 0..=180 {
                let mut f = frame(tick, FilteredCategory::Air, Some(name));
                // Use the actual Skate 3 body-flip transform producer, not just yaw.
                skate_core::air::body_flip::update(
                    &mut flip,
                    &settings,
                    &skate_core::air::body_flip::BodyFlipInput {
                        requested_speed: if tick == 0 {
                            0.
                        } else {
                            std::f32::consts::TAU / 3.
                        },
                        spin_angle: (degrees * tick as f32 / 180.).to_radians(),
                        normal: [0., 1., 0., 0.],
                        flip_axis: [if front { -1. } else { 1. }, 0., 0., 0.],
                        timestep: 1. / 60.,
                        perfect_body_flips: false,
                    },
                );
                f.player_basis =
                    std::array::from_fn(|i| std::array::from_fn(|j| flip.combined_transform[i][j]));
                f.board_basis = f.player_basis;
                f.body_flip = tick > 30;
                f.front_flip = front;
                r.advance(f).unwrap();
            }
            eprintln!("TAILWALK front={front} degrees={degrees} score={} spin={} metrics={:?} factor={} repetition={} carrier={:?}", r.sequence_score, r.spin.to_degrees(), r.air_metrics, r.air_factor, r.air_repetition, r.carriers[0]);
            let suffix = if front { "FRONTFLIP" } else { "BACKFLIP" };
            assert_eq!(r.body_flip_count, if front { 1 } else { -1 });
            assert!(r.trick_name.starts_with(&label), "{}", r.trick_name);
            assert!(r.trick_name.ends_with(suffix), "{}", r.trick_name);
            assert_eq!(
                r.spin_turns.abs(),
                (degrees.abs() / 180.) as i32,
                "spin radians {}",
                r.spin
            );
            assert!(r.air_metrics[4] > 0.);
            if degrees != 0. {
                assert!(r.trick_name.contains("540"), "{}", r.trick_name);
            }
            let displayed = r.trick_name.clone();
            for tick in 181..196 {
                r.advance(frame(tick, FilteredCategory::Ground, None))
                    .unwrap();
            }
            assert_eq!(r.landing_seq, 1);
            assert_eq!(r.landed_trick, displayed);
            assert!(r.sequence_score > 0.);
        }
    }
    let r = Runtime::load(&data).unwrap();
    for flip in [-1, 1] {
        let (name, _) = display::compose(&r.data, Some(252), 2, flip, false, false, true);
        assert_eq!(name, "ID_TRICK_GRAB_MIRACLE_WHIP 360");
    }
}


#[test]
#[ignore = "requires private authored scoring data"]
fn quarter_pipe_takeoff_uses_riding_plane_speed() {
    let data = authored();
    let mut factors = Vec::new();
    for (velocity, up) in [
        ([0.0, 0.0, 12.0], [0.0, 1.0, 0.0]),
        ([0.0, 12.0, 0.0], [0.0, 0.0, 1.0]),
        ([0.0, 12.0, 0.0], [0.0, 0.0, -1.0]),
        ([0.0, 12.0, 0.0], [0.0, 1.0, 0.0]),
    ] {
        let mut r = Runtime::load(&data).unwrap();
        let name = r.data.by_id(172).unwrap().encoded_name;
        let mut f = frame(0, FilteredCategory::Air, Some(name));
        f.velocity = velocity;
        f.reckoning_up = up;
        r.advance(f).unwrap();
        factors.push(r.air_factor);
    }
    assert_eq!(factors[0], factors[1], "quarter-pipe takeoff lost reward scale");
    assert_eq!(factors[0], factors[2], "opposite wall lost reward scale");
    assert!(factors[3] < factors[0], "stationary vertical hop must retain native penalty");
}
