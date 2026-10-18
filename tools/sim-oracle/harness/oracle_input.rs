//! Parity oracle harness, input half.
//!
//! The same file is compiled as a child of `input` in two trees:
//! - skate-sim (feature `oracle`), from `crates/skate-sim/src/input.rs`;
//! - a scratch copy of the reference skate-game (`cfg(test)`), injected by
//!   `tools/sim-oracle/reference.sh`.
//! As a child of `input` it can reach the `pub(super)` publication functions in both.
use super::{
    controllers::ControllerInput,
    platform::{DeviceError, DevicePacket},
    PublishedTickInput,
};
use skate_core::input::xbox::XboxState;

/// One production input publication for a raw pad state on slot 0: the four-slot
/// `collect` of skate-game's `poll_controllers` (slots 1–3 disconnected, packet
/// number and subtype as in `sample_raw_for_test`), then the FixedUpdate
/// `publish_actions` system (`publish_actions` + `tick_input`) with gameplay active.
pub(crate) fn publish_raw(input: &mut ControllerInput, state: XboxState) -> PublishedTickInput {
    input.collect([
        Ok(DevicePacket {
            kind: None,
            number: input.publications as u32 + 1,
            state,
            subtype: 1,
        }),
        Err(DeviceError::Disconnected),
        Err(DeviceError::Disconnected),
        Err(DeviceError::Disconnected),
    ]);
    input.publish_actions();
    PublishedTickInput(input.tick_input())
}
