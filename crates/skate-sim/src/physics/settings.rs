//! Stock skater XML values bound to the recovered physical calculations.
use skate_core::{
    math::Vector3,
    physics::{
        board_step::BoardStepSettings,
        contact::RetailContactMaterial,
        drive_frames::{
            AuthoredTransformInputs, RetailAffineTransform, RetailTruckTransformInputs,
            authored_body_transforms, calculate_truck_transforms,
        },
        drive_parameters::{RetailTruckDriveSettings, retail_truck_drive_dynamics},
        mass::{
            DeckGeometry, DeckGeometrySettings, MassShape, TruckMassSettings, WheelMassSettings,
            deck_mass_properties, truck_mass_input, truck_mass_properties, wheel_mass_properties,
        },
        rigid_body::{RetailBodyMassProperties, RetailSimulationStep},
    },
};
use skate_data::collections::Collections;

/// Constraint iterations of the skater's `rw::physics::Simulation` (+176), read by the solver
/// stages `82AE30F0` / `82AE27D0` as their loop count. Measured in TU3 (recomp, Aletown,
/// board + skater pipeline with 28 joints / 53 drives): 50 on every solve. Setup `82DC2840`
/// first copies 25 from the hard-coded config built by `8275DCC8` (`li r27,25`); the value is
/// 50 before the first gameplay solve (the later writer is not identified yet). The stock
/// `physics/default.RWMaxIterations` is 25, equal to the setup value but not the live one;
/// solving with 25 left wheels sinking ~10x faster than retail (-0.012 vs -0.001 m/s at rest)
/// and the ollie pop's deck ~1 cm low by its 4th frame (retail-matching with 50).
pub(crate) const SIMULATION_ITERATIONS: u32 = 50;

#[derive(Clone)]
pub(crate) struct PhysicsSettings {
    pub step: BoardStepSettings,
    pub masses: [RetailBodyMassProperties; 7],
    pub authored: [RetailAffineTransform; 7],
    pub deck_geometry: DeckGeometry,
    pub truck_shape: MassShape,
    pub truck_collisions: bool,
    pub truck_material: RetailContactMaterial,
    pub deck_material: RetailContactMaterial,
    pub wheel_radius: f32,
    pub wheel_material: RetailContactMaterial,
    pub standard_wheel_material: RetailContactMaterial,
    pub floor_material: RetailContactMaterial,
    pub input_magnitude_threshold: f32,
    /// Object-move (Move Object mode) stick curves, inputlistener 8259C4B0.
    pub object_move: skate_core::input::offboard_intentions::ObjectMoveCurves,
    /// Move Object command tuning (class 3EDA5B140604613D); the stock
    /// fallback only when the collection lacks it.
    pub move_object: skate_core::player::offboard::move_object::MoveObjectTuning,
    /// `physics_ai` `default`: the AI board path gains (`82C05EC0`).
    pub physics_ai: skate_core::riding::grounded::state::board_path::PhysicsAiTuning,
    /// `scoring_wipeouts` `default`: the Hall of Meat bones and impact scale; None leaves the injury stats unfed.
    pub hall_of_meat: Option<skate_core::hall_of_meat::Tuning>,
    /// The same record's BodyTweak poses and gesture dead zone; None leaves the pose stats unfed.
    pub hall_of_meat_poses: Option<skate_core::hall_of_meat::PoseTable>,
}

impl PhysicsSettings {
    pub fn load(data: &Collections) -> Result<Self, String> {
        let f = |class, field| data.float(class, "default", field);
        let b = |class, field| data.boolean(class, "default", field);
        let mass_factor = f("physics_world", "SkateboardMassFactor")?;
        let wheel_radius = f("physicswheels", "WheelRadius")?;
        let wheel_x_distance = f("physicswheels", "WheelXDist")?;
        let geometry = AuthoredTransformInputs {
            deck_mid_length: f("physicsdeck", "DeckMidLength")?,
            wheel_x_distance,
            truck_z_position_front: f("physicstrucks", "TruckZPosFront")?,
            truck_z_position_back: f("physicstrucks", "TruckZPosBack")?,
            truck_y_position: f("physicstrucks", "TruckYPos")?,
        };
        let wheel = wheel_mass_properties(WheelMassSettings {
            radius: wheel_radius,
            mass: f("physicswheels", "WheelMass")?,
            mass_factor,
        });
        let truck_settings = TruckMassSettings {
            wheel_radius,
            wheel_x_distance,
            mass_factor,
            radius_scalar: f("physicstrucks", "RadiusScalar")?,
            half_height_scalar: f("physicstrucks", "HalfHeightScalar")?,
            mass: f("physicstrucks", "TruckMass")?,
        };
        let truck = truck_mass_properties(truck_settings);
        let gravity = data
            .words::<4>("physics", "default", "WorldGravity")?
            .map(f32::from_bits);
        let simulation = RetailSimulationStep::fixed_60_hz(
            0, // Our host keeps its single board active; sleeping is not scheduled.
            f("physics", "FreezingEnergy")?,
            Vector3::new(gravity[0], gravity[1], gravity[2]),
        );
        let deck_geometry = DeckGeometry::new(DeckGeometrySettings {
            width: f("physicsdeck", "DeckWidth")?,
            mid_length: geometry.deck_mid_length,
            thickness: f("physicsdeck", "DeckThickness")?,
            back_end_size: f("physicsdeck", "DeckBackEndSize")?,
            front_end_angle_degrees: f("physicsdeck", "DeckFrontEndAngle")?,
            back_end_angle_degrees: f("physicsdeck", "DeckBackEndAngle")?,
            end_capsule_count: data.integer("physicsdeck", "default", "DeckEndCapsules")? as i32,
            enable_deck_volume_collisions: b("physicsdeck", "DeckEnableDeckVolumeCollisions")?,
            enable_end_volume_collisions: b("physicsdeck", "DeckEnableEndVolumeCollisions")?,
        });
        let deck = deck_mass_properties(
            &deck_geometry,
            f("physicsdeck", "DeckMass")? * mass_factor,
            f("physicsdeck", "DeckAngularDrag")? * simulation.frequency,
        );
        let step = BoardStepSettings {
            simulation,
            iterations: SIMULATION_ITERATIONS,
            base_truck_transforms: calculate_truck_transforms(RetailTruckTransformInputs {
                deck_mid_length: geometry.deck_mid_length,
                truck_z_position_front: geometry.truck_z_position_front,
                truck_z_position_back: geometry.truck_z_position_back,
                truck_y_position: geometry.truck_y_position,
                truck_rotation_axis_angle_degrees: f("physicstrucks", "TruckRotationAxisAngle")?,
            }),
            truck_dynamics: retail_truck_drive_dynamics(RetailTruckDriveSettings {
                use_linear_drives: b("physicstrucks", "UseTruckDrives")?,
                use_hard_linear_drives: b("physicsdeck", "UseHardDrives")?,
                angular_displacement: f("physicstrucks_drives", "Angular_Hard_Displacement")?,
                angular_damping: f("physicstrucks_drives", "Angular_Hard_Damping")?,
                angular_strength: f("physicstrucks_drives", "Angular_Hard_Strength")?,
            }),
            force_point_y_offset: f("physicsdeck", "DeckForceYOffset")?,
        };
        if wheel_radius <= 0.0 || mass_factor <= 0.0 {
            return Err("Invalid stock board radius or mass factor".into());
        }
        Ok(Self {
            step,
            masses: [wheel, wheel, wheel, wheel, truck, truck, deck],
            authored: authored_body_transforms(geometry),
            deck_geometry,
            truck_shape: truck_mass_input(truck_settings).shape,
            truck_collisions: b("physicstrucks", "TruckEnableVolumeCollisions")?,
            truck_material: RetailContactMaterial {
                static_friction: f("physicstrucks", "TruckStaticFriction")?,
                dynamic_friction: f("physicstrucks", "TruckDynamicFriction")?,
                restitution: f("physicstrucks", "TruckRestitution")?,
            },
            deck_material: RetailContactMaterial {
                static_friction: f("physicsdeck", "DeckStaticFriction")?,
                dynamic_friction: f("physicsdeck", "DeckDynamicFriction")?,
                restitution: f("physicsdeck", "DeckRestitution")?,
            },
            wheel_radius,
            wheel_material: RetailContactMaterial {
                static_friction: f("physicswheels", "WheelStaticFriction")?,
                dynamic_friction: f("physicswheels", "WheelDynamicFriction")?,
                restitution: f("physicswheels", "WheelRestitution")?,
            },
            standard_wheel_material: RetailContactMaterial {
                static_friction: f("physicswheels", "WheelStaticFriction")?,
                dynamic_friction: f("physicswheels", "WheelDynamicFriction")?,
                restitution: f("physicswheels", "WheelRestitution")?,
            },
            floor_material: RetailContactMaterial {
                // Ground job8277C5D8 uses context83034F34/38/3C. The
                // max/max/min combine preserves the moving volume material.
                static_friction: 0.0,
                dynamic_friction: 0.0,
                restitution: 1.0,
            },
            input_magnitude_threshold: f("inputlistener", "StickMagnitudeMinToCountHeld")?,
            object_move: super::offboard::settings::load_object_move_curves(data)?,
            move_object: super::offboard::settings::load_move_object_tuning(data).unwrap_or_else(|error| {
                tracing::warn!("SKATE_MOVE_OBJECT: stock tuning missing ({error}); using built-in stock values");
                Default::default()
            }),
            physics_ai: super::board_path::load_physics_ai(data).unwrap_or_else(|error| {
                tracing::warn!("SKATE_PHYSICS_AI: stock physics_ai missing ({error}); using the default record's values");
                Default::default()
            }),
            hall_of_meat: load_hall_of_meat(data).map_err(|error| tracing::warn!("HALL_OF_MEAT: no scoring_wipeouts tuning ({error})")).ok(),
            hall_of_meat_poses: load_hall_of_meat_poses(data).map_err(|error| tracing::warn!("HALL_OF_MEAT: no BodyTweak poses ({error})")).ok(),
        })
    }
}

/// `scoring_wipeouts/default` (class 9E4C471BC6EAB615): BoneSet at layout +0 and the impact scale at +0xBFC
/// (`Hash_B7514DE7EBDE9D01`; b594 read the layout offsets from the schema).
pub(crate) fn load_hall_of_meat(data: &Collections) -> Result<skate_core::hall_of_meat::Tuning, String> {
    const CLASS: &str = "Hash_9E4C471BC6EAB615";
    let words = data.words::<525>(CLASS, "default", "Hash_1F9B5EA147CBEEE9")?;
    let scale = data.float(CLASS, "default", "Hash_B7514DE7EBDE9D01")?;
    skate_core::hall_of_meat::Tuning::from_words(&words, scale).ok_or_else(|| "BoneSet size".to_owned())
}

/// `scoring_wipeouts/default` BodyTweak elements (`Hash_5717CFCCCA59BD47`, 24 bytes each) and the gesture dead zone
/// (`Hash_9C494E9614FA5ED6`, read at run time by 0x82DAD728; 0 when missing, as retail).
pub(crate) fn load_hall_of_meat_poses(data: &Collections) -> Result<skate_core::hall_of_meat::PoseTable, String> {
    const CLASS: &str = "Hash_9E4C471BC6EAB615";
    let field = data.field(CLASS, "default", "Hash_5717CFCCCA59BD47")?;
    let items = field.array.as_ref().ok_or("BodyTweak is not an array")?.items.iter().map(|hex| {
        let words: Vec<u32> = (0..6).filter_map(|i| hex.get(i * 8..i * 8 + 8).and_then(|w| u32::from_str_radix(w, 16).ok())).collect();
        <[u32; 6]>::try_from(words).map_err(|_| format!("BodyTweak element {hex}"))
    }).collect::<Result<Vec<_>, _>>()?;
    let dead_zone = data.float(CLASS, "default", "Hash_9C494E9614FA5ED6").unwrap_or(0.0);
    Ok(skate_core::hall_of_meat::PoseTable::from_items(&items, dead_zone))
}

#[cfg(test)]
mod hall_of_meat_tests {
    /// Data-gated: the stock BoneSet has 25 bones (bone 0 links to 23 and 1) and the impact scale is 0.75.
    #[test]
    fn stock_hall_of_meat_tuning_loads() {
        let Some(root) = std::env::var_os("SKATE3_ASSET_ROOT").map(std::path::PathBuf::from) else {
            eprintln!("skipped: set SKATE3_ASSET_ROOT to the converted assets");
            return;
        };
        let data = super::Collections::load(&root).expect("stock collections");
        let t = super::load_hall_of_meat(&data).expect("scoring_wipeouts");
        assert_eq!(t.impact_scale, 0.75);
        assert_eq!(t.bones[0].links[3], Some((1, 0.5)));
        assert_eq!(t.bones[0].levels[1], (1000, 14.0));
        let p = super::load_hall_of_meat_poses(&data).expect("BodyTweak");
        assert_eq!((p.poses.len(), p.dead_zone), (4, f32::from_bits(0x3F19_999A)));
        assert_eq!((p.poses[1].pose, p.poses[1].direction, p.poses[1].frames, p.poses[1].points), (2, [1.0, 0.0], 15, 500));
    }
}
