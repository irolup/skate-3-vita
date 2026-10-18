//! A traffic car slowing for a skater coming up BEHIND it, so they can catch it (and skitch). Retail (TU3, evidence
//! only; re-implemented; `.local/research/npc/b63-traffic-skater-scan.md`, main checked the scan's constants, the
//! quad B builder and the planner's vault hashes):
//! - the per-car tick (`sub_82C3D918`) runs the skater scan `sub_82C414A8` after the look-ahead: nothing while a
//!   skater holds the car (`+4402` bit 0x02); else every actor of the skater list (index order) that is not
//!   skitching (state 104), whose distance `|p - car| - 0.5 x size` is under the range (`Hash_33466832D8178EAF`,
//!   40 m) and that lies in the rear zone (quad B, `+3520`) writes the distance (`+3760`) and its speed (`+3756`);
//!   one facing the car's way also sets `+4403` bit 0x10 (and bit 0x08 from a skater byte). Last match wins; 0x10
//!   is never cleared inside the loop;
//! - the speed planner `sub_82C3FA08`, only while the limiter kind is free: with 0x10 set and both speeds above
//!   `Hash_D20826F15FB15A2E` (20 km/h) the acceleration cap becomes 0, and the car brakes toward the skater's speed
//!   minus `Hash_256A412E350A2659` (20 km/h): `(max(vs - m, 0)^2 - v^2) / (2 D + 0.001)`, D =
//!   `Hash_3AB7FC7CF7A17C81` (20 m) when the release grace is over and the skater is in range (FAR), D =
//!   `Hash_F682D359CDBC4D12` (5 m) when bit 0x08 is set and the skater is within `Hash_D49FC49019181EE5` (20 m)
//!   (NEAR, overwrites FAR, not gated by the grace);
//! - the release edge `sub_82C34B30`, once per traffic step after every car: a car let go this step gets the grace
//!   `Hash_4727CF785EF735C8` (2.5 s), otherwise the grace drops by the step and sits at -1 once negative.

use super::super::Vec3;
use super::obstacles::CarFrame;

/// The retail numbers (vehicle characteristics, code constants). Every field is data a mod may override per car.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SkaterFollowParams {
    /// Scan range (`Hash_33466832D8178EAF`, 40 m), also how far quad B reaches back past the car.
    pub range: f32,
    /// Factor on the car's size subtracted from the distance (`0x8209975C`, 0.5).
    pub size_factor: f32,
    /// Both speeds must exceed this (`Hash_D20826F15FB15A2E`, 20 km/h), m/s.
    pub min_speed: f32,
    /// Target speed below the skater's (`Hash_256A412E350A2659`, 20 km/h), m/s.
    pub margin: f32,
    /// Braking distance of the FAR rule (`Hash_3AB7FC7CF7A17C81`, 20 m).
    pub far_distance: f32,
    /// NEAR rule range and braking distance (`Hash_D49FC49019181EE5` 20 m, `Hash_F682D359CDBC4D12` 5 m).
    pub near_range: f32,
    pub near_distance: f32,
    /// Grace after a skater lets go (`Hash_4727CF785EF735C8`, 2.5 s): no FAR rule meanwhile.
    pub release_grace: f32,
}

impl Default for SkaterFollowParams {
    fn default() -> Self {
        Self { range: 40.0, size_factor: 0.5, min_speed: 20.0 / 3.6, margin: 20.0 / 3.6, far_distance: 20.0, near_range: 20.0, near_distance: 5.0, release_grace: 2.5 }
    }
}

/// One entry of the skater list the scan walks (retail `*(0x83085480)`), in a stable order.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScanActor {
    pub position: Vec3,
    pub velocity: Vec3,
    /// Horizontal facing (unit, x / z).
    pub forward: [f32; 2],
    /// In physical state 104 (skitching): skipped.
    pub skitching: bool,
    /// Bit 0 of the skater byte `[[state+52]+55]` (meaning open, b63): enables the NEAR rule.
    pub near_flag: bool,
}

/// The scan's result on the car (`+4403` bits 0x10 / 0x08, `+3756`, `+3760`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SkaterScan {
    pub same_direction: bool,
    pub near: bool,
    /// m/s.
    pub speed: f32,
    /// m; -1 = no match.
    pub distance: f32,
}

impl SkaterScan {
    pub const NONE: Self = Self { same_direction: false, near: false, speed: 0.0, distance: -1.0 };
}

impl Default for SkaterScan {
    fn default() -> Self {
        Self::NONE
    }
}

fn add(a: [f32; 2], b: [f32; 2], s: f32) -> [f32; 2] {
    [a[0] + b[0] * s, a[1] + b[1] * s]
}

/// Quad B (`sub_82C400A8`, `+3520 / +3536 / +3552 / +3568`): front corners at the front bumper -/+ the base width,
/// rear corners at the back -/+ the speed-widened width, both pushed `range` further back along their side edges.
/// `base_width` / `rear_width` are the half widths (`hw x (1 + E88C)`, `hw x (1 + 02A6 x speed ratio)`).
pub fn rear_zone_quad(car: &CarFrame, base_width: f32, rear_width: f32, range: f32) -> [[f32; 2]; 4] {
    let c = [car.position[0], car.position[2]];
    let front = add(c, car.forward, car.half_length);
    let back = add(c, car.forward, -car.half_length);
    let q0 = add(front, car.side, base_width);
    let q1 = add(front, car.side, -base_width);
    let mut q2 = add(back, car.side, rear_width);
    let mut q3 = add(back, car.side, -rear_width);
    for (q, f) in [(&mut q2, q0), (&mut q3, q1)] {
        let e = [q[0] - f[0], q[1] - f[1]];
        let l = (e[0] * e[0] + e[1] * e[1]).sqrt().max(1e-6);
        *q = add(*q, [e[0] / l, e[1] / l], range);
    }
    [q0, q1, q2, q3]
}

fn cross(o: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0])
}

/// `sub_82C41A90` as retail tests it: three same-side tests (line q2-q0 against q3, q0-q1 against q2, q1-q3
/// against q0); the edge q2-q3 (the far end) is never tested.
pub fn in_quad_three_edges(p: [f32; 2], q: &[[f32; 2]; 4]) -> bool {
    let same = |a: [f32; 2], b: [f32; 2], r: [f32; 2]| cross(a, b, p) * cross(a, b, r) >= 0.0;
    same(q[2], q[0], q[3]) && same(q[0], q[1], q[2]) && same(q[1], q[3], q[0])
}

fn length(v: Vec3) -> f32 {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}

/// `sub_82C414A8`. `size` = the car's full extents (width, height, length; retail `car+36` vt+172, which of the
/// car's vectors it is stays open, b63): the distance uses its first lane, the range test needs every lane.
pub fn scan(car: &CarFrame, size: Vec3, held: bool, zone: &[[f32; 2]; 4], actors: &[ScanActor], p: &SkaterFollowParams) -> SkaterScan {
    let mut out = SkaterScan::NONE;
    if held {
        return out;
    }
    for a in actors.iter().filter(|a| !a.skitching) {
        let d = length([a.position[0] - car.position[0], a.position[1] - car.position[1], a.position[2] - car.position[2]]);
        let lanes = size.map(|s| d - p.size_factor * s);
        if !lanes.iter().all(|l| p.range > *l) || !in_quad_three_edges([a.position[0], a.position[2]], zone) {
            continue;
        }
        out.distance = lanes[0];
        out.speed = length(a.velocity);
        if a.forward[0] * car.forward[0] + a.forward[1] * car.forward[1] > 0.0 {
            out.same_direction = true;
            out.near = a.near_flag;
        }
    }
    out
}

/// The planner's skater branch (`sub_82C3FA08`, limiter kind free only): `Some(acceleration)` with the cap of 0
/// applied when it is active, `None` otherwise. `accel` is the planner's acceleration so far.
pub fn skater_follow(p: &SkaterFollowParams, speed: f32, scan: &SkaterScan, grace: f32, accel: f32) -> Option<f32> {
    if !scan.same_direction || !(speed > p.min_speed && scan.speed > p.min_speed) {
        return None;
    }
    let target = (scan.speed - p.margin).max(0.0);
    let rule = |d: f32| (target * target - speed * speed) / (2.0 * d + 0.001);
    let mut a = accel;
    if grace < 0.0 && scan.distance < p.range {
        a = rule(p.far_distance);
    }
    if scan.near && scan.distance < p.near_range {
        a = rule(p.near_distance);
    }
    Some(a.min(0.0))
}

/// `sub_82C34B30` for one car, once per traffic step after every car: the grace after a release.
pub fn grace_step(grace: f32, held_last: bool, held: bool, dt: f32, p: &SkaterFollowParams) -> f32 {
    if held_last && !held {
        return p.release_grace;
    }
    let g = grace - dt;
    if g < 0.0 { -1.0 } else { g }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn car() -> CarFrame {
        CarFrame { position: [0.0; 3], forward: [0.0, 1.0], side: [1.0, 0.0], half_width: 0.9, half_length: 2.2 }
    }

    fn skater(z: f32, speed: f32, forward: f32) -> ScanActor {
        ScanActor { position: [0.0, 0.0, z], velocity: [0.0, 0.0, speed], forward: [0.0, forward], skitching: false, near_flag: false }
    }

    fn zone() -> [[f32; 2]; 4] {
        rear_zone_quad(&car(), 0.9, 0.9, 40.0)
    }

    const SIZE: Vec3 = [1.8, 1.5, 4.4];

    #[test]
    fn a_skater_behind_the_car_is_found_and_one_ahead_is_not() {
        let p = SkaterFollowParams::default();
        let s = scan(&car(), SIZE, false, &zone(), &[skater(-30.0, 11.0, 1.0)], &p);
        assert!(s.same_direction && (s.distance - (30.0 - 0.9)).abs() < 1e-4 && (s.speed - 11.0).abs() < 1e-4);
        assert_eq!(scan(&car(), SIZE, false, &zone(), &[skater(10.0, 11.0, 1.0)], &p), SkaterScan::NONE);
        // Beyond the range (every lane must be under 40 m).
        assert_eq!(scan(&car(), SIZE, false, &zone(), &[skater(-41.0, 11.0, 1.0)], &p), SkaterScan::NONE);
        // A held car and a skitching skater: nothing.
        assert_eq!(scan(&car(), SIZE, true, &zone(), &[skater(-30.0, 11.0, 1.0)], &p), SkaterScan::NONE);
        let mut sk = skater(-30.0, 11.0, 1.0);
        sk.skitching = true;
        assert_eq!(scan(&car(), SIZE, false, &zone(), &[sk], &p), SkaterScan::NONE);
    }

    #[test]
    fn last_match_wins_and_the_direction_bit_sticks() {
        let p = SkaterFollowParams::default();
        let s = scan(&car(), SIZE, false, &zone(), &[skater(-10.0, 9.0, 1.0), skater(-20.0, 7.0, -1.0)], &p);
        assert!(s.same_direction);
        assert!((s.distance - 19.1).abs() < 1e-4 && (s.speed - 7.0).abs() < 1e-4);
    }

    #[test]
    fn the_far_edge_of_the_rear_zone_is_open() {
        let q = zone();
        assert!(in_quad_three_edges([0.0, -50.0], &q));
        assert!(!in_quad_three_edges([0.0, 5.0], &q));
        assert!(!in_quad_three_edges([3.0, -10.0], &q));
    }

    #[test]
    fn the_car_brakes_toward_the_skater_speed_minus_the_margin() {
        let p = SkaterFollowParams::default();
        let v = 50.0 / 3.6;
        let s = SkaterScan { same_direction: true, near: false, speed: 40.0 / 3.6, distance: 30.0 };
        let far = skater_follow(&p, v, &s, -1.0, 1.0).unwrap();
        let want = ((20.0f32 / 3.6).powi(2) - v * v) / 40.001;
        assert!((far - want).abs() < 1e-4, "{far} {want}");
        // During the grace only the cap of 0 applies.
        assert_eq!(skater_follow(&p, v, &s, 1.0, 1.0), Some(0.0));
        assert_eq!(skater_follow(&p, v, &s, 1.0, -0.5), Some(-0.5));
        // NEAR: 5 m braking distance, even during the grace.
        let near = SkaterScan { near: true, distance: 15.0, ..s };
        let want = ((20.0f32 / 3.6).powi(2) - v * v) / 10.001;
        assert!((skater_follow(&p, v, &near, 1.0, 1.0).unwrap() - want).abs() < 1e-4);
        // Slow skater or car: no branch.
        assert_eq!(skater_follow(&p, v, &SkaterScan { speed: 5.0, ..s }, -1.0, 1.0), None);
        assert_eq!(skater_follow(&p, 5.0, &s, -1.0, 1.0), None);
        // A faster skater: the rule would accelerate, the cap holds it at 0.
        assert_eq!(skater_follow(&p, 6.0, &SkaterScan { speed: 20.0, ..s }, -1.0, 1.0), Some(0.0));
    }

    #[test]
    fn the_release_grace_lasts_150_steps() {
        let p = SkaterFollowParams::default();
        let dt = 1.0 / 60.0;
        let mut g = grace_step(-1.0, true, false, dt, &p);
        assert_eq!(g, 2.5);
        let mut steps = 0;
        while g >= 0.0 {
            g = grace_step(g, false, false, dt, &p);
            steps += 1;
        }
        assert!((149..=151).contains(&steps), "{steps}");
        assert_eq!(g, -1.0);
    }
}
