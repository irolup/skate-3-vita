//! The NPC skater's obstacle avoider (retail ObstacleAvoider, AI controller +80), pure.
//! Retail mechanism (8246D560 and callees): docs/hails-additions/26e-living-world-skater-ai.md#obstacle-avoider

use super::Vec3;

/// Retail obstacle type codes (descriptor `+164`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ObstacleKind {
    Pedestrian = 2,
    Skater = 3,
    Vehicle = 4,
    Prop = 5,
}

impl ObstacleKind {
    /// Vehicles and props block the path with their box corners, the others as circles
    /// (`sub_82462C18`: types 4..5 -> `sub_82462448`).
    pub fn is_box(self) -> bool {
        matches!(self, ObstacleKind::Vehicle | ObstacleKind::Prop)
    }
}

/// Retail avoider values, data-driven (mod tuning domain `npc_avoid`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AvoidSettings {
    /// The global avoidance switch (retail byte `+608` of a game object, clear = on).
    pub enabled: bool,
    /// Gather radii, metres: skaters 64 (`0x820ED958`), peds 8 (`0x82099250`), vehicles 20
    /// (`0x820996EC`), props 16 (`0x820C2054`).
    pub radius_skater: f32,
    pub radius_pedestrian: f32,
    pub radius_vehicle: f32,
    pub radius_prop: f32,
    /// Entry add cones (`0x822F91B4` 40 deg, `0x822F9554` 88 deg) and the near distance for the
    /// wide cone (`0x8208EB60` 1.8 m).
    pub cone: f32,
    pub wide_cone: f32,
    pub wide_cone_distance: f32,
    /// Entries per tick (avoider `+5968` < 16).
    pub max_entries: usize,
    /// The skater's avoidance radius ([data] `ai_skater` `8EF4FB9D11A9358A` = 0.5).
    pub skater_radius: f32,
    /// Speed margin of the floor / cap ([data] `ai_skater` `C373BAC23C5881FA` = 1.0).
    pub speed_margin: f32,
    /// Close blocker gaps: 2.2 m (`0x822570E0`), or 6.0 m (`0x8208F74C`) inside 18 deg
    /// (`0x822F9284`); props 5.0 m (`0x821F1790`) both.
    pub stop_gap: f32,
    pub stop_gap_far: f32,
    pub stop_cone: f32,
    pub stop_gap_prop: f32,
    /// Side-on test (`0x822F94F0`, 36 deg, after folding to 0..90 deg).
    pub side_on_angle: f32,
    /// The floor is dropped when it exceeds the own speed by more than this (`0x821F1790`, 5.0).
    pub floor_headroom: f32,
    /// Skitch candidate: both cosines above this (`0x8208824C`, 0.8).
    pub skitch_cos: f32,
    /// Ticks without a new skitch target after losing one (`sub_824650A0`, 60).
    pub skitch_cooldown_ticks: u32,
    /// Low prop height (`0x8208824C`, 0.8 m), mode 6 time (`0x8231A844`, 1.0 s), mode 7 cap and
    /// time (`0x820641A8` 0.1, `0x822249B4` 1.5 s).
    pub low_prop_height: f32,
    pub low_prop_time: f32,
    pub step_off_cap: f32,
    pub step_off_time: f32,
}

impl Default for AvoidSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            radius_skater: 64.0,
            radius_pedestrian: 8.0,
            radius_vehicle: 20.0,
            radius_prop: 16.0,
            cone: 0.698_132,
            wide_cone: 1.535_89,
            wide_cone_distance: 1.8,
            max_entries: 16,
            skater_radius: 0.5,
            speed_margin: 1.0,
            stop_gap: 2.2,
            stop_gap_far: 6.0,
            stop_cone: 0.314_159,
            stop_gap_prop: 5.0,
            side_on_angle: 0.628_319,
            floor_headroom: 5.0,
            skitch_cos: 0.8,
            skitch_cooldown_ticks: 60,
            low_prop_height: 0.8,
            low_prop_time: 1.0,
            step_off_cap: 0.1,
            step_off_time: 1.5,
        }
    }
}

impl AvoidSettings {
    pub fn radius(&self, kind: ObstacleKind) -> f32 {
        match kind {
            ObstacleKind::Pedestrian => self.radius_pedestrian,
            ObstacleKind::Skater => self.radius_skater,
            ObstacleKind::Vehicle => self.radius_vehicle,
            ObstacleKind::Prop => self.radius_prop,
        }
    }
}

/// One obstacle near the NPC (descriptor of the gatherers).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Obstacle {
    /// Stable id (living-world id, prop id, player 0).
    pub id: u64,
    pub kind: ObstacleKind,
    /// Centre (boxes) or feet (circles), world.
    pub position: Vec3,
    pub velocity: Vec3,
    /// Box axes (unit; x side, y up, z forward).
    pub axes: [Vec3; 3],
    /// Full extents along the axes (descriptor `+148/+152/+156`: length along z, width along x,
    /// height; retail halves them where it uses them).
    pub length: f32,
    pub width: f32,
    pub height: f32,
}

/// Where a point falls on the NPC's path (`sub_82462C18`: node interpolation `sub_82459F68`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PathPoint {
    pub point: Vec3,
    /// Unit path direction there.
    pub direction: Vec3,
    /// Path width left / right of the line there, metres (node bytes `+0x25/+0x26` / 50,
    /// `sub_82F71580`).
    pub width_left: f32,
    pub width_right: f32,
    /// Node index and fraction (orders entries along the path, entry `+176/+180`).
    pub node: u32,
    pub t: f32,
}

/// The NPC skater as the avoider sees it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AvoidSelf {
    pub position: Vec3,
    pub velocity: Vec3,
    /// Unit facing.
    pub forward: Vec3,
    /// Lateral position on the path in path widths (controller `+864`; 0 = on the line).
    pub lateral: f32,
}

/// Retail's avoider modes (`+6000`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum AvoidMode {
    #[default]
    None = 0,
    /// Speed up to the floor to cross before the obstacle arrives.
    SpeedUp = 1,
    /// Slow to the cap (0 = stop).
    SlowDown = 2,
    /// The path is split by obstacles: steer into a gap (gap choice not ported).
    Steer = 3,
    /// A car ahead moving the same way: steer to it and raise `GrabWorld` (skitch).
    Skitch = 4,
    /// A low prop about to be hit (consumer not read; [inferred] ollie over it).
    LowProp = 6,
    /// Stopped in front of a prop (read by the path-end hand-over; [inferred] step off).
    StepOff = 7,
}

impl AvoidMode {
    pub fn name(self) -> &'static str {
        match self {
            AvoidMode::None => "none",
            AvoidMode::SpeedUp => "speed_up",
            AvoidMode::SlowDown => "slow_down",
            AvoidMode::Steer => "steer",
            AvoidMode::Skitch => "skitch",
            AvoidMode::LowProp => "low_prop",
            AvoidMode::StepOff => "step_off",
        }
    }
}

/// One kept obstacle (the 368-byte entry, the fields the port needs).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Entry {
    pub id: u64,
    pub kind: ObstacleKind,
    pub distance: f32,
    pub angle: f32,
    /// Closing (`+359`), time to contact (`+352`, `f32::MAX` when not closing).
    pub closing: bool,
    pub time_to_contact: f32,
    /// Blocks the path (`+363`) and the blocked lateral interval (`+336/+340`, path widths).
    pub blocks: bool,
    pub lateral: [f32; 2],
    /// Height above the path (`+344`).
    pub height: f32,
    pub side_on: bool,
    /// Close blocker (`+360`).
    pub close: bool,
    pub floor: Option<f32>,
    pub cap: Option<f32>,
    /// Skitch candidate (`+365`).
    pub skitch: bool,
    /// Path order key.
    pub order: (u32, f32),
    /// Where the obstacle falls on the path (entry `+224`).
    pub path: Option<PathPoint>,
}

/// Persistent avoider state (skitch cooldown, `+5980/+6009`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AvoidState {
    pub skitch_cooldown: u32,
    pub had_skitch: bool,
}

/// The avoider's answer for this tick.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AvoidOutput {
    pub mode: AvoidMode,
    pub own_speed: f32,
    /// `+5988` / `+6004`.
    pub floor: f32,
    pub floor_valid: bool,
    /// `+5992` / `+6005` (`f32::MAX` = no cap).
    pub cap: f32,
    pub cap_valid: bool,
    /// The steer target and skitch target (entry ids).
    pub steer_target: Option<u64>,
    pub skitch_target: Option<u64>,
    /// `+5996`: where to steer across the path, in path widths (modes 3 and 4).
    pub lateral: f32,
    /// The free gaps of the path (`+0..+64`).
    pub gaps: Vec<[f32; 2]>,
    pub entries: Vec<Entry>,
}

impl AvoidOutput {
    /// The speed the record builder may use (`sub_82470830`): mode 1 floors, mode 2 caps (unless
    /// `bypass_cap`, controller `+6007`).
    pub fn shape_speed(&self, speed: f32, bypass_cap: bool) -> f32 {
        match self.mode {
            AvoidMode::SpeedUp => speed.max(self.floor),
            AvoidMode::SlowDown if !bypass_cap => speed.min(self.cap),
            _ => speed,
        }
    }
}

fn sub(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn add(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn scale(a: Vec3, s: f32) -> Vec3 {
    [a[0] * s, a[1] * s, a[2] * s]
}
fn dot(a: Vec3, b: Vec3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn len(a: Vec3) -> f32 {
    dot(a, a).sqrt()
}
/// Right of a direction (y up): the path's lateral axis, d x up (`sub_82454798`, b527: +0x25 on the negative side).
fn right_of(d: Vec3) -> Vec3 {
    let r = [-d[2], 0.0, d[0]];
    let l = len(r);
    if l > 1e-6 {
        scale(r, 1.0 / l)
    } else {
        [1.0, 0.0, 0.0]
    }
}
/// Signed angle from `a` to `b` about up, in -pi..pi (`sub_8296EC98`).
fn yaw_between(a: Vec3, b: Vec3) -> f32 {
    let ya = a[0].atan2(a[2]);
    let yb = b[0].atan2(b[2]);
    let mut d = yb - ya;
    while d > core::f32::consts::PI {
        d -= core::f32::consts::TAU;
    }
    while d < -core::f32::consts::PI {
        d += core::f32::consts::TAU;
    }
    d
}

/// Entry add `sub_82463A40`: the cone test.
pub fn keeps(s: &AvoidSettings, me: &AvoidSelf, o: &Obstacle, distance: f32) -> bool {
    let offset = sub(o.position, me.position);
    if dot(offset, offset) < 0.0001 {
        return false;
    }
    let angle = yaw_between(me.forward, offset).abs();
    let reach = o.length.max(o.width) * 0.5;
    angle < s.cone || (angle < s.wide_cone && distance - reach < s.wide_cone_distance)
}

/// The free lateral intervals left on the path after removing blocked ones (`sub_82461DC8` on
/// the avoider's interval set, which starts as [-1, 1]).
pub fn free_gaps(blocked: &[[f32; 2]]) -> Vec<[f32; 2]> {
    let mut free = vec![[-1.0f32, 1.0f32]];
    for b in blocked {
        let mut next = Vec::new();
        for f in free {
            if b[1] <= f[0] || b[0] >= f[1] {
                next.push(f);
                continue;
            }
            if b[0] > f[0] {
                next.push([f[0], b[0]]);
            }
            if b[1] < f[1] {
                next.push([b[1], f[1]]);
            }
        }
        free = next;
    }
    free
}

/// Fill one entry (`sub_824636B0`). `path` projects a world point onto the NPC's path.
pub fn fill(s: &AvoidSettings, me: &AvoidSelf, o: &Obstacle, distance: f32, path: &dyn Fn(Vec3) -> Option<PathPoint>) -> Entry {
    let offset = sub(o.position, me.position);
    let angle = yaw_between(me.forward, offset);
    // Side-on: the obstacle's heading against our motion, folded to 0..90 deg (`+364`).
    let mut rel = yaw_between(me.velocity, o.axes[2]).abs();
    if rel >= core::f32::consts::FRAC_PI_2 {
        rel = core::f32::consts::PI - rel;
    }
    let side_on = rel.abs() >= s.side_on_angle;
    // Extent along our motion (`364 ? +312 : +308`) and across it (the reverse).
    let along = if side_on { o.width } else { o.length };
    let across = if side_on { o.length } else { o.width };
    let mut e = Entry {
        id: o.id,
        kind: o.kind,
        distance,
        angle,
        closing: false,
        time_to_contact: f32::MAX,
        blocks: false,
        lateral: [0.0, 0.0],
        height: o.height,
        side_on,
        close: false,
        floor: None,
        cap: None,
        skitch: false,
        order: (u32::MAX, 0.0),
        path: None,
    };
    // Path interval (`sub_82462C18`).
    if let Some(p) = path(o.position) {
        e.order = (p.node, p.t);
        e.path = Some(p);
        if o.kind.is_box() {
            box_interval(s, &mut e, o, &p, path);
        } else {
            circle_interval(s, &mut e, o, &p);
        }
    }
    // Closing (`sub_82462D00`): distance after 1/60 s with both velocities.
    let next = len(sub(add(o.position, scale(o.velocity, 1.0 / 60.0)), add(me.position, scale(me.velocity, 1.0 / 60.0))));
    let rate = (distance - next) * 60.0;
    if rate > 0.0 {
        e.closing = true;
        let gap = distance - (along * 0.5 + s.skater_radius);
        e.time_to_contact = if gap > 0.0 { gap / rate } else { 0.0 };
    }
    // Skitch candidate (`sub_82462ED0`), vehicles only.
    if o.kind == ObstacleKind::Vehicle {
        let vo = len(o.velocity);
        let vm = len(me.velocity);
        if vo >= 0.0001 && vm >= 0.0001 {
            let rel_pos = len(offset);
            if rel_pos > 1e-6 {
                let ahead = dot(me.velocity, offset) / (vm * rel_pos);
                let same = dot(o.velocity, me.velocity) / (vo * vm);
                e.skitch = ahead.clamp(-1.0, 1.0) > s.skitch_cos && same.clamp(-1.0, 1.0) > s.skitch_cos;
            }
        }
    }
    if e.closing && e.blocks {
        speed_limits(s, &mut e, me, o, along, across);
    }
    // Close blocker (`+360`).
    let gap = distance - along * 0.5;
    let (near, far) = if o.kind == ObstacleKind::Prop { (s.stop_gap_prop, s.stop_gap_prop) } else { (s.stop_gap, s.stop_gap_far) };
    e.close = gap < near || (gap < far && angle.abs() < s.stop_cone);
    e
}

/// `sub_824629A8`: a circle of radius extent / 2 + skater radius across the path.
fn circle_interval(s: &AvoidSettings, e: &mut Entry, o: &Obstacle, p: &PathPoint) {
    let along = if e.side_on { o.width } else { o.length };
    let c = along * 0.5 + s.skater_radius;
    let w = p.width_left.max(p.width_right);
    let d = sub(o.position, p.point);
    if dot(d, d) >= (w + c) * (w + c) {
        e.blocks = false;
        return;
    }
    e.blocks = true;
    let off = dot(d, right_of(p.direction));
    let (lo, hi) = (off - c, off + c);
    e.lateral = if w > 0.0001 { [lo / w, hi / w] } else { [lo, hi] };
    e.height = o.height;
}

/// `sub_82462448`: the 8 box corners projected across the path, each widened by the skater
/// radius and divided by the path width on its side.
fn box_interval(s: &AvoidSettings, e: &mut Entry, o: &Obstacle, p: &PathPoint, path: &dyn Fn(Vec3) -> Option<PathPoint>) {
    let reach = o.length.max(o.width).max(o.height) * 0.5 + s.skater_radius;
    let d = sub(o.position, p.point);
    let side = if dot(d, right_of(p.direction)) > 0.0 { p.width_right } else { p.width_left };
    if dot(d, d) >= (side + reach) * (side + reach) {
        e.blocks = false;
        return;
    }
    let (mut lo, mut hi, mut top) = (10_000.0f32, -10_000.0f32, 0.0f32);
    let mut order = e.order;
    let half = [o.width * 0.5, o.height * 0.5, o.length * 0.5];
    for k in 0..8 {
        let sx = if k & 1 == 0 { -1.0 } else { 1.0 };
        let sy = if k & 2 == 0 { -1.0 } else { 1.0 };
        let sz = if k & 4 == 0 { -1.0 } else { 1.0 };
        let corner = add(o.position, add(scale(o.axes[0], sx * half[0]), add(scale(o.axes[1], sy * half[1]), scale(o.axes[2], sz * half[2]))));
        let Some(q) = path(corner) else { continue };
        if (q.node, q.t) < order {
            order = (q.node, q.t);
        }
        let rel = sub(corner, q.point);
        let up = rel[1];
        if !(-10_000.0..10_000.0).contains(&up) {
            continue;
        }
        let lat = dot(rel, right_of(q.direction));
        let (wl, wr) = (q.width_left, q.width_right);
        let low = if lat < 0.0 && wl > 0.0001 { (lat - s.skater_radius) / wl } else if wr > 0.0001 { (lat - s.skater_radius) / wr } else { lat - s.skater_radius };
        let high = if lat > 0.0 && wr > 0.0001 { (lat + s.skater_radius) / wr } else if wl > 0.0001 { (lat + s.skater_radius) / wl } else { lat + s.skater_radius };
        lo = lo.min(low);
        hi = hi.max(high);
        top = top.max(up);
    }
    e.order = order;
    e.blocks = lo <= hi;
    e.lateral = [lo, hi];
    e.height = top;
}

/// `sub_82463200`: the floor (cross before the obstacle arrives) and cap (arrive after it
/// passed) for a moving obstacle.
fn speed_limits(s: &AvoidSettings, e: &mut Entry, me: &AvoidSelf, o: &Obstacle, along: f32, across: f32) {
    let vo = len(o.velocity);
    let vm = len(me.velocity);
    if vo < 0.001 || vm < 0.001 {
        return;
    }
    // Distance from the obstacle to our line of motion.
    let t = dot(sub(o.position, me.position), me.velocity) / (vm * vm);
    let closest = add(me.position, scale(me.velocity, t));
    let d = len(sub(closest, o.position));
    if d < 0.001 {
        return;
    }
    let cos = (dot(o.velocity, me.velocity) / (vo * vm)).clamp(-1.0, 1.0);
    let sin = cos.acos().sin();
    if sin < 0.0001 {
        return;
    }
    let t_cross = |x: f32| x / (sin * vo);
    let crossing = add(o.position, scale(o.velocity, t_cross(d)));
    let to_cross = sub(crossing, me.position);
    let l = len(to_cross);
    let behind = l > 0.001 && dot(to_cross, me.velocity) < 0.0;
    let c = across * 0.5 + s.skater_radius;
    if d > c + 0.0001 || (behind && d > s.skater_radius) {
        let f = if behind { 0.0 } else { l / t_cross(d - c) };
        e.floor = Some(f + s.speed_margin);
    }
    let reach = (l - along * 0.5).max(0.0);
    e.cap = Some((reach / t_cross(d + c) - s.speed_margin).max(0.0));
}

/// One tick of the avoider for one NPC: gather (already filtered by radius and kind by the
/// caller or here), fill, aggregate and pick the mode.
pub fn evaluate(s: &AvoidSettings, state: &mut AvoidState, me: &AvoidSelf, obstacles: &[Obstacle], path: &dyn Fn(Vec3) -> Option<PathPoint>) -> AvoidOutput {
    let own = len(me.velocity);
    let mut out = AvoidOutput { own_speed: own, floor: 0.0, cap: f32::MAX, ..Default::default() };
    if s.enabled {
        // Gather order as retail: skaters, peds, vehicles, props; the caller passes them sorted
        // by id inside a kind (deterministic).
        for kind in [ObstacleKind::Skater, ObstacleKind::Pedestrian, ObstacleKind::Vehicle, ObstacleKind::Prop] {
            for o in obstacles.iter().filter(|o| o.kind == kind) {
                if out.entries.len() >= s.max_entries {
                    break;
                }
                let distance = len(sub(o.position, me.position));
                if distance > s.radius(kind) || !keeps(s, me, o, distance) {
                    continue;
                }
                out.entries.push(fill(s, me, o, distance, path));
            }
        }
    }
    aggregate(s, &mut out);
    pick_targets(s, state, &mut out);
    pick_mode(s, me, &mut out);
    out
}

/// `sub_82464FA8`.
fn aggregate(s: &AvoidSettings, out: &mut AvoidOutput) {
    out.floor_valid = true;
    out.cap_valid = true;
    for e in out.entries.iter().filter(|e| e.closing) {
        match e.floor {
            Some(f) => out.floor = out.floor.max(f),
            None => {
                out.floor_valid = false;
                if e.close {
                    out.floor = f32::MAX;
                }
            }
        }
        match e.cap {
            Some(c) => out.cap = out.cap.min(c),
            None => {
                out.cap_valid = false;
                if e.close {
                    out.cap = 0.0;
                }
            }
        }
    }
    if out.own_speed + s.floor_headroom < out.floor {
        out.floor_valid = false;
        out.floor = f32::MAX;
    }
}

fn earliest<'a>(it: impl Iterator<Item = &'a Entry>) -> Option<&'a Entry> {
    it.fold(None, |best: Option<&Entry>, e| match best {
        Some(b) if b.order <= e.order => Some(b),
        _ => Some(e),
    })
}

/// `sub_824650A0` (skitch target with a cooldown) and `sub_82465198` (steer target, free gaps).
fn pick_targets(s: &AvoidSettings, state: &mut AvoidState, out: &mut AvoidOutput) {
    let had = state.had_skitch;
    if state.skitch_cooldown > 0 {
        state.skitch_cooldown -= 1;
        out.skitch_target = None;
    } else {
        out.skitch_target = earliest(out.entries.iter().filter(|e| e.skitch)).map(|e| e.id);
    }
    state.had_skitch = out.skitch_target.is_some();
    if had && !state.had_skitch && state.skitch_cooldown == 0 {
        state.skitch_cooldown = s.skitch_cooldown_ticks;
    }
    let blocking: Vec<&Entry> = out.entries.iter().filter(|e| e.blocks).collect();
    out.steer_target = earliest(blocking.iter().copied()).map(|e| e.id);
    out.gaps = free_gaps(&blocking.iter().map(|e| e.lateral).collect::<Vec<_>>());
}

/// `sub_82465280`: the gap to steer into. On or near the path (|lateral| < 2 path widths,
/// `0x82060C50`) the gap nearest the skater's lateral position, else the gap whose centre on the
/// target's path point is nearest the skater; its centre (`0.5 x (lo + hi)`). 0 without gaps.
pub fn gap_choice(gaps: &[[f32; 2]], me: &AvoidSelf, target: Option<&PathPoint>) -> f32 {
    let mut best = (f32::MAX, 0.0f32);
    for g in gaps {
        let centre = (g[0] + g[1]) * 0.5;
        let d = if me.lateral.abs() < 2.0 {
            if me.lateral < g[0] {
                g[0] - me.lateral
            } else if me.lateral > g[1] {
                me.lateral - g[1]
            } else {
                0.0
            }
        } else {
            match target {
                Some(p) => {
                    let q = steer_point(p, centre);
                    let d = sub(q, me.position);
                    dot(d, d)
                }
                None => continue,
            }
        };
        if d < best.0 {
            best = (d, centre);
        }
    }
    best.1
}

/// `sub_82464E30` -> `sub_82454798`: the path point moved `lateral` path widths across the path
/// (the left width for negative values, the right one otherwise).
pub fn steer_point(p: &PathPoint, lateral: f32) -> Vec3 {
    let w = if lateral < 0.0 { p.width_left } else { p.width_right };
    add(p.point, scale(right_of(p.direction), lateral * w))
}

/// `sub_82465578`.
fn pick_mode(s: &AvoidSettings, me: &AvoidSelf, out: &mut AvoidOutput) {
    // `+6006` = free gap count != 0 (`0x82465268`: addic / subfe): steer whenever any lateral gap is free; slow
    // down only when blockers cover the whole width (b112).
    let path_split = !out.gaps.is_empty();
    out.lateral = 0.0;
    let Some(target) = out.steer_target.and_then(|id| out.entries.iter().find(|e| e.id == id)).copied() else {
        out.mode = AvoidMode::None;
        return;
    };
    if let Some(sk) = out.skitch_target.and_then(|id| out.entries.iter().find(|e| e.id == id)) {
        out.mode = AvoidMode::Skitch;
        out.lateral = ((sk.lateral[0] + sk.lateral[1]) * 0.5).clamp(-1.0, 1.0);
        return;
    }
    if path_split {
        out.mode = AvoidMode::Steer;
        out.lateral = gap_choice(&out.gaps, me, target.path.as_ref());
        return;
    }
    if out.floor_valid {
        out.mode = AvoidMode::SpeedUp;
        return;
    }
    out.mode = AvoidMode::SlowDown;
    let prop = target.kind == ObstacleKind::Prop;
    if prop && target.height < s.low_prop_height {
        out.mode = if target.time_to_contact < s.low_prop_time { AvoidMode::LowProp } else { AvoidMode::None };
    }
    if out.mode == AvoidMode::SlowDown && out.cap < s.step_off_cap && prop {
        out.mode = if target.time_to_contact < s.step_off_time { AvoidMode::StepOff } else { AvoidMode::None };
    }
}

#[cfg(test)]
#[path = "avoid_tests.rs"]
mod tests;
