//! sim-oracle: parity oracle for skate-sim (PLAN.md §5.4). Desktop only.
//!
//!   sim-oracle scenarios OUT_DIR              write the scripted corpus as .input files
//!   sim-oracle run ASSETS SCENARIOS OUT_DIR   run the candidate (this build) on the corpus
//!   sim-oracle compare REF_DIR CAND_DIR [--exact | --tolerance CM]
//!   sim-oracle states DUMP_DIR                physical-state coverage per scenario
//!   sim-oracle state-stages DUMP_DIR          mean stage time per physical state, from the `.stages` files
//!   sim-oracle trace DUMP N                   the last N ticks of a dump: state, board and body motion
//!   sim-oracle degenerate PACK...             degenerate retail collision triangles in map packs
//!   sim-oracle hud-trace ASSETS SCENARIOS OUT the scenarios through the live session and the original HUD movie: every text field when it changes
//!   sim-oracle ui-probe MOVIE.json [FRAMES]   run a front-end screen movie with a recording host: every native call it makes
//!   sim-oracle ui-render MOVIE.json ROOT OUT.png [FRAMES]   a linked screen drawn on the desktop (software rasterizer, the Vita shader's maths); MODEL_CALLS runs entry points first
//!   sim-oracle ui-crossbar LINKED.json ROOT OUT.png EVENTS   the crossbar with a sample menu model: EVENTS (Left,Right,Up,Down,Accept,Back, comma separated) are applied with 30 frames after each; prints the outcomes
//!   sim-oracle ui-analyzer LINKED.json ROOT OUT.png TRICK_ID   the trick analyzer showing the gesture of a trick (e.g. ID_TRICK_FLIP_KICKFLIP); ANALYZER_SWITCH=1 in switch
//!   sim-oracle ui-meter LINKED.json ROOT OUT.png VALUES   the manual meter fed VALUES (tilt fractions -1..1, comma separated, 20 frames each); the last is drawn
//!   sim-oracle state-times DUMP_DIR           tick time per physical state, from the `.ticks` files
//!   sim-oracle check-assets ASSETS [MAPS]     load every private asset (PLAN.md §0.5)
//!   sim-oracle audio-render ASSETS SOUND_PAK SCENARIO.input OUT.wav   a scenario through the live session and the
//!                                             reference's audio host (sound.pak), rendered to 48 kHz stereo (retail-audio S1)
//!
//! The candidate's arithmetic follows how this binary was built: default = fused
//! multiply-add like the reference, `--features no-fma` = the Vita arithmetic.
//! The reference dumps come from tools/sim-oracle/reference.sh.
mod dump;
#[cfg(feature = "heap-probe")]
mod heap_probe;
mod scenarios;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // ORACLE_WINDOW=HALF,TRIGGER: streamed collision (M10 stage 1; metres), the world holds the
    // clusters within HALF m of the deck and is rebuilt once it is TRIGGER m from the centre.
    if let Ok(v) = std::env::var("ORACLE_WINDOW") {
        let mut it = v.split(',').filter_map(|x| x.trim().parse::<f32>().ok());
        if let (Some(half), Some(trigger)) = (it.next(), it.next()) {
            skate_sim::collision_window::configure(half, trigger);
            // ORACLE_WINDOW_BG=1: build the windows on a thread, as the device does.
            if std::env::var_os("ORACLE_WINDOW_BG").is_some() {
                skate_sim::collision_window::configure_background(None);
            }
        }
    }
    let result = match args.first().map(String::as_str) {
        Some("scenarios") if args.len() == 2 => write_scenarios(Path::new(&args[1])),
        Some("run") if args.len() == 4 => run(&args[1], &args[2], &args[3]),
        Some("run-live") if args.len() == 4 => run_live(&args[1], &args[2], &args[3]),
        Some("run-live") if args.len() == 5 && args[1] == "--packs" => {
            mount_packs(Path::new(&args[2])).and_then(|root| run_live(&root, &args[3], &args[4]))
        }
        Some("run-live") if args.len() == 7 && args[1] == "--packs" && args[3] == "--map" => {
            // A district pack (district_<name>.pak, M10 stage 3) wins over a map pack.
            let district = Path::new(&args[2]).join(format!("district_{}.pak", args[4]));
            let pack = if district.is_file() { district } else { Path::new(&args[2]).join(format!("map_{}.pak", args[4])) };
            MAP_PACK.set(pack).unwrap();
            mount_packs(Path::new(&args[2])).and_then(|root| run_live(&root, &args[5], &args[6]))
        }
        #[cfg(feature = "heap-probe")]
        Some("heap-probe-ui") if args.len() == 3 && args[1] == "--packs" => heap_probe::run_ui(Path::new(&args[2])),
        #[cfg(feature = "heap-probe")]
        Some("heap-probe") if args.len() == 6 && args[1] == "--packs" && args[3] == "--map" => {
            args[5].parse().map_err(|_| "heap-probe: TICKS must be a number".to_owned()).and_then(|n| heap_probe::run(Path::new(&args[2]), &args[4], n))
        }
        Some("audio-render") if args.len() == 5 => audio_render(&args[1], &args[2], &args[3], &args[4]),
        Some("state-times") if args.len() == 2 => state_times(Path::new(&args[1])),
        Some("state-stages") if args.len() == 2 => state_stages(Path::new(&args[1])),
        Some("trace") if args.len() == 3 => {
            args[2].parse().map_err(|_| "trace: N must be a number".to_owned()).and_then(|n| trace(Path::new(&args[1]), n))
        }
        Some("degenerate") if args.len() >= 2 => degenerate(&args[1..]),
        Some("hud-trace") if args.len() == 4 => hud_trace(&args[1], &args[2], &args[3]),
        Some("ui-probe") if args.len() >= 2 => ui_probe(&args[1], args.get(2).and_then(|n| n.parse().ok()).unwrap_or(120)),
        Some("ui-render") if args.len() >= 4 => ui_render(&args[1], &args[2], &args[3], args.get(4).and_then(|n| n.parse().ok()).unwrap_or(60)),
        Some("ui-analyzer") if args.len() >= 5 => ui_analyzer(&args[1], &args[2], &args[3], &args[4]),
        Some("ui-meter") if args.len() >= 5 => ui_meter(&args[1], &args[2], &args[3], &args[4]),
        Some("ui-replay-help") if args.len() >= 5 => ui_replay_help(&args[1], &args[2], &args[3], &args[4]),
        Some("ui-popup") if args.len() >= 4 => ui_popup(&args[1], &args[2], &args[3]),
        Some("ui-replay") if args.len() >= 5 => ui_replay(&args[1], &args[2], &args[3], &args[4]),
        Some("ui-crossbar") if args.len() >= 5 => ui_crossbar(&args[1], &args[2], &args[3], &args[4]),
        Some("compare") if args.len() >= 3 => compare(&args[1], &args[2], &args[3..]),
        Some("states") if args.len() == 2 => states(Path::new(&args[1])),
        Some("check-assets") if (2..=3).contains(&args.len()) => {
            check_assets(Path::new(&args[1]), args.get(2).map(PathBuf::from))
        }
        _ => Err("usage: sim-oracle scenarios OUT | run ASSETS SCENARIOS OUT | \
                  run-live ASSETS SCENARIOS OUT | run-live --packs DIR [--map NAME] SCENARIOS OUT | \
                  compare REF CAND [--exact | --tolerance CM] | states DUMPS | \
                  check-assets ASSETS [MAPS]"
            .into()),
    };
    match result {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(e) => {
            eprintln!("sim-oracle: {e}");
            ExitCode::from(2)
        }
    }
}

fn write_scenarios(out: &Path) -> Result<bool, String> {
    std::fs::create_dir_all(out).map_err(|e| e.to_string())?;
    for s in scenarios::all() {
        let path = out.join(format!("{}.input", s.name));
        std::fs::write(&path, scenarios::write_input(&s)).map_err(|e| e.to_string())?;
        println!("{}", path.display());
    }
    Ok(true)
}

fn run(assets: &str, scenarios: &str, out: &str) -> Result<bool, String> {
    let fma = if cfg!(feature = "no-fma") { "no-fma" } else { "fma" };
    eprintln!("candidate arithmetic: {fma}");
    let results = skate_sim::oracle_harness::run_dir(Path::new(assets), Path::new(scenarios), Path::new(out))?;
    Ok(results.iter().all(|(_, s)| s.error.is_none()))
}

/// `run-live --map NAME`: the map pack whose sim part the sessions load.
static MAP_PACK: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

/// Mounts DIR/sim.pak and DIR/anim.pak (vita-bake) as the loaders' files, strictly (no
/// disk fallback), and returns the virtual asset root to load from (M6).
fn mount_packs(dir: &Path) -> Result<String, String> {
    let root = "vfs:/assets";
    for name in ["sim.pak", "anim.pak"] {
        let bytes = std::fs::read(dir.join(name)).map_err(|e| format!("{}: {e}", dir.join(name).display()))?;
        for (path, data) in vita_pack::files::from_pack(&bytes)? {
            skate_data::vfs::mount(Path::new(root).join(path), data);
        }
    }
    // L1b: the movable props, when baked (props.pak).
    if let Ok(bytes) = std::fs::read(dir.join("props.pak")) {
        for (path, data) in vita_pack::files::from_pack(&bytes)? {
            skate_data::vfs::mount(Path::new(root).join(path), data);
        }
        eprintln!("mounted {}/props.pak", dir.display());
    }
    skate_data::vfs::set_strict(true);
    eprintln!("mounted {}/sim.pak + anim.pak at {root} (strict)", dir.display());
    Ok(root.into())
}

/// The corpus through the game loop's `skate_sim::live::Session` (M4). Its dumps must
/// equal `run`'s: the live tick is the oracle tick.
fn run_live(assets: &str, scenarios: &str, out: &str) -> Result<bool, String> {
    let io = |e: std::io::Error| e.to_string();
    std::fs::create_dir_all(out).map_err(io)?;
    let mut inputs: Vec<PathBuf> = std::fs::read_dir(scenarios)
        .map_err(|e| format!("{scenarios}: {e}"))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "input"))
        .collect();
    inputs.sort();
    // ORACLE_ANIM_BUDGET=BYTES: the decoded-animation bound (M7); 0 releases every
    // idle clip's keys each second, so the corpus re-decodes them all the time.
    let budget = match std::env::var("ORACLE_ANIM_BUDGET") {
        Ok(v) => Some(v.parse::<usize>().map_err(|e| format!("ORACLE_ANIM_BUDGET: {e}"))?),
        Err(_) => None,
    };
    let mut clean = true;
    for path in inputs {
        let name = path.file_stem().unwrap().to_string_lossy().to_string();
        let scenario = skate_sim::oracle_harness::read_inputs(&path)?;
        // The park's sim part from its map pack (M6), or ORACLE_MAP's .skate, or the Course.
        let map = match MAP_PACK.get() {
            Some(pack) if pack.file_name().is_some_and(|n| n.to_string_lossy().starts_with("district_")) => {
                // The collision is streamed from the pack by offset.
                let (district, file) = vita_pack::district::open(pack)?;
                skate_sim::collision_window::install_district(skate_sim::collision_window::DistrictCollision {
                    clusters: district.clusters.clone(),
                    source: std::sync::Arc::new(file),
                    limits: district.limits,
                });
                Some(district.sim)
            }
            Some(pack) => {
                let bytes = std::fs::read(pack).map_err(|e| format!("{}: {e}", pack.display()))?;
                Some(vita_pack::map::Map::from_pack(&bytes)?.sim_map())
            }
            None => skate_sim::oracle_harness::oracle_map()?,
        };
        let mut session = skate_sim::live::Session::load_map(Path::new(assets), &scenario.difficulty, map)?;
        if let Some(n) = std::env::var("ORACLE_ITERATIONS").ok().and_then(|v| v.parse().ok()) {
            session.set_solver_iterations(n);
        }
        if let Some(budget) = budget {
            session.set_animation_budget(budget);
        }
        let mut animation_peak = 0;
        let file = std::fs::File::create(Path::new(out).join(format!("{name}.dump"))).map_err(io)?;
        let mut w = std::io::BufWriter::new(file);
        session.write_header(&mut w).map_err(io)?;
        let mut error = None;
        let mut tick_us: Vec<u32> = Vec::new();
        for state in scenario.ticks {
            let started = std::time::Instant::now();
            let report = session.step(state);
            tick_us.push(started.elapsed().as_micros() as u32);
            if let Some(trick) = &report.new_trick {
                eprintln!("{name} tick {}: trick {trick:?}", session.ticks() - 1);
            }
            session.write_tick(&mut w, report.error.is_some()).map_err(io)?;
            animation_peak = animation_peak.max(session.animation_bytes());
            if report.error.is_some() {
                error = report.error;
                break;
            }
        }
        std::io::Write::flush(&mut w).map_err(io)?;
        if let Some((props, awake)) = session.prop_counts() {
            eprintln!("{name}: props {props}, awake at the end {awake}");
        }
        // ORACLE_TICKS=1: the per-tick time (u32 LE microseconds) next to the dump, as the device writes.
        if std::env::var_os("ORACLE_TICKS").is_some() {
            let bytes: Vec<u8> = tick_us.iter().flat_map(|v| v.to_le_bytes()).collect();
            std::fs::write(Path::new(out).join(format!("{name}.ticks")), bytes).map_err(io)?;
        }
        println!(
            "{name}: {} ticks, decoded animation peak {:.1} MiB{}",
            session.ticks(),
            animation_peak as f64 / 1048576.0,
            error.map(|e| format!(", error: {e}")).unwrap_or_default()
        );
        clean &= session.error().is_none();
    }
    Ok(clean)
}

fn dumps(dir: &Path) -> Result<Vec<(String, PathBuf)>, String> {
    let mut v: Vec<_> = std::fs::read_dir(dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "dump"))
        .map(|p| (p.file_stem().unwrap().to_string_lossy().to_string(), p))
        .collect();
    v.sort();
    Ok(v)
}

fn compare(reference: &str, candidate: &str, options: &[String]) -> Result<bool, String> {
    let tolerance_cm = match options {
        [] => None,
        [o] if o == "--exact" => None,
        [o, cm] if o == "--tolerance" => Some(cm.parse::<f32>().map_err(|e| e.to_string())?),
        _ => return Err("compare options: --exact | --tolerance CM".into()),
    };
    let mut all_ok = true;
    let refs = dumps(Path::new(reference))?;
    if refs.is_empty() {
        return Err(format!("no .dump files in {reference}"));
    }
    println!(
        "{:<28} {:>6} {:>6} {:>10} {:>10} {:>12} {:>16}  result",
        "scenario", "ticks", "cand", "first-bit", "first-seq", "err@600 cm", "max err cm (tick)"
    );
    for (name, path) in refs {
        let cand_path = Path::new(candidate).join(format!("{name}.dump"));
        let a = dump::read(&path)?;
        let b = dump::read(&cand_path)?;
        let c = dump::compare(&a, &b)?;
        let fmt = |v: Option<u32>| v.map_or("-".to_string(), |t| t.to_string());
        let ok = match tolerance_cm {
            None => c.first_raw_difference.is_none() && c.ticks.0 == c.ticks.1,
            // PLAN.md §5.4: same physical state and graph node sequence, and board error
            // below the tolerance, at tick 600. Later divergence is reported, not gated.
            Some(cm) => {
                c.discrete_difference_by_600.is_none()
                    && c.error_at_600.is_some_and(|e| e * 100.0 < cm)
            }
        };
        all_ok &= ok;
        println!(
            "{:<28} {:>6} {:>6} {:>10} {:>10} {:>12} {:>9.4} ({:>4})  {}",
            name,
            c.ticks.0,
            c.ticks.1,
            fmt(c.first_raw_difference),
            fmt(c.first_discrete_difference),
            c.error_at_600.map_or("-".into(), |e| format!("{:.4}", e * 100.0)),
            c.max_error * 100.0,
            c.max_error_tick,
            if ok { "PASS" } else { "FAIL" }
        );
    }
    println!(
        "mode: {}",
        tolerance_cm.map_or("exact (every dumped word bit-identical; any two NaNs count as equal)".to_string(), |cm| format!(
            "tolerance (same state + graph sequence through tick 600, board error at tick 600 < {cm} cm; first-seq shows later divergence)"
        ))
    );
    Ok(all_ok)
}

fn states(dir: &Path) -> Result<bool, String> {
    for (name, path) in dumps(dir)? {
        let d = dump::read(&path)?;
        let runs = dump::state_runs(&d);
        let mut distinct: Vec<u32> = runs.iter().map(|r| r.0).collect();
        distinct.sort();
        distinct.dedup();
        println!("{name}: {} ticks, states {:?}, transitions {}", d.ticks.len(), distinct, runs.len() - 1);
    }
    Ok(true)
}

/// The scenarios through `live::Session` with the original HUD movie fed every tick
/// as the game does (M9): writes `NAME.hud`, one line per change of a text field
/// (`tick path text`), and prints the draw-batch counts. The movie is
/// `ASSETS/private/hud/runtime/trickdisplay.json`.
fn hud_trace(assets: &str, scenarios: &str, out: &str) -> Result<bool, String> {
    use skate_ui::hud_runtime::{Input, Runtime};
    let io = |e: std::io::Error| e.to_string();
    std::fs::create_dir_all(out).map_err(io)?;
    let json: serde_json::Value = serde_json::from_slice(
        &std::fs::read(Path::new(assets).join("private/hud/runtime/trickdisplay.json")).map_err(io)?,
    )
    .map_err(|e| e.to_string())?;
    let shapes: skate_ui::apt_scene::Shapes = serde_json::from_value(json["shapes"].clone()).map_err(|e| e.to_string())?;
    let to_input = |f: &skate_sim::live::HudFeed| Input {
        sequence_score: f.sequence_score,
        line_score: f.line_score,
        sequence_timer: f.sequence_timer,
        line_time: f.line_time,
        line_capacity: f.line_capacity,
        multiplier: f.multiplier,
        clean: f.clean,
        sketchy: f.sketchy,
        stance: f.stance,
        trick_name: f.trick_name.clone(),
        trick_metrics: [
            skate_ui::apt_vm::Value::Text(f.trick_name.clone()),
            skate_ui::apt_vm::Value::Bool(false),
            skate_ui::apt_vm::Value::Bool(false),
            skate_ui::apt_vm::Value::Bool(false),
            skate_ui::apt_vm::Value::Bool(false),
        ],
        context_tricks: Vec::new(),
    };
    let mut inputs: Vec<PathBuf> = std::fs::read_dir(scenarios)
        .map_err(|e| format!("{scenarios}: {e}"))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "input"))
        .collect();
    inputs.sort();
    for path in inputs {
        let name = path.file_stem().unwrap().to_string_lossy().to_string();
        let scenario = skate_sim::oracle_harness::read_inputs(&path)?;
        let mut session = skate_sim::live::Session::load(Path::new(assets), &scenario.difficulty)?;
        if let Some(n) = std::env::var("ORACLE_ITERATIONS").ok().and_then(|v| v.parse().ok()) {
            session.set_solver_iterations(n);
        }
        let mut runtime = Runtime::load(&json, to_input(&session.hud_feed()))?;
        let mut log = String::new();
        let mut last: std::collections::BTreeMap<String, String> = Default::default();
        let (mut max_draws, mut pulses) = (0usize, 0u32);
        for (tick, state) in scenario.ticks.into_iter().enumerate() {
            let report = session.step(state);
            let feed = session.hud_feed();
            pulses += u32::from(feed.new_trick || feed.modified_trick || feed.close_tricks);
            runtime.update(to_input(&feed), feed.new_trick, feed.modified_trick, feed.close_tricks)?;
            let draws = skate_ui::apt_scene::draw(&runtime.bindings.movie, &runtime.vm, &shapes)?;
            max_draws = max_draws.max(draws.len());
            for (&id, instance) in &runtime.bindings.movie.instances {
                let character = &runtime.bindings.movie.characters[&instance.character];
                if character.text.is_none() {
                    continue;
                }
                let text = runtime.vm.get(id, "_displayText").text();
                let key = format!("{}#{id}", instance.placement.as_ref().map_or("?", |p| p.name.as_str()));
                if last.get(&key) != Some(&text) {
                    log += &format!("{tick} {key} {text:?}\n");
                    last.insert(key, text);
                }
            }
            if report.error.is_some() {
                break;
            }
        }
        std::fs::write(Path::new(out).join(format!("{name}.hud")), &log).map_err(io)?;
        println!("{name}: {} text changes, {pulses} pulse ticks, at most {max_draws} draw batches", log.lines().count());
    }
    Ok(true)
}

/// A linked screen rendered to a PNG on the desktop (M9): the movie runs in
/// `menu_runtime` with the probe's fake data model (`MODEL_CALLS` as in `ui-probe`), its
/// draw batches are rasterized at 960 x 544 with the Vita shader's maths (stage scaled by
/// 0.75, sRGB textures decoded, `clamp(tex * multiply + add)`, encoded back, alpha over a
/// dark background) so the look can be checked without a device. Textures are read from
/// ROOT (the prepare output folder) by the paths the movie names.
fn ui_render(path: &str, root: &str, out: &str, frames: usize) -> Result<bool, String> {
    use skate_ui::apt_scene::{self, Shapes};
    use skate_ui::apt_vm::{Value, Vm};
    use skate_ui::menu_runtime::{MenuRuntime, Natives};
    use std::collections::HashMap;

    // STATE=menu,option,suboption: the model's current indices; TITLE: the mode title.
    struct Model {
        state: [f64; 3],
        level: f64,
        title: String,
    }
    impl Natives for Model {
        fn constants(&self) -> Vec<(String, Value)> {
            // Key codes and the crossbar's native states (any distinct numbers).
            [("AptUp", 1.0), ("AptDown", 2.0), ("AptPrevious", 3.0), ("FECrossBarScreenMode_RefreshMenu", 10.0), ("FECrossBarScreenMode_RefreshOption", 11.0), ("FECrossBarScreenMode_InitialiseMenu", 12.0), ("FECrossBarScreenMode_InitialiseSubOption", 13.0), ("FECrossBarScreenMode_InitSubOptionOnlineTeam", 14.0), ("FECrossBarScreenMode_RefreshDLCBanner", 15.0)]
                .iter()
                .map(|(k, v)| ((*k).to_owned(), Value::Number(*v)))
                .collect()
        }
        fn names(&self) -> Vec<String> {
            ["coremenu", "FELanguage", "Game", "ScreenManager"].iter().map(|s| s.to_string()).collect()
        }
        fn call(&mut self, _vm: &mut Vm, native: &str, method: &str, args: &[Value]) -> Result<Value, String> {
            let n = |i: usize| args.get(i).map_or(0.0, Value::number);
            if std::env::var_os("TRACE").is_some() {
                eprintln!("  host call {native}.{method}({})", args.iter().map(|a| format!("{a:?}")).collect::<Vec<_>>().join(", "));
            }
            Ok(match (native, method) {
                ("coremenu", "GetIsDemo" | "IsDLCEnabled") => Value::Bool(false),
                ("coremenu", "GetNumMenus") => Value::Number(3.0),
                ("coremenu", "GetNumOptions") => Value::Number(4.0),
                // Option 1 has SUB sub-options; the menu level is LEVEL (1 option, 2 sub-option).
                ("coremenu", "GetNumSubOptions") => Value::Number(if n(1) == 1.0 { std::env::var("SUB").ok().and_then(|v| v.parse().ok()).unwrap_or(0.0) } else { 0.0 }),
                ("coremenu", "GetMaxSubOptions") => Value::Number(std::env::var("SUB").ok().and_then(|v| v.parse().ok()).unwrap_or(0.0)),
                ("coremenu", "GetCurrentMenuLevel") => Value::Number(self.level),
                ("coremenu", "GetCurrentMenuIndex") => Value::Number(self.state[0]),
                ("coremenu", "GetCurrentOptionIndex") => Value::Number(self.state[1]),
                ("coremenu", "GetCurrentSubOptionIndex") => Value::Number(self.state[2]),
                ("coremenu", "IsItemEnabled") | ("Game", "OnMenuGetItemEnabled") => Value::Bool(true),
                ("coremenu", "GetMenuName") => Value::Text(["SKATE", "LEARN", "OPTIONS"][n(0) as usize % 3].into()),
                ("coremenu", "GetOptionName") => Value::Text(["Skate", "Park", "Difficulty", "Respawn"][n(1) as usize % 4].into()),
                ("coremenu", "GetSubOptionName") => Value::Text(["Course", "StartPark", "MegaPark", "BlackBoxPark", "Industrial", "DownTown", "Maloof"][n(args.len().saturating_sub(1)) as usize % 7].into()),
                ("coremenu", "GetMenuIconName") => Value::Text(["singleplayer", "learn", "options"][n(0) as usize % 3].into()),
                ("coremenu", "GetOptionIconName") => Value::Text(["freeskate", "skatepark", "settings", "restart"][n(1) as usize % 4].into()),
                ("coremenu", "GetCurrentModeTitle") => Value::Text(self.title.clone()),
                ("coremenu", "GetOptionHelperDescription") => Value::Text("Start skating in the selected park.".into()),
                _ => Value::Undefined,
            })
        }
    }
    let json: serde_json::Value = serde_json::from_slice(&std::fs::read(path).map_err(|e| format!("{path}: {e}"))?).map_err(|e| e.to_string())?;
    let shapes: Shapes = serde_json::from_value(json["shapes"].clone()).map_err(|e| e.to_string())?;
    let st: Vec<f64> = std::env::var("STATE").unwrap_or_default().split(',').filter_map(|x| x.parse().ok()).collect();
    let state = [st.first().copied().unwrap_or(0.0), st.get(1).copied().unwrap_or(0.0), st.get(2).copied().unwrap_or(0.0)];
    let mut runtime = MenuRuntime::load(&json, Model { state, level: std::env::var("LEVEL").ok().and_then(|v| v.parse().ok()).unwrap_or(1.0), title: std::env::var("TITLE").unwrap_or_else(|_| "SKATE VITA".into()) })?;
    // HIDE=name,name: clips of the root to hide (authored placeholder panels).
    for name in std::env::var("HIDE").unwrap_or_default().split(',').filter(|n| !n.is_empty()) {
        if let Value::Object(clip) = runtime.vm.get(runtime.bindings.movie.root, name) {
            runtime.vm.set(clip, "_visible", Value::Bool(false))?;
        }
    }
    for call in std::env::var("MODEL_CALLS").unwrap_or_default().split(';').filter(|c| !c.is_empty()) {
        let (name, arg) = call.split_once('(').map_or((call, None), |(n, a)| (n, Some(a.trim_end_matches(')'))));
        let args: Vec<Value> = arg.map(|a| a.split(',').filter(|x| !x.is_empty()).map(|x| x.parse::<f64>().map_or_else(|_| match x { "AptUp" => Value::Number(1.0), "AptDown" => Value::Number(2.0), "AptPrevious" => Value::Number(3.0), _ => Value::Text(x.into()) }, Value::Number)).collect()).unwrap_or_default();
        // `level=N`, `menu=N`, `option=N`, `sub=N` change the fake model between calls.
        if let Some((key, value)) = call.split_once('=') {
            let v: f64 = value.parse().map_err(|_| format!("{call}: not a number"))?;
            let model = &mut runtime.bindings.natives;
            match key {
                "level" => model.level = v,
                "menu" => model.state[0] = v,
                "option" => model.state[1] = v,
                "sub" => model.state[2] = v,
                _ => return Err(format!("{call}: unknown model field")),
            }
            continue;
        }
        // `a.b.method(args)` calls a method of the clip at that path from the root.
        if let Some((path, method)) = name.rsplit_once('.') {
            let mut object = runtime.bindings.movie.root;
            for part in path.split('.') {
                match runtime.vm.get(object, part) {
                    Value::Object(next) => object = next,
                    other => return Err(format!("{call}: {part} is {other:?}")),
                }
            }
            runtime.vm.begin_update();
            runtime.vm.call_method(object, method, args, &mut runtime.bindings).map_err(|e| format!("{call}: {e}"))?;
            runtime.drain()?;
        } else {
            runtime.call_screen(name, args).map_err(|e| format!("screen.{call}: {e}"))?;
        }
        for _ in 0..frames {
            runtime.update()?;
        }
    }
    for _ in 0..frames {
        runtime.update()?;
    }
    let draws = apt_scene::draw(&runtime.bindings.movie, &runtime.vm, &shapes)?;
    rasterize(&draws, root, out)
}

/// Rasterizes draw batches to a 960 x 544 PNG with the Vita shader's maths (see `ui_render`).
fn rasterize(draws: &[skate_ui::apt_scene::Draw], root: &str, out: &str) -> Result<bool, String> {
    use skate_ui::apt_scene;
    use std::collections::HashMap;
    // Textures by the paths the draws name.
    let mut textures: HashMap<String, (usize, usize, Vec<u8>)> = HashMap::new();
    textures.insert("white".into(), (1, 1, vec![255; 4]));
    for d in draws {
        if !textures.contains_key(&d.texture) {
            match std::fs::read(Path::new(root).join(&d.texture)) {
                Ok(bytes) if bytes.len() == d.size[0] as usize * d.size[1] as usize * 4 => {
                    textures.insert(d.texture.clone(), (d.size[0] as usize, d.size[1] as usize, bytes));
                }
                other => eprintln!("texture {}: {:?}", d.texture, other.map(|b| b.len())),
            }
        }
    }
    const W: usize = 960;
    const H: usize = 544;
    let mut frame = vec![[0.10f32, 0.11, 0.14, 1.0]; W * H];
    let lin = |c: f32| c.powf(2.2);
    for d in draws {
        let Some((tw, th, tex)) = textures.get(&d.texture) else { continue };
        let project = |v: &apt_scene::Vertex| (v.position[0] * 0.75, v.position[1] * 0.75 + 2.0, v.uv);
        for tri in d.vertices.chunks_exact(3) {
            let p: Vec<_> = tri.iter().map(project).collect();
            let area = (p[1].0 - p[0].0) * (p[2].1 - p[0].1) - (p[2].0 - p[0].0) * (p[1].1 - p[0].1);
            if area.abs() < 1e-6 {
                continue;
            }
            let (x0, x1) = (p.iter().map(|v| v.0).fold(f32::MAX, f32::min).floor().max(0.0) as usize, (p.iter().map(|v| v.0).fold(f32::MIN, f32::max).ceil() as i64).clamp(0, W as i64 - 1) as usize);
            let (y0, y1) = (p.iter().map(|v| v.1).fold(f32::MAX, f32::min).floor().max(0.0) as usize, (p.iter().map(|v| v.1).fold(f32::MIN, f32::max).ceil() as i64).clamp(0, H as i64 - 1) as usize);
            for y in y0..=y1 {
                for x in x0..=x1 {
                    let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                    let w0 = ((p[1].0 - px) * (p[2].1 - py) - (p[2].0 - px) * (p[1].1 - py)) / area;
                    let w1 = ((p[2].0 - px) * (p[0].1 - py) - (p[0].0 - px) * (p[2].1 - py)) / area;
                    let w2 = 1.0 - w0 - w1;
                    if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                        continue;
                    }
                    let u = w0 * p[0].2[0] + w1 * p[1].2[0] + w2 * p[2].2[0];
                    let v = w0 * p[0].2[1] + w1 * p[1].2[1] + w2 * p[2].2[1];
                    // Bilinear, clamped.
                    let (fx, fy) = ((u * *tw as f32 - 0.5).clamp(0.0, *tw as f32 - 1.0), (v * *th as f32 - 0.5).clamp(0.0, *th as f32 - 1.0));
                    let (ix, iy) = (fx.floor() as usize, fy.floor() as usize);
                    let (jx, jy) = ((ix + 1).min(tw - 1), (iy + 1).min(th - 1));
                    let (ax, ay) = (fx - ix as f32, fy - iy as f32);
                    let mut t = [0.0f32; 4];
                    for (c, slot) in t.iter_mut().enumerate() {
                        let s = |xx: usize, yy: usize| f32::from(tex[(yy * tw + xx) * 4 + c]) / 255.0;
                        *slot = (s(ix, iy) * (1.0 - ax) + s(jx, iy) * ax) * (1.0 - ay) + (s(ix, jy) * (1.0 - ax) + s(jx, jy) * ax) * ay;
                    }
                    let mut src = [0.0f32; 4];
                    for c in 0..4 {
                        let base = if c < 3 { lin(t[c]) } else { t[c] };
                        src[c] = (base * d.multiply[c] + d.add[c]).clamp(0.0, 1.0);
                    }
                    let dst = &mut frame[y * W + x];
                    for c in 0..3 {
                        dst[c] = src[c].powf(1.0 / 2.2) * src[3] + dst[c] * (1.0 - src[3]);
                    }
                }
            }
        }
    }
    let mut bytes = Vec::with_capacity(W * H * 3);
    for px in &frame {
        bytes.extend(px[..3].iter().map(|c| (c.clamp(0.0, 1.0) * 255.0 + 0.5) as u8));
    }
    let file = std::fs::File::create(out).map_err(|e| e.to_string())?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), W as u32, H as u32);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header().map_err(|e| e.to_string())?.write_image_data(&bytes).map_err(|e| e.to_string())?;
    println!("{} draw batches, {} textures -> {out}", draws.len(), textures.len());
    Ok(true)
}

/// The crossbar (`skate_ui::crossbar`) with a sample menu model, driven by EVENTS (M9): the
/// events are applied in order with 30 frames after each, their outcomes are printed and
/// the final frame is rendered.
fn ui_crossbar(path: &str, root: &str, out: &str, events: &str) -> Result<bool, String> {
    use skate_ui::crossbar::{Action, Crossbar, Event, MenuDef, OptionDef};
    let option = |name: &str, icon: &'static str, help: &str, action: Action, values: &[&str], current: usize| OptionDef {
        name: name.to_owned(),
        icon,
        help: help.to_owned(),
        action,
        values: values.iter().map(|v| (*v).to_owned()).collect(),
        current,
    };
    let menus = vec![
        MenuDef {
            name: "SKATE",
            icon: "singleplayer",
            options: vec![
                option("Resume", "freeskate", "Back to the park.", Action::Resume, &[], 0),
                option("Respawn", "restart", "Return to the park's spawn point.", Action::Respawn, &[], 0),
                option("Park", "skatepark", "Choose where to skate.", Action::Park, &["Course", "StartPark", "MegaPark", "BlackBoxPark", "IndustrialSkatePark", "DownTownSkatePark", "MaloofMoneyCup", "SkateSchool"], 3),
                option("Difficulty", "settings", "How forgiving the board is.", Action::Difficulty, &["Easy", "Normal", "Hardcore"], 1),
                option("Quit", "quit", "Leave the game.", Action::Quit, &[], 0),
            ],
        },
        MenuDef {
            name: "LEARN",
            icon: "learn",
            options: vec![
                option("Controls", "trickguide", "Buttons and what they do.", Action::Controls, &[], 0),
                option("Stick calibration", "onscreenhelp", "Check the sticks.", Action::Calibration, &[], 0),
            ],
        },
        MenuDef {
            name: "OPTIONS",
            icon: "options",
            options: vec![
                option("Volume", "settings", "Sound level.", Action::Volume, &["0", "1", "2", "3", "4", "5", "6", "7", "8", "9", "10"], 8),
                option("Debug overlay", "view", "Frame rate and timing.", Action::DebugHud, &["Off", "On"], 0),
            ],
        },
    ];
    let json: serde_json::Value = serde_json::from_slice(&std::fs::read(path).map_err(|e| format!("{path}: {e}"))?).map_err(|e| e.to_string())?;
    let mut crossbar = Crossbar::load(&json, menus.clone())?;
    println!("loaded: {} instances", crossbar.runtime.bindings.movie.instances.len());
    crossbar.open(menus, 0, 0).map_err(|e| format!("open: {e} ({} instances)", crossbar.runtime.bindings.movie.instances.len()))?;
    for _ in 0..90 {
        crossbar.update()?;
    }
    for name in events.split(',').filter(|e| !e.is_empty()) {
        let event = match name {
            "Left" => Event::Left,
            "Right" => Event::Right,
            "Up" => Event::Up,
            "Down" => Event::Down,
            "Accept" => Event::Accept,
            "Back" => Event::Back,
            other => return Err(format!("unknown event {other}")),
        };
        let outcome = crossbar.event(event)?;
        for _ in 0..30 {
            crossbar.update()?;
        }
        println!("{name:<7} -> {outcome:?}   cursor {:?}  picker {}", crossbar.cursor(), crossbar.in_picker());
    }
    let draws = crossbar.draw()?;
    rasterize(&draws, root, out)
}

fn ui_analyzer(path: &str, root: &str, out: &str, trick: &str) -> Result<bool, String> {
    let json: serde_json::Value = serde_json::from_slice(&std::fs::read(path).map_err(|e| format!("{path}: {e}"))?).map_err(|e| e.to_string())?;
    let mut analyzer = skate_ui::analyzer::TrickAnalyzer::load(&json)?;
    for _ in 0..90 {
        analyzer.update(trick, false, std::env::var_os("ANALYZER_SWITCH").is_some())?;
    }
    let draws = analyzer.draw()?;
    println!("{} batches", draws.len());
    for d in &draws {
        println!("  {} {:?} mul {:?}", d.texture, d.vertices.first().map(|v| v.position), d.multiply);
    }
    rasterize(&draws, root, out)
}

/// The retail list popup with the replay editor's Finished options, the second selected.
fn ui_popup(path: &str, root: &str, out: &str) -> Result<bool, String> {
    let json: serde_json::Value = serde_json::from_slice(&std::fs::read(path).map_err(|e| format!("{path}: {e}"))?).map_err(|e| e.to_string())?;
    let mut popup = skate_ui::popup::Popup::load(&json)?;
    popup.open(["ID_REPLAYEDITOR_UPLOAD", "ID_REPLAYEDITOR_BASIC_SAVE_REPLAY", "ID_REPLAYEDITOR_EXIT"].map(String::from).to_vec(), "")?;
    for _ in 0..30 {
        popup.update()?;
    }
    for (name, b) in popup.glyphs() {
        println!("glyph {name} {b:?}");
    }
    {
        use skate_ui::apt_vm::Value;
        let vm = &popup.runtime.vm;
        let root = popup.runtime.bindings.movie.root;
        println!("root vis {:?} alpha {:?} frame {:?}", vm.get(root, "_visible"), vm.get(root, "_alpha"), vm.get(root, "_currentframe"));
        if let Value::Object(panel) = vm.get(root, "mcPanel") {
            println!("panel x {:?} y {:?} alpha {:?} vis {:?} frame {:?}", vm.get(panel, "_x"), vm.get(panel, "_y"), vm.get(panel, "_alpha"), vm.get(panel, "_visible"), vm.get(panel, "_currentframe"));
            if let Value::Object(fb) = vm.get(panel, "mFrameBottom") {
                if let Value::Object(b) = vm.get(fb, "mButtons") {
                    println!("mButtons x {:?} y {:?} vis {:?} alpha {:?} count {:?}", vm.get(b, "_x"), vm.get(b, "_y"), vm.get(b, "_visible"), vm.get(b, "_alpha"), vm.get(b, "mItemCount"));
                    for k in ["mButton0", "mButton1", "mText0", "mText1"] {
                        if let Value::Object(o) = vm.get(b, k) {
                            println!("   {k}: x {:?} y {:?} vis {:?} w {:?} img {:?} text {:?}", vm.get(o, "_x"), vm.get(o, "_y"), vm.get(o, "_visible"), vm.get(o, "_width"), vm.get(o, "mButtonImage"), vm.get(o, "text"));
                        }
                    }
                }
            }
            if let Value::Object(p) = vm.get(panel, "mMenuPicker") {
                let mut keys: Vec<String> = vm.objects[p].fields.iter().map(|(k, v)| format!("{k}={}", match v { Value::Object(_) => "{}".to_owned(), v => format!("{v:?}") })).collect();
                keys.sort();
                println!("picker fields: {}", keys.join(" "));
                let mv = &popup.runtime.bindings.movie;
                println!("picker display children: {}", mv.instances.get(&p).map_or(0, |i| i.children.len()));
            }
            for k in ["mFrameTop", "mFrameBottom", "mMenuPicker", "body"] {
                if let Value::Object(o) = vm.get(panel, k) {
                    println!("  {k}: x {:?} y {:?} alpha {:?} vis {:?}", vm.get(o, "_x"), vm.get(o, "_y"), vm.get(o, "_alpha"), vm.get(o, "_visible"));
                }
            }
        }
    }
    let draws = popup.draw()?;
    println!("{} batches", draws.len());
    rasterize(&draws, root, out)
}

/// The retail replay help for an editor mode number (0 instant, 1 main, 2 effects, 6 free camera).
fn ui_replay_help(path: &str, root: &str, out: &str, mode: &str) -> Result<bool, String> {
    let json: serde_json::Value = serde_json::from_slice(&std::fs::read(path).map_err(|e| format!("{path}: {e}"))?).map_err(|e| e.to_string())?;
    let mut help = skate_ui::replay_editor::ReplayHelp::load(&json)?;
    for _ in 0..30 {
        help.update()?;
    }
    help.open(mode.parse().map_err(|_| "bad mode")?)?;
    for _ in 0..30 {
        help.update()?;
    }
    {
        use skate_ui::apt_vm::Value;
        let mv = &help.runtime.bindings.movie;
        let vm = &help.runtime.vm;
        for (&id, inst) in &mv.instances {
            for key in ["_x", "_y", "_xscale", "_yscale", "_alpha", "_width", "_height"] {
                if let Value::Number(n) = vm.get(id, key) {
                    if !n.is_finite() {
                        println!("NONFINITE id {id} char {} {key}={n}", inst.character);
                    }
                }
            }
        }
    }
    for (name, b) in help.glyphs() {
        println!("glyph {name} {b:?}");
    }
    let draws = help.draw()?;
    println!("{} batches", draws.len());
    rasterize(&draws, root, out)
}

/// The retail replay editor in a mode (`scrub`, `main`, `effects`) with the playhead at 40 % and the clip trimmed to 20..80 %.
fn ui_replay(path: &str, root: &str, out: &str, mode: &str) -> Result<bool, String> {
    use skate_ui::replay_editor::{ReplayEditor, state};
    let json: serde_json::Value = serde_json::from_slice(&std::fs::read(path).map_err(|e| format!("{path}: {e}"))?).map_err(|e| e.to_string())?;
    let mut editor = ReplayEditor::load(&json)?;
    editor.intro()?;
    for _ in 0..60 {
        editor.update()?;
    }
    println!("ready: {}", editor.model().ready);
    let m = editor.model();
    (m.cursor, m.start, m.end, m.length) = (0.4, 0.2, 0.8, "0:36".into());
    m.instant = mode == "scrub";
    if mode != "none" {
        editor.set_state(match mode {
            "main" => state::MAIN_EDIT,
            "effects" => state::EFFECTS,
            "scrub" => state::SCRUB,
            number => number.parse().map_err(|_| format!("unknown mode {number}"))?,
        })?;
    }
    for s in [state::MOVE_PLAYHEAD, state::MOVE_START, state::MOVE_END] {
        editor.set_state(s)?;
    }
    for _ in 0..std::env::var("FRAMES").ok().and_then(|v| v.parse().ok()).unwrap_or(40) {
        editor.update()?;
    }
    {
        use skate_ui::apt_vm::Value;
        let vm = &editor.runtime.vm;
        let cls = vm.get(vm.global, "replay_editor2");
        if let Value::Object(c) = cls {
            if let Value::Object(screen) = vm.get(c, "mScreen") {
                println!("mScreen visible {:?} alpha {:?}", vm.get(screen, "_visible"), vm.get(screen, "_alpha"));
                if let Value::Object(r) = vm.get(screen, "mReplay") {
                    for (k, v) in &vm.objects[r].fields {
                        if let Value::Object(o) = v {
                            if k.starts_with('_') || k == "prototype" {
                                continue;
                            }
                            println!("  {k}: x {:?} y {:?} vis {:?} alpha {:?} frame {:?}", vm.get(*o, "_x"), vm.get(*o, "_y"), vm.get(*o, "_visible"), vm.get(*o, "_alpha"), vm.get(*o, "_currentframe"));
                        }
                    }
                    let mv = &editor.runtime.bindings.movie;
                    let inst = &mv.instances[&r];
                    println!("mReplay display children {} character {} frames {}", inst.children.len(), inst.character, mv.characters[&inst.character].frames.len());
                    for (d, c) in &inst.children {
                        let ci = &mv.instances[c];
                        println!("   depth {d}: instance {c} char {} type {} children {}", ci.character, mv.characters[&ci.character].type_name, ci.children.len());
                    }
                    {
                        fn walk(mv: &skate_ui::apt_movie::Movie, vm: &skate_ui::apt_vm::Vm, id: usize, path: &str, depth: usize, stats: &mut [usize; 4], a: f32) {
                            let inst = &mv.instances[&id];
                            stats[0] += 1;
                            if mv.characters[&inst.character].type_name == "shape" {
                                stats[1] += 1;
                            }
                            if !vm.get(id, "_visible").truth() {
                                stats[2] += 1;
                                if depth < 3 {
                                    println!("   hidden: {path}");
                                }
                                return;
                            }
                            if vm.get(id, "_alpha").number() <= 0.0 {
                                stats[3] += 1;
                                if depth < 3 {
                                    println!("   alpha0: {path}");
                                }
                                return;
                            }
                            let a = a * (vm.get(id, "_alpha").number() as f32 / 100.0) * inst.placement.as_ref().map_or(1.0, |p| p.color[0] as f32 / 255.0);
                            for key in ["_x", "_y", "_xscale", "_yscale"] {
                                if let skate_ui::apt_vm::Value::Number(n) = vm.get(id, key) {
                                    if !n.is_finite() {
                                        println!("   NONFINITE {path} char {} {key}={n}", inst.character);
                                    }
                                }
                            }
                            if depth <= 2 && path.starts_with("root/3") {
                                println!("   NODE {path} a={a} _alpha={:?} place={:?}", vm.get(id, "_alpha"), inst.placement.as_ref().map(|p| p.color));
                            }
                            if depth <= 2 && path.starts_with("root/3") {
                                println!("   NODE {path} a={a} _alpha={:?} place={:?}", vm.get(id, "_alpha"), inst.placement.as_ref().map(|p| p.color));
                            }
                            if mv.characters[&inst.character].type_name == "shape" && stats[1] < 400 {
                                if a > 0.0 {
                                    stats[3] += 0;
                                    println!("   SHAPE visible {path} a={a}");
                                }
                            }
                            for (d, c) in &inst.children {
                                walk(mv, vm, *c, &format!("{path}/{d}"), depth + 1, stats, a);
                            }
                        }
                        let mut stats = [0; 4];
                        walk(mv, vm, mv.root, "root", 0, &mut stats, 1.0);
                        println!("   mReplay id {r}, screen id {screen}, root {}", mv.root);
                        for (d, c) in &mv.instances[&screen].children {
                            println!("   mScreen child depth {d}: {c} char {}", mv.instances[c].character);
                        }
                        println!("   walk: instances {} shapes {} hidden {} alpha0 {}", stats[0], stats[1], stats[2], stats[3]);
                    }
                    println!("mReplay y {:?} frame {:?} alpha {:?} visible {:?}", vm.get(r, "_y"), vm.get(r, "_currentframe"), vm.get(r, "_alpha"), vm.get(r, "_visible"));
                }
            }
        }
    }
    for (c, st, en) in [(0.0f32, 0.0f32, 1.0f32), (1.0, 0.0, 1.0), (0.5, 0.25, 0.75)] {
        let m = editor.model();
        (m.cursor, m.start, m.end) = (c, st, en);
        for state in [state::MOVE_PLAYHEAD, state::MOVE_START, state::MOVE_END] {
            editor.set_state(state)?;
        }
        println!("cursor {c} start {st} end {en}: playhead/in/out stage x {:?}", editor.timeline_positions());
    }
    for (name, b) in editor.glyphs() {
        println!("glyph {name} {b:?}");
    }
    {
        use skate_ui::apt_vm::Value;
        let vm = &editor.runtime.vm;
        if let Value::Object(c) = vm.get(vm.global, "replay_editor2") {
            if let Value::Object(screen) = vm.get(c, "mScreen") {
                if let Value::Object(r) = vm.get(screen, "mReplay") {
                    for name in ["mcStart", "mcEnd", "mcPlay", "mcCamZoom", "mcCamUpDown"] {
                        if let Value::Object(clip) = vm.get(r, name) {
                            let item = match vm.get(clip, "mButtonItem") { Value::Object(i) => Some(i), _ => None };
                            println!("{name}: x {:?} y {:?} alpha {:?} vis {:?}; item {:?} x {:?} y {:?} image {:?} vis {:?}", vm.get(clip, "_x"), vm.get(clip, "_y"), vm.get(clip, "_alpha"), vm.get(clip, "_visible"), item, item.map(|i| vm.get(i, "_x")), item.map(|i| vm.get(i, "_y")), item.map(|i| vm.get(i, "mButtonImage")), item.map(|i| vm.get(i, "_visible")));
                        }
                    }
                }
            }
        }
    }
    let draws = editor.draw()?;
    for d in draws.iter().filter(|d| d.texture == "white") {
        let xs = d.vertices.iter().map(|v| v.position[0]);
        let ys = d.vertices.iter().map(|v| v.position[1]);
        println!("WHITE x {:.0}..{:.0} y {:.0}..{:.0} mul {:?} add {:?}", xs.clone().fold(1e9, f32::min), xs.fold(-1e9, f32::max), ys.clone().fold(1e9, f32::min), ys.fold(-1e9, f32::max), d.multiply, d.add);
    }
    println!("{} batches", draws.len());
    rasterize(&draws, root, out)
}

fn ui_meter(path: &str, root: &str, out: &str, values: &str) -> Result<bool, String> {
    let json: serde_json::Value = serde_json::from_slice(&std::fs::read(path).map_err(|e| format!("{path}: {e}"))?).map_err(|e| e.to_string())?;
    let mut meter = skate_ui::meter::ManualMeter::load(&json)?;
    for v in values.split(',').filter(|v| !v.is_empty()) {
        let v: f32 = v.parse().map_err(|_| format!("bad value {v}"))?;
        for _ in 0..20 {
            meter.update(true, v)?;
        }
    }
    let draws = meter.draw()?;
    println!("{} batches", draws.len());
    rasterize(&draws, root, out)
}

/// Runs a linked front-end screen (`tools/link_ui_screen.py`) in `skate_ui::menu_runtime`
/// with a host that records every native call and answers `undefined` (M9: sizing the
/// menu work; the native objects named by NATIVES, comma separated, are defined).
fn ui_probe(path: &str, frames: usize) -> Result<bool, String> {
    use skate_ui::apt_vm::{ObjectKind, Value, Vm};
    use skate_ui::menu_runtime::{MenuRuntime, Natives};
    use std::collections::BTreeMap;

    #[derive(Default)]
    struct Probe {
        calls: BTreeMap<String, (u32, usize, String)>,
    }
    impl Natives for Probe {
        fn names(&self) -> Vec<String> {
            std::env::var("NATIVES")
                .unwrap_or_else(|_| "coremenu,FELanguage,Game,ScreenManager".into())
                .split(',')
                .map(str::to_owned)
                .collect()
        }
        fn call(&mut self, _vm: &mut Vm, native: &str, method: &str, args: &[Value]) -> Result<Value, String> {
            let shown: Vec<String> = args.iter().map(|a| format!("{a:?}").chars().take(36).collect()).collect();
            let entry = self.calls.entry(format!("{native}.{method}")).or_insert((0, args.len(), shown.join(", ")));
            entry.0 += 1;
            // A tiny fake data model: 3 menus of 3 options, no sub-options.
            Ok(match (native, method) {
                ("coremenu", "GetIsDemo" | "IsDLCEnabled") => Value::Bool(false),
                ("coremenu", "GetNumMenus") => Value::Number(3.0),
                ("coremenu", "GetNumOptions") => Value::Number(3.0),
                ("coremenu", "GetNumSubOptions" | "GetMaxSubOptions" | "GetCurrentMenuLevel") => Value::Number(0.0),
                ("coremenu", "GetCurrentMenuIndex" | "GetCurrentOptionIndex" | "GetCurrentSubOptionIndex") => Value::Number(0.0),
                ("coremenu", "IsItemEnabled") | ("Game", "OnMenuGetItemEnabled") => Value::Bool(true),
                ("coremenu", "GetMenuName") => Value::Text(format!("Menu {}", args.first().map_or(0.0, Value::number))),
                ("coremenu", "GetOptionName" | "GetSubOptionName") => Value::Text(format!("Option {:?}", args)),
                ("coremenu", "GetMenuIconName") => Value::Text(["singleplayer", "learn", "options"][args.first().map_or(0.0, Value::number) as usize % 3].into()),
                ("coremenu", "GetOptionIconName") => Value::Text(["freeskate", "skatepark", "settings"][args.get(1).map_or(0.0, Value::number) as usize % 3].into()),
                ("coremenu", "GetCurrentModeTitle" | "GetOptionHelperDescription") => Value::Text("Title".into()),
                _ => Value::Undefined,
            })
        }
    }
    let json: serde_json::Value = serde_json::from_slice(&std::fs::read(path).map_err(|e| format!("{path}: {e}"))?).map_err(|e| e.to_string())?;
    let mut runtime = MenuRuntime::load(&json, Probe::default())?;
    let describe = |vm: &Vm, object: usize| -> String {
        let o = &vm.objects[object];
        let mut names: Vec<String> = o
            .fields
            .iter()
            .map(|(k, v)| match v {
                Value::Object(f) if matches!(vm.objects[*f].kind, ObjectKind::Function(_)) => format!("{k}()"),
                Value::Object(_) => format!("{k}{{}}"),
                _ => k.clone(),
            })
            .collect();
        names.sort();
        names.join(" ")
    };
    let mut errors: Vec<String> = Vec::new();
    for frame in 0..frames {
        if let Err(e) = runtime.update() {
            errors.push(format!("frame {frame}: {e}"));
            break;
        }
    }
    // Entry points, as the native side would call them (MODEL_CALLS="Name;Name"): after
    // each, a few frames.
    for call in std::env::var("MODEL_CALLS").unwrap_or_default().split(';').filter(|c| !c.is_empty()) {
        let (name, arg) = call.split_once('(').map_or((call, None), |(n, a)| (n, Some(a.trim_end_matches(')'))));
        let args: Vec<Value> = arg.map(|a| a.split(',').filter(|x| !x.is_empty()).map(|x| x.parse::<f64>().map_or_else(|_| match x { "AptUp" => Value::Number(1.0), "AptDown" => Value::Number(2.0), "AptPrevious" => Value::Number(3.0), _ => Value::Text(x.into()) }, Value::Number)).collect()).unwrap_or_default();
        let before = runtime.bindings.natives.calls.len();
        match runtime.call_screen(name, args) {
            Ok(v) => println!("screen.{call} -> {v:?}"),
            Err(e) => errors.push(format!("screen.{call}: {e}")),
        }
        for _ in 0..10 {
            if let Err(e) = runtime.update() {
                errors.push(format!("after {call}: {e}"));
                break;
            }
        }
        let _ = before;
    }
    println!("{path}: {} characters, {} classes registered, {frames} frames", runtime.bindings.movie.characters.len(), runtime.bindings.movie.class_for.len());
    match runtime.controller {
        Some(c) => {
            println!("_root.screen fields: {}", describe(&runtime.vm, c));
            if let Some(p) = runtime.vm.objects[c].prototype {
                println!("_root.screen class: {}", describe(&runtime.vm, p));
            }
        }
        None => println!("_root.screen: not set"),
    }
    println!("_root fields: {}", describe(&runtime.vm, runtime.bindings.movie.root));
    if let Value::Object(class) = runtime.vm.get(runtime.vm.global, "core_menu") {
        let mut consts: Vec<String> = runtime.vm.objects[class].fields.iter().filter(|(_, v)| !matches!(v, Value::Object(_))).map(|(k, v)| format!("{k}={}", v.text())).collect();
        consts.sort();
        println!("core_menu statics: {}", consts.join(" "));
    }
    if let Some(c) = runtime.controller {
        let mut vals: Vec<String> = runtime.vm.objects[c].fields.iter().filter(|(_, v)| !matches!(v, Value::Object(_))).map(|(k, v)| format!("{k}={}", v.text())).collect();
        vals.sort();
        println!("controller state: {}", vals.join(" "));
    }
    println!("native calls (count, arity, first arguments):");
    for (name, (n, arity, first)) in &runtime.bindings.natives.calls {
        println!("  {name:<46} x{n:<5} {arity} args  {first}");
    }
    println!("{} distinct native calls", runtime.bindings.natives.calls.len());
    println!("errors:");
    for e in errors.iter().take(8) {
        println!("  {e}");
    }
    Ok(true)
}

/// Degenerate triangles in the retail collision of map packs (M8): zero or tiny area
/// (the normal is a division by that length), non-finite points, and coincident vertices.
fn degenerate(packs: &[String]) -> Result<bool, String> {
    for path in packs {
        let bytes = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
        let sim = vita_pack::map::Map::from_pack(&bytes)?.sim_map();
        let Some(rwcm) = sim.extensions.iter().find(|e| &e.tag == b"RWCM") else {
            println!("{path}: no RWCM collision");
            continue;
        };
        let (mut total, mut zero, mut tiny, mut nonfinite, mut coincident) = (0u64, 0u64, 0u64, 0u64, 0u64);
        skate_data::retail_collision::visit_clusters(&rwcm.payload, |_, triangles| {
            for t in triangles {
                total += 1;
                let [a, b, c] = t.points;
                let e1 = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
                let e2 = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
                let n = [e1[1] * e2[2] - e1[2] * e2[1], e1[2] * e2[0] - e1[0] * e2[2], e1[0] * e2[1] - e1[1] * e2[0]];
                let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
                if !len.is_finite() || t.points.iter().flatten().any(|x| !x.is_finite()) {
                    nonfinite += 1;
                } else if len == 0.0 {
                    zero += 1;
                } else if len < 1e-6 {
                    tiny += 1;
                }
                if a == b || b == c || a == c {
                    coincident += 1;
                }
            }
            Ok(())
        })?;
        println!("{path}: {total} triangles; area*2 == 0: {zero}, < 1e-6: {tiny}, non-finite: {nonfinite}, repeated vertex: {coincident}");
    }
    Ok(true)
}

/// The last `n` ticks of a dump (M8): state, board position and speed, and the fastest
/// skeleton body (linear and angular), to see what precedes a failure.
fn trace(path: &Path, n: usize) -> Result<bool, String> {
    let d = dump::read(path)?;
    let words = 13;
    let f = |w: u32| f32::from_bits(w);
    let start = d.ticks.len().saturating_sub(n);
    println!("{} ticks, {} board bodies, {} skeleton bodies", d.ticks.len(), d.board_bodies, d.skeleton_bodies);
    println!("{:>5} {:>6} {:>7} {:>26} {:>8} {:>10} {:>10}", "tick", "state", "failed", "board position", "speed", "max v skel", "max w skel");
    for t in &d.ticks[start..] {
        let body = |i: usize| &t.bodies[i * words..(i + 1) * words];
        let board = body(0);
        let speed = (f(board[7]).powi(2) + f(board[8]).powi(2) + f(board[9]).powi(2)).sqrt();
        let mut max_v = 0.0f32;
        let mut max_w = 0.0f32;
        let mut nan_body = None;
        for i in 0..d.board_bodies + d.skeleton_bodies {
            let b = body(i);
            let v = (f(b[7]).powi(2) + f(b[8]).powi(2) + f(b[9]).powi(2)).sqrt();
            let w = (f(b[10]).powi(2) + f(b[11]).powi(2) + f(b[12]).powi(2)).sqrt();
            if !v.is_finite() || !w.is_finite() || b[..13].iter().any(|&x| !f(x).is_finite()) {
                nan_body.get_or_insert(i);
            }
            if i >= d.board_bodies {
                max_v = max_v.max(v);
                max_w = max_w.max(w);
            }
        }
        println!(
            "{:>5} {:>6} {:>7} {:>8.2} {:>8.2} {:>8.2} {:>8.2} {:>10.2} {:>10.2}{}",
            t.tick, t.state, t.failed, f(board[0]), f(board[1]), f(board[2]), speed, max_v, max_w,
            nan_body.map_or(String::new(), |i| format!("   non-finite in body {i}"))
        );
    }
    Ok(true)
}

/// Tick time per physical state (M8): joins each dump's state per tick with the
/// `.ticks` file beside it (u32 LE µs per tick, written by sim-bench and the app's
/// autoplay) and prints the count, mean, p95 and max per state over all scenarios.
fn state_times(dir: &Path) -> Result<bool, String> {
    let mut by_state: std::collections::BTreeMap<u32, Vec<u32>> = Default::default();
    for (name, path) in dumps(dir)? {
        let d = dump::read(&path)?;
        let ticks_path = path.with_extension("ticks");
        let bytes = match std::fs::read(&ticks_path) {
            Ok(b) => b,
            Err(_) => {
                eprintln!("{name}: no .ticks file, skipped");
                continue;
            }
        };
        let us: Vec<u32> = bytes.chunks_exact(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect();
        for (t, &v) in d.ticks.iter().zip(&us) {
            by_state.entry(t.state).or_default().push(v);
        }
    }
    println!("{:>6} {:>7} {:>9} {:>9} {:>9}", "state", "ticks", "mean ms", "p95 ms", "max ms");
    for (state, mut v) in by_state {
        v.sort_unstable();
        let mean = v.iter().map(|&x| u64::from(x)).sum::<u64>() as f64 / v.len() as f64;
        println!(
            "{state:>6} {:>7} {:>9.2} {:>9.2} {:>9.2}",
            v.len(),
            mean / 1000.0,
            f64::from(v[v.len() * 95 / 100]) / 1000.0,
            f64::from(*v.last().unwrap()) / 1000.0
        );
    }
    Ok(true)
}

/// Mean stage time per physical state (M8): joins each dump's state per tick with the
/// per-tick stage CSV beside it (`.stages`, sim-bench feature `tick-stages`: a header
/// `tick,stage,...` then one row per tick; µs, or counters named `n_*`/`q_*`). Prints,
/// per stage, the mean in each state that has at least 50 ticks, and the difference to
/// state 100 (PhysicsGround).
fn state_stages(dir: &Path) -> Result<bool, String> {
    let mut names: Vec<String> = Vec::new();
    // state -> (ticks, per-stage sums)
    let mut sums: std::collections::BTreeMap<u32, (u64, Vec<f64>)> = Default::default();
    for (name, path) in dumps(dir)? {
        let Ok(text) = std::fs::read_to_string(path.with_extension("stages")) else {
            eprintln!("{name}: no .stages file, skipped");
            continue;
        };
        let d = dump::read(&path)?;
        let mut lines = text.lines();
        let header: Vec<String> = lines.next().ok_or("empty .stages")?.split(',').skip(1).map(str::to_owned).collect();
        if names.is_empty() {
            names = header.clone();
        }
        for (t, line) in d.ticks.iter().zip(lines) {
            let entry = sums.entry(t.state).or_insert_with(|| (0, vec![0.0; names.len()]));
            entry.0 += 1;
            for (col, v) in line.split(',').skip(1).enumerate() {
                if let Some(i) = names.iter().position(|n| *n == header[col]) {
                    entry.1[i] += v.parse::<f64>().unwrap_or(0.0);
                }
            }
        }
    }
    let states: Vec<u32> = sums.iter().filter(|(_, v)| v.0 >= 50).map(|(k, _)| *k).collect();
    print!("{:<34}", "stage (µs/tick, counters raw)");
    for s in &states {
        print!("{:>9}", format!("{s}"));
    }
    println!("{:>11}", "300 - 100");
    print!("{:<34}", "(ticks)");
    for s in &states {
        print!("{:>9}", sums[s].0);
    }
    println!();
    let mean = |s: u32, i: usize| sums.get(&s).map_or(0.0, |v| v.1[i] / v.0 as f64);
    let mut order: Vec<usize> = (0..names.len()).collect();
    order.sort_by(|&a, &b| (mean(300, b) - mean(100, b)).abs().total_cmp(&(mean(300, a) - mean(100, a)).abs()));
    for i in order {
        print!("{:<34}", names[i]);
        for s in &states {
            print!("{:>9.0}", mean(*s, i));
        }
        println!("{:>11.0}", mean(300, i) - mean(100, i));
    }
    Ok(true)
}

/// PLAN.md §0.5: loads everything with the skate-data parsers and the production loaders.
/// MAPS defaults to ASSETS/../maps (the converter's installation layout).
fn check_assets(assets: &Path, maps: Option<PathBuf>) -> Result<bool, String> {
    let mut ok = true;
    for line in skate_sim::oracle_harness::check_assets(assets)? {
        println!("{line}");
    }
    // skater.glb is parsed by the renderer only; check its glTF 2.0 header here.
    let glb = assets.join("private/skater.glb");
    match std::fs::read(&glb) {
        Ok(b) if b.len() >= 12
            && &b[..4] == b"glTF"
            && u32::from_le_bytes(b[4..8].try_into().unwrap()) == 2
            && u32::from_le_bytes(b[8..12].try_into().unwrap()) as usize == b.len() =>
        {
            println!("ok  {:<28} {} bytes, glTF 2.0", "skater.glb", b.len())
        }
        Ok(_) => {
            println!("ERR {:<28} not a glTF 2.0 binary", "skater.glb");
            ok = false;
        }
        Err(e) => {
            println!("ERR {:<28} {e}", "skater.glb");
            ok = false;
        }
    }
    let maps = maps.unwrap_or_else(|| assets.join("../maps"));
    let skate_files = |dir: &Path| -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
            .map(|entries| {
                entries
                    .filter_map(|e| e.ok().map(|e| e.path()))
                    .filter(|p| p.extension().is_some_and(|x| x == "skate"))
                    .collect()
            })
            .unwrap_or_default();
        v.sort();
        v
    };
    // Prop and backdrop packages are render-only (skate-game retail_backdrop.rs loads
    // them with parse_render_only); they have no collision.
    for dir in ["private/native-props", "private/native-backdrops"] {
        for path in skate_files(&assets.join(dir)) {
            let label = format!("{dir}/{}", path.file_name().unwrap().to_string_lossy());
            let result = std::fs::read(&path)
                .map_err(|e| e.to_string())
                .and_then(|data| skate_data::skate_map::SkateMap::parse_render_only(&data));
            match result {
                Ok(_) => println!("ok  {label:<45} render-only package"),
                Err(e) => {
                    println!("ERR {label:<45} {e}");
                    ok = false;
                }
            }
        }
    }
    for path in skate_files(&maps) {
        let label = path.strip_prefix(assets.parent().unwrap_or(assets)).unwrap_or(&path).display().to_string();
        match skate_sim::oracle_harness::check_map(assets, &path) {
            Ok(detail) => println!("ok  {label:<45} {detail}"),
            Err(e) => {
                println!("ERR {label:<45} {e}");
                ok = false;
            }
        }
    }
    Ok(ok)
}

/// `audio-render`: the scenario through `Session` (the Course, or ORACLE_MAP), one audio host frame
/// per tick (60 Hz: one physics step each) and 800 stereo frames rendered per tick.
fn audio_render(assets: &str, sound_pak: &str, scenario: &str, out: &str) -> Result<bool, String> {
    // AUDIO_MAP=<retail map name> plays that map's zone beds (AUDIO_AMBIENCE=<ambience.pak> holds them).
    let audio_map = std::env::var("AUDIO_MAP").ok();
    let rss = || std::fs::read_to_string("/proc/self/statm").ok().and_then(|s| s.split_whitespace().nth(1)?.parse::<u64>().ok()).unwrap_or(0) * 4096 / 1_048_576;
    let before = rss();
    let root = Path::new("vfs:/sound");
    {
        let bytes = std::fs::read(sound_pak).map_err(|e| format!("{sound_pak}: {e}"))?;
        // AUDIO_SKIP=a,b: leave those banks out (the Vita does not mount the ones its host never loads).
        let skip: Vec<String> = std::env::var("AUDIO_SKIP").map(|v| v.split(',').map(str::to_owned).collect()).unwrap_or_default();
        for (path, data) in vita_pack::files::from_pack(&bytes)? {
            // `grain-bands`: the grains' speed bands (grains/<surface>/<n>.ima), unused by the host.
            let band = skip.iter().any(|b| b == "grain-bands") && path.contains("/grains/") && path.rsplit('/').nth(1).is_some_and(|d| d != "grains");
            if band || skip.iter().any(|b| path.contains(&format!("/banks/{b}/")) || path.ends_with(&format!("/{b}.abk"))) {
                continue;
            }
            skate_data::vfs::mount(root.join(path), data);
        }
    }
    skate_data::vfs::set_strict(true);
    // AUDIO_WORLD=<world.pak>: the emitters' banks.
    if let Some(pak) = std::env::var_os("AUDIO_WORLD") {
        let bytes = std::fs::read(&pak).map_err(|e| format!("{}: {e}", pak.to_string_lossy()))?;
        for (path, data) in vita_pack::files::from_pack(&bytes)? {
            skate_data::vfs::mount(root.join(path), data);
        }
    }
    if let Some(pak) = std::env::var_os("AUDIO_AMBIENCE") {
        let bytes = std::fs::read(&pak).map_err(|e| format!("{}: {e}", pak.to_string_lossy()))?;
        for (path, data) in vita_pack::files::from_pack(&bytes)? {
            skate_data::vfs::mount(root.join(path), data);
        }
    }
    let mounted = rss();
    let library = game_audio::library::Library::load(root)?;
    let loaded = rss();
    let mut native = game_audio::native::Native::start(&library)?;
    // Strict for the audio's own loads only (the sim reads the unpacked assets).
    skate_data::vfs::set_strict(false);
    eprintln!("memory: mounted +{} MiB, library +{} MiB, host +{} MiB", mounted - before, loaded - mounted, rss() - loaded);
    let input = skate_sim::oracle_harness::read_inputs(Path::new(scenario))?;
    let map = skate_sim::oracle_harness::oracle_map()?;
    let mut session = skate_sim::live::Session::load_map(Path::new(assets), &input.difficulty, map)?;
    if let Some(n) = std::env::var("ORACLE_ITERATIONS").ok().and_then(|v| v.parse().ok()) {
        session.set_solver_iterations(n);
    }
    let session_loaded = rss();
    let dt = 1.0 / 60.0;
    let mut pcm: Vec<f32> = Vec::with_capacity(input.ticks.len() * 1600);
    let mut block = [0.0f32; 1600];
    let (mut host_us, mut render_us) = (0u128, 0u128);
    let mut voices_max = 0usize;
    for state in input.ticks {
        let report = session.step(state);
        if let Some(e) = report.error {
            eprintln!("simulation error at tick {}: {e}", session.ticks());
            break;
        }
        let (riding, steps) = session.take_audio();
        let listener = session.presentation().map(|(_, now)| {
            let t = now.camera;
            game_audio::native::Listener { position: t.translation, forward: (t.rotation * glam::Vec3::NEG_Z).normalize_or_zero() }
        });
        let started = std::time::Instant::now();
        native.frame(&library, game_audio::native::FrameInput {
            riding: &riding, steps, listener, cuts: (session.presentation_cuts(), 0), overstep: 0.0, game_dt: dt, real_dt: dt, silenced: false, frontend: &[], teleport: None, map: audio_map.as_deref().map(|m| (m, 1)),
        });
        host_us += started.elapsed().as_micros();
        let started = std::time::Instant::now();
        native.render(&mut block);
        render_us += started.elapsed().as_micros();
        let v = native.voices();
        voices_max = voices_max.max(v);
        if audio_map.is_some() && session.ticks() % 300 == 0 {
            let b = session.board().0;
            eprintln!("tick {} board ({:.0}, {:.0}) bed {:?}", session.ticks(), b[0], b[2], native.bed_playing());
        }
        if std::env::var_os("AUDIO_VOICES").is_some() && session.ticks() % 60 == 0 {
            eprintln!("tick {} voices {v}", session.ticks());
        }
        pcm.extend_from_slice(&block);
    }
    eprintln!("memory: after the session load {session_loaded} MiB, after the run {} MiB (PCM out {} MiB)", rss(), pcm.len() * 4 / 1_048_576);
    let ticks = (pcm.len() / 1600).max(1) as u128;
    let peak = pcm.iter().fold(0.0f32, |m, x| m.max(x.abs()));
    let rms = (pcm.iter().map(|x| f64::from(*x).powi(2)).sum::<f64>() / pcm.len().max(1) as f64).sqrt();
    let mut wav = Vec::with_capacity(44 + pcm.len() * 2);
    let data_len = (pcm.len() * 2) as u32;
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_len).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    for v in [16u32] { wav.extend_from_slice(&v.to_le_bytes()); }
    for v in [1u16, 2] { wav.extend_from_slice(&v.to_le_bytes()); }
    for v in [48_000u32, 48_000 * 4] { wav.extend_from_slice(&v.to_le_bytes()); }
    for v in [4u16, 16] { wav.extend_from_slice(&v.to_le_bytes()); }
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    for x in &pcm {
        wav.extend_from_slice(&((x.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
    }
    std::fs::write(out, wav).map_err(|e| format!("{out}: {e}"))?;
    println!("{out}: {:.1} s, peak {peak:.3}, rms {rms:.4}; host {} us/tick, render {} us/tick, voices max {voices_max}", pcm.len() as f32 / 96_000.0, host_us / ticks, render_us / ticks);
    Ok(true)
}
