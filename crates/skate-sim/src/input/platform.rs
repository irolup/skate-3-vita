//! Device packet types from skate-game input/platform.rs (cb79689). The Windows XInput
//! transport is not copied; the Vita supplies packets from SceCtrl (vita-platform).
use skate_core::input::xbox::XboxState;

pub(crate) struct DevicePacket {
    pub number: u32,
    pub state: XboxState,
    pub subtype: u8,
    /// Who is in the slot (b3c9679). Metadata only: gameplay reads `state`/`subtype`.
    /// The Vita has no controller identities, so it is always `None`.
    pub kind: Option<std::sync::Arc<ControllerKind>>,
}

/// skate_vita stand-in for the reference's `input::controller_kind::ControllerKind`
/// (SDL/XInput pad identity), which the Vita does not have.
#[derive(Debug)]
pub(crate) struct ControllerKind;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DeviceError {
    Disconnected,
    State(u32),
    Capabilities(u32),
    #[cfg(not(windows))]
    UnsupportedPlatform,
}
