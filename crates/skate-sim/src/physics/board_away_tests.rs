//! Tick cost with a dropped board far from the skater (fix15, docs 26). Real
//! map and stock graphs, driven by pad input like the climbing tests: ride,
//! step off (Y), drop the board (LT), then park the board at growing
//! distances. Past `MaxDistance` the controller hides the board (state 3,
//! all its volumes disabled); that tick used to scan every world triangle.
use super::*;
use std::time::Instant;

fn pad(buttons: u16, triggers: [u8; 2]) -> skate_core::input::xbox::XboxState {
    skate_core::input::xbox::XboxState { buttons, triggers, left: [0; 2], right: [0; 2] }
}

#[test]
#[ignore = "requires private stock graphs and an installed map (SKATE3_ASSET_ROOT, SKATE3_MAP=<maps/DownTown.skate>)"]
fn hidden_board_tick_costs_no_more_than_a_dropped_board() {
    let root = std::path::PathBuf::from(std::env::var_os("SKATE3_ASSET_ROOT").unwrap());
    let map_path = std::path::PathBuf::from(std::env::var_os("SKATE3_MAP").unwrap());
    let map = skate_data::skate_map::SkateMap::load(&map_path).unwrap();
    let assets = skate_data::GameAssets::load(&root).unwrap();
    let graphs = crate::graph_runtime::StockGraphs::load(&root, &assets).unwrap();
    let mut physics =
        GamePhysics::load_with_difficulty(&root, Some(&map), crate::difficulty::Difficulty::Easy)
            .unwrap();
    let mut skater = SkaterRuntime::load(&root, &graphs, &physics, "easy").unwrap();
    let mut camera = crate::camera::CameraRuntime::load(&root).unwrap();
    let mut controls = PlayerControls::default();
    let mut input = crate::input::ControllerInput::default();
    // Board offsets east of the skater, one per 120 ticks; the last one is
    // beyond the hide distance (30 m in stock data).
    let steps = [0.0_f32, 10.0, 25.0, 50.0, 50.0];
    let mut dropped = Vec::new();
    let mut hidden = Vec::new();
    for tick in 0..(240 + 120 * steps.len() as u32) {
        if tick >= 240 && (tick - 240) % 120 == 0 {
            let d = steps[((tick - 240) / 120) as usize];
            let at = skater.animated_skeleton.roots.animation_to_world[3];
            let mut deck = physics.board.part_transforms()[BodyId::Deck.index()];
            if skater.skateboard_controller.fields.state_448 == 2 {
                deck.translation = skate_core::math::Vector3::new(at[0] + d, at[1] + 1.0, at[2]);
                physics.board.set_transform(deck);
            }
        }
        input.sample_raw_for_test(pad(
            if tick == 120 { 0x8000 } else { 0 },
            if tick == 180 { [255, 0] } else { [0; 2] },
        ));
        let mut actions = input.player_actions();
        controls.update_for_physics(&mut actions, &physics, &skater, &camera).unwrap();
        let start = Instant::now();
        frame::advance(&mut physics, &mut skater, &mut controls, &graphs, &mut actions, true, &mut camera)
            .unwrap();
        let cost = start.elapsed().as_secs_f64() * 1000.0;
        if tick >= 300 {
            match skater.skateboard_controller.fields.state_448 {
                2 => dropped.push(cost),
                3 => hidden.push(cost),
                _ => {}
            }
        }
    }
    let mean = |v: &[f64]| v.iter().sum::<f64>() / v.len().max(1) as f64;
    eprintln!(
        "BOARD_AWAY dropped {} ticks mean {:.3} ms, hidden {} ticks mean {:.3} ms",
        dropped.len(),
        mean(&dropped),
        hidden.len(),
        mean(&hidden)
    );
    assert!(hidden.len() > 60, "the board never hid (state 3)");
    assert!(dropped.len() > 60, "the board was not dropped (state 2)");
    // Before fix15 a hidden board cost ~10x a dropped one on DownTown.
    assert!(mean(&hidden) < 2.0 * mean(&dropped));
}
