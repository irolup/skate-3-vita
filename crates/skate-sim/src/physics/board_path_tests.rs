//! AI board path (`82C05EC0`) on the real board: never runs for the player, and a steering record
//! moves the deck like retail.
use super::*;
use skate_core::physics::board::BodyId;
use skate_core::player::state::PhysicalStateId;
use skate_core::riding::grounded::state::board_path::{flags, PhysicsAiTuning};

fn pad() -> skate_core::input::xbox::XboxState {
    skate_core::input::xbox::XboxState::default()
}

#[test]
#[ignore = "requires private stock graphs and an installed map (SKATE3_ASSET_ROOT, SKATE3_MAP=<maps/DownTown.skate>)"]
fn the_board_path_never_runs_for_the_player_and_steers_the_deck_with_a_record() {
    let root_dir = std::path::PathBuf::from(std::env::var_os("SKATE3_ASSET_ROOT").unwrap());
    let map_path = std::path::PathBuf::from(std::env::var_os("SKATE3_MAP").unwrap());
    let map = skate_data::skate_map::SkateMap::load(&map_path).unwrap();
    let assets = skate_data::GameAssets::load(&root_dir).unwrap();
    let graphs = crate::graph_runtime::StockGraphs::load(&root_dir, &assets).unwrap();
    let mut physics = GamePhysics::load_with_difficulty(&root_dir, Some(&map), crate::difficulty::Difficulty::Easy).unwrap();
    assert_eq!(physics.settings.physics_ai, PhysicsAiTuning::DEFAULT_RECORD, "physics_ai/default from the setup collections");
    let mut skater = SkaterRuntime::load(&root_dir, &graphs, &physics, "easy").unwrap();
    let mut camera = crate::camera::CameraRuntime::load(&root_dir).unwrap();
    let mut controls = PlayerControls::default();
    let mut input = crate::input::ControllerInput::default();
    let mut ground_ticks = 0;
    for _ in 0..240 {
        input.sample_raw_for_test(pad());
        let mut actions = input.player_actions();
        controls.update_for_physics(&mut actions, &mut physics, &mut skater, &mut camera).unwrap();
        frame::advance(&mut physics, &mut skater, &mut controls, &graphs, &mut actions, true, &mut camera).unwrap();
        assert_eq!(skater.player_input.processed.external_physics_1616.flags & flags::STEER, 0, "the player's record never steers");
        ground_ticks += usize::from(skater.player_state.current() == PhysicalStateId::PhysicsGround);
    }
    assert!(ground_ticks > 100, "riding on the ground: {ground_ticks}");
    assert_eq!(skater.player_state.current(), PhysicalStateId::PhysicsGround);
    // A steering record: target 1 m along +x from the deck, velocity +2 m/s on x.
    let deck = physics.board.part_transforms()[BodyId::Deck.index()];
    let wheel = physics.board.part_transforms()[BodyId::RightFrontWheel.index()];
    let velocity = physics.board.bodies()[BodyId::Deck.index()].rates.linear_velocity;
    let words = |x: f32, y: f32, z: f32| [x.to_bits(), y.to_bits(), z.to_bits(), 1.0f32.to_bits()];
    let record = &mut skater.player_input.processed.external_physics_1616;
    record.vectors[3] = words(deck.translation.x + 1.0, deck.translation.y, deck.translation.z);
    record.vectors[4] = words(velocity.x + 2.0, velocity.y, velocity.z);
    record.flags = flags::STEER | flags::POSITION | flags::VELOCITY;
    assert!(board_path::post_physics(&mut physics, &skater));
    let after = physics.board.part_transforms()[BodyId::Deck.index()];
    assert!((after.translation.x - deck.translation.x - 0.02).abs() < 1e-4, "2 cm per tick: {:?} -> {:?}", deck.translation, after.translation);
    assert!((after.translation.y - deck.translation.y).abs() < 1e-5);
    let v = physics.board.bodies()[BodyId::Deck.index()].rates.linear_velocity;
    assert!((v.x - velocity.x - 0.2).abs() < 1e-4, "0.2 m/s per tick: {velocity:?} -> {v:?}");
    // Only the deck part moved (82D9C8C8 sets part 6 alone).
    assert_eq!(physics.board.part_transforms()[BodyId::RightFrontWheel.index()].translation, wheel.translation);
    // Without the steer bit nothing runs.
    skater.player_input.processed.external_physics_1616.flags = flags::POSITION;
    assert!(!board_path::post_physics(&mut physics, &skater));
}
