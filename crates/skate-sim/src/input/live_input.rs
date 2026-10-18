//! skate_vita addition: live input publication for one pad on slot 0. The same
//! steps as the oracle harness's `publish_raw` (tools/sim-oracle/harness/oracle_input.rs),
//! which the parity oracle proved against the reference: the four-slot `collect` of
//! skate-game's `poll_controllers` (slots 1–3 disconnected, packet number advancing
//! every publication, gamepad subtype 1), then `publish_actions` and `tick_input`.
use super::{
    controllers::ControllerInput,
    platform::{DeviceError, DevicePacket},
    PublishedTickInput,
};
use skate_core::input::xbox::XboxState;

pub(crate) fn publish(input: &mut ControllerInput, state: XboxState) -> PublishedTickInput {
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
