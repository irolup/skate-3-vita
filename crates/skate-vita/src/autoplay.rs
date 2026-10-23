//! Render-active benchmark (feature `replay`): if
//! `ux0:data/SKATEVITA/autoplay/*.input` exist, the simulation thread plays those
//! scenarios (sim-oracle format) in real time instead of reading the pad, through the
//! same fixed-step accumulator and snapshot publication as live play, while the main
//! thread renders as usual. For each scenario it writes the per-tick simulation times
//! (`.ticks`, u32 LE µs, as sim-bench) and the oracle dump (`.dump`) to
//! `ux0:data/SKATEVITA/autoplay-out/`, then exits the app. scripts/vita-render-bench.py
//! drives it.
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use skate_sim::live::Session;

use crate::lab::{self, Shared};

const INPUTS: &str = "ux0:data/SKATEVITA/autoplay";
const OUTPUT: &str = "ux0:data/SKATEVITA/autoplay-out";

/// Plays every scenario if there are any, then asks the main thread to exit; returns
/// when there is nothing to play.
pub(crate) fn run(shared: &Mutex<Shared>, assets: &Path) {
    let mut scenarios: Vec<PathBuf> = match std::fs::read_dir(INPUTS) {
        Ok(dir) => dir
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "input"))
            .collect(),
        Err(_) => return,
    };
    if scenarios.is_empty() {
        return;
    }
    scenarios.sort();
    let _ = std::fs::create_dir_all(OUTPUT);
    println!("AUTOPLAY {} scenarios with rendering", scenarios.len());
    if let Ok(park) = std::fs::read_to_string(format!("{INPUTS}/park.txt")) {
        println!("AUTOPLAY park {}", park.trim());
    }
    for path in scenarios {
        let name = path.file_stem().unwrap().to_string_lossy().to_string();
        if let Err(e) = play(shared, assets, &path, &name) {
            println!("AUTOPLAY ERROR {name}: {e}");
        }
    }
    println!("AUTOPLAY DONE");
    // The main thread exits the app (process exit from this thread did not close it).
    shared.lock().unwrap().quit = true;
    loop {
        std::thread::sleep(Duration::from_secs(1));
    }
}

fn play(shared: &Mutex<Shared>, assets: &Path, path: &Path, name: &str) -> Result<(), String> {
    let io = |e: std::io::Error| e.to_string();
    let scenario = skate_sim::oracle_harness::read_inputs(path)?;
    {
        let mut s = shared.lock().unwrap();
        s.loading = true;
        s.error = Some(format!("autoplay {name}: loading"));
    }
    // `autoplay/park.txt` (a park name) runs the scenarios on that park's map pack.
    let park = std::fs::read_to_string(format!("{INPUTS}/park.txt")).ok().map(|t| t.trim().to_owned()).filter(|t| !t.is_empty());
    let mut session = match &park {
        Some(name) => {
            let (sim, render) = crate::packs::load_park(name)?;
            let world = crate::packs::WORLDS.iter().position(|w| w == name).unwrap_or(0);
            shared.lock().unwrap().park_render = Some((world, Some(render)));
            Session::load_map(assets, &scenario.difficulty, Some(sim))?
        }
        None => {
            shared.lock().unwrap().park_render = Some((0, None));
            Session::load(assets, &scenario.difficulty)?
        }
    };
    // A district plays as the game runs it (`lab`): the living world with the saved settings, the Physics setting
    // and the animation budget. Its dump is not kept (the district is at the heap's limit; the ticks are measured).
    let district = park.as_deref().is_some_and(|p| crate::packs::DISTRICTS.contains(&p));
    if district {
        let (living, iterations) = {
            let s = shared.lock().unwrap();
            (s.living.clone(), s.solver_iterations)
        };
        session.set_solver_iterations(iterations);
        session.set_animation_budget(lab::ANIMATION_BUDGET);
        println!("AUTOPLAY {name}: living world {} ({iterations} solver iterations)", session.start_living_world(living));
    }
    {
        let mut s = shared.lock().unwrap();
        s.loading = false;
        s.error = Some(format!("autoplay {name}"));
        s.frames.clear();
        s.bone_names = Some(session.bone_names().to_vec());
    }
    // The dump stays in memory until the scenario ends: writing it to the memory card
    // during the run stalls the simulation thread now and then (first run: ticks
    // dropped by the accumulator, frames held), which is not what is being measured.
    let total = scenario.ticks.len();
    let _ = skate_sim::profile_take();
    let mut dump: Vec<u8> = Vec::with_capacity(if district { 0 } else { 64 + total * 2048 });
    session.write_header(&mut dump).map_err(io)?;
    let mut ticks = scenario.ticks.into_iter();
    let mut tick_us: Vec<u32> = Vec::with_capacity(total);
    let mut dropped = 0u64;
    let mut next = Instant::now();
    let mut done = false;
    let mut progress = Instant::now();
    while !done {
        if progress.elapsed() >= Duration::from_secs(2) {
            progress = Instant::now();
            let last = tick_us.last().copied().unwrap_or(0);
            let slowest = tick_us.iter().rev().take(120).copied().max().unwrap_or(0);
            println!("AUTOPLAY {name} progress tick={} last_us={last} slowest_of_120_us={slowest} dropped={dropped}", tick_us.len());
        }
        let now = Instant::now();
        let period = session.period();
        let mut due = if now >= next { (now - next).as_nanos() / period.as_nanos() + 1 } else { 0 } as u64;
        if due > 3 {
            dropped += due - 3;
            next += period * (due - 3) as u32;
            due = 3;
        }
        for _ in 0..due {
            let Some(state) = ticks.next() else {
                done = true;
                break;
            };
            let start = Instant::now();
            let report = session.step(state);
            tick_us.push(start.elapsed().as_micros() as u32);
            let due_at = next;
            next += session.period();
            if !district {
                session.write_tick(&mut dump, report.error.is_some()).map_err(io)?;
            }
            let mut s = shared.lock().unwrap();
            if let Some((previous, current)) = session.presentation() {
                lab::publish(&mut s.frames, due_at, session.period(), previous, current, &[], &[], &[], &[]);
            }
            s.ticks = session.ticks();
            s.tick_us = *tick_us.last().unwrap();
            s.dropped = dropped;
            s.state = session.state_name();
            s.speed = session.board().1;
            if report.error.is_some() {
                done = true;
                break;
            }
        }
        let now = Instant::now();
        if !done && next > now {
            std::thread::sleep(next - now);
        }
    }
    // Writing the results pauses publication like a load does (not counted as holds).
    shared.lock().unwrap().loading = true;
    std::fs::write(format!("{OUTPUT}/{name}.dump"), &dump).map_err(io)?;
    let bytes: Vec<u8> = tick_us.iter().flat_map(|us| us.to_le_bytes()).collect();
    std::fs::write(format!("{OUTPUT}/{name}.ticks"), bytes).map_err(io)?;
    let mut sorted = tick_us.clone();
    sorted.sort_unstable();
    let mean = tick_us.iter().map(|&v| u64::from(v)).sum::<u64>() as f64 / tick_us.len().max(1) as f64;
    if let Some((builds, last, max, triangles)) = session.window_stats() {
        println!("AUTOPLAY {name} collision windows: {builds} built, last {last:.0} ms, slowest {max:.0} ms, {triangles} triangles now");
    }
    println!(
        "AUTOPLAY {name} ticks={} mean_us={mean:.0} p95_us={} max_us={} dropped={dropped}",
        tick_us.len(),
        sorted.get(sorted.len() * 95 / 100).copied().unwrap_or(0),
        sorted.last().copied().unwrap_or(0),
    );
    // Feature `stage-profile`: the stage totals over the scenario.
    let mut profile = skate_sim::profile_take();
    profile.sort_by_key(|p| std::cmp::Reverse(p.1));
    let stages: Vec<String> = profile.iter().filter(|p| p.0.starts_with("z_")).take(30).map(|(n, t, _)| format!("{n}={:.2}", *t as f64 / 1e6 / total.max(1) as f64)).collect();
    if !stages.is_empty() {
        println!("AUTOPLAY {name} STAGES ms/tick: {}", stages.join(" "));
    }
    Ok(())
}
