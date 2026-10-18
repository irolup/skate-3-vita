//! The front-end sounds (`crate::ui_audio::FrontendSound` → retail's front-end audio object,
//! `skate_audio::frontend`): the `fe` records from the install, played as `sk8_menu` Splice
//! one-shots in the audio pass (after the MixMap tick, once per pass, dt = the pass's host ticks).
//! Requests that arrive on a frame without a pass wait for the next one, as retail's queued
//! message waits for the next audio frame.
//!
//! Copy of the reference's `skate-game/src/game_audio/frontend.rs` (`2e16697`) without Bevy: the
//! Bevy system `frame` is `Native::frontend_frame`, the tests are not copied (they need the
//! install). See PROVENANCE.md.
use skate_audio::frontend::{FeTable, Frontend};
use tracing::{debug, info, warn};

use crate::library::Library;
use crate::native::Native;

/// The front-end object and the records it plays.
pub(crate) struct FrontendHost {
    pub(crate) core: Frontend,
    pub(crate) table: FeTable,
    /// Splice sounds started (diagnostics, tests).
    pub(crate) started: Vec<(u64, u32)>,
}

impl FrontendHost {
    /// The install's `fe` records and their Splice bank, loaded into the runtime's Splice player;
    /// None (logged) on installs without them.
    pub(super) fn load(native: &Native, library: &Library) -> Option<Self> {
        let Some(table) = library.frontend_sounds() else {
            warn!("Game audio: this install has no front-end sounds: the session marker is silent (run setup to refresh the audio)");
            return None;
        };
        let Some((bank, pcm)) = library.splice_bank(&table.bank) else {
            warn!("Game audio: {} has no patch tree in this install: the session marker is silent (run setup to refresh the audio)", table.bank);
            return None;
        };
        let mut runtime = native.shared.lock().ok()?;
        let rt = &mut *runtime;
        rt.splice.load_bank(&table.bank, bank, pcm, &mut rt.mixer);
        Some(Self { core: Frontend::default(), table, started: Vec::new() })
    }

    /// Queue one record (an unknown key or a silent record plays nothing).
    pub(crate) fn request(&mut self, key: u64) -> bool {
        match self.table.sounds.get(&key) {
            Some(sound) => self.core.request(key, sound, false),
            None => {
                debug!("Game audio: no front-end sound {key:016X}");
                false
            }
        }
    }

    /// One audio frame of `dt` seconds.
    pub(crate) fn frame(&mut self, dt: f32, runtime: &mut skate_audio::runtime::Runtime) {
        if !self.core.busy() {
            return;
        }
        let report = self.core.frame(dt, &self.table.bank, &mut runtime.splice_host());
        for &(key, id) in &report.started {
            if std::env::var_os("SKATE_AUDIO_TRACE").is_some() {
                info!("AUDIO_NATIVE frontend {key:016X} sk8_menu {id}");
            }
        }
        self.started.extend(report.started);
    }
}

/// The front-end volume from the MixMap word (int → single × retail's 1/32767 constant `0x38000100`).
pub(crate) fn volume(word: i32) -> f32 {
    ((word as f64) as f32) * f32::from_bits(0x3800_0100)
}
