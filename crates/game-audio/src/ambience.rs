//! The zone beds: the reference's `game_audio/ambience.rs` (`2e16697`) without Bevy. The native TU3
//! zone-bed phase machine ([`skate_audio::world::ambience::Controller`]) and the MixMap's AMBIENCE
//! controls are as upstream's; where upstream plays the beds as Bevy voices (its "interim playback"),
//! the beds here are the runtime's direct voices over `.ima` files streamed from the card
//! ([`skate_audio::stream`]), at the level the MixMap's output 0 and the zone's volume give, with
//! the MixMap's pitch output 2. The Bevy layer's `RETAIL_SCALE` (×2) is taken out again while the
//! native player runs, so the voice level is the measured retail level.
//!
//! Not ported yet: the crossfade bank's transition sounds (`start_crossfade` / `stop_crossfade`
//! only drive the phase machine; the map's bed fades are the zone's `time_a` / `time_b`), the
//! per-map fallback bed, the mod events. See PROVENANCE.md.
use std::collections::HashMap;
use std::sync::Arc;

use skate_audio::formats::SampleHeader;
use skate_audio::mixmap::keys;
use skate_audio::world::ambience::Controller;
use tracing::{info, warn};

use crate::library::Library;
use crate::native::{FrameInput, Native};

/// The mixer bank of the beds' samples (one slot per bed met on this map).
pub(crate) const BANK: usize = 1 << 22;
/// The mixer bank of the crossfade bank's samples (slot = the bank's sample).
const XBANK: usize = (1 << 22) + 1;

#[derive(Default)]
pub(crate) struct AmbienceHost {
    /// Map name and generation the state belongs to.
    map: Option<(String, u64)>,
    control: Controller,
    /// The bed voice and the bed it plays; `want` is the bed the controller started (opened when
    /// the mixer has a voice free).
    voice: Option<u32>,
    want: Option<String>,
    slots: HashMap<String, u16>,
    bank_added: bool,
    /// Beds that would not load (logged once).
    failed: Vec<String>,
    /// The zone crossfade's looping voices (voice, pan degrees, voice level) and the zone pair's level.
    crossfade: Vec<(u32, f32, f32)>,
    crossfade_level: f32,
    /// Slots set in the crossfade bank (cleared on a new map).
    xslots: Vec<u16>,
    xbank_added: bool,
}

/// The map's crossfade bank (the three districts' `Main_Ambience_Crossfade_*`).
fn crossfade_bank(map: &str) -> Option<&'static str> {
    match map.to_ascii_lowercase().as_str() {
        "downtown" => Some("Main_Ambience_Crossfade_DT"),
        "industrial" => Some("Main_Ambience_Crossfade_Ind"),
        "university" => Some("Main_Ambience_Crossfade_Uni"),
        _ => None,
    }
}

/// Whether the map has a crossfade bank (the three districts: `Main_Ambience_Crossfade_*`), which
/// is what `skate_audio::world::ambience::Controller::process` calls "ready".
fn crossfade_ready(map: &str) -> bool {
    ["downtown", "industrial", "university"].iter().any(|d| map.eq_ignore_ascii_case(d))
}

impl AmbienceHost {
    fn stop_bed(&mut self, shared: &crate::native::Shared) {
        self.want = None;
        let Some(voice) = self.voice.take() else { return };
        if let Ok(mut rt) = shared.lock() {
            skate_audio::eval::VoiceHost::release(&mut rt.mixer, voice);
        }
    }

    fn stop_crossfade(&mut self, shared: &crate::native::Shared) {
        if self.crossfade.is_empty() {
            return;
        }
        if let Ok(mut rt) = shared.lock() {
            for (voice, ..) in self.crossfade.drain(..) {
                skate_audio::eval::VoiceHost::release(&mut rt.mixer, voice);
            }
        }
        self.crossfade.clear();
    }

    /// The zone crossfade between `from` and `to` (the reference's `start_crossfade`): the group
    /// the pair names plays its four looping directional voices until the phase machine stops it.
    fn start_crossfade(&mut self, library: &Library, shared: &crate::native::Shared, map: &str, from: u64, to: u64) {
        let Some(bank) = crossfade_bank(map) else { return };
        let (group, level) = library.crossfade(from, to).map_or((1, 1.0), |c| (c.group, c.level));
        let Some((.., voices)) = crate::crossfade_groups::GROUPS.iter().find(|(b, g, _)| *b == bank && *g == group) else {
            warn!("Game audio: crossfade bank {bank} has no group {group}; its crossfade is silent");
            return;
        };
        let Ok(mut rt) = shared.lock() else { return };
        if self.xslots.is_empty() {
            if !self.xbank_added {
                rt.mixer.add_bank(XBANK, Vec::new(), Vec::new());
                self.xbank_added = true;
            }
            for (slot, pcm) in library.bank_pcm(bank).into_iter().enumerate() {
                let Some(pcm) = pcm else { continue };
                let header = SampleHeader {
                    codec: 3,
                    channels: pcm.channels.len().clamp(1, 6) as u8,
                    rate: pcm.rate,
                    frames: pcm.channels.first().map_or(0, |c| c.len() as u32),
                    loop_start: Some(0),
                };
                rt.mixer.set_bank_sample(XBANK, slot as u16, header, pcm);
                self.xslots.push(slot as u16);
            }
        }
        self.crossfade_level = level;
        for &(sample, pan, voice_level) in voices.iter() {
            if let Some(voice) = rt.mixer.open_direct(XBANK, sample as u16, 0.0, 1.0, 0.0, Some(pan)) {
                self.crossfade.push((voice, pan, voice_level));
            }
        }
        info!("AUDIO_AMBIENCE crossfade {bank} group {group} level {level:.2}");
    }

    /// Frees the map's beds (a new map): every slot and its streams.
    fn clear(&mut self, shared: &crate::native::Shared) {
        self.stop_crossfade(shared);
        if let Ok(mut rt) = shared.lock() {
            for slot in self.xslots.drain(..) {
                rt.mixer.clear_bank_sample(XBANK, slot);
            }
        }
        self.stop_bed(shared);
        if let Ok(mut rt) = shared.lock() {
            for &slot in self.slots.values() {
                rt.mixer.clear_bank_sample(BANK, slot);
            }
        }
        self.slots.clear();
        self.failed.clear();
        self.control = Controller::default();
    }

    /// The slot of `bed` in the beds' bank (loaded on first use).
    fn slot(&mut self, library: &Library, shared: &crate::native::Shared, bed: &str) -> Option<u16> {
        if let Some(&slot) = self.slots.get(bed) {
            return Some(slot);
        }
        if self.failed.iter().any(|b| b == bed) {
            return None;
        }
        let Some(pcm) = library.bed_pcm(bed) else {
            warn!("Game audio: the ambience bed {bed} is not in this install (ambience.pak); it stays silent");
            self.failed.push(bed.to_owned());
            return None;
        };
        let header = SampleHeader {
            codec: 3,
            channels: pcm.channels.len().clamp(1, 6) as u8,
            rate: pcm.rate,
            frames: pcm.channels.first().map_or(0, |c| c.len() as u32),
            loop_start: Some(0),
        };
        let slot = self.slots.len() as u16;
        let mut rt = shared.lock().ok()?;
        if !self.bank_added {
            rt.mixer.add_bank(BANK, Vec::new(), Vec::new());
            self.bank_added = true;
        }
        rt.mixer.set_bank_sample(BANK, slot, header, pcm);
        drop(rt);
        self.slots.insert(bed.to_owned(), slot);
        Some(slot)
    }
}

impl Native {
    /// The reference's `ambience::update` (before the MixMap tick): the zone under the board, the
    /// phase machine, the bed's start and stop, the MixMap's AMBIENCE input.
    pub(crate) fn ambience_update(&mut self, library: &Library, f: &FrameInput) {
        let Some((map, generation)) = f.map else { return };
        let shared = self.shared.clone();
        let host = &mut self.ambience;
        let identity = (map.to_owned(), generation);
        if host.map.as_ref() != Some(&identity) {
            // A new map starts from silence.
            host.clear(&shared);
            host.map = Some(identity);
        }
        if !library.has_zones() {
            return;
        }
        let at = f.riding.board;
        let desired = library
            .region_key(map, "audio_ambience", at.x, at.z)
            .filter(|key| library.zone(*key).and_then(|z| z.bed.as_ref()).is_some())
            .unwrap_or(0);
        let fades = |key: u64| library.zone(key).map_or((1.0, 1.0), |z| (z.time_b, z.time_a));
        let (fade_in, fade_out) = fades(host.control.current);
        let change = host.control.process(desired, f.game_dt, fade_in, fade_out, crossfade_ready(map));
        if change.stop_crossfade {
            host.stop_crossfade(&shared);
        }
        if change.start_crossfade {
            let from = host.control.current;
            host.start_crossfade(library, &shared, map, from, desired);
        }
        if change.stop_bed {
            host.stop_bed(&shared);
        }
        if change.start_bed {
            if let Some(zone) = library.zone(host.control.current) {
                if let Some(bed) = &zone.bed {
                    info!("AUDIO_AMBIENCE zone {} bed {bed}", zone.name.as_deref().unwrap_or("?"));
                }
                host.want = zone.bed.clone();
            }
        }
        if host.voice.is_none() {
            if let Some(bed) = host.want.clone() {
                if let Some(slot) = host.slot(library, &shared, &bed) {
                    if let Ok(mut rt) = shared.lock() {
                        // Silent until the MixMap's level is read after its tick; None when the
                        // mixer has no voice free (tried again next frame).
                        host.voice = rt.mixer.open_direct(BANK, slot, 0.0, 1.0, 0.0, None);
                    }
                }
            }
        }
        let (fade_in, fade_out) = fades(host.control.current);
        if let Some(m) = self.mixmap.as_mut() {
            m.set_input(keys::AMBIENCE, 0, host.control.input(fade_in, fade_out));
        }
    }

    /// The reference's `ambience::mixmap_output` (after the MixMap tick): the bed's level
    /// (AMBIENCE output 0 × the zone's volume) and pitch (output 2).
    pub(crate) fn ambience_output(&mut self, library: &Library) {
        let host = &self.ambience;
        let Some(m) = self.mixmap.as_ref() else { return };
        let Ok(mut rt) = self.shared.lock() else { return };
        if let (Some(voice), Some(zone)) = (host.voice, library.zone(host.control.current)) {
            let gain = (m.level(keys::AMBIENCE, 0) as f32 * skate_audio::dsp::INV_32767) * zone.volume;
            let pitch = m.pitch_4096(keys::AMBIENCE, 2) as f32 / 4096.0;
            rt.mixer.set_direct(voice, pitch, gain, None);
        }
        // The crossfade voices: the MixMap's output 1 × the zone pair's level, each at its own level.
        let word = ((m.level(keys::AMBIENCE, 1) as f32 * host.crossfade_level) as i32).clamp(0, 32767);
        let level = word as f32 * skate_audio::dsp::INV_32767;
        for &(voice, pan, voice_level) in &host.crossfade {
            rt.mixer.set_direct(voice, 1.0, voice_level * level, Some(pan));
        }
    }

    /// The zone crossfade's voices playing (tests, logs).
    pub fn crossfade_playing(&self) -> usize {
        self.ambience.crossfade.len()
    }

    /// The bed that is playing, if any (tests, logs).
    pub fn bed_playing(&self) -> Option<&str> {
        self.ambience.voice?;
        self.ambience.want.as_deref()
    }
}
