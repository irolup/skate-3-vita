//! `PhysState_PhysicsGround` / `SlideGround` board path `82C05EC0` (doc 26, "NPC skater
//! steering"): the external (AI) physics record at Processed `1616` pulls the deck toward the
//! recorded pose every tick.
//!
//! Retail (TU3, evidence only; re-implemented): `UpdatePostPhysics 82D387A8` runs the ground
//! wipeout check `82D8F9E0`, then, while record flag bit 31 ("steer to target", Processed `1776`)
//! is set, `82C05EC0(controller, physics_ai)`:
//! - bit 30: position `82C056B0`: `d = (target - deck) * gain`, the deck translation moves by
//!   `min(|d|, max_step)` along `d` (only when `|d| > 1e-6`), set on part 6 alone (`82D9C8C8`
//!   -> `82BD4318`);
//! - bit 28: velocity `82C05868`: the deck body's linear velocity moves toward the target the
//!   same way (`gain`, `max_change`);
//! - bit 29: facing `82C05988`: the target forward in the ground frame (`controller+292` ->
//!   `+752`) with y = 0 and normalised, its signed angle from +Z about +Y (`8296EC98`) wrapped to
//!   [-pi, pi], times `gain`, clamped to `max_degrees` (deg -> rad by `0x8206D110`); the deck
//!   rotation becomes `D * F^T * Ry(step) * F` about its own position and the whole board is set
//!   (`82C0B2C8`).
//!
//! The gains are the `physics_ai` vault record (`*(*(0x830CFDA4)+272)+4`, class
//! `527C93F55CFC663D`; `default` record values below). The player never has the record: its
//! Processed `1776` stays clear, so this never runs for it.

use crate::madd::Madd as _;
use crate::math::{Basis3, Vector3};
use crate::physics::board_motion_output::inverse_length_squared;
use crate::physics::drive_frames::RetailAffineTransform;
use crate::physics::native_arithmetic::dot3;

/// Record flag bits (Processed `1776`, record `+160`).
pub mod flags {
    pub const STEER: u32 = 1 << 31;
    pub const POSITION: u32 = 1 << 30;
    pub const FACING: u32 = 1 << 29;
    pub const VELOCITY: u32 = 1 << 28;
}

/// `physics_ai` (layout offsets in the names).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PhysicsAiTuning {
    /// +0: largest velocity change per tick (m/s).
    pub velocity_max_change: f32,
    /// +4: velocity gain.
    pub velocity_gain: f32,
    /// +8: largest position step per tick (m).
    pub position_max_step: f32,
    /// +12: position gain.
    pub position_gain: f32,
    /// +16: largest yaw step per tick (degrees).
    pub facing_max_degrees: f32,
    /// +20: yaw gain.
    pub facing_gain: f32,
}

impl PhysicsAiTuning {
    /// [data] `physics_ai/default` (skaterschema / skatercollections; checked against the export by
    /// the data-gated test in `skate-data`).
    pub const DEFAULT_RECORD: Self = Self { velocity_max_change: 0.2, velocity_gain: 0.5, position_max_step: 0.02, position_gain: 1.0, facing_max_degrees: 2.0, facing_gain: 0.5 };
}

impl Default for PhysicsAiTuning {
    fn default() -> Self {
        Self::DEFAULT_RECORD
    }
}

/// `0x830BD350` (initialiser `82F826F8` broadcasts `0x82181A88`).
const EPSILON: f32 = 1e-6;
/// `0x8206D110`: degrees to radians.
const DEGREES: f32 = f32::from_bits(0x3c8e_fa35);

/// The record's vectors as used here.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SteerTarget {
    /// +32: the target frame's forward row.
    pub forward: Vector3,
    /// +48: the target position.
    pub position: Vector3,
    /// +64: the target velocity.
    pub velocity: Vector3,
    /// +160.
    pub flags: u32,
}

impl SteerTarget {
    pub fn from_words(vectors: &[[u32; 4]; 10], flags: u32) -> Self {
        let v = |i: usize| Vector3::new(f32::from_bits(vectors[i][0]), f32::from_bits(vectors[i][1]), f32::from_bits(vectors[i][2]));
        Self { forward: v(2), position: v(3), velocity: v(4), flags }
    }
}

/// What the board path reads and writes on the board.
pub trait BoardPathServices {
    /// Deck part 6 transform (`82585CB0`).
    fn deck_transform(&self) -> RetailAffineTransform;
    /// `82D9C8C8(board, transform, 6)`: part 6 alone.
    fn set_deck_transform(&mut self, transform: RetailAffineTransform);
    /// Deck body linear velocity (`*(part 6 + 76) + 32`).
    fn deck_velocity(&self) -> Vector3;
    fn set_deck_velocity(&mut self, velocity: Vector3);
    /// The ground frame (`controller+292` -> `+752`).
    fn ground_frame(&self) -> Basis3;
    /// Board SetTransform `82C0B2C8` (every part moves rigidly with the deck).
    fn set_board_transform(&mut self, transform: RetailAffineTransform);
}

fn sub(a: Vector3, b: Vector3) -> Vector3 {
    Vector3::new(a.x - b.x, a.y - b.y, a.z - b.z)
}
fn lanes(v: Vector3) -> [f32; 4] {
    [v.x, v.y, v.z, 0.0]
}

/// `82C056B0` / `82C05868`: `current` moved toward `target` by `min(|(target - current) * gain|,
/// max)` (no move when that length is at most 1e-6).
pub fn approach(current: Vector3, target: Vector3, gain: f32, max: f32) -> Vector3 {
    let d = sub(target, current);
    let d = Vector3::new(d.x * gain, d.y * gain, d.z * gain);
    let squared = dot3(lanes(d), lanes(d));
    let inverse = inverse_length_squared(squared, 2);
    let length = if squared == 0.0 { 0.0 } else { squared * inverse };
    if !(length > EPSILON) {
        return current;
    }
    let step = length.min(max);
    Vector3::new((d.x * inverse).madd(step, current.x), (d.y * inverse).madd(step, current.y), (d.z * inverse).madd(step, current.z))
}

/// `82C05988`: the yaw step (radians) toward `forward` in the ground frame `frame`.
pub fn facing_step(frame: &Basis3, forward: Vector3, gain: f32, max_degrees: f32) -> f32 {
    let c = &frame.columns;
    // forward * F^T: its components along the frame's Ri / Up / At, y dropped.
    let along = |r: [f32; 3]| (forward.z).madd(r[2], (forward.y).madd(r[1], forward.x * r[0]));
    let local = Vector3::new(along(c[0]), 0.0, along(c[2]));
    let squared = dot3(lanes(local), lanes(local));
    let inverse = inverse_length_squared(squared, 2);
    let length = if squared == 0.0 { 0.0 } else { squared * inverse };
    let direction = if length > EPSILON { Vector3::new(local.x * inverse, 0.0, local.z * inverse) } else { Vector3::new(0.0, 0.0, 0.0) };
    let angle = crate::riding::collision_response::signed_angle(Vector3::new(0.0, 0.0, 1.0), direction, Vector3::new(0.0, 1.0, 0.0));
    let turns = angle * f32::from_bits(0x3e22_f983);
    let fraction = turns - turns.floor();
    let wrapped = (fraction - if fraction > 0.5 { 1.0 } else { 0.0 }) * f32::from_bits(0x40c9_0fdb);
    let max = max_degrees * DEGREES;
    (wrapped * gain).max(-max).min(max)
}

/// The deck rotated by `step` about the ground frame's up axis (`D * F^T * Ry * F`), its
/// position kept.
pub fn rotate_about_ground_up(deck: RetailAffineTransform, frame: &Basis3, step: f32) -> RetailAffineTransform {
    let (s, c) = crate::trigonometry::sin_cos(step);
    let ry = [[c, 0.0, -s], [0.0, 1.0, 0.0], [s, 0.0, c]];
    let f = &frame.columns;
    let axis = |v: [f32; 3]| {
        // v * F^T
        let l: [f32; 3] = core::array::from_fn(|j| v[2].madd(f[j][2], v[1].madd(f[j][1], v[0] * f[j][0])));
        // * Ry
        let m: [f32; 3] = core::array::from_fn(|k| l[2].madd(ry[2][k], l[1].madd(ry[1][k], l[0] * ry[0][k])));
        // * F
        core::array::from_fn(|k| m[2].madd(f[2][k], m[1].madd(f[1][k], m[0] * f[0][k])))
    };
    let d = &deck.basis.columns;
    RetailAffineTransform { basis: Basis3 { columns: [axis(d[0]), axis(d[1]), axis(d[2])] }, translation: deck.translation }
}

/// `82C05EC0`: the three steps in retail order. Callers check [`flags::STEER`] first.
pub fn update_board_path(target: &SteerTarget, tuning: &PhysicsAiTuning, services: &mut impl BoardPathServices) {
    if target.flags & flags::POSITION != 0 {
        let mut deck = services.deck_transform();
        deck.translation = approach(deck.translation, target.position, tuning.position_gain, tuning.position_max_step);
        services.set_deck_transform(deck);
    }
    if target.flags & flags::VELOCITY != 0 {
        let v = approach(services.deck_velocity(), target.velocity, tuning.velocity_gain, tuning.velocity_max_change);
        services.set_deck_velocity(v);
    }
    if target.flags & flags::FACING != 0 {
        let frame = services.ground_frame();
        let step = facing_step(&frame, target.forward, tuning.facing_gain, tuning.facing_max_degrees);
        let deck = services.deck_transform();
        services.set_board_transform(rotate_about_ground_up(deck, &frame, step));
    }
}

#[cfg(test)]
#[path = "board_path_tests.rs"]
mod tests;
