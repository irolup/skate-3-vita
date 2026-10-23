//! Ped chases (doc 26 "Ped chases"): the per-type chase record and where a chasing ped runs.
//! Retail (TU3, evidence only; re-implemented; `.local/research/peds/b10-ped-chase-takedown.md`,
//! `b11-ped-intercept-takedown.md`, main checked the constants and the angle source):
//! - each ped type points at a `livingworld_entities_chase` record (escape distance, exhaustion,
//!   run speed, max lead time, ...); the chase manager holds the `global` record (predict angle);
//! - InterceptChasee Update (`826A3BF0`) asks the solver (`sub_82E3D4D8`) for a goal: the flat
//!   intercept time `t` with `|Q + V t - P| = speed t` (`sub_82E15CD0`); no solution or `t` not
//!   below the record's max lead time -> no new goal this tick. When the angle between
//!   (chaser - chasee) and the chasee's velocity lies strictly between the predict angle and
//!   180 deg minus it, the goal leads the chasee (`Q + V t`); else it is the chasee itself.
//!   The goal keeps the higher of the two heights, the nav runs at the run speed with a 0.5 m
//!   arrival radius (`0x8209975C`).
//! Which root `sub_82E15CD0` takes when there are two is not fully decoded: we take the smallest
//! positive one (inferred).

use super::super::Vec3;
use std::collections::BTreeMap;

/// Field hashes of `livingworld_entities_chase` (setup data keeps unknown ones as `Hash_...`).
pub mod field {
    pub const ESCAPE_DISTANCE: &str = "escape_distance";
    pub const EXHAUSTION_LIMIT: &str = "exhaustion_limit";
    pub const GIVE_UP_AFTER_TAKEDOWNS: &str = "give_up_after_takedowns";
    pub const ALERT_DISTANCE: &str = "alert_distance";
    pub const ALERT_TIMER: &str = "alert_timer";
    pub const UNREACHABLE_TIMER: &str = "unreachable_timer";
    /// The run speed (`CD65`), m/s.
    pub const RUN_SPEED: &str = "Hash_CD6575C0E03860E7";
    /// The intercept's max lead time, s (`47C9`, read by `826A3BF0`).
    pub const MAX_LEAD_TIME: &str = "Hash_47C96C544337FF32";
    /// The intercept's predict angle, degrees (`5D48`, the `global` record, `826B5D80`).
    pub const PREDICT_ANGLE: &str = "Hash_5D480F3FFC02E98A";
    /// Rest after exhaustion, s (`5960`, RestFromChase; peds 5, jocks 2).
    pub const REST_TIME: &str = "Hash_59604F998608081E";
    /// Investigate a lost chasee, s (`1AE2`, StartInvestigateTimer; peds 10, security 20).
    pub const INVESTIGATE_TIME: &str = "Hash_1AE29113978BE5AA";
    /// Secondary chasers' block rule (`82D99540`; all stock records 20 / 1.0 / 5.0 / 45).
    pub const BLOCK_RANGE: &str = "Hash_8BAE6F439F2B83E8";
    pub const BLOCK_RADIUS: &str = "Hash_F3E721EBA7F9F040";
    pub const BLOCK_CLOSING_SPEED: &str = "Hash_670A0B1FA8A3FE21";
    pub const BLOCK_CONE: &str = "Hash_E48BCCF9EF473450";
    /// How many chasers a chasee takes (`CEA5`, read as a byte, at most 25; `826ACAC8`).
    pub const MAX_CHASERS: &str = "Hash_CEA5D982FD05BE0B";
}

/// Chase end reasons (`sub_82BFEF30`, inferred to be the lookup behind the `reason` attribute;
/// an unknown name is -1).
pub mod reason {
    pub const DEFAULT: i32 = 0;
    pub const AGGRESSIVE_CAPTURE: i32 = 1;
    pub const RETURN_TO_PATROL_ZONE: i32 = 2;
    pub const LOST_INTEREST: i32 = 3;
    pub fn parse(name: &str) -> i32 {
        match name {
            "default" => DEFAULT,
            "aggressivecapture" => AGGRESSIVE_CAPTURE,
            "returntopatrolzone" => RETURN_TO_PATROL_ZONE,
            "lostinterest" => LOST_INTEREST,
            _ => -1,
        }
    }
}

/// A chase group's inline capacity (`8258E588`: 25 entries of 64 bytes).
pub const GROUP_CAPACITY: usize = 25;

/// One chase record, parents resolved (plain data; mods patch the setup data).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ChaseRecord {
    pub fields: BTreeMap<String, f32>,
}

impl ChaseRecord {
    pub fn get(&self, name: &str) -> Option<f32> {
        self.fields.get(name).copied()
    }
    /// `826AC668`: fallback 0.0 (`0x82165A10`).
    pub fn escape_distance(&self) -> f32 {
        self.get(field::ESCAPE_DISTANCE).unwrap_or(0.0)
    }
    /// `826A3BF0`: a record without the field reads 2^45 (`0x820D0850`), no limit.
    pub fn max_lead_time(&self) -> f32 {
        self.get(field::MAX_LEAD_TIME).unwrap_or(35_184_372_088_832.0)
    }
    /// `826A8810`: the unreachable wait, s; 0.0 without the field.
    pub fn unreachable_timer(&self) -> f32 {
        self.get(field::UNREACHABLE_TIMER).unwrap_or(0.0)
    }
    pub fn run_speed(&self) -> Option<f32> {
        self.get(field::RUN_SPEED)
    }
    /// `826ACAC8`: the byte value, at most 25; a chasee without a record takes 25.
    pub fn max_chasers(record: Option<&Self>) -> usize {
        record.and_then(|r| r.get(field::MAX_CHASERS)).map_or(GROUP_CAPACITY, |v| (v as u32 & 0xFF) as usize).min(GROUP_CAPACITY)
    }
    /// `82E3D7B0`: chasing time after which a ped is exhausted, s (`4686`).
    pub fn exhaustion_limit(&self) -> Option<f32> {
        self.get(field::EXHAUSTION_LIMIT)
    }
    /// Chaser vfn124 / vfn120: rest and investigate times, 0.0 without the field.
    pub fn rest_time(&self) -> f32 {
        self.get(field::REST_TIME).unwrap_or(0.0)
    }
    pub fn investigate_time(&self) -> f32 {
        self.get(field::INVESTIGATE_TIME).unwrap_or(0.0)
    }
    /// `826AD690`: takedowns after which a chaser gives up (`78E2`).
    pub fn give_up_after_takedowns(&self) -> Option<u32> {
        self.get(field::GIVE_UP_AFTER_TAKEDOWNS).map(|v| v as u32)
    }
}

/// A chasee's chase group (the chasee interface, base `8258E588`; on the player's actor and on
/// every ped): its chasers in join order (entry 0 = the primary chaser) and the group's end
/// reason. Host-owned plain data keyed by the chasee's stable id.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ChaseGroup {
    pub chasers: Vec<u64>,
    /// Each chaser's formation offset from the chasee (entry `+16`: chaser - chasee at the join,
    /// `82D96C38`; no later writer found).
    pub offsets: BTreeMap<u64, Vec3>,
    /// `group+1660`; `None` = -1, not ending.
    pub end_reason: Option<i32>,
}

/// What a chaser's graph reads of a group (copied per tick).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ChaseGroupInfo {
    pub count: usize,
    pub primary: Option<u64>,
    pub end_reason: Option<i32>,
    /// CanChaseeAddNewChaser: fewer chasers than the chasee's record allows and not ending.
    pub can_add: bool,
}

impl ChaseGroup {
    pub fn info(&self, max_chasers: usize) -> ChaseGroupInfo {
        ChaseGroupInfo { count: self.chasers.len(), primary: self.chasers.first().copied(), end_reason: self.end_reason, can_add: self.chasers.len() < max_chasers && self.end_reason.is_none() }
    }
    /// `82D96C38`: no duplicates, at most [`GROUP_CAPACITY`]; the first chaser resets the end
    /// reason.
    /// `82D96C38`: a join is refused when the group already holds `max_chasers` (vfn16, the chasee's record) or the
    /// capacity.
    pub fn add(&mut self, chaser: u64, offset: Vec3, max_chasers: usize) -> bool {
        if self.chasers.contains(&chaser) || self.chasers.len() >= max_chasers.min(GROUP_CAPACITY) {
            return false;
        }
        if self.chasers.is_empty() {
            self.end_reason = None;
        }
        self.chasers.push(chaser);
        self.offsets.insert(chaser, offset);
        true
    }
    /// `82D97400`: where a chaser holds formation, the chasee's position plus its offset.
    pub fn formation_point(&self, chaser: u64, chasee: Vec3) -> Vec3 {
        let o = self.offsets.get(&chaser).copied().unwrap_or([0.0; 3]);
        [chasee[0] + o[0], chasee[1] + o[1], chasee[2] + o[2]]
    }
    /// `82D96E88`: emptying the group resets the end reason.
    pub fn remove(&mut self, chaser: u64) -> bool {
        let Some(i) = self.chasers.iter().position(|c| *c == chaser) else { return false };
        self.chasers.remove(i);
        self.offsets.remove(&chaser);
        if self.chasers.is_empty() {
            self.end_reason = None;
        }
        true
    }
    /// `82D97108`: with at least two chasers, the primary swaps places with entry 1 (the op
    /// passes no preferred chaser). The trailing reorder after the swap is not decoded.
    pub fn give_up_primary(&mut self, chaser: u64) -> bool {
        if self.chasers.len() < 2 || self.chasers[0] != chaser {
            return false;
        }
        // `82D97108`: the primary swaps with the next chaser, then the old primary moves to the back, so with three or
        // more chasers the primary goes round the group. NOT RETAIL YET: retail can name the new primary (the
        // caller's handle; the next chaser without one) and raises the old primary's rest time; ours always takes
        // the next chaser.
        self.chasers.swap(0, 1);
        let old = self.chasers.remove(1);
        self.chasers.push(old);
        true
    }
}

/// The chase manager's record (`global`): the predict angle, degrees; 0.0 without it (`826B5D80`).
pub fn predict_angle(global: Option<&ChaseRecord>) -> f32 {
    global.and_then(|g| g.get(field::PREDICT_ANGLE)).unwrap_or(0.0)
}

/// The intercept arrival radius, m (`0x8209975C`).
pub const INTERCEPT_ARRIVAL: f32 = 0.5;

/// An intercept goal.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Intercept {
    pub point: Vec3,
    /// Seconds to the point (lead) or distance / speed (direct pursuit).
    pub time: f32,
    pub leads: bool,
}

/// `sub_82E15CD0` (flat): the smallest positive `t` with `|q + v t - p| = speed t`.
fn intercept_time(chaser: Vec3, chasee: Vec3, velocity: Vec3, speed: f32) -> Option<f32> {
    let d = [chasee[0] - chaser[0], chasee[2] - chaser[2]];
    let v = [velocity[0], velocity[2]];
    let a = v[0] * v[0] + v[1] * v[1] - speed * speed;
    let b = 2.0 * (d[0] * v[0] + d[1] * v[1]);
    let c = d[0] * d[0] + d[1] * d[1];
    if a.abs() < 1e-6 {
        return (b.abs() > 1e-6).then(|| -c / b).filter(|t| *t > 0.0);
    }
    let disc = b * b - 4.0 * a * c;
    if disc < 0.0 {
        return None;
    }
    let s = disc.sqrt();
    let (t1, t2) = ((-b - s) / (2.0 * a), (-b + s) / (2.0 * a));
    let (lo, hi) = if t1 < t2 { (t1, t2) } else { (t2, t1) };
    if lo > 0.0 {
        Some(lo)
    } else if hi > 0.0 {
        Some(hi)
    } else {
        None
    }
}

/// `sub_82E3D4D8`: where a chaser at `chaser` running at `speed` heads for a chasee at `chasee`
/// moving at `velocity`; `None` = no new goal this tick.
pub fn intercept(chaser: Vec3, chasee: Vec3, velocity: Vec3, speed: f32, max_lead_time: f32, predict_angle_deg: f32) -> Option<Intercept> {
    let t = intercept_time(chaser, chasee, velocity, speed)?;
    if !(max_lead_time > t) {
        return None;
    }
    let rel = [chaser[0] - chasee[0], chaser[2] - chasee[2]];
    let v = [velocity[0], velocity[2]];
    let (lr, lv) = ((rel[0] * rel[0] + rel[1] * rel[1]).sqrt(), (v[0] * v[0] + v[1] * v[1]).sqrt());
    // `sub_8296EBB0`: 0 when either length is about 0.
    let angle = if lr > 1e-5 && lv > 1e-5 { ((rel[0] * v[0] + rel[1] * v[1]) / (lr * lv)).clamp(-1.0, 1.0).acos() } else { 0.0 };
    let k = predict_angle_deg.to_radians();
    let y = chaser[1].max(chasee[1]);
    if angle > k && angle < std::f32::consts::PI - k {
        Some(Intercept { point: [chasee[0] + velocity[0] * t, y, chasee[2] + velocity[2] * t], time: t, leads: true })
    } else {
        Some(Intercept { point: [chasee[0], y, chasee[2]], time: if speed > 0.0 { lr / speed } else { f32::MAX }, leads: false })
    }
}

/// `82D99540` UpdateBlockPrediction: does a secondary chaser at `chaser` block a chasee at
/// `chasee` moving at `velocity`? Within the range (flat), either touching (`chasee_radius` +
/// the block radius; the chasee radius `G+1648` has no known writer, 0.0 here) or the chasee
/// runs at the chaser faster than the closing speed within the cone. The block point is then the
/// formation point (the entry direction stays 0).
pub fn should_block(chaser: Vec3, chasee: Vec3, velocity: Vec3, chasee_radius: f32, record: &ChaseRecord) -> bool {
    let g = |k: &str| record.get(k).unwrap_or(0.0);
    let d = [chasee[0] - chaser[0], chasee[2] - chaser[2]];
    let d2 = d[0] * d[0] + d[1] * d[1];
    let range = g(field::BLOCK_RANGE);
    if d2 >= range * range {
        return false;
    }
    let touch = chasee_radius + g(field::BLOCK_RADIUS);
    if d2 - touch * touch < 0.0 {
        return true;
    }
    let l = d2.sqrt();
    if l < 1e-6 {
        return false;
    }
    let n = [d[0] / l, d[1] / l];
    let v = [velocity[0], velocity[2]];
    let closing = -(n[0] * v[0] + n[1] * v[1]);
    if closing <= g(field::BLOCK_CLOSING_SPEED) {
        return false;
    }
    let lv = (v[0] * v[0] + v[1] * v[1]).sqrt();
    let w = [-n[0], -n[1]];
    let angle = if lv > 1e-6 { ((v[0] * w[0] + v[1] * w[1]) / lv).clamp(-1.0, 1.0).acos() } else { 0.0 };
    angle < g(field::BLOCK_CONE).to_radians()
}

/// `826AC668` ChaseeEscaped: the chasee is farther than the escape distance (3D).
pub fn escaped(chaser: Vec3, chasee: Vec3, record: &ChaseRecord) -> bool {
    let d = [chasee[0] - chaser[0], chasee[1] - chaser[1], chasee[2] - chaser[2]];
    let e = record.escape_distance();
    d[0] * d[0] + d[1] * d[1] + d[2] * d[2] - e * e > 0.0
}

/// The mover's reachability check (`82E2CB08`, every 10 mover updates: `82C465A8` snaps the ped and its current
/// destination onto the navmesh, `82C464F0` tests that both have a polygon and are connected; research b177 / b302).
/// `ped` and `destination` are the located polygons (`None` = not on the mesh, e.g. a chasee in deep water).
/// True when the check reports the destination unreachable (retail result codes 0, 2, 3, 4 post an event; 1 posts
/// none). Ours: the reachable-but-high-area-class case (`82C47D60`) is not ported.
pub fn reach_failed(mesh: &super::nav::NavMesh, ped: Option<u32>, destination: Option<u32>) -> bool {
    !matches!((ped, destination), (Some(a), Some(b)) if mesh.reachable(a, b))
}

/// What the intercept goal's event handler (`82E31D58`) does with a report: an unreachable report rewrites
/// CannotReachChaseTarget (`brain+3280` bit 0x08) to "within the unreachable radius of the chasee"; a reachable
/// destination posts nothing and the bit keeps its value (b302).
pub fn apply_reach_report(unreachable: &mut bool, failed: bool, near: bool) {
    if failed {
        *unreachable = near;
    }
}

/// The line the HUD's central message shows when a chase reaches the player (`826B4FC0`, chase manager
/// `0x830854B8` vtable slot +64): `caught` false = the chase ended (`ID_CHASE_OVER`); a taze picks by halves, a
/// takedown the four "caught" lines by quarters. `roll` is retail's `rand() % 101 * 0.01`. Language ids: the text is
/// the HUD movie's language table ("ZAP!", "Tazed!", "BAM!", ...).
pub fn chase_message(caught: bool, tazed: bool, roll: f32) -> &'static str {
    match (caught, tazed) {
        (false, _) => "ID_CHASE_OVER",
        (true, true) if roll < 0.5 => "ID_CHASE_TAZER_1",
        (true, true) => "ID_CHASE_TAZER_2",
        (true, false) if roll < 0.25 => "ID_CHASE_CAUGHT_1",
        (true, false) if roll < 0.5 => "ID_CHASE_CAUGHT_2",
        (true, false) if roll < 0.75 => "ID_CHASE_CAUGHT_3",
        (true, false) => "ID_CHASE_CAUGHT_4",
    }
}

/// Retail's roll for [`chase_message`] from a random word: `r % 101 * 0.01` (0.00 ..= 1.00).
pub fn chase_message_roll(r: u32) -> f32 {
    (r % 101) as f32 * 0.01
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::living_world::peds::nav::{AREA_DEFAULT, NavMesh, NavMeshInput, NavPolyInput, NavRules};

    /// Two unconnected 4 m squares: the bank (x 0..4) and an island (x 10..14); water between.
    fn bank_and_island() -> NavMesh {
        let quad = |x: f32| NavPolyInput { verts: vec![[x, 0.0, 0.0], [x + 4.0, 0.0, 0.0], [x + 4.0, 0.0, 4.0], [x, 0.0, 4.0]], neighbours: vec![None; 4], area: AREA_DEFAULT };
        NavMesh::build(&NavMeshInput { agent: [0.12, 0.35, 0.2, 1.6], polygons: vec![quad(0.0), quad(10.0)] }, NavRules::default())
    }

    #[test]
    fn reach_reports_fail_off_the_mesh_and_across_islands_and_only_they_rewrite_the_bit() {
        let m = bank_and_island();
        let poly = |p: Vec3| m.locate(p).map(|n| n.poly);
        let (ped, bank, water, island) = (poly([1.0, 0.0, 1.0]), poly([3.0, 0.0, 3.0]), poly([7.0, 0.0, 2.0]), poly([12.0, 0.0, 2.0]));
        assert!(water.is_none(), "the water between is off the mesh");
        assert!(!reach_failed(&m, ped, bank));
        assert!(reach_failed(&m, ped, water), "a chasee in the water");
        assert!(reach_failed(&m, ped, island), "a chasee on an unconnected island");
        assert!(reach_failed(&m, None, bank), "the ped itself off the mesh");
        // 82E31D58: an unreachable report sets the bit within the radius and clears it beyond; a reachable
        // destination posts nothing, so the bit stays (a chasee back on the bank does not end the wait).
        let mut bit = false;
        apply_reach_report(&mut bit, true, true);
        assert!(bit);
        apply_reach_report(&mut bit, false, true);
        assert!(bit, "reachable again keeps the bit");
        apply_reach_report(&mut bit, true, false);
        assert!(!bit, "unreachable but farther than the radius clears it");
    }

    #[test]
    fn chase_messages_follow_retails_bands() {
        assert_eq!(chase_message(true, true, chase_message_roll(49)), "ID_CHASE_TAZER_1");
        assert_eq!(chase_message(true, true, chase_message_roll(50)), "ID_CHASE_TAZER_2");
        assert_eq!(chase_message(true, false, chase_message_roll(24)), "ID_CHASE_CAUGHT_1");
        assert_eq!(chase_message(true, false, chase_message_roll(25)), "ID_CHASE_CAUGHT_2");
        assert_eq!(chase_message(true, false, chase_message_roll(74)), "ID_CHASE_CAUGHT_3");
        assert_eq!(chase_message(true, false, chase_message_roll(100)), "ID_CHASE_CAUGHT_4");
        assert_eq!(chase_message(false, true, 0.0), "ID_CHASE_OVER");
        assert_eq!(chase_message_roll(101), 0.0);
    }

    #[test]
    fn crossing_chasee_is_led_and_one_running_away_is_chased_directly() {
        // Chasee 10 m ahead (+z), running across at 5 m/s; chaser runs 11 m/s.
        let i = intercept([0.0, 0.0, 0.0], [0.0, 1.0, 10.0], [5.0, 0.0, 0.0], 11.0, 6.0, 22.5).unwrap();
        assert!(i.leads);
        let reach = ((i.point[0]).powi(2) + (i.point[2]).powi(2)).sqrt();
        assert!((reach - 11.0 * i.time).abs() < 1e-3, "{i:?}");
        assert_eq!(i.point[1], 1.0);
        // Running straight away (angle 180 deg, outside the window): the goal is the chasee.
        let i = intercept([0.0, 0.0, 0.0], [0.0, 0.0, 10.0], [0.0, 0.0, 5.0], 11.0, 6.0, 22.5).unwrap();
        assert!(!i.leads);
        assert_eq!(i.point, [0.0, 0.0, 10.0]);
        // Too far ahead for the max lead time: no goal.
        assert!(intercept([0.0, 0.0, 0.0], [0.0, 0.0, 100.0], [5.0, 0.0, 0.0], 11.0, 6.0, 22.5).is_none());
        // Faster chasee running away: no solution.
        assert!(intercept([0.0, 0.0, 0.0], [0.0, 0.0, 10.0], [0.0, 0.0, 15.0], 11.0, 6.0, 22.5).is_none());
    }

    #[test]
    fn groups_keep_join_order_and_hand_over_the_primary() {
        let mut g = ChaseGroup { end_reason: Some(reason::LOST_INTEREST), ..Default::default() };
        assert!(g.add(4, [0.0; 3], 25));
        assert_eq!(g.end_reason, None, "the first chaser resets the end reason");
        assert!(g.add(7, [2.0, 0.0, 0.0], 25) && !g.add(7, [0.0; 3], 25));
        assert_eq!(g.formation_point(7, [10.0, 0.0, 0.0]), [12.0, 0.0, 0.0]);
        assert_eq!(g.info(2), ChaseGroupInfo { count: 2, primary: Some(4), end_reason: None, can_add: false });
        assert!(!g.give_up_primary(7));
        assert!(g.give_up_primary(4));
        assert_eq!(g.chasers, vec![7, 4]);
        // A full group refuses the join (`82D96C38`); with three chasers the primary goes round.
        assert!(!g.add(9, [0.0; 3], 2) && g.add(9, [0.0; 3], 3));
        assert!(g.give_up_primary(7));
        assert_eq!(g.chasers, vec![4, 9, 7]);
        assert!(g.give_up_primary(4));
        assert_eq!(g.chasers, vec![9, 7, 4]);
        assert!(g.remove(9));
        g.end_reason = Some(reason::AGGRESSIVE_CAPTURE);
        assert!(!g.info(5).can_add);
        assert!(g.remove(7) && g.remove(4));
        assert!(g.offsets.is_empty());
        assert_eq!(g.end_reason, None);
        let mut r = ChaseRecord::default();
        assert_eq!(ChaseRecord::max_chasers(None), 25);
        r.fields.insert(field::MAX_CHASERS.into(), 300.0);
        assert_eq!(ChaseRecord::max_chasers(Some(&r)), 25);
        r.fields.insert(field::MAX_CHASERS.into(), 5.0);
        assert_eq!(ChaseRecord::max_chasers(Some(&r)), 5);
        assert_eq!((reason::parse("lostinterest"), reason::parse("x")), (3, -1));
    }

    #[test]
    fn secondary_chasers_block_a_chasee_running_at_them() {
        let mut r = ChaseRecord::default();
        for (k, v) in [(field::BLOCK_RANGE, 20.0), (field::BLOCK_RADIUS, 1.0), (field::BLOCK_CLOSING_SPEED, 5.0), (field::BLOCK_CONE, 45.0)] {
            r.fields.insert(k.into(), v);
        }
        // Chasee 10 m away running at the chaser at 6 m/s: block; at 4 m/s or sideways: no.
        assert!(should_block([0.0; 3], [0.0, 0.0, 10.0], [0.0, 0.0, -6.0], 0.0, &r));
        assert!(!should_block([0.0; 3], [0.0, 0.0, 10.0], [0.0, 0.0, -4.0], 0.0, &r));
        assert!(!should_block([0.0; 3], [0.0, 0.0, 10.0], [6.0, 0.0, 0.0], 0.0, &r));
        // Touching range, any motion; out of range, nothing.
        assert!(should_block([0.0; 3], [0.0, 0.0, 0.5], [6.0, 0.0, 0.0], 0.0, &r));
        assert!(!should_block([0.0; 3], [0.0, 0.0, 25.0], [0.0, 0.0, -9.0], 0.0, &r));
    }

    #[test]
    fn record_fallbacks_and_escape() {
        let mut r = ChaseRecord::default();
        assert_eq!(r.max_lead_time(), 35_184_372_088_832.0);
        assert!(escaped([0.0; 3], [0.0, 0.0, 0.1], &r));
        r.fields.insert(field::ESCAPE_DISTANCE.into(), 65.0);
        assert!(!escaped([0.0; 3], [0.0, 0.0, 65.0], &r));
        assert!(escaped([0.0; 3], [0.0, 0.0, 65.1], &r));
        assert_eq!(predict_angle(None), 0.0);
    }
}
