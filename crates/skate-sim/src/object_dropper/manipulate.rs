//! The held item (retail Manipulate: the dropper's +0x1768 modes 1-3, set by the screen's 825CA178 through
//! 8288B4E0). Holding RT rotates it (mode 2, screen mode RotateObject), holding RB moves it along its own axes
//! (mode 3, MoveOnLockedAxis); letting go returns to the plain move (mode 1). LT, the Advanced modifier, makes the
//! rotation and the locked-axis move smooth; without it they go in steps on the stick's `.repeat`.
//!
//! Retail 8288C310: a rotation step is 90 degrees (0x82181C50) about the vertical (stick left / right, action
//! 206 / 207) or the tilt axis (up / down, 208 / 209), whichever the stick pushes further; the smooth rotation turns
//! by the mode's rotate rate times the stick (actions 202-205). A locked-axis step is 0.75 m (0x821814A0), 2.25 m
//! (0x822F911C) once more than two steps followed each other; one axis at a time, the one pushed further.
use glam::{Quat, Vec3};

use super::camera::{Camera, EditorInput, DOWN, LEFT, RIGHT, UP};
use super::tuning::{ModeTuning, Tuning};

/// The held item's shift state (the screen's +0x204 and the dropper's mode).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shift {
    /// Mode 1: the item follows the cursor.
    Move,
    /// Mode 2 (RT held).
    Rotate,
    /// Mode 3 (RB held).
    LockedAxis,
}

impl Shift {
    /// 825CA178: from the plain move, RT or RB starts a shift (RT first when both); a shift lasts while its
    /// button is held.
    pub fn next(self, input: &EditorInput) -> Self {
        match self {
            Shift::Move if input.rotate => Shift::Rotate,
            Shift::Move if input.lock => Shift::LockedAxis,
            Shift::Rotate if !input.rotate => Shift::Move,
            Shift::LockedAxis if !input.lock => Shift::Move,
            s => s,
        }
    }
}

/// An item in the editor's hands: its catalogue item and template, its turn (it stands at the cursor), and where it
/// came from: a new one from the catalogue, or one taken from the world (its scene id and the pose it had).
#[derive(Clone, Debug, PartialEq)]
pub struct Held {
    pub item: String,
    pub template: String,
    pub rotation: Quat,
    pub source: Option<(u32, Vec3, Quat)>,
    /// The item's own centre (its footprint's middle on the ground, scaled, before the turn): the cursor holds the
    /// item there, so it turns and moves about its middle rather than its model's origin (a corner for most pieces).
    pub pivot: Vec3,
    /// Its colour (`Tuning::colours` index).
    pub tint: u8,
    /// The rest of a held group (group select's X, `8288D418`): each kept in the first item's frame, so the group moves
    /// and turns as one.
    pub others: Vec<Member>,
}

/// One more item of a held group, relative to the group's first item.
#[derive(Clone, Debug, PartialEq)]
pub struct Member {
    pub item: String,
    pub template: String,
    /// Where it came from (a world item: its scene id and pose), None for a copy.
    pub source: Option<(u32, Vec3, Quat)>,
    /// Its origin and turn in the first item's frame.
    pub offset: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
    pub tint: u8,
}

const QUARTER: f32 = std::f32::consts::FRAC_PI_2;
const STEP: f32 = 0.75;
const LONG_STEP: f32 = 2.25;

/// The held item's own axis (signed) nearest to `direction` (retail `82887FA8`: the direction in the item's frame,
/// its largest component picks the axis, its sign the side). Turning about the item's own axes keeps every 90 degree
/// step square with the item; a turn about the camera's right left it skewed.
fn nearest_axis(rotation: Quat, direction: Vec3) -> Vec3 {
    let local = rotation.inverse() * direction;
    let (a, side) = (local.abs(), |v: f32| if v < 0.0 { -1.0 } else { 1.0 });
    let axis = if a.x > a.y && a.x > a.z {
        Vec3::X * side(local.x)
    } else if a.y > a.z {
        Vec3::Y * side(local.y)
    } else {
        Vec3::Z * side(local.z)
    };
    rotation * axis
}

/// The tilt direction: the camera's right on the ground.
fn camera_right(camera: &Camera) -> Vec3 {
    let f = camera.ground_forward();
    Vec3::new(-f.z, 0.0, f.x)
}

/// One update of the RT rotation (`8288C310` mode 2): left / right turns about the item's axis nearest the vertical,
/// up / down tilts about its axis nearest the camera's right (`8288BF28` keeps that one at +0x1500). NOT RETAIL YET:
/// the vertical turn's axis is a stack value the decompile loses; the item's axis nearest +y is our reading.
pub fn rotate(held: &mut Held, mode: &ModeTuning, camera: &Camera, input: &EditorInput) {
    let (yaw_axis, tilt) = (nearest_axis(held.rotation, Vec3::Y), nearest_axis(held.rotation, camera_right(camera)));
    let turn = if input.advanced {
        let yaw = (input.left[RIGHT] - input.left[LEFT]) * mode.rotate_rate;
        let pitch = (input.left[UP] - input.left[DOWN]) * mode.rotate_rate;
        if pitch.abs() <= yaw.abs() { Quat::from_axis_angle(yaw_axis, yaw) } else { Quat::from_axis_angle(tilt, pitch) }
    } else {
        let sideways = (input.left[RIGHT] - input.left[LEFT]).abs();
        let upwards = (input.left[UP] - input.left[DOWN]).abs();
        let sign = |positive: bool, negative: bool| f32::from(u8::from(positive)) - f32::from(u8::from(negative));
        if upwards <= sideways {
            Quat::from_axis_angle(yaw_axis, QUARTER * sign(input.step[RIGHT], input.step[LEFT]))
        } else {
            Quat::from_axis_angle(tilt, QUARTER * sign(input.step[UP], input.step[DOWN]))
        }
    };
    held.rotation = (turn * held.rotation).normalize();
}

/// One update of the RB move along the item's own axes; returns the move. `steps` counts the steps in a row
/// (retail +0x17A4). NOT RETAIL YET: retail animates a step over a few updates (the cursor's +0x1C8 block); ours
/// moves at once.
pub fn locked_move(
    held: &Held,
    tuning: &Tuning,
    mode: &ModeTuning,
    camera: &Camera,
    input: &EditorInput,
    speed: f32,
    steps: &mut u32,
) -> Vec3 {
    let (x_axis, z_axis) = (held.rotation * Vec3::X, held.rotation * Vec3::NEG_Z);
    if input.advanced {
        *steps = 0;
        let s = mode.speed_curve.at(camera.zoom_fraction(tuning)) * speed * mode.move_scale;
        let up = f32::from(u8::from(input.elevate[0])) - f32::from(u8::from(input.elevate[1]));
        return x_axis * (input.left[RIGHT] - input.left[LEFT]) * s + z_axis * (input.left[UP] - input.left[DOWN]) * s + Vec3::Y * up * s;
    }
    let size = if *steps > 2 { LONG_STEP } else { STEP };
    let one = |on: bool| if on { size } else { 0.0 };
    let (right, left, forward, back) = (one(input.step[RIGHT]), one(input.step[LEFT]), one(input.step[UP]), one(input.step[DOWN]));
    let (up, down) = (one(input.step_elevate[0]), one(input.step_elevate[1]));
    // One horizontal axis, the one pushed further (ties go to forward / back as retail's compare does).
    let (x, z) = if right.max(left) > forward.max(back) { (right - left, 0.0) } else { (0.0, forward - back) };
    let moved = x_axis * x + z_axis * z + Vec3::Y * (up - down);
    if moved != Vec3::ZERO {
        *steps += 1;
    }
    let pushed = input.left.iter().any(|v| *v != 0.0) || input.elevate.iter().any(|b| *b);
    if !pushed {
        *steps = 0;
    }
    moved
}
