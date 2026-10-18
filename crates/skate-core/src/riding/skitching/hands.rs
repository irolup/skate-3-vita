//! The skitch hands (`sub_82D4A378`): where each hand grips the grab edge, when a hand lets go, and the hand IK
//! weights. Retail (TU3, evidence only; re-implemented; `.local/research/npc/b59-skitch-hand-targets.md`; main checked
//! the IK write path against ours):
//! - sub-mode 0 clears the hand-off flags (1328 / 1332); a lean yaw (940) outside -60..35 / -35..60 degrees sets them;
//! - 988 follows the world grab height: `clamp(988 + 0.1 ((c M).y - S(908).y), 0, 0.8)`, `c = (0, grab y, grab z)`;
//! - each hand's spline parameter is the posed hand (908) plus its part's skeleton-local x minus the anchor's x,
//!   clamped to the range (x / z mirrored by Processed 2476 bit 2); a shoulder part farther than 0.9 + 0.7 m (through
//!   the skeleton and the car's last-frame motion) from its spline point lets that hand go;
//! - on a flag change the hand's weight target becomes 0 (off) or 1 (on); the weight moves toward it by +0.03 / -0.2;
//! - a hand with weight > 0 gets an IK target: the spline point plus the car's velocity over one tick, clamped to
//!   0.9 m from the shoulder; the bitmask 984 says which hands are on.

use super::super::super::living_world::Vec3;

fn sub(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn add(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn length(a: Vec3) -> f32 {
    (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt()
}

/// `physics_state_skitching/default` values and code constants (mod-overridable).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HandSettings {
    /// Lean-yaw bounds, degrees, for hand A (`E2DC234CA3A588E9` -60, `11B7A22CEE198ED8` 35) and hand B
    /// (`C6C3D74A4A964B3D` -35, `C50525FE8E95D076` 60).
    pub angle_a: [f32; 2],
    pub angle_b: [f32; 2],
    /// Reach (`2C7533BE2CFAD9B0` 0.9) and slack (`EFF730F7546C6422` 0.7), m.
    pub reach: f32,
    pub slack: f32,
    /// The hand anchor offset (`858D610290DE4F96` (0, -0.06, 0.025)).
    pub anchor: Vec3,
    /// 988's gain and cap (0.1, 0.8); the weight rates per update (+0.03, -0.2).
    pub height_gain: f32,
    pub height_max: f32,
    pub weight_up: f32,
    pub weight_down: f32,
}

impl Default for HandSettings {
    fn default() -> Self {
        Self { angle_a: [-60.0, 35.0], angle_b: [-35.0, 60.0], reach: 0.9, slack: 0.7, anchor: [0.0, -0.06, 0.025], height_gain: 0.1, height_max: 0.8, weight_up: 0.03, weight_down: 0.2 }
    }
}

/// The hands' persistent part (state+916..+988, 1328..1340).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HandState {
    /// 1328 / 1332: the hand is off; 1336 / 1340: last frame's flags.
    pub off: [bool; 2],
    pub latched: [bool; 2],
    /// 916 / 920 reach lengths, 968 / 972 IK weights, 976 / 980 their targets.
    pub reach: [f32; 2],
    pub weight: [f32; 2],
    pub weight_target: [f32; 2],
    /// 984 (bit 0 hand A, bit 1 hand B) and 988.
    pub mask: u32,
    pub grip_height: f32,
}

impl Default for HandState {
    /// `82D47318` sets 1328..1340 to 1.
    fn default() -> Self {
        Self { off: [true; 2], latched: [true; 2], reach: [0.0; 2], weight: [0.0; 2], weight_target: [0.0; 2], mask: 0, grip_height: 0.0 }
    }
}

/// What the hand step reads. Skeleton points are skeleton-local; `to_world` is the skeleton's animation-to-world
/// (Skeleton+11920), `car_motion` the car's last-frame motion (state+448).
pub struct HandInput<'a> {
    pub sub_mode: u8,
    /// 940, 908 (posed hand), 928 (half range), the car velocity at the grab (752), Processed 2476 bit 2.
    pub lean_yaw: f32,
    pub posed: f32,
    pub half_range: f32,
    pub car_velocity: Vec3,
    pub mirrored: bool,
    /// World grab y / z (Processed 2816 / 2820).
    pub world_grab: [f32; 2],
    /// Parts 3 / 7 (hands) and 5 / 9 (shoulder side) of the original selected globals, translation rows.
    pub hands: [Vec3; 2],
    pub shoulders: [Vec3; 2],
    pub to_world: &'a dyn Fn(Vec3) -> Vec3,
    pub car_motion: &'a dyn Fn(Vec3) -> Vec3,
    /// The grab spline point at an along coordinate (`82D2D550`, current world pose).
    pub spline: &'a dyn Fn(f32) -> Vec3,
    pub dt: f32,
}

/// One hand's IK request (`82BD9728` hand A / `82BD97D0` hand B).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HandTarget {
    pub position: Vec3,
    pub weight: f32,
}

impl HandState {
    /// `82D4A378`. Returns the IK requests of hand A and hand B.
    pub fn step(&mut self, i: &HandInput, s: &HandSettings) -> [Option<HandTarget>; 2] {
        if i.sub_mode == 0 {
            self.off = [false; 2];
        }
        let a = i.lean_yaw.to_degrees();
        if a > s.angle_a[1] || a < s.angle_a[0] {
            self.off[0] = true;
        }
        if a > s.angle_b[1] || a < s.angle_b[0] {
            self.off[1] = true;
        }
        let c = (i.to_world)([0.0, i.world_grab[0], i.world_grab[1]]);
        self.grip_height = (self.grip_height + s.height_gain * (c[1] - (i.spline)(i.posed)[1])).clamp(0.0, s.height_max);
        let anchor_x = s.anchor[0];
        let mirror = |v: Vec3| if i.mirrored { [-v[0], v[1], -v[2]] } else { v };
        let mut points = [[0.0; 3]; 2];
        for h in 0..2 {
            let hand = mirror(i.hands[h]);
            let t = (i.posed + hand[0] - anchor_x).clamp(-i.half_range, i.half_range);
            points[h] = (i.spline)(t);
            let shoulder = (i.car_motion)((i.to_world)(i.shoulders[h]));
            self.reach[h] = length(sub(shoulder, points[h]));
            if self.reach[h] > s.reach + s.slack {
                self.off[h] = true;
            }
        }
        let mut out = [None; 2];
        self.mask = 0;
        for h in 0..2 {
            if self.off[h] != self.latched[h] {
                self.latched[h] = self.off[h];
                self.weight_target[h] = if self.off[h] { 0.0 } else { 1.0 };
            }
            self.weight[h] = self.weight_target[h].clamp(self.weight[h] - s.weight_down, self.weight[h] + s.weight_up);
            if self.weight[h] > 0.0 {
                self.mask |= 1 << h;
                let target = add(points[h], [i.car_velocity[0] * i.dt, i.car_velocity[1] * i.dt, i.car_velocity[2] * i.dt]);
                let shoulder = (i.to_world)(i.shoulders[h]);
                let d = sub(target, shoulder);
                let l = length(d);
                let position = if l > s.reach { add(shoulder, [d[0] * s.reach / l, d[1] * s.reach / l, d[2] * s.reach / l]) } else { target };
                out[h] = Some(HandTarget { position, weight: self.weight[h] });
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hands_grip_the_edge_and_let_go_when_out_of_reach() {
        let s = HandSettings::default();
        let mut h = HandState::default();
        let id = |p: Vec3| p;
        let edge = |t: f32| [t, 1.0, 0.5];
        let mut input = HandInput {
            sub_mode: 0,
            lean_yaw: 0.0,
            posed: 0.0,
            half_range: 0.8,
            car_velocity: [0.0, 0.0, 6.0],
            mirrored: false,
            world_grab: [1.0, 0.5],
            hands: [[-0.2, 1.0, 0.4], [0.2, 1.0, 0.4]],
            shoulders: [[-0.2, 1.4, 0.0], [0.2, 1.4, 0.0]],
            to_world: &id,
            car_motion: &id,
            spline: &edge,
            dt: 1.0 / 60.0,
        };
        // Sub-mode 0 turns both hands on: the weight rises 0.03 per update.
        let out = h.step(&input, &s);
        assert_eq!(h.off, [false, false]);
        assert!((h.weight[0] - 0.03).abs() < 1e-6 && h.mask == 3);
        let a = out[0].unwrap();
        assert!((a.position[0] + 0.2).abs() < 1e-5 && (a.position[2] - 0.6).abs() < 1e-5, "{a:?}");
        // The skater leans 40 degrees: hand A (bound 35) lets go, hand B (bound 60) holds.
        input.sub_mode = 1;
        input.lean_yaw = 40f32.to_radians();
        h.step(&input, &s);
        assert_eq!(h.off, [true, false]);
        assert_eq!(h.weight[0], 0.0, "down at 0.2 per update");
        // A shoulder 2 m from the edge: hand B lets go too.
        input.lean_yaw = 0.0;
        input.shoulders[1] = [0.2, 1.4, -2.0];
        h.step(&input, &s);
        assert!(h.off[1] && h.reach[1] > 1.6);
    }
}
