//! Recovered board-part mass properties, separate from integration.
use crate::madd::Madd as _;
use super::{
    native_arithmetic,
    rigid_body::{RetailBodyMassProperties, RetailInertiaDynamics, RetailLocalMassFrame},
};
use crate::math::{Basis3, Vector3};

#[path = "construction/primitive_mass.rs"]
mod primitive_mass;
pub use primitive_mass::{MassShape, PrimitiveMass, primitive_mass};
#[path = "construction/board_parts.rs"]
mod board_parts;
pub use board_parts::{
    ForwardMassProperties, PartMassInput, TruckMassSettings, WheelMassSettings,
    forward_mass_properties, truck_mass_input, wheel_mass_input,
};

#[path = "construction/mass_moments.rs"]
mod mass_moments;
#[path = "construction/principal_axes.rs"]
mod principal_axes;
pub use mass_moments::{AggregateMassProperties, MassMoments};
#[path = "construction/deck_geometry.rs"]
mod deck_geometry;
pub use deck_geometry::{DeckChild, DeckGeometry, DeckGeometrySettings, DeckShape};

/// TU3 ComputeMassProperties82AE7770 for one centered primitive. Skeleton
/// parts use this same finalizer as board parts, including volume fallback
/// when the requested mass is strictly below MIN_POSITIVE; NaN is preserved.
pub fn primitive_mass_properties(
    input: PartMassInput,
    maximum_angular_velocity: f32,
    angular_drag: f32,
) -> Option<RetailBodyMassProperties> {
    let primitive = primitive_mass(input.shape)?;
    let mass = if input.requested_mass < f32::MIN_POSITIVE {
        primitive.volume
    } else {
        input.requested_mass
    };
    let inertia = primitive.moments_per_unit_mass;
    Some(finalize_principal_mass(
        RetailLocalMassFrame::IDENTITY,
        mass,
        Vector3::new(inertia.x * mass, inertia.y * mass, inertia.z * mass),
        maximum_angular_velocity,
        angular_drag,
    ))
}

/// TU3 ComputeMassProperties82AE7770 after compound-volume accumulation.
/// The caller may apply a source-owned part-frame override after this result.
pub fn aggregate_mass_properties(
    mut moments: MassMoments,
    requested_mass: f32,
    maximum_angular_velocity: f32,
    angular_drag: f32,
) -> RetailBodyMassProperties {
    let properties = moments.principal_properties();
    // 82AE79EC BO4/BI24 skips fallback when CR6.LT is clear, including NaN.
    let mass = if requested_mass < f32::MIN_POSITIVE {
        properties.volume
    } else {
        requested_mass
    };
    let inertia = properties.moments_per_unit_mass;
    finalize_principal_mass(
        inverse_mass_frame(properties.local_mass_frame, inertia),
        mass,
        Vector3::new(inertia.x * mass, inertia.y * mass, inertia.z * mass),
        maximum_angular_velocity,
        angular_drag,
    )
}

/// 82AE78BC..79DC: omit negligible local-frame corrections, otherwise store
/// inverseBodyLTM. The inverse translation starts with Z, then adds Y and X
/// through fused operations, as in the native matrix transpose/inverse path.
fn inverse_mass_frame(forward: RetailLocalMassFrame, inertia: Vector3) -> RetailLocalMassFrame {
    let center = forward.translation;
    let center_squared = native_arithmetic::dot3(
        [center.x, center.y, center.z, 0.0],
        [center.x, center.y, center.z, 0.0],
    );
    let minimum_offset_squared =
        ((inertia.y + inertia.z) + inertia.x) * f32::from_bits(0x3586_37BE);
    // S3 82AE78EC / S2 82AE9BE0: bge after fcmpu means LT is clear,
    // so an unordered comparison also retains the inverse mass frame.
    let translated = !(center_squared < minimum_offset_squared);
    let rotated = forward
        .basis
        .columns
        .iter()
        .enumerate()
        .any(|(column, axis)| {
            axis.iter().enumerate().any(|(row, value)| {
                let identity = if column == row { 1.0 } else { 0.0 };
                (*value - identity).abs() > f32::from_bits(0x3A83_126F)
            })
        });
    if !translated && !rotated {
        return RetailLocalMassFrame::IDENTITY;
    }
    let columns = core::array::from_fn::<_, 3, _>(|column| {
        core::array::from_fn(|row| forward.basis.columns[row][column])
    });
    let negative_center = [0.0 - center.x, 0.0 - center.y, 0.0 - center.z];
    let translation = core::array::from_fn::<_, 3, _>(|row| {
        negative_center[0].madd(
            columns[0][row],
            negative_center[1].madd(columns[1][row], negative_center[2] * columns[2][row]),
        )
    });
    RetailLocalMassFrame {
        basis: Basis3 { columns },
        translation: Vector3::new(translation[0], translation[1], translation[2]),
    }
}

#[cfg(test)]
#[path = "tests/deck_geometry.rs"]
mod deck_geometry_tests;
#[cfg(test)]
#[path = "tests/mass_moments.rs"]
mod mass_moment_tests;

pub const RETAIL_UNBOUNDED_VELOCITY: f32 = f32::from_bits(0x7F7F_FFFF);
pub const RETAIL_WHEEL_MAXIMUM_ANGULAR_VELOCITY: f32 = f32::from_bits(0x476A_5FFF);

/// Stock wheel properties produced by TU3 `0x82C0AA78 -> 0x82AE7770`.
pub fn retail_wheel_mass_properties() -> RetailBodyMassProperties {
    wheel_mass_properties(WheelMassSettings::STOCK)
}

pub fn wheel_mass_properties(settings: WheelMassSettings) -> RetailBodyMassProperties {
    let forward = forward_mass_properties(wheel_mass_input(settings))
        .expect("the stock wheel is a supported sphere");
    finalize_principal_mass(
        RetailLocalMassFrame::IDENTITY,
        forward.mass,
        forward.principal_moments,
        RETAIL_WHEEL_MAXIMUM_ANGULAR_VELOCITY,
        0.0,
    )
}

/// Stock truck properties produced by TU3 `0x82C0A6F8 -> 0x82AE7770`.
pub fn retail_truck_mass_properties() -> RetailBodyMassProperties {
    truck_mass_properties(TruckMassSettings::STOCK)
}

pub fn truck_mass_properties(settings: TruckMassSettings) -> RetailBodyMassProperties {
    let forward = forward_mass_properties(truck_mass_input(settings))
        .expect("the stock truck is a supported capsule");
    finalize_principal_mass(
        RetailLocalMassFrame::IDENTITY,
        forward.mass,
        forward.principal_moments,
        RETAIL_UNBOUNDED_VELOCITY,
        0.0,
    )
}

/// TU3 deck construction82C09290 -> mass accumulation82AE7058 ->
/// ComputeMassProperties82AE7770. Every authored child contributes to mass.
pub fn deck_mass_properties(
    geometry: &DeckGeometry,
    mass: f32,
    angular_drag: f32,
) -> RetailBodyMassProperties {
    let mut properties = aggregate_mass_properties(
        geometry.mass_moments(),
        mass,
        RETAIL_UNBOUNDED_VELOCITY,
        angular_drag,
    );
    // 82C0A264..278 overrides the deck inverse-body local transform after
    // calculating its principal inertia. Keep that inertia, reset the frame.
    properties.local_mass_frame = RetailLocalMassFrame::IDENTITY;
    properties
}

/// Stock convenience constructor; the game supplies its loaded XML settings.
pub fn retail_deck_mass_properties() -> RetailBodyMassProperties {
    let geometry = DeckGeometry::new(DeckGeometrySettings::STOCK);
    // physicsdeck.DeckMass * physics_world.SkateboardMassFactor,
    // DeckAngularDrag * the refined fixed-step Simulation::frequency.
    deck_mass_properties(
        &geometry,
        6.0,
        f32::from_bits(0x3EE6_6666) * f32::from_bits(0x426F_FFFF),
    )
}

/// Body order used by `SkateboardBody`: wheels 0..3, trucks 4..5, deck 6.
pub fn default_skateboard_mass_properties() -> [RetailBodyMassProperties; 7] {
    [
        retail_wheel_mass_properties(),
        retail_wheel_mass_properties(),
        retail_wheel_mass_properties(),
        retail_wheel_mass_properties(),
        retail_truck_mass_properties(),
        retail_truck_mass_properties(),
        retail_deck_mass_properties(),
    ]
}

/// Final finite path in TU3 `ComputeMassProperties` (`0x82AE7770`). The Xenon
/// reciprocal estimate is followed by two fused Newton refinements for each
/// principal moment. Inverse mass and the spherical energy term use scalar
/// division in the recovered function.
fn finalize_principal_mass(
    local_mass_frame: RetailLocalMassFrame,
    mass: f32,
    principal_moments: Vector3,
    maximum_angular_velocity: f32,
    angular_drag: f32,
) -> RetailBodyMassProperties {
    let inverse_tensor = Vector3::new(
        refined_reciprocal(principal_moments.x),
        refined_reciprocal(principal_moments.y),
        refined_reciprocal(principal_moments.z),
    );
    // S3 82AE7A98/7AAC and S2 82AE9DA0/9DB4 retain the left operand
    // only for ordered LT. Rust min would discard a right-hand NaN.
    let smallest_xy = if inverse_tensor.x < inverse_tensor.y {
        inverse_tensor.x
    } else {
        inverse_tensor.y
    };
    let smallest_inverse = if smallest_xy < inverse_tensor.z {
        smallest_xy
    } else {
        inverse_tensor.z
    };
    RetailBodyMassProperties {
        local_mass_frame,
        dynamics: RetailInertiaDynamics {
            inverse_tensor,
            inverse_mass: 1.0 / mass,
            spherical: 1.0 / smallest_inverse,
            maximum_linear_velocity: RETAIL_UNBOUNDED_VELOCITY,
            maximum_angular_velocity,
            linear_drag: 0.0,
            angular_drag,
        },
    }
}

/// Per-type body values a dynamic world object (DMO) builds its Inertia from
/// (TU3 82C4E568, once per body build): the type record's mass (data +304),
/// velocity caps (+292 linear, +296 angular), drag (+308 / +336) and the box
/// inertia shape (+16 scale and +32 offset on the AABB half extents).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DmoBodyData {
    pub mass: f32,
    pub maximum_linear_velocity: f32,
    pub maximum_angular_velocity: f32,
    pub linear_drag: f32,
    pub angular_drag: f32,
    pub inertia_scale: Vector3,
    pub inertia_offset: Vector3,
}

/// TU3 82C4E568 Inertia fill for a DMO body from its local AABB half extents:
/// inverse mass = 1 / mass (+16), caps and drag copied as is (+24 / +28 /
/// +32 / +36), and the box inertia of 82C47FC8 on
/// `h = half_extents * inertia_scale + inertia_offset` (vmaddfp, fused). When
/// any of h.x, h.y, h.z is not > 0 (NaN included) retail uses (1000, 1000,
/// 1000) instead (0x82256FE8). 82C47FC8: k = (1/3) / inverse mass, inverse
/// tensor = 1 / (k (h.y^2 + h.z^2), k (h.x^2 + h.z^2), k (h.x^2 + h.y^2)) with
/// two Newton steps on the estimate, +20 = 1 / the smallest inverse moment.
pub fn dmo_body_inertia(half_extents: Vector3, data: DmoBodyData) -> RetailInertiaDynamics {
    let inverse_mass = 1.0 / data.mass;
    let h = Vector3::new(
        half_extents.x.madd(data.inertia_scale.x, data.inertia_offset.x),
        half_extents.y.madd(data.inertia_scale.y, data.inertia_offset.y),
        half_extents.z.madd(data.inertia_scale.z, data.inertia_offset.z),
    );
    let h = if h.x > 0.0 && h.y > 0.0 && h.z > 0.0 {
        h
    } else {
        Vector3::new(1000.0, 1000.0, 1000.0)
    };
    // 0x822F87B8 = 1/3 as f32, divided by the inverse mass (fdivs).
    let k = (1.0f32 / 3.0) / inverse_mass;
    let (x2, y2, z2) = (h.x * h.x, h.y * h.y, h.z * h.z);
    let inverse_tensor = Vector3::new(
        refined_reciprocal((y2 + z2) * k),
        refined_reciprocal((x2 + z2) * k),
        refined_reciprocal((x2 + y2) * k),
    );
    let smallest_xy = if inverse_tensor.x < inverse_tensor.y {
        inverse_tensor.x
    } else {
        inverse_tensor.y
    };
    let smallest_inverse = if smallest_xy < inverse_tensor.z {
        smallest_xy
    } else {
        inverse_tensor.z
    };
    RetailInertiaDynamics {
        inverse_tensor,
        inverse_mass,
        spherical: 1.0 / smallest_inverse,
        maximum_linear_velocity: data.maximum_linear_velocity,
        maximum_angular_velocity: data.maximum_angular_velocity,
        linear_drag: data.linear_drag,
        angular_drag: data.angular_drag,
    }
}

fn refined_reciprocal(value: f32) -> f32 {
    let mut estimate = native_arithmetic::reciprocal_estimate(value);
    for _ in 0..2 {
        let residual = (-estimate).madd(value, 1.0);
        estimate = estimate.madd(residual, estimate);
    }
    estimate
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dmo_body_inertia_is_the_scaled_box_and_falls_back_to_1000() {
        let data = DmoBodyData {
            mass: 100.0,
            maximum_linear_velocity: 100.0,
            maximum_angular_velocity: 50.0,
            linear_drag: 0.1,
            angular_drag: 0.35,
            inertia_scale: Vector3::new(1.2, 1.2, 1.2),
            inertia_offset: Vector3::ZERO,
        };
        let i = dmo_body_inertia(Vector3::new(0.5, 1.0, 0.25), data);
        assert_eq!(i.inverse_mass, 0.01);
        assert_eq!((i.maximum_linear_velocity, i.maximum_angular_velocity, i.linear_drag, i.angular_drag), (100.0, 50.0, 0.1, 0.35));
        let (x, y, z) = (0.6f32, 1.2f32, 0.3f32);
        let k = 100.0 / 3.0;
        let moments = [(y * y + z * z) * k, (x * x + z * z) * k, (x * x + y * y) * k];
        for (got, want) in [i.inverse_tensor.x, i.inverse_tensor.y, i.inverse_tensor.z].into_iter().zip(moments) {
            assert!((got * want - 1.0).abs() < 1e-5);
        }
        // +20 is the largest moment (1 / the smallest inverse): z, x^2 + y^2.
        assert!((i.spherical / moments[2] - 1.0).abs() < 1e-5);
        // Any half extent not > 0 after scale + offset: retail's 1000 box.
        let flat = dmo_body_inertia(Vector3::new(0.5, 0.0, 0.25), data);
        let big = (1000.0f32 * 1000.0 * 2.0) * k;
        assert!((flat.inverse_tensor.x * big - 1.0).abs() < 1e-5);
    }

    #[test]
    fn both_mass_finalizers_preserve_nan_and_use_volume_only_below_threshold() {
        // A unit cube has volume one. Both original shape paths converge at
        // 82AE79E8/79EC: fallback only when CR6.LT is set by requested mass.
        let shape = MassShape::RoundedBox {
            half_extents: Vector3::new(0.5, 0.5, 0.5),
            radius: 0.0,
        };
        let moments = MassMoments::from_primitive(primitive_mass(shape).unwrap());
        for (requested_mass, expected_inverse) in [(f32::NAN, f32::NAN), (0.0, 1.0), (2.0, 0.5)] {
            let primitive = primitive_mass_properties(
                PartMassInput {
                    shape,
                    requested_mass,
                },
                RETAIL_UNBOUNDED_VELOCITY,
                0.0,
            )
            .unwrap();
            let aggregate =
                aggregate_mass_properties(moments, requested_mass, RETAIL_UNBOUNDED_VELOCITY, 0.0);
            for body in [primitive, aggregate] {
                if expected_inverse.is_nan() {
                    assert!(body.dynamics.inverse_mass.is_nan());
                } else {
                    assert_eq!(body.dynamics.inverse_mass, expected_inverse);
                }
            }
        }
    }

    #[test]
    fn stock_simple_parts_reach_recovered_finalizer_words() {
        let wheel = retail_wheel_mass_properties().dynamics;
        assert_eq!(
            [
                wheel.inverse_tensor.x.to_bits(),
                wheel.inverse_tensor.y.to_bits(),
                wheel.inverse_tensor.z.to_bits(),
                wheel.inverse_mass.to_bits(),
                wheel.spherical.to_bits(),
            ],
            [
                0x45C3_E491,
                0x45C3_E491,
                0x45C3_E491,
                0x401A_3785,
                0x3927_466F,
            ]
        );

        let truck = retail_truck_mass_properties().dynamics;
        assert_eq!(
            [
                truck.inverse_tensor.x.to_bits(),
                truck.inverse_tensor.y.to_bits(),
                truck.inverse_tensor.z.to_bits(),
                truck.inverse_mass.to_bits(),
                truck.spherical.to_bits(),
            ],
            [
                0x438E_E68E,
                0x438E_E68E,
                0x4690_4D8B,
                0x3F03_4835,
                0x3B65_4E66,
            ]
        );
    }

    #[test]
    fn deck_frame_override_retains_geometry_driven_inertia() {
        let stock = DeckGeometry::new(DeckGeometrySettings::STOCK);
        let deck = deck_mass_properties(&stock, 6.0, 27.0);
        assert_eq!(deck.local_mass_frame, RetailLocalMassFrame::IDENTITY);
        assert_eq!(deck.dynamics.inverse_mass, 1.0 / 6.0);
        let doubled_mass = deck_mass_properties(&stock, 12.0, 27.0);
        for (one, two) in [
            (
                deck.dynamics.inverse_tensor.x,
                doubled_mass.dynamics.inverse_tensor.x,
            ),
            (
                deck.dynamics.inverse_tensor.y,
                doubled_mass.dynamics.inverse_tensor.y,
            ),
            (
                deck.dynamics.inverse_tensor.z,
                doubled_mass.dynamics.inverse_tensor.z,
            ),
        ] {
            assert!((one - 2.0 * two).abs() < 1e-5);
        }
        let mut wider = DeckGeometrySettings::STOCK;
        wider.width *= 1.5;
        let changed = deck_mass_properties(&DeckGeometry::new(wider), 6.0, 27.0);
        assert_ne!(
            deck.dynamics.inverse_tensor,
            changed.dynamics.inverse_tensor
        );
    }
}
