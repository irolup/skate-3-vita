//! Ground carried-board Update82D31D1C..DB0. State60 is SkateboardController;
//! state52 in Sync82D324B0 is a DIFFERENT grab-spline manager.
use super::{GamePhysics, SkaterRuntime};

/// Executes original Stop/Hold effects on the existing possession/solver owner.
/// Does not run its common per-frame Update a second time.
pub(crate) fn update_possession(physics: &mut GamePhysics, skater: &mut SkaterRuntime) {
    let flags = skater.player_input.processed.flags_2484;
    let state = skater.skateboard_controller.fields.state_448;
    let target = if flags & 1 != 0 {
        if matches!(state, 2 | 3 | 4) {
            return;
        }
        5
    } else {
        if state != 5 {
            return;
        }
        1
    };
    skater.skateboard_controller.fields.word_444 = 0;
    if state == target {
        return;
    }
    let observation = super::super::offboard::board_manager::runtime::observe(physics, skater);
    let mut effects = skater.board_possession_live.effects(
        physics,
        &mut skater.ground_lifecycle.board_animated_290,
        skater.player_input.processed.timestep_2604,
    );
    if target == 5 {
        skater.board_possession.stop(
            &mut skater.skateboard_controller.fields,
            &observation,
            &mut effects,
        );
    } else {
        skater.board_possession.hold(
            &mut skater.skateboard_controller.fields,
            &observation,
            &mut effects,
        );
    }
    //82D31D80/DB0 assigns state only AFTER the physical action.
    skater.skateboard_controller.fields.state_448 = target;
    skater.board_possession_live.publish_volumes(physics);
}

/// Board part of the Move Object enter 82D442D0 (state 502), after the shared
/// off-board enter: a carried board is let go (82D75440, +448 = 2), a board
/// being retrieved is hidden (82D755E0, +448 = 3), see
/// `skate_core::player::offboard::move_object::board_on_grab`. `drop_board`
/// false (mod knob `carry.drop_board`) keeps the board as it is.
pub(crate) fn enter_move_object(physics: &mut GamePhysics, skater: &mut SkaterRuntime, drop_board: bool) {
    use skate_core::player::offboard::move_object::{BoardOnGrab, board_on_grab};
    let action = if drop_board {
        board_on_grab(skater.skateboard_controller.fields.state_448)
    } else {
        BoardOnGrab::Keep
    };
    let target = match action {
        BoardOnGrab::LetGo => 2,
        BoardOnGrab::Hide => 3,
        BoardOnGrab::Keep => return,
    };
    //82D4432C / 82D44350: +444 = 0 before the call.
    skater.skateboard_controller.fields.word_444 = 0;
    let observation = super::super::offboard::board_manager::runtime::observe(physics, skater);
    let mut effects = skater.board_possession_live.effects(
        physics,
        &mut skater.ground_lifecycle.board_animated_290,
        skater.player_input.processed.timestep_2604,
    );
    if target == 2 {
        skater.board_possession.let_go(
            &mut skater.skateboard_controller.fields,
            &observation,
            &mut effects,
        );
    } else {
        skater.board_possession.hide(&observation, &mut effects);
    }
    //82D44344 / 82D44360: the state is written after the call.
    skater.skateboard_controller.fields.state_448 = target;
    skater.board_possession_live.publish_volumes(physics);
}
