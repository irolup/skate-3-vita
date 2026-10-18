//! Original TU3 trajectory launch calculations82D67D08/82D67320/82D682E8.
use crate::madd::Madd as _;
use super::{LaunchInfo, QueryRequest, SelectorInput, SelectorSettings, Trajectory, math::*};

pub(super) fn adjust_velocity(
    info: &mut LaunchInfo,
    input: SelectorInput,
    s: &SelectorSettings,
) -> bool {
    let n = input.ground_normal;
    let v = info.start_velocity;
    let mut normal = scale(n, dot(v, n));
    normal[1] = 0.0;
    let combined = add([0.0, v[1], 0.0, 0.0], normal);
    let residual = sub(v, combined);
    let lean = (input.directional_input - 0.25).max(-1.0).min(1.0);
    let aligned = normalize(sub(UP, scale(n, lean * s.vert_jump_align_max_angle)));
    if n[1] >= s.vert_jump_align_max_ground_normal_y {
        return false;
    }
    if normalize(combined)[1] > s.vert_jump_align_min_direction_y {
        //The second term really uses UNIT velocity, not the original speed.
        let left = scale(
            add(scale(aligned, length(combined)), residual),
            s.vert_jump_align_factor,
        );
        let right = scale(normalize(v), 1.0 - s.vert_jump_align_factor);
        info.start_velocity = scale(normalize(add(left, right)), length(v));
        true
    } else {
        if !info.player_jumped && input.previous_physics_state != 200 {
            let normal = scale(n, dot(v, n));
            info.start_velocity = add(
                scale(normal, s.natural_air_off_verts_scalar),
                sub(v, normal),
            );
        }
        false
    }
}

pub(super) fn candidate_velocities(info: LaunchInfo, s: &SelectorSettings) -> Vec<Vector> {
    let count = usize::from(info.trajectory_count.min(7));
    let mut velocities = Vec::with_capacity(count);
    if count == 0 {
        return velocities;
    }
    let v = info.start_velocity;
    velocities.push(v);
    let right = normalize(cross(UP, v));
    let forward = normalize(cross(right, UP));
    let speed = length(v);
    let cone_speed = if info.player_jumped {
        length(info.com_velocity)
    } else {
        speed
    };
    let radians = f32::from_bits(0x3c8e_fa35);
    let x = info.cone_angle_x * radians;
    let z = (s.cone_angle_z_vs_speed.evaluate(cone_speed * 0.05) * info.cone_angle_z) * radians;
    let cone_speed = cone_speed.max(s.speed_factor_min).min(s.speed_factor_max);
    let right = scale(right, crate::trigonometry::sin(x) * cone_speed);
    let forward = scale(forward, crate::trigonometry::sin(z) * cone_speed);
    if count > 1 {
        let step = f32::from_bits(0x40c9_0fdb) / (count - 1) as f32;
        let mut angle = 0.0;
        for _ in 1..count {
            let candidate = madd(
                forward,
                crate::trigonometry::cos(angle),
                madd(right, crate::trigonometry::sin(angle), v),
            );
            velocities.push(scale(normalize(candidate), length(candidate).min(speed)));
            angle += step;
        }
    }
    velocities
}

#[derive(Clone, Debug)]
pub(super) struct LaunchBatch {
    pub requests: Vec<QueryRequest>,
    pub velocities: Vec<Vector>,
    pub origin: Vector,               //2864
    pub board_position: Vector,       //2848
    pub local_board_position: Vector, //2832
    pub local_com_position: Vector,   //2784
    pub com_displacement: Vector,     //2768
    ///9659: the recorded AI arc was cast (sub_82D68C80 then skips the grind lock and pass 2).
    pub recorded: bool,
}
pub(super) fn batch(info: LaunchInfo, input: SelectorInput, s: &SelectorSettings) -> LaunchBatch {
    let velocities = candidate_velocities(info, s);
    let ground = s
        .displacement_vs_ground_normal
        .evaluate(input.ground_normal[1].abs());
    let speed = s
        .displacement_vs_speed
        .evaluate(input.board_vertical_velocity * 0.1);
    let blend = speed.max(ground);
    let height = dot(
        sub(info.animation_com_position, input.contact_position),
        info.reckoning_transform[1],
    ) - s.trajectory_radius;
    let displacement =
        height.min(s.trajectory_displacement) * (1.0 - blend) + s.trajectory_displacement * blend;
    let mut origin = sub(
        info.animation_com_position,
        scale(info.reckoning_transform[1], displacement),
    );
    let mut board_position = info.board_position;
    if info.use_position_override {
        origin = info.start_position_override;
        board_position = info.board_position_override;
    }
    let com_displacement = sub(info.animation_com_position, origin);
    let duration = max_time(info.start_velocity, s);
    //sub_82D682E8 -> sub_82D67B50 / sub_82D67A00: an AI skater taking off near its recorded jump
    //casts that one arc (count 1, flag 9659); the computed start velocity (2192) stays.
    if let Some(arc) = input
        .recorded_arc
        .filter(|arc| {
            let window = (s.recorded_arc_radius_squared, s.recorded_arc_speed_ratio_min, s.recorded_arc_speed_ratio_max);
            accepts_recorded(*arc, info.start_velocity, origin, info.timestep, window)
        })
    {
        return LaunchBatch {
            requests: vec![QueryRequest {
                trajectory: Trajectory {
                    position: arc.position,
                    velocity: arc.velocity,
                    acceleration: arc.acceleration,
                    duration,
                },
                radius: s.trajectory_radius,
                start_error: s.recorded_arc_error,
                end_error: s.recorded_arc_error,
            }],
            velocities: vec![info.start_velocity],
            origin,
            board_position,
            com_displacement,
            local_board_position: transform(info.reckoning_inverse, sub(info.board_position, origin)),
            local_com_position: transform(info.reckoning_inverse, com_displacement),
            recorded: true,
        };
    }
    let requests = velocities
        .iter()
        .map(|&velocity| QueryRequest {
            trajectory: Trajectory {
                position: madd(velocity, info.timestep, origin),
                velocity,
                acceleration: input.gravity,
                duration,
            },
            radius: s.trajectory_radius,
            start_error: s.trajectory_error_start,
            end_error: s.trajectory_error_end,
        })
        .collect();
    LaunchBatch {
        requests,
        velocities,
        origin,
        board_position,
        com_displacement,
        local_board_position: transform(info.reckoning_inverse, sub(info.board_position, origin)),
        local_com_position: transform(info.reckoning_inverse, com_displacement),
        recorded: false,
    }
}
///sub_82D67B50: the board one step after take-off within the radius of the recorded start, moving
///with the recorded velocity, at a speed ratio inside the window (unchecked below 0.0001 m/s).
pub(super) fn accepts_recorded(
    arc: super::RecordedArc,
    velocity: Vector,
    origin: Vector,
    timestep: f32,
    (radius_squared, ratio_min, ratio_max): (f32, f32, f32),
) -> bool {
    let next = madd(velocity, timestep, origin);
    let d = sub(next, arc.position);
    if dot(d, d) >= radius_squared || dot(velocity, arc.velocity) < 0.0 {
        return false;
    }
    let speed = length(velocity);
    if speed <= f32::from_bits(0x38d1_b717) {
        return true;
    }
    let ratio = length(arc.velocity) / speed;
    (ratio_min..=ratio_max).contains(&ratio)
}

#[cfg(test)]
mod recorded_tests {
    use super::*;

    #[test]
    fn a_recorded_arc_is_accepted_only_near_its_start_and_speed() {
        let window = (4.0, 0.333, 3.0);
        let arc = super::super::RecordedArc { position: [0.0, 0.0, 1.0, 0.0], velocity: [0.0, 3.0, 6.0, 0.0], acceleration: [0.0, -9.8, 0.0, 0.0] };
        let dt = 1.0 / 60.0;
        let v = [0.0, 3.0, 6.0, 0.0];
        assert!(accepts_recorded(arc, v, [0.0, 0.0, 0.0, 1.0], dt, window));
        // More than 2 m from the recorded start one step later.
        assert!(!accepts_recorded(arc, v, [0.0, 0.0, -1.5, 1.0], dt, window));
        // Moving against the recorded velocity.
        assert!(!accepts_recorded(arc, [0.0, -3.0, -6.0, 0.0], [0.0, 0.0, 0.0, 1.0], dt, window));
        // Recorded speed more than 3x / less than 1/3 of the take-off speed.
        assert!(!accepts_recorded(arc, [0.0, 0.5, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0], dt, window));
        assert!(!accepts_recorded(arc, [0.0, 12.0, 24.0, 0.0], [0.0, 0.0, 0.0, 1.0], dt, window));
        // Standing still: the ratio is not checked.
        assert!(accepts_recorded(arc, [0.0; 4], [0.0, 0.0, 0.0, 1.0], dt, window));
    }
}
///82D68698 deliberately uses the original -19.6 and -1/9.8 coefficients.
fn max_time(velocity: Vector, s: &SelectorSettings) -> f32 {
    let square = velocity[1].madd(
        velocity[1],
        -(s.trajectory_max_drop * f32::from_bits(0xc19c_cccd)),
    );
    let root = if square == 0.0 {
        0.0
    } else {
        square * inverse_length(square)
    };
    ((-velocity[1] - root) * f32::from_bits(0xbdd0_fac6)).min(s.trajectory_max_time)
}

///tTrajectory::AdjustTrajectory82D609E0 changes velocity only.
pub(super) fn adjust_trajectory(
    trajectory: &mut Trajectory,
    frame: i32,
    adjustment: Vector,
    maximum: f32,
) {
    if frame <= 0 {
        return;
    }
    let correction = scale(adjustment, reciprocal(frame as f32 * STEP));
    let scalar = if dot(correction, correction) > maximum * maximum {
        maximum / length(correction)
    } else {
        1.0
    };
    trajectory.velocity = madd(correction, scalar, trajectory.velocity);
}
