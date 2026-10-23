//! CalcSuggestedState82D8ADE8 gets actual contacts, line distances and pose.
use super::*;
use skate_core::player::selector::{
    ProcessedStateInput, StateSelectionInput,
    conditions::{BoardBodyState, SkeletonAnimationState},
};
pub(super) fn advance(
    physics: &mut GamePhysics,
    skater: &mut SkaterRuntime,
    snapshot: skate_core::player::input_phase::ProcessedPhysicsSnapshot,
) -> Result<(), String> {
    debug_assert_eq!(snapshot.tick, physics.ticks);
    let p = &snapshot.input;
    let input = StateSelectionInput {
        processed: ProcessedStateInput {
            grind_type_1248: p.grind.family_1248,
            grind_candidate_1488: p.grind.valid_1488,
            grind_investigation_flags_1516: p.grind.flags_1516,
            field_1776: p.external_physics_1616.flags as i32,
            flags_2468: p.flags_2468,
            flags_2472: p.flags_2472,
            flags_2476: p.flags_2476,
            flags_2480: p.flags_2480,
            flags_2484: p.flags_2484,
            flags_2488: p.flags_2488,
            category_2512: p.category_2512,
            wheel_contact_count_2556: p.wheel_count_2556 as i32,
            field_2572: skater.player_state.post.state_frames,
            state_timer_2664: p.state_timer_2664,
            field_2732: skater.animation_input.fields.slide,
            field_2744: skater.animation_input.extra.revert_direction,
            trajectory_collision_time_2772: p.air_scalar_2772,
        },
        skateboard_contact_count_869: physics.riding.ground.wheel_contact_count,
        board_body: BoardBodyState {
            field_856: physics.riding.wheel_lines.minimum_distance,
            field_7692: physics.riding.ground.time_without_wheel_contact,
        },
        skeleton: SkeletonAnimationState {
            mode_16420: skater.skeleton_input.force_mode,
            back_chain_lane_12656: skater.skeleton_input.drive_frames[0][2][1],
            threshold_800: skater.player_state.animated_board_threshold,
        },
        normal_off_ground: skater.player_state.normal_off_ground,
        skitching_off_ground: skater.player_state.skitching_off_ground,
    };
    let current = skater.player_state.current();
    let requested = skater.player_state.selector.calculate(current, &input);
    // SKITCH_RELEASE_TRACE: the release tick and the 30 after it, so a squash bail after letting go of a car shows
    // how the pose error and the car contact developed (research b180: logs had only the bail tick).
    use skate_core::player::state::PhysicalStateId as Sid;
    if current == Sid::Skitching && requested != Sid::Skitching {
        skater.skitch_release_trace = 31;
    }
    if skater.skitch_release_trace > 0 {
        skater.skitch_release_trace -= 1;
        let g = &skater.wipeout.settings().ground;
        let c = &skater.collision_feedback;
        tracing::info!(
            "SKITCH_RELEASE_TRACE t={} {current:?}->{requested:?} max_pose_error={:?} pose_error=[{:.3}, {:.3}, {:.3}] squash_limit={:.3} vehicle_contact={} vehicle_force={:.1} time_on_ground={:.3} tick={}",
            30 - skater.skitch_release_trace,
            skater.collision_maximum_error,
            skater.collision_pose_error[0],
            skater.collision_pose_error[1],
            skater.collision_pose_error[2],
            if c.flags.group_8 { g.vehicle_scalar } else { 1.0 } * g.max_squash,
            c.flags.group_8,
            c.maximum_group_8_force,
            p.time_on_ground_2752,
            physics.ticks
        );
    }
    skater.player_state.requested_state = requested;
    if skater.player_state.selector.request_teleport {
        //0x82D8B034: count > limit sets selector+57 (request teleport -> state 702).
        let deck = physics.board.part_transforms()[skate_core::physics::board::BodyId::Deck.index()].translation;
        skater.respawn.note_request(
            super::super::respawn::RespawnReason::AirTimeout,
            skater.player_state.selector.air_frames,
            [deck.x, deck.y, deck.z],
        );
    }
    if requested != current {
        //82DB5FF4 restores the actual counter also used by ground-history.
        skater.player_input.player.ground_history_frames_1304 = 100;
        super::transition::set(physics, skater, requested)?;
    }
    // WIPEOUT_REASONS: which Wipeout requests (indices into the request array) and values sent the
    // skater into WipeoutGround, plus the 2480 bit 3 override. The tracing span names the system
    // (player `physics::advance`, NPC `npc_sim::simulate`) for NPC skaters bailing on placement.
    if requested != current && requested == skate_core::player::state::PhysicalStateId::WipeoutGround {
        let w = &skater.wipeout.state;
        let reasons: Vec<(usize, f32)> = (0..w.reasons.len()).filter(|&i| w.reasons[i]).map(|i| (i, w.values[i])).collect();
        // The inputs of the ground squash / displacement checks (`82D8F9E0`): reason 18 = pose error over the squash
        // limit (x the vehicle scalar while touching a vehicle).
        let g = &skater.wipeout.settings().ground;
        let c = &skater.collision_feedback;
        tracing::info!(
            "WIPEOUT_REASONS from={current:?} reasons={reasons:?} count={} flags_2480_bit3={} tick={} max_pose_error={:?} squash_limit={:.3} vehicle_contact={} vehicle_force={:.1} ped_force={:.1} side_on_trick={} trick_scalar={:.2} time_on_ground={:.3}",
            w.count,
            p.flags_2480 & (1 << 3) != 0,
            physics.ticks,
            skater.collision_maximum_error,
            if c.flags.group_8 { g.vehicle_scalar } else { 1.0 } * g.max_squash,
            c.flags.group_8,
            c.maximum_group_8_force,
            // A bail from a ped hit (research b448): the ped body parts' contact force (group 11, the ordinary
            // force check) and whether the side-on mid-trick factor applied.
            c.maximum_group_11_force,
            p.flags_2472 & (1 << 15) != 0,
            g.trick_scalar,
            p.time_on_ground_2752
        );
    }
    //SystemLogic82DB6000 clears the per-frame Wipeout request accumulator.
    skater.wipeout.state.clear_after_selection();
    Ok(())
}
