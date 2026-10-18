//! The decoded audio described by private/audio/audio_manifest.json. Clips are
//! read on first use (sample banks are small; an ambience bed is ~25 MB of
//! PCM) and kept until released.
use tracing::{info, warn};
use crate::skate_mods;
use serde::Deserialize;
use std::{
    collections::{BTreeMap, HashMap},
    path::{Component, Path, PathBuf},
    sync::Arc,
};

/// Manifests this build reads. Version 4 added the world emitters (`emitters`), version 5 the
/// native AEMS runtime's banks and projects (`aems`); an older install still plays everything else
/// until setup refreshes it.
const MANIFEST_VERSIONS: std::ops::RangeInclusive<u32> = 3..=5;

#[derive(Debug, Deserialize)]
pub(crate) struct Entry {
    pub file: String,
    // Manifests also carry `seconds` (the sample length): read only by the removed measured
    // emitter relay, so it is ignored.
}

/// One record of a map's `.ems` emitter file with its sound's attributes
/// (tools/asset_pipeline/audio_export.py `emitters`).
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct EmitterRecord {
    pub flags: u32,
    pub position: [f32; 3],
    pub extent: [f32; 3],
    pub scalars: [f32; 4],
    #[serde(default)]
    pub kind: i32,
    #[serde(default)]
    pub volume: f32,
    #[serde(default)]
    pub falloff: i32,
    #[serde(default)]
    pub bank: Option<String>,
    /// The attribute patch index: `c_emitter`'s selector (payload word 8).
    #[serde(default)]
    pub patch: i32,
    /// The record's index in its file, its sound id (16 hex digits) and, for reverb zones
    /// (`kind` 5), the attribute's reverb preset key (`99FD793BC30CF0FA`, 16 hex digits; setups
    /// before the reverb-zone stage have none).
    #[serde(default)]
    pub index: u32,
    #[serde(default)]
    pub sound_id: String,
    #[serde(default)]
    pub reverb: Option<String>,
}

/// The native AEMS runtime's inputs (audio_export.aems_files): Csis projects in install order and
/// module banks by stem, as files under the audio folder.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct AemsFiles {
    #[serde(default)]
    pub projects: Vec<String>,
    #[serde(default)]
    pub banks: BTreeMap<String, String>,
    /// `MixMapSK8.mxb` (setup copies it next to the banks; absent before 2026-10-02 installs).
    #[serde(default)]
    pub mixmap: Option<String>,
    /// SPLC patch trees by bank stem (`audio_export.splice_trees`; absent before 2026-10-02).
    #[serde(default)]
    pub splice: BTreeMap<String, String>,
}

/// A rolling grain member: the whole recording and its raw `.grain` member for the native grain
/// player (the manifest's speed `bands`, the removed interim loop's, are ignored).
#[derive(Debug, Deserialize)]
struct Grain {
    #[serde(default)]
    file: Option<String>,
    #[serde(default)]
    grain: Option<String>,
}

/// The vault's grain tuning (audio_export.grain_tuning): every float exactly as stored.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct GrainTuningJson {
    #[serde(default)]
    surfaces: BTreeMap<String, SurfaceJson>,
    #[serde(default)]
    default: SurfaceJson,
    #[serde(default)]
    owner: OwnerJson,
    /// Collision material (tag − 1) → rolling surface 1–14 (`Sk8::AudioSurfaceMap`).
    /// Read only by the vault check (`grain_for_matches_the_vault_surface_map_and_the_bed_loads`).
    #[serde(default)]
    #[cfg_attr(not(test), allow(dead_code))]
    pub surface_map: Vec<u32>,
}

#[derive(Debug, Default, Clone, Deserialize)]
struct SurfaceJson {
    max_kmh: Option<f32>,
    bezier: Option<[f32; 4]>,
    params: Option<Vec<Vec<f32>>>,
    turn_cap: Option<f32>,
    turn_rise_step: Option<f32>,
    turn_fall_step: Option<f32>,
    special_gain: Option<f32>,
    special_shift_hz: Option<f32>,
    b_slope_gain: Option<f32>,
    b_slope_ramp_kmh: Option<f32>,
    a_shift_per_slope_hz: Option<f32>,
    b_base_shift_hz: Option<f32>,
    b_shift_per_slope_hz: Option<f32>,
    push_ramp_kmh: Option<f32>,
    push_scale_low: Option<f32>,
    push_scale_high: Option<f32>,
    push_shift_low_hz: Option<f32>,
    push_shift_high_hz: Option<f32>,
    push_scale_attack_ms: Option<f32>,
    push_scale_hold_ms: Option<f32>,
    push_scale_return_ms: Option<f32>,
    push_shift_attack_ms: Option<f32>,
    push_shift_hold_ms: Option<f32>,
    push_shift_return_ms: Option<f32>,
    slope_down_divisor: Option<f32>,
    slope_up_divisor: Option<f32>,
}

#[derive(Debug, Default, Deserialize)]
struct OwnerJson {
    rocket_start_kmh: Option<f32>,
    rocket_top_kmh: Option<f32>,
    rocket_gain_word: Option<i32>,
    rocket_params: Option<Vec<f32>>,
    g1_level_start_kmh: Option<f32>,
    g1_level_end_kmh: Option<f32>,
    g1_level_floor: Option<f32>,
    // The board chains' tuning (`grain::chain::ChainTuning`; absent before the chain export).
    g3_send_start_kmh: Option<f32>,
    g3_send_end_kmh: Option<f32>,
    g3_send_max: Option<f32>,
    g3_clip: Option<f32>,
    g3_shelf_hz: Option<f32>,
    g3_shelf_gain: Option<f32>,
    wobble_start_kmh: Option<f32>,
    wobble_end_kmh: Option<f32>,
    wobble_a_ms_low: Option<i32>,
    wobble_a_ms_high: Option<i32>,
    wobble_a_low: Option<f32>,
    wobble_a_high: Option<f32>,
    wobble_b_ms_low: Option<i32>,
    wobble_b_ms_high: Option<i32>,
    wobble_b_low: Option<f32>,
    wobble_b_high: Option<f32>,
}

impl SurfaceJson {
    /// The member's tuning with every missing field from `default`; None if a field is missing
    /// from both (an install without the vault tuning).
    fn tuning(&self, d: &SurfaceJson) -> Option<skate_audio::grain::board::SurfaceTuning> {
        use skate_audio::grain::{GrainParams, board::{PushTuning, SurfaceTuning}};
        let f = |a: Option<f32>, b: Option<f32>| a.or(b);
        let params = self.params.as_ref().or(d.params.as_ref())?;
        let params = [GrainParams::from_slice(params.first()?)?, GrainParams::from_slice(params.get(1)?)?];
        Some(SurfaceTuning {
            max_kmh: f(self.max_kmh, d.max_kmh)?,
            bezier: self.bezier.or(d.bezier)?,
            params,
            turn_cap: f(self.turn_cap, d.turn_cap)?,
            turn_rise_step: f(self.turn_rise_step, d.turn_rise_step)?,
            turn_fall_step: f(self.turn_fall_step, d.turn_fall_step)?,
            special_gain: f(self.special_gain, d.special_gain)?,
            special_shift_hz: f(self.special_shift_hz, d.special_shift_hz)?,
            b_slope_gain: f(self.b_slope_gain, d.b_slope_gain)?,
            b_slope_ramp_kmh: f(self.b_slope_ramp_kmh, d.b_slope_ramp_kmh)?,
            a_shift_per_slope_hz: f(self.a_shift_per_slope_hz, d.a_shift_per_slope_hz)?,
            b_base_shift_hz: f(self.b_base_shift_hz, d.b_base_shift_hz)?,
            b_shift_per_slope_hz: f(self.b_shift_per_slope_hz, d.b_shift_per_slope_hz)?,
            // Installs staged before these fields were read fall back to the vault's values.
            slope_divisors: (
                f(self.slope_down_divisor, d.slope_down_divisor).unwrap_or(-10.0),
                f(self.slope_up_divisor, d.slope_up_divisor).unwrap_or(10.0),
            ),
            push: PushTuning {
                ramp_kmh: f(self.push_ramp_kmh, d.push_ramp_kmh)?,
                scale_low: f(self.push_scale_low, d.push_scale_low)?,
                scale_high: f(self.push_scale_high, d.push_scale_high)?,
                shift_low_hz: f(self.push_shift_low_hz, d.push_shift_low_hz)?,
                shift_high_hz: f(self.push_shift_high_hz, d.push_shift_high_hz)?,
                scale_ms: [
                    f(self.push_scale_attack_ms, d.push_scale_attack_ms)?,
                    f(self.push_scale_hold_ms, d.push_scale_hold_ms)?,
                    f(self.push_scale_return_ms, d.push_scale_return_ms)?,
                ],
                shift_ms: [
                    f(self.push_shift_attack_ms, d.push_shift_attack_ms)?,
                    f(self.push_shift_hold_ms, d.push_shift_hold_ms)?,
                    f(self.push_shift_return_ms, d.push_shift_return_ms)?,
                ],
            },
        })
    }
}

/// One sound of a location set (audio_export.random_sets).
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct RandomSound {
    pub bank: String,
    pub volume: f32,
    /// Retail's timeout for the post (seconds).
    pub seconds: f32,
    pub weight: i32,
}

/// A location set of random distant one-shots (class `aud_wp_emitters`).
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct RandomSet {
    #[serde(default)]
    pub name: Option<String>,
    pub sounds: Vec<RandomSound>,
    #[serde(default = "default_min_level")]
    pub min_level: f32,
    #[serde(default = "one")]
    pub max_level: f32,
    #[serde(default = "default_min_interval")]
    pub min_interval: f32,
    #[serde(default = "default_max_interval")]
    pub max_interval: f32,
}
fn default_min_level() -> f32 { 0.2 }
fn one() -> f32 { 1.0 }
fn default_min_interval() -> f32 { 10.0 }
fn default_max_interval() -> f32 { 20.0 }

/// One tile of a world-painter region layer: a quadtree of (x, z) boxes whose leaves index `keys`
/// (audio_formats.region_layers).
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct RegionTile {
    /// Centre x, z and half sizes x, z.
    pub r#box: [f32; 4],
    /// Four child indices, then the value; child 0 = `NO_KEY` marks a leaf.
    pub nodes: Vec<[u16; 5]>,
    pub keys: Vec<String>,
}
pub(crate) const NO_KEY: u16 = 0xFFFF;

impl RegionTile {
    /// The key at (x, z): None outside the tile or on a leaf without a key (retail's walk:
    /// inclusive edges, children tested -x-z, -x+z, +x-z, +x+z).
    pub(crate) fn key(&self, x: f32, z: f32) -> Option<u64> {
        let [mut cx, mut cz, mut hx, mut hz] = self.r#box;
        if self.nodes.is_empty() || (x - cx).abs() > hx || (z - cz).abs() > hz {
            return None;
        }
        let mut index = 0usize;
        for _ in 0..64 {
            let node = self.nodes.get(index)?;
            if node[0] == NO_KEY {
                return if node[4] == NO_KEY { None } else { u64::from_str_radix(self.keys.get(usize::from(node[4]))?, 16).ok() };
            }
            hx *= 0.5;
            hz *= 0.5;
            let child = [(-1.0, -1.0), (-1.0, 1.0), (1.0, -1.0), (1.0, 1.0)].iter().position(|(sx, sz)| {
                (x - (cx + sx * hx)).abs() <= hx && (z - (cz + sz * hz)).abs() <= hz
            })?;
            let (sx, sz) = [(-1.0, -1.0), (-1.0, 1.0), (1.0, -1.0), (1.0, 1.0)][child];
            cx += sx * hx;
            cz += sz * hz;
            index = usize::from(node[child]);
        }
        None
    }
}

/// A zone ambience (class `aud_wp_ambiences`): its bed and fades (audio_export.ambience_zones).
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct Zone {
    #[serde(default)]
    pub name: Option<String>,
    /// The ambience bed stream (e.g. `06_dt_open`), None for zones without one.
    #[serde(default)]
    pub bed: Option<String>,
    #[serde(default = "one")]
    pub volume: f32,
    /// Fade-out time (s) when leaving this zone (record +8).
    #[serde(default = "one")]
    pub time_a: f32,
    /// Fade-in time (s) when entering it (record +12).
    #[serde(default = "one")]
    pub time_b: f32,
}

/// A zone-pair crossfade (class `aud_wp_ambience_crossfades`).
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct Crossfade {
    pub from: String,
    pub to: String,
    pub group: u32,
    #[serde(default = "one")]
    pub level: f32,
}

#[derive(Debug, Deserialize)]
struct Manifest {
    version: u32,
    ambience: BTreeMap<String, Entry>,
    grains: BTreeMap<String, Grain>,
    wheels: BTreeMap<String, Entry>,
    banks: BTreeMap<String, Vec<Entry>>,
    /// Emitter file stem (`sfx_university`, ...) -> its records.
    #[serde(default)]
    emitters: BTreeMap<String, Vec<EmitterRecord>>,
    /// Location set key (16 hex digits) -> set.
    #[serde(default)]
    random_sets: BTreeMap<String, RandomSet>,
    /// Zone key (16 hex digits) -> zone ambience.
    #[serde(default)]
    zones: BTreeMap<String, Zone>,
    #[serde(default)]
    crossfades: Vec<Crossfade>,
    /// District (map stem) -> region layer name -> tiles.
    #[serde(default)]
    regions: BTreeMap<String, BTreeMap<String, Vec<RegionTile>>>,
    #[serde(default)]
    aems: AemsFiles,
    /// The native grain player's vault tuning (absent before 2026-10-02 installs).
    #[serde(default)]
    grain_player: GrainTuningJson,
    /// The native player components' vault tuning (audio_export.player_tuning; optional).
    #[serde(default)]
    player_tuning: PlayerTuningJson,
    /// The native environment network's presets and the eEQChain buses (optional).
    #[serde(default)]
    bus_tuning: BusTuningJson,
    /// The world sources' vault tuning (traffic engine records, ped footsteps; optional).
    #[serde(default)]
    world_tuning: crate::world_sources::WorldTuningJson,
    /// The streamed speech exports (`speech.livingworld`: the index and, after the opt-in decode
    /// `SKATE_SETUP_SPEECH=1`, the decoded takes; optional).
    #[serde(default)]
    speech: BTreeMap<String, SpeechEntry>,
    /// The front-end sounds (`fe` records: the session marker's cellphone UI, menus; optional,
    /// audio_export.frontend_sounds).
    #[serde(default)]
    frontend: FrontendJson,
    // Audio content overlays only (`skate_mods::audio_merge`; never in an install's manifest).
    /// Bank → S10A slot → the loop start (frames) of a mod WAV.
    #[serde(default)]
    mod_sample_loops: BTreeMap<String, BTreeMap<String, u32>>,
    /// Mod banks' load policy and volume group.
    #[serde(default)]
    mod_banks: BTreeMap<String, ModBankJson>,
    /// Speech archive → replaced takes and extra takes.
    #[serde(default)]
    mod_speech: BTreeMap<String, ModSpeechJson>,
    /// Bank → the interim location-set layers of a mod bank.
    #[serde(default)]
    mod_location_programs: BTreeMap<String, Vec<LayerJson>>,
    /// Crossfade bank → group (decimal) → declared crossfade voices.
    #[serde(default)]
    mod_crossfade_layouts: BTreeMap<String, BTreeMap<String, Vec<CrossfadeVoiceJson>>>,
    /// Map stem → map audio from overlays.
    #[serde(default)]
    mod_maps: BTreeMap<String, MapJson>,
}

#[derive(Debug, Default, Clone, Deserialize)]
struct ModBankJson {
    #[serde(default)]
    preload: bool,
    #[serde(default)]
    group: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct ModSpeechJson {
    /// Clip (without `.dat`) → take → file.
    #[serde(default)]
    takes: BTreeMap<String, BTreeMap<String, String>>,
    /// Clip → extra takes after the clip's own.
    #[serde(default)]
    extra: BTreeMap<String, Vec<String>>,
}

/// One voice of a declared crossfade group (`add.crossfade_layouts`): sample slot, degrees
/// (0 = ahead, 90 = right), level.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct CrossfadeVoiceJson {
    pub sample: usize,
    #[serde(default)]
    pub pan: f32,
    #[serde(default = "one")]
    pub level: f32,
}

/// One layer of a mod bank's location-set post (`random_sets` interim player).
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct LayerJson {
    #[serde(default)]
    pub delay: f32,
    /// A slot, or `"shuffle"`.
    pub sample: serde_json::Value,
    #[serde(default = "one")]
    pub level: f32,
    #[serde(default)]
    pub pan_sweep: f32,
    #[serde(default)]
    pub looping: bool,
}

/// A map's audio from a mod overlay, a `<map>.audio.json` sidecar or a `.skate` `AUDO` tag
/// (`skate_mods::audio_content::MapAudioDef`; records with their index).
#[derive(Debug, Default, Clone, Deserialize)]
pub(crate) struct MapJson {
    #[serde(default)]
    pub district: Option<String>,
    #[serde(default)]
    pub ems: Option<Vec<String>>,
    #[serde(default)]
    pub crossfade_bank: Option<String>,
    #[serde(default)]
    pub fallback_bed: Option<String>,
    #[serde(default)]
    pub emitters: Vec<EmitterRecord>,
    #[serde(default)]
    pub regions: BTreeMap<String, Vec<BoxJson>>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct BoxJson {
    pub r#box: [f32; 4],
    pub key: String,
}

/// `audio_export.frontend_sounds`: the Splice bank and every `fe` record by key (16 hex digits).
#[derive(Debug, Default, Deserialize)]
struct FrontendJson {
    #[serde(default)]
    bank: String,
    #[serde(default)]
    sounds: BTreeMap<String, FeSoundJson>,
}

#[derive(Debug, Deserialize)]
struct FeSoundJson {
    #[serde(default)]
    name: String,
    #[serde(default)]
    id: i32,
    #[serde(default = "one")]
    level: f32,
    #[serde(default)]
    hom: i32,
    #[serde(default)]
    moment: i32,
    #[serde(default)]
    alt_bus: bool,
}


/// One speech archive's export: the index JSON and the folder of decoded takes (None: not decoded).
#[derive(Debug, Default, Deserialize)]
struct SpeechEntry {
    #[serde(default)]
    index: String,
    #[serde(default)]
    audio: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct JitterJson {
    /// The collection key (16 hex digits; absent before the eEQChain bus export).
    #[serde(default)]
    key: String,
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    id: i64,
    params: Vec<f32>,
}

#[derive(Debug, Default, Deserialize)]
struct SeamJson {
    gain_low: Option<f32>,
    gain_high: Option<f32>,
    ms_low: Option<i32>,
    ms_high: Option<i32>,
    // Class_Seams' pattern fields (absent before the seams export).
    gain: Option<f32>,
    angle: Option<i32>,
    grid_z: Option<f32>,
    grid_x: Option<f32>,
    class: Option<i32>,
    mode: Option<i32>,
    min_frames: Option<i32>,
    speed_threshold: Option<f32>,
    spacing: Option<f32>,
    level: Option<f32>,
    surface3_scale: Option<f32>,
}

/// The native buses' vault tuning (`audio_export.bus_tuning`; optional).
#[derive(Debug, Default, Deserialize)]
struct BusTuningJson {
    /// Reverb preset key (16 hex digits) → its 44 record values by offset.
    #[serde(default)]
    reverb: BTreeMap<String, Vec<f32>>,
    #[serde(default)]
    eq_buses: Vec<EqBusJson>,
    /// The two FlangeSub effect returns' records (A, B), nine values each by record offset.
    #[serde(default)]
    flange: Vec<Vec<f32>>,
}

#[derive(Debug, Default, Deserialize)]
struct EqBusJson {
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    clip: f32,
    #[serde(default)]
    ranges: Vec<[f32; 2]>,
}

#[derive(Debug, Default, Deserialize)]
struct GrindJson {
    v: Vec<f32>,
    f: Vec<f32>,
    /// The grind contact sounds (exported since 2026-10-03; optional).
    #[serde(default)]
    metal: bool,
    on: Option<GrindContactJson>,
    off: Option<GrindContactJson>,
}

#[derive(Debug, Default, Deserialize)]
struct GrindContactJson {
    ids: Vec<i32>,
    gain: Vec<f32>,
    level: Vec<f32>,
    pitch: Vec<f32>,
}

impl GrindContactJson {
    fn contact(&self) -> skate_audio::player::tuning::GrindContact {
        let d = skate_audio::player::tuning::GrindContact::default();
        let two = |v: &[f32], d: [f32; 2]| if v.len() == 2 { [v[0], v[1]] } else { d };
        skate_audio::player::tuning::GrindContact {
            ids: std::array::from_fn(|i| self.ids.get(i).copied().unwrap_or(-1)),
            gain: std::array::from_fn(|i| self.gain.get(i).copied().unwrap_or(1.0)),
            level: two(&self.level, d.level),
            pitch: two(&self.pitch, d.pitch),
        }
    }
}

#[derive(Debug, Default, Deserialize)]
struct PlayerTuningJson {
    #[serde(default)]
    surface_table: Vec<Vec<i32>>,
    #[serde(default)]
    jitter: Vec<JitterJson>,
    #[serde(default)]
    seam_wobbles: Vec<SeamJson>,
    #[serde(default)]
    grind: Vec<GrindJson>,
    #[serde(default)]
    landing_materials: Vec<u32>,
    wheel_bucket_high: Option<f32>,
    wheel_bucket_low: Option<f32>,
    jump_thresholds: Option<[f32; 2]>,
    /// Name hash (16 hex digits) → audio trick id.
    #[serde(default)]
    audio_tricks: BTreeMap<String, i32>,
    /// The scorables' second audio trick field (`A2C5C22C5BE725F8`, state `+352`), keyed the same way.
    #[serde(default)]
    audio_tricks_2: BTreeMap<String, i32>,
    /// The collision manager's material table and the posters' vault values
    /// (`audio_export.collision_tuning`; optional).
    #[serde(default)]
    collision: CollisionJson,
    /// The grind contact sounds' eEQChain bus (`D1A87641CCB98787`; optional).
    grind_contact_eq: Option<u8>,
}

#[derive(Debug, Default, Deserialize)]
struct CollisionJson {
    #[serde(default)]
    materials: Vec<MaterialJson>,
    #[serde(default)]
    posters: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Default, Deserialize)]
struct MaterialJson {
    #[serde(default = "no_kind")]
    kind: i32,
    #[serde(default)]
    ids: Vec<i32>,
    #[serde(default)]
    gain: i32,
    pitch: Option<i32>,
    #[serde(default)]
    pitch_flag: bool,
    #[serde(default)]
    pitch_alt: i32,
    #[serde(default)]
    category: i32,
    #[serde(default)]
    landing: bool,
    #[serde(default)]
    windows: Vec<i32>,
    #[serde(default)]
    scale: f32,
    #[serde(default)]
    bands: Vec<f32>,
    /// The footstep layer (`player::footsteps::FootstepMaterial`; absent before that export).
    #[serde(default)]
    footsteps: bool,
    step_gain: Option<i32>,
    step_landing_gain: Option<i32>,
}

fn no_kind() -> i32 {
    -1
}

impl CollisionJson {
    fn tuning(&self, surface_table: &[Vec<i32>]) -> skate_audio::player::collision::CollisionTuning {
        use skate_audio::player::collision::{CollisionTuning, Material};
        CollisionTuning {
            materials: self
                .materials
                .iter()
                .map(|m| Material {
                    kind: m.kind,
                    ids: std::array::from_fn(|i| m.ids.get(i).copied().unwrap_or(0)),
                    gain: m.gain,
                    pitch: m.pitch.unwrap_or(4096),
                    pitch_flag: m.pitch_flag,
                    pitch_alt: m.pitch_alt,
                    category: m.category,
                    landing: m.landing,
                    windows: std::array::from_fn(|i| m.windows.get(i).copied().unwrap_or(0)),
                    scale: m.scale,
                    bands: std::array::from_fn(|i| m.bands.get(i).copied().unwrap_or(0.0)),
                })
                .collect(),
            // AudioSurfaceMap word 7 (+28): the collision class.
            surface_class: surface_table.iter().map(|r| r.get(7).copied().unwrap_or(0)).collect(),
            // Words 12..16 (+48..+64): the collision voices' eEQChain bus by tier / class.
            surface_eq: surface_table.iter().map(|r| std::array::from_fn(|i| r.get(12 + i).copied().unwrap_or(8))).collect(),
        }
    }

    /// The posters' vault values over the retail defaults.
    pub(crate) fn contacts(&self) -> skate_audio::player::contacts::ContactsTuning {
        let mut c = skate_audio::player::contacts::ContactsTuning::default();
        let f = |k: &str| self.posters.get(k).and_then(serde_json::Value::as_f64).map(|v| v as f32);
        let ids = |k: &str| -> Option<Vec<u32>> {
            self.posters.get(k)?.as_array().map(|a| a.iter().filter_map(|v| v.as_u64().map(|v| v as u32)).collect())
        };
        let three = |v: Option<Vec<u32>>, d: [u32; 3]| v.filter(|v| v.len() == 3).map_or(d, |v| [v[0], v[1], v[2]]);
        let two = |v: Option<Vec<u32>>, d: [u32; 2]| v.filter(|v| v.len() == 2).map_or(d, |v| [v[0], v[1]]);
        c.grind_split = f("grind_split").unwrap_or(c.grind_split);
        c.grind_high = f("grind_high").unwrap_or(c.grind_high);
        c.landing_air = f("landing_air").unwrap_or(c.landing_air);
        c.landing_board = f("landing_board").map_or(c.landing_board, |v| v as i32);
        c.landing_split = f("landing_split").unwrap_or(c.landing_split);
        c.landing_high = f("landing_high").unwrap_or(c.landing_high);
        c.landing_scale = [f("landing_scale_a").unwrap_or(c.landing_scale[0]), f("landing_scale_b").unwrap_or(c.landing_scale[1])];
        c.deck_cooldown = f("deck_cooldown").unwrap_or(c.deck_cooldown);
        c.scuff_speed = f("scuff_speed").unwrap_or(c.scuff_speed);
        c.scuff_ids = two(ids("scuff_ids"), c.scuff_ids);
        c.scuff_ids_soft = two(ids("scuff_ids_soft"), c.scuff_ids_soft);
        c.tap_off_ms = f("tap_off_ms").unwrap_or(c.tap_off_ms);
        c.tap_speed = f("tap_speed").unwrap_or(c.tap_speed);
        c.tap_mid = f("tap_mid").unwrap_or(c.tap_mid);
        c.tap_high = f("tap_high").unwrap_or(c.tap_high);
        c.tap_ids = [
            three(ids("tap_first"), c.tap_ids[0]),
            three(ids("tap_second"), c.tap_ids[1]),
            three(ids("tap_both"), c.tap_ids[2]),
            three(ids("tap_special"), c.tap_ids[3]),
        ];
        c.tap_ids_soft = [
            three(ids("tap_first_soft"), c.tap_ids_soft[0]),
            three(ids("tap_second_soft"), c.tap_ids_soft[1]),
            three(ids("tap_other_soft"), c.tap_ids_soft[2]),
        ];
        // The push foot's plant / lift ids by material kind, their eEQChain bus; the body poster's
        // cooldown and pad thresholds (exported since 2026-10-03; the retail defaults otherwise).
        let five = |v: Option<Vec<u32>>, d: [u32; 5]| v.filter(|v| v.len() == 5).map_or(d, |v| [v[0], v[1], v[2], v[3], v[4]]);
        c.plant_ids = five(ids("plant_ids"), c.plant_ids);
        c.lift_ids = five(ids("lift_ids"), c.lift_ids);
        c.plant_eq = f("plant_eq").map_or(c.plant_eq, |v| v as u8);
        c.body_cooldown = f("body_cooldown").unwrap_or(c.body_cooldown);
        // The bridge's speed graph (exported since 2026-10-03; the stock vault's words otherwise).
        let eight = |k: &str| -> Option<[f32; 8]> {
            let a = self.posters.get(k)?.as_array()?;
            let v: Vec<f32> = a.iter().filter_map(|v| v.as_f64().map(|v| v as f32)).collect();
            (v.len() == 8).then(|| std::array::from_fn(|i| v[i]))
        };
        if let (Some(x), Some(y)) = (eight("body_speed_x"), eight("body_speed_y")) {
            c.body_speed_curve = skate_audio::player::contacts::SpeedGraph8 { x, y };
        }
        let pair = |a: &str, b: &str, d: [f32; 2]| [f(a).unwrap_or(d[0]), f(b).unwrap_or(d[1])];
        c.body_110 = [pair("body_110_head_low", "body_110_head_high", c.body_110[0]), pair("body_110_torso_low", "body_110_torso_high", c.body_110[1])];
        c.body_111 = pair("body_111_low", "body_111_high", c.body_111);
        c.body_112 = [pair("body_112_low0", "body_112_high0", c.body_112[0]), pair("body_112_low1", "body_112_high1", c.body_112[1])];
        c
    }
}

impl PlayerTuningJson {
    fn tuning(&self) -> skate_audio::player::tuning::PlayerTuning {
        use skate_audio::player::tuning::{GrindSurface, JitterParams, PlayerTuning, SeamWobble};
        let d = PlayerTuning::default();
        let four = |v: &[f32]| -> [f32; 4] { std::array::from_fn(|i| v.get(i).copied().unwrap_or(1.0)) };
        PlayerTuning {
            surface_table: self.surface_table.iter().map(|r| std::array::from_fn(|i| r.get(i).copied().unwrap_or(0))).collect(),
            jitter: self.jitter.iter().filter(|j| j.params.len() == 4).map(|j| JitterParams {
                enabled: j.enabled,
                id: j.id.clamp(0, 15) as usize,
                centre: j.params[0],
                range: j.params[1],
                max_step: j.params[2],
                min_step: j.params[3],
            }).collect(),
            // A pattern without a collection reads the image's zero block (all fields 0).
            seam_wobbles: self.seam_wobbles.iter().map(|w| SeamWobble {
                gain_low: w.gain_low.unwrap_or(0.0),
                gain_high: w.gain_high.unwrap_or(0.0),
                ms_low: w.ms_low.unwrap_or(0),
                ms_high: w.ms_high.unwrap_or(0),
            }).collect(),
            grind: self.grind.iter().map(|g| GrindSurface {
                v: four(&g.v),
                f: four(&g.f),
                metal: g.metal,
                on: g.on.as_ref().map_or_else(Default::default, GrindContactJson::contact),
                off: g.off.as_ref().map_or_else(Default::default, GrindContactJson::contact),
            }).collect(),
            grind_contact_eq: self.grind_contact_eq.unwrap_or(d.grind_contact_eq),
            landing_materials: self.landing_materials.clone(),
            wheel_bucket_high: self.wheel_bucket_high.unwrap_or(d.wheel_bucket_high),
            wheel_bucket_low: self.wheel_bucket_low.unwrap_or(d.wheel_bucket_low),
            jump_thresholds: self.jump_thresholds.unwrap_or(d.jump_thresholds),
            audio_tricks: self.audio_tricks.iter().filter_map(|(k, v)| Some((u64::from_str_radix(k, 16).ok()?, *v))).collect(),
            audio_tricks_2: self.audio_tricks_2.iter().filter_map(|(k, v)| Some((u64::from_str_radix(k, 16).ok()?, *v))).collect(),
            // The rolling layers', the Tricks component's and Class_Treatment's vault words: the
            // defaults are the user's vault values (not exported by setup yet).
            rolling: Default::default(),
            tricks: Default::default(),
            treatment: Default::default(),
            collision: self.collision.tuning(&self.surface_table),
            seam_patterns: self.seam_wobbles.iter().map(|w| skate_audio::player::tuning::SeamPattern {
                gain: w.gain.unwrap_or(0.0),
                angle: w.angle.unwrap_or(0),
                grid_z: w.grid_z.unwrap_or(0.0),
                grid_x: w.grid_x.unwrap_or(0.0),
                class: w.class.unwrap_or(0),
                mode: w.mode.unwrap_or(0),
                min_frames: w.min_frames.unwrap_or(0),
                speed_threshold: w.speed_threshold.unwrap_or(0.0),
                spacing: w.spacing.unwrap_or(0.0),
                level: w.level.unwrap_or(0.0),
            }).collect(),
            seam_surface3_scale: self.seam_wobbles.first().and_then(|w| w.surface3_scale).unwrap_or(1.0),
            eq_jitter: {
                let channels: Vec<&JitterJson> = self.jitter.iter().filter(|j| j.params.len() == 4).collect();
                skate_audio::player::tuning::EQ_JITTER_KEYS.map(|k| {
                    channels.iter().position(|j| u64::from_str_radix(&j.key, 16).ok() == Some(k))
                })
            },
        }
    }
}


/// Channel count of a WAV's `fmt ` chunk (1 if unreadable).
pub(crate) fn wav_channels(bytes: &[u8]) -> u16 {
    let mut at = 12;
    while at + 8 <= bytes.len() {
        let size = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().unwrap()) as usize;
        let body = at + 8;
        if &bytes[at..at + 4] == b"fmt " && body + 4 <= bytes.len() {
            return u16::from_le_bytes([bytes[body + 2], bytes[body + 3]]).max(1);
        }
        at = body + size + (size & 1);
    }
    1
}

/// Loudest sample of a PCM16 WAV as a fraction of full scale (1.0 if unreadable).
pub(crate) fn wav_peak(bytes: &[u8]) -> f32 {
    let mut at = 12;
    while at + 8 <= bytes.len() {
        let size = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().unwrap()) as usize;
        let body = at + 8;
        if &bytes[at..at + 4] == b"data" {
            let data = &bytes[body..(body + size).min(bytes.len())];
            let max = data.chunks_exact(2).map(|s| i16::from_le_bytes([s[0], s[1]]).unsigned_abs()).max().unwrap_or(0);
            return (max as f32 / 32768.0).max(1e-3);
        }
        at = body + size + (size & 1);
    }
    1.0
}

/// A PCM16 WAV as planar f32 (−1..1), or None if it is not one.
pub(crate) fn wav_pcm(bytes: &[u8]) -> Option<skate_audio::mixer::Pcm> {
    // skate_vita: sound.pak holds the decoded WAVs as `.ima` (4-bit IMA ADPCM, `skate_audio::adpcm`).
    if bytes.starts_with(b"SKIM") {
        return skate_audio::adpcm::pcm_from_ima(bytes).ok();
    }
    let (mut channels, mut rate, mut bits) = (0usize, 0u32, 0u16);
    let mut at = 12;
    while at + 8 <= bytes.len() {
        let size = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().ok()?) as usize;
        let body = at + 8;
        match &bytes[at..at + 4] {
            b"fmt " if body + 16 <= bytes.len() => {
                channels = usize::from(u16::from_le_bytes([bytes[body + 2], bytes[body + 3]]));
                rate = u32::from_le_bytes(bytes[body + 4..body + 8].try_into().ok()?);
                bits = u16::from_le_bytes([bytes[body + 14], bytes[body + 15]]);
            }
            b"data" if channels > 0 && bits == 16 => {
                let data = &bytes[body..(body + size).min(bytes.len())];
                let mut planar = vec![Vec::with_capacity(data.len() / (2 * channels)); channels];
                for (i, s) in data.chunks_exact(2).enumerate() {
                    planar[i % channels].push(f32::from(i16::from_le_bytes([s[0], s[1]])) / 32768.0);
                }
                return Some(skate_audio::mixer::Pcm { rate, channels: planar.into_iter().map(skate_audio::adpcm::Samples::F32).collect() });
            }
            _ => {}
        }
        at = body + size + (size & 1);
    }
    None
}

/// A bank's decoded samples by S10A slot (None where a WAV is missing or unreadable).
pub(crate) type BankPcm = Vec<Option<Arc<skate_audio::mixer::Pcm>>>;

/// skate_vita: a sample file as PCM; an `.ima` keeps sharing the mounted bytes (no second copy
/// of sound.pak).
pub(crate) fn shared_pcm(file: &Path) -> Option<skate_audio::mixer::Pcm> {
    let (bytes, range) = skate_data::vfs::read_shared(file).ok()?;
    if bytes[range.clone()].starts_with(b"SKIM") {
        return skate_audio::adpcm::pcm_from_ima_shared(bytes, range).ok();
    }
    wav_pcm(&bytes[range])
}

/// Read and decode the WAVs in order (`Library::bank_pcm` and `BankSource::load`).
fn decode_wavs<'a>(files: impl Iterator<Item = &'a Path>) -> BankPcm {
    files
        .map(|file| {
            #[cfg(test)]
            WAV_DECODES.with(|n| n.set(n.get() + 1));
            shared_pcm(file).map(Arc::new)
        })
        .collect()
}

/// The sample header a mod WAV gets in an AEMS bank: voices play by the header (frames, rate,
/// channels, loop), so a replacement of another length or rate needs its own. The codec stays the
/// slot's; a slot that looped loops from frame 0 unless the overlay gives `loop_start`.
pub(crate) fn mod_header(old: Option<skate_audio::formats::SampleHeader>, pcm: &skate_audio::mixer::Pcm, loop_start: Option<u32>) -> skate_audio::formats::SampleHeader {
    let frames = pcm.channels.first().map_or(0, |c| c.len() as u32);
    let looped = loop_start.or(old.and_then(|h| h.loop_start).map(|_| 0));
    skate_audio::formats::SampleHeader {
        codec: old.map_or(0, |h| h.codec),
        channels: pcm.channels.len().clamp(1, 6) as u8,
        rate: pcm.rate,
        frames,
        loop_start: looped.filter(|&l| l < frames),
    }
}

#[cfg(test)]
thread_local! {
    /// WAVs read and decoded on this thread (tests: no decode on the game thread at emitter start).
    pub(crate) static WAV_DECODES: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// Everything needed to read, parse and decode one AEMS bank (`Library::bank_source`), with no
/// reference to the library: `native::prefetch` runs [`BankSource::load`] on its worker thread.
/// The game thread's own load (`Native::ensure_bank`) runs the same function, so both give the
/// same bank and PCM.
/// Each file is an absolute path (a retail bank with a replaced WAV mixes the install and a mod);
/// `rebuild` lists the slots whose header comes from their (mod) WAV, with the loop start.
#[derive(Clone, Debug)]
pub(crate) struct BankSource {
    stem: String,
    file: String,
    abk: PathBuf,
    wavs: Vec<PathBuf>,
    rebuild: Vec<(usize, Option<u32>)>,
}

impl BankSource {
    #[cfg(test)]
    pub(crate) fn for_test(root: PathBuf, stem: &str, file: &str, wavs: Vec<String>) -> Self {
        Self { stem: stem.to_owned(), file: file.to_owned(), abk: root.join(file), wavs: wavs.iter().map(|w| root.join(w)).collect(), rebuild: Vec::new() }
    }

    pub(crate) fn stem(&self) -> &str {
        &self.stem
    }

    /// The `.abk` parsed and its WAVs decoded; the errors are `ensure_bank`'s. Mod WAVs get their
    /// headers from the PCM ([`mod_header`]).
    pub(crate) fn load(&self) -> Result<(skate_audio::formats::Bank, BankPcm), String> {
        let bytes = skate_data::vfs::read(&self.abk).map_err(|e| format!("{}: {e}", self.file))?;
        let mut bank = skate_audio::formats::Bank::parse(&self.stem, bytes).map_err(|e| e.to_string())?;
        let pcm = decode_wavs(self.wavs.iter().map(PathBuf::as_path));
        for &(slot, loop_start) in &self.rebuild {
            if let (Some(entry), Some(Some(p))) = (bank.samples.get_mut(slot), pcm.get(slot)) {
                entry.1 = Some(mod_header(entry.1, p, loop_start));
            }
        }
        Ok((bank, pcm))
    }
}

pub struct Library {
    root: PathBuf,
    manifest: Manifest,
    /// The merged audio content overlays' roots by mod id (`mod:<id>/<path>` files).
    mods: BTreeMap<String, PathBuf>,
    failed: std::collections::HashSet<String>,
    /// The tuning sections as loaded (JSON, the overlays' merge included): the base of the runtime
    /// tuning writes (`tuning.rs`). Read from the install manifest on first use when no overlay was
    /// merged (nothing is parsed twice without tuning writes).
    tuning_raw: std::sync::OnceLock<Result<serde_json::Map<String, serde_json::Value>, String>>,
    /// The loaded tuning sections a runtime tuning write replaced, kept to put back exactly.
    tuning_saved: TuningSaved,
    /// Mod files' stamps (size, modification time) taken at load (`stamp`).
    stamps: HashMap<String, String>,
}

/// The tuning sections the runtime tuning writes replace (`Library::set_tuning`).
pub(crate) const TUNING_SECTIONS: [&str; 3] = ["player_tuning", "world_tuning", "bus_tuning"];

#[derive(Default)]
struct TuningSaved {
    player: Option<PlayerTuningJson>,
    world: Option<crate::world_sources::WorldTuningJson>,
    bus: Option<BusTuningJson>,
}

fn tuning_sections(v: &serde_json::Value) -> serde_json::Map<String, serde_json::Value> {
    TUNING_SECTIONS.iter().filter_map(|k| v.get(*k).map(|x| ((*k).to_owned(), x.clone()))).collect()
}

fn safe_relative(file: &str) -> bool {
    let path = Path::new(file);
    // ':' also rules out drive prefixes on platforms that would parse "C:" as a name.
    !file.is_empty() && !file.contains(':') && path.components().all(|c| matches!(c, Component::Normal(_)))
}



impl Library {
    /// A speech archive's export (`livingworld`): the index file and the decoded takes' folder
    /// (None when the takes were not decoded). None without an index.
    pub(crate) fn speech(&self, archive: &str) -> Option<(PathBuf, Option<PathBuf>)> {
        let e = self.manifest.speech.get(archive)?;
        if !safe_relative(&e.index) {
            return None;
        }
        let audio = e.audio.as_deref().filter(|a| safe_relative(a)).map(|a| self.root.join(a));
        Some((self.root.join(&e.index), audio))
    }

    /// A speech archive's mod takes: replaced (clip without `.dat`, take) → file, and extra takes
    /// by clip after the clip's own (empty without overlays).
    pub(crate) fn speech_mods(&self, archive: &str) -> (HashMap<(String, u32), PathBuf>, HashMap<String, Vec<PathBuf>>) {
        let Some(s) = self.manifest.mod_speech.get(archive) else { return Default::default() };
        let takes = s.takes.iter().flat_map(|(clip, takes)| {
            takes.iter().filter_map(move |(take, file)| Some(((clip.clone(), take.parse().ok()?), self.path(file))))
        }).collect();
        let extra = s.extra.iter().map(|(clip, files)| (clip.clone(), files.iter().map(|f| self.path(f)).collect())).collect();
        (takes, extra)
    }

    /// Where a manifest file is: under the install's audio folder, or under a mod's root for a
    /// `mod:<id>/<path>` reference.
    pub(crate) fn path(&self, file: &str) -> PathBuf {
        match skate_mods::audio_merge::split_mod_ref(file).and_then(|(id, rel)| Some(self.mods.get(id)?.join(rel))) {
            Some(p) => p,
            None => self.root.join(file),
        }
    }

    /// A manifest file's identity for the hot swap (`swap.rs`): its reference, and for a mod file
    /// also its size and modification time (a mod file edited in place is new content; install
    /// files do not change while the game runs).
    pub(crate) fn stamp(&self, file: &str) -> String {
        if skate_mods::audio_merge::split_mod_ref(file).is_none() {
            return file.to_owned();
        }
        match self.stamps.get(file) {
            Some(s) => s.clone(),
            None => self.file_stamp(file),
        }
    }

    fn file_stamp(&self, file: &str) -> String {
        let meta = std::fs::metadata(self.path(file)).ok();
        let modified = meta.as_ref().and_then(|m| m.modified().ok()).and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_nanos());
        format!("{file}|{}|{modified}", meta.map_or(0, |m| m.len()))
    }

    /// An AEMS bank's content key (its `.abk`, WAVs by slot, rebuilt headers' loop starts, the
    /// overlay's volume group); None when the bank is not in the audio.
    pub(crate) fn bank_key(&self, stem: &str) -> Option<String> {
        let abk = self.manifest.aems.banks.get(stem)?;
        let wavs: Vec<String> = self.manifest.banks.get(stem).map_or(Vec::new(), |e| e.iter().map(|e| self.stamp(&e.file)).collect());
        Some(format!("{}|{:?}|{:?}|{:?}", self.stamp(abk), wavs, self.manifest.mod_sample_loops.get(stem), self.bank_group(stem)))
    }

    /// A Splice bank's content key (its patch tree and its WAVs).
    pub(crate) fn splice_key(&self, stem: &str) -> Option<String> {
        let tree = self.manifest.aems.splice.get(stem)?;
        let wavs: Vec<String> = self.manifest.banks.get(stem).map_or(Vec::new(), |e| e.iter().map(|e| self.stamp(&e.file)).collect());
        Some(format!("{}|{wavs:?}", self.stamp(tree)))
    }

    /// The wheel-spin streams' content key.
    pub(crate) fn wheels_key(&self) -> String {
        format!("{:?}", self.manifest.wheels.iter().map(|(k, e)| (k, self.stamp(&e.file))).collect::<Vec<_>>())
    }

    /// The MixMap file's content key.
    pub(crate) fn mixmap_key(&self) -> Option<String> {
        self.manifest.aems.mixmap.as_deref().map(|f| self.stamp(f))
    }

    /// The rolling bed's content key (its grain recordings and the grain player's tuning: the bed
    /// is built at the runtime's start).
    pub(crate) fn grain_key(&self) -> String {
        let grains: Vec<(String, Option<String>, Option<String>)> = self.manifest.grains.iter().map(|(k, g)| (k.clone(), g.file.as_deref().map(|f| self.stamp(f)), g.grain.as_deref().map(|f| self.stamp(f)))).collect();
        format!("{grains:?}|{:?}", self.manifest.grain_player)
    }

    /// A speech archive's content key (its index and the overlays' takes).
    pub(crate) fn speech_key(&self, archive: &str) -> String {
        let e = self.manifest.speech.get(archive).map(|e| (e.index.clone(), e.audio.clone()));
        let mods = self.manifest.mod_speech.get(archive).map(|s| {
            let takes: Vec<(String, String, String)> = s.takes.iter().flat_map(|(c, t)| t.iter().map(move |(k, f)| (c.clone(), k.clone(), f.clone()))).map(|(c, k, f)| (c, k, self.stamp(&f))).collect();
            let extra: Vec<(String, Vec<String>)> = s.extra.iter().map(|(c, fs)| (c.clone(), fs.iter().map(|f| self.stamp(f)).collect())).collect();
            format!("{takes:?}|{extra:?}")
        });
        format!("{e:?}|{mods:?}")
    }

    /// The Csis project files in install order, split into the install's and the overlays' (`mod:`
    /// files, each with its stamp).
    pub(crate) fn project_files(&self) -> (Vec<String>, Vec<(String, String)>) {
        let (mods, retail): (Vec<&String>, Vec<&String>) = self.manifest.aems.projects.iter().partition(|f| skate_mods::audio_merge::split_mod_ref(f).is_some());
        (retail.into_iter().cloned().collect(), mods.into_iter().map(|f| (f.clone(), self.stamp(f))).collect())
    }

    /// The world layer's content key: zone beds (with their files), zones, crossfades, regions,
    /// `.ems` records, location sets, location programs, crossfade layouts and map audio. When it
    /// changes, a hot swap rebuilds the map-keyed world state (emitters, reverb zones, zone
    /// ambience); when it does not, they keep playing through the swap.
    pub(crate) fn world_key(&self) -> String {
        let m = &self.manifest;
        let beds: Vec<(&String, String)> = m.ambience.iter().map(|(k, e)| (k, self.stamp(&e.file))).collect();
        format!("{beds:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}", m.zones, m.crossfades, m.regions, m.emitters, m.random_sets, m.mod_location_programs, m.mod_crossfade_layouts, m.mod_maps)
    }

    /// The tuning sections as loaded (JSON; `Null` when the install's manifest cannot be read).
    pub(crate) fn tuning_sections(&self) -> serde_json::Map<String, serde_json::Value> {
        self.tuning_base().cloned().unwrap_or_default()
    }

    /// Banks overlays load at audio start and keep across map changes, with their group
    /// (`true` = the player group).
    pub(crate) fn resident_banks(&self) -> Vec<(String, bool)> {
        self.manifest.mod_banks.iter().filter(|(_, b)| b.preload).map(|(s, b)| (s.clone(), b.group.as_deref() == Some("player"))).collect()
    }

    /// An overlay's volume group for a bank (`Some(true)` = player, `Some(false)` = world).
    pub(crate) fn bank_group(&self, stem: &str) -> Option<bool> {
        self.manifest.mod_banks.get(stem).and_then(|b| b.group.as_deref()).map(|g| g == "player")
    }

    /// Whether a bank comes from an audio content overlay (all its files are mod files).
    pub(crate) fn is_mod_bank(&self, stem: &str) -> bool {
        let is_mod = |f: &str| skate_mods::audio_merge::split_mod_ref(f).is_some();
        let wavs = self.manifest.banks.get(stem);
        let abk = self.manifest.aems.banks.get(stem);
        (wavs.is_some() || abk.is_some()) && wavs.is_none_or(|w| w.iter().all(|e| is_mod(&e.file))) && abk.is_none_or(|a| is_mod(a))
    }

    /// A mod bank's interim location-set layers.
    pub(crate) fn location_program(&self, bank: &str) -> Option<&[LayerJson]> {
        self.manifest.mod_location_programs.get(bank).map(Vec::as_slice)
    }

    /// An audio content overlay's declared crossfade layout for a bank (group → voices).
    pub(crate) fn crossfade_layout(&self, bank: &str) -> Option<&BTreeMap<String, Vec<CrossfadeVoiceJson>>> {
        self.manifest.mod_crossfade_layouts.get(bank)
    }

    /// An overlay's map audio for a map stem.
    pub(crate) fn mod_map(&self, stem: &str) -> Option<&MapJson> {
        self.manifest.mod_maps.get(stem)
    }

    /// The location sets' names (the audio catalog).
    pub(crate) fn random_set_names(&self) -> Vec<String> {
        self.manifest.random_sets.values().filter_map(|s| s.name.clone()).collect()
    }

    /// The zones' names (the audio catalog).
    pub(crate) fn zone_names(&self) -> Vec<String> {
        self.manifest.zones.values().filter_map(|z| z.name.clone()).collect()
    }

    /// A zone's key by its name.
    pub(crate) fn zone_named(&self, name: &str) -> Option<u64> {
        self.manifest.zones.iter().find(|(_, z)| z.name.as_deref() == Some(name)).and_then(|(k, _)| u64::from_str_radix(k, 16).ok())
    }

    /// The world sources' tuning (`world_sources`; defaults on installs without it).
    pub(crate) fn world_tuning(&self) -> &crate::world_sources::WorldTuningJson {
        &self.manifest.world_tuning
    }

    /// The tuning sections as loaded (see `tuning_raw`): the base the runtime tuning writes patch.
    pub(crate) fn tuning_base(&self) -> Result<&serde_json::Map<String, serde_json::Value>, String> {
        self.tuning_raw
            .get_or_init(|| {
                let path = self.root.join("audio_manifest.json");
                let bytes = skate_data::vfs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
                let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(|e| format!("{}: {e}", path.display()))?;
                Ok(tuning_sections(&value))
            })
            .as_ref()
            .map_err(Clone::clone)
    }

    /// Whether a tuning section's JSON reads back as the typed tuning the game uses.
    pub(crate) fn check_tuning(section: &str, value: &serde_json::Value) -> Result<(), String> {
        let read = match section {
            "player_tuning" => serde_json::from_value::<PlayerTuningJson>(value.clone()).map(|_| ()),
            "world_tuning" => serde_json::from_value::<crate::world_sources::WorldTuningJson>(value.clone()).map(|_| ()),
            "bus_tuning" => serde_json::from_value::<BusTuningJson>(value.clone()).map(|_| ()),
            _ => return Err(format!("no tuning section {section}")),
        };
        read.map_err(|e| format!("{section} does not read back: {e}"))
    }

    /// Replace a tuning section with `value` (a runtime tuning write); the loaded section is kept
    /// and comes back with [`Library::restore_tuning`].
    pub(crate) fn set_tuning(&mut self, section: &str, value: &serde_json::Value) -> Result<(), String> {
        let err = |e: serde_json::Error| format!("{section} does not read back: {e}");
        match section {
            "player_tuning" => {
                let new = serde_json::from_value::<PlayerTuningJson>(value.clone()).map_err(err)?;
                let old = std::mem::replace(&mut self.manifest.player_tuning, new);
                self.tuning_saved.player.get_or_insert(old);
            }
            "world_tuning" => {
                let new = serde_json::from_value::<crate::world_sources::WorldTuningJson>(value.clone()).map_err(err)?;
                let old = std::mem::replace(&mut self.manifest.world_tuning, new);
                self.tuning_saved.world.get_or_insert(old);
            }
            "bus_tuning" => {
                let new = serde_json::from_value::<BusTuningJson>(value.clone()).map_err(err)?;
                let old = std::mem::replace(&mut self.manifest.bus_tuning, new);
                self.tuning_saved.bus.get_or_insert(old);
            }
            _ => return Err(format!("no tuning section {section}")),
        }
        Ok(())
    }

    /// Put the loaded tuning section back (the very value loaded, not a re-read).
    pub(crate) fn restore_tuning(&mut self, section: &str) {
        match section {
            "player_tuning" => {
                if let Some(old) = self.tuning_saved.player.take() {
                    self.manifest.player_tuning = old;
                }
            }
            "world_tuning" => {
                if let Some(old) = self.tuning_saved.world.take() {
                    self.manifest.world_tuning = old;
                }
            }
            "bus_tuning" => {
                if let Some(old) = self.tuning_saved.bus.take() {
                    self.manifest.bus_tuning = old;
                }
            }
            _ => {}
        }
    }

    /// The player components' vault tuning (empty tables on installs set up before it existed).
    pub(crate) fn player_tuning(&self) -> skate_audio::player::tuning::PlayerTuning {
        self.manifest.player_tuning.tuning()
    }

    /// The environment network's reverb presets by key and the eight eEQChain bus records (empty
    /// on installs set up before the bus export: no wet path, buses pass dry).
    pub(crate) fn bus_tuning(&self) -> (std::collections::HashMap<u64, skate_audio::bus::env::Preset>, Vec<skate_audio::bus::eqchain::EqRecord>) {
        let b = &self.manifest.bus_tuning;
        let presets = b.reverb.iter().filter_map(|(k, v)| {
            let key = u64::from_str_radix(k, 16).ok()?;
            (v.len() == 44).then(|| (key, skate_audio::bus::env::Preset(std::array::from_fn(|i| v[i]))))
        }).collect();
        let eq = b.eq_buses.iter().filter(|e| e.ranges.len() == 6).map(|e| skate_audio::bus::eqchain::EqRecord {
            enabled: e.enabled,
            clip: e.clip,
            ranges: std::array::from_fn(|i| e.ranges[i]),
        }).collect();
        (presets, eq)
    }

    /// The FlangeSub effect returns' presets (A, B), when the install has them.
    pub(crate) fn flange_presets(&self) -> Option<[skate_audio::bus::flange::FlangePreset; 2]> {
        let f = &self.manifest.bus_tuning.flange;
        let preset = |v: &Vec<f32>| (v.len() == 9).then(|| skate_audio::bus::flange::FlangePreset(std::array::from_fn(|i| v[i])));
        Some([preset(f.first()?)?, preset(f.get(1)?)?])
    }

    /// The footstep layer of materials 0..142 (empty on installs set up before that export: no
    /// material footstep layer, the step / walking layers still play).
    pub(crate) fn footstep_materials(&self) -> Vec<skate_audio::player::footsteps::FootstepMaterial> {
        let rows = &self.manifest.player_tuning.collision.materials;
        if rows.iter().all(|m| m.step_gain.is_none()) {
            return Vec::new();
        }
        rows.iter()
            .map(|m| {
                let ids: [i32; 7] = std::array::from_fn(|i| m.ids.get(i).copied().unwrap_or(0));
                skate_audio::player::footsteps::FootstepMaterial::from_collision(m.kind, &ids, m.footsteps, m.step_gain.unwrap_or(32767), m.step_landing_gain.unwrap_or(32767))
            })
            .collect()
    }

    /// The Contacts posters' vault values (collision posters, foot taps, scuffs).
    pub(crate) fn contacts_tuning(&self) -> skate_audio::player::contacts::ContactsTuning {
        self.manifest.player_tuning.collision.contacts()
    }

    /// A wheel-spin recording (`SFXObj_Wheels`) as planar PCM, decoded now.
    pub(crate) fn wheels_pcm(&self, name: &str) -> Option<Arc<skate_audio::mixer::Pcm>> {
        let entry = self.manifest.wheels.get(name)?;
        self.read(&entry.file).ok().and_then(|bytes| wav_pcm(&bytes)).map(Arc::new)
    }

    /// The install's audio, no overlays.
    /// skate_vita: the reference's `load_with` without content overlays (no mods here): the
    /// manifest read as it is, every file a plain relative path. Files are read through
    /// `skate_data::vfs` (sound.pak mounted, else the disk).
    pub fn load(asset_root: &Path) -> Result<Self, String> {
        let root = asset_root.join("private/audio");
        let path = root.join("audio_manifest.json");
        let bytes = skate_data::vfs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mods = BTreeMap::new();
        let raw = std::sync::OnceLock::new();
        let manifest: Manifest = serde_json::from_slice(&bytes).map_err(|e| format!("{}: {e}", path.display()))?;
        if !MANIFEST_VERSIONS.contains(&manifest.version) {
            return Err(format!("{}: unsupported version {}", path.display(), manifest.version));
        }
        let files = manifest.ambience.values().chain(manifest.wheels.values())
            .chain(manifest.banks.values().flatten());
        let aems = manifest.aems.projects.iter().chain(manifest.aems.banks.values()).chain(manifest.aems.mixmap.iter())
            .chain(manifest.aems.splice.values())
            .chain(manifest.grains.values().flat_map(|g| g.file.iter().chain(g.grain.iter())));
        for f in files.map(|e| &e.file).chain(aems) {
            if !safe_relative(f) {
                return Err(format!("{}: invalid file path {f:?}", path.display()));
            }
        }
        info!(
            "Game audio: {} ambience beds, {} rolling grains, {} sample banks",
            manifest.ambience.len(), manifest.grains.len(), manifest.banks.len()
        );
        Ok(Self { root, manifest, mods, failed: Default::default(), tuning_raw: raw, tuning_saved: TuningSaved::default(), stamps: HashMap::new() })
    }




    /// Number of samples in a bank (0 when the bank was not exported).
    pub(crate) fn bank_len(&self, bank: &str) -> usize {
        self.manifest.banks.get(bank).map_or(0, Vec::len)
    }

    /// The records of an `.ems` emitter file (empty when absent).
    pub(crate) fn emitters(&self, file: &str) -> &[EmitterRecord] {
        self.manifest.emitters.get(file).map_or(&[], Vec::as_slice)
    }

    /// The key a district's region layer holds at (x, z), if any tile covers it.
    pub(crate) fn region_key(&self, district: &str, layer: &str, x: f32, z: f32) -> Option<u64> {
        self.manifest.regions.get(district)?.get(layer)?.iter().find_map(|tile| tile.key(x, z))
    }

    /// Whether this install has the retail zone ambience data (manifest v4 with zones).
    pub(crate) fn has_zones(&self) -> bool {
        !self.manifest.zones.is_empty()
    }
    pub(crate) fn zone(&self, key: u64) -> Option<&Zone> {
        self.manifest.zones.get(&format!("{key:016X}"))
    }
    /// The crossfade joining two zones, in either order.
    pub(crate) fn crossfade(&self, a: u64, b: u64) -> Option<&Crossfade> {
        let (a, b) = (format!("{a:016X}"), format!("{b:016X}"));
        self.manifest.crossfades.iter().find(|c| (c.from == a && c.to == b) || (c.from == b && c.to == a))
    }

    /// A location set by its u64 key, or by name.
    pub(crate) fn random_set(&self, key: u64) -> Option<&RandomSet> {
        self.manifest.random_sets.get(&format!("{key:016X}"))
    }
    pub(crate) fn random_set_named(&self, name: &str) -> Option<(u64, &RandomSet)> {
        self.manifest.random_sets.iter().find(|(_, s)| s.name.as_deref() == Some(name))
            .and_then(|(k, s)| Some((u64::from_str_radix(k, 16).ok()?, s)))
    }

    /// The native AEMS runtime's files (empty before manifest v5).
    pub(crate) fn aems(&self) -> &AemsFiles {
        &self.manifest.aems
    }

    /// A grain member's whole recording and raw `.grain` file (native grain player), if staged.
    pub(crate) fn grain_whole(&self, name: &str) -> Option<(&str, &str)> {
        let g = self.manifest.grains.get(name)?;
        Some((g.file.as_deref()?, g.grain.as_deref()?))
    }

    /// The vault tuning of a grain member (its collection over `default`).
    pub(crate) fn grain_tuning(&self, name: &str) -> Option<skate_audio::grain::board::SurfaceTuning> {
        let t = &self.manifest.grain_player;
        t.surfaces.get(name)?.tuning(&t.default)
    }

    /// The grain class's `default` collection as a tuning (what a bind with the `default` key uses:
    /// rolling surface 0, `player::rolling::member`).
    pub(crate) fn grain_default_tuning(&self) -> Option<skate_audio::grain::board::SurfaceTuning> {
        let t = &self.manifest.grain_player;
        t.default.tuning(&t.default)
    }

    /// The rocket layer's tuning (owner class `default`).
    pub(crate) fn rocket_tuning(&self) -> Option<skate_audio::grain::board::RocketTuning> {
        let o = &self.manifest.grain_player.owner;
        Some(skate_audio::grain::board::RocketTuning {
            start_kmh: o.rocket_start_kmh?,
            top_kmh: o.rocket_top_kmh?,
            gain_word: o.rocket_gain_word?,
            params: skate_audio::grain::GrainParams::from_slice(o.rocket_params.as_deref()?)?,
        })
    }

    /// Collision material → rolling surface (`Sk8::AudioSurfaceMap`, 95 entries; empty before
    /// 2026-10-02 installs).
    #[cfg(test)]
    pub(crate) fn surface_map(&self) -> &[u32] {
        &self.manifest.grain_player.surface_map
    }

    /// The board chains' tuning (owner class `default`): the install's values over the vault
    /// defaults `ChainTuning::default()` holds (installs set up before the chain export).
    pub(crate) fn chain_tuning(&self) -> skate_audio::grain::chain::ChainTuning {
        let o = &self.manifest.grain_player.owner;
        let mut t = skate_audio::grain::chain::ChainTuning::default();
        let set = |v: &mut f32, x: Option<f32>| if let Some(x) = x { *v = x };
        let seti = |v: &mut i32, x: Option<i32>| if let Some(x) = x { *v = x };
        set(&mut t.send_start_kmh, o.g3_send_start_kmh);
        set(&mut t.send_end_kmh, o.g3_send_end_kmh);
        set(&mut t.send_max, o.g3_send_max);
        set(&mut t.level_start_kmh, o.g1_level_start_kmh);
        set(&mut t.level_end_kmh, o.g1_level_end_kmh);
        set(&mut t.level_floor, o.g1_level_floor);
        set(&mut t.wobble_start_kmh, o.wobble_start_kmh);
        set(&mut t.wobble_end_kmh, o.wobble_end_kmh);
        seti(&mut t.wobble[0].ms_low, o.wobble_a_ms_low);
        seti(&mut t.wobble[0].ms_high, o.wobble_a_ms_high);
        set(&mut t.wobble[0].low, o.wobble_a_low);
        set(&mut t.wobble[0].high, o.wobble_a_high);
        seti(&mut t.wobble[1].ms_low, o.wobble_b_ms_low);
        seti(&mut t.wobble[1].ms_high, o.wobble_b_ms_high);
        set(&mut t.wobble[1].low, o.wobble_b_low);
        set(&mut t.wobble[1].high, o.wobble_b_high);
        set(&mut t.clip, o.g3_clip);
        set(&mut t.shelf_hz, o.g3_shelf_hz);
        set(&mut t.shelf_gain, o.g3_shelf_gain);
        t
    }


    /// Read a manifest-listed file (paths were validated at load).
    pub(crate) fn read(&self, file: &str) -> std::io::Result<Vec<u8>> {
        skate_data::vfs::read(&self.path(file))
    }

    /// skate_vita: a zone bed (`private/audio/ambience/<bed>.ima` of ambience.pak): streamed from its pack file
    /// on the card when it is mounted from there, else read like any sample.
    pub(crate) fn bed_pcm(&self, bed: &str) -> Option<Arc<skate_audio::mixer::Pcm>> {
        let path = self.path(&format!("ambience/{bed}.ima"));
        if let Some((pack, offset, len)) = skate_data::vfs::disk_location(&path) {
            return skate_audio::stream::pcm_from_ima_file(&pack, offset, len).ok().map(Arc::new);
        }
        shared_pcm(&path).map(Arc::new)
    }

    /// skate_vita: a manifest-listed sample file as PCM without copying an `.ima` ([`shared_pcm`]).
    pub(crate) fn read_pcm(&self, file: &str) -> Option<skate_audio::mixer::Pcm> {
        shared_pcm(&self.path(file))
    }

    /// A Splice bank for the native player: its patch tree and its samples (stream n = WAV n), or
    /// None when the install lacks the tree or the WAVs don't match its sample count.
    /// The front-end sounds (`fe` records by key) and their Splice bank; None on installs set up
    /// before the export (the session marker's sounds are then silent).
    pub(crate) fn frontend_sounds(&self) -> Option<skate_audio::frontend::FeTable> {
        let f = &self.manifest.frontend;
        if f.bank.is_empty() || f.sounds.is_empty() {
            return None;
        }
        let sounds = f
            .sounds
            .iter()
            .filter_map(|(k, s)| {
                let key = u64::from_str_radix(k, 16).ok()?;
                Some((key, skate_audio::frontend::FeSound { name: s.name.clone(), id: s.id, level: s.level, hom: s.hom, moment: s.moment, alt_bus: s.alt_bus }))
            })
            .collect();
        Some(skate_audio::frontend::FeTable { bank: f.bank.clone(), sounds })
    }

    pub(crate) fn splice_bank(&self, stem: &str) -> Option<(skate_audio::splice::SpliceBank, Vec<Option<Arc<skate_audio::mixer::Pcm>>>)> {
        let file = self.manifest.aems.splice.get(stem)?;
        let bank = skate_audio::splice::SpliceBank::parse(&self.read(file).ok()?).ok()?;
        let pcm = self.bank_pcm(stem);
        (pcm.len() == bank.samples).then_some((bank, pcm))
    }

    /// A bank's decoded samples as planar f32 PCM by S10A slot (the WAV order); None where a file
    /// is missing or unreadable (the native runtime then plays silence of the right length).
    pub(crate) fn bank_pcm(&self, bank: &str) -> BankPcm {
        let Some(entries) = self.manifest.banks.get(bank) else { return Vec::new() };
        let paths: Vec<PathBuf> = entries.iter().map(|e| self.path(&e.file)).collect();
        decode_wavs(paths.iter().map(PathBuf::as_path))
    }

    /// What reading and decoding an AEMS bank needs, detached from the library so a background
    /// thread can do it (`native::prefetch`). Err = the bank is not in the install.
    pub(crate) fn bank_source(&self, stem: &str) -> Result<BankSource, String> {
        let file = self.manifest.aems.banks.get(stem).ok_or_else(|| format!("bank {stem} is not in the install"))?;
        let entries = self.manifest.banks.get(stem).map_or(&[][..], Vec::as_slice);
        let loops = self.manifest.mod_sample_loops.get(stem);
        Ok(BankSource {
            stem: stem.to_owned(),
            file: file.clone(),
            abk: self.path(file),
            wavs: entries.iter().map(|e| self.path(&e.file)).collect(),
            rebuild: entries.iter().enumerate().filter(|(_, e)| skate_mods::audio_merge::split_mod_ref(&e.file).is_some())
                .map(|(i, _)| (i, loops.and_then(|l| l.get(&i.to_string()).copied()))).collect(),
        })
    }

}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// The bridge's speed graph: the exported posters override the default, and the default is
    /// the stock vault's record word for word (data-gated: the converted `skater-collections.json`
    /// of the user's own disc, class `6EBA5BCD3E38A98A` `default` field `8B164823E008749C`, a
    /// `Sk8::PointNegGraphData8`: 16-byte header, x at +16, y at +48).
    #[test]
    #[ignore = "needs the private install data"]
    fn the_body_speed_graph_is_the_stock_vault_record() {
        use skate_audio::player::contacts::SpeedGraph8;
        let json: CollisionJson = serde_json::from_str(r#"{"posters": {"body_speed_x": [0, 1, 2, 3, 4, 5, 6, 7], "body_speed_y": [1, 1, 1, 1, 2, 2, 2, 2]}}"#).unwrap();
        let g = json.contacts().body_speed_curve;
        assert_eq!((g.x[7], g.y[4]), (7.0, 2.0), "exported values win");
        assert_eq!(CollisionJson::default().contacts().body_speed_curve, SpeedGraph8::BODY_SPEED, "else the vault's words");
        let path = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/private/stock/skater-collections.json"));
        let Ok(text) = skate_data::vfs::read_to_string(path.as_ref()) else {
            panic!("missing private data: no converted skater collections");
        };
        let all: serde_json::Value = serde_json::from_str(&text).unwrap();
        let rec = all["collections"].as_array().unwrap().iter()
            .find(|c| c["class"] == "Hash_6EBA5BCD3E38A98A" && c["key"] == "default").expect("the Contacts body record");
        let field = &rec["fields"]["Hash_8B164823E008749C"];
        assert_eq!(field["type"], "Sk8::PointNegGraphData8");
        let hex: String = field["data"].as_str().unwrap().split_whitespace().collect();
        let word = |i: usize| f32::from_bits(u32::from_str_radix(&hex[8 * i..8 * i + 8], 16).unwrap());
        let vault = SpeedGraph8 { x: std::array::from_fn(|i| word(4 + i)), y: std::array::from_fn(|i| word(12 + i)) };
        assert_eq!(vault, SpeedGraph8::BODY_SPEED);
    }

    pub(crate) fn test_wav(frames: usize, rate: u32, value: i16) -> Vec<u8> {
        let data = frames * 2;
        let mut b = b"RIFF".to_vec();
        b.extend((36 + data as u32).to_le_bytes());
        b.extend(b"WAVEfmt ");
        b.extend(16u32.to_le_bytes());
        b.extend([1, 0, 1, 0]);
        b.extend(rate.to_le_bytes());
        b.extend((rate * 2).to_le_bytes());
        b.extend([2, 0, 16, 0]);
        b.extend(b"data");
        b.extend((data as u32).to_le_bytes());
        for _ in 0..frames {
            b.extend(value.to_le_bytes());
        }
        b
    }

    /// A temp install with a sample bank `x` (2 WAVs), an AEMS bank `T` (two S10A slots, the
    /// second looping) with its 2 WAVs, an ambience bed and a speech index; and a mod folder.
    pub(crate) fn content_fixture(name: &str) -> (PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!("skate-audio-content-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let audio = dir.join("private/audio");
        for d in ["banks/x", "banks/T", "aems", "ambience"] {
            std::fs::create_dir_all(audio.join(d)).unwrap();
        }
        std::fs::write(audio.join("banks/x/0.wav"), test_wav(100, 48000, 100)).unwrap();
        std::fs::write(audio.join("banks/x/1.wav"), test_wav(100, 48000, 200)).unwrap();
        std::fs::write(audio.join("banks/T/0.wav"), test_wav(300, 48000, 300)).unwrap();
        std::fs::write(audio.join("banks/T/1.wav"), test_wav(400, 48000, 400)).unwrap();
        std::fs::write(audio.join("ambience/bed.wav"), test_wav(1000, 48000, 500)).unwrap();
        use skate_audio::eval::synthetic;
        let bank = synthetic::bank(&[synthetic::player_module(4)], &[], &[(300, false), (400, true)]);
        std::fs::write(audio.join("aems/T.abk"), &bank.data).unwrap();
        std::fs::write(audio.join("audio_manifest.json"), serde_json::json!({
            "version": 5,
            "ambience": {"bed": {"file": "ambience/bed.wav"}},
            "grains": {}, "wheels": {},
            "banks": {"x": [{"file": "banks/x/0.wav"}, {"file": "banks/x/1.wav"}], "T": [{"file": "banks/T/0.wav"}, {"file": "banks/T/1.wav"}]},
            "zones": {"00000000000000AB": {"name": "plaza", "bed": "bed"}},
            "aems": {"projects": [], "banks": {"T": "aems/T.abk"}},
            "player_tuning": {"wheel_bucket_high": 3.0, "surface_table": [[1, 2]]}
        }).to_string()).unwrap();
        let mods = dir.join("mods/dev.a");
        std::fs::create_dir_all(mods.join("audio")).unwrap();
        (dir, mods)
    }










    #[test]
    fn wav_peak_reads_the_data_chunk() {
        let mut wav = b"RIFF\x00\x00\x00\x00WAVEfmt \x10\x00\x00\x00".to_vec();
        wav.extend([1, 0, 1, 0, 0x80, 0xbb, 0, 0, 0, 0x77, 1, 0, 2, 0, 16, 0]);
        wav.extend(b"LIST\x03\x00\x00\x00abc\x00");
        wav.extend(b"data\x06\x00\x00\x00");
        for s in [100i16, -8192, 50] {
            wav.extend(s.to_le_bytes());
        }
        assert_eq!(wav_peak(&wav), 0.25);
        assert_eq!(wav_peak(b"RIFF"), 1.0);
        assert_eq!(wav_channels(&wav), 1);
        wav[22] = 2;
        assert_eq!(wav_channels(&wav), 2);
        assert_eq!(wav_channels(b"RIFF"), 1);
    }

    #[test]
    fn wav_pcm_decodes_planar_channels() {
        let mut wav = b"RIFF\x00\x00\x00\x00WAVEfmt \x10\x00\x00\x00".to_vec();
        wav.extend([1, 0, 2, 0, 0x80, 0xbb, 0, 0, 0, 0xee, 2, 0, 4, 0, 16, 0]);
        wav.extend(b"data\x08\x00\x00\x00");
        for s in [16384i16, -32768, 0, 8192] {
            wav.extend(s.to_le_bytes());
        }
        let pcm = wav_pcm(&wav).unwrap();
        assert_eq!(pcm.rate, 48000);
        assert_eq!(pcm.channels, vec![vec![0.5, 0.0].into(), vec![-1.0, 0.25].into()]);
        assert!(wav_pcm(b"RIFF").is_none());
    }

    #[test]
    fn manifest_paths_must_stay_inside_the_audio_folder() {
        assert!(safe_relative("banks/GRINDS/0001.wav"));
        for bad in ["", "../x.wav", "/x.wav", "C:/x.wav", "banks/../../x.wav"] {
            assert!(!safe_relative(bad), "{bad}");
        }
    }

    }
