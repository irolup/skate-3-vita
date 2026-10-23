use super::{GamePhysics, SkaterRuntime};
mod entry;
pub(crate) mod feet;
pub(crate) mod grab;
pub(crate) mod grab_runtime;
pub(crate) mod ground_board;
mod lifecycle;
pub(crate) mod services;
pub(crate) mod sync;
pub(crate) use lifecycle::{fill, post, publish_fields};
use skate_core::player::offboard::{contact_toolkit, ground_entry, ground_job};
pub(crate) struct Owner {
    pub controller: skate_core::player::offboard::controller::Controller,
    pub result: Option<skate_core::player::offboard::controller::GroundResult>,
    pub ground: ground_entry::State,
    pub contact: contact_toolkit::ContactPrefix,
    pub geometry_adjustment: Option<skate_core::player::offboard::ground_query::GroundAdjustment>,
    skeleton_state: super::offboard::skeleton_ground::State,
    geometry: super::offboard::ground_geometry::State,
    movement_vs_stick_angle: skate_core::point_graph::PointGraph<8>,
    turn_vs_stick_angle: skate_core::point_graph::PointGraph<8>,
    air_launch: skate_core::player::offboard::air_launch::Settings,
    collision_settings: skate_core::player::offboard::ground_lifecycle::CollisionSettings,
    pub grab_settings: skate_core::player::offboard::ground_sync::BoardSettings,
    /// The last `WALKER_STEP` (prop, outcome), so the line is written on change only.
    step_log: Option<(u32, &'static str)>,
}

impl Owner {
    ///Only the shared82D8E3E0 callee, not Ground movement/lifecycle.
    ///Ground82BDE2CC and Air82BDDFFC both supply enable-body-spin r7=1.
    ///Each caller supplies its own native up/forward/blend and phase ordering.
    pub(crate) fn finish_reckoning(
        &self,
        update: super::offboard::skeleton_ground::ReckoningUpdate,
        riding: &mut super::riding_outputs::RidingOutputs,
        air: &mut super::air_reckoning::AirReckoning,
        processed: &skate_core::player::input_phase::ProcessedPhysicsInput,
        physical_body_spin: f32,
    ) -> skate_core::player::offboard::ground_reckoning::Output {
        self.skeleton_state
            .finish_reckoning(update, riding, air, processed, physical_body_spin)
    }

    pub(crate) fn air_settings(
        &self,
    ) -> (
        skate_core::point_graph::PointGraph<8>,
        skate_core::player::offboard::air_launch::Settings,
    ) {
        (self.turn_vs_speed(), self.air_launch)
    }

    fn turn_vs_speed(&self) -> skate_core::point_graph::PointGraph<8> {
        self.controller.settings.movement_velocity.turn_vs_speed
    }

    pub(crate) fn load(
        data: &skate_data::collections::Collections,
        metadata: &skate_data::animation_metadata::AnimationMetadata,
    ) -> Result<Self, String> {
        let settings = super::offboard::settings::Settings::load(data, metadata)?;
        let grab_settings = settings.board;
        let (
            controller_settings,
            metrics,
            movement_vs_stick_angle,
            turn_vs_stick_angle,
            air_launch,
        ) = settings.into_controller_parts();
        Ok(Self {
            controller: skate_core::player::offboard::controller::Controller::new(
                controller_settings,
                metrics,
            ),
            result: None,
            ground: ground_entry::State::default(),
            contact: contact_toolkit::ContactPrefix::reset(),
            geometry_adjustment: None,
            skeleton_state: super::offboard::skeleton_ground::State::load(data)?,
            geometry: super::offboard::ground_geometry::State::load(data)?,
            movement_vs_stick_angle,
            turn_vs_stick_angle,
            air_launch,
            collision_settings: lifecycle::load_collision_settings(data)?,
            grab_settings,
            step_log: None,
        })
    }

    pub(crate) fn reset(&mut self, toolkit: &mut contact_toolkit::Owner) {
        //82D30BD0 is a Ground-state reset, not Biped82D7B1C0.
        //Keep controller cadence, correction vectors and shared candidate alive.
        self.result = None;
        self.ground.flags_144_to_150 = [false; 7];
        self.ground.counter_152 = 0;
        self.ground.counter_156 = 0;
        self.ground.elapsed_160 = 0.;
        toolkit.reset_history();
        self.ground = ground_entry::State::default();
        self.contact = contact_toolkit::ContactPrefix::reset();
        self.geometry_adjustment = None;
    }

    pub(crate) fn run(
        &mut self,
        job: skate_core::player::offboard::controller::GroundJob,
    ) -> skate_core::player::offboard::controller::GroundResult {
        let result = self.controller.step_ground(&job);
        self.result = Some(result);
        result
    }

    pub(crate) fn enter(
        &mut self,
        input: ground_entry::Input,
        current_state: u32,
        previous_frame: [[f32; 4]; 4],
        toolkit: &mut contact_toolkit::Owner,
    ) {
        self.reset(toolkit);
        self.geometry.reset();
        let placement = self.ground.enter(&input);
        self.controller
            .place(skate_core::player::offboard::controller::PlacementInput {
                frame: placement.frame,
                velocity: placement.planar_velocity,
                body_position: placement.body_position,
                current_state,
                previous_state: input.previous_state_2504,
                previous_frame,
            });
    }
}


pub(crate) fn enter(_physics: &mut GamePhysics, skater: &mut SkaterRuntime) -> Result<(), String> {
    let p = &skater.player_input.processed;
    //82D30848 calls GetEffectiveRoot82BE3650 BEFORE Ground placement.
    let frame = entry::effective_root(
        skater.animated_skeleton.roots.animation_to_world,
        p.flags_2476,
    );
    let previous_frame = p
        .effective_anim_transform_192
        .map(|v| v.map(f32::from_bits));
    skater.biped_ground.enter(
        ground_entry::Input {
            animation_frame: frame,
            processed_velocity_608: p.vectors_544_560_592_608[3].map(f32::from_bits),
            processed_flags_2484: p.flags_2484,
            processed_flags_2476: p.flags_2476,
            requested_angle_2936: skater.animation_input.extra.biped_start_angle,
            requested_duration_2896: skater.animation_input.fields.animation_time,
            previous_state_2504: p.state_2504,
            previous_frame_up_208: previous_frame[1],
            //82BDE398..3E0: COM frame15824 + translation48 = Skeleton15872.
            body_position_15872: skater.animated_skeleton.board_frames.com_frame[3],
        },
        p.state_2508,
        previous_frame,
        &mut skater.offboard_contact,
    );
    //82D30B44: actual state56 ConsiderReset, after placement and before mode4.
    services::enter_feet(skater);
    //82D30B54 requests the shared skeleton collision controller's mode4.
    skater
        .ground_lifecycle
        .skeleton_controller
        .request(4, &mut skater.skeleton_collision)?;
    //82D30B5C..B64: state52 grab manager, NOT SkateboardController60.
    skater.offboard_grab.enter_reset();
    Ok(())
}

pub(crate) fn exit(skater: &mut SkaterRuntime) {
    //82D30CD8 refreshes, then30CE4..CF8 clears readiness/history before
    //releasing Ground geometry. Candidate and Biped cadence remain alive.
    skater.offboard_contact.reset_history();
    //82D30CC0 releases the ground query. It does NOT reset shared Biped
    //motion/cadence/correction; BipedAir and subsequent placement consume them.
    skater.biped_ground.geometry.reset();
}

/// Diagnostics for walking onto props (bench walk-over bumps, laptop 2026-10-18): which prop the walker's contact
/// target or edge is on, the step height above the feet, and whether the step was taken or why not. Written when the
/// prop or the outcome changes. Outcomes: `step` (target contact on the prop, rise over 5 cm: the contact
/// classifier's step threshold 82D837C8), `flat` (target on the prop, no rise), `no-candidate` (packet flag 1024: no
/// candidate before the forward limit), `obstacle` (contact correction pushes the walker back), `no-target`.
fn log_step(owner: &mut Owner, physics: &GamePhysics, prefix: contact_toolkit::ContactPrefix, frame: [[f32; 4]; 4]) {
    let Some(props) = physics.prop_dynamics() else { return };
    let prop_at = |p: [f32; 4]| props.nearest_body(skate_core::math::Vector3::new(p[0], p[1], p[2]), 0.1).map(|(id, _)| id);
    let target = prefix.flags_176 & 2 != 0;
    let prop = target
        .then(|| prop_at(prefix.target_position))
        .flatten()
        .or_else(|| (prefix.flags_176 & 4 != 0).then(|| prop_at(prefix.edge_position)).flatten());
    let Some(prop) = prop else {
        if let Some((id, _)) = owner.step_log.take() {
            tracing::info!("WALKER_STEP prop={id} left");
        }
        return;
    };
    let rise = prefix.target_position[1] - frame[3][1];
    let outcome = if target && rise > 0.05 {
        "step"
    } else if target {
        "flat"
    } else if prefix.flags_176 & 1024 != 0 {
        "no-candidate"
    } else if owner.controller.state.contact.active {
        "obstacle"
    } else {
        "no-target"
    };
    if owner.step_log == Some((prop, outcome)) {
        return;
    }
    owner.step_log = Some((prop, outcome));
    let edge_rise = prefix.edge_position[1] - frame[3][1];
    let speed = owner.controller.state.motion.speed_704;
    tracing::info!(
        "WALKER_STEP prop={prop} ({}) {outcome} rise {rise:.3} m edge_rise {edge_rise:.3} m ahead {:.2} m speed {speed:.2} m/s flags {:#x} kind {} at [{:.2}, {:.2}, {:.2}]",
        props.describe_template(prop),
        prefix.distance_168.min(99.0),
        prefix.flags_176,
        prefix.kind_164,
        frame[3][0],
        frame[3][1],
        frame[3][2],
    );
}

pub(crate) fn update(
    physics: &mut GamePhysics,
    skater: &mut SkaterRuntime,
    _controls: &super::PlayerControls,
    contact: ground_job::ContactSnapshot,
) -> Result<contact_toolkit::Input, String> {
    let p = skater.player_input.processed;
    let fields = skater.animation_input.fields;
    let extra = skater.animation_input.extra;
    let animation_motion_sq = fields.animation_translation[..3]
        .iter()
        .map(|value| value * value)
        .sum::<f32>();
    if p.flags_2488 & 0x0800_0000 != 0
        && skater.biped_ground.controller.state.motion.target_scale_784 < 0.0
        && animation_motion_sq < f32::from_bits(0x3a83_126f)
        && !(fields.animation_time > 0.0)
    {
        // Temporary first-producer trace. Native Ground movement82D7F4BC enters
        // the directed branch and82D7F624 divides target distance by AnimTime;
        // stock graph publication must make this packet valid before that call.
        tracing::error!(
            "OFFBOARD_DIRECTED_INPUT_INVALID state={}/{} previous={} filtered={} motion_state={:?} flags={:08x}/{:08x}/{:08x}/{:08x}/{:08x} grind_words={:08x?} grind_flags={:02x}/{:02x} offboard_edge_distance={} offboard_obstacle_distance={} anim_time={} anim_translation={:?} anim_velocity={:?} target_scale={} frame={:?} attributes={:?}",
            p.state_2508,
            p.category_2512,
            p.state_2504,
            skater.player_input.physical.filtered_state_0,
            skater.animation.motion_controller.frame.current,
            p.flags_2468,
            p.flags_2472,
            p.flags_2476,
            p.flags_2480,
            p.flags_2488,
            p.grind_words_2532_2536,
            skater.player_input.physical.grinds.flag_318,
            skater.player_input.physical.grinds.flag_322,
            skater.player_input.physical.off_board.distance_116,
            skater.player_input.physical.off_board.scalar_112,
            fields.animation_time,
            fields.animation_translation,
            skater.animated_skeleton.motion.velocity_world,
            skater.biped_ground.controller.state.motion.target_scale_784,
            skater.biped_ground.ground.frame_80,
            skater.animation.attributes.entries(),
        );
    }
    let frame = skater.biped_ground.ground.frame_80;
    // Move Object (502): the stock graph swaps OBGround's locomotion
    // AttachIntents for MovingObjectNew's OB_ObjectMv set, and retail moves
    // the OBJECT from them (82D45318, prop_carry) while the skater follows the
    // object's grab edge: 82D45D30 builds the skater's target frame from the
    // grab record, 82D46218 / 82D463D8 blend toward it, 82BDF268 moves the
    // character there. The frame blend is a snap at retail data (it only
    // blends when the frame jumps >= 60 m per tick: physics_state_offboard
    // +448 = 1.0 x 60, 82D46218). The root yaw is NOT that frame: 82D46610
    // writes +1124 = wrapped angle to the edge frame x curve 023D73966B8669A5
    // at the time held, and 82BE1058 (from 82BDF268) turns the character by
    // it, so the facing eases in over the first 0.5 s of a grab (0.18 of the
    // angle at the grab) and turns the whole angle from then on (b243 / b245).
    // The skater target is the root moved by the step of the
    // retail follow point +416 published by prop_carry (the step is bounded
    // there, 82BD41B0), one tick behind the prop. NOT RETAIL YET: 82BDF268's
    // translation (the +288 hands step, then a pull toward +416 of at most
    // 0.3 m); the walking job's velocity override moves the
    // root onto the target each tick. The walking stick stays idle.
    let moving_object = (p.state_2508 == 502 && physics.prop_carry.held().is_some())
        .then(|| physics.prop_carry.skater_target())
        .flatten()
        .map(|(target, facing)| {
            let root = skater.animated_skeleton.roots.animation_to_world[3];
            let dt = physics.settings.step.simulation.time_step.max(1e-4);
            let (dx, dz) = (target.x - root[0], target.z - root[2]);
            let velocity = [dx / dt, 0.0, dz / dt, 0.0];
            let wrap = |a: f32| (a + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
            let turn = wrap(facing.x.atan2(facing.z) - frame[2][0].atan2(frame[2][2])) * physics.prop_carry.root_yaw_gain();
            (velocity, turn)
        });
    let (stick_x, stick_z) = if moving_object.is_some() {
        (0.0, 0.0)
    } else {
        (extra.biped_world_x, extra.biped_world_z)
    };
    let input = skate_core::player::offboard::ground_input::GroundInput {
        processed_flags_2472: p.flags_2472,
        processed_direct_2684: extra.offboard_magnitude,
        processed_direct_2680: extra.offboard_turn,
        processed_stick_2692: stick_x,
        processed_stick_2688: stick_z,
        processed_scale_2912: fields.magnitude_scale,
        processed_scale_2908: fields.turn_scale,
        frame_forward_112: [frame[2][0], frame[2][1], frame[2][2]],
    };
    let prepared = skate_core::player::offboard::ground_input::calculate(
        &input,
        &skater.biped_ground.movement_vs_stick_angle,
        &skater.biped_ground.turn_vs_stick_angle,
    );
    let third_line = p.line_tests_960_1008_1056[2];
    let owner = &mut skater.biped_ground;
    let mut job = ground_job::prepare(
        &mut owner.contact,
        &mut owner.ground.distance_164,
        ground_job::Input {
            contact,
            frame,
            previous_state: p.state_2504,
            third_line_position: (third_line.valid != 0)
                .then(|| third_line.position.map(f32::from_bits)),
            frames_since_teleport: p.frames_since_teleport_2584,
            processed_position: p.vectors_544_560_592_608[2].map(f32::from_bits),
            processed_velocity: p.vectors_544_560_592_608[3].map(f32::from_bits),
            controls: prepared,
            collision_displacements: skater.collision_extra_errors,
            animation_motion: fields.animation_translation,
            animation_velocity: skater.animated_skeleton.motion.velocity_world,
            requested_duration: fields.animation_time,
            requested_phase: fields.cadence_end_percent,
            override_duration: fields.animation_physics_blend_seconds,
            flags_2472: p.flags_2472,
            flags_2476: p.flags_2476,
            flags_2480: p.flags_2480,
            flags_2484: p.flags_2484,
            flags_2488: p.flags_2488,
        },
        |input| Ok::<_, String>(owner.geometry.consume(input)),
    )?;
    owner.geometry_adjustment = Some(job.geometry);
    if let Some((velocity, angle)) = moving_object {
        // Face the grab edge (the prop's turn carries the skater round).
        if angle != 0.0 {
            let motion = &mut owner.controller.state.motion;
            motion.frame_0[0] = super::prop_carry::yaw_row(motion.frame_0[0], angle);
            motion.frame_0[2] = super::prop_carry::yaw_row(motion.frame_0[2], angle);
        }
        // Drive the planar velocity through the controller's velocity
        // override (gate >= 0, zero blend time: the target is taken as is),
        // so contacts and obstacle rejection still apply. Every held tick,
        // also a zero step: retail moves the character to the follow point
        // each tick (82D44A10 -> 82BDF268), so a skater already at the point
        // stays there. Gating a zero step out handed the root back to the
        // walking approach, which walked the skater into a resting prop
        // until the hold rule failed (2026-10-08, exposed by the retail row
        // solver: a prop at rest no longer jitters the follow point).
        {
            // The override gate is the job's requested phase (296), which the
            // cadence 82D80720 also takes as a phase request over the
            // requested duration (288). Request the phase the cadence is at
            // over a positive duration: the walk cycle holds still while
            // carrying. Phase 0 over the carry state's zero duration divided
            // by zero (2026-10-05 crash: NaN CadenceStartPercent picked no
            // walk-into-stand clip when the prop was let go).
            job.job.requested_phase = owner.controller.state.cadence.phase.phase;
            job.job.requested_duration = 1.0;
            job.job.override_duration = 0.0;
            job.job.animation_velocity = velocity;
            // NOT RETAIL YET (part of the follow move above): the walking
            // controller's approach (82D7F458..FDD0) steps toward the
            // contact target projected onto the FACING line, so following a
            // prop that slides along its edge would come out as a forward
            // step. Retail 502 never runs that job (82D4E2F8 is only
            // submitted from BipedGround 82D30D30); it moves the character
            // with 82BDF268, not decoded. Until it is, the follow move drops
            // the target-contact bit (2) for this job only (support contact
            // bit 1 still keeps the feet on the ground; no 0.3 m step-up).
            job.job.flags &= !2;
        }
    }
    let previous_position=owner.controller.state.position_368;
    let mut result=owner.run(job.job);
    super::solid_contacts::constrain_ground(owner,&mut result,previous_position,&physics.network_proxies.solids);
    log_step(owner, physics, contact.prefix, frame);
    if result
        .physical_frame
        .iter()
        .flatten()
        .chain(result.animation_frame.iter().flatten())
        .chain(result.surface_frame.iter().flatten())
        .chain(result.velocity.iter())
        .chain(result.position.iter())
        .any(|value| !value.is_finite())
    {
        // Temporary first-divergence trace; do not repair or suppress the value.
        tracing::error!(
            "OFFBOARD_GROUND_RESULT_NONFINITE job={:?} result={:?} controller={:?}",
            job.job,
            result,
            owner.controller.state,
        );
    }
    //82D31CE8..D04 is Ground FeetIK, not Air's timer-reset wrapper.
    services::update_feet(skater);
    //82D31D14 calls the shared board steering owner with zero input before
    //possession. Offboard Ground must still advance its tilt and timers.
    skater.ground.steering.update(
        0.0,
        skater.air_settings.steering_blend,
        p.flags_2468,
        p.flags_2472,
    );
    //82D31D1C..DB0 operates the EXISTING carried-board controller.
    ground_board::update_possession(physics, skater);
    //82D32338 precedes contact correction and angular unwind. Submit through
    //the parent's persistent selector; Ground never selects Air speculatively.
    //Current authored scene has no grind edges. Typed absence is accepted by
    //every native non-mode1 launch; no fabricated departure vectors.
    let launch_input = services::launch_input(skater, None)?;
    if let Some(packet) = skater.biped_ground.prepare_air(&launch_input)? {
        services::submit_air(physics, skater, packet)?;
    }
    let owner = &mut skater.biped_ground;
    owner.ground.flags_144_to_150[0] = false;
    // Sync82D31E64..82D32070: physical contact correction and the Enter
    // angular unwind must precede BOTH Skeleton and the next query submission.
    let animation_frame = ground_job::sync_frames(&mut owner.ground, owner.contact, result);
    let collision = super::input_phase::collision(skater);
    //82D32084 passes state1056 (GroundResult208), not processed752.
    let centre_of_mass = result.position;
    let body_spin = skater.animation_input.extra.physical_body_spin;
    let simulation = physics.settings.step.simulation;
    let mut owners = super::skeleton_input_runtime::SkeletonOwners {
        animated: &mut skater.animated_skeleton,
        body: &mut skater.skeleton,
        drives: &mut skater.skeleton_drives,
        ik: &mut skater.foot_ik,
        animation_input: &mut skater.animation_input,
        correction: &mut skater.skeleton_output.correction,
        pose_errors: &mut skater.pose_errors,
    };
    let mut reckoning_update = None;
    skater.skeleton_input.update_biped_ground(
        &mut skater.skeleton_air,
        &mut physics.board,
        crate::physics::offboard::skeleton_ground::Input {
            world_frame: &animation_frame,
            centre_of_mass_1056: centre_of_mass,
        },
        &mut skater.biped_ground.skeleton_state,
        &mut skater.player_input.processed,
        &mut owners,
        &skater.animation.packet.hierarchy,
        &collision,
        simulation,
        |update| {
            reckoning_update = Some(update);
            Ok(())
        },
    )?;
    if let Some(update) = reckoning_update {
        skater.biped_ground.skeleton_state.finish_reckoning(
            update,
            &mut physics.riding,
            &mut skater.air_reckoning,
            &skater.player_input.processed,
            body_spin,
        );
    }
    //82D32088..32108: real grab-spline host executes grab::sync here.
    services::sync_grab(skater);
    //82D32130..164: surface-frame axes, NOT physical-frame axes.
    Ok(contact_toolkit::Input {
        position: skater.biped_ground.ground.frame_80[3],
        forward: result.surface_frame[2],
        up: result.surface_frame[1],
        right: result.surface_frame[0],
        velocity: result.velocity,
        animation_up: animation_frame[1],
        animation_right: animation_frame[0],
    })
}

/// Call immediately AFTER submitting update's toolkit input through the shared
/// owner. The seven real line results become visible at the NEXT PreUpdate.
pub(crate) fn submit_geometry(
    physics: &GamePhysics,
    skater: &mut SkaterRuntime,
) -> Result<(), String> {
    let p = &skater.player_input.processed;
    let owner = &mut skater.biped_ground;
    let result = owner
        .result
        .ok_or("Ground geometry submission requires completed motion")?;
    owner.geometry.submit(
        physics.biped_world(),
        &physics.network_proxies.solids,
        owner.ground.frame_80,
        result.velocity,
        skate_core::player::offboard::ground_query::QueryContext {
            selection_flags_2948: p.actor_query_2948,
            matching_id_2952: p.actor_query_2952 as i32,
        },
        p.flags_2488,
    )
}
