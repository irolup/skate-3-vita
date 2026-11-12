//! skate-sim: the Bevy-free simulation extracted from skate-3-rust-engine's skate-game
//! (`cb79689`).
//!
//! The module tree mirrors skate-game's crate root, so copied files keep their
//! `crate::…` paths unchanged. Only the edits listed in PROVENANCE.md were made.
//! The skate-game binary used most `pub(crate)` items from its app and renderer
//! modules, which are not copied here, hence the dead-code allowance.
#![allow(dead_code)]

// The copied files log with bevy's re-exported tracing macros (`info!`, `warn!`).
#[macro_use]
extern crate tracing;

/// A data-gated test whose private inputs are missing prints one `TEST_SKIPPED` line (also appended to the file in
/// `SKATE_TEST_SKIPS` when set) and returns (upstream `main.rs`, PR #52 `02ce8847`).
#[cfg(test)]
macro_rules! test_skip {
    ($($why:tt)*) => {{
        let line = format!("TEST_SKIPPED {}: {}", module_path!(), format_args!($($why)*));
        eprintln!("{line}");
        if let Some(path) = std::env::var_os("SKATE_TEST_SKIPS") {
            use std::io::Write as _;
            // One write per line, so parallel tests do not interleave inside a line.
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
                let _ = f.write_all(format!("{line}\n").as_bytes());
            }
        }
        return;
    }};
}

mod animation;
mod audio_events;
/// The skater's audio publication after each tick (retail audio).
pub use audio_events::Riding as AudioRiding;
/// The props' audio entries the host reads (`PropDynamics::take_audio_samples`).
pub use physics::prop_dynamics::{PropAudioSample, PropAudioType};
mod animation_pose;
mod camera;
mod career_stats;
pub mod catalog_shapes;
// The living world's items the physics names, and stand-ins for upstream's diagnostics and mod hooks.
mod frame_timing;
mod living_world;
mod modding;
pub mod object_dropper;
mod trace_all;
pub mod collision_window;
mod custom_difficulty;
mod customiser;
mod dynamics;
mod difficulty;
mod graph_host;
mod graph_runtime;
mod grind_world;
mod input;
mod physics;
mod presentation;
mod scoring_runtime;
mod session_marker;
mod park_layout;
pub mod park_bake;
pub mod park_file;
mod skate_world;
mod skater_animation;
mod transform;

/// Which `madd` arithmetic this build uses.
pub fn arithmetic() -> &'static str {
    if cfg!(feature = "no-fma") {
        "no-fma (a*b+c)"
    } else if cfg!(feature = "soft-fma") {
        "soft-fma (exact, software)"
    } else if cfg!(feature = "fma-f64") {
        "fma-f64 (double rounding, diagnostic)"
    } else {
        "mul_add"
    }
}

/// skate-core stage timers (feature `stage-profile`).
pub fn profile_take() -> Vec<(&'static str, u64, u64)> {
    #[allow(unused_mut)]
    let mut v = skate_core::profile::take();
    #[cfg(feature = "madd-count")]
    if cfg!(feature = "soft-fma") && !cfg!(feature = "no-fma") {
        let n = skate_core::madd::MADD_CALLS.swap(0, std::sync::atomic::Ordering::Relaxed);
        v.push(("n_madd_total", n, 1));
    }
    v
}

/// Live simulation session for the game loop.
pub use physics::live;
pub use physics::live::ManualView;
/// The living world's settings and the cars the renderer draws.
pub use living_world::{CarPose, KindSetting, LivingWorldSettings};
pub use living_world::npc_skaters::NpcPose;
pub use living_world::peds::{HandPropPose, PedPose};
pub use session_marker::{AUDIO_DENIED, AUDIO_OPEN, AUDIO_RETURN, AUDIO_SET, DissolveNoise, MarkerView};

/// Parity oracle harness (feature `oracle`).
#[cfg(feature = "oracle")]
pub use physics::oracle_harness;

#[cfg(test)]
#[path = "tests/action_host.rs"]
mod action_host_tests;

