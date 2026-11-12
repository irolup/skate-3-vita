//! A ped fleeing a prop that is moving at it (`82E2EDC0` -> `82E2F018` -> `82E2EF40`, ask 374).
//! Retail rules: docs/hails-additions/26g-living-world-ped-interactions.md#object-flee

use super::super::Vec3;

/// Retail values (data-driven).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ObjectFleeParams {
    pub enabled: bool,
    /// Flat distance from the prop's centre (8 m: 64.0 squared at 0x820ED958).
    pub radius: f32,
    /// The prop's speed floor (0.75 m/s: 0.5625 squared at 0x822F915C).
    pub min_speed: f32,
    /// Room on either side of the prop's swept lane (0x821814A0).
    pub lane_margin: f32,
}

impl Default for ObjectFleeParams {
    fn default() -> Self {
        Self { enabled: true, radius: 8.0, min_speed: 0.75, lane_margin: 0.75 }
    }
}

/// A moving prop: box centre, linear velocity and its four footprint corners (world x, z).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MovingObject {
    pub centre: Vec3,
    pub velocity: Vec3,
    pub corners: [[f32; 2]; 4],
}

/// Whether a ped at `ped` should flee `object`: near it, the prop fast enough, the ped inside the lane the prop's
/// footprint sweeps and ahead of the prop's corner on its side.
pub fn in_path(ped: Vec3, object: &MovingObject, p: &ObjectFleeParams) -> bool {
    if !p.enabled {
        return false;
    }
    let to_ped = [ped[0] - object.centre[0], ped[2] - object.centre[2]];
    if to_ped[0] * to_ped[0] + to_ped[1] * to_ped[1] > p.radius * p.radius {
        return false;
    }
    let v = object.velocity;
    if v[0] * v[0] + v[1] * v[1] + v[2] * v[2] < p.min_speed * p.min_speed {
        return false;
    }
    let flat = (v[0] * v[0] + v[2] * v[2]).sqrt();
    if flat < 1e-5 {
        return false;
    }
    let side = [v[2] / flat, -v[0] / flat];
    let (mut low, mut high) = ((0.0f32, [object.centre[0], object.centre[2]]), (0.0f32, [object.centre[0], object.centre[2]]));
    for c in object.corners {
        let s = (c[0] - object.centre[0]) * side[0] + (c[1] - object.centre[2]) * side[1];
        if s > high.0 {
            high = (s, c);
        } else if s < low.0 {
            low = (s, c);
        }
    }
    let lateral = to_ped[0] * side[0] + to_ped[1] * side[1];
    if lateral - p.lane_margin >= high.0 || lateral + p.lane_margin <= low.0 {
        return false;
    }
    let corner = if lateral <= 0.0 { low.1 } else { high.1 };
    (ped[0] - corner[0]) * v[0] + (ped[2] - corner[1]) * v[2] > 0.0
}

/// A ped fleeing a car near it (`82E2C0C8`, IVehicle path, W8), either direction; an opt-in still in test:
/// docs/hails-additions/26g-living-world-ped-interactions.md#vehicle-flee-opt-in-2026-10-29-w8-research-b656
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VehicleFleeParams {
    pub enabled: bool,
    /// Distance to the car (10 m: 100.0 squared at 0x820ED57C).
    pub radius: f32,
    /// Dot of the flat ped-to-car vector and the car's velocity (3.0 at 0x82063B08).
    pub min_dot: f32,
    /// Retail's code test: the car moving away along the ped-to-car line (dot > `min_dot`).
    pub away: bool,
    /// The car moving at the ped (dot < -`min_dot`).
    pub toward: bool,
    /// Retail's gate A (ped vf200 `82E38960`, b821): only a ped standing on a road triangle flees.
    pub require_on_road: bool,
    /// Retail's gate B (IVehicle vf0x60 `82C34598`, b821): only a flagged car, held / skitched this frame (`+4402`
    /// 0x02; the entered-on-red half, `+3424` 0x04, is not ported), makes a ped flee.
    pub require_flagged: bool,
}

impl Default for VehicleFleeParams {
    fn default() -> Self {
        Self { enabled: false, radius: 10.0, min_dot: 3.0, away: true, toward: true, require_on_road: true, require_flagged: true }
    }
}

/// [`flees_vehicle`] behind retail's two gates: the ped on a road and the car flagged (`82E2C0C8`, b821). A car that
/// fails a gate is steered round in retail (`0x82E2C3C0`, not ported), never fled.
pub fn flees_vehicle_gated(ped: Vec3, car: Vec3, velocity: Vec3, ped_on_road: bool, car_flagged: bool, p: &VehicleFleeParams) -> bool {
    (ped_on_road || !p.require_on_road) && (car_flagged || !p.require_flagged) && flees_vehicle(ped, car, velocity, p)
}

/// Whether a ped at `ped` flees a car at `car` moving with `velocity`.
pub fn flees_vehicle(ped: Vec3, car: Vec3, velocity: Vec3, p: &VehicleFleeParams) -> bool {
    if !p.enabled {
        return false;
    }
    let d = [car[0] - ped[0], car[1] - ped[1], car[2] - ped[2]];
    if d[0] * d[0] + d[1] * d[1] + d[2] * d[2] >= p.radius * p.radius {
        return false;
    }
    let dot = d[0] * velocity[0] + d[2] * velocity[2];
    (p.away && dot > p.min_dot) || (p.toward && dot < -p.min_dot)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bin(centre: Vec3, velocity: Vec3) -> MovingObject {
        let (x, z) = (centre[0], centre[2]);
        MovingObject { centre, velocity, corners: [[x - 0.6, z - 0.6], [x + 0.6, z - 0.6], [x + 0.6, z + 0.6], [x - 0.6, z + 0.6]] }
    }

    #[test]
    fn a_bin_pushed_at_a_ped_within_eight_metres_makes_it_flee() {
        let p = ObjectFleeParams::default();
        assert!(in_path([0.0, 0.0, 5.0], &bin([0.0, 0.0, 0.0], [0.0, 0.0, 2.0]), &p));
        assert!(!in_path([0.0, 0.0, 9.0], &bin([0.0, 0.0, 0.0], [0.0, 0.0, 2.0]), &p), "beyond 8 m");
        assert!(!in_path([0.0, 0.0, 5.0], &bin([0.0, 0.0, 0.0], [0.0, 0.0, 0.7]), &p), "under 0.75 m/s");
        assert!(!in_path([0.0, 0.0, -3.0], &bin([0.0, 0.0, 0.0], [0.0, 0.0, 2.0]), &p), "behind the bin");
    }

    #[test]
    fn the_lane_is_the_footprint_plus_the_margin() {
        let p = ObjectFleeParams::default();
        let b = bin([0.0, 0.0, 0.0], [0.0, 0.0, 2.0]);
        assert!(in_path([1.3, 0.0, 4.0], &b, &p), "0.7 m outside the 0.6 m half width");
        assert!(!in_path([1.4, 0.0, 4.0], &b, &p), "0.8 m outside");
        assert!(!in_path([0.0, 0.0, 5.0], &b, &ObjectFleeParams { enabled: false, ..p }), "a mod turned it off");
    }

    #[test]
    fn a_car_within_ten_metres_moving_along_the_line_either_way_makes_the_ped_flee() {
        let on = VehicleFleeParams { enabled: true, ..Default::default() };
        let ped = [0.0, 0.0, 0.0];
        assert!(flees_vehicle(ped, [0.0, 0.0, 6.0], [0.0, 0.0, -5.0], &on), "driving at the ped");
        assert!(flees_vehicle(ped, [0.0, 0.0, 6.0], [0.0, 0.0, 5.0], &on), "driving away (retail's code test)");
        assert!(!flees_vehicle(ped, [0.0, 0.0, 6.0], [5.0, 0.0, 0.0], &on), "crossing: no motion along the line");
        assert!(!flees_vehicle(ped, [0.0, 0.0, 6.0], [0.0, 0.0, -0.4], &on), "dot 2.4 under 3.0");
        assert!(!flees_vehicle(ped, [0.0, 0.0, 10.0], [0.0, 0.0, -9.0], &on), "10 m is outside");
        assert!(!flees_vehicle(ped, [0.0, 0.0, 6.0], [0.0, 0.0, -5.0], &VehicleFleeParams::default()), "off by default");
        assert!(!flees_vehicle(ped, [0.0, 0.0, 6.0], [0.0, 0.0, -5.0], &VehicleFleeParams { toward: false, ..on }), "retail's direction only");
    }

    /// b821 (ask 497): a ped on the sidewalk, or any ped by a car nobody holds, never flees it.
    #[test]
    fn only_a_ped_on_the_road_flees_a_held_car() {
        let on = VehicleFleeParams { enabled: true, ..Default::default() };
        let (ped, car, v) = ([0.0, 0.0, 0.0], [0.0, 0.0, 6.0], [0.0, 0.0, -5.0]);
        assert!(flees_vehicle_gated(ped, car, v, true, true, &on), "on the road, a skitched car");
        assert!(!flees_vehicle_gated(ped, car, v, false, true, &on), "on the sidewalk");
        assert!(!flees_vehicle_gated(ped, car, v, true, false, &on), "plain traffic driving past");
        let loose = VehicleFleeParams { require_on_road: false, require_flagged: false, ..on };
        assert!(flees_vehicle_gated(ped, car, v, false, false, &loose), "a mod can drop both gates");
    }
}
