//! PhysicsSkitching 104 (TU3 vtable `0x82327398`; doc 26h "Skitching step 4e"). Enter `82D47278` + reset
//! `82D47318`, Exit `82B61BB8` (empty), Update `82D477C0`, publication `82D4C078`. The maths lives in
//! `skate_core::riding::skitching` (frame, sub-mode, hold, shimmy, target, spring); this module composes it in
//! retail order (`.local/research/npc/b50-skitch-prestep-update.md` section 1, b51 sections 1-3).
//! Always on in the game since 2026-10-15 (`ground_runtime::skitch::latch_enabled`, set by `LivingWorldPlugin`).
//! The grab point and the hands' grip points are the record's spline at an along position (`82D2D550`, research
//! b148), the chord only while no geometry is published. NOT RETAIL YET: the sub-mode's hard event comes from the
//! shimmy state.
use super::{GamePhysics, SkaterRuntime};
use skate_core::physics::force_queue::QueuedPointForce;
use skate_core::math::Vector3;
use skate_core::player::offboard::grab_scene::Descriptor;
use skate_core::player::selector::conditions::{condition_is_off_ground_skitching, BoardBodyState, TwoStageThresholds};
use skate_core::riding::skitching::{frame, hands, hold, lean, shimmy, target, SkitchSpringInput, SkitchSpringSettings, SkitchSubMode, SkitchSubModeInput, SkitchSubModeSettings};

/// Frames the hands, forearms and head stay out of collision each update (`82D91298(state+28, 5)`).
const CONTACT_OFF_FRAMES: u32 = 5;

/// The state-104 settings (`physics_state_skitching/default`; retail values as defaults, mod-overridable).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct SkitchSettings {
    pub spring: SkitchSpringSettings,
    pub sub_mode: SkitchSubModeSettings,
    pub frame: frame::FrameSettings,
    pub target: target::TargetSettings,
    pub hold: hold::HoldSettings,
    pub shimmy: shimmy::ShimmySettings,
    pub lean: lean::LeanSettings,
    pub hands: hands::HandSettings,
    /// The pre-step's off-ground test (`82D47FC0`: `40AAD3FD99B464F4` 0.1, `F178F963558D24A0` 0.05,
    /// `03A59CDFA0B0B967` 0.1).
    pub off_ground: TwoStageThresholds,
}

impl Default for SkitchSettings {
    fn default() -> Self {
        Self {
            spring: Default::default(),
            sub_mode: Default::default(),
            frame: Default::default(),
            target: Default::default(),
            hold: Default::default(),
            shimmy: Default::default(),
            lean: Default::default(),
            hands: Default::default(),
            off_ground: TwoStageThresholds { field_856_primary: 0.1, field_856_secondary: 0.05, field_7692: 0.1 },
        }
    }
}

/// The per-skater state 104 (state offsets in the field docs; serialisable plain data).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct SkitchState {
    pub settings: SkitchSettings,
    pub frame: frame::FrameState,
    pub sub: SkitchSubMode,
    pub hold: hold::HoldState,
    pub shimmy: shimmy::ShimmyState,
    /// The car's grab record (1312 type / 1316 id).
    pub car: Option<Descriptor>,
    /// 1344 bit 0x80 (the record is ready: Processed 2480 bit 22), 1345 bits 0x20 (board off the ground) / 0x40
    /// (tows fast).
    pub ready: bool,
    pub off_ground: bool,
    pub tows_fast: bool,
    /// The held record runs against the board side this tick (`82D48148` swapped it; logged on change).
    pub reoriented: bool,
    /// 992: time in the state; 784: last frame's target direction (the spring reads it before the target step).
    pub time: f32,
    pub target_dir: [f32; 3],
    /// The hands (`82D4A378`: 1328 / 1332 off flags, weights, 984 / 988).
    pub hands: hands::HandState,
    /// 816: the last spring force.
    pub spring_force: [f32; 3],
    /// The published grab height (280), closing rate (956 -> 284) and along ratio (936 -> 288).
    pub grab_height: f32,
    pub absorb: f32,
    pub along_ratio: f32,
    /// 996 (`82D47BD8`, b61 / b67): the tow speed plus up to 200 x the held record's inverse mass (cap 3.5) toward 8 m/s.
    pub push_speed: f32,
    /// 1345 bit 0x10 (`82D47BD8`, b67): set while the animation carries PushContact (Processed2488 bit 29); while
    /// set, 996 keeps its value.
    pub push_latched: bool,
    /// 940 (the target step's lean yaw, kept across sub-mode 4 frames) and 944 (the smoothed lean angle).
    pub lean_yaw: f32,
    pub lean: f32,
}

fn v(words: &[u32; 72], offset: usize) -> [f32; 3] {
    let i = offset / 4;
    [f32::from_bits(words[i]), f32::from_bits(words[i + 1]), f32::from_bits(words[i + 2])]
}
/// A point through an animation transform (rows; `row3 + x row0 + y row1 + z row2`).
fn affine(t: &[[f32; 4]; 4], p: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|i| t[3][i] + p[0] * t[0][i] + p[1] * t[1][i] + p[2] * t[2][i])
}
fn v4(r: [u32; 4]) -> [f32; 3] {
    [f32::from_bits(r[0]), f32::from_bits(r[1]), f32::from_bits(r[2])]
}

/// Enter `82D47278` + reset `82D47318` (b53 section 4).
pub(crate) fn enter(physics: &mut GamePhysics, skater: &mut SkaterRuntime) -> Result<(), String> {
    physics.board.hook_mut().drive.disable_animation(&mut skater.ground_lifecycle.board_animated_290);
    let words = skater.player_input.processed.grab_records_1888_2176[0];
    let s = &mut skater.skitch_state;
    let settings = s.settings;
    s.frame = frame::FrameState::default();
    s.sub = SkitchSubMode::default();
    s.hold.reset(&settings.hold);
    s.shimmy = shimmy::ShimmyState::default();
    s.car = Some(Descriptor { kind: words[47], id: words[48] });
    s.time = 0.0;
    s.hands = hands::HandState::default();
    s.spring_force = [0.0; 3];
    s.lean_yaw = 0.0;
    s.push_speed = 0.0;
    s.push_latched = false;
    s.lean = 0.0;
    // SKITCH_ENTER: the grabbed record (kind 1 = traffic car; its serial when it is one), so a skitching session
    // shows when and on what each grab started.
    tracing::info!(
        "SKITCH_ENTER kind={} id={:#x} car={:?} speed={:.2}",
        words[47],
        words[48],
        s.held_car_serial(),
        skater.player_input.processed.scalar_2612
    );
    Ok(())
}

/// `82D477C0`.
pub(crate) fn update(physics: &mut GamePhysics, skater: &mut SkaterRuntime) -> Result<(), String> {
    skater.skeleton_collision.disable_handplant_contacts(CONTACT_OFF_FRAMES);
    let p = &skater.player_input.processed;
    let dt = skate_core::riding::skitching::DT;
    let grab_input = p.flags_2476 & 0x0040_0000 != 0;
    let ready = p.flags_2480 & 0x0040_0000 != 0;
    let words = p.grab_records_1888_2176[0];
    let t = skater.player_input.toolkit.as_ref().ok_or("Skitching requires current BoardToolkit")?;
    let mass = t.total_mass;
    // Processed+128..+191 = the effective board transform (82C013F0 / 82C01BF8; b68): row 0 side, row 2 forward,
    // row 3 the deck position.
    let row = |i: usize| [t.effective[i][0], t.effective[i][1], t.effective[i][2]];
    let (board_side, forward, board_position) = (row(0), row(2), row(3));
    let position = v4(p.vectors_544_560_592_608[2]);
    let up = v4(p.vectors_464_480_496_512_528[0]);
    let fields = &skater.animation_input.fields;
    // 924 (`82D47834` / `82D478B0`, b55): with turn (2676) and spin (2672) not of opposite signs, the larger
    // magnitude on turn's side (s = +1 only for turn > 0); else turn.
    let (turn, spin) = (fields.turn, fields.spin);
    let stick = if turn == 0.0 || spin == 0.0 || turn.signum() == spin.signum() {
        let s = if turn > 0.0 { 1.0 } else { -1.0 };
        s * (s * turn).max(s * spin)
    } else {
        turn
    };
    let world_grab_z = skater.animation_input.extra.world_grab_z;
    // 82D4B500: z of Skeleton+14208 (part 0's raw global translation; b55).
    let body_height = skater.animated_skeleton.raw_part_globals[0][3][2];
    let raw_parts = skater.animated_skeleton.raw_part_globals;
    let to_world_m = skater.animated_skeleton.roots.animation_to_world;
    let world_grab_y = skater.animation_input.extra.world_grab_y;
    let board_speed = p.scalar_2612;
    let negate_height = p.flags_2476 & 0x4 != 0;
    let body = BoardBodyState { field_856: physics.riding.wheel_lines.minimum_distance, field_7692: physics.riding.ground.time_without_wheel_contact };
    let geometry = skater.grab_geometry[0].clone();
    let s = &mut skater.skitch_state;
    let settings = s.settings;
    s.ready = ready;
    let mut forces: Vec<QueuedPointForce> = Vec::new();
    let mut board: Option<BoardStep> = None;
    let mut lean_override = None;
    let mut hand_targets: [Option<hands::HandTarget>; 2] = [None; 2];
    let suppress_lean = p.flags_2488 & 0x0080_0000 != 0;
    if ready {
        // Pre-step 82D47FC0.
        s.off_ground = condition_is_off_ground_skitching(body, settings.off_ground);
        // Frame step 82D48148 (record copied from Processed+1888 by 82762AB0).
        // `82D48148` re-orients the held record copy itself (state+1024): with the board side (Processed+128 row 0)
        // against the direction it swaps +64 / +80, negates +112 and toggles the reverse bit (+200 bit 0x20), so
        // every later spline lookup (the grab point, the hands' grip points) runs from the skater's side. Re-orienting
        // only inside the frame step left the hands on the mirrored points (arms crossed, session 2026-10-16).
        let mut endpoints = [v(&words, 64), v(&words, 80)];
        let mut direction = v(&words, 112);
        let reoriented = board_side[0] * direction[0] + board_side[1] * direction[1] + board_side[2] * direction[2] < 0.0;
        if reoriented {
            endpoints.swap(0, 1);
            direction = direction.map(|c| -c);
        }
        if reoriented != s.reoriented {
            tracing::info!("SKITCH_REORIENT {} -> {reoriented} board_side={board_side:.2?} direction={direction:.2?} sub_mode={}", s.reoriented, s.sub.mode);
            s.reoriented = reoriented;
        }
        let half_range = f32::from_bits(words[45]);
        let mid = [(endpoints[0][0] + endpoints[1][0]) * 0.5, (endpoints[0][1] + endpoints[1][1]) * 0.5, (endpoints[0][2] + endpoints[1][2]) * 0.5];
        // 82D2D550 on the re-oriented held copy (ends swapped, +112 negated, reverse bit +200 0x20 toggled): the point on
        // the spline `t` along it from the middle (b148); the chord while no geometry is published.
        let record = geometry.map(|g| {
            let mut w = words;
            if reoriented {
                for i in 0..4 {
                    w.swap(16 + i, 20 + i);
                }
                for word in &mut w[28..31] {
                    *word = (-f32::from_bits(*word)).to_bits();
                }
                w[50] ^= 0x20 << 24;
            }
            skate_core::player::offboard::grab_scene::Record(w, g)
        });
        let chord = move |t: f32| match &record {
            Some(r) => {
                let q = skate_core::player::offboard::grab_scene::at_along(r, t);
                [q[0], q[1], q[2]]
            }
            None => [mid[0] + direction[0] * t, mid[1] + direction[1] * t, mid[2] + direction[2] * t],
        };
        let input = frame::FrameInput { endpoints, direction, half_range, up, position, board_side, shimmy_velocity: s.shimmy.velocity };
        let f = frame::step(&input, &mut s.frame, &settings.frame, &chord);
        s.tows_fast = f.tows_fast;
        // Sub-mode 82D49580 and its release bits.
        let from = s.sub.mode;
        let released_before = s.hold.released();
        // `82D49580` reads the hand-off flags themselves (1328 / 1332, 1 = off: `lwz 1328; cmpwi 0; beq` leaves mode 0),
        // so mode 0 waits for a hand to grip (or the settle time). Passing `!off` left mode 0 at once with both hands
        // off, the hands were never cleared and the hold let go after 0.5 s (headless test, 2026-10-16).
        s.sub.step(
            &SkitchSubModeInput { position: f.along, half_length: half_range, stick, event: s.shimmy.event, car_accel: s.shimmy.car_accel, ready_a: f32::from(u8::from(s.hands.off[0])), ready_b: f32::from(u8::from(s.hands.off[1])) },
            &settings.sub_mode,
            dt,
        );
        s.hold.mode_bits(from, s.sub.mode, s.sub.switched);
        if s.sub.mode != from {
            // SKITCH_SUBMODE: each 82D49580 switch (the tail's SKITCH_MODE line only compares before / after the tick).
            tracing::info!(
                "SKITCH_SUBMODE {from} -> {} along={:.2} half={:.2} stick={stick:.2} turn={turn:.2} spin={spin:.2} hands_off={:?}",
                s.sub.mode,
                f.along,
                half_range,
                s.hands.off
            );
        }
        if s.sub.left_zero {
            s.hold.posed = f.along;
            s.shimmy.target = f.along;
        }
        // Along chain 82D48C98: the grab point's along displacement over the last frame (frames 128 vs 64, after
        // the shift) times 7199.999 (0x822F8BDC; b55).
        let [old, prev, _] = s.frame.frames;
        let at = |fr: &frame::Frame| [fr[3][0] + fr[0][0] * f.along, fr[3][1] + fr[0][1] * f.along, fr[3][2] + fr[0][2] * f.along];
        let (a, b) = (at(&prev), at(&old));
        let accel = f32::from_bits(0x45e0_fffe) * ((a[0] - b[0]) * prev[0][0] + (a[1] - b[1]) * prev[0][1] + (a[2] - b[2]) * prev[0][2]);
        s.shimmy.step(
            &shimmy::ShimmyInput { accel, stick, sub_mode: s.sub.mode, tows_fast: f.tows_fast, along: f.along, edge_point: s.shimmy.edge_point, limit: f.along_limit, tow_speed: f.tow_speed, side_distance: f.side_distance, dt },
            &settings.shimmy,
        );
        // 82D49AB8: the per-mode writes (hands re-grip in sub-mode 1, the edge latch 900 and the outer hand off in 2 /
        // 3, 904 from 864 in 0, the push timer 960), then its hold step 82D49D70.
        skate_core::riding::skitching::mode_writes(
            s.sub.mode,
            f.along,
            half_range,
            f.along_limit,
            s.shimmy.car_accel,
            skate_core::riding::skitching::SkitchModeWrites {
                hold_target: &mut s.hold.target,
                hand_target: &mut s.shimmy.target,
                edge_point: &mut s.shimmy.edge_point,
                edge_latched: &mut s.shimmy.latched,
                snapped: s.shimmy.snapped,
                hands_off: &mut s.hands.off,
                pushed: &mut s.sub.pushed,
            },
            &settings.sub_mode,
            dt,
        );
        s.hold.step(f.along, f.along_rate, f.along_limit, stick, &settings.hold, dt);
        // 82D49AB8's 936 and the publication's grab height (previous frame at the posed hand, minus the skater).
        s.along_ratio = (f.along / half_range.max(1e-6)).clamp(-1.0, 1.0);
        let fr = s.frame.frames[1];
        s.grab_height = fr[3][1] + fr[0][1] * s.hold.posed - position[1];
        s.absorb = -f.axis_distance_rate;
        // 82D47BD8 (b67): unless latched, 996 moves toward 8 m/s by 200 x state+1264 (= the held record's +240,
        // its inverse mass: 82D477C0 copies the record to state+1024), cap 3.5; the PushContact animation attribute
        // (Processed2488 bit 29) latches it until the attribute drops. The reel-in / let-out forces of the same
        // function go to the held object, which retail drops for cars (kind 1, b50): not applied.
        let push_contact = p.flags_2488 & 0x2000_0000 != 0;
        if !s.push_latched {
            let inverse_mass = f32::from_bits(words[60]);
            s.push_speed = f.tow_speed + (8.0 - f.tow_speed).max(0.0).min((200.0 * inverse_mass).min(3.5));
            s.push_latched = push_contact;
        } else if !push_contact {
            s.push_latched = false;
        }
        // Hands 82D4A378: grip points, release flags, IK weights (writes the hand IK after this borrow).
        let m = |p: [f32; 3]| affine(&to_world_m, p);
        let delta = f.car_delta;
        let motion = move |p: [f32; 3]| frame::to_world(&delta, p);
        let part = |i: usize| [raw_parts[i][3][0], raw_parts[i][3][1], raw_parts[i][3][2]];
        hand_targets = s.hands.step(
            &hands::HandInput {
                sub_mode: s.sub.mode,
                lean_yaw: s.lean_yaw,
                posed: s.hold.posed,
                half_range,
                car_velocity: f.car_velocity,
                mirrored: negate_height,
                world_grab: [world_grab_y, world_grab_z],
                hands: [part(3), part(7)],
                shoulders: [part(5), part(9)],
                to_world: &m,
                car_motion: &motion,
                spline: &chord,
                dt,
            },
            &settings.hands,
        );
        // Forces 82D4AC38 (sub-mode 4: steering only, nothing queued).
        if s.sub.mode != 4 {
            let spring = skate_core::riding::skitching::tow_spring(
                &SkitchSpringInput {
                    target_dir: s.target_dir,
                    side_dir: f.side_dir,
                    grab_side_offset: f.grab_side_offset,
                    axis_distance: f.axis_distance,
                    tow_speed: f.tow_speed,
                    axis_distance_rate: f.axis_distance_rate,
                    board_speed,
                    world_grab_z,
                    body_height,
                    negate_height,
                    sub_mode: s.sub.mode,
                    mass,
                },
                &settings.spring,
            );
            let tgt = target::step(
                &target::TargetInput {
                    side_origin: [s.frame.frames[1][3][0] + s.frame.frames[1][2][0] * settings.frame.side_push, s.frame.frames[1][3][1] + s.frame.frames[1][2][1] * settings.frame.side_push, s.frame.frames[1][3][2] + s.frame.frames[1][2][2] * settings.frame.side_push],
                    side_axis: s.frame.frames[1][0],
                    up,
                    position: board_position,
                    facing: forward,
                    hand_along: s.shimmy.target,
                    along: f.along,
                    side_dir: f.side_dir,
                    tows_fast: f.tows_fast,
                    skitch_time: s.time,
                    tow_speed: f.tow_speed,
                    shimmy_by_velocity: s.shimmy.by_velocity,
                    dt,
                },
                &settings.target,
            );
            s.target_dir = tgt.target_dir;
            s.lean_yaw = tgt.lean_yaw;
            s.spring_force = spring;
            board = Some(BoardStep { spring: Some(spring), yaw: f.tows_fast.then_some(tgt.yaw_correction) });
            s.time += dt;
        } else {
            board = Some(BoardStep { spring: None, yaw: None });
        }
        // Lean 82D4A0C0: the board offset's orientation channel.
        if let Some(m) = lean::step(&mut s.lean, s.lean_yaw, suppress_lean, &settings.lean) {
            lean_override = Some(m);
        }
        // Tail 82D4AFB8.
        let (mode, impulse) = s.hold.tail(s.sub.mode, grab_input, s.hands.off[0] && s.hands.off[1], &settings.hold, dt);
        if mode != from || s.hold.released() != released_before {
            // SKITCH_MODE: every sub-mode change and the hold's release, with where along the car the skater is.
            tracing::info!(
                "SKITCH_MODE {from} -> {mode} released={} along={:.2} tow_speed={:.2} side_distance={:.2} hands_off={:?} grab_input={grab_input} lean_yaw_deg={:.1} reach={:.2?}",
                s.hold.released(),
                s.along_ratio,
                f.tow_speed,
                f.side_distance,
                s.hands.off,
                s.lean_yaw.to_degrees(),
                s.hands.reach
            );
        }
        s.sub.mode = mode;
        let impulse_force = match impulse {
            Some(hold::Impulse::PullIn) => Some(target::pull_in_force(f.side_dir, f.axis_distance_rate, f.tow_speed, mass, dt, &settings.target)),
            Some(hold::Impulse::PushOff) => Some(target::push_off_force(f.side_dir, f.axis_distance_rate, mass, dt, &settings.target)),
            None => None,
        };
        if let Some(fv) = impulse_force {
            forces.push(QueuedPointForce { tag: 6, force_world: Vector3::new(fv[0], fv[1], fv[2]), point_body: Vector3::ZERO });
        }
    }
    if let Some(step) = board {
        compose_board(physics, skater, step)?;
    }
    if let Some(m) = lean_override {
        skater.animated_skeleton.board_offset.refresh_orientation(m);
    }
    // 82BD9728 / 82BD97D0: hand A -> IK limb 2, hand B -> limb 3 (the handplant hand IK's slots).
    for (h, t) in hand_targets.iter().enumerate() {
        if let Some(t) = t {
            let limb = 2 + h;
            skater.foot_ik.state.external_targets[limb].world_position = [t.position[0], t.position[1], t.position[2], 1.0];
            skater.foot_ik.state.limbs[limb].external_target_set = true;
            skater.foot_ik.state.limbs[limb].target_blend = t.weight;
        }
    }
    let q = physics.board.forces_mut();
    for force in forces {
        q.append(force);
    }
    // Step 14 (b50 table row 14, gated): `82585DD8` keeps the held record (Player+188 / +192) and the grab interface
    // re-queries the held object's spline (`[Player+12816]` vt+44, slot 11), so the owner publishes it again next tick
    // and Processed 2480 bit 22 stays set while the car is held. Without it the record was published once (from the
    // latch's request) and the skater dropped back to the ground after two ticks (headless test, 2026-10-16).
    if ready {
        if let Some(car) = skater.skitch_state.car {
            skater.offboard_grab.request_primary(car);
        }
    }
    // Always: reckoning (82D8E5C0 / 82D8C8F0) and the skeleton ground update (82BDF530), as Slide does.
    let p = &skater.player_input.processed;
    let t = skater.player_input.toolkit.as_ref().ok_or("Skitching requires current BoardToolkit")?;
    physics.riding.update_slide_reckoning(p, t, skater.animation_input.extra.physical_body_spin, skater.animation_input.fields.balance);
    super::input_phase::update_ground(physics, skater)
}

/// What the gated body hands to the board composition: the spring force (None in sub-mode 4) and the yaw
/// correction (only while towing fast).
struct BoardStep {
    spring: Option<[f32; 3]>,
    yaw: Option<[f32; 3]>,
}

struct Angle;
impl skate_core::physics::manual::controller::ManualAngleMeasurement for Angle {
    type Error = std::convert::Infallible;
    fn angle_between(&mut self, a: [f32; 4], b: [f32; 4], c: [f32; 4]) -> Result<f32, Self::Error> {
        let v = |x: [f32; 4]| Vector3::new(x[0], x[1], x[2]);
        Ok(skate_core::riding::collision_response::signed_angle(v(a), v(b), v(c)))
    }
}

/// `82D4AC38` (b51 section 2, arguments b56): capture, spring (tag 6), slide friction (tag 1, heading time = the
/// sub-mode time 948), tilt (no push / damped-turn history; 0 while the board is off the ground), speed wobble
/// (skipped off the ground), anti-flip, truck targets, heading (when balance and spin are both set), manual
/// (powersliding off), the manual and anti-flip displacements, the yaw correction (`82C07328`) while towing fast.
/// Sub-mode 4: tilt, wobble and truck targets only.
fn compose_board(physics: &mut GamePhysics, skater: &mut SkaterRuntime, step: BoardStep) -> Result<(), String> {
    use skate_core::physics::manual::controller;
    use skate_core::riding::{anti_flip, heading, slide_friction, speed_wobble, steering};
    let target = skater.animated_skeleton.board_frames.animation_target;
    skater.skeleton_air.capture_physics_error(&physics.board, &target);
    let p = &skater.player_input.processed;
    let t = skater.player_input.toolkit.as_ref().ok_or("Skitching requires current BoardToolkit")?;
    let edge = skater.ground_lifecycle.edge;
    let mut input = skater.ground_settings.input(
        t,
        p,
        &skater.animation_input,
        &skater.ground.pumping,
        skater.ground.pumping_settings.mode(p.state_variant_index_2528)?.unintentional_scalar,
        &physics.riding,
        &skater.animated_skeleton,
        physics.settings.step.base_truck_transforms,
        super::ground_runtime::GroundInputObservations {
            manual_drag_2724: skater.ground_lifecycle.manual_drag_2724,
            trajectory_state_bits: p.external_physics_1616.flags,
            edge_flags: edge.map_or(0, |e| e.flags),
            edge_point: edge.map_or([0.0; 4], |e| e.point),
        },
    );
    let settings = skater.ground_settings.board();
    let off_ground = skater.skitch_state.off_ground;
    let mut tilt = if off_ground { 0.0 } else { steering::calculate_tilt(settings.steering, input.steering, None, None) };
    if !off_ground {
        input.speed_wobble.tilt = tilt;
        input.speed_wobble.center_of_mass_height = skate_core::riding::ground_correction_math::center_of_mass_height(skater.animated_skeleton.record.com_to_deck_world);
        tilt = speed_wobble::calculate(&mut skater.ground.wobble, settings.speed_wobble, input.speed_wobble);
    }
    let Some(spring) = step.spring else {
        skater.ground.steering.update(tilt, settings.steering.tilt_blending, p.flags_2468, p.flags_2472);
        return Ok(());
    };
    input.slide_friction.heading_time = skater.skitch_state.sub.time;
    let friction = slide_friction::calculate(settings.slide_friction, &input.slide_friction);
    let anti = anti_flip::calculate(settings.anti_flip, &input.anti_flip);
    skater.ground.steering.update(tilt, settings.steering.tilt_blending, p.flags_2468, p.flags_2472);
    if skater.animation_input.fields.balance != 0.0 && p.spin_input_2672 != 0.0 {
        let h = heading::calculate(settings.heading, &input.heading, &mut skater.ground.heading_previous);
        skater.ground_runtime.apply_angular_target(&mut physics.board, h);
    }
    input.manual.powersliding = false;
    let manual = controller::calculate(&mut skater.ground.manual, settings.manual, settings.manual_mode, &input.manual, &mut Angle)
        .map_err(|e| format!("Skitching manual controller: {e:?}"))?;
    skater.ground_runtime.apply_angular_displacement(&mut physics.board, manual.angular_displacement);
    skater.ground_runtime.apply_angular_displacement(&mut physics.board, anti);
    if let Some(y) = step.yaw {
        skate_core::physics::deck_angular_correction::apply_axis_displacement(
            &mut physics.board.bodies_mut()[skate_core::physics::board::BodyId::Deck.index()].rates,
            Vector3::new(y[0], y[1], y[2]),
        );
    }
    let q = physics.board.forces_mut();
    q.append(QueuedPointForce { tag: 6, force_world: Vector3::new(spring[0], spring[1], spring[2]), point_body: Vector3::ZERO });
    q.append(QueuedPointForce { tag: 1, force_world: Vector3::new(friction[0], friction[1], friction[2]), point_body: Vector3::new(friction[4], friction[5], friction[6]) });
    Ok(())
}

/// What `82D4C078` publishes.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct SkitchOutput {
    pub flag_304: bool,
    pub counter_36: u32,
    pub skitch_value_40: u32,
    pub scalar_292: f32,
    pub grab_height_280: f32,
    pub absorb_284: f32,
    pub along_288: f32,
    pub push_308: f32,
    pub shimmy_136: f32,
    pub grip_132: f32,
    pub hands_140: u32,
}

impl SkitchState {
    /// The traffic car this skater holds this tick (`82C361E8` runs every gated frame): its serial, decoded from
    /// the car's grab spline id (`living_world::vehicles::CAR_GRAB_TAG | serial << 3 | index`).
    pub(crate) fn held_car(&self) -> Option<u32> {
        self.ready.then(|| self.held_car_serial()).flatten()
    }

    /// The held record's traffic car serial, ready or not.
    fn held_car_serial(&self) -> Option<u32> {
        let car = self.car?;
        (car.kind == 1 && car.id & crate::living_world::vehicles::CAR_GRAB_TAG != 0)
            .then_some((car.id & !crate::living_world::vehicles::CAR_GRAB_TAG) >> 3)
    }

    /// `82D4C078`: flag_304 only while ready and not released; the rest every frame.
    pub(crate) fn output(&self) -> SkitchOutput {
        let car = self.car.unwrap_or(Descriptor { kind: 0, id: 0 });
        SkitchOutput {
            flag_304: self.ready && !self.hold.released(),
            counter_36: car.kind,
            skitch_value_40: car.id,
            scalar_292: self.hold.regrab_block,
            grab_height_280: self.grab_height,
            absorb_284: self.absorb,
            along_288: self.along_ratio,
            push_308: self.push_speed,
            shimmy_136: self.hold.posed_step * 60.0,
            grip_132: self.hands.grip_height,
            hands_140: self.hands.mask,
        }
    }
}
