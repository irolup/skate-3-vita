//! The skater's audio inputs. `observe` runs after every physics tick and publishes the retail
//! audio-state record (`skate_audio::player::AudioState`, [`audio_state`]) and the board state the
//! native runtime reads ([`Riding`]: the granular bed, the player components, the emitters' and
//! ambience's listener position), writes the state log, and logs the animation's push / brake
//! events and grind starts / stops as `AUDIO_EVENT` lines. Every sound is the native runtime's
//! (2026-10-03: the interim cue tables and their `play` system are gone; doc 11).
use crate::physics::{GamePhysics, SkaterRuntime};
use glam::Vec3;
use tracing::info;
use skate_core::{
    physics::{board::BodyId, filtered_state::FilteredCategory},
    player::state::PhysicalStateId,
};

/// Foot braking (the bed's brake input, `grain_bed.rs`): the board must move this fast (m/s). The
/// foot-down brake event repeats every tick while the foot brakes (then foot-up repeats while it
/// lifts), so braking ends this long (s) after the last foot-down.
const DRAG_MIN_SPEED: f32 = 0.5;
const BRAKE_TIMEOUT: f32 = 0.1;

/// Continuous state sampled at the last physics tick.
#[derive(Clone, Copy, Debug, Default)]
pub struct Riding {
    pub board: Vec3,
    pub speed: f32,
    /// Majority audio surface tag under the wheels in contact (0 with none).
    pub surface: u32,
    pub grinding: bool,
    pub braking: bool,
    /// Wheels in contact (0..4) and pushes since start (the native rolling bed reads these).
    pub wheels: u32,
    pub pushes: u32,
    /// The retail audio-state view of this tick for the native runtime (`skate_audio::player`).
    pub audio: skate_audio::player::AudioState,
    /// The loose-board state's inputs (`PlayerAudio::loose_board`, conditioner `sub_827A1B78`):
    /// `up_dot` = SkateboardReckoning+80 (the deck's physical up) · Ground+80 (the retained
    /// wheel-contact normal, Board Fill82C03318); the deck contact Collision+3475 and its material
    /// Collision+12 − 1 (read raw: the riderless-board gate of the record's wheel fields does not
    /// apply to these).
    pub deck_up: f32,
    pub deck_contact: bool,
    pub deck_material: u32,
    /// The wheel positions of the physics step before this one (this step's on the first): with
    /// `audio.wheel_position` the pair the rendered board interpolates between, which Class_Seams
    /// reads (`PlayerAudio::step_wheels`).
    pub wheels_before: [[f32; 3]; 4],
}

/// What `observe` publishes for the audio host (`native::mixmap_frame`).
///
/// The host's clock is the physics steps (2026-10-03, PR #32 review): `observe` runs once per
/// physics step and counts the steps since the host last took them ([`Cues::take_steps`]); the
/// host runs inputs / process / tick / update only on frames that took steps, as many ticks as
/// steps (capped), and none while nothing steps (menu pause). The one-step pulses of the steps
/// not yet taken are OR-latched into the published sample ([`latch_pulses`]), so a frame that takes
/// two steps still sees the first one's pulse and a frame without a step re-processes nothing.
#[derive(Default)]
pub(crate) struct Cues {
    pub riding: Riding,
    /// Physics steps published since the host last took them.
    pub steps: u32,
}

impl Cues {
    /// One physics step's sample: latched onto the pending one while the host has not taken it.
    pub(super) fn publish(&mut self, mut next: Riding) {
        if self.steps > 0 {
            latch_pulses(&self.riding, &mut next);
        }
        self.riding = next;
        self.steps = self.steps.saturating_add(1);
    }

    /// The steps since the last call (the host's ticks for this frame); the latched pulses are
    /// consumed with them (the next [`Cues::publish`] starts fresh).
    pub(super) fn take_steps(&mut self) -> u32 {
        std::mem::take(&mut self.steps)
    }
}

/// The audio state's one-step pulses, OR-ed over the steps a host pass takes at once: `+335` (the
/// push plant's rise, `push_trigger`). Every other field is a level (the latest step's value is
/// the one retail's per-frame audio manager reads too), a latch (`+228`, `+468`, `+192` / `+692`),
/// a 4-frame max (`+668`, `+496..`) or a counter (`Riding::pushes`), and the components detect
/// their own edges (landing, bail, grind) against the last sample they processed.
pub(super) fn latch_pulses(pending: &Riding, next: &mut Riding) {
    next.audio.push_trigger |= pending.audio.push_trigger;
}

/// `observe`'s own memory: the local player's [`SkaterAudioMemory`] plus what only the local
/// player's logs and the bed's brake input need.
#[derive(Default)]
pub(crate) struct Seen {
    /// The audio-state builder's per-skater memory.
    skater: SkaterAudioMemory,
    /// Foot braking, time since the last foot-down event, and the last event (+1/-1/0).
    braking: bool,
    brake_time: f32,
    brake_event: i32,
    on_rail: bool,
    trace: Option<bool>,
    /// Rows written to the audio state log.
    log_frame: u64,
}

/// The per-skater memory the audio-state builder ([`skater_audio_state`]) keeps between physics
/// steps: the latches, rings and edges retail's bridge (`sub_824B0DA8`) and conditioner keep per
/// skater. One per simulated skater (the local player's lives in `observe`; an AI-skater system
/// keeps one per skater it fills an `AudioState` for).
#[derive(Clone, Debug, Default)]
pub(crate) struct SkaterAudioMemory {
    started: bool,
    /// Last tick's animation push contact.
    push: bool,
    /// Last tick's audio-state push plant (`+333 || +334`, see [`push_plant`]).
    planted: bool,
    /// Seconds in the current air phase (audio state `+236`).
    air_time: f32,
    /// Latched grind family / material (`+192` / `+692`).
    grind_family: Option<i32>,
    grind_material: Option<u32>,
    /// `+468` (written while Air440 holds).
    jump_velocity: f32,
    /// `+228` the last positive grind impact, and the conditioner's 4-frame deck-impact ring
    /// (`+668`).
    grind_impact: f32,
    deck_ring: [f32; 4],
    deck_at: usize,
    /// The conditioner's 4-frame ring of the body regions' impacts (`sub_82773298`, `+640`).
    region_ring: [[f32; 6]; 4],
    region_at: usize,
    /// The bridge's `+212` (|COM v|) of the last tick: this tick's `+216`, the speed graph's input
    /// (`sub_824B0DA8`).
    com_speed_212: f32,
    /// The conditioner's step code (`+740`, `sub_827729B8`).
    step_code: skate_audio::player::footsteps::StepCode,
    offboard_hold: skate_audio::player::bridge::OffboardHold,
}

fn vec(v: skate_core::math::Vector3) -> Vec3 {
    Vec3::new(v.x, v.y, v.z)
}

fn grinding(state: u32) -> bool {
    (PhysicalStateId::GrindBoardslide as u32..=PhysicalStateId::GrindDarkslide as u32).contains(&state)
}

/// Majority audio surface under the wheels that touch the ground.
/// The skater is off the board or wiping out (Wipeout 300, Offboard 500 / 501 / 502): the riding
/// board's contacts are not the player's. Retail's audio record reports no wheels (material 143)
/// through such stretches — clean GREC sessions show it moving at speed with wheels 0, air 0 and
/// material 143 for whole stretches, never flickering — while our riding ground state keeps
/// reporting the riderless board's touches (the 19:29 listening test: a bed and skid burst when
/// the dropped board touched down after a trick). UNCERTAIN: the record writer itself (+152 bits
/// 20–22) is not located; this is the measured behaviour.
pub(super) fn board_unridden(state: u32) -> bool {
    matches!(state, 300 | 500..=502)
}

fn wheel_surface(physics: &GamePhysics) -> (u32, u32) {
    let ground = &physics.riding.ground;
    let lines = &physics.riding.wheel_lines;
    let touching: Vec<u32> = (0..4).filter(|&i| ground.parts[i].in_contact).map(|i| lines.audio_surfaces[i]).collect();
    let surface = touching.iter().copied().max_by_key(|s| touching.iter().filter(|t| *t == s).count());
    (touching.len() as u32, surface.unwrap_or(0))
}

/// What one builder step gives besides the record (for `observe`'s publish and logs).
struct Built {
    state: skate_audio::player::AudioState,
    /// The physical state, wheels down, majority surface tag, on a rail.
    state_id: u32,
    wheels: u32,
    surface: u32,
    on_rail: bool,
    /// The plant's rise (`+335`) and the animation's push contact edge (the state log's `push`).
    plant_edge: bool,
    push_edge: bool,
    /// State55 (the foot plant flag, the state log's `plant`).
    state55: bool,
    board: Vec3,
    unridden: bool,
}

/// The retail audio-state record of one skater after a physics step: the only correct way to fill
/// an [`skate_audio::player::AudioState`] from a `GamePhysics` / `SkaterRuntime` pair (air time,
/// grind family / material latch, the deck and region rings, the step code, the push plant, `+216`).
/// The local player's `observe` publishes exactly this; an AI-skater system that simulates its
/// skaters with the player's physics calls it once per physics step per skater (only for skaters
/// near the camera, see `crate::world_audio`), each with its own `memory`. `dt` = the step's
/// seconds. Reads the animation's latched push contact without taking it.
#[cfg_attr(not(test), allow(dead_code))] // the hook for a future AI-skater system (doc 11)
pub(crate) fn skater_audio_state(physics: &GamePhysics, skater: &SkaterRuntime, memory: &mut SkaterAudioMemory, dt: f32) -> skate_audio::player::AudioState {
    build(physics, skater, memory, dt, skater.animation_input.audio.push).state
}

fn build(physics: &GamePhysics, skater: &SkaterRuntime, memory: &mut SkaterAudioMemory, dt: f32, push: bool) -> Built {
    let state = skater.player_state.current() as u32;
    let deck = physics.board.bodies()[BodyId::Deck.index()].rates;
    let board = vec(deck.position);
    let grinds = &skater.player_input.physical.grinds;
    let on_rail = (grinds.grinding_316 != 0 || grinding(state)) && grinds.leaving_317 == 0;
    let unridden = board_unridden(state);
    let (wheels, surface) = if unridden { (0, 0) } else { wheel_surface(physics) };
    // The animation's push contact edge (the state log's `push` column).
    let push_edge = memory.started && push && !memory.push;
    let state55 = skater.player_state.state_flags.get(55 - 52).copied().unwrap_or(false);
    let (planted, plant_edge) = push_plant(state55, memory.planted);
    let plant_edge = memory.started && plant_edge;
    let m = &mut *memory;
    let mut audio_state = audio_state(physics, skater, AudioFrame {
        dt,
        state,
        unridden,
        wheels,
        board: (board, vec(deck.linear_velocity)),
        on_rail,
        push: planted,
        push_trigger: plant_edge,
        air_time: &mut m.air_time,
        grind_family: &mut m.grind_family,
        grind_material: &mut m.grind_material,
        jump_velocity: &mut m.jump_velocity,
        grind_impact: &mut m.grind_impact,
        deck_ring: &mut m.deck_ring,
        deck_at: &mut m.deck_at,
        region_ring: &mut m.region_ring,
        region_at: &mut m.region_at,
        step_code: &mut m.step_code,
        slip: if wheels > 0 {
            let ri = physics.board.part_transforms()[BodyId::Deck.index()].basis.columns[0];
            skate_audio::player::state::slip(vec(deck.linear_velocity).dot(Vec3::from_array(ri)))
        } else {
            0.0
        },
        deck_spin: {
            let at = physics.board.part_transforms()[BodyId::Deck.index()].basis.columns[2];
            vec(deck.angular_velocity).dot(Vec3::from_array(at))
        },
        deck_spin_xy: {
            let basis = physics.board.part_transforms()[BodyId::Deck.index()].basis.columns;
            let w = vec(deck.angular_velocity);
            [w.dot(Vec3::from_array(basis[0])), w.dot(Vec3::from_array(basis[1]))]
        },
    });
    // `+216` = last tick's `+212`; the body poster applies the graph at it (`player::contacts`).
    //82DB6EC0 copies Processed2480bit18 into OffBoard309;827A1B78 packs record164bit0.
    //824B0DA8 applies the per-skater hold clock before advancing it by dt.
    audio_state.offboard_310 = memory.offboard_hold.tick(
        skater.player_input.processed.flags_2480 & (1 << 18) != 0,
        audio_state.ground_speed,
        dt,
    );
    audio_state.com_speed_216 = memory.com_speed_212;
    memory.com_speed_212 = audio_state.com_speed();
    memory.started = true;
    memory.push = push;
    memory.planted = planted;
    Built { state: audio_state, state_id: state, wheels, surface, on_rail, plant_edge, push_edge, state55, board, unridden }
}

/// skate_vita: the reference's Bevy system as a plain function, run after every physics tick;
/// `dt` is that tick's seconds (the system read `Time::delta_secs`).
pub(crate) fn observe(
    physics: &GamePhysics,
    skater: &mut SkaterRuntime,
    cues: &mut Cues,
    seen: &mut Seen,
    dt: f32,
) {
    // Animation events latched during the tick (physics never reads these).
    let audio = std::mem::take(&mut skater.animation_input.audio);
    let p = &skater.player_input.physical;
    let filtered = p.filtered_state_0;
    let state = skater.player_state.current() as u32;
    let deck = physics.board.bodies()[BodyId::Deck.index()].rates;
    let board = vec(deck.position);
    let ground = FilteredCategory::Ground as u32;
    let trace = *seen.trace.get_or_insert_with(|| std::env::var_os("SKATE_AUDIO_TRACE").is_some_and(|v| v != "0"));
    if trace && (audio.footstep > 0.0 || audio.push || audio.brake_down || audio.brake_up) {
        info!("AUDIO_TRACE footstep={:.3} push={} brake_down={} brake_up={} state={state}",
            audio.footstep, audio.push, audio.brake_down, audio.brake_up);
    }
    // Foot braking lasts from the foot-down brake event to the foot-up one,
    // only while riding on the ground (a safety timeout covers a missed foot-up).
    let speed = physics.riding.motion.ground_speed.abs();
    // Logged on change only (the events repeat every tick).
    let brake_event = if audio.brake_down { 1 } else if audio.brake_up { -1 } else { 0 };
    if brake_event != 0 && brake_event != seen.brake_event {
        info!("AUDIO_EVENT brake {} speed={speed:.2} state={state}", if brake_event > 0 { "down" } else { "up" });
    }
    if audio.push && !seen.skater.push {
        info!("AUDIO_EVENT push speed={speed:.2} state={state}");
    }
    let mut brake_time = if audio.brake_down { 0.0 } else { seen.brake_time + dt };
    let mut braking = (seen.braking || audio.brake_down) && !audio.brake_up;
    if !matches!(state, 100 | 101) || filtered != ground || brake_time > BRAKE_TIMEOUT {
        braking = false;
        brake_time = BRAKE_TIMEOUT;
    }
    // Grind: the selector's grinding flag can lead the named grind state, and
    // `leaving` marks coming off before the state changes.
    let grinds = &skater.player_input.physical.grinds;
    let on_rail = (grinds.grinding_316 != 0 || grinding(state)) && grinds.leaving_317 == 0;
    if on_rail != seen.on_rail {
        info!("AUDIO_EVENT grind {} surface={} ledge={} flag={} leaving={} state={state}", if on_rail { "start" } else { "stop" },
            grinds.audio_surface_216, grinds.is_ledge_320, grinds.grinding_316, grinds.leaving_317);
    }
    let started = seen.skater.started;
    let built = build(&physics, &skater, &mut seen.skater, dt, audio.push);
    debug_assert_eq!((built.state_id, built.on_rail, built.board), (state, on_rail, board));
    let Built { state: audio_state, wheels, surface, plant_edge, .. } = built;
    // skate_vita: the reference writes its state log here (diagnostics, not copied).
    // The bed's push envelopes (`grain_bed.rs` `pushes_seen`) count the same edge as `+335`.
    let pushes = cues.riding.pushes.wrapping_add(u32::from(plant_edge));
    let wheels_before = if started { cues.riding.audio.wheel_position } else { audio_state.wheel_position };
    cues.publish(Riding {
        board,
        speed: physics.riding.motion.ground_speed,
        surface,
        grinding: on_rail,
        braking: braking && speed > DRAG_MIN_SPEED,
        wheels,
        pushes,
        audio: audio_state,
        deck_up: deck_up(&physics, &skater),
        deck_contact: skater.player_input.physical.collision.flag_3475 != 0,
        deck_material: skate_audio::player::state::material_of_tag(physics.riding.ground.part_audio_surfaces[2]),
        wheels_before,
    });
    seen.braking = braking;
    seen.brake_time = brake_time;
    seen.brake_event = brake_event;
    seen.on_rail = on_rail;
}

/// The audio state's `+236` (Air+176, the time in the air *state*) after this tick. It counts
/// while the air state (200..300) holds, so it keeps counting on the landing tick (wheels down,
/// `+332` already clear, the state still air) and drops to 0 with the state — the recomp's TREAT
/// capture (session all_20261002_223306: last air tick 0.750 s, landing tick 0.767 s, then 0; the
/// same one-tick hold as `+240`).
pub(super) fn air_time_236(previous: f32, state: u32, dt: f32) -> f32 {
    if (200..300).contains(&state) { previous + dt } else { 0.0 }
}

/// The audio state's push plant `+333 || +334` and its edge `+335` (the bridge `sub_824B0DA8`):
/// `+335` = State55 (record +148 `0x40000000`, the foot plant) rising against last frame's
/// `+333` / `+334`; `+333` = State55 && !`+338` && State56, `+334` = State55 && `+338`. `+338`
/// (record +160 bit 30) is not mapped yet, so the plant is State55 itself (session review
/// 2026-10-03 #1). Every push sound but the stroke foley reads these: the rattle and the bed's
/// push envelopes (`sub_824C6198`), the clothing plant, the Contacts' plant / lift. Retail plants
/// land +548 ms after the stroke (`+337`); the animation's push contact fires near the stroke.
/// Returns (planted, edge) from this tick's State55 and last tick's plant.
pub(super) fn push_plant(state55: bool, was_planted: bool) -> (bool, bool) {
    (state55, state55 && !was_planted)
}

/// What [`audio_state`] needs beyond the physics resources.
struct AudioFrame<'a> {
    dt: f32,
    state: u32,
    /// [`board_unridden`]: no board contacts reach the record.
    unridden: bool,
    wheels: u32,
    board: (Vec3, Vec3),
    on_rail: bool,
    /// `+333 || +334` and `+335` ([`push_plant`]).
    push: bool,
    push_trigger: bool,
    air_time: &'a mut f32,
    grind_family: &'a mut Option<i32>,
    grind_material: &'a mut Option<u32>,
    jump_velocity: &'a mut f32,
    grind_impact: &'a mut f32,
    deck_ring: &'a mut [f32; 4],
    deck_at: &'a mut usize,
    region_ring: &'a mut [[f32; 6]; 4],
    region_at: &'a mut usize,
    step_code: &'a mut skate_audio::player::footsteps::StepCode,
    /// `+232` from the deck's lateral speed (0 with no wheel down).
    slip: f32,
    /// `+488` the deck's angular velocity about its At axis.
    deck_spin: f32,
    /// `+480` / `+484` the same about its Ri and Up axes.
    deck_spin_xy: [f32; 2],
}

fn raw3(v: [u32; 4]) -> [f32; 3] {
    [f32::from_bits(v[0]), f32::from_bits(v[1]), f32::from_bits(v[2])]
}

/// The loose-board `up_dot` (`sub_827A1B78` at 0x827A2A60): SkateboardReckoning+80 (Y of
/// GetEffectiveTransform82C01BF8 = the physical deck's up; the stance flip only negates X / Z) ·
/// Ground+80 (the retained wheel-contact normal), three lanes.
fn deck_up(physics: &GamePhysics, skater: &SkaterRuntime) -> f32 {
    let up = physics.board.part_transforms()[BodyId::Deck.index()].basis.columns[1];
    let n = raw3(skater.player_input.physical.ground.vector_80);
    up[0] * n[0] + up[1] * n[1] + up[2] * n[2]
}

/// physics_collision `default` `+164` (field `1430BD50F0A33475`): the region impact scale.
const REGION_IMPACT_SCALE: f32 = 10.0;
/// The image's floor of a region impact (`0x82063A48`).
const REGION_IMPACT_FLOOR: f32 = f32::from_bits(0x3A83_126F);

/// The six body regions' impacts as `sub_82BD60C8` publishes them into Collision `+80..+100`
/// (audio state `+496..+516` after the conditioner's 4-frame max): per region with a contact
/// part, clamp01(max(0.001, (SkeletonState4048 Δv of the part · the region's contact normal) ×
/// the part's mass (SkeletonState4560: the bone-box volume) × 10)); 0 without a part. Regions 6
/// and 7 (the feet) are not read by the audio code.
pub(super) fn region_impacts(
    regions: &[skate_core::physics::skeleton_body::ContactRegion; 8],
    velocity_changes: &[[f32; 4]],
    masses: &[f32; 24],
) -> [f32; 6] {
    std::array::from_fn(|i| {
        let r = &regions[i];
        let Some(part) = r.part else { return 0.0 };
        let dv = velocity_changes.get(part).copied().unwrap_or([0.0; 4]);
        let n = r.normal;
        // |Δv·n|: retail masks the dot product's sign bit (`vandc128` with `v63 << 31` after the
        // `vmsum3fp128`), so a part pulled away from the surface counts as much as one stopped by it.
        let along = (dv[0] * n[0] + dv[1] * n[1] + dv[2] * n[2]).abs();
        let x = along * masses.get(part).copied().unwrap_or(0.0) * REGION_IMPACT_SCALE;
        let x = if REGION_IMPACT_FLOOR - x >= 0.0 { REGION_IMPACT_FLOOR } else { x };
        skate_audio::player::clamp01(x)
    })
}

/// |v| over three lanes (retail: x · the refined reciprocal square root, 0 for x = 0).
fn length3(v: [f32; 3]) -> f32 {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}

/// The retail audio-state record (`skate_audio::player::state`) from this tick's physics: what the
/// bridge `sub_824B0DA8` reads, mapped to the engine's equivalents (field docs there).
/// - `+332` in the air = state 200..300 with no wheel down (ProcessOutput's KnownAir test); our
///   engine rarely selects KnownAir itself, so `+236` is our own time in that air phase.
/// - `+343` / `+372`: the score packet's scorable id (bit 24 or 25 of its flags, else none);
///   hippy jump = id 234 (`hippyjump`) while in the air.
/// - `+192` / `+692` latch the grind family / material while grinding (−1 / 143 before the first).
fn audio_state(physics: &GamePhysics, skater: &SkaterRuntime, f: AudioFrame) -> skate_audio::player::AudioState {
    use skate_audio::player::state::{AudioState, material_of_tag};
    let p = &skater.player_input.physical;
    let flags = &skater.player_state.state_flags;
    let flag = |byte: usize| flags.get(byte - 52).copied().unwrap_or(false);
    let ground = &physics.riding.ground;
    let lines = &physics.riding.wheel_lines;
    let wheel_contact: [bool; 4] = std::array::from_fn(|i| !f.unridden && ground.parts[i].in_contact);
    let airborne = (200..300).contains(&f.state) && f.wheels == 0;
    *f.air_time = air_time_236(*f.air_time, f.state, f.dt);
    let grinding = f.on_rail;
    if p.grinds.grinding_316 != 0 || grinding {
        *f.grind_family = Some(p.grinds.words_136_140[0] as i32);
        // `+692` is the material, not the tag: the packer `sub_827A1B78` stores Grinds+216 − 1 at record
        // `+512` (0 or out of 0..143 → 143) exactly as it does for the wheels, and the bridge copies it.
        *f.grind_material = Some(material_of_tag(p.grinds.audio_surface_216));
    }
    let score = &skater.animation.motion.score_packet;
    let scorable = if score.flags & 0x0300_0000 != 0 {
        score.trick_names.first.and_then(|name| {
            skater.scoring.data.definitions.iter().find(|d| d.encoded_name == name).map(|d| d.metadata.id as i64)
        })
    } else {
        None
    };
    // +468: the conditioner writes |Air+112| / 2.65 while Air440 (Processed2468 bit 22) holds.
    if skater.player_input.processed.flags_2468 & 0x0040_0000 != 0 {
        let [x, y, z] = raw3(p.air.jump_velocity_delta_112);
        *f.jump_velocity = skate_audio::player::state::jump_velocity((x * x + y * y + z * z).sqrt());
    }
    let (board, board_velocity) = f.board;
    // +228: B40+32 = Grinds+128 when positive, else the last one (`sub_827731C8`).
    if p.grinds.impact_speed_128 > 0.0 {
        *f.grind_impact = p.grinds.impact_speed_128;
    }
    // +668: the conditioner's max of the last 4 Collision+24 = clamp01(|deck acceleration · deck
    // contact normal| × 0.00125) with the deck in contact (`sub_82C02A80`, `sub_82772748`).
    let deck = BodyId::Deck.index();
    let deck_contact = !f.unridden && ground.parts[deck].in_contact;
    let impact = if deck_contact {
        let (a, n) = (ground.accelerations[deck], ground.parts[deck].normal);
        let d = (a.x * n.x + a.y * n.y + a.z * n.z).abs() * f32::from_bits(0x3AA3_D70A);
        skate_audio::player::clamp01(d)
    } else {
        0.0
    };
    f.deck_ring[*f.deck_at % 4] = impact;
    *f.deck_at = (*f.deck_at + 1) % 4;
    let deck_impact = f.deck_ring.iter().copied().fold(0.0f32, f32::max);
    // +268..+280: the toes' local velocities in the physical board's frame (Skeleton+192..+216).
    let toes = skater.foot_physical.output.local_velocity;
    // Off-board inputs (`player::footsteps` / `clothing` / `step_on`; spec
    // `audio-specs/aems-offboard-clothing-spec.md` §3). Foot A = toe part 19 (OffBoard side 1),
    // foot B = part 15.
    let world_toes = skater.foot_physical.output.world_velocity;
    let push_planted = flag(55);
    // Skeleton+602/+603: the IK's hand targets (limbs 2 / 3) relative to the animated board inside
    // the deck box padded by physics_skeletonik `A28E50D30B0506A4`.
    let (half_width, half_length) = skater.foot_physical.deck_box();
    let hands = [2usize, 3].map(|limb| {
        let t = skater.foot_ik.state.frames[limb].board[3];
        skate_audio::player::step_on::hand_in_deck_box([t[0], t[1], t[2]], half_width, half_length, skate_audio::player::step_on::HAND_PADDING)
    });
    // Skeleton::FillPhysOut (`sub_82BE1AE8`) publishes, from the physical record and the
    // skeleton assembly's bodies (assembly +76 + 96·part):
    // - +144 / +160 = the physical pose translations of toe parts 15 / 19 (the step code's plant
    //   heights);
    // - +288 / +304 / +320 = the angular velocity (body +48) of parts 23 / 20 / 16, which the record
    //   writer turns into +328 = |+288| (the body speed) and +296 / +292 = |y| of +304 / +320 (foot
    //   A / B "vertical" speeds);
    // - +560..+572 = |record velocity − Skeleton16176 (the physical COM velocity)| of parts 17, 21,
    //   4, 8, averaged into +672 = 0.25 · (572 + 568 + 564 + 560).
    let record = &skater.skeleton.record;
    let bodies = skater.skeleton.bodies();
    let spin = |part: usize| {
        let w = bodies[part].rates.angular_velocity;
        [w.x, w.y, w.z]
    };
    let com16176 = raw3(p.reckoning.vector_16);
    let limb = |part: usize| {
        let v = record.velocities[part];
        length3([v[0] - com16176[0], v[1] - com16176[1], v[2] - com16176[2]])
    };
    let limb_speed = (((limb(8) + limb(4)) + limb(21)) + limb(17)) * 0.25;
    let step_code = f.step_code.update(
        [p.off_board.flags_306_307[0] != 0, p.off_board.flags_306_307[1] != 0],
        [skater.player_input.processed.left_surface_2596, skater.player_input.processed.right_surface_2600],
        [record.pose[15][3][1], record.pose[19][3][1]],
    );
    // The ragdoll's body regions (SkeletonCollision, published by `sub_82BD60C8` into
    // Collision+80..+195 and copied to the audio state +496..+611): per region with a contact
    // part its tangential (slide) speed (+944 + 4i → +528..+548) and surface tag (+976 + 4i →
    // +560..+580; the per-frame PhysOut reset leaves 0 without one), and the face point's contact
    // (specific point 1, byte 4009 → +593).
    let fb = &skater.collision_feedback;
    // +496..+516: the regions' impacts, kept as the max of the last 4 frames (`region_impacts`).
    f.region_ring[*f.region_at % 4] = region_impacts(&fb.regions, &record.velocity_changes, &skater.skeleton.definition.animation_masses.part_weights);
    *f.region_at = (*f.region_at + 1) % 4;
    let body_impact: [f32; 6] = std::array::from_fn(|i| f.region_ring.iter().map(|r| r[i]).fold(0.0f32, |m, x| if x > m { x } else { m }));
    let xz = |v: [f32; 4]| if v[0].abs() - v[2].abs() >= 0.0 { v[0].abs() } else { v[2].abs() };
    // Water (`sub_827A1B78` → record `+172` bits 30 / 29 / 28 → `+811` / `+812` / `+813`):
    // in water = the state's `+81` (Wipeout300's water contact); under the surface = in water and
    // the state's surface height `+32` above the Y of PhysOut Skeleton `+128` / `+112` / `+32`.
    // `+32` is part 1's physical pose translation (Skeleton+8128). UNCERTAIN: `+128` / `+112` are
    // parts 15 / 19 (the toes) with a per-part local point (Skeleton+2560 / +2816, translation at
    // +48) that the engine does not publish; the toes' pose translations stand in for them.
    let in_water = flag(81);
    let surface = p.state.surface_height_32;
    let under_water = in_water && [1usize, 15, 19].iter().any(|&part| surface > record.pose[part][3][1]);
    AudioState {
        dt: f.dt,
        ground_speed: physics.riding.motion.ground_speed,
        com_velocity: raw3(p.reckoning.vector_16),
        com_position: raw3(p.reckoning.vector_64),
        board_position: board.to_array(),
        board_velocity: board_velocity.to_array(),
        wheel_count: f.wheels,
        wheel_contact,
        // `+620..+632` / `+636..+648` come from the wheel lines (82C079E0: each wheel's 0.2 m ray), not
        // from contact: retail keeps wheel 0's material on 100 % of 3-wheel and 96 % of 2-wheel frames
        // and changes it 0.45 times a second while rolling (GREC, local rider only;
        // `tools/recomp-trace/grec_material.py`).
        // Gating them by contact made every contact flicker a material change, i.e. a Class_Seams
        // transition hit (5–9 changes/s on wheel 0 in the 20:08 / 20:13 sessions).
        wheel_material: std::array::from_fn(|i| if f.unridden { 143 } else { material_of_tag(lines.audio_surfaces[i]) }),
        seam_pattern: std::array::from_fn(|i| if f.unridden { 0 } else { lines.seam_patterns[i] }),
        wheel_position: {
            let parts = physics.board.part_transforms();
            std::array::from_fn(|i| {
                let p = parts[i].translation;
                [p.x, p.y, p.z]
            })
        },
        turn: skater.animation_input.fields.turn,
        slope: skater.ground.pumping.absorption,
        airborne,
        air_time: *f.air_time,
        brake: flag(52),
        manual_brake: flag(54),
        balance: flag(60),
        grinding,
        trick_active: scorable.is_some(),
        hippy_jump: airborne && scorable == Some(234),
        bail: flag(59),
        bail_end: p.skeleton.over_599 != 0,
        on_foot: f.state == 500,
        soft_wheels: skater.player_input.processed.scalar_2764 < 0.5,
        push_planted: f.push,
        push_trigger: f.push_trigger,
        feet_in_deck_box: [p.skeleton.flag_600 != 0, p.skeleton.flag_601 != 0],
        grind_family: f.grind_family.unwrap_or(-1),
        grind_material: f.grind_material.unwrap_or(143),
        local: true,
        jump_velocity: *f.jump_velocity,
        audio_trick: -1,
        scorable: scorable.map_or(-1, |id| id as i32),
        slip: f.slip,
        // +690: RevertGround (102) with State+66 set.
        revert: f.state == 102 && flag(66),
        // `+308` = OffBoard 311 (the board held in hand).
        offboard_308: p.off_board.flag_311 != 0,
        deck_tilt: skater.ground.steering.deck_tilt,
        deck_spin: f.deck_spin,
        grind_impact: *f.grind_impact,
        deck_impact,
        deck_material: if f.unridden { 143 } else { material_of_tag(ground.part_audio_surfaces[2]) },
        foot_speed_y: [toes[0][1].abs(), toes[1][1].abs()],
        foot_speed_xz: [xz(toes[0]), xz(toes[1])],
        // Common82DB6EC0 copies Processed2624 into Ground300;82772D30 buckets it.
        jump_strength: skater.animation_input.extra.jump_strength,
        jump_bucket: 0, // PlayerAudio::resolved uses the exported native thresholds.
        // `+352`: the host resolves it from the scorable (`PlayerTuning::audio_trick_2`).
        audio_trick_2: -1,
        offboard_310: false, // build applies the native bridge clock after publication.
        deck_spin_xy: f.deck_spin_xy,
        // `+240` / `+260`: KnownAir's predicted time until landing and jump height (Air+184 / +200).
        air_until_landing: p.air.scalar_184,
        jump_height: p.air.jump_height_200,
        // `+220` the game time scale and `+224` (0 in free skate).
        time_scale: 1.0,
        global_224: false,
        foot_down: [
            p.off_board.flags_306_307[1] != 0 || p.air.footplant_right_450 != 0 || (push_planted && flag(57)) || flag(52),
            p.off_board.flags_306_307[0] != 0 || p.air.footplant_left_449 != 0 || (push_planted && !flag(57) && flag(56)),
        ],
        foot_material: [
            material_of_tag(skater.player_input.processed.right_surface_2600 & 0x7F),
            material_of_tag(skater.player_input.processed.left_surface_2596 & 0x7F),
        ],
        foot_xz_speed: [xz(world_toes[1]), xz(world_toes[0])],
        foot_vertical_speed: [spin(20)[1].abs(), spin(16)[1].abs()],
        step_code,
        footstep_strength: skater.animation_input.extra.footstep_strength,
        // `+300`: the host's LandingBucket (it needs the resolved audio trick).
        landing_bucket: 1,
        footplant: p.air.flag_448 != 0,
        offboard_air: p.filtered_state_0 == 7,
        push_stroke: flag(56),
        body_speed: length3(spin(23)),
        limb_speed,
        body_slide: std::array::from_fn(|i| if fb.regions[i].part.is_some() { fb.regions[i].tangent_speed } else { 0.0 }),
        body_tag: std::array::from_fn(|i| if fb.regions[i].part.is_some() { fb.regions[i].material_flags } else { 0 }),
        body_slide_flag: fb.specific[1].current,
        body_impact,
        // `+216`: set by `observe` (last tick's `+212`).
        com_speed_216: 0.0,
        hands_on_deck: skate_audio::player::step_on::hands_on_deck(hands, f.state == 500, p.off_board.flag_311 != 0),
        in_water,
        under_water,
        board_in_water: crate::physics::board_surface(physics) == 12,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `+236` across an ollie as the recomp's TREAT capture has it (session all_20261002_223306,
    /// 60 Hz ticks): 0 on the ground, 1/60 on the first air tick, still counting on the landing
    /// tick (state 201, wheels down), 0 once the state leaves the air.
    #[test]
    fn air_time_holds_through_the_landing_tick() {
        let dt = 1.0 / 60.0;
        // (state, wheels down) per tick: rolling, takeoff, 3 air ticks, landing tick, rolling.
        let ticks = [(103, true), (201, false), (201, false), (201, false), (201, false), (201, true), (100, true)];
        let mut t = 0.0f32;
        let run: Vec<i32> = ticks.iter().map(|&(state, _down)| {
            t = air_time_236(t, state, dt);
            (t * 60.0).round() as i32
        }).collect();
        assert_eq!(run, [0, 1, 2, 3, 4, 5, 0]);
    }

    /// The push plant follows State55 (not the animation's push contact) and fires its edge once
    /// per plant.
    #[test]
    fn the_push_plant_is_state55_and_its_rise() {
        // (State55, push contact) per tick: the stroke's contact first, the plant ~0.5 s later.
        let ticks = [(false, true), (false, true), (false, false), (true, false), (true, false), (false, false)];
        let mut planted = false;
        let run: Vec<(bool, bool)> = ticks.iter().map(|&(s55, _push)| {
            let r = push_plant(s55, planted);
            planted = r.0;
            r
        }).collect();
        assert_eq!(run, [(false, false), (false, false), (false, false), (true, true), (true, false), (false, false)]);
    }

    #[test]
    fn region_impacts_scale_the_velocity_change_by_the_part_mass() {
        use skate_core::physics::skeleton_body::ContactRegion;
        let mut regions = [ContactRegion::default(); 8];
        regions[1] = ContactRegion { part: Some(2), normal: [0.0, 1.0, 0.0, 0.0], ..ContactRegion::default() };
        regions[2] = ContactRegion { part: Some(3), normal: [0.0, 1.0, 0.0, 0.0], ..ContactRegion::default() };
        regions[3] = ContactRegion { part: Some(7), normal: [0.0, 1.0, 0.0, 0.0], ..ContactRegion::default() };
        let mut dv = [[0.0f32; 4]; 26];
        dv[2] = [0.0, 3.0, 0.0, 0.0];
        dv[3] = [0.0, 50.0, 0.0, 0.0];
        dv[7] = [0.0, -2.0, 0.0, 0.0];
        let mut masses = [0.0f32; 24];
        masses[2] = 0.02;
        masses[3] = 0.01;
        masses[7] = 0.01;
        let r = region_impacts(&regions, &dv, &masses);
        assert_eq!(r[0], 0.0, "no contact part");
        assert!((r[1] - 0.6).abs() < 1e-6, "3 m/s × 0.02 × 10");
        assert_eq!(r[2], 1.0, "clamped");
        assert!((r[3] - 0.2).abs() < 1e-6, "away from the surface counts too: |−2| × 0.01 × 10 = {}", r[3]);
        // Under the floor either way.
        dv[7] = [0.0, -0.001, 0.0, 0.0];
        assert_eq!(region_impacts(&regions, &dv, &masses)[3], f32::from_bits(0x3A83_126F), "the floor 0.001");
    }

    /// Against the recomp (data-gated): every BAILREG line of the fixed-hook bail runs (`audiox`;
    /// `X region part impact |dv.n| v_old.n v_new.n |dv| normal mass slide tag`) gives the impact the
    /// game wrote from Δv·n = v_new·n − v_old·n along its normal and its mass, within the lines' 4-decimal
    /// rounding. The signed form misses the lines with Δv·n < 0.
    #[test]
    #[ignore = "needs the private install data"]
    fn region_impacts_match_the_recomps_bail_lines() {
        use skate_core::physics::skeleton_body::ContactRegion;
        let Some(root) = std::env::var_os("SKATE_RECOMP_SESSIONS").filter(|v| !v.is_empty()).map(std::path::PathBuf::from) else {
            panic!("missing private data: SKATE_RECOMP_SESSIONS (the recomp session folders) is not set");
        };
        let mut lines = Vec::new();
        for run in ["bailrun_t2_20261003_100040", "bailrun_ok_20261003_100226", "bailrun_ok2_20261003_100514", "bailrun_ok3_20261003_101354"] {
            let Ok(text) = std::fs::read_to_string(root.join(run).join("trace.tsv")) else {
                panic!("missing private data: no recomp bail trace {run}");
            };
            for l in text.lines().filter(|l| l.starts_with("BAILREG\t")) {
                let f: Vec<&str> = l.split('\t').collect();
                assert_eq!(f.len(), 14, "malformed BAILREG line: {l}");
                lines.push(f.iter().map(|x| x.to_string()).collect::<Vec<_>>());
            }
        }
        let (mut ok, mut negative) = (0usize, 0usize);
        for f in &lines {
            let num = |i: usize| f[i].parse::<f32>().unwrap();
            let (impact, v_old, v_new, mass) = (num(5), num(7), num(8), num(11));
            let n: Vec<f32> = f[10].split(' ').map(|x| x.parse().unwrap()).collect();
            let d = v_new - v_old;
            negative += usize::from(d < 0.0);
            let mut regions = [ContactRegion::default(); 8];
            regions[0] = ContactRegion { part: Some(1), normal: [n[0], n[1], n[2], 0.0], ..ContactRegion::default() };
            let len2 = n[0] * n[0] + n[1] * n[1] + n[2] * n[2];
            let mut dv = [[0.0f32; 4]; 26];
            dv[1] = [n[0] * d / len2, n[1] * d / len2, n[2] * d / len2, 0.0];
            let mut masses = [0.0f32; 24];
            masses[1] = mass;
            let ours = region_impacts(&regions, &dv, &masses)[0];
            ok += usize::from((ours - impact).abs() <= 0.002);
        }
        println!("BAILREG lines {}, Δv·n < 0 on {negative}, ours within 0.002 on {ok}", lines.len());
        assert!(lines.len() > 10_000 && negative > 1_000);
        assert_eq!(ok, lines.len(), "every line");
    }

    #[test]
    fn grind_states_are_the_six_grinds() {
        assert!(grinding(PhysicalStateId::GrindBoardslide as u32));
        assert!(grinding(PhysicalStateId::GrindDarkslide as u32));
        assert!(!grinding(PhysicalStateId::WipeoutGround as u32));
        assert!(!grinding(PhysicalStateId::SlideGround as u32));
    }
}
