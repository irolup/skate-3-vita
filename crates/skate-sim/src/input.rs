//! Platform input adapter; no animation or physics state mutation here.
//!
//! skate-game `input.rs` (`cb79689`) without its Bevy plugin and systems
//! (`InputPlugin`, `poll_controllers`, `publish_actions`) and without the XInput
//! transport: `platform` keeps only the device packet types.

mod controllers;
pub(crate) mod gesture_catalog;
mod gesture_mapping_data;
pub(crate) mod gesture_mapping;
pub(crate) mod gesture_input;
pub(crate) mod platform;
pub(crate) use controllers::{ControllerInput, ControllerStatus, RawInput};
use skate_core::input::tick::TickInput;

#[derive(Clone, Copy, Debug)]
pub(crate) struct PublishedTickInput(pub TickInput);

impl Default for PublishedTickInput {
    fn default() -> Self {
        Self(TickInput::new(
            0,
            skate_core::input::gameplay_map::GameplayActions::from_values([0.0; 18]),
            false,
        ))
    }
}


// skate_vita addition: live input publication for the game loop.
pub(crate) mod live_input;

// skate_vita addition: parity oracle harness, input half.
#[cfg(feature = "oracle")]
#[path = "../../../tools/sim-oracle/harness/oracle_input.rs"]
pub(crate) mod oracle_input;
