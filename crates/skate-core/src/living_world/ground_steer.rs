//! Retail's analog steer of an AI skater on the ground (research `.local/research/npc/b121-ai-ground-steer.md`):
//! `sub_8246AA00` turns the skater's heading against its line into the error `pc+824`, and `sub_82471070` writes it
//! as stick input to `Turn`, `BodySpin` and `KickTurn`. It runs on and off the path: off the path (beyond 0.4 m) it
//! aims from the far look-ahead point toward the near one, all near from 3 m, which turns a skater back to its line
//! while the steering record carries no target (`sub_8246DB50`).
//!
//! Pure and deterministic. Retail constants are the tuning defaults (data, a mod may change them).

use super::Vec3;

/// `sub_8246AA00` / `sub_82471070` values (retail defaults; addresses in b121).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GroundSteerTuning {
    /// Full stick at this error, degrees (tunable `FED24A60BD8606F7`, 18).
    pub full_deg: f32,
    /// "Near the lane centre": inside the lane and |lateral ratio| below this (`0x821EE79C`, 0.6).
    pub lane_centre: f32,
    /// The rejoin aim starts beyond this distance from the line (`0x82181B90`, 0.4 m) ...
    pub rejoin_start: f32,
    /// ... and aims fully at the near point from this distance (`0x82063B08`, 3.0 m).
    pub rejoin_full: f32,
    /// Closer than this (and not rejoining or avoiding) the alignment and aim errors are averaged (`0x8231A844`, 1.0 m).
    pub blend_dist: f32,
    /// Look-ahead of the near / far aim points along the line, recorded ticks (`pc+144` A + 2, `pc+352` A + 45).
    pub near_ticks: f32,
    pub far_ticks: f32,
    /// Below this squared length the aim direction is degenerate and the alignment error is used (`0x82195100`;
    /// the image value is not readable in our dump, taken as small).
    pub aim_eps: f32,
}

impl Default for GroundSteerTuning {
    fn default() -> Self {
        Self { full_deg: 18.0, lane_centre: 0.6, rejoin_start: 0.4, rejoin_full: 3.0, blend_dist: 1.0, near_ticks: 2.0, far_ticks: 45.0, aim_eps: 1e-6 }
    }
}

/// What `sub_8246AA00` reads.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GroundSteerInput {
    /// The deck's forward (`[PhysOut+0]+64` row 2) and riding fakie (`[PhysOut+56]+155`: negates it).
    pub forward: Vec3,
    pub fakie: bool,
    /// The skater's up (used for the yaw plane).
    pub up: Vec3,
    /// The line's forward at the near look-ahead (`pc+304`).
    pub path_forward: Vec3,
    /// `pc+920`.
    pub on_path: bool,
    /// The projected point (`sub_82467FC8`) and the near / far aim points on the line.
    pub position: Vec3,
    pub near: Vec3,
    pub far: Vec3,
    /// Distance to cursor A (`pc+828`), inside the lane (`pc+932`), lateral ratio (`pc+868`).
    pub dist: f32,
    pub inside_lane: bool,
    pub lateral: f32,
    /// The obstacle avoider steers (`pc+933`); the pending node trick is category 9 (`pc+832`).
    pub avoiding: bool,
    pub category_9: bool,
}

/// `pc+824` and the two flags `sub_8246AA00` writes.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GroundSteer {
    pub error: f32,
    /// `pc+944`: near the lane centre.
    pub centred: bool,
    /// `pc+945`: rejoining (aim moves toward the near point).
    pub rejoining: bool,
}

fn dot(a: Vec3, b: Vec3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: Vec3, b: Vec3) -> Vec3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn flatten(v: Vec3, up: Vec3) -> Vec3 {
    let uu = dot(up, up);
    if uu <= 0.0 {
        return v;
    }
    let k = dot(v, up) / uu;
    [v[0] - up[0] * k, v[1] - up[1] * k, v[2] - up[2] * k]
}

/// `sub_824536C8`: the signed yaw from `a` to `b` about `up` in (-pi, pi], positive when `cross(a, b) . up > 0`;
/// 0 when either is degenerate (`sub_8296EC98`, |v|^2 <= 1e-4).
pub fn yaw(a: Vec3, b: Vec3, up: Vec3) -> f32 {
    let (a, b) = (flatten(a, up), flatten(b, up));
    let (aa, bb) = (dot(a, a), dot(b, b));
    if aa <= 1e-4 || bb <= 1e-4 {
        return 0.0;
    }
    let c = (dot(a, b) / (aa * bb).sqrt()).clamp(-1.0, 1.0);
    let mut angle = c.acos();
    if dot(cross(a, b), up) < 0.0 {
        angle = std::f32::consts::TAU - angle;
    }
    if angle > std::f32::consts::PI {
        angle -= std::f32::consts::TAU;
    }
    angle
}

/// `sub_8246AA00`.
pub fn heading_error(i: &GroundSteerInput, t: &GroundSteerTuning) -> GroundSteer {
    let cur = if i.fakie { i.forward.map(|x| -x) } else { i.forward };
    let mut target = i.path_forward;
    if i.on_path && dot(target, cur) < 0.0 {
        target = target.map(|x| -x);
    }
    let e0 = yaw(cur, target, i.up);
    let centred = i.inside_lane && i.lateral.abs() < t.lane_centre;
    let rejoining = !i.avoiding && !i.category_9 && !centred && i.dist > t.rejoin_start;
    let aim = if !rejoining {
        i.far
    } else if i.dist > t.rejoin_full {
        i.near
    } else {
        let k = (i.dist - t.rejoin_start) / (t.rejoin_full - t.rejoin_start);
        std::array::from_fn(|n| i.far[n] + (i.near[n] - i.far[n]) * k)
    };
    let d = flatten([aim[0] - i.position[0], aim[1] - i.position[1], aim[2] - i.position[2]], i.up);
    let e1 = if dot(d, d) > t.aim_eps { yaw(cur, d, i.up) } else { e0 };
    let error = if !i.avoiding && i.dist < t.blend_dist && !rejoining { (e0 + e1) * 0.5 } else { e1 };
    GroundSteer { error, centred, rejoining }
}

/// `sub_82471070`: the stick value, full at `full_deg` (no dead zone), opposite to the error's sign; a missing
/// tunable (0) gives full stick.
pub fn stick(error: f32, full_deg: f32) -> f32 {
    let full = full_deg.to_radians();
    let m = if full > 0.0 { (error.abs() / full).min(1.0) } else if error != 0.0 { 1.0 } else { 0.0 };
    if error > 0.0 { -m } else { m }
}

/// The control-map writes, in retail order (`Turn` `0x830BFD74`, `BodySpin` `0x830BE600`, `KickTurn` `0x830BE1E0`).
/// None for a centred stick: our skeleton handler treats a present `KickTurn` as pressed.
pub fn intents(s: f32) -> Vec<(&'static str, f32)> {
    if s == 0.0 {
        return Vec::new();
    }
    vec![("Turn", s), ("BodySpin", s), ("KickTurn", s)]
}

#[cfg(test)]
mod tests {
    use super::*;

    const UP: Vec3 = [0.0, 1.0, 0.0];

    fn input() -> GroundSteerInput {
        GroundSteerInput {
            forward: [0.0, 0.0, 1.0],
            fakie: false,
            up: UP,
            path_forward: [0.0, 0.0, 1.0],
            on_path: true,
            position: [0.0; 3],
            near: [0.0, 0.0, 2.0],
            far: [0.0, 0.0, 20.0],
            dist: 0.2,
            inside_lane: true,
            lateral: 0.1,
            avoiding: false,
            category_9: false,
        }
    }

    #[test]
    fn stick_is_linear_to_full_and_opposite_to_the_error() {
        assert_eq!(stick(0.0, 18.0), 0.0);
        assert!((stick(9f32.to_radians(), 18.0) + 0.5).abs() < 1e-5);
        assert!((stick(-9f32.to_radians(), 18.0) - 0.5).abs() < 1e-5);
        assert_eq!(stick(40f32.to_radians(), 18.0), -1.0);
        assert_eq!(stick(0.01, 0.0), -1.0);
        assert!(intents(0.0).is_empty());
        assert_eq!(intents(0.5).iter().map(|x| x.0).collect::<Vec<_>>(), ["Turn", "BodySpin", "KickTurn"]);
    }

    #[test]
    fn yaw_is_positive_turning_left_about_up() {
        // +Z to +X about +Y: cross(z, x) = +y.
        assert!((yaw([0.0, 0.0, 1.0], [1.0, 0.0, 0.0], UP) - std::f32::consts::FRAC_PI_2).abs() < 1e-5);
        assert!((yaw([0.0, 0.0, 1.0], [-1.0, 0.0, 0.0], UP) + std::f32::consts::FRAC_PI_2).abs() < 1e-5);
        assert_eq!(yaw([0.0; 3], [1.0, 0.0, 0.0], UP), 0.0);
    }

    #[test]
    fn aligned_on_the_line_is_zero_and_a_far_aim_to_the_side_is_blended() {
        let t = GroundSteerTuning::default();
        assert_eq!(heading_error(&input(), &t).error, 0.0);
        let side = GroundSteerInput { far: [20f32 * 10f32.to_radians().sin(), 0.0, 20f32 * 10f32.to_radians().cos()], ..input() };
        let e = heading_error(&side, &t).error;
        assert!((e - 5f32.to_radians()).abs() < 1e-4, "(0 + 10 deg) / 2: {e}");
    }

    #[test]
    fn on_the_path_a_reversed_skater_rides_it_backwards_off_the_path_it_turns_round() {
        let t = GroundSteerTuning::default();
        let back = GroundSteerInput { forward: [0.0, 0.0, -1.0], far: [0.0, 0.0, -20.0], ..input() };
        assert!(heading_error(&back, &t).error.abs() < 1e-5);
        let off = GroundSteerInput { on_path: false, dist: 0.3, far: [0.0, 0.0, 20.0], ..back };
        assert!(heading_error(&off, &t).error.abs() > 3.0);
        let fakie = GroundSteerInput { forward: [0.0, 0.0, -1.0], fakie: true, ..input() };
        assert!(heading_error(&fakie, &t).error.abs() < 1e-5, "riding fakie negates the forward");
    }

    #[test]
    fn off_the_line_the_aim_moves_from_the_far_to_the_near_point() {
        let t = GroundSteerTuning::default();
        // The skater 2 m to the right (+X), line along +Z at x = 0: near (0, 0, 2), far (0, 0, 20).
        let at = |dist: f32| GroundSteerInput { position: [dist, 0.0, 0.0], dist, inside_lane: false, on_path: false, ..input() };
        let toward = |p: Vec3, d: f32| yaw([0.0, 0.0, 1.0], [p[0] - d, 0.0, p[2]], UP);
        let r = heading_error(&at(0.3), &t);
        assert!(!r.rejoining);
        let r = heading_error(&at(1.7), &t);
        assert!(r.rejoining);
        assert!((r.error - toward([0.0, 0.0, 11.0], 1.7)).abs() < 1e-4, "halfway between far and near");
        let r = heading_error(&at(5.0), &t);
        assert!((r.error - toward([0.0, 0.0, 2.0], 5.0)).abs() < 1e-4, "all near");
        // Facing +Z with +Y up, +X is to the left: back to x = 0 is a right turn (negative yaw, positive stick).
        assert!(r.error < 0.0 && stick(r.error, 18.0) > 0.0);
        // Not while avoiding, for a category 9 trick, or near the lane centre.
        assert!(!heading_error(&GroundSteerInput { avoiding: true, ..at(1.7) }, &t).rejoining);
        assert!(!heading_error(&GroundSteerInput { category_9: true, ..at(1.7) }, &t).rejoining);
        assert!(!heading_error(&GroundSteerInput { inside_lane: true, lateral: 0.5, ..at(1.7) }, &t).rejoining);
        // A degenerate aim falls back to the alignment error.
        let on_aim = GroundSteerInput { far: [0.0; 3], near: [0.0; 3], dist: 0.1, ..input() };
        assert_eq!(heading_error(&on_aim, &t).error, 0.0);
    }
}
