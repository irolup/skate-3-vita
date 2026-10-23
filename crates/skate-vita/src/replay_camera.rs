//! The replay editor's cameras: the retail Game, Follow, Tripod and Free cameras and their
//! advanced versions (key markers the camera blends between), evaluated over the recorded frames.
//!
//! The retail help describes them: the Game camera is the camera of play with a zoom; the Follow
//! camera follows and points at the skater (angle, distance, zoom, aim on the body); the Tripod
//! camera stays in one spot and points at the skater, and gives way to the Game camera when the
//! skater is too far; the Free camera is a fixed shot. How they move is ours (the retail
//! cameras are native code): the Follow angle is relative to the skater's direction of travel.
#![cfg_attr(not(target_os = "vita"), allow(dead_code))]
use glam::{EulerRot, Quat, Vec3};
use skate_sim::live::Transform;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Game,
    Follow,
    Tripod,
    Free,
}

impl Kind {
    pub fn code(self) -> u8 {
        self as u8
    }
    pub fn from_code(code: u8) -> Option<Self> {
        [Kind::Game, Kind::Follow, Kind::Tripod, Kind::Free].get(usize::from(code)).copied()
    }
}

/// One camera setting; the fields a kind uses:
/// - Game: `fov` (zoom: `BASE_FOV` is the recorded camera unchanged);
/// - Follow: `yaw` and `pitch` (the orbit around the skater, yaw 0 behind the direction of
///   travel), `distance`, `aim` (height on the skater), `fov`;
/// - Tripod: `position`, `aim`, `fov`;
/// - Free: `position`, `yaw` and `pitch` (where it looks), `fov`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cam {
    pub position: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub distance: f32,
    pub aim: f32,
    pub fov: f32,
}

/// The field of view that counts as zoom 1 (and the Game camera's "as recorded").
pub const BASE_FOV: f32 = 0.9;
/// The Tripod camera gives way to the Game camera beyond this (metres).
const TRIPOD_RANGE: f32 = 40.0;
/// The default aim: about the skater's chest (metres above the root).
const AIM: f32 = 1.0;

fn wrap(a: f32) -> f32 {
    let t = std::f32::consts::TAU;
    (a + std::f32::consts::PI).rem_euclid(t) - std::f32::consts::PI
}

fn lerp_angle(a: f32, b: f32, t: f32) -> f32 {
    a + wrap(b - a) * t
}

impl Cam {
    pub fn to_array(&self) -> [f32; 8] {
        [self.position.x, self.position.y, self.position.z, self.yaw, self.pitch, self.distance, self.aim, self.fov]
    }

    pub fn from_array(v: [f32; 8]) -> Self {
        Self { position: Vec3::new(v[0], v[1], v[2]), yaw: v[3], pitch: v[4], distance: v[5], aim: v[6], fov: v[7] }
    }

    fn blend(&self, other: &Cam, t: f32) -> Cam {
        Cam {
            position: self.position.lerp(other.position, t),
            yaw: lerp_angle(self.yaw, other.yaw, t),
            pitch: self.pitch + (other.pitch - self.pitch) * t,
            distance: self.distance + (other.distance - self.distance) * t,
            aim: self.aim + (other.aim - self.aim) * t,
            fov: self.fov + (other.fov - self.fov) * t,
        }
    }

    /// The camera of a kind that starts where the recorded camera is: the skater at `skater`,
    /// travelling towards `behind + PI` (see [`behind`]).
    pub fn starting(kind: Kind, recorded: &Transform, recorded_fov: f32, skater: Vec3, behind: f32) -> Cam {
        let (look_yaw, look_pitch, _) = recorded.rotation.to_euler(EulerRot::YXZ);
        let target = skater + Vec3::Y * AIM;
        let offset = recorded.translation - target;
        let distance = offset.length().clamp(1.5, 30.0);
        let orbit_yaw = offset.x.atan2(offset.z);
        let orbit_pitch = (offset.y / offset.length().max(1e-3)).clamp(-1.0, 1.0).asin();
        match kind {
            Kind::Game => Cam { position: recorded.translation, yaw: 0.0, pitch: 0.0, distance, aim: AIM, fov: BASE_FOV },
            Kind::Follow => Cam { position: recorded.translation, yaw: wrap(orbit_yaw - behind), pitch: orbit_pitch, distance, aim: AIM, fov: recorded_fov },
            Kind::Tripod => Cam { position: recorded.translation, yaw: orbit_yaw, pitch: orbit_pitch, distance, aim: AIM, fov: recorded_fov },
            Kind::Free => Cam { position: recorded.translation, yaw: look_yaw, pitch: look_pitch, distance, aim: AIM, fov: recorded_fov },
        }
    }
}

/// A camera at `eye` looking at `target` (the convention of the game's cameras: -Z forward).
fn look_at(eye: Vec3, target: Vec3) -> Transform {
    let f = (target - eye).normalize_or_zero();
    let yaw = (-f.x).atan2(-f.z);
    let pitch = f.y.clamp(-1.0, 1.0).asin();
    Transform { translation: eye, rotation: Quat::from_euler(EulerRot::YXZ, yaw, pitch, 0.0), scale: Vec3::ONE }
}

/// The world angle about +Y of "behind the skater" at a tick: opposite the travel over half a
/// second around it, turning towards the recorded camera's side as the skater slows (standing
/// still, the camera of play decides).
pub fn behind(travel: Vec3, skater: Vec3, recorded: &Transform) -> f32 {
    let from_camera = recorded.translation - skater;
    let still = from_camera.x.atan2(from_camera.z);
    let speed = Vec3::new(travel.x, 0.0, travel.z).length();
    if speed < 1e-4 {
        return still;
    }
    let moving = (-travel.x).atan2(-travel.z);
    let weight = ((speed - 0.2) / 0.6).clamp(0.0, 1.0);
    lerp_angle(still, moving, weight)
}

/// A clip's camera: its kind, whether it is the advanced version and its setting, or (advanced)
/// its keys by tick, sorted.
#[derive(Clone, Debug, PartialEq)]
pub struct ClipCamera {
    pub kind: Kind,
    pub advanced: bool,
    pub cam: Cam,
    pub keys: Vec<(f32, Cam)>,
}

impl ClipCamera {
    pub fn game() -> Self {
        Self { kind: Kind::Game, advanced: false, cam: Cam { position: Vec3::ZERO, yaw: 0.0, pitch: 0.0, distance: 4.0, aim: AIM, fov: BASE_FOV }, keys: Vec::new() }
    }

    /// The setting at a tick: the keys blended (smoothly) for an advanced camera, else the one setting.
    pub fn at(&self, tick: f32) -> Cam {
        if !self.advanced || self.keys.is_empty() {
            return self.cam;
        }
        let next = self.keys.iter().position(|(t, _)| *t > tick);
        match next {
            Some(0) => self.keys[0].1,
            None => self.keys[self.keys.len() - 1].1,
            Some(i) => {
                let (t0, a) = self.keys[i - 1];
                let (t1, b) = self.keys[i];
                let x = ((tick - t0) / (t1 - t0).max(1e-3)).clamp(0.0, 1.0);
                a.blend(&b, x * x * (3.0 - 2.0 * x))
            }
        }
    }

    /// Sets (or replaces, within `near` ticks) a key.
    pub fn set_key(&mut self, tick: f32, cam: Cam, near: f32) {
        if let Some(k) = self.keys.iter_mut().find(|(t, _)| (t - tick).abs() <= near) {
            *k = (tick, cam);
        } else {
            let at = self.keys.iter().position(|(t, _)| *t > tick).unwrap_or(self.keys.len());
            self.keys.insert(at, (tick, cam));
        }
    }
}

/// The camera transform and field of view of a kind's setting for one recorded frame: the skater
/// at `skater`, `behind` from [`behind`], the frame's own camera `recorded`.
pub fn eval(kind: Kind, cam: &Cam, skater: Vec3, behind: f32, recorded: (Transform, f32)) -> (Transform, f32) {
    let (mut view, fov) = eval_free(kind, cam, skater, behind, recorded);
    view.translation = leash(view.translation, skater);
    (view, fov)
}

/// Retail's replay camera leash (`82682CB0`, research b381 / b449): every evaluated replay camera farther than
/// [`LEASH`] from the skater's recorded position is pulled back onto that sphere (its rotation kept).
/// NOT RETAIL YET: retail's swept-sphere collision after it (`82682D80`) is not ported: a district streams its
/// collision around the live skater, not around the replay's.
pub fn leash(eye: Vec3, skater: Vec3) -> Vec3 {
    let offset = eye - skater;
    let d = offset.length();
    if d > LEASH { skater + offset * (LEASH / d) } else { eye }
}

/// The 40 m leash (constant at 0x821EB794).
pub const LEASH: f32 = 40.0;

// The replay cameras' world constraint (skate-game `replay/camera_collision.rs`, PR #52 `b110bab`, research b449):
// retail runs it on every evaluated replay camera (`82C8B698` -> `82683680`): the 40 m leash, then a swept sphere from
// the last accepted position to the new one with a slide along the hit plane, plus a line of sight from the player
// (`82682D80`). NOT RETAIL YET (as upstream): the 0.8 m / world radius triggers, the sweep's collision layers (the map
// and its external queries, no actors) and retail's snap request (ours: a camera cut). skate_vita: the sweep is the
// paused session's collision world (`Session::camera_world`): a district's streamed window around the live skater,
// so a replay camera outside it passes through as before.

/// The constraint's data; defaults are retail's.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CameraCollision {
    /// Off = the cameras pass through the world (a mod's noclip shots); the leash still holds.
    pub enabled: bool,
    /// Sphere radius (m) times its scale: retail 0.1 x 1.3 (0x820641A8, 0x82099484).
    pub radius: f32,
    pub radius_scale: f32,
    /// A move with no component above this skips the query (FLT_EPSILON, 0x821EB480).
    pub move_epsilon: f32,
    /// How far the camera may go from the player, m (0x821EB794).
    pub leash: f32,
}

impl Default for CameraCollision {
    fn default() -> Self {
        Self { enabled: true, radius: 0.1, radius_scale: 1.3, move_epsilon: f32::EPSILON, leash: 40.0 }
    }
}

/// A sweep hit: the fraction along the segment and the hit normal.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    pub fraction: f32,
    pub normal: Vec3,
}

/// The constrained camera position, and whether the camera should be turned back to its look point (a non-Free camera
/// that slid while the player is hidden, retail `82670600`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Constrained {
    pub position: Vec3,
    pub relook: bool,
}

/// `new` constrained against the last accepted position `old` (None = a camera cut: only the line of sight is checked
/// and the camera is pulled in front of an occluder, our stand-in for retail's snap request), the player `anchor`.
/// `free` = retail camera type 2 (keeps its rotation). `sweep(from, to, radius)` is the world query.
pub fn constrain(old: Option<Vec3>, new: Vec3, anchor: Vec3, free: bool, rules: &CameraCollision,
    mut sweep: impl FnMut(Vec3, Vec3, f32) -> Option<Hit>) -> Constrained {
    let offset = new - anchor;
    let new = if offset.length() > rules.leash { anchor + offset.normalize_or_zero() * rules.leash } else { new };
    let keep = Constrained { position: new, relook: false };
    if !rules.enabled {
        return keep;
    }
    let radius = rules.radius * rules.radius_scale;
    let Some(old) = old else {
        return match sweep(anchor, new, radius) {
            Some(hit) => Constrained { position: anchor + (new - anchor) * hit.fraction, relook: false },
            None => keep,
        };
    };
    let delta = new - old;
    if delta.abs().max_element() <= rules.move_epsilon {
        return keep;
    }
    let Some(hit) = sweep(old, new, radius) else { return keep };
    let slid = new - hit.normal * delta.dot(hit.normal);
    let position = if sweep(old, slid, radius).is_none() { slid } else { old };
    // Retail queues the line of sight to the new position with the motion sweep (sweep B).
    let hidden = sweep(anchor, new, radius).is_some();
    Constrained { position, relook: hidden && !free }
}



fn eval_free(kind: Kind, cam: &Cam, skater: Vec3, behind: f32, recorded: (Transform, f32)) -> (Transform, f32) {
    let target = skater + Vec3::Y * cam.aim;
    match kind {
        Kind::Game => (recorded.0, recorded.1 * cam.fov / BASE_FOV),
        Kind::Free => (
            Transform { translation: cam.position, rotation: Quat::from_euler(EulerRot::YXZ, cam.yaw, cam.pitch, 0.0), scale: Vec3::ONE },
            cam.fov,
        ),
        Kind::Follow => {
            let angle = behind + cam.yaw;
            let offset = Vec3::new(angle.sin() * cam.pitch.cos(), cam.pitch.sin(), angle.cos() * cam.pitch.cos()) * cam.distance;
            (look_at(target + offset, target), cam.fov)
        }
        Kind::Tripod => {
            if cam.position.distance(target) > TRIPOD_RANGE {
                recorded
            } else {
                (look_at(cam.position, target), cam.fov)
            }
        }
    }
}

/// The controls a camera edit reads, already in the Xbox layout (see `session_replay::Controls`).
pub struct Input {
    pub left: [f32; 2],
    pub right: [f32; 2],
    pub up: bool,
    pub down: bool,
    pub zoom_in: bool,
    pub zoom_out: bool,
    pub fast: bool,
}

/// The key edit's rates as retail's editor has them (upstream `replay/camera_key.rs` `CameraRules` at `74c124a`,
/// research b402 / b549), per console frame at `CONSOLE_HZ`: orbits turn at stick x 2.5 degrees capped at 3, pitch
/// stops at 85 degrees; distance and free moves are the stick clamped to 0.2 m (full speed from 20 % stick); the free
/// camera turns at stick x 5 degrees capped at 1.6 and moves level along its heading; its height eases; the look
/// height steps 0.01 m within 1 m of the default aim; the lens steps 0.5 mm within 24-75 mm.
const CONSOLE_HZ: f32 = 30.0;
const ORBIT_DEG_PER_UNIT: f32 = 2.5;
const ORBIT_DEG_CAP: f32 = 3.0;
const PITCH_LIMIT: f32 = 85.0 * std::f32::consts::PI / 180.0;
const DISTANCE_STEP: f32 = 0.2;
const LOOK_STEP: f32 = 0.01;
const LOOK_RANGE: f32 = 1.0;
const FREE_DEG_PER_UNIT: f32 = 5.0;
const FREE_DEG_CAP: f32 = 1.6;
const FREE_MOVE: f32 = 0.2;
const FREE_HEIGHT_STEP: f32 = 0.015;
const FREE_HEIGHT_CAP: f32 = 0.2;
const FREE_HEIGHT_DECAY: f32 = 0.6;
const FREE_HEIGHT_RANGE: f32 = 10.0;
const LENS_STEP: f32 = 0.5;
const LENS_RANGE: (f32, f32) = (24.0, 75.0);

/// The free camera's eased height during one edit (retail S+0x7C): its velocity and the offset added since the
/// edit began.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FreeHeight {
    velocity: f32,
    offset: f32,
}

/// One frame of editing a setting with the sticks and the D-pad (the retail editor's icons: right
/// stick rotates, left stick moves in and out, the D-pad up and down moves up and down, left and
/// right zoom). `anchor` is the skater's aim point where the edit started (the Tripod orbits it).
/// Ours: the stick dead zone, LB moving three times as far, the Follow and Tripod distance ranges (the 40 m leash).
pub fn edit(kind: Kind, cam: &mut Cam, dt: f32, i: &Input, anchor: Vec3, height: &mut FreeHeight) -> bool {
    let dead = |v: f32| if v.abs() > 0.15 { v } else { 0.0 };
    let (lx, ly, rx, ry) = (dead(i.left[0]), dead(i.left[1]), dead(i.right[0]), dead(i.right[1]));
    let vertical = f32::from(i.up) - f32::from(i.down);
    let zoom = f32::from(i.zoom_out) - f32::from(i.zoom_in);
    let fast = if i.fast { 3.0 } else { 1.0 };
    let frames = dt * CONSOLE_HZ;
    let cap = |v: f32, c: f32| v.clamp(-c, c);
    let orbit = |stick: f32| cap(stick * ORBIT_DEG_PER_UNIT, ORBIT_DEG_CAP).to_radians() * frames;
    let step = |stick: f32| cap(stick, DISTANCE_STEP) * fast * frames;
    let look = |aim: f32| (aim + vertical * LOOK_STEP * frames).clamp(AIM - LOOK_RANGE, AIM + LOOK_RANGE);
    let before = *cam;
    // The lens in millimetres on a 35 mm frame (24 mm tall), as `zoom_text` shows it.
    let lens = (12.0 / (cam.fov * 0.5).tan() - zoom * LENS_STEP * frames).clamp(LENS_RANGE.0, LENS_RANGE.1);
    cam.fov = 2.0 * (12.0 / lens).atan();
    match kind {
        Kind::Game => {}
        Kind::Free => {
            cam.yaw -= cap(rx * FREE_DEG_PER_UNIT, FREE_DEG_CAP).to_radians() * frames;
            cam.pitch = (cam.pitch + cap(ry * FREE_DEG_PER_UNIT, FREE_DEG_CAP).to_radians() * frames).clamp(-PITCH_LIMIT, PITCH_LIMIT);
            let heading = Quat::from_rotation_y(cam.yaw);
            cam.position += heading * Vec3::new(cap(lx, FREE_MOVE), 0.0, -cap(ly, FREE_MOVE)) * fast * frames;
            // The height eases: the velocity gains a step, is applied, then decays (per console frame).
            height.velocity = cap(height.velocity + vertical * FREE_HEIGHT_STEP * frames, FREE_HEIGHT_CAP);
            let offset = (height.offset + height.velocity * frames).clamp(-FREE_HEIGHT_RANGE, FREE_HEIGHT_RANGE);
            cam.position.y += offset - height.offset;
            height.offset = offset;
            height.velocity *= FREE_HEIGHT_DECAY.powf(frames);
        }
        Kind::Follow => {
            cam.yaw = wrap(cam.yaw - orbit(rx));
            cam.pitch = (cam.pitch - orbit(ry)).clamp(-PITCH_LIMIT, PITCH_LIMIT);
            cam.distance = (cam.distance - step(ly)).clamp(1.0, 30.0);
            cam.aim = look(cam.aim);
        }
        Kind::Tripod => {
            // Orbit the spot around the skater where the edit started; in and out along the line. The lift moves the
            // look point only (retail `82C80EA0`).
            let target = anchor + Vec3::Y * (cam.aim - AIM);
            let offset = cam.position - target;
            let mut r = offset.length().max(0.5);
            let mut yaw = offset.x.atan2(offset.z);
            let mut pitch = (offset.y / r).clamp(-1.0, 1.0).asin();
            yaw -= orbit(rx);
            pitch = (pitch - orbit(ry)).clamp(-PITCH_LIMIT, PITCH_LIMIT);
            r = (r - step(ly)).clamp(1.0, TRIPOD_RANGE - 1.0);
            cam.position = target + Vec3::new(yaw.sin() * pitch.cos(), pitch.sin(), yaw.cos() * pitch.cos()) * r;
            cam.aim = look(cam.aim);
        }
    }
    *cam != before
}

/// The zoom as the retail editor shows it (`ID_REPLAYEDITOR_ZOOM`, "Zoom {0:d} mm"): the focal
/// length on a 35 mm frame (24 mm tall) for the vertical field of view.
pub fn zoom_text(fov: f32) -> String {
    format!("Zoom {:.0} mm", 12.0 / (fov * 0.5).tan())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cam(yaw: f32) -> Cam {
        Cam { position: Vec3::ZERO, yaw, pitch: 0.0, distance: 4.0, aim: 1.0, fov: 0.9 }
    }

    /// Retail's rates (b549): the free camera turns at the 1.6 degree cap from 32 % stick, moves at most 0.2 m per
    /// axis per frame, its height eases to 0.0375 m a frame; the lens steps 0.5 mm a frame and stops at 75 mm.
    #[test]
    fn the_key_edit_takes_retail_rates() {
        let mut c = cam(0.0);
        let mut height = FreeHeight::default();
        let input = Input { left: [0.2, 0.5], right: [0.32, 0.0], up: true, down: false, zoom_in: true, zoom_out: false, fast: false };
        for _ in 0..60 {
            edit(Kind::Free, &mut c, 1.0 / 30.0, &input, Vec3::ZERO, &mut height);
        }
        assert!((c.yaw + 96f32.to_radians()).abs() < 1e-4, "{}", c.yaw.to_degrees());
        assert!((height.velocity / FREE_HEIGHT_DECAY - 0.0375).abs() < 1e-4, "{height:?}");
        assert!(c.position.y > 0.0 && c.position.y < 60.0 * 0.0375 + 1e-3);
        assert!(glam::Vec2::new(c.position.x, c.position.z).length() <= 60.0 * 0.2 * 2f32.sqrt() + 1e-3);
        assert_eq!(zoom_text(c.fov), "Zoom 55 mm", "0.5 mm a frame from 24.6 mm");
        for _ in 0..60 {
            edit(Kind::Free, &mut c, 1.0 / 30.0, &input, Vec3::ZERO, &mut height);
        }
        assert_eq!(zoom_text(c.fov), "Zoom 75 mm", "the lens stops at 75 mm");
    }

    #[test]
    fn keys_blend_and_hold_at_the_ends() {
        let mut c = ClipCamera { kind: Kind::Follow, advanced: true, cam: cam(0.0), keys: Vec::new() };
        c.set_key(10.0, cam(0.0), 2.0);
        c.set_key(30.0, cam(1.0), 2.0);
        assert_eq!(c.at(0.0).yaw, 0.0);
        assert_eq!(c.at(40.0).yaw, 1.0);
        assert!((c.at(20.0).yaw - 0.5).abs() < 1e-5);
        c.set_key(31.0, cam(2.0), 2.0);
        assert_eq!(c.keys.len(), 2);
        assert_eq!(c.at(40.0).yaw, 2.0);
    }

    #[test]
    fn the_leash_pulls_a_far_camera_onto_40_m() {
        let skater = Vec3::new(10.0, 0.0, 0.0);
        assert_eq!(leash(Vec3::new(20.0, 0.0, 0.0), skater), Vec3::new(20.0, 0.0, 0.0));
        let pulled = leash(Vec3::new(110.0, 0.0, 0.0), skater);
        assert!((pulled - Vec3::new(50.0, 0.0, 0.0)).length() < 1e-4, "{pulled:?}");
        let mut far = cam(0.0);
        far.position = Vec3::new(0.0, 0.0, 500.0);
        let (view, _) = eval(Kind::Free, &far, Vec3::ZERO, 0.0, (Transform::IDENTITY, 1.0));
        assert!((view.translation.length() - LEASH).abs() < 1e-3);
    }

    #[test]
    fn follow_looks_at_the_skater_from_behind() {
        let skater = Vec3::new(5.0, 0.0, 5.0);
        // Travelling towards -Z: behind is +Z.
        let b = behind(Vec3::new(0.0, 0.0, -2.0), skater, &Transform::IDENTITY);
        let c = Cam { position: Vec3::ZERO, yaw: 0.0, pitch: 0.0, distance: 4.0, aim: 1.0, fov: 0.9 };
        let (t, _) = eval(Kind::Follow, &c, skater, b, (Transform::IDENTITY, 0.9));
        assert!((t.translation - Vec3::new(5.0, 1.0, 9.0)).length() < 1e-4);
        let forward = t.rotation * Vec3::NEG_Z;
        assert!((forward - Vec3::NEG_Z).length() < 1e-4);
    }
}

#[cfg(test)]
mod collision_tests {
    use super::*;

    /// A wall at x = 1 facing -x.
    fn wall(from: Vec3, to: Vec3, r: f32) -> Option<Hit> {
        let plane = 1.0 - r;
        (from.x < plane && to.x >= plane).then(|| Hit { fraction: (plane - from.x) / (to.x - from.x), normal: Vec3::NEG_X })
    }

    #[test]
    fn slides_along_a_wall() {
        let c = constrain(Some(Vec3::ZERO), Vec3::new(2.0, 0.0, 1.0), Vec3::new(-1.0, 0.0, 0.0), true, &Default::default(), wall);
        assert_eq!(c.position, Vec3::new(0.0, 0.0, 1.0));
    }

    #[test]
    fn a_blocked_slide_stops_at_the_old_position() {
        let corner = |from: Vec3, to: Vec3, r: f32| wall(from, to, r).or_else(|| (to.z > 0.5).then_some(Hit { fraction: 0.5, normal: Vec3::NEG_Z }));
        let c = constrain(Some(Vec3::ZERO), Vec3::new(2.0, 0.0, 1.0), Vec3::new(-1.0, 0.0, 0.0), true, &Default::default(), corner);
        assert_eq!(c.position, Vec3::ZERO);
    }

    #[test]
    fn no_motion_runs_no_query() {
        let mut calls = 0;
        let p = Vec3::new(0.5, 0.0, 0.0);
        let c = constrain(Some(p), p, Vec3::ZERO, false, &Default::default(), |_, _, _| { calls += 1; None });
        assert_eq!((c.position, calls), (p, 0));
    }

    #[test]
    fn a_cut_behind_a_wall_pulls_in_front() {
        let c = constrain(None, Vec3::new(3.0, 0.0, 0.0), Vec3::ZERO, false, &Default::default(), wall);
        assert!((c.position.x - (1.0 - 0.13)).abs() < 1e-5, "{c:?}");
    }

    #[test]
    fn leash_first_then_collision() {
        let mut seen = Vec::new();
        constrain(Some(Vec3::ZERO), Vec3::new(0.0, 0.0, 50.0), Vec3::ZERO, true, &Default::default(), |_, to, r| { seen.push((to, r)); None });
        assert_eq!(seen, vec![(Vec3::new(0.0, 0.0, 40.0), 0.1 * 1.3)]);
    }

    #[test]
    fn a_hidden_player_turns_a_non_free_camera_back() {
        let behind = |from: Vec3, to: Vec3, r: f32| wall(from, to, r).or_else(|| (from == Vec3::new(5.0, 0.0, 0.0)).then_some(Hit { fraction: 0.5, normal: Vec3::X }));
        let anchor = Vec3::new(5.0, 0.0, 0.0);
        assert!(constrain(Some(Vec3::ZERO), Vec3::new(2.0, 0.0, 1.0), anchor, false, &Default::default(), behind).relook);
        assert!(!constrain(Some(Vec3::ZERO), Vec3::new(2.0, 0.0, 1.0), anchor, true, &Default::default(), behind).relook);
    }

    #[test]
    fn disabled_passes_through() {
        let rules = CameraCollision { enabled: false, ..Default::default() };
        let c = constrain(Some(Vec3::ZERO), Vec3::new(2.0, 0.0, 0.0), Vec3::new(-1.0, 0.0, 0.0), true, &rules, wall);
        assert_eq!(c.position, Vec3::new(2.0, 0.0, 0.0));
    }
}
