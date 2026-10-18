//! The skater touching a ped (doc 26, "Skater hits peds"): knock-down or standing stumble, the hit
//! direction and the reaction animation.
//!
//! Retail [code, TU3; evidence only, `.local/research/peds/skater-ped-contact.md`]: the ped contact
//! callback `sub_82E38FB8`, kind 5 (an `IActor` owner, e.g. the skater):
//! - inputs: the length of the ped body's linear (f29) and angular (f28) velocity after the contact
//!   solve, gated by entity `C04236FB548697D0` / `797AA1D5F828819B` (0 for every stock entity);
//! - kind: knock-down when the ped's animation set allows it (byte 17, field `5B92564B352A9FAA`;
//!   off for the marquee sets) and either speed is above `541FFA2E9D81C947` / `7C5E39ECE5A5572E`
//!   (3.0 / 3.0); otherwise a standing stumble, unless the ped's brain says immune (`+3277` bit 0x10).
//!   The 6.0 thresholds (`081F978F8706D77B` / `5E566360FDC5FDD1`) and the "no reaction" cases are for
//!   a ped touching a ped (the toucher casts to `IPedestrian`, `sub_826C2C70`);
//! - direction: the contact normal (y dropped) against the ped's forward: `a = acos(dot)`, negated when
//!   `cross(forward, normal).y > 0`, in degrees: 1 FromBack (|a| < 35), 2 FromLeft (35..145),
//!   3 FromRight (-145..-35), 0 FromFront (beyond 145) (constants `0x8206D148`, `0x822F9428..30`;
//!   names from the table `0x830218B0`: FromFront, FromBack, FromLeft, FromRight, Knockdown, Standing).
//! - reaction (`MotionGraph_Pedestrian` `motiongraph_collision.xml` / `template/knockdown.xml` [data]):
//!   see [`reaction_steps`]. The ped lies on the ground until the AI's `Recover` intent; the ground time
//!   is animation-set field `AD3C483F0C9DAD67` (1.5 s, security 0.5 s; read by `sub_8269A990` in
//!   `PedestrianColliding`'s update). Where retail starts counting it is not pinned [inferred: on the
//!   ground].

/// Retail reaction kind (`ped+2496`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReactionKind {
    Knockdown,
    Standing,
}

impl ReactionKind {
    pub fn name(self) -> &'static str {
        match self {
            Self::Knockdown => "Knockdown",
            Self::Standing => "Standing",
        }
    }
}

/// Retail hit direction (`ped+2500`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReactionDirection {
    FromFront,
    FromBack,
    FromLeft,
    FromRight,
}

impl ReactionDirection {
    pub fn name(self) -> &'static str {
        match self {
            Self::FromFront => "FromFront",
            Self::FromBack => "FromBack",
            Self::FromLeft => "FromLeft",
            Self::FromRight => "FromRight",
        }
    }
}

/// The animation set's collision values [data] (`livingworld_entity_animation`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CollisionRules {
    /// Knock-down above these body speeds (linear, angular), skater contacts (3.0 / 3.0).
    pub knockdown_speed: [f32; 2],
    /// The same for a ped touching the ped (`081F978F8706D77B` / `5E566360FDC5FDD1`, 6.0 / 6.0).
    pub ped_knockdown_speed: [f32; 2],
    /// Byte 17: the set allows knock-downs (false for the marquee sets).
    pub can_knock_down: bool,
    /// Seconds on the ground before getting up (1.5; security 0.5).
    pub ground_seconds: f32,
}

impl Default for CollisionRules {
    /// The stock `default` set's values (used only when the export has no set).
    fn default() -> Self {
        Self { knockdown_speed: [3.0, 3.0], ped_knockdown_speed: [6.0, 6.0], can_knock_down: true, ground_seconds: 1.5 }
    }
}

/// [code] `sub_82E38FB8` direction buckets: `normal` is the contact normal and `forward` the ped's
/// forward, both in the ground plane (x, z). The normal is taken as the push on the ped (from the
/// skater into the ped) [inferred: Havok's normal convention for the ped's callback is not read].
pub fn reaction_direction(normal: [f32; 2], forward: [f32; 2]) -> ReactionDirection {
    let len = |v: [f32; 2]| (v[0] * v[0] + v[1] * v[1]).sqrt();
    let (ln, lf) = (len(normal), len(forward));
    if ln <= 0.0 || lf <= 0.0 {
        return ReactionDirection::FromFront;
    }
    let (n, f) = ([normal[0] / ln, normal[1] / ln], [forward[0] / lf, forward[1] / lf]);
    let mut a = crate::trigonometry::acos((n[0] * f[0] + n[1] * f[1]).clamp(-1.0, 1.0)).to_degrees();
    // [code] the vpermwi cross product's y lane: cross(forward, normal).y = f.z * n.x - f.x * n.z.
    if f[1] * n[0] - f[0] * n[1] > 0.0 {
        a = -a;
    }
    if a > -35.0 && a < 35.0 {
        ReactionDirection::FromBack
    } else if (-145.0..=-35.0).contains(&a) {
        ReactionDirection::FromRight
    } else if (35.0..=145.0).contains(&a) {
        ReactionDirection::FromLeft
    } else {
        ReactionDirection::FromFront
    }
}

/// [code] `sub_82E38FB8` kind 5, the skater as toucher: `body_speed` = (linear, angular) speed of the
/// ped body after the contact; `gate` = the entity's thresholds (0, 0 stock); `immune` = the brain's
/// `+3277` bit 0x10. `None` = no reaction.
pub fn skater_reaction(body_speed: [f32; 2], gate: [f32; 2], rules: &CollisionRules, immune: bool) -> Option<ReactionKind> {
    if !(body_speed[0] > gate[0] || body_speed[1] > gate[1]) {
        return None;
    }
    if rules.can_knock_down && (body_speed[0] > rules.knockdown_speed[0] || body_speed[1] > rules.knockdown_speed[1]) {
        return Some(ReactionKind::Knockdown);
    }
    (!immune).then_some(ReactionKind::Standing)
}

/// [code] `sub_82E38FB8` kind 5 with a ped as toucher (`sub_826C2C70`): the ped thresholds; under the zombie cheat
/// (`0x83085494` vfn156) a ped's touch never knocks down and gives no stumble either; `forced` = the toucher's chaser
/// component answers vfn92 (`82E3D968`, its `brain+3276` bit 0x02), a knock-down whenever the set allows one.
/// NOT RETAIL YET: what sets that bit (chaser vfn88 `82E3D920`) is not found, so callers pass false.
pub fn ped_reaction(body_speed: [f32; 2], gate: [f32; 2], rules: &CollisionRules, immune: bool, zombie: bool, forced: bool) -> Option<ReactionKind> {
    if !(body_speed[0] > gate[0] || body_speed[1] > gate[1]) {
        return None;
    }
    let fast = body_speed[0] > rules.ped_knockdown_speed[0] || body_speed[1] > rules.ped_knockdown_speed[1];
    if rules.can_knock_down && (forced || (fast && !zombie)) {
        return Some(ReactionKind::Knockdown);
    }
    (!immune && !zombie).then_some(ReactionKind::Standing)
}

/// One animation step of a reaction: the logical (remapped) animation, mirrored or not, the blend in,
/// and whether it cycles until the ped recovers.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReactionStep {
    pub anim: &'static str,
    pub mirror: bool,
    pub blend: f32,
    pub cycle: bool,
}

/// The motion graph's reaction [data]: a standing stumble is one clip (0.3 s blend); a knock-down is
/// the fall (0.4 s, `FromStand`), the ground cycle (0.1 s, until `Recover`) and the get-up (0.1 s).
pub fn reaction_steps(kind: ReactionKind, direction: ReactionDirection) -> Vec<ReactionStep> {
    use ReactionDirection::*;
    let step = |anim, mirror, blend, cycle| ReactionStep { anim, mirror, blend, cycle };
    match kind {
        ReactionKind::Standing => {
            let (anim, mirror) = match direction {
                FromFront => ("CollisionBackStanding", false),
                FromBack => ("CollisionFwdStanding", false),
                FromLeft => ("CollisionLeftStanding", true),
                FromRight => ("CollisionLeftStanding", false),
            };
            vec![step(anim, mirror, 0.3, false)]
        }
        ReactionKind::Knockdown => {
            let (side, mirror) = match direction {
                FromFront => ("Back", false),
                FromBack => ("Fwd", false),
                FromLeft => ("Left", true),
                FromRight => ("Left", false),
            };
            let name = |part: &str| -> &'static str {
                match (side, part) {
                    ("Back", "Fall") => "WipeoutBackFall",
                    ("Back", "Ground") => "WipeoutBackGroundCyc",
                    ("Back", _) => "WipeoutBackGetUp",
                    ("Fwd", "Fall") => "WipeoutFwdFall",
                    ("Fwd", "Ground") => "WipeoutFwdGroundCyc",
                    ("Fwd", _) => "WipeoutFwdGetUp",
                    (_, "Fall") => "WipeoutLeftFall",
                    (_, "Ground") => "WipeoutLeftGroundCyc",
                    _ => "WipeoutLeftGetUp",
                }
            };
            vec![step(name("Fall"), mirror, 0.4, false), step(name("Ground"), mirror, 0.1, true), step(name("GetUp"), mirror, 0.1, false)]
        }
    }
}

/// Every logical animation the reactions name (the ped animation loader resolves these).
pub const REACTION_ANIMS: &[&str] = &[
    "CollisionBackStanding", "CollisionFwdStanding", "CollisionLeftStanding",
    "WipeoutBackFall", "WipeoutBackGroundCyc", "WipeoutBackGetUp",
    "WipeoutFwdFall", "WipeoutFwdGroundCyc", "WipeoutFwdGetUp",
    "WipeoutLeftFall", "WipeoutLeftGroundCyc", "WipeoutLeftGetUp",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hit_direction_buckets() {
        let f = [0.0, 1.0];
        assert_eq!(reaction_direction([0.0, 1.0], f), ReactionDirection::FromBack, "pushed along its forward");
        assert_eq!(reaction_direction([0.0, -1.0], f), ReactionDirection::FromFront);
        let at = |deg: f32| [deg.to_radians().sin(), deg.to_radians().cos()];
        let (l, r) = (reaction_direction(at(90.0), f), reaction_direction(at(-90.0), f));
        assert!(matches!((l, r), (ReactionDirection::FromLeft, ReactionDirection::FromRight) | (ReactionDirection::FromRight, ReactionDirection::FromLeft)) && l != r);
        assert_eq!(reaction_direction(at(30.0), f), ReactionDirection::FromBack);
        assert_eq!(reaction_direction(at(150.0), f), ReactionDirection::FromFront);
        assert_ne!(reaction_direction(at(40.0), f), ReactionDirection::FromBack);
    }

    #[test]
    fn knock_down_above_three_stumble_below_none_when_still() {
        let r = CollisionRules::default();
        assert_eq!(skater_reaction([3.5, 0.0], [0.0, 0.0], &r, false), Some(ReactionKind::Knockdown));
        assert_eq!(skater_reaction([0.0, 3.1], [0.0, 0.0], &r, false), Some(ReactionKind::Knockdown));
        assert_eq!(skater_reaction([3.0, 0.0], [0.0, 0.0], &r, false), Some(ReactionKind::Standing), "not above 3.0");
        assert_eq!(skater_reaction([1.0, 0.0], [0.0, 0.0], &r, true), None, "immune");
        assert_eq!(skater_reaction([0.0, 0.0], [0.0, 0.0], &r, false), None, "the contact did not move it");
        let marquee = CollisionRules { can_knock_down: false, ..r };
        assert_eq!(skater_reaction([9.0, 0.0], [0.0, 0.0], &marquee, false), Some(ReactionKind::Standing));
    }

    #[test]
    fn reaction_steps_follow_the_motion_graph() {
        let s = reaction_steps(ReactionKind::Standing, ReactionDirection::FromLeft);
        assert_eq!((s.len(), s[0].anim, s[0].mirror, s[0].blend), (1, "CollisionLeftStanding", true, 0.3));
        let k = reaction_steps(ReactionKind::Knockdown, ReactionDirection::FromFront);
        assert_eq!(k.iter().map(|s| s.anim).collect::<Vec<_>>(), ["WipeoutBackFall", "WipeoutBackGroundCyc", "WipeoutBackGetUp"]);
        assert!(k[1].cycle && !k[0].cycle && !k[2].cycle);
        assert!(reaction_steps(ReactionKind::Knockdown, ReactionDirection::FromRight).iter().all(|s| !s.mirror && s.anim.starts_with("WipeoutLeft")));
        for kind in [ReactionKind::Knockdown, ReactionKind::Standing] {
            for d in [ReactionDirection::FromFront, ReactionDirection::FromBack, ReactionDirection::FromLeft, ReactionDirection::FromRight] {
                assert!(reaction_steps(kind, d).iter().all(|s| REACTION_ANIMS.contains(&s.anim)));
            }
        }
    }

    #[test]
    fn a_ped_touching_a_ped_uses_the_ped_thresholds_and_zombie_mode_drops_it() {
        let r = CollisionRules::default();
        let gate = [0.0, 0.0];
        assert_eq!(ped_reaction([4.0, 0.0], gate, &r, false, false, false), Some(ReactionKind::Standing), "below 6.0");
        assert_eq!(skater_reaction([4.0, 0.0], gate, &r, false), Some(ReactionKind::Knockdown), "a skater at 4 m/s knocks down");
        assert_eq!(ped_reaction([6.5, 0.0], gate, &r, false, false, false), Some(ReactionKind::Knockdown));
        assert_eq!(ped_reaction([6.5, 0.0], gate, &r, false, true, false), None, "zombie mode");
        assert_eq!(ped_reaction([1.0, 0.0], gate, &r, false, true, true), Some(ReactionKind::Knockdown), "a forcing toucher");
        assert_eq!(ped_reaction([1.0, 0.0], gate, &r, true, false, false), None, "IgnoreStandingCollisions");
        let marquee = CollisionRules { can_knock_down: false, ..r };
        assert_eq!(ped_reaction([6.5, 0.0], gate, &marquee, false, false, true), Some(ReactionKind::Standing));
    }
}
