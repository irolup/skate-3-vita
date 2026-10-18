//! Ped takedowns (doc 26 "Ped takedowns"): which takedown a chasing ped can try on its target.
//! Retail (TU3, evidence only; re-implemented; `.local/research/peds/b16-takedown-choice-contact.md`,
//! main checked the setup data): the ped type's `livingworld_entity_takedown` record lists
//! takedowns in priority order. Each tick (`82E3C000`, from TakeDownTargetablePredictions):
//! - the approach side: `n = normalize(ped - target)`, `dot = n . target velocity`; side 1 when
//!   the target comes towards the ped (`dot > 0`), else 2; `speed = |dot|`;
//! - an entry with a side needs that side and a speed above its minimum;
//! - per entry (`82E3B548`): the target `lead + 0.06` s ahead (`pos + vel t`); more than 1.25 m
//!   above or below fails, else flattened to the ped's height; the distance from the ped's
//!   reach point (`ped + 0.2 x forward`) must lie in `(min + 0.2 + 0.06 s, reach + 0.2 + 0.06 s]`
//!   (`s` = the ped's speed); the signed bearing (about up, positive when
//!   `(forward x to_target) . up > 0`) in `[angle_min, angle_max]` degrees, or, for an entry that
//!   allows it, in `[-angle_max, -angle_min]` (mirrored, the `anim_b` clip);
//! - the first entry that fits wins.

use super::super::Vec3;

/// One `takedowns` entry (`livingworld_entity_takedown`).
#[derive(Clone, Debug, PartialEq)]
pub struct TakedownEntry {
    pub anim_name: String,
    /// Seconds the target is predicted ahead (`f32_8`).
    pub lead: f32,
    /// The distance window's near edge (`f32_12`) and far edge (`reach`), m.
    pub min: f32,
    pub reach: f32,
    pub angle_min: f32,
    pub angle_max: f32,
    /// Required approach side (`u32_28`; 0 = any).
    pub side: u32,
    /// The target's minimum approach speed for a sided entry, m/s (`u32_32` as a float).
    pub min_speed: f32,
    /// The entry may be mirrored (byte `+36`).
    pub mirrors: bool,
}

/// A ped type's takedown table.
#[derive(Clone, Debug, PartialEq)]
pub struct TakedownTable {
    pub entries: Vec<TakedownEntry>,
    /// The reach point's offset ahead of the ped, m (`8B45`, 0.2).
    pub reach_offset: f32,
    /// Prediction slack and the ped-speed scale of the window, s (`98DA`, 0.06).
    pub slack: f32,
}

impl Default for TakedownTable {
    fn default() -> Self {
        Self { entries: Vec::new(), reach_offset: 0.2, slack: 0.06 }
    }
}

/// The height difference that rules a takedown out, m (`82E3B548`).
pub const MAX_HEIGHT_GAP: f32 = 1.25;

/// The chosen takedown (`brain+3228` entry, `+3136` predicted position, `+3276` bit 0x80).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TakedownChoice {
    pub entry: usize,
    pub mirrored: bool,
    pub predicted: Vec3,
}

/// `82E3C000`: the first takedown in `table` that fits a ped at `ped` facing `forward` (flat
/// unit) moving at `ped_speed` against a target at `target` moving at `velocity`.
pub fn choose(table: &TakedownTable, ped: Vec3, forward: [f32; 2], ped_speed: f32, target: Vec3, velocity: Vec3) -> Option<TakedownChoice> {
    let d = [ped[0] - target[0], ped[1] - target[1], ped[2] - target[2]];
    let l = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
    let dot = if l > 1e-6 { (d[0] * velocity[0] + d[1] * velocity[1] + d[2] * velocity[2]) / l } else { 0.0 };
    let side = if dot > 0.0 { 1 } else { 2 };
    let speed = dot.abs();
    let reach_point = [ped[0] + forward[0] * table.reach_offset, ped[2] + forward[1] * table.reach_offset];
    let slack = table.reach_offset + table.slack * ped_speed;
    table.entries.iter().enumerate().find_map(|(i, e)| {
        if e.side != 0 && (e.side != side || !(e.min_speed < speed)) {
            return None;
        }
        let t = e.lead + table.slack;
        let p = [target[0] + velocity[0] * t, target[1] + velocity[1] * t, target[2] + velocity[2] * t];
        if (p[1] - ped[1]).abs() > MAX_HEIGHT_GAP {
            return None;
        }
        let r = [p[0] - reach_point[0], p[2] - reach_point[1]];
        let dist = (r[0] * r[0] + r[1] * r[1]).sqrt();
        if !(e.min + slack < dist && dist <= e.reach + slack) {
            return None;
        }
        let to = [p[0] - ped[0], p[2] - ped[2]];
        // Signed bearing about up (y): positive when (forward x to) . up > 0.
        let cross = forward[1] * to[0] - forward[0] * to[1];
        let angle = cross.atan2(forward[0] * to[0] + forward[1] * to[1]).to_degrees();
        let mirrored = if (e.angle_min..=e.angle_max).contains(&angle) {
            false
        } else if e.mirrors && (-e.angle_max..=-e.angle_min).contains(&angle) {
            true
        } else {
            return None;
        };
        Some(TakedownChoice { entry: i, mirrored, predicted: [p[0], ped[1], p[2]] })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(min: f32, reach: f32, side: u32, min_speed: f32) -> TakedownEntry {
        TakedownEntry { anim_name: "t".into(), lead: 0.3, min, reach, angle_min: -30.0, angle_max: 30.0, side, min_speed, mirrors: true }
    }

    #[test]
    fn first_fitting_takedown_wins() {
        // A never-fitting entry (min above reach, as the stock PushCatch), then a sided one, then any side.
        let table = TakedownTable { entries: vec![entry(0.1, 0.0, 0, 0.0), entry(0.0, 1.0, 1, 6.0), entry(0.0, 1.2, 0, 0.0)], ..Default::default() };
        // Target 1 m ahead (+z), coming at the ped at 3 m/s: side 1 but too slow for entry 1.
        let c = choose(&table, [0.0; 3], [0.0, 1.0], 0.0, [0.0, 0.0, 2.0], [0.0, 0.0, -3.0]).unwrap();
        assert_eq!(c.entry, 2);
        assert!((c.predicted[2] - (2.0 - 3.0 * 0.36)).abs() < 1e-5);
        // Too far, too high, or behind: nothing.
        assert!(choose(&table, [0.0; 3], [0.0, 1.0], 0.0, [0.0, 0.0, 5.0], [0.0; 3]).is_none());
        assert!(choose(&table, [0.0; 3], [0.0, 1.0], 0.0, [0.0, 1.5, 1.0], [0.0; 3]).is_none());
        assert!(choose(&table, [0.0; 3], [0.0, 1.0], 0.0, [0.0, 0.0, -1.0], [0.0; 3]).is_none());
    }
}
