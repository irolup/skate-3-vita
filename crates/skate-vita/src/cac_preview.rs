//! The customisation's preview (docs/customization-survey.md, step 3): retail's set behind the skater, the skater
//! playing its posture's stand on it, the camera row of the page. The set is retail `DIST_CreateSakter` (`cas/set.map`,
//! drawn in the demo slot the Trick Guide uses; the two are never open together), its floor at y 200
//! ([`Camera::STAGE`]); the cameras and the skater's spot (`cac_body`) are offsets from that floor (82531868,
//! 82532800). The stands are `cac_edit_<gender>.abin` clips (82539688 picks one by posture).
//! NOT RETAIL YET: the cameras cut instead of moving between rows; the stand plays on its own (retail blends 0.1 s
//! between clips and plays the gesture demos); the set's sky, sun and fog are not drawn (its horizon colour clears).
use crate::trick_demo::{native_matrix, pose, Reader, View};
use glam::{Mat4, Vec3};
use skate_core::cac::Preview;
use skate_core::cas_rules::Camera;
use skate_core::skater_record::Gender;
use skate_data::scene_clip::SceneClip;
use skate_sim::live::{Snapshot, Transform};

/// The set's map pack in `cas.pak`.
pub const SET: &str = "cas/set.map";

#[derive(Default)]
pub struct Stage {
    /// The stand playing (bank path and clip name) and its time.
    playing: Option<(String, SceneClip)>,
    time: f32,
    /// A stand that failed to load (not asked for again while it stays wanted).
    failed: String,
}

fn bank(gender: Gender) -> &'static str {
    if gender == Gender::Female {
        "cas/anim/cac_edit_female.abin"
    } else {
        "cas/anim/cac_edit_male.abin"
    }
}

impl Stage {
    /// One frame: the stand the screen wants plays, looping.
    pub fn update(&mut self, preview: &Preview, seconds: f32, read: Reader) {
        let key = format!("{}#{}", bank(preview.gender), preview.stand);
        if self.playing.as_ref().is_none_or(|(k, _)| *k != key) && self.failed != key {
            match read(bank(preview.gender)).and_then(|b| SceneClip::parse_named(b, Some(&preview.stand))) {
                Ok(clip) => {
                    self.playing = Some((key, clip));
                    self.time = 0.0;
                }
                Err(e) => {
                    println!("CAC preview: {key}: {e}");
                    self.failed = key;
                }
            }
        }
        if let Some((_, clip)) = &self.playing {
            self.time = (self.time + seconds) % clip.duration().max(1e-3);
        }
    }

    /// The skater on the set and the camera (the stand's bones by the simulation rig's names); the skater stands
    /// still in the clip's first pose until a stand plays.
    pub fn view(&self, preview: &Preview, bone_names: &[String]) -> Option<View> {
        let (_, clip) = self.playing.as_ref()?;
        let stage = Vec3::from_array(Camera::STAGE);
        let place = Mat4::from_translation(stage + Vec3::from_array(preview.position)) * Mat4::from_rotation_y(preview.heading);
        // The stand's TRAJECTORY bone holds a quarter turn about up; the character's own transform places it
        // (82532800 sets the heading), so the bones are taken relative to the trajectory. Inferred from the data:
        // with it the 260 degree heading shows the skater from the side, without it facing the camera row.
        let globals: Vec<Mat4> = pose(clip, self.time)?.into_iter().map(native_matrix).collect();
        let trajectory = globals.first().copied().unwrap_or(Mat4::IDENTITY).inverse();
        let matrices: Vec<Mat4> = globals.into_iter().map(|m| place * trajectory * m).collect();
        let bones = bone_names
            .iter()
            .map(|name| clip.bone_names.iter().position(|b| b == name).map_or(place, |i| matrices[i]))
            .collect();
        let snapshot = Snapshot { root: Transform::IDENTITY, bones, camera: Transform::IDENTITY, fov: 1.0 };
        let c = &preview.camera;
        let (eye, target) = (stage + Vec3::from_array(c.eye), stage + Vec3::from_array(c.target));
        let camera = Mat4::look_at_rh(eye, target, Vec3::Y).inverse();
        Some(View { snapshot, camera, fov: Some(c.fov.to_radians()) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use skate_core::cac::Cac;

    /// `SKATEVITA` at the folder holding the real `cas.pak`: the set loads, the stand plays on its floor, and the
    /// standing camera has the skater in front of it.
    #[test]
    #[ignore = "requires a baked cas.pak (SKATEVITA)"]
    fn the_skater_stands_on_the_set_in_front_of_the_camera() {
        let dir = std::env::var("SKATEVITA").expect("SKATEVITA");
        let pack = std::fs::read(format!("{dir}/cas.pak")).unwrap();
        let index = vita_pack::files::index_of(&pack).unwrap();
        let read = |p: &str| {
            let (_, at, len) = index.iter().find(|e| e.0 == p).ok_or_else(|| format!("{p}: missing"))?;
            Ok(pack[*at as usize..*at as usize + len].to_vec())
        };
        let set = vita_pack::map::Map::from_pack(&read(SET).unwrap()).unwrap();
        let data = crate::cas_look::Builder::new(&read).unwrap().screen_data(&read).unwrap();
        let record = data.fallback[1].clone();
        let cac = Cac::open(record, &skate_core::unlocks::UnlockStore::fresh_profile(), Some(std::sync::Arc::new(data)));
        let preview = cac.preview().unwrap();
        let mut stage = Stage::default();
        for _ in 0..30 {
            stage.update(&preview, 1.0 / 30.0, &read);
        }
        let names: Vec<String> = ["HIPS", "HEAD", "LEFTFOOT", "LEFTTOEBASE"].map(String::from).to_vec();
        let view = stage.view(&preview, &names).expect("the stand plays");
        let [hips, head, foot] = [0, 1, 2].map(|i| view.snapshot.bones[i].w_axis.truncate());
        let eye = view.camera.w_axis.truncate();
        let forward = -view.camera.z_axis.truncate();
        println!("set draws {:?}; hips {hips:?} head {head:?} foot {foot:?} eye {eye:?}", set.render.draws.len());
        // On the floor (y 200), standing: the head well above the feet.
        assert!((199.8..200.4).contains(&foot.y), "feet at {}", foot.y);
        assert!(head.y - foot.y > 1.3, "standing");
        // In view of the standing camera, a few metres ahead.
        let to = hips - eye;
        assert!(to.normalize().dot(forward) > 0.9, "the camera looks at the skater");
        assert!((2.0..5.0).contains(&to.length()));
        // Facing it: the toes point from the heel towards the camera.
        let toe = view.snapshot.bones[3].w_axis.truncate();
        let toward = (eye - hips) * Vec3::new(1.0, 0.0, 1.0);
        println!("toe {toe:?}: facing {:.2}", ((toe - foot) * Vec3::new(1.0, 0.0, 1.0)).normalize().dot(toward.normalize()));
        assert!(((toe - foot) * Vec3::new(1.0, 0.0, 1.0)).normalize().dot(toward.normalize()) > 0.7, "facing the camera");
    }
}
