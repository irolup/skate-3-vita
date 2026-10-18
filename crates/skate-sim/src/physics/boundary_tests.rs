//! Retail sea reset (doc 09, "Falling into the Industrial sea"): Industrial's sea has an
//! invisible floor at y -4.0 (surface 768, physics type 6 `physics_unrideable`). Touching it
//! raises the teleport request (82DB8120), so the checkpoint reset carries reason `boundary`.
//! Real map and stock graphs: drop the skater over open sea, read the respawn event.
use super::*;

#[test]
#[ignore = "requires private stock graphs and Industrial (SKATE3_ASSET_ROOT, SKATE3_MAP=<maps/Industrial.skate>)"]
fn skater_dropped_onto_the_industrial_sea_floor_respawns_with_reason_boundary() {
    let root = std::path::PathBuf::from(std::env::var_os("SKATE3_ASSET_ROOT").unwrap());
    let map_path = std::path::PathBuf::from(std::env::var_os("SKATE3_MAP").unwrap());
    let map = skate_data::skate_map::SkateMap::load(&map_path).unwrap();
    let assets = skate_data::GameAssets::load(&root).unwrap();
    let graphs = crate::graph_runtime::StockGraphs::load(&root, &assets).unwrap();
    let mut physics =
        GamePhysics::load_with_difficulty(&root, Some(&map), crate::difficulty::Difficulty::Easy)
            .unwrap();
    // First column whose top surface is the type-6 sea floor (x -3679..1839, z -492..1457).
    let mut sea = None;
    'scan: for x in (-3600..1800).step_by(100) {
        for z in (-400..1400).step_by(100) {
            let (x, z) = (x as f32, z as f32);
            let probe = offboard::contact_queries::Probe {
                start: [x, 60., z, 0.],
                end: [x, -20., z, 0.],
                radius: 0.,
            };
            if let Some(hit) = offboard::contact_queries::query(&physics.world, probe, 0).unwrap() {
                let surface = u32::from(hit.packed_surface);
                if (surface >> 7) & 31 == 6 && hit.geometry.position.y < -3. {
                    sea = Some((x, hit.geometry.position.y, z, surface));
                    break 'scan;
                }
            }
        }
    }
    let (x, floor, z, surface) = sea.expect("no open-sea column with the type-6 floor on top");
    eprintln!("BOUNDARY_TEST sea column x={x} z={z} floor={floor} surface={surface}");
    let mut skater = SkaterRuntime::load(&root, &graphs, &physics, "easy").unwrap();
    let mut camera = crate::camera::CameraRuntime::load(&root).unwrap();
    let mut controls = PlayerControls::default();
    let mut input = crate::input::ControllerInput::default();
    let mut events = Vec::new();
    let mut dropped_at = 0;
    for tick in 0..600u32 {
        if tick == 60 {
            let mut over = skater.animated_skeleton.roots.animation_to_world;
            over[3][0] = x;
            over[3][1] = floor + 3.;
            over[3][2] = z;
            skater.travel_to(over).unwrap();
            dropped_at = physics.ticks;
        }
        input.sample_raw_for_test(skate_core::input::xbox::XboxState::default());
        let mut actions = input.player_actions();
        controls.update_for_physics(&mut actions, &physics, &skater, &camera).unwrap();
        frame::advance(&mut physics, &mut skater, &mut controls, &graphs, &mut actions, true, &mut camera)
            .unwrap();
        events.append(&mut skater.respawn.outbox);
        if !events.is_empty() {
            break;
        }
    }
    eprintln!("BOUNDARY_TEST dropped_at={dropped_at} events={events:?}");
    let event = events.first().expect("no checkpoint respawn after the drop");
    assert!(event.tick > dropped_at, "respawn before the drop");
    assert_eq!(event.reason, respawn::RespawnReason::Boundary);
    // Far below the 5 s air timeout: the floor, not the air count, made the request.
    assert!(event.tick - dropped_at < 300, "{} ticks after the drop", event.tick - dropped_at);
    let json = serde_json::to_string(event).unwrap();
    assert!(json.contains("\"reason\":\"boundary\""), "{json}");
}
