use super::*;
use skate_core::input::xbox::convert;

/// Front panel active area of a PCH-2000 (sceTouchGetPanelInfo reports 0..1919 ×
/// 0..1087); the mapping only depends on the area it is given.
const FRONT: Area = Area {
    min_x: 0,
    min_y: 0,
    max_x: 1919,
    max_y: 1087,
};
const REAR: Area = Area {
    min_x: 0,
    min_y: 108,
    max_x: 1919,
    max_y: 889,
};

fn mapped(pad: Pad, front: &[Touch], rear: &[Touch], settings: &Settings) -> XboxState {
    map(pad, front, FRONT, rear, REAR, settings)
}

#[test]
fn stick_axes_centre_scale_negate_and_clamp() {
    assert_eq!(axis(128, false, 1.0), 0);
    assert_eq!(axis(128, true, 1.0), 0);
    assert_eq!(axis(255, false, 1.0), 32512);
    assert_eq!(axis(0, false, 1.0), -32767); // -32768 clamped
    // Y: Vita up (0) is Xbox up (positive).
    assert_eq!(axis(0, true, 1.0), 32767);
    assert_eq!(axis(255, true, 1.0), -32512);
    assert_eq!(axis(129, true, 1.0), -256);
    // Gain scales before the clamp.
    assert_eq!(axis(192, false, 1.25), 20480);
    assert_eq!(axis(255, false, 1.25), 32767);
    assert_eq!(axis(0, true, 1.25), 32767);
}

#[test]
fn full_deflection_converts_to_full_magnitude_in_every_direction() {
    // The converted magnitude must reach 0.95 at full throw. With the u8 extremes
    // the axis is ±32512..32767 before the reference's radial deadzone.
    for (lx, ly, slot) in [(255, 128, 16), (0, 128, 17), (128, 0, 18), (128, 255, 19)] {
        let state = mapped(
            Pad {
                lx,
                ly,
                ..Pad::default()
            },
            &[],
            &[],
            &Settings::default(),
        );
        let values = convert(&state, 0);
        assert!(values[slot] >= 0.95, "({lx}, {ly}) -> slot {slot} = {}", values[slot]);
    }
    let rest = convert(&mapped(Pad::default(), &[], &[], &Settings::default()), 0);
    assert!(rest[16..24].iter().all(|&v| v == 0.0));
}

#[test]
fn face_buttons_start_select_and_dpad_follow_physical_positions() {
    let cases = [
        (sce::CROSS, xbox::A),
        (sce::CIRCLE, xbox::B),
        (sce::SQUARE, xbox::X),
        (sce::TRIANGLE, xbox::Y),
        (sce::START, xbox::START),
        (sce::SELECT, xbox::BACK),
        (sce::UP, xbox::DPAD_UP),
        (sce::DOWN, xbox::DPAD_DOWN),
        (sce::LEFT, xbox::DPAD_LEFT),
        (sce::RIGHT, xbox::DPAD_RIGHT),
        (sce::L1, xbox::LEFT_SHOULDER),
        (sce::R1, xbox::RIGHT_SHOULDER),
        (sce::L3, xbox::LEFT_THUMB),
        (sce::R3, xbox::RIGHT_THUMB),
    ];
    for (vita, expected) in cases {
        let state = mapped(
            Pad {
                buttons: vita,
                ..Pad::default()
            },
            &[],
            &[],
            &Settings::default(),
        );
        assert_eq!(state.buttons, expected, "{vita:#x}");
        assert_eq!(state.triggers, [0, 0]);
    }
}

#[test]
fn l_and_r_are_full_triggers() {
    let state = mapped(
        Pad {
            buttons: sce::LTRIGGER | sce::RTRIGGER,
            ..Pad::default()
        },
        &[],
        &[],
        &Settings::default(),
    );
    assert_eq!(state.buttons, 0);
    assert_eq!(state.triggers, [255, 255]);
    // Ground grabs need exactly 1.0 after conversion.
    let values = convert(&state, 0);
    assert_eq!((values[10], values[11]), (1.0, 1.0));
}

#[test]
fn rear_halves_are_bumpers_and_front_halves_are_thumbs() {
    let left = |y| Touch { x: 500, y };
    let right = |y| Touch { x: 1500, y };
    let s = Settings::default();
    assert_eq!(mapped(Pad::default(), &[], &[left(500)], &s).buttons, xbox::LEFT_SHOULDER);
    assert_eq!(mapped(Pad::default(), &[], &[right(500)], &s).buttons, xbox::RIGHT_SHOULDER);
    assert_eq!(
        mapped(Pad::default(), &[], &[left(500), right(500)], &s).buttons,
        xbox::LEFT_SHOULDER | xbox::RIGHT_SHOULDER
    );
    assert_eq!(mapped(Pad::default(), &[left(500)], &[], &s).buttons, xbox::LEFT_THUMB);
    assert_eq!(mapped(Pad::default(), &[right(500)], &[], &s).buttons, xbox::RIGHT_THUMB);
    // Both settings off: touches do nothing.
    let off = Settings {
        rear_touch_bumpers: false,
        front_touch_thumbs: false,
        ..Settings::default()
    };
    assert_eq!(mapped(Pad::default(), &[left(500)], &[right(500)], &off).buttons, 0);
}

#[test]
fn touches_near_the_edges_are_ignored() {
    let s = Settings::default();
    // 10% of 1919 is ~192; of the rear's 781-unit height ~78 (108 + 78 = 186).
    for t in [
        Touch { x: 150, y: 500 },
        Touch { x: 1800, y: 500 },
        Touch { x: 960, y: 150 },
        Touch { x: 960, y: 860 },
    ] {
        assert_eq!(mapped(Pad::default(), &[], &[t], &s).buttons, 0, "{t:?}");
    }
    assert_eq!(half(Touch { x: 200, y: 500 }, REAR), Some(Half::Left));
    assert_eq!(half(Touch { x: 1700, y: 500 }, REAR), Some(Half::Right));
    let empty = Area {
        min_x: 0,
        min_y: 0,
        max_x: 0,
        max_y: 0,
    };
    assert_eq!(half(Touch { x: 0, y: 0 }, empty), None);
}

#[test]
fn dpad_bumper_alternative_replaces_dpad_left_and_right() {
    let s = Settings {
        dpad_bumpers: true,
        ..Settings::default()
    };
    let pad = |buttons| Pad {
        buttons,
        ..Pad::default()
    };
    assert_eq!(mapped(pad(sce::LEFT), &[], &[], &s).buttons, xbox::LEFT_SHOULDER);
    assert_eq!(mapped(pad(sce::RIGHT), &[], &[], &s).buttons, xbox::RIGHT_SHOULDER);
    assert_eq!(mapped(pad(sce::UP | sce::DOWN), &[], &[], &s).buttons, xbox::DPAD_UP | xbox::DPAD_DOWN);
}

#[test]
fn sticks_and_gains_reach_the_state() {
    let s = Settings {
        left_gain: StickGain { x: 1.0, y: 1.1 },
        ..Settings::default()
    };
    let state = mapped(
        Pad {
            buttons: 0,
            lx: 200,
            ly: 60,
            rx: 10,
            ry: 250,
        },
        &[],
        &[],
        &s,
    );
    assert_eq!(state.left, [72 * 256, 19149]); // 68 * 256 * 1.1 = 19148.8
    assert_eq!(state.right, [-118 * 256, -122 * 256]);
}
