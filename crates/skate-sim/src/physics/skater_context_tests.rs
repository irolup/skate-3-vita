//! Simulated NPC skaters share `GamePhysics` through the per-skater context swap: the local
//! player's simulation is bit-identical with or without an NPC skater ticking in between.
use super::*;
use skate_core::physics::board::BodyId;
use skate_core::player::state::PhysicalStateId;

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

/// Deck position and velocity bits per tick.
fn trace(physics: &GamePhysics) -> [u32; 6] {
    let deck = &physics.board.bodies()[BodyId::Deck.index()].rates;
    let (p, v) = (deck.position, deck.linear_velocity);
    [p.x.to_bits(), p.y.to_bits(), p.z.to_bits(), v.x.to_bits(), v.y.to_bits(), v.z.to_bits()]
}

#[test]
#[ignore = "requires private stock graphs and an installed map (SKATE3_ASSET_ROOT, SKATE3_MAP=<maps/DownTown.skate>)"]
fn the_player_is_bit_identical_with_a_simulated_npc_skater_in_the_same_world() {
    let root_dir = std::path::PathBuf::from(std::env::var_os("SKATE3_ASSET_ROOT").unwrap());
    let map_path = std::path::PathBuf::from(std::env::var_os("SKATE3_MAP").unwrap());
    let map = skate_data::skate_map::SkateMap::load(&map_path).unwrap();
    let assets = skate_data::GameAssets::load(&root_dir).unwrap();
    let rig = Rig { graphs: crate::graph_runtime::StockGraphs::load(&root_dir, &assets).unwrap(), root_dir: root_dir.clone() };
    let load = || GamePhysics::load_with_difficulty(&root_dir, Some(&map), crate::difficulty::Difficulty::Easy).unwrap();
    // Push forward (A held) for a second, then roll.
    let input = |t: u32| skate_core::input::xbox::XboxState { buttons: if t < 60 { 0x1000 } else { 0 }, ..pad() };

    let mut alone = load();
    let mut player = rig.skater(&alone);
    let mut expected = Vec::new();
    for t in 0..300 {
        rig.step(&mut alone, &mut player, input(t));
        expected.push(trace(&alone));
    }

    let mut shared = load();
    let mut player = rig.skater(&shared);
    // The NPC skater: its own board 6 m to the player's side, its own runtime.
    let mut spawn = shared.board.part_transforms()[BodyId::Deck.index()];
    spawn.translation.x += 6.0;
    let mut context = shared.new_skater_context(spawn).unwrap();
    shared.swap_skater_context(&mut context);
    let mut npc = rig.skater(&shared);
    shared.swap_skater_context(&mut context);
    let start = context.board.part_transforms()[BodyId::Deck.index()].translation;
    for t in 0..300 {
        rig.step(&mut shared, &mut player, input(t));
        assert_eq!(trace(&shared), expected[t as usize], "player diverged at tick {t}");
        shared.swap_skater_context(&mut context);
        rig.step(&mut shared, &mut npc, input(t));
        shared.swap_skater_context(&mut context);
    }
    // The NPC skater simulated on its own board: it moved and did not fall through the world.
    let end = context.board.part_transforms()[BodyId::Deck.index()].translation;
    let moved = ((end.x - start.x).powi(2) + (end.z - start.z).powi(2)).sqrt();
    assert!(moved > 1.0 && (end.y - start.y).abs() < 2.0, "npc board {start:?} -> {end:?}");
    assert!(!context.failed && context.owns_props == false && shared.owns_props);
    assert_eq!(context.ticks, 300);
}

/// Steps a + b of the simulated tier: a skater with only the AI record (no pad) rides a recorded
/// DownTown NPC line on its own physics, steered by the ground states' board path.
#[test]
#[ignore = "requires private stock graphs, the living-world export and an installed map (SKATE3_ASSET_ROOT, SKATE3_MAP=<maps/DownTown.skate>)"]
fn a_simulated_skater_rides_a_recorded_line_from_its_ai_record() {
    use skate_core::living_world::replay::{path_frame, Decider, LineCursor, ReplayLine};
    let root_dir = std::path::PathBuf::from(std::env::var_os("SKATE3_ASSET_ROOT").unwrap());
    let map_path = std::path::PathBuf::from(std::env::var_os("SKATE3_MAP").unwrap());
    let map = skate_data::skate_map::SkateMap::load(&map_path).unwrap();
    let assets = skate_data::GameAssets::load(&root_dir).unwrap();
    let rig = Rig { graphs: crate::graph_runtime::StockGraphs::load(&root_dir, &assets).unwrap(), root_dir: root_dir.clone() };
    let mut physics = GamePhysics::load_with_difficulty(&root_dir, Some(&map), crate::difficulty::Difficulty::Easy).unwrap();
    let pack = std::fs::read(root_dir.join("private/living_world/skater_paths/DownTown.bin")).unwrap();
    let tiles = skate_data::aipath::parse_pack(&pack).unwrap();
    let (paths, _) = skate_data::aipath::district_paths(&tiles).unwrap();
    let lines: std::collections::BTreeMap<[u8; 16], ReplayLine> =
        paths.iter().filter(|p| p.path.id.is_ambient()).map(|p| (p.path.id.0, skate_data::living_world::replay_line(&p.path))).collect();
    let which = std::env::var("LINE_INDEX").ok().and_then(|v| v.parse().ok()).unwrap_or(0usize);
    // A line rolling on the ground for its first 300 frames.
    let line = lines.values().filter(|l| l.duration_frames() > 600 && l.nodes.iter().take(40).all(|n| n.flags & 0x0e == 0)).nth(which).expect("a ground line");
    let node = &line.nodes[0];
    let q = path_frame(node);
    let basis = skate_core::physics::rigid_body::basis_from_quaternion(skate_core::physics::rigid_body::RetailQuaternion { x: q[0], y: q[1], z: q[2], w: q[3] });
    let spawn = RetailAffineTransform { basis, translation: skate_core::math::Vector3::new(node.position[0], node.position[1], node.position[2]) };
    let mut context = physics.new_skater_context(spawn).unwrap();
    physics.swap_skater_context(&mut context);
    let mut npc = rig.skater(&physics);
    let mut cursor = LineCursor::spawn(&lines, line.id, 0);
    let mut errors = Vec::new();
    let start = physics.board.part_transforms()[BodyId::Deck.index()].translation;
    for tick in 0..300 {
        let target = cursor.line_target(&lines).unwrap();
        let deck = physics.board.part_transforms()[BodyId::Deck.index()];
        let forward = [deck.basis.columns[2][0], deck.basis.columns[2][1], deck.basis.columns[2][2]];
        let record = skate_core::living_world::ai_record::build(&target, forward, &Default::default());
        npc.runtime.ai_physics = Some(super::skater::AiPhysicsSource { record, fresh: true });
        // Retail spawn push while still on the start node (824701F8 -> 82C04168, every part).
        let node = &line.nodes[cursor.node as usize];
        let deck_at = [deck.translation.x, deck.translation.y, deck.translation.z];
        if let Some(v) = skate_core::living_world::ai_record::spawn_push(deck_at, node.position, target.step, cursor.node == 0, false) {
            for body in physics.board.bodies_mut() {
                body.rates.linear_velocity = skate_core::math::Vector3::new(v[0], v[1], v[2]);
            }
        }
        rig.step(&mut physics, &mut npc, pad());
        assert_eq!(npc.runtime.player_state.current(), PhysicalStateId::PhysicsGround, "on-board steering (bit 25) keeps the skater in ground physics, tick {tick}");
        cursor.step(&lines, &mut Decider::Stay, &mut Vec::new());
        let deck = physics.board.part_transforms()[BodyId::Deck.index()].translation;
        let e = ((deck.x - target.position[0]).powi(2) + (deck.z - target.position[2]).powi(2)).sqrt();
        errors.push(e);
        if tick % 30 == 0 {
            let v = physics.board.bodies()[BodyId::Deck.index()].rates.linear_velocity;
            let speed = (v.x * v.x + v.z * v.z).sqrt();
            let want = (target.step[0].powi(2) + target.step[2].powi(2)).sqrt() * 60.0;
            eprintln!("tick {tick} state {:?} speed {speed:.2} target speed {want:.2} error {e:.2}", npc.runtime.player_state.current());
        }
    }
    let end = physics.board.part_transforms()[BodyId::Deck.index()].translation;
    physics.swap_skater_context(&mut context);
    let mut sorted = errors.clone();
    sorted.sort_by(f32::total_cmp);
    eprintln!("error median {:.3} p90 {:.3} max {:.3}", sorted[150], sorted[270], sorted[299]);
    // With the retail spawn push it holds the recorded line: on DownTown lines 0..6 the median
    // error is 0.06..0.2 m (max under 0.6 m) except line 5, where the board loses speed on ground
    // geometry the recording rolls through (open, doc 26).
    if std::env::var_os("LINE_INDEX").is_none() {
        assert!(sorted[299] < 1.0 && sorted[150] < 0.5, "tracking: median {} max {}", sorted[150], sorted[299]);
    }
    let moved = ((end.x - start.x).powi(2) + (end.z - start.z).powi(2)).sqrt();
    assert!(moved > 5.0 && (end.y - start.y).abs() < 1.0, "{start:?} -> {end:?}");
}

/// A ped takedown on the player (`82592390`: actor `1904` bit 30 + direction) is published as the
/// packet's external impulse; retail's motion graph enters WipeOut on `IsPhysicsWiping` (b15).
/// Prints whether our skater reaches WipeoutGround (retail's switch is not proven statically).
#[test]
#[ignore = "requires private stock graphs and an installed map (SKATE3_ASSET_ROOT, SKATE3_MAP=<maps/DownTown.skate>)"]
fn a_ped_takedown_publishes_the_external_impulse_and_wipes_the_skater_out() {
    let root_dir = std::path::PathBuf::from(std::env::var_os("SKATE3_ASSET_ROOT").unwrap());
    let map_path = std::path::PathBuf::from(std::env::var_os("SKATE3_MAP").unwrap());
    let map = skate_data::skate_map::SkateMap::load(&map_path).unwrap();
    let assets = skate_data::GameAssets::load(&root_dir).unwrap();
    let rig = Rig { graphs: crate::graph_runtime::StockGraphs::load(&root_dir, &assets).unwrap(), root_dir: root_dir.clone() };
    let mut physics = GamePhysics::load_with_difficulty(&root_dir, Some(&map), crate::difficulty::Difficulty::Easy).unwrap();
    let mut player = rig.skater(&physics);
    let input = |t: u32| skate_core::input::xbox::XboxState { buttons: if t < 60 { 0x1000 } else { 0 }, ..pad() };
    for t in 0..120 {
        rig.step(&mut physics, &mut player, input(t));
    }
    assert_eq!(player.runtime.player_state.current(), PhysicalStateId::PhysicsGround);
    // The chaser hits from behind: 1 m against the board's motion.
    let deck = physics.board.bodies()[BodyId::Deck.index()].rates;
    let v = deck.linear_velocity;
    let speed = (v.x * v.x + v.z * v.z).sqrt().max(1e-3);
    let at = [deck.position.x, deck.position.y, deck.position.z];
    let chaser = [at[0] - v.x / speed, at[1], at[2] - v.z / speed];
    player.runtime.takedown = Some(super::skater::Takedown::from_positions(at, chaser));
    let mut states = Vec::new();
    for _ in 0..90 {
        rig.step(&mut physics, &mut player, pad());
        states.push(player.runtime.player_state.current());
    }
    eprintln!("takedown states: {:?}", states.iter().map(|s| *s as u32).collect::<Vec<_>>());
    assert!(states.contains(&PhysicalStateId::WipeoutGround), "no wipeout within 3 s of the takedown");
    assert!(player.runtime.takedown.is_none(), "the latch is used by WipeoutGround Enter");
}

/// Retail's car-hit bail (`sub_82D90C98`, b37): the skeleton's largest relative normal speed
/// against a vehicle-group body above `Wipeout_GroundVehicleContact` (9.0) wipes the skater
/// out. A traffic car (kinematic box in the vehicle group, as `living_world::vehicles::proxy`)
/// driving into the standing skater at 12 m/s knocks them down at once.
#[test]
#[ignore = "requires private stock graphs and an installed map (SKATE3_ASSET_ROOT, SKATE3_MAP=<maps/DownTown.skate>)"]
fn a_traffic_car_knocks_the_skater_down_above_the_contact_limit() {
    use skate_dynamics::rapier3d::prelude::{Pose, Rotation, SharedShape, Vector};
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
        for _ in 0..60 {
            x -= speed / 60.0;
            let centre = Vector::new(x, deck.y + 0.75, deck.z);
            let solid = skate_dynamics::SolidBody {
                id: 0x7E57_0000_0000_0001,
                pose: Pose::from_parts(centre, Rotation::IDENTITY),
                center_of_mass: centre,
                inertia_rotation: Rotation::IDENTITY,
                inverse_mass: 0.0,
                inverse_inertia: Vector::new(0.0, 0.0, 0.0),
                linvel: Vector::new(-speed, 0.0, 0.0),
                angvel: Vector::new(0.0, 0.0, 0.0),
                contact_group: group,
                colliders: vec![skate_dynamics::SolidCollider { shape: SharedShape::cuboid(2.15, 0.75, 0.9), pose: Pose::from_parts(centre, Rotation::IDENTITY), friction: 0.5 }],
            };
            let mut proxies = network::Proxies::default();
            proxies.append_solid(solid, &physics, &player.runtime, false);
            physics.network_proxies = proxies;
            rig.step(&mut physics, &mut player, pad());
            states.push(player.runtime.player_state.current());
        }
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

/// A ped's thrown can (`living_world::ped_hand_props::push_hand_prop_proxies`: a small finite-mass box in group 14)
/// hitting the standing skater's chest at the attack throw's 10 m/s is an ordinary contact (b92 Q2, b94): it reaches the
/// region forces (halved as a small object below 5.5 kg) and the usual wipeout check decides. Logs the outcome.
#[test]
#[ignore = "requires private stock graphs and an installed map (SKATE3_ASSET_ROOT, SKATE3_MAP=<maps/DownTown.skate>)"]
fn a_thrown_can_reaches_the_skater_region_forces() {
    use skate_dynamics::rapier3d::prelude::{Pose, Rotation, SharedShape, Vector};
    let root_dir = std::path::PathBuf::from(std::env::var_os("SKATE3_ASSET_ROOT").unwrap());
    let map_path = std::path::PathBuf::from(std::env::var_os("SKATE3_MAP").unwrap());
    let map = skate_data::skate_map::SkateMap::load(&map_path).unwrap();
    let assets = skate_data::GameAssets::load(&root_dir).unwrap();
    let rig = Rig { graphs: crate::graph_runtime::StockGraphs::load(&root_dir, &assets).unwrap(), root_dir: root_dir.clone() };
    let mut physics = GamePhysics::load_with_difficulty(&root_dir, Some(&map), crate::difficulty::Difficulty::Easy).unwrap();
    let mut player = rig.skater(&physics);
    for _ in 0..30 {
        rig.step(&mut physics, &mut player, pad());
    }
    let deck = physics.board.bodies()[BodyId::Deck.index()].rates.position;
    let (speed, mass) = (10.0_f32, 0.4_f32);
    let mut x = deck.x + 1.5;
    let (mut peak, mut wipeout) = (0.0_f32, None);
    for tick in 0..30 {
        x -= speed / 60.0;
        let centre = Vector::new(x, deck.y + 1.3, deck.z);
        let solid = skate_dynamics::SolidBody {
            id: 0x7E57_0000_0000_0002,
            pose: Pose::from_parts(centre, Rotation::IDENTITY),
            center_of_mass: centre,
            inertia_rotation: Rotation::IDENTITY,
            inverse_mass: 1.0 / mass,
            inverse_inertia: Vector::new(500.0, 500.0, 500.0),
            linvel: Vector::new(-speed, 0.0, 0.0),
            angvel: Vector::new(0.0, 0.0, 0.0),
            contact_group: 14,
            colliders: vec![skate_dynamics::SolidCollider { shape: SharedShape::cuboid(0.04, 0.07, 0.04), pose: Pose::from_parts(centre, Rotation::IDENTITY), friction: 0.5 }],
        };
        let mut proxies = network::Proxies::default();
        proxies.append_solid(solid, &physics, &player.runtime, false);
        physics.network_proxies = proxies;
        rig.step(&mut physics, &mut player, pad());
        peak = player.runtime.collision_feedback.regions.iter().map(|r| r.force).fold(peak, f32::max);
        if wipeout.is_none() && player.runtime.player_state.current() == PhysicalStateId::WipeoutGround {
            wipeout = Some(tick);
        }
    }
    eprintln!("thrown can {speed} m/s {mass} kg: peak region force {peak:.2}, wipeout tick {wipeout:?}");
    assert!(peak > 1.0, "the can's contact reaches the region forces: {peak}");
}

/// Peds are solid (`living_world::ped_body`, retail group 11, b138): a ped body closing on the standing skater is
/// solved against, so the contact reaches the group-11 force and the region forces (the skater no longer passes
/// through), and the ordinary wipeout check decides. Walking speed does not bail the skater; the closing speed of a
/// fast skater (relative speed is what the solve sees) is logged with its outcome.
#[test]
#[ignore = "requires private stock graphs and an installed map (SKATE3_ASSET_ROOT, SKATE3_MAP=<maps/DownTown.skate>)"]
fn a_ped_body_is_solid_to_the_skater() {
    let root_dir = std::path::PathBuf::from(std::env::var_os("SKATE3_ASSET_ROOT").unwrap());
    let map_path = std::path::PathBuf::from(std::env::var_os("SKATE3_MAP").unwrap());
    let map = skate_data::skate_map::SkateMap::load(&map_path).unwrap();
    let assets = skate_data::GameAssets::load(&root_dir).unwrap();
    let rig = Rig { graphs: crate::graph_runtime::StockGraphs::load(&root_dir, &assets).unwrap(), root_dir: root_dir.clone() };
    let run = |speed: f32| {
        let mut physics = GamePhysics::load_with_difficulty(&root_dir, Some(&map), crate::difficulty::Difficulty::Easy).unwrap();
        let mut player = rig.skater(&physics);
        for _ in 0..30 {
            rig.step(&mut physics, &mut player, pad());
        }
        let deck = physics.board.bodies()[BodyId::Deck.index()].rates.position;
        let mut x = deck.x + 1.2;
        let (mut group11, mut peak, mut wipeout) = (0.0_f32, 0.0_f32, None);
        for tick in 0..40 {
            x -= speed / 60.0;
            let feet = bevy::math::Vec3::new(x, deck.y - 0.1, deck.z);
            // Retail part 0 (the hips capsule) of a standing ped: hips 0.95 m, head 1.6 m above the feet.
            let part = &crate::living_world::ped_body::PedBodySettings::default().parts[0];
            let (hips, head) = (feet + bevy::math::Vec3::Y * 0.95, feet + bevy::math::Vec3::Y * 1.6);
            let solid = crate::living_world::ped_body::part_proxy(1, 0, hips, head, bevy::math::Vec3::new(-speed, 0.0, 0.0), part, 0.0);
            let mut proxies = network::Proxies::default();
            proxies.append_solid(solid, &physics, &player.runtime, false);
            physics.network_proxies = proxies;
            rig.step(&mut physics, &mut player, pad());
            group11 = group11.max(player.runtime.collision_feedback.maximum_group_11_force);
            peak = player.runtime.collision_feedback.regions.iter().map(|r| r.force).fold(peak, f32::max);
            if wipeout.is_none() && player.runtime.player_state.current() == PhysicalStateId::WipeoutGround {
                wipeout = Some(tick);
            }
        }
        eprintln!("ped body at {speed} m/s: group 11 force {group11:.2}, peak region force {peak:.2}, wipeout tick {wipeout:?}");
        (group11, peak, wipeout)
    };
    let (walk11, walk_peak, walk_wipeout) = run(1.4);
    assert!(walk11 > 0.0 && walk_peak > 0.0, "the walking ped's contact is solved (group 11 {walk11}, region {walk_peak})");
    assert!(walk_wipeout.is_none(), "a walking ped does not bail the skater");
    // Retail part 0 (10 kg, r 0.20); 2026-10-16 with the earlier 75 kg stand-in: 1.30 / 20.61, wipeout tick 8.
    let (fast11, _, fast_wipeout) = run(9.0);
    assert!(fast11 > walk11, "a harder hit gives a larger group-11 force ({fast11} vs {walk11})");
    assert!(fast_wipeout.is_some(), "running into a ped fast bails the skater (user: retail does)");
}

/// DMO streaming on DownTown's placed props (doc 27 "DMO streaming"): from the map spawn the fill pass keeps at most
/// 49 live props, all within 90 m; the others go dormant (collision parked, out of the obstacle list); walking 300 m away
/// culls the near ones and brings the props there in.
#[test]
#[ignore = "requires an installed map (SKATE3_ASSET_ROOT, SKATE3_MAP=<maps/DownTown.skate>)"]
fn dmo_streaming_keeps_49_live_props_round_the_spawn() {
    use skate_core::living_world::dmo::{DmoDecision, DmoStream, DmoStreamSettings, DmoView};
    use skate_core::living_world::Observer;
    let root_dir = std::path::PathBuf::from(std::env::var_os("SKATE3_ASSET_ROOT").unwrap());
    let map_path = std::path::PathBuf::from(std::env::var_os("SKATE3_MAP").unwrap());
    let map = skate_data::skate_map::SkateMap::load(&map_path).unwrap();
    let mut physics = GamePhysics::load_with_difficulty(&root_dir, Some(&map), crate::difficulty::Difficulty::Easy).unwrap();
    let placements = physics.prop_dynamics().unwrap().dmo_placements();
    let total = placements.len();
    let settings = DmoStreamSettings::default();
    let mut stream = DmoStream::new(placements, &settings);
    let deck = physics.board.bodies()[BodyId::Deck.index()].rates.position;
    let view = |x: f32, z: f32| (Observer { position: [x, deck.y, z], velocity: [0.0; 3] }, DmoView { camera: [x, deck.y + 2.0, z], forward: [0.0, 0.0, 1.0], reference: [x, deck.y, z] });
    let here = view(deck.x, deck.z);
    stream.step(&[here], &settings, true, &|_| false);
    let dormant: Vec<u32> = stream.placements.iter().filter(|p| !stream.live.contains(&p.id)).map(|p| p.id).collect();
    for &id in &dormant {
        assert!(physics.stream_prop(id, true, true));
    }
    let live = stream.live.len();
    eprintln!("DownTown: {total} placed props, {live} live after the fill, {} dormant", dormant.len());
    assert!(live <= 49 && live > 0);
    let flat = |p: [f32; 3]| ((p[0] - deck.x).powi(2) + (p[2] - deck.z).powi(2)).sqrt();
    assert!(stream.placements.iter().filter(|p| stream.live.contains(&p.id)).all(|p| flat(p.position) <= 90.0));
    let dynamics = physics.prop_dynamics().unwrap();
    assert_eq!(dynamics.obstacle_boxes().len(), live, "dormant props leave the obstacle list");
    assert!(dormant.iter().all(|&id| dynamics.is_dormant(id)));
    // Knock one live prop 2 m aside: after a cull it comes back at its authored pose (retail, b98).
    let moved = *stream.live.iter().next().unwrap();
    let (spawn_origin, spawn_basis) = physics.prop_dynamics().unwrap().spawn_pose(moved).unwrap();
    let shifted = skate_core::math::Vector3::new(spawn_origin.x + 2.0, spawn_origin.y, spawn_origin.z);
    physics.prop_dynamics_mut().unwrap().teleport(moved, shifted, spawn_basis);
    // Far away: the spawn's props cull, others come in.
    let far = view(deck.x + 300.0, deck.z);
    let mut changes = Vec::new();
    for _ in 0..20 {
        changes.extend(stream.step(&[far], &settings, false, &|_| false));
    }
    let culled = changes.iter().filter(|d| matches!(d, DmoDecision::Cull(_))).count();
    eprintln!("300 m away: {culled} culled, {} spawned, {} live", changes.iter().filter(|d| matches!(d, DmoDecision::Spawn(_))).count(), stream.live.len());
    assert!(culled > 0);
    for d in changes {
        match d {
            DmoDecision::Spawn(id) => assert!(physics.stream_prop(id, false, true)),
            DmoDecision::Cull(id) | DmoDecision::Evict(id) => assert!(physics.stream_prop(id, true, true)),
        }
    }
    assert_eq!(physics.prop_dynamics().unwrap().obstacle_boxes().len(), stream.live.len());
    // Back at the spawn: the knocked prop is in again, at its authored pose.
    for _ in 0..20 {
        for d in stream.step(&[here], &settings, false, &|_| false) {
            match d {
                DmoDecision::Spawn(id) => assert!(physics.stream_prop(id, false, true)),
                DmoDecision::Cull(id) | DmoDecision::Evict(id) => assert!(physics.stream_prop(id, true, true)),
            }
        }
    }
    assert!(stream.live.contains(&moved) && !physics.prop_dynamics().unwrap().is_dormant(moved));
    let (origin, _) = physics.prop_dynamics().unwrap().pose(moved).unwrap();
    assert!((origin.x - spawn_origin.x).abs() < 1e-4 && (origin.z - spawn_origin.z).abs() < 1e-4, "authored pose: {origin:?} vs {spawn_origin:?}");
}

/// Mode 7's controller B (NavMeshController, b78 to b80) on a simulated skater: B's intents alone (no pad, the AI
/// source present but not fresh) make it press the off-board toggle, step off, turn on foot through `OB_Steer` ->
/// `ob_Turn` (Processed +2680 = -OB_Steer) and line up with the node direction, then hand back (arrived).
#[test]
#[ignore = "requires private stock graphs and an installed map (SKATE3_ASSET_ROOT, SKATE3_MAP=<maps/DownTown.skate>)"]
fn controller_b_steps_off_and_walks_the_skater_onto_the_node() {
    use skate_core::living_world::controller_b::{BTick, ControllerB, ControllerBSettings, HandBack, PathElement, PathService};
    // The goal as the only corner: what the peds' navmesh returns on open ground.
    struct Corner;
    impl PathService for Corner {
        fn solve(&self, _from: [f32; 3], to: [f32; 3]) -> Option<Vec<PathElement>> {
            Some(vec![PathElement { entry: to, exit: to }])
        }
    }
    let root_dir = std::path::PathBuf::from(std::env::var_os("SKATE3_ASSET_ROOT").unwrap());
    let map_path = std::path::PathBuf::from(std::env::var_os("SKATE3_MAP").unwrap());
    let map = skate_data::skate_map::SkateMap::load(&map_path).unwrap();
    let assets = skate_data::GameAssets::load(&root_dir).unwrap();
    let graphs = crate::graph_runtime::StockGraphs::load(&root_dir, &assets).unwrap();
    let mut physics = GamePhysics::load_with_difficulty(&root_dir, Some(&map), crate::difficulty::Difficulty::Easy).unwrap();
    let spawn = physics.board.part_transforms()[BodyId::Deck.index()];
    let mut context = physics.new_skater_context(spawn).unwrap();
    let mut runtime = physics.load_skater_in_context(&mut context, &root_dir, &graphs, "easy", None).unwrap();
    let mut controls = PlayerControls::default();
    let mut camera = crate::camera::CameraRuntime::load(&root_dir).unwrap();
    // Settle on the board for half a second (an empty, present AI source, as while B is active).
    let record = skate_core::living_world::ai_record::build(
        &skate_core::living_world::ai_record::LineTarget { position: [spawn.translation.x, spawn.translation.y, spawn.translation.z], frame: [0.0, 0.0, 0.0, 1.0], step: [0.0; 3] },
        [0.0, 0.0, 1.0],
        &Default::default(),
    );
    runtime.ai_physics = Some(super::skater::AiPhysicsSource { record, fresh: false });
    for _ in 0..30 {
        physics.advance_npc_skater(&mut context, &mut runtime, &mut controls, &graphs, &mut camera, &[]).unwrap();
    }
    let root = runtime.animated_skeleton.roots.animation_to_world;
    let at = [root[3][0], root[3][1], root[3][2]];
    let forward = [root[2][0], 0.0, root[2][2]];
    let right = [forward[2], 0.0, -forward[0]];
    let l = (right[0] * right[0] + right[2] * right[2]).sqrt();
    let right = right.map(|x| x / l);
    let target = [at[0] + right[0] * 3.0, at[1], at[2] + right[2] * 3.0];
    let s = ControllerBSettings::default();
    let input = crate::living_world::npc_sim::walker_input(&runtime);
    let mut b = ControllerB::activate(&s, &Corner, target, right, &input).expect("the corner plan accepts the goal");
    let (mut off_board_at, mut turn_checked, mut result) = (None, false, None);
    for tick in 0..600 {
        let input = crate::living_world::npc_sim::walker_input(&runtime);
        let at = input.position;
        if input.byte_161 && off_board_at.is_none() {
            off_board_at = Some(tick);
        }
        let intents: Vec<(String, f32)> = match b.tick(&s, &Corner, &input) {
            BTick::Controls(c) => c.intents().into_iter().map(|(k, v)| (k.to_string(), v)).collect(),
            BTick::HandBack { reason, .. } => {
                result = Some((tick, reason));
                break;
            }
            BTick::Idle => Vec::new(),
        };
        let steer = intents.iter().find(|x| x.0 == "OB_Steer").map(|x| x.1);
        physics.advance_npc_skater(&mut context, &mut runtime, &mut controls, &graphs, &mut camera, &intents).unwrap();
        if runtime.player_state.current() == PhysicalStateId::BipedGround && tick < 40 {
            let x = &runtime.animation_input.extra;
            eprintln!("  tick {tick}: OB_Mag sent {:?} -> magnitude {:.3}, OB_Steer sent {steer:?} -> turn {:.3}", intents.iter().find(|x| x.0 == "OB_Mag").map(|x| x.1), x.offboard_magnitude, x.offboard_turn);
        }
        if let (Some(steer), PhysicalStateId::BipedGround) = (steer, runtime.player_state.current()) {
            if steer.abs() > 0.1 && !turn_checked && tick < 60 {
                let turn = runtime.animation_input.extra.offboard_turn;
                eprintln!("tick {tick}: OB_Steer {steer:.3} -> offboard_turn {turn:.3}, state {:?}", runtime.player_state.current());
                if (turn + steer).abs() < 1e-4 {
                    turn_checked = true;
                }
            }
        }
        if tick % 30 == 0 {
            eprintln!("tick {tick} state {:?} at {at:?} steer {steer:?} heading error {:.2}", runtime.player_state.current(), b.heading_error);
        }
    }
    eprintln!("off board at {off_board_at:?}, result {result:?}");
    assert!(off_board_at.is_some(), "the toggle presses never took the skater off the board");
    assert!(turn_checked, "no steer while off the board");
    assert!(matches!(result, Some((_, HandBack::Arrived))), "{result:?}");
}

/// Two simulated NPC skaters stepped on their own workers in parallel (`GamePhysics::npc_worker`, the compute pool)
/// match the same NPCs stepped one after another through the shared physics, bit for bit, every tick: the players too.
/// Both runs give each NPC the proxies as they stand after the player's step and swap its own collision settings.
#[test]
#[ignore = "requires private stock graphs and an installed map (SKATE3_ASSET_ROOT, SKATE3_MAP=<maps/DownTown.skate>)"]
fn npc_skaters_stepped_in_parallel_match_the_serial_steps_bit_for_bit() {
    let root_dir = std::path::PathBuf::from(std::env::var_os("SKATE3_ASSET_ROOT").unwrap());
    let map_path = std::path::PathBuf::from(std::env::var_os("SKATE3_MAP").unwrap());
    let map = skate_data::skate_map::SkateMap::load(&map_path).unwrap();
    let assets = skate_data::GameAssets::load(&root_dir).unwrap();
    let rig = Rig { graphs: crate::graph_runtime::StockGraphs::load(&root_dir, &assets).unwrap(), root_dir: root_dir.clone() };
    let load = || GamePhysics::load_with_difficulty(&root_dir, Some(&map), crate::difficulty::Difficulty::Easy).unwrap();
    let input = |t: u32| skate_core::input::xbox::XboxState { buttons: if t < 60 { 0x1000 } else { 0 }, ..pad() };
    struct Npc {
        context: SkaterPhysicsContext,
        runtime: SkaterRuntime,
        controls: PlayerControls,
        camera: crate::camera::CameraRuntime,
    }
    let npcs = |physics: &mut GamePhysics| -> Vec<Npc> {
        [4.0f32, -4.0].iter().map(|dx| {
            let mut spawn = physics.board.part_transforms()[BodyId::Deck.index()];
            spawn.translation.x += dx;
            let mut context = physics.new_skater_context(spawn).unwrap();
            GamePhysics::set_context_velocity(&mut context, [0.0, 0.0, 3.0]);
            let runtime = physics.load_skater_in_context(&mut context, &root_dir, &rig.graphs, "easy", None).unwrap();
            Npc { context, runtime, controls: PlayerControls::default(), camera: crate::camera::CameraRuntime::load(&root_dir).unwrap() }
        }).collect()
    };
    let deck = |c: &SkaterPhysicsContext| {
        let r = &c.board.bodies()[BodyId::Deck.index()].rates;
        [r.position.x, r.position.y, r.position.z, r.linear_velocity.x, r.linear_velocity.y, r.linear_velocity.z].map(f32::to_bits)
    };
    let intents = |t: u32| if t % 90 < 45 { vec![("Turn".to_string(), 0.5f32)] } else { Vec::new() };

    // Serial: each NPC through the shared physics, from the post-player proxies.
    let mut a = load();
    let mut player_a = rig.skater(&a);
    let mut npcs_a = npcs(&mut a);
    let mut serial = Vec::new();
    for t in 0..240 {
        rig.step(&mut a, &mut player_a, input(t));
        let proxies = a.network_proxies.bodies.clone();
        for n in &mut npcs_a {
            a.network_proxies.bodies.clone_from(&proxies);
            a.advance_npc_skater(&mut n.context, &mut n.runtime, &mut n.controls, &rig.graphs, &mut n.camera, &intents(t)).unwrap();
        }
        serial.push((trace(&a), npcs_a.iter().map(|n| deck(&n.context)).collect::<Vec<_>>()));
    }

    // Parallel: each NPC on its own worker, on the compute pool.
    let mut b = load();
    let mut player_b = rig.skater(&b);
    let mut npcs_b = npcs(&mut b);
    let mut workers: Vec<GamePhysics> = (0..npcs_b.len()).map(|_| b.npc_worker().unwrap()).collect();
    for w in &mut workers {
        GamePhysics::release_npc_worker(w);
    }
    for t in 0..240 {
        rig.step(&mut b, &mut player_b, input(t));
        for w in &mut workers {
            b.sync_npc_worker(w);
        }
        let graphs = &rig.graphs;
        let step_intents = intents(t);
        let results = bevy::tasks::ComputeTaskPool::get_or_init(bevy::tasks::TaskPool::default).scope(|scope| {
            for (n, w) in npcs_b.iter_mut().zip(workers.iter_mut()) {
                let i = &step_intents;
                scope.spawn(async move { w.advance_npc_skater(&mut n.context, &mut n.runtime, &mut n.controls, graphs, &mut n.camera, i) });
            }
        });
        results.into_iter().for_each(|r| r.unwrap());
        b.network_proxies.clone_from(&workers.last().unwrap().network_proxies);
        for w in &mut workers {
            GamePhysics::release_npc_worker(w);
        }
        let got = (trace(&b), npcs_b.iter().map(|n| deck(&n.context)).collect::<Vec<_>>());
        assert_eq!(got, serial[t as usize], "parallel diverged from serial at tick {t}");
    }
    let moved = npcs_b.iter().map(|n| n.context.board.part_transforms()[BodyId::Deck.index()].translation.z).collect::<Vec<_>>();
    assert!(npcs_b.iter().all(|n| !n.context.failed), "npc physics failed; decks {moved:?}");
}

/// The player and a simulated NPC skater collide as full skaters (`living_world::npc_skaters::push_proxies` and
/// `npc_sim::run_steps`: each solve sees the other's board and skeleton as a peer proxy, retail group 5; research m7 /
/// b125): an NPC riding hard into the standing player is knocked down by its own skeleton contact and wipeout check; a
/// slow roll-in is not. Logs both sides' first wipeout tick.
#[test]
#[ignore = "requires private stock graphs and an installed map (SKATE3_ASSET_ROOT, SKATE3_MAP=<maps/DownTown.skate>)"]
fn a_hard_bump_knocks_a_simulated_npc_skater_down() {
    let root_dir = std::path::PathBuf::from(std::env::var_os("SKATE3_ASSET_ROOT").unwrap());
    let map_path = std::path::PathBuf::from(std::env::var_os("SKATE3_MAP").unwrap());
    let map = skate_data::skate_map::SkateMap::load(&map_path).unwrap();
    let assets = skate_data::GameAssets::load(&root_dir).unwrap();
    let rig = Rig { graphs: crate::graph_runtime::StockGraphs::load(&root_dir, &assets).unwrap(), root_dir: root_dir.clone() };
    let at = skate_net::prediction::CollisionPrediction::at(0.0).unwrap();
    let run = |speed: f32| {
        let mut physics = GamePhysics::load_with_difficulty(&root_dir, Some(&map), crate::difficulty::Difficulty::Easy).unwrap();
        let mut player = rig.skater(&physics);
        for _ in 0..30 {
            rig.step(&mut physics, &mut player, pad());
        }
        let schema = network::Schema::new(&physics, &player.runtime).unwrap();
        let mut spawn = physics.board.part_transforms()[BodyId::Deck.index()];
        spawn.translation.z -= 4.0;
        let mut context = physics.new_skater_context(spawn).unwrap();
        let mut npc = physics.load_skater_in_context(&mut context, &root_dir, &rig.graphs, "easy", None).unwrap();
        let mut controls = PlayerControls::default();
        let mut camera = crate::camera::CameraRuntime::load(&root_dir).unwrap();
        let mut npc_body = GamePhysics::capture_context_body(&context, &npc);
        let (mut mine, mut theirs, mut touched) = (None, None, 0usize);
        for t in 0..240 {
            let mut proxies = network::Proxies::default();
            proxies.append(crate::living_world::npc_skaters::PLAYER_PEER | 1, &npc_body, &schema, &physics, &player.runtime, at);
            physics.network_proxies = proxies;
            rig.step(&mut physics, &mut player, pad());
            let player_body = network::capture_body(&physics, &player.runtime);
            let mut proxies = network::Proxies::default();
            proxies.append(crate::living_world::npc_skaters::PLAYER_PEER, &player_body, &schema, &physics, &npc, at);
            physics.network_proxies = proxies;
            // Riding in at `speed` until it touches the player (the sim's spawn push does the same, `ai_record::spawn_push`).
            if touched == 0 {
                GamePhysics::set_context_velocity(&mut context, [0.0, 0.0, speed]);
            }
            physics.advance_npc_skater(&mut context, &mut npc, &mut controls, &rig.graphs, &mut camera, &[]).unwrap();
            npc_body = GamePhysics::capture_context_body(&context, &npc);
            touched = touched.max(physics.network_contacts);
            if std::env::var_os("BUMP_TRACE").is_some() && t % 5 == 0 {
                let d = GamePhysics::context_deck(&context).translation;
                let v = context.board.bodies()[BodyId::Deck.index()].rates.linear_velocity;
                let p = physics.board.bodies()[BodyId::Deck.index()].rates.position;
                eprintln!("t {t} npc z {:.2} v {:.2} player z {:.2} contacts {} states {:?} {:?}", d.z, v.z, p.z, physics.network_contacts, npc.player_state.current(), player.runtime.player_state.current());
            }
            if theirs.is_none() && npc.player_state.current() == PhysicalStateId::WipeoutGround {
                theirs = Some(t);
            }
            if mine.is_none() && player.runtime.player_state.current() == PhysicalStateId::WipeoutGround {
                mine = Some(t);
            }
        }
        eprintln!("{speed} m/s: npc wipeout tick {theirs:?}, player wipeout tick {mine:?}, most contacts {touched}");
        (theirs, mine, touched)
    };
    // 2026-10-16 (DownTown, 4 m run-up, pushed until the first contact): 9 m/s -> the NPC wipes out at tick 26; the
    // standing player (its side: force x SkaterSkeletonScalar, thresholds x SkaterSkaterThresholdScalar) goes down at
    // tick 36 as the sliding NPC keeps pushing. Only the NPC side is asserted (no retail measurement of the player's).
    let (npc, _, touched) = run(9.0);
    assert!(touched > 0 && npc.is_some(), "a 9 m/s skater riding into the player wipes out");
    let (npc, player, touched) = run(2.0);
    assert!(touched > 0, "the slow skater reaches the player");
    assert_eq!((npc, player), (None, None), "a slow roll-in knocks nobody down");
}

/// Skitching end to end (2026-10-16: the first live skitch failed in PreState, then would have in FillPhysOut): the
/// skater rolls, a taxi with its exported rear grab spline drives just ahead at the skater's speed (grab scene +
/// solid proxy, as `living_world::vehicles` feeds them), grab is held: the skater must latch (Skitching), be towed
/// for seconds through every frame stage without an error, and leave Skitching after letting go.
#[test]
#[ignore = "requires private stock graphs, the living-world export and an installed map (SKATE3_ASSET_ROOT, SKATE3_MAP=<maps/DownTown.skate>)"]
fn a_skater_latches_onto_a_traffic_car_rides_and_lets_go() {
    if std::env::var_os("SKATE_TEST_LOG").is_some() {
        let _ = tracing::tracing_subscriber::fmt().with_writer(std::io::stderr).with_ansi(false).try_init();
    }
    use skate_dynamics::rapier3d::prelude::{Pose, Rotation, Vector};
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
    for t in 0..420 {
        origin += heading * (car_speed / 60.0);
        let transform = Transform::from_translation(origin).with_rotation(rotation);
        let object = crate::living_world::vehicles::grab_object_at(5, &splines, &transform, heading * car_speed);
        physics.set_grab_cars(vec![object]).unwrap();
        let centre = Vector::new(origin.x, origin.y + 0.75, origin.z);
        let q = Rotation::from_rotation_y(heading.x.atan2(heading.z));
        let solid = skate_dynamics::SolidBody {
            id: 0x7E57_0000_0000_0005,
            pose: Pose::from_parts(centre, q),
            center_of_mass: centre,
            inertia_rotation: Rotation::IDENTITY,
            inverse_mass: 0.0,
            inverse_inertia: Vector::new(0.0, 0.0, 0.0),
            linvel: Vector::new(heading.x * car_speed, 0.0, heading.z * car_speed),
            angvel: Vector::new(0.0, 0.0, 0.0),
            contact_group: VEHICLE_GROUP,
            colliders: vec![skate_dynamics::SolidCollider { shape: mesh.clone(), pose: Pose::from_parts(Vector::new(origin.x, origin.y, origin.z), q), friction: 0.5 }],
        };
        let mut proxies = network::Proxies::default();
        proxies.append_solid(solid, &physics, &player.runtime, false);
        physics.network_proxies = proxies;
        // Grab for 5 s, then let go.
        rig.step(&mut physics, &mut player, if t < 300 { grab } else { pad() });
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

/// A simulated NPC body built on another body's animation source (`NpcSimPool::source`, the load-time optimisation of
/// 2026-10-16: no second disk read of the banks and poses) rides exactly as a body loaded from disk: both ride the same
/// push for 240 ticks in their own contexts and their decks and skeleton roots match bit for bit every tick.
#[test]
#[ignore = "requires private stock graphs and an installed map (SKATE3_ASSET_ROOT, SKATE3_MAP=<maps/DownTown.skate>)"]
fn a_body_on_a_shared_animation_source_rides_like_a_fresh_load() {
    let root_dir = std::path::PathBuf::from(std::env::var_os("SKATE3_ASSET_ROOT").unwrap());
    let map_path = std::path::PathBuf::from(std::env::var_os("SKATE3_MAP").unwrap());
    let map = skate_data::skate_map::SkateMap::load(&map_path).unwrap();
    let assets = skate_data::GameAssets::load(&root_dir).unwrap();
    let graphs = crate::graph_runtime::StockGraphs::load(&root_dir, &assets).unwrap();
    let mut physics = GamePhysics::load_with_difficulty(&root_dir, Some(&map), crate::difficulty::Difficulty::Easy).unwrap();
    let spawn = physics.board.part_transforms()[BodyId::Deck.index()];
    let mut fresh_context = physics.new_skater_context(spawn).unwrap();
    let t0 = std::time::Instant::now();
    let mut fresh = physics.load_skater_in_context(&mut fresh_context, &root_dir, &graphs, "easy", None).unwrap();
    let fresh_ms = t0.elapsed().as_secs_f64() * 1000.0;
    let mut shared_context = physics.new_skater_context(spawn).unwrap();
    let t0 = std::time::Instant::now();
    let mut shared = physics.load_skater_in_context(&mut shared_context, &root_dir, &graphs, "easy", Some(fresh.animation.source.clone())).unwrap();
    let shared_ms = t0.elapsed().as_secs_f64() * 1000.0;
    eprintln!("load: from disk {fresh_ms:.1} ms, on the shared source {shared_ms:.1} ms");
    let (mut controls_a, mut controls_b) = (PlayerControls::default(), PlayerControls::default());
    let mut camera_a = crate::camera::CameraRuntime::load(&root_dir).unwrap();
    let mut camera_b = crate::camera::CameraRuntime::load(&root_dir).unwrap();
    for t in 0..240 {
        if t < 30 {
            GamePhysics::set_context_velocity(&mut fresh_context, [0.0, 0.0, 5.0]);
            GamePhysics::set_context_velocity(&mut shared_context, [0.0, 0.0, 5.0]);
        }
        physics.advance_npc_skater(&mut fresh_context, &mut fresh, &mut controls_a, &graphs, &mut camera_a, &[]).unwrap();
        physics.advance_npc_skater(&mut shared_context, &mut shared, &mut controls_b, &graphs, &mut camera_b, &[]).unwrap();
        let a = GamePhysics::context_deck(&fresh_context);
        let b = GamePhysics::context_deck(&shared_context);
        assert_eq!(format!("{a:?}"), format!("{b:?}"), "deck differs at tick {t}");
        assert_eq!(
            format!("{:?}", fresh.animated_skeleton.roots.animation_to_world),
            format!("{:?}", shared.animated_skeleton.roots.animation_to_world),
            "skeleton root differs at tick {t}"
        );
        assert_eq!(fresh.player_state.current(), shared.player_state.current(), "state differs at tick {t}");
    }
}

/// A simulated NPC's audio record (`npc_sim::run_steps`: its context swapped in, `skate_events::skater_audio_state`)
/// is its own: its board position and its wheels on the ground, not the player's (research b161 item 1: the NPC
/// tiers sent a cut-down record, so bails, pushes and impacts never sounded).
#[test]
#[ignore = "requires private stock graphs and an installed map (SKATE3_ASSET_ROOT, SKATE3_MAP=<maps/DownTown.skate>)"]
fn a_simulated_npc_skaters_audio_record_is_its_own() {
    let root_dir = std::path::PathBuf::from(std::env::var_os("SKATE3_ASSET_ROOT").unwrap());
    let map_path = std::path::PathBuf::from(std::env::var_os("SKATE3_MAP").unwrap());
    let map = skate_data::skate_map::SkateMap::load(&map_path).unwrap();
    let assets = skate_data::GameAssets::load(&root_dir).unwrap();
    let graphs = crate::graph_runtime::StockGraphs::load(&root_dir, &assets).unwrap();
    let mut physics = GamePhysics::load_with_difficulty(&root_dir, Some(&map), crate::difficulty::Difficulty::Easy).unwrap();
    let mut spawn = physics.board.part_transforms()[BodyId::Deck.index()];
    spawn.translation.z -= 6.0;
    let mut context = physics.new_skater_context(spawn).unwrap();
    let mut npc = physics.load_skater_in_context(&mut context, &root_dir, &graphs, "easy", None).unwrap();
    let mut controls = PlayerControls::default();
    let mut camera = crate::camera::CameraRuntime::load(&root_dir).unwrap();
    let mut memory = crate::game_audio::skate_events::SkaterAudioMemory::default();
    let mut state = None;
    for t in 0..90 {
        if t < 30 {
            GamePhysics::set_context_velocity(&mut context, [0.0, 0.0, 4.0]);
        }
        physics.advance_npc_skater(&mut context, &mut npc, &mut controls, &graphs, &mut camera, &[]).unwrap();
        physics.swap_skater_context(&mut context);
        state = Some(crate::game_audio::skate_events::skater_audio_state(&physics, &npc, &mut memory, 1.0 / 30.0));
        physics.swap_skater_context(&mut context);
    }
    let state = state.unwrap();
    let deck = GamePhysics::context_deck(&context).translation;
    let player = physics.board.part_transforms()[BodyId::Deck.index()].translation;
    let d = |p: [f32; 3], q: skate_core::math::Vector3| ((p[0] - q.x).powi(2) + (p[2] - q.z).powi(2)).sqrt();
    eprintln!("npc deck {deck:?}, record board {:?}, wheels {}", state.board_position, state.wheel_count);
    assert!(d(state.board_position, deck) < 0.05, "the record is at the NPC's board");
    assert!(d(state.board_position, player) > 1.0, "not at the player's board");
    assert!(state.wheel_count > 0, "the rolling NPC has its wheels down");
}
