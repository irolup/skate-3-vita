//! Parity oracle harness.
//!
//! The same file is compiled as a child of `physics` in two trees:
//! - skate-sim (feature `oracle`), from `crates/skate-sim/src/physics.rs`, where
//!   `tools/sim-oracle` calls [`run_dir`];
//! - a scratch copy of the reference skate-game (`cfg(test)`), injected by
//!   `tools/sim-oracle/reference.sh`, where the ignored test `oracle_dump` calls it.
//! As a child of `physics` it can call `frame::advance` the way the headless tests do.
//! It uses only code that is identical in both trees, so any dump difference comes
//! from the simulation.
//!
//! One oracle tick is skate-game's production fixed tick:
//! 1. PreUpdate `customiser::preferences`: `apply_preferences` with an empty profile
//!    (every key at its default);
//! 2. SimulationSet::Input: `publish_raw` (`input/oracle_input.rs`);
//! 3. SimulationSet::Controls: the body of `controls::sample`;
//! 4. SimulationSet::Physics: the body of the `advance` system (`frame::advance`).
//! Not reproduced: session markers (no LB+D-pad in the corpus), presentation capture,
//! HUD, mods and multiplayer (render or disabled paths).
//!
//! Input file (`.input`): optional `# difficulty=<easy|normal|hardcore>` header, then one
//! line per tick: `buttons lt rt lx ly rx ry` as XInput integers.
//!
//! Dump file (`.dump`), little-endian u32 words: magic `SKODUMP1`, version, board and
//! skeleton body counts, then one record per tick (see [`write_tick`]). Floats are
//! `f32::to_bits`. Names and maps are 64-bit FNV-1a hashes of their `Debug` text.
use super::{frame, GamePhysics, PlayerControls, SkaterRuntime};
use skate_core::input::xbox::XboxState;
use std::io::Write;
use std::path::Path;

pub const MAGIC: &[u8; 8] = b"SKODUMP1";

/// Diagnostic only: called after each tick's time is recorded (sim-bench takes a
/// per-tick stage profile here). Never part of a measured tick or of the dump.
pub static TICK_HOOK: std::sync::OnceLock<fn(usize)> = std::sync::OnceLock::new();
pub const VERSION: u32 = 1;
/// Words per body: position xyz, orientation xyzw, linear velocity xyz, angular velocity xyz.
pub const BODY_WORDS: usize = 13;

pub struct Inputs {
    pub difficulty: String,
    pub ticks: Vec<XboxState>,
}

pub fn read_inputs(path: &Path) -> Result<Inputs, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut difficulty = "easy".to_string();
    let mut ticks = Vec::new();
    for (number, line) in text.lines().enumerate() {
        let line = line.trim();
        if let Some(header) = line.strip_prefix('#') {
            if let Some(value) = header.trim().strip_prefix("difficulty=") {
                difficulty = value.trim().to_string();
            }
            continue;
        }
        if line.is_empty() {
            continue;
        }
        let v: Vec<i32> = line
            .split_whitespace()
            .map(|w| w.parse::<i32>())
            .collect::<Result<_, _>>()
            .map_err(|e| format!("{}:{}: {e}", path.display(), number + 1))?;
        if v.len() != 7 {
            return Err(format!("{}:{}: expected 7 values", path.display(), number + 1));
        }
        ticks.push(XboxState {
            buttons: v[0] as u16,
            triggers: [v[1] as u8, v[2] as u8],
            left: [v[3] as i16, v[4] as i16],
            right: [v[5] as i16, v[6] as i16],
        });
    }
    Ok(Inputs { difficulty, ticks })
}

pub struct RunSummary {
    pub ticks: usize,
    /// Wall time of each tick's simulation work (steps 1-4 below; dump writing
    /// excluded), in microseconds, for the benchmark.
    pub tick_us: Vec<u32>,
    /// The `frame::advance` part of each tick, in microseconds.
    pub advance_us: Vec<u32>,
    /// Production load steps and their wall time in seconds.
    pub load_s: Vec<(&'static str, f64)>,
    /// Tick and message of the first simulation error, after which the run stops.
    pub error: Option<(usize, String)>,
}

/// Runs one input sequence from a fresh production load and writes its dump.
pub fn run(asset_root: &Path, inputs: &Inputs, out: &mut dyn Write) -> Result<RunSummary, String> {
    let difficulty = crate::difficulty::Difficulty::parse(&inputs.difficulty)?;
    let mut load_s = Vec::new();
    let mut clock = std::time::Instant::now();
    let mut lap = |name: &'static str| {
        load_s.push((name, clock.elapsed().as_secs_f64()));
        clock = std::time::Instant::now();
    };
    // skate-game main.rs load order.
    let manifest = skate_data::GameAssets::load(asset_root).map_err(|e| format!("{e:?}"))?;
    lap("manifest");
    let graphs = crate::graph_runtime::StockGraphs::load(asset_root, &manifest)?;
    lap("graphs");
    let map = oracle_map()?;
    let mut physics = GamePhysics::load_with_difficulty(asset_root, map.as_ref(), difficulty)?;
    // ORACLE_ITERATIONS=N: the solve's constraint iterations (the Vita's "Fast" physics is
    // 25; retail 50). reference.sh sets the reference's constant to the same value.
    skate_vita_only! {
        if let Some(n) = std::env::var("ORACLE_ITERATIONS").ok().and_then(|v| v.parse().ok()) {
            physics.set_solver_iterations(n);
        }
    }
    drop(map); // skate-game drops the decoded map once the world is built
    lap("physics");
    let mut skater = SkaterRuntime::load(asset_root, &graphs, &physics, difficulty.key())?;
    lap("skater");
    let mut controls = PlayerControls::load(asset_root)?;
    lap("controls");
    let mut camera = crate::camera::CameraRuntime::load(asset_root)?;
    lap("camera");
    let mut input = crate::input::ControllerInput::default();
    let profile = serde_json::json!({});

    let io = |e: std::io::Error| e.to_string();
    write_header(out, &physics, &skater).map_err(io)?;

    // Diagnostic only (both trees): ORACLE_TRACE=1 prints physical state and motion
    // animation changes to stderr.
    let trace = std::env::var_os("ORACLE_TRACE").is_some();
    let mut last_state = String::new();
    let mut last_animation = String::new();
    let mut summary = RunSummary {
        ticks: 0,
        tick_us: Vec::with_capacity(inputs.ticks.len()),
        advance_us: Vec::with_capacity(inputs.ticks.len()),
        load_s,
        error: None,
    };
    // Diagnostic only: ORACLE_TRAP_TICK=N enables x86 floating-point invalid-operation
    // traps just before frame::advance on tick N (after input conversion, which makes
    // guarded NaNs every tick), so a debugger stops at the first NaN in the simulation.
    let trap_tick: Option<usize> = std::env::var("ORACLE_TRAP_TICK").ok().and_then(|v| v.parse().ok());
    for (tick, state) in inputs.ticks.iter().enumerate() {
        let tick_start = std::time::Instant::now();
        let mut advance_time = std::time::Duration::ZERO;
        // 1. PreUpdate customiser preferences.
        crate::customiser::apply_preferences(&profile, &mut physics, &mut skater.animation);
        // 2. Input.
        let raw = XboxState {
            buttons: state.buttons,
            triggers: state.triggers,
            left: state.left,
            right: state.right,
        };
        let published = crate::input::oracle_input::publish_raw(&mut input, raw);
        // 3. Controls: body of controls::sample.
        let mut map = published.0.actions();
        controls
            .update_for_physics(&mut map, &physics, &skater, &camera)
            .unwrap_or_else(|error| panic!("Offboard controller publication: {error}"));
        controls.publish_gestures(
            physics.animation_profile.physics_mode,
            skater.player_input.physical.state.state_16,
        );
        // 4. Physics: body of the advance system (vehicles are never occupied here).
        let mut result = Ok(());
        // Streamed collision follows the deck (a no-op for whole worlds). The
        // reference has no windows: there `skate_vita_only!` expands to nothing.
        skate_vita_only! {
            if let Err(e) = physics.follow_window(&skater) {
                physics.failed = true;
                result = Err(e);
            }
        }
        if !physics.failed {
            let mut actions = published.0.actions();
            let input_available = published.0.controller_available();
            if trap_tick == Some(tick) {
                trap_invalid();
            }
            let advance_start = std::time::Instant::now();
            result = frame::advance(
                &mut physics,
                &mut skater,
                &mut controls,
                &graphs,
                &mut actions,
                input_available,
                &mut camera,
            );
            advance_time = advance_start.elapsed();
            if result.is_err() {
                physics.failed = true;
            }
        }
        summary.tick_us.push(tick_start.elapsed().as_micros() as u32);
        summary.advance_us.push(advance_time.as_micros() as u32);
        if let Some(hook) = TICK_HOOK.get() {
            hook(tick);
        }
        write_tick(out, tick as u32, result.is_err(), &physics, &skater, &camera).map_err(io)?;
        if trace {
            let state = format!("{:?}", skater.player_state.current());
            let animation = skater.animation.motion.animation.current_name.clone().unwrap_or_default();
            if tick % 30 == 0 {
                let deck = &physics.board.bodies()[6].rates;
                eprintln!(
                    "TRACE {tick} deck=({:.2},{:.2},{:.2}) v=({:.2},{:.2},{:.2})",
                    deck.position.x, deck.position.y, deck.position.z,
                    deck.linear_velocity.x, deck.linear_velocity.y, deck.linear_velocity.z
                );
            }
            if state != last_state || animation != last_animation {
                eprintln!("TRACE {tick} {state} {animation}");
                last_state = state;
                last_animation = animation;
            }
        }
        summary.ticks = tick + 1;
        if let Err(message) = result {
            summary.error = Some((tick, message));
            break;
        }
    }
    Ok(summary)
}

/// Runs every `*.input` in `scenarios` and writes `<name>.dump` to `out_dir`.
/// Optional retail map for the runs: `ORACLE_MAP=path/to/Name.skate` loads that
/// map, validated as skate-game does, instead of the procedural Course.
pub fn oracle_map() -> Result<Option<skate_data::skate_map::SkateMap>, String> {
    let Some(path) = std::env::var_os("ORACLE_MAP") else { return Ok(None) };
    let mut map = skate_data::skate_map::SkateMap::load(Path::new(&path))?;
    crate::skate_world::validate_runtime(&map)?;
    // `ORACLE_SPAWN=x,y,z,heading` replaces the map's authored spawn (map data only).
    if let Ok(spawn) = std::env::var("ORACLE_SPAWN") {
        let v: Vec<f32> = spawn.split(',').map(|w| w.trim().parse::<f32>().map_err(|e| e.to_string())).collect::<Result<_, _>>()?;
        let [x, y, z, heading] = v[..] else { return Err("ORACLE_SPAWN needs x,y,z,heading".into()) };
        map.spawn = [x, y, z];
        map.heading = heading;
    }
    Ok(Some(map))
}

pub fn run_dir(asset_root: &Path, scenarios: &Path, out_dir: &Path) -> Result<Vec<(String, RunSummary)>, String> {
    std::fs::create_dir_all(out_dir).map_err(|e| e.to_string())?;
    let mut paths: Vec<_> = std::fs::read_dir(scenarios)
        .map_err(|e| format!("{}: {e}", scenarios.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "input"))
        .collect();
    paths.sort();
    let mut results = Vec::new();
    for path in paths {
        let name = path.file_stem().unwrap().to_string_lossy().to_string();
        let inputs = read_inputs(&path)?;
        let file = std::fs::File::create(out_dir.join(format!("{name}.dump")))
            .map_err(|e| e.to_string())?;
        let mut out = std::io::BufWriter::new(file);
        let summary = run(asset_root, &inputs, &mut out)?;
        out.flush().map_err(|e| e.to_string())?;
        eprintln!(
            "ORACLE {name}: {} ticks{}",
            summary.ticks,
            summary.error.as_ref().map(|(t, m)| format!(", error at tick {t}: {m}")).unwrap_or_default()
        );
        results.push((name, summary));
    }
    Ok(results)
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn trap_invalid() {
    unsafe extern "C" {
        fn feenableexcept(excepts: i32) -> i32;
    }
    const FE_INVALID: i32 = 1;
    unsafe { feenableexcept(FE_INVALID) };
}
#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
fn trap_invalid() {}

/// `check-assets`: the whole production load (skate-game main.rs order:
/// input config, manifest, graphs, physics, skater, controls, camera). Returns one
/// line per step with its time.
pub fn check_assets(asset_root: &Path) -> Result<Vec<String>, String> {
    let mut lines = Vec::new();
    let mut step = |name: &str, f: &mut dyn FnMut() -> Result<String, String>| -> Result<(), String> {
        let start = std::time::Instant::now();
        let detail = f().map_err(|e| format!("{name}: {e}"))?;
        lines.push(format!("ok  {name:<28} {:>7.1} ms  {detail}", start.elapsed().as_secs_f64() * 1e3));
        Ok(())
    };
    step("input.cfg", &mut || {
        skate_data::input_config::StockGameplayConfig::load(asset_root)
            .map(|_| String::new())
            .map_err(|e| format!("{e:?}"))
    })?;
    let manifest = skate_data::GameAssets::load(asset_root).map_err(|e| format!("manifest: {e:?}"))?;
    let mut graphs = None;
    step("stock graphs", &mut || {
        graphs = Some(crate::graph_runtime::StockGraphs::load(asset_root, &manifest)?);
        Ok(String::new())
    })?;
    let graphs = graphs.unwrap();
    for difficulty in crate::difficulty::Difficulty::ALL {
        let mut physics = None;
        step(&format!("physics ({})", difficulty.key()), &mut || {
            physics = Some(GamePhysics::load_with_difficulty(asset_root, None, difficulty)?);
            Ok(String::new())
        })?;
        let physics = physics.unwrap();
        step(&format!("skater ({})", difficulty.key()), &mut || {
            let skater = SkaterRuntime::load(asset_root, &graphs, &physics, difficulty.key())?;
            Ok(format!("{} skeleton bodies", skater.skeleton.bodies().len()))
        })?;
    }
    step("controls (PAT gestures)", &mut || PlayerControls::load(asset_root).map(|_| String::new()))?;
    step("camera graph", &mut || crate::camera::CameraRuntime::load(asset_root).map(|_| String::new()))?;
    Ok(lines)
}

/// `check-assets` for one `.skate` map: parse, the runtime validation,
/// the collision world (retail RWCM or authored triangles) and the rail provider.
pub fn check_map(asset_root: &Path, path: &Path) -> Result<String, String> {
    let start = std::time::Instant::now();
    let map = skate_data::skate_map::SkateMap::load(path)?;
    let parsed = start.elapsed();
    crate::skate_world::validate_runtime(&map)?;
    let data = skate_data::collections::Collections::load(asset_root)?;
    let settings = super::settings::PhysicsSettings::load(&data)?;
    let world = crate::skate_world::collision_world(&map, settings.floor_material)?;
    let rails = crate::grind_world::StaticProvider::new(Some(&map))?;
    Ok(format!(
        "{} collision triangles, {} rail splines, {} rail primitives; parse {:.0} ms, total {:.0} ms",
        world.triangles().len(),
        map.rails.len(),
        rails.primitives().len(),
        parsed.as_secs_f64() * 1e3,
        start.elapsed().as_secs_f64() * 1e3
    ))
}

fn fnv(text: &str) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in text.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

/// One tick record, in this order (all u32):
/// tick, failed, physical state id,
/// action graph: current state (u32::MAX = none), last state, active count, then
///   (behaviour, instance) pairs; the same for the motion graph,
/// action intents hash (2 words), motion intents hash (2 words),
/// grind: family (u32::MAX = none), active, name hash (2 words), output word, distance,
/// camera: present, position xyz,
/// board bodies × 13 words, skeleton bodies × 13 words.
/// Dump header: magic, version, board and skeleton body counts.
pub(crate) fn write_header(
    out: &mut dyn Write,
    physics: &GamePhysics,
    skater: &SkaterRuntime,
) -> std::io::Result<()> {
    out.write_all(MAGIC)?;
    for word in [
        VERSION,
        physics.board.bodies().len() as u32,
        skater.skeleton.bodies().len() as u32,
    ] {
        out.write_all(&word.to_le_bytes())?;
    }
    Ok(())
}

pub(crate) fn write_tick(
    out: &mut dyn Write,
    tick: u32,
    failed: bool,
    physics: &GamePhysics,
    skater: &SkaterRuntime,
    camera: &crate::camera::CameraRuntime,
) -> std::io::Result<()> {
    let mut w: Vec<u32> = Vec::with_capacity(512);
    let hash = |w: &mut Vec<u32>, h: u64| {
        w.push(h as u32);
        w.push((h >> 32) as u32);
    };
    w.push(tick);
    w.push(u32::from(failed));
    w.push(skater.player_state.current() as u32);
    for controller in [
        &skater.animation.action_controller,
        &skater.animation.motion_controller,
    ] {
        w.push(controller.frame.current.map_or(u32::MAX, |s| s as u32));
        w.push(controller.frame.last.map_or(u32::MAX, |s| s as u32));
        w.push(controller.active.len() as u32);
        for active in &controller.active {
            w.push(active.behavior as u32);
            w.push(active.instance);
        }
    }
    hash(&mut w, fnv(&format!("{:?}", skater.animation.action.action_intents)));
    hash(&mut w, fnv(&format!("{:?}", skater.animation.action.motion_intents)));
    // The grind publication (b3c9679: `active_name` replaced `active_family` and
    // `mod_grind`): the active family, the raw animation chromosome, word 136, distance.
    let grinds = &skater.player_input.physical.grinds;
    hash(&mut w, fnv(skater.grind.active_name().unwrap_or("")));
    w.extend(grinds.animation_chromosome_268);
    w.push(grinds.words_136_140[0]);
    w.push(skater.grind.last_grind_distance().to_bits());
    match camera.frame {
        Some(frame) => {
            w.push(1);
            w.extend(frame.position[..3].iter().map(|v| v.to_bits()));
        }
        None => w.extend([0, 0, 0, 0]),
    }
    for body in physics.board.bodies().iter().chain(skater.skeleton.bodies()) {
        let r = &body.rates;
        w.extend(
            [
                r.position.x, r.position.y, r.position.z,
                r.orientation.x, r.orientation.y, r.orientation.z, r.orientation.w,
                r.linear_velocity.x, r.linear_velocity.y, r.linear_velocity.z,
                r.angular_velocity.x, r.angular_velocity.y, r.angular_velocity.z,
            ]
            .map(f32::to_bits),
        );
    }
    let bytes: Vec<u8> = w.iter().flat_map(|v| v.to_le_bytes()).collect();
    out.write_all(&bytes)
}

/// Reference entry point (scratch skate-game copy). Environment: ORACLE_ASSETS,
/// ORACLE_SCENARIOS, ORACLE_OUT.
#[cfg(test)]
#[test]
#[ignore = "parity oracle: run through tools/sim-oracle/reference.sh"]
fn oracle_dump() {
    let var = |k: &str| std::path::PathBuf::from(std::env::var_os(k).unwrap_or_else(|| panic!("set {k}")));
    let results = run_dir(&var("ORACLE_ASSETS"), &var("ORACLE_SCENARIOS"), &var("ORACLE_OUT")).unwrap();
    assert!(!results.is_empty(), "no .input files");
}
