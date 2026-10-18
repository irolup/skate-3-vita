//! Retail air timeout (doc 09, "Falling into the Industrial sea"): a skater with no ground below
//! is sent to the checkpoint once `CalcSuggestedState` counts more than 300 air ticks. Real map
//! and stock graphs: move the skater far outside the map, let them fall, read the respawn event.
use super::*;

#[test]
#[ignore = "requires private stock graphs and an installed map (SKATE3_ASSET_ROOT, SKATE3_MAP=<maps/DownTown.skate>)"]
fn skater_with_no_ground_below_respawns_after_301_air_ticks() {
    let root = std::path::PathBuf::from(std::env::var_os("SKATE3_ASSET_ROOT").unwrap());
    let map_path = std::path::PathBuf::from(std::env::var_os("SKATE3_MAP").unwrap());
    let map = skate_data::skate_map::SkateMap::load(&map_path).unwrap();
    let assets = skate_data::GameAssets::load(&root).unwrap();
    let graphs = crate::graph_runtime::StockGraphs::load(&root, &assets).unwrap();
    let mut physics =
        GamePhysics::load_with_difficulty(&root, Some(&map), crate::difficulty::Difficulty::Easy)
            .unwrap();
    let mut skater = SkaterRuntime::load(&root, &graphs, &physics, "easy").unwrap();
    assert_eq!(
        skater.player_state.selector.air_timeout.0,
        skate_core::player::selector::RETAIL_AIR_TIMEOUT_FRAMES,
        "retail default"
    );
    let mut camera = crate::camera::CameraRuntime::load(&root).unwrap();
    let mut controls = PlayerControls::default();
    let mut input = crate::input::ControllerInput::default();
    let mut events = Vec::new();
    let mut first_air_tick = None;
    let mut max_air = 0;
    for tick in 0..900u32 {
        if tick == 60 {
            // 5 km east of the spawn: no collision anywhere below.
            let mut away = skater.animated_skeleton.roots.animation_to_world;
            away[3][0] += 5000.0;
            away[3][1] += 50.0;
            skater.travel_to(away).unwrap();
        }
        input.sample_raw_for_test(skate_core::input::xbox::XboxState::default());
        let mut actions = input.player_actions();
        controls.update_for_physics(&mut actions, &physics, &skater, &camera).unwrap();
        frame::advance(&mut physics, &mut skater, &mut controls, &graphs, &mut actions, true, &mut camera)
            .unwrap();
        let air = skater.player_state.selector.air_frames;
        max_air = max_air.max(air);
        if tick > 60 && air == 1 && first_air_tick.is_none() {
            first_air_tick = Some(physics.ticks);
        }
        events.append(&mut skater.respawn.outbox);
        if !events.is_empty() {
            break;
        }
    }
    eprintln!("AIR_TIMEOUT_TEST first_air_tick={first_air_tick:?} max_air={max_air} events={events:?}");
    let event = events.first().expect("no checkpoint respawn after 900 ticks");
    assert_eq!(event.reason, respawn::RespawnReason::AirTimeout);
    assert_eq!(event.air_frames, 301, "request on the 301st air tick (count > 300)");
    assert_eq!(event.player_id, respawn::LOCAL_PLAYER_ID);
    let first = first_air_tick.expect("the skater never counted air");
    // Request on air tick 301, then state 702 and the checkpoint reply follow within a few ticks.
    let elapsed = event.tick - first + 1;
    assert!((301..=320).contains(&elapsed), "respawn {elapsed} ticks after leaving the ground");
    // Serialisable for a future multiplayer host.
    let json = serde_json::to_string(event).unwrap();
    assert!(json.contains("\"reason\":\"air_timeout\""), "{json}");
}
