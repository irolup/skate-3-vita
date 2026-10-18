//! Xbox 360 → Vita controls. Only the first box of the reference input
//! path changes: this builds the native `XboxState { buttons, triggers, left, right }`
//! from the Vita's pad and touch samples, and the reference's `xbox::convert` and
//! everything after it stay identical.
//!
//! | Xbox | Vita |
//! |---|---|
//! | sticks | sticks (centre 128 → 0, ×256, Y negated, optional per-axis gain) |
//! | A / B / X / Y | Cross / Circle / Square / Triangle |
//! | LT / RT | L / R, digital, sent as 255 (ground grabs need exactly 1.0) |
//! | LB / RB | rear touch, left / right half, held (or D-pad Left / Right) |
//! | L3 / R3 | front touch, left / right half, held while touched |
//! | D-pad, Start, Back | D-pad, Start, Select |
//!
//! Touches within [`EDGE_DEAD_BAND`] of a panel edge are ignored.
use skate_core::input::xbox::XboxState;

/// XInput button bits, as the reference's Windows transport passes them unchanged to
/// `xbox::convert` (Windows SDK `XInput.h`).
pub mod xbox {
    pub const DPAD_UP: u16 = 0x0001;
    pub const DPAD_DOWN: u16 = 0x0002;
    pub const DPAD_LEFT: u16 = 0x0004;
    pub const DPAD_RIGHT: u16 = 0x0008;
    pub const START: u16 = 0x0010;
    pub const BACK: u16 = 0x0020;
    pub const LEFT_THUMB: u16 = 0x0040;
    pub const RIGHT_THUMB: u16 = 0x0080;
    pub const LEFT_SHOULDER: u16 = 0x0100;
    pub const RIGHT_SHOULDER: u16 = 0x0200;
    pub const A: u16 = 0x1000;
    pub const B: u16 = 0x2000;
    pub const X: u16 = 0x4000;
    pub const Y: u16 = 0x8000;
}

/// `SceCtrlButtons` bits (VitaSDK `psp2common/ctrl.h`). On the handheld, L and R are
/// `LTRIGGER`/`RTRIGGER`; `L1`/`R1`/`L3`/`R3` only come from PS TV controllers.
pub mod sce {
    pub const SELECT: u32 = 0x0000_0001;
    pub const L3: u32 = 0x0000_0002;
    pub const R3: u32 = 0x0000_0004;
    pub const START: u32 = 0x0000_0008;
    pub const UP: u32 = 0x0000_0010;
    pub const RIGHT: u32 = 0x0000_0020;
    pub const DOWN: u32 = 0x0000_0040;
    pub const LEFT: u32 = 0x0000_0080;
    pub const LTRIGGER: u32 = 0x0000_0100;
    pub const RTRIGGER: u32 = 0x0000_0200;
    pub const L1: u32 = 0x0000_0400;
    pub const R1: u32 = 0x0000_0800;
    pub const TRIANGLE: u32 = 0x0000_1000;
    pub const CIRCLE: u32 = 0x0000_2000;
    pub const CROSS: u32 = 0x0000_4000;
    pub const SQUARE: u32 = 0x0000_8000;
}

/// One `SceCtrlData` sample (`sceCtrlPeekBufferPositive`, analog wide mode).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pad {
    pub buttons: u32,
    pub lx: u8,
    pub ly: u8,
    pub rx: u8,
    pub ry: u8,
}
impl Default for Pad {
    fn default() -> Self {
        Self {
            buttons: 0,
            lx: 128,
            ly: 128,
            rx: 128,
            ry: 128,
        }
    }
}

/// A touch report position, in panel units.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Touch {
    pub x: i16,
    pub y: i16,
}

/// A panel's active area (`SceTouchPanelInfo` min/maxAa).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Area {
    pub min_x: i16,
    pub min_y: i16,
    pub max_x: i16,
    pub max_y: i16,
}

/// Per-axis gain applied to a stick before `xbox::convert` (stick calibration). It only
/// exists to let a Vita stick reach the 360 stick's full range; 1.0 is unchanged.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StickGain {
    pub x: f32,
    pub y: f32,
}
impl Default for StickGain {
    fn default() -> Self {
        Self { x: 1.0, y: 1.0 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    /// LB / RB on the rear touch halves (on by default).
    pub rear_touch_bumpers: bool,
    /// Alternative: holding D-pad Left / Right acts as LB / RB (D-pad Left / Right are
    /// then not sent as D-pad).
    pub dpad_bumpers: bool,
    /// L3 / R3 on the front touch halves (on by default).
    pub front_touch_thumbs: bool,
    pub left_gain: StickGain,
    pub right_gain: StickGain,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            rear_touch_bumpers: true,
            dpad_bumpers: false,
            front_touch_thumbs: true,
            left_gain: StickGain::default(),
            right_gain: StickGain::default(),
        }
    }
}

/// Touches this close to a panel edge (fraction of its width / height) are ignored.
pub const EDGE_DEAD_BAND: f32 = 0.10;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Half {
    Left,
    Right,
}

/// Which half of the panel a touch is on, or None inside the edge dead band.
pub fn half(touch: Touch, area: Area) -> Option<Half> {
    let width = f32::from(area.max_x) - f32::from(area.min_x);
    let height = f32::from(area.max_y) - f32::from(area.min_y);
    if width <= 0.0 || height <= 0.0 {
        return None;
    }
    let u = (f32::from(touch.x) - f32::from(area.min_x)) / width;
    let v = (f32::from(touch.y) - f32::from(area.min_y)) / height;
    let inside = |t: f32| (EDGE_DEAD_BAND..=1.0 - EDGE_DEAD_BAND).contains(&t);
    if !inside(u) || !inside(v) {
        return None;
    }
    Some(if u < 0.5 { Half::Left } else { Half::Right })
}

/// A Vita stick axis (u8, centre 128) as an XInput axis: `(v - 128) * 256`, negated
/// for Y (Vita up is 0, Xbox up is positive), times the calibration gain, clamped to
/// ±32767.
pub fn axis(value: u8, negate: bool, gain: f32) -> i16 {
    let centred = (i32::from(value) - 128) * 256;
    let signed = if negate { -centred } else { centred };
    let scaled = if gain == 1.0 {
        signed
    } else {
        (signed as f32 * gain).round() as i32
    };
    scaled.clamp(-32767, 32767) as i16
}

/// The controls mapping: one Vita sample → the reference's native pad record.
pub fn map(
    pad: Pad,
    front: &[Touch],
    front_area: Area,
    rear: &[Touch],
    rear_area: Area,
    settings: &Settings,
) -> XboxState {
    let held = |mask: u32| pad.buttons & mask != 0;
    let touched = |touches: &[Touch], area: Area, side: Half| {
        touches.iter().any(|&t| half(t, area) == Some(side))
    };
    let mut buttons = 0u16;
    let mut set = |on: bool, bit: u16| {
        if on {
            buttons |= bit;
        }
    };
    set(held(sce::CROSS), xbox::A);
    set(held(sce::CIRCLE), xbox::B);
    set(held(sce::SQUARE), xbox::X);
    set(held(sce::TRIANGLE), xbox::Y);
    set(held(sce::START), xbox::START);
    set(held(sce::SELECT), xbox::BACK);
    set(held(sce::UP), xbox::DPAD_UP);
    set(held(sce::DOWN), xbox::DPAD_DOWN);
    if settings.dpad_bumpers {
        set(held(sce::LEFT), xbox::LEFT_SHOULDER);
        set(held(sce::RIGHT), xbox::RIGHT_SHOULDER);
    } else {
        set(held(sce::LEFT), xbox::DPAD_LEFT);
        set(held(sce::RIGHT), xbox::DPAD_RIGHT);
    }
    // PS TV controllers have real shoulder and stick buttons.
    set(held(sce::L1), xbox::LEFT_SHOULDER);
    set(held(sce::R1), xbox::RIGHT_SHOULDER);
    set(held(sce::L3), xbox::LEFT_THUMB);
    set(held(sce::R3), xbox::RIGHT_THUMB);
    if settings.rear_touch_bumpers {
        set(touched(rear, rear_area, Half::Left), xbox::LEFT_SHOULDER);
        set(touched(rear, rear_area, Half::Right), xbox::RIGHT_SHOULDER);
    }
    if settings.front_touch_thumbs {
        set(touched(front, front_area, Half::Left), xbox::LEFT_THUMB);
        set(touched(front, front_area, Half::Right), xbox::RIGHT_THUMB);
    }
    let trigger = |on: bool| if on { 255 } else { 0 };
    XboxState {
        buttons,
        triggers: [trigger(held(sce::LTRIGGER)), trigger(held(sce::RTRIGGER))],
        left: [
            axis(pad.lx, false, settings.left_gain.x),
            axis(pad.ly, true, settings.left_gain.y),
        ],
        right: [
            axis(pad.rx, false, settings.right_gain.x),
            axis(pad.ry, true, settings.right_gain.y),
        ],
    }
}

#[cfg(test)]
#[path = "controls_tests.rs"]
mod tests;
