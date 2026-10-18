//! Render-only snapshots of completed fixed ticks. Camera, root and skin use
//! the same interval; interpolated values never feed back into simulation.
//!
//! skate-game `presentation.rs` (`cb79689`) without Bevy: the plugin is
//! gone, the `capture` system is `Presentation::capture` with the fixed timestep passed
//! in, and replay recording and playback are left out.
use crate::transform::Transform;
use crate::{camera::CameraRuntime, physics::SkaterRuntime};
use glam::{Mat3, Mat4, Quat, Vec3};
use skate_core::camera::CameraFrame;

#[derive(Clone)]
pub struct Snapshot {
    pub root: Transform,
    pub bones: Vec<Mat4>,
    pub camera: Transform,
    pub fov: f32,
}

#[derive(Default)]
pub struct Presentation {
    pair: Option<(Snapshot, Snapshot)>,
    generation: u64,
    period: std::time::Duration,
    /// Snaps so far (teleports, camera cuts, cadence changes): the audio host resets its
    /// camera velocity (Doppler) on a change (`game_audio::native::mixmap_frame`).
    pub cuts: u64,
}
impl Presentation {
    pub fn view(&self, alpha: f32) -> Option<(&Snapshot, &Snapshot, f32)> {
        self.pair().map(|(a, b)| (a, b, alpha.clamp(0.0, 1.0)))
    }
    pub fn pair(&self) -> Option<(&Snapshot, &Snapshot)> {
        self.pair.as_ref().map(|(a, b)| (a, b))
    }
}

/// Runs after every fixed tick; `timestep` is that tick's fixed period.
pub(crate) fn capture(skater: &SkaterRuntime, camera: &CameraRuntime,
    timestep: std::time::Duration, history: &mut Presentation) {
    if skater.pose_generation == history.generation && !camera.manual_cam.manual_cam_active() {
        return;
    }
    let Some(frame) = camera.presentation_frame() else { return; };
    let next = Snapshot {
        root: Transform::from_matrix(crate::animation::native_matrix(
            skater.animated_skeleton.roots.animation_to_world)),
        bones: skater.render_pose.iter().copied()
            .map(crate::animation::native_matrix).collect(),
        camera: camera_transform(frame),
        fov: frame.field_of_view_degrees.to_radians(),
    };
    // Snap teleports/cuts and cadence changes instead of sweeping through the
    // world or blending snapshots across different simulation-clock periods.
    let reset = frame.discontinuity
        || skater.player_input.processed.flags_2472 & (1 << 10) != 0
        || history.period != timestep;
    history.cuts += u64::from(reset);
    if let Some((previous, current)) = history.pair.as_mut() {
        if reset || current.bones.len() != next.bones.len() {
            *previous = next.clone();
        } else {
            std::mem::swap(previous, current);
        }
        *current = next;
    } else {
        history.pair = Some((next.clone(), next));
    }
    history.generation = skater.pose_generation;
    history.period = timestep;
}

fn camera_transform(frame: CameraFrame) -> Transform {
    let [right, up, at] = frame.basis.columns.map(Vec3::from_array);
    Transform {
        translation: Vec3::new(frame.position[0], frame.position[1], frame.position[2]),
        // Native At is forward; Bevy cameras look along local -Z.
        rotation: Quat::from_mat3(&Mat3::from_cols(-right, up, -at)).normalize(),
        ..Default::default()
    }
}

pub fn blend(a: Transform, b: Transform, alpha: f32) -> Transform {
    Transform {
        translation: a.translation.lerp(b.translation, alpha),
        rotation: a.rotation.normalize().slerp(b.rotation.normalize(), alpha),
        scale: a.scale.lerp(b.scale, alpha),
    }
}
