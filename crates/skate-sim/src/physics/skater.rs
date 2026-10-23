//! The game's single skater: stock graph animation and its physical skeleton.
//! Loading creates the real bodies from the animator's initial evaluated pose.
use super::{
    GamePhysics,
    animated_skeleton::AnimatedSkeleton,
    animation_feedback::AnimationFeedback,
    animation_input::AnimationInput,
    foot_ik::FootIk,
    foot_physical_output::FootPhysicalOutputs,
    ground_runtime::{GroundRuntime, GroundSettings, GroundState},
    player_input::PlayerInputRuntime,
    skeleton_body,
    skeleton_input_runtime::SkeletonInputRuntime,
    skeleton_output::SkeletonOutput,
};
use crate::{graph_runtime::StockGraphs, skater_animation::SkaterAnimation};
use skate_core::physics::{
    board::BodyId,
    skeleton_animation_record::{IDENTITY, map_animation_parts},
    skeleton_body::{
        SkeletonBody, SkeletonCollisionMode, SkeletonDriveBatch, SkeletonDrives, SkeletonJoints,
    },
};
use skate_data::collections::Collections;
use std::path::Path;

/// A pending ped takedown on the skater (see [`SkaterRuntime::takedown`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Takedown {
    /// Unit direction from the chaser to the skater (zero when they coincide).
    pub direction: [f32; 3],
    /// Seconds the latch has been set.
    pub age: f32,
}

impl Takedown {
    /// `82592390`: normalize(skater - chaser), zero when coincident.
    pub fn from_positions(skater: [f32; 3], chaser: [f32; 3]) -> Self {
        let d = [skater[0] - chaser[0], skater[1] - chaser[1], skater[2] - chaser[2]];
        let l = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        let direction = if l > 1e-6 { [d[0] / l, d[1] / l, d[2] / l] } else { [0.0; 3] };
        Self { direction, age: 0.0 }
    }
}

/// How long a takedown latch waits for the wipeout before it is dropped, s. NOT RETAIL: retail's
/// clear of `1904` bit 30 is not found (b15); this keeps a missed wipeout from holding the flag.
pub(crate) const TAKEDOWN_LATCH_SECONDS: f32 = 2.0;

pub(crate) struct SkaterRuntime {
    /// The AI controller's physics record (an NPC skater; `None` for the local player). Retail:
    /// AIPhysicsInput `*(skater+1840)`, written by the PathController, with the actor's "input
    /// fresh" bit (`1904` bit 27) and the controller pointer (`1828`).
    pub ai_physics: Option<AiPhysicsSource>,
    /// A ped took this skater down (retail `82592390`: actor `1904` bit 30 and the direction
    /// away from the chaser at `*(actor+1832)`), published as the packet's external impulse until
    /// WipeoutGround Enter used it.
    pub takedown: Option<Takedown>,
    /// The vehicle-group solids (traffic car proxies, mod vehicles) the skeleton touched in the last
    /// solve: (solid id, contact point, relative speed at the contact m/s). The living world maps them to
    /// cars (the hit brake, the parked car alarm).
    pub vehicle_hits: Vec<(u64, [f32; 3], f32)>,
    /// The ped bodies (group 11 proxies, `living_world::ped_body`) the skeleton touched in the last solve, same
    /// layout: the ped's knock-down comes from these real contacts (research b185).
    /// Ped parts the skeleton touched: proxy id, contact point, the part's (|v|, |w|) after the solve.
    pub ped_hits: Vec<(u64, [f32; 3], [f32; 2])>,
    /// Every ped part in the last solve, touched or not: proxy id, (|v|, |w|) after the solve. The reaction reads
    /// the touched ped's `ped_body.reaction_part` here (retail: the torso, also for an arm-only touch).
    pub ped_after: Vec<(u64, [f32; 2])>,
    /// The ped bodies that stopped the on-foot walker's capsule this step (`solid_contacts::constrain_ground`), same
    /// layout; the closing speed is the walker's, not yet the part's (the ped side applies the masses).
    pub scoring: crate::scoring_runtime::Runtime,
    pub climbing: super::climbing::Runtime,
    /// Completed physical pose in native animation space, read by rendering.
    pub render_pose: Vec<skate_core::animation::output::NativeMatrix>,
    pub pose_generation: u64,
    pub centre_of_mass_filter: skate_core::physics::centre_of_mass_filter::CentreOfMassFilter,
    pub centre_of_mass_output: skate_core::physics::centre_of_mass_filter::CentreOfMassOutput,
    pub animation: SkaterAnimation,
    pub animated_skeleton: AnimatedSkeleton,
    pub skeleton_air: super::skeleton_air::SkeletonAir,
    pub air_reckoning: super::air_reckoning::AirReckoning,
    pub air_state: skate_core::air::state::PhysicsAirState,
    pub air_settings: super::air_phase::AirSettings,
    pub known_air: super::known_air::KnownAir,
    pub biped_air: super::biped_air::BipedAir,
    pub landing_on_deck: super::landing_on_deck::Runtime,
    pub landing_deck: super::offboard::landing_deck::Owner,
    pub grind_trick: super::grind_trick::GrindTrick,
    pub ground_animation: super::ground_animation::GroundAnimationRuntime,
    pub ground_animation_settings: super::ground_animation::GroundAnimationSettings,
    pub revert_state: super::revert_state::RevertState,
    pub slide_state: super::slide_state::SlideState,
    pub skitch_state: super::skitch_state::SkitchState,
    /// The spline geometry of the published grab records (Processed 1888 / 2176 keep only the 72 words; the skitch
    /// grab point walks the points, 82D2D550 via the held record's +196).
    pub grab_geometry: [Option<std::sync::Arc<skate_core::player::offboard::grab_scene::Geometry>>; 2],
    pub trajectory: super::air_trajectory::AirTrajectoryRuntime,
    pub grind_camera: super::grind_camera::GrindCamera,
    pub grind: super::grind::Runtime,
    pub footplant: super::footplant::Footplant,
    pub boneless: super::boneless::Boneless,
    pub handplant: super::handplant::Handplant,
    pub wipeout: super::wipeout::Wipeout,
    pub wipeout_state: super::wipeout_states::WipeoutState,
    pub(super) respawn: super::respawn::Runtime,
    pub teleport_state: super::teleport_state::Runtime,
    pub skeleton: SkeletonBody,
    pub skeleton_joints: SkeletonJoints,
    pub skeleton_drives: SkeletonDrives,
    pub skeleton_collision: SkeletonCollisionMode,
    pub collision_feedback: skate_core::physics::skeleton_body::SkeletonCollisionFeedback,
    pub pose_errors: skate_core::physics::skeleton_body::SkeletonPoseErrors,
    pub collision_pose_error: [f32; 4],
    /// Skeleton16288/16304, retained by the post-solver response producer.
    pub collision_extra_errors: [[f32; 4]; 2],
    ///Skeleton16384 is published by the completed pose-error response. Wipeout
    ///runs after that publication; no fabricated pre-solve measurement exists.
    pub collision_maximum_error: Option<f32>,
    /// Ticks left of the SKITCH_RELEASE_TRACE log after leaving Skitching (the stopped-car release bail, 2026-10-17).
    pub skitch_release_trace: u8,
    /// A new actor's trajectory selector reset waits for its placement (b310): the placement runs the old state's Exit
    /// first (`frame.rs`, after `enter_after_teleport`), and KnownAir's Exit reads the selection.
    pub trajectory_reset_pending: bool,
    pub solved_drives: Option<SkeletonDriveBatch>,
    pub foot_ik: FootIk,
    pub foot_physical: FootPhysicalOutputs,
    pub player_input: PlayerInputRuntime,
    pub player_state: super::player_state::PlayerState,
    pub skeleton_input: SkeletonInputRuntime,
    pub ground: GroundState,
    pub ground_runtime: GroundRuntime,
    pub ground_profiles: super::ground_runtime::GroundProfiles,
    pub ground_settings: std::sync::Arc<GroundSettings>,
    pub ground_lifecycle: super::ground_phase::GroundLifecycle,
    pub biped_ground: super::biped_ground::Owner,
    pub offboard_contact: super::offboard::contact_toolkit::Owner,
    pub offboard_air_selector: super::offboard::air_selector::AirSelector,
    pub offboard_feet: skate_core::player::offboard::board_possession::manager::State,
    pub offboard_grab: super::biped_ground::grab_runtime::Owner,
    pub animation_input: AnimationInput,
    pub animation_feedback: AnimationFeedback,
    pub physical_feedback: skate_core::animation::physical_feedback::PhysicalFeedback,
    pub landing_quality: skate_core::animation::landing_quality::Output,
    pub landing_quality_settings: skate_core::animation::landing_quality::Settings,
    pub skeleton_output: SkeletonOutput,
    pub skateboard_controller: super::skateboard_controller::SkateboardController,
    pub board_possession: super::offboard::board_manager::Owner,
    pub board_possession_live: super::offboard::board_manager::runtime::LiveState,
}

impl SkaterRuntime {
    /// A pooled body becoming a new NPC: retail builds a new actor and PhysicalPlayer per NPC (`82590DC0` ->
    /// `82DB18A0`, b295). Re-runs the load-time initialisers of the per-actor parts the placement does not route
    /// through the state machine (b299); the placement's ResetSystems port and state Exit / Enter do the rest.
    pub(crate) fn reset_for_new_actor(&mut self, data: &Collections) -> Result<(), String> {
        self.player_input.reset_for_new_actor(data)?;
        self.player_state.reset_for_new_actor();
        // After the placement's Exit / Enter, not now: a body pooled in KnownAir exits KnownAir there, which needs
        // its old selection (laptop 2026-10-19: "KnownAir requires a completed selected trajectory" on the first step).
        self.trajectory_reset_pending = true;
        self.grind.reset_for_new_actor();
        self.takedown = None;
        self.ai_physics = None;
        self.skitch_release_trace = 0;
        Ok(())
    }
    /// Prepare all settings before committing; never replace active physics state.
    pub(crate) fn reload_difficulty(&mut self, data: &Collections) -> Result<(), String> {
        let profiles = super::ground_runtime::GroundProfiles::load(data)?;
        let ground_settings = profiles.select(self.player_input.processed.state_variant_index_2528,
            self.player_input.processed.surface_mode_2540.clamp(1, 5))?;
        let ground = GroundState::load(data, "test", true)?;
        let air = super::air_phase::AirSettings::load(data)?;
        let reckoning = super::air_reckoning::AirReckoning::load(data)?;
        let known = super::known_air::KnownAir::load(data)?;
        let wipeout = super::wipeout::Wipeout::load(data)?;
        let grind = super::grind::Runtime::load(data)?;
        let jumps = super::ground_animation::GroundAnimationSettings::load(data)?;
        let animation = AnimationInput::load(data, &self.animation.evaluator.frames, "test")?;
        let player = PlayerInputRuntime::load(data)?;
        self.ground_profiles = profiles; self.ground_settings = ground_settings;
        self.ground.adopt_mode_settings(ground);
        self.air_settings = air;
        self.air_reckoning.adopt_mode_settings(reckoning);
        self.known_air.adopt_mode_settings(known);
        self.wipeout.adopt_mode_settings(wipeout);
        self.grind.adopt_mode_settings(grind);
        self.ground_animation_settings = jumps;
        self.animation_input.adopt_mode_settings(animation);
        self.player_input.player.state_variants_1408 = player.player.state_variants_1408;
        Ok(())
    }

    /// Completed placements of this skater (retail place-skater counter +1864).
    pub(crate) fn placements(&self) -> u32 {
        self.respawn.placements
    }

    pub(crate) fn travel_to(&mut self, transform: [[f32; 4]; 4]) -> Result<(), String> {
        self.travel(transform, None)
    }

    pub(crate) fn travel(
        &mut self,
        transform: [[f32; 4]; 4],
        velocity: Option<[f32; 3]>,
    ) -> Result<(), String> {
        self.travel_ex(transform, velocity, true)
    }

    /// A teleport that lands on the board or on foot (retail's Challenge Map teleports arrive off the board, the
    /// event's IsOnBoard = 0, b244d; the on-foot landing is BipedGround through the teleport state).
    pub(crate) fn travel_ex(
        &mut self,
        transform: [[f32; 4]; 4],
        velocity: Option<[f32; 3]>,
        on_board: bool,
    ) -> Result<(), String> {
        self.player_input.request_teleport_ex(transform, velocity)?;
        self.teleport_state.request_manual(transform, on_board);
        Ok(())
    }

    /// An AI skater's respawn destination for the next wipeout-end teleport (instead of the checkpoint history).
    pub(crate) fn set_respawn_destination(&mut self, transform: [[f32; 4]; 4], on_board: bool) {
        self.respawn.destination = Some(super::teleport_state::Checkpoint { transform, on_board });
    }

    pub(crate) fn clear_respawn_destination(&mut self) {
        self.respawn.destination = None;
    }

    /// The actor reply alone (retail `825926F8`, NPC skater placements): one reset that also moves the animated
    /// skeleton's root, unlike the host teleport (which left a placed skater drawn where its body was before).
    pub(crate) fn place_by_reply(&mut self, transform: [[f32; 4]; 4], on_board: bool) {
        self.teleport_state.request_manual(transform, on_board);
    }
    pub fn load(
        asset_root: &Path,
        graphs: &StockGraphs,
        physics: &GamePhysics,
        mode: &str,
    ) -> Result<Self, String> {
        Self::load_for_world(asset_root, graphs, physics, mode, None)
    }

    pub(crate) fn load_for_world(
        asset_root: &Path, graphs: &StockGraphs, physics: &GamePhysics, mode: &str,
        source: Option<std::sync::Arc<crate::skater_animation::AnimationSource>>,
    ) -> Result<Self, String> {
        let data = crate::custom_difficulty::load_collections(asset_root)?;
        // With a shared source its banks are the same file already read; otherwise read them here.
        let loaded;
        let banks = match &source {
            Some(s) => s.banks(),
            None => {
                loaded = skate_data::animation_banks::AnimationBanks::load(asset_root)?;
                &loaded
            }
        };
        let animation_metadata = banks.metadata()?;
        // The host's current character is a custom skater with no pro selector
        // or equipped physical hat. These are profile choices, not force values.
        let mut animation = match source {
            Some(source) => SkaterAnimation::from_source(&data, graphs, b"", source)?,
            None => SkaterAnimation::load(asset_root, &data, graphs, b"")?,
        };
        let initial_hierarchy = animation.evaluate_initial_pose()?;
        let mut animated_skeleton =
            AnimatedSkeleton::load(asset_root, &data, &animation.evaluator.frames, false)?;
        let initial_parts = map_animation_parts(
            &initial_hierarchy,
            &animated_skeleton.bone_indices,
            &animated_skeleton.physics_frames,
        )?;
        let deck = physics.board.part_transforms()[BodyId::Deck.index()];
        let mut spawn = IDENTITY;
        for (column, axis) in deck.basis.columns.iter().enumerate() {
            spawn[column][..3].copy_from_slice(axis);
        }
        spawn[3] = [
            deck.translation.x,
            deck.translation.y,
            deck.translation.z,
            0.0,
        ];
        let mut skeleton = skeleton_body::load(
            asset_root,
            &data,
            &animation.evaluator.frames.source_sha256,
            &initial_parts,
            spawn,
            physics.settings.step.simulation,
            None,
        )?;
        animated_skeleton
            .roots
            .reset_initial_alignment(skeleton.animation_to_world);
        animated_skeleton.board_frames.reset(spawn);
        for _ in 0..2 {
            animated_skeleton.board_frames.publish_centre_of_mass(
                skeleton.record.centre_of_mass,
                physics.settings.step.simulation.time_step,
                0,
            );
        }
        animated_skeleton
            .board_frames
            .publish_local_observations(&animated_skeleton.roots, &spawn);
        let animation_input = AnimationInput::load(&data, &animation.evaluator.frames, mode)?;
        let skeleton_joints = skeleton_body::load_joints(
            asset_root,
            &data,
            &animation.evaluator.frames.source_sha256,
            &initial_hierarchy,
            &animation.evaluator.frames.parents,
            &animated_skeleton.bone_indices,
        )?;
        let mut skeleton_drives = skeleton_body::load_drives(
            asset_root,
            &data,
            &animation.evaluator.frames.source_sha256,
            &initial_hierarchy,
            &animated_skeleton.bone_indices,
            &initial_parts,
            &skeleton_joints,
            skeleton.animation_to_world,
            spawn,
            physics.settings.step.simulation,
        )?;
        // Reset82BD9990 finishes by publishing both COM targets to the
        // actual extra bodies. These bodies subsequently share the solver.
        let initial_targets = skeleton_drives.targets.update_extra_targets(
            &mut skeleton,
            &animated_skeleton.board_frames.com_frame,
            &animated_skeleton.board_frames.lifted_com_frame,
        );
        let foot_ik = FootIk::load(&data, &animation.evaluator.frames, &animated_skeleton)?;
        let skeleton_output =
            SkeletonOutput::load(&data, &animation.evaluator.frames, &animated_skeleton)?;
        // Actor82591448 passes network || ghost for self-pair suppression.
        // This local game has neither networking nor a ghost actor.
        let skeleton_collision = skeleton_body::load_collision(
            asset_root,
            &data,
            &animation.evaluator.frames.source_sha256,
            false,
        )?;
        let wipeout_state = super::wipeout_states::WipeoutState::load(
            &data,
            asset_root,
            &animation.evaluator.frames.source_sha256,
        )?;
        let respawn = super::respawn::Runtime::load(&data, spawn, animation.checkpoint_stance())?;
        let mut trajectory = super::air_trajectory::AirTrajectoryRuntime::load(&data)?;
        trajectory.bind_grind_world(std::sync::Arc::clone(&physics.grind_world));
        let mut skeleton_input = SkeletonInputRuntime::load(&data)?;
        skeleton_input.drive_frames = initial_parts;
        skeleton_input.extra_target_positions = [
            initial_targets.com,
            initial_targets.lifted_com,
            initial_targets.following_com,
        ];
        Ok(Self {
            ai_physics: None,
            takedown: None,
            vehicle_hits: Vec::new(),
            ped_hits: Vec::new(),
            ped_after: Vec::new(),
            respawn,
            scoring: crate::scoring_runtime::Runtime::load(&data)?,
            climbing: super::climbing::Runtime::load(asset_root, &animation.evaluator.frames.bone_names)?,
            render_pose: initial_hierarchy,
            pose_generation: 0,
            centre_of_mass_filter: Default::default(),
            // PhysOut reset82DE53F0 clears these observations before first output.
            centre_of_mass_output: skate_core::physics::centre_of_mass_filter::CentreOfMassOutput {
                velocity: [0.0; 4],
                acceleration: [0.0; 4],
                position: [0.0; 4],
            },
            animation,
            landing_quality: Default::default(),
            landing_quality_settings: super::landing_quality::load(&data)?,
            animated_skeleton,
            skeleton_air: super::skeleton_air::SkeletonAir::load(&data)?,
            air_reckoning: super::air_reckoning::AirReckoning::load(&data)?,
            air_state: Default::default(),
            air_settings: super::air_phase::AirSettings::load(&data)?,
            known_air: super::known_air::KnownAir::load(&data)?,
            biped_air: super::biped_air::BipedAir::load(&data)?,
            landing_on_deck: super::landing_on_deck::Runtime::load(&data)?,
            landing_deck: super::offboard::landing_deck::Owner::load(&data)?,
            grind_trick: Default::default(),
            ground_animation: Default::default(),
            ground_animation_settings: super::ground_animation::GroundAnimationSettings::load(&data)?,
            revert_state: super::revert_state::RevertState::load(&data)?,
            slide_state: super::slide_state::SlideState::load(&data)?,
            skitch_state: Default::default(),
            grab_geometry: [None, None],
            trajectory,
            grind_camera: super::grind_camera::GrindCamera::default(),
            grind: super::grind::Runtime::load(&data)?,
            footplant: super::footplant::Footplant::load(&data)?,
            boneless: super::boneless::Boneless::load(&data)?,
            handplant: super::handplant::Handplant::load(&data)?,
            wipeout: super::wipeout::Wipeout::load(&data)?,
            wipeout_state,
            teleport_state: super::teleport_state::Runtime::new(
                #[cfg(test)]
                super::teleport_state::Checkpoint {
                    transform: spawn,
                    on_board: true,
                },
            ),
            skeleton,
            skeleton_joints,
            skeleton_drives,
            collision_feedback: skeleton_body::load_feedback(&data, skeleton_collision.settings)?,
            skeleton_collision,
            pose_errors: skate_core::physics::skeleton_body::SkeletonPoseErrors {
                targets: [
                    initial_targets.com,
                    initial_targets.lifted_com,
                    initial_targets.following_com,
                ],
                ..Default::default()
            },
            collision_pose_error: [0.0; 4],
            collision_extra_errors: [[0.0; 4]; 2],
            collision_maximum_error: None,
            skitch_release_trace: 0,
            trajectory_reset_pending: false,
            solved_drives: None,
            foot_ik,
            foot_physical: FootPhysicalOutputs::load(&data)?,
            player_input: PlayerInputRuntime::load(&data)?,
            player_state: super::player_state::PlayerState::load(&data, mode)?,
            skeleton_input,
            ground: GroundState::load(&data, mode, true)?,
            ground_runtime: GroundRuntime::load(&data)?,
            ground_profiles: super::ground_runtime::GroundProfiles::load(&data)?,
            ground_settings: std::sync::Arc::new(GroundSettings::load(&data, mode, "smooth")?),
            ground_lifecycle: super::ground_phase::GroundLifecycle::new(),
            biped_ground: super::biped_ground::Owner::load(&data, &animation_metadata)?,
            offboard_contact: Default::default(),
            offboard_air_selector: super::offboard::air_selector::AirSelector::new(
                super::offboard::air_selector::Settings::load(&data)?,
            ),
            offboard_feet: Default::default(),
            offboard_grab: Default::default(),
            animation_input,
            animation_feedback: AnimationFeedback::load(&data)?,
            physical_feedback: super::animation_phase::initial_feedback(),
            skeleton_output,
            skateboard_controller: super::skateboard_controller::SkateboardController::new(),
            board_possession: super::offboard::board_manager::Owner::load(&data)?,
            board_possession_live: super::offboard::board_manager::runtime::LiveState::load(
                &data, physics,
            )?,
        })
    }
}

/// An NPC skater's AI physics record for the next tick (doc 26, "Simulated NPC skaters").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct AiPhysicsSource {
    /// The record (`skate_core::riding::grounded::state::board_path` reads +32 / +48 / +64 / +160).
    pub record: skate_core::animation::output::actor_packet::ExternalPhysicsInput,
    /// Set when the AI wrote `record` this tick (`1904` bit 27); the next packet consumes it.
    pub fresh: bool,
    /// Where the record's trajectory came from this tick, for the `NPC_GRIND_ENTRY` log (b294).
    pub cursor: Option<AiRecordCursor>,
}

/// The line cursor behind an AI record's trajectory (diagnostics only, never read by the simulation).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct AiRecordCursor {
    pub node: u32,
    pub frame_in_segment: u32,
    /// The node whose ext the record read (`ai_record::trajectory_node`).
    pub ext_node: u32,
    pub has_trajectory: bool,
    /// The node `lands_in_grind` started from (`max(node - 1, 0)`).
    pub lands_node: u32,
}
