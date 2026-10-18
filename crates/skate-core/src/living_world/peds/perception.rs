//! What a ped knows about and sees (doc 26 "Ped perception"). Retail (TU3, evidence only;
//! re-implemented; `.local/research/peds/b17-chase-rest-lost.md`, `b18-ped-perception-mood-ops.md`,
//! main checked the setup data):
//! - the brain keeps at most 5 perception entries (`brain+624`, 80 bytes each); a new entry
//!   (`82E42690`) evicts the oldest (largest age), starts with "just told" (flags 0x80), the
//!   given memory, age 0 and no mood suppression;
//! - each tick (`82E418E8`) memory counts down, age up, the suppression down; the vision test sets
//!   "visible" (0x40) and refreshes the last position and velocity while seen; "just told" is
//!   cleared; an entry whose target is gone, or whose memory ran out while not just told, is erased;
//! - the vision test (`82E27240`): seen within 1 m on the ground plane and 2 m in height (touch),
//!   or within the near range from the eye, or within the far range inside the view cone
//!   (`dot(dir, forward) > |cos(fov)|`) with a clear line of sight. Near / far = the mood event
//!   category `chasedpersondetectibilitytest` (20 / 30 m) times the ped type's perception scales
//!   (`D036` / `D238`, pedestrians 1.0); fov = `7435` (120 deg).

use super::super::Vec3;

/// One perception entry.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Perception {
    pub target: u64,
    /// Last seen position and velocity (`+0`, `+32`).
    pub position: Vec3,
    pub velocity: Vec3,
    /// Seconds the ped keeps knowing (`+56`), seconds known (`+60`), mood suppression (`+64`).
    pub memory: f32,
    pub age: f32,
    pub suppress: f32,
    /// Seen this tick (0x40) / just told about it (0x80).
    pub visible: bool,
    pub told: bool,
    /// Reactions about the target the ped has finished (`+68`): a want without flag 0x40 that was set and is
    /// unset adds one (`82E42A58`); the mood records' `7FF7` test reads it. Zero on a new entry.
    pub reacted: u32,
}

/// At most this many entries (`82E42690`).
pub const CAPACITY: usize = 5;

/// The brain's perception list (host-owned plain data).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Perceptions {
    pub entries: Vec<Perception>,
}

impl Perceptions {
    pub fn get(&self, target: u64) -> Option<&Perception> {
        self.entries.iter().find(|e| e.target == target)
    }
    pub fn get_mut(&mut self, target: u64) -> Option<&mut Perception> {
        self.entries.iter_mut().find(|e| e.target == target)
    }
    /// `82E42868`: keep `target`'s entry known for at least `memory` seconds, creating it (just told) when missing.
    pub fn know(&mut self, target: u64, memory: f32) {
        if let Some(e) = self.get_mut(target) {
            e.memory = e.memory.max(memory);
            return;
        }
        if self.entries.len() >= CAPACITY {
            if let Some(i) = self.entries.iter().enumerate().max_by(|a, b| a.1.age.total_cmp(&b.1.age)).map(|(i, _)| i) {
                self.entries.remove(i);
            }
        }
        self.entries.push(Perception { target, position: [0.0; 3], velocity: [0.0; 3], memory, age: 0.0, suppress: 0.0, visible: false, told: true, reacted: 0 });
    }
    /// `82E3D240`: mood events about `target` are blocked for at least `seconds`.
    pub fn suppress(&mut self, target: u64, seconds: f32) {
        if let Some(e) = self.get_mut(target) {
            e.suppress = e.suppress.max(seconds);
        }
    }
    pub fn suppressed(&self, target: u64) -> bool {
        self.get(target).is_some_and(|e| e.suppress > 0.0)
    }
    pub fn forget(&mut self, target: u64) {
        self.entries.retain(|e| e.target != target);
    }
    /// `82E418E8`: one tick. `look(target)` answers where the target is now (`None` = gone) and
    /// whether the vision test passes.
    pub fn tick(&mut self, dt: f32, look: &dyn Fn(u64) -> Option<(Vec3, Vec3, bool)>) {
        self.entries.retain_mut(|e| {
            let Some((position, velocity, seen)) = look(e.target) else { return false };
            e.memory -= dt;
            e.age += dt;
            e.suppress = (e.suppress - dt).max(0.0);
            e.visible = seen;
            if seen || e.told {
                e.position = position;
                e.velocity = velocity;
            }
            let keep = e.memory > 0.0 || e.told;
            e.told = false;
            keep
        });
    }
}

/// The vision test's ranges for one ped type.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sight {
    pub near: f32,
    pub far: f32,
    /// `|cos(fov)|`.
    pub cone_cos: f32,
}

impl Sight {
    /// Category ranges times the ped type's scales; fov in degrees.
    pub fn new(near: f32, far: f32, near_scale: f32, far_scale: f32, fov_deg: f32) -> Self {
        Self { near: near * near_scale, far: far * far_scale, cone_cos: fov_deg.to_radians().cos().abs() }
    }
}

/// `82E27240`: does a ped at `ped` (feet), looking from `eye` along `forward` (unit), see
/// `target`? `line_of_sight` is asked only for the cone test.
pub fn sees(ped: Vec3, eye: Vec3, forward: Vec3, target: Vec3, sight: &Sight, line_of_sight: &dyn Fn(Vec3, Vec3) -> bool) -> bool {
    let flat = ((target[0] - ped[0]).powi(2) + (target[2] - ped[2]).powi(2)).sqrt();
    if flat < 1.0 && (target[1] - ped[1]).abs() < 2.0 {
        return true;
    }
    let d = [target[0] - eye[0], target[1] - eye[1], target[2] - eye[2]];
    let dist = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
    if dist <= sight.near {
        return true;
    }
    if dist > sight.far || dist < 1e-6 {
        return false;
    }
    let dot = (d[0] * forward[0] + d[1] * forward[1] + d[2] * forward[2]) / dist;
    dot > sight.cone_cos && line_of_sight(eye, target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_follow_the_retail_list_rules() {
        let mut p = Perceptions::default();
        for t in 0..5 {
            p.know(t, 30.0);
            p.entries.last_mut().unwrap().age = t as f32;
        }
        p.know(9, 30.0);
        assert!(p.get(4).is_none(), "the oldest entry is evicted");
        assert!(p.get(9).unwrap().told);
        p.suppress(9, 5.5);
        assert!(p.suppressed(9));
        // A tick refreshes a just-told entry's position, then clears the flag.
        p.tick(1.0, &|t| Some(([t as f32, 0.0, 0.0], [0.0; 3], false)));
        let e = *p.get(9).unwrap();
        assert_eq!((e.position[0], e.told, e.memory, e.suppress), (9.0, false, 29.0, 4.5));
        // A gone target is erased; memory running out erases too.
        p.tick(1.0, &|t| (t != 0).then_some(([0.0; 3], [0.0; 3], false)));
        assert!(p.get(0).is_none());
        p.tick(30.0, &|_| Some(([0.0; 3], [0.0; 3], true)));
        assert!(p.entries.is_empty());
    }

    #[test]
    fn vision_has_touch_near_and_a_cone_with_line_of_sight() {
        let s = Sight::new(20.0, 30.0, 1.0, 1.0, 120.0);
        let eye = [0.0, 1.6, 0.0];
        let fwd = [0.0, 0.0, 1.0];
        let clear = |_: Vec3, _: Vec3| true;
        let blocked = |_: Vec3, _: Vec3| false;
        // Behind but within 20 m: the near sphere.
        assert!(sees([0.0; 3], eye, fwd, [0.0, 0.0, -15.0], &s, &blocked));
        // 25 m ahead: the cone, only with a clear line.
        assert!(sees([0.0; 3], eye, fwd, [0.0, 0.0, 25.0], &s, &clear));
        assert!(!sees([0.0; 3], eye, fwd, [0.0, 0.0, 25.0], &s, &blocked));
        // 25 m behind: outside the 60 deg half-angle.
        assert!(!sees([0.0; 3], eye, fwd, [0.0, 0.0, -25.0], &s, &clear));
        // Touch.
        assert!(sees([0.0; 3], [0.0, 100.0, 0.0], fwd, [0.5, 1.0, 0.0], &Sight::new(0.0, 0.0, 1.0, 1.0, 120.0), &blocked));
    }
}
