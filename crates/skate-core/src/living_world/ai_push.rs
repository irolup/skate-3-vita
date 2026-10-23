//! The AI skater's push: retail's PathController posts the same push intents the player's pad listener does
//! while the skater is slower than its line wants, so an AI skater plays the normal Push animation (doc 26e "AI
//! skaters push when slower than their line", research b341).
//!
//! Retail (TU3, evidence only; re-implemented):
//! - `sub_8246BAA8` (every board tick, from `sub_8246FA30` on the ground, not in the air): the target speed is the
//!   recorded step x 60 (`pc+0x1F0`), floored at tunable `8213474C` (4.0 m/s) when the skater is more than tunable
//!   `A8027415` (2.0 m) off the line, then shaped by the avoider (`sub_82470830`, ours
//!   [`super::avoid::AvoidState::shape_speed`]). At or above it the brake writer runs (`sub_8246B810`, not ported
//!   [`brake`]); below it the push writer `sub_8246B4C8`.
//! - `sub_8246B4C8`: no push in trick categories 4, 7, 10, 12 (`pc+0x340`), stuck stage 4 (`pc+0x238`), or when the
//!   line direction (`pc+0x26C`) points against the board's forward. The factor is 0.97 (0.99 when
//!   `sub_824681D8() == 23`, mode not decoded), or 0.3 when a start-trick node is under 40 recorded frames ahead
//!   (`pc+0x31C`, `pc+0x308`) and the speed is above 2.0 m/s; x 0.3 when an off-board node is under 60 frames ahead
//!   (`pc+0x39B`, `pc+0x354`), x 0.3 on a slope under -0.22 (`sub_8246B9C0`), x 0.3 above 11.0 m/s. No push at or
//!   above factor x target, or with a heading error of tunable `86923C10` (40 degrees) or more. Else
//!   `RightPush` or `LeftPush`, `NewPush` and `Pushing`, all 1.0, every tick the condition holds (no timer).
//! - The foot (research b347, main checked both getters): `RightPush` when the SkaterAnim component's (type
//!   `0x82453410`, "ISkaterAnim") stance-mirror bit (slot +0x1C `82B97140`, flags bit 30) differs from its riding-fakie
//!   bit (slot +0x0C `82B970D8`, bit 29), else `LeftPush`: regular forward pushes right, goofy forward left, fakie
//!   flips it. Ours: the same two bits of `SkaterAnimation::stance()`.
//! - The brake writer `sub_8246B810` (b347), at or above the target, see [`brake`]: `Brake` (or `ManualBrake` in a
//!   manual, category 8) with a ramped magnitude, only far above the target speed.

use super::replay::{node_events, node_flags, ReplayLine};
use super::Vec3;

/// Retail values (data-driven, mod-tunable through `npc_push` in the world tuning).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PushSettings {
    /// AI skaters push at all (ours; retail always does).
    pub enabled: bool,
    /// Base factor of the target speed below which the skater pushes (`0x8208F68C`, 0.97).
    pub factor: f32,
    /// The reduced factor and the multiplier of each reducing condition (`0x820D06C0`, 0.3).
    pub reduce: f32,
    /// A start-trick node this close ahead, in recorded frames, reduces the factor (`pc+0x308` < 40)...
    pub trick_frames: u32,
    /// ...when the skater is faster than this, m/s (`0x82060C50`, 2.0).
    pub trick_min_speed: f32,
    /// An off-board node this close ahead, in recorded frames, reduces the factor (`pc+0x354` < 60).
    pub off_board_frames: u32,
    /// The look-aheads only count within this distance of the line, m (`sub_824690B0` / `sub_824691A0`, 3.0).
    pub look_ahead_distance: f32,
    /// A line slope (rise over run) below this reduces the factor (`0x822572AC`, -0.22).
    pub downhill_slope: f32,
    /// Faster than this reduces the factor, m/s (`0x82257278`, 11.0).
    pub fast_speed: f32,
    /// No push with a heading error this large or larger, radians (tunable `86923C10`, 40 degrees).
    pub max_heading_error: f32,
    /// Off the line by more than this (m, tunable `A8027415`, 2.0) the target is at least `min_target` (m/s,
    /// tunable `8213474C`, 4.0).
    pub off_line_distance: f32,
    pub min_target: f32,
    /// A fixed foot for every push (mod override: `Some(true)` = `RightPush`); `None` = retail's stance rule.
    pub right_foot: Option<bool>,
}

impl Default for PushSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            factor: 0.97,
            reduce: 0.3,
            trick_frames: 40,
            trick_min_speed: 2.0,
            off_board_frames: 60,
            look_ahead_distance: 3.0,
            downhill_slope: -0.22,
            fast_speed: 11.0,
            max_heading_error: 40f32.to_radians(),
            off_line_distance: 2.0,
            min_target: 4.0,
            right_foot: None,
        }
    }
}

/// The trick categories that never push (`sub_8246B4C8`).
pub const NO_PUSH_CATEGORIES: [u8; 4] = [4, 7, 10, 12];
/// The stuck stage that never pushes (`pc+0x238`).
pub const NO_PUSH_STUCK_STAGE: u8 = 4;

/// What the push writer sees this tick.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PushInput {
    /// The skater's speed, m/s (`pc+808`).
    pub speed: f32,
    /// The shaped target speed, m/s ([`target_speed`], then the avoider's shaping).
    pub target: f32,
    /// The trick slot's category (`pc+0x340`).
    pub category: u8,
    /// The stuck tracker's stage (`pc+0x238`).
    pub stuck_stage: u8,
    /// The line direction points against the board's forward.
    pub against_forward: bool,
    /// Angle between the skater's facing and the line, radians (`pc+0x338`).
    pub heading_error: f32,
    /// Recorded frames to the next start-trick / off-board node, when within the look-ahead ([`frames_to`]).
    pub trick_frames: Option<u32>,
    pub off_board_frames: Option<u32>,
    /// The line slope at the cursor ([`slope`]).
    pub slope: f32,
    /// The skater's stance-mirror and riding-fakie bits (`SkaterAnimation::stance()`), which pick the foot.
    pub mirrored: bool,
    pub fakie: bool,
}

/// The recorded step's speed (x 60), floored when the skater is far off the line (`sub_8246BAA8`); the avoider
/// shapes it after this.
pub fn target_speed(step: Vec3, off_line: f32, s: &PushSettings) -> f32 {
    let v = step.map(|x| x * super::ai_record::FRAMES_PER_SECOND);
    let speed = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if off_line > s.off_line_distance { speed.max(s.min_target) } else { speed }
}

/// Rise over horizontal run of the recorded step (`sub_8246B9C0`: 0 when the run is under 0.001). Retail takes the
/// recorded position (`pc+0x1E0`) and a second point; the step (`pc+0x1F0`) as that difference is inferred.
pub fn slope(step: Vec3) -> f32 {
    let run = step[0].hypot(step[2]);
    if run < 0.001 { 0.0 } else { step[1] / run }
}

/// Recorded frames from (`node`, `into`) to the first later node `pred` accepts, if it is under `limit` frames away
/// (the walk of `sub_82456550`, as [`super::ai_signals::upcoming_start`]).
pub fn frames_to(line: &ReplayLine, node: u32, into: u32, limit: u32, pred: impl Fn(&super::replay::ReplayNode) -> bool) -> Option<u32> {
    let mut ahead = -i64::from(into);
    for n in line.nodes.iter().skip(node as usize + 1) {
        ahead += i64::from(n.frames);
        if ahead >= i64::from(limit) {
            return None;
        }
        if pred(n) {
            return Some(ahead.max(0) as u32);
        }
    }
    None
}

/// The next start-trick node's distance in frames (for [`PushInput::trick_frames`]).
pub fn trick_frames(line: &ReplayLine, node: u32, into: u32, s: &PushSettings) -> Option<u32> {
    frames_to(line, node, into, s.trick_frames, |n| n.event == node_events::START_TRICK)
}

/// The next off-board node's distance in frames (for [`PushInput::off_board_frames`]).
pub fn off_board_frames(line: &ReplayLine, node: u32, into: u32, s: &PushSettings) -> Option<u32> {
    frames_to(line, node, into, s.off_board_frames, |n| n.flags & node_flags::OFF_BOARD != 0)
}

/// The push intents of this tick, or `None` (`sub_8246B4C8`). The caller runs it only on the ground, on the board,
/// outside a hold, and below the target (at or above it retail brakes instead).
pub fn push(input: &PushInput, s: &PushSettings) -> Option<[(&'static str, f32); 3]> {
    if !s.enabled || input.speed >= input.target {
        return None;
    }
    if NO_PUSH_CATEGORIES.contains(&input.category) || input.stuck_stage == NO_PUSH_STUCK_STAGE || input.against_forward {
        return None;
    }
    let trick_near = input.trick_frames.is_some_and(|f| f < s.trick_frames) && input.speed > s.trick_min_speed;
    let mut k = if trick_near { s.reduce } else { s.factor };
    if input.off_board_frames.is_some_and(|f| f < s.off_board_frames) {
        k *= s.reduce;
    }
    if input.slope < s.downhill_slope {
        k *= s.reduce;
    }
    if input.speed > s.fast_speed {
        k *= s.reduce;
    }
    if k * input.target <= input.speed || input.heading_error.abs() >= s.max_heading_error {
        return None;
    }
    let right = s.right_foot.unwrap_or(input.mirrored != input.fakie);
    Some([(if right { "RightPush" } else { "LeftPush" }, 1.0), ("NewPush", 1.0), ("Pushing", 1.0)])
}

/// The brake writer's values (`sub_8246B810`, research b347; data-driven, mod-tunable through `npc_push`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BrakeSettings {
    /// AI skaters brake at all (ours; retail always does).
    pub enabled: bool,
    /// Brake when speed x this exceeds the target (`0x820D06C0`, 0.3)...
    pub factor: f32,
    /// ...or x this with an off-board node ahead (`0x822F9418`, 0.12).
    pub off_board_factor: f32,
    /// Uphill steeper than this (`0x820C2070`, 0.22) multiplies the factor by `uphill_factor` (`0x82181B90`, 0.4).
    pub uphill_slope: f32,
    pub uphill_factor: f32,
    /// The magnitude reaches 1 this far (m/s) past the threshold (tunable `7E7024BD`, 0.1).
    pub ramp: f32,
    /// No brake with a heading error this large or larger, radians (tunable `8AC7C833`, 40 degrees).
    pub max_heading_error: f32,
}

impl Default for BrakeSettings {
    fn default() -> Self {
        Self { enabled: true, factor: 0.3, off_board_factor: 0.12, uphill_slope: 0.22, uphill_factor: 0.4, ramp: 0.1, max_heading_error: 40f32.to_radians() }
    }
}

/// The trick category that brakes with `ManualBrake` (a manual).
pub const MANUAL_CATEGORY: u8 = 8;

/// The brake intent of this tick, or `None` (`sub_8246B810`). Runs where the push does not: at or above the target.
/// `on_line` = the line cursor is past its first node (`pc+0x28C` >= 1). Retail brakes only when speed x factor is
/// above the target (about 3.3 x the target on flat ground), so between the target and that it glides.
pub fn brake(input: &PushInput, on_line: bool, s: &BrakeSettings, push: &PushSettings) -> Option<(&'static str, f32)> {
    if !s.enabled || !on_line || input.speed < input.target {
        return None;
    }
    let mut k = if input.off_board_frames.is_some_and(|f| f < push.off_board_frames) { s.off_board_factor } else { s.factor };
    if input.slope > s.uphill_slope {
        k *= s.uphill_factor;
    }
    let over = input.speed * k - input.target;
    if over <= 0.0 || input.heading_error.abs() >= s.max_heading_error {
        return None;
    }
    let magnitude = if s.ramp > 0.0 { (over / s.ramp).min(1.0) } else { 1.0 };
    Some((if input.category == MANUAL_CATEGORY { "ManualBrake" } else { "Brake" }, magnitude))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(speed: f32, target: f32) -> PushInput {
        PushInput { speed, target, category: 0, stuck_stage: 0, against_forward: false, heading_error: 0.0, trick_frames: None, off_board_frames: None, slope: 0.0, mirrored: true, fakie: false }
    }

    #[test]
    fn pushes_below_the_factor_and_glides_near_the_target() {
        let s = PushSettings::default();
        assert_eq!(push(&input(5.0, 8.0), &s), Some([("RightPush", 1.0), ("NewPush", 1.0), ("Pushing", 1.0)]));
        // Within 3 % of the target: glide.
        assert_eq!(push(&input(7.9, 8.0), &s), None);
        // At or above the target: retail's brake branch, no push.
        assert_eq!(push(&input(8.0, 8.0), &s), None);
        assert_eq!(push(&input(5.0, 8.0), &PushSettings { right_foot: Some(false), ..s }).unwrap()[0].0, "LeftPush");
    }

    #[test]
    fn the_foot_follows_stance_and_fakie_like_retail() {
        // b347: RightPush when the stance-mirror bit differs from the riding-fakie bit.
        let s = PushSettings::default();
        let foot = |mirrored, fakie| push(&PushInput { mirrored, fakie, ..input(5.0, 8.0) }, &s).unwrap()[0].0;
        assert_eq!(foot(true, false), "RightPush", "regular, forward");
        assert_eq!(foot(false, false), "LeftPush", "goofy, forward");
        assert_eq!(foot(true, true), "LeftPush", "regular, fakie");
        assert_eq!(foot(false, true), "RightPush", "goofy, fakie");
        assert_eq!(push(&PushInput { mirrored: false, ..input(5.0, 8.0) }, &PushSettings { right_foot: Some(true), ..s }).unwrap()[0].0, "RightPush", "mod override");
    }

    #[test]
    fn brakes_only_far_above_the_target_with_a_ramp() {
        let (b, p) = (BrakeSettings::default(), PushSettings::default());
        // Flat: 0.3 x speed must beat the target, so 20 m/s brakes against 5 and glides against 7.
        assert_eq!(brake(&input(20.0, 7.0), true, &b, &p), None);
        let (key, m) = brake(&input(20.0, 5.95), true, &b, &p).unwrap();
        assert_eq!(key, "Brake");
        assert!((m - 0.5).abs() < 1e-4, "ramp: 0.05 past the threshold over 0.1 = {m}");
        assert_eq!(brake(&input(20.0, 5.0), true, &b, &p).unwrap().1, 1.0);
        // Below the target the push writer runs instead; off the first node, or turned away, no brake.
        assert_eq!(brake(&input(4.0, 5.0), true, &b, &p), None);
        assert_eq!(brake(&input(20.0, 5.0), false, &b, &p), None);
        assert_eq!(brake(&PushInput { heading_error: 41f32.to_radians(), ..input(20.0, 5.0) }, true, &b, &p), None);
        // Uphill (x 0.4) and an off-board node ahead (0.12) raise the threshold; a manual brakes with ManualBrake.
        assert_eq!(brake(&PushInput { slope: 0.3, ..input(20.0, 5.0) }, true, &b, &p), None);
        assert_eq!(brake(&PushInput { off_board_frames: Some(10), ..input(20.0, 5.0) }, true, &b, &p), None);
        assert_eq!(brake(&PushInput { category: MANUAL_CATEGORY, ..input(20.0, 5.0) }, true, &b, &p).unwrap().0, "ManualBrake");
    }

    #[test]
    fn fast_lines_glide_up_to_the_recorded_speed() {
        // b338's University case: 14 m/s against a 19 m/s line. Above 11 m/s the factor is 0.97 x 0.3 = 0.291.
        let s = PushSettings::default();
        assert_eq!(push(&input(14.0, 19.0), &s), None);
        assert!(push(&input(10.0, 19.0), &s).is_some(), "under 11 m/s the full factor applies");
    }

    #[test]
    fn gates_and_reductions_follow_retail() {
        let s = PushSettings::default();
        for category in NO_PUSH_CATEGORIES {
            assert_eq!(push(&PushInput { category, ..input(2.0, 8.0) }, &s), None);
        }
        assert_eq!(push(&PushInput { stuck_stage: 4, ..input(2.0, 8.0) }, &s), None);
        assert_eq!(push(&PushInput { against_forward: true, ..input(2.0, 8.0) }, &s), None);
        assert_eq!(push(&PushInput { heading_error: 41f32.to_radians(), ..input(2.0, 8.0) }, &s), None);
        // A trick 30 frames ahead above 2 m/s: factor 0.3, so 3 m/s against 8 m/s glides.
        assert_eq!(push(&PushInput { trick_frames: Some(30), ..input(3.0, 8.0) }, &s), None);
        assert!(push(&PushInput { trick_frames: Some(30), ..input(1.5, 8.0) }, &s).is_some(), "at or below 2 m/s the trick does not reduce");
        // Downhill and an off-board node each multiply by 0.3.
        assert_eq!(push(&PushInput { slope: -0.3, ..input(3.0, 8.0) }, &s), None);
        assert_eq!(push(&PushInput { off_board_frames: Some(10), ..input(3.0, 8.0) }, &s), None);
        assert!(push(&PushInput { slope: -0.2, ..input(3.0, 8.0) }, &s).is_some());
        assert_eq!(push(&input(2.0, 8.0), &PushSettings { enabled: false, ..s }), None);
    }

    #[test]
    fn target_speed_floors_far_off_the_line_and_slope_is_rise_over_run() {
        let s = PushSettings::default();
        let step = [0.05, 0.0, 0.0]; // 3 m/s
        assert!((target_speed(step, 1.0, &s) - 3.0).abs() < 1e-5);
        assert!((target_speed(step, 2.5, &s) - 4.0).abs() < 1e-5);
        assert!((slope([3.0, -1.0, 4.0]) + 0.2).abs() < 1e-6);
        assert_eq!(slope([0.0, 1.0, 0.0]), 0.0);
    }
}
