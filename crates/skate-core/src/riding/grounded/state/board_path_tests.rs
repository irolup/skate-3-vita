use super::*;

fn v(x: f32, y: f32, z: f32) -> Vector3 {
    Vector3::new(x, y, z)
}

const IDENTITY: Basis3 = Basis3 { columns: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]] };

#[test]
fn approach_steps_at_most_the_maximum_and_ignores_tiny_errors() {
    let t = PhysicsAiTuning::DEFAULT_RECORD;
    // 1 m off: 2 cm per tick.
    let p = approach(v(0.0, 0.0, 0.0), v(1.0, 0.0, 0.0), t.position_gain, t.position_max_step);
    assert!((p.x - 0.02).abs() < 1e-6 && p.y == 0.0 && p.z == 0.0, "{p:?}");
    // 1 cm off: the whole error (gain 1).
    let p = approach(v(0.0, 0.0, 0.0), v(0.0, 0.01, 0.0), t.position_gain, t.position_max_step);
    assert!((p.y - 0.01).abs() < 1e-6, "{p:?}");
    // At most 1e-6: no move.
    assert_eq!(approach(v(1.0, 2.0, 3.0), v(1.0, 2.0, 3.0 + 5e-7), 1.0, 1.0), v(1.0, 2.0, 3.0));
    // Velocity: gain 0.5 of a 0.3 m/s error = 0.15, under the 0.2 cap; 4 m/s -> capped 0.2.
    let u = approach(v(5.0, 0.0, 0.0), v(5.3, 0.0, 0.0), t.velocity_gain, t.velocity_max_change);
    assert!((u.x - 5.15).abs() < 1e-5, "{u:?}");
    let u = approach(v(5.0, 0.0, 0.0), v(9.0, 0.0, 0.0), t.velocity_gain, t.velocity_max_change);
    assert!((u.x - 5.2).abs() < 1e-5, "{u:?}");
}

#[test]
fn facing_turns_toward_the_target_at_most_two_degrees_a_tick() {
    let t = PhysicsAiTuning::DEFAULT_RECORD;
    let max = 2.0f32.to_radians();
    // Target 90 deg to +X: positive angle from +Z, clamped.
    assert!((facing_step(&IDENTITY, v(1.0, 0.0, 0.0), t.facing_gain, t.facing_max_degrees) - max).abs() < 1e-6);
    // Target 90 deg to -X: wrapped to -90 deg, clamped negative.
    assert!((facing_step(&IDENTITY, v(-1.0, 0.0, 0.0), t.facing_gain, t.facing_max_degrees) + max).abs() < 1e-6);
    // A small error: half of it (gain 0.5).
    let a = 1.0f32.to_radians();
    let s = facing_step(&IDENTITY, v(a.sin(), 0.0, a.cos()), t.facing_gain, t.facing_max_degrees);
    assert!((s - a * 0.5).abs() < 1e-4, "{s}");
    // Aligned, straight up, or zero: nothing.
    // (retail acos(1) is 2.4e-7, not exactly 0)
    assert!(facing_step(&IDENTITY, v(0.0, 0.0, 1.0), 0.5, 2.0).abs() < 1e-6);
    assert_eq!(facing_step(&IDENTITY, v(0.0, 1.0, 0.0), 0.5, 2.0), 0.0);
    // In a frame turned 90 deg about up, the same world target is aligned with the frame's At.
    let turned = Basis3 { columns: [[0.0, 0.0, -1.0], [0.0, 1.0, 0.0], [1.0, 0.0, 0.0]] };
    assert!(facing_step(&turned, v(1.0, 0.0, 0.0), 0.5, 2.0).abs() < 1e-6);
}

#[test]
fn the_deck_rotates_about_the_ground_up_and_keeps_its_position() {
    let deck = RetailAffineTransform { basis: IDENTITY, translation: v(3.0, 1.0, -2.0) };
    let r = rotate_about_ground_up(deck, &IDENTITY, 0.1);
    assert_eq!(r.translation, deck.translation);
    let at = r.basis.columns[2];
    assert!((at[0] - 0.1f32.sin()).abs() < 1e-6 && (at[2] - 0.1f32.cos()).abs() < 1e-6 && at[1].abs() < 1e-7, "{at:?}");
    assert_eq!(r.basis.columns[1], [0.0, 1.0, 0.0]);
    // The forward moves toward the target the step was measured against.
    let step = facing_step(&IDENTITY, v(1.0, 0.0, 0.0), 0.5, 2.0);
    let r = rotate_about_ground_up(deck, &IDENTITY, step);
    assert!(r.basis.columns[2][0] > 0.0);
}

#[derive(Default)]
struct Board {
    deck: Option<RetailAffineTransform>,
    velocity: Vector3,
    calls: Vec<&'static str>,
}

impl BoardPathServices for Board {
    fn deck_transform(&self) -> RetailAffineTransform {
        self.deck.unwrap_or(RetailAffineTransform { basis: IDENTITY, translation: v(0.0, 0.0, 0.0) })
    }
    fn set_deck_transform(&mut self, transform: RetailAffineTransform) {
        self.calls.push("deck");
        self.deck = Some(transform);
    }
    fn deck_velocity(&self) -> Vector3 {
        self.velocity
    }
    fn set_deck_velocity(&mut self, velocity: Vector3) {
        self.calls.push("velocity");
        self.velocity = velocity;
    }
    fn ground_frame(&self) -> Basis3 {
        IDENTITY
    }
    fn set_board_transform(&mut self, transform: RetailAffineTransform) {
        self.calls.push("board");
        self.deck = Some(transform);
    }
}

#[test]
fn the_board_path_runs_position_velocity_facing_by_flag() {
    let words = |x: f32, y: f32, z: f32| [x.to_bits(), y.to_bits(), z.to_bits(), 0];
    let mut vectors = [[0u32; 4]; 10];
    vectors[2] = words(1.0, 0.0, 0.0);
    vectors[3] = words(1.0, 0.0, 0.0);
    vectors[4] = words(0.0, 0.0, 3.0);
    let all = flags::STEER | flags::POSITION | flags::VELOCITY | flags::FACING;
    let target = SteerTarget::from_words(&vectors, all);
    let mut b = Board::default();
    update_board_path(&target, &PhysicsAiTuning::DEFAULT_RECORD, &mut b);
    assert_eq!(b.calls, ["deck", "velocity", "board"], "retail order");
    let d = b.deck.unwrap();
    assert!((d.translation.x - 0.02).abs() < 1e-6, "position step kept by the facing set");
    assert!((b.velocity.z - 0.2).abs() < 1e-6);
    assert!(d.basis.columns[2][0] > 0.0, "turned toward +X");
    // Only the flagged steps run.
    let mut b = Board::default();
    update_board_path(&SteerTarget { flags: flags::STEER | flags::VELOCITY, ..target }, &PhysicsAiTuning::DEFAULT_RECORD, &mut b);
    assert_eq!(b.calls, ["velocity"]);
}
