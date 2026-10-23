//! ICE camera tracks from the player's disc (scene.big `data/scene/ice_%03d.bin`, loaded by TU3 sub_82827EB0;
//! doc 31 "Trick Guide camera"). The file is little-endian (sub_828EDD80 swaps it in place on the console).
//! A scene block is `{u32 1, u32 size, u32 scene hash, u32 track count}` followed by the track records back to
//! back (sub_828F51E8). A record: +12 f32 length (s), +20 37 key groups `{u16 segments, u16 keys}`, then from +168
//! `sum(max(segments-2, 0))` u16, aligned to 2, `sum(max(segments-1, 0))` u16 interior key times, aligned to 4,
//! then every element's values in element id order (sub_828EE070): element `id` of group g holds `segments` values
//! (per-segment elements) or `keys` values, each a MSB-first bit stream padded to `((bits*count+31)>>3) & !3` bytes
//! (32-bit elements are raw f32). The element table is `ELEMENTS` (registered by sub_82903840).
//! Read here: group 0's keyed camera elements EYE_X..LOOK_Z (f32), DUTCH, TANGENT_EYE, TANGENT_LOOK, LENS_LENGTH, and
//! its per-segment CUBIC_EYE / CUBIC_LOOK, SPACE_EYE / SPACE_LOOK, AVATAR_EYE / AVATAR_LOOK (values checked against
//! the recomp's per-track player words for tracks 0, 1, 3, 18, 19 and 20).
//! NOT RETAIL YET (inferred from the data): interior key time = u16/65535 * length and the dequantisation
//! `min + raw*(max-min)/2^bits`.
use skate_core::camera_track::{CameraKey, CameraSegment, CameraSpace, CameraTrack};

/// The Trick Guide's ViewScene scene hash (miscboot.big scene_000.bin record #254).
pub const VIEW_SCENE: u32 = 0x7F29_8808;
const GROUPS: usize = 37;
const VALUES: usize = 168;

/// The ICE element table in id order (sub_82903840): (name, key group, one value per segment instead of per key,
/// bits). It fixes where every element's values sit in a record, so the whole table is needed to reach ids 87+.
pub const ELEMENTS: [(&str, u8, bool, u32); 148] = [
    ("EYE_X", 0, false, 32), ("EYE_Y", 0, false, 32), ("EYE_Z", 0, false, 32), ("LOOK_X", 0, false, 32),
    ("LOOK_Y", 0, false, 32), ("LOOK_Z", 0, false, 32), ("DUTCH", 0, false, 10), ("TANGENT_EYE", 0, false, 6),
    ("TANGENT_LOOK", 0, false, 6), ("LENS_LENGTH", 0, false, 9), ("LENS_LENGTH_AUX", 1, false, 9),
    ("NEAR_CLIP", 1, false, 12), ("TANGENT_LENS", 1, false, 6), ("CLIPPING_FAR", 1, false, 12),
    ("CLIPPING_NEAR", 1, false, 12), ("FOCAL_DISTANCE", 2, false, 16), ("APERTURE", 2, false, 6),
    ("TANGENT_FOCUS", 2, false, 6), ("NEAR_FOCUS", 3, false, 16), ("FAR_FOCUS", 3, false, 16),
    ("BLUR_FALLOFF", 3, false, 7), ("BLUR_INTENSITY", 3, false, 7), ("TANGENT_RAWFOCUS", 3, false, 6),
    ("SHAKE_AMPLITUDE", 4, false, 7), ("SHAKE_FREQUENCY", 4, false, 7), ("TIME_SCALE", 5, false, 7),
    ("LETTERBOX", 8, false, 7), ("FADE", 9, false, 7), ("LIGHT_RIG_DIR_U", 10, false, 8),
    ("LIGHT_RIG_DIR_V", 10, false, 8), ("LIGHT_RIG_KEY_INTENSITY", 10, false, 8),
    ("LIGHT_RIG_SEC_INTENSITY", 10, false, 8), ("LIGHT_RIG_FILL_INTENSITY", 10, false, 8),
    ("LIGHT_RIG_RIM_INTENSITY", 10, false, 8), ("TONEMAP_VALUE", 11, false, 16), ("MOTION_BLUR_VALUE", 12, false, 8),
    ("BLOOM_INTENSITY", 13, false, 8), ("BLOOM_THRESHOLD", 13, false, 8), ("TANGENT_BLOOM_INTENSITY", 13, false, 6),
    ("TANGENT_BLOOM_THRESHOLD", 13, false, 6), ("NOISE_INTENSITY", 14, false, 8), ("NOISE_SCALEX", 14, false, 8),
    ("NOISE_SCALEY", 14, false, 8), ("NOISE_SCROLLRATE", 14, false, 8), ("NOISE_SCROLLX", 14, false, 8),
    ("NOISE_SCROLLY", 14, false, 8), ("FISHEYE_HORIZONTAL", 15, false, 8), ("FISHEYE_MULTIPLIER", 15, false, 8),
    ("FISHEYE_VERTICAL", 15, false, 8), ("INTERPOLATION_FAR", 16, false, 32), ("INTERPOLATION_NEAR", 16, false, 32),
    ("BRIGHTNESS_FAR", 17, false, 8), ("BRIGHTNESS_NEAR", 17, false, 8), ("CONTRAST_FAR", 18, false, 16),
    ("CONTRAST_FAR_MIDPOINT", 18, false, 8), ("CONTRAST_NEAR", 18, false, 16),
    ("CONTRAST_NEAR_MIDPOINT", 18, false, 8), ("SATURATE_FAR", 19, false, 8), ("SATURATE_NEAR", 19, false, 8),
    ("TINT_NEAR_COLOR_X", 20, false, 8), ("TINT_NEAR_COLOR_Y", 20, false, 8), ("TINT_NEAR_COLOR_Z", 20, false, 8),
    ("TINT_FAR_COLOR_X", 20, false, 8), ("TINT_FAR_COLOR_Y", 20, false, 8), ("TINT_FAR_COLOR_Z", 20, false, 8),
    ("CHARACTER_EFFECT_MASK_VALUE", 21, false, 8), ("FXMODE_VALUE", 22, false, 8),
    ("SCREEN_WIPE_VALUE", 23, false, 8), ("SHADOW_SCALE", 24, false, 16), ("SHADOW_Y_OFFSET", 24, false, 16),
    ("SHADOW_Z_OFFSET", 24, false, 16), ("SHADOW_X_OFFSET", 24, false, 16), ("SHADOW_BIAS", 25, false, 32),
    ("SHADOW_PRECISION", 26, false, 32), ("SCANLINE_Y_OFFSET", 27, false, 16), ("TANGENT_SCANLINE", 27, false, 6),
    ("RADIALBLUR", 28, false, 8), ("TANGENT_RADIALBLUR", 28, false, 6), ("SUN_DIRECTION_X", 29, false, 16),
    ("SUN_DIRECTION_Y", 29, false, 16), ("SUN_DIRECTION_Z", 29, false, 16), ("SHAKE_QUAT_X", 36, false, 16),
    ("SHAKE_QUAT_Y", 36, false, 16), ("SHAKE_QUAT_Z", 36, false, 16), ("SHAKE_POS_X", 36, false, 16),
    ("SHAKE_POS_Y", 36, false, 16), ("SHAKE_POS_Z", 36, false, 16), ("CUBIC_EYE", 0, true, 1),
    ("CUBIC_LOOK", 0, true, 1), ("SPACE_EYE", 0, true, 3), ("SPACE_LOOK", 0, true, 3), ("AVATAR_EYE", 0, true, 5),
    ("AVATAR_LOOK", 0, true, 5), ("AVATAR_ORBIT", 0, true, 5), ("AVOID_DYNAMIC_OBJECTS", 0, true, 1),
    ("AVOID_STATIC_OBJECTS", 0, true, 1), ("PLAYER_SPACE_LOCK_TO_RENDER_OBJECT", 0, true, 1),
    ("PLAYER_SPACE_NOROLL", 0, true, 1), ("PLAYER_SPACE_LAG", 0, true, 1), ("PLAYER_SPACE_BUNGEE", 0, true, 1),
    ("CUBIC_LENS", 1, true, 1), ("LENS_OVERRIDE", 1, true, 1), ("LENS_USE_DYNAMIC", 1, true, 1),
    ("CUBIC_FOCUS", 2, true, 1), ("CUBIC_RAWFOCUS", 3, true, 1), ("RAWFOCUS_OVERRIDE", 3, true, 1),
    ("RAWFOCUS_RELATIVE", 3, true, 1), ("SHAKE_TYPE", 4, true, 5), ("EVENT_TAG", 6, true, 32),
    ("OVERLAY", 7, true, 16), ("FADE_TO_COLOR", 9, true, 3), ("TONEMAP_OVERRIDE", 11, true, 1),
    ("MOTION_BLUR_OVERRIDE", 12, true, 1), ("MOTION_BLUR_OBJECTAPPLY", 12, true, 1), ("BLOOM_OVERRIDE", 13, true, 1),
    ("CUBIC_BLOOM_INTENSITY", 13, true, 1), ("CUBIC_BLOOM_THRESHOLD", 13, true, 1), ("FISHEYE_OVERRIDE", 15, true, 1),
    ("NEAR_FAR_OVERRIDE", 16, true, 1), ("BRIGHTNESS_OVERRIDE", 17, true, 1), ("CONTRAST_OVERRIDE", 18, true, 1),
    ("SATURATE_OVERRIDE", 19, true, 1), ("TINT_NEAR_NEGATIVE", 20, true, 1), ("TINT_FAR_NEGATIVE", 20, true, 1),
    ("TINT_OVERRIDE", 20, true, 1), ("CHARACTER_EFFECT_MASK_OVERRIDE", 21, true, 1), ("FXMODE", 22, true, 4),
    ("SHADOW_OVERRIDE", 24, true, 1), ("SHADOW_PRECISION_OVERRIDE", 26, true, 1), ("SCANLINE_OVERRIDE", 27, true, 1),
    ("CUBIC_SCANLINE", 27, true, 1), ("RADIALBLUR_OVERRIDE", 28, true, 1), ("CUBIC_RADIALBLUR", 28, true, 1),
    ("SUN_DIRECTION_OVERRIDE", 29, true, 1), ("AUDIO_EFFECT_CMD", 30, true, 2), ("AUDIO_EFFECT_ID", 30, true, 32),
    ("AUDIO_MUSIC_CMD", 31, true, 2), ("AUDIO_MUSIC_ID", 31, true, 32), ("AUDIO_SPEECH_CMD", 32, true, 2),
    ("AUDIO_SPEECH_ID", 32, true, 32), ("AUDIO_SPEECH_CHARACTER", 32, true, 8), ("AUDIO_POST_CMD", 33, true, 2),
    ("AUDIO_POST_ID", 33, true, 32), ("FREEZE_CMD", 34, true, 4), ("FREEZE_TIME", 34, true, 16),
    ("TAKE_START", 35, true, 16), ("TAKE_NUMBER", 35, true, 6), ("CONTAINS_SUBTAKE", 35, true, 1),
];

/// Element ids read here.
const DUTCH: usize = 6;
const CUBIC_EYE: usize = 87;
/// (min, max) of DUTCH, TANGENT_EYE, TANGENT_LOOK, LENS_LENGTH (ids 6..9).
const QUANTISED: [(f32, f32); 4] = [(-0.25, 0.25), (0.0, 8.0), (0.0, 8.0), (10.0, 500.0)];

fn u16_at(b: &[u8], at: usize) -> Result<u16, String> {
    b.get(at..at + 2).map(|s| u16::from_le_bytes([s[0], s[1]])).ok_or_else(|| format!("truncated at {at:#x}"))
}
fn u32_at(b: &[u8], at: usize) -> Result<u32, String> {
    b.get(at..at + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]])).ok_or_else(|| format!("truncated at {at:#x}"))
}
fn f32_at(b: &[u8], at: usize) -> Result<f32, String> {
    u32_at(b, at).map(f32::from_bits)
}

/// `count` values of `bits` each, MSB first.
fn bit_values(b: &[u8], at: usize, bits: u32, count: usize) -> Result<Vec<u32>, String> {
    (0..count).map(|i| {
        let mut v = 0u32;
        for bit in 0..bits as usize {
            let n = i * bits as usize + bit;
            let byte = *b.get(at + n / 8).ok_or_else(|| format!("truncated bit stream at {at:#x}"))?;
            v = (v << 1) | u32::from(byte >> (7 - n % 8) & 1);
        }
        Ok(v)
    }).collect()
}

/// The record at `s` and the offset just past its values.
fn track(b: &[u8], s: usize) -> Result<(CameraTrack, usize), String> {
    let length = f32_at(b, s + 12)?;
    let pairs: Vec<(usize, usize)> = (0..GROUPS)
        .map(|k| Ok((u16_at(b, s + 20 + 4 * k)? as usize, u16_at(b, s + 22 + 4 * k)? as usize)))
        .collect::<Result<_, String>>()?;
    let n2: usize = pairs.iter().map(|(seg, _)| seg.saturating_sub(2)).sum();
    let n1: usize = pairs.iter().map(|(seg, _)| seg.saturating_sub(1)).sum();
    let times_at = s + ((2 * n2 + VALUES + 1) & !1);
    let values_at = s + ((times_at - s + 2 * n1 + 3) & !3);
    // Where each element's values start.
    let mut offsets = [0usize; ELEMENTS.len()];
    let mut end = values_at;
    for (id, &(_, group, per_segment, bits)) in ELEMENTS.iter().enumerate() {
        offsets[id] = end;
        let (segments, keys) = pairs[group as usize];
        end += ((bits as usize * if per_segment { segments } else { keys } + 31) >> 3) & !3;
    }
    let (segments, keys) = pairs[0];
    if keys == 0 {
        return Ok((CameraTrack { length, ..Default::default() }, end));
    }
    let mut times = vec![0.0];
    for i in 0..keys.saturating_sub(2) {
        times.push(u16_at(b, times_at + 2 * i)? as f32 / 65535.0 * length);
    }
    if keys > 1 {
        times.push(length);
    }
    let channel = |c: usize, k: usize| f32_at(b, offsets[c] + 4 * k);
    let quantised = |q: usize| -> Result<Vec<f32>, String> {
        let (min, max) = QUANTISED[q];
        let bits = ELEMENTS[DUTCH + q].3;
        Ok(bit_values(b, offsets[DUTCH + q], bits, keys)?.iter()
            .map(|&r| min + r as f32 * (max - min) / (1u32 << bits) as f32).collect())
    };
    let [dutch, tangent_eye, tangent_look, lens] = [0, 1, 2, 3].map(quantised);
    let (dutch, tangent_eye, tangent_look, lens) = (dutch?, tangent_eye?, tangent_look?, lens?);
    // Group 0's per-segment ints, ids 87..92 (min 0, so the raw value).
    let per_segment = |id: usize| bit_values(b, offsets[id], ELEMENTS[id].3, segments);
    let [cubic_eye, cubic_look, space_eye, space_look, avatar_eye, avatar_look] =
        [0, 1, 2, 3, 4, 5].map(|i| per_segment(CUBIC_EYE + i));
    let (cubic_eye, cubic_look, space_eye, space_look, avatar_eye, avatar_look) =
        (cubic_eye?, cubic_look?, space_eye?, space_look?, avatar_eye?, avatar_look?);
    let mut out = Vec::with_capacity(keys);
    for k in 0..keys {
        out.push(CameraKey {
            time: times[k],
            eye: [channel(0, k)?, channel(1, k)?, channel(2, k)?],
            look: [channel(3, k)?, channel(4, k)?, channel(5, k)?],
            dutch: dutch[k],
            tangent_eye: tangent_eye[k],
            tangent_look: tangent_look[k],
            lens_mm: lens[k],
        });
    }
    let segments = (0..segments).map(|i| CameraSegment {
        cubic_eye: cubic_eye[i] != 0,
        cubic_look: cubic_look[i] != 0,
        eye_space: CameraSpace { mode: space_eye[i] as u8, avatar: avatar_eye[i] as u8 },
        look_space: CameraSpace { mode: space_look[i] as u8, avatar: avatar_look[i] as u8 },
    }).collect();
    Ok((CameraTrack { length, keys: out, segments }, end))
}

/// The tracks of the scene block with `scene_hash`, in file order (track index = position).
pub fn scene_tracks(b: &[u8], scene_hash: u32) -> Result<Vec<CameraTrack>, String> {
    let block = (0..b.len().saturating_sub(16)).step_by(4)
        .find(|&o| u32_at(b, o) == Ok(1) && u32_at(b, o + 8) == Ok(scene_hash))
        .ok_or_else(|| format!("no ICE scene block {scene_hash:#010x}"))?;
    let count = u32_at(b, block + 12)? as usize;
    let mut tracks = Vec::with_capacity(count);
    let mut at = block + 16;
    // Records sit back to back: the next one starts where the element table says this one's values end.
    for _ in 0..count {
        let (track, end) = track(b, at)?;
        tracks.push(track);
        at = end;
    }
    if at > b.len() {
        return Err(format!("ICE scene block {scene_hash:#010x} runs past the file ({at:#x})"));
    }
    Ok(tracks)
}
