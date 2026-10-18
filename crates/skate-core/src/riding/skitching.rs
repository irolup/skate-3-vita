//! Skitching (physical state 104, held to a car's grab spline): the tow / ride-height spring
//! (`sub_82D4B500`, board force tag 6). Retail (TU3, evidence only; re-implemented;
//! `.local/research/npc/b27-skitch-state.md`, main checked the vault keys and the constants):
//! a velocity-target spring along the hold axis `n` (state+784), not a position pin:
//! ```text
//! cd    = clamp(n . state+624, 0.5, 1.0)
//! pred  = state+848 + state+852 + dt * (2 * state+832 - p+2612)
//! z     = body height (negated when p+2476 bit 2 is set)
//! err   = pred - (p+2820 - z + offset)        offset: 0.1 (sub-mode 2: a second key, 0.1)
//! v     = curve(err)                          vault PointGraph, -20 .. +4 m/s
//! dv    = (v - damping * (-state+856)) / cd   damping 0.2
//! F     = n * dv * mass / dt, length-clamped to 500
//! ```
//! The state fields (+624, +832, +848, +852, +856) come from the skitch frame step `sub_82D48148`
//! ([`frame`]); this module keeps the spring formula with named inputs.

use crate::point_graph::PointGraph;

pub mod frame;
pub mod target;
pub mod hold;
pub mod shimmy;
pub mod lean;
pub mod hands;

/// `physics_state_skitching/default` values the spring reads (vault data).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SkitchSpringSettings {
    /// `B7A817055629EC4B` (0.1) and, in sub-mode 2, `95D9DE65A73BB685` (0.1).
    pub height_offset: f32,
    pub height_offset_mode_2: f32,
    /// `00A1B0B33F1BA1BF`: height error -> target speed along the axis.
    pub speed_curve: PointGraph<8>,
    /// `71A894F6AD65C859` (0.2).
    pub damping: f32,
}

impl Default for SkitchSpringSettings {
    fn default() -> Self {
        Self {
            height_offset: 0.1,
            height_offset_mode_2: 0.1,
            speed_curve: PointGraph { x: [-2.0, -1.1075, -1.0489, -1.0, 0.0, 1.0, 1.5, 2.0], y: [-20.0, -20.0, -20.0, -16.0, 0.0, 4.0, 4.0, 4.0] },
            damping: 0.2,
        }
    }
}

/// The spring's inputs (state and processed-input fields, named by role).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SkitchSpringInput {
    /// Horizontal unit direction to the hold target (state+784, last frame's: `82D4BAC0` runs after
    /// the spring) and to the side point 0.5 m off the grab axis (state+624).
    pub target_dir: [f32; 3],
    pub side_dir: [f32; 3],
    /// The grab point's signed horizontal offset (state+848), the horizontal distance to the grab
    /// axis (+852), the car's tow speed (+832) and the distance's rate (+856).
    pub grab_side_offset: f32,
    pub axis_distance: f32,
    pub tow_speed: f32,
    pub axis_distance_rate: f32,
    /// The board's signed forward speed (p+2612), the world grab height (p+2820, inferred), the body
    /// height and whether it is negated (p+2476 bit 2).
    pub board_speed: f32,
    pub world_grab_z: f32,
    pub body_height: f32,
    pub negate_height: bool,
    /// Sub-mode (state+1324) and the board mass (p+2660).
    pub sub_mode: u8,
    pub mass: f32,
}

/// The native step (`0x820849C8`).
pub const DT: f32 = 1.0 / 60.0;
/// The force cap (`0x820BD5C4`).
pub const MAX_FORCE: f32 = 500.0;

/// `sub_82D4B500`: the tow force (state+816).
pub fn tow_spring(i: &SkitchSpringInput, s: &SkitchSpringSettings) -> [f32; 3] {
    let n = i.target_dir;
    let cd = (n[0] * i.side_dir[0] + n[1] * i.side_dir[1] + n[2] * i.side_dir[2]).clamp(0.5, 1.0);
    let pred = i.grab_side_offset + i.axis_distance + DT * (2.0 * i.tow_speed - i.board_speed);
    let z = if i.negate_height { -i.body_height } else { i.body_height };
    let offset = if i.sub_mode == 2 { s.height_offset_mode_2 } else { s.height_offset };
    let err = pred - (i.world_grab_z - z + offset);
    let target = s.speed_curve.evaluate(err);
    let dv = (target - s.damping * -i.axis_distance_rate) / cd;
    let k = dv * i.mass / DT;
    let f = [n[0] * k, n[1] * k, n[2] * k];
    // `82BD3D90`: a length clamp (inferred from its 500.0 argument).
    let l = (f[0] * f[0] + f[1] * f[1] + f[2] * f[2]).sqrt();
    if l > MAX_FORCE {
        [f[0] / l * MAX_FORCE, f[1] / l * MAX_FORCE, f[2] / l * MAX_FORCE]
    } else {
        f
    }
}

/// The skitch sub-mode (state+1324, `sub_82D49580`): where the skater is in the grab range.
/// Readings (b31, inferred): 1 inside, 2 at the edge, 3 stepping off the edge, 4 let go / thrown
/// (terminal here).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SkitchSubMode {
    pub mode: u8,
    /// Seconds in the current mode (state+948; `82D49AB8` adds the tick).
    pub time: f32,
    /// Seconds the car's acceleration has pushed outward (state+960).
    pub pushed: f32,
    /// A mode switch happened this tick (state+1344 bit 0x08).
    pub switched: bool,
    /// This tick left sub-mode 0 (`82D49580` then writes `908 = 864 = 836`).
    pub left_zero: bool,
}

/// What `82D49AB8`'s per-mode writes change (named by role; state offsets in the field docs).
pub struct SkitchModeWrites<'a> {
    /// 904 the hold's along target, 864 the shimmy's hand target, 900 the latched edge point, 1344 bits 0x02
    /// (edge latched) / 0x01 (snapped), 1328 / 1332 the hands' off flags, 960 the outward push timer.
    pub hold_target: &'a mut f32,
    pub hand_target: &'a mut f32,
    pub edge_point: &'a mut f32,
    pub edge_latched: &'a mut bool,
    pub snapped: bool,
    pub hands_off: &'a mut [bool; 2],
    pub pushed: &'a mut f32,
}

/// `82D49AB8` before its hold step (`82D49D70`), after the shimmy chain (TU3 call order in the state update:
/// `82D49580`, `82D48C98`, `82D49AB8`). Retail (evidence only; re-implemented; main read the switch on 1324):
/// - 0: `904 = clamp(864, -928, 928)`;
/// - 1: both hands grip again (`1328 = 1332 = 0`) every tick;
/// - 2: the edge point `e = +-932` (sign of 836, + only for 836 > 0) latches into 900 once (1344 bit 0x02), a
///   snapped hand (bit 0x01) takes it as 864; 960 counts while the car's along acceleration 868 pushes outward past
///   50 (both 868 and 836 non-zero, same sign), else 0; the hand on the outer side lets go (836 > 0: hand A off,
///   B on; else A on, B off);
/// - 3: the same latch with `e = +-(928 + 0.8)`, both hands off.
pub fn mode_writes(mode: u8, along: f32, half_length: f32, along_limit: f32, car_accel: f32, w: SkitchModeWrites, s: &SkitchSubModeSettings, dt: f32) {
    let sign = if along > 0.0 { 1.0 } else { -1.0 };
    let latch = |e: f32, w: &mut SkitchModeWrites| {
        if !*w.edge_latched {
            *w.edge_point = e;
            *w.edge_latched = true;
        }
        if w.snapped {
            *w.hand_target = e;
        }
    };
    match mode {
        0 => *w.hold_target = w.hand_target.clamp(-half_length, half_length),
        1 => *w.hands_off = [false; 2],
        2 => {
            let mut w = w;
            latch(sign * along_limit, &mut w);
            let outward = car_accel != 0.0 && along != 0.0 && (car_accel > 0.0) == (along > 0.0) && car_accel.abs() > s.push_accel;
            *w.pushed = if outward { *w.pushed + dt } else { 0.0 };
            *w.hands_off = if along > 0.0 { [true, false] } else { [false, true] };
        }
        3 => {
            let mut w = w;
            latch(sign * (half_length + s.step_off), &mut w);
            *w.hands_off = [true; 2];
        }
        _ => {}
    }
}

/// What `82D49580` reads each tick (named by role).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SkitchSubModeInput {
    /// Position along the grab range (state+836) and its half length (state+928 = +1204).
    pub position: f32,
    pub half_length: f32,
    /// The stick along the range (state+924).
    pub stick: f32,
    /// The hard-acceleration event sign (state+1000: -1, 0, +1) and the car's along-range
    /// acceleration (state+868).
    pub event: f32,
    pub car_accel: f32,
    /// Mode 0 also leaves when either of these (state+1328 / +1332, the hands' off flags, 1 = off) is 0, i.e. when
    /// a hand grips.
    pub ready_a: f32,
    pub ready_b: f32,
}

/// `physics_state_skitching` values of the sub-mode machine (vault: six 0.5 s timers, the 0.4 m
/// edge band `6027...`, the 0.2 s push time `08FE...`, the 50 outward acceleration `007C...`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SkitchSubModeSettings {
    pub settle: f32,
    pub edge_band: f32,
    pub push_time: f32,
    pub push_accel: f32,
    /// Sub-mode 3's edge point past the half length (`82D49AB8`, `Hash_AED1D7B20DA7CBBA`, 0.8).
    pub step_off: f32,
}

impl Default for SkitchSubModeSettings {
    fn default() -> Self {
        Self { settle: 0.5, edge_band: 0.4, push_time: 0.2, push_accel: 50.0, step_off: 0.8 }
    }
}

impl SkitchSubMode {
    /// One tick of `82D49580` (+ `82D49AB8`'s timer 948). Leaving sub-mode 0 also sets the posed hand 908 and the
    /// hand target 864 to the along position 836: the caller applies that when `left_zero` is true.
    pub fn step(&mut self, i: &SkitchSubModeInput, s: &SkitchSubModeSettings, dt: f32) {
        let inside = i.position.abs() <= i.half_length;
        // `82D471A8`: inside the outer band.
        let in_band = i.half_length < i.position.abs() && i.position.abs() <= i.half_length + s.edge_band;
        // `82D4C180` / `82D4C1F0`: the stick points to the centre / outward.
        let toward = i.stick.abs() > 0.01 && i.stick.signum() != i.position.signum();
        let outward = i.stick.abs() > 0.5 && i.stick.signum() == i.position.signum();
        let next = match self.mode {
            0 if self.time > s.settle || i.ready_a == 0.0 || i.ready_b == 0.0 => Some(if inside { 1 } else { 2 }),
            1 if !(inside || toward) => Some(2),
            2 if self.time > s.settle => {
                if in_band && outward && (i.event == 0.0 || i.event.signum() == i.position.signum()) {
                    Some(3)
                } else if inside || toward {
                    Some(1)
                } else if self.pushed > s.push_time {
                    Some(4)
                } else {
                    None
                }
            }
            3 if self.time < s.settle && !outward => Some(2),
            3 if self.time > s.settle => Some(4),
            _ => None,
        };
        self.switched = false;
        self.left_zero = self.mode == 0 && next.is_some();
        if let Some(m) = next {
            // The 3 -> 4 switch keeps the timer and sets no switch bit.
            if !(self.mode == 3 && m == 4) {
                self.time = 0.0;
                self.switched = true;
            }
            self.mode = m;
        }
        self.time += dt;
    }
}

/// The riding skater's skitch query (`sub_82D39BB8` / `sub_82D39D98`, `b33-skitch-wiring.md`):
/// `physics_state_skitching/default` values.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SkitchQuerySettings {
    /// The grab box half extents and offset (`978B2A7533B6675D`, `8E3C1298E8072745`).
    pub box_extents: [f32; 3],
    pub box_offset: [f32; 3],
    /// Distance ahead that needs no closing (`BF433C58D0F12390`, 1.0 m) and the closing-speed floor
    /// (0.5 m/s, a code constant).
    pub reach: f32,
    pub min_closing: f32,
    /// Latch when the time to the spline is below this (`424A1C69E0076DBB`, 0.1 s).
    pub latch_time: f32,
    /// The query's CanGrabSpline limits (`82D39BB8` constants: margin 0.25 at `0x820991A0`, angles 60 and 30
    /// degrees at `0x8209919C` / `0x82099198`, times the degree constant `0x8206D110`) and the result cap
    /// (`82D74200` passes 5).
    pub margin: f32,
    pub angle_a_degrees: f32,
    pub angle_b_degrees: f32,
    pub capacity: usize,
    /// The latch's own CanGrabSpline limits (`82D39D98`: margin 0.0 at `0x82165A10`, angles 60 and 40 degrees at
    /// `0x8209918C` / `0x82099190`).
    pub latch_margin: f32,
    pub latch_angle_a_degrees: f32,
    pub latch_angle_b_degrees: f32,
}

impl Default for SkitchQuerySettings {
    fn default() -> Self {
        Self { box_extents: [0.48, 0.45, 2.0], box_offset: [0.0, 0.86, 0.0], reach: 1.0, min_closing: 0.5, latch_time: 0.1, margin: 0.25, angle_a_degrees: 60.0, angle_b_degrees: 30.0, capacity: 5, latch_margin: 0.0, latch_angle_a_degrees: 60.0, latch_angle_b_degrees: 40.0 }
    }
}

/// One grab-spline candidate the query returned (288-byte entries at owner+11312), already past
/// CanGrabSpline (`82E08DB8`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SkitchCandidate {
    /// Entry +188 (type) and +192 (the spline's id; b34 corrects b27's +196, which is the geometry).
    pub kind: u32,
    pub object: u32,
    /// Distance ahead of the skater along its forward axis, m, and the closing speed, m/s.
    pub ahead: f32,
    pub closing: f32,
}

/// The latch (`state+2729` with +2560 type, +2564 / +2736 object, +2664 time).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SkitchLatch {
    pub kind: u32,
    pub object: u32,
    pub time: f32,
}

/// `sub_82D39D98`: time to a candidate, `max(0, (ahead - reach) / max(closing, floor))`.
pub fn time_to_skitch(c: &SkitchCandidate, s: &SkitchQuerySettings) -> f32 {
    ((c.ahead - s.reach) / c.closing.max(s.min_closing)).max(0.0)
}

/// `sub_82D39D98`: the first candidate that latches. While the re-grab cooldown runs
/// (Processed+2848 > 0) only a type-1 candidate on another object than the last skitched one
/// (Processed+2592) counts.
pub fn choose_skitch(candidates: &[SkitchCandidate], cooldown: f32, last_object: Option<u32>, s: &SkitchQuerySettings) -> Option<SkitchLatch> {
    candidates.iter().find_map(|c| {
        if cooldown > 0.0 && !(c.kind == 1 && Some(c.object) != last_object) {
            return None;
        }
        let time = time_to_skitch(c, s);
        (time < s.latch_time).then_some(SkitchLatch { kind: c.kind, object: c.object, time })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tow_spring_follows_the_speed_curve_and_is_capped() {
        let s = SkitchSpringSettings::default();
        let base = SkitchSpringInput {
            target_dir: [0.0, 1.0, 0.0],
            side_dir: [0.0, 1.0, 0.0],
            grab_side_offset: 0.0,
            axis_distance: 0.0,
            tow_speed: 0.0,
            axis_distance_rate: 0.0,
            board_speed: 0.0,
            world_grab_z: -0.1,
            body_height: 0.0,
            negate_height: false,
            sub_mode: 0,
            mass: 0.01,
        };
        // Zero error: target 0, no rate: no force.
        assert_eq!(tow_spring(&base, &s), [0.0, 0.0, 0.0]);
        // Error +1: target 4 m/s along the target_dir: 4 * 0.01 * 60 = 2.4.
        let up = tow_spring(&SkitchSpringInput { grab_side_offset: 1.0, ..base }, &s);
        assert!((up[1] - 2.4).abs() < 1e-4, "{up:?}");
        // A shallow axis (cd clamped at 0.5) doubles it; a heavy board is capped at 500.
        let shallow = tow_spring(&SkitchSpringInput { grab_side_offset: 1.0, side_dir: [1.0, 0.0, 0.0], ..base }, &s);
        assert!((shallow[1] - 4.8).abs() < 1e-4);
        let heavy = tow_spring(&SkitchSpringInput { grab_side_offset: -2.0, mass: 100.0, ..base }, &s);
        assert!((heavy[1] + MAX_FORCE).abs() < 1e-3);
    }

    #[test]
    fn the_sub_mode_tracks_the_grab_range() {
        let s = SkitchSubModeSettings::default();
        let dt = 1.0 / 60.0;
        let mut m = SkitchSubMode::default();
        let mut i = SkitchSubModeInput { position: 0.2, half_length: 0.5, stick: 0.0, event: 0.0, car_accel: 0.0, ready_a: 1.0, ready_b: 1.0 };
        for _ in 0..40 {
            m.step(&i, &s, dt);
        }
        assert_eq!(m.mode, 1, "settled inside the range");
        i.position = 0.7;
        m.step(&i, &s, dt);
        assert_eq!((m.mode, m.switched), (2, true), "outside: at the edge");
        // Holding outward in the band steps off the edge once settled, then lets go.
        i.stick = 0.8;
        for _ in 0..40 {
            m.step(&i, &s, dt);
        }
        assert_eq!(m.mode, 3);
        for _ in 0..40 {
            m.step(&i, &s, dt);
        }
        assert_eq!(m.mode, 4);
    }

    #[test]
    fn each_sub_mode_writes_its_hands_and_edge_point() {
        let s = SkitchSubModeSettings::default();
        let dt = 1.0 / 60.0;
        let (mut hold, mut hand, mut edge, mut latched, mut off, mut pushed) = (0.0, 0.9, 0.0, false, [true; 2], 0.0);
        let mut run = |mode: u8, along: f32, accel: f32, snapped: bool, hold: &mut f32, hand: &mut f32, edge: &mut f32, latched: &mut bool, off: &mut [bool; 2], pushed: &mut f32| {
            let w = SkitchModeWrites { hold_target: hold, hand_target: hand, edge_point: edge, edge_latched: latched, snapped, hands_off: off, pushed };
            mode_writes(mode, along, 0.5, 0.9, accel, w, &s, dt);
        };
        // 0: the hold target follows the hand target, clamped to the range.
        run(0, 0.0, 0.0, false, &mut hold, &mut hand, &mut edge, &mut latched, &mut off, &mut pushed);
        assert_eq!(hold, 0.5);
        // 1: both hands grip again every tick.
        run(1, 0.2, 0.0, false, &mut hold, &mut hand, &mut edge, &mut latched, &mut off, &mut pushed);
        assert_eq!(off, [false; 2]);
        // 2 at the + end: the edge point latches once, the outer hand (A) lets go, outward push counts.
        run(2, 0.6, 60.0, false, &mut hold, &mut hand, &mut edge, &mut latched, &mut off, &mut pushed);
        assert_eq!((edge, latched, off), (0.9, true, [true, false]));
        assert!((pushed - dt).abs() < 1e-6);
        run(2, -0.6, 60.0, true, &mut hold, &mut hand, &mut edge, &mut latched, &mut off, &mut pushed);
        assert_eq!((edge, hand, off, pushed), (0.9, -0.9, [false, true], 0.0), "latched edge kept; snapped hand takes the new end");
        // 3: both hands off.
        run(3, 0.6, 0.0, false, &mut hold, &mut hand, &mut edge, &mut latched, &mut off, &mut pushed);
        assert_eq!(off, [true; 2]);
    }

    #[test]
    fn the_skitch_query_latches_the_first_spline_within_reach() {
        let s = SkitchQuerySettings::default();
        let far = SkitchCandidate { kind: 1, object: 7, ahead: 3.0, closing: 5.0 };
        let near = SkitchCandidate { kind: 1, object: 8, ahead: 1.2, closing: 5.0 };
        // 2 m to cover at 5 m/s is 0.4 s; 0.2 m is 0.04 s.
        assert!((time_to_skitch(&far, &s) - 0.4).abs() < 1e-6);
        assert_eq!(choose_skitch(&[far, near], 0.0, None, &s).map(|l| l.object), Some(8));
        // A slow closing speed uses the 0.5 m/s floor.
        assert!((time_to_skitch(&SkitchCandidate { closing: 0.0, ..near }, &s) - 0.4).abs() < 1e-6);
        // During the cooldown the last car is skipped.
        assert_eq!(choose_skitch(&[near], 1.0, Some(8), &s), None);
        assert!(choose_skitch(&[near], 1.0, Some(9), &s).is_some());
    }
}
