//! The editor's tuning: the vault records `catalog.json` carries (`tools/prepare_object_catalog.py` exports them
//! raw), resolved as retail reads them. Field meanings come from the code that reads each one (survey "Tuning in
//! the vault"); a field not listed here is not used yet.
//!
//! `objectdropper` has one record per editor mode (`default`, `free_cam`, `manipulate`, `object_spawner`), each
//! inheriting the fields it leaves out from its parent; `editor_camera` and `editor_cursor` have one record each.
use std::collections::BTreeMap;

/// `objectdropper` fields (record +0x1758 of the dropper, the mode's record).
mod od {
    /// Horizontal move scale (8288C310).
    pub const MOVE: &str = "BF2D64741246823F";
    /// Elevation scale (8288C310).
    pub const ELEVATE: &str = "23E188CB1C0B3760";
    /// The move-speed curve over the zoom fraction (8288C310, evaluated by 82481E10).
    pub const SPEED_CURVE: &str = "D6BCD32064145E72";
    /// Camera pitch per update at full stick (8288CE40).
    pub const PITCH_RATE: &str = "E2A0591E7E13858C";
    /// Camera yaw per update at full stick (8288CE40).
    pub const YAW_RATE: &str = "D5ABC2A4567B072F";
    /// Zoom per update at full press (8288CE40).
    pub const ZOOM_RATE: &str = "5A1E29986CA4DCFC";
    /// Object rotation per update at full stick (8288C310, the rotate mode).
    pub const ROTATE_RATE: &str = "32D332B6A37CB956";
    /// The move also follows the camera's own forward (8288C310: on in `free_cam`).
    pub const ALONG_CAMERA: &str = "D41775D720BA7D9E";
}

/// `editor_camera` fields. The first five are its fixed block (vault layout: offsets 0-16), which 82886CC8 reads by
/// offset; the others by hash.
mod cam {
    pub const SMOOTHING: &str = "F648C92478E50280";
    pub const ORBIT_PITCH_MIN: &str = "2355E78BA27DE039";
    pub const FREE_PITCH_MIN: &str = "0F3107CDC2702A9E";
    pub const ORBIT_PITCH_MAX: &str = "42414237536F3CDF";
    pub const FREE_PITCH_MAX: &str = "D506436BE5C56398";
    pub const START_PITCH: &str = "93D10E5C9044D9EC";
    pub const START_DISTANCE: &str = "61D0436EB466B4C3";
    pub const MIN_DISTANCE: &str = "7F9E4FD28E069D0B";
    pub const MAX_DISTANCE: &str = "C1D41A1A66112171";
}

/// `editor_cursor` fields.
mod cursor {
    /// The cursor follows the ground under it (828883D8).
    pub const FOLLOW_GROUND: &str = "26835378D7B948AF";
    /// A ground step larger than this is not followed at once (828883D8).
    pub const GROUND_STEP: &str = "0B2454C666410860";
}

/// A piecewise-linear curve (`Sk8::PointNegGraphData8`: four header floats, eight x, eight y), evaluated as 82481E10:
/// clamped to the first and last y outside the x range, linear between points.
#[derive(Clone, Debug, PartialEq)]
pub struct Curve {
    pub x: Vec<f32>,
    pub y: Vec<f32>,
}

impl Curve {
    pub fn at(&self, t: f32) -> f32 {
        let (x, y) = (&self.x, &self.y);
        let (Some(&first), Some(&last)) = (x.first(), x.last()) else { return 0.0 };
        if t < first {
            return y[0];
        }
        if t >= last {
            return y[y.len() - 1];
        }
        for i in 1..x.len() {
            if t < x[i] {
                let span = x[i] - x[i - 1];
                if span <= 0.0 {
                    return y[i];
                }
                return (y[i] - y[i - 1]) / span * (t - x[i - 1]) + y[i - 1];
            }
        }
        y[0]
    }
}

/// One editor mode's values (an `objectdropper` record resolved through its parents).
#[derive(Clone, Debug, PartialEq)]
pub struct ModeTuning {
    pub move_scale: f32,
    pub elevate_scale: f32,
    pub speed_curve: Curve,
    pub pitch_rate: f32,
    pub yaw_rate: f32,
    pub zoom_rate: f32,
    pub rotate_rate: f32,
    pub along_camera: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Tuning {
    pub default: ModeTuning,
    pub free_cam: ModeTuning,
    pub manipulate: ModeTuning,
    pub object_spawner: ModeTuning,
    /// The camera's ease toward its target distance, per update (0..1).
    pub smoothing: f32,
    /// Orbit (the editor's camera around the cursor) pitch limits, radians.
    pub orbit_pitch: [f32; 2],
    /// The other camera mode's pitch limits, radians (not used by the editor yet).
    pub free_pitch: [f32; 2],
    pub start_pitch: f32,
    pub start_distance: f32,
    pub distance: [f32; 2],
    pub follow_ground: bool,
    pub ground_step: f32,
    /// The Colour page's swatches (`skatepark_presentation` / `tint` / `TintColours`, 64 rgb in 0..1; index 0 white,
    /// the colour of an uncoloured piece).
    pub colours: Vec<[f32; 3]>,
}

/// The tint table (`skatepark_presentation` record `tint` = `Hash_7628C35B98754D68`, field `TintColours` =
/// `Hash_FD2B311040A28477`, read by `825CF010`): Vector4 big-endian floats, the alpha dropped. White alone when absent.
fn tint_colours(tuning: &serde_json::Value) -> Vec<[f32; 3]> {
    let items = &tuning["skatepark_presentation"]["Hash_7628C35B98754D68"]["fields"]["Hash_FD2B311040A28477"]["array"]["items"];
    let colours: Vec<[f32; 3]> = items
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| {
            let hex = v.as_str()?;
            let word = |k: usize| u32::from_str_radix(hex.get(k * 8..k * 8 + 8)?, 16).ok().map(f32::from_bits);
            Some([word(0)?, word(1)?, word(2)?])
        })
        .collect();
    if colours.is_empty() {
        vec![[1.0; 3]]
    } else {
        colours
    }
}

/// Degrees to radians as retail scales them (0x8206D110).
const DEGREES: f32 = 0.017_453_292;

type Fields = BTreeMap<String, (String, String)>;

/// A class's records: key -> (parent, fields as hash -> (type, hex data)).
fn records(tuning: &serde_json::Value, class: &str) -> BTreeMap<String, (Option<String>, Fields)> {
    let mut out = BTreeMap::new();
    for (key, record) in tuning[class].as_object().into_iter().flatten() {
        let mut fields = Fields::new();
        for (name, field) in record["fields"].as_object().into_iter().flatten() {
            let hash = name.strip_prefix("Hash_").unwrap_or(name).to_ascii_uppercase();
            let kind = field["type"].as_str().unwrap_or_default().to_owned();
            fields.insert(hash, (kind, field["data"].as_str().unwrap_or_default().to_owned()));
        }
        out.insert(key.clone(), (record["parent"].as_str().map(str::to_owned), fields));
    }
    out
}

/// The `objectdropper` record keys the export may give as their lookup8 hash (`Hash_...`) instead of the name.
const KEY_HASHES: [(&str, &str); 3] =
    [("free_cam", "Hash_3340A8163F2B09C8"), ("manipulate", "Hash_F363F96CB5A9916A"), ("object_spawner", "Hash_0E4B664580F987BE")];

/// A record's fields with its parents' filling the gaps (the vault's inheritance).
fn resolved(records: &BTreeMap<String, (Option<String>, Fields)>, key: &str) -> Result<Fields, String> {
    let mut out = Fields::new();
    let mut next = Some(key.to_owned());
    let mut seen = 0;
    while let Some(k) = next {
        let hashed = KEY_HASHES.iter().find(|(name, _)| *name == k).map(|(_, hash)| *hash);
        let (parent, fields) = records
            .get(&k)
            .or_else(|| hashed.and_then(|h| records.get(h)))
            .ok_or_else(|| format!("object dropper tuning: no record {k}"))?;
        for (hash, value) in fields {
            out.entry(hash.clone()).or_insert_with(|| value.clone());
        }
        next = parent.clone();
        seen += 1;
        if seen > 16 {
            return Err("object dropper tuning: inheritance loop".into());
        }
    }
    Ok(out)
}

fn words(hex: &str) -> Result<Vec<u32>, String> {
    (0..hex.len() / 8).map(|i| u32::from_str_radix(&hex[i * 8..i * 8 + 8], 16).map_err(|e| e.to_string())).collect()
}

fn float(fields: &Fields, hash: &str) -> Result<f32, String> {
    let (_, data) = fields.get(hash).ok_or_else(|| format!("object dropper tuning: no field {hash}"))?;
    Ok(f32::from_bits(*words(data)?.first().ok_or_else(|| format!("object dropper tuning: {hash} empty"))?))
}

fn flag(fields: &Fields, hash: &str) -> Result<bool, String> {
    let (_, data) = fields.get(hash).ok_or_else(|| format!("object dropper tuning: no field {hash}"))?;
    Ok(data.get(..2).is_some_and(|b| b != "00"))
}

fn curve(fields: &Fields, hash: &str) -> Result<Curve, String> {
    let (_, data) = fields.get(hash).ok_or_else(|| format!("object dropper tuning: no field {hash}"))?;
    let w: Vec<f32> = words(data)?.into_iter().map(f32::from_bits).collect();
    if w.len() < 20 {
        return Err(format!("object dropper tuning: curve {hash} has {} values", w.len()));
    }
    Ok(Curve { x: w[4..12].to_vec(), y: w[12..20].to_vec() })
}

fn mode(fields: &Fields) -> Result<ModeTuning, String> {
    Ok(ModeTuning {
        move_scale: float(fields, od::MOVE)?,
        elevate_scale: float(fields, od::ELEVATE)?,
        speed_curve: curve(fields, od::SPEED_CURVE)?,
        pitch_rate: float(fields, od::PITCH_RATE)?,
        yaw_rate: float(fields, od::YAW_RATE)?,
        zoom_rate: float(fields, od::ZOOM_RATE)?,
        rotate_rate: float(fields, od::ROTATE_RATE)?,
        along_camera: flag(fields, od::ALONG_CAMERA)?,
    })
}

/// The only record of a class (`editor_camera` and `editor_cursor` hold one, `objectdropper`).
fn single(records: &BTreeMap<String, (Option<String>, Fields)>, class: &str) -> Result<Fields, String> {
    let key = records.keys().next().ok_or_else(|| format!("object dropper tuning: no {class} record"))?;
    resolved(records, key)
}

impl Tuning {
    /// From `catalog.json`'s `tuning` object.
    pub fn from_catalog(tuning: &serde_json::Value) -> Result<Self, String> {
        let modes = records(tuning, "objectdropper");
        let camera = single(&records(tuning, "editor_camera"), "editor_camera")?;
        let cursor = single(&records(tuning, "editor_cursor"), "editor_cursor")?;
        let angle = |hash: &str| float(&camera, hash).map(|v| v * DEGREES);
        Ok(Self {
            default: mode(&resolved(&modes, "default")?)?,
            free_cam: mode(&resolved(&modes, "free_cam")?)?,
            manipulate: mode(&resolved(&modes, "manipulate")?)?,
            object_spawner: mode(&resolved(&modes, "object_spawner")?)?,
            smoothing: float(&camera, cam::SMOOTHING)?,
            orbit_pitch: [angle(cam::ORBIT_PITCH_MIN)?, angle(cam::ORBIT_PITCH_MAX)?],
            free_pitch: [angle(cam::FREE_PITCH_MIN)?, angle(cam::FREE_PITCH_MAX)?],
            start_pitch: angle(cam::START_PITCH)?,
            start_distance: float(&camera, cam::START_DISTANCE)?,
            distance: [float(&camera, cam::MIN_DISTANCE)?, float(&camera, cam::MAX_DISTANCE)?],
            follow_ground: flag(&cursor, cursor::FOLLOW_GROUND)?,
            ground_step: float(&cursor, cursor::GROUND_STEP)?,
            colours: tint_colours(tuning),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curve_clamps_and_interpolates_as_retail() {
        let c = Curve { x: vec![0.0, 0.5, 1.0], y: vec![0.2, 1.0, 1.5] };
        assert_eq!(c.at(-1.0), 0.2);
        assert_eq!(c.at(2.0), 1.5);
        assert!((c.at(0.25) - 0.6).abs() < 1e-6);
        assert!((c.at(0.75) - 1.25).abs() < 1e-6);
    }
}
