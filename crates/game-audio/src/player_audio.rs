//! The local player's sounds on the native runtime (`skate_audio::player`), hosted by
//! `native::mixmap_frame` once per frame that took physics steps (`native::HostClock`):
//! 1. inputs: PlayerPhysics, the two 3DObjPos blocks, Jitter, Contacts, Rail, OffBoard;
//! 2. components' process (posts / releases) — before the MixMap tick;
//! 3. components' update (packets rewritten from the MixMap outputs, redelivered) — after it.
//!
//! The player components run whenever their banks are in the install (2026-10-03: the interim
//! cue tables and their opt-outs are gone; without the banks the skater's sounds are silent and the
//! host logs an error).
use std::collections::HashMap;

use skate_audio::eval::NodeId;
use skate_audio::mixmap::{MixMap, keys};
use skate_audio::player::collision::{self as collisions, CollisionManager};
use skate_audio::player::components::{self, Command, FootDrag, Grind, SenseOfSpeed, Skid, Slot, Squeaks};
use skate_audio::player::contacts::{self as board_contacts, ContactsTuning};
use skate_audio::player::inputs::{self, Contacts, Physics};
use skate_audio::player::jitter::Jitter;
use skate_audio::player::objpos::{Listener, ObjPos};
use skate_audio::player::tuning::PlayerTuning;
use skate_audio::player::clothing::{Clothing, ClothingTuning};
use skate_audio::player::footsteps::{FootstepTuning, Footsteps, LandingBucket};
use skate_audio::player::globals::Globals;
use skate_audio::player::rolling::{self, BoardSlide, Rattle, Rolling, RollingInputs, Routed};
use skate_audio::player::treatment::{self, Treatment, TreatmentGlobals};
use skate_audio::player::tricks::{self, Tricks};
use skate_audio::player::wheels::{Wheels, WheelsTuning};
use skate_audio::player::{AudioState, Owner};
use skate_audio::runtime::Runtime;

pub(crate) struct PlayerAudio {
    pub(crate) tuning: PlayerTuning,
    physics: Physics,
    contacts: Contacts,
    jitter: Jitter,
    /// The Jitter walk steps for the next [`Self::write_inputs`]: `Some(n)` under the MixMap's
    /// console cadence (n console evaluations in this pass, `native::mixmap_frame`), `None`: one
    /// step per call (the old 60 Hz host and the tests).
    pub(crate) jitter_steps: Option<usize>,
    positions: [ObjPos; 2],
    was_grinding: bool,
    last_camera: Option<[f32; 3]>,
    /// The components run (banks loaded, classes bound).
    pub(crate) components: bool,
    grind: Grind,
    speed: SenseOfSpeed,
    foot_drag: FootDrag,
    skid: Skid,
    squeaks: Squeaks,
    /// `Class_Seams` (the wheels' seam / crack hits, `Seams_Bank`).
    seams: skate_audio::player::seams::Seams,
    /// The rendered board's interpolation factor for this call (the fixed step's overstep fraction):
    /// Class_Seams reads the wheel positions interpolated between the last two physics samples, as
    /// retail's per-rendered-frame process does (Listening test 9). `None`: the physics positions.
    pub(crate) seam_alpha: Option<f32>,
    /// The last two physics samples of the wheel positions (previous, current).
    seam_wheels: Option<([[f32; 3]; 4], [[f32; 3]; 4])>,
    /// `SFXObj_Contacts`' Splice one-shots (pops, landings, touchdowns); on when the install has
    /// the `Skate_Collisions` patch tree ([`PlayerAudio::contacts_on`]).
    board: board_contacts::Contacts,
    pub(crate) contact_tuning: ContactsTuning,
    pub(crate) contacts_on: bool,
    /// The collision manager (`CSTATEMGR_Collision`): the contact pairs the board contacts post
    /// (grind start, landing pair, deck impacts); on with the contacts.
    collision: CollisionManager,
    /// `SFXObj_Wheels` (the spin-down streams); on when the install has both recordings
    /// ([`WHEEL_STREAMS`], decoded at start).
    wheels: Wheels,
    pub(crate) wheels_tuning: WheelsTuning,
    pub(crate) wheels_on: bool,
    /// `SFXObj_SkateBoard`'s rolling layers (`player::rolling`): the two-truck surface routing with
    /// its Class_rolling patches and held layers (on with `PatchBank_Rolling_Surfaces`), the rattle
    /// (`Rolling_Rattles`) and the loose-board slide (`board_scrapes`).
    rolling: Rolling,
    rattle: Rattle,
    slide: BoardSlide,
    pub(crate) rolling_on: bool,
    pub(crate) rattle_on: bool,
    pub(crate) slide_on: bool,
    /// The routing's grain binds / stops since the bed last took them (the bed follows the
    /// routing while [`PlayerAudio::rolling_on`]).
    pub(crate) routed: Routed,
    /// The Tricks component (Class_Flips, cloth_trick; `Sk8_Air_Flip_Tricks`, `Foley_Cloth`) and
    /// Class_Treatment (`Treatments`), each on when its banks are in the install.
    tricks: Tricks,
    treatment: Treatment,
    pub(crate) tricks_on: bool,
    pub(crate) treatment_on: bool,
    /// The game-mode globals those read (free skate).
    globals: Globals,
    /// The teleport effect amount this pass (`ui_audio::TeleportEffect`: the session marker's Go To
    /// Marker hold, or a mod's): the presentation block's teleport field `B+164` / `B+168` that
    /// Class_Treatment's update turns into w12 / w13 (its program's teleport crackle). None = absent.
    pub(crate) teleport_effect: Option<f32>,
    /// SFXObj_OffBoard's footsteps (packets + the foot-down / walking / jump Splice sounds) and the
    /// Clothing component (cloth falls, push foley, body slide): on with the board contacts (their
    /// Splice banks), plus OffBoard's water splash.
    footsteps: Footsteps,
    footstep_tuning: FootstepTuning,
    clothing: Clothing,
    pub(crate) clothing_tuning: ClothingTuning,
    pub(crate) footsteps_on: bool,
    /// `+300` the landing bucket (it reads the resolved audio trick), this frame's value.
    landing: LandingBucket,
    landing_bucket: i32,
    /// The last listener the position writers used (the collision slots' 3-D blocks).
    last_listener: Option<Listener>,
    nodes: HashMap<Slot, NodeId>,
    classes: HashMap<&'static str, usize>,
    /// Commands of the last frame (for the log on change and the tests).
    pub(crate) posts: u64,
    /// Audio event rows (posts, releases, Splice starts) while some mod subscribes
    /// (`mod_audio::events_frame`); None otherwise: nothing is recorded.
    pub(crate) events: super::mod_audio::EventBuf,
    /// The mods' mute / replace / layer rules (`mod_rules`); None without rules.
    pub(crate) rules: Option<std::sync::Arc<super::mod_rules::RuleSet>>,
}

impl PlayerAudio {
    /// SFXObj_Jitter's generator (`seed.rs`, doc 16 L5).
    pub(crate) fn jitter_rng(&self) -> skate_audio::eval::rng::Rng {
        self.jitter.rng
    }
    pub(crate) fn jitter_rng_mut(&mut self) -> &mut skate_audio::eval::rng::Rng {
        &mut self.jitter.rng
    }

    /// A runtime tuning write changed the player tuning (`tuning.rs`): the vault tuning, the
    /// Contacts posters' values and the footstep materials follow. SFXObj_Jitter keeps the walk it
    /// was built with (its parameters change at the next start).
    pub(crate) fn retune(&mut self, library: &super::library::Library) {
        self.tuning = library.player_tuning();
        self.contact_tuning = library.contacts_tuning();
        self.set_footstep_materials(library.footstep_materials());
    }

    pub(crate) fn new(tuning: PlayerTuning, components: bool) -> Self {
        let jitter = Jitter::new(&tuning.jitter, skate_audio::grain::GrainBed::SEED);
        // The collision voices play through their Collision SubMix (`sub_824D25E0`: mono, env send
        // at the category's Collision output, eEQChain bus).
        let mut collision = CollisionManager::default();
        collision.submix = true;
        let mut board = board_contacts::Contacts::default();
        // Session review 2026-10-03: the push foot's plant / lift (#2), the body poster (#4) and the
        // grind on / off sounds (#5); the bridge's speed graph on the body impacts (`sub_824B0DA8`).
        board.plant_lift_on = true;
        board.body_on = true;
        board.body_speed_on = true;
        let mut grind = Grind::default();
        grind.onoff = true;
        Self {
            tuning,
            physics: Physics::default(),
            contacts: Contacts::default(),
            jitter,
            jitter_steps: None,
            positions: [ObjPos::default(); 2],
            was_grinding: false,
            last_camera: None,
            components,
            grind,
            speed: SenseOfSpeed::default(),
            foot_drag: FootDrag::default(),
            skid: Skid::default(),
            squeaks: Squeaks::default(),
            seams: Default::default(),
            seam_alpha: None,
            seam_wheels: None,
            board,
            contact_tuning: ContactsTuning::default(),
            contacts_on: false,
            collision,
            wheels: Wheels::default(),
            wheels_tuning: WheelsTuning::default(),
            wheels_on: false,
            rolling: Rolling::default(),
            rattle: Rattle::default(),
            slide: BoardSlide::default(),
            rolling_on: false,
            rattle_on: false,
            slide_on: false,
            routed: Routed::default(),
            tricks: Tricks::default(),
            treatment: Treatment::default(),
            tricks_on: false,
            treatment_on: false,
            globals: Globals::default(),
            teleport_effect: None,
            footsteps: Footsteps::default(),
            footstep_tuning: FootstepTuning::default(),
            clothing: Clothing::default(),
            clothing_tuning: ClothingTuning::default(),
            footsteps_on: false,
            landing: LandingBucket::default(),
            landing_bucket: 1,
            last_listener: None,
            nodes: HashMap::new(),
            classes: HashMap::new(),
            posts: 0,
            events: None,
            rules: None,
        }
    }

    /// The body poster's messages so far and their digest (`Contacts::body_posts` / `body_digest`).
    #[cfg(test)]
    pub(crate) fn body_trace(&self) -> (u64, u64) {
        (self.board.body_posts, self.board.body_digest)
    }

    /// The deck poster's messages so far and their digest (`Contacts::deck_posts` / `deck_digest`).
    #[cfg(test)]
    pub(crate) fn deck_trace(&self) -> (u64, u64) {
        (self.board.deck_posts, self.board.deck_digest)
    }

    /// Diagnostics (the e2e harness's `E2E_BODY_LOG`): keep every body-poster message.
    #[cfg(test)]
    pub(crate) fn set_body_log(&mut self, on: bool) {
        self.board.body_log = on.then(Vec::new);
    }

    /// The body-poster messages since the last call (region, impact read, message).
    #[cfg(test)]
    pub(crate) fn take_body_log(&mut self) -> Vec<(usize, f32, skate_audio::player::collision::Message)> {
        self.board.body_log.as_mut().map(std::mem::take).unwrap_or_default()
    }

    /// Collision messages of another Player-slot owner (an NPC skater, `npc_skaters.rs`): retail
    /// has one `CSTATEMGR_Collision`, so they join the local player's (processed at its next
    /// `process`). No messages, no change.
    pub(crate) fn post_collisions(&mut self, msgs: Vec<skate_audio::player::collision::Message>, rt: &mut Runtime) {
        if msgs.is_empty() || !self.contacts_on {
            return;
        }
        let mut access = rt.splice_host();
        let mut host = super::mod_audio::Observed::new(&mut access, &mut self.events, super::mod_audio::Source::Player, 0).rules(self.rules.as_deref());
        for msg in msgs {
            self.collision.post(msg, &mut host);
        }
    }

    /// The materials' footstep layers (`Library::footstep_materials`).
    pub(crate) fn set_footstep_materials(&mut self, materials: Vec<skate_audio::player::footsteps::FootstepMaterial>) {
        self.footstep_tuning.materials = materials;
    }

    /// The listener as the position controllers see it: the camera (position, view, velocity from
    /// the last frame) and the followed point (the skater's COM).
    /// `dt`: the time since the last call.
    pub(crate) fn listener(&mut self, camera: [f32; 3], view: [f32; 3], dt: f32, s: &AudioState) -> Listener {
        let velocity = match self.last_camera {
            Some(last) if dt > 0.0 => std::array::from_fn(|i| (camera[i] - last[i]) / dt),
            _ => [0.0; 3],
        };
        self.last_camera = Some(camera);
        Listener {
            camera,
            view,
            camera_velocity: velocity,
            followed: s.com_position,
            // Frame B's direction is only read by the skater-frame azimuth (input 2), which no B
            // lookup of the MixMap uses; the COM velocity stands in for the facing.
            facing: s.com_velocity,
            followed_velocity: s.com_velocity,
        }
    }

    /// Step 1: every input the player's writers supply (mixmap-spec §7).
    pub(crate) fn write_inputs(&mut self, m: &mut MixMap, s: &AudioState, listener: Option<&Listener>) -> bool {
        self.last_listener = listener.copied();
        self.physics.write(m, s);
        if let Some(l) = listener {
            // 60010010 follows the skater (COM, COM velocity); 60010020 the board (deck, its
            // velocity), as `sub_824B0C48` binds them.
            self.positions[0].write(m, keys::obj_pos(0), l, Some((s.com_position, s.com_velocity)));
            self.positions[1].write(m, keys::obj_pos2(0), l, Some((s.board_position, s.board_velocity)));
        }
        for _ in 0..self.jitter_steps.unwrap_or(1) {
            // The MixMap writes don't touch the walk: the same draws and writes as collecting first.
            self.jitter.process_each(|id, word| m.set_input(keys::JITTER, id, word));
        }
        let landed = self.contacts.write(m, s, &self.tuning);
        inputs::write_rail(m, s.grinding, self.was_grinding);
        self.was_grinding = s.grinding;
        inputs::write_off_board(m, s);
        landed
    }

    fn apply(&mut self, rt: &mut Runtime, cmds: Vec<Command>) {
        for cmd in cmds {
            match cmd {
                Command::Post { slot, class, words } => {
                    let id = *self.classes.entry(class).or_insert_with(|| rt.eval.class_id(class).unwrap_or(usize::MAX));
                    if id == usize::MAX {
                        continue;
                    }
                    if let Some(old) = self.nodes.remove(&slot) {
                        rt.release(old);
                    }
                    // A mod rule may drop the post (its redeliveries and release then find no node).
                    let muted = self.rules.as_deref().is_some_and(|r| {
                        let (name, index) = super::mod_audio::player_slot(&slot);
                        r.mutes(&super::mod_audio::EventRow { kind: super::mod_audio::EventKind::Post, source: super::mod_audio::Source::Player, class, slot: name, id: index, owner: 0 })
                    });
                    if !muted {
                        self.nodes.insert(slot, rt.post(id, &words));
                    }
                    self.posts += 1;
                    if self.events.is_some() {
                        let (name, index) = super::mod_audio::player_slot(&slot);
                        super::mod_audio::record(&mut self.events, super::mod_audio::EventRow { kind: super::mod_audio::EventKind::Post, source: super::mod_audio::Source::Player, class, slot: name, id: index, owner: 0 });
                    }
                    if trace() {
                        tracing::info!("AUDIO_NATIVE post {class} {slot:?} words={words:?}");
                    }
                }
                Command::Redeliver { slot, words } => {
                    if let Some(&node) = self.nodes.get(&slot) {
                        rt.redeliver(node, &words);
                    }
                }
                Command::Release { slot } => {
                    if let Some(node) = self.nodes.remove(&slot) {
                        rt.release(node);
                        if self.events.is_some() {
                            let (name, index) = super::mod_audio::player_slot(&slot);
                            super::mod_audio::record(&mut self.events, super::mod_audio::EventRow { kind: super::mod_audio::EventKind::Release, source: super::mod_audio::Source::Player, class: "", slot: name, id: index, owner: 0 });
                        }
                        if trace() {
                            tracing::info!("AUDIO_NATIVE release {slot:?}");
                        }
                    }
                }
            }
        }
    }

    /// The state with the host-resolved fields (`+348` / `+352` the audio tricks of the scorable).
    fn resolved(&self, s: &AudioState) -> AudioState {
        let mut s = *s;
        let name = usize::try_from(s.scorable).ok().and_then(|id| skate_core::scoring::catalog::IDENTIFIERS.get(id)).map(|(name, ..)| name);
        s.audio_trick = name.map_or(-1, |n| self.tuning.audio_trick(n));
        s.audio_trick_2 = name.map_or(-1, |n| self.tuning.audio_trick_2(n));
        s.jump_bucket = self.tuning.jump_bucket(s.jump_strength);
        s
    }

    /// The loose-board state (`+780`, `rolling::loose_board`): the deck in contact while the rider
    /// bails or walks, by how the deck lies (`skate_events::Riding`: `up_dot` = SkateboardReckoning
    /// +80 · Ground+80, the deck contact Collision+3475 and its material Collision+12 − 1).
    pub(crate) fn loose_board(s: &AudioState, riding: &skate_sim::AudioRiding) -> u32 {
        rolling::loose_board(s.bail || s.on_foot, riding.deck_contact, riding.deck_material, riding.deck_up)
    }

    /// The six jitter values the jittered eEQChain buses 5–7 read (`sub_82491180`), when the
    /// install's tuning names all six channels.
    pub(crate) fn eq_jitter(&self) -> Option<[f32; 6]> {
        let idx = self.tuning.eq_jitter;
        if idx.iter().any(Option::is_none) {
            return None;
        }
        let mut out = [0.0f32; 6];
        for (o, i) in out.iter_mut().zip(idx) {
            *o = self.jitter.channels.get(i?)?.value;
        }
        Some(out)
    }

    /// Class_Seams' hits so far and how many were material changes (diagnostics, e2e).
    #[cfg(test)]
    pub(crate) fn seam_hits(&self) -> (u64, u64) {
        (self.seams.hits, self.seams.transitions)
    }

    fn seam_commands(cmds: Vec<skate_audio::player::seams::SeamCommand>) -> Vec<Command> {
        use skate_audio::player::seams::{CLASS, SeamCommand};
        cmds.into_iter()
            .map(|c| match c {
                SeamCommand::Post { wheel, words } => Command::Post { slot: Slot::Seam(wheel as u8), class: CLASS, words },
                SeamCommand::Redeliver { wheel, words } => Command::Redeliver { slot: Slot::Seam(wheel as u8), words },
                // Scheduled on the audio clock by `seam_frame`; never reaches here.
                SeamCommand::RedeliverAt { wheel, words, .. } => Command::Redeliver { slot: Slot::Seam(wheel as u8), words },
            })
            .collect()
    }

    /// Step 2 (before the tick): the components' process. `speed_scale`: the bed's push
    /// speed-scale envelope while it runs; `loose`: the loose-board state ([`Self::loose_board`]).
    pub(crate) fn process(&mut self, m: &mut MixMap, s: &AudioState, rt: &mut Runtime, speed_scale: Option<f32>, loose: u32) {
        if !self.components {
            return;
        }
        let m_mut = m;
        let mut resolved = self.resolved(s);
        self.landing_bucket = self.landing.update(resolved.com_velocity[1], resolved.offboard_air, resolved.trick_active, resolved.audio_trick);
        resolved.landing_bucket = self.landing_bucket;
        let s = &resolved;
        let mut rolling_cmds = Vec::new();
        if self.rolling_on {
            // SFXObj_SkateBoard's process: the routing, then the rattle (it reads the routing),
            // then the board slide; SkateBoard inputs 0 / 6 from the routing.
            let (cmds, routed) = self.rolling.process(s, &self.tuning, &RollingInputs { speed_scale });
            rolling_cmds = cmds;
            let board = keys::skateboard(0);
            m_mut.set_input(board, 0, if routed.surface_pulse { 32767 } else { 0 });
            m_mut.set_input(board, 6, if routed.on_metal { 32767 } else { 0 });
            self.routed.grains.extend(routed.grains);
            self.routed.primary = routed.primary;
            self.routed.surface_pulse = routed.surface_pulse;
            self.routed.on_metal = routed.on_metal;
        }
        if self.rattle_on {
            rolling_cmds.extend(self.rattle.process(s, &self.rolling, &self.tuning.rolling));
        }
        if self.slide_on {
            rolling_cmds.extend(self.slide.process(loose, &self.tuning.rolling));
        }
        let seam_state = self.seam_state(s);
        // Class_Seams' process runs on the console cadence in `seam_frame`; the tick only creates
        // the packets and writes Cracks.in0.
        let seams = Self::seam_commands(self.seams.process_tick(&seam_state, &self.tuning, m_mut));
        let m: &MixMap = m_mut;
        let rail = Owner { mixmap: m, key: keys::rail(0) };
        let mut cmds = self.grind.process(s, &self.tuning, &rail);
        cmds.extend(self.speed.process(s));
        cmds.extend(self.foot_drag.process(s, &self.tuning));
        let board = Owner { mixmap: m, key: keys::skateboard(0) };
        cmds.extend(self.skid.process(s, &self.tuning, &board));
        cmds.extend(self.squeaks.process(s));
        cmds.extend(seams);
        cmds.extend(rolling_cmds);
        if self.tricks_on {
            cmds.extend(self.tricks.process(s, &self.tuning.tricks, &self.globals, &Owner { mixmap: m, key: keys::tricks(0) }));
        }
        if self.treatment_on {
            cmds.extend(self.treatment.process(s, &self.tuning.treatment, &self.globals, &Owner { mixmap: m, key: keys::treatments(0) }));
        }
        self.apply(rt, cmds);
        if self.contacts_on {
            // The grind on / off contact sounds of this frame's Rail posts (Splice).
            self.grind.sounds(s, &self.tuning, &mut super::mod_audio::Observed::new(&mut rt.splice_host(), &mut self.events, super::mod_audio::Source::Player, 0).rules(self.rules.as_deref()));
            let before = self.board.starts;
            // The body / deck posters run once per console evaluation of this pass (their 15 / 6-frame
            // cooldowns = 0.5 / 0.2 s at any frame rate; `jitter_steps` None = per call, the tests).
            self.board.body_calls = self.jitter_steps;
            self.board.deck_calls = self.jitter_steps;
            self.board.process(s, self.contacts.buckets(), &self.tuning, &self.contact_tuning, &mut super::mod_audio::Observed::new(&mut rt.splice_host(), &mut self.events, super::mod_audio::Source::Player, 0).rules(self.rules.as_deref()));
            self.posts += self.board.starts - before;
            // The collision manager runs after the player's components (its own state manager).
            let mut access = rt.splice_host();
            let mut host = super::mod_audio::Observed::new(&mut access, &mut self.events, super::mod_audio::Source::Player, 0).rules(self.rules.as_deref());
            for msg in std::mem::take(&mut self.board.outbox) {
                self.collision.post(msg, &mut host);
            }
            self.collision.process(m_mut, self.last_listener.as_ref());
        }
        if self.footsteps_on {
            let splashes = self.footsteps.splash.starts;
            let mut cmds = self.footsteps.process(s, &self.tuning, &self.footstep_tuning, &mut super::mod_audio::Observed::new(&mut rt.splice_host(), &mut self.events, super::mod_audio::Source::Player, 0).rules(self.rules.as_deref()));
            if self.footsteps.splash.starts != splashes {
                // OffBoard's water splash (`player::footsteps::Splash`, retail `sub_824EBB58`).
                tracing::info!("AUDIO_EVENT splash native Skate_Collisions:{}", self.footsteps.splash.last_id);
            }
            cmds.extend(self.clothing.process(s, &self.tuning, &self.clothing_tuning, &mut super::mod_audio::Observed::new(&mut rt.splice_host(), &mut self.events, super::mod_audio::Source::Player, 0).rules(self.rules.as_deref())));
            self.apply(rt, cmds);
        }
    }

    /// The wheel positions of the last two physics steps (`Riding::wheels_before`, this sample's),
    /// which the rendered board interpolates between: the pair [`Self::seam_state`] reads. Hosts
    /// set it before every call (2026-10-03: the pair follows the physics steps, so a frame that
    /// takes two steps interpolates between the last two, and a board at rest has no spread);
    /// `before = now` after a camera cut / teleport. Without it the pair follows the samples' changes.
    pub(crate) fn step_wheels(&mut self, before: [[f32; 3]; 4], now: [[f32; 3]; 4]) {
        self.seam_wheels = Some((before, now));
    }

    /// Forget the camera's last position (teleport, camera cut, map change): the next listener
    /// has no velocity instead of the jump's (Doppler).
    pub(crate) fn reset_listener(&mut self) {
        self.last_camera = None;
    }

    /// The state Class_Seams reads: with [`Self::seam_alpha`], the wheel positions of the rendered
    /// board (the last two physics samples interpolated), else the physics sample.
    fn seam_state(&mut self, s: &AudioState) -> AudioState {
        let Some(alpha) = self.seam_alpha else { return *s };
        let now = s.wheel_position;
        let (prev, cur) = match self.seam_wheels {
            // The host's pair for this sample ([`Self::step_wheels`]).
            Some(pair) if pair.1 == now => pair,
            Some((_, cur)) => (cur, now),
            None => (now, now),
        };
        self.seam_wheels = Some((prev, cur));
        let mut out = *s;
        let u = alpha.clamp(0.0, 1.0);
        out.wheel_position = std::array::from_fn(|w| std::array::from_fn(|c| prev[w][c] + (cur[w][c] - prev[w][c]) * u));
        out
    }

    /// Class_Seams on the console cadence (Listening test 9): called on every rendered frame of
    /// `dt` seconds, before [`Self::process`] on frames that tick. Runs the seams' process on a 30 Hz
    /// virtual grid at the rendered wheel positions (`seams::Seams::frame`) and their update once
    /// per console frame; the tick only creates the packets and writes Cracks.in0.
    pub(crate) fn seam_frame(&mut self, m: &MixMap, s: &AudioState, dt: f32, rt: &mut Runtime) {
        if !self.components {
            return;
        }
        let seam_state = self.seam_state(s);
        let calls = self.seams.calls;
        let (now, later): (Vec<_>, Vec<_>) = self
            .seams
            .frame(&seam_state, &self.tuning, dt, rt.blocks)
            .into_iter()
            .partition(|c| !matches!(c, skate_audio::player::seams::SeamCommand::RedeliverAt { .. }));
        if !now.is_empty() {
            self.apply(rt, Self::seam_commands(now));
        }
        for c in later {
            if let skate_audio::player::seams::SeamCommand::RedeliverAt { wheel, words, block } = c {
                if let Some(&node) = self.nodes.get(&Slot::Seam(wheel as u8)) {
                    rt.redeliver_at(node, &words, block);
                }
            }
        }
        // The update half once per console frame too (its turn word slews per packet write), with
        // the last tick's Cracks outputs.
        if self.seams.calls != calls {
            let cracks = Owner { mixmap: m, key: keys::cracks(0) };
            let cmds = Self::seam_commands(self.seams.update(&seam_state, &cracks));
            self.apply(rt, cmds);
        }
    }

    /// Step 3 (after the tick): the components' update.
    pub(crate) fn update(&mut self, m: &MixMap, s: &AudioState, rt: &mut Runtime, speed_scale: Option<f32>, loose: u32) {
        if !self.components {
            return;
        }
        let mut resolved = self.resolved(s);
        resolved.landing_bucket = self.landing_bucket;
        let s = &resolved;
        let rail = Owner { mixmap: m, key: keys::rail(0) };
        let sos = Owner { mixmap: m, key: keys::sense_of_speed(0) };
        let contacts = Owner { mixmap: m, key: keys::contacts(0) };
        let mut cmds = self.grind.update(s, &self.tuning, &rail);
        cmds.extend(self.speed.update(s, &sos));
        cmds.extend(self.foot_drag.update(s, &self.tuning, &contacts));
        let board = Owner { mixmap: m, key: keys::skateboard(0) };
        cmds.extend(self.skid.update(s, &self.tuning, &board));
        cmds.extend(self.squeaks.update(s, &board));
        // The seams' update runs in `seam_frame` (the console cadence).
        if self.rolling_on {
            cmds.extend(self.rolling.update(s, &self.tuning, &RollingInputs { speed_scale }, &board));
        }
        if self.rattle_on {
            cmds.extend(self.rattle.update(&board));
        }
        if self.slide_on {
            cmds.extend(self.slide.update(s, loose, &self.tuning.rolling, &board));
        }
        if self.tricks_on {
            cmds.extend(self.tricks.update(s, &self.tuning.tricks, &Owner { mixmap: m, key: keys::tricks(0) }));
        }
        if self.treatment_on {
            // B+16 / B+24 (the bail field) stay at their reset values: writer not ported.
            let b = TreatmentGlobals { flag_164: self.teleport_effect.is_some(), value_168: self.teleport_effect.unwrap_or(0.0), ..Default::default() };
            cmds.extend(self.treatment.update(s, &b, &self.tuning.treatment, &Owner { mixmap: m, key: keys::treatments(0) }));
        }
        self.apply(rt, cmds);
        if self.contacts_on {
            // The Rail updater's end (`sub_824C42A8`): the grind on / off sounds, after its packets
            // (the family-change starts of `Grind::update` first).
            self.grind.sounds(s, &self.tuning, &mut super::mod_audio::Observed::new(&mut rt.splice_host(), &mut self.events, super::mod_audio::Source::Player, 0).rules(self.rules.as_deref()));
            self.grind.update_sounds(s, &rail, &mut super::mod_audio::Observed::new(&mut rt.splice_host(), &mut self.events, super::mod_audio::Source::Player, 0).rules(self.rules.as_deref()));
            self.board.update(s, &contacts, &self.contact_tuning, &mut super::mod_audio::Observed::new(&mut rt.splice_host(), &mut self.events, super::mod_audio::Source::Player, 0).rules(self.rules.as_deref()));
            let before = self.collision.starts;
            self.collision.update(m, &self.tuning.collision, s.dt, &mut super::mod_audio::Observed::new(&mut rt.splice_host(), &mut self.events, super::mod_audio::Source::Player, 0).rules(self.rules.as_deref()));
            self.posts += self.collision.starts - before;
        }
        if self.footsteps_on {
            let off = Owner { mixmap: m, key: keys::off_board(0) };
            let mut cmds = self.footsteps.update(s, &self.tuning, &self.footstep_tuning, &off, &mut super::mod_audio::Observed::new(&mut rt.splice_host(), &mut self.events, super::mod_audio::Source::Player, 0).rules(self.rules.as_deref()));
            let cloth = Owner { mixmap: m, key: keys::clothing(0) };
            cmds.extend(self.clothing.update(s, &self.tuning, &self.clothing_tuning, &cloth, &mut super::mod_audio::Observed::new(&mut rt.splice_host(), &mut self.events, super::mod_audio::Source::Player, 0).rules(self.rules.as_deref())));
            self.apply(rt, cmds);
        }
        if self.wheels_on {
            let owner = Owner { mixmap: m, key: keys::wheels(0) };
            let before = self.wheels.starts;
            self.wheels.update(s, &owner, &self.wheels_tuning, &mut rt.stream_host());
            self.posts += self.wheels.starts - before;
        }
    }
}

/// `SKATE_AUDIO_TRACE=1`: the `AUDIO_NATIVE post` / `release` lines (one per component post or
/// release, written under the runtime lock; off by default since 2026-10-03).
fn trace() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("SKATE_AUDIO_TRACE").is_some_and(|v| v != "0"))
}

/// The banks the components post into.
pub(crate) const BANKS: &[&str] = components::BANKS;
/// Optional component banks: a component runs when its first bank loads (the rest add layers).
pub(crate) const ROLLING_BANKS: &[&str] = &["PatchBank_Rolling_Surfaces", "PatchBank_SpiderCracks", "PatchBank_Objects", "PatchBank_RocksBounce"];
pub(crate) const RATTLE_BANKS: &[&str] = &["Rolling_Rattles"];
pub(crate) const SLIDE_BANKS: &[&str] = &["board_scrapes"];
pub(crate) const TRICKS_BANKS: &[&str] = tricks::BANKS;
pub(crate) const TREATMENT_BANKS: &[&str] = treatment::BANKS;
pub(crate) const OPTIONAL_BANKS: [&[&str]; 5] = [ROLLING_BANKS, RATTLE_BANKS, SLIDE_BANKS, TRICKS_BANKS, TREATMENT_BANKS];
/// The Splice banks the components play (patch trees from `audio_export.splice_trees`): the board
/// contacts need the first; the collision manager's metal family and the shoe scuffs use the others
/// when present.
pub(crate) const SPLICE_BANKS: &[&str] = &[board_contacts::BANK, collisions::BANKS[1], board_contacts::FOLEY];
/// `SFXObj_Wheels`' recordings (stream 0 jump, 1 manual) in the manifest's `wheels`.
pub(crate) const WHEEL_STREAMS: [&str; 2] = ["Whls_spins_Jump_1", "Whls_spins_Man_1"];

#[cfg(test)]
mod tests {
    use super::*;
    use skate_audio::grain::board::{self, BoardInputs, TurnIntensity};

    fn install() -> Option<(super::super::library::Library, Vec<u8>)> {
        let root = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets"));
        let library = super::super::library::Library::load(root).ok()?;
        let mxb = library.aems().mixmap.clone()?;
        let bytes = library.read(&mxb).ok()?;
        Some((library, bytes))
    }

    /// Straight rolling at a speed: the skater along +x, the board 0.9 m below the COM, the camera
    /// 3.5 m behind and 1.4 m above looking ahead; four wheels down; free-skate globals.
    fn rolling(kmh: f32, turn: f32) -> AudioState {
        let v = kmh / 3.6;
        AudioState {
            ground_speed: v,
            com_velocity: [v, 0.0, 0.0],
            com_position: [0.0, 1.0, 0.0],
            board_position: [0.0, 0.1, 0.0],
            board_velocity: [v, 0.0, 0.0],
            wheel_count: 4,
            wheel_contact: [true; 4],
            wheel_material: [3; 4],
            turn,
            ..Default::default()
        }
    }

    fn globals(m: &mut MixMap) {
        for id in 1..=4 {
            m.set_input(keys::MASTER, id, 32767);
        }
        for id in [1, 2, 5] {
            m.set_input(keys::MUSIC, id, 32767);
        }
        m.set_input(keys::REVERB, 5, 32767);
    }

    fn median(mut v: Vec<f32>) -> f32 {
        v.sort_by(f32::total_cmp);
        v[v.len() / 2]
    }

    /// The rolling bed's gain A with every input the game now writes (Jitter included), against
    /// retail's capture (upstream PR #4's transfer table: median gain A 0.06 at 5 km/h, 0.07–0.17 from
    /// 10 km/h; level(1) p90 0.24). Prints the table (`--nocapture`); asserts only the shape: Jitter
    /// lowers level(1) a little at most and never raises it.
    #[test]
    #[ignore = "needs the private install data"]
    fn bed_gain_a_with_the_full_inputs() {
        let Some((library, base)) = install() else { panic!("missing private data: no install with a MixMap") };
        let tuning = library.player_tuning();
        if tuning.jitter.is_empty() {
            panic!("missing private data: no player_tuning (stage_player_tuning.py)");
        }
        let surface = library.grain_tuning("asphalt_smooth_hard").expect("grain tuning");
        println!("km/h  level(1) no-jitter  level(1) jitter p10/p50/p90   gainA(turn 0)  gainA / gainB (turn 0.5)  level(2)");
        for kmh in [5.0f32, 10.0, 15.0, 20.0, 25.0, 30.0, 35.0, 40.0, 45.0] {
            let mut out = [Vec::new(), Vec::new()];
            let mut b_level = Vec::new();
            let mut turned = Vec::new();
            let mut turned_b = Vec::new();
            for (pass, jitter) in [false, true].into_iter().enumerate() {
                let mut m = MixMap::from_bytes(&base).unwrap();
                let mut p = PlayerAudio::new(PlayerTuning { jitter: if jitter { tuning.jitter.clone() } else { Vec::new() }, ..tuning.clone() }, false);
                let mut ti = TurnIntensity::default();
                for frame in 0..900 {
                    let s = rolling(kmh, 0.0);
                    globals(&mut m);
                    let l = p.listener([-3.5, 2.4, 0.0], [1.0, -0.3, 0.0], 1.0 / 60.0, &s);
                    p.write_inputs(&mut m, &s, Some(&l));
                    m.tick(1.0 / 60.0);
                    if frame >= 300 {
                        let a = m.level(keys::skateboard(0), 1);
                        out[pass].push(a as f32 / 32767.0);
                        if jitter {
                            b_level.push(m.level(keys::skateboard(0), 2) as f32 / 32767.0);
                            let i = ti.step(&surface, s.com_speed(), 0.5, false, 1.0);
                            let rec = board::records(&surface, &BoardInputs {
                                speed: s.ground_speed, speed_scale: None, level_a: a, level_b: m.level(keys::skateboard(0), 2),
                                pitch: 4096, turn: i, brake: 0.0, special: false, downhill: 0.0, seam: None,
                            });
                            turned.push(rec[0].gain);
                            turned_b.push(rec[1].gain);
                        }
                    }
                }
            }
            let mut j = out[1].clone();
            j.sort_by(f32::total_cmp);
            let pct = |q: f32| j[((j.len() - 1) as f32 * q) as usize];
            println!(
                "{kmh:4.0}  {:.3}               {:.3} / {:.3} / {:.3}        {:.3}          {:.3} / {:.3}            {:.3}",
                median(out[0].clone()), pct(0.1), pct(0.5), pct(0.9), median(out[1].clone()), median(turned), median(turned_b), median(b_level)
            );
            assert!(pct(0.9) <= median(out[0].clone()) + 1e-3, "Jitter never raises gain A");
        }
    }

    /// Diagnostics: the property records each player bank's voices get at open and on change
    /// (ids 0 pitch, 2 volume, 5 FXWET0 = the env send, 6/7 filters, 8 dry, ≥ 9 routing).
    #[test]
    #[ignore = "diagnostic print; needs the install"]
    fn player_voice_properties() {
        struct Log<'a> {
            inner: &'a mut skate_audio::mixer::Mixer,
            seen: std::collections::BTreeMap<(usize, u8), (i32, i32, u32)>,
            bank_of: HashMap<u32, usize>,
        }
        impl skate_audio::eval::VoiceHost for Log<'_> {
            fn open(&mut self, r: &skate_audio::eval::OpenRequest) -> Option<u32> {
                let v = self.inner.open(r)?;
                self.bank_of.insert(v, r.bank);
                for &(id, value) in r.inputs {
                    let e = self.seen.entry((r.bank, id)).or_insert((i32::MAX, i32::MIN, 0));
                    *e = (e.0.min(value), e.1.max(value), e.2 + 1);
                }
                Some(v)
            }
            fn release(&mut self, v: u32) { self.inner.release(v) }
            fn pause(&mut self, v: u32) { self.inner.pause(v) }
            fn resume(&mut self, v: u32) { self.inner.resume(v) }
            fn set(&mut self, v: u32, id: u8, value: i32) {
                if let Some(&b) = self.bank_of.get(&v) {
                    let e = self.seen.entry((b, id)).or_insert((i32::MAX, i32::MIN, 0));
                    *e = (e.0.min(value), e.1.max(value), e.2 + 1);
                }
                self.inner.set(v, id, value)
            }
            fn set_azimuth(&mut self, v: u32, value: i32) { self.inner.set_azimuth(v, value) }
            fn query(&mut self, v: u32) -> skate_audio::eval::VoiceStatus { self.inner.query(v) }
        }
        let Some((library, base)) = install() else { panic!("missing private data") };
        let (mut rt, names) = runtime(&library);
        let mut m = MixMap::from_bytes(&base).unwrap();
        let mut p = PlayerAudio::new(library.player_tuning(), true);
        let mut log = std::collections::BTreeMap::new();
        let mut bank_of = HashMap::new();
        for f in 0..600 {
            let mut s = rolling(if f < 300 { 40.0 } else { 15.0 }, 0.0);
            s.grinding = (200..400).contains(&f);
            s.grind_family = 1;
            s.grind_material = 40;
            s.brake = (450..520).contains(&f);
            globals(&mut m);
            let l = p.listener([s.com_position[0] - 3.5, 2.4, s.com_position[2]], [1.0, -0.3, 0.0], 1.0 / 60.0, &s);
            p.write_inputs(&mut m, &s, Some(&l));
            // Class_Seams on its console cadence, every frame before the pass (as `mixmap_frame`).
            p.seam_frame(&m, &s, 1.0 / 60.0, &mut rt);
            p.process(&mut m, &s, &mut rt, None, 0);
            m.tick(1.0 / 60.0);
            p.update(&m, &s, &mut rt, None, 0);
            for _ in 0..3 {
                rt.mixer.render(&mut [[0.0; skate_audio::BLOCK]; 6]);
                let Runtime { eval, mixer, .. } = &mut rt;
                let mut host = Log { inner: mixer, seen: std::mem::take(&mut log), bank_of: std::mem::take(&mut bank_of) };
                eval.block(&mut host);
                log = host.seen;
                bank_of = host.bank_of;
            }
        }
        for ((bank, id), (lo, hi, n)) in &log {
            println!("{:<18} id {id:>3}: {lo}..{hi} ({n}×)", names.get(bank).copied().unwrap_or("?"));
        }
    }

    /// A runtime with every project, the utility posted (as at boot) and the player banks.
    fn runtime(library: &super::super::library::Library) -> (Runtime, HashMap<usize, &'static str>) {
        use skate_audio::formats::{Bank, Project};
        let mut rt = Runtime::new();
        for file in &library.aems().projects {
            rt.install_project(&Project::parse(file, &library.read(file).unwrap()).unwrap());
        }
        let mut names = HashMap::new();
        for stem in std::iter::once("emitter_utility").chain(BANKS.iter().copied()) {
            let file = &library.aems().banks[stem];
            let bank = Bank::parse(stem, library.read(file).unwrap()).unwrap();
            let id = rt.load_bank(bank, library.bank_pcm(stem));
            names.insert(id, stem);
        }
        let utility = rt.eval.class_id("c_emitter_utility").unwrap();
        rt.post(utility, &[]);
        // Retail's second boot utility: the Seams program's sample shuffles (Listening test 8).
        use skate_audio::player::seams::{UTILITY, UTILITY_BANK};
        if let Some(file) = library.aems().banks.get(UTILITY_BANK) {
            let id = rt.load_bank(Bank::parse(UTILITY_BANK, library.read(file).unwrap()).unwrap(), Vec::new());
            names.insert(id, UTILITY_BANK);
            rt.post(rt.eval.class_id(UTILITY).unwrap(), &[]);
        }
        (rt, names)
    }

    struct Run {
        /// Per bank: voice gains (master × dry) over the frames a voice sounded, and opens.
        gains: HashMap<&'static str, Vec<f32>>,
        opens: HashMap<&'static str, std::collections::BTreeSet<u32>>,
        rms_db: f32,
        posts: u64,
    }

    /// Drive the player for `frames` 60 Hz frames through the real banks, MixMap and runtime.
    fn run(library: &super::super::library::Library, mxb: &[u8], frames: usize, state: impl Fn(usize) -> AudioState) -> Run {
        let (mut rt, names) = runtime(library);
        let mut m = MixMap::from_bytes(mxb).unwrap();
        let mut p = PlayerAudio::new(library.player_tuning(), true);
        let mut out = vec![0.0f32; 1600];
        let mut run = Run { gains: HashMap::new(), opens: HashMap::new(), rms_db: 0.0, posts: 0 };
        let (mut sum, mut n) = (0.0f64, 0usize);
        for f in 0..frames {
            let s = state(f);
            globals(&mut m);
            let l = p.listener([s.com_position[0] - 3.5, 2.4, s.com_position[2]], [1.0, -0.3, 0.0], 1.0 / 60.0, &s);
            p.write_inputs(&mut m, &s, Some(&l));
            // Class_Seams on its console cadence, every frame before the pass (as `mixmap_frame`).
            p.seam_frame(&m, &s, 1.0 / 60.0, &mut rt);
            p.process(&mut m, &s, &mut rt, None, 0);
            m.tick(1.0 / 60.0);
            p.update(&m, &s, &mut rt, None, 0);
            rt.fill_stereo(&mut out);
            for v in rt.mixer.snapshot() {
                if let Some(&stem) = names.get(&v.bank) {
                    run.gains.entry(stem).or_default().push(v.gain);
                    run.opens.entry(stem).or_default().insert(v.id);
                }
            }
            sum += out.iter().map(|&x| f64::from(x) * f64::from(x)).sum::<f64>();
            n += out.len();
        }
        run.rms_db = (10.0 * (sum / n.max(1) as f64).max(1e-12).log10()) as f32;
        run.posts = p.posts;
        run
    }

    fn stats(v: &[f32]) -> (f32, f32) {
        let mut v = v.to_vec();
        v.sort_by(f32::total_cmp);
        (v.get(v.len() / 2).copied().unwrap_or(0.0), v.last().copied().unwrap_or(0.0))
    }

    /// OffBoard's water splash (`player::footsteps::Splash`) through the real MixMap, Splice banks
    /// and runtime: dry, in water on the surface for a frame, under it (1197: the time in water is
    /// past 0.001 s), then the board in water (1198). Both sound (their levels come from OffBoard
    /// outputs 13 / 16, printed with 14 / 15).
    #[test]
    #[ignore = "needs the private install data"]
    fn the_water_splash_plays_through_the_real_mixmap() {
        let Some((library, mxb)) = install() else { panic!("missing private data: no install with a MixMap") };
        let (mut rt, _) = runtime(&library);
        for stem in SPLICE_BANKS {
            let Some((bank, pcm)) = library.splice_bank(stem) else { panic!("missing private data: no {stem} patch tree") };
            let r = &mut rt;
            r.splice.load_bank(stem, bank, pcm, &mut r.mixer);
        }
        let mut m = MixMap::from_bytes(&mxb).unwrap();
        let mut p = PlayerAudio::new(library.player_tuning(), true);
        p.footsteps_on = true;
        let mut out = vec![0.0f32; 1600];
        let mut energy = [0.0f64; 3];
        for f in 0..150 {
            let mut s = rolling(0.0, 0.0);
            s.in_water = f >= 30;
            s.under_water = f >= 31;
            s.board_in_water = f >= 90;
            globals(&mut m);
            let l = p.listener([s.com_position[0] - 3.5, 2.4, s.com_position[2]], [1.0, -0.3, 0.0], 1.0 / 60.0, &s);
            p.write_inputs(&mut m, &s, Some(&l));
            // Class_Seams on its console cadence, every frame before the pass (as `mixmap_frame`).
            p.seam_frame(&m, &s, 1.0 / 60.0, &mut rt);
            p.process(&mut m, &s, &mut rt, None, 0);
            m.tick(1.0 / 60.0);
            p.update(&m, &s, &mut rt, None, 0);
            rt.fill_stereo(&mut out);
            let e: f64 = out.iter().map(|&x| f64::from(x) * f64::from(x)).sum();
            energy[if f < 30 { 0 } else if f < 90 { 1 } else { 2 }] += e;
            if f == 40 {
                let k = keys::off_board(0);
                println!("OffBoard level 13 / 15 / 16 = {} / {} / {}, pitch 14 = {}", m.level(k, 13), m.level(k, 15), m.level(k, 16), m.pitch_4096(k, 14));
            }
        }
        let splash = &p.footsteps.splash;
        println!("splash starts {} (last {}), energy dry / entry / board {energy:?}", splash.starts, splash.last_id);
        assert_eq!((splash.starts, splash.last_id), (2, 1198));
        assert!(energy[1] > 10.0 * energy[0].max(1e-9) && energy[2] > 0.0, "{energy:?}");
    }

    /// The ported components through the real banks, MixMap and voice graph, headless: what they
    /// post, how many voices open and at which gain (master × dry), against the retail trace
    /// (session 20261001_211347 report: per-sample audible level = gain × send, median / max:
    /// Class_grind GRINDS ≤ 0.28 (medians 0.004–0.11), SenseOfSpeed_wind ≤ 0.54, rattle ≤ 0.28,
    /// Class_foot_drag FOOT_DRAG ≤ 0.28 (medians 0.002–0.07)). Prints the table (`--nocapture`).
    #[test]
    #[ignore = "needs the private install data"]
    fn components_play_their_retail_banks() {
        let Some((library, mxb)) = install() else { panic!("missing private data: no install with a MixMap") };
        if !BANKS.iter().all(|b| library.aems().banks.contains_key(*b)) {
            panic!("missing private data: the install lacks the player banks");
        }
        let metal = skate_audio::player::state::material_of_tag(9);
        let concrete = skate_audio::player::state::material_of_tag(3);
        let cases: Vec<(&str, &str, Box<dyn Fn(usize) -> AudioState>)> = vec![
            ("grind metal 6 m/s (family 1)", "GRINDS", Box::new(move |_| AudioState { grinding: true, grind_family: 1, grind_material: metal, ..rolling(21.6, 0.0) })),
            ("grind ledge 6 m/s (family 0)", "GRINDS", Box::new(move |_| AudioState { grinding: true, grind_family: 0, grind_material: concrete, ..rolling(21.6, 0.0) })),
            ("wind 20 km/h", "sense_of_speed", Box::new(|_| rolling(20.0, 0.0))),
            ("wind + rattle 40 km/h", "sense_of_speed", Box::new(|_| rolling(40.0, 0.0))),
            ("wind + rattle 55 km/h", "sense_of_speed", Box::new(|_| rolling(55.0, 0.0))),
            ("foot drag 4 m/s", "FOOT_DRAG", Box::new(|_| AudioState { brake: true, ..rolling(14.4, 0.0) })),
        ];
        println!("case                           bank            posts voices  gain median / max   RMS dBFS");
        for (name, bank, state) in cases {
            let r = run(&library, &mxb, 180, state);
            let (med, max) = stats(r.gains.get(bank).map_or(&[][..], |v| &v[..]));
            let voices = r.opens.get(bank).map_or(0, |s| s.len());
            println!("{name:30} {bank:15} {:5} {voices:6}  {med:.3} / {max:.3}       {:.1}", r.posts, r.rms_db);
            assert!(r.posts > 0, "{name}: nothing posted");
            assert!(voices > 0, "{name}: no voice opened in {bank}");
            assert!(max <= 1.0, "{name}: gain above unity");
        }
    }



    /// Class_Seams through the real bank and MixMap: rolling over the sidewalk pattern (11) at three
    /// speeds, the wheels moving along +x. Prints hits, voices and the Seams_Bank voice gains next to
    /// retail's Seams_Bank level (164620 report: median 0.005, p90 0.095); asserts that hits play,
    /// stay at or below unity and that the first hit's voice gain equals the later hits' (first-trigger
    /// rule: the bank is decoded at start, the same path plays every hit).
    #[test]
    #[ignore = "needs the private install data"]
    fn seams_play_their_retail_bank() {
        let Some((library, mxb)) = install() else { panic!("missing private data: no install with a MixMap") };
        let tuning = library.player_tuning();
        if tuning.seam_patterns.len() < 16 || !library.aems().banks.contains_key("Seams_Bank") {
            panic!("missing private data: no seam patterns (stage_bus_tuning.py) or Seams_Bank");
        }
        println!("pattern km/h  hits  voices  gain first / median / p90 / max");
        for (pattern, kmh) in [(11u32, 10.0f32), (11, 20.0), (11, 30.0), (4, 20.0)] {
            let v = kmh / 3.6;
            let r = run(&library, &mxb, 300, |f| {
                let x = v * f as f32 / 60.0;
                let mut s = rolling(kmh, 0.0);
                s.seam_pattern = [pattern; 4];
                s.wheel_position = [[x + 0.3, 0.0, 0.1], [x + 0.3, 0.0, -0.1], [x - 0.3, 0.0, 0.1], [x - 0.3, 0.0, -0.1]];
                s
            });
            let mut g: Vec<f32> = r.gains.get("Seams_Bank").cloned().unwrap_or_default().into_iter().filter(|&x| x > 0.0).collect();
            let first = g.first().copied().unwrap_or(0.0);
            g.sort_by(f32::total_cmp);
            let q = |p: f32| if g.is_empty() { 0.0 } else { g[((g.len() - 1) as f32 * p) as usize] };
            let voices = r.opens.get("Seams_Bank").map_or(0, |s| s.len());
            println!("{pattern:7} {kmh:4.0}  {:4}  {voices:6}  {first:.3} / {:.3} / {:.3} / {:.3}", r.posts, q(0.5), q(0.9), q(1.0));
            assert!(voices > 0, "pattern {pattern} at {kmh} km/h: no seam voice");
            assert!(q(1.0) <= 1.0);
        }
    }

    /// Retail's boot utility `Start_up_Play_ctl` (Common.abk) answers the Seams program's `rnd_call`
    /// with shuffled `send_random_0_to_9*` globals: on brick (material 65, pattern 13) at 30 km/h the
    /// hits spread over both eight-sample blocks (32–39 and 80–87, retail 180430), not one sample
    /// per block (Listening test 8).
    #[test]
    #[ignore = "needs the private install data"]
    fn seam_hits_shuffle_their_samples() {
        let Some((library, mxb)) = install() else { panic!("missing private data: no install with a MixMap") };
        let tuning = library.player_tuning();
        let banks = &library.aems().banks;
        if tuning.seam_patterns.len() < 16 || !banks.contains_key("Seams_Bank") || !banks.contains_key(skate_audio::player::seams::UTILITY_BANK) {
            panic!("missing private data: no seam patterns, Seams_Bank or Common.abk (stage_extra_banks.py Common)");
        }
        let v = 30.0f32 / 3.6;
        let (mut rt, names) = runtime(&library);
        let mut m = MixMap::from_bytes(&mxb).unwrap();
        let mut p = PlayerAudio::new(library.player_tuning(), true);
        let mut out = vec![0.0f32; 1600];
        let mut seen = std::collections::BTreeSet::new();
        let mut slots = std::collections::BTreeSet::new();
        for f in 0..300 {
            let x = v * f as f32 / 60.0;
            let mut s = rolling(30.0, 0.0);
            s.wheel_material = [65; 4];
            s.seam_pattern = [13; 4];
            s.wheel_position = [[x + 0.3, 0.0, 0.1], [x + 0.3, 0.0, -0.1], [x - 0.3, 0.0, 0.1], [x - 0.3, 0.0, -0.1]];
            globals(&mut m);
            let l = p.listener([s.com_position[0] - 3.5, 2.4, s.com_position[2]], [1.0, -0.3, 0.0], 1.0 / 60.0, &s);
            p.write_inputs(&mut m, &s, Some(&l));
            // Class_Seams on its console cadence, every frame before the pass (as `mixmap_frame`).
            p.seam_frame(&m, &s, 1.0 / 60.0, &mut rt);
            p.process(&mut m, &s, &mut rt, None, 0);
            m.tick(1.0 / 60.0);
            p.update(&m, &s, &mut rt, None, 0);
            rt.fill_stereo(&mut out);
            for vi in rt.mixer.snapshot() {
                if names.get(&vi.bank) == Some(&"Seams_Bank") && seen.insert(vi.id) {
                    slots.insert(vi.slot);
                }
            }
        }
        println!("brick 30 km/h: {} voices over samples {slots:?}", seen.len());
        for block in [32u16, 80] {
            let n = slots.iter().filter(|&&s| (block..block + 8).contains(&s)).count();
            assert!(n >= 6, "block {block}: only {n} of 8 samples in 5 s ({slots:?})");
        }
    }

    /// Diagnostic (`--ignored --nocapture`): Class_Seams voice starts per second and the samples they
    /// play, per seam pattern, on one material at 30 km/h, against retail's mixes on that material
    /// (`SEAM_MATERIAL`, default 65 = tag 66: 180430 plays 36 voices/s over streams 45, 83, 170, 80,
    /// 176, 34 …).
    #[test]
    #[ignore = "diagnostic print"]
    fn seam_samples_by_pattern() {
        let Some((library, mxb)) = install() else { panic!("missing private data: no install with a MixMap") };
        let material: u32 = std::env::var("SEAM_MATERIAL").ok().and_then(|v| v.parse().ok()).unwrap_or(65);
        let kmh = 30.0f32;
        let v = kmh / 3.6;
        for pattern in 0u32..16 {
            let (mut rt, names) = runtime(&library);
            let mut m = MixMap::from_bytes(&mxb).unwrap();
            let mut p = PlayerAudio::new(library.player_tuning(), true);
            let mut out = vec![0.0f32; 1600];
            let mut seen = std::collections::BTreeSet::new();
            let mut slots: std::collections::BTreeMap<u16, u32> = Default::default();
            let frames = 300;
            for f in 0..frames {
                let x = v * f as f32 / 60.0;
                let mut s = rolling(kmh, 0.0);
                s.wheel_material = [material; 4];
                s.seam_pattern = [pattern; 4];
                s.wheel_position = [[x + 0.3, 0.0, 0.1], [x + 0.3, 0.0, -0.1], [x - 0.3, 0.0, 0.1], [x - 0.3, 0.0, -0.1]];
                globals(&mut m);
                let l = p.listener([s.com_position[0] - 3.5, 2.4, s.com_position[2]], [1.0, -0.3, 0.0], 1.0 / 60.0, &s);
                p.write_inputs(&mut m, &s, Some(&l));
                p.process(&mut m, &s, &mut rt, None, 0);
                m.tick(1.0 / 60.0);
                p.update(&m, &s, &mut rt, None, 0);
                rt.fill_stereo(&mut out);
                for vi in rt.mixer.snapshot() {
                    if names.get(&vi.bank) == Some(&"Seams_Bank") && seen.insert(vi.id) {
                        *slots.entry(vi.slot).or_default() += 1;
                    }
                }
            }
            let (hits, _) = p.seam_hits();
            let mut top: Vec<_> = slots.into_iter().collect();
            top.sort_by(|a, b| b.1.cmp(&a.1));
            println!("pattern {pattern:2}: hits {:5.1}/s voices {:5.1}/s samples {:?}", hits as f32 / 5.0, seen.len() as f32 / 5.0, &top[..top.len().min(8)]);
        }
    }

    /// Diagnostic (`--ignored --nocapture`): which Seams_Bank samples the Class_Seams program plays
    /// for each surface word w10 and class word w13, posting one packet and toggling w7 like the
    /// hits do (every 4th frame).
    #[test]
    #[ignore = "diagnostic print"]
    fn seam_samples_by_words() {
        let Some((library, _)) = install() else { panic!("missing private data: no install with a MixMap") };
        // SEAM_SWEEP=word:value,… overrides words of the base packet (w10 1, w13 12) instead.
        let sweep: Vec<(usize, i32)> = std::env::var("SEAM_SWEEP")
            .map(|v| v.split(',').filter_map(|p| p.split_once(':')).filter_map(|(a, b)| Some((a.parse().ok()?, b.parse().ok()?))).collect())
            .unwrap_or_default();
        let grid: Vec<(i32, i32)> = if sweep.is_empty() { [0, 2, 8, 12].iter().flat_map(|&c| (0..9).map(move |s| (c, s))).collect() } else { vec![(12, 1)] };
        for (w13, w10) in grid {
            {
                let (mut rt, names) = runtime(&library);
                let class = rt.eval.class_id("Class_Seams").unwrap();
                let mut w = vec![0i32; 20];
                (w[0], w[1], w[4], w[5], w[6], w[8], w[10], w[13], w[16], w[18], w[19]) = (32767, 20000, 4096, 25000, 25000, 6000, w10, w13, 32767, 32767, 8);
                for &(i, v) in &sweep {
                    w[i] = v;
                }
                let node = rt.post(class, &w);
                let mut out = vec![0.0f32; 1600];
                let mut seen = std::collections::BTreeSet::new();
                let mut slots: std::collections::BTreeMap<u16, u32> = Default::default();
                let mut toggle = false;
                for f in 0..600 {
                    w[7] = if f % 4 == 0 {
                        toggle = !toggle;
                        if toggle { 1 } else { 2 }
                    } else {
                        0
                    };
                    rt.redeliver(node, &w);
                    rt.fill_stereo(&mut out);
                    for vi in rt.mixer.snapshot() {
                        if names.get(&vi.bank) == Some(&"Seams_Bank") && seen.insert(vi.id) {
                            *slots.entry(vi.slot).or_default() += 1;
                        }
                    }
                }
                let mut top: Vec<_> = slots.into_iter().collect();
                top.sort_by(|a, b| b.1.cmp(&a.1));
                println!("w13 {w13:2} w10 {w10}: voices {:3} samples {:?}", seen.len(), &top[..top.len().min(10)]);
            }
        }
    }

    /// SenseOfSpeed_wind's level word w7 = SenseOfSpeed level(4) at matched COM speeds against
    /// retail's capture medians (upstream PR #4's driver notes §11, ±0.5 km/h bins: 25 km/h 343,
    /// 30 km/h 730, 35 km/h 1194, 40 km/h 2017): within 1 dB.
    #[test]
    #[ignore = "needs the private install data"]
    fn wind_level_matches_the_retail_medians() {
        let Some((library, mxb)) = install() else { panic!("missing private data: no install with a MixMap") };
        for (kmh, retail) in [(25.0f32, 343.0f32), (30.0, 730.0), (35.0, 1194.0), (40.0, 2017.0)] {
            let mut m = MixMap::from_bytes(&mxb).unwrap();
            let mut p = PlayerAudio::new(library.player_tuning(), false);
            for _ in 0..120 {
                let s = rolling(kmh, 0.0);
                globals(&mut m);
                let l = p.listener([-3.5, 2.4, 0.0], [1.0, -0.3, 0.0], 1.0 / 60.0, &s);
                p.write_inputs(&mut m, &s, Some(&l));
                m.tick(1.0 / 60.0);
            }
            let w7 = m.level(keys::sense_of_speed(0), 4) as f32;
            let db = 20.0 * (w7 / retail).log10();
            println!("wind {kmh} km/h: w7 {w7} (retail median {retail}, {db:+.2} dB)");
            assert!(db.abs() < 1.0, "{kmh} km/h: {db:+.2} dB");
        }
    }
}
