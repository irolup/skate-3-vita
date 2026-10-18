//! Dynamic objects (DMOs: benches, bins, cones, ...) streamed by the living-world census, as retail does
//! (research b97; doc 27 "DMO streaming").
//!
//! Per census pass (`sub_826B7980`) [code]:
//! 1. the circle from the `dynamicobjects` range record (same lerp as peds; retail data has no speed keys and no
//!    forward offset for DMOs);
//! 2. cull (`sub_826BAD98`): a live, cullable DMO whose HORIZONTAL distance to the centre exceeds the cull radius goes,
//!    with its whole linked group (`sub_82C52600`); then, once per pass, when the pool holds `cap` (49) or more live
//!    objects, the front of the eviction queue goes;
//! 3. spawn (`sub_826B9D58`): placements within the outer radius, expanded by touching neighbours within 20 m so
//!    clusters come together, up to `budget` (49, 100 in fill mode) per pass; with the pool full a placement spawns
//!    only when its score beats the eviction front's, which is evicted for it.
//!
//! The score (`sub_82C4A130` placements / `sub_82C55160` live) weighs priority, a count and the distance, with the
//! five `livingworld.dynamicobjects` values (2.5 in front of the camera, 2.0 for the keep flag, 250, 300, 450).
//!
//! NOT RETAIL YET: the score's count `n` (`sub_82C503F0`, meaning open) is 1; the overlap blocker (a live DMO in the
//! placement's volume) is not modelled; touching (`factor 0.5`) is bounding spheres; the eviction queue is ordered by
//! the live score (its insert point is not read); multiple observers are ours (retail runs this for one player).

use super::census::CensusRange;
use super::{Observer, Vec3};
use std::collections::BTreeSet;

/// The five `livingworld.dynamicobjects` score weights [data, b97 §1.4].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DmoWeights {
    /// `Hash_74FE9111054663C9` 2.5: in front of the camera.
    pub in_front: f32,
    /// `Hash_883771F11981A45D` 2.0: the keep flag (placement flags 0x10).
    pub keep_flag: f32,
    /// `Hash_16ADB1DBA4AA77D1` 250: priority.
    pub priority: f32,
    /// `Hash_C891B78CB013B593` 300: the count `n`.
    pub count: f32,
    /// `Hash_ABD7EA6B3B93846E` 450: distance.
    pub distance: f32,
}

impl Default for DmoWeights {
    fn default() -> Self {
        Self { in_front: 2.5, keep_flag: 2.0, priority: 250.0, count: 300.0, distance: 450.0 }
    }
}

/// Streaming values with retail defaults (the cap, budgets and group values are code constants in retail).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DmoStreamSettings {
    /// `livingworld_census_ranges` `dynamicobjects` [data]: 0 / 90 / 100 m.
    pub range: CensusRange,
    pub weights: DmoWeights,
    /// Live objects before the eviction (`subfic 49`, b91 / b97) [code].
    pub cap: usize,
    /// Spawns per pass: 49, or 100 in fill mode [code].
    pub budget: usize,
    pub fill_budget: usize,
    /// Group expansion: neighbours within this distance (`0x820996EC` 20 m) that touch.
    pub group_radius: f32,
}

impl Default for DmoStreamSettings {
    fn default() -> Self {
        let c = super::census::CensusCircle { spawn_inner: 0.0, spawn_outer: 90.0, cull: 100.0, forward_offset: 0.0, speed_kmh: 0.0 };
        Self { range: CensusRange { slow: c, fast: c }, weights: DmoWeights::default(), cap: 49, budget: 49, fill_budget: 100, group_radius: 20.0 }
    }
}

/// One authored placement (`0x00EB001D DMODATA` record) as the census sees it.
#[derive(Clone, Debug, PartialEq)]
pub struct DmoPlacement {
    /// The map prop id (stable across machines).
    pub id: u32,
    /// Where it spawns (placement record +0; the host keeps it current).
    pub position: Vec3,
    /// Bounding sphere radius, m (touching test).
    pub radius: f32,
    /// The type's `livingworld_dynamicobject_priority` value; `None` = `keepalways` (0xFFFFFFFF).
    pub priority: Option<u32>,
    /// Placement flags 0x10 (keep: never scored below `keepalways`).
    pub keep: bool,
    /// Placement flags 0x80 (streamed by the census) and the type's cullable bit.
    pub streamable: bool,
}

/// The camera position, its forward and the reference position the score reads (`[0x83085494]` vfuncs +124 / +136
/// / +132; camera vs player is [inferred]).
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct DmoView {
    pub camera: Vec3,
    pub forward: Vec3,
    pub reference: Vec3,
}

/// What a pass decided (serialisable; the host applies it to the prop bodies).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DmoDecision {
    Spawn(u32),
    /// Beyond the cull radius (with its group).
    Cull(u32),
    /// The pool was full: the lowest-scoring live object made room.
    Evict(u32),
}

fn flat_dist(a: Vec3, b: Vec3) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

/// The score vector `s[1..=4]` of a placement or live object [code `sub_82C4A130`]; compare with [`score_value`].
pub fn score(p: &DmoPlacement, view: &DmoView, w: &DmoWeights) -> [f32; 4] {
    let to = [p.position[0] - view.camera[0], p.position[1] - view.camera[1], p.position[2] - view.camera[2]];
    let in_front = to[0] * view.forward[0] + to[1] * view.forward[1] + to[2] * view.forward[2] > 0.0;
    let k_view = if in_front { w.in_front } else { 1.0 };
    let k_flag = if p.keep { w.keep_flag } else { 1.0 };
    let s1 = match (p.keep, p.priority) {
        (false, Some(prio)) => w.priority * prio as f32 * k_flag,
        _ => 2f32.powi(31),
    };
    let n = 1.0;
    let s2 = w.count * -n * k_flag;
    let d = {
        let r = view.reference;
        ((p.position[0] - r[0]).powi(2) + (p.position[1] - r[1]).powi(2) + (p.position[2] - r[2]).powi(2)).sqrt()
    };
    let s3 = w.distance * -d / (k_flag * k_view * k_view);
    [s1, s2, s3, k_view]
}

/// `(s1 + s2 + s3) * s4`.
pub fn score_value(s: [f32; 4]) -> f32 {
    (s[0] + s[1] + s[2]) * s[3]
}

/// The census's DMO state: the placements, which are live, and the touching groups.
#[derive(Clone, Debug, Default)]
pub struct DmoStream {
    pub placements: Vec<DmoPlacement>,
    pub live: BTreeSet<u32>,
    /// Group index per placement (touching clusters).
    group: Vec<usize>,
    /// skate_vita (performance): each group's placements in ascending order, by group index.
    members: Vec<Vec<usize>>,
}

/// skate_vita: a key whose unsigned order is `f32::total_cmp`'s (std's transform).
fn total_key(x: f32) -> u32 {
    let mut bits = x.to_bits() as i32;
    bits ^= (((bits >> 31) as u32) >> 1) as i32;
    (bits as u32) ^ 0x8000_0000
}

impl DmoStream {
    pub fn new(placements: Vec<DmoPlacement>, settings: &DmoStreamSettings) -> Self {
        let n = placements.len();
        let mut group: Vec<usize> = (0..n).collect();
        fn root(g: &mut [usize], mut i: usize) -> usize {
            while g[i] != i {
                g[i] = g[g[i]];
                i = g[i];
            }
            i
        }
        for i in 0..n {
            for j in i + 1..n {
                let (a, b) = (&placements[i], &placements[j]);
                let d = flat_dist(a.position, b.position);
                if d <= settings.group_radius && d <= a.radius + b.radius {
                    let (ri, rj) = (root(&mut group, i), root(&mut group, j));
                    group[ri] = rj;
                }
            }
        }
        let group: Vec<usize> = (0..n).map(|i| root(&mut group, i)).collect();
        let mut members = vec![Vec::new(); n];
        for (i, &g) in group.iter().enumerate() {
            members[g].push(i);
        }
        Self { placements, live: BTreeSet::new(), group, members }
    }

    fn index(&self, id: u32) -> Option<usize> {
        self.placements.iter().position(|p| p.id == id)
    }

    /// One census pass for `observers` (each with its score view). `exempt` ids (held or carried props) are never
    /// culled or evicted. `fill` uses the larger spawn budget (a map load).
    pub fn step(&mut self, observers: &[(Observer, DmoView)], settings: &DmoStreamSettings, fill: bool, exempt: &dyn Fn(u32) -> bool) -> Vec<DmoDecision> {
        let mut out = Vec::new();
        let circles: Vec<_> = observers.iter().map(|(o, v)| (settings.range.around(o), *v)).filter(|((c, centre), _)| c.cull.is_finite() && centre.iter().all(|x| x.is_finite())).collect();
        if circles.is_empty() {
            return out;
        }
        // Cull: beyond every observer's circle, with the whole group.
        let beyond = |p: &DmoPlacement| circles.iter().all(|((c, centre), _)| flat_dist(p.position, *centre) > c.cull);
        let mut cull_groups = BTreeSet::new();
        for (i, p) in self.placements.iter().enumerate() {
            if self.live.contains(&p.id) && p.streamable && !exempt(p.id) && beyond(p) {
                cull_groups.insert(self.group[i]);
            }
        }
        for (i, p) in self.placements.iter().enumerate() {
            if cull_groups.contains(&self.group[i]) && self.live.contains(&p.id) && !exempt(p.id) {
                out.push(DmoDecision::Cull(p.id));
            }
        }
        for d in &out {
            if let DmoDecision::Cull(id) = d {
                self.live.remove(id);
            }
        }
        // The best score over the observers' views.
        let best = |p: &DmoPlacement| circles.iter().map(|(_, v)| score_value(score(p, v, &settings.weights))).fold(f32::MIN, f32::max);
        // skate_vita (performance): a placement's score at most once per pass, when first needed (it reads only
        // the views), and the live, non-exempt objects ordered by (score in `total_cmp` order, id): the eviction
        // front is the first, the same minimum as scanning them; upstream rescanned every placement per spawn attempt.
        let mut memo: Vec<Option<f32>> = vec![None; self.placements.len()];
        let placements = &self.placements;
        let mut score = |i: usize| *memo[i].get_or_insert_with(|| best(&placements[i]));
        let mut queue: BTreeSet<(u32, u32, usize)> = BTreeSet::new();
        for (i, p) in placements.iter().enumerate() {
            if self.live.contains(&p.id) && !exempt(p.id) {
                queue.insert((total_key(score(i)), p.id, i));
            }
        }
        // One eviction per pass while the pool is full.
        if self.live.len() >= settings.cap {
            if let Some((_, id, _)) = queue.pop_first() {
                self.live.remove(&id);
                out.push(DmoDecision::Evict(id));
            }
        }
        // Spawn per observer: candidates within the outer radius, nearest first, plus their touching groups.
        let mut budget = if fill { settings.fill_budget } else { settings.budget };
        for ((circle, centre), _) in &circles {
            let mut cands: Vec<(f32, usize)> = self
                .placements
                .iter()
                .enumerate()
                .filter(|(_, p)| p.streamable && !self.live.contains(&p.id))
                .map(|(i, p)| (flat_dist(p.position, *centre), i))
                .filter(|(d, _)| *d <= circle.spawn_outer && (fill || *d >= circle.spawn_inner))
                .collect();
            cands.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
            // skate_vita (performance): the group's members in ascending order and a seen flag, the same list
            // as scanning every placement and the list for each candidate.
            let mut list: Vec<usize> = Vec::new();
            let mut seen = vec![false; self.placements.len()];
            for &(_, i) in &cands {
                for &j in &self.members[self.group[i]] {
                    let p = &self.placements[j];
                    let inside = flat_dist(p.position, *centre) <= circle.cull;
                    if inside && !seen[j] && p.streamable {
                        seen[j] = true;
                        list.push(j);
                    }
                }
            }
            for i in list {
                if budget == 0 {
                    break;
                }
                let p = &self.placements[i];
                if self.live.contains(&p.id) {
                    continue;
                }
                if self.live.len() >= settings.cap {
                    match queue.first().copied() {
                        Some((key, victim, v)) if score(i) > score(v) => {
                            queue.remove(&(key, victim, v));
                            self.live.remove(&victim);
                            out.push(DmoDecision::Evict(victim));
                        }
                        _ => continue,
                    }
                }
                self.live.insert(p.id);
                if !exempt(p.id) {
                    queue.insert((total_key(score(i)), p.id, i));
                }
                out.push(DmoDecision::Spawn(p.id));
                budget -= 1;
            }
        }
        out
    }

    /// The host moved a placement's object (keep where it was left): the next spawn uses this position.
    pub fn set_position(&mut self, id: u32, position: Vec3) {
        if let Some(i) = self.index(id) {
            self.placements[i].position = position;
        }
    }
}

#[cfg(test)]
#[path = "dmo_tests.rs"]
mod tests;
