//! skate_vita: the traffic-car tests of upstream's `physics/skater_context_tests.rs` (PR #52
//! `618e824`) and the rig they use, copied verbatim (`skate_dynamics::` -> `crate::dynamics::`).
//! The rest of that module needs Bevy, `skate_net` or the peds and stays out of the build.
use super::*;
use skate_core::physics::board::BodyId;
use skate_core::player::state::PhysicalStateId;
// skate_vita: upstream's `use super::*` brings Bevy's prelude (Quat, Transform).
use crate::transform::Transform;
use glam::Quat;

/// skate_vita: a hash of the deck's position and velocity bits over every tick, so a performance
/// change in the solid contacts can be checked bit-identical (printed by the tests).
fn fold(hash: &mut u64, physics: &GamePhysics) {
    let deck = &physics.board.bodies()[BodyId::Deck.index()].rates;
    for v in [deck.position, deck.linear_velocity] {
        for x in [v.x, v.y, v.z] {
            *hash = (*hash ^ x.to_bits() as u64).wrapping_mul(0x0100_0000_01b3);
        }
    }
}

fn pad() -> skate_core::input::xbox::XboxState {
    skate_core::input::xbox::XboxState::default()
}

struct Rig {
    graphs: crate::graph_runtime::StockGraphs,
    root_dir: std::path::PathBuf,
}

struct Skater {
    runtime: SkaterRuntime,
    camera: crate::camera::CameraRuntime,
    controls: PlayerControls,
    input: crate::input::ControllerInput,
}

impl Rig {
    fn skater(&self, physics: &GamePhysics) -> Skater {
        Skater {
            runtime: SkaterRuntime::load(&self.root_dir, &self.graphs, physics, "easy").unwrap(),
            camera: crate::camera::CameraRuntime::load(&self.root_dir).unwrap(),
            controls: PlayerControls::default(),
            input: crate::input::ControllerInput::default(),
        }
    }
    fn step(&self, physics: &mut GamePhysics, s: &mut Skater, state: skate_core::input::xbox::XboxState) {
        s.input.sample_raw_for_test(state);
        let mut actions = s.input.player_actions();
        s.controls.update_for_physics(&mut actions, physics, &mut s.runtime, &mut s.camera).unwrap();
        frame::advance(physics, &mut s.runtime, &mut s.controls, &self.graphs, &mut actions, true, &mut s.camera).unwrap();
    }
}

/// Car hit (`sub_82BD4A30` / `sub_82D90C98`, `Wipeout_GroundVehicleContact`): a relative normal speed
/// against a vehicle-group body above `Wipeout_GroundVehicleContact` (9.0) wipes the skater
/// out. A traffic car (kinematic box in the vehicle group, as `living_world::vehicles::proxy`)
/// driving into the standing skater at 12 m/s knocks them down at once.
#[test]
#[ignore = "requires private stock graphs and an installed map (SKATE3_ASSET_ROOT, SKATE3_MAP=<maps/DownTown.skate>)"]
fn a_traffic_car_knocks_the_skater_down_above_the_contact_limit() {
    use crate::dynamics::rapier3d::prelude::{Pose, Rotation, SharedShape, Vector};
    let root_dir = std::path::PathBuf::from(std::env::var_os("SKATE3_ASSET_ROOT").unwrap());
    let map_path = std::path::PathBuf::from(std::env::var_os("SKATE3_MAP").unwrap());
    let map = skate_data::skate_map::SkateMap::load(&map_path).unwrap();
    let assets = skate_data::GameAssets::load(&root_dir).unwrap();
    let rig = Rig { graphs: crate::graph_runtime::StockGraphs::load(&root_dir, &assets).unwrap(), root_dir: root_dir.clone() };
    let run = |speed: f32, group: u32| {
        let mut physics = GamePhysics::load_with_difficulty(&root_dir, Some(&map), crate::difficulty::Difficulty::Easy).unwrap();
        let mut player = rig.skater(&physics);
        for _ in 0..30 {
            rig.step(&mut physics, &mut player, pad());
        }
        let deck = physics.board.bodies()[BodyId::Deck.index()].rates.position;
        // The car starts 4 m to the skater's +x and drives at them along -x.
        let mut x = deck.x + 4.0;
        let mut states = Vec::new();
        let mut hash = 0xcbf2_9ce4_8422_2325u64;
        for _ in 0..60 {
            x -= speed / 60.0;
            let centre = Vector::new(x, deck.y + 0.75, deck.z);
            let solid = crate::dynamics::SolidBody {
                id: 0x7E57_0000_0000_0001,
                pose: Pose::from_parts(centre, Rotation::IDENTITY),
                center_of_mass: centre,
                inertia_rotation: Rotation::IDENTITY,
                inverse_mass: 0.0,
                inverse_inertia: Vector::new(0.0, 0.0, 0.0),
                linvel: Vector::new(-speed, 0.0, 0.0),
                angvel: Vector::new(0.0, 0.0, 0.0),
                contact_group: group,
                colliders: vec![crate::dynamics::SolidCollider { shape: SharedShape::cuboid(2.15, 0.75, 0.9), pose: Pose::from_parts(centre, Rotation::IDENTITY), friction: 0.5 }],
            };
            let mut proxies = network::Proxies::default();
            proxies.append_solid(solid, &physics, &player.runtime, false);
            physics.network_proxies = proxies;
            rig.step(&mut physics, &mut player, pad());
            fold(&mut hash, &physics);
            states.push(player.runtime.player_state.current());
        }
        eprintln!("CAR_KNOCK_HASH {speed} {group} {hash:016x}");
        states
    };
    let first = |v: &[PhysicalStateId]| v.iter().position(|s| *s == PhysicalStateId::WipeoutGround);
    // 2026-10-09 (DownTown): 12 m/s group 8 -> tick 6, group 0 -> tick 7; 4 m/s group 8 -> tick 22 (this
    // box never brakes, unlike a retail car that hit an actor; the vehicle scalar 0.35 shrinks the other
    // limits while touching it), group 0 -> none.
    let fast = first(&run(12.0, VEHICLE_GROUP));
    eprintln!("12 m/s: first wipeout tick {fast:?}");
    assert!(fast.is_some_and(|t| t < 10), "a 12 m/s car hit wipes the skater out at once");
}

/// Skitching end to end (2026-10-16: the first live skitch failed in PreState, then would have in FillPhysOut): the
/// skater rolls, a taxi with its exported rear grab spline drives just ahead at the skater's speed (grab scene +
/// solid proxy, as `living_world::vehicles` feeds them), grab is held: the skater must latch (Skitching), be towed
/// for seconds through every frame stage without an error, and leave Skitching after letting go.
#[test]
#[ignore = "requires private stock graphs, the living-world export and an installed map (SKATE3_ASSET_ROOT, SKATE3_MAP=<maps/DownTown.skate>)"]
fn a_skater_latches_onto_a_traffic_car_rides_and_lets_go() {
    // skate_vita: upstream installs a log subscriber here with SKATE_TEST_LOG (no subscriber crate in skate-sim).
    use crate::dynamics::rapier3d::prelude::{Pose, Rotation, Vector};
    let root_dir = std::path::PathBuf::from(std::env::var_os("SKATE3_ASSET_ROOT").unwrap());
    let map_path = std::path::PathBuf::from(std::env::var_os("SKATE3_MAP").unwrap());
    let map = skate_data::skate_map::SkateMap::load(&map_path).unwrap();
    let assets = skate_data::GameAssets::load(&root_dir).unwrap();
    let dir = root_dir.join("private/living_world");
    let data = crate::living_world::vehicles::parse_vehicle_data(&std::fs::read(dir.join("vehicles.json")).unwrap(), std::fs::read(dir.join("tables.json")).ok().as_deref()).unwrap();
    let splines = data.models.get("vehicle_taxi01").map(|m| m.grab_splines.clone()).filter(|s| !s.is_empty()).expect("taxi grab spline");
    // The taxi's retail chassis mesh (model space), as the traffic proxies collide with it.
    let mesh = data.models.get("vehicle_taxi01").and_then(|m| m.collision.as_ref()).map(|c| c.shape.clone()).expect("taxi collision mesh");
    let rear_z = splines.iter().flat_map(|s| s.points.iter().map(|p| p[2])).fold(f32::INFINITY, f32::min);
    let rig = Rig { graphs: crate::graph_runtime::StockGraphs::load(&root_dir, &assets).unwrap(), root_dir: root_dir.clone() };
    let mut physics = GamePhysics::load_with_difficulty(&root_dir, Some(&map), crate::difficulty::Difficulty::Easy).unwrap();
    crate::physics::set_skitch_enabled(true);
    let mut player = rig.skater(&physics);
    // Push off and roll.
    for t in 0..150 {
        rig.step(&mut physics, &mut player, skate_core::input::xbox::XboxState { buttons: if t < 90 { 0x1000 } else { 0 }, ..pad() });
    }
    let deck = physics.board.bodies()[BodyId::Deck.index()].rates;
    let v = deck.linear_velocity;
    let speed = (v.x * v.x + v.z * v.z).sqrt();
    assert!(speed > 1.0, "the skater rolls ({speed:.2} m/s)");
    let heading = Vec3::new(v.x / speed, 0.0, v.z / speed);
    let car_speed = speed.max(4.0);
    let rotation = Quat::from_rotation_y(heading.x.atan2(heading.z));
    // The car's rear grab spline 0.8 m ahead of the deck.
    let mut origin = Vec3::new(deck.position.x, deck.position.y - 0.1, deck.position.z) + heading * (0.8 - rear_z);
    let grab = skate_core::input::xbox::XboxState { buttons: 0x0200, ..pad() }; // RB, retail GrabWorld
    let mut states = Vec::new();
    let (mut crossed, mut gripping, mut arm_contacts) = (false, 0, 0usize);
    let (mut hash, mut step_time) = (0xcbf2_9ce4_8422_2325u64, std::time::Duration::ZERO);
    for t in 0..420 {
        origin += heading * (car_speed / 60.0);
        let transform = Transform::from_translation(origin).with_rotation(rotation);
        let object = crate::living_world::vehicles::grab_object_at(5, &splines, &transform, heading * car_speed);
        physics.set_grab_cars(vec![object]).unwrap();
        let centre = Vector::new(origin.x, origin.y + 0.75, origin.z);
        let q = Rotation::from_rotation_y(heading.x.atan2(heading.z));
        let solid = crate::dynamics::SolidBody {
            id: 0x7E57_0000_0000_0005,
            pose: Pose::from_parts(centre, q),
            center_of_mass: centre,
            inertia_rotation: Rotation::IDENTITY,
            inverse_mass: 0.0,
            inverse_inertia: Vector::new(0.0, 0.0, 0.0),
            linvel: Vector::new(heading.x * car_speed, 0.0, heading.z * car_speed),
            angvel: Vector::new(0.0, 0.0, 0.0),
            contact_group: VEHICLE_GROUP,
            colliders: vec![crate::dynamics::SolidCollider { shape: mesh.clone(), pose: Pose::from_parts(Vector::new(origin.x, origin.y, origin.z), q), friction: 0.5 }],
        };
        let mut proxies = network::Proxies::default();
        proxies.append_solid(solid, &physics, &player.runtime, false);
        physics.network_proxies = proxies;
        // Grab for 5 s, then let go.
        let started = std::time::Instant::now();
        rig.step(&mut physics, &mut player, if t < 300 { grab } else { pad() });
        step_time += started.elapsed();
        fold(&mut hash, &physics);
        states.push(player.runtime.player_state.current());
        if player.runtime.player_state.current() == PhysicalStateId::Skitching {
            // Each gripping hand reaches to its own side of the bumper (retail re-orients the held record to the
            // board side in 82D48148; without it the hands crossed over).
            let k = &player.runtime.skitch_state;
            let d = physics.board.bodies()[BodyId::Deck.index()].rates;
            let deck = Vec3::new(d.position.x, d.position.y, d.position.z);
            let side = rotation * Vec3::X;
            let m = player.runtime.animated_skeleton.roots.animation_to_world;
            let world = |i: usize| {
                let p = player.runtime.animated_skeleton.raw_part_globals[i][3];
                Vec3::new(m[0][0] * p[0] + m[1][0] * p[1] + m[2][0] * p[2] + m[3][0], m[0][1] * p[0] + m[1][1] * p[1] + m[2][1] * p[2] + m[3][1], m[0][2] * p[0] + m[1][2] * p[1] + m[2][2] * p[2] + m[3][2])
            };
            for (hand, (shoulder, limb)) in [(5, 2), (9, 3)].into_iter().enumerate() {
                let w = player.runtime.foot_ik.state.external_targets[limb].world_position;
                let (a, b) = ((world(shoulder) - deck).dot(side), (Vec3::new(w[0], w[1], w[2]) - deck).dot(side));
                if k.hands.weight[hand] > 0.0 && a.abs() > 0.05 && b.abs() > 0.05 {
                    crossed |= a.signum() != b.signum();
                }
            }
            // No upper-arm contact through the tow: the bounds box put its rear wall between the hands and the
            // shoulders (arm hits for the whole tow, session 2026-10-16).
            let fb = &player.runtime.collision_feedback;
            // Ticks 10..70: towed in the open (the box car drags the skater into a building from about tick 75).
            if (10..70).contains(&t) {
                arm_contacts += usize::from(fb.regions[2].part.is_some() || fb.regions[3].part.is_some());
            }
            if k.sub.mode == 1 && k.hands.off == [false; 2] {
                gripping += 1;
            }
        }
        if std::env::var_os("SKATE_TEST_LOG").is_some() && player.runtime.player_state.current() == PhysicalStateId::Skitching {
            let k = &player.runtime.skitch_state;
            let d = physics.board.bodies()[BodyId::Deck.index()].rates;
            let sp = (d.linear_velocity.x.powi(2) + d.linear_velocity.z.powi(2)).sqrt();
            let rear = origin + rotation * Vec3::new(0.0, 0.0, rear_z);
            let gap = Vec3::new(rear.x - d.position.x, 0.0, rear.z - d.position.z).length();
            eprintln!("SKITCH_TICK t={t} mode={} ready={} deck_speed={sp:.2} car={car_speed:.2} gap={gap:.2} spring={:?} hands_off={:?} reach={:?} dy={:.2} hand_target={:.2} posed={:.2} lean_yaw={:.2}", k.sub.mode, k.ready, k.spring_force, k.hands.off, k.hands.reach, d.position.y - origin.y, k.shimmy.target, k.hold.posed, k.lean_yaw);
            // Hand sides across the car (+ = car right): shoulder (parts 5 / 9) vs the hand IK target (limbs 2 / 3).
            let side = rotation * Vec3::X;
            let m = player.runtime.animated_skeleton.roots.animation_to_world;
            let world = |i: usize| {
                let p = player.runtime.animated_skeleton.raw_part_globals[i][3];
                Vec3::new(m[0][0] * p[0] + m[1][0] * p[1] + m[2][0] * p[2] + m[3][0], m[0][1] * p[0] + m[1][1] * p[1] + m[2][1] * p[2] + m[3][1], m[0][2] * p[0] + m[1][2] * p[1] + m[2][2] * p[2] + m[3][2])
            };
            let tgt = |l: usize| {
                let w = player.runtime.foot_ik.state.external_targets[l].world_position;
                Vec3::new(w[0], w[1], w[2])
            };
            let pr = &player.runtime.player_input.processed;
            let w = pr.grab_records_1888_2176[0];
            let dir = Vec3::new(f32::from_bits(w[28]), f32::from_bits(w[29]), f32::from_bits(w[30]));
            let row0 = Vec3::new(m[0][0], m[0][1], m[0][2]);
            eprintln!("SKITCH_AXES t={t} mirrored={} hand_x=[{:.2},{:.2}] dir_side={:.2} row0_side={:.2} dir_row0={:.2} row2_fwd={:.2}", pr.flags_2476 & 4 != 0, player.runtime.animated_skeleton.raw_part_globals[3][3][0], player.runtime.animated_skeleton.raw_part_globals[7][3][0], dir.dot(side), row0.dot(side), dir.dot(row0), Vec3::new(m[2][0], m[2][1], m[2][2]).dot(heading));
            eprintln!("SKITCH_HANDS t={t} shoulder_side=[{:.2},{:.2}] target_side=[{:.2},{:.2}] weight={:?}", (world(5) - Vec3::new(d.position.x, d.position.y, d.position.z)).dot(side), (world(9) - Vec3::new(d.position.x, d.position.y, d.position.z)).dot(side), (tgt(2) - Vec3::new(d.position.x, d.position.y, d.position.z)).dot(side), (tgt(3) - Vec3::new(d.position.x, d.position.y, d.position.z)).dot(side), k.hands.weight);
        }
    }
    let skitching = states.iter().filter(|s| **s == PhysicalStateId::Skitching).count();
    let first = states.iter().position(|s| *s == PhysicalStateId::Skitching);
    eprintln!("skitch: first latch tick {first:?}, {skitching} ticks skitching, last state {:?}", states.last());
    eprintln!("SKITCH_HASH {hash:016x} steps {:.1} ms", step_time.as_secs_f32() * 1000.0);
    let mut runs: Vec<(PhysicalStateId, usize)> = Vec::new();
    for st in &states {
        match runs.last_mut() {
            Some((k, n)) if k == st => *n += 1,
            _ => runs.push((*st, 1)),
        }
    }
    eprintln!("skitch states: {runs:?}");
    assert!(first.is_some_and(|t| t < 120), "the skater latches within 2 s of grabbing behind the car");
    assert!(skitching >= 60, "towed for at least a second ({skitching} ticks)");
    assert!(!crossed, "each hand grips its own side of the bumper");
    assert_eq!(arm_contacts, 0, "the arms do not ride against the held car's chassis");
    assert!(gripping >= 60, "sub-mode 1 keeps both hands on for at least a second ({gripping} ticks)");
    assert!(states[340..].iter().all(|s| *s != PhysicalStateId::Skitching), "letting go leaves Skitching");
}
