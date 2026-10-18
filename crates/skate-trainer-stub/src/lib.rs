//! Replaces `skate_mods::TrainerTuning` for `skate-sim` (PLAN.md §5.2).
//!
//! Copied from `skate-3-rust-engine/crates/skate-mods/src/schema.rs` (`cb79689`) with the
//! serde derives and attributes removed; the fields, `Default` values and `valid()` are
//! unchanged. The defaults (all multipliers 1.0, `hold_fakie` false) mean "no trainer", and
//! the simulation only retunes ground settings when the value differs from `Default`.

/// Whitelisted native trainer multipliers. 1.0 preserves the installed stock value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TrainerTuning {
    pub pop: f32,
    pub grind_pop: f32,
    pub push_speed: f32,
    pub push_power: f32,
    pub braking: f32,
    pub steering: f32,
    pub wobble: f32,
    pub offboard_jump: f32,
    pub grip: f32,
    pub turn_power: f32,
    pub manual_drag: f32,
    pub hold_fakie: bool,
}
impl Default for TrainerTuning {
    fn default() -> Self {
        Self {
            pop: 1.,
            grind_pop: 1.,
            push_speed: 1.,
            push_power: 1.,
            braking: 1.,
            steering: 1.,
            wobble: 1.,
            offboard_jump: 1.,
            grip: 1.,
            turn_power: 1.,
            manual_drag: 1.,
            hold_fakie: false,
        }
    }
}
impl TrainerTuning {
    pub fn valid(&self) -> bool {
        [
            self.pop,
            self.grind_pop,
            self.push_speed,
            self.push_power,
            self.braking,
            self.steering,
            self.offboard_jump,
            self.grip,
            self.turn_power,
            self.manual_drag,
        ]
        .into_iter()
        .all(|v| v.is_finite() && (0.25..=4.).contains(&v))
            && self.wobble.is_finite()
            && (0. ..=2.).contains(&self.wobble)
    }
}

#[cfg(test)]
mod tests {
    use super::TrainerTuning;

    #[test]
    fn default_is_no_trainer_and_valid() {
        let t = TrainerTuning::default();
        assert!(t.valid());
        assert!(!t.hold_fakie);
        for v in [
            t.pop,
            t.grind_pop,
            t.push_speed,
            t.push_power,
            t.braking,
            t.steering,
            t.wobble,
            t.offboard_jump,
            t.grip,
            t.turn_power,
            t.manual_drag,
        ] {
            assert_eq!(v, 1.0);
        }
    }
}

/// `skate-mods/src/presentation.rs` `CameraShotTuning` (b3c9679), without the serde
/// derives: the stock camera's per-shot tuning hook. Nothing sets one on the Vita, so
/// every shot keeps its stock values.
pub mod presentation {
    /// Replacement values for one stock camera shot (`camera_shots` collection), named
    /// after the retail attributes and in their units (metres, degrees, seconds). Unset
    /// fields keep the stock value. Applies to every place the shot is used, including
    /// blend trees that name it as a child.
    #[derive(Clone, Debug, Default, PartialEq)]
    pub struct CameraShotTuning {
        pub position_distance: Option<f32>,
        pub position_elevation: Option<f32>,
        pub position_heading: Option<f32>,
        pub framing_lens_length: Option<f32>,
        pub framing_roll: Option<f32>,
        pub framing_yaw: Option<f32>,
        pub framing_pitch: Option<f32>,
        pub reference_board_offset: Option<f32>,
        pub smoothing_direction: Option<f32>,
        pub smoothing_elevation: Option<f32>,
        pub smoothing_yaw: Option<f32>,
        pub smoothing_pitch: Option<f32>,
        pub transition_time: Option<f32>,
    }
}
