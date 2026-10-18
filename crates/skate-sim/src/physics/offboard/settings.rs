//! Stock Biped settings82D7B630 and metric queries82D7AFD8/82D16680.
mod board;
mod curves;
mod metrics;
#[cfg(test)]
mod tests;
use skate_core::{
    player::offboard::{air_launch, controller, movement_intent, movement_velocity},
    point_graph::PointGraph,
};
use skate_data::{animation_metadata::AnimationMetadata, collections::Collections};

/// Object-move curves of the inputlistener collection read by 8259C4B0
/// (`skate_core::input::offboard_intentions::produce_object_move`).
pub(crate) fn load_object_move_curves(
    data: &Collections,
) -> Result<skate_core::input::offboard_intentions::ObjectMoveCurves, String> {
    Ok(skate_core::input::offboard_intentions::ObjectMoveCurves {
        x_gain: curves::load::<8>(data, "inputlistener", "Hash_1A1A7AC37A72DF87")?.0,
        z_gain: curves::load::<8>(data, "inputlistener", "Hash_05BA8B52C23B3481")?.0,
        rotation: curves::load::<16>(data, "inputlistener", "Hash_9ADFC2E222938C1E")?.0,
    })
}

/// Move Object tuning, attribute class `3EDA5B140604613D` key `default`
/// (read by 82D444A0 / 82D45318). Image constants keep their defaults.
pub(crate) fn load_move_object_tuning(
    data: &Collections,
) -> Result<skate_core::player::offboard::move_object::MoveObjectTuning, String> {
    use skate_core::player::offboard::move_object::{ControllerGains, MoveObjectTuning};
    const CLASS: &str = "Hash_3EDA5B140604613D";
    let f = |name: &str| data.float(CLASS, "default", name);
    let curve = |name: &str| curves::load::<8>(data, CLASS, name).map(|c| c.0);
    let gains = |name: &str| data.words::<4>(CLASS, "default", name).map(ControllerGains::from_words);
    // 82D46610 hand IK window: the larger x end (bounds[2]) of the two enter curves.
    let (hand_ik_curve, ik_bounds) = curves::load::<8>(data, CLASS, "Hash_702F25BA3A5AAA56")?;
    let (_, weight_bounds) = curves::load::<8>(data, CLASS, "Hash_1348E9A1F213B42D")?;
    Ok(MoveObjectTuning {
        hand_ik_enter: f("Hash_5E35DB02BE697A58")?,
        hand_ik_window: ik_bounds[2].max(weight_bounds[2]),
        hand_ik_curve,
        push_speed: f("Hash_2258076B612569A9")?,
        pull_speed: f("Hash_F1C038722EC7D0C6")?,
        side_speed: f("Hash_096A4FA6489E5541")?,
        lever_rotation: curve("Hash_E4FF0185DA44CDBD")?,
        lever_yaw: curve("Hash_BFB3BEF0BB2661C0")?,
        mass_speed: curve("Hash_57D37D696363167E")?,
        inertia_yaw_gain: curve("Hash_EABFCC79873A2859")?,
        yaw_clamp: f("Hash_AD327350D151B1E3")?,
        linear_clamp: f("Hash_791421DDAF54C2D5")?,
        relatch: f("Hash_557FA142008FD7CE")?,
        lift_gain: f("Hash_FDE807D9B85A6AC2")?,
        linear_controller: gains("Hash_DF79539DBDA006EE")?,
        yaw_controller: gains("Hash_B46764285AD1DC5F")?,
        // 82D46610 / 82D43B20: anchor reach from this collection.
        anchor_reach: f("Hash_96ECC98838ECCC11")?,
        // Hold qualification (82D44A10 -> 82E08EE8): physics_state_offboard
        // `default` +0 / +32 / +452 / +436 (the settings at state+56).
        hold_box_extents: offboard_vector(data, "GrabBoxSizeGrabbing")?,
        hold_box_offset: offboard_vector(data, "GrabBoxOffset")?,
        hold_angle_limit: data.float("physics_state_offboard", "default", "GrabSplineAngleLimitGrabbing")?,
        hold_max_angle_to_horizontal: data.float("physics_state_offboard", "default", "GrabSplineMaxAngleToHorizontalGrabbing")?,
        // 82D444A0 grip clamp: physics_state_offboard `default` +444.
        grab_end_exclusion: data.float("physics_state_offboard", "default", "GrabSplineEndExclusion")?,
        ..MoveObjectTuning::default()
    })
}

fn offboard_vector(data: &Collections, name: &str) -> Result<[f32; 3], String> {
    let v = data.words::<4>("physics_state_offboard", "default", name)?.map(f32::from_bits);
    if v.iter().any(|x| !x.is_finite()) {
        return Err(format!("{name}: non-finite vector"));
    }
    Ok([v[0], v[1], v[2]])
}

pub(crate) struct Settings {
    pub controller: controller::Settings,
    pub board: skate_core::player::offboard::ground_sync::BoardSettings,
    pub metrics: [Option<controller::ClipMetric>; 3],
    ///82D310F8 full key DF759B46440F16E9.
    pub movement_vs_stick_angle: PointGraph<8>,
    ///82D310F8 full key2DD95B399BAE313E.
    pub turn_vs_stick_angle: PointGraph<8>,
    pub air_launch: air_launch::Settings,
}
impl Settings {
    /// Share the same native ABIN metadata loaded for SkaterAnimation. Required
    /// stock fields/banks/clips fail loading; an absent matching attribute is None.
    pub(crate) fn load(data: &Collections, metadata: &AnimationMetadata) -> Result<Self, String> {
        let biped = |name| curves::load::<8>(data, "physics_biped", name);
        let (sprint_blend, bounds) = biped("Hash_6B93C51256A30FB4")?;
        Ok(Self {
            controller: controller::Settings {
                movement_intent: movement_intent::Settings {
                    sprint_speed: curves::load::<4>(
                        data,
                        "physics_biped",
                        "Hash_7209DCFDF3015EBF",
                    )?
                    .0,
                    normal_speed: curves::load::<16>(data, "physics_biped", "SpeedVsInput")?.0,
                    sprint_blend,
                    sprint_time_cap: bounds[2],
                    slide_steering: biped("AutoTurnVsAngle")?.0,
                },
                movement_velocity: movement_velocity::Settings {
                    slope_speed_scalar: biped("Hash_31309236050A8F09")?.0,
                    slope_mode_speed: biped("Hash_CE45C724B30F9134")?.0,
                    turn_vs_speed: biped("TurnVsSpeed")?.0,
                    turn_delta_vs_speed: biped("TurnDeltaVsSpeed")?.0,
                },
                slide_vs_slope: biped("SlideVsSlope")?.0,
                slide_vs_speed: biped("SlideVsSpeed")?.0,
            },
            metrics: metrics::load(metadata)?,
            board: board::load(data)?,
            movement_vs_stick_angle: curves::load::<8>(
                data,
                "physics_state_offboard",
                "Hash_DF759B46440F16E9",
            )?
            .0,
            turn_vs_stick_angle: curves::load::<8>(
                data,
                "physics_state_offboard",
                "TurnVsStickAngle",
            )?
            .0,
            air_launch: air_launch::Settings {
                jump_speed_scalar: data.float("physics_biped", "default", "JumpSpeedScalar")?,
                jump_height: data.float("physics_biped", "default", "JumpHeight")?,
            },
        })
    }

    pub(crate) fn into_controller_parts(
        self,
    ) -> (
        controller::Settings,
        [Option<controller::ClipMetric>; 3],
        PointGraph<8>,
        PointGraph<8>,
        air_launch::Settings,
    ) {
        (
            self.controller,
            self.metrics,
            self.movement_vs_stick_angle,
            self.turn_vs_stick_angle,
            self.air_launch,
        )
    }
}
