//! The dynamic props' sounds (`skate_audio::world::props`, MixMap slot 8): the props' contact peaks, the five prop
//! SFX objects with their owner records (`sub_824B5558`), impact posts into the local player's collision manager and
//! the held rolling / sliding loops: docs/hails-additions/27-dynamic-props.md#prop-sounds
//! skate_vita: skate-game `game_audio/prop_sounds.rs` (PR #52 `3d14a7bb`) without Bevy: the two systems are
//! [`PropSounds::frame_pre`] and [`PropSounds::frame_post`], called by [`super::native::Native::frame`] around the
//! MixMap ticks; the props' samples come from the simulation thread through the audio mailbox
//! (`skate_sim::PropAudioSample`). The mods' per-type patches are not ported (no mods on the Vita), and a bank the
//! card has not delivered yet holds the loops back (a card read on the host thread would stall the pass).
use std::collections::HashMap;
use tracing::{info, warn};

use skate_audio::eval::NodeId;
use skate_audio::player::contacts::SpliceHost;
use skate_audio::player::{Outputs, Owner};
use skate_audio::player::objpos::Listener;
use skate_audio::splice::SoundId;
use skate_audio::world::keys;
use skate_audio::world::moveable::{self, EqManager, Judder, MoveInput, MovingType};
use skate_audio::world::owners::Positions;
use skate_audio::world::{Draw, Lcg};
use skate_audio::world::props::{self, ContactPeak, ImpactPoster, LOOP_BANKS, LoopCommand, LoopInput, PropEntry, PropLoop, PropSfxRecord};

use super::native::Native;
use skate_sim::{PropAudioSample, PropAudioType};

fn requested() -> bool {
    static ENV: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENV.get_or_init(|| !std::env::var("SKATE_PROP_SOUNDS").is_ok_and(|v| v == "0")) && ENABLED.load(std::sync::atomic::Ordering::Relaxed)
}

static ENABLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

/// skate_vita: `PROP_SOUNDS` lines on the device log (the tracing macros print nothing there): the first few of each
/// kind, so a silent prop can be traced to the step that stopped it.
fn trace(kind: usize, line: impl FnOnce() -> String) {
    static COUNTS: [std::sync::atomic::AtomicU32; 6] = [const { std::sync::atomic::AtomicU32::new(0) }; 6];
    if COUNTS[kind].fetch_add(1, std::sync::atomic::Ordering::Relaxed) < 12 {
        println!("PROP_SOUNDS {}", line());
    }
}

/// Whether the props' sounds are on (the setting and `SKATE_PROP_SOUNDS`): the simulation side skips its entries when off.
pub fn enabled() -> bool {
    requested()
}

/// The game's Prop sounds setting (World / Vita system settings): off silences the props' sounds and frees their
/// voices at the next frame. On by default.
pub fn set_enabled(on: bool) {
    ENABLED.store(on, std::sync::atomic::Ordering::Relaxed);
}

/// `0x820BD16C`: evaluations the owner stays "in contact" after the last contact, over the game time scale
/// (`sub_824B5558`: 15 / max(scale, 0.1)).
const CONTACT_HOLD: f64 = 15.0;

/// One prop SFX object (`sub_824F4108`'s pool entry) and its owner record.
struct Object {
    id: u64,
    poster: ImpactPoster,
    looping: PropLoop,
    node: Option<NodeId>,
    positions: Positions,
    /// Owner `+0x74`, `+0x54`, `+0xA0`, `+0xA4`.
    moved: bool,
    smoothed: f32,
    apart: u32,
    in_contact: bool,
    /// This pass's loop input (the update reads it after the ticks).
    input: Option<LoopInput>,
    /// The pushed-prop object (`sub_824E48D0`): its type, start / main loops, the bank it waits for, the one-shots.
    moving: Moving,
}

#[derive(Default)]
struct Moving {
    data: Option<std::sync::Arc<MovingType>>,
    sliding: bool,
    start: Option<(NodeId, [i32; moveable::START_WORDS])>,
    main: Option<(NodeId, Vec<i32>)>,
    /// The main bank requested and not yet started (obj +0x97).
    pending: Option<String>,
    /// The one-shot timer (s; obj +0x240, running once the stream bank is in) and the playing one-shot.
    oneshot_timer: Option<f32>,
    oneshot: Option<SoundId>,
    /// This pass's owner values for the update.
    input: Option<MoveInput>,
    /// Owner `+0x78`: 1 push, 2 pull (dot(prop velocity, the local skater's) < 0; retail's second vector, player
    /// +0x10, is not traced).
    mode: u8,
}

impl Moving {
    fn held(&self) -> bool {
        self.start.is_some() || self.main.is_some()
    }
}

impl Object {
    fn new(id: u64, g: usize) -> Self {
        Self {
            id,
            poster: ImpactPoster::default(),
            looping: PropLoop::default(),
            node: None,
            positions: Positions::new(&[keys::dynamic_pos(g as u32)]),
            moved: false,
            smoothed: 0.0,
            apart: 0,
            in_contact: false,
            input: None,
            moving: Moving::default(),
        }
    }
}

pub(crate) struct PropSounds {
    samples: Vec<PropAudioSample>,
    entries: Vec<PropEntry>,
    /// The ring of every prop in the table (reset when it leaves).
    peaks: HashMap<u64, ContactPeak>,
    /// The five instances of MixMap slot 8.
    objects: [Option<Object>; keys::DYNAMIC_INSTANCES],
    /// `Native::map_epoch` the held nodes belong to.
    epoch: Option<u64>,
    /// Loop banks missing from the install (warned once).
    banks_missing: bool,
    /// `Prefetch::clears` when the shared banks were last requested (a clear forgets them).
    prefetched: Option<u64>,
    /// The dynamic-object manager's current record key (`+0x38`, never cleared), its judder and generator.
    manager_key: u64,
    judder: Judder,
    eq: EqManager,
    /// The manager's mode (`+0x48`) and the FSS0 shift it last posted.
    mode: u8,
    shift: f32,
    rng: Lcg,
}

impl Default for PropSounds {
    fn default() -> Self {
        Self {
            samples: Vec::new(),
            entries: Vec::new(),
            peaks: HashMap::new(),
            objects: Default::default(),
            epoch: None,
            banks_missing: false,
            prefetched: None,
            manager_key: 0,
            judder: Judder::default(),
            eq: EqManager::default(),
            mode: 0,
            shift: 0.0,
            rng: Lcg(1),
        }
    }
}

/// Release the dropped objects' loops and deactivate their position blocks.
fn incoming_empty_hint(samples: &[PropAudioSample]) -> bool {
    !samples.iter().any(|s| s.audio.is_some() && (s.pushed || s.contact.is_some()))
}

fn drop_objects(native: &mut Native, dropped: Vec<Object>) {
    if dropped.is_empty() {
        return;
    }
    let l = Listener::default();
    let Native { mixmap, shared, .. } = native;
    let mut runtime = shared.lock().ok();
    for mut o in dropped {
        if let Some(rt) = runtime.as_deref_mut() {
            for node in [o.node.take(), o.moving.start.take().map(|s| s.0), o.moving.main.take().map(|m| m.0)].into_iter().flatten() {
                rt.release(node);
            }
            if let Some(sound) = o.moving.oneshot.take() {
                rt.splice_host().release(sound);
            }
        }
        if let Some(m) = mixmap.as_mut() {
            o.positions.deactivate(m, &l);
        }
    }
}

impl PropSounds {
    /// Drop every object (map change, props gone, mod off).
    fn clear(&mut self, native: &mut Native) {
        let dropped: Vec<Object> = self.objects.iter_mut().filter_map(Option::take).collect();
        drop_objects(native, dropped);
        self.peaks.clear();
    }
}

/// The owner writer `sub_824B5558` for one evaluation: moved state, smoothed speed, contact hold.
fn owner_step(o: &mut Object, e: &PropEntry, audio: &PropAudioType, contact: bool, other_pushed: bool, time_scale: f32) -> LoopInput {
    let speed = e.owner_speed();
    if e.pushed && props::LOOP_SPEED < speed {
        o.moved = true;
    } else if o.moved && speed <= props::LOOP_SPEED {
        o.moved = false;
    }
    if o.moved && other_pushed {
        o.moved = false;
    }
    match audio.primary {
        Some(r) if o.smoothed < speed => o.smoothed += r.looping.rise,
        _ => o.smoothed = speed,
    }
    o.smoothed = o.smoothed.clamp(0.0, speed);
    if contact {
        o.in_contact = true;
        o.apart = 0;
    } else {
        o.apart += 1;
        if f64::from(o.apart) >= (CONTACT_HOLD / f64::from(time_scale).max(0.1)).trunc() {
            o.in_contact = false;
        }
    }
    LoopInput {
        speed,
        word_speed: if o.moved { o.smoothed } else { speed },
        in_contact: o.in_contact,
        sliding: audio.sliding,
        moved: o.moved.then_some(audio.moved_levels[1]),
    }
}

/// The process half of the pass (with the world / NPC owners', before the MixMap ticks).
/// The start bank and loop banks every prop shares, queued on the prefetch worker from a map's first frame so the
/// first push or loop does not decode them on the game thread: docs/hails-additions/27-dynamic-props.md#prop-sound-banks
fn prefetch_shared_banks(prefetched: &mut Option<u64>, native: &mut Native, library: &crate::library::Library) {
    let clears = native.prefetch.clears();
    if *prefetched == Some(clears) {
        return;
    }
    *prefetched = Some(clears);
    for stem in std::iter::once(moveable::START_BANK).chain(LOOP_BANKS) {
        if !native.bank_loaded(stem) {
            match library.bank_source(stem) {
                Ok(source) => {
                    trace(3, || format!("bank {stem} requested"));
                    native.prefetch.request(source);
                }
                Err(error) => trace(3, || format!("bank {stem} not available: {error}")),
            }
        }
    }
}

impl PropSounds {
    /// The shared banks ready: loaded, or delivered by the prefetch worker (nothing here reads the card).
    fn shared_ready(native: &Native) -> bool {
        std::iter::once(moveable::START_BANK).chain(LOOP_BANKS).all(|stem| native.bank_loaded(stem) || native.prefetch.ready(stem))
    }

    /// The process half of the pass, after `mixmap_frame` and before the MixMap ticks. `samples` are the props'
    /// audio entries the simulation published since the last frame (replaced by this call's: it is the scratch
    /// buffer); `local` is the local skater's audio state.
    pub(crate) fn frame_pre(&mut self, native: &mut Native, library: &crate::library::Library, incoming: &mut Vec<PropAudioSample>, local: &skate_audio::player::AudioState) {
        let state = self;
        if state.epoch != Some(native.map_epoch) {
            state.epoch = Some(native.map_epoch);
            state.clear(native);
        }
        if !requested() {
            state.clear(native);
            incoming.clear();
            return;
        }
        // The shared start and loop banks (about 2.5 MB resident) are asked for when a prop first makes a sound that
        // needs them (a push, a contact), not at every map's start: the Vita's heap has no room to spare.
        if state.prefetched.is_some() || incoming.iter().any(|s| s.audio.is_some() && (s.pushed || s.contact.is_some())) {
            prefetch_shared_banks(&mut state.prefetched, native, library);
        }
        let calls = native.pending.map_or(0, |p| p.calls);
        if calls == 0 {
            return;
        }
        // The props' entries come from the simulation thread: each frame's, or none (a pause, a world without props).
        std::mem::swap(&mut state.samples, incoming);
        incoming.clear();
        if !incoming_empty_hint(&state.samples) {
            trace(0, || format!("{} props with sound data this frame (banks ready {})", state.samples.iter().filter(|s| s.audio.is_some()).count(), Self::shared_ready(native)));
        }
        if !Self::shared_ready(native) {
            trace(4, || {
                let stems: Vec<String> = std::iter::once(moveable::START_BANK).chain(LOOP_BANKS).map(|s| format!("{s}: loaded {} ready {} asked {}", native.bank_loaded(s), native.prefetch.ready(s), native.prefetch.requested(s))).collect();
                format!("banks wait: {}", stems.join("; "))
            });
            state.samples.clear();
        }
        if state.samples.is_empty() && state.objects.iter().all(Option::is_none) {
            state.peaks.clear();
            return;
        }
    let Some(player) = native.player.as_ref() else { return };
    let listener = player.last_listener();
    let collision = &player.tuning.collision;
    let mut messages = Vec::new();
    let mut commands: Vec<(usize, LoopCommand)> = Vec::new();
    let mut dropped = Vec::new();
    for call in 0..calls {
        state.entries.clear();
        for s in &state.samples {
            let (impact, contact_material) = state.peaks.entry(s.id).or_default().push(if call == 0 { s.contact } else { None });
            let position = if s.pushed { local.com_position } else { s.position };
            state.entries.push(PropEntry { id: s.id, position, speed: s.speed, spin: s.spin, pushed: s.pushed, impact, contact_material });
        }
        let picked = props::pick(&state.entries);
        for slot in &mut state.objects {
            if slot.as_ref().is_some_and(|o| !picked.contains(&o.id)) {
                dropped.extend(slot.take());
            }
        }
        for id in picked {
            if state.objects.iter().flatten().any(|o| o.id == id) {
                continue;
            }
            if let Some((g, slot)) = state.objects.iter_mut().enumerate().find(|(_, s)| s.is_none()) {
                *slot = Some(Object::new(id, g));
            }
        }
        for (g, slot) in state.objects.iter_mut().enumerate() {
            let Some(o) = slot.as_mut() else { continue };
            let (Some(e), Some(s)) = (state.entries.iter().find(|e| e.id == o.id), state.samples.iter().find(|s| s.id == o.id)) else { continue };
            let Some(audio) = s.audio.as_ref() else { continue };
            let other_pushed = state.entries.iter().any(|x| x.pushed && x.id != o.id);
            let input = owner_step(o, e, audio, call == 0 && s.contact.is_some(), other_pushed, local.time_scale);
            if let Some(rec) = audio.primary {
                let posted = o.poster.process(&rec, e, collision);
                if let Some(first) = posted.first() {
                    info!(
                        "PROP_IMPACT prop={} impact {:.2} m/s (min {:.2} max {:.2}) materials {:?} tiers {:?} levels {:?} posts {}",
                        o.id, e.impact, rec.impact_min, rec.impact_max, first.material, first.tier, first.level, posted.len()
                    );
                }
                if !posted.is_empty() {
                    trace(1, || format!("impact posts {} (contacts on {})", posted.len(), player.contacts_on));
                }
                messages.extend(posted);
            }
            // sub_824E38B8: the moved prop's loop plays its +0x88 record (none = no loop).
            let record = if o.moved { audio.moved } else { audio.primary };
            if let Some(cmd) = o.looping.process(&input, record.as_ref().map(|r| (r.key, &r.looping))) {
                commands.push((g, cmd));
            }
            o.input = Some(input);
            if call + 1 == calls {
                o.moving.data = audio.moving.clone();
                o.moving.sliding = audio.sliding;
                o.moving.input = Some(MoveInput {
                    speed: o.smoothed,
                    spin: e.spin,
                    level: audio.moved_levels[0],
                    b0: 0.0,
                    judder: 0,
                    second_pitch: 0,
                });
            }
        }
    }
    let samples = &state.samples;
    state.peaks.retain(|id, _| samples.iter().any(|s| s.id == *id));
    drop_objects(native, dropped);
    // MixMap inputs before the ticks: each object's position block and DynamicObjectPhysics in0.
    if let (Some(m), Some(l)) = (native.mixmap.as_mut(), listener.as_ref()) {
        for (g, slot) in state.objects.iter_mut().enumerate() {
            let Some(o) = slot.as_mut() else { continue };
            let Some(s) = state.samples.iter().find(|s| s.id == o.id) else { continue };
            let position = if s.pushed { local.com_position } else { s.position };
            o.positions.write(m, l, &[Some((position, s.velocity))]);
            let max = s.audio.as_ref().and_then(|a| a.primary).map_or(1.0, |r| r.looping.moved_max_kmh);
            let spin = if o.moved { ((s.spin * props::KMH / max).clamp(0.0, 1.0) * 32767.0) as i32 } else { 0 };
            m.set_input(keys::dynamic_physics(g as u32), 0, spin.clamp(0, 0x7FFF));
        }
    }
    // sub_824E48D0 once per pass: start loop on the first moved frame, the prop's main bank, then the main loop.
    let mut moving_posts: Vec<(usize, &'static str, Vec<i32>, bool)> = Vec::new();
    let mut moving_releases: Vec<NodeId> = Vec::new();
    for (g, slot) in state.objects.iter_mut().enumerate() {
        let Some(o) = slot.as_mut() else { continue };
        let Some(data) = o.moving.data.clone() else { continue };
        if !(o.moved && o.in_contact) {
            moving_releases.extend(o.moving.start.take().map(|s| s.0));
            moving_releases.extend(o.moving.main.take().map(|m| m.0));
            o.moving.pending = None;
            o.moving.oneshot_timer = None;
            continue;
        }
        let key = state.samples.iter().find(|s| s.id == o.id).and_then(|s| s.audio.as_ref()).and_then(|a| a.primary).map_or(0, |r| r.key);
        if key != state.manager_key {
            state.manager_key = key;
            state.judder = Judder::default();
            state.eq = EqManager::default();
            state.rng = Lcg((key as u32) ^ (o.id as u32));
        }
        // sub_824F4BD0: a mode change posts the record's push / pull shift to both graphs' FSS0.
        if let Some(s) = state.samples.iter().find(|s| s.id == o.id) {
            let v = local.com_velocity;
            let dot = s.velocity[0] * v[0] + s.velocity[1] * v[1] + s.velocity[2] * v[2];
            o.moving.mode = if dot < 0.0 { 2 } else { 1 };
        }
        if o.moving.mode != state.mode {
            state.mode = o.moving.mode;
            state.shift = data.push_pull[usize::from(state.mode - 1)];
        }
        if !o.moving.held() && o.moving.pending.is_none() {
            let class = if o.moving.sliding { moveable::SLIDING_START } else { moveable::ROLLING_START };
            moving_posts.push((g, class, moveable::start_packet(&data.record).to_vec(), true));
            // skate_vita: only a bank that is not loaded yet (the worker would read and decode a second copy that nothing
            // takes, about 1 MB a bank: the 11 Nov 2026 crash on `audio-bank-prefetch`).
            if !native.bank_loaded(&data.record.bank)
                && let Ok(source) = library.bank_source(&data.record.bank)
            {
                native.prefetch.request(source);
            }
            o.moving.pending = Some(data.record.bank.clone());
            info!("PROP_MOVE prop={} start {class} patch {} bank {}", o.id, data.record.patch, data.record.bank);
        }
        if let Some(bank) = o.moving.pending.clone()
            && (native.bank_loaded(&bank) || native.prefetch.ready(&bank))
        {
            o.moving.pending = None;
            match native.ensure_bank(&library, &bank) {
                Ok(_) => {
                    let class = if o.moving.sliding { moveable::SLIDING_MAIN } else { moveable::ROLLING_MAIN };
                    let speed = o.moving.input.map_or(0, |i| {
                        let v = i.speed * props::KMH / data.record.max_kmh;
                        (v.clamp(0.0, 1.0) * 1000.0) as i32
                    });
                    moving_posts.push((g, class, moveable::main_packet(o.moving.sliding, speed, 0.0, &data.record), false));
                    o.moving.oneshot_timer = data.oneshot.as_ref().map(|r| moveable::oneshot_interval(r, o.smoothed, &mut || state.rng.draw()));
                    info!("PROP_MOVE prop={} main {class} bank {bank}", o.id);
                }
                Err(error) => warn!("PROP_MOVE prop={} bank {bank}: {error}", o.id),
            }
        }
    }
    if moving_posts.iter().any(|p| p.3) {
        if let Err(error) = native.ensure_bank(&library, moveable::START_BANK)
            && !state.banks_missing
        {
            warn!("PROP_SOUNDS start bank {}: {error}", moveable::START_BANK);
            state.banks_missing = true;
        }
    }
    if !moving_posts.is_empty() || !moving_releases.is_empty() {
        let Ok(mut runtime) = native.shared.lock() else { return };
        for node in moving_releases.drain(..) {
            runtime.release(node);
        }
        for (g, class, words, start) in moving_posts.drain(..) {
            let (Some(class_id), Some(o)) = (runtime.eval.class_id(class), state.objects[g].as_mut()) else { continue };
            let node = runtime.post(class_id, &words);
            if start {
                o.moving.start = Some((node, std::array::from_fn(|i| words[i])));
            } else {
                o.moving.main = Some((node, words));
            }
        }
    }
    if commands.iter().any(|(_, c)| matches!(c, LoopCommand::Post { .. })) {
        for stem in LOOP_BANKS {
            if let Err(error) = native.ensure_bank(&library, stem)
                && !state.banks_missing
            {
                warn!("PROP_SOUNDS loop bank {stem}: {error}");
                state.banks_missing = true;
            }
        }
    }
    if messages.is_empty() && commands.is_empty() {
        return;
    }
    let Ok(mut runtime) = native.shared.lock() else { return };
    for (g, cmd) in commands {
        let Some(o) = state.objects[g].as_mut() else { continue };
        match cmd {
            LoopCommand::Post { class, words } => {
                let Some(class_id) = runtime.eval.class_id(class) else { continue };
                if let Some(old) = o.node.take() {
                    runtime.release(old);
                }
                o.node = Some(runtime.post(class_id, &words));
                trace(2, || format!("loop start prop {} class {class}", o.id));
                info!("PROP_LOOP prop={} start {class} patch {} speed word {}", o.id, words[8], words[3]);
            }
            LoopCommand::Release => {
                if let Some(node) = o.node.take() {
                    runtime.release(node);
                }
            }
        }
    }
    if !messages.is_empty()
        && let Some(player) = native.player.as_mut()
    {
        player.post_collisions(messages, &mut runtime);
    }
}

}

/// The update half after the MixMap ticks (`sub_824E2940`, `sub_824E5F18`): every held loop's words from its
/// Dynamic / Moving outputs, the manager's judder, the pushed prop's one-shots.
pub(crate) fn frame_post(state: &mut PropSounds, native: &mut Native) {
    if state.epoch != Some(native.map_epoch) {
        return;
    }
    let Some(pass) = native.pending else { return };
    let dt = skate_audio::mixmap::cadence::CONSOLE_DT * pass.calls.min(4) as f32;
    let Some(m) = native.mixmap.as_ref() else { return };
    let Ok(mut runtime) = native.shared.lock() else { return };
    for (g, slot) in state.objects.iter_mut().enumerate() {
        let Some(o) = slot.as_mut() else { continue };
        if let (Some(node), Some(input)) = (o.node, o.input)
            && let Some(words) = o.looping.update(&input, &Owner { mixmap: m, key: keys::dynamic(g as u32) })
        {
            runtime.redeliver(node, &words);
        }
        let Some(data) = o.moving.data.clone() else { continue };
        let Some(mut input) = o.moving.input else { continue };
        if !o.moving.held() {
            continue;
        }
        // The manager (one per game, the moved prop's record): judder per tick, its pitch term.
        state.judder.tick(&data.judder, dt, input.speed * props::KMH, &mut || state.rng.draw());
        input.judder = state.judder.value;
        input.second_pitch = data.judder.pitch;
        let out = Owner { mixmap: m, key: keys::moving(g as u32) };
        // sub_824F4540: the two bus graphs' EQs and gains, the pan = the routed voices' azimuth (the main loop's w2).
        let angle = o.moving.main.as_ref().map_or(0.0, |(_, w)| w[1] as f32 * (360.0 / 65536.0));
        let params = state.eq.tick(&data.manager, dt, input.speed * props::KMH, angle, state.shift, &mut || state.rng.draw());
        for (i, p) in params.iter().enumerate() {
            runtime.mixer.buses.dynamic.set(i, p);
        }
        if let Some((node, words)) = o.moving.start.as_mut() {
            moveable::start_update(words, o.moving.sliding, &data.record, &out);
            runtime.redeliver(*node, &words[..]);
        }
        if let Some((node, words)) = o.moving.main.as_mut() {
            moveable::main_update(words, o.moving.sliding, &data.record, &input, &out);
            runtime.redeliver(*node, words);
        }
        // sub_824E6D60: a random one-shot from the stream bank when its timer runs out, updated while it plays.
        let Some(rec) = data.oneshot.as_ref() else { continue };
        let block = |out: &Owner| {
            let gain = out.level(12) as f32 / 32767.0 * rec.level;
            let pitch = out.pitch(13) as f32 / 4096.0 * rec.pitch;
            [gain, pitch, out.raw(6) as f32 * (180.0 / 32767.0), dt, 0.0, 1.0]
        };
        let mut host = runtime.splice_host();
        if let Some(sound) = o.moving.oneshot {
            if host.alive(sound) {
                host.update(sound, block(&out));
                continue;
            }
            host.release(sound);
            o.moving.oneshot = None;
            o.moving.oneshot_timer = Some(moveable::oneshot_interval(rec, input.speed, &mut || state.rng.draw()));
        }
        let Some(timer) = o.moving.oneshot_timer.as_mut() else { continue };
        *timer -= dt;
        if *timer > 0.0 {
            continue;
        }
        if rec.id >= 0 && (state.rng.draw() % 100) < rec.chance as u32 {
            o.moving.oneshot = host.start(&rec.bank, rec.id as u32, block(&out));
        }
        if o.moving.oneshot.is_none() {
            o.moving.oneshot_timer = Some(moveable::oneshot_interval(rec, input.speed, &mut || state.rng.draw()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(speed: f32, pushed: bool) -> PropEntry {
        PropEntry { id: 1, position: [0.0; 3], speed, spin: 0.0, pushed, impact: 0.0, contact_material: 143 }
    }

    fn audio() -> PropAudioType {
        let mut r = PropSfxRecord { key: 5, ..PropSfxRecord::default() };
        r.looping.rise = 0.5;
        PropAudioType { primary: Some(r), moved: None, sliding: true, moved_levels: [1.0, 0.25], moving: None }
    }

    #[test]
    fn the_owner_holds_contact_for_15_evaluations_at_normal_speed() {
        let mut o = Object::new(1, 0);
        assert!(owner_step(&mut o, &entry(2.0, false), &audio(), true, false, 1.0).in_contact);
        for _ in 0..14 {
            assert!(owner_step(&mut o, &entry(2.0, false), &audio(), false, false, 1.0).in_contact);
        }
        assert!(!owner_step(&mut o, &entry(2.0, false), &audio(), false, false, 1.0).in_contact, "the 15th evaluation apart");
        let mut slow = Object::new(1, 0);
        owner_step(&mut slow, &entry(2.0, false), &audio(), true, false, 0.5);
        for _ in 0..29 {
            assert!(owner_step(&mut slow, &entry(2.0, false), &audio(), false, false, 0.5).in_contact, "slow motion holds longer");
        }
    }

    #[test]
    fn a_pushed_prop_is_moved_with_a_rising_speed_until_another_is_pushed() {
        let mut o = Object::new(1, 0);
        let i = owner_step(&mut o, &entry(2.0, true), &audio(), true, false, 1.0);
        assert_eq!((i.moved, i.word_speed), (Some(0.25), 0.5), "moved: the loop speed rises by the record's 0.5 per evaluation");
        assert_eq!(owner_step(&mut o, &entry(2.0, false), &audio(), true, false, 1.0).word_speed, 1.0, "still moved after the push while moving");
        assert_eq!(owner_step(&mut o, &entry(2.0, false), &audio(), true, true, 1.0).moved, None, "another pushed prop ends it");
    }
}
