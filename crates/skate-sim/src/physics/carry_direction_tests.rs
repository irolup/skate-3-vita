//! Move Object direction (fix20, docs 26): grab a real map prop with RB and
//! push the left stick in eight directions through the stock graphs, setup
//! data and the whole frame (input, animation phase, OffBoardPushing ground
//! step, prop follow). The pair must travel the way the stick points in the
//! skater's frame: up pushes forward, down pulls back, left/right side-step.
//! Before fix20 every direction moved the pair forward at the speed of the
//! requested velocity (the walking approach 82D7F458 follows the facing line).
use super::*;
use skate_core::player::state::PhysicalStateId;

const RB: u16 = 0x0200;

fn pad(buttons: u16, triggers: [u8; 2], left: [i16; 2]) -> skate_core::input::xbox::XboxState {
    skate_core::input::xbox::XboxState { buttons, triggers, left, right: [0; 2] }
}

fn root(skater: &SkaterRuntime) -> ([f32; 3], [f32; 3]) {
    let m = skater.animated_skeleton.roots.animation_to_world;
    let l = (m[2][0] * m[2][0] + m[2][2] * m[2][2]).sqrt().max(1e-6);
    ([m[3][0], m[3][1], m[3][2]], [m[2][0] / l, 0.0, m[2][2] / l])
}

#[test]
#[ignore = "requires private stock graphs and an installed map (SKATE3_ASSET_ROOT, SKATE3_MAP=<maps/DownTown.skate>)"]
fn move_object_follows_the_left_stick_in_the_skater_frame() {
    carry(false);
}

/// Same with the board dropped and hidden beyond the hide distance (board
/// state 3) before the grab: the 2026-10-05 crash session carried that way.
#[test]
#[ignore = "requires private stock graphs and an installed map (SKATE3_ASSET_ROOT, SKATE3_MAP=<maps/DownTown.skate>)"]
fn move_object_with_the_board_hidden() {
    carry(true);
}

fn carry(hide_board: bool) {
    let root_dir = std::path::PathBuf::from(std::env::var_os("SKATE3_ASSET_ROOT").unwrap());
    let map_path = std::path::PathBuf::from(std::env::var_os("SKATE3_MAP").unwrap());
    let map = skate_data::skate_map::SkateMap::load(&map_path).unwrap();
    let assets = skate_data::GameAssets::load(&root_dir).unwrap();
    let graphs = crate::graph_runtime::StockGraphs::load(&root_dir, &assets).unwrap();
    let mut physics =
        GamePhysics::load_with_difficulty(&root_dir, Some(&map), crate::difficulty::Difficulty::Easy)
            .unwrap();
    let mut skater = SkaterRuntime::load(&root_dir, &graphs, &physics, "easy").unwrap();
    let mut camera = crate::camera::CameraRuntime::load(&root_dir).unwrap();
    let mut controls = PlayerControls::default();
    let mut input = crate::input::ControllerInput::default();
    let mut tick = 0u32;
    let mut step = |physics: &mut GamePhysics,
                    skater: &mut SkaterRuntime,
                    controls: &mut PlayerControls,
                    camera: &mut crate::camera::CameraRuntime,
                    state: skate_core::input::xbox::XboxState| {
        input.sample_raw_for_test(state);
        let mut actions = input.player_actions();
        controls.update_for_physics(&mut actions, physics, skater, camera).unwrap();
        frame::advance(physics, skater, controls, &graphs, &mut actions, true, camera).unwrap();
        tick += 1;
    };
    // Ride, step off (Y), settle on foot.
    for t in 0..240 {
        step(&mut physics, &mut skater, &mut controls, &mut camera, pad(if t == 120 { 0x8000 } else { 0 }, [0; 2], [0; 2]));
    }
    assert_eq!(skater.player_state.current(), PhysicalStateId::BipedGround, "not on foot");
    if hide_board {
        // Drop the board (LT), then park it 50 m away until it hides.
        for t in 0..60 {
            step(&mut physics, &mut skater, &mut controls, &mut camera, pad(0, if t == 0 { [255, 0] } else { [0; 2] }, [0; 2]));
        }
        for _ in 0..120 {
            if skater.skateboard_controller.fields.state_448 == 2 {
                let at = skater.animated_skeleton.roots.animation_to_world[3];
                let mut deck = physics.board.part_transforms()[BodyId::Deck.index()];
                deck.translation = skate_core::math::Vector3::new(at[0] + 50.0, at[1] + 1.0, at[2]);
                physics.board.set_transform(deck);
            }
            step(&mut physics, &mut skater, &mut controls, &mut camera, pad(0, [0; 2], [0; 2]));
        }
        assert_eq!(skater.skateboard_controller.fields.state_448, 3, "board not hidden");
    }
    // Put the nearest map prop in front of the skater, end-on (its longest
    // box axis pointing at the skater, near face 1.5 m away, centre beyond
    // the 3.5 m hold limit for a bench), then hold RB.
    // A centre pulled in to the old 0.9 m hold put the skater inside a long
    // prop such as a bench.
    let (at, forward) = root(&skater);
    let dynamics = physics.prop_dynamics_mut().expect("map props");
    let probe = skate_core::math::Vector3::new(at[0], at[1], at[2]);
    let (id, _) = dynamics.nearest_body(probe, 500.0).expect("a prop near the spawn");
    let (origin, old_basis) = dynamics.pose(id).unwrap();
    let centre = dynamics.position_of(id).unwrap();
    let (_, _, _, half, _, _) = dynamics.obstacle_boxes().into_iter().find(|b| b.0 == id).unwrap();
    let long = half.x.max(half.z);
    let (f, side) = ([forward[0], 0.0, forward[2]], [-forward[2], 0.0, forward[0]]);
    let basis = skate_core::math::Basis3 {
        columns: if half.x >= half.z { [f, [0.0, 1.0, 0.0], side] } else { [[forward[2], 0.0, -forward[0]], [0.0, 1.0, 0.0], f] },
    };
    // Template origin relative to the box centre, re-expressed in the new basis.
    let rel = [origin.x - centre.x, origin.y - centre.y, origin.z - centre.z];
    let ob = old_basis.columns;
    let local: Vec<f32> = (0..3).map(|i| rel[0] * ob[i][0] + rel[1] * ob[i][1] + rel[2] * ob[i][2]).collect();
    let nb = basis.columns;
    let reach = long + 1.5;
    let target = [at[0] + forward[0] * reach, centre.y.max(at[1]), at[2] + forward[2] * reach];
    let moved = skate_core::math::Vector3::new(
        target[0] + (0..3).map(|i| local[i] * nb[i][0]).sum::<f32>(),
        target[1] + (0..3).map(|i| local[i] * nb[i][1]).sum::<f32>(),
        target[2] + (0..3).map(|i| local[i] * nb[i][2]).sum::<f32>(),
    );
    dynamics.teleport(id, moved, basis);
    let mut flips = 0;
    let mut last = skater.player_state.current();
    for _ in 0..60 {
        step(&mut physics, &mut skater, &mut controls, &mut camera, pad(RB, [0; 2], [0; 2]));
        let now = skater.player_state.current();
        flips += u32::from(now != last);
        last = now;
    }
    // One entry into Move Object, no drop/re-grab flip every tick.
    assert_eq!(flips, 1, "state flipped {flips} times while holding RB");
    assert_eq!(physics.prop_carry.held(), Some(id), "prop {id} not grabbed");
    assert_eq!(skater.player_state.current(), PhysicalStateId::OffBoardPushing, "not in Move Object");
    // Move Object enter 82D442D0: a carried board is let go (82D75440,
    // SkateboardController+448 = 2) where it is; a hidden board stays hidden (3).
    assert_eq!(
        skater.skateboard_controller.fields.state_448,
        if hide_board { 3 } else { 2 },
        "board state after the grab"
    );
    // Held by its near face, not pulled into the skater (fix20).
    let gap = |physics: &GamePhysics, skater: &SkaterRuntime| {
        let (at, _) = root(skater);
        let boxes = physics.prop_dynamics().unwrap().obstacle_boxes();
        let (_, c, basis, half, _, _) = boxes.into_iter().find(|b| b.0 == id).unwrap();
        let d = [at[0] - c.x, 0.0, at[2] - c.z];
        let b = basis.columns;
        let local: Vec<f32> = (0..3).map(|i| d[0] * b[i][0] + d[2] * b[i][2]).collect();
        let outside = [(local[0].abs() - half.x).max(0.0), (local[2].abs() - half.z).max(0.0)];
        ((outside[0] * outside[0] + outside[1] * outside[1]).sqrt(), half)
    };
    let (grab_gap, half) = gap(&physics, &skater);
    let grab_height = physics.prop_dynamics().unwrap().position_of(id).unwrap().y;
    eprintln!("CARRY_GRAB id={id} half_extents={half:?} gap={grab_gap:.3} height={grab_height:.3}");
    assert!(grab_gap > 0.1, "skater inside the held prop (gap {grab_gap})");
    // Eight stick directions: (pad X, pad Y) and the expected local
    // direction (right, forward) of the travel.
    let s = 0.7071_f32;
    let dirs: [([i16; 2], [f32; 2]); 8] = [
        ([0, 32000], [0.0, 1.0]),
        ([22600, 22600], [s, s]),
        ([32000, 0], [1.0, 0.0]),
        ([22600, -22600], [s, -s]),
        ([0, -32000], [0.0, -1.0]),
        ([-22600, -22600], [-s, -s]),
        ([-32000, 0], [-1.0, 0.0]),
        ([-22600, 22600], [-s, s]),
    ];
    let mut failures = Vec::new();
    for (stick, expected) in dirs {
        for _ in 0..20 {
            step(&mut physics, &mut skater, &mut controls, &mut camera, pad(RB, [0; 2], [0; 2]));
        }
        let (start, f) = root(&skater);
        let r = [f[2], 0.0, -f[0]];
        let prop_start = physics.prop_dynamics().unwrap().position_of(id).unwrap();
        for _ in 0..45 {
            step(&mut physics, &mut skater, &mut controls, &mut camera, pad(RB, [0; 2], stick));
            // The walk cadence must stay finite while carrying (a NaN here
            // crashed the walk-into-stand selection on release, 2026-10-05).
            let cadence = skater.biped_ground.controller.state.cadence.phase.phase;
            assert!(cadence.is_finite(), "walk cadence phase {cadence} while carrying, stick {stick:?}");
        }
        let (end, _) = root(&skater);
        let d = [end[0] - start[0], end[2] - start[2]];
        let local = [d[0] * r[0] + d[1] * r[2], d[0] * f[0] + d[1] * f[2]];
        let length = (local[0] * local[0] + local[1] * local[1]).sqrt();
        let cos = if length > 1e-4 { (local[0] * expected[0] + local[1] * expected[1]) / length } else { 0.0 };
        let prop_end = physics.prop_dynamics().unwrap().position_of(id).unwrap();
        let pd = [prop_end.x - prop_start.x, prop_end.z - prop_start.z];
        let extra = skater.animation_input.extra;
        eprintln!(
            "CARRY_DIR stick={stick:?} expected={expected:?} local=[{:.3},{:.3}] len={length:.3} cos={cos:.3} prop_moved=[{:.3},{:.3}] mv_x={:.3} mv_z={:.3} mv_rot={:.3} state={:?} held={:?} anim_t={:?}",
            local[0], local[1], pd[0], pd[1],
            extra.object_move_x, extra.object_move_z, extra.object_move_rotation,
            skater.player_state.current(), physics.prop_carry.held(),
            skater.animation_input.fields.animation_translation,
        );
        let (push_gap, _) = gap(&physics, &skater);
        eprintln!("CARRY_HOLD gap={push_gap:.3} prop_height_change={:.3}", prop_end.y - grab_height);
        if !(length > 0.2 && cos > 0.9) || physics.prop_carry.held() != Some(id) || push_gap < 0.1 {
            failures.push((stick, local, cos));
        }
    }
    assert!(failures.is_empty(), "pair did not follow the stick: {failures:?}");
    // Drop while still pulling (2026-10-05 crash: releasing RB with the stick
    // held back-left left BipedGround's selection with no finite candidate,
    // "SelectionSpace has no finite native minimum", and the game exited).
    let pull = [-14770, -29250];
    for _ in 0..45 {
        step(&mut physics, &mut skater, &mut controls, &mut camera, pad(RB, [0; 2], pull));
    }
    for _ in 0..90 {
        step(&mut physics, &mut skater, &mut controls, &mut camera, pad(0, [0; 2], pull));
    }
    assert_eq!(physics.prop_carry.held(), None, "prop still held after releasing RB");
    assert_eq!(skater.player_state.current(), PhysicalStateId::BipedGround, "not back on foot after the drop");
}
