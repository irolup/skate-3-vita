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

/// One frame of editing a setting with the sticks and the D-pad (the retail editor's icons: right
/// stick rotates, left stick moves in and out, the D-pad up and down moves up and down, left and
/// right zoom). `anchor` is the skater's aim point where the edit started (the Tripod orbits it).
pub fn edit(kind: Kind, cam: &mut Cam, dt: f32, i: &Input, anchor: Vec3) -> bool {
    let dead = |v: f32| if v.abs() > 0.15 { v } else { 0.0 };
    let (lx, ly, rx, ry) = (dead(i.left[0]), dead(i.left[1]), dead(i.right[0]), dead(i.right[1]));
    let vertical = f32::from(i.up) - f32::from(i.down);
    let zoom = f32::from(i.zoom_out) - f32::from(i.zoom_in);
    let fast = if i.fast { 3.0 } else { 1.0 };
    let before = *cam;
    cam.fov = (cam.fov + zoom * 0.5 * dt).clamp(0.2, 1.8);
    match kind {
        Kind::Game => {}
        Kind::Free => {
            let rotation = Quat::from_euler(EulerRot::YXZ, cam.yaw, cam.pitch, 0.0);
            let forward = rotation * Vec3::NEG_Z;
            let right = rotation * Vec3::X;
            cam.position += (right * lx + forward * ly) * 6.0 * fast * dt;
            cam.position.y += vertical * 4.0 * fast * dt;
            cam.yaw -= rx * 1.6 * dt;
            cam.pitch = (cam.pitch + ry * 1.2 * dt).clamp(-1.4, 1.4);
        }
        Kind::Follow => {
            cam.yaw = wrap(cam.yaw - rx * 1.6 * dt);
            cam.pitch = (cam.pitch - ry * 1.2 * dt).clamp(-1.2, 1.4);
            cam.distance = (cam.distance - ly * 6.0 * fast * dt).clamp(1.0, 30.0);
            cam.aim = (cam.aim + vertical * 1.0 * dt).clamp(-0.5, 2.5);
        }
        Kind::Tripod => {
            // Orbit the spot around the skater where the edit started; in and out along the line.
            let target = anchor + Vec3::Y * (cam.aim - AIM);
            let offset = cam.position - target;
            let mut r = offset.length().max(0.5);
            let mut yaw = offset.x.atan2(offset.z);
            let mut pitch = (offset.y / r).clamp(-1.0, 1.0).asin();
            yaw -= rx * 1.6 * dt;
            pitch = (pitch - ry * 1.2 * dt).clamp(-1.2, 1.4);
            r = (r - ly * 6.0 * fast * dt).clamp(1.0, TRIPOD_RANGE - 1.0);
            cam.position = target + Vec3::new(yaw.sin() * pitch.cos(), pitch.sin(), yaw.cos() * pitch.cos()) * r;
            cam.aim = (cam.aim + vertical * 1.0 * dt).clamp(-0.5, 2.5);
            cam.position.y += vertical * 1.0 * dt;
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
