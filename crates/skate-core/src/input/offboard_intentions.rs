//! Off-board inputs from original TU3 Fill825999F0 and helper8259C260.
//! The stock ActionGraph owns activation; these inputs do not change state.
use crate::madd::Madd as _;
use super::controller::DerivedControllerInput;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OffboardIntent {
    pub name: &'static str,
    pub value: f32,
}

/// Read the already-updated controller prefix exactly once per input tick.
/// `air_reckoning_active` is actual PhysOut bundle+8 byte452, not a state test.
/// Analog movement has a separate physical-output-dependent producer.
pub fn produce_discrete(
    controller: &DerivedControllerInput,
    actor_flags: u32,
    air_reckoning_active: bool,
) -> Vec<OffboardIntent> {
    let words = controller.words();
    let previous = words[6];
    let current = words[13];
    let axis = |slot: usize| f32::from_bits(words[slot]);
    let held = |bit: u32| current & (1u32 << bit) != 0;
    let rising = |bit: u32| held(bit) && previous & (1u32 << bit) == 0;
    let mut output = Vec::with_capacity(12);
    let mut emit = |name, value| output.push(OffboardIntent { name, value });

    //8259AA8C invokes8259C260 before the later toggle/sprint publications.
    if rising(23) {
        emit("OB_Jump", 1.0);
    }
    let right_x = axis(9);
    let right_y = axis(10);
    if right_x != 0.0 {
        emit("OB_LookAtX", right_x);
    }
    if right_y != 0.0 {
        emit("OB_LookAtY", right_y);
    }
    let drop_board = axis(4) != 1.0 && axis(11) == 1.0;
    let throw_board = axis(5) != 1.0 && axis(12) == 1.0;
    if drop_board {
        emit("OB_DropBoard", 1.0);
    }
    if throw_board {
        emit("OB_ThrowBoard", 1.0);
    }
    if drop_board || throw_board {
        emit("OB_RetrieveBoard", 1.0);
    }
    if held(28) || held(30) {
        emit("OB_DoAirBodyTweak", 1.0);
    }
    emit("OB_AirBodyTweakX", right_x);
    emit("OB_AirBodyTweakY", right_y);

    //8259A604..A6C0 computes both;8259AF80 gates only NewToggle by actor bit10.
    //The listener has no biped/category gate. Stock OffBoard AG maps these
    //same toggle intentions to OB_Mount/OB_MountRaw; trigger edges recall.
    if !air_reckoning_active && !held(29) && held(22) {
        //Native bge does not take the branch for unordered timer values.
        if (rising(22) || !(axis(23) >= f32::from_bits(0x3cf5_c28f)))
            && actor_flags & (1 << 10) == 0
        {
            emit("NewToggleOffBoardState", 1.0);
        }
        emit("ToggleOffBoardState", 1.0);
    }
    if !(axis(21) > 0.0) && axis(20) > 0.0 {
        emit("OB_Sprint", 1.0);
    }
    output
}

/// Completed physical publication read by Fill825999F0.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AnalogObservation {
    /// PhysOutSkeleton+0: effective animation root Z, including processed2476bit2.
    pub effective_skeleton_z: [f32; 4],
    /// Some(OffBoard288) iff actual OffBoard333 is nonzero. This vector is not
    /// normalized here. A missing Biped producer must not be converted to None.
    pub biped_correction: Option<[f32; 4]>,
}

///8259A7A8..A9BC calculation and8259AFFC..B088 ordered movement publications.
pub fn produce_analog(
    controller: &DerivedControllerInput,
    observation: AnalogObservation,
) -> [OffboardIntent; 4] {
    let words = controller.words();
    let stick = [f32::from_bits(words[7]), 0.0, f32::from_bits(words[8]), 0.0];
    let mut horizontal = observation.effective_skeleton_z;
    horizontal[1] = 0.0;
    let forward = safe_unit(horizontal);
    let magnitude = if let Some(normal) = observation.biped_correction {
        let negative = normal.map(|value| f32::from_bits(value.to_bits() ^ 0x8000_0000));
        let projection = dot(stick, negative);
        let lower = if -projection >= 0.0 { 0.0 } else { projection };
        let coefficient = if 1.0 - lower >= 0.0 { lower } else { 1.0 };
        //8259A994 ble takes only ordered <=. Unordered follows the zero branch.
        if !(dot(safe_unit(stick), negative) <= f32::from_bits(0x3f66_6666)) {
            0.0
        } else {
            let corrected = std::array::from_fn(|i| normal[i].madd(coefficient, stick[i]));
            dot(forward, corrected)
        }
    } else {
        dot(forward, stick)
    };
    [
        OffboardIntent { name: "OB_Mag", value: magnitude },
        OffboardIntent { name: "OB_BipedWorldZ", value: stick[2] },
        OffboardIntent { name: "OB_BipedWorldX", value: stick[0] },
        OffboardIntent { name: "OB_BipedStickMag", value: super::controller::magnitude(dot(stick, stick)) },
    ]
}

fn dot(a: [f32; 4], b: [f32; 4]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// Shipped inputlistener curves read by the object-move producer 8259C4B0,
/// all evaluated at |stick angle| / pi (angle = atan2(x, -y) of the raw left
/// stick, constant 0x3EA2F983 at 822F8610). Defaults come from setup data; a
/// mod may override them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ObjectMoveCurves {
    /// Key 1A1A7AC37A72DF87 (PointNegGraphData8): OB_ObjectMvX gain.
    pub x_gain: crate::point_graph::PointGraph<8>,
    /// Key 05BA8B52C23B3481 (PointNegGraphData8): OB_ObjectMvZ gain.
    pub z_gain: crate::point_graph::PointGraph<8>,
    /// Key 9ADFC2E222938C1E (PointNegGraphData16): left-stick share of
    /// OB_ObjectMvRot. Every shipped Y value is zero, so in retail the left
    /// stick never rotates the skater and object; only the right stick does.
    pub rotation: crate::point_graph::PointGraph<16>,
}

impl ObjectMoveCurves {
    /// Unit gains and no left-stick rotation (the shipped shape without the
    /// Z dip on diagonals); for tests and hosts without setup data.
    pub fn flat() -> Self {
        Self {
            x_gain: crate::point_graph::PointGraph { x: [0.0; 8], y: [1.0; 8] },
            z_gain: crate::point_graph::PointGraph { x: [0.0; 8], y: [1.0; 8] },
            rotation: crate::point_graph::PointGraph { x: [0.0; 16], y: [0.0; 16] },
        }
    }
}

/// Object-move inputs while carrying a prop (retail Move Object mode),
/// 8259C4B0 called from Fill825999F0 with the current RawControllerInput
/// (derived words 7..13). The stock OffBoard AG forwards all three to the MG,
/// where MovingObjectNew attaches them to the same-named skeleton-input
/// channels:
/// - OB_ObjectMvX = raw left X * x_gain(|a|/pi);
/// - OB_ObjectMvZ = raw left Y * z_gain(|a|/pi);
/// - OB_ObjectMvRot = clamp(raw right X + sign(a) * rotation(|a|/pi) * s, -1, 1).
///
/// `a` is atan2(x, -y) wrapped to (-pi, pi]. The scale `s` (caller f21) is
/// not resolved; it only multiplies the rotation curve, which is all zero in
/// the shipped data, so the left-stick term is left out.
pub fn produce_object_move(
    controller: &DerivedControllerInput,
    curves: &ObjectMoveCurves,
) -> [OffboardIntent; 3] {
    let words = controller.words();
    let x = f32::from_bits(words[7]);
    let y = f32::from_bits(words[8]);
    let right_x = f32::from_bits(words[9]);
    let angle = x.atan2(-y);
    let key = angle.abs() * f32::from_bits(0x3ea2_f983);
    [
        OffboardIntent { name: "OB_ObjectMvZ", value: curves.z_gain.evaluate(key) * y },
        OffboardIntent { name: "OB_ObjectMvX", value: curves.x_gain.evaluate(key) * x },
        OffboardIntent { name: "OB_ObjectMvRot", value: right_x.clamp(-1.0, 1.0) },
    ]
}

fn safe_unit(vector: [f32; 4]) -> [f32; 4] {
    let squared = dot(vector, vector);
    let mut inverse = crate::physics::reciprocal_sqrt::estimate(squared);
    for _ in 0..2 {
        let correction = (-squared).madd(inverse * inverse, 1.0);
        inverse = (inverse * 0.5).madd(correction, inverse);
    }
    let length = if squared == 0.0 { 0.0 } else { squared * inverse };
    //Initializer82F826F8 supplies830BD350 from82181A88: LENGTH threshold1e-6.
    if length > f32::from_bits(0x3586_37bd) {
        vector.map(|value| value * inverse)
    } else {
        [0.0; 4]
    }
}
#[cfg(test)]
#[path = "offboard_intentions/tests.rs"]
mod tests;
