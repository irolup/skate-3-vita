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

mod animation;
mod audio_events;
/// The skater's audio publication after each tick (retail audio).
pub use audio_events::Riding as AudioRiding;
mod animation_pose;
mod camera;
mod career_stats;
// The living world's items the physics names, and stand-ins for upstream's diagnostics and mod hooks.
mod frame_timing;
mod living_world;
mod modding;
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

