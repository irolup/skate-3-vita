//! The skitch lean (`sub_82D4A0C0`): the skater leans toward the hold target. Retail (TU3, evidence only;
//! re-implemented; `.local/research/npc/b58-skitch-lean-outputs.md` part 1, b51 section 2):
//! - `T = sign(940) * graph(|940| in degrees) in radians`, 0 while Processed 2488 bit 0x00800000 is set;
//! - `944 += clamp((T - 944) * 0.25, -2 deg, +2 deg)` per update;
//! - above 2 degrees the board offset's orientation becomes a rotation of 944 about Y for 15 updates (Skeleton+15696,
//!   +16389 = 1, +16392 = 15; the height channel is not touched).

use crate::point_graph::PointGraph;

/// `physics_state_skitching/default` (mod-overridable).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LeanSettings {
    /// `30FDFCE185CD3D4F`: lean (degrees) over the target yaw (degrees).
    pub curve: PointGraph<8>,
    /// Rate (`0x820C6D98`, 0.25) and the per-update step cap / write threshold (2 degrees).
    pub rate: f32,
    pub step_degrees: f32,
}

impl Default for LeanSettings {
    fn default() -> Self {
        Self {
            curve: PointGraph { x: [0.0, 14.9, 20.9, 25.5, 29.9, 35.7, 43.3, 50.0], y: [0.0, 14.1, 19.9, 24.3, 28.0, 32.8, 39.5, 45.0] },
            rate: 0.25,
            step_degrees: 2.0,
        }
    }
}

/// `82D4A0C0`: advances the lean angle (944) toward the target from the lean yaw (940) and returns the board-offset
/// orientation (rows, row-vector convention) when it is past the threshold.
pub fn step(angle: &mut f32, lean_yaw: f32, suppressed: bool, s: &LeanSettings) -> Option<[[f32; 4]; 4]> {
    let sign = if lean_yaw >= 0.0 { 1.0 } else { -1.0 };
    let target = if suppressed { 0.0 } else { s.curve.evaluate(lean_yaw.abs().to_degrees()).to_radians() * sign };
    let limit = s.step_degrees.to_radians();
    *angle += ((target - *angle) * s.rate).clamp(-limit, limit);
    (angle.abs() > limit).then(|| {
        let (sn, c) = angle.sin_cos();
        [[c, 0.0, -sn, 0.0], [0.0, 1.0, 0.0, 0.0], [sn, 0.0, c, 0.0], [0.0, 0.0, 0.0, 1.0]]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_lean_eases_in_two_degrees_at_most_and_writes_past_the_threshold() {
        let s = LeanSettings::default();
        let mut a = 0.0;
        // 30 degrees of yaw: target about 28 degrees.
        let first = step(&mut a, 30f32.to_radians(), false, &s);
        assert!((a.to_degrees() - 2.0).abs() < 1e-4, "{}", a.to_degrees());
        assert!(first.is_none(), "not past 2 degrees yet");
        for _ in 0..40 {
            step(&mut a, 30f32.to_radians(), false, &s);
        }
        assert!((a.to_degrees() - 28.0).abs() < 0.2, "{}", a.to_degrees());
        let m = step(&mut a, 30f32.to_radians(), false, &s).unwrap();
        assert!((m[0][0] - a.cos()).abs() < 1e-6 && (m[2][0] - a.sin()).abs() < 1e-6);
        // Suppressed: back toward 0.
        let before = a;
        step(&mut a, 30f32.to_radians(), true, &s);
        assert!(a < before);
    }
}
