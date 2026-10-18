//! Integration with the extracted stock data and the real simulation pipeline.
use super::*;
use crate::difficulty::Difficulty;

#[test]
#[ignore = "requires private stock animation banks and collections"]
fn stock_difficulties_switch_without_resetting_the_player() {
    let root = std::env::var_os("SKATE3_ASSET_ROOT").expect("SKATE3_ASSET_ROOT");
    let root = std::path::Path::new(&root);
    let data = crate::custom_difficulty::load_collections(root).unwrap();
    let assets = skate_data::GameAssets::load(root).unwrap();
    let graphs = crate::graph_runtime::StockGraphs::load(root, &assets).unwrap();
    let mut physics = GamePhysics::load_with_difficulty(root, None, Difficulty::Easy).unwrap();
    let mut skater = SkaterRuntime::load(root, &graphs, &physics, "easy").unwrap();
    let mut controls = PlayerControls::load(root).unwrap();
    let mut input = crate::input::ControllerInput::default();
    let mut camera = crate::camera::CameraRuntime::load(root).unwrap();

    // Check the complete mode/surface product before exercising live switches.
    for mode in Difficulty::ALL {
        for surface in 1..=5 {
            let settings = skater.ground_profiles.select(mode as u32, surface).unwrap();
            let key = ground_runtime::surface_key(surface).unwrap();
            assert_eq!(settings.wheel_material.dynamic_friction.to_bits(),
                data.float("physics_surfaces", key, "WheelDynamicFriction").unwrap().to_bits());
            assert_eq!(settings.board().propulsion.mode_speed_changes.map(f32::to_bits),
                ["MaxPushDVStart", "MaxPushDVEnd"].map(|f| data.float("physics_mode", mode.profile_key(), f).unwrap().to_bits()));
            assert_eq!(settings.wobble_amplitude.to_bits(),
                data.float("physics_mode", mode.profile_key(), "Hash_5B57F2CCCCEEF430").unwrap().to_bits());
        }
    }
    assert!(skater.ground_profiles.select(5, 1).is_err());
    assert!(skater.ground_profiles.select(0, 0).is_err());

    for mode in [Difficulty::Easy, Difficulty::Normal, Difficulty::Hardcore, Difficulty::Motorized, Difficulty::Custom, Difficulty::Easy] {
        let before = physics.board.bodies().map(|b| (b.rates.position, b.rates.linear_velocity));
        let ticks = physics.ticks;
        let generation = skater.pose_generation;
        physics.set_difficulty(mode);
        assert_eq!(physics.ticks, ticks);
        assert_eq!(skater.pose_generation, generation);
        assert_eq!(physics.board.bodies().map(|b| (b.rates.position, b.rates.linear_velocity)), before);
        for _ in 0..60 {
            input.sample_raw_for_test(skate_core::input::xbox::XboxState {
                buttons: 0, triggers: [0; 2], left: [0; 2], right: [0; 2],
            });
            let mut actions = input.player_actions();
            controls.update(&mut actions, physics.settings.step.simulation.time_step,
                physics.settings.input_magnitude_threshold, skater.player_input.physical.scoring.capabilities_204);
            controls.publish_gestures(physics.difficulty_index(), skater.player_input.physical.state.state_16);
            frame::advance(&mut physics, &mut skater, &mut controls, &graphs, &mut actions, true, &mut camera).unwrap();
            assert_eq!(skater.player_input.processed.state_variant_index_2528, mode as u32);
            let selected = skater.ground_profiles.select(mode as u32, skater.player_input.processed.surface_mode_2540).unwrap();
            assert!(std::sync::Arc::ptr_eq(&selected, &skater.ground_settings));
        }
        assert_eq!(physics.ticks, ticks + 60);
    }
}


#[test]
#[ignore = "requires private stock animation banks and collections"]
fn motorized_rb_accelerates_and_custom_settings_reload_without_reset() {
    let root = std::env::var_os("SKATE3_ASSET_ROOT").expect("SKATE3_ASSET_ROOT");
    let root = std::path::Path::new(&root);
    let data = crate::custom_difficulty::load_collections(root).unwrap();
    let assets = skate_data::GameAssets::load(root).unwrap();
    let graphs = crate::graph_runtime::StockGraphs::load(root, &assets).unwrap();
    let mut speeds = Vec::new();
    for held in [false, true] {
        let mut physics = GamePhysics::load_with_difficulty(root, None, Difficulty::Motorized).unwrap();
        let mut skater = SkaterRuntime::load(root, &graphs, &physics, "motorized").unwrap();
        let mut controls = PlayerControls::load(root).unwrap();
        let mut input = crate::input::ControllerInput::default();
        let mut camera = crate::camera::CameraRuntime::load(root).unwrap();
        let mut active = false;
        let mut peak = 0.0f32;
        for tick in 0..180 {
            input.sample_raw_for_test(skate_core::input::xbox::XboxState {
                buttons: if held && tick >= 30 {0x0200} else {0},
                triggers: [0;2], left:[0;2], right:[0;2],
            });
            let mut actions = input.player_actions();
            controls.update(&mut actions, physics.settings.step.simulation.time_step,
                physics.settings.input_magnitude_threshold, skater.player_input.physical.scoring.capabilities_204);
            controls.publish_gestures(physics.difficulty_index(), skater.player_input.physical.state.state_16);
            frame::advance(&mut physics, &mut skater, &mut controls, &graphs, &mut actions, true, &mut camera).unwrap();
            active |= skater.player_input.processed.flags_2476 & 0x0040_0000 != 0;
            peak = peak.max(physics.riding.motion.forward_speed.abs());
        }
        assert_eq!(active, held, "RB must reach the native motor activation flag");
        speeds.push(peak);
        if held {
            let mut tuning = crate::custom_difficulty::Tuning::defaults(&data).unwrap();
            tuning.values.insert("MotorTopSpeed".into(), 123.);
            tuning.values.insert("MotorEnabled".into(), 1.);
            let mut custom_data = data.clone(); tuning.overlay(&mut custom_data).unwrap();
            let before = physics.board.bodies().map(|b|(b.rates.position,b.rates.linear_velocity));
            let generation = skater.pose_generation;
            skater.reload_difficulty(&custom_data).unwrap();
            physics.set_difficulty(Difficulty::Custom);
            assert_eq!(before,physics.board.bodies().map(|b|(b.rates.position,b.rates.linear_velocity)));
            assert_eq!(generation,skater.pose_generation);
            let settings=skater.ground_profiles.select(4,1).unwrap();
            assert_eq!(settings.board().speed_model.override_speed,123.);
            assert!(settings.board().speed_model.override_enabled);
        }
    }
    eprintln!("Motorized peak speed: RB released={} m/s; RB held={} m/s",speeds[0],speeds[1]);
    assert!(speeds[1]>speeds[0]+1., "Holding RB must produce meaningful acceleration");
}


#[test]
#[ignore = "requires private stock animation banks and collections"]
fn custom_push_strength_changes_real_push_acceleration() {
    let root=std::env::var_os("SKATE3_ASSET_ROOT").unwrap();let root=std::path::Path::new(&root);
    let data=crate::custom_difficulty::load_collections(root).unwrap();
    let assets=skate_data::GameAssets::load(root).unwrap();
    let graphs=crate::graph_runtime::StockGraphs::load(root,&assets).unwrap();
    let mut results=Vec::new();
    for strength in [1.,5.] {
        let mut physics=GamePhysics::load_with_difficulty(root,None,Difficulty::Custom).unwrap();
        let mut skater=SkaterRuntime::load(root,&graphs,&physics,"test").unwrap();
        let mut tuning=crate::custom_difficulty::Tuning::defaults(&data).unwrap();
        tuning.values.insert("HostPushStrength".into(),strength);
        tuning.values.insert("MaxPushDVStart".into(),10.);
        tuning.values.insert("MaxPushDVEnd".into(),10.);
        let mut modified=data.clone();tuning.overlay(&mut modified).unwrap();skater.reload_difficulty(&modified).unwrap();
        let mut controls=PlayerControls::load(root).unwrap();
        let mut input=crate::input::ControllerInput::default();
        let mut camera=crate::camera::CameraRuntime::load(root).unwrap();
        let mut peak=0.0f32;let mut push_frames=0;
        for tick in 0..100 {
            input.sample_raw_for_test(skate_core::input::xbox::XboxState {
                buttons:if (30..45).contains(&tick) {0x1000} else {0},triggers:[0;2],left:[0;2],right:[0;2],
            });
            let mut actions=input.player_actions();
            controls.update(&mut actions,physics.settings.step.simulation.time_step,
                physics.settings.input_magnitude_threshold,skater.player_input.physical.scoring.capabilities_204);
            controls.publish_gestures(physics.difficulty_index(),skater.player_input.physical.state.state_16);
            frame::advance(&mut physics,&mut skater,&mut controls,&graphs,&mut actions,true,&mut camera).unwrap();
            peak=peak.max(physics.riding.motion.forward_speed.abs());
            if skater.player_input.processed.flags_2468 & 0x0200_0000 != 0 {push_frames+=1;}
        }
        assert!(push_frames>0,"controller input must generate actual animation push events");
        results.push(peak);
    }
    eprintln!("Actual push peak speeds: 1x={} m/s, 5x={} m/s",results[0],results[1]);
    assert!(results[1]>results[0]*1.15,"stronger pushes must accelerate faster through real gameplay");
}
