//! Dynamic objects as navigation obstacles for peds (doc 26, fix 11): props (DMOs) and mod bodies
//! cut out of the walkable area while they rest, so peds plan their paths round them.
//!
//! What retail does [code, TU3; addresses are evidence only]:
//! - Every world object has an obstacle interface on its vtable: register `sub_82595140` (via the
//!   base slot `sub_82595010`, vtable + 4) and update `sub_82595298`. Slot +68 says whether the
//!   object is an obstacle at all. It returns 1 for the DynamicObject class (vtable `0x82322D50`,
//!   the string `dynamicobject` sits at `0x82322D2C`), the player skater (`0x82300910`) and one
//!   more class (`0x823220E0`); peds (`0x8232BE80`) and the other actor classes return 0.
//! - A DynamicObject gets its obstacle record from the pool `DynamicObject Obstacle MemStore`
//!   (slot +76 `sub_82C57268`, record ctor `sub_82C46778`, record vtable `0x82322D10`). The box
//!   comes from its collision body (slot +96 `sub_82C486F8` -> body vfunc 160 = half extents;
//!   slot +92 `sub_82C48688` = orientation from the collision shape). Slot +88 `sub_82C48648`
//!   switches the obstacle off while the object's state word `+144+4252` equals 1. That word is set
//!   to 1 only by the component's slot 32 (`sub_82C56B00`, which also moves every collision element
//!   of the object to collision group 13; slots 33/34 set it back to 0 with group 12 or 14). It means
//!   "attached to a ped's hand" (b349: slot 32 is the hand attach, slot 34 `82C56C70` the release that
//!   `82E3EBE0` calls for a drop or throw), so a ped's hand prop is no obstacle while held and a normal cut
//!   from its release on. It is NOT the Move Object hold: the hold's keep-alive (interface slot
//!   10 -> DMO handler slot 6 `sub_82C4C450` -> component slot 30 `sub_82C485D0`) only sets the
//!   held bit `DMO+4464 & 0x20`, which the obstacle code never reads. So a held prop stays an
//!   obstacle: cut while slower than 0.4 m/s, a moving avoider while faster (2026-10-08).
//! - Placement `sub_82C477B0`, called on every update: half extents below 0.2 m are raised to
//!   0.2 (`0x82099280`); while the object moves faster than 0.4 m/s (`0x82181B90`, slot +24 =
//!   velocity) its cut is removed and NavPower's moving avoider (`+64`, `sub_82E99998`) takes
//!   over; at rest the box is cut into the NavPower mesh (`sub_8293EF28` with the record
//!   `{-1, 15.0, type 4}`, then `sub_8293F340`), and re-cut only after the object moved more than
//!   0.25 (`0x820C6D98`) x its smallest half extent from where it was cut.
//! - NavPower then plans every bot's path on the cut mesh (`dynAreas` in its planner), so a
//!   resting prop is walked round, a prop the player moved counts where it lies now, and
//!   a rolling prop (or a held one being moved faster than 0.4 m/s) is not part of the mesh but a
//!   NavPower moving avoider.
//!
//! Ours (stated): NavPower's polygon cutting and its moving avoider are not decoded. A cut is an
//! oriented rectangle in xz (the box projected onto the ground); a path leg that crosses a cut
//! (grown by the agent radius plus [`ObstacleParams::detour_margin`]) gets a detour corner at the
//! cheapest free rectangle corner on the mesh; a target inside a cut does not fit (retail: its
//! end point no longer snaps to the cut mesh). Whether the record's `-1` / 15.0 mean "blocks
//! every mover" / "penalty x15" is not decoded; cuts block here. Bodies (cut or moving) are also
//! solid for a ped's step ([`NavObstacles::step_ok`]), standing in for the physical contact.
//!
//! Deterministic: obstacles are kept by stable id in a `BTreeMap`, the grid is a `BTreeMap`, ties
//! are broken by id; [`NavObstacles::version`] changes only when a cut changes, so nothing is
//! rebuilt per tick while props rest.

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
    /// A moving (uncut) object is solid for a ped's step ([`NavObstacles::step_ok`]). NOT RETAIL
    /// YET: retail hands a moving object to NavPower's moving avoider instead (`sub_82E99998`:
    /// an 88-byte record in the planner's obstacle database, position and velocity refreshed every
    /// tick by `sub_82E998C8`, radius = 0.35 x a planner-wide value, independent of the box); how
    /// NavPower's bots steer round it is middleware internals, not decoded. `false` = moving
    /// objects do not block a ped's step (the switch a decoded avoider port would replace).
    pub moving_solid: bool,
    /// Player skaters are obstacles for peds (research b135: the skater's obstacle interface `0x82300910` slot
    /// +104 returns 1, so `sub_82C477B0` always takes the moving-avoider branch, at any speed, standing included).
    pub skaters: bool,
    /// A skater obstacle's half extents (x, y, z), m. NOT RETAIL YET: retail's avoider is `{a, 0.2a, 0.2a}`,
    /// `a = 0.25 R` with `R` a planner-wide value not read yet; this is a body-sized stand-in.
    pub skater_half_extents: Vec3,
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
        Self { enabled: true, min_half_extent: 0.2, moving_speed: 0.4, recut_fraction: 0.25, detour_margin: 0.1, max_detours: 8, target_snap: 0.5, step_height: 0.0, held_is_obstacle: true, moving_solid: true, skaters: true, skater_half_extents: [0.3, 0.9, 0.3] }
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
            let state = self.states.entry(input.id).or_insert(ObstacleState { cut: None, cut_at: input.center, now, moving, inactive, held: input.held });
            state.now = now;
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

    /// Bend a corner list (the path after `start`) round the cuts: every leg that enters a cut
    /// grown by `radius + detour_margin` gets the cheapest corner of that rectangle (grown a little
    /// more) that is on the mesh, outside every other cut and not itself blocked from the leg start.
    /// Legs with no such corner stay (the step check then holds the ped and it re-plans).
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

    /// The step a ped body of `radius` takes from `from` toward `to`: `to` when free
    /// ([`Self::step_ok`]); else slid along the face of the obstacle it would enter (the move into
    /// the face removed), when that slide is free; else `None` (the ped stays). Ours: stands in
    /// for the physical contact that keeps a retail ped out of a prop.
    pub fn resolve_step(&self, from: Vec3, to: Vec3, radius: f32) -> Option<Vec3> {
        self.resolve_step_ignoring(from, to, radius, Ignore::default())
    }

    /// [`Self::resolve_step`] for a ped whose collision ignores one object (`ignore`, an obstacle id): retail
    /// `DisableCollisionsWithBehaviourSource` (`826A7378`, `82E3DB68` -> `ped+5916`) lets a ped reach a seat whose
    /// waypoint lies inside its own bench.
    pub fn resolve_step_ignoring(&self, from: Vec3, to: Vec3, radius: f32, ignore: Ignore) -> Option<Vec3> {
        if self.step_ok_ignoring(from, to, radius, ignore) {
            return Some(to);
        }
        let r = radius + 0.01;
        let mut blocking: Vec<Footprint> = self.cuts_near([to[0] - r, to[2] - r], [to[0] + r, to[2] + r]).into_iter().filter(|(id, _)| !ignore.skips(*id)).map(|(_, f)| f).collect();
        blocking.extend(self.loose.iter().filter(|id| !ignore.skips(**id)).filter_map(|id| self.states.get(id).map(|s| s.now)));
        let f = blocking.into_iter().find(|f| f.applies(to[1], self.params.step_height) && f.contains(to, radius) && !f.contains(from, radius))?;
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
