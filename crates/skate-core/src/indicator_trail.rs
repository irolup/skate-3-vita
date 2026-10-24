//! The ribbon trail a called skater leaves behind (retail `827AEC80` / `827B24F8`, research b566): every period the
//! anchor point is sampled and appended once it moved far enough; a jump clears the trail.
//! Details: docs/hails-additions/26b-living-world-population.md#called-skaters-fe-62-call-skater

#[derive(Clone, Debug, PartialEq)]
pub struct TrailRules {
    /// Sample period, seconds (0x82165A00 = 0.05; the timer keeps the remainder).
    pub period: f32,
    /// A point is added when the squared distance to the last one exceeds this (0.0625 = 0.25 m).
    pub spacing2: f32,
    /// Points kept, oldest dropped (71).
    pub max_points: usize,
    /// Squared distance minus `spacing2` above which the trail starts over (5.0: a teleport).
    pub clear: f32,
    /// Height added to the anchor (0x820C6D98 = 0.25 m).
    pub lift: f32,
    /// The `visualindicators` row (`trail_follow` for a called skater, 0x821AEB70).
    pub style: String,
    /// The anchor bone (SkaterPresentationDB `skateboard_root`, 82787EA8 +8; b567).
    pub bone: String,
}

impl Default for TrailRules {
    fn default() -> Self {
        Self { period: 0.05, spacing2: 0.0625, max_points: 71, clear: 5.0, lift: 0.25, style: "trail_follow".into(), bone: "Skateboard_Root".into() }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Trail {
    /// Oldest first.
    pub points: Vec<[f32; 3]>,
    timer: f32,
}

impl Trail {
    /// Advance by `dt`; `anchor` is the skater's point (before the lift). True when the points changed.
    pub fn step(&mut self, dt: f32, anchor: [f32; 3], rules: &TrailRules) -> bool {
        self.timer += dt;
        if self.timer < rules.period {
            return false;
        }
        self.timer -= rules.period;
        if self.timer >= rules.period {
            self.timer %= rules.period;
        }
        let p = [anchor[0], anchor[1] + rules.lift, anchor[2]];
        let Some(last) = self.points.last() else {
            self.points.push(p);
            return true;
        };
        let d = (0..3).map(|i| (p[i] - last[i]).powi(2)).sum::<f32>() - rules.spacing2;
        if d > rules.clear {
            self.points.clear();
            self.points.push(p);
            return true;
        }
        if d <= 0.0 {
            return false;
        }
        self.points.push(p);
        if self.points.len() > rules.max_points {
            let extra = self.points.len() - rules.max_points;
            self.points.drain(..extra);
        }
        true
    }

    /// The drawn curve (`827B2BA8`, b567): a uniform cubic B-spline over the points (it does not pass through them),
    /// `samples` per segment, N - 3 segments, nothing below 4 points; each with its u in segment units.
    pub fn curve(&self, samples: usize) -> Vec<([f32; 3], f32)> {
        let p = &self.points;
        let n = samples.max(1);
        let mut out = Vec::new();
        for k in 0..p.len().saturating_sub(3) {
            for i in 0..n {
                let s = i as f32 / n as f32;
                let b = [(1.0 - s).powi(3), 3.0 * s.powi(3) - 6.0 * s * s + 4.0, -3.0 * s.powi(3) + 3.0 * s * s + 3.0 * s + 1.0, s.powi(3)];
                let point = std::array::from_fn(|a| (0..4).map(|j| b[j] * p[k + j][a]).sum::<f32>() / 6.0);
                out.push((point, k as f32 + s));
            }
        }
        out
    }

    pub fn clear(&mut self) {
        self.points.clear();
        self.timer = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn samples_every_period_with_spacing_cap_and_clear() {
        let rules = TrailRules::default();
        let mut t = Trail::default();
        assert!(!t.step(0.04, [0.0; 3], &rules));
        assert!(t.step(0.02, [0.0; 3], &rules));
        assert_eq!(t.points, [[0.0, 0.25, 0.0]]);
        // Closer than 0.25 m: nothing added.
        assert!(!t.step(0.05, [0.2, 0.0, 0.0], &rules));
        for i in 1..=100 {
            t.step(0.05, [i as f32 * 0.3, 0.0, 0.0], &rules);
        }
        assert_eq!(t.points.len(), 71);
        assert!((t.points.last().unwrap()[0] - 30.0).abs() < 1e-3);
        // A jump of more than ~2.25 m starts over.
        assert!(t.step(0.05, [40.0, 0.0, 0.0], &rules));
        assert_eq!(t.points.len(), 1);
    }

    #[test]
    fn curve_is_a_b_spline_from_four_points() {
        let mut t = Trail::default();
        t.points = vec![[0.0; 3], [1.0, 0.0, 0.0], [2.0, 0.0, 0.0]];
        assert!(t.curve(10).is_empty());
        t.points.push([3.0, 0.0, 0.0]);
        t.points.push([4.0, 5.0, 0.0]);
        let c = t.curve(10);
        assert_eq!(c.len(), 2 * 10);
        // A straight run maps onto itself: segment 0 starts at (p0 + 4 p1 + p2) / 6 = 1.
        assert!((c[0].0[0] - 1.0).abs() < 1e-6 && c[0].0[1].abs() < 1e-6);
        // It does not pass through the kink at p4.
        assert!(c.iter().all(|(q, _)| q[1] < 5.0));
    }
}
