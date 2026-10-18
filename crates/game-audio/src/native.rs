//! The reference's `game_audio/native.rs` (`b3c9679`) without Bevy: the host of `skate_audio`'s
//! native runtime for the board sounds. Kept verbatim: the host clock, the start (projects,
//! MixMap, the player components and the rolling bed, the buses, the boot posts), the bank
//! loading. Adapted (see `Native::frame` / `Native::finish`): the Bevy systems `mixmap_frame`,
//! `grain_bed::update`, `mixmap_tick` and `reverb_frame` as methods the game calls once per
//! rendered frame. Not ported: the Bevy output stream (the Vita's audio thread renders through
//! `Runtime::fill_stereo`), the bank prefetch worker, the world emitters, the front end and mod
//! content (later stages).
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use skate_audio::eval::NodeId;
use skate_audio::formats::Project;
use skate_audio::mixmap::{MixMap, keys};
use skate_audio::runtime::Runtime;
use tracing::{error, info, warn};

use crate::library::Library;

/// The host's step: one 60 Hz physics step. The host ticks once per physics step ([`HostClock`]);
/// the MixMap evaluates on every second one (the console's 30 Hz, `skate_audio::mixmap::cadence`).
pub(crate) const MIX_STEP: f32 = 1.0 / 60.0;

/// The most host ticks one frame takes (the steps beyond are dropped; their pulses stay latched).
/// Retail's audio manager runs once per rendered frame and never catches up: a long frame is one
/// call with its long dt (both halves once). Our host counts physics steps to stay on the console
/// grid at any frame rate, so it bounds the catch-up instead: 4 steps = 2 console evaluations,
/// enough for frame-rate independence down to 15 fps (the old Real-time accumulator's cap, 4 ×
/// [`MIX_STEP`]), while a hitch (Bevy runs up to 15 physics steps after a 250 ms frame) never
/// releases a burst of evaluations, Jitter steps or poster calls.
pub(crate) const MAX_STEPS_PER_FRAME: u32 = 4;

/// What a frame's host pass runs ([`HostClock::pass`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Pass {
    /// Host ticks: the physics steps taken (capped at [`MAX_STEPS_PER_FRAME`]).
    pub ticks: usize,
    /// Console evaluations they complete (the console cadence).
    pub calls: usize,
    /// `sub_82491180` (the eEQChain clear) runs in this pass.
    pub clear_eq: bool,
}

/// The audio host's clock: the physics steps `skate_events::observe` published since the last
/// frame (2026-10-03, PR #32 review: before, a `Time<Real>` accumulator of its own drifted from
/// the physics grid after hitches, re-processed a stale sample on frames without a step, dropped
/// the first of two steps' pulses and kept ticking while the menu paused the game).
#[derive(Default)]
pub(crate) struct HostClock {
    /// The console's 30 Hz evaluation grid over the steps.
    cadence: skate_audio::mixmap::cadence::Cadence,
}

impl HostClock {
    /// Take the frame's steps. `None` (no pass: no inputs, process, tick or update) when nothing
    /// stepped, or while the game is silenced (menu, replay): those steps and their pulses are
    /// dropped, as the stream is paused.
    /// skate_vita: `steps` = the physics steps published since the last frame
    /// (`skate_sim::live::Session::take_audio`, the reference's `Cues::take_steps`).
    pub(crate) fn pass(&mut self, steps: u32, silenced: bool) -> Option<Pass> {
        if steps == 0 || silenced {
            return None;
        }
        let ticks = steps.min(MAX_STEPS_PER_FRAME) as usize;
        // The console cadence (`skate_audio::mixmap::cadence`): retail's audio manager evaluates the
        // MixMap, steps the Jitter and clears the eEQChain buses once per 1/30 s console frame with
        // dt 1/30; here every second step, the flag inputs held in between.
        let calls = self.cadence.advance(ticks);
        // sub_82491180 in half 1 of the audio manager: every console frame (both halves run when the
        // frame is longer than 20 ms).
        let clear_eq = calls > 0;
        Some(Pass { ticks, calls, clear_eq })
    }
}

/// CSTATEMGR_Emitter's pool = the MixMap's Emitter instances.
pub(crate) const EMITTER_STATES: usize = 5;

/// The runtime, shared with the audio thread.
pub(crate) type Shared = Arc<Mutex<Runtime>>;

/// The running native runtime and what the game has loaded into it.
pub struct Native {
    pub(crate) shared: Shared,
    /// Bank stem → runtime bank id.
    pub(crate) banks: HashMap<String, usize>,
    pub(crate) emitter_class: Option<usize>,
    /// The MixMap, when the install has `MixMapSK8.mxb`.
    pub(crate) mixmap: Option<MixMap>,
    /// The front-end sounds (the session marker's cellphone), when the install has them.
    pub(crate) frontend: Option<crate::frontend::FrontendHost>,
    /// The zone beds (ambience).
    pub(crate) ambience: crate::ambience::AmbienceHost,
    /// Emitter banks read ahead of need (off the host thread).
    pub(crate) prefetch: crate::prefetch::Prefetch,
    /// The sound emitters of the map.
    pub(crate) emitters: crate::emitters::EmitterHost,
    /// The reverb zones the listener is in.
    pub(crate) zones: crate::emitters::ZoneHost,
    /// The host's clock (the physics steps).
    clock: HostClock,
    /// This frame's host ticks (0: no pass), for the systems after [`mixmap_frame`] (the bed).
    pub(crate) frame_ticks: usize,
    /// Camera cuts seen (teleport, map change; [`mixmap_frame`]): the world hosts reset their own
    /// camera velocity when it changes.
    pub(crate) cuts: u64,
    /// The cut signal last seen (`Presentation::cuts`, the map generation) and the wheel sample
    /// of the step at the cut (no seam interpolation across it).
    cut_seen: Option<(u64, u64)>,
    cut_wheels: Option<[[f32; 3]; 4]>,
    /// The flag inputs are held between evaluations (set once, at the first pass).
    holds: bool,
    /// The local player's MixMap inputs and components (None without a MixMap).
    pub(crate) player: Option<super::player_audio::PlayerAudio>,
    /// Which emitter states (MixMap Emitter instances 0..4) are taken.
    pub(crate) emitter_states: [bool; EMITTER_STATES],
    /// The granular rolling bed's game-side state (None: no rolling sound; an error is logged).
    pub(crate) bed: Option<super::grain_bed::Bed>,
    /// Bumped by every [`Native::unload_map_banks`] (map change): the world / NPC hosts release
    /// their nodes and reset their objects when it changes (an owner that survives the change would
    /// otherwise keep redelivering to a node whose instances the unload destroyed).
    pub(crate) map_epoch: u64,
    /// The world / NPC owners' instance counts the MixMap was built with.
    pub(crate) world: WorldInstances,
    /// This frame's pass between [`mixmap_frame`] (inputs, the local player's process) and
    /// [`mixmap_tick`] (the ticks, the update): the world / NPC hosts' process runs in between, as
    /// retail runs every owner's process before its tick and the update after it.
    pub(crate) pending: Option<PendingPass>,
    /// Banks audio content overlays load at start and keep across map changes (`preload`).
    pub(crate) resident: Vec<String>,
    /// The overlays' Csis projects installed in the runtime: (file, content stamp, registry token).
    pub(crate) mod_projects: Vec<(String, String, u64)>,
}

/// A pass [`mixmap_frame`] began and [`mixmap_tick`] finishes.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PendingPass {
    /// Console evaluations (MixMap ticks) in this pass.
    pub(crate) calls: usize,
    s: skate_audio::player::AudioState,
    speed_scale: Option<f32>,
    loose: u32,
    reverb: bool,
}

/// How many MixMap instances the world / NPC owners get: retail's free-skate layout
/// (`mixmap::RETAIL_INSTANCES`: 4 traffic, 15 pedestrian, 2 Player = the local player + 1 NPC
/// skater) or the opt-in non-retail "more audible" layout (settings/audio.json
/// `"more_audible_world": true`, user decision 2026-10-03; read at start): 8 traffic, 24
/// pedestrian, 4 Player (3 NPC / remote skaters). The MixMap is built with these counts, so the
/// extra objects get the same B lookups, distance curves and posts as retail's instances; only
/// their number is not retail. Instance 0 of every slot is unchanged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct WorldInstances {
    pub traffic: usize,
    pub peds: usize,
    /// Player-slot instances for NPC / remote skaters (instances 1..=npc).
    pub npc: usize,
}

impl WorldInstances {
    pub(crate) const RETAIL: Self = Self { traffic: 4, peds: 15, npc: 1 };
    pub(crate) const MORE_AUDIBLE: Self = Self { traffic: 8, peds: 24, npc: 3 };

    /// The MixMap's instances per slot.
    pub(crate) fn mixmap_instances(self) -> [usize; 14] {
        let mut n = skate_audio::mixmap::RETAIL_INSTANCES;
        n[skate_audio::world::keys::TRAFFIC as usize] = self.traffic;
        n[skate_audio::world::keys::PEDESTRIAN as usize] = self.peds;
        n[1] = 1 + self.npc;
        n
    }
}

impl Native {
    /// Start with retail's instance layout (the data-gated tests).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn start(library: &Library) -> Result<Self, String> {
        Self::start_with(library, WorldInstances::RETAIL)
    }

    pub(super) fn start_with(library: &Library, world: WorldInstances) -> Result<Self, String> {
        Self::start_from(library, world, 1)
    }

    /// Start, the evaluator handing out post ids from `first_node` on (1 = a fresh runtime; a
    /// restart continues the old runtime's counter, `content::restart`).
    pub(super) fn start_from(library: &Library, world: WorldInstances, first_node: u32) -> Result<Self, String> {
        let files = library.aems();
        if files.projects.is_empty() {
            return Err("this install has no AEMS banks (run setup to refresh the audio)".into());
        }
        let mut runtime = Runtime::new();
        runtime.eval.continue_nodes(first_node);
        let mut mod_projects = Vec::new();
        for file in &files.projects {
            let bytes = library.read(file).map_err(|e| format!("{file}: {e}"))?;
            let project = Project::parse(file, &bytes).map_err(|e| e.to_string())?;
            let token = runtime.install_project(&project);
            // An overlay's project (doc 16 "Mod Csis projects"): after the install's, in mod-id order.
            if crate::skate_mods::audio_merge::split_mod_ref(file).is_some() {
                mod_projects.push((file.clone(), library.stamp(file), token));
            }
        }
        let mixmap = match &files.mixmap {
            Some(file) => {
                let bytes = library.read(file).map_err(|e| format!("{file}: {e}"))?;
                let file = skate_audio::mixmap::MixMapFile::parse(&bytes).map_err(|e| e.to_string())?;
                let m = MixMap::new(&file, &world.mixmap_instances());
                if world != WorldInstances::RETAIL {
                    info!("Game audio: more audible world (not retail): {} traffic, {} pedestrian, {} NPC skater instances", world.traffic, world.peds, world.npc);
                }
                info!("Game audio: MixMap {} controllers ({} output blocks)", m.controller_count(), m.output_blocks());
                Some(m)
            }
            None => {
                error!("Game audio: this install has no MixMapSK8.mxb: the skater's sounds and rolling are silent and emitter words use defaults (run setup to refresh the audio)");
                None
            }
        };
        let bed = if mixmap.is_some() { super::grain_bed::Bed::new(library) } else { None };
        let player = mixmap.as_ref().map(|_| {
            super::player_audio::PlayerAudio::new(library.player_tuning(), true)
        });
        if bed.is_none() {
            error!("Game audio: no MixMap, whole grain recordings or grain tuning in this install: rolling is silent (run setup to refresh the audio)");
        }
        let mut native = Self {
            shared: Arc::new(Mutex::new(runtime)),
            banks: HashMap::new(),
            emitter_class: None,
            mixmap,
            frontend: None,
            ambience: Default::default(),
            prefetch: Default::default(),
            emitters: Default::default(),
            zones: Default::default(),
            clock: HostClock::default(),
            frame_ticks: 0,
            cuts: 0,
            cut_seen: None,
            cut_wheels: None,
            holds: false,
            player,
            emitter_states: [false; EMITTER_STATES],
            bed,
            map_epoch: 0,
            world,
            pending: None,
            resident: Vec::new(),
            mod_projects,
        };
        // The environment (reverb) network and the eEQChain buses (optional install data).
        let (presets, eq) = library.bus_tuning();
        if !presets.is_empty() || !eq.is_empty() {
            let mut runtime = native.shared.lock().map_err(|_| "audio lock poisoned")?;
            info!("Game audio: native buses on ({} reverb presets, {} eEQChain buses)", presets.len(), eq.len());
            runtime.mixer.buses.env.presets = presets;
            runtime.mixer.buses.eq.set_records(&eq);
        }
        // The FlangeSub effect returns (GRINDS and the skid send into return A).
        if let Some([a, b]) = library.flange_presets() {
            let mut runtime = native.shared.lock().map_err(|_| "audio lock poisoned")?;
            info!("Game audio: native FlangeSub returns on");
            runtime.mixer.buses.flange.set_presets(a, b);
        }
        // The FootStep SubMix graphs (`sub_82494188`: each foot sound through its EQ record, env
        // send and panner).
        if let Ok(mut runtime) = native.shared.lock() {
            runtime.mixer.buses.submix.enabled = true;
        }
        native.ensure_bank(library, "emitter_utility")?;
        native.load_player_banks(library);
        let mut runtime = native.shared.lock().map_err(|_| "audio lock poisoned")?;
        let utility = runtime.eval.class_id("c_emitter_utility").ok_or("no c_emitter_utility class")?;
        runtime.post(utility, &[]);
        drop(runtime);
        // Retail's second boot utility (Start_up_Play_ctl, Common.abk): the Seams program's sample
        // shuffles. Older installs without Common.abk keep the fixed seam samples.
        match native.ensure_bank(library, skate_audio::player::seams::UTILITY_BANK) {
            Ok(_) => {
                let mut runtime = native.shared.lock().map_err(|_| "audio lock poisoned")?;
                if let Some(class) = runtime.eval.class_id(skate_audio::player::seams::UTILITY) {
                    runtime.post(class, &[]);
                }
            }
            Err(e) => warn!("Game audio: {e}; seam hits play fixed samples (rerun setup)"),
        }
        // Then the foley utility (the cloth_trick programs call it): retail's boot order
        // c_emitter_utility → Start_up_Play_ctl → c_foley_utility, which matters because every
        // program shares one random generator.
        if native.player.as_ref().is_some_and(|p| p.components && p.tricks_on) {
            let mut runtime = native.shared.lock().map_err(|_| "audio lock poisoned")?;
            if let Some(id) = runtime.eval.class_id(skate_audio::player::tricks::FOLEY_UTILITY) {
                runtime.post(id, &[]);
            }
        }
        let runtime = native.shared.lock().map_err(|_| "audio lock poisoned")?;
        native.emitter_class = runtime.eval.class_id("c_emitter");
        drop(runtime);
        // The front-end sounds (the session marker's cellphone UI; sk8_menu).
        native.frontend = crate::frontend::FrontendHost::load(&native, library);
        Ok(native)
    }

    /// Load a bank (and its samples) unless it is loaded already. A bank the prefetch worker has
    /// read and decoded is taken from it (waiting if it is mid-decode); otherwise it is read and
    /// decoded here, by the same `BankSource::load`. Either way `load_bank` runs now.
    pub(crate) fn ensure_bank(&mut self, library: &Library, stem: &str) -> Result<usize, String> {
        if let Some(&id) = self.banks.get(stem) {
            return Ok(id);
        }
        let (bank, pcm) = match self.prefetch.take(stem) {
            Some(data) => data,
            None => library.bank_source(stem)?.load()?,
        };
        let mut runtime = self.shared.lock().map_err(|_| "audio lock poisoned")?;
        let id = runtime.load_bank(bank, pcm);
        // An overlay can put a bank in a volume group (none without overlays).
        if let Some(player) = library.bank_group(stem) {
            runtime.mixer.set_bank_group(id, if player { skate_audio::mixer::GROUP_PLAYER } else { skate_audio::mixer::GROUP_WORLD });
        }
        drop(runtime);
        self.banks.insert(stem.to_owned(), id);
        Ok(id)
    }

    /// The player components' banks, in the player volume group; without them the components
    /// stay off and the skater's sounds are silent.
    fn load_player_banks(&mut self, library: &Library) {
        if !self.player.as_ref().is_some_and(|p| p.components) {
            return;
        }
        for stem in super::player_audio::BANKS {
            match self.ensure_bank(library, stem) {
                Ok(id) => {
                    if let Ok(mut runtime) = self.shared.lock() {
                        runtime.mixer.set_bank_group(id, skate_audio::mixer::GROUP_PLAYER);
                    }
                }
                Err(e) => {
                    error!("Game audio: the skater's sounds are silent ({e}; run setup to refresh the audio)");
                    if let Some(p) = &mut self.player {
                        p.components = false;
                    }
                    return;
                }
            }
        }
        info!("Game audio: native player components on (Class_grind, SenseOfSpeed, Class_foot_drag)");
        self.load_optional_player_banks(library);
        // The Splice banks (pops, landings, touchdowns), decoded up front so a sample's first
        // trigger sounds like every later one.
        let mut first = false;
        for (i, stem) in super::player_audio::SPLICE_BANKS.iter().enumerate() {
            match library.splice_bank(stem) {
                Some((bank, pcm)) => {
                    if let Ok(mut runtime) = self.shared.lock() {
                        let rt = &mut *runtime;
                        rt.splice.load_bank(stem, bank, pcm, &mut rt.mixer);
                        first |= i == 0;
                    }
                }
                None => error!("Game audio: {stem} has no patch tree in this install: its sounds are silent (run setup to refresh the audio)"),
            }
        }
        // SFXObj_Wheels' spin-down recordings, decoded now (first trigger = later triggers).
        let streams: Vec<_> = super::player_audio::WHEEL_STREAMS.iter().map(|n| library.wheels_pcm(n)).collect();
        let wheels = streams.iter().all(Option::is_some);
        if let Ok(mut runtime) = self.shared.lock() {
            runtime.load_streams(streams);
        }
        if let Some(p) = &mut self.player {
            p.contacts_on = first;
            p.set_footstep_materials(library.footstep_materials());
            p.footsteps_on = first;
            p.contact_tuning = library.contacts_tuning();
            p.wheels_on = wheels;
            if p.contacts_on {
                info!("Game audio: native board contacts on (Splice: pops, landings, touchdowns, foot taps, scuffs; collision pairs: {} materials)", p.tuning.collision.materials.len());
            }
            if wheels {
                info!("Game audio: native wheel spin on (SFXObj_Wheels)");
            }
        }
    }

    /// The optional components' banks (rolling layers, rattle, board slide, tricks, treatment):
    /// each component runs when its first bank is in the install; missing later banks only drop
    /// their layers (a Class_rolling post reaches every bank bound to the class).
    fn load_optional_player_banks(&mut self, library: &Library) {
        use super::player_audio::{RATTLE_BANKS, ROLLING_BANKS, SLIDE_BANKS, TREATMENT_BANKS, TRICKS_BANKS};
        let load = |me: &mut Self, banks: &[&str]| -> bool {
            let mut first = false;
            for (i, stem) in banks.iter().enumerate() {
                match me.ensure_bank(library, stem) {
                    Ok(id) => {
                        if let Ok(mut runtime) = me.shared.lock() {
                            runtime.mixer.set_bank_group(id, skate_audio::mixer::GROUP_PLAYER);
                        }
                        first |= i == 0;
                    }
                    Err(e) => warn!("Game audio: {e}; its native layers stay off (run setup to refresh the audio)"),
                }
            }
            first
        };
        let rolling = load(self, ROLLING_BANKS);
        let rattle = load(self, RATTLE_BANKS);
        let slide = load(self, SLIDE_BANKS);
        let tricks = load(self, TRICKS_BANKS);
        let treatment = load(self, TREATMENT_BANKS);
        // With the tricks the foley utility is posted at boot, after Start_up_Play_ctl (`start`).
        if let Some(p) = &mut self.player {
            (p.rolling_on, p.rattle_on, p.slide_on, p.tricks_on, p.treatment_on) = (rolling, rattle, slide, tricks, treatment);
            info!("Game audio: native rolling layers {rolling}, rattle {rattle}, board slide {slide}, tricks {tricks}, treatment {treatment}");
        }
    }

    /// The runtime's id of a loaded bank.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn bank_id(&self, stem: &str) -> Option<usize> {
        self.banks.get(stem).copied()
    }

    /// Whether the runtime holds the bank.
    pub(crate) fn bank_loaded(&self, stem: &str) -> bool {
        self.banks.contains_key(stem)
    }

    /// The loaded banks' stems, sorted (the audio catalog).
    pub(crate) fn loaded_banks(&self) -> Vec<String> {
        let mut v: Vec<String> = self.banks.keys().cloned().collect();
        v.sort();
        v
    }

    pub(crate) fn redeliver(&self, node: NodeId, payload: &[i32]) {
        if let Ok(mut runtime) = self.shared.lock() {
            runtime.redeliver(node, payload);
        }
    }

    pub(crate) fn release(&self, node: NodeId) {
        if let Ok(mut runtime) = self.shared.lock() {
            runtime.release(node);
        }
    }
}

/// The one-frame flag inputs the console's writers set for a whole evaluation, held between our 60
/// Hz writes and the 30 Hz evaluations ([`MixMap::hold_input`]): Contacts 1 / 6 (the landing
/// swell and the landing-material flag), Rail 1 (the grind ended), SkateBoard 0 / 4 (the surface
/// change, the push plant), Cracks 0 (a seam hit). Everything else the host writes is a level that
/// holds between writes.
pub(crate) fn hold_flag_inputs(m: &mut MixMap) {
    for (key, id) in [(keys::contacts(0), 1), (keys::contacts(0), 6), (keys::rail(0), 1), (keys::skateboard(0), 0), (keys::skateboard(0), 4), (keys::cracks(0), 0)] {
        m.hold_input(key, id);
    }
}

/// The `c_emitter` azimuth word for a source seen from the listener: 0 = straight ahead,
/// increasing clockwise (to the right), 65536 = 360°.
pub(crate) fn azimuth(listener: &Listener, source: glam::Vec3) -> i32 {
    // skate_vita: the reference takes the camera's transform; here its position and forward
    // (local x = the source along the camera's right, −z along its forward).
    let to = source - listener.position;
    let right = listener.forward.cross(glam::Vec3::Y).normalize_or_zero();
    let degrees = to.dot(right).atan2(to.dot(listener.forward)).to_degrees();
    ((degrees / 360.0 * 65536.0).round() as i32).rem_euclid(65536)
}

/// skate_vita: [`Native::render`] from another thread.
pub struct Renderer(Shared);

impl Renderer {
    pub fn render(&self, out: &mut [f32]) {
        if let Ok(mut runtime) = self.0.lock() {
            runtime.fill_stereo(out);
        }
    }

    pub fn voices(&self) -> usize {
        self.0.lock().map_or(0, |r| r.mixer.voice_count())
    }
}

/// skate_vita: the camera the game renders from (the reference's `GameAudioListener` transform).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Listener {
    pub position: glam::Vec3,
    /// Unit view direction.
    pub forward: glam::Vec3,
}

/// What one rendered frame gives the host (skate_vita: the reference's Bevy resources).
#[derive(Clone, Copy, Debug)]
pub struct FrameInput<'a> {
    /// The skater's publication (`skate_events::Cues::riding`) and the physics steps since the
    /// last frame (`Cues::take_steps`): `skate_sim::live::Session::take_audio`.
    pub riding: &'a skate_sim::AudioRiding,
    pub steps: u32,
    /// The camera (`GameAudioListener`), when there is one.
    pub listener: Option<Listener>,
    /// The cut signal: `Presentation::cuts` and the map generation.
    pub cuts: (u64, u64),
    /// The fixed clock's overstep fraction (`Time<Fixed>::overstep_fraction`).
    pub overstep: f32,
    /// This frame's game time (`Time<Virtual>::delta_secs`; 0 while paused) and real time
    /// (`Time<Real>::delta_secs`, the reverb crossfade).
    pub game_dt: f32,
    pub real_dt: f32,
    /// The menu or a replay silences the game (`super::silenced`).
    pub silenced: bool,
    /// Front-end records to play (`skate_audio::frontend::key` of their names: the session
    /// marker's `frontend_marker` sounds, `ui_audio::FrontendSound`).
    pub frontend: &'a [u64],
    /// The teleport effect amount (`ui_audio::TeleportEffect`): the session marker's Go To Marker
    /// hold, 0..=1; None when not sent.
    pub teleport: Option<f32>,
    /// The map the skater is in: its name (the retail map stem, `DownTown`) and a number that
    /// changes with every load; None while there is no map (no beds).
    pub map: Option<(&'a str, u64)>,
}

impl Native {
    /// skate_vita: interleaved 48 kHz stereo from the runtime (the reference's Bevy stream pulled
    /// it through `Runtime::fill_stereo`).
    /// skate_vita: a handle the audio thread renders through while the host pass runs elsewhere.
    pub fn renderer(&self) -> Renderer {
        Renderer(self.shared.clone())
    }

    /// skate_vita: the most voices that may sound at once (retail retires voices by CPU; the Vita
    /// cannot mix the reference's 96).
    pub fn set_max_voices(&self, n: usize) {
        if let Ok(mut runtime) = self.shared.lock() {
            runtime.mixer.max_voices = n;
        }
    }

    /// skate_vita: voices the world group (emitters, beds) leaves free for the player's sounds.
    pub fn set_world_reserve(&self, n: usize) {
        if let Ok(mut runtime) = self.shared.lock() {
            runtime.mixer.world_reserve = n;
        }
    }

    /// skate_vita diagnostic: the mixer's open voices.
    pub fn voices(&self) -> usize {
        self.shared.lock().map_or(0, |r| r.mixer.voice_count())
    }

    pub fn render(&self, out: &mut [f32]) {
        if let Ok(mut runtime) = self.shared.lock() {
            runtime.fill_stereo(out);
        }
    }

    /// One rendered frame of the host, in the reference's system order for the board path:
    /// `native::mixmap_frame` → `native::mixmap_tick` → `grain_bed::update` → `native::reverb_frame`
    /// (the world / NPC owners, the front end and the mods between them are not ported).
    pub fn frame(&mut self, library: &Library, f: FrameInput) {
        self.ambience_update(library, &f);
        self.emitters_frame(library, &f);
        self.mixmap_frame(&f);
        self.mixmap_tick();
        self.ambience_output(library);
        self.frontend_frame(f.frontend);
        self.grain_bed_update(library, f.riding);
        self.reverb_frame(library, &f);
    }

    /// The reference's `mixmap_frame` body (resources as arguments).
    fn mixmap_frame(&mut self, f: &FrameInput) {
        let silenced = f.silenced;
        let native = self;
        // For the systems after this one (the bed): no pass until the clock says so.
        native.frame_ticks = 0;
        native.pending = None;
        if let Some(bed) = &mut native.bed {
            bed.slew_calls = Some(0);
        }
        if native.mixmap.is_none() {
            return;
        }
        // A teleport, camera cut or map change: no camera velocity across it (Doppler), and no seam
        // interpolation from the old place (the step at the cut is both ends of the pair).
        let signal = f.cuts;
        let now = f.riding.audio.wheel_position;
        if native.cut_seen.is_some_and(|seen| seen != signal) {
            native.cuts += 1;
            native.cut_wheels = Some(now);
            if let Some(player) = &mut native.player {
                player.reset_listener();
            }
        }
        native.cut_seen = Some(signal);
        if native.cut_wheels.is_some_and(|w| w != now) {
            native.cut_wheels = None;
        }
        let before = if native.cut_wheels.is_some() { now } else { f.riding.wheels_before };
        // Class_Seams on the console's 30 fps process cadence at the rendered board's wheels, on every
        // rendered frame (Listening test 9), in game time (`Time<Virtual>`: still while paused).
        let Some(m) = &mut native.mixmap else { return };
        if let Some(player) = &mut native.player {
            player.seam_alpha = Some(f.overstep);
            player.step_wheels(before, now);
            if !silenced {
                if let Ok(mut runtime) = native.shared.lock() {
                    player.seam_frame(m, &f.riding.audio, f.game_dt, &mut runtime);
                }
            }
        }
        let Some(pass) = native.clock.pass(f.steps, silenced) else { return };
        native.frame_ticks = pass.ticks;
        for id in 1..=4 {
            m.set_input(keys::MASTER, id, 32767);
        }
        for id in [1, 2, 5] {
            m.set_input(keys::MUSIC, id, 32767);
        }
        // SFXObj_Reverb's first step (`sub_824DF468`): Reverb.in0..6 from the number of the preset
        // being faded to (in5 = reverb11/12/15/16/22, which ducks the global env scale out4 by 4 dB).
        // (Without the runtime lock: in5 = 32767 and no scale this pass.)
        let reverb = native.shared.lock().ok().map(|r| r.mixer.buses.env.reverb_inputs());
        match reverb {
            Some(v) => {
                for (id, x) in v.into_iter().enumerate() {
                    m.set_input(keys::REVERB, id, x);
                }
            }
            None => m.set_input(keys::REVERB, 5, 32767),
        }
        m.set_input(keys::PAUSE, 0, 0);
        let (ticks, calls) = (pass.ticks, pass.calls);
        // The bed's turn / brake slews step once per console evaluation (`grain_bed::Bed::slew_calls`).
        if let Some(bed) = &mut native.bed {
            bed.slew_calls = Some(calls);
        }
        if !native.holds {
            hold_flag_inputs(m);
            native.holds = true;
        }
        // sub_82491180 (half 1): the jittered eEQChain buses take the walk's values and every bus may
        // re-roll again.
        if pass.clear_eq {
            let jitter = native.player.as_ref().and_then(|p| p.eq_jitter());
            if let Ok(mut runtime) = native.shared.lock() {
                runtime.mixer.buses.eq.clear(jitter);
            }
        }
        let s = f.riding.audio;
        if let Some(player) = &mut native.player {
            let dt = ticks as f32 * MIX_STEP;
            let l = f.listener.map(|t| player.listener(t.position.to_array(), t.forward.to_array(), dt, &s));
            // SFXObj_Jitter's walk steps once per console evaluation (half 1's process).
            player.jitter_steps = Some(calls);
            // The presentation block's teleport field (Class_Treatment's update reads it): the
            // session marker's Go To Marker hold.
            player.teleport_effect = f.teleport;
            player.write_inputs(m, &s, l.as_ref());
        }
        // With the native rolling layers the owner's surface routing (player::rolling) writes
        // SkateBoard inputs 0 / 6 and drives the bed's binds; otherwise the bed routes itself.
        let routing = native.player.as_ref().is_some_and(|p| p.components && p.rolling_on);
        if let Some(bed) = &mut native.bed {
            bed.write_inputs(m, &s, !routing);
        }
        let speed_scale = native.bed.as_ref().and_then(|b| b.push_scale());
        let loose = crate::player_audio::PlayerAudio::loose_board(&s, f.riding);
        if let (Some(player), Ok(mut runtime)) = (&mut native.player, native.shared.lock()) {
            player.process(m, &s, &mut runtime, speed_scale, loose);
        }
        native.pending = Some(PendingPass { calls, s, speed_scale, loose, reverb: reverb.is_some() });
    }

    /// The reference's `mixmap_tick` body (no mods' input writes).
    fn mixmap_tick(&mut self) {
        let native = self;
        let Some(PendingPass { calls, s, speed_scale, loose, reverb }) = native.pending else { return };
        let Some(m) = &mut native.mixmap else { return };
        for _ in 0..calls {
            m.tick(skate_audio::mixmap::cadence::CONSOLE_DT);
        }
        if let (Some(player), Ok(mut runtime)) = (&mut native.player, native.shared.lock()) {
            player.update(m, &s, &mut runtime, speed_scale, loose);
        }
        // SFXObj_Reverb's update (`sub_824DF220`): the FlangeSub returns' levels and the global env
        // scale (manager +104 = out4) from the Reverb owner.
        if let Ok(mut runtime) = native.shared.lock() {
            runtime.mixer.buses.flange.frame(std::array::from_fn(|i| m.level(keys::REVERB, i)));
            if reverb {
                runtime.mixer.buses.env.scale_frame(m.level(keys::REVERB, 4));
            }
        }
    }

    /// Whether the install has front-end sounds (the session marker's cellphone).
    pub fn has_frontend(&self) -> bool {
        self.frontend.is_some()
    }

    /// The reference's `game_audio::frontend::frame` system: this frame's requests, then (on frames
    /// with a pass) the object once, its volume from the MixMap's Master output 0 (`sub_824958F0`:
    /// 14568 with free-skate's Master inputs).
    fn frontend_frame(&mut self, requests: &[u64]) {
        let Some(host) = &mut self.frontend else { return };
        for &key in requests {
            host.request(key);
        }
        if self.frame_ticks == 0 {
            return;
        }
        if let Some(m) = &self.mixmap {
            host.core.volume = crate::frontend::volume(m.level(keys::MASTER, 0));
        }
        let dt = self.frame_ticks as f32 * MIX_STEP;
        if let Ok(mut runtime) = self.shared.lock() {
            host.frame(dt, &mut runtime);
        }
    }

    /// The reference's `grain_bed::update` body.
    fn grain_bed_update(&mut self, library: &Library, riding: &skate_sim::AudioRiding) {
        let native = self;
        let ticks = native.frame_ticks;
        if ticks == 0 {
            return;
        }
        // The owner's routing (native rolling layers): its binds / stops since the last pass.
        let routed = native.player.as_mut().filter(|p| p.components && p.rolling_on).map(|p| (std::mem::take(&mut p.routed.grains), p.routed.primary));
        // The seam wobbles live with the player tuning.
        static NO_TUNING: std::sync::OnceLock<skate_audio::player::tuning::PlayerTuning> = std::sync::OnceLock::new();
        let seams = native.player.as_ref().map_or_else(|| NO_TUNING.get_or_init(Default::default), |p| &p.tuning);
        let (Some(bed), Some(m)) = (&mut native.bed, &native.mixmap) else { return };
        crate::grain_bed::step(bed, library, m, &native.shared, riding, ticks as f32 * MIX_STEP, seams, routed);
    }

    /// The reference's `reverb_frame` body: SFXObj_Reverb's update with the preset of the map's
    /// `audio_reverb` region under the skater (reverb01 where none covers it) and the reverb-zone
    /// emitters the listener is in (`emitters::ZoneHost`).
    fn reverb_frame(&mut self, library: &Library, f: &FrameInput) {
        // The region layer's preset where the skater stands (the reference's key where none covers).
        let at = f.riding.board;
        let key = f
            .map
            .and_then(|(map, _)| library.region_key(map, "audio_reverb", at.x, at.z))
            .unwrap_or(skate_audio::bus::env::DEFAULT_PRESET);
        self.zones.update(library, f.map, f.listener.map(|t| t.position));
        let camera = f.listener.map(|t| skate_audio::bus::zones::Camera { position: t.position.to_array(), forward: t.forward.to_array() });
        if let Ok(mut runtime) = self.shared.lock() {
            let env = &mut runtime.mixer.buses.env;
            if env.enabled() {
                env.update(f.real_dt.clamp(0.0, 0.25), key, &self.zones.out, camera.as_ref());
            }
        }
    }
}
