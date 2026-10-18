//! The game side of the native granular rolling bed (`skate_audio::grain`, spec
//! `audio-specs/grain-player-spec.md`), on when the native runtime runs with a MixMap and the
//! install has the whole grain recordings and their vault tuning (`audio_export.grain_whole` /
//! `grain_tuning`). Without them rolling is silent and the host logs an error (2026-10-03: the
//! interim speed-band loop is gone).
//!
//! Per frame, after the MixMap tick (`native::mixmap_frame`):
//! - **Surface routing** (§1.4, one sounding truck): the wheels' majority surface → grain member
//!   ([`grain_for`], the retail AudioSurfaceMap); a change stops both players and binds the new
//!   member (pulsing SkateBoard input 0); grinding (surface 14) stops them. In the air the last
//!   member keeps playing and the MixMap's no-contact duck silences it (what retail does with the
//!   wheel material in the air is UNCERTAIN, spec §2.10).
//! - **Records** (§2.2): position A from the member's Bézier of speed (push-scaled), B 0.1 behind;
//!   gains from SkateBoard level(1)/(2), pitch from pitch(3); the push speed-scale envelope, the
//!   brake slew, the turn intensity (`sub_824C8588`: |COM v|·0.24 capped × the turn input, slewed),
//!   the manual / trick latches ("special": A × special gain, no turn layer), the downhill level D
//!   (`sub_824CA738`, owner inputs 2/3) and wheel 0's seam-pattern gain envelope (`sub_824CA448`).
//!   Player B is the turning / downhill layer. The turn and brake slews step once per console
//!   frame (`Bed::slew_calls`, the MixMap cadence's evaluations; 2026-10-03), as retail's per-call
//!   steps on the ~30 fps console.
//! - **Soft wheels** (`sub_824C8370`): the soft member of surfaces 1..6 while the board's wheel
//!   hardness is below 0.5 (audio state `+684` = Motion+200 < 0.5); metal has only a hard member.
//! - **Chains** (§3.2, `skate_audio::grain::chain`): high-/low-pass from level(12)/(11), pan from
//!   raw(0), the FrequencyShiftSsb values (the +150 Hz special shift, the push shift envelope, the
//!   slope terms), the graph-1 level ramp and gain wobbles, the graph-3 send by speed, the env send
//!   level(13) and the FlangeSub send level(21)/(22) (carried, not rendered).
//! - **Rocket** (§2.8): `x_jet_rolling` above the owner's start speed (35 km/h), into the default bus.
//! - **Routing:** with the native rolling layers (`PlayerAudio::rolling_on`) the owner's two-truck
//!   surface routing (`player::rolling`, `sub_824C5CA8`) decides the binds and stops ([`GrainEvent`]s)
//!   and writes SkateBoard inputs 0 / 6; otherwise the bed routes one truck itself (as before).
//! - **The NPC skater's bed** ([`Bed::for_instance`], `npc_skaters.rs`, 2026-10-03): retail's
//!   SkateBoard update `sub_824C6BD8` and most of its process run for every Player-slot instance, so
//!   the NPC holding instance 1 gets its own bed (`Runtime::npc_grains`) on its own SkateBoard
//!   outputs, fed by its routing. The parts with a local test (`[owner+16]+72`) stay the local
//!   player's: the seam-pattern gain envelope (`sub_824CA448`), the graph-1 → graph-3 send and level
//!   ramp (`sub_824CAEC0`), the gain wobbles (`sub_824CB180`), graph 3 itself (`sub_824C8878`), the
//!   graph-2 FlangeSub sends (level(21) / (22), `sub_824C9058`) and the rocket (SenseOfSpeed).
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use glam::Vec3;
use tracing::{info, warn};
use skate_audio::eval::rng::Rng;
use skate_audio::grain::board::{self, BoardInputs, Latches, OwnerSlews, PushEnvelope, RocketTuning, SeamEnvelope, SurfaceTuning};
use skate_audio::grain::chain::{self, ChainFrame, ChainState, ChainTuning, PushShift};
use skate_audio::grain::{GrainFile, GrainSource};
use skate_audio::player::rolling::{self, GrainEvent};
use skate_audio::mixmap::{MixMap, keys};

use super::library::Library;

/// Every member [`grain_for`] can name, plus the rocket layer.
const MEMBERS: &[&str] = &[
    "asphalt_rough_hard",
    "concrete_rough_hard",
    "concrete_smooth_hard",
    "wood_ramp_hard",
    "concrete_aggregate_hard",
    "metal_smooth_hard",
    "asphalt_smooth_hard",
];
/// Soft-wheel members (surfaces 1..6), used when the install has them.
const SOFT_MEMBERS: &[&str] = &[
    "asphalt_rough_soft",
    "concrete_rough_soft",
    "concrete_smooth_soft",
    "wood_ramp_soft",
    "concrete_aggregate_soft",
    "asphalt_smooth_soft",
];
const ROCKET: &str = "x_jet_rolling";

/// Rolling grain for a wheel's 7-bit audio surface tag, as retail maps it: tag → material (tag − 1) →
/// the vault's `Sk8::AudioSurfaceMap` → rolling surface 1–14 → grain member
/// (audio-specs/grain-player-spec.md §1.4). Only the hard-wheel members: [`Bed::member`] swaps in
/// the soft one. The bed's own one-truck routing uses it (the native rolling layers route with
/// `player::rolling`).
pub(super) fn grain_for(audio_surface: u32) -> &'static str {
    match audio_surface & 0x7F {
        2 => "asphalt_rough_hard",                                                    // surface 1
        4 | 66 => "concrete_rough_hard",                                              // surface 2
        3 | 51..=54 | 57 | 60 | 63..=65 | 92 | 93 => "concrete_smooth_hard",          // surface 4
        6 | 7 | 41..=50 | 58 | 59 | 94 => "wood_ramp_hard",                           // surface 5
        5 => "concrete_aggregate_hard",                                               // surface 6
        9 | 11..=36 | 38..=40 | 84 | 85 | 89 | 91 => "metal_smooth_hard",             // surface 9
        // Stand-ins: retail plays no grain on these. Rolling surfaces 7 (tags 10, 70), 8 (tag 8),
        // 10 (67), 12 (68, 69) and 13 (37) post a per-surface `Class_rolling` patch (the native
        // rolling layers, `player::rolling`).
        8 | 10 | 70 => "concrete_aggregate_hard",
        37 | 67..=69 => "metal_smooth_hard",
        // UNCERTAIN: tag 90 maps to rolling surface 0, which has no grain member.
        90 => "wood_ramp_hard",
        // Surface 3 asphalt_smooth: tags 1, 55, 56, 61, 62, 71–83, 86–88, ≥ 95 and 0 (no material).
        _ => "asphalt_smooth_hard",
    }
}
/// The `default` collection's tuning key in [`Bed`]'s map (rolling surface 0 binds
/// asphalt_rough_hard with it, `rolling::member`).
const DEFAULT_TUNING: &str = "default";

/// A bound truck: the recording and the collection whose tuning it plays with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Bound {
    stem: &'static str,
    tuning: &'static str,
}

pub(crate) struct Bed {
    tunings: HashMap<&'static str, SurfaceTuning>,
    rocket: Option<RocketTuning>,
    chain_tuning: ChainTuning,
    tuned: bool,
    sources: HashMap<String, Arc<GrainSource>>,
    failed: HashSet<String>,
    /// Per truck (the bed's own routing uses truck 0 only).
    bound: [Option<Bound>; 2],
    /// The routing's primary truck (`+1500`).
    primary: usize,
    rocket_on: bool,
    /// SkateBoard owner inputs for the next tick: 0 surface-change pulse, 4 push plant.
    pulse_surface: bool,
    pulse_push: bool,
    pushes_seen: Option<u32>,
    push_scale: PushEnvelope,
    /// The push frequency-shift envelope (`+1036`).
    shift: PushShift,
    /// The turn intensity (`sub_824C8588`) and brake slews: retail steps each once per call of the
    /// SkateBoard process, no dt.
    slews: OwnerSlews,
    /// The console evaluations ending in this pass (`mixmap::cadence`), set by the host before each
    /// [`step`]; taken by it. The slews run on the console cadence (2026-10-03, user decision "lets
    /// go for like retail"): one retail step per console frame, so a release takes the console's
    /// ~333 ms at any real frame rate. `None` (unit tests only): the per-frame slews (the step ×
    /// dt·60).
    pub(crate) slew_calls: Option<usize>,
    /// Send, level ramp, wobbles and the gains each chain holds.
    chain: ChainState,
    latches: Latches,
    seam: SeamEnvelope,
    /// Our own instance of the title generator for the seam draws (retail's is shared and
    /// unreproducible anyway).
    rng: Rng,
    /// Downhill level D (owner `+1508`), computed with the owner inputs.
    downhill: f32,
    /// The Player-slot instance this bed belongs to: 0 = the local player (`Runtime::grains`),
    /// 1 = the NPC skater holding instance 1 (`Runtime::npc_grains`, [`Bed::for_instance`]).
    instance: u32,
}

impl Bed {
    pub(crate) fn new(library: &Library) -> Option<Self> {
        let mut tunings = HashMap::new();
        for &name in MEMBERS {
            library.grain_whole(name)?;
            tunings.insert(name, library.grain_tuning(name)?);
        }
        for &name in SOFT_MEMBERS {
            if let (Some(_), Some(t)) = (library.grain_whole(name), library.grain_tuning(name)) {
                tunings.insert(name, t);
            }
        }
        if let Some(t) = library.grain_default_tuning() {
            tunings.insert(DEFAULT_TUNING, t);
        }
        let rocket = library.rocket_tuning().filter(|_| library.grain_whole(ROCKET).is_some());
        let chain_tuning = library.chain_tuning();
        let mut bed = Self {
            tunings,
            rocket,
            chain_tuning,
            tuned: false,
            sources: HashMap::new(),
            failed: HashSet::new(),
            bound: [None; 2],
            primary: 0,
            rocket_on: false,
            pulse_surface: false,
            pulse_push: false,
            pushes_seen: None,
            push_scale: PushEnvelope::default(),
            shift: PushShift::default(),
            slews: OwnerSlews::default(),
            slew_calls: None,
            chain: ChainState::default(),
            latches: Latches::default(),
            seam: SeamEnvelope::default(),
            rng: Rng::new(skate_audio::grain::GrainBed::SEED),
            downhill: 0.0,
            instance: 0,
        };
        // Decode every recording now: a grain's first trigger must sound like the later ones
        // (no decode on the game thread at first use).
        let names: Vec<&'static str> = bed.tunings.keys().copied().filter(|n| *n != DEFAULT_TUNING).chain(bed.rocket.map(|_| ROCKET)).collect();
        for name in names {
            bed.source(library, name);
        }
        Some(bed)
    }

    /// A fresh bed for Player-slot instance `instance` ≥ 1 (an NPC skater's board): the same
    /// tunings and decoded recordings (shared `Arc`s, nothing decoded again), no rocket (local
    /// only), every modulator at its start.
    pub(crate) fn for_instance(&self, instance: u32) -> Self {
        Self {
            tunings: self.tunings.clone(),
            rocket: None,
            chain_tuning: self.chain_tuning,
            tuned: false,
            sources: self.sources.clone(),
            failed: self.failed.clone(),
            bound: [None; 2],
            primary: 0,
            rocket_on: false,
            pulse_surface: false,
            pulse_push: false,
            pushes_seen: None,
            push_scale: PushEnvelope::default(),
            shift: PushShift::default(),
            slews: OwnerSlews::default(),
            slew_calls: None,
            chain: ChainState::default(),
            latches: Latches::default(),
            seam: SeamEnvelope::default(),
            rng: Rng::new(skate_audio::grain::GrainBed::SEED),
            downhill: 0.0,
            instance,
        }
    }

    /// The member for a wheel surface tag: the soft member while the wheels are soft and the
    /// install has it (`sub_824C8370`).
    fn member(&self, tag: u32, soft: bool) -> &'static str {
        let hard = grain_for(tag);
        if soft {
            if let Some(&name) = SOFT_MEMBERS.iter().find(|n| n.strip_suffix("_soft") == hard.strip_suffix("_hard")) {
                if self.tunings.contains_key(name) {
                    return name;
                }
            }
        }
        hard
    }

    /// The primary truck's binding (or the other's when only that one runs).
    fn primary_bound(&self) -> Option<Bound> {
        self.bound[self.primary].or(self.bound[1 - self.primary])
    }

    /// The slewed turn intensity (signed, owner `+1160`) and brake (`+1168`): diagnostics (e2e).
    #[cfg(test)]
    pub(crate) fn slews(&self) -> (f32, f32) {
        (self.slews.turn.value, self.slews.brake)
    }

    /// The push speed-scale envelope while it runs (`player::rolling` scales its patch speeds).
    pub(crate) fn push_scale(&self) -> Option<f32> {
        self.push_scale.value()
    }

    /// The board owner's MixMap inputs (written before the tick): 0 = 32767 for the frame after a
    /// surface change, 4 = 32767 on a push plant, 6 = 32767 on metal, 2 / 3 = the downhill /
    /// uphill levels (`sub_824CA738`: the primary truck on a grain surface, slope `+712` over the
    /// member's divisors). Heading rate (5) and skid (1) reach no MixMap output and are not written.
    /// `own_routing`: the bed routes itself and writes 0 / 6 (else `player::rolling` does).
    pub(crate) fn write_inputs(&mut self, m: &mut MixMap, s: &skate_audio::player::AudioState, own_routing: bool) {
        let owner = keys::skateboard(self.instance);
        let pulse = std::mem::take(&mut self.pulse_surface);
        if own_routing {
            m.set_input(owner, 0, if pulse { 32767 } else { 0 });
            m.set_input(owner, 6, if self.bound[0].is_some_and(|b| b.stem == "metal_smooth_hard") { 32767 } else { 0 });
        }
        m.set_input(owner, 4, if std::mem::take(&mut self.pulse_push) { 32767 } else { 0 });
        let tuning = self.primary_bound().and_then(|b| self.tunings.get(b.tuning));
        let (down, up) = match tuning {
            Some(t) => board::slope_levels(s.slope, true, t.slope_divisors.0, t.slope_divisors.1),
            None => (0.0, 0.0),
        };
        self.downhill = down;
        let word = |x: f32| ((x * 32767.0) as i32).clamp(0, 32767);
        m.set_input(owner, 2, word(down));
        m.set_input(owner, 3, word(up));
    }

    /// The decoded recording of a member (all are decoded in [`Bed::new`]).
    fn source(&mut self, library: &Library, name: &str) -> Option<Arc<GrainSource>> {
        if let Some(s) = self.sources.get(name) {
            return Some(s.clone());
        }
        if self.failed.contains(name) {
            return None;
        }
        let loaded = (|| -> Result<Arc<GrainSource>, String> {
            let (wav, raw) = library.grain_whole(name).ok_or("not in the install")?;
            let header = GrainFile::parse(&library.read(raw).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
            // skate_vita: the `.ima` shares sound.pak's bytes (was read, copied and decoded).
            let pcm = library.read_pcm(wav).ok_or("not a PCM16 WAV")?;
            if pcm.rate != header.stream.rate {
                return Err(format!("WAV rate {} ≠ stream rate {}", pcm.rate, header.stream.rate));
            }
            Ok(Arc::new(GrainSource { name: name.to_owned(), duration: header.duration, pcm: Arc::new(pcm) }))
        })();
        match loaded {
            Ok(s) => {
                self.sources.insert(name.to_owned(), s.clone());
                Some(s)
            }
            Err(e) => {
                warn!("Game audio: grain {name}: {e}");
                self.failed.insert(name.to_owned());
                None
            }
        }
    }
}

/// One game frame of the bed (after the MixMap tick): [`update`]'s body, callable headless.
/// `routed`: the owner routing's grain events since the last frame and its primary truck (native
/// rolling layers), or `None` for the bed's own one-truck routing.
#[allow(clippy::too_many_arguments)]
pub(super) fn step(
    bed: &mut Bed,
    library: &Library,
    m: &MixMap,
    shared: &std::sync::Mutex<skate_audio::runtime::Runtime>,
    r: &skate_sim::AudioRiding,
    dt: f32,
    seams: &skate_audio::player::tuning::PlayerTuning,
    routed: Option<(Vec<GrainEvent>, usize)>,
) {
    step_with(bed, library, m, r, dt, seams, routed, |apply| {
        if let Ok(mut runtime) = shared.lock() {
            apply(&mut runtime);
        }
    });
}

/// [`step`] with the runtime handed in by `with_runtime` (which calls the given closure with it,
/// or not at all when it can't get it): everything up to the runtime writes runs first, as in
/// [`step`]. The NPC skaters' host calls it with the runtime it already holds.
#[allow(clippy::too_many_arguments)]
pub(super) fn step_with(
    bed: &mut Bed,
    library: &Library,
    m: &MixMap,
    r: &skate_sim::AudioRiding,
    dt: f32,
    seams: &skate_audio::player::tuning::PlayerTuning,
    routed: Option<(Vec<GrainEvent>, usize)>,
    with_runtime: impl FnOnce(&mut dyn FnMut(&mut skate_audio::runtime::Runtime)),
) {
    // The local player's bed (instance 0) or an NPC skater's (module docs: what is local-only).
    let local = bed.instance == 0;
    let r = *r;
    let s = r.audio;
    let frames = (dt * 60.0).clamp(0.0, 6.0);
    let slew_calls = bed.slew_calls.take();
    let speed = r.speed.abs();

    // Surface routing: the wanted binding per truck (None = stopped).
    let mut wanted = bed.bound;
    match &routed {
        Some((events, primary)) => {
            bed.primary = *primary;
            for e in events {
                match *e {
                    GrainEvent::Bind { truck, surface, soft } => {
                        let m = rolling::member(surface, soft);
                        let tuning = if m.default_tuning { DEFAULT_TUNING } else { m.stem };
                        // Soft members the install lacks play their hard member (as before).
                        let stem = if bed.tunings.contains_key(m.stem) { m.stem } else { rolling::member(surface, false).stem };
                        let tuning = if bed.tunings.contains_key(tuning) { tuning } else { stem };
                        wanted[truck] = Some(Bound { stem, tuning });
                    }
                    GrainEvent::Stop { truck } => wanted[truck] = None,
                }
            }
        }
        None => {
            bed.primary = 0;
            wanted[0] = if r.grinding {
                None
            } else if r.wheels == 0 {
                bed.bound[0]
            } else {
                let stem = bed.member(r.surface, s.soft_wheels);
                Some(Bound { stem, tuning: stem })
            };
        }
    }
    let primary = wanted[bed.primary].or(wanted[1 - bed.primary]);
    let tuning = primary.and_then(|b| bed.tunings.get(b.tuning)).cloned();

    // Modulators.
    if bed.pushes_seen.is_some_and(|seen| seen != r.pushes) {
        bed.pulse_push = true;
        if let Some(t) = &tuning {
            let (scale, shift) = board::push_peaks(&t.push, speed);
            bed.push_scale.trigger(scale, 1.0, t.push.scale_ms);
            bed.shift.trigger(shift, t.push.shift_ms);
        }
    }
    bed.pushes_seen = Some(r.pushes);
    bed.push_scale.advance(dt);
    bed.shift.advance(dt);
    let special = bed.latches.update(s.balance, s.wheel_count, s.hippy_jump, s.feet_in_deck_box);
    // `sub_824C6198`: the turn intensity and the brake move by their step per call (no dt).
    let turn = bed.slews.step(tuning.as_ref(), s.com_speed(), s.turn, special, r.braking, slew_calls, frames);
    if local {
        let rng = &mut bed.rng;
        bed.seam.update(s.seam_pattern[0], dt, |p| seams.seam_wobble(p), || rng.draw());
        // The chain modulators share the title generator, after the seam draws (retail's frame order).
        let rng = &mut bed.rng;
        bed.chain.frame(&bed.chain_tuning, speed, dt, [wanted[0].is_some(), wanted[1].is_some()], || rng.draw());
    }

    let owner = keys::skateboard(bed.instance);
    let inputs = BoardInputs {
        speed,
        speed_scale: bed.push_scale.value(),
        level_a: m.level(owner, 1),
        level_b: m.level(owner, 2),
        pitch: m.pitch_4096(owner, 3),
        turn,
        brake: bed.slews.brake,
        special,
        downhill: bed.downhill,
        seam: if local { bed.seam.value() } else { None },
    };
    let truck_tuning = |b: Option<Bound>| b.and_then(|b| bed.tunings.get(b.tuning)).cloned();
    let tunings = [truck_tuning(wanted[0]), truck_tuning(wanted[1])];
    let records = tunings.clone().map(|t| t.map(|t| board::records(&t, &inputs)));
    let (push_shift, downhill) = (bed.shift.value(), bed.downhill);
    let frame = |t: &SurfaceTuning| ChainFrame {
        highpass_hz: m.filter_hz(owner, 12) as f32,
        lowpass_hz: m.filter_hz(owner, 11) as f32,
        pan_degrees: m.raw(owner, 0) as f32 * chain::DEGREES_PER_RAW,
        env_send: m.level(owner, 13) as f32 * chain::PER_LEVEL,
        flange_send: if local { [m.level(owner, 21) as f32 * chain::PER_LEVEL, m.level(owner, 22) as f32 * chain::PER_LEVEL] } else { [0.0; 2] },
        fss_hz: chain::fss_shifts(t, tuning.as_ref().unwrap_or(t), special, push_shift, downhill),
    };
    let sos = keys::sense_of_speed(0);
    let rocket_record = bed.rocket.map(|t| board::rocket_record(&t, speed, m.level(sos, 5), m.pitch_4096(sos, 3)));
    let sources: [Option<Arc<GrainSource>>; 2] = std::array::from_fn(|t| {
        if wanted[t] != bed.bound[t] { wanted[t].and_then(|b| bed.source(library, b.stem)) } else { None }
    });
    let rocket_source = match (bed.rocket, bed.rocket_on) {
        (Some(t), false) if speed * 3.6 > t.start_kmh => bed.source(library, ROCKET),
        _ => None,
    };

    let routed_none = routed.is_none();
    let mut apply = |runtime: &mut skate_audio::runtime::Runtime| {
        // The local bed, or the NPC skater's (created at its first use, without graph 3), whose picks
        // draw from the local bed's generator (retail's one title-wide generator).
        let (grains, mut shared_rng) = if local {
            (&mut runtime.grains, None)
        } else {
            let npc = runtime.npc_grains.get_or_insert_with(|| {
                let mut g = Box::new(skate_audio::grain::GrainBed::new());
                g.local = false;
                g
            });
            (&mut **npc, Some(&mut runtime.grains.rng))
        };
        if !bed.tuned {
            grains.chain_tuning = bed.chain_tuning;
            bed.tuned = true;
        }
        for t in 0..2 {
            if wanted[t] != bed.bound[t] {
                grains.stop_truck(t);
                if routed_none {
                    bed.pulse_surface = bed.bound[t].is_some();
                }
                bed.bound[t] = None;
                if let (Some(b), Some(source), Some(tu), Some(rec)) = (wanted[t], sources[t].clone(), &tunings[t], records[t]) {
                    if local {
                        info!("AUDIO_EVENT grain bind truck {t} {} ({} tuning, surface tag {})", b.stem, b.tuning, r.surface);
                    } else {
                        info!("AUDIO_NPC grain bind instance {} truck {t} {} ({} tuning)", bed.instance, b.stem, b.tuning);
                    }
                    bed.chain.rebuilt(t);
                    match shared_rng.as_deref_mut() {
                        Some(rng) => grains.share_rng(rng, |g| g.bind_truck(t, source, tu.params, rec)),
                        None => grains.bind_truck(t, source, tu.params, rec),
                    }
                    bed.bound[t] = Some(b);
                }
            } else if let Some(rec) = records[t] {
                grains.set_records(t, rec);
            }
            if let Some(tu) = &tunings[t] {
                grains.set_chains(t, bed.chain.values(t, &frame(tu)));
            }
        }
        if let (Some(t), Some(rec)) = (bed.rocket, rocket_record) {
            if bed.rocket_on && speed * 3.6 <= t.start_kmh {
                grains.stop_rocket();
                bed.rocket_on = false;
            } else if let Some(source) = rocket_source.clone() {
                grains.start_rocket(source, t.params, rec);
                bed.rocket_on = true;
            } else if bed.rocket_on {
                grains.rocket.record = rec;
            }
        }
    };
    with_runtime(&mut apply);
}

/// Stop an NPC skater's bed (its instance was released; `Runtime::npc_grains`).
pub(super) fn stop_npc(rt: &mut skate_audio::runtime::Runtime) {
    if let Some(g) = rt.npc_grains.as_deref_mut() {
        g.stop_truck(0);
        g.stop_truck(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rolling_grains_follow_the_retail_surface_map() {
        // Spot checks against the vault's AudioSurfaceMap (audio-specs/grain-player-spec.md §1.4).
        for (tag, grain) in [
            (2, "asphalt_rough_hard"), (66, "concrete_rough_hard"), (1, "asphalt_smooth_hard"),
            (55, "asphalt_smooth_hard"), (0, "asphalt_smooth_hard"), (100, "asphalt_smooth_hard"),
            (3, "concrete_smooth_hard"), (93, "concrete_smooth_hard"), (47, "wood_ramp_hard"),
            (94, "wood_ramp_hard"), (5, "concrete_aggregate_hard"), (84, "metal_smooth_hard"),
            (36, "metal_smooth_hard"), (130, "asphalt_rough_hard"), (6, "wood_ramp_hard"),
            (31, "metal_smooth_hard"), (127, "asphalt_smooth_hard"),
        ] {
            assert_eq!(grain_for(tag), grain, "tag {tag}");
        }
    }

    /// [`grain_for`] against the install's own `Sk8::AudioSurfaceMap` (when present): tag →
    /// material (tag − 1; 0 = none → 143 → surface 3) → rolling surface → grain member, for every
    /// surface that plays a grain (1–6, 9). And every member has its tuning and recording.
    #[test]
    #[ignore = "needs the private install data"]
    fn grain_for_matches_the_vault_surface_map_and_the_bed_loads() {
        let root = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets"));
        let Ok(library) = Library::load(root) else {
            panic!("missing private data: no audio install under {}", root.display());
        };
        let map = library.surface_map();
        if map.is_empty() {
            panic!("missing private data: the install has no grain tuning (stage_grain_mixmap.py)");
        }
        assert_eq!(map.len(), 95);
        for tag in 0..128u32 {
            let material = if tag == 0 { 143 } else { tag - 1 };
            let surface = if material == 143 { 3 } else { map[material.min(94) as usize] };
            let want = match surface {
                1 => "asphalt_rough_hard",
                2 => "concrete_rough_hard",
                3 => "asphalt_smooth_hard",
                4 => "concrete_smooth_hard",
                5 => "wood_ramp_hard",
                6 => "concrete_aggregate_hard",
                9 => "metal_smooth_hard",
                _ => continue, // Class_rolling surfaces and surface 0: stand-ins
            };
            assert_eq!(grain_for(tag), want, "tag {tag} (surface {surface})");
        }
        let bed = Bed::new(&library).expect("every member has its recording and tuning");
        assert_eq!(bed.tunings.len(), MEMBERS.len() + SOFT_MEMBERS.len() + 1, "hard and soft members and the `default` collection");
        assert!(bed.rocket.is_some_and(|r| r.start_kmh == 35.0 && r.gain_word == 22000));
        assert_eq!(bed.tunings["concrete_rough_hard"].max_kmh, 60.0);
        // Soft wheels pick the soft member where one exists; metal has only the hard one.
        assert_eq!(bed.member(4, true), "concrete_rough_soft");
        assert_eq!(bed.member(4, false), "concrete_rough_hard");
        assert_eq!(bed.member(9, true), "metal_smooth_hard");
    }
}
