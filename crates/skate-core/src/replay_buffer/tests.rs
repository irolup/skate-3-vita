use super::*;

const DT: f64 = 1.0 / 60.0;

fn yaw(angle: f32) -> [f32; 4] {
    [0.0, (angle / 2.0).sin(), 0.0, (angle / 2.0).cos()]
}

fn pose(x: f32, angle: f32) -> ActorPose {
    ActorPose {
        root: JointPose { translation: [x, 0.0, 0.0], rotation: yaw(angle), ..JointPose::default() },
        joints: vec![
            JointPose { translation: [0.0, 1.0, 0.0], rotation: yaw(angle * 2.0), ..JointPose::default() },
            JointPose { translation: [0.0, 0.5, 0.1], rotation: yaw(-angle), ..JointPose::default() },
        ],
        hidden: false,
    }
}

fn angle_between(a: [f32; 4], b: [f32; 4]) -> f32 {
    let dot: f32 = a.iter().zip(&b).map(|(x, y)| x * y).sum();
    2.0 * dot.abs().min(1.0).acos()
}

#[test]
fn replay_buffer_quantises_rotations_within_retail_precision() {
    let mut buffer = ReplayBuffer::default();
    buffer.begin_frame(0.0);
    let mut p = pose(12.345, 1.234);
    p.joints[1].translation = [0.3, -0.2, 0.05];
    p.joints[0].rotation = [-0.3, 0.4, 0.5, -0.7071];
    buffer.record(7, &p);
    let out = buffer.sample(7, 0.0).unwrap();
    assert_eq!(out.root.translation, p.root.translation);
    assert_eq!(out.joints[1].translation, [0.3, -0.2, 0.05]);
    for (a, b) in out.joints.iter().zip(&p.joints) {
        let q = normalise(b.rotation);
        assert!(angle_between(a.rotation, q) < 2.0e-4, "{:?} vs {:?}", a.rotation, q);
    }
}

#[test]
fn replay_buffer_writes_a_key_only_when_the_pose_changes_and_holds_across_gaps() {
    let mut buffer = ReplayBuffer::default();
    for i in 0..10 {
        buffer.begin_frame(i as f64 * DT);
        buffer.record(1, &pose(0.0, 0.0));
    }
    assert_eq!(buffer.tracks[&1].keys.len(), 1);
    buffer.begin_frame(10.0 * DT);
    buffer.record(1, &pose(1.0, 0.0));
    // The held pose repeats at frame 9, so frame 9.5 blends 9 -> 10 and frame 5 still holds.
    assert_eq!(buffer.tracks[&1].keys.len(), 3);
    assert_eq!(buffer.sample(1, 5.0 * DT).unwrap().root.translation[0], 0.0);
    let mid = buffer.sample(1, 9.5 * DT).unwrap().root.translation[0];
    assert!((mid - 0.5).abs() < 1.0e-3, "{mid}");
}

#[test]
fn replay_buffer_hides_an_actor_before_it_spawned_and_after_it_left() {
    let mut buffer = ReplayBuffer::default();
    buffer.begin_frame(0.0);
    buffer.begin_frame(DT);
    buffer.record(2, &pose(0.0, 0.0));
    buffer.begin_frame(2.0 * DT);
    buffer.record(2, &pose(1.0, 0.0));
    buffer.begin_frame(3.0 * DT);
    buffer.end(2);
    assert!(buffer.sample(2, 0.0).is_none());
    assert!(buffer.sample(2, 1.5 * DT).is_some());
    assert!(buffer.sample(2, 2.5 * DT).is_some());
    assert!(buffer.sample(2, 3.0 * DT).is_none());
    assert!(buffer.sample(99, DT).is_none());
}

#[test]
fn replay_buffer_nlerps_joints_between_keys() {
    let mut buffer = ReplayBuffer::default();
    buffer.begin_frame(0.0);
    buffer.record(3, &pose(0.0, 0.0));
    buffer.begin_frame(DT);
    buffer.record(3, &pose(2.0, 1.0));
    let mid = buffer.sample(3, 0.5 * DT).unwrap();
    assert!((mid.root.translation[0] - 1.0).abs() < 1.0e-5);
    // Joint 0 turns 0 -> 2 rad: nlerp's midpoint is the 1 rad rotation.
    assert!(angle_between(mid.joints[0].rotation, yaw(1.0)) < 2.0e-3);
    assert!((mid.joints[0].rotation.iter().map(|c| c * c).sum::<f32>() - 1.0).abs() < 1.0e-5);
}

#[test]
fn replay_buffer_evicts_the_oldest_frames_at_the_retail_frame_cap() {
    let mut buffer = ReplayBuffer::new(ReplayLimits { max_frames: 120, ..ReplayLimits::default() });
    buffer.begin_frame(0.0);
    buffer.record(4, &pose(0.0, 0.0));
    buffer.record(5, &pose(0.0, 0.0));
    buffer.begin_frame(DT);
    buffer.end(5);
    for i in 2..400 {
        buffer.begin_frame(i as f64 * DT);
        buffer.record(4, &pose(i as f32, 0.0));
    }
    assert_eq!(buffer.frame_count(), 120);
    let (start, end) = buffer.bounds().unwrap();
    assert!((end - start - 119.0 * DT).abs() < 1.0e-9);
    // The ended actor is gone; the live one keeps the key that holds at the window start.
    assert!(!buffer.ids().any(|id| id == 5));
    assert!(buffer.tracks[&4].keys.len() <= 120 + TRIM_EVERY + 1);
    assert!(buffer.tracks[&4].keys[0].time <= start);
    assert!(buffer.sample(4, start).is_some());
    assert_eq!(ReplayLimits::default().max_frames, 5400);
}

#[test]
fn replay_buffer_evicts_by_bytes_when_the_budget_fills_first() {
    let limits = ReplayLimits { max_frames: 5400, byte_budget: 20_000 };
    let mut buffer = ReplayBuffer::new(limits);
    for i in 0..2000 {
        buffer.begin_frame(i as f64 * DT);
        buffer.record(6, &pose(i as f32, 0.0));
    }
    assert!(buffer.bytes() <= limits.byte_budget, "{}", buffer.bytes());
    assert!(buffer.frame_count() < 2000);
    let (start, end) = buffer.bounds().unwrap();
    assert!(buffer.sample(6, start).is_some() && buffer.sample(6, end).is_some());
    // Byte accounting stays exact through eviction.
    let counted: usize = buffer.tracks.values().map(Track::bytes).sum();
    assert_eq!(counted, buffer.bytes());
}

#[test]
fn replay_buffer_clears_when_the_clock_goes_back() {
    let mut buffer = ReplayBuffer::default();
    buffer.begin_frame(5.0);
    buffer.record(1, &pose(0.0, 0.0));
    buffer.begin_frame(1.0);
    assert!(buffer.sample(1, 5.0).is_none());
    assert_eq!(buffer.frame_count(), 1);
    assert_eq!(buffer.bytes(), 0);
}
