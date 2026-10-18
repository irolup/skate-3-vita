//! Diagnostic: drop the skater into University's fountain basin and trace how
//! the body and board move through the water, tick by tick.
//! SKATE3_ASSET_ROOT=<assets> SKATE3_WATER_MAP=<University.skate>
//! cargo test --release --bin skate3rust -- --ignored --nocapture water_drop
use super::*;
use skate_core::player::state::PhysicalStateId;

#[test]
#[ignore = "requires private assets and a converted University map; diagnostic only"]
fn water_drop_trace() {
    let root = std::env::var_os("SKATE3_ASSET_ROOT").expect("set SKATE3_ASSET_ROOT");
    let root = std::path::Path::new(&root);
    let map_path = std::env::var_os("SKATE3_WATER_MAP").expect("set SKATE3_WATER_MAP");
    let map = skate_data::skate_map::SkateMap::load(std::path::Path::new(&map_path)).unwrap();
    let assets = skate_data::GameAssets::load(root).unwrap();
    let graphs = crate::graph_runtime::StockGraphs::load(root, &assets).unwrap();
    let mut physics = GamePhysics::load_with_world(root, ground::Terrain::Course, Some(&map)).unwrap();
    let mut skater = SkaterRuntime::load(root, &graphs, &physics, "normal").unwrap();
    let mut controls = PlayerControls::default();
    let mut input = crate::input::ControllerInput::default();
    let mut camera = crate::camera::CameraRuntime::load(root).unwrap();
    // F2 of the water test mod: 2 m above the basin's water (67.94 m).
    let drop: [f32; 3] = std::env::var("SKATE3_WATER_DROP")
        .ok()
        .and_then(|v| {
            let p: Vec<f32> = v.split(',').filter_map(|s| s.trim().parse().ok()).collect();
            (p.len() == 3).then(|| [p[0], p[1], p[2]])
        })
        .unwrap_or([340.3, 69.94, -294.3]);
    let water = physics
        .world
        .water_surface_at(skate_core::math::Vector3::new(drop[0], drop[1] - 3., drop[2]), 0.05, 4.);
    eprintln!("drop {drop:?}, water surface below it: {water:?}");
    let surface = water.unwrap_or(drop[1]);
    eprintln!(
        "water at the drop point is {}",
        if physics.world.water_shallow_at(skate_core::math::Vector3::new(drop[0], surface, drop[2])) { "shallow (floor within FLOAT_DEPTH)" } else { "deep" }
    );
    if let Ok(near) = std::env::var("SKATE3_WATER_NEAR") {
        // Open water near x,z within r: a ray from 4 m above the water hits the
        // water first (nothing on top), with the depth of what lies under it.
        let v: Vec<f32> = near.split(',').filter_map(|s| s.trim().parse().ok()).collect();
        let (cx, cz, r) = (v[0], v[1], v[2]);
        let mut found = 0;
        for t in physics.world.triangles() {
            if !skate_core::physics::board_world::is_water_tag(t.tag) {
                continue;
            }
            let [a, b, c] = t.triangle.vertices;
            let p = skate_core::math::Vector3::new((a.x + b.x + c.x) / 3., (a.y + b.y + c.y) / 3., (a.z + b.z + c.z) / 3.);
            if (p.x - cx).hypot(p.z - cz) > r {
                continue;
            }
            let v3 = skate_core::math::Vector3::new;
            let top = physics.world.query_thin_line(v3(p.x, p.y + 4., p.z), v3(p.x, p.y - 0.01, p.z)).unwrap();
            let open = top.is_some_and(|h| skate_core::physics::board_world::is_water_tag(h.tag));
            let under = physics.world.query_thin_line(v3(p.x, p.y - 0.001, p.z), v3(p.x, p.y - 20., p.z)).unwrap();
            let depth = under.map_or(20., |h| p.y - h.geometry.position.y);
            if open {
                found += 1;
                eprintln!("open water at ({:.1}, {:.2}, {:.1}) depth {depth:.2} m", p.x, p.y, p.z);
            }
        }
        eprintln!("{found} open water triangles near ({cx}, {cz})");
    }
    if std::env::var_os("SKATE3_WATER_SCAN").is_some() {
        // Every water triangle: centroid and the depth of the first geometry under it.
        let mut rows = Vec::new();
        for t in physics.world.triangles() {
            if !skate_core::physics::board_world::is_water_tag(t.tag) {
                continue;
            }
            let [a, b, c] = t.triangle.vertices;
            let p = skate_core::math::Vector3::new((a.x + b.x + c.x) / 3., (a.y + b.y + c.y) / 3., (a.z + b.z + c.z) / 3.);
            let hit = physics
                .world
                .query_thin_line(skate_core::math::Vector3::new(p.x, p.y - 0.001, p.z), skate_core::math::Vector3::new(p.x, p.y - 20., p.z))
                .unwrap();
            let depth = hit.map_or(20., |h| p.y - h.geometry.position.y);
            rows.push((depth, p));
        }
        rows.sort_by(|a, b| b.0.total_cmp(&a.0));
        for (depth, p) in rows.iter().take(25) {
            eprintln!("deep water {depth:5.2} m at ({:.1}, {:.2}, {:.1})", p.x, p.y, p.z);
        }
    }
    if std::env::var_os("SKATE3_WATER_DEPTH_MAP").is_some() {
        // Water depth over the first floor under the surface ('.' no water).
        eprintln!("depth map, 1 m grid, x left->right {:.0}..{:.0}, z top->bottom", drop[0] - 8., drop[0] + 8.);
        for dz in -8..=8 {
            let mut row = String::new();
            for dx in -8..=8 {
                let (x, z) = (drop[0] + dx as f32, drop[2] + dz as f32);
                let p = skate_core::math::Vector3::new(x, drop[1] - 3., z);
                let cell = match physics.world.water_surface_at(p, 10., 6.) {
                    None => "   .".to_string(),
                    Some(h) => {
                        let hit = physics
                            .world
                            .query_thin_line(skate_core::math::Vector3::new(x, h - 0.001, z), skate_core::math::Vector3::new(x, h - 6., z))
                            .unwrap();
                        hit.map_or(" 6+ ".to_string(), |hit| format!("{:4.1}", h - hit.geometry.position.y))
                    }
                };
                row.push_str(&cell);
            }
            eprintln!("z {:7.1} {row}", drop[2] + dz as f32);
        }
    }
    // Jitter: mean and peak body-part speed once settled.
    let (mut speed_sum, mut speed_peak, mut speed_n) = (0.0f32, 0.0f32, 0u32);
    for tick in 0..520 {
        if tick == 30 {
            let mut transform = [[0.; 4]; 4];
            transform[0][0] = 1.;
            transform[1][1] = 1.;
            transform[2][2] = 1.;
            transform[3] = [drop[0], drop[1], drop[2], 1.];
            skater.travel_to(transform).unwrap();
        }
        input.sample_raw_for_test(skate_core::input::xbox::XboxState {
            buttons: 0,
            triggers: [0; 2],
            left: [0; 2],
            right: [0; 2],
        });
        let mut actions = input.player_actions();
        controls.update(
            &mut actions,
            physics.settings.step.simulation.time_step,
            physics.settings.input_magnitude_threshold,
            skater.player_input.physical.scoring.capabilities_204,
        );
        frame::advance(&mut physics, &mut skater, &mut controls, &graphs, &mut actions, true, &mut camera)
            .unwrap_or_else(|e| panic!("tick {tick}: {e}"));
        if (150..330).contains(&tick) && skater.player_state.current() == PhysicalStateId::WipeoutGround {
            for b in &skater.skeleton.bodies()[1..] {
                let v = b.rates.linear_velocity;
                let speed = (v.x * v.x + v.y * v.y + v.z * v.z).sqrt();
                speed_sum += speed;
                speed_peak = speed_peak.max(speed);
                speed_n += 1;
            }
        }
        if tick == 330 {
            eprintln!(
                "settled part speed (ticks 150..330, wipeout): mean {:.3} m/s, peak {:.3} m/s over {speed_n} samples",
                speed_sum / speed_n.max(1) as f32,
                speed_peak
            );
        }
        if tick < 28 || tick % 3 != 0 && tick > 120 {
            continue;
        }
        let parts = skater.skeleton.bodies();
        let ys: Vec<f32> = parts[1..].iter().map(|b| b.rates.position.y).collect();
        let (lo, hi) = ys.iter().fold((f32::MAX, f32::MIN), |(a, b), &y| (a.min(y), b.max(y)));
        let vy = parts[0].rates.linear_velocity.y;
        let deck = physics.board.bodies()[skate_core::physics::board::BodyId::Deck.index()].rates;
        let w = &skater.wipeout_state.state;
        let p = &skater.player_input.processed;
        let state = skater.player_state.current();
        eprintln!(
            "t{tick:3} {:<14} root y {:7.3} vy {:6.2} parts {:7.3}..{:7.3} deck y {:7.3} vy {:6.2} | water flag {} h {:6.2} | special {} below {} surf {:6.2} | cam {:?}",
            format!("{state:?}"),
            parts[0].rates.position.y,
            vy,
            lo,
            hi,
            deck.position.y,
            deck.linear_velocity.y,
            u8::from(p.flags_2488 & 0x4000_0000 != 0),
            p.collision_scalar_2924,
            u8::from(w.special_surface),
            u8::from(w.below_surface),
            w.surface_height,
            camera.frame.as_ref().map(|f| [f.position[0], f.position[1], f.position[2]]),
        );
        if state == PhysicalStateId::Teleporting && tick > 200 {
            eprintln!("respawn teleport at tick {tick}");
        }
        if tick == 300 {
            // What solid geometry lies under the board and the body?
            for (name, p) in [("deck", deck.position), ("root", parts[0].rates.position)] {
                let mut top = p.y + 0.5;
                for _ in 0..4 {
                    let hit = physics
                        .world
                        .query_thin_line(skate_core::math::Vector3::new(p.x, top, p.z), skate_core::math::Vector3::new(p.x, p.y - 6., p.z))
                        .unwrap();
                    let Some(hit) = hit else { break };
                    eprintln!(
                        "under {name} at ({:.2}, {:.2}): hit y {:.3} surface type {} normal y {:.2}",
                        p.x, p.z, hit.geometry.position.y, (hit.tag >> 7) & 31, hit.geometry.normal.y
                    );
                    top = hit.geometry.position.y - 0.01;
                }
            }
        }
    }
}
