//! skate vita platform layer. Everything that touches the Vita's
//! system libraries lives here (and in `vita-render` / `skate-vita`), so `skate-core`
//! and `skate-sim` stay platform-agnostic.
//!
//! - [`controls`]: the Xbox 360 → Vita mapping, as a pure function from pad and
//!   touch samples to the reference's `XboxState`. It builds and is tested on the
//!   desktop.
//! - [`vita`] (Vita only): pad/touch polling, power clocks, thread placement, idle
//!   timers and the data root.
//! - [`ime`] (Vita only): the system keyboard.
pub mod controls;
#[cfg(target_os = "vita")]
pub mod ime;
#[cfg(target_os = "vita")]
pub mod vita;

/// Data root on the memory card. Assets live in `assets/`, settings in the root.
pub const DATA_ROOT: &str = "ux0:data/SKATEVITA";
