//! Scene clips: one-clip banks from `scene.big` (`data/scene/trickguide/<name>1.abin`
//! for the Trick Guide demo). Each file holds the full character hierarchy (131
//! bones: body, board, fingers, twist helpers, face), one RAW clip and one RAW pose.
//! The clip channels are deltas on the file's pose (SK83_TPOSE): each frame's local is retail's AddSQT
//! (`pose_add::add`, 0x828CCC08) of the stored delta (identity where nothing is stored) onto the pose. Evidence:
//! limb channels sit near identity where the pose has large rotations (LEFTSHOULDER, LEFTUPLEG), and the HIPS
//! channel is the inverse of the bind trajectory, so HIPS adds up to identity under the trajectory.
//! NOT RETAIL YET: the add is inferred from the data; what selects it in the RAW job (field +62) is unread.
use crate::{
    abin::{Bank, Codec, PartEntry, RecordData},
    animation_frames::SampleWords,
};
use skate_core::animation::{output::Sqt, pose_add::add, raw::RawDecoder};

fn sqt(w: SampleWords) -> Sqt {
    let [sx, sy, sz, x, y, z, w, tx, ty, tz] = w.map(f32::from_bits);
    Sqt { scale: [sx, sy, sz, 1.0], rotation: [x, y, z, w], translation: [tx, ty, tz, 1.0] }
}
fn words(s: Sqt) -> SampleWords {
    let [sx, sy, sz, _] = s.scale;
    let [x, y, z, w] = s.rotation;
    let [tx, ty, tz, _] = s.translation;
    [sx, sy, sz, x, y, z, w, tx, ty, tz].map(f32::to_bits)
}

#[derive(Clone, Debug)]
pub struct SceneClip {
    pub name: String,
    pub bone_names: Vec<String>,
    pub parents: Vec<i32>,
    pub fps: f32,
    ///Per frame, per hierarchy bone: local scaleXYZ, quaternionXYZW, translationXYZ.
    pub frames: Vec<Vec<SampleWords>>,
}

impl SceneClip {
    pub fn parse(bytes: Vec<u8>) -> Result<Self, String> {
        let bank = Bank::parse(bytes).map_err(|e| e.to_string())?;
        let hierarchy = bank.hierarchy().ok_or("Scene clip has no hierarchy")?;
        let bones = hierarchy.bone_count as usize;
        let mut clip = None;
        let mut pose = None;
        for record in bank.records() {
            match &record.data {
                RecordData::Clip(c) if clip.is_none() => clip = Some((&record.header, c)),
                RecordData::Pose(p) if pose.is_none() => pose = Some((&record.header, p)),
                _ => {}
            }
        }
        let (header, clip) = clip.ok_or("Scene clip bank has no clip")?;
        let (pose_header, pose) = pose.ok_or("Scene clip bank has no pose")?;
        for codec in [header.codec(), pose_header.codec()] {
            if codec != Codec::Raw {
                return Err(format!("{}: scene clip codec {codec:?} is not RAW", header.name));
            }
        }
        let count = f32::from_bits(clip.frame_count_bits);
        if !count.is_finite() || count < 1.0 || count.fract() != 0.0 {
            return Err(format!("{}: invalid frame count", header.name));
        }
        let fps = f32::from_bits(clip.fps_bits);
        if !(fps.is_finite() && fps > 0.0) {
            return Err(format!("{}: invalid frame rate", header.name));
        }
        let identity = [1.0f32, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0].map(f32::to_bits);
        let base = fill(&bank, &pose.parts, 1, vec![vec![identity; bones]], true)
            .map_err(|e| format!("{} pose: {e}", header.name))?
            .remove(0);
        let deltas = fill(&bank, &clip.parts, count as usize, vec![vec![identity; bones]; count as usize], false)
            .map_err(|e| format!("{}: {e}", header.name))?;
        let frames = deltas.into_iter().map(|frame| frame.into_iter().zip(&base)
            .map(|(delta, pose)| words(add(sqt(delta), sqt(*pose), true))).collect()).collect();
        Ok(Self {
            name: header.name.clone(),
            bone_names: hierarchy.bone_names.clone(),
            parents: hierarchy.parents.clone(),
            fps,
            frames,
        })
    }
    pub fn duration(&self) -> f32 {
        self.frames.len() as f32 / self.fps
    }
}

///Write each stored part over its hierarchy slot. The slot is the part header's
///low six bits / 2 (checked: the 17 pose parts read 0, 2 .. 32 in order).
fn fill(
    bank: &Bank,
    parts: &[PartEntry],
    count: usize,
    mut frames: Vec<Vec<SampleWords>>,
    complete: bool,
) -> Result<Vec<Vec<SampleWords>>, String> {
    let hierarchy = bank.hierarchy().unwrap();
    let mut assigned = vec![false; hierarchy.parts.len()];
    for part in parts.iter().filter_map(|p| p.part.as_ref()) {
        let slot = ((part.header_word & 0x3f) >> 1) as usize;
        let layout = hierarchy.parts.get(slot).ok_or("Part slot outside hierarchy")?;
        if std::mem::replace(&mut assigned[slot], true) {
            return Err(format!("{}: part stored twice", layout.name));
        }
        if part.channel_count as u32 != layout.bone_count {
            return Err(format!("{}: channel count disagrees with hierarchy", layout.name));
        }
        if part.compressed_size == 0 {
            continue;
        }
        let decoder = RawDecoder::new(
            bank.bytes(),
            part.offset,
            part.compression_header_relative as usize,
            part.compressed_data_relative as usize,
            part.channel_count as usize,
        )?;
        if decoder.frames_in(part.compressed_size as usize) < count {
            return Err(format!("{}: fewer RAW frames than the clip header", layout.name));
        }
        let start = usize::try_from(layout.sqt_offset).map_err(|_| "Negative part offset")?;
        let channels = part.channel_count as usize;
        for index in 0..count {
            // Components the frame does not store keep the pose's (frames start as copies of it).
            let mut samples = frames[index][start..start + channels].to_vec();
            decoder.decode_into(index, &mut samples)?;
            for (channel, sample) in samples.into_iter().enumerate() {
                // Authored NaNs exist (trickguide_bs_dark_slide1 frame 103, RIGHT_WHEELBACK
                // translation): hold the previous frame's channel (frame 0: the pose).
                let finite = sample.iter().all(|w| f32::from_bits(*w).is_finite());
                if finite {
                    frames[index][start + channel] = sample;
                } else if index > 0 {
                    frames[index][start + channel] = frames[index - 1][start + channel];
                }
            }
        }
    }
    if complete && assigned.iter().any(|a| !a) {
        return Err("pose leaves hierarchy parts unset".into());
    }
    if frames.iter().flatten().flatten().any(|w| !f32::from_bits(*w).is_finite()) {
        return Err("nonfinite RAW sample".into());
    }
    Ok(frames)
}

/// Diagnostic: parses the installed Trick Guide scene (`SKATE3_TRICK_GUIDE_SCENE=<assets>/private/hud/scene`).
#[cfg(test)]
#[test]
#[ignore = "diagnostic; requires the installed scene"]
fn installed_scene_clips_parse() {
    let root = std::path::PathBuf::from(std::env::var_os("SKATE3_TRICK_GUIDE_SCENE").unwrap());
    let tracks = crate::ice_camera::scene_tracks(&std::fs::read(root.join("ice_000.bin")).unwrap(), crate::ice_camera::VIEW_SCENE).unwrap();
    eprintln!("SCENE {} camera tracks", tracks.len());
    let mut names: Vec<_> = std::fs::read_dir(root.join("trickguide")).unwrap().filter_map(|e| e.ok()).map(|e| e.path()).collect();
    names.sort();
    let mut total = 0usize;
    for path in names.iter().take(5) {
        let clip = SceneClip::parse(std::fs::read(path).unwrap()).unwrap();
        total += clip.frames.len() * clip.bone_names.len() * 40;
        eprintln!("SCENE {}: {} bones, {} frames at {} fps; first bones {:?}", clip.name, clip.bone_names.len(), clip.frames.len(), clip.fps, &clip.bone_names[..8]);
    }
    eprintln!("SCENE decoded bytes of 5 clips: {} KiB", total / 1024);
}
