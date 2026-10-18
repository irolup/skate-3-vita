//! What a traffic car sees ahead (V4, doc 26h "Traffic: obstacles ahead"). Retail (TU3, evidence only;
//! re-implemented; `.local/research/npc/b36-traffic-v4-driver.md`, `b41-v4-obstacles-corridor.md`, main checked the
//! characteristics / driver values and the 0.68 s constant):
//! - the traffic manager refills two obstacle lists every frame (`sub_826B2EE0`): skaters (radius 0, "soft") and
//!   world actors (peds radius 0.5 x their size, soft; cars radius 0.5 x their smallest extent, hard);
//! - each car builds a look-ahead quad (`sub_82C400A8`): from the car centre (half width x (1 + `E88C`, 0.0)) to the
//!   front bumper, widened on the turning side by half width x (1 + `02A6` (2.0) x speed ratio), then both front
//!   corners pushed `speed + standoff` further along their side edges;
//! - a record is in the way when its circle touches the quad's side / front edges or its centre is inside
//!   (`sub_82C41BD0`, `sub_82C41A90`); its free distance is `|pos - car| - radius - 0.68 x speed` (`0x82099764`);
//!   below 20 km/h soft records count only for drivers that opt in (`+4401` bit 0x08, every stock driver does);
//! - the nearest one becomes the planner's obstacle (`sub_82C412D8`): braking `-v^2 / (2 (d - standoff) + 0.001)`
//!   when `standoff < d <= v + standoff`, `-v` inside the standoff (`sub_82C3FA08`).

use super::super::Vec3;

/// One obstacle record (48 bytes in retail: position, radius, flag +32, handle +36).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Obstacle {
    pub position: Vec3,
    pub radius: f32,
    /// Flag 0 in retail: a skater or a ped (honked at; ignored at low speed without the opt-in).
    pub soft: bool,
    /// Stable id for the honk notify (peds); `None` for records without a handle.
    pub id: Option<u64>,
}

/// The car's frame on the ground plane (unit forward / side, centre, half extents).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CarFrame {
    pub position: Vec3,
    pub forward: [f32; 2],
    pub side: [f32; 2],
    pub half_width: f32,
    pub half_length: f32,
}

/// Retail values (vehicle characteristics / driver data, code constants).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LookAheadParams {
    /// `02A68A53BE0C9EF6` (2.0) and `E88C9A50820CC422` (0.0).
    pub turn_widen: f32,
    pub base_widen: f32,
    /// The reaction time subtracted from every gap (`0x82099764`, 0.68 s).
    pub reaction: f32,
    /// Soft records are skipped below this speed unless the driver opts in (20 km/h, driver fields `E939...` /
    /// `6367...`).
    pub soft_min_speed: f32,
}

impl Default for LookAheadParams {
    fn default() -> Self {
        Self { turn_widen: 2.0, base_widen: 0.0, reaction: 0.68, soft_min_speed: 20.0 / 3.6 }
    }
}

/// Which way the car turns (`+4402` bit 0x40 with the sign of `+3748`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Turn {
    Left,
    Right,
}

fn add(a: [f32; 2], b: [f32; 2], s: f32) -> [f32; 2] {
    [a[0] + b[0] * s, a[1] + b[1] * s]
}

/// `sub_82C400A8`: quad A (rear left, rear right, front left, front right) on the ground plane (x, z).
pub fn look_ahead_quad(car: &CarFrame, speed: f32, standoff: f32, speed_ratio: f32, turn: Option<Turn>, p: &LookAheadParams) -> [[f32; 2]; 4] {
    let c = [car.position[0], car.position[2]];
    let w0 = car.half_width * (1.0 + p.base_widen);
    let wt = car.half_width * (1.0 + p.turn_widen * speed_ratio);
    let front = add(c, car.forward, car.half_length);
    let (wl, wr) = match turn {
        Some(Turn::Left) => (wt, w0),
        Some(Turn::Right) => (w0, wt),
        None => (w0, w0),
    };
    let rl = add(c, car.side, -w0);
    let rr = add(c, car.side, w0);
    let mut fl = add(front, car.side, -wl);
    let mut fr = add(front, car.side, wr);
    let reach = speed + standoff;
    for (f, r) in [(&mut fl, rl), (&mut fr, rr)] {
        let e = [f[0] - r[0], f[1] - r[1]];
        let l = (e[0] * e[0] + e[1] * e[1]).sqrt().max(1e-6);
        *f = add(*f, [e[0] / l, e[1] / l], reach);
    }
    [rl, rr, fl, fr]
}

fn cross(o: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0])
}

/// `sub_82C41A90`: point in the convex quad (ground plane).
pub fn in_quad(p: [f32; 2], q: &[[f32; 2]; 4]) -> bool {
    // Walk the outline rl -> fl -> fr -> rr.
    let ring = [q[0], q[2], q[3], q[1]];
    let s: Vec<f32> = (0..4).map(|i| cross(ring[i], ring[(i + 1) % 4], p)).collect();
    s.iter().all(|v| *v >= 0.0) || s.iter().all(|v| *v <= 0.0)
}

fn segment_distance(p: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    let ab = [b[0] - a[0], b[1] - a[1]];
    let l2 = (ab[0] * ab[0] + ab[1] * ab[1]).max(1e-12);
    let t = (((p[0] - a[0]) * ab[0] + (p[1] - a[1]) * ab[1]) / l2).clamp(0.0, 1.0);
    let d = [p[0] - a[0] - ab[0] * t, p[1] - a[1] - ab[1] * t];
    (d[0] * d[0] + d[1] * d[1]).sqrt()
}

/// `sub_82C41BD0` + the `sub_82C41A90` fallback: the record touches the quad's left, front or right edge (the rear
/// edge at the car centre is not tested) or lies inside it.
pub fn in_the_way(o: &Obstacle, q: &[[f32; 2]; 4]) -> bool {
    let p = [o.position[0], o.position[2]];
    if o.radius > 0.0 && [(q[0], q[2]), (q[2], q[3]), (q[3], q[1])].iter().any(|(a, b)| segment_distance(p, *a, *b) <= o.radius) {
        return true;
    }
    in_quad(p, q)
}

/// The nearest record in the way and its free distance (`sub_82C40B70`).
pub fn nearest(car: &CarFrame, quad: &[[f32; 2]; 4], obstacles: &[Obstacle], speed: f32, soft_opt_in: bool, p: &LookAheadParams) -> Option<(usize, f32)> {
    obstacles
        .iter()
        .enumerate()
        .filter(|(_, o)| !(o.soft && speed < p.soft_min_speed && !soft_opt_in))
        .filter(|(_, o)| in_the_way(o, quad))
        .map(|(i, o)| {
            let d = [o.position[0] - car.position[0], o.position[1] - car.position[1], o.position[2] - car.position[2]];
            (i, (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt() - o.radius - p.reaction * speed)
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
}

/// `sub_82C3FA08` for an obstacle at free distance `d`: braking to stop by the standoff; inside it `-speed`.
/// `None` when the obstacle is beyond one second of travel plus the standoff.
pub fn obstacle_accel(speed: f32, d: f32, standoff: f32) -> Option<f32> {
    if d <= standoff {
        Some(-speed)
    } else if d <= speed + standoff {
        Some(-(speed * speed) / (2.0 * (d - standoff) + 0.001))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn car() -> CarFrame {
        CarFrame { position: [0.0; 3], forward: [0.0, 1.0], side: [1.0, 0.0], half_width: 0.9, half_length: 2.2 }
    }

    #[test]
    fn the_look_ahead_reaches_speed_plus_standoff_beyond_the_bumper() {
        let p = LookAheadParams::default();
        let q = look_ahead_quad(&car(), 10.0, 2.0, 0.0, None, &p);
        // Front corners: bumper (2.2) + 12 m along the straight sides.
        assert!((q[2][1] - 14.2).abs() < 1e-4 && (q[3][1] - 14.2).abs() < 1e-4);
        assert!(in_quad([0.0, 10.0], &q) && !in_quad([0.0, 15.0], &q) && !in_quad([2.0, 5.0], &q));
        // A turn to the right widens the right front corner.
        let t = look_ahead_quad(&car(), 10.0, 2.0, 0.5, Some(Turn::Right), &p);
        assert!(t[3][0] > q[3][0]);
    }

    #[test]
    fn the_nearest_obstacle_brakes_the_car() {
        let p = LookAheadParams::default();
        let c = car();
        let q = look_ahead_quad(&c, 10.0, 2.0, 0.0, None, &p);
        let ped = Obstacle { position: [0.0, 0.0, 12.0], radius: 0.3, soft: true, id: Some(4) };
        let side = Obstacle { position: [3.0, 0.0, 6.0], radius: 0.3, soft: true, id: Some(5) };
        let (i, d) = nearest(&c, &q, &[side, ped], 10.0, true, &p).unwrap();
        assert_eq!(i, 1);
        // 12 - 0.3 - 0.68 x 10.
        assert!((d - 4.9).abs() < 1e-4);
        let a = obstacle_accel(10.0, d, 2.0).unwrap();
        assert!((a + 100.0 / (2.0 * 2.9 + 0.001)).abs() < 1e-3);
        assert_eq!(obstacle_accel(10.0, 1.0, 2.0), Some(-10.0));
        assert_eq!(obstacle_accel(10.0, 13.0, 2.0), None);
        // A slow car without the opt-in ignores soft records.
        assert!(nearest(&c, &q, &[ped], 1.0, false, &p).is_none());
    }
}
