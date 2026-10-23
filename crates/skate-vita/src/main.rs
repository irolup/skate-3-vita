#[cfg(target_os = "vita")]
mod app;
#[cfg(all(target_os = "vita", feature = "replay"))]
mod autoplay;
#[cfg(all(target_os = "vita", feature = "soak"))]
mod soak;
#[cfg(target_os = "vita")]
mod lab;
#[cfg(target_os = "vita")]
mod packs;
#[cfg(target_os = "vita")]
mod fe_menus;
#[cfg(target_os = "vita")]
mod retail_hud;
mod alloc;
mod audio;
#[cfg(target_os = "vita")]
mod retail_audio;
#[cfg(target_os = "vita")]
mod district_stream;
mod hud;
mod menu;
mod button_icons;
mod replay_camera;
mod session_replay;
mod world_replay;
mod teleporters;
#[cfg(target_os = "vita")]
mod colour_grade;
mod settings;
mod vita_menus;
mod trick_demo;
mod skin;
#[cfg(feature = "stage-profile")]
mod span_profile;

/// Small blocks are cached per thread: no lock on the hot path (see `alloc.rs`).
#[cfg(target_os = "vita")]
#[global_allocator]
static ALLOC: alloc::Cached = alloc::Cached;

#[cfg(target_os = "vita")]
use vita_newlib_shims as _;

// Memory layout. Stack, newlib heap and sceLibc heap all live in MAIN.
#[cfg(target_os = "vita")]
#[used]
#[unsafe(export_name = "sceUserMainThreadStackSize")]
pub static STACK: u32 = 2 * 1024 * 1024;
#[cfg(target_os = "vita")]
#[used]
#[unsafe(export_name = "_newlib_heap_size_user")]
pub static HEAP: u32 = 304 * 1024 * 1024;
#[cfg(target_os = "vita")]
#[used]
#[unsafe(export_name = "sceLibcHeapSize")]
pub static LIBC: u32 = 8 * 1024 * 1024;

#[cfg(target_os = "vita")]
fn main() {
    // The 3D view of the live simulation, with the input lab one chord away,
    // and the game loop.
    #[cfg(feature = "stage-profile")]
    span_profile::install();
    app::run();
}

#[cfg(not(target_os = "vita"))]
fn main() {
    println!(
        "skate vita {}: this binary runs on the PS Vita; build it with `cargo vita build vpk -- --release`",
        env!("CARGO_PKG_VERSION")
    );
}
