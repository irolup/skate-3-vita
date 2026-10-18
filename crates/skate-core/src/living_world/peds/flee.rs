//! Where a fleeing pedestrian runs (doc 26 "Ped behaviour runtime"). Retail (TU3, evidence only;
//! re-implemented; `.local/research/peds/b8-ped-flee-steering.md`, main checked the leg length,
//! the speed gate and the blend):
//! - the Flee intent installs a goal provider (`sub_82E35E80`, provider vtable `0x8232C818`) and an
//!   arrival radius of 2.0 m (`S.vfn136`, `0x82060C50`);
//! - the direction (`sub_82E318F8`): `away` = flat ped position - flat threat position; when the
//!   threat moves slower than 1 m/s (`0x8231A844`) or the ped is behind its motion, straight away;
//!   else `0.5 x (side + normalize(away))` (`0x8209975C`; not renormalised), `side` = the threat's
//!   motion direction x up, on the ped's side;
//! - the goal = position + direction x 15 m (`0x820BD16C`), then a navmesh cast (`sub_82C465A8`;
//!   a blocked goal is rewritten by `sub_82C46208`: we stop at the last clear point, inferred);
//! - a new leg when the ped is within the arrival radius of its goal (`sub_82E2BC70`, 3D).
//! The run speed is the ped type's chase record (`CD65`, 11 for pedestrians) turned into a gait
//! (`sub_82E2D260`); our body plays the run cycle and moves by its root motion.

use super::super::Vec3;

/// Retail values (data-driven).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FleeParams {
    pub leg: f32,
    pub arrival_radius: f32,
    pub threat_speed_gate: f32,
    pub side_blend: f32,
}

impl Default for FleeParams {
    fn default() -> Self {
        Self { leg: 15.0, arrival_radius: 2.0, threat_speed_gate: 1.0, side_blend: 0.5 }
    }
}

/// `sub_82E318F8`: the flat direction to flee in (unit, or shorter when blended).
pub fn direction(position: Vec3, threat: Vec3, threat_velocity: Vec3, p: &FleeParams) -> [f32; 2] {
    let away = [position[0] - threat[0], position[2] - threat[2]];
    let len = (away[0] * away[0] + away[1] * away[1]).sqrt();
    let away = if len > 1e-5 { [away[0] / len, away[1] / len] } else { [0.0, 1.0] };
    let v = [threat_velocity[0], threat_velocity[2]];
    let speed = (v[0] * v[0] + v[1] * v[1]).sqrt();
    if speed < p.threat_speed_gate {
        return away;
    }
    let m = [v[0] / speed, v[1] / speed];
    // The ped behind the threat's motion: straight away.
    if away[0] * m[0] + away[1] * m[1] < 0.0 {
        return away;
    }
    // Motion direction x up, flipped to the ped's side.
    let mut side = [m[1], -m[0]];
    if side[0] * away[0] + side[1] * away[1] < 0.0 {
        side = [-side[0], -side[1]];
    }
    [p.side_blend * (side[0] + away[0]), p.side_blend * (side[1] + away[1])]
}

/// The next leg's goal before the navmesh cast.
pub fn goal(position: Vec3, threat: Vec3, threat_velocity: Vec3, p: &FleeParams) -> Vec3 {
    let d = direction(position, threat, threat_velocity, p);
    [position[0] + d[0] * p.leg, position[1], position[2] + d[1] * p.leg]
}

/// `sub_82E2BC70`: time for a new leg.
pub fn arrived(position: Vec3, goal: Vec3, p: &FleeParams) -> bool {
    let d = [position[0] - goal[0], position[1] - goal[1], position[2] - goal[2]];
    d[0] * d[0] + d[1] * d[1] + d[2] * d[2] <= p.arrival_radius * p.arrival_radius
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_slow_threat_sends_the_ped_straight_away_15_m() {
        let p = FleeParams::default();
        let g = goal([0.0; 3], [0.0, 0.0, -3.0], [0.0, 0.0, 0.5], &p);
        assert!((g[2] - 15.0).abs() < 1e-4 && g[0].abs() < 1e-4, "{g:?}");
        assert!(arrived([0.0, 0.0, 13.5], g, &p));
        assert!(!arrived([0.0, 0.0, 12.5], g, &p));
    }

    #[test]
    fn a_threat_coming_at_the_ped_sends_it_off_to_the_side() {
        let p = FleeParams::default();
        // Threat 3 m behind (-z), moving +z at 8 m/s, the ped slightly to its right (+x).
        let d = direction([0.5, 0.0, 0.0], [0.0, 0.0, -3.0], [0.0, 0.0, 8.0], &p);
        // Side = +x (the ped's side), away mostly +z: blended and not renormalised.
        assert!(d[0] > 0.4 && d[1] > 0.4, "{d:?}");
        assert!((d[0] * d[0] + d[1] * d[1]).sqrt() < 1.0);
        // Behind the threat's motion: straight away.
        let b = direction([0.0, 0.0, -6.0], [0.0, 0.0, -3.0], [0.0, 0.0, 8.0], &p);
        assert!((b[1] + 1.0).abs() < 1e-4, "{b:?}");
    }
}
