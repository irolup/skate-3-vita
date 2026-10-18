//! Running from a honking car (doc 26g "Peds run from honking cars"). Retail (TU3, evidence only;
//! re-implemented; `.local/research/npc/b44-horn-honker.md`, `.local/research/peds/b45-runfromhonker-flee-timeout.md`;
//! main re-read the honk receiver `sub_82E3C3D0` and the shape of `826A1358`):
//! - a traffic car's horn kind 2 writes its id into the ped brain's honker (`+3232`); `IsBeingHonkedAt` leads
//!   Wander / WanderFollow into RunFromHonker; only Wander's Begin clears the honker;
//! - RunFromHonker Begin (`826A1330`) posts motion intent 5; Update (`826A1358`) looks the car up every frame and,
//!   while it exists, sends the ped to a goal `side_distance` sideways of the car's line, on the ped's side (strict
//!   `dot > 0` test, a tie goes to the minus side), at `run_speed` when the goal is more than `near_radius` away,
//!   else `near_speed`. A car that is gone leaves the last goal and speed; the op's `timeout` (30) is never read.
//! The car vector Update reads (`[car+164]+144`) is not settled: velocity or an axis (open, b45); ours uses the
//! car's forward axis, so a standing car still gives a side.

use super::super::Vec3;

/// Retail values (`826A1358` constants; data a mod may override through `ped_brain`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RunFromHonkerParams {
    /// m sideways of the car's line.
    pub side_distance: f32,
    /// m: a goal this close gives the near speed.
    pub near_radius: f32,
    pub run_speed: f32,
    pub near_speed: f32,
}

impl Default for RunFromHonkerParams {
    fn default() -> Self {
        Self { side_distance: 10.0, near_radius: 2.0, run_speed: 6.0, near_speed: 3.0 }
    }
}

/// `826A1358`: the goal and speed for a ped at `ped` running from a car at `car` along `dir`. `None` when the
/// direction has no horizontal part (degenerate, as the normalise in retail).
pub fn run_goal(ped: Vec3, car: Vec3, dir: Vec3, p: &RunFromHonkerParams) -> Option<(Vec3, f32)> {
    // S = norm(D x (0, -1, 0)) = norm((Dz, 0, -Dx)).
    let l = (dir[2] * dir[2] + dir[0] * dir[0]).sqrt();
    if !(l > 1e-6) {
        return None;
    }
    let s = [dir[2] / l, 0.0, -dir[0] / l];
    let side = (ped[0] - car[0]) * s[0] + (ped[2] - car[2]) * s[2];
    let k = if side > 0.0 { p.side_distance } else { -p.side_distance };
    let goal = [ped[0] + s[0] * k, ped[1], ped[2] + s[2] * k];
    let d2 = (goal[0] - ped[0]).powi(2) + (goal[2] - ped[2]).powi(2);
    Some((goal, if d2 - p.near_radius * p.near_radius > 0.0 { p.run_speed } else { p.near_speed }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ped_runs_ten_metres_sideways_on_its_own_side() {
        let p = RunFromHonkerParams::default();
        // Car at the origin driving +z; S = (1, 0, 0).
        let (g, v) = run_goal([2.0, 0.0, 5.0], [0.0; 3], [0.0, 0.0, 1.0], &p).unwrap();
        assert_eq!((g, v), ([12.0, 0.0, 5.0], 6.0));
        let (g, _) = run_goal([-2.0, 0.0, 5.0], [0.0; 3], [0.0, 0.0, 1.0], &p).unwrap();
        assert_eq!(g, [-12.0, 0.0, 5.0]);
        let (g, _) = run_goal([0.0, 0.0, 5.0], [0.0; 3], [0.0, 0.0, 1.0], &p).unwrap();
        assert_eq!(g, [-10.0, 0.0, 5.0], "on the line: the minus side");
        assert_eq!(run_goal([0.0; 3], [0.0; 3], [0.0, 1.0, 0.0], &p), None);
        let near = RunFromHonkerParams { side_distance: 1.0, ..p };
        assert_eq!(run_goal([1.0, 0.0, 0.0], [0.0; 3], [0.0, 0.0, 1.0], &near).unwrap().1, 3.0);
    }
}
