//! The skitch frame step (`sub_82D48148`, state 104). Retail (TU3, evidence only; re-implemented;
//! `.local/research/npc/b32-skitch-frame.md` section 1, `b47-skitch-frame-transforms.md` section 1; main checked the
//! constants in b32 and the query side of b47):
//! - re-orient: when the board points against the spline direction the endpoints swap and the direction flips;
//! - frame history 64 <- 128 <- 192 <- new (row0 spline direction, row1 up, row2 `normalize(d x n)`, row3 the
//!   endpoints' midpoint; all three = new on the first frame after a reset);
//! - the skater's along-axis coordinate `836` in the previous frame (128) and its rate `840`; the axis point
//!   `A = prev.row3 + 836 prev.row0` (b47 corrects b32: the previous frame directly);
//! - the grab point (`82D2D550`, current world pose) mapped by `320 = inv(current) * previous` into the previous
//!   pose, `672 = g' - A` (zero outside the half range `928`);
//! - the car velocity at the grab location `752` (finite difference of frames 128 and 64), the horizontal
//!   direction and distance to the axis point (`656`, `852`, rate `856`), the signed sideways offset `848` / `688`,
//!   the tow speed `832`, the side target (`624`, `860`, origin pushed 0.5 m along row2) and the "tows fast"
//!   gate (state+1345 bit 0x40, `832 >= 1F85F500908C5E17`).
//! Frames are rigid, row-vector style: `p = row3 + x row0 + y row1 + z row2`. The relative transforms 256 / 448
//! (car motion over the last frame) are kept for the hand targets (`82D4A378`, not ported); 384 is identity in
//! retail and left out.

use super::super::super::living_world::Vec3;

/// A rigid frame: rows 0..2 the axes, row 3 the origin.
pub type Frame = [Vec3; 4];

fn add(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn sub(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
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
fn length(a: Vec3) -> f32 {
    dot(a, a).sqrt()
}
/// Zero-safe (ours: a zero vector stays zero; retail's epsilon vector at `0x830BD350` is not read).
fn normalize(a: Vec3) -> Vec3 {
    let l = length(a);
    if l > 1e-12 { scale(a, 1.0 / l) } else { [0.0; 3] }
}

/// `p = row3 + x row0 + y row1 + z row2`.
pub fn to_world(f: &Frame, local: Vec3) -> Vec3 {
    add(f[3], add(scale(f[0], local[0]), add(scale(f[1], local[1]), scale(f[2], local[2]))))
}

/// The inverse of [`to_world`] (the transpose of the rotation, as retail builds it).
pub fn to_local(f: &Frame, world: Vec3) -> Vec3 {
    let d = sub(world, f[3]);
    [dot(d, f[0]), dot(d, f[1]), dot(d, f[2])]
}

/// What the frame step reads of the grab record and the skater each update.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrameInput {
    /// The spline endpoints (record +64 / +80, state+1088 / +1104) and direction (state+1136).
    pub endpoints: [Vec3; 2],
    pub direction: Vec3,
    /// The record's half range (state+1204, `928`).
    pub half_range: f32,
    /// Up `n` (state+800) and the skater position `P` (state+592).
    pub up: Vec3,
    pub position: Vec3,
    /// The board's side axis (`[state+16]+128`, row 0 of the effective board transform written by 82C013F0; b68:
    /// not the forward), for the re-orient.
    pub board_side: Vec3,
    /// The shimmy velocity along the bumper (`892`, the along chain's output of the last update).
    pub shimmy_velocity: f32,
}

/// The frame step's persistent part (reset by `82D47318`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrameState {
    /// Frames 64, 128, 192.
    pub frames: [Frame; 3],
    pub along: f32,
    pub axis_distance: f32,
    /// state+1345 bit 0x80: the first frame after a reset.
    pub first: bool,
}

impl Default for FrameState {
    fn default() -> Self {
        let identity = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [0.0; 3]];
        Self { frames: [identity; 3], along: 0.0, axis_distance: 0.0, first: true }
    }
}

/// The frame step's outputs (state offsets in the names' docs).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FrameOutput {
    /// The endpoints were swapped this update (state+1224 bit 0x20 toggled).
    pub reoriented: bool,
    /// 932 = 928 + the shimmy margin (`Hash_60272903AD60FF3B`, 0.4).
    pub along_limit: f32,
    /// 836 / 840.
    pub along: f32,
    pub along_rate: f32,
    /// 640 = A - P and 672 = g' - A.
    pub to_axis: Vec3,
    pub grab_from_axis: Vec3,
    /// 844 = 840 - 892.
    pub along_slip: f32,
    /// 752: the car's velocity at the grab location.
    pub car_velocity: Vec3,
    /// 656, 848 (signed), 688.
    pub axis_dir: Vec3,
    pub grab_side_offset: f32,
    pub grab_side_dir: Vec3,
    /// 852 / 856.
    pub axis_distance: f32,
    pub axis_distance_rate: f32,
    /// 832: the tow speed.
    pub tow_speed: f32,
    /// 624 / 860.
    pub side_dir: Vec3,
    pub side_distance: f32,
    /// state+1345 bit 0x40.
    pub tows_fast: bool,
    /// 256 (= 448): the car's motion over the last frame, `inv(previous) * current`.
    pub car_delta: Frame,
}

/// Retail values the frame step reads (`physics_state_skitching/default`; data a mod may override).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrameSettings {
    /// `Hash_60272903AD60FF3B` (0.4): added to the half range.
    pub along_margin: f32,
    /// `Hash_1F85F500908C5E17` (3.0): the tow speed gate.
    pub tow_speed_gate: f32,
    /// The side target's push along row2 (0.5, `0x8209975C`).
    pub side_push: f32,
}

impl Default for FrameSettings {
    fn default() -> Self {
        Self { along_margin: 0.4, tow_speed_gate: 3.0, side_push: 0.5 }
    }
}

/// `inv(a) * b` as a frame: a point given in `a`'s world pose -> the same car-fixed point in `b`'s.
fn relative(a: &Frame, b: &Frame) -> Frame {
    let map = |p: Vec3| to_world(b, to_local(a, p));
    let o = map([0.0; 3]);
    [sub(map([1.0, 0.0, 0.0]), o), sub(map([0.0, 1.0, 0.0]), o), sub(map([0.0, 0.0, 1.0]), o), o]
}

/// `sub_82D48148`. `grab_point(t)` is `82D2D550` (the spline point at along coordinate `t`, current world pose).
pub fn step(input: &FrameInput, state: &mut FrameState, s: &FrameSettings, grab_point: &dyn Fn(f32) -> Vec3) -> FrameOutput {
    let hor = |v: Vec3| sub(v, scale(input.up, dot(v, input.up)));
    // 1. Re-orient.
    let mut endpoints = input.endpoints;
    let mut d = input.direction;
    let reoriented = dot(input.board_side, d) < 0.0;
    if reoriented {
        endpoints.swap(0, 1);
        d = scale(d, -1.0);
    }
    // 2-3. Ranges and the frame history.
    let along_limit = input.half_range + s.along_margin;
    let new: Frame = [d, input.up, normalize(cross(d, input.up)), scale(add(endpoints[0], endpoints[1]), 0.5)];
    let first = state.first;
    state.frames = if first { [new; 3] } else { [state.frames[1], state.frames[2], new] };
    let [older, prev, curr] = state.frames;
    // 4. Along coordinate in the previous frame.
    let along = dot(sub(input.position, prev[3]), prev[0]);
    let along_rate = (along - state.along) * 60.0;
    // 5. Axis point, grab point mapped into the previous pose.
    let axis = add(prev[3], scale(prev[0], along));
    let to_axis = sub(axis, input.position);
    let grab = if along.abs() < input.half_range { to_world(&prev, to_local(&curr, grab_point(along))) } else { axis };
    let grab_from_axis = sub(grab, axis);
    // 6-7. Slip along the bumper, the car's velocity at the grab location.
    let along_slip = along_rate - input.shimmy_velocity;
    let car_velocity = scale(sub(add(prev[3], scale(prev[0], along)), add(older[3], scale(older[0], along))), 60.0);
    // 8-10. Directions and distances.
    let axis_dir = normalize(hor(to_axis));
    let h = hor(grab_from_axis);
    let mut grab_side_offset = length(h);
    let grab_side_dir = if grab_side_offset > 0.0 { scale(h, 1.0 / grab_side_offset) } else { [0.0; 3] };
    if dot(h, prev[2]) < 0.0 {
        grab_side_offset = -grab_side_offset;
    }
    let axis_distance = length(hor(to_axis));
    let axis_distance_rate = if first { 0.0 } else { (axis_distance - state.axis_distance) * 60.0 };
    // 11. Tow speed.
    let tow_speed = length(scale(axis_dir, dot(car_velocity, axis_dir)));
    // 12. Side target.
    let origin = add(prev[3], scale(prev[2], s.side_push));
    let to_side = sub(add(origin, scale(prev[0], along)), input.position);
    let side_dir = normalize(hor(to_side));
    let side_distance = length(hor(to_side));
    state.along = along;
    state.axis_distance = axis_distance;
    state.first = false;
    FrameOutput {
        reoriented,
        along_limit,
        along,
        along_rate,
        to_axis,
        grab_from_axis,
        along_slip,
        car_velocity,
        axis_dir,
        grab_side_offset,
        grab_side_dir,
        axis_distance,
        axis_distance_rate,
        tow_speed,
        side_dir,
        side_distance,
        tows_fast: tow_speed >= s.tow_speed_gate,
        car_delta: relative(&prev, &curr),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(z: f32) -> FrameInput {
        // A car whose rear edge runs along x at z, the skater 1 m behind it.
        FrameInput {
            endpoints: [[-0.8, 0.9, z], [0.8, 0.9, z]],
            direction: [1.0, 0.0, 0.0],
            half_range: 0.8,
            up: [0.0, 1.0, 0.0],
            position: [0.3, 0.0, -1.0],
            board_side: [1.0, 0.0, 0.0],
            shimmy_velocity: 0.0,
        }
    }

    #[test]
    fn a_standing_car_gives_the_axis_point_and_no_tow() {
        let mut st = FrameState::default();
        let s = FrameSettings::default();
        let grab = |t: f32| [t, 0.9, 0.0];
        let o = step(&input(0.0), &mut st, &s, &grab);
        assert!((o.along - 0.3).abs() < 1e-6);
        assert_eq!(o.to_axis, [0.0, 0.9, 1.0]);
        assert_eq!(o.grab_from_axis, [0.0; 3]);
        assert_eq!((o.axis_distance, o.axis_distance_rate, o.tow_speed, o.tows_fast), (1.0, 0.0, 0.0, false));
        assert_eq!(o.axis_dir, [0.0, 0.0, 1.0]);
        // Row2 = d x n = (0, 0, 1): the side target is 0.5 m further along +z.
        assert!((o.side_distance - 1.5).abs() < 1e-6);
        assert_eq!(o.car_delta[3], [0.0; 3]);
    }

    #[test]
    fn a_car_pulling_away_tows_and_the_grab_point_is_compared_in_the_previous_pose() {
        let mut st = FrameState::default();
        let s = FrameSettings::default();
        let grab = |t: f32| [t, 0.9, 0.0];
        step(&input(0.0), &mut st, &s, &grab);
        // The car moves 0.1 m per frame along +z (6 m/s): frames 128 and 192 differ.
        step(&input(0.1), &mut st, &s, &grab);
        let moved = |t: f32| [t, 0.9, 0.2];
        let o = step(&input(0.2), &mut st, &s, &moved);
        // 752 from frames 128 (z 0.1) and 64 (z 0.0): 6 m/s along +z, all of it towards the axis point.
        assert!((o.car_velocity[2] - 6.0).abs() < 1e-4, "{:?}", o.car_velocity);
        assert!((o.tow_speed - 6.0).abs() < 1e-4 && o.tows_fast);
        // The current grab point (z 0.2) mapped by inv(current) * previous lands on the previous edge (z 0.1).
        assert!(length(o.grab_from_axis) < 1e-5, "{:?}", o.grab_from_axis);
        assert!((o.car_delta[3][2] - 0.1).abs() < 1e-6);
        assert!((o.axis_distance_rate - 6.0).abs() < 1e-3, "the axis point moved 0.1 m away in a frame");
    }

    #[test]
    fn a_board_facing_the_other_way_swaps_the_endpoints() {
        let mut st = FrameState::default();
        let mut i = input(0.0);
        i.board_side = [-1.0, 0.0, 0.0];
        let o = step(&i, &mut st, &FrameSettings::default(), &|t: f32| [-t, 0.9, 0.0]);
        assert!(o.reoriented);
        assert_eq!(st.frames[2][0], [-1.0, 0.0, 0.0]);
        assert!((o.along + 0.3).abs() < 1e-6);
    }
}
