//! The skitch hold target (`sub_82D4B8C0` / `82D4BAC0` / `82D4BC58` / `82D4BCF8`) and the release impulses
//! (`82D4B1D0` / `82D4B378`). Retail (TU3, evidence only; re-implemented; `.local/research/npc/b52-skitch-target-impulses.md`
//! sections 1-2; main checked the impulse vault keys):
//! - the hold target point `736 = 576 + 864 * 528.row0` (the side-shifted previous frame at the hand's along
//!   coordinate 864); `784` = the horizontal unit direction skater -> 736; `952` = the yaw from the direction to the
//!   current grab point (836) to 784;
//! - `at` = yaw from the skater's facing to 784, `as` = yaw from the facing to the side direction 624;
//!   `940 = tows fast ? wrap(at - as) : -as`;
//! - the yaw correction `V = up' * sin(clamp(at, +-180 deg/s * dt)) * max(g1(992), g3 * g2(832))` (`up'` = up
//!   flipped to y >= 0), applied by `82C07328` only while towing fast and not releasing;
//! - release impulses (tag 6, one frame): pull in `624 * min(max(-832, 856 - 2.0), 0) * m / dt` or push off
//!   `624 * (856 + 2.5) * m / dt`.
//! The signed angle `8286CD88` is read as the angle a -> b about the axis (internals not decoded [inferred]).

use super::super::super::living_world::Vec3;
use crate::point_graph::PointGraph;

fn sub(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn add(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn scale(a: Vec3, s: f32) -> Vec3 {
    [a[0] * s, a[1] * s, a[2] * s]
}
fn dot(a: Vec3, b: Vec3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: Vec3, b: Vec3) -> Vec3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn normalize(a: Vec3) -> Vec3 {
    let l = dot(a, a).sqrt();
    if l > 1e-12 { scale(a, 1.0 / l) } else { [0.0; 3] }
}

/// Wrap to [-pi, pi] the way retail does (`frac(a / 2pi)`, minus 1 above one half).
pub fn wrap(a: f32) -> f32 {
    let f = (a / std::f32::consts::TAU).rem_euclid(1.0);
    (f - if f > 0.5 { 1.0 } else { 0.0 }) * std::f32::consts::TAU
}

/// `8286CD88`: the signed angle from `a` to `b` about `axis` [inferred: internals not decoded].
pub fn yaw(a: Vec3, b: Vec3, axis: Vec3) -> f32 {
    let ha = sub(a, scale(axis, dot(a, axis)));
    let hb = sub(b, scale(axis, dot(b, axis)));
    dot(axis, cross(ha, hb)).atan2(dot(ha, hb))
}

/// `physics_state_skitching/default` values (mod-overridable).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TargetSettings {
    /// `0B199E27AF7B96A8`: yaw rate cap, degrees per second (180).
    pub yaw_rate_degrees: f32,
    /// `3F79D445A24E12BA`: turn-in over the skitch time 992.
    pub turn_in: PointGraph<8>,
    /// `6FB7A3D992163663` over the tow speed 832 (the stored graph between the other two keys is
    /// `HeadingAdjustVsSpeed`; that it is this key is unverified).
    pub speed_gain: PointGraph<8>,
    /// `1FF78FAE8AD467FC`: the small-angle gain while the shimmy moves by velocity (1004 == 1, |d| < 1 degree).
    pub small_angle: PointGraph<8>,
    /// `58B697CA9FF570E4` (-2.0) and `8524E22B25D7D0D7` (2.5): the release impulses' rate offsets.
    pub pull_in: f32,
    pub push_off: f32,
}

impl Default for TargetSettings {
    fn default() -> Self {
        Self {
            yaw_rate_degrees: 180.0,
            turn_in: PointGraph { x: [0.0, 0.2606, 0.3681, 0.4007, 0.5358, 0.7117, 0.8502, 1.0], y: [0.0, 0.0, 0.7, 0.7, 0.675, 0.44, 0.135, 0.0] },
            speed_gain: PointGraph { x: [0.0, 0.0358, 0.1401, 0.3485, 0.6124, 0.8111, 1.3062, 2.0], y: [0.0, 0.0, 0.2142, 0.6786, 0.8821, 0.9571, 1.0, 1.0] },
            small_angle: PointGraph { x: [0.0, 0.1, 0.2, 0.3, 0.4, 0.5, 0.75, 1.0], y: [0.5, 0.55, 0.6, 0.65, 0.7, 0.8, 0.9, 1.0] },
            pull_in: -2.0,
            push_off: 2.5,
        }
    }
}

/// What the target step reads.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TargetInput {
    /// The side-shifted previous frame (528: row0 = spline direction, origin 576) and up (800).
    pub side_origin: Vec3,
    pub side_axis: Vec3,
    pub up: Vec3,
    /// The skater's position (`[S+16]+176`) and facing (`+160`): rows 3 and 2 of the effective board transform (b68).
    pub position: Vec3,
    pub facing: Vec3,
    /// The hand's along coordinate (864) and the skater's (836).
    pub hand_along: f32,
    pub along: f32,
    /// The side direction 624 (frame step).
    pub side_dir: Vec3,
    /// 1345 bit 0x40, the skitch time 992, the tow speed 832, 1004 (shimmy moves by velocity).
    pub tows_fast: bool,
    pub skitch_time: f32,
    pub tow_speed: f32,
    pub shimmy_by_velocity: bool,
    pub dt: f32,
}

/// The target step's outputs.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TargetOutput {
    pub target_point: Vec3,
    pub target_dir: Vec3,
    pub grab_yaw: f32,
    pub lean_yaw: f32,
    /// The yaw correction (`82C07328`'s vector); the caller applies it only while towing fast and not releasing.
    pub yaw_correction: Vec3,
}

/// `82D4B8C0` (with `82D4BAC0`, `82D4BC58`, `82D4BCF8`).
pub fn step(i: &TargetInput, s: &TargetSettings) -> TargetOutput {
    let hor = |v: Vec3| sub(v, scale(i.up, dot(v, i.up)));
    let target_point = add(i.side_origin, scale(i.side_axis, i.hand_along));
    let target_dir = normalize(hor(sub(target_point, i.position)));
    let grab_dir = normalize(hor(sub(add(i.side_origin, scale(i.side_axis, i.along)), i.position)));
    let grab_yaw = wrap(yaw(grab_dir, target_dir, i.up));
    let at = wrap(yaw(i.facing, target_dir, i.up));
    let as_ = wrap(yaw(i.facing, i.side_dir, i.up));
    let d = wrap(at - as_);
    let lean_yaw = if i.tows_fast { d } else { -as_ };
    let limit = s.yaw_rate_degrees.to_radians() * i.dt;
    let c = at.clamp(-limit, limit);
    let u = if i.up[1] < 0.0 { scale(i.up, -1.0) } else { i.up };
    let g1 = s.turn_in.evaluate(i.skitch_time);
    let g2 = s.speed_gain.evaluate(i.tow_speed);
    let degrees = d.to_degrees();
    let g3 = if i.shimmy_by_velocity && degrees.abs() < 1.0 { s.small_angle.evaluate(degrees) } else { 1.0 };
    TargetOutput { target_point, target_dir, grab_yaw, lean_yaw, yaw_correction: scale(u, c.sin() * g1.max(g3 * g2)) }
}

/// `82D4B1D0`: the pull-in impulse (1344 bit 0x40) as a tag-6 force.
pub fn pull_in_force(side_dir: Vec3, axis_rate: f32, tow_speed: f32, mass: f32, dt: f32, s: &TargetSettings) -> Vec3 {
    let dv = (axis_rate + s.pull_in).max(-tow_speed).min(0.0);
    scale(side_dir, dv * mass / dt)
}

/// `82D4B378`: the push-off impulse (1344 bit 0x20) as a tag-6 force.
pub fn push_off_force(side_dir: Vec3, axis_rate: f32, mass: f32, dt: f32, s: &TargetSettings) -> Vec3 {
    scale(side_dir, (axis_rate + s.push_off) * mass / dt)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input() -> TargetInput {
        TargetInput {
            side_origin: [0.0, 0.0, 0.5],
            side_axis: [1.0, 0.0, 0.0],
            up: [0.0, 1.0, 0.0],
            position: [0.0, 0.0, -1.0],
            facing: [0.0, 0.0, 1.0],
            hand_along: 0.0,
            along: 0.0,
            side_dir: [0.0, 0.0, 1.0],
            tows_fast: true,
            skitch_time: 0.4,
            tow_speed: 5.0,
            shimmy_by_velocity: false,
            dt: 1.0 / 60.0,
        }
    }

    #[test]
    fn facing_the_target_needs_no_yaw() {
        let o = step(&input(), &TargetSettings::default());
        assert_eq!(o.target_point, [0.0, 0.0, 0.5]);
        assert_eq!(o.target_dir, [0.0, 0.0, 1.0]);
        assert_eq!((o.grab_yaw, o.lean_yaw), (0.0, 0.0));
        assert!(o.yaw_correction.iter().all(|v| v.abs() < 1e-6));
    }

    #[test]
    fn a_target_to_the_side_turns_at_most_the_rate_cap() {
        let mut i = input();
        i.hand_along = 1.5; // target at x 1.5: 45 degrees off the facing
        let s = TargetSettings::default();
        let o = step(&i, &s);
        let limit = 180f32.to_radians() / 60.0;
        let expected = limit.sin() * s.turn_in.evaluate(0.4).max(s.speed_gain.evaluate(5.0));
        assert!((o.yaw_correction[1].abs() - expected).abs() < 1e-5, "{:?}", o.yaw_correction);
        assert!((o.lean_yaw.abs() - std::f32::consts::FRAC_PI_4).abs() < 1e-4);
        // Retail's wrap: exactly one half wraps to +pi, past it to the negative side.
        assert!((wrap(3.0 * std::f32::consts::PI) - std::f32::consts::PI).abs() < 1e-5);
        assert!((wrap(-std::f32::consts::FRAC_PI_2) + std::f32::consts::FRAC_PI_2).abs() < 1e-5);
    }

    #[test]
    fn release_impulses_pull_in_never_outward_and_push_off() {
        let s = TargetSettings::default();
        // Rate +1 (moving away): pull-in to 1 - 2 = -1 m/s; mass 70, 60 Hz.
        assert_eq!(pull_in_force([0.0, 0.0, 1.0], 1.0, 5.0, 70.0, 1.0 / 60.0, &s), [0.0, 0.0, -1.0 * 70.0 * 60.0]);
        // Never faster than the tow speed and never outward.
        assert_eq!(pull_in_force([0.0, 0.0, 1.0], -10.0, 3.0, 1.0, 1.0, &s)[2], -3.0);
        assert_eq!(pull_in_force([0.0, 0.0, 1.0], 5.0, 3.0, 1.0, 1.0, &s)[2], 0.0);
        assert_eq!(push_off_force([0.0, 0.0, 1.0], 0.5, 1.0, 1.0, &s)[2], 3.0);
    }
}
