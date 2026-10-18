use super::*;
use skate_core::player::offboard::grab_scene::{AssemblyData, Geometry, Object, Provider, Spline};
use std::sync::Arc;

fn car_record(z: f32, kind: u32) -> Record {
    let object = Object {
        id: 9,
        provider: if kind == 1 { Provider::Vehicle } else { Provider::LivingWorld },
        disabled: false,
        assembly_ready: true,
        assembly: Some(AssemblyData { identity: 9, first_part: None }),
        frame: [[1., 0., 0., 0.], [0., 1., 0., 0.], [0., 0., 1., 0.], [0., 0., z, 1.]],
        object_vector_128: [0., 0., 5., 0.],
        splines: vec![],
    };
    let spline = Spline {
        descriptor: Descriptor { kind, id: 77 },
        geometry: Arc::new(Geometry { id: 77, points: vec![[0.8, 0.9, 0., 1.], [-0.8, 0.9, 0., 1.]], approach_vectors: vec![[0., 0., -1., 0.]], word_60: 0 }),
        word_272: 0,
    };
    object.record(&spline).unwrap()
}

fn decide(skater_z: f32, records: &[Record], cooldown: f32, last: u32) -> LatchDecision {
    let s = SkitchQuerySettings::default();
    let frame = [[1., 0., 0., 0.], [0., 1., 0., 0.], [0., 0., 1., 0.], [0., 0., skater_z, 1.]];
    let (bounds, limits) = query_shape(frame, &s);
    let mut p = ProcessedPhysicsInput::default();
    p.ground_timer_2848 = cooldown;
    p.skitch_value_2592 = last;
    latch(records, frame, [0., 0., skater_z, 1.], [0., 0., 6., 0.], [0.; 4], bounds, limits, &p, &s)
}

#[test]
fn the_box_reaches_two_metres_ahead_along_the_frame_forward() {
    let frame = [[1., 0., 0., 0.], [0., 1., 0., 0.], [0., 0., 1., 0.], [0., 0., 0., 1.]];
    let (b, l) = query_shape(frame, &SkitchQuerySettings::default());
    assert_eq!((b.frame[2], b.frame[0], b.frame[1]), ([0., 0., 1., 0.], [1., 0., 0., 0.], [0., 1., 0., 0.]));
    assert_eq!(b.frame[3], [0., 0.86, 2., 1.]);
    assert!((l.angle_a - 60f32.to_radians()).abs() < 1e-5 && (l.margin - 0.25).abs() < 1e-6);
}

/// `82D39D98` (b149 / b150): the latch point is the edge's centreline crossing (x = 0 in the box frame) and `+2668`
/// its height above the board deck, written for a gated candidate even when it does not latch.
#[test]
fn the_latch_height_is_the_crossing_above_the_deck() {
    let far = decide(0.0, &[car_record(1.5, 1)], 0.0, 0);
    assert_eq!(far.latch, None);
    assert!((far.height.unwrap() - 0.9).abs() < 1e-5, "{far:?}");
    let s = SkitchQuerySettings::default();
    let frame = [[1., 0., 0., 0.], [0., 1., 0., 0.], [0., 0., 1., 0.], [0., 0., 0., 1.]];
    let (bounds, limits) = query_shape(frame, &s);
    let raised = latch(&[car_record(1.5, 1)], frame, [0., 0., 0., 1.], [0., 0., 6., 0.], [0., 0.3, 0., 1.], bounds, limits, &ProcessedPhysicsInput::default(), &s);
    assert!((raised.height.unwrap() - 0.6).abs() < 1e-5, "{raised:?}");
}

#[test]
fn a_close_car_latches_and_a_farther_one_only_binds() {
    // Car edge 1.5 m ahead, closing 1 m/s: (1.5 - 1.0) / 1 = 0.5 s, bound but not latched.
    let far = decide(0.0, &[car_record(1.5, 1)], 0.0, 0);
    assert_eq!(far.bound, Some(Descriptor { kind: 1, id: 77 }));
    assert!((far.time.unwrap() - 0.5).abs() < 1e-4, "{far:?}");
    assert_eq!(far.latch, None);
    // 1.05 m ahead: 0.05 s, latched.
    let near = decide(0.45, &[car_record(1.5, 1)], 0.0, 0);
    assert_eq!(near.latch, Some((1, 77)), "{near:?}");
    // The re-grab cooldown skips the car skitched last; another one still latches.
    assert_eq!(decide(0.45, &[car_record(1.5, 1)], 1.0, 77).latch, None);
    assert_eq!(decide(0.45, &[car_record(1.5, 1)], 1.0, 5).latch, Some((1, 77)));
}

#[test]
fn world_objects_are_not_latched_yet() {
    assert_eq!(decide(0.45, &[car_record(1.5, 2)], 0.0, 0), LatchDecision::default());
}

#[test]
fn the_latch_rechecks_with_its_own_wider_limits() {
    // 82D39D98: margin 0, angles 60 / 40 degrees (the query uses 0.25 / 60 / 30).
    let s = SkitchQuerySettings::default();
    let (_, query) = super::query_shape([[1., 0., 0., 0.], [0., 1., 0., 0.], [0., 0., 1., 0.], [0., 0., 0., 1.]], &s);
    let latch = super::latch_limits(&s);
    assert_eq!(latch.margin, 0.0);
    assert!(latch.margin < query.margin && latch.angle_b > query.angle_b && latch.angle_a == query.angle_a);
    assert!((latch.angle_b.to_degrees() - 40.0).abs() < 1e-3);
}
