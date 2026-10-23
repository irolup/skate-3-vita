//! R2: the Trick Guide's 3D demo (skate-game `trick_guide_scene.rs`, PR #52 `fad5aae`, without Bevy). The screen's
//! highlighted entry names a clip and a camera track; the skater plays the clip (`scene/trickguide/<name>1.abin`)
//! in the guide's set, and the camera follows the ViewScene track fitted to the clip's length. Upstream parents the
//! set to the player's root; here the set stays at the origin and the skater is posed in its space (the same
//! picture). The clip's bones become a simulation snapshot (`Binding::palette` skins the skater from it).
//! NOT RETAIL YET (as upstream): looping assumed; the lens angle taken as the vertical field of view; the followed
//! root taken as the clip's bone 0; the set's sky, sun and fog not drawn (the set's horizon colour clears).
use glam::{Mat4, Vec3};
use skate_core::animation::output::{self, NativeMatrix};
use skate_core::animation::pose_sample::{sample_key, select_frames};
use skate_core::camera_track::{CameraSpace, CameraTrack};
use skate_data::scene_clip::SceneClip;
use skate_sim::live::{Snapshot, Transform};

/// The scene's files (vita-bake `trickguide.pak`), relative to the asset root.
pub const ROOT: &str = "private/hud/scene";
/// sub_82619A20: a new selection plays after S+332 = 60.0 counts down by the FE update's milliseconds.
const SETTLE_MS: f32 = 60.0;
/// Init 0x82618A40: the clip before any trick is highlighted (S+204) and its camera track (S+200 = 1).
const DEFAULT_CLIP: &str = "trickguide_fs_noseblunt_01";
const DEFAULT_TRACK: i16 = 1;

/// Reads a file by its path relative to the asset root.
pub type Reader<'a> = &'a dyn Fn(&str) -> Result<Vec<u8>, String>;

/// `native_matrix` (skate-sim `animation.rs`, private there).
fn native_matrix(m: NativeMatrix) -> Mat4 {
    Mat4::from_cols(
        Vec3::from_array(m[0][..3].try_into().unwrap()).extend(0.0),
        Vec3::from_array(m[1][..3].try_into().unwrap()).extend(0.0),
        Vec3::from_array(m[2][..3].try_into().unwrap()).extend(0.0),
        Vec3::from_array(m[3][..3].try_into().unwrap()).extend(1.0),
    )
}

/// A clip's local samples at `time`, composed into scene-space bone matrices (upstream `pose`).
pub fn pose(clip: &SceneClip, time: f32) -> Option<Vec<NativeMatrix>> {
    let selection = select_frames(time, clip.fps, clip.frames.len(), true, 0.0).ok()?;
    let sqt = |w: skate_data::animation_frames::SampleWords| {
        let [sx, sy, sz, x, y, z, w, tx, ty, tz] = w.map(f32::from_bits);
        output::Sqt { scale: [sx, sy, sz, 1.0], rotation: [x, y, z, w], translation: [tx, ty, tz, 1.0] }
    };
    let locals: Vec<_> = clip.frames[selection.first]
        .iter()
        .zip(&clip.frames[selection.second])
        .map(|(&a, &b)| sample_key(sqt(a), sqt(b), selection))
        .collect();
    // The scene hierarchy has no trajectory link (HIPS parent -1); the HIPS delta adds up to identity, so the
    // trajectory carries the body. NOT RETAIL YET: parenting the root bones to it is inferred from the data.
    let parents: Vec<i32> =
        clip.parents.iter().enumerate().map(|(bone, &parent)| if parent == -1 && bone != 0 { 0 } else { parent }).collect();
    let mut globals: Vec<_> = locals.into_iter().map(output::sqt_to_matrix).collect();
    output::compose_hierarchy_in_place(globals.len() as i32, &parents, -1, &mut globals).ok()?;
    Some(globals)
}

/// A track point in scene space (sub_828F36B0): track space is Z-up and maps to the scene as (y, z, x); SPACE 2
/// adds the followed object's position (AVATAR 5: the skater's root at height 0, AVATAR 7: at full height).
fn place([x, y, z]: [f32; 3], space: CameraSpace, skater: Vec3) -> Vec3 {
    let point = Vec3::new(y, z, x);
    match (space.mode, space.avatar) {
        (2, 5) => point + skater * Vec3::new(1.0, 0.0, 1.0),
        (2, 7) => point + skater,
        // NOT RETAIL YET (as upstream): the other spaces follow the skater at height 0.
        (0 | 2 | 3, _) => point + skater * Vec3::new(1.0, 0.0, 1.0),
        _ => point,
    }
}

/// What the demo shows this frame: the skater's bones (a snapshot by the simulation rig's bone names) and the
/// camera (world transform, vertical field of view when the track sets one).
pub struct View {
    pub snapshot: Snapshot,
    pub camera: Mat4,
    pub fov: Option<f32>,
}

#[derive(Default)]
pub struct Demo {
    tracks: Vec<CameraTrack>,
    /// Requested clip and its track, the settle countdown; the playing clip, its track and time.
    requested: String,
    requested_track: i16,
    settle_ms: f32,
    playing: Option<(String, SceneClip)>,
    track: i16,
    time: f32,
    /// A clip that failed to load (not asked for again while it stays highlighted).
    failed: String,
}

impl Demo {
    /// The camera tracks (`ice_000.bin`); the set is loaded by the caller (`TrickGuide.map`).
    pub fn load(read: Reader) -> Result<Self, String> {
        let tracks = skate_data::ice_camera::scene_tracks(&read(&format!("{ROOT}/ice_000.bin"))?, skate_data::ice_camera::VIEW_SCENE)?;
        Ok(Self { tracks, ..Self::default() })
    }

    /// One frame: the highlighted entry's clip and track (an empty clip keeps the current one), `seconds` of time.
    pub fn update(&mut self, animation: &str, camera_track: i16, seconds: f32, read: Reader) {
        if self.requested.is_empty() {
            self.requested = DEFAULT_CLIP.into();
            self.requested_track = DEFAULT_TRACK;
        }
        if !animation.is_empty() && animation != self.requested && animation != self.failed {
            self.requested = animation.to_owned();
            self.requested_track = camera_track;
            self.settle_ms = SETTLE_MS;
        }
        if self.settle_ms > 0.0 {
            self.settle_ms -= seconds * 1000.0;
        }
        let wanted = self.settle_ms <= 0.0 && self.playing.as_ref().is_none_or(|(p, _)| *p != self.requested);
        if wanted {
            // sub_82826488: "scene/trickguide/" + name + "1.abin".
            let path = format!("{ROOT}/trickguide/{}1.abin", self.requested);
            match read(&path).and_then(SceneClip::parse) {
                Ok(clip) => {
                    self.playing = Some((self.requested.clone(), clip));
                    self.track = self.requested_track;
                    self.time = 0.0;
                }
                Err(e) => {
                    println!("trick guide demo: {path}: {e}");
                    // Keep what plays; not asked for again while it stays highlighted.
                    self.failed = std::mem::take(&mut self.requested);
                    self.requested = self.playing.as_ref().map_or_else(String::new, |(n, _)| n.clone());
                }
            }
        }
        if let Some((_, clip)) = &self.playing {
            self.time = (self.time + seconds) % clip.duration().max(1e-3);
        }
    }

    /// The skater and the camera now (None before a clip plays). `bone_names`: the simulation rig's bones.
    pub fn view(&self, bone_names: &[String]) -> Option<View> {
        let (_, clip) = self.playing.as_ref()?;
        let globals = pose(clip, self.time)?;
        let matrices: Vec<Mat4> = globals.iter().copied().map(native_matrix).collect();
        let bones = bone_names
            .iter()
            .map(|name| clip.bone_names.iter().position(|b| b == name).map_or(Mat4::IDENTITY, |i| matrices[i]))
            .collect();
        let snapshot = Snapshot { root: Transform::IDENTITY, bones, camera: Transform::IDENTITY, fov: 1.0 };
        let track = usize::try_from(self.track).ok().and_then(|i| self.tracks.get(i));
        let (eye, look, roll, fov) = match track {
            Some(track) => {
                // Retail fits the track to the clip (sub_82820E98 writes the clip length).
                let time = self.time * track.length / clip.duration().max(1e-3);
                let sample = track.sample(time)?;
                let skater = matrices[0].w_axis.truncate();
                (place(sample.eye, sample.eye_space, skater), place(sample.look, sample.look_space, skater), sample.dutch, Some(sample.fov_radians()))
            }
            // No track: a side view of the hips, the skater in the left third.
            None => {
                let hips = clip.bone_names.iter().position(|n| n == "HIPS").unwrap_or(0);
                let target = matrices[hips].w_axis.truncate();
                (target + Vec3::new(1.2, 0.6, 5.5), target + Vec3::new(1.6, 0.0, 0.0), 0.0, None)
            }
        };
        // DUTCH is in turns: the camera rolls by -dutch turns about its forward axis.
        let camera = Mat4::look_at_rh(eye, look, Vec3::Y).inverse() * Mat4::from_rotation_z(-roll * std::f32::consts::TAU);
        Some(View { snapshot, camera, fov })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The installed scene (`SKATE3_TRICK_GUIDE_SCENE=<assets>/private/hud/scene`): the default clip plays, the
    /// skater's bones are finite and the camera looks at the skater from a few metres.
    #[test]
    fn the_default_clip_plays_and_the_camera_frames_the_skater() {
        let Some(root) = std::env::var_os("SKATE3_TRICK_GUIDE_SCENE") else {
            eprintln!("skipped: SKATE3_TRICK_GUIDE_SCENE not set");
            return;
        };
        let root = std::path::PathBuf::from(root);
        let read = |p: &str| std::fs::read(root.join(p.strip_prefix(&format!("{ROOT}/")).unwrap_or(p))).map_err(|e| e.to_string());
        let mut demo = Demo::load(&read).unwrap();
        demo.update("", 0, 1.0 / 60.0, &read);
        for _ in 0..90 {
            demo.update("", 0, 1.0 / 60.0, &read);
        }
        let names: Vec<String> = ["HIPS", "SPINE", "HEAD", "NOT_A_BONE"].map(String::from).to_vec();
        let view = demo.view(&names).expect("a clip plays");
        assert!(view.snapshot.bones.iter().all(|m| m.is_finite()));
        let hips = view.snapshot.bones[0].w_axis.truncate();
        let eye = view.camera.w_axis.truncate();
        let distance = eye.distance(hips);
        eprintln!("hips {hips:?} eye {eye:?} distance {distance:.2} fov {:?}", view.fov);
        assert!((0.5..30.0).contains(&distance), "the camera is {distance} m from the hips");
        // Changing the selection waits the settle time, then plays the new clip.
        demo.update("trickguide_360flip_01", 3, 1.0 / 60.0, &read);
        for _ in 0..10 {
            demo.update("trickguide_360flip_01", 3, 1.0 / 60.0, &read);
        }
        assert!(demo.playing.as_ref().is_some_and(|(n, _)| n == "trickguide_360flip_01"));
        // A missing clip keeps the playing one and is not read again.
        for _ in 0..10 {
            demo.update("trickguide_not_a_clip", 3, 1.0 / 60.0, &read);
        }
        assert!(demo.playing.as_ref().is_some_and(|(n, _)| n == "trickguide_360flip_01"));
        assert_eq!(demo.failed, "trickguide_not_a_clip");
    }
}
