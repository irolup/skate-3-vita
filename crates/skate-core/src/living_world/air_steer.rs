//! Retail's analog steer of an AI skater in the air (research `.local/research/npc/b123-ai-air-steer.md`):
//! `sub_8246ACF0`'s airborne arm `sub_8246B358` turns the character toward the recorded skater forward at the near
//! look-ahead (`pc+208`, turned 180 degrees when the takeoff latch `pc+927` is set), and `sub_82471188` writes the
//! error as stick input to `Turn`, `BodySpin` and `KickTurn` outside a dead zone. The latch (`sub_8246A700`) is set
//! on the takeoff edge when the character faces against the current node's recorded skater forward.
//!
//! Pure and deterministic. Retail tunables are the defaults (data, a mod may change them).

use super::replay::{decode_orientation, retail, rotate, slerp, ReplayLine};
use super::Vec3;

/// `sub_82471188` values (tunables `DD8843F793462295` 2.0 deg and `281F55D7BB965ADC` 10.0 deg).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AirSteerTuning {
    pub dead_zone_deg: f32,
    pub full_deg: f32,
}

impl Default for AirSteerTuning {
    fn default() -> Self {
        Self { dead_zone_deg: retail::STEER_DEAD_ZONE_DEG, full_deg: retail::STEER_FULL_DEG }
    }
}

/// Controller `+927` (flip) and `+928` (airborne last tick), `sub_8246A700`; both cleared by the reset `sub_82468AC8`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AirLatch {
    pub flip: bool,
    pub airborne: bool,
}

impl AirLatch {
    /// Every path tick: on the takeoff edge `flip = dot(character forward, node skater forward) < 0`. True on takeoff.
    pub fn tick(&mut self, airborne_now: bool, character_forward: Vec3, node_skater_forward: Vec3) -> bool {
        let takeoff = airborne_now && !self.airborne;
        if takeoff {
            self.flip = dot(character_forward, node_skater_forward) < 0.0;
        }
        self.airborne = airborne_now;
        takeoff
    }
}

fn dot(a: Vec3, b: Vec3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// The recorded skater forward of node `i` (`sub_82453B70`, not interpolated).
pub fn node_skater_forward(line: &ReplayLine, i: u32) -> Vec3 {
    let n = &line.nodes[(i as usize).min(line.nodes.len() - 1)];
    rotate(decode_orientation(n.skater), [0.0, 0.0, 1.0])
}

/// `pc+208`'s forward: the recorded skater frames of the two nodes around cursor `c`, slerped by the fraction.
pub fn skater_forward_at(line: &ReplayLine, c: (u32, f32)) -> Vec3 {
    let i = (c.0 as usize).min(line.nodes.len() - 1);
    let j = (i + 1).min(line.nodes.len() - 1);
    let q = slerp(decode_orientation(line.nodes[i].skater), decode_orientation(line.nodes[j].skater), c.1);
    rotate(q, [0.0, 0.0, 1.0])
}

/// `sub_8246B358`'s yaw error: from the character forward to the target forward (negated when `flip`) about the
/// character up.
pub fn error(character_forward: Vec3, character_up: Vec3, target_forward: Vec3, flip: bool) -> f32 {
    let target = if flip { target_forward.map(|x| -x) } else { target_forward };
    super::ground_steer::yaw(character_forward, target, character_up)
}

/// `sub_82471188`: nothing inside the dead zone (no write), else the stick rising to full at `full_deg`, opposite to
/// the error's sign.
pub fn stick(e: f32, t: &AirSteerTuning) -> Option<f32> {
    let dz = t.dead_zone_deg.to_radians();
    let x = e.abs() - dz;
    if x <= 0.0 {
        return None;
    }
    let span = t.full_deg.to_radians() - dz;
    let m = if x < span { x / span } else { 1.0 };
    Some(if e > 0.0 { -m } else { m })
}

#[cfg(test)]
mod tests {
    use super::*;

    const UP: Vec3 = [0.0, 1.0, 0.0];

    #[test]
    fn stick_has_a_dead_zone_and_matches_the_replay_steer_outside_it() {
        let t = AirSteerTuning::default();
        assert_eq!(stick(1f32.to_radians(), &t), None);
        assert_eq!(stick(2f32.to_radians(), &t), None);
        assert_eq!(stick(0.0, &t), None);
        assert!((stick(6f32.to_radians(), &t).unwrap() + 0.5).abs() < 1e-4);
        assert!((stick(-6f32.to_radians(), &t).unwrap() - 0.5).abs() < 1e-4);
        assert_eq!(stick(40f32.to_radians(), &t), Some(-1.0));
        assert_eq!(stick(12f32.to_radians(), &AirSteerTuning { dead_zone_deg: 10.0, full_deg: 5.0 }), Some(-1.0));
        for k in -30..=30 {
            let e = k as f32 * 0.1;
            if let Some(s) = stick(e, &t) {
                assert!((s - super::super::replay::steer_input(e, t.dead_zone_deg, t.full_deg)).abs() < 1e-5);
            }
        }
    }

    #[test]
    fn the_error_turns_toward_the_target_or_its_reverse_when_latched() {
        let left = [30f32.to_radians().sin(), 0.0, 30f32.to_radians().cos()];
        assert!(error([0.0, 0.0, 1.0], UP, [0.0, 0.0, 1.0], false).abs() < 1e-6);
        assert!((error([0.0, 0.0, 1.0], UP, left, false) - 30f32.to_radians()).abs() < 1e-4);
        // Latched: the reverse of a backwards target is straight ahead.
        assert!(error([0.0, 0.0, 1.0], UP, [0.0, 0.0, -1.0], true).abs() < 1e-6);
    }

    #[test]
    fn the_flip_latches_only_on_takeoff() {
        let mut l = AirLatch::default();
        assert!(!l.tick(false, [0.0, 0.0, 1.0], [0.0, 0.0, -1.0]));
        assert!(!l.flip);
        assert!(l.tick(true, [0.0, 0.0, 1.0], [0.0, 0.0, -1.0]));
        assert!(l.flip, "facing against the recorded skater at takeoff");
        // Held in the air and on landing.
        assert!(!l.tick(true, [0.0, 0.0, 1.0], [0.0, 0.0, 1.0]));
        assert!(!l.tick(false, [0.0, 0.0, 1.0], [0.0, 0.0, 1.0]));
        assert!(l.flip);
        // The next takeoff latches again.
        assert!(l.tick(true, [0.0, 0.0, 1.0], [0.0, 0.0, 1.0]));
        assert!(!l.flip);
    }
}
