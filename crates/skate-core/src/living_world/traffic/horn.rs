//! The traffic horn (V4, doc 26h "Traffic: the horn"). Retail (TU3, evidence only; re-implemented;
//! `.local/research/npc/b36-traffic-v4-driver.md` sections 2, 4, 5 and 8, `b41-v4-obstacles-corridor.md` section 3,
//! `b44-horn-honker.md`; main re-read the honk receiver `sub_82E3C3D0`):
//! - every driving-state update sets the limiter kind `+4392` (0 free, 1 behind a lead that waits itself, 2 stop
//!   point, 3 behind any other lead, 4 obstacle from the look-ahead, 5 junction wait); the nearest limit wins. The
//!   junction answer is stored in the same field (1 signal, 2 approach, 3 yield, 4 blocked, 5 flagged yield), so a
//!   car queued behind one waiting at a red light is "behind a waiting lead" and never gets the blocked horn;
//! - blocked timer `+3704` (`sub_82C41120`): no lead or not inside the standoff behind it -> 0; inside it with kind 3
//!   and slower than `honk_approach_speed_kmh` -> `+= dt`;
//! - obstacle timer `+3708` (`sub_82C412D8`): no obstacle -> 0; kind 4, not inside a lead's standoff and slow ->
//!   `+= dt`; otherwise it holds;
//! - the horn decider `sub_82C40660` writes the horn state `+3420` every frame, first match wins: horn disabled
//!   (driver bit 0x01) -> 0; kind 5 -> 3; blocked > `honk_blocked_time` -> 4 (driver bit 0x02) or 5; an obstacle
//!   with record flag 0 (skater / ped): obstacle timer > `honk_obstacle_time` -> 2 and the honked-at notify to the
//!   record's handle, else under 2 s to it (`2 v > d`) and faster than `honk_approach_speed_kmh` -> 1;
//! - driver bits (`sub_82C42348`): each a percent roll `rand() % 100 + 1 <= chance x 100` on a driver field.
//! The horn sounds for as long as the decider returns a kind (no hold time in the AI); the sound per kind is the
//! horn AEMS program's (`SFXObj_TrafficHorn`). The parked car alarm (state 6) is the car alarm's, not this.

/// Limiter kinds (`+4392`).
pub mod limiter {
    pub const FREE: u8 = 0;
    pub const BEHIND_WAITING_LEAD: u8 = 1;
    pub const STOP_POINT: u8 = 2;
    pub const BEHIND_LEAD: u8 = 3;
    pub const OBSTACLE: u8 = 4;
    pub const JUNCTION_WAIT: u8 = 5;
}

/// The driver record's horn values (`livingworld_vehicle_drivers`; data a mod may override per driver class).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HornParams {
    /// `honk_blocked_time` (`BC1A827C21919C3B`), s: 4, taxi 1.
    pub blocked_time: f32,
    /// `honk_obstacle_time` (`FE83E2E0A19A9AFE`), s: 2.
    pub obstacle_time: f32,
    /// `honk_approach_speed_kmh` (`40540E1A0A5D447A`) in m/s: 5 km/h, taxi 10.
    pub approach_speed: f32,
    /// Time to the obstacle under which the approach horn sounds, s (`0x82060C50`, 2.0).
    pub approach_ttc: f32,
    /// Driver bit 0x01 chance (driver block +36, `Hash_50E084076390A573`; `82C42348`, b74, main checked): the horn
    /// is enabled. 1.0 in every stock driver.
    pub enabled_chance: f32,
    /// Driver bit 0x02 chance (driver block +20, `Hash_20E9C6487FDDBDE8`): blocked horn kind 4 (else 5). 0.5, normal
    /// 0.3, reckless 0.8. (Before b74 these two read `B5C60C1D` (+12, bit 0x04) and `7C6B48BD` (+24, the lane
    /// change direction roll): b41's field order was the hash-sorted JSON, not the schema.)
    pub blocked_long_chance: f32,
}

impl Default for HornParams {
    /// The `default` driver record.
    fn default() -> Self {
        Self { blocked_time: 4.0, obstacle_time: 2.0, approach_speed: 5.0 / 3.6, approach_ttc: 2.0, enabled_chance: 1.0, blocked_long_chance: 0.5 }
    }
}

/// The rolled driver bits (`+4401` 0x01 / 0x02).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DriverBits {
    pub horn: bool,
    pub blocked_long: bool,
}

/// `sub_8269A588`: `rand() % 100 + 1 <= chance x 100` with `unit` uniform [0, 1).
pub fn percent_roll(chance: f32, unit: f32) -> bool {
    (((unit * 100.0) as u32).min(99) + 1) as f32 <= chance * 100.0
}

impl DriverBits {
    /// `sub_82C42348` (bit 0x01 then 0x02; the roll order of the other bits is not modelled).
    pub fn roll(p: &HornParams, rand: &mut dyn FnMut() -> f32) -> Self {
        let horn = percent_roll(p.enabled_chance, rand());
        let blocked_long = percent_roll(p.blocked_long_chance, rand());
        Self { horn, blocked_long }
    }
}

/// The nearest look-ahead obstacle as the horn sees it (`+3584..+3620`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ObstacleHit {
    /// Free distance (m).
    pub distance: f32,
    /// Record flag 0 (skater / ped).
    pub soft: bool,
    /// The record's handle (`+3620`; peds), `None` = -1.
    pub id: Option<u64>,
}

/// The two horn timers (`+3704` blocked, `+3708` obstacle), s.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct HornTimers {
    pub blocked: f32,
    pub obstacle: f32,
}

impl HornTimers {
    /// `sub_82C41120` and `sub_82C412D8` for one frame. `lead_close`: `Some(inside the standoff)` when there is a
    /// lead, `None` without one.
    pub fn update(&mut self, p: &HornParams, kind: u8, lead_close: Option<bool>, obstacle: bool, speed: f32, dt: f32) {
        let slow = speed < p.approach_speed;
        match lead_close {
            None | Some(false) => self.blocked = 0.0,
            Some(true) if kind == limiter::BEHIND_LEAD && slow => self.blocked += dt,
            Some(true) => {}
        }
        if !obstacle {
            self.obstacle = 0.0;
        } else if kind == limiter::OBSTACLE && lead_close != Some(true) && slow {
            self.obstacle += dt;
        }
    }
}

/// `sub_82C40660`: the horn kind (0 = silent) and the honked-at notify target (kind 2 with a handle).
pub fn decide(p: &HornParams, bits: DriverBits, kind: u8, timers: &HornTimers, obstacle: Option<ObstacleHit>, speed: f32) -> (u8, Option<u64>) {
    if !bits.horn {
        return (0, None);
    }
    if kind == limiter::JUNCTION_WAIT {
        return (3, None);
    }
    if timers.blocked > p.blocked_time {
        return (if bits.blocked_long { 4 } else { 5 }, None);
    }
    if let Some(o) = obstacle.filter(|o| o.soft) {
        if timers.obstacle > p.obstacle_time {
            return (2, o.id);
        }
        if p.approach_ttc * speed > o.distance && speed > p.approach_speed {
            return (1, None);
        }
    }
    (0, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DT: f32 = 1.0 / 30.0;

    #[test]
    fn the_percent_roll_matches_retail_bounds() {
        assert!(percent_roll(1.0, 0.999));
        assert!(!percent_roll(0.0, 0.0));
        assert!(percent_roll(0.2, 0.19) && !percent_roll(0.2, 0.2));
    }

    #[test]
    fn a_car_stuck_behind_a_lead_honks_after_the_blocked_time() {
        let p = HornParams::default();
        let bits = DriverBits { horn: true, blocked_long: false };
        let mut t = HornTimers::default();
        for _ in 0..121 {
            t.update(&p, limiter::BEHIND_LEAD, Some(true), false, 0.0, DT);
        }
        assert_eq!(decide(&p, bits, limiter::BEHIND_LEAD, &t, None, 0.0), (5, None), "4 s passed: kind 5 without bit 0x02");
        assert_eq!(decide(&p, DriverBits { blocked_long: true, ..bits }, limiter::BEHIND_LEAD, &t, None, 0.0).0, 4);
        t.update(&p, limiter::BEHIND_LEAD, Some(false), false, 0.0, DT);
        assert_eq!(t.blocked, 0.0, "out of the standoff resets it");
        t.blocked = 10.0;
        t.update(&p, limiter::BEHIND_WAITING_LEAD, Some(true), false, 0.0, DT);
        assert_eq!(t.blocked, 10.0, "behind a waiting lead the timer holds");
        t.update(&p, limiter::FREE, None, false, 0.0, DT);
        assert_eq!(t.blocked, 0.0);
    }

    #[test]
    fn an_obstacle_gets_the_approach_horn_then_the_long_one_with_a_notify() {
        let p = HornParams::default();
        let bits = DriverBits { horn: true, blocked_long: true };
        let mut t = HornTimers::default();
        let ped = ObstacleHit { distance: 15.0, soft: true, id: Some(7) };
        assert_eq!(decide(&p, bits, limiter::OBSTACLE, &t, Some(ped), 10.0), (1, None), "under 2 s to it at 36 km/h");
        assert_eq!(decide(&p, bits, limiter::OBSTACLE, &t, Some(ped), 5.0), (0, None), "over 2 s");
        assert_eq!(decide(&p, bits, limiter::OBSTACLE, &t, Some(ObstacleHit { soft: false, ..ped }), 10.0), (0, None), "props are not honked at");
        for _ in 0..61 {
            t.update(&p, limiter::OBSTACLE, None, true, 0.0, DT);
        }
        assert_eq!(decide(&p, bits, limiter::OBSTACLE, &t, Some(ped), 0.0), (2, Some(7)));
        assert_eq!(decide(&p, bits, limiter::OBSTACLE, &t, Some(ObstacleHit { id: None, ..ped }), 0.0), (2, None), "a skater: horn, no notify");
        t.update(&p, limiter::OBSTACLE, None, true, 5.0, DT);
        assert!(t.obstacle > 2.0, "faster than the approach speed: the timer holds");
        t.update(&p, limiter::FREE, None, false, 0.0, DT);
        assert_eq!(t.obstacle, 0.0);
    }

    #[test]
    fn a_disabled_horn_and_the_junction_wait() {
        let p = HornParams::default();
        let t = HornTimers { blocked: 9.0, obstacle: 9.0 };
        assert_eq!(decide(&p, DriverBits::default(), limiter::JUNCTION_WAIT, &t, None, 0.0), (0, None));
        assert_eq!(decide(&p, DriverBits { horn: true, blocked_long: true }, limiter::JUNCTION_WAIT, &t, None, 0.0), (3, None));
    }
}
