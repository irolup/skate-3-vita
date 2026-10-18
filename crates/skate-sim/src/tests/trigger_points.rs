//! The player's trigger query points on the real stock skater, against what the
//! recomp's skater entity reports (hook on `82DD80B8`, session trig_questions):
//! head 1.61-1.65 m and hips 0.97-1.00 m above the feet point, which lies on the
//! ground plane while standing.
use super::{GamePhysics, PlayerControls, SkaterRuntime};

fn heights(skater: &SkaterRuntime) -> (f32, f32, f32) {
    let [feet, head, hips] = crate::trigger_volumes::player_points(skater);
    let parts = skater.skeleton.part_transforms();
    let toes = (parts[15][3][1] + parts[19][3][1]) * 0.5;
    (head[1] - feet[1], hips[1] - feet[1], toes - feet[1])
}

#[test]
#[ignore = "requires private stock skater, graph and collection assets"]
fn stock_player_points_match_the_recomp_heights() {
    let root = std::path::PathBuf::from(std::env::var_os("SKATE3_ASSET_ROOT").expect("set SKATE3_ASSET_ROOT"));
    let assets = skate_data::GameAssets::load(&root).unwrap();
    let graphs = crate::graph_runtime::StockGraphs::load(&root, &assets).unwrap();
    let mut physics = GamePhysics::load_with_difficulty(&root, None, crate::difficulty::Difficulty::Normal).unwrap();
    let mut skater = SkaterRuntime::load(&root, &graphs, &physics, "normal").unwrap();
    let mut camera = crate::camera::CameraRuntime::load(&root).unwrap();
    let mut controls = PlayerControls::default();
    let mut input = crate::input::ControllerInput::default();
    let mut on_board = None;
    // Settle on the board, then step off it (Y) and settle on foot.
    for tick in 0..480 {
        input.sample_raw_for_test(skate_core::input::xbox::XboxState {
            buttons: if tick == 240 { 0x8000 } else { 0 },
            triggers: [0; 2],
            left: [0; 2],
            right: [0; 2],
        });
        controls.update(
            &mut input.player_actions(),
            physics.settings.step.simulation.time_step,
            physics.settings.input_magnitude_threshold,
            skater.player_input.physical.scoring.capabilities_204,
        );
        super::frame::advance(&mut physics, &mut skater, &mut controls, &graphs, &mut input.player_actions(), true, &mut camera)
            .unwrap();
        if tick == 239 {
            on_board = Some(heights(&skater));
        }
    }
    let on_foot = heights(&skater);
    eprintln!("on board (head, hips, toes above feet) = {on_board:?}; on foot = {on_foot:?}");
    for (head, hips, toes) in [on_board.unwrap(), on_foot] {
        assert!((1.45..1.8).contains(&head), "head {head} m above the feet");
        assert!((0.85..1.1).contains(&hips), "hips {hips} m above the feet");
        assert!((-0.05..0.25).contains(&toes), "toes {toes} m above the feet point");
    }
}
