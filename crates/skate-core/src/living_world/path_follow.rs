//! Retail's AI path projection (research `.local/research/npc/b110-ai-path-projection.md`): a physics AI skater's
//! place on its recorded line follows the skater, not the recording's clock. Per tick (`sub_8246D560`):
//! 1. the nearest node in a forward window from the search start (`sub_8246D3C0` -> `sub_82455BB0`, start..=start+4,
//!    squared 3D distance; the start only grows);
//! 2. the projection onto the two segments around it (`sub_824563D8`), kept or replaced against cursor B with a
//!    0.5 hysteresis, and clamped forward to A + 0.25 of A->B when it falls back (`sub_8246C930`): cursor A;
//! 3. distance to A and the recorded lane test (`sub_8246CCC8`, node widths x 1/50);
//! 4. off path: the rejoin commit within 0.5 m or inside the lane (`sub_8246E2B8`);
//! 5. cursor B = A + the speed-shape ratio in recorded frames (`sub_8246E3B8` -> `sub_82456628`);
//! 6. on path is dropped beyond 1.0 m outside the lane (`sub_8246DB50`).
//!
//! Positions are (node, t): t in [0, 1] along the segment node -> node + 1. Pure and deterministic (no clock).
//! 0. "lost" (`sub_8246D3C0` -> `sub_82468DD8`, b115): on a tick the search node did not advance, a skater that was
//!    off the path, farther than 6 m from the search node and from both lane edge points, and slower than 14 m/s is
//!    lost; the caller turns it into the reposition request (`ctrl+941`) that hands it to controller B.
//!
//! NOT RETAIL YET: the 4.5 m lost test after a line-end re-pick, the node-change handler (`sub_8246BEE0`), the
//! `G+608` and skater flag `0x04000000` in the lost test; which body point retail
//! projects (`sub_82467FC8`) is open (ours: the deck), and so is the lane edge axis (`sub_82454648`; ours: the
//! segment's right).

use super::replay::{ReplayLine, WIDTH_SCALE};
use super::Vec3;

/// The projection's values, retail defaults ([code] addresses in b110; the two `tunable` ones are live tunables).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PathTuning {
    /// Forward search window from the search start, nodes (`sub_82455BB0`: start..=start+4).
    pub window: u32,
    /// Keep B unless the projection is closer by this much (squared distances, `sub_8246C930`).
    pub hysteresis: f32,
    /// A fallen-back candidate is clamped to A + this fraction of A->B (`sub_8246C930`).
    pub clamp_fraction: f32,
    /// ... when the clamped point is within this horizontal / vertical distance of the skater.
    pub clamp_horizontal: f32,
    pub clamp_vertical: f32,
    /// Rejoin commit distance (tunable `84FD`, 0.5 m).
    pub commit: f32,
    /// On-path drop distance (tunable `996C`, 1.0 m).
    pub drop: f32,
    /// The start gate latches once A is past this node (`pc+937`).
    pub start_node: u32,
    /// Lost: farther than this from the search node and its lane edges (`0x8208F74C`, 6.0 m) ...
    pub lost_radius: f32,
    /// ... while slower than this along its heading (`pc+808` < `0x8208EB10`, 14.0 m/s).
    pub lost_max_speed: f32,
    /// The progress flag's share of A -> B the raw projection must advance per tick (0.025, b110).
    pub progress_fraction: f32,
}

impl Default for PathTuning {
    fn default() -> Self {
        Self { window: 4, hysteresis: 0.5, clamp_fraction: 0.25, clamp_horizontal: 1.0, clamp_vertical: 5.0, commit: 0.5, drop: 1.0, start_node: 3, lost_radius: 6.0, lost_max_speed: 14.0, progress_fraction: 0.025 }
    }
}

/// One AI skater's projection state (`pc+608..+944`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PathFollow {
    /// `pc+816`: the nearest-node search start; only grows.
    pub search_start: u32,
    /// Cursor A (the skater's projection) and cursor B (A plus one shaped recorded tick).
    pub a: (u32, f32),
    pub b: (u32, f32),
    /// `pc+764/768`: the last raw projection.
    pub last: (u32, f32),
    /// `pc+920`: on path. `pc+937`: past the start nodes.
    pub on_path: bool,
    pub past_start: bool,
    /// `pc+828`: distance skater -> A; `pc+932`: inside the recorded lane.
    pub dist: f32,
    pub inside_lane: bool,
    /// `pc+868`: distance to A over the lane half width on that side (999 without a width).
    pub lateral: f32,
    /// `pc+820`: the B node of the previous tick (events up to it are handled).
    pub handled: u32,
    /// This tick's lost test (`sub_82468DD8`): the caller raises the reposition request.
    pub lost: bool,
    /// This tick's progress flag for the stuck tracker (`sub_8246C930`, b110): false when cursor B is ahead of the
    /// candidate, A is past the start and the raw projection advanced less than `progress_fraction` of A -> B.
    pub progress: bool,
}

impl PathFollow {
    /// A skater placed on `node` (spawn or respawn): on the path, A = B = the node.
    pub fn start(node: u32, tuning: &PathTuning) -> Self {
        Self { search_start: node, a: (node, 0.0), b: (node, 0.0), last: (node, 0.0), on_path: true, past_start: node > tuning.start_node, dist: 0.0, inside_lane: true, lateral: 0.0, handled: node, lost: false, progress: true }
    }

    /// One tick at skater position `pos`; `ratio` = shaped speed / recorded speed at A (1.0 unshaped); `speed` = the
    /// skater's signed speed along its heading (`pc+808`, `sub_8246BCF8`); `airborne` = the skater state byte `+438`
    /// (an air state 200..299 with no wheel touching, `sub_82DB6EC0`; b119): no on-path drop while airborne
    /// (`sub_8246DB50`).
    pub fn tick(&mut self, line: &ReplayLine, pos: Vec3, ratio: f32, speed: f32, airborne: bool, tuning: &PathTuning) {
        self.lost = false;
        self.progress = true;
        if line.nodes.len() < 2 {
            return;
        }
        let n = nearest_forward(line, pos, self.search_start, tuning.window);
        // A node advance runs the node-change handler instead of the lost test (`sub_8246D3C0`); the test reads the
        // previous tick's on-path flag.
        if n == self.search_start {
            self.lost = !self.on_path && speed < tuning.lost_max_speed && lost(line, n, pos, tuning.lost_radius);
        }
        self.search_start = n;
        // Projection -> A.
        let proj = project(line, n, pos);
        let p = point(line, proj);
        let mut cand = proj;
        if dist2(point(line, self.b), pos) < dist2(p, pos) + tuning.hysteresis {
            cand = self.b;
        }
        let b_ahead = self.b.0 > cand.0 || (self.b.0 == cand.0 && self.b.1 > cand.1);
        if b_ahead && (self.a.0 > 0 || self.a.1 > 0.1) {
            let dab = along(line, self.a, self.b);
            if dab > 0.0 && along(line, self.last, proj) < tuning.progress_fraction * dab {
                self.progress = false;
            }
            if dab > 0.0 && along(line, self.a, cand) < tuning.clamp_fraction * dab {
                let q = advance(line, self.a, tuning.clamp_fraction * dab);
                let qp = point(line, q);
                let horizontal = (qp[0] - pos[0]).powi(2) + (qp[2] - pos[2]).powi(2);
                if horizontal < tuning.clamp_horizontal * tuning.clamp_horizontal && (qp[1] - pos[1]).abs() < tuning.clamp_vertical {
                    cand = q;
                }
            }
        }
        self.last = proj;
        self.a = cand;
        self.search_start = self.search_start.max(self.a.0);
        // Distance and lane.
        let ap = point(line, self.a);
        let offset = [pos[0] - ap[0], pos[1] - ap[1], pos[2] - ap[2]];
        self.dist = dist2(ap, pos).sqrt();
        self.lateral = 0.0;
        self.inside_lane = if self.dist <= 0.0001 {
            true
        } else {
            let next = line.nodes.get(self.a.0 as usize + 1).unwrap_or(&line.nodes[self.a.0 as usize]);
            let dir = next.step;
            // Right of the direction (dir x up); width[0] = left, width[1] = right.
            let right = [dir[2], 0.0, -dir[0]];
            let side = usize::from(offset[0] * right[0] + offset[2] * right[2] >= 0.0);
            let w = f32::from(line.nodes[self.a.0 as usize].width[side]) * WIDTH_SCALE;
            self.lateral = if w > 0.0 { self.dist / w } else { 999.0 };
            self.dist < w
        };
        // Off path: rejoin commit.
        if !self.on_path && (self.a.0 > 0 || self.a.1 > 0.0001) && (self.inside_lane || self.dist < tuning.commit) {
            self.b = self.a;
            self.on_path = true;
        }
        if self.on_path && !self.past_start {
            self.past_start = self.a.0 > tuning.start_node || (self.a.0 == tuning.start_node && self.a.1 > 0.0001);
        }
        // Cursor B.
        self.handled = self.b.0;
        self.b = if ratio > 1e-5 { advance(line, self.a, ratio) } else { self.a };
        // On-path drop.
        if self.on_path && !airborne && !self.inside_lane && self.dist > tuning.drop {
            self.on_path = false;
        }
    }

    /// A skater placed again on the line by controller B's hand-back (`sub_824689C0`): the controller reset
    /// (`sub_82468AC8`: off the path, the start gate cleared) with the search at `node`; the normal rejoin commit puts
    /// it back on the path.
    pub fn reattach(node: u32, tuning: &PathTuning) -> Self {
        Self { on_path: false, past_start: false, ..Self::start(node, tuning) }
    }

    /// A line switch (junction `sub_8246BEE0` or line end `sub_8246C7F8`): the reset `sub_82468AC8` (b118) leaves the
    /// skater off the path with the search at `node`, A / B / the handled node cleared, and keeps the start gate
    /// (`pc+937` is only cleared by the full reset).
    pub fn switch_to(&self, node: u32) -> Self {
        Self { search_start: node, a: (0, 0.0), b: (0, 0.0), last: (0, 0.0), on_path: false, past_start: self.past_start, dist: 0.0, inside_lane: false, lateral: 999.0, handled: 0, lost: false, progress: true }
    }

    /// Recorded frames from cursor (`node`, `frame_in_segment`) forward to B (0 when B is not ahead).
    pub fn frames_to_b(&self, line: &ReplayLine, node: u32, frame_in_segment: u32) -> u64 {
        let seg = |i: u32| line.segment_frames(i) as f32;
        let from = (node, if seg(node) > 0.0 { frame_in_segment as f32 / seg(node) } else { 0.0 });
        along(line, from, self.b).max(0.0).floor() as u64
    }
}

/// Point on the line at (node, t).
/// `sub_82468DD8`'s distance part: farther than `r` from node `n` and from both of its recorded lane edge points (node
/// -/+ the lateral axis x the left / right width).
pub fn lost(line: &ReplayLine, n: u32, pos: Vec3, r: f32) -> bool {
    let node = &line.nodes[n as usize];
    let c = node.position;
    let far = |p: Vec3| dist2(p, pos) > r * r;
    if !far(c) {
        return false;
    }
    let next = line.nodes.get(n as usize + 1).unwrap_or(node);
    let dir = if next.step != [0.0; 3] { next.step } else { node.step };
    let len = dir[0].hypot(dir[2]);
    if len <= 1e-6 {
        return true;
    }
    let right = [dir[2] / len, 0.0, -dir[0] / len];
    let (wl, wr) = (f32::from(node.width[0]) * WIDTH_SCALE, f32::from(node.width[1]) * WIDTH_SCALE);
    far([c[0] - right[0] * wl, c[1], c[2] - right[2] * wl]) && far([c[0] + right[0] * wr, c[1], c[2] + right[2] * wr])
}

/// The node nearest `pos` over the whole line, first on a tie (`sub_82455108`, the hand-back's search node).
pub fn nearest_node(line: &ReplayLine, pos: Vec3) -> u32 {
    let mut best = (0u32, f32::INFINITY);
    for (i, node) in line.nodes.iter().enumerate() {
        let d = dist2(node.position, pos);
        if d < best.1 {
            best = (i as u32, d);
        }
    }
    best.0
}

/// The line's forward at cursor `c`: the path frames' forward rows of the two nodes, blended by the fraction (the
/// look-ahead frame's forward, `pc+304`, for the ground steer).
pub fn forward(line: &ReplayLine, c: (u32, f32)) -> Vec3 {
    let at = |i: usize| super::replay::rotate(super::replay::path_frame(&line.nodes[i]), [0.0, 0.0, 1.0]);
    let i = (c.0 as usize).min(line.nodes.len() - 1);
    let j = (i + 1).min(line.nodes.len() - 1);
    let (a, b) = (at(i), at(j));
    let v: Vec3 = std::array::from_fn(|k| a[k] + (b[k] - a[k]) * c.1);
    let l = dist2(v, [0.0; 3]).sqrt();
    if l > 1e-6 { v.map(|x| x / l) } else { a }
}

pub fn point(line: &ReplayLine, c: (u32, f32)) -> Vec3 {
    let i = (c.0 as usize).min(line.nodes.len() - 1);
    let a = line.nodes[i].position;
    match line.nodes.get(i + 1) {
        Some(b) => std::array::from_fn(|k| a[k] + (b.position[k] - a[k]) * c.1),
        None => a,
    }
}

/// Recorded frames from `from` to `to` along the line (negative when `to` is behind).
pub fn along(line: &ReplayLine, from: (u32, f32), to: (u32, f32)) -> f32 {
    let pos = |c: (u32, f32)| -> f32 { (0..c.0).map(|i| line.segment_frames(i) as f32).sum::<f32>() + c.1 * line.segment_frames(c.0) as f32 };
    pos(to) - pos(from)
}

/// `sub_82456628`: `amount` recorded frames ahead of `c`, clamped at the last node.
pub fn advance(line: &ReplayLine, c: (u32, f32), amount: f32) -> (u32, f32) {
    let last = line.nodes.len().saturating_sub(1) as u32;
    let (mut i, mut t) = c;
    let mut left = amount.max(0.0);
    for _ in 0..4096 {
        if i >= last {
            return (last, 0.0);
        }
        let seg = line.segment_frames(i) as f32;
        let rest = (1.0 - t) * seg;
        if seg > 0.0 && left < rest {
            return (i, t + left / seg);
        }
        left -= rest.max(0.0);
        i += 1;
        t = 0.0;
        if left <= 0.0 {
            return (i, 0.0);
        }
    }
    (i, t)
}

/// `sub_82455BB0`: the nearest node in start..=start+window (squared 3D distance; forward only).
pub fn nearest_forward(line: &ReplayLine, pos: Vec3, start: u32, window: u32) -> u32 {
    let last = line.nodes.len().saturating_sub(1) as u32;
    let start = start.min(last);
    (start..=(start + window).min(last)).min_by(|&a, &b| dist2(line.nodes[a as usize].position, pos).total_cmp(&dist2(line.nodes[b as usize].position, pos))).unwrap_or(start)
}

/// `sub_824563D8`: project onto the segments around node `n` (n-1 and n; segment 0 at the start, n-1 at the end).
pub fn project(line: &ReplayLine, n: u32, pos: Vec3) -> (u32, f32) {
    let last = line.nodes.len().saturating_sub(1) as u32;
    let seg_t = |i: u32| -> f32 {
        let a = line.nodes[i as usize].position;
        let b = line.nodes[i as usize + 1].position;
        let d = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let l2 = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
        if l2 <= 1e-12 {
            return 1.0;
        }
        ((pos[0] - a[0]) * d[0] + (pos[1] - a[1]) * d[1] + (pos[2] - a[2]) * d[2]) / l2
    };
    if n == 0 {
        return (0, seg_t(0).clamp(0.0, 1.0));
    }
    if n >= last {
        return (last - 1, seg_t(last - 1).clamp(0.0, 1.0));
    }
    let (tp, tn) = (seg_t(n - 1), seg_t(n));
    let inside = |t: f32| (0.0..=1.0).contains(&t);
    match (inside(tp), inside(tn)) {
        (true, true) => {
            let (pp, pn) = (point(line, (n - 1, tp)), point(line, (n, tn)));
            if dist2(pp, pos) <= dist2(pn, pos) { (n - 1, tp) } else { (n, tn) }
        }
        (true, false) => (n - 1, tp),
        (false, true) => (n, tn),
        (false, false) if tp < 0.0 => (n - 1, 0.0),
        (false, false) if tn > 1.0 => (n, 1.0),
        _ => (n, 0.0),
    }
}

fn dist2(a: Vec3, b: Vec3) -> f32 {
    (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::living_world::replay::ReplayNode;

    /// A straight line along +z: nodes every 1 m, 10 frames per segment (0.1 m per frame).
    fn line(n: usize) -> ReplayLine {
        let node = |i: usize| ReplayNode { position: [0.0, 0.0, i as f32], step: [0.0, 0.0, 0.1], board: [128; 4], skater: [128; 4], frames: if i == 0 { 0 } else { 10 }, event: 0, flags: 0, jump: None, width: [25, 25] };
        ReplayLine { id: [0; 16], flags: 0, skill: -1, nodes: (0..n).map(node).collect(), jumps: Vec::new(), groups: Vec::new() }
    }

    #[test]
    fn b_follows_the_skater_not_a_clock() {
        let l = line(40);
        let t = PathTuning::default();
        let mut f = PathFollow::start(0, &t);
        // The skater rolls at half the recorded speed (0.05 m per tick): A stays beside it (the 0.5 hysteresis
        // lets it lead by at most ~0.7 m), B one recorded frame ahead of A; a clock would run away from it.
        for k in 0..200 {
            let z = 0.05 * k as f32;
            f.tick(&l, [0.1, 0.0, z], 1.0, 0.0, false, &t);
            let a = point(&l, f.a)[2];
            assert!(a >= z - 0.05 && a <= z + 0.75, "tick {k}: A at {a}, skater at {z}");
            assert!((along(&l, f.a, f.b) - 1.0).abs() < 1e-3);
        }
        assert!(f.on_path);
    }

    #[test]
    fn off_the_lane_beyond_1_m_drops_the_path_and_rejoins_within_0_5_m() {
        let l = line(40);
        let t = PathTuning::default();
        let mut f = PathFollow::start(0, &t);
        let mut z = 0.0;
        let mut step = |f: &mut PathFollow, x: f32| {
            z += 0.1;
            f.tick(&l, [x, 0.0, z], 1.0, 0.0, false, &t);
        };
        for _ in 0..30 {
            step(&mut f, 0.0);
        }
        // Lane 0.5 m each side (25 / 50): 1.2 m to the side is outside and beyond 1 m.
        step(&mut f, 1.2);
        assert!(!f.on_path);
        step(&mut f, 0.8);
        assert!(!f.on_path, "0.8 m: outside the lane and over the 0.5 m commit");
        for _ in 0..3 {
            step(&mut f, 0.2);
        }
        assert!(f.on_path, "back inside the lane: rejoined ({f:?})");
    }

    #[test]
    fn the_search_only_moves_forward() {
        let l = line(20);
        let t = PathTuning::default();
        let mut f = PathFollow::start(10, &t);
        f.tick(&l, [0.0, 0.0, 2.0], 1.0, 0.0, false, &t);
        assert!(f.search_start >= 10 && f.a.0 >= 9, "{:?}", f);
    }

    #[test]
    fn frames_to_b_counts_recorded_frames_from_the_cursor() {
        let l = line(20);
        let t = PathTuning::default();
        let mut f = PathFollow::start(0, &t);
        f.tick(&l, [0.0, 0.0, 3.0], 1.0, 0.0, false, &t);
        // Cursor at node 1 frame 0: B at about 3.1 m = 21 frames on.
        assert_eq!(f.frames_to_b(&l, 1, 0), 21);
        assert_eq!(f.frames_to_b(&l, 5, 0), 0, "B behind the cursor");
    }

    #[test]
    fn lost_is_beyond_the_radius_from_the_node_and_both_lane_edges() {
        let mut l = line(10);
        l.nodes[4].width = [0, 0];
        assert!(!lost(&l, 4, [5.9, 0.0, 4.0], 6.0));
        assert!(lost(&l, 4, [6.1, 0.0, 4.0], 6.0));
        assert!(!lost(&l, 4, [4.4, 0.0, 4.0], 4.5));
        assert!(lost(&l, 4, [4.6, 0.0, 4.0], 4.5));
        // 7 m to the side but within 6 m of a 2 m lane edge point: not lost (either side).
        l.nodes[4].width = [100, 100];
        assert!(!lost(&l, 4, [7.0, 0.0, 4.0], 6.0));
        assert!(!lost(&l, 4, [-7.0, 0.0, 4.0], 6.0));
        assert!(lost(&l, 4, [8.5, 0.0, 4.0], 6.0));
    }

    #[test]
    fn the_lost_test_needs_an_off_path_tick_no_node_advance_and_low_speed() {
        let l = line(10);
        let t = PathTuning::default();
        let mut f = PathFollow::start(4, &t);
        // On path the previous tick: not lost yet, but dropped from the path.
        f.tick(&l, [9.0, 0.0, 4.0], 1.0, 5.0, false, &t);
        assert!(!f.lost && !f.on_path);
        // Off path, same search node, slow: lost; fast (>= 14 m/s) is not; reversing (negative) is.
        let mut fast = f;
        fast.tick(&l, [9.0, 0.0, 4.0], 1.0, 14.1, false, &t);
        assert!(!fast.lost);
        let mut back = f;
        back.tick(&l, [9.0, 0.0, 4.0], 1.0, -20.0, false, &t);
        assert!(back.lost);
        f.tick(&l, [9.0, 0.0, 4.0], 1.0, 13.9, false, &t);
        assert!(f.lost);
        // A node advance suppresses the test on that tick.
        let mut adv = f;
        adv.tick(&l, [9.0, 0.0, 6.0], 1.0, 0.0, false, &t);
        assert!(adv.search_start > 4 && !adv.lost);
    }

    #[test]
    fn a_hand_back_reattaches_off_path_at_the_nearest_node_and_rejoins_within_half_a_metre() {
        let l = line(10);
        let t = PathTuning::default();
        assert_eq!(nearest_node(&l, [0.3, 0.0, 7.2]), 7);
        assert_eq!(nearest_node(&l, [0.0, 0.0, 2.5]), 2, "first on a tie");
        let mut f = PathFollow::reattach(7, &t);
        assert!(!f.on_path && !f.past_start);
        f.tick(&l, [0.4, 0.0, 7.0], 1.0, 0.0, false, &t);
        assert!(f.on_path && f.past_start);
    }

    #[test]
    fn a_line_switch_leaves_the_path_keeps_the_start_gate_and_rejoins_by_the_commit() {
        let l = line(10);
        let t = PathTuning::default();
        let mut f = PathFollow::start(6, &t);
        f.tick(&l, [0.0, 0.0, 6.2], 1.0, 0.0, false, &t);
        assert!(f.on_path && f.past_start);
        let s = f.switch_to(2);
        assert!(!s.on_path && s.past_start);
        assert_eq!((s.search_start, s.a, s.b, s.handled), (2, (0, 0.0), (0, 0.0), 0));
        let mut s = s;
        s.tick(&l, [0.3, 0.0, 2.1], 1.0, 0.0, false, &t);
        assert!(s.on_path, "inside the lane / within 0.5 m commits");
    }

    #[test]
    fn airborne_the_path_is_not_dropped() {
        let l = line(10);
        let t = PathTuning::default();
        let mut air = PathFollow::start(4, &t);
        air.tick(&l, [3.0, 1.0, 4.0], 1.0, 5.0, true, &t);
        assert!(air.on_path);
        let mut ground = PathFollow::start(4, &t);
        ground.tick(&l, [3.0, 1.0, 4.0], 1.0, 5.0, false, &t);
        assert!(!ground.on_path);
    }
}
