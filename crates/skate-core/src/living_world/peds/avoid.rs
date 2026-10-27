//! A ped's SkaterAvoidance (nav modifier 2, the avoider at `ped+2832`): the closest-approach test on a moving skater
//! (82E2E538 / 82E2E738) and the Jump / Step decision with a direction (82E2E890), and what a seated ped plays for it.
//! Retail rules and values: docs/hails-additions/26f-living-world-peds.md#seated-peds-avoid-an-approaching-skater

use super::super::Vec3;

/// The avoider's values (retail defaults: `AVOIDK` / `AVOIDTEST` trace lines, research b583).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AvoidRules {
    /// Movers farther than this are not tested, m (0x820ED57C = 100 = 10 m squared).
    pub sense_radius: f32,
    /// Slower movers are not tested, m/s (0x8231A844).
    pub min_speed: f32,
    /// The closest approach must come within this many seconds (`+0x44`).
    pub look_ahead: f32,
    /// ... and pass within this of the ped, m (`+0x38`).
    pub avoid_radius: f32,
    /// A Jump instead of a Step: closer than this, m (`+0x3C`) ...
    pub jump_radius: f32,
    /// ... and faster than this, m/s (`+0x4C`).
    pub jump_speed: f32,
    /// Half width of the Forward / Back sectors, rad (`+0x40`, 30 degrees).
    pub sector_half_angle: f32,
    /// How often a ped tests the movers, s (the recomp re-tests about every 1/3 s; ours: the ped's own phase).
    pub period: f32,
}

impl AvoidRules {
    pub const RETAIL: Self = Self {
        sense_radius: 10.0,
        min_speed: 1.0,
        look_ahead: 0.75,
        avoid_radius: 3.0,
        jump_radius: 1.0,
        jump_speed: 5.0,
        sector_half_angle: 0.523_6,
        period: 1.0 / 3.0,
    };
}

impl Default for AvoidRules {
    fn default() -> Self {
        Self::RETAIL
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AvoidKind {
    Jump,
    Step,
}

/// Where the escape point lies from the ped's facing (Left = the ped's own left side).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AvoidDir {
    Forward,
    Left,
    Back,
    Right,
}

impl AvoidKind {
    pub fn name(self) -> &'static str {
        match self {
            Self::Jump => "jump",
            Self::Step => "step",
        }
    }
}

impl AvoidDir {
    pub fn name(self) -> &'static str {
        match self {
            Self::Forward => "forward",
            Self::Left => "left",
            Self::Back => "back",
            Self::Right => "right",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AvoidDecision {
    pub kind: AvoidKind,
    pub dir: AvoidDir,
    /// Seconds to the closest approach and the distance then, m.
    pub t: f32,
    pub miss: f32,
    pub speed: f32,
    /// The escape vector's angle from the facing about up, rad in [0, 2 pi).
    pub angle: f32,
    /// Where the ped would step to (ped + the horizontal escape vector); the caller checks it is on the ground.
    pub escape_point: Vec3,
}

/// The test for one mover: `facing` is the ped's horizontal forward (x, z). None = no avoid.
pub fn decide(ped: Vec3, facing: [f32; 2], mover: Vec3, velocity: Vec3, rules: &AvoidRules) -> Option<AvoidDecision> {
    let d = [mover[0] - ped[0], mover[1] - ped[1], mover[2] - ped[2]];
    let dot = |a: Vec3, b: Vec3| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    if dot(d, d) > rules.sense_radius * rules.sense_radius {
        return None;
    }
    let vv = dot(velocity, velocity);
    if vv < rules.min_speed * rules.min_speed {
        return None;
    }
    // 82E2E738: the straight-line closest approach of the mover to the ped (3D, as the trace's t and miss).
    let t = -dot(d, velocity) / vv;
    if !(t > 0.0 && t < rules.look_ahead) {
        return None;
    }
    let closest = [d[0] + velocity[0] * t, d[1] + velocity[1] * t, d[2] + velocity[2] * t];
    let miss = dot(closest, closest).sqrt();
    if miss >= rules.avoid_radius {
        return None;
    }
    let speed = vv.sqrt();
    let kind = if speed > rules.jump_speed && miss < rules.jump_radius { AvoidKind::Jump } else { AvoidKind::Step };
    // The escape vector (ped minus the closest point) flattened; its angle from the facing about up (8296EC98: acos of
    // the normalised dot, 2 pi minus it when cross(facing, escape) points down).
    let escape = [-closest[0], -closest[2]];
    let len = (escape[0] * escape[0] + escape[1] * escape[1]).sqrt();
    let flen = (facing[0] * facing[0] + facing[1] * facing[1]).sqrt();
    if len <= f32::EPSILON || flen <= f32::EPSILON {
        return None;
    }
    let cos = ((facing[0] * escape[0] + facing[1] * escape[1]) / (len * flen)).clamp(-1.0, 1.0);
    // Up is +y; cross(facing, escape).y for x / z vectors is f.z * e.x - f.x * e.z.
    let up = facing[1] * escape[0] - facing[0] * escape[1];
    let theta = cos.acos();
    let angle = if up < 0.0 { std::f32::consts::TAU - theta } else { theta };
    let w = rules.sector_half_angle;
    let pi = std::f32::consts::PI;
    let dir = if angle < w || angle > std::f32::consts::TAU - w {
        AvoidDir::Forward
    } else if angle < pi - w {
        AvoidDir::Left
    } else if angle < pi + w {
        AvoidDir::Back
    } else {
        AvoidDir::Right
    };
    Some(AvoidDecision { kind, dir, t, miss, speed, angle, escape_point: [ped[0] + escape[0], ped[1], ped[2] + escape[1]] })
}

/// What a seated ped plays for a decision (`motiongraph_avoid.xml` from `Locomotion.Sit.SittingCyc`, b229 1e) [data].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SeatedAvoid {
    pub clip: &'static str,
    pub mirror: bool,
    /// The ped leaves the seat (SitJump Forward / Left / Right); else it returns to the sitting cycle.
    pub leaves_seat: bool,
    /// `PedestrianSpeech` PedestrianAvoidJump on the SitJump transitions.
    pub speech: Option<i32>,
}

pub const SIT_AVOID_SPEECH: i32 = 4;

/// The seated clips, for the anim set's logical names.
pub const SEATED_CLIPS: [&str; 6] = ["SitAvoidFwd", "SitAvoidLeft", "SitAvoidBack", "SitAvoidSmallFront", "SitAvoidSmallLeft", "SitAvoidSmallBack"];

pub fn seated(kind: AvoidKind, dir: AvoidDir) -> SeatedAvoid {
    let leave = |clip, mirror| SeatedAvoid { clip, mirror, leaves_seat: true, speech: Some(SIT_AVOID_SPEECH) };
    let stay = |clip, mirror| SeatedAvoid { clip, mirror, leaves_seat: false, speech: None };
    match (kind, dir) {
        (AvoidKind::Jump, AvoidDir::Forward) => leave("SitAvoidFwd", false),
        (AvoidKind::Jump, AvoidDir::Left) => leave("SitAvoidLeft", false),
        (AvoidKind::Jump, AvoidDir::Right) => leave("SitAvoidLeft", true),
        // ReturnToSit.StillSitting: the back jump keeps the seat (and the graph's speech node is on SitJump only).
        (AvoidKind::Jump, AvoidDir::Back) => stay("SitAvoidBack", false),
        (AvoidKind::Step, AvoidDir::Forward) => stay("SitAvoidSmallFront", false),
        (AvoidKind::Step, AvoidDir::Left) => stay("SitAvoidSmallLeft", false),
        (AvoidKind::Step, AvoidDir::Right) => stay("SitAvoidSmallLeft", true),
        (AvoidKind::Step, AvoidDir::Back) => stay("SitAvoidSmallBack", false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const R: AvoidRules = AvoidRules::RETAIL;

    #[test]
    fn the_traced_step_and_jump() {
        // AVOIDCPA 45654.2 (recomp, research b583): t 0.519, miss 2.081, 8.199 m/s -> Step.
        let ped = [-137.3908, 12.632, 410.2867];
        let d = decide(ped, [0.0, 1.0], [-132.7152, 12.7194, 411.0442], [-6.7124, 0.4632, -4.6862], &R).unwrap();
        assert_eq!(d.kind, AvoidKind::Step);
        assert!((d.t - 0.519).abs() < 0.002 && (d.miss - 2.081).abs() < 0.002, "{d:?}");
        // AVOIDCPA 46584.1: t 0.715, miss 0.425, 7.82 m/s -> Jump (AVOID 46584.1 jump).
        let d = decide([-144.6017, 12.632, 407.5328], [0.0, 1.0], [-138.9936, 12.7173, 407.4622], [-7.8074, 0.4408, -0.0967], &R).unwrap();
        assert_eq!(d.kind, AvoidKind::Jump, "{d:?}");
        assert!((d.t - 0.715).abs() < 0.002 && (d.miss - 0.425).abs() < 0.005, "{d:?}");
    }

    #[test]
    fn gates() {
        let ped = [0.0; 3];
        let at = [5.0, 0.0, 0.5];
        // Coming straight past at 8 m/s: t 0.625 inside the window.
        assert!(decide(ped, [0.0, 1.0], at, [-8.0, 0.0, 0.0], &R).is_some());
        assert!(decide(ped, [0.0, 1.0], at, [-0.9, 0.0, 0.0], &R).is_none(), "too slow");
        assert!(decide(ped, [0.0, 1.0], at, [8.0, 0.0, 0.0], &R).is_none(), "moving away (t < 0)");
        assert!(decide(ped, [0.0, 1.0], at, [-6.0, 0.0, 0.0], &R).is_none(), "t 0.83 > 0.75");
        assert!(decide(ped, [0.0, 1.0], [5.0, 0.0, 3.1], [-8.0, 0.0, 0.0], &R).is_none(), "miss 3.1");
        assert!(decide(ped, [0.0, 1.0], [10.5, 0.0, 0.0], [-20.0, 0.0, 0.0], &R).is_none(), "farther than 10 m");
        // Close but at 4.9 m/s: a Step.
        assert_eq!(decide(ped, [0.0, 1.0], [3.0, 0.0, 0.4], [-4.9, 0.0, 0.0], &R).unwrap().kind, AvoidKind::Step);
    }

    #[test]
    fn directions_follow_the_peds_own_sides() {
        // Facing +z (Bevy right-handed, y up): the ped's left is +x (up x facing). A skater passing on the ped's right
        // (-x side) leaves the escape point on its left.
        let ped = [0.0; 3];
        let facing = [0.0, 1.0];
        let pass = |side: f32, along: [f32; 2]| decide(ped, facing, [side - along[0] * 4.0, 0.0, -along[1] * 4.0], [along[0] * 8.0, 0.0, along[1] * 8.0], &R).unwrap().dir;
        assert_eq!(pass(-0.8, [0.0, 1.0]), AvoidDir::Left);
        assert_eq!(pass(0.8, [0.0, 1.0]), AvoidDir::Right);
        // Passing in front (+z side) leaves the escape point behind it, passing behind leaves it in front.
        let cross = |z: f32| decide(ped, facing, [-4.0, 0.0, z], [8.0, 0.0, 0.0], &R).unwrap().dir;
        assert_eq!(cross(0.8), AvoidDir::Back);
        assert_eq!(cross(-0.8), AvoidDir::Forward);
    }

    #[test]
    fn seated_outcomes() {
        assert_eq!(seated(AvoidKind::Jump, AvoidDir::Forward), SeatedAvoid { clip: "SitAvoidFwd", mirror: false, leaves_seat: true, speech: Some(4) });
        assert_eq!(seated(AvoidKind::Jump, AvoidDir::Right), SeatedAvoid { clip: "SitAvoidLeft", mirror: true, leaves_seat: true, speech: Some(4) });
        assert!(!seated(AvoidKind::Jump, AvoidDir::Back).leaves_seat);
        assert!(SEATED_CLIPS.iter().all(|c| [AvoidKind::Jump, AvoidKind::Step].iter().any(|k| [AvoidDir::Forward, AvoidDir::Left, AvoidDir::Back].iter().any(|d| seated(*k, *d).clip == *c))));
        assert_eq!(seated(AvoidKind::Step, AvoidDir::Right), SeatedAvoid { clip: "SitAvoidSmallLeft", mirror: true, leaves_seat: false, speech: None });
    }
}
