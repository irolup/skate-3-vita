//! The held grab record of Move Object (state +720) and the grip along it
//! (+1128), research b62 [code, TU3 recomp]:
//! - 82D444A0(state, full): `full` (enter 82D442D0 and the path B re-grab)
//!   copies the record's reversed bit (rec+200 bit 0x20) into +1200 bit 0x04
//!   and places the grip at the nearest arc distance (82D2CFE8) of the skater
//!   reference (+272, bone 23), clamped to [h, length - h] with
//!   h = min(GrabSplineEndExclusion, length / 2). Always: the side test
//!   c = cross(rec+80 - ref, rec+64 - ref).y; c < 0 swaps the ends, negates
//!   the edge direction (+112) and toggles the reversed bit; when the record's
//!   bit then differs from +1200 bit 0x04 the grip is mirrored
//!   (length - grip) and the bit copied, so it stays on the same physical
//!   point of the edge.
//! - Path A of the held update 82D44A10 refreshes the record from the
//!   object's current pose every tick and runs 82D444A0 without `full`.
//! - 82D45D30 evaluates the record at the grip (82D2D2B0, arc distance).
//!
//! Multiplayer: [`HeldGrip`] is the whole per-skater state (plain `Copy`
//! data); the record itself is rebuilt from the object every tick.
use crate::player::offboard::grab_scene::{Record, at_distance, nearest_distance};

type Vector = [f32; 4];

/// Per-skater held record state: descriptor (+704 kind, +708 id), grip arc
/// distance (+1128) and the reversed bit (+1200 bit 0x04).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct HeldGrip {
    pub descriptor: (u32, u32),
    pub grip: f32,
    pub reversed: bool,
}

/// The held record evaluated at the grip (82D45D30 inputs).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RecordFrame {
    /// Grip point on the edge (at_distance(grip)).
    pub point: [f32; 3],
    /// Record ends (+64, +80) after the side test.
    pub ends: [[f32; 3]; 2],
    /// Approach vector (+96): from the edge toward the skater.
    pub approach: [f32; 3],
}

fn reversed(record: &Record) -> bool {
    record.byte(200) & 0x20 != 0
}

/// Side test of 82D444A0 (loc_82D44780): orient the record so the skater
/// reference sees its ends in a fixed winding.
fn orient(record: &mut Record, reference: [f32; 3]) {
    let [a, b] = record.endpoints();
    let to_a = [a[0] - reference[0], a[2] - reference[2]];
    let to_b = [b[0] - reference[0], b[2] - reference[2]];
    // cross(b - ref, a - ref).y
    let c = to_b[1] * to_a[0] - to_b[0] * to_a[1];
    if c < 0.0 {
        record.set_vector(64, b);
        record.set_vector(80, a);
        let edge: Vector = record.vector(112).map(|v| -v);
        record.set_vector(112, edge);
        record.0[50] ^= 0x2000_0000;
    }
}

fn mirror(record: &Record, held: &mut HeldGrip) {
    let bit = reversed(record);
    if bit != held.reversed {
        held.grip = record.scalar(176) - held.grip;
        held.reversed = bit;
    }
}

/// 82D444A0 with `full`: bind `record` and place the grip from the skater
/// reference. `record` is oriented in place.
pub fn begin_grip(record: &mut Record, reference: [f32; 3], end_exclusion: f32) -> HeldGrip {
    let length = record.scalar(176);
    let h = end_exclusion.min(length * 0.5);
    let s = nearest_distance(record, [reference[0], reference[1], reference[2], 1.0]);
    let mut held = HeldGrip {
        descriptor: (record.0[47], record.0[48]),
        grip: s.max(h).min(length - h),
        reversed: reversed(record),
    };
    orient(record, reference);
    mirror(record, &mut held);
    held
}

/// 82D444A0 without `full` (path A refresh): the grip and its arc are kept;
/// the freshly built `record` is oriented and the grip mirrored with it.
pub fn continue_grip(record: &mut Record, reference: [f32; 3], held: &mut HeldGrip) {
    held.descriptor = (record.0[47], record.0[48]);
    orient(record, reference);
    mirror(record, held);
}

/// The record at the grip (82D2D2B0 from 82D45D30).
pub fn record_frame(record: &Record, grip: f32) -> RecordFrame {
    let v3 = |v: Vector| [v[0], v[1], v[2]];
    let [a, b] = record.endpoints();
    RecordFrame { point: v3(at_distance(record, grip)), ends: [v3(a), v3(b)], approach: v3(record.vector(96)) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::player::offboard::grab_scene::best_spline_excluding;
    use crate::player::offboard::move_object::edge_record;

    fn close(a: [f32; 3], b: [f32; 3]) -> bool {
        (0..3).all(|i| (a[i] - b[i]).abs() < 1e-4)
    }

    #[test]
    fn straight_record_grip_is_the_projected_reference() {
        // Edge along +X at y 1, skater on the +Z side: the old box result.
        let mut r = edge_record(7, [-1.0, 1.0, 0.0], [1.0, 1.0, 0.0], [0.0, 0.0, 1.0]).unwrap();
        let held = begin_grip(&mut r, [0.3, 1.2, 1.0], 0.25);
        let f = record_frame(&r, held.grip);
        assert!(close(f.point, [0.3, 1.0, 0.0]), "{f:?}");
        assert_eq!(held.descriptor, (2, 7));
    }

    #[test]
    fn grip_is_clamped_by_the_end_exclusion() {
        let mut r = edge_record(7, [-1.0, 1.0, 0.0], [1.0, 1.0, 0.0], [0.0, 0.0, 1.0]).unwrap();
        let held = begin_grip(&mut r, [5.0, 1.0, 1.0], 0.25);
        assert!(close(record_frame(&r, held.grip).point, [0.75, 1.0, 0.0]));
        // Exclusion above half the length: the grip sits in the middle.
        let mut short = edge_record(8, [0.0, 1.0, 0.0], [0.4, 1.0, 0.0], [0.0, 0.0, 1.0]).unwrap();
        let held = begin_grip(&mut short, [5.0, 1.0, 1.0], 0.25);
        assert!((held.grip - 0.2).abs() < 1e-5);
    }

    #[test]
    fn flip_keeps_the_world_grip_point() {
        let build = || edge_record(7, [-1.0, 1.0, 0.0], [1.0, 1.0, 0.0], [0.0, 0.0, 1.0]).unwrap();
        // From +Z the record keeps its order; from -Z it flips (c < 0).
        let mut r = build();
        let mut held = begin_grip(&mut r, [0.4, 1.0, 1.0], 0.25);
        let world = record_frame(&r, held.grip).point;
        assert!(close(world, [0.4, 1.0, 0.0]));
        // The skater walks to the other side: the refreshed record orients
        // the other way, the grip arc is mirrored, the point stays put.
        let mut refreshed = build();
        continue_grip(&mut refreshed, [0.4, 1.0, -1.0], &mut held);
        assert!(close(record_frame(&refreshed, held.grip).point, world));
        let mut again = build();
        continue_grip(&mut again, [0.4, 1.0, 1.0], &mut held);
        assert!(close(record_frame(&again, held.grip).point, world));
    }

    #[test]
    fn mode_one_skips_the_held_descriptor() {
        let near = edge_record(7, [-1.0, 1.0, 0.0], [1.0, 1.0, 0.0], [0.0, 0.0, 1.0]).unwrap();
        let far = edge_record(9, [-1.0, 1.0, 0.5], [1.0, 1.0, 0.5], [0.0, 0.0, 1.0]).unwrap();
        let records = [near, far];
        let p = [0.0, 1.0, -0.5, 1.0];
        assert_eq!(best_spline_excluding(&records, p, None).unwrap().0[48], 7);
        assert_eq!(best_spline_excluding(&records, p, Some((2, 7))).unwrap().0[48], 9);
    }
}
