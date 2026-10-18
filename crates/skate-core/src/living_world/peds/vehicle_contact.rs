//! A traffic car touching a ped (doc 26, "Cars hit peds"): detection and the ped's response.
//!
//! What retail does [code, TU3; addresses are evidence only]:
//! - Every contact on a ped's collision body goes through the ped's contact callback
//!   `sub_82E38FB8` (ped vtable `0x8232BE80`). It first classifies the contact with vtable slot
//!   +164, `sub_82E38400`, which returns a contact kind 0..5 ([`RetailContactKind`]). The other
//!   body's owner (`[[contact+76]+32]`) is checked with the interface cast `sub_82965630`: an
//!   owner that is an `IVehicle` (type getter `0x82C34050` -> `0x823220B8`, string `IVehicle`)
//!   gives kind 2, with no speed or angle test.
//! - The callback's switch at `0x82E39230` sends kinds 1 and 2 to the same block (`0x82E3926C`):
//!   the ped's root is set to its collision body's position after the solver moved the body
//!   (body pose from `sub_82585CB0`, minus the body-to-root offset at `[ped+5756]+19872` when the
//!   ped's slot +180 says so), keeping the root's height (`vrlimi` keeps y), skipped when the
//!   result is not finite or out of range (`0x822F88D4`). Nothing else: no reaction kind or
//!   direction (`ped+2496` / `+2500`), no `Collision` intent, no speech, no brain flag
//!   (`+3196` bit 0x80 is set only for kinds 4 and 5). So a car never knocks a ped down,
//!   never makes it stumble, ragdoll or fade: the car's (infinite mass, kinematic) box shoves
//!   the ped's body out of the way and the ped walks on.
//! - Kind 5 (an `IActor` owner, `0x82586478` -> `0x823000F0`, e.g. the skater) is the
//!   knock-down / stumble path (3.0 / 6.0 thresholds, `peds-re.md` section 5); kind 4 is an actor
//!   contact on body part 1 or 2 (acted on only while `[ped+5756]+140` is 7); kind 3 sets `+3278` bit 0x80.
//! - The car side (`sub_82C3C150`, vehicle collision interface `+136`) records an `IActor`
//!   toucher in its "hit by" mask `+4248` and sets `+4401` bit 0x20 for a contact ahead of the
//!   car; it does not stop or honk there. Peds flee from cars only through the horn
//!   (`sub_82C40660` kind 2, the honked-at input, `RunFromHonker`; `traffic/horn.rs` sets the
//!   ped's honker, the RunFromHonker op is not ported yet).
//!
//! Ours (stated, NOT RETAIL YET where marked): the car is our kinematic box (GLB bounds), the
//! ped body is a vertical cylinder of the NavPower agent radius and height (retail's Havok ped
//! shape, `sub_82E26430`, is not decoded); the solver's separation is the smallest push that
//! takes the cylinder out of the box in the ground plane (Havok's own penetration recovery is
//! not decoded). The game then keeps the pushed ped on its navmesh, standing in for the world
//! collision the retail body has.
//!
//! Deterministic: pure functions of the poses; the game resolves peds and cars in id order.

use crate::living_world::Vec3;

/// The contact kinds `sub_82E38400` returns [code].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RetailContactKind {
    /// 0: ignored.
    None,
    /// 1: a contact with the ped's own body against the world (flag `+5936` bit 0x40 and a
    /// closing speed test).
    Static,
    /// 2: the other body belongs to an `IVehicle`.
    Vehicle,
    /// 3: the object the ped is attached to (`ped+5916`).
    Attached,
    /// 4: an actor contact on body part 1 or 2 (acted on only while `[ped+5756]+140` is 7).
    ActorBusy,
    /// 5: an `IActor` contact: knock-down or stumble.
    Actor,
}

/// What the ped's contact callback does for a kind (`sub_82E38FB8` switch at `0x82E39230`) [code].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RetailContactResponse {
    Nothing,
    /// The root follows the pushed body (xz), height kept.
    FollowBody,
    /// Brain flag `+3278` bit 0x80.
    FlagAttached,
    /// Actor-contact impulse path (slot +204 with 8).
    ActorImpulse,
    /// Knock-down (kind 0) or stumble (kind 1) by the 3.0 / 6.0 thresholds.
    KnockDownOrStumble,
}

/// The callback's switch [code `0x82E39230`]: kinds 1 and 2 share one block.
pub fn retail_response(kind: RetailContactKind) -> RetailContactResponse {
    match kind {
        RetailContactKind::None => RetailContactResponse::Nothing,
        RetailContactKind::Static | RetailContactKind::Vehicle => RetailContactResponse::FollowBody,
        RetailContactKind::Attached => RetailContactResponse::FlagAttached,
        RetailContactKind::ActorBusy => RetailContactResponse::ActorImpulse,
        RetailContactKind::Actor => RetailContactResponse::KnockDownOrStumble,
    }
}

/// What a car contact does to a ped (the event's `reaction`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VehicleContactReaction {
    /// Retail: the ped is shoved out of the car's box and walks on.
    Push,
    /// The contact is reported only (a mod switched the push off).
    Reported,
}

impl VehicleContactReaction {
    pub fn name(self) -> &'static str {
        match self {
            Self::Push => "push",
            Self::Reported => "reported",
        }
    }
}

/// Rules: retail values as defaults (a mod overrides any of them).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VehicleContactParams {
    /// Cars and peds touch at all (retail: yes, the ped callback handles `IVehicle` contacts).
    pub enabled: bool,
    /// The ped is pushed out of the car (retail: yes, kind 2 -> [`RetailContactResponse::FollowBody`]).
    pub push: bool,
}

impl Default for VehicleContactParams {
    fn default() -> Self {
        Self { enabled: true, push: true }
    }
}

impl VehicleContactParams {
    /// The reaction for a detected contact (retail has no speed threshold for kind 2).
    pub fn reaction(&self) -> VehicleContactReaction {
        if self.push && retail_response(RetailContactKind::Vehicle) == RetailContactResponse::FollowBody {
            VehicleContactReaction::Push
        } else {
            VehicleContactReaction::Reported
        }
    }
}

/// A car's collision box, world space: centre, unit forward in xz, half extents (x right,
/// y up, z forward).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CarBox {
    pub center: Vec3,
    pub forward: [f32; 2],
    pub half: Vec3,
}

/// A ped's body: feet position, radius and height.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PedCylinder {
    pub feet: Vec3,
    pub radius: f32,
    pub height: f32,
}

/// One detected contact.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VehicleContact {
    /// Unit direction (xz) from the car's box towards the ped.
    pub normal: [f32; 2],
    /// Overlap along `normal`, metres.
    pub depth: f32,
    /// Ped feet position after the push (height kept, as retail keeps the root's y).
    pub pushed: Vec3,
}

/// Does the ped's cylinder overlap the car's box, and what push separates them? `None` when
/// apart (touching at zero depth counts as apart).
pub fn detect(car: &CarBox, ped: &PedCylinder) -> Option<VehicleContact> {
    let [fx, fz] = car.forward;
    let len = (fx * fx + fz * fz).sqrt();
    if !(len > 1e-6) || !(ped.radius >= 0.0) {
        return None;
    }
    let (fx, fz) = (fx / len, fz / len);
    // Right = forward turned -90 degrees about +y (x right, z forward).
    let (rx, rz) = (fz, -fx);
    // Vertical overlap of [feet, feet + height] with the box.
    let (lo, hi) = (car.center[1] - car.half[1], car.center[1] + car.half[1]);
    if ped.feet[1] >= hi || ped.feet[1] + ped.height <= lo {
        return None;
    }
    let (dx, dz) = (ped.feet[0] - car.center[0], ped.feet[2] - car.center[2]);
    let u = dx * rx + dz * rz;
    let w = dx * fx + dz * fz;
    let (hu, hw) = (car.half[0], car.half[2]);
    let (cu, cw) = (u.clamp(-hu, hu), w.clamp(-hw, hw));
    let (nu, nw, depth) = if cu != u || cw != w {
        let (eu, ew) = (u - cu, w - cw);
        let d = (eu * eu + ew * ew).sqrt();
        if d >= ped.radius {
            return None;
        }
        (eu / d, ew / d, ped.radius - d)
    } else {
        // Centre inside the box: out through the nearest side.
        let (pu, pw) = (hu - u.abs(), hw - w.abs());
        if pu <= pw {
            (if u < 0.0 { -1.0 } else { 1.0 }, 0.0, pu + ped.radius)
        } else {
            (0.0, if w < 0.0 { -1.0 } else { 1.0 }, pw + ped.radius)
        }
    };
    let normal = [nu * rx + nw * fx, nu * rz + nw * fz];
    if !(depth > 0.0) || !depth.is_finite() {
        return None;
    }
    let pushed = [ped.feet[0] + normal[0] * depth, ped.feet[1], ped.feet[2] + normal[1] * depth];
    Some(VehicleContact { normal, depth, pushed })
}

/// The car's speed into the ped along the contact normal (m/s, positive = closing).
pub fn closing_speed(normal: [f32; 2], car_velocity: Vec3, ped_velocity: Vec3) -> f32 {
    (car_velocity[0] - ped_velocity[0]) * normal[0] + (car_velocity[2] - ped_velocity[2]) * normal[1]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn car() -> CarBox {
        // A 1.8 x 1.5 x 4.4 m car at the origin facing +z.
        CarBox { center: [0.0, 0.75, 0.0], forward: [0.0, 1.0], half: [0.9, 0.75, 2.2] }
    }

    fn ped(x: f32, z: f32) -> PedCylinder {
        PedCylinder { feet: [x, 0.0, z], radius: 0.3, height: 1.6 }
    }

    #[test]
    fn retail_vehicle_contact_follows_the_body_and_never_knocks_down() {
        assert_eq!(retail_response(RetailContactKind::Vehicle), RetailContactResponse::FollowBody);
        assert_eq!(retail_response(RetailContactKind::Static), RetailContactResponse::FollowBody);
        assert_eq!(retail_response(RetailContactKind::Actor), RetailContactResponse::KnockDownOrStumble);
        assert_eq!(VehicleContactParams::default(), VehicleContactParams { enabled: true, push: true });
        assert_eq!(VehicleContactParams::default().reaction(), VehicleContactReaction::Push);
        assert_eq!(VehicleContactParams { enabled: true, push: false }.reaction(), VehicleContactReaction::Reported);
    }

    #[test]
    fn a_ped_in_front_of_the_bumper_is_pushed_forward() {
        let c = detect(&car(), &ped(0.0, 2.3)).expect("0.1 m from the bumper with a 0.3 m body touches");
        assert!((c.normal[0]).abs() < 1e-6 && (c.normal[1] - 1.0).abs() < 1e-6);
        assert!((c.depth - 0.2).abs() < 1e-5);
        assert!((c.pushed[2] - 2.5).abs() < 1e-5 && c.pushed[1] == 0.0);
    }

    #[test]
    fn a_ped_beside_or_clear_of_the_car_is_not_touched() {
        assert!(detect(&car(), &ped(1.21, 0.0)).is_none());
        assert!(detect(&car(), &ped(0.0, 2.51)).is_none());
        // Above the roof (a ped on a bridge over the car).
        let mut p = ped(0.0, 0.0);
        p.feet[1] = 1.6;
        assert!(detect(&car(), &p).is_none());
    }

    #[test]
    fn a_ped_inside_the_box_leaves_through_the_nearest_side() {
        let c = detect(&car(), &ped(0.6, 0.0)).unwrap();
        assert!((c.normal[0] - 1.0).abs() < 1e-6);
        assert!((c.pushed[0] - 1.2).abs() < 1e-5, "{:?}", c.pushed);
        let c = detect(&car(), &ped(-0.1, -2.0)).unwrap();
        assert!((c.normal[1] + 1.0).abs() < 1e-6);
        assert!((c.pushed[2] + 2.5).abs() < 1e-5);
    }

    #[test]
    fn a_turned_car_pushes_along_its_own_axes() {
        // Facing +x: its right is -z, a ped at x = 2.3 is at its front bumper.
        let b = CarBox { forward: [1.0, 0.0], ..car() };
        let c = detect(&b, &ped(2.3, 0.0)).unwrap();
        assert!((c.normal[0] - 1.0).abs() < 1e-6 && c.normal[1].abs() < 1e-6);
        assert!(closing_speed(c.normal, [10.0, 0.0, 0.0], [0.0; 3]) > 9.9);
        assert!(closing_speed(c.normal, [-10.0, 0.0, 0.0], [0.0; 3]) < 0.0);
    }
}
