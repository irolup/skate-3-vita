//! The editor's input, camera and cursor (retail 8288C310 / 8288CE40 / 82886CC8 / 828883D8, the dropper's camera at
//! +0x1660 and cursor at +0x1450).
//!
//! Input is retail's `input.cfg` "Object Dropper" section (the GodHand actions) over the 24 native pad slots
//! (`skate_core::input::xbox::convert`: stick half-axes after the retail deadzone, triggers, buttons): moves and
//! camera turns are a half-axis cubed, elevation and zoom are the d-pad held.
//!
//! The camera is the record's free camera (owner, on retail: the right stick looks around, it does not turn about the
//! cursor): yaw and pitch turn by the mode's rates times the stick, pitch within the free limits; the zoom moves a
//! target distance the camera eases to, and the cursor's speed follows a curve of how far the camera is zoomed out.
//! The cursor stays at the middle of the screen (the screen's reticle): where the camera's look meets the cursor's
//! height. NOT RETAIL YET: where retail puts the eye is not decoded; ours stands above the cursor by the zoomed
//! distance at the start pitch's angle, and a turn moves the cursor along the ground, not the eye.
use glam::Vec3;

use super::tuning::{ModeTuning, Tuning};

/// The GodHand actions of one update.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EditorInput {
    /// `GodHand_Cursor_MoveLeft` / `Right` / `Forward` / `Back` (also `GodHand_Rotate_*`): left stick half-axes cubed.
    pub left: [f32; 4],
    /// `GodHand_Camera_RotateLeft` / `Right` / `Up` / `Down`: right stick half-axes cubed.
    pub right: [f32; 4],
    /// `GodHand_Cursor_ElevateUp` / `Down`: d-pad up / down `.held`.
    pub elevate: [bool; 2],
    /// `GodHand_Camera_ZoomIn` / `Out`: d-pad right / left `.held`.
    pub zoom: [bool; 2],
    /// `GodHand_Cursor_Simple_Move*` (also `GodHand_SimpleRotate_*`): left stick half-axes `.repeat`.
    pub step: [bool; 4],
    /// `GodHand_Cursor_Simple_ElevateUp` / `Down`: d-pad up / down `.repeat`.
    pub step_elevate: [bool; 2],
    /// D-pad left / right `.repeat` (the menus' Left / Right).
    pub step_side: [bool; 2],
    /// `GodHand_AdvancedModifier`: LT `.held`.
    pub advanced: bool,
    /// The screen's shift buttons, held: RT rotates the held item, RB moves it on its own axes, LB duplicates.
    pub rotate: bool,
    pub lock: bool,
    pub duplicate: bool,
    /// Y held (the delete's screen state lasts while it is down, `825CBB18` key 48; group select's remove).
    pub delete: bool,
    /// A held (the free camera's hold into group select, `825CBB18` / `825C84A8`; group select's add).
    pub select: bool,
    /// Buttons pressed this poll (the screen's FE inputs).
    pub pressed: Pressed,
}

/// Buttons pressed this poll (the pad record's pressed flag).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Pressed {
    pub a: bool,
    pub b: bool,
    pub x: bool,
    pub y: bool,
    /// The left stick's click (Reset).
    pub l3: bool,
    /// Back / Select (Properties).
    pub back: bool,
    /// The triggers (the catalogue's super category keys, Apt RShift2 / LShift2).
    pub rt: bool,
    pub lt: bool,
}

/// Half-axis order in [`EditorInput::left`] / [`EditorInput::right`] / [`EditorInput::step`].
pub const LEFT: usize = 0;
pub const RIGHT: usize = 1;
pub const UP: usize = 2;
pub const DOWN: usize = 3;

/// Native pad slots (`skate_core::input::xbox::convert`): d-pad up / down / left / right, LB, RB, LT, RT, the left
/// stick's right / left / up / down, the right stick's.
const DPAD: [usize; 4] = [0, 1, 2, 3];
const BACK: usize = 5;
const L3: usize = 6;
const LB: usize = 8;
const RB: usize = 9;
const LT: usize = 10;
const RT: usize = 11;
const LEFT_STICK: usize = 16;
const RIGHT_STICK: usize = 20;
const A: usize = 12;
const B: usize = 13;
const X: usize = 14;
const Y: usize = 15;

/// The editor's input state: the native pad publication (`skate_core::input::pad::Pad`, retail 82966F30), whose
/// records give `.held` (down above 0.5, a change kept back for three polls) and `.repeat` (the first poll held,
/// then after 24 polls, then every 12), the flags `input.cfg`'s expressions read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Controls {
    pad: skate_core::input::pad::Pad,
}

impl Default for Controls {
    fn default() -> Self {
        Self { pad: skate_core::input::pad::Pad::new() }
    }
}

impl Controls {
    /// One poll: the GodHand actions of these pad slots.
    pub fn read(&mut self, s: &[f32; 24]) -> EditorInput {
        self.pad.update(s);
        let records = self.pad.records();
        let held = |slot: usize| (records[slot][1] >> 8) & 255 != 0;
        let repeat = |slot: usize| records[slot][1] & 255 != 0;
        let pressed = |slot: usize| (records[slot][1] >> 24) & 255 != 0;
        let cube = |v: f32| v * v * v;
        let stick = |base: usize| [cube(s[base + 1]), cube(s[base]), cube(s[base + 2]), cube(s[base + 3])];
        let l = LEFT_STICK;
        EditorInput {
            left: stick(LEFT_STICK),
            right: stick(RIGHT_STICK),
            elevate: [held(DPAD[0]), held(DPAD[1])],
            zoom: [held(DPAD[3]), held(DPAD[2])],
            step: [repeat(l + 1), repeat(l), repeat(l + 2), repeat(l + 3)],
            step_elevate: [repeat(DPAD[0]), repeat(DPAD[1])],
            step_side: [repeat(DPAD[2]), repeat(DPAD[3])],
            advanced: held(LT),
            rotate: held(RT),
            lock: held(RB),
            duplicate: held(LB),
            delete: held(Y),
            select: held(A),
            pressed: Pressed {
                a: pressed(A),
                b: pressed(B),
                x: pressed(X),
                y: pressed(Y),
                l3: pressed(L3),
                back: pressed(BACK),
                rt: pressed(RT),
                lt: pressed(LT),
            },
        }
    }
}

/// The larger of two opposite half-axes, signed toward `positive` (8288CE40: one direction or the other, the one
/// pushed further wins).
fn dominant(positive: f32, negative: f32) -> f32 {
    if negative <= positive { positive } else { -negative }
}

/// The cursor's farthest reach ahead of the eye, in farthest zooms (NOT RETAIL: ours).
const REACH: f32 = 1.5;

/// How far the camera stays from what stops it, in metres: the radius of retail's move query (`82886500`, 0x82181B90).
pub const SWEEP_RADIUS: f32 = 0.4;
/// A surface whose normal has at least this much height is a floor or a roof, not a wall.
pub const FLOOR_NORMAL: f32 = 0.7;

/// How low D-pad Down brings the camera over the cursor, in metres (above the sweep's radius, so the ground never stops it
/// first). Retail's free camera has distance 0 (`82886CC8`, 0x82165A10): it stands at the cursor and flies down to the
/// ground; ours stands above the cursor, so it comes down as far as this.
const LOWEST_EYE: f32 = 0.8;

/// The free camera (+0x1660). Angles in radians; `pitch` is where it looks, above (+) or below (-) the horizon.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    pub yaw: f32,
    pub pitch: f32,
    pub distance: f32,
    pub target_distance: f32,
    /// The eye's angle above the cursor, seen from the cursor (the start pitch's).
    pub elevation: f32,
    /// The distance D-pad Down brought it in from, which D-pad Up goes back to before it lifts the cursor; 0 when not lowered.
    pub lowered_from: f32,
}

impl Camera {
    /// The starting camera (828863A0: the record's start distance and pitch, -20 degrees: looking down at the cursor).
    pub fn new(tuning: &Tuning, yaw: f32) -> Self {
        Self {
            yaw,
            pitch: tuning.start_pitch.clamp(tuning.free_pitch[0], tuning.free_pitch[1]),
            distance: tuning.start_distance,
            target_distance: tuning.start_distance,
            elevation: tuning.start_pitch.abs(),
            lowered_from: 0.0,
        }
    }

    /// One update of the camera's turns and zoom (8288CE40, then 82886CC8). `invert`: the Quick Menu's Invert X / Y
    /// (+0x1774 / +0x1778, 1 or -1).
    pub fn update(&mut self, tuning: &Tuning, mode: &ModeTuning, input: &EditorInput, invert: [f32; 2]) {
        // Right turns right (yaw 0 looks down -z; a free camera's turn, the orbit's was the other way).
        self.yaw -= invert[0] * mode.yaw_rate * dominant(input.right[RIGHT], input.right[LEFT]);
        self.pitch += invert[1] * mode.pitch_rate * dominant(input.right[UP], input.right[DOWN]);
        self.pitch = self.pitch.clamp(tuning.free_pitch[0], tuning.free_pitch[1]);
        let zoom = dominant(f32::from(u8::from(input.zoom[1])), f32::from(u8::from(input.zoom[0])));
        // A camera D-pad Down brought in closer than the zoom goes stays there until it zooms out.
        self.target_distance = (self.target_distance + mode.zoom_rate * zoom).clamp(tuning.distance[0].min(self.target_distance), tuning.distance[1]);
        if self.target_distance >= self.lowered_from {
            self.lowered_from = 0.0;
        }
        self.distance += (self.target_distance - self.distance) * tuning.smoothing;
    }

    /// D-pad Down with the cursor already as low as it goes (`metres` negative) brings the eye lower over the cursor; D-pad
    /// Up (positive) takes it back to where it came from.
    pub fn move_height(&mut self, metres: f32) {
        if metres == 0.0 {
            return;
        }
        let sine = self.elevation.sin().max(0.1);
        if metres < 0.0 && self.lowered_from == 0.0 {
            self.lowered_from = self.distance;
        }
        self.target_distance = (self.target_distance + metres / sine).clamp(LOWEST_EYE / sine, self.lowered_from);
        self.distance = self.target_distance;
        if metres > 0.0 && self.target_distance >= self.lowered_from {
            self.lowered_from = 0.0;
        }
    }

    /// How far the camera is zoomed out, 0 (closest) to 1 (farthest): the speed curve's input.
    pub fn zoom_fraction(&self, tuning: &Tuning) -> f32 {
        let [near, far] = tuning.distance;
        if far > near { ((self.distance - near) / (far - near)).clamp(0.0, 1.0) } else { 0.0 }
    }

    /// The camera's forward on the ground plane (unit; yaw 0 looks down -z).
    pub fn ground_forward(&self) -> Vec3 {
        Vec3::new(-self.yaw.sin(), 0.0, -self.yaw.cos())
    }

    /// How high above the cursor the eye stands.
    pub fn height(&self) -> f32 {
        self.distance * self.elevation.sin()
    }

    /// How far ahead of the eye the cursor is on the ground plane: where the look comes down to the cursor's height
    /// (at most [`REACH`] times the farthest zoom, looking level or up).
    pub fn reach(&self, tuning: &Tuning) -> f32 {
        self.reach_from(tuning, self.height())
    }

    /// How far ahead the look comes down by `drop` metres (at most [`REACH`] times the farthest zoom).
    pub fn reach_from(&self, tuning: &Tuning, drop: f32) -> f32 {
        let far = REACH * tuning.distance[1];
        if drop <= 0.0 {
            if self.pitch > 0.01 { far } else { 0.0 }
        } else if self.pitch < -0.01 {
            (drop / (-self.pitch).tan()).min(far)
        } else {
            far
        }
    }

    /// The camera's position for a cursor at `focus`.
    pub fn eye(&self, tuning: &Tuning, focus: Vec3) -> Vec3 {
        focus - self.ground_forward() * self.reach(tuning) + Vec3::Y * self.height()
    }

    /// Where the camera looks (unit).
    pub fn look(&self) -> Vec3 {
        self.ground_forward() * self.pitch.cos() + Vec3::Y * self.pitch.sin()
    }
}

/// What the cursor stands on: the height of the ground at (x, z) at or below `from` (the world's collision).
pub trait Ground {
    fn below(&self, x: f32, z: f32, from: f32) -> Option<f32>;

    /// The height of what stops D-pad Down for a cursor over a surface at height `at` at (x, z): by default the next surface
    /// under that one.
    fn floor_below(&self, x: f32, z: f32, at: f32) -> Option<f32> {
        self.below(x, z, at - UNDER)
    }

    /// Where a camera move `from` -> `to` first meets something that stops it: the fraction of the move and the
    /// surface's normal; None when nothing does.
    fn sweep(&self, _from: Vec3, _to: Vec3) -> Option<(f32, Vec3)> {
        None
    }
}

/// The cursor (+0x1450): where the next item goes, the camera's focus.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cursor {
    pub position: Vec3,
    /// Height above the ground the d-pad has lifted it (with ground following).
    pub lift: f32,
    /// The elevation ramp (+0x17A8): grows while the d-pad is held.
    pub ramp: f32,
}

/// How far under the followed surface the next one down is looked for.
pub(super) const UNDER: f32 = 0.05;

/// The elevation ramp's rest value, step per update and cap (8288C310: 0x820D06C0, 0x820994B4, 0x82063B08).
const RAMP_REST: f32 = 0.3;
const RAMP_STEP: f32 = 0.15;
const RAMP_MAX: f32 = 3.0;
/// How far under the ground Down takes the cursor in the open world (metres; retail's elevate limit is not read).
const LOWEST_UNDER_GROUND: f32 = 20.0;

impl Cursor {
    pub fn new(position: Vec3) -> Self {
        Self { position, lift: 0.0, ramp: RAMP_REST }
    }

    /// One update of the free move (8288C310, modes 1-3 analog path). `speed`: the Quick Menu's cursor speed
    /// (+0x17B0, 1 by default). `locks`: the x / y / z axis locks (+0x1770 bits 0-2).
    ///
    /// NOT RETAIL YET: the move's frame is the camera's ground forward and its right (retail builds it from the
    /// camera's forward in vector code read only in part), the `along_camera` term of `free_cam` is not applied, and
    /// the ground rule below is ours (retail 828883D8 follows the ground under a step of `ground_step` metres).
    ///
    /// Returns what of the d-pad's move the cursor did not make, for the camera: Down with nothing lower (negative), and Up
    /// while the camera is lowered (positive; it goes back first and the cursor stays).
    ///
    /// `open_world`: the open world's dropper (owner, on retail): Down goes on through the ground, and the cursor does not
    /// go where no ground is loaded (the district's collision ends about 60 m out; past it the cursor kept its height and
    /// sank into the hills, and what was placed there stood in the ground).
    pub fn update(
        &mut self,
        tuning: &Tuning,
        mode: &ModeTuning,
        camera: &Camera,
        input: &EditorInput,
        speed: f32,
        locks: [bool; 3],
        open_world: bool,
        ground: &dyn Ground,
    ) -> f32 {
        let s = mode.speed_curve.at(camera.zoom_fraction(tuning)) * speed;
        let horizontal = mode.move_scale * s;
        let vertical = mode.elevate_scale * s * self.ramp;
        let x = if locks[0] { 0.0 } else { (input.left[RIGHT] - input.left[LEFT]) * horizontal };
        let z = if locks[2] { 0.0 } else { (input.left[UP] - input.left[DOWN]) * horizontal };
        let y = if locks[1] { 0.0 } else { (f32::from(u8::from(input.elevate[0])) - f32::from(u8::from(input.elevate[1]))) * vertical };
        self.ramp = if input.elevate == [false, false] { RAMP_REST } else { (self.ramp + RAMP_STEP).min(RAMP_MAX) };
        let forward = camera.ground_forward();
        let right = Vec3::new(-forward.z, 0.0, forward.x);
        let moved = self.position + right * x + forward * z;
        if tuning.follow_ground {
            // The ground under the cursor, followed up a step of at most `ground_step` and down any drop (owner, device: off
            // the edge of something tall the cursor and the camera stayed up there; retail's 828883D8 keeps no such height);
            // the d-pad lifts above it.
            let at = self.position.y;
            let floor = at - self.lift;
            let floor = match ground.below(moved.x, moved.z, floor + tuning.ground_step) {
                Some(h) => h,
                None if open_world && (moved - self.position).length_squared() > 0.0 => return 0.0,
                None => floor,
            };
            // Below the surface it followed (Down, through a piece) it keeps its height when that surface changes: the old
            // lift from a lower surface would put it under the ground.
            if self.lift < 0.0 {
                self.lift = (at - floor).min(0.0);
            }
            // Down goes on to what stops it under that surface (`Ground::floor_below`), never past it, also when the ground
            // rises under it (owner, device: a piece snapped onto a higher one could not be lowered again; NOT RETAIL YET,
            // retail's elevate limit is not read). Nothing under it: no lower.
            let lowest = if open_world {
                -LOWEST_UNDER_GROUND
            } else {
                ground.floor_below(moved.x, moved.z, floor).map_or(self.lift.min(0.0), |h| (h - floor).min(0.0))
            };
            self.lift = self.lift.max(lowest);
            let (lift, rest) = if y > 0.0 && camera.lowered_from > 0.0 {
                (self.lift, y)
            } else {
                let lift = (self.lift + y).max(lowest);
                (lift, (self.lift + y) - lift)
            };
            self.lift = lift;
            self.position = Vec3::new(moved.x, floor + self.lift, moved.z);
            rest
        } else {
            self.position = moved + Vec3::Y * y;
            0.0
        }
    }
}
