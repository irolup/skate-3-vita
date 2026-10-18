//! The along-the-bumper offset chain (`sub_82D48C98`): where on the grab edge the skater holds and how the hand
//! shimmies. Retail (TU3, evidence only; re-implemented; `.local/research/npc/b30-skitch-pull-release.md` section 4,
//! `b31-skitch-submode-hold.md` section 2):
//! - the car's along acceleration is low-passed (`868 = 0.9 * 868 + 0.1 a`), a hard event (`1000`) is its sign past
//!   40, and the excess over a dead-band curve (`880`) pushes the hand;
//! - the stick pushes the hand at 10 (`876`); with neither a velocity servo damps it to rest (`884`);
//! - the velocity `892` integrates the total (`888`), capped at 1 (or the stick gain while an event opposes the
//!   stick) and unbounded in the event's direction; the hand target `864` moves by it, clamped to the edge range
//!   (+-932), and snaps onto the latched edge point 900 when it crosses it.
//! The acceleration `a` is the caller's (its derivation from the frame history is open after b47's frame layout).

use crate::point_graph::PointGraph;

/// `physics_state_skitching/default` values (mod-overridable).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShimmySettings {
    /// `1EA52A4BDF8964F7` (0.1): low-pass gain; `30DAD137EE602142` (40): hard event threshold.
    pub low_pass: f32,
    pub event_threshold: f32,
    /// `466FC01CF6CA2FD2`: dead-band over |868|.
    pub dead_band: PointGraph<8>,
    /// `E4965C6A27F1FF96` (10): stick acceleration and servo limit.
    pub limit: f32,
    /// `7FA1FB6630F75C67`: stick gain against an opposing event, over |868|.
    pub stick_gain: PointGraph<8>,
}

impl Default for ShimmySettings {
    fn default() -> Self {
        Self {
            low_pass: 0.1,
            event_threshold: 40.0,
            dead_band: PointGraph { x: [60.0, 70.0, 80.0, 100.0, 120.0, 140.0, 160.0, 180.0], y: [2.5, 2.5, 2.75, 3.0, 3.5, 4.0, 4.0, 4.0] },
            limit: 10.0,
            stick_gain: PointGraph { x: [0.0, 100.0, 110.0, 120.0, 130.0, 140.0, 150.0, 160.0], y: [1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0] },
        }
    }
}

/// The chain's persistent part.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ShimmyState {
    /// 864 hand target, 868 low-passed car acceleration, 876 / 880 / 884 / 888 accelerations, 892 velocity, 896
    /// correction velocity, 1000 event sign, 1004 (moves by velocity), 1344 bits 0x01 (snapped) / 0x02 (edge latched).
    pub target: f32,
    pub car_accel: f32,
    pub stick_accel: f32,
    pub excess: f32,
    pub servo: f32,
    pub total: f32,
    pub velocity: f32,
    pub correction: f32,
    pub event: f32,
    pub by_velocity: bool,
    pub snapped: bool,
    pub latched: bool,
    /// 900: the latched edge point (`82D49AB8` sub-modes 2 / 3, `skitching::mode_writes`).
    pub edge_point: f32,
}

/// What the chain reads each step.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShimmyInput {
    /// The car's along acceleration (`a`), the stick (924), the sub-mode (1324), "tows fast" (1345 bit 0x40).
    pub accel: f32,
    pub stick: f32,
    pub sub_mode: u8,
    pub tows_fast: bool,
    /// 836 along coordinate, 844 slip, 900 latched edge point, 932 range limit, 832 tow speed, 860 side distance.
    pub along: f32,
    pub edge_point: f32,
    pub limit: f32,
    pub tow_speed: f32,
    pub side_distance: f32,
    pub dt: f32,
}

impl ShimmyState {
    /// `82D48C98`.
    pub fn step(&mut self, i: &ShimmyInput, s: &ShimmySettings) {
        let k = s.low_pass;
        let stick = if i.stick.abs() > 0.01 { i.stick } else { 0.0 };
        let active = i.stick.abs() > 0.01 || self.event != 0.0;
        // `82D48FA8..82D49034` (r28 = active, r22 = 1, r25 = 0): a stick past 0.01 or a car event clears the edge latch
        // and the snap (1344 bits 0x02 / 0x01); only an idle hand keeps them (r27 = the kept latch, read below). The
        // inverse (clearing only while idle) pinned the hand on the edge for good: the stick could not bring the
        // skater back to the middle (session 2026-10-16).
        if active {
            self.latched = false;
            self.snapped = false;
        }
        let hold = self.latched;
        // 1. Gate.
        if i.sub_mode == 0 || i.sub_mode == 4 || !i.tows_fast {
            self.stick_accel = 0.0;
            self.car_accel *= 1.0 - k;
            self.excess = 0.0;
            self.servo = 0.0;
            self.total = 0.0;
            self.velocity = 0.0;
            self.correction = 0.0;
            return;
        }
        // 2. Snapped onto the edge point.
        if self.latched && self.snapped {
            self.stick_accel = 0.0;
            self.car_accel = (1.0 - k) * self.car_accel + k * i.accel;
            self.target = i.edge_point;
            self.excess = 0.0;
            self.servo = 0.0;
            self.total = 0.0;
            self.velocity = 0.0;
            self.correction = 0.0;
            return;
        }
        // b30: the low-pass, the event and the dead-band excess.
        self.car_accel = (1.0 - k) * self.car_accel + k * i.accel;
        self.event = if self.car_accel.abs() > s.event_threshold { self.car_accel.signum() } else { 0.0 };
        let g = s.dead_band.evaluate(self.car_accel.abs());
        self.excess = -self.car_accel.signum() * (self.car_accel.abs() - g).max(0.0);
        // 3. Stick gain.
        let gain = if self.event != 0.0 && stick != 0.0 && self.car_accel != 0.0 && stick.signum() != self.car_accel.signum() {
            (stick * s.stick_gain.evaluate(self.car_accel.abs())).abs()
        } else if stick != 0.0 {
            stick.abs()
        } else {
            0.0
        };
        // 4-5. Stick acceleration or the velocity servo.
        let sg = if stick == 0.0 { 0.0 } else { stick.signum() };
        self.stick_accel = s.limit * sg;
        self.servo = 0.0;
        if !active {
            let t = if self.latched { ((i.edge_point - i.along) / i.dt).clamp(-s.limit, s.limit) } else { 0.0 };
            self.servo = ((t - (self.velocity + (self.car_accel + self.excess) * i.dt)) / i.dt).clamp(-s.limit, s.limit);
        }
        // 7. Total and velocity.
        self.total = self.servo + self.car_accel + self.excess + self.stick_accel;
        let mut hi = if hold { 1.0 } else { gain };
        let mut lo = -hi;
        if self.event > 0.0 {
            hi = f32::MAX;
        } else if self.event < 0.0 {
            lo = -f32::MAX;
        }
        self.velocity = (self.velocity + self.total * i.dt).clamp(lo, hi);
        self.velocity *= i.tow_speed.max(0.0) / i.tow_speed.max(0.01);
        // 8. Move the hand target.
        if !active && !hold {
            self.by_velocity = true;
            return;
        }
        let v = self.velocity;
        let p = if self.event != 0.0 {
            self.by_velocity = true;
            self.target + v * i.dt
        } else {
            self.by_velocity = false;
            i.along + v * i.side_distance.max(0.01) / i.tow_speed.max(0.01)
        };
        let pc = p.clamp(-i.limit, i.limit);
        self.correction = (pc - i.along) * i.tow_speed.max(0.0) / i.side_distance.max(0.01);
        let moved = i.along + self.correction * i.side_distance.max(0.01) / i.tow_speed.max(0.01);
        let a = i.edge_point - self.target;
        let b = moved - i.edge_point;
        if hold && a != 0.0 && b != 0.0 && a.signum() == b.signum() {
            self.target = i.edge_point;
            self.snapped = true;
        } else {
            self.target = moved;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input() -> ShimmyInput {
        ShimmyInput { accel: 0.0, stick: 0.0, sub_mode: 1, tows_fast: true, along: 0.0, edge_point: 0.0, limit: 1.2, tow_speed: 5.0, side_distance: 1.0, dt: 1.0 / 60.0 }
    }

    #[test]
    fn the_stick_moves_the_hand_along_the_bumper_at_most_one_unit_per_second() {
        let s = ShimmySettings::default();
        let mut st = ShimmyState::default();
        let mut i = input();
        i.stick = 1.0;
        for _ in 0..60 {
            st.step(&i, &s);
            i.along = st.target;
        }
        assert!((st.velocity - 1.0).abs() < 1e-5, "capped at 1: {}", st.velocity);
        assert!(st.target > 0.1 && st.target <= 1.2, "{}", st.target);
        // Released: the servo brings the velocity back to rest.
        i.stick = 0.0;
        for _ in 0..120 {
            st.step(&i, &s);
            i.along = st.target;
        }
        assert!(st.velocity.abs() < 0.05, "{}", st.velocity);
    }

    #[test]
    fn the_stick_frees_a_hand_snapped_to_the_edge() {
        let s = ShimmySettings::default();
        let mut st = ShimmyState { latched: true, snapped: true, target: -1.2, edge_point: -1.2, ..Default::default() };
        let mut i = ShimmyInput { along: -1.2, edge_point: -1.2, ..input() };
        // Idle: the hand stays on the latched edge.
        st.step(&i, &s);
        assert_eq!((st.target, st.latched), (-1.2, true));
        // The stick toward the middle clears the latch and moves the hand off the edge.
        i.stick = 1.0;
        for _ in 0..30 {
            st.step(&i, &s);
            i.along = st.target;
        }
        assert!(!st.latched && !st.snapped && st.target > -1.0, "{st:?}");
    }

    #[test]
    fn a_hard_car_acceleration_is_an_event_and_the_gate_resets() {
        let s = ShimmySettings::default();
        let mut st = ShimmyState::default();
        let mut i = input();
        i.accel = 100.0;
        for _ in 0..30 {
            st.step(&i, &s);
        }
        assert!(st.car_accel > 40.0 && st.event == 1.0, "{st:?}");
        assert!(st.excess < 0.0, "the excess pushes against the acceleration");
        i.tows_fast = false;
        st.step(&i, &s);
        assert_eq!((st.velocity, st.total), (0.0, 0.0));
    }
}
