//! Dynamic objects (props, mod bodies) as ped navigation obstacles (fix 11): cut out while at rest.
//! Retail mechanism and our stand-ins: docs/hails-additions/26f-living-world-peds.md#ped-navigation-obstacles

use std::collections::BTreeMap;

use crate::living_world::Vec3;

use super::nav::NavMesh;

/// Obstacle rules: retail values as defaults (a mod overrides any of them).
#[derive(Clone, Debug, PartialEq)]
pub struct ObstacleParams {
    /// Dynamic objects are obstacles at all (retail: yes, DynamicObject slot +68 = 1).
    pub enabled: bool,
    /// Smallest half extent of a cut [code `0x82099280` = 0.2 in `sub_82C477B0`].
    pub min_half_extent: f32,
    /// Above this speed (m/s) an object is not cut (it moves) [code `0x82181B90` = 0.4].
    pub moving_speed: f32,
    /// Re-cut once the object moved more than this x its smallest half extent from the cut
    /// [code `0x820C6D98` = 0.25].
    pub recut_fraction: f32,
    /// Clearance added to the agent radius for detour corners, metres (ours).
    pub detour_margin: f32,
    /// Detour corners one path may get (ours, a cost cap).
    pub max_detours: usize,
    /// A path end inside a cut moves to the nearest free point when that is at most this far (xz), metres: retail
    /// snaps a bot destination onto the cut mesh within a 0.5 m window (`82C46348` / `82C460E0`, radius
    /// `0x8209975C` = 0.5, research b302), so a plugin waypoint just in front of its own prop is reached.
    pub target_snap: f32,
    /// Objects whose box top (after the 0.2 m minimum) is no higher than this above the ped's feet
    /// are stepped over (ours; 0 = every box counts, as retail's ground cut has no height test).
    pub step_height: f32,
    /// A held prop (Move Object) or an attached mod body stays an obstacle (retail: yes, the hold
    /// does not set the obstacle-off word, see the module docs). `false` = the earlier port's rule
    /// (held objects are ignored), kept as a mod option.
    pub held_is_obstacle: bool,
    /// A moving object is solid for a ped's step (NOT RETAIL YET: NavPower moving avoider 82E99998): docs/hails-additions/26f-living-world-peds.md#moving-obstacle-solid
    pub moving_solid: bool,
    /// Player skaters are obstacles for peds (research b135: the skater's obstacle interface `0x82300910` slot
    /// +104 returns 1, so `sub_82C477B0` always takes the moving-avoider branch, at any speed, standing included).
    pub skaters: bool,
    /// A skater obstacle's half extents (x, y, z), m. NOT RETAIL YET: retail's avoider is `{a, 0.2a, 0.2a}`,
    /// `a = 0.25 R` with `R` a planner-wide value not read yet; this is a body-sized stand-in.
    pub skater_half_extents: Vec3,
    /// Contact slide round a moving body (`82E9D0C0`, b698): tangential step as this fraction of the push
    /// [code `0x821161AC` = 0.7]; 0 = the face clamp only.
    pub contact_slide: f32,
    /// The slide needs the step heading into the body: `dot(dir, n)` below this [code `0x822F862C` = -0.3].
    pub contact_slide_dot: f32,
    /// Predictive side-step round a moving body (`82E9CE90` / `82E9CBD0`, b698 / b699): look-ahead, s [code
    /// `0x82063B08` = 3.0]; 0 = off.
    pub avoid_horizon: f32,
    /// Contact ray against this x the combined radius [code `0x820ED5E8` = 0.99].
    pub avoid_ray_scale: f32,
    /// Side-step acceleration base [code `0x82063B08` = 3.0] times the NavPower global scale [data `0x82099484` = 1.3,
    /// set by `8292E230`], m/s^2.
    pub avoid_accel: f32,
    pub avoid_scale: f32,
    /// Head-on below this velocity dot, same direction above `avoid_crossing_dot` [code `0x822F8F80` = -0.7,
    /// `0x821161AC` = 0.7].
    pub avoid_head_on_dot: f32,
    pub avoid_crossing_dot: f32,
    /// A moving avoider's extents `{a, t a, t a}`, `a = avoid_scale x avoider_extent`, summed into its radius whatever
    /// the box (`82C477B0`: [code `0x820C6D98` = 0.25, `0x82099280` = 0.2], sum 0.455 at scale 1.3).
    pub avoider_extent: f32,
    pub avoider_extent_tail: f32,
    /// The path turn term that pulls a ped's sideways velocity back every controller step (`82E9DF70`, research
    /// b716): m/s^2 (`settings+0x2C`, retail 40).
    pub path_turn_accel: f32,
    /// Full turn from this heading error, degrees (5); below it the turn scales with the error.
    pub path_turn_full_deg: f32,
    /// Above this error the turn is boosted (10 degrees) by `path_turn_boost_rate` per radian (4.5837) up to
    /// `path_turn_boost_max` times (5); no turn at or above `path_turn_max_deg` (120).
    pub path_turn_boost_deg: f32,
    pub path_turn_boost_rate: f32,
    pub path_turn_boost_max: f32,
    pub path_turn_max_deg: f32,
    /// The turn never exceeds the centripetal limit `v^2 / (2 r)` (`settings+0x08`, 0.35 m).
    pub path_turn_radius: f32,
    /// The controller's fixed step (`f1`, 1 / 30 s): the turn removes at most the sideways speed over one step.
    pub controller_step: f32,
    /// The contact slide's speed tests (`82E9D0C0`, research b717 / b718): a ped at or below this fraction (0.5,
    /// `0x8209975C`) of its path speed slides toward its steer target, faster along its velocity; and a body moving
    /// away along that direction at this fraction of the path speed or more gets no slide (push-out only).
    pub contact_slow_fraction: f32,
    /// The ped's path speed `S` (`settings+0` x agent `+0x14C` x `+0x144`): 2.0 m/s for walking peds (1.0 on some legs;
    /// run / flee not traced).
    pub contact_max_speed: f32,
    /// Braking for a blocker at the path end (`82E9FC20` / `82E9F9F0`, research b716): within `brake_goal_radius` of
    /// the steer target (2.0 m; retail sets it per path leg, 0.75 / 0.31 also seen) a moving body ahead closer than the
    /// stopping distance `v^2 / (2 brake_decel)` (80 m/s^2) stops the ped and skips its side-step. The ray tests
    /// `brake_body_radius` (1.225 m = settings +0x08 + 0x0C + 0x10) plus the body's avoider extents; bodies moving away
    /// at `brake_away_fraction` (0.25) of the ped's speed or more are left out. 0 decel = off.
    pub brake_decel: f32,
    pub brake_goal_radius: f32,
    pub brake_body_radius: f32,
    pub brake_away_fraction: f32,
}

/// Obstacle ids of player skaters (`SKATER_OBSTACLE_BASE + player index`); always moving avoiders.
pub const SKATER_OBSTACLE_BASE: u64 = 1 << 41;

/// What a ped's step does not collide with: one object (its plugin prop) and, with its SkaterAvoidance nav
/// modifier off (`ChangeNavModifierSetting` index 2: chasers, plugin bodies, spectators), the skaters.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Ignore {
    pub id: Option<u64>,
    pub skaters: bool,
}

impl Ignore {
    fn skips(&self, id: u64) -> bool {
        Some(id) == self.id || (self.skaters && id >= SKATER_OBSTACLE_BASE)
    }
}

impl Default for ObstacleParams {
    fn default() -> Self {
        Self { enabled: true, min_half_extent: 0.2, moving_speed: 0.4, recut_fraction: 0.25, detour_margin: 0.1, max_detours: 8, target_snap: 0.5, step_height: 0.0, held_is_obstacle: true, moving_solid: true, skaters: true, skater_half_extents: [0.3, 0.9, 0.3], contact_slide: 0.7, contact_slide_dot: -0.3, avoid_horizon: 3.0, avoid_ray_scale: 0.99, avoid_accel: 3.0, avoid_scale: 1.3, avoid_head_on_dot: -0.7, avoid_crossing_dot: 0.7, avoider_extent: 0.25, avoider_extent_tail: 0.2, path_turn_accel: 40.0, path_turn_full_deg: 5.0, path_turn_boost_deg: 10.0, path_turn_boost_rate: 4.5837, path_turn_boost_max: 5.0, path_turn_max_deg: 120.0, path_turn_radius: 0.35, controller_step: 1.0 / 30.0, contact_slow_fraction: 0.5, contact_max_speed: 2.0, brake_decel: 80.0, brake_goal_radius: 2.0, brake_body_radius: 1.225, brake_away_fraction: 0.25 }
    }
}

/// One dynamic object as the game sees it this tick.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ObstacleInput {
    /// Stable id (props: their prop id; mod bodies: a separate range, see the game host).
    pub id: u64,
    /// Box centre (world).
    pub center: Vec3,
    /// Box axes (world, unit columns) and half extents along them.
    pub axes: [Vec3; 3],
    pub half_extents: Vec3,
    pub velocity: Vec3,
    /// Not an obstacle right now (retail state word `+144+4252 == 1`, meaning not decoded).
    pub inactive: bool,
    /// Held by the player (Move Object) or an attached mod body; an obstacle unless
    /// [`ObstacleParams::held_is_obstacle`] is off.
    pub held: bool,
}

/// A box's ground footprint: an oriented rectangle in xz plus its height span.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Footprint {
    pub center: [f32; 2],
    /// Unit axis `u` in xz (`v` = `u` turned +90 degrees).
    pub u: [f32; 2],
    pub half: [f32; 2],
    pub y_min: f32,
    pub y_max: f32,
}

impl Footprint {
    /// The rectangle covering the box's projection onto the ground.
    pub fn of(input: &ObstacleInput, min_half: f32) -> Self {
        let h = input.half_extents.map(|v| v.abs().max(min_half));
        // u: the longest ground projection among the axes (ties: lowest index).
        let mut u = [1.0f32, 0.0];
        let mut best = 1e-6f32;
        for (k, a) in input.axes.iter().enumerate() {
            let l = (a[0] * a[0] + a[2] * a[2]).sqrt() * h[k];
            if l > best + 1e-6 {
                best = l;
                let n = (a[0] * a[0] + a[2] * a[2]).sqrt();
                u = [a[0] / n, a[2] / n];
            }
        }
        let v = [-u[1], u[0]];
        let mut half = [0.0f32; 2];
        let mut ey = 0.0f32;
        for (k, a) in input.axes.iter().enumerate() {
            half[0] += (a[0] * u[0] + a[2] * u[1]).abs() * h[k];
            half[1] += (a[0] * v[0] + a[2] * v[1]).abs() * h[k];
            ey += a[1].abs() * h[k];
        }
        Self { center: [input.center[0], input.center[2]], u, half, y_min: input.center[1] - ey, y_max: input.center[1] + ey }
    }

    fn local(&self, p: [f32; 2]) -> [f32; 2] {
        let d = [p[0] - self.center[0], p[1] - self.center[1]];
        [d[0] * self.u[0] + d[1] * self.u[1], -d[0] * self.u[1] + d[1] * self.u[0]]
    }

    fn world(&self, l: [f32; 2]) -> [f32; 2] {
        [self.center[0] + l[0] * self.u[0] - l[1] * self.u[1], self.center[1] + l[0] * self.u[1] + l[1] * self.u[0]]
    }

    /// Whether this footprint is in the way of a ped whose feet are at height `y`.
    pub fn applies(&self, y: f32, step_height: f32) -> bool {
        self.y_max > y + step_height && self.y_min < y + 2.0
    }

    /// Signed distance-like test: the point is inside the rectangle grown by `grow`.
    pub fn contains(&self, p: Vec3, grow: f32) -> bool {
        let l = self.local([p[0], p[2]]);
        l[0].abs() < self.half[0] + grow && l[1].abs() < self.half[1] + grow
    }

    /// Entry fraction of segment `a`-`b` into the rectangle grown by `grow` (slab test), if it
    /// enters (a start inside counts as 0).
    pub fn segment_hit(&self, a: Vec3, b: Vec3, grow: f32) -> Option<f32> {
        let (la, lb) = (self.local([a[0], a[2]]), self.local([b[0], b[2]]));
        let (mut t0, mut t1) = (0.0f32, 1.0f32);
        for k in 0..2 {
            let ext = self.half[k] + grow;
            let d = lb[k] - la[k];
            if d.abs() < 1e-9 {
                if la[k].abs() >= ext {
                    return None;
                }
            } else {
                let (mut e, mut x) = ((-ext - la[k]) / d, (ext - la[k]) / d);
                if e > x {
                    std::mem::swap(&mut e, &mut x);
                }
                t0 = t0.max(e);
                t1 = t1.min(x);
                if t0 >= t1 {
                    return None;
                }
            }
        }
        Some(t0)
    }

    /// The four corners of the rectangle grown by `grow` (counter-clockwise from +u +v).
    pub fn corners(&self, grow: f32) -> [[f32; 2]; 4] {
        let (x, z) = (self.half[0] + grow, self.half[1] + grow);
        [self.world([x, z]), self.world([-x, z]), self.world([-x, -z]), self.world([x, -z])]
    }
}

/// Per-object obstacle state (serialisable plain data).
#[derive(Clone, Debug, PartialEq)]
pub struct ObstacleState {
    /// The footprint cut into the walkable area (retail `+74` set), if any.
    pub cut: Option<Footprint>,
    /// Box centre when it was cut (retail `+48`).
    pub cut_at: Vec3,
    /// Footprint now (cut or not).
    pub now: Footprint,
    /// Moving faster than [`ObstacleParams::moving_speed`] (retail: moving avoider instead).
    pub moving: bool,
    pub inactive: bool,
    /// Held this tick (diagnostics; an obstacle unless `held_is_obstacle` is off).
    pub held: bool,
    /// Ground velocity (x, z) this tick, m/s.
    pub velocity: [f32; 2],
}

const CELL: f32 = 4.0;

/// All dynamic obstacles of the map, by stable id.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NavObstacles {
    pub params: ObstacleParams,
    pub states: BTreeMap<u64, ObstacleState>,
    /// Changes whenever a cut appears, moves or goes (paths re-check their legs then).
    pub version: u64,
    grid: BTreeMap<(i32, i32), Vec<u64>>,
    /// Active objects without a cut (moving), for the step check.
    loose: Vec<u64>,
}

fn cell_of(x: f32) -> i32 {
    (x / CELL).floor() as i32
}


/// The ped's own motion for the contact slide's tests: its velocity (x, z) and its steer target (x, z).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Contact {
    pub velocity: Option<[f32; 2]>,
    pub target: Option<[f32; 2]>,
}

impl NavObstacles {
    pub fn new(params: ObstacleParams) -> Self {
        Self { params, ..Self::default() }
    }

    /// Change the rules; every cut is redone on the next [`Self::update`].
    pub fn set_params(&mut self, params: ObstacleParams) {
        if params != self.params {
            self.params = params;
            self.states.clear();
            self.grid.clear();
            self.version += 1;
        }
    }

    /// Retail's per-object update (`sub_82595298` -> `sub_82C477B0`) for every object present this
    /// tick; objects not listed are removed. Returns whether any cut changed.
    pub fn update(&mut self, inputs: &[ObstacleInput]) -> bool {
        let p = self.params.clone();
        let mut changed = false;
        let mut seen = std::collections::BTreeSet::new();
        let mut sorted: Vec<&ObstacleInput> = inputs.iter().collect();
        sorted.sort_by_key(|i| i.id);
        for input in sorted {
            if !seen.insert(input.id) {
                continue;
            }
            let now = Footprint::of(input, p.min_half_extent);
            let speed = (input.velocity[0].powi(2) + input.velocity[1].powi(2) + input.velocity[2].powi(2)).sqrt();
            // A skater is always a moving avoider (never a cut), whatever its speed.
            let moving = speed > p.moving_speed || input.id >= SKATER_OBSTACLE_BASE;
            let inactive = input.inactive || (input.held && !p.held_is_obstacle) || !p.enabled;
            let state = self.states.entry(input.id).or_insert(ObstacleState { cut: None, cut_at: input.center, now, moving, inactive, held: input.held, velocity: [0.0; 2] });
            state.now = now;
            state.velocity = [input.velocity[0], input.velocity[2]];
            state.moving = moving;
            state.inactive = inactive;
            state.held = input.held;
            let want_cut = !inactive && !moving;
            let recut = state.cut.is_some_and(|_| {
                let d = [input.center[0] - state.cut_at[0], input.center[1] - state.cut_at[1], input.center[2] - state.cut_at[2]];
                let smallest = input.half_extents.iter().map(|v| v.abs().max(p.min_half_extent)).fold(f32::INFINITY, f32::min);
                (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt() > p.recut_fraction * smallest
            });
            if !want_cut && state.cut.is_some() {
                state.cut = None;
                changed = true;
            } else if want_cut && (state.cut.is_none() || recut) {
                state.cut = Some(now);
                state.cut_at = input.center;
                changed = true;
            }
        }
        let before = self.states.len();
        self.states.retain(|id, s| {
            let keep = seen.contains(id);
            if !keep && s.cut.is_some() {
                changed = true;
            }
            keep
        });
        if changed || before != self.states.len() {
            self.rebuild_grid();
        }
        self.loose = if p.moving_solid { self.states.iter().filter(|(_, s)| s.cut.is_none() && !s.inactive).map(|(id, _)| *id).collect() } else { Vec::new() };
        if changed {
            self.version += 1;
        }
        changed
    }

    fn rebuild_grid(&mut self) {
        self.grid.clear();
        for (id, s) in &self.states {
            let Some(f) = s.cut else { continue };
            let r = f.half[0].hypot(f.half[1]) + 1.0;
            for i in cell_of(f.center[0] - r)..=cell_of(f.center[0] + r) {
                for j in cell_of(f.center[1] - r)..=cell_of(f.center[1] + r) {
                    self.grid.entry((i, j)).or_default().push(*id);
                }
            }
        }
    }

    /// Cut footprints near the xz box `min`..`max` (sorted by id, no duplicates).
    fn cuts_near(&self, min: [f32; 2], max: [f32; 2]) -> Vec<(u64, Footprint)> {
        let mut ids = Vec::new();
        for i in cell_of(min[0])..=cell_of(max[0]) {
            for j in cell_of(min[1])..=cell_of(max[1]) {
                if let Some(v) = self.grid.get(&(i, j)) {
                    ids.extend_from_slice(v);
                }
            }
        }
        ids.sort_unstable();
        ids.dedup();
        ids.into_iter().filter_map(|id| self.states.get(&id).and_then(|s| s.cut.map(|f| (id, f)))).collect()
    }

    /// Number of cut obstacles.
    pub fn cut_count(&self) -> usize {
        self.states.values().filter(|s| s.cut.is_some()).count()
    }

    /// Whether `p` lies inside a cut grown by `grow` (a target there does not fit).
    pub fn blocked(&self, p: Vec3, grow: f32) -> bool {
        let r = grow + 0.01;
        self.cuts_near([p[0] - r, p[2] - r], [p[0] + r, p[2] + r]).iter().any(|(_, f)| f.applies(p[1], self.params.step_height) && f.contains(p, grow))
    }

    /// `p` moved out of the cuts grown by `grow` to the nearest free point, when that is within
    /// [`ObstacleParams::target_snap`] (xz); `p` itself when it is free; `None` when it cannot snap.
    pub fn snap_out(&self, p: Vec3, grow: f32) -> Option<Vec3> {
        if !self.blocked(p, grow) {
            return Some(p);
        }
        let r = grow + self.params.target_snap + 0.01;
        let mut best: Option<(f32, Vec3)> = None;
        for (_, f) in self.cuts_near([p[0] - r, p[2] - r], [p[0] + r, p[2] + r]) {
            if !f.applies(p[1], self.params.step_height) || !f.contains(p, grow) {
                continue;
            }
            // Out through the nearest edge of this rectangle (just past it).
            let l = f.local([p[0], p[2]]);
            let ext = [f.half[0] + grow + 1e-3, f.half[1] + grow + 1e-3];
            for (k, s) in [(0, 1.0f32), (0, -1.0), (1, 1.0), (1, -1.0)] {
                let mut q = l;
                q[k] = s * ext[k];
                let w = f.world(q);
                let c = [w[0], p[1], w[1]];
                let d = super::nav::dist_xz(p, c);
                if d <= self.params.target_snap && !self.blocked(c, grow) && best.is_none_or(|(b, _)| d < b - 1e-6) {
                    best = Some((d, c));
                }
            }
        }
        best.map(|(_, c)| c)
    }

    /// Id of the obstacle (cut or moving, not carried) whose footprint grown by `grow` holds `p`
    /// at its height, for diagnostics (lowest id first).
    pub fn blocker_at(&self, p: Vec3, grow: f32) -> Option<u64> {
        let r = grow + 0.01;
        let sh = self.params.step_height;
        let cut = self.cuts_near([p[0] - r, p[2] - r], [p[0] + r, p[2] + r]).into_iter().find(|(_, f)| f.applies(p[1], sh) && f.contains(p, grow)).map(|(id, _)| id);
        cut.or_else(|| self.loose.iter().copied().find(|id| self.states.get(id).is_some_and(|s| s.now.applies(p[1], sh) && s.now.contains(p, grow))))
    }

    /// The first cut (grown by `grow`) the leg `a`-`b` enters: (fraction, id, footprint).
    pub fn first_hit(&self, a: Vec3, b: Vec3, grow: f32) -> Option<(f32, u64, Footprint)> {
        let min = [a[0].min(b[0]) - grow, a[2].min(b[2]) - grow];
        let max = [a[0].max(b[0]) + grow, a[2].max(b[2]) + grow];
        let mut best: Option<(f32, u64, Footprint)> = None;
        for (id, f) in self.cuts_near(min, max) {
            if !f.applies(a[1].min(b[1]), self.params.step_height) {
                continue;
            }
            if let Some(t) = f.segment_hit(a, b, grow) {
                if best.is_none_or(|(bt, ..)| t < bt) {
                    best = Some((t, id, f));
                }
            }
        }
        best
    }

    /// Bend a corner list round the cuts (cheapest free grown corner); legs without one stay.
    pub fn detour(&self, mesh: &NavMesh, start: Vec3, corners: &[Vec3], radius: f32) -> Vec<Vec3> {
        if self.grid.is_empty() || corners.is_empty() {
            return corners.to_vec();
        }
        let grow = radius + self.params.detour_margin;
        let mut out: Vec<Vec3> = Vec::with_capacity(corners.len() + 2);
        let mut from = start;
        let mut inserted = 0;
        let mut i = 0;
        while i < corners.len() {
            let to = corners[i];
            let hit = if inserted < self.params.max_detours { self.first_hit(from, to, grow) } else { None };
            // A leg ending inside the margin only (a path end snapped out of the cut, [`Self::snap_out`]) is tested
            // against the body's rectangle: the end is reachable without entering the cut.
            let hit = hit.and_then(|h| if h.2.contains(to, grow) && !h.2.contains(to, radius) { self.first_hit(from, to, radius) } else { Some(h) });
            // A start inside the body's rectangle (a prop pushed onto the ped): leave the leg. A ped standing against
            // the prop (inside the margin only) is still outside the cut and is routed round it, as on retail's cut
            // mesh (2026-10-19, a ped pinned 55 s against the back of a newspaper box re-planned straight through it).
            let Some((t, id, f)) = hit.filter(|(t, _, f)| *t > 0.0 || !f.contains(from, radius)) else {
                out.push(to);
                from = to;
                i += 1;
                continue;
            };
            let _ = t;
            let mut best: Option<(f32, Vec3)> = None;
            for c in f.corners(grow + 0.05) {
                let Some(on) = mesh.locate([c[0], from[1], c[1]]) else { continue };
                let p = on.position;
                // Never the corner the leg starts from: its zero-length leg never hits, so it won every time and a
                // detour needing a second corner repeated the first one up to `max_detours` (2026-10-19).
                if super::nav::dist_xz(p, [c[0], p[1], c[1]]) > 0.05 || self.blocked(p, radius) || super::nav::dist_xz(p, from) < 1e-3 {
                    continue;
                }
                // The leg to the corner must not enter this rectangle again (or any earlier cut); from inside the
                // margin, not the body's rectangle.
                let leg = if f.contains(from, grow) { radius } else { grow };
                if self.first_hit(from, p, leg).is_some_and(|(_, hid, _)| hid == id) {
                    continue;
                }
                let cost = super::nav::dist_xz(from, p) + super::nav::dist_xz(p, to);
                if best.is_none_or(|(b, _)| cost < b - 1e-5) {
                    best = Some((cost, p));
                }
            }
            match best {
                Some((_, p)) => {
                    out.push(p);
                    from = p;
                    inserted += 1;
                }
                None => {
                    out.push(to);
                    from = to;
                    i += 1;
                }
            }
        }
        out
    }

    /// A ped body's step: `to` when free, else slid along the obstacle face, else `None` (ours).
    pub fn resolve_step(&self, from: Vec3, to: Vec3, radius: f32) -> Option<Vec3> {
        self.resolve_step_ignoring(from, to, radius, Ignore::default())
    }

    /// [`Self::resolve_step`] for a ped whose collision ignores one object (`ignore`, an obstacle id): retail
    /// `DisableCollisionsWithBehaviourSource` (`826A7378`, `82E3DB68` -> `ped+5916`) lets a ped reach a seat whose
    /// waypoint lies inside its own bench.
    pub fn resolve_step_ignoring(&self, from: Vec3, to: Vec3, radius: f32, ignore: Ignore) -> Option<Vec3> {
        self.resolve_step_contact(from, to, radius, ignore, Contact::default())
    }

    /// [`NavObstacles::resolve_step_ignoring`] with the ped's own motion for the contact slide's tests.
    pub fn resolve_step_contact(&self, from: Vec3, to: Vec3, radius: f32, ignore: Ignore, contact: Contact) -> Option<Vec3> {
        if self.step_ok_ignoring(from, to, radius, ignore) {
            return Some(to);
        }
        let r = radius + 0.01;
        let mut blocking: Vec<(Footprint, Option<[f32; 2]>)> = self.cuts_near([to[0] - r, to[2] - r], [to[0] + r, to[2] + r]).into_iter().filter(|(id, _)| !ignore.skips(*id)).map(|(_, f)| (f, None)).collect();
        blocking.extend(self.loose.iter().filter(|id| !ignore.skips(**id)).filter_map(|id| self.states.get(id).map(|s| (s.now, Some(s.velocity)))));
        let (f, loose) = blocking.into_iter().find(|(f, _)| f.applies(to[1], self.params.step_height) && f.contains(to, radius) && !f.contains(from, radius))?;
        let to = match loose {
            Some(v) => self.contact_slide(&f, from, to, v, contact).unwrap_or(to),
            None => to,
        };
        if !f.contains(to, radius) {
            return self.step_ok_ignoring(from, to, radius, ignore).then_some(to);
        }
        // Keep `to` on the face `from` is outside of (the axis where `from` is beyond the grown half).
        let (lf, lt) = (f.local([from[0], from[2]]), f.local([to[0], to[2]]));
        let mut l = lt;
        let ext = [f.half[0] + radius + 1e-4, f.half[1] + radius + 1e-4];
        let k = if lf[0].abs() - ext[0] >= lf[1].abs() - ext[1] { 0 } else { 1 };
        l[k] = ext[k].copysign(lf[k]);
        let w = f.world(l);
        let slid = [w[0], to[1], w[1]];
        self.step_ok_ignoring(from, slid, radius, ignore).then_some(slid)
    }

    /// The moving bodies (no cut, active) in id order, without those `ignore` skips.
    pub fn moving_avoiders(&self, ignore: Ignore) -> impl Iterator<Item = (u64, &ObstacleState)> + '_ {
        self.loose.iter().filter(move |id| !ignore.skips(**id)).filter_map(|id| self.states.get(id).map(|s| (*id, s)))
    }

    /// Retail's predictive side-step (`82E9CE90` picks the avoider, `82E9CBD0` the force): the lateral acceleration
    /// (x, z) for a ped at `p` moving with `v`: docs/hails-additions/26f-living-world-peds.md#predictive-side-step
    pub fn side_step(&self, p: Vec3, v: [f32; 2], radius: f32, ignore: Ignore) -> [f32; 2] {
        let pr = &self.params;
        let speed = v[0].hypot(v[1]);
        if pr.avoid_horizon <= 0.0 || speed < 1e-4 {
            return [0.0; 2];
        }
        let cross = |a: [f32; 2], b: [f32; 2]| a[0] * b[1] - a[1] * b[0];
        let me = [p[0], p[2]];
        let r_sum = radius + pr.avoid_scale * pr.avoider_extent * (1.0 + 2.0 * pr.avoider_extent_tail);
        let mut best: Option<(f32, [f32; 2], [f32; 2])> = None;
        for (_, s) in self.moving_avoiders(ignore) {
            if !s.now.applies(p[1], pr.step_height) {
                continue;
            }
            let dv = [s.velocity[0] - v[0], s.velocity[1] - v[1]];
            let dvl = dv[0].hypot(dv[1]);
            let rel = [s.now.center[0] - me[0], s.now.center[1] - me[1]];
            let dist2 = rel[0] * rel[0] + rel[1] * rel[1];
            if dvl <= 0.0 || dist2 < r_sum * r_sum {
                continue;
            }
            // Ray from the avoider along the relative velocity against the ped's circle.
            let dir = [dv[0] / dvl, dv[1] / dvl];
            let along = -(rel[0] * dir[0] + rel[1] * dir[1]);
            let rr = pr.avoid_ray_scale * r_sum;
            let off2 = dist2 - along * along;
            if along < 0.0 || off2 > rr * rr {
                continue;
            }
            let t = (along - (rr * rr - off2).sqrt()) / dvl;
            if (0.0..pr.avoid_horizon).contains(&t) && best.is_none_or(|(b, _, _)| t < b) {
                best = Some((t, s.now.center, s.velocity));
            }
        }
        let Some((t, c, va)) = best else { return [0.0; 2] };
        let u = [v[0] / speed, v[1] / speed];
        let val = va[0].hypot(va[1]);
        let at = [c[0] + va[0] * t, c[1] + va[1] * t];
        // Retail's (x, y) is our (x, -z), so its `cross <= 0` is ours `>= 0` and its left perpendicular is (u.z, -u.x).
        let left = if val <= 0.0 || (u[0] * va[0] + u[1] * va[1]) / val < pr.avoid_head_on_dot {
            cross(u, [at[0] - me[0], at[1] - me[1]]) >= 0.0
        } else if (u[0] * va[0] + u[1] * va[1]) / val <= pr.avoid_crossing_dot {
            // Skaters and props weigh FLT_MAX (`82E99998`): the ped never has right of way.
            cross(u, va) >= 0.0
        } else {
            let ua = [va[0] / val, va[1] / val];
            let mine = [me[0] + v[0] * t, me[1] + v[1] * t];
            cross(u, [at[0] - me[0], at[1] - me[1]]) >= cross(ua, [mine[0] - c[0], mine[1] - c[1]])
        };
        let k = pr.avoid_accel * pr.avoid_scale * if left { 1.0 } else { -1.0 };
        [u[1] * k, -u[0] * k]
    }

    /// Whether a ped at `p` moving with `v` brakes this step (`82E9FC20`, research b716): within the goal radius of
    /// its steer target `target`, the nearest moving body along `v` (not moving away) is closer than the stopping
    /// distance: docs/hails-additions/26f-living-world-peds.md#predictive-side-step
    pub fn brakes(&self, p: Vec3, v: [f32; 2], target: [f32; 2], ignore: Ignore) -> bool {
        let pr = &self.params;
        let speed = v[0].hypot(v[1]);
        if pr.brake_decel <= 0.0 || speed < 1e-4 || (target[0] - p[0]).hypot(target[1] - p[2]) >= pr.brake_goal_radius {
            return false;
        }
        let stop = speed * speed / (2.0 * pr.brake_decel);
        let u = [v[0] / speed, v[1] / speed];
        let r = pr.brake_body_radius + pr.avoid_scale * pr.avoider_extent * (1.0 + 2.0 * pr.avoider_extent_tail);
        self.moving_avoiders(ignore).any(|(_, s)| {
            if s.velocity[0] * u[0] + s.velocity[1] * u[1] >= pr.brake_away_fraction * speed {
                return false;
            }
            // The ray from the ped along `u` against a circle of `r` round the body: the first hit distance.
            let c = [s.now.center[0] - p[0], s.now.center[1] - p[2]];
            let along = c[0] * u[0] + c[1] * u[1];
            let off2 = c[0] * c[0] + c[1] * c[1] - along * along;
            if off2 > r * r {
                return false;
            }
            let hit = along - (r * r - off2).sqrt();
            hit < stop && along >= 0.0
        })
    }

    /// Retail's path turn term (`82E9DF70`, research b716) on the sideways velocity `sv` of a ped moving `speed` along
    /// `base`: the heading error of `base * speed + sv` against `base` turns it back at `path_turn_accel` (scaled below
    /// `path_turn_full_deg`, boosted above `path_turn_boost_deg`), never more than the sideways speed over one
    /// controller step nor the centripetal limit. Returns the new `sv`: docs/hails-additions/26f-living-world-peds.md#predictive-side-step
    pub fn path_turn(&self, sv: [f32; 2], base: [f32; 2], speed: f32, dt: f32) -> [f32; 2] {
        let p = &self.params;
        let l = sv[0].hypot(sv[1]);
        if l <= 0.0 {
            return sv;
        }
        let want = [base[0] * speed + sv[0], base[1] * speed + sv[1]];
        let wl = want[0].hypot(want[1]);
        let angle = if wl > 1e-6 { ((want[0] * base[0] + want[1] * base[1]) / wl).clamp(-1.0, 1.0).acos() } else { std::f32::consts::PI };
        if angle >= p.path_turn_max_deg.to_radians() {
            return sv;
        }
        let boost = ((angle - p.path_turn_boost_deg.to_radians()) * p.path_turn_boost_rate + 1.0).clamp(1.0, p.path_turn_boost_max.max(1.0));
        let full = p.path_turn_full_deg.to_radians();
        let scale = if full > 0.0 { (angle / full).min(1.0) } else { 1.0 };
        let mut turn = p.path_turn_accel * boost * scale;
        if p.controller_step > 0.0 {
            turn = turn.min(l / p.controller_step);
        }
        if p.path_turn_radius > 0.0 {
            turn = turn.min(speed * speed / (2.0 * p.path_turn_radius));
        }
        let cut = (turn * dt).min(l);
        [sv[0] - sv[0] / l * cut, sv[1] - sv[1] / l * cut]
    }

    /// Retail's contact slide round a moving body (`82E9D0C0`): docs/hails-additions/26f-living-world-peds.md#contact-slide
    fn contact_slide(&self, f: &Footprint, from: Vec3, to: Vec3, avoider: [f32; 2], contact: Contact) -> Option<Vec3> {
        let (dx, dz) = (to[0] - from[0], to[2] - from[2]);
        let len = dx.hypot(dz);
        let (nx, nz) = (from[0] - f.center[0], from[2] - f.center[1]);
        let nl = nx.hypot(nz);
        if self.params.contact_slide <= 0.0 || len < 1e-6 || nl < 1e-6 {
            return None;
        }
        let unit = |v: [f32; 2]| {
            let l = v[0].hypot(v[1]);
            (l > 1e-6).then(|| [v[0] / l, v[1] / l])
        };
        // The direction (b717 / b718): toward the steer target at or below the slow fraction of the path speed, else
        // along the velocity; the step itself without the ped's motion.
        let slow = self.params.contact_slow_fraction * self.params.contact_max_speed;
        let step = [dx / len, dz / len];
        let d = match contact.velocity {
            Some(v) if v[0].hypot(v[1]) <= slow => contact.target.and_then(|t| unit([t[0] - from[0], t[1] - from[2]])).unwrap_or(step),
            Some(v) => unit(v).unwrap_or(step),
            None => step,
        };
        let n = [nx / nl, nz / nl];
        let into = d[0] * n[0] + d[1] * n[1];
        if into >= self.params.contact_slide_dot {
            return None;
        }
        // A body moving away along it at the slow fraction of the path speed or more: push-out only.
        if avoider[0] * d[0] + avoider[1] * d[1] >= slow {
            return None;
        }
        // Tangent on the side the step already leans to; dead centre takes retail's fixed side.
        let mut t = [n[1], -n[0]];
        let along = d[0] * t[0] + d[1] * t[1];
        if along < 0.0 {
            t = [-t[0], -t[1]];
        }
        let s = (along.abs() + self.params.contact_slide * -into) * len;
        Some([from[0] + t[0] * s, to[1], from[2] + t[1] * s])
    }

    /// Whether a ped body of `radius` may step from `from` to `to`: no obstacle (cut or moving,
    /// not carried) overlaps it afterwards, or the step does not go deeper into one it already
    /// overlaps (a prop pushed onto a ped lets it walk out).
    pub fn step_ok(&self, from: Vec3, to: Vec3, radius: f32) -> bool {
        self.step_ok_ignoring(from, to, radius, Ignore::default())
    }

    /// [`Self::step_ok`] without the obstacle `ignore` (see [`Self::resolve_step_ignoring`]).
    pub fn step_ok_ignoring(&self, from: Vec3, to: Vec3, radius: f32, ignore: Ignore) -> bool {
        if !self.params.enabled {
            return true;
        }
        let sh = self.params.step_height;
        let r = radius + 0.01;
        let ok = |f: &Footprint| {
            if !f.applies(to[1], sh) || !f.contains(to, radius) {
                return true;
            }
            // Already inside: allow steps that move away from the centre.
            let d = |p: Vec3| (p[0] - f.center[0]).hypot(p[2] - f.center[1]);
            f.contains(from, radius) && d(to) > d(from)
        };
        self.cuts_near([to[0] - r, to[2] - r], [to[0] + r, to[2] + r]).iter().all(|(id, f)| ignore.skips(*id) || ok(f))
            && self.loose.iter().all(|id| ignore.skips(*id) || self.states.get(id).is_none_or(|s| ok(&s.now)))
    }
}
