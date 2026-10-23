//! Pedestrian wandering (doc 26, peds milestone M3): retail's `NoRoadWander` goal on the navmesh,
//! a mod-only crosswalk rule, ped-to-ped avoidance and the walk / turn intents for the animation
//! player.
//!
//! What retail does [code, TU3; addresses are evidence only]:
//! - **The ped graph never takes the road branch.** `Pedestrian.xml`'s `Wander` state picks
//!   `WanderMode.Road` (FollowRoad + `UseCrossWalk`) while `HasRoadWanderTarget`, and goes to
//!   `WanderOnRoad` on `IsOnRoad`; in `StateGraph::TheConditionFactory` (`sub_826C1730`) both
//!   names are registered with the generic factory `sub_82BC3F68` (`sub_82F7A698`,
//!   `sub_82F79698`), whose evaluate slot (vtable `0x8231ED5C` + 48) is `sub_8274CA90` = return
//!   0. So every ped runs `WanderMode.NoRoad`: `CheckForRoadTarget` (all slots no-ops) and
//!   `NoRoadWander` (`sub_826A2FB8`). The crosswalk states and `WalkSignSaysGo` (`sub_826AC190`)
//!   are unreachable from the ambient ped graph. (The real `IsOnRoad` / `IsNearCrossWalk`,
//!   `sub_826C48F0` / `sub_826C4878`, live in `BehaviourGraph::TheTransferConditionFactory` for
//!   plugin entry rules such as `usetrashbin.xml`.)
//! - `NoRoadWander` (and `Wander`) switch the ped to its NavPower mover (`sub_82E360F0(1)`:
//!   `ped+2224`) in mode 6 (`sub_82E35E80`): the mover's goal becomes the wander goal at
//!   `ped+5632` (vtable `0x8232C9C8`), its value `+12` = 2.0 (`0x82060C50`).
//! - **Target choice** (`sub_82E30F58`): direction = the ped's forward vector (owner vfunc 20),
//!   or, after event 4, the unit vector from the old target to the ped. Unless an event 1-3 came
//!   in, probe 5 extra directions within 90 degrees at 40 m (`0x821EB794`, `0x82256FD8`); if none
//!   fits, 9 within 270 degrees at 10 m (`0x821963E4`, `0x82063B20`); if none fits either, the
//!   target is `position + direction x max(3, 1.1 x value)` (`0x82063B08`, `0x82099148`).
//! - **The probe** (`sub_82E311A8`): straight ahead first, then probe `i = 1..=n` at angle
//!   `ceil(i / 2) x (spread / 2) / (n / 2)` (n / 2 rounded down), negative for odd `i`
//!   (`0x8216DEE0` = -1); a direction fits when its end point is on the navmesh and reachable
//!   from the ped's polygon (`sub_82C464F0`). The fitting end point becomes the target.
//! - **Re-targeting** (`sub_82E2CB08`, mover vfunc 44): when the NavPower bot reports state 1
//!   the goal picks a new target from the ped's position and NavPower is sent there.
//! - Speed: `SuggestVelocity linear=3.0` in `Wander` [data]; the walk itself is root motion
//!   (walk clip 1.325 m/s, recomp walking median 1.30 [trace]).
//!
//! Ours (stated): NavPower's path following and local avoidance are not decoded. A ped follows
//! the funnel corners of [`NavMesh::find_path`], steers toward the next corner at
//! [`WanderParams::steer_rate`] (measured 255 deg/s) below the first turn edge and posts retail's turn buckets past it
//! (`82E2A500`: 45 / 90 / 135 / 180, standing or walking, the clip turns the ped; the angle is
//! to the next corner, retail's against NavPower's steering velocity `82E2ABF8` / `82E2AAA0`, b169), re-targets on arrival (the bot's state 1) and,
//! when a path cannot be found, as after an event 1-3 (short fan next). Avoidance: a ped yields
//! (stands) while a ped with a lower id is inside its personal space ahead of it, side-steps a
//! ped with a higher id standing in its way, steers away from close peds, and never takes a
//! step that brings it within twice the agent radius of another ped ([`separation_ok`]). The skater is not avoided by navigation
//! (retail peds walk into a standing skater, PEDCOLL; the motion graph's `Avoid` reactions come
//! with the behaviour milestones).

use crate::living_world::Vec3;
use crate::living_world::traffic::signals::Light;

use super::anim::{Intent, Locomotion};
use super::nav::{NavMesh, NavPoint, dist_xz};
use super::obstacles::NavObstacles;

/// One probe fan [code `sub_82E311A8` arguments].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fan {
    pub rays: u32,
    pub distance: f32,
    pub spread_degrees: f32,
}

/// Wander parameters: retail values as defaults (a mod overrides any of them).
#[derive(Clone, Debug, PartialEq)]
pub struct WanderParams {
    /// First fan: 5 rays, 40 m, 90 degrees [code].
    pub long: Fan,
    /// Second fan: 9 rays, 10 m, 270 degrees [code].
    pub short: Fan,
    /// Fallback step: `max(fallback_min, value x fallback_scale)` [code 3.0 / 1.1].
    pub fallback_min: f32,
    pub fallback_scale: f32,
    /// The mover value the wander mode sets (`+12` = 2.0) [code].
    pub mover_value: f32,
    /// Distance at which a corner counts as reached (ours).
    pub corner_radius: f32,
    /// Turn rate while walking, radians per second. Default from `livingworld_entities_locomotion`
    /// `Hash_AD1EA18F819BF397` = 45 read as degrees per second (meaning unverified).
    pub turn_rate: f32,
    /// The walking steer rate, radians per second: 255 deg/s [trace, recomp 2026-10-16, PEDFACE at PCU Library and
    /// Aletown: 30 peds, 9136 walking samples, the facing follows NavPower's steering direction with no lag and its
    /// corrections outside turn clips peak at 257 deg/s, p99 149]. Code candidate (b158, meaning unverified): the bot
    /// value +0x80 = clamp(255 x max(1, s), 90, 720) of the truncated speed (`82E2D260`), 255 at s = 1; the 510 it gives
    /// at the wander speed 2.0 is never seen in the trace.
    pub steer_rate: f32,
    /// The turn bucket edges, degrees of heading error (`82E2A500`: 25 / 70 / 115 / 160, constants `0x8209994C` /
    /// `0x8218155C` / `0x822F929C` / `0x82099154` and their negatives; research b167) [code, values read by the
    /// research agent]: from the first edge up a standing or walking ped posts the 45 / 90 / 135 / 180 turn of that
    /// side; below it the ped steers.
    pub turn_edges: [f32; 4],
    /// Heading error above which a running ped, or a walking ped the avoidance stand-in steers, stops to pivot, and
    /// below which such a standing ped starts walking (ours, until the run turns and NavPower's avoidance are ported).
    pub stop_heading: f32,
    pub walk_heading: f32,
    /// Seconds a walking ped may stay inside the corner radius without getting closer before it moves on to the next
    /// corner (ours).
    pub stall_seconds: f32,
    /// A walking ped that stays within `stuck_distance` (0.13 m) for `stuck_seconds` (0.5 s) re-paths to the SAME
    /// target, once per still spell. The values are the bot's still timer (`82E97118`, b313 / b314), but retail runs
    /// that timer only in bot mode 4, not for walking peds (b336): on walking peds this re-path is OURS. The outcome
    /// matches retail, which never gives a walking ped's target up for lack of progress (b190, b332).
    pub stuck_distance: f32,
    pub stuck_seconds: f32,
    /// NOT RETAIL (ours, a safety net): still pinned after this many seconds, the ped gives up its target and plans
    /// away from it (the pre-2026-10-19 stuck watch). 0 = off.
    pub give_up_seconds: f32,
    /// Personal space for yielding, metres ahead (ours: the navigation record's 2.0,
    /// `Hash_7C8E344CDEC7510B`, meaning unverified).
    pub yield_distance: f32,
    /// NOT RETAIL (ours): seconds a yielding ped waits before it re-paths to the same target. Keeping the target is
    /// retail's outcome; the 2.0 s is borrowed from the bot's mode-0 (no path) controller `82EA4408`, which walking path
    /// peds never run (b335). Retail's walking path step `82EA0808` has no neighbour handling at all (b336): our yield,
    /// sidestep and push are ours; what keeps retail walking peds apart is not read yet.
    pub yield_patience: f32,
}

impl Default for WanderParams {
    fn default() -> Self {
        Self {
            long: Fan { rays: 5, distance: 40.0, spread_degrees: 90.0 },
            short: Fan { rays: 9, distance: 10.0, spread_degrees: 270.0 },
            fallback_min: 3.0,
            fallback_scale: 1.1,
            mover_value: 2.0,
            corner_radius: 0.5,
            turn_rate: 45f32.to_radians(),
            steer_rate: 255f32.to_radians(),
            turn_edges: [25.0, 70.0, 115.0, 160.0],
            stop_heading: 60f32.to_radians(),
            walk_heading: 15f32.to_radians(),
            stall_seconds: 0.5,
            stuck_distance: 0.13,
            stuck_seconds: 0.5,
            give_up_seconds: 2.0,
            yield_distance: 2.0,
            yield_patience: 2.0,
        }
    }
}

/// Crosswalk rule. Retail ambient peds never use crosswalks (the road branch is unreachable,
/// see the module notes), so `Off` is the retail default; `WalkSignal` is the rule the unused
/// `UseCrossWalk` state describes (wait while the walk sign does not say go), for mods.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CrosswalkRule {
    #[default]
    Off,
    WalkSignal,
}

/// Where a ped is about to step onto a road and what the walk light there shows (the game
/// answers from the road network and the shared signal clock).
pub trait WalkSignals {
    /// The walk light for a crossing that starts at `at` (`None` = no signalled crossing).
    fn walk_light(&self, at: Vec3) -> Option<Light>;
}

/// No signals anywhere.
pub struct NoSignals;
impl WalkSignals for NoSignals {
    fn walk_light(&self, _: Vec3) -> Option<Light> {
        None
    }
}

/// Another agent near a ped (ordered by `order`, the population id).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Neighbour {
    pub order: u64,
    pub position: Vec3,
}

/// What the navigator asks of the body this tick.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NavOutput {
    pub intent: Intent,
    /// Heading change to apply this tick (radians, counter-clockwise seen from above).
    pub turn: f32,
    pub waiting: NavWait,
    /// With a turn intent: the bucket, 45 / 90 / 135 / 180 degrees (0 otherwise).
    pub turn_degrees: u16,
    /// The steering turn this tick even with a turn intent: a host whose animation set lacks the bucket's clip
    /// pivots by it instead (ours).
    pub steer: f32,
}

/// The turn bucket for a heading error (radians, positive = left), `82E2A500`: the largest edge reached picks
/// 180 / 135 / 90 / 45; below the first edge no turn.
pub fn turn_bucket(error: f32, edges: &[f32; 4]) -> Option<u16> {
    let deg = error.to_degrees().abs();
    [(edges[3], 180), (edges[2], 135), (edges[1], 90), (edges[0], 45)].into_iter().find(|(e, _)| deg >= *e).map(|(_, b)| b)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NavWait {
    #[default]
    None,
    /// Yielding to another ped.
    Yield,
    /// Waiting for the walk light (mod crosswalk rule).
    Crosswalk,
    /// Off the navmesh (no data here): standing.
    OffMesh,
}

/// A route a mod gives a ped (followed in order instead of wandering; `looped` repeats it).
#[derive(Clone, Debug, PartialEq)]
pub struct PedRoute {
    pub points: Vec<Vec3>,
    pub looped: bool,
}

/// Per-ped navigation state.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct PedNav {
    pub target: Option<Vec3>,
    pub corners: Vec<Vec3>,
    pub next: usize,
    /// Retail flag `+32` (events 1-3): skip the long fan on the next target.
    pub skip_long: bool,
    pub waiting: NavWait,
    pub waited: f32,
    pub targets_chosen: u32,
    pub route: Option<PedRoute>,
    route_index: usize,
    /// [`NavObstacles::version`] the corners were last checked against (fix 11).
    pub obstacle_version: u64,
    /// The polygon the body stands on (kept by [`constrain_move`]; fix 17). `None` = locate.
    pub poly: Option<u32>,
    /// The corner stall watch (corner index, closest distance, seconds without getting closer).
    pub stall: Option<(usize, f32, f32)>,
    /// The stuck watch (anchor position, seconds within `stuck_distance` of it), the reverse flag for the next plan
    /// and how often the ped gave up (logs, tests).
    pub stuck: Option<(Vec3, f32)>,
    pub reverse: bool,
    pub stuck_count: u32,
    /// The still timer fired in this still spell (it fires once until the ped moves out of `stuck_distance`).
    pub still_fired: bool,
    /// Re-path to the same target on the next step (the still timer, a yield, the host's refused steps).
    pub repath_due: bool,
    /// How often the ped re-pathed to its target (logs, tests).
    pub repaths: u32,
}

fn rotate_y(v: [f32; 2], angle: f32) -> [f32; 2] {
    let (s, c) = angle.sin_cos();
    [v[0] * c + v[1] * s, -v[0] * s + v[1] * c]
}

/// Forward vector of a heading (the ped's frame: +z forward, yaw about +y).
pub fn forward(heading: f32) -> [f32; 2] {
    [heading.sin(), heading.cos()]
}

fn heading_of(d: [f32; 2]) -> f32 {
    d[0].atan2(d[1])
}

fn wrap(a: f32) -> f32 {
    let t = std::f32::consts::TAU;
    let mut a = a % t;
    if a > std::f32::consts::PI {
        a -= t;
    } else if a < -std::f32::consts::PI {
        a += t;
    }
    a
}

/// The probe angles of a fan in order (radians): the straight probe, then `i = 1..=n`.
pub fn fan_angles(fan: &Fan) -> Vec<f32> {
    let half = (fan.rays / 2).max(1) as f32;
    let step = fan.spread_degrees * (std::f32::consts::PI / 360.0) / half; // 0x822F8F08 = pi / 360
    let mut out = vec![0.0];
    for i in 1..=fan.rays {
        let k = i.div_ceil(2) as f32;
        out.push(if i % 2 == 1 { -k * step } else { k * step });
    }
    out
}

/// Retail's fan probe: the first direction whose end point is on the mesh and reachable.
pub fn probe_fan(mesh: &NavMesh, from: NavPoint, dir: [f32; 2], fan: &Fan) -> Option<NavPoint> {
    probe_fan_clear(mesh, from, dir, fan, None)
}

/// [`probe_fan`] on the cut mesh: an end point inside a resting obstacle (grown by the agent
/// radius) does not fit, as retail's snap to the cut NavPower mesh fails there (fix 11).
pub fn probe_fan_clear(mesh: &NavMesh, from: NavPoint, dir: [f32; 2], fan: &Fan, obstacles: Option<&NavObstacles>) -> Option<NavPoint> {
    for angle in fan_angles(fan) {
        let d = rotate_y(dir, angle);
        let end = [from.position[0] + d[0] * fan.distance, from.position[1], from.position[2] + d[1] * fan.distance];
        if let Some(p) = mesh.locate(end) {
            if mesh.reachable(from.poly, p.poly) && !obstacles.is_some_and(|o| o.blocked(p.position, mesh.agent[1])) {
                return Some(p);
            }
        }
    }
    None
}

/// Retail's target choice (`sub_82E30F58`) from `from` facing `dir`.
pub fn choose_target(mesh: &NavMesh, params: &WanderParams, from: NavPoint, dir: [f32; 2], skip_long: bool) -> (Vec3, Option<NavPoint>) {
    choose_target_clear(mesh, params, from, dir, skip_long, None)
}

/// [`choose_target`] with resting obstacles cut out (fix 11).
pub fn choose_target_clear(mesh: &NavMesh, params: &WanderParams, from: NavPoint, dir: [f32; 2], skip_long: bool, obstacles: Option<&NavObstacles>) -> (Vec3, Option<NavPoint>) {
    if !skip_long {
        if let Some(p) = probe_fan_clear(mesh, from, dir, &params.long, obstacles) {
            return (p.position, Some(p));
        }
    }
    if let Some(p) = probe_fan_clear(mesh, from, dir, &params.short, obstacles) {
        return (p.position, Some(p));
    }
    let step = params.fallback_min.max(params.mover_value * params.fallback_scale);
    let t = [from.position[0] + dir[0] * step, from.position[1], from.position[2] + dir[1] * step];
    (t, mesh.locate(t))
}

impl PedNav {
    pub fn set_route(&mut self, route: Option<PedRoute>) {
        self.route = route;
        self.route_index = 0;
        self.target = None;
        self.corners.clear();
    }

    fn plan(&mut self, mesh: &NavMesh, params: &WanderParams, here: NavPoint, heading: f32, obstacles: Option<&NavObstacles>) {
        // Reverse (retail goal event 4, `82E31498`: the next target direction = old target to ped; pm3 / b158 item 9).
        let dir = match (std::mem::take(&mut self.reverse), self.target) {
            (true, Some(t)) if dist_xz(t, here.position) > 1e-3 => {
                let (dx, dz) = (here.position[0] - t[0], here.position[2] - t[2]);
                let l = (dx * dx + dz * dz).sqrt();
                [dx / l, dz / l]
            }
            (true, _) => {
                let f = forward(heading);
                [-f[0], -f[1]]
            }
            _ => forward(heading),
        };
        let skip = std::mem::take(&mut self.skip_long);
        let (target, located) = if let Some(route) = &self.route {
            if route.points.is_empty() {
                (here.position, Some(here))
            } else {
                if self.route_index >= route.points.len() {
                    self.route_index = if route.looped { 0 } else { route.points.len() - 1 };
                }
                let mut t = route.points[self.route_index];
                self.route_index += 1;
                // A route point inside a cut (a plugin waypoint just in front of its own prop): retail snaps the bot
                // destination onto the cut mesh within 0.5 m (b302), so the path ends at the nearest free point.
                if let Some(s) = obstacles.and_then(|o| o.snap_out(t, mesh.agent[1])) {
                    t = s;
                }
                (t, mesh.locate(t))
            }
        } else {
            choose_target_clear(mesh, params, here, dir, skip, obstacles)
        };
        self.targets_chosen += 1;
        self.target = Some(target);
        self.next = 0;
        self.stall = None;
        self.corners = match located.and_then(|to| mesh.find_path(here, to)) {
            Some(c) => c,
            None => {
                // No path (retail: a NavPower failure event 1-3 sets `+32`): go straight and
                // take the short fan next time.
                self.skip_long = true;
                vec![target]
            }
        };
        if let Some(o) = obstacles {
            // NavPower plans on the cut mesh: bend the path round resting obstacles.
            self.corners = o.detour(mesh, here.position, &self.corners, mesh.agent[1]);
            self.obstacle_version = o.version;
        }
        // The path search or a detour can repeat a corner; the pass test needs a clear line to a DIFFERENT next one.
        self.corners.dedup_by(|x, y| dist_xz(*x, *y) < 1e-3);
    }

    /// Re-path from `here` to the SAME target (retail re-requests the bot's destination, `82E957F8`; b315, b332): the
    /// target and a route's index stay. No path: the corners are dropped and the long fan skipped, so the arrival
    /// branch picks a new target (retail: a failed request ends the path, then the bot is done, b190).
    pub fn repath(&mut self, mesh: &NavMesh, here: NavPoint, obstacles: Option<&NavObstacles>) {
        let Some(target) = self.target else { return };
        self.repaths += 1;
        self.next = 0;
        self.stall = None;
        match mesh.locate(target).and_then(|to| mesh.find_path(here, to)) {
            Some(c) => {
                self.corners = c;
                if let Some(o) = obstacles {
                    self.corners = o.detour(mesh, here.position, &self.corners, mesh.agent[1]);
                    self.obstacle_version = o.version;
                }
                self.corners.dedup_by(|x, y| dist_xz(*x, *y) < 1e-3);
            }
            None => {
                self.corners.clear();
                self.skip_long = true;
            }
        }
    }

    /// One world tick. `position` / `heading` / `state` are the body's; `others` the nearby
    /// peds (any order); `me` this ped's order key.
    #[allow(clippy::too_many_arguments)]
    pub fn step(
        &mut self,
        mesh: &NavMesh,
        params: &WanderParams,
        rule: CrosswalkRule,
        signals: &dyn WalkSignals,
        me: u64,
        position: Vec3,
        heading: f32,
        state: Locomotion,
        others: &[Neighbour],
        dt: f32,
    ) -> NavOutput {
        self.step_avoiding(mesh, params, rule, signals, me, position, heading, state, others, None, dt)
    }

    /// [`Self::step`] with the map's dynamic obstacles (fix 11): targets inside a resting obstacle
    /// do not fit, paths bend round them, and when a cut changes the rest of the path is checked
    /// again (retail: NavPower re-plans on the re-cut mesh).
    #[allow(clippy::too_many_arguments)]
    pub fn step_avoiding(
        &mut self,
        mesh: &NavMesh,
        params: &WanderParams,
        rule: CrosswalkRule,
        signals: &dyn WalkSignals,
        me: u64,
        position: Vec3,
        heading: f32,
        state: Locomotion,
        others: &[Neighbour],
        obstacles: Option<&NavObstacles>,
        dt: f32,
    ) -> NavOutput {
        let here = self.poly.and_then(|k| mesh.point_on(k, position)).or_else(|| mesh.locate(position));
        let Some(here) = here else {
            self.waiting = NavWait::OffMesh;
            return NavOutput { intent: Intent::Idle, turn: 0.0, waiting: NavWait::OffMesh, turn_degrees: 0, steer: 0.0 };
        };
        // Ours (the still timer's values, `82E97118`, which retail runs in bot mode 4 only, b336): a walking ped still for
        // `stuck_seconds` re-paths to the same target, once per still spell. Retail has no give-up (b190, b332);
        // `give_up_seconds` is our NOT RETAIL safety net (0 = off).
        let mut give_up = false;
        if matches!(state, Locomotion::Walk | Locomotion::Start) {
            self.stuck = match self.stuck {
                Some((a, t)) if dist_xz(a, position) < params.stuck_distance => {
                    let t = t + dt;
                    if params.give_up_seconds > 0.0 && t >= params.give_up_seconds {
                        give_up = true;
                        None
                    } else {
                        if t >= params.stuck_seconds && !self.still_fired {
                            self.still_fired = true;
                            self.repath_due = true;
                        }
                        Some((a, t))
                    }
                }
                _ => Some((position, 0.0)),
            };
        } else {
            self.stuck = None;
        }
        if self.stuck.is_none_or(|(_, t)| t == 0.0) {
            self.still_fired = false;
        }
        if std::mem::take(&mut self.repath_due) && !give_up {
            self.repath(mesh, here, obstacles);
        }
        // Arrival / no plan: choose the next target (the bot's state 1).
        let arrived = self.corners.is_empty() || (self.next + 1 >= self.corners.len() && self.corners.last().is_some_and(|c| dist_xz(*c, position) < params.corner_radius));
        if give_up {
            self.reverse = true;
            self.skip_long = true;
            self.stuck_count += 1;
        }
        if arrived || give_up {
            self.plan(mesh, params, here, heading, obstacles);
        } else if let Some(o) = obstacles.filter(|o| o.version != self.obstacle_version) {
            let rest = self.corners.split_off(self.next.min(self.corners.len()));
            self.corners = o.detour(mesh, position, &rest, mesh.agent[1]);
            self.corners.dedup_by(|x, y| dist_xz(*x, *y) < 1e-3);
            self.next = 0;
            self.obstacle_version = o.version;
        }
        // A corner counts as passed within the corner radius once the next one is in straight
        // reach over the surface; before that the ped keeps walking to the corner (the corners
        // are mesh boundary vertices, so cutting one early walks into the boundary: fix 17).
        let at = NavPoint { poly: here.poly, position };
        while self.next + 1 < self.corners.len() && dist_xz(self.corners[self.next], position) < params.corner_radius && mesh.clear_line(at, self.corners[self.next + 1]) {
            self.next += 1;
        }
        // Ours (NavPower slides past such a corner, not decoded): a walking ped inside the corner radius whose distance
        // to the corner stops shrinking for `stall_seconds` is at it and walks on to the next corner (a boundary vertex
        // it cannot step onto, with the next corner out of straight reach; 2026-10-16).
        if self.next + 1 < self.corners.len() && matches!(state, Locomotion::Walk | Locomotion::Start) && dist_xz(self.corners[self.next], position) < params.corner_radius {
            let d = dist_xz(self.corners[self.next], position);
            self.stall = match self.stall {
                Some((k, best, t)) if k == self.next && d >= best - 0.01 => Some((k, best, t + dt)),
                _ => Some((self.next, d, 0.0)),
            };
            if self.stall.is_some_and(|(_, _, t)| t >= params.stall_seconds) {
                self.next += 1;
                self.stall = None;
            }
        } else {
            self.stall = None;
        }
        let corner = self.corners[self.next.min(self.corners.len() - 1)];
        let to = [corner[0] - position[0], corner[2] - position[2]];
        let desired = if to[0] * to[0] + to[1] * to[1] > 1e-8 { heading_of(to) } else { heading };
        let mut error = wrap(desired - heading);
        // The turn bucket comes from the path direction (`82E2ABF8`), not the avoidance steering below.
        let bucket = turn_bucket(error, &params.turn_edges);
        let left = error > 0.0;

        // Avoidance: steer away from close peds; yield to a lower id inside the space ahead.
        let radius = mesh.agent[1].max(0.0);
        let f = forward(heading);
        let mut push = [0.0f32; 2];
        let to_len = (to[0] * to[0] + to[1] * to[1]).sqrt();
        let way = if to_len > 1e-4 { [to[0] / to_len, to[1] / to_len] } else { f };
        let mut yield_to = false;
        // The closest ped I have priority over standing in my way (I walk round it).
        let mut blocker: Option<(f32, [f32; 2])> = None;
        for o in others {
            if o.order == me {
                continue;
            }
            let d = [o.position[0] - position[0], o.position[2] - position[2]];
            let dist = (d[0] * d[0] + d[1] * d[1]).sqrt();
            if dist < 1e-4 || (o.position[1] - position[1]).abs() > 2.0 {
                continue;
            }
            // Measured along the way to the corner (not the current heading, which a pivot changes).
            let ahead = (d[0] * way[0] + d[1] * way[1]) / dist;
            if dist < params.yield_distance && ahead > std::f32::consts::FRAC_1_SQRT_2 && o.order < me {
                yield_to = true;
            }
            if dist < 2.0 * radius + params.yield_distance * 0.5 && ahead > 0.5 && o.order > me && blocker.is_none_or(|(b, _)| dist < b) {
                blocker = Some((dist, d));
            }
            if dist < 2.0 * radius + params.yield_distance * 0.5 {
                let w = (2.0 * radius + params.yield_distance * 0.5 - dist) / dist;
                push = [push[0] - d[0] * w, push[1] - d[1] * w];
            }
        }
        if let Some((_, d)) = blocker {
            // Side-step: the perpendicular to the blocker closer to the corner (ties: +90
            // degrees), on walkable ground (one agent diameter of room); with no room either
            // side (a narrow strip) the ped yields instead of walking into the boundary (fix 17).
            let (a, b) = ([d[1], -d[0]], [-d[1], d[0]]);
            let (first, second) = if a[0] * to[0] + a[1] * to[1] > b[0] * to[0] + b[1] * to[1] { (a, b) } else { (b, a) };
            let room = |v: [f32; 2]| {
                let l = (v[0] * v[0] + v[1] * v[1]).sqrt().max(1e-6);
                let reach = 2.0 * radius;
                mesh.clear_line(at, [position[0] + v[0] / l * reach, position[1], position[2] + v[1] / l * reach])
            };
            match [first, second].into_iter().find(|v| room(*v)) {
                Some(side) => error = wrap(heading_of(side) - heading),
                None => yield_to = true,
            }
        } else if push != [0.0, 0.0] {
            let steer = [to[0] + push[0] * 2.0, to[1] + push[1] * 2.0];
            error = wrap(heading_of(steer) - heading);
        }
        // Ours: while the avoidance stand-in steers, no turn is posted and the stand-in's stop / pivot rule runs (it
        // walks the ped off its path on purpose; retail's avoidance is NavPower's, not decoded).
        let avoiding = blocker.is_some() || push != [0.0, 0.0];

        // Crosswalk rule (mods): wait before the first road polygon while the walk light is not
        // green.
        let mut wait = NavWait::None;
        if rule == CrosswalkRule::WalkSignal && mesh.polys[here.poly as usize].area != super::nav::AREA_ROAD {
            let probe = [position[0] + f[0] * params.corner_radius * 2.0, position[1], position[2] + f[1] * params.corner_radius * 2.0];
            if let Some(next) = mesh.locate(probe) {
                if mesh.polys[next.poly as usize].area == super::nav::AREA_ROAD {
                    if let Some(light) = signals.walk_light(probe) {
                        if light != Light::Green {
                            wait = NavWait::Crosswalk;
                        }
                    }
                }
            }
        }
        if yield_to && wait == NavWait::None {
            wait = NavWait::Yield;
        }
        if wait != NavWait::None {
            self.waited += dt;
            if wait == NavWait::Yield && self.waited > params.yield_patience {
                // Re-path to the same target (retail never gives the target up, b332; the wait is ours, b335).
                self.waited = 0.0;
                self.repath_due = true;
            }
            self.waiting = wait;
            return NavOutput { intent: Intent::Idle, turn: 0.0, waiting: wait, turn_degrees: 0, steer: 0.0 };
        }
        self.waited = 0.0;
        self.waiting = NavWait::None;
        let turn = error.clamp(-params.steer_rate * dt, params.steer_rate * dt);
        let out = |intent, turn| NavOutput { intent, turn, waiting: NavWait::None, turn_degrees: 0, steer: turn };
        // A posted turn clears the steering (`82E2A500`: command +0x64 = 0) and the clip turns the ped.
        let post = |degrees| NavOutput { intent: if left { Intent::TurnLeft } else { Intent::TurnRight }, turn: 0.0, waiting: NavWait::None, turn_degrees: degrees, steer: turn };
        match (state, bucket) {
            // Turning: the clip's root yaw turns the ped until the turn completes; then a walk turn walks on.
            (Locomotion::TurnLeft | Locomotion::TurnRight, _) => out(Intent::Idle, 0.0),
            (Locomotion::WalkTurnLeft | Locomotion::WalkTurnRight, _) => out(Intent::Walk, 0.0),
            // Standing, stopping or walking past the first edge: the bucket's turn (stand or walk clip).
            (Locomotion::Idle | Locomotion::Stop | Locomotion::Walk, Some(degrees)) if !avoiding => post(degrees),
            // Avoiding while standing (ours): pivot, walk once roughly facing the way.
            (Locomotion::Idle | Locomotion::Stop, _) if avoiding => out(if error.abs() <= params.walk_heading { Intent::Walk } else { Intent::Idle }, turn),
            // Below the first edge: walk and steer.
            (Locomotion::Idle | Locomotion::Stop | Locomotion::Walk | Locomotion::Start, _) if !avoiding => out(Intent::Walk, turn),
            // Running (no run turns yet) or avoiding while walking (ours): a way far off to the side stops the ped to pivot.
            _ => out(if error.abs() > params.stop_heading { Intent::Idle } else { Intent::Walk }, turn),
        }
    }
}

/// Keep a move on walkable ground: the step from `from` to `to` moves over the polygons linked
/// to the one under `from` ([`NavMesh::move_along`]: across tile seams, sliding along boundary
/// edges, never onto an unconnected layer); the position comes back on the surface. A body off
/// the mesh stays.
pub fn constrain_step(mesh: &NavMesh, from: Vec3, to: Vec3) -> (Vec3, bool) {
    let (p, _, ok) = constrain_move(mesh, None, from, to);
    (p, ok)
}

/// [`constrain_step`] from a known polygon (`poly`, e.g. [`PedNav::poly`]; `None` or a polygon
/// `from` is not on = locate). Returns the position, its polygon and whether the body is on the
/// mesh (fix 17).
pub fn constrain_move(mesh: &NavMesh, poly: Option<u32>, from: Vec3, to: Vec3) -> (Vec3, Option<u32>, bool) {
    let here = poly.and_then(|k| mesh.point_on(k, from)).or_else(|| mesh.locate(from));
    match here {
        Some(h) => {
            let m = mesh.move_along(NavPoint { poly: h.poly, position: from }, to);
            (m.position, Some(m.poly), true)
        }
        None => (from, None, false),
    }
}

/// Whether a ped may step from `from` to `to`: afterwards no other ped is closer than twice the
/// agent radius (ours: the NavPower agent radius 0.35 [data]), or the step at least does not
/// close in on one that already is (peds that spawn overlapping can still separate). A step
/// that fails is not taken.
pub fn separation_ok(from: Vec3, to: Vec3, me: u64, others: &[Neighbour], radius: f32) -> bool {
    separation_blocker(from, to, me, others, radius).is_none()
}

/// The first neighbour (index into `others`) that refuses the step: the two bodies touch there.
pub fn separation_blocker(from: Vec3, to: Vec3, me: u64, others: &[Neighbour], radius: f32) -> Option<usize> {
    others.iter().position(|o| {
        if o.order == me || (o.position[1] - to[1]).abs() > 2.0 {
            return false;
        }
        let after = dist_xz(o.position, to);
        !(after >= 2.0 * radius || after >= dist_xz(o.position, from))
    })
}

/// The mod crosswalk rule as a step check: under [`CrosswalkRule::WalkSignal`] a step from off
/// the road onto a road polygon is only taken while the walk light there is green (or there is
/// no signalled crossing). Always true under the retail default (`Off`).
pub fn crosswalk_ok(mesh: &NavMesh, rule: CrosswalkRule, signals: &dyn WalkSignals, from: Vec3, to: Vec3) -> bool {
    if rule == CrosswalkRule::Off {
        return true;
    }
    let area = |p: Vec3| mesh.locate(p).map(|x| mesh.polys[x.poly as usize].area);
    if area(from) == Some(super::nav::AREA_ROAD) || area(to) != Some(super::nav::AREA_ROAD) {
        return true;
    }
    signals.walk_light(to).is_none_or(|l| l == Light::Green)
}
