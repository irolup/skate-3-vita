//! A traffic car's lane-change passage (retail TU3, evidence only; re-implemented;
//! `.local/research/npc/b69-traffic-manoeuvres.md` "b71 / b72 extensions"; main checked the progress step 82C3C3C0
//! and the road-network values 1.3 / 10 in the exported tables):
//! - enter `sub_82C3A9E0`: from distance d0 on the source lane to d1 = d0 + ext x the spec's passage factor
//!   (`Hash_328B9F4685A14018`, `+3692`) + speed (one second of travel) on the target lane;
//! - `sub_82C3F540`: a cubic Hermite curve between the two lane points (`82E14860`), tangents = the lane direction x
//!   (d1 - d0) x the road network's tangent scale (`Hash_D2C11CC10C9E0E49`, 1.3);
//! - `sub_82E10C88`: an arc-length table of `Hash_E35079BDD5BAE286` (10) chords; its total is the passage length;
//! - every tick (`sub_82C3C3C0`): progress += speed x (+1, -1 reversing) x 1/60; the distance along the road is
//!   lerp(d0, d1, progress / length); the car sits on the curve at the arc-length parameter, facing its tangent
//!   (`sub_82C3F0C8`); done when progress >= length (`sub_82C3A5A8`), the car's lane becomes the target.
//!
//! Multiplayer: the host owns (lanes, d0, d1, progress); the curve and table rebuild from them and the road.

use super::super::Vec3;

/// Road-network values (`roadnetwork` collection); a mod may override them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PassageParams {
    /// Tangent scale (`Hash_D2C11CC10C9E0E49`, 1.3).
    pub tangent_scale: f32,
    /// Arc-table chords (`Hash_E35079BDD5BAE286`, 10).
    pub chords: u32,
}

impl Default for PassageParams {
    fn default() -> Self {
        Self { tangent_scale: 1.3, chords: 10 }
    }
}

/// Most arc-table chords a passage keeps (plain `Copy` data; retail uses 10).
pub const MAX_CHORDS: usize = 32;

/// One lane change in progress.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Passage {
    pub from_lane: u8,
    pub to_lane: u8,
    pub d0: f32,
    pub d1: f32,
    pub progress: f32,
    pub p0: Vec3,
    pub p1: Vec3,
    pub t0: Vec3,
    pub t1: Vec3,
    /// Running chord lengths at parameters k / chords (k = 0..=chords); entry `chords` is the length.
    pub table: [f32; MAX_CHORDS + 1],
    pub chords: u8,
}

fn hermite(p0: Vec3, p1: Vec3, t0: Vec3, t1: Vec3, t: f32) -> Vec3 {
    let (t2, t3) = (t * t, t * t * t);
    let (h00, h10, h01, h11) = (2.0 * t3 - 3.0 * t2 + 1.0, t3 - 2.0 * t2 + t, -2.0 * t3 + 3.0 * t2, t3 - t2);
    std::array::from_fn(|i| h00 * p0[i] + h10 * t0[i] + h01 * p1[i] + h11 * t1[i])
}

fn hermite_derivative(p0: Vec3, p1: Vec3, t0: Vec3, t1: Vec3, t: f32) -> Vec3 {
    let t2 = t * t;
    let (d00, d10, d01, d11) = (6.0 * t2 - 6.0 * t, 3.0 * t2 - 4.0 * t + 1.0, -6.0 * t2 + 6.0 * t, 3.0 * t2 - 2.0 * t);
    std::array::from_fn(|i| d00 * p0[i] + d10 * t0[i] + d01 * p1[i] + d11 * t1[i])
}

fn length(v: Vec3) -> f32 {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}

impl Passage {
    /// `sub_82C3A9E0` + `sub_82C3F540` + `sub_82E10C88`. `from` / `to` = the lane point and unit direction at d0 on
    /// the source lane and at d1 on the target lane (the caller computes d1 with [`Passage::end_distance`]).
    /// `None` when the table has fewer than 2 chords (retail returns a zero length).
    pub fn begin(from_lane: u8, to_lane: u8, d0: f32, d1: f32, from: (Vec3, Vec3), to: (Vec3, Vec3), p: &PassageParams) -> Option<Self> {
        if p.chords < 2 {
            return None;
        }
        let n = p.chords.min(MAX_CHORDS as u32);
        let scale = (d1 - d0) * p.tangent_scale;
        let (p0, p1) = (from.0, to.0);
        let (t0, t1) = (from.1.map(|v| v * scale), to.1.map(|v| v * scale));
        let mut table = [0.0; MAX_CHORDS + 1];
        let mut prev = p0;
        let mut total = 0.0;
        for i in 1..=n {
            let q = hermite(p0, p1, t0, t1, i as f32 / n as f32);
            total += length([q[0] - prev[0], q[1] - prev[1], q[2] - prev[2]]);
            table[i as usize] = total;
            prev = q;
        }
        Some(Self { from_lane, to_lane, d0, d1, progress: 0.0, p0, p1, t0, t1, table, chords: n as u8 })
    }

    /// d1 = d0 + ext x passage factor + speed (one second of travel).
    pub fn end_distance(d0: f32, ext: f32, passage_factor: f32, speed: f32) -> f32 {
        d0 + ext * passage_factor + speed
    }

    pub fn length(&self) -> f32 {
        self.table[self.chords as usize]
    }

    /// `sub_82C3C3C0` (one 60 Hz step of `step` seconds); true when done (`sub_82C3A5A8`).
    pub fn advance(&mut self, speed: f32, reversing: bool, step: f32) -> bool {
        self.progress += speed * if reversing { -1.0 } else { 1.0 } * step;
        self.progress >= self.length()
    }

    /// The distance along the road: lerp(d0, d1, progress / length).
    pub fn distance(&self) -> f32 {
        let l = self.length();
        let f = if l > 0.0 { (self.progress / l).clamp(0.0, 1.0) } else { 1.0 };
        self.d0 + (self.d1 - self.d0) * f
    }

    /// The curve parameter at the current progress (arc table, linear within a chord).
    pub fn parameter(&self) -> f32 {
        let n = self.chords as usize;
        let s = self.progress.clamp(0.0, self.length());
        for k in 1..=n {
            if s <= self.table[k] {
                let span = (self.table[k] - self.table[k - 1]).max(1e-6);
                return ((k - 1) as f32 + (s - self.table[k - 1]) / span) / n as f32;
            }
        }
        1.0
    }

    /// Position on the curve and the unit tangent (the car's heading, `sub_82C3F0C8`).
    pub fn sample(&self) -> (Vec3, Vec3) {
        let t = self.parameter();
        let d = hermite_derivative(self.p0, self.p1, self.t0, self.t1, t);
        let l = length(d).max(1e-6);
        (hermite(self.p0, self.p1, self.t0, self.t1, t), d.map(|v| v / l))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn passage() -> Passage {
        // Lanes 3.5 m apart along +z; d0 = 0, d1 = 20.
        Passage::begin(0, 1, 0.0, 20.0, ([0.0; 3], [0.0, 0.0, 1.0]), ([3.5, 0.0, 20.0], [0.0, 0.0, 1.0]), &PassageParams::default()).unwrap()
    }

    #[test]
    fn the_passage_runs_from_lane_to_lane_along_its_arc_length() {
        let mut p = passage();
        let l = p.length();
        assert!(l > 20.0 && l < 21.0, "{l}");
        assert_eq!(p.sample().0, [0.0; 3]);
        let mut ticks = 0;
        while !p.advance(10.0, false, 1.0 / 60.0) {
            ticks += 1;
        }
        // About length / speed seconds at 60 Hz.
        assert!((ticks as f32 - l / 10.0 * 60.0).abs() <= 1.0, "{ticks}");
        let (end, heading) = p.sample();
        assert!((end[0] - 3.5).abs() < 1e-3 && (end[2] - 20.0).abs() < 1e-3);
        assert!((heading[2] - 1.0).abs() < 1e-3);
        assert_eq!(p.distance(), 20.0);
    }

    #[test]
    fn halfway_the_car_is_between_the_lanes_and_turned() {
        let mut p = passage();
        p.progress = p.length() * 0.5;
        let (mid, heading) = p.sample();
        assert!((mid[0] - 1.75).abs() < 0.05 && (mid[2] - 10.0).abs() < 0.5, "{mid:?}");
        assert!(heading[0] > 0.05, "{heading:?}");
        assert!((p.distance() - 10.0).abs() < 1e-4);
    }

    #[test]
    fn the_end_distance_is_one_second_of_travel_past_the_passage_factor() {
        assert_eq!(Passage::end_distance(10.0, 2.0, 3.0, 12.0), 28.0);
        assert!(Passage::begin(0, 1, 0.0, 1.0, ([0.0; 3], [0.0, 0.0, 1.0]), ([1.0; 3], [0.0, 0.0, 1.0]), &PassageParams { chords: 1, ..Default::default() }).is_none());
    }
}
