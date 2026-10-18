//! Retail world sound emitters: the map's `.ems` records (`eVolumeType` 1) with their sound
//! attributes, from the manifest. The sound-emitter half of the reference's `game_audio/emitters.rs`
//! (`2e16697`) without Bevy: the record test (sphere or ellipsoid with an inner core), the falloff
//! curve, at most [`MAX_ACTIVE`] playing in the order they were reached, a record the listener
//! leaves stopping at once, the five MixMap emitter states, `c_emitter` posts redelivered every
//! frame. Not ported: the reverb zones (`eVolumeType` 5: their own step), published emitters
//! (mods, engine systems), the mod rules and events. See PROVENANCE.md.
//!
//! On the Vita the banks are `world.pak` on the card and are read by proximity: the worker
//! prefetches the banks of the emitters within [`AHEAD`] metres and drops the unused ones beyond
//! [`EVICT`]; a loaded bank no emitter near the listener plays is unloaded again, so only a ring
//! of banks around the listener is resident.
use glam::Vec3;
use skate_audio::eval::NodeId;
use skate_audio::mixmap::{MixMap, keys};
use tracing::{info, warn};

use crate::library::Library;
use crate::native::{FrameInput, Listener, Native};
use crate::prefetch::{AHEAD, EVICT};

/// CSTATEMGR_Emitter's pool size.
const MAX_ACTIVE: usize = 5;

/// The `.ems` files a retail map loads, in order (the reference's `map_audio::tests::old_ems_files`;
/// upstream reads them from the stock collections).
pub(crate) fn ems_files(map: &str) -> &'static [&'static str] {
    match map.to_ascii_lowercase().as_str() {
        "university" => &["music_university", "sfx_university", "reverb_university", "speakers_university", "crowds_university"],
        "downtown" => &["music_downtown", "sfx_downtown", "reverb_downtown", "speakers_downtown", "crowds_downtown"],
        "industrial" => &["music_industrial", "sfx_industrial", "reverb_industrial", "speakers_industrial", "crowds_industrial"],
        "downtownskatepark" => &["sfx_dt_skatepark"],
        "industrialskatepark" => &["sfx_ind_skatepark"],
        "megapark" => &["sfx_mega_skatepark"],
        "maloofmoneycup" => &["sfx_maloof_money_cup"],
        "startpark" => &["sfx_startpark"],
        "blackboxpark" => &["sfx_blackbox_park"],
        "skateschool" => &["skateschool"],
        _ => &[],
    }
}

/// A record's shape, in world space.
#[derive(Clone, Copy, Debug)]
struct Shape {
    position: Vec3,
    extent: Vec3,
    forward: Vec3,
    core: f32,
}

/// Normalised distance of `listener` in the shape (0 at the core, 1 at the edge), or None outside.
fn reach(shape: &Shape, listener: Vec3) -> Option<f32> {
    let delta = listener - shape.position;
    let e = shape.extent;
    let d = if e.x == e.y && e.y == e.z {
        delta.length() / e.x
    } else {
        let side = Vec3::Y.cross(shape.forward).normalize_or_zero();
        let up = shape.forward.cross(side).normalize_or_zero();
        let (f, u, s) = (delta.dot(shape.forward) / e.x, delta.dot(up) / e.y, delta.dot(side) / e.z);
        (f * f + u * u + s * s).sqrt()
    };
    if !d.is_finite() || d >= 1.0 {
        return None;
    }
    let core = shape.core;
    Some(if core > 0.0 && d < core { 0.0 } else if core >= 1.0 { 0.0 } else { (d - core) / (1.0 - core) })
}

/// Retail falloff curve by `eVolumeFalloffType`.
fn falloff(kind: i32, d: f32) -> f32 {
    match kind {
        0 => (1.0 - d) * (1.0 - d),
        1 => 1.0 - d,
        _ => 1.0,
    }
}

struct Emitter {
    shape: Shape,
    volume: f32,
    falloff: i32,
    bank: String,
    patch: i32,
}

/// A reached emitter: its native post and emitter state once started.
struct Node {
    record: usize,
    started: bool,
    post: Option<NodeId>,
    state: Option<usize>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BankStatus {
    Idle,
    Requested,
    Loaded,
}

#[derive(Default)]
pub(crate) struct EmitterHost {
    /// Map name and generation the state belongs to.
    map: Option<(String, u64)>,
    emitters: Vec<Emitter>,
    /// Reached records in discovery order (retail's node list).
    nodes: Vec<Node>,
    /// The map's native emitter banks, each emitter's index into them and each bank's status.
    banks: Vec<String>,
    emitter_bank: Vec<usize>,
    bank_status: Vec<BankStatus>,
    /// Per bank: the listener's distance to its nearest emitter's bounding sphere (scratch).
    near: Vec<f32>,
    order: Vec<(f32, usize)>,
    frames: u32,
}

/// The listener's azimuth to `source`, clockwise from the view direction on the 65536 scale.
pub(crate) fn azimuth(listener: &Listener, source: Vec3) -> i32 {
    let delta = source - listener.position;
    let forward = listener.forward;
    let right = forward.cross(Vec3::Y).normalize_or_zero();
    let degrees = delta.dot(right).atan2(delta.dot(forward)).to_degrees();
    ((degrees / 360.0 * 65536.0).round() as i32).rem_euclid(65536)
}

/// An emitter state's 3DObjPos-style input block (mixmap-spec §7.3): f32 distances from the skater
/// and from the camera, the azimuths (both in the camera frame), active.
fn write_position(m: &mut MixMap, key: u32, listener: &Listener, skater: Vec3, source: Vec3) {
    let az = azimuth(listener, source);
    m.set_input_f32(key, keys::pos::DIST_SKATER, skater.distance(source));
    m.set_input_f32(key, keys::pos::DIST_CAMERA, listener.position.distance(source));
    m.set_input(key, keys::pos::AZ_SKATER, az);
    m.set_input(key, keys::pos::AZ_CAMERA, az);
    m.set_input(key, keys::pos::FLAGS, 1);
}

impl Native {
    /// Whether the install has the bank (the manifest lists it).
    pub(crate) fn has_bank(&self, library: &Library, stem: &str) -> bool {
        library.aems().banks.contains_key(stem)
    }

    /// Unload every bank but the utility and the player's (map change); forget the prefetched ones.
    pub(crate) fn unload_map_banks(&mut self) {
        self.map_epoch += 1;
        self.prefetch.clear();
        let Ok(mut runtime) = self.shared.lock() else { return };
        let resident = &self.resident;
        self.banks.retain(|stem, id| {
            let keep = stem == "emitter_utility"
                || stem == skate_audio::player::seams::UTILITY_BANK
                || crate::player_audio::BANKS.contains(&stem.as_str())
                || crate::player_audio::OPTIONAL_BANKS.iter().any(|set| set.contains(&stem.as_str()))
                || resident.iter().any(|r| r == stem);
            if !keep {
                runtime.unload_bank(*id);
            }
            keep
        });
    }

    /// Unload one emitter bank (the listener moved away from every emitter that plays it).
    fn unload_bank(&mut self, stem: &str) {
        if let Some(id) = self.banks.remove(stem) {
            if let Ok(mut runtime) = self.shared.lock() {
                runtime.unload_bank(id);
            }
        }
    }

    /// Post to `c_emitter` (payload words: level, dry, send, azimuth, pitch, low-pass, high-pass,
    /// unused, selector).
    fn post_emitter(&self, payload: &[i32; 9]) -> Option<NodeId> {
        let class = self.emitter_class?;
        Some(self.shared.lock().ok()?.post(class, payload))
    }

    /// Take a free emitter state (0..4), if any.
    fn claim_emitter_state(&mut self) -> Option<usize> {
        let g = self.emitter_states.iter().position(|used| !used)?;
        self.emitter_states[g] = true;
        Some(g)
    }

    /// Free an emitter state; its 3-D input goes inactive.
    fn release_emitter_state(&mut self, g: usize) {
        if let Some(used) = self.emitter_states.get_mut(g) {
            *used = false;
        }
        if let Some(m) = &mut self.mixmap {
            m.set_input(keys::emitter_pos(g as u32), keys::pos::FLAGS, 0);
        }
    }

    fn set_emitter_position(&mut self, g: usize, listener: &Listener, skater: Vec3, source: Vec3) {
        let Some(m) = &mut self.mixmap else { return };
        write_position(m, keys::emitter_pos(g as u32), listener, skater, source);
    }

    /// The positional `c_emitter` payload from the MixMap (mixmap-spec §6.3, `sub_824DCF08`): w1 dry
    /// = out4 × level, w2 send = out8 × level, w3 pan = out0, w4 pitch = out5, w5 low-pass = out6,
    /// w8 = the attribute patch. Without a MixMap (or a state) the old defaults apply.
    fn emitter_payload(&self, g: Option<usize>, level: f32, azimuth: i32, patch: i32) -> [i32; 9] {
        let level = level.clamp(0.0, 1.0);
        let patch = patch.clamp(0, 500);
        match (&self.mixmap, g) {
            (Some(m), Some(g)) => {
                let key = keys::emitter(g as u32);
                let scaled = |id: usize| (m.level(key, id) as f32 * level) as i32;
                [32767, scaled(4), scaled(8), m.raw(key, 0), m.pitch_4096(key, 5), m.filter_hz(key, 6), 0, 0, patch]
            }
            _ => [32767, (level * 32767.0).round() as i32, 0, azimuth, 4096, 25000, 0, 0, patch],
        }
    }

    /// The sound emitters of the map, once per rendered frame (the reference's `emitters::update`).
    pub(crate) fn emitters_frame(&mut self, library: &Library, f: &FrameInput) {
        let (Some((map, generation)), Some(listener)) = (f.map, f.listener) else { return };
        let mut state = std::mem::take(&mut self.emitters);
        update(&mut state, self, library, map, generation, &listener, f.riding.board, f.silenced);
        self.emitters = state;
    }

    fn prefetch_ready(&self, stem: &str) -> bool {
        self.prefetch.ready(stem)
    }

    /// Emitters playing (diagnostics).
    pub fn emitters_playing(&self) -> usize {
        self.emitters.nodes.iter().filter(|n| n.started && n.post.is_some()).count()
    }
}

/// Ask the prefetch worker for the banks of the emitters the listener is getting close to (nearest
/// first), drop the unused ones it moved away from, and unload loaded banks nothing near plays.
fn prefetch_near(state: &mut EmitterHost, native: &mut Native, library: &Library, ear: Vec3) {
    state.near.clear();
    state.near.resize(state.banks.len(), f32::INFINITY);
    for (e, &b) in state.emitters.iter().zip(&state.emitter_bank) {
        if let Some(near) = state.near.get_mut(b) {
            *near = near.min(e.shape.position.distance(ear) - e.shape.extent.max_element());
        }
    }
    state.order.clear();
    for (b, &d) in state.near.iter().enumerate() {
        match state.bank_status[b] {
            BankStatus::Idle if d <= AHEAD => state.order.push((d, b)),
            BankStatus::Requested if d > EVICT => {
                native.prefetch.drop_bank(&state.banks[b]);
                state.bank_status[b] = BankStatus::Idle;
            }
            // A loaded bank no started emitter plays and the listener left: unloaded (memory).
            BankStatus::Loaded if d > EVICT && !state.nodes.iter().any(|n| n.started && state.emitter_bank.get(n.record) == Some(&b)) => {
                native.unload_bank(&state.banks[b]);
                state.bank_status[b] = BankStatus::Idle;
            }
            _ => {}
        }
    }
    state.order.sort_by(|a, b| a.0.total_cmp(&b.0));
    for &(_, b) in &state.order {
        let stem = &state.banks[b];
        if native.bank_loaded(stem) {
            state.bank_status[b] = BankStatus::Loaded;
            continue;
        }
        if let Ok(source) = library.bank_source(stem) {
            native.prefetch.request(source);
            state.bank_status[b] = BankStatus::Requested;
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn update(state: &mut EmitterHost, native: &mut Native, library: &Library, map: &str, generation: u64, listener: &Listener, skater: Vec3, silent: bool) {
    let identity = (map.to_owned(), generation);
    if state.map.as_ref() != Some(&identity) {
        for node in state.nodes.drain(..) {
            if let Some(post) = node.post {
                native.release(post);
            }
            if let Some(g) = node.state {
                native.release_emitter_state(g);
            }
        }
        native.unload_map_banks();
        // Retail's emitter system loads every file of the map's entry and dispatches by the
        // attribute's eVolumeType: 1 = looping emitter (here), 5 = reverb zone (not ported yet), 4 =
        // music zone (not ported), 6 / 7 are not dispatched by the emitter system.
        let records: Vec<&crate::library::EmitterRecord> = ems_files(map).iter().flat_map(|file| library.emitters(file)).collect();
        state.emitters = records
            .iter()
            .filter(|r| r.kind == 1 && r.flags == 0)
            .filter_map(|r| {
                let bank = r.bank.clone().filter(|b| native.has_bank(library, b))?;
                let s = r.scalars;
                Some(Emitter {
                    shape: Shape { position: Vec3::from(r.position), extent: Vec3::from(r.extent), forward: Vec3::new(s[1], s[2], s[3]), core: s[0] },
                    volume: r.volume,
                    falloff: r.falloff,
                    bank,
                    patch: r.patch,
                })
            })
            .collect();
        info!("World emitters: {} of {} records on {map} have a played sound", state.emitters.len(), records.len());
        state.banks.clear();
        state.emitter_bank.clear();
        for e in &state.emitters {
            let b = match state.banks.iter().position(|s| *s == e.bank) {
                Some(b) => b,
                None => {
                    state.banks.push(e.bank.clone());
                    state.banks.len() - 1
                }
            };
            state.emitter_bank.push(b);
        }
        state.bank_status = vec![BankStatus::Idle; state.banks.len()];
        state.map = Some(identity);
    }
    let ear = listener.position;
    state.frames = state.frames.wrapping_add(1);
    prefetch_near(state, native, library, ear);

    // Release nodes the listener left (or everything while silenced).
    let reached: Vec<Option<f32>> = state.emitters.iter().map(|e| if silent { None } else { reach(&e.shape, ear) }).collect();
    state.nodes.retain(|node| {
        let keep = reached[node.record].is_some();
        if !keep {
            if node.started {
                info!("AUDIO_EMITTER stop {} #{}", state.emitters[node.record].bank, node.record);
            }
            if let Some(post) = node.post {
                native.release(post);
            }
            if let Some(g) = node.state {
                native.release_emitter_state(g);
            }
        }
        keep
    });
    // New hits join the node list in discovery order.
    for (index, hit) in reached.iter().enumerate() {
        if hit.is_some() && !state.nodes.iter().any(|n| n.record == index) {
            state.nodes.push(Node { record: index, started: false, post: None, state: None });
        }
    }
    // Waiting nodes take free states in list order (retail's 5).
    let mut active = state.nodes.iter().filter(|n| n.started).count();
    for node in state.nodes.iter_mut().filter(|n| !n.started) {
        if active >= MAX_ACTIVE {
            continue;
        }
        let e = &state.emitters[node.record];
        // The bank is read ahead on the worker; one that is not ready yet waits (a card read
        // would stall the host), the node tries again next frame.
        if !native.bank_loaded(&e.bank) && !native.prefetch_ready(&e.bank) {
            if let (Some(&b), Ok(source)) = (state.emitter_bank.get(node.record), library.bank_source(&e.bank)) {
                if state.bank_status[b] == BankStatus::Idle {
                    native.prefetch.request(source);
                    state.bank_status[b] = BankStatus::Requested;
                }
            }
            continue;
        }
        active += 1;
        node.started = true;
        info!("AUDIO_EMITTER start {} #{} at {:.1?} volume {:.2} (native)", e.bank, node.record, e.shape.position.to_array(), e.volume);
        match native.ensure_bank(library, &e.bank) {
            Ok(_) => {
                if let Some(status) = state.emitter_bank.get(node.record).and_then(|&b| state.bank_status.get_mut(b)) {
                    *status = BankStatus::Loaded;
                }
                let level = e.volume * falloff(e.falloff, reached[node.record].unwrap_or(1.0));
                node.state = native.claim_emitter_state();
                if let Some(g) = node.state {
                    native.set_emitter_position(g, listener, skater, e.shape.position);
                }
                let payload = native.emitter_payload(node.state, level, azimuth(listener, e.shape.position), e.patch);
                node.post = native.post_emitter(&payload);
            }
            Err(error) => warn!("AUDIO_EMITTER {}: {error}", e.bank),
        }
    }

    // The bank's program does the rest; only the game-side words change.
    for node in state.nodes.iter_mut().filter(|n| n.started) {
        let emitter = &state.emitters[node.record];
        let (Some(d), Some(post)) = (reached[node.record], node.post) else { continue };
        let level = emitter.volume * falloff(emitter.falloff, d);
        if let Some(g) = node.state {
            native.set_emitter_position(g, listener, skater, emitter.shape.position);
        }
        let payload = native.emitter_payload(node.state, level, azimuth(listener, emitter.shape.position), emitter.patch);
        if let Ok(mut runtime) = native.shared.lock() {
            runtime.redeliver(post, &payload);
        }
    }
}

// ---- Reverb zones (`eVolumeType` 5) -------------------------------------------------------------

/// A reverb-zone record: its shape, its attribute and the attribute's reverb preset key.
struct ZoneRecord {
    shape: Shape,
    id: u64,
    attribute: u64,
    preset: u64,
    /// The preset is one of the 24 the install knows (`sub_824A2438`): a zone that is not stays in
    /// the list, disabled, as retail's query stops at the first one that fails the check.
    enabled: bool,
}

/// The reverb zones the listener is inside (the reference's `emitters::reverb_zones`).
#[derive(Default)]
pub(crate) struct ZoneHost {
    map: Option<(String, u64)>,
    records: Vec<ZoneRecord>,
    /// Reached records in discovery order (retail's active node list).
    active: Vec<usize>,
    pub(crate) out: Vec<skate_audio::bus::zones::Zone>,
}

impl ZoneHost {
    /// One frame: the zones around `ear` (empty without a listener), as `reverb_frame` reads them.
    pub(crate) fn update(&mut self, library: &Library, map: Option<(&str, u64)>, ear: Option<Vec3>) {
        let Some((name, generation)) = map else {
            self.out.clear();
            return;
        };
        let identity = (name.to_owned(), generation);
        if self.map.as_ref() != Some(&identity) {
            let (presets, _) = library.bus_tuning();
            self.records.clear();
            for (file, group) in ems_files(name).iter().enumerate() {
                for r in library.emitters(group).iter().filter(|r| r.kind == 5 && r.flags == 0) {
                    let preset = r.reverb.as_deref().and_then(|k| u64::from_str_radix(k, 16).ok()).unwrap_or(0);
                    let s = r.scalars;
                    self.records.push(ZoneRecord {
                        shape: Shape { position: Vec3::from(r.position), extent: Vec3::from(r.extent), forward: Vec3::new(s[1], s[2], s[3]), core: s[0] },
                        id: ((file as u64) << 32) | u64::from(r.index),
                        attribute: u64::from_str_radix(&r.sound_id, 16).unwrap_or(0),
                        preset,
                        enabled: preset != 0 && presets.contains_key(&preset),
                    });
                }
            }
            self.active.clear();
            if !self.records.is_empty() {
                info!("Reverb zones: {} records on {name}", self.records.len());
            }
            self.map = Some(identity);
        }
        self.out.clear();
        let Some(ear) = ear else { return };
        // One frame of the zone node list: drop the records the listener left, append new hits in
        // record order, list the active ones (in node order) with their distance.
        let reached: Vec<Option<f32>> = self.records.iter().map(|z| reach(&z.shape, ear)).collect();
        self.active.retain(|&i| reached[i].is_some());
        for (i, hit) in reached.iter().enumerate() {
            if hit.is_some() && !self.active.contains(&i) {
                self.active.push(i);
            }
        }
        for &i in &self.active {
            let (z, d) = (&self.records[i], reached[i].unwrap_or(1.0));
            self.out.push(skate_audio::bus::zones::Zone {
                id: z.id,
                attribute: z.attribute,
                preset: z.preset,
                d,
                position: [z.shape.position.x, z.shape.position.z],
                enabled: z.enabled,
            });
        }
    }
}
