//! Unattended soak test (feature `soak`): the real game loop (sim thread,
//! menus, audio, renderer) driven by scripted input for `DURATION`, so memory and
//! timing can be watched over a long session with nobody holding the Vita.
//!
//! The pad is replaced by the sim-oracle scenarios in `ux0:data/SKATEVITA/autoplay/`
//! played back to back. Between scenarios the skater respawns and the difficulty
//! cycles; every `PARK_EVERY` scenarios the world changes (the Course, then the
//! retail parks); the main thread opens the pause menu for a few
//! seconds every two minutes. The heap is logged every 10 s as in normal play.
use std::sync::Mutex;
use std::time::{Duration, Instant};

use skate_core::input::xbox::XboxState;
use skate_sim::live::Session;

use crate::lab::Shared;
use crate::settings::Difficulty;

const INPUTS: &str = "ux0:data/SKATEVITA/autoplay";
const DURATION: Duration = Duration::from_secs(30 * 60);
const PARK_EVERY: usize = 3;
/// Optional `soak.txt` beside the scenarios: `duration=SECONDS` and `switch_every=SECONDS`
/// (a world change at that interval, whatever the scenario is doing; for measuring
/// world switches).
const CONFIG: &str = "ux0:data/SKATEVITA/autoplay/soak.txt";

struct Soak {
    scenarios: Vec<(String, Vec<XboxState>)>,
    scenario: usize,
    tick: usize,
    rounds: usize,
    started: Option<Instant>,
    duration: Duration,
    switch_every: Option<Duration>,
    last_switch: Instant,
    /// `districts=1`: world changes rotate the three districts only.
    districts: bool,
}
static SOAK: Mutex<Option<Soak>> = Mutex::new(None);

fn load() -> Soak {
    let mut paths: Vec<_> = std::fs::read_dir(INPUTS)
        .map(|d| d.filter_map(|e| e.ok().map(|e| e.path())).collect())
        .unwrap_or_default();
    paths.retain(|p: &std::path::PathBuf| p.extension().is_some_and(|x| x == "input"));
    paths.sort();
    let scenarios: Vec<_> = paths
        .iter()
        .filter_map(|p| {
            let name = p.file_stem()?.to_string_lossy().to_string();
            match skate_sim::oracle_harness::read_inputs(p) {
                Ok(s) if !s.ticks.is_empty() => Some((name, s.ticks)),
                Ok(_) => None,
                Err(e) => {
                    println!("SOAK {name}: {e}");
                    None
                }
            }
        })
        .collect();
    let mut duration = DURATION;
    let mut switch_every = None;
    let mut districts = false;
    if let Ok(text) = std::fs::read_to_string(CONFIG) {
        for line in text.lines() {
            match line.split_once('=') {
                Some(("duration", v)) => duration = Duration::from_secs(v.trim().parse().unwrap_or(1800)),
                Some(("switch_every", v)) => switch_every = v.trim().parse().ok().map(Duration::from_secs),
                Some(("districts", v)) => districts = v.trim() == "1",
                _ => {}
            }
        }
    }
    println!("SOAK {} scenarios, {} s, world change every {}{}", scenarios.len(), duration.as_secs(),
        switch_every.map_or("3 scenarios".to_owned(), |d| format!("{} s", d.as_secs())),
        if districts { ", districts only" } else { "" });
    Soak { scenarios, scenario: 0, tick: 0, rounds: 0, started: None, duration, switch_every, last_switch: Instant::now(), districts }
}

/// The pad state for this tick. Called by the sim thread before each step with the
/// shared lock released; at a scenario boundary it respawns the skater, cycles the
/// difficulty and every `PARK_EVERY` scenarios asks for another world.
pub(crate) fn state(shared: &Mutex<Shared>, session: &mut Session) -> XboxState {
    let neutral = XboxState { buttons: 0, triggers: [0, 0], left: [0, 0], right: [0, 0] };
    let mut guard = SOAK.lock().unwrap();
    let soak = guard.get_or_insert_with(load);
    if soak.scenarios.is_empty() {
        return neutral;
    }
    let started = *soak.started.get_or_insert_with(Instant::now);
    if started.elapsed() >= soak.duration {
        let mut s = shared.lock().unwrap();
        if !s.quit {
            println!("SOAK DONE after {} scenarios", soak.rounds);
            s.quit = true;
        }
        return neutral;
    }
    if let Some(every) = soak.switch_every {
        if soak.last_switch.elapsed() >= every {
            soak.last_switch = Instant::now();
            let mut s = shared.lock().unwrap();
            let world = next_world(s.world_loaded, soak.districts);
            println!("SOAK t={}s -> world {}", started.elapsed().as_secs(), crate::packs::WORLDS[world]);
            s.world = world;
            s.reload = true;
        }
    }
    if soak.tick >= soak.scenarios[soak.scenario].1.len() {
        soak.tick = 0;
        soak.scenario = (soak.scenario + 1) % soak.scenarios.len();
        soak.rounds += 1;
        let mut s = shared.lock().unwrap();
        s.difficulty = match soak.rounds % 3 {
            0 => Difficulty::Normal,
            1 => Difficulty::Easy,
            _ => Difficulty::Hardcore,
        };
        if soak.switch_every.is_none() && soak.rounds % PARK_EVERY == 0 {
            let world = next_world(s.world_loaded, soak.districts);
            println!("SOAK t={}s scenario #{} -> world {}", started.elapsed().as_secs(), soak.rounds, crate::packs::WORLDS[world]);
            s.world = world;
            s.reload = true;
        } else {
            drop(s);
            if let Err(e) = session.respawn() {
                println!("SOAK respawn: {e}");
            }
        }
    }
    let t = &soak.scenarios[soak.scenario].1[soak.tick];
    let state = XboxState { buttons: t.buttons, triggers: t.triggers, left: t.left, right: t.right };
    soak.tick += 1;
    state
}

/// Whether the main thread shows the pause menu now (3 s of every 2 minutes).
pub(crate) fn pause_now(elapsed: Duration) -> bool {
    (60..63).contains(&(elapsed.as_secs() % 120))
}

/// The instant replay for 6 s of every 2 minutes, half a minute after the pause.
pub(crate) fn replay_now(elapsed: Duration) -> bool {
    (90..96).contains(&(elapsed.as_secs() % 120))
}

/// The world after `world`: the next of all worlds, or with `districts` the next district
/// (worlds `PARKS..`; a park goes to the first district).
fn next_world(world: usize, districts: bool) -> usize {
    let n = crate::packs::WORLDS.len();
    if districts {
        let d = n - crate::packs::PARKS;
        crate::packs::PARKS + if world < crate::packs::PARKS { 0 } else { (world - crate::packs::PARKS + 1) % d }
    } else {
        (world + 1) % n
    }
}
