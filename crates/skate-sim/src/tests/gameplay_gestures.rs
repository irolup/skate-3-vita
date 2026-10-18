//! Raw D-pad -> gameplay AG/MG -> CharacterGesture channels, without UI preview.
use super::*;
use skate_core::input::xbox::XboxState;

#[test]
#[ignore = "requires private stock animation banks and collections"]
fn gameplay_gestures_play_all_directions_and_saved_selections() {
    let root = std::path::PathBuf::from(std::env::var_os("SKATE3_ASSET_ROOT").unwrap());
    let assets = skate_data::GameAssets::load(&root).unwrap();
    let graphs = crate::graph_runtime::StockGraphs::load(&root, &assets).unwrap();
    let mut physics = GamePhysics::load(&root).unwrap();
    let mut skater = SkaterRuntime::load(&root, &graphs, &physics, "normal").unwrap();
    let mut controls = PlayerControls::default();
    let mut input = crate::input::ControllerInput::default();
    let mut camera = crate::camera::CameraRuntime::load(&root).unwrap();
    let mut seen = [false; 8];
    for tick in 0..1080 {
        if tick == 60 {
            physics.set_gesture_preferences(None);
        }
        if tick == 540 {
            physics.set_gesture_preferences(Some([3, 2, 1, 0]));
        }
        let slot = if tick >= 60 {
            Some((tick - 60) / 120)
        } else {
            None
        };
        let held = slot.is_some_and(|s| s < 8) && (tick - 60) % 120 < 60;
        input.sample_raw_for_test(XboxState {
            buttons: if held {
                [1, 2, 4, 8][slot.unwrap() % 4]
            } else {
                0
            },
            triggers: [0; 2],
            left: [0; 2],
            right: [0; 2],
        });
        let mut actions = input.player_actions();
        controls.update(
            &mut actions,
            physics.settings.step.simulation.time_step,
            physics.settings.input_magnitude_threshold,
            skater.player_input.physical.scoring.capabilities_204,
        );
        frame::advance(
            &mut physics,
            &mut skater,
            &mut controls,
            &graphs,
            &mut actions,
            true,
            &mut camera,
        )
        .unwrap_or_else(|e| panic!("gameplay gesture tick {tick}: {e}"));
        if held {
            let s = slot.unwrap();
            if let Some(publication) = skater.animation.motion.gesture_publication {
                let expected = if s < 4 { s as u32 } else { 3 - (s % 4) as u32 };
                if publication.gesture == expected {
                    assert!(
                        ["GestureBoth", "GestureLeft", "GestureRight"]
                            .iter()
                            .any(|name| skater.animation.motion.animation.channels.has(name))
                    );
                    seen[s] = true;
                }
            }
        }
    }
    assert_eq!(
        seen, [true; 8],
        "Every default/custom D-pad gesture must reach gameplay playback"
    );
    assert!(
        ["GestureBoth", "GestureLeft", "GestureRight"]
            .iter()
            .all(|name| !skater.animation.motion.animation.channels.has(name)),
        "released channels must end"
    );
    assert!(
        skater.animation.motion.gesture_publication.is_none(),
        "released gesture must finish"
    );
}
