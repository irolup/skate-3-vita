//! Career stats for the local player (`skate_core::career_stats`), fed from the fixed step (skate-game
//! `career_stats_host.rs`, PR #52 `8df32c51`; doc 31 "Career stats").
//!
//! skate_vita: Bevy-free and without file I/O. [`Session`](crate::live::Session) owns the tracker and runs it after
//! each tick; the game keeps the totals and writes them from its main thread (card writes from the simulation thread
//! stall it). A map change is a new session, so upstream's "transition busy" reset is the new tracker's empty state.
use skate_core::career_stats::{row, BailRecord, CareerStats, PushLatch};

/// A position step longer than this in one fixed tick is a teleport or a map switch, not travel.
const MAX_STEP: f32 = 5.0;
/// Bail distance steps at or under this are not counted (0x82775158, 0.005 m).
const BAIL_MIN_STEP: f32 = 0.005;

/// The hit rows by ped entity key (0x826C0730 compares the entity key's hash64); any other ped counts as a bum.
const HIT_TYPES: [(&str, usize); 13] = [
    ("jock01", 1),
    ("teenage_female01", 2),
    ("teenage_male01", 3),
    ("adult_male01", 4),
    ("adult_female01", 5),
    ("business_male01", 6),
    ("business_female01", 7),
    ("skater_male01", 8),
    ("skater_female01", 9),
    ("tourist_male01", 10),
    ("tourist_female01", 11),
    ("security_street01", 12),
    ("security_nsz01", 12),
];
/// security_nsz02 also counts as a security guard.
const SECURITY_NSZ02: &str = "security_nsz02";

/// The ped type names by type (`ID_LIVINGWORLD_<TYPE>`), bum first.
pub const TYPE_NAMES: [&str; 14] = [
    "BUM", "JOCK", "TEENGIRL", "TEENGUY", "ADULTMALE", "ADULTFEMALE", "BUSINESSMAN", "BUSINESSWOMAN", "MALESKATER",
    "FEMALESKATER", "MALETOURIST", "FEMALETOURIST", "SECURITYGUARD", "GRANNY",
];

pub(crate) fn hit_type(entity: &str) -> usize {
    if entity == SECURITY_NSZ02 {
        return 12;
    }
    HIT_TYPES.iter().find(|(k, _)| *k == entity).map_or(0, |(_, t)| *t)
}

/// Upstream's `CareerStatsHost` without the file and the table (the game has those).
#[derive(Default)]
pub(crate) struct CareerStatsTracker {
    pub stats: CareerStats,
    last: Option<[f32; 3]>,
    bail_seq: Option<u32>,
    /// Bail distance not yet added (0x82775018 adds whole metres).
    bail_metres: f32,
    pushes: PushLatch,
    /// The bail running now (Hall of Meat record), added to the rows when it ends.
    bail: Option<BailRecord>,
    /// Totals changed since the game last took them.
    pub dirty: bool,
}

impl CareerStatsTracker {
    pub fn new(stats: CareerStats) -> Self {
        Self { stats, ..Default::default() }
    }

    /// A scoring runtime event: trick counts (0x82773D98: success / failure, flip or late flip), move ends with their
    /// time or distance (0x82774230: invert, manual, powerslide, grind), body flips (0x82773C00 / 0x82775328).
    fn event(&mut self, event: crate::scoring_runtime::StatEvent) {
        use crate::scoring_runtime::StatEvent;
        use skate_core::scoring::catalog;
        match event {
            StatEvent::TrickEnd { id, success, announced, held, distance } => {
                let i = id as i16;
                if announced {
                    self.record(if success { row::SUCCESSFUL_TRICKS } else { row::UNSUCCESSFUL_TRICKS }, 1.0);
                    if success && catalog::category(i) == Some(catalog::FLIP) {
                        let late = catalog::base(i) == catalog::OLLIE && i != catalog::OLLIE;
                        self.record(if late { row::LATE_FLIPS } else { row::FLIP_TRICKS }, 1.0);
                    }
                }
                if success {
                    match catalog::category(i) {
                        Some(catalog::HANDPLANT) => self.record(row::INVERT_TIME, held),
                        Some(catalog::MANUAL) => self.record(row::MANUAL_DISTANCE, distance),
                        Some(catalog::POWERSLIDE) => self.record(row::POWERSLIDE_DISTANCE, distance),
                        Some(catalog::GRIND) => self.record(row::GRIND_DISTANCE, distance),
                        _ => {}
                    }
                }
            }
            StatEvent::BodyFlipAttempt => self.record(row::BODY_FLIP_ATTEMPTS, 1.0),
            StatEvent::BodyFlips(n) => self.record(row::BODY_FLIPS, n as f32),
        }
    }

    pub fn record(&mut self, index: usize, value: f32) {
        if value != 0.0 {
            self.stats.record(index, value);
            self.dirty = true;
        }
    }

    /// One fixed step of the local player: time played, distance on and off the board, skitching, push starts, bails and
    /// their distance (0x82775158, 0x82774DA0), and the scoring runtime's trick, move and body flip events.
    pub fn track(&mut self, physics: &crate::physics::GamePhysics, s: &mut crate::physics::SkaterRuntime, dt: f32) {
        for event in std::mem::take(&mut s.scoring.stat_events) {
            self.event(event);
        }
        let bail_seq = s.scoring.bail_seq;
        if let Some(last) = self.bail_seq.filter(|l| bail_seq > *l) {
            self.record(row::WIPEOUTS, (bail_seq - last) as f32);
        }
        self.bail_seq = Some(bail_seq);
        let packet = &s.animation.packet;
        if let Some(i) = self.pushes.update(s.player_input.processed.flags_2468, packet.regular_stance, packet.riding_switch) {
            self.record(i, 1.0);
        }
        let root = s.animated_skeleton.roots.animation_to_world;
        let at = [root[3][0], root[3][1], root[3][2]];
        self.record(row::TIME_PLAYED, dt);
        let wiping = physics.board_wiping_out;
        if wiping {
            let skeleton = &s.player_input.physical.skeleton;
            let (over, strength) = (skeleton.over_599 != 0, skeleton.response_strength_580);
            let peds = s.ped_hits.iter().map(|h| h.0);
            let vehicles = s.vehicle_hits.iter().map(|h| h.0);
            let contacts = &s.collision_feedback.bones;
            let airborne = skate_core::hall_of_meat::airborne(&contacts.map(|b| b.groups[3]), &contacts.map(|b| b.normal[1]),
                &contacts.map(|b| b.specific_normal[1]));
            let bail = self.bail.get_or_insert_default();
            let frames = &s.animated_skeleton.board_frames;
            let speed = frames.com_velocity[..3].iter().map(|v| v * v).sum::<f32>().sqrt();
            let hips = s.skeleton.record.pose[23];
            let tick = skate_core::career_stats::BailTick {
                dt,
                over,
                frozen: s.player_input.physical.air.use_air_reckoning_452 != 0,
                response_strength: strength,
                airborne,
                centre: (frames.centre_of_mass[1], speed),
                hips: [hips[0], hips[1], hips[2]],
                stick: s.animation_input.extra.wipeout_gesture,
                poses: physics.settings.hall_of_meat_poses.as_ref(),
            };
            bail.tick(&tick, peds, vehicles);
            if let Some(tuning) = physics.settings.hall_of_meat.as_ref() {
                let bones = &s.collision_feedback.bones;
                let impacts = skate_core::hall_of_meat::part_impacts(&bones.map(|b| b.force), &bones.map(|b| b.specific_force));
                bail.injuries.tick(tuning, &impacts, dt * BailRecord::TICK_RATE);
            }
        } else if let Some(bail) = self.bail.take() {
            for (i, v) in bail.rows() {
                self.record(i, v);
            }
        }
        let step = self.last.map(|l| ((at[0] - l[0]).powi(2) + (at[1] - l[1]).powi(2) + (at[2] - l[2]).powi(2)).sqrt()).filter(|d| d.is_finite() && *d <= MAX_STEP);
        self.last = Some(at);
        let Some(step) = step else { return };
        let on_board = s.player_input.physical.state.category_12 != 500;
        self.record(if on_board { row::DISTANCE } else { row::DISTANCE_OFFBOARD }, step);
        if s.skitch_state.car.is_some() {
            self.record(row::SKITCH_DISTANCE, step);
            self.record(row::SKITCH_TIME, dt);
        }
        if wiping && step > BAIL_MIN_STEP {
            self.bail_metres += step;
            let whole = self.bail_metres.floor();
            if whole >= 1.0 {
                self.bail_metres -= whole;
                self.record(row::BAIL_DISTANCE, whole);
            }
        }
    }

    /// The local player's ped hits by type (0x826C0730): the ped's entity key.
    pub fn ped_hit(&mut self, entity: &str) {
        self.record(row::PED_HITS + hit_type(entity), 1.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entity_keys_pick_the_hit_row_and_others_count_as_bums() {
        assert_eq!(hit_type("jock01"), 1);
        assert_eq!(hit_type("security_nsz02"), 12);
        assert_eq!(hit_type("jock02"), 0);
        assert_eq!(hit_type(""), 0);
    }

    /// Trick ends count announced tricks, flips and late flips apart, and add a move's time or distance only when it
    /// completed (0x82773D98, 0x82774230).
    #[test]
    fn scoring_events_feed_the_trick_and_move_rows() {
        use crate::scoring_runtime::StatEvent;
        let mut host = CareerStatsTracker::default();
        let end = |id: usize, success: bool, held: f32, distance: f32| StatEvent::TrickEnd { id, success, announced: true, held, distance };
        host.event(end(92, true, 0.0, 0.0)); // a flip
        host.event(end(100, true, 0.0, 0.0)); // a late flip
        host.event(end(2, true, 0.0, 4.5)); // nose manual
        host.event(end(0, false, 0.0, 9.0)); // a failed powerslide: no distance
        host.event(StatEvent::TrickEnd { id: 7, success: true, announced: false, held: 0.0, distance: 3.0 }); // grind
        host.event(StatEvent::BodyFlipAttempt);
        host.event(StatEvent::BodyFlips(2));
        let v = |r: usize| host.stats.value(r);
        assert_eq!((v(row::SUCCESSFUL_TRICKS), v(row::UNSUCCESSFUL_TRICKS)), (3.0, 1.0));
        assert_eq!((v(row::FLIP_TRICKS), v(row::LATE_FLIPS)), (1.0, 1.0));
        assert_eq!((v(row::MANUAL_DISTANCE), v(row::POWERSLIDE_DISTANCE), v(row::GRIND_DISTANCE)), (4.5, 0.0, 3.0));
        assert_eq!((v(row::BODY_FLIP_ATTEMPTS), v(row::BODY_FLIPS)), (1.0, 2.0));
        assert!(host.dirty);
    }

    #[test]
    fn ped_hits_count_by_type() {
        let mut host = CareerStatsTracker::default();
        host.ped_hit("jock01");
        host.ped_hit("unknown01");
        assert_eq!((host.stats.value(row::PED_HITS + 1), host.stats.value(row::PED_HITS)), (1.0, 1.0));
    }
}
