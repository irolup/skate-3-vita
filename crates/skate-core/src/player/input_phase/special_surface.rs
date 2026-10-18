//! TU3 ProcessOutput's special-surface writer 82DB8120 and helper 82DB80C8.
//! Consumes the completed per-state packet; never infers contact from proximity.
use super::{PhysicalPlayerInput, ProcessedPhysicsInput};

/// 8208081C in the TU3 mapped image, used by both air-probe branches.
const PROBE_TIME: f32 = f32::from_bits(0x3d85_1eb8);

/// A teleport request raised by a type-6 `physics_unrideable` contact (82DB80C8 type 6 or the
/// ragdoll SkeletonCollision+214 branch at 82DB81CC), returned so the host can record why the
/// checkpoint reset happened. Plain copyable data.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BoundaryContact {
    /// Selected physical state (ProcessedPhysicsInput+2508) of the branch that saw the contact.
    pub state: u32,
    /// Packed surface word when the branch reads one (feet line tests, plants); `None` for the
    /// board (Collision+16 holds the type only) and ragdoll (+214 is a flag) branches.
    pub packed_surface: Option<u32>,
}

fn react(out: &mut PhysicalPlayerInput, flags: &mut [bool; 36], surface: u32) -> bool {
    match surface {
        6 => {
            flags[69 - 52] = true;
            out.state.flag_69 = 1;
            return true;
        }
        9 | 12 => flags[65 - 52] = true,
        _ => {}
    }
    false
}
fn water(out: &mut PhysicalPlayerInput, height: f32) {
    out.collision.flag_3481 = 1;
    out.collision.scalar_28 = height;
}

pub fn publish_special_surface(
    out: &mut PhysicalPlayerInput,
    p: &ProcessedPhysicsInput,
    flags: &mut [bool; 36],
    board_flags: u32,
    board_height: f32,
) -> Option<BoundaryContact> {
    let state = p.state_2508;
    let mut boundary = None;
    let mut hit = |raised: bool, packed_surface: Option<u32>| {
        if raised && boundary.is_none() {
            boundary = Some(BoundaryContact { state, packed_surface });
        }
    };
    match p.state_2508 {
        100..=104 | 503 => {
            hit(react(out, flags, out.collision.surface_type_16), None);
            out.collision.flag_3481 = ((board_flags >> 25) & 1) as u8;
            // The native branch copies height even when the flag is zero.
            out.collision.scalar_28 = board_height;
        }
        201 => {
            if out.air.surface_category_232 == 12 && out.air.scalar_184 < PROBE_TIME {
                flags[65 - 52] = true;
                water(out, f32::from_bits(out.air.collision_position_16[1]));
            }
        }
        300 => {
            if out.collision.flag_214 != 0 {
                flags[69 - 52] = true;
                out.state.flag_69 = 1;
                hit(true, None);
            }
            if out.collision.flag_217 != 0 {
                water(out, out.collision.surface_height_208);
            } else if out.collision.flag_3483 != 0 {
                water(out, f32::from_bits(out.collision.predicted_position_64[1]));
            }
        }
        500 | 502 => {
            let [left, right, _] = p.line_tests_960_1008_1056;
            let a = (left.surface >> 7) & 31;
            let b = (right.surface >> 7) & 31;
            if out.off_board.flags_306_307[0] != 0 {
                hit(react(out, flags, a), Some(left.surface));
                if a == 12 {
                    water(out, f32::from_bits(left.position[1]));
                }
            } else if out.off_board.flags_306_307[1] != 0 {
                hit(react(out, flags, b), Some(right.surface));
                if b == 12 && out.collision.flag_3481 == 0 {
                    water(out, f32::from_bits(right.position[1]));
                }
            } else if a == 12 && b == 12 {
                flags[65 - 52] = true;
                if out.collision.flag_3481 == 0 {
                    let l = f32::from_bits(left.position[1]);
                    let r = f32::from_bits(right.position[1]);
                    // vminfp v59,v60,v61 (right, left), including unordered/equal selection.
                    water(out, if r < l { r } else { l });
                }
            }
        }
        501 => {
            if out.off_board.word_144 == 12 && out.off_board.scalar_32 < PROBE_TIME {
                flags[65 - 52] = true;
                water(out, out.off_board.scalar_148);
            }
        }
        601 if out.air.flag_448 != 0 => {
            let packed = out.air.footplant_surface_224;
            let surface = (packed >> 7) & 31;
            hit(react(out, flags, surface), Some(packed));
            if surface == 12 {
                water(out, out.air.footplant_surface_height_216);
            }
        }
        602 => {
            let line = p.line_tests_960_1008_1056[2];
            if line.valid != 0 {
                let surface = (line.surface >> 7) & 31;
                hit(react(out, flags, surface), Some(line.surface));
                if surface == 12 {
                    water(out, f32::from_bits(line.position[1]));
                }
            }
        }
        _ => {}
    }
    boundary
}

#[cfg(test)]
mod tests {
    use super::*;
    fn run(
        state: u32,
        out: &mut PhysicalPlayerInput,
        p: &mut ProcessedPhysicsInput,
        board: u32,
    ) -> [bool; 36] {
        p.state_2508 = state;
        let mut flags = [false; 36];
        publish_special_surface(out, p, &mut flags, board, 8.0);
        flags
    }
    // 82DB832C..8388 admits board feedback only in these six selected states.
    #[test]
    fn board_feedback_is_owned_by_riding_states() {
        for state in [200, 202, 400, 401, 500, 501, 502, 600, 601, 602, 701, 702] {
            let mut out = PhysicalPlayerInput::default();
            out.collision.surface_type_16 = 12;
            assert!(
                !run(
                    state,
                    &mut out,
                    &mut ProcessedPhysicsInput::default(),
                    1 << 25
                )[13]
            );
            assert_eq!(out.collision.flag_3481, 0);
        }
        for state in [100, 101, 102, 103, 104, 503] {
            for (surface, bail, teleport) in [
                (0, false, false),
                (6, false, true),
                (9, true, false),
                (12, true, false),
            ] {
                let mut out = PhysicalPlayerInput::default();
                out.collision.surface_type_16 = surface;
                let f = run(
                    state,
                    &mut out,
                    &mut ProcessedPhysicsInput::default(),
                    1 << 25,
                );
                assert_eq!((f[13], f[17]), (bail, teleport));
                assert_eq!(out.state.flag_69 != 0, teleport);
                assert_eq!(out.collision.flag_3481, 1);
                run(state, &mut out, &mut ProcessedPhysicsInput::default(), 0);
                assert_eq!(out.collision.flag_3481, 0);
                assert_eq!(out.collision.scalar_28, 8.0);
            }
        }
    }
    // fcmpu/bge 82DB8190/82DB82DC excludes equality and unordered comparisons.
    #[test]
    fn air_probes_use_the_exact_strict_threshold() {
        for state in [201, 501] {
            for (t, expected) in [
                (f32::from_bits(0x3d851eb7), true),
                (PROBE_TIME, false),
                (f32::from_bits(0x3d851eb9), false),
                (f32::NAN, false),
            ] {
                let mut out = PhysicalPlayerInput::default();
                out.air.surface_category_232 = 12;
                out.air.scalar_184 = t;
                out.air.collision_position_16[1] = 3.0f32.to_bits();
                out.off_board.word_144 = 12;
                out.off_board.scalar_32 = t;
                out.off_board.scalar_148 = 3.0;
                let f = run(state, &mut out, &mut ProcessedPhysicsInput::default(), 0);
                assert_eq!(f[13], expected, "state {state}, time {t}");
                assert_eq!(out.collision.flag_3481 != 0, expected);
                if expected {
                    assert_eq!(out.collision.scalar_28, 3.0);
                }
            }
        }
    }
    // 82DB8390..8478: the left foot overrides height; the right and the
    // unselected-feet branch preserve a preceding publisher's height.
    #[test]
    fn foot_priority_and_existing_height_match_the_native_branches() {
        for state in [500, 502] {
            let mut p = ProcessedPhysicsInput::default();
            for (line, h) in p.line_tests_960_1008_1056[..2]
                .iter_mut()
                .zip([8.0f32, 4.0])
            {
                line.surface = 12 << 7;
                line.position[1] = h.to_bits();
            }
            let mut out = PhysicalPlayerInput::default();
            assert!(run(state, &mut out, &mut p, 0)[13]);
            assert_eq!(out.collision.scalar_28, 4.0);
            out.collision.scalar_28 = 20.0;
            run(state, &mut out, &mut p, 0);
            assert_eq!(out.collision.scalar_28, 20.0);
            out.off_board.flags_306_307 = [1, 1];
            run(state, &mut out, &mut p, 0);
            assert_eq!(out.collision.scalar_28, 8.0);
            out.off_board.flags_306_307 = [0, 1];
            run(state, &mut out, &mut p, 0);
            assert_eq!(out.collision.scalar_28, 8.0);
            out.collision.flag_3481 = 0;
            run(state, &mut out, &mut p, 0);
            assert_eq!(out.collision.scalar_28, 4.0);
            out = PhysicalPlayerInput::default();
            p.line_tests_960_1008_1056[1].surface = 0;
            assert!(!run(state, &mut out, &mut p, 0)[13]);
            assert_eq!(out.collision.flag_3481, 0);
        }
    }
    // 82DB80C8 type 6 and 82DB81CC (+214) are the only branches that raise +69; the host gets
    // the state (and packed surface where the branch reads one) to tag the reset `boundary`.
    #[test]
    fn type_six_contacts_report_a_boundary_request() {
        let mut flags = [false; 36];
        let mut p = ProcessedPhysicsInput::default();
        for (surface, expected) in [(6, true), (0, false), (9, false), (12, false)] {
            let mut out = PhysicalPlayerInput::default();
            out.collision.surface_type_16 = surface;
            p.state_2508 = 100;
            let got = publish_special_surface(&mut out, &p, &mut flags, 0, 0.0);
            assert_eq!(
                got,
                expected.then_some(BoundaryContact { state: 100, packed_surface: None })
            );
        }
        let mut out = PhysicalPlayerInput::default();
        out.collision.flag_214 = 1;
        p.state_2508 = 300;
        assert_eq!(
            publish_special_surface(&mut out, &p, &mut flags, 0, 0.0),
            Some(BoundaryContact { state: 300, packed_surface: None })
        );
        assert_eq!(out.state.flag_69, 1);
        // Feet: Industrial's sea floor packs type 6 with material 0 (surface 768).
        let mut out = PhysicalPlayerInput::default();
        p.state_2508 = 500;
        p.line_tests_960_1008_1056[0].surface = 6 << 7;
        out.off_board.flags_306_307 = [1, 0];
        assert_eq!(
            publish_special_surface(&mut out, &p, &mut flags, 0, 0.0),
            Some(BoundaryContact { state: 500, packed_surface: Some(768) })
        );
        // Water stays a bail (+65), never a boundary request.
        let mut out = PhysicalPlayerInput::default();
        out.collision.flag_214 = 0;
        out.collision.flag_217 = 1;
        p.state_2508 = 300;
        assert_eq!(publish_special_surface(&mut out, &p, &mut flags, 0, 0.0), None);
    }
    // 82DB81CC..8240: ragdoll contacts outrank prediction and never re-bail.
    #[test]
    fn ragdoll_contacts_outrank_prediction_without_a_new_bail() {
        let mut out = PhysicalPlayerInput::default();
        let mut p = ProcessedPhysicsInput::default();
        out.collision.flag_214 = 1;
        out.collision.flag_217 = 1;
        out.collision.flag_3483 = 1;
        out.collision.surface_height_208 = 6.0;
        out.collision.predicted_position_64[1] = 10.0f32.to_bits();
        let f = run(300, &mut out, &mut p, 0);
        assert!(!f[13]);
        assert!(f[17]);
        assert_eq!(out.collision.scalar_28, 6.0);
        out.collision.flag_217 = 0;
        run(300, &mut out, &mut p, 0);
        assert_eq!(out.collision.scalar_28, 10.0);
    }
    // 82DB847C..8520: plant contacts require their own validity fields,
    // and preserve the packed surface category rather than the low material ID.
    #[test]
    fn plant_contacts_use_the_native_validity_and_packed_surface() {
        for state in [601, 602] {
            for valid in [false, true] {
                for surface in [0, 6, 9, 12] {
                    let mut out = PhysicalPlayerInput::default();
                    let mut p = ProcessedPhysicsInput::default();
                    out.air.flag_448 = u8::from(valid);
                    out.air.footplant_surface_224 = (surface << 7) | 37;
                    out.air.footplant_surface_height_216 = 7.0;
                    let line = &mut p.line_tests_960_1008_1056[2];
                    line.valid = u8::from(valid);
                    line.surface = (surface << 7) | 37;
                    line.position[1] = 7.0f32.to_bits();
                    let flags = run(state, &mut out, &mut p, 0);
                    assert_eq!(flags[13], valid && matches!(surface, 9 | 12));
                    assert_eq!(flags[17], valid && surface == 6);
                    assert_eq!(out.collision.flag_3481 != 0, valid && surface == 12);
                    if valid && surface == 12 {
                        assert_eq!(out.collision.scalar_28, 7.0);
                    }
                }
            }
        }
    }
}
