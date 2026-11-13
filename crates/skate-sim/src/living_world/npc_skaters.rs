//! NPC skaters, replay tier: skate-game `living_world/npc_skaters.rs` (upstream PR #52 `618e824`)
//! without Bevy (L1d). The population's skater records become puppets riding their recorded lines.
//!
//! skate_vita: the functions are copied verbatim; the Bevy plumbing becomes plain data. An NPC
//! entity (`NpcSkater`, `NpcReplay`, `NpcFade`, `NpcStanceTrack`) is an [`NpcEntity`] in
//! [`NpcState::index`]; the systems are functions the living world calls in upstream's order
//! ([`super::LivingWorldState`]). The puppet pose is computed on each fixed step from the cursor
//! (upstream: every render frame from the cursor one tick back plus the render fraction) and drawn
//! between the last two steps' poses like the player. Not taken: the look (Bevy scene of the
//! character's roster GLB: the Vita draws the stock skater, upstream's own fallback without the
//! roster), the blended fade materials (the Vita's renderer fades), the audio (NPC board and
//! speech), the simulated tier (`npc_sim`), the client mirror, the backwards diagnostic log.
//!
//! Upstream's module docs follow.
//!
//! NPC skaters, replay tier (doc 26, milestone 3): turns the population's skater spawn / despawn
//! records into visible skaters that ride their recorded lines.
//!
//! - **Entity** per spawn record: [`NpcSkater`] (stable `LivingWorldId`, character key, slot,
//!   seed, voice) + [`NpcReplay`] (the `skate_core::living_world::replay` cursor) + `Transform`.
//!   The id map [`NpcSkaterIndex`] finds it again for the despawn.
//! - **Motion** (`FixedUpdate`, after the population step): each cursor is kept at
//!   `population tick - spawn tick` recording frames (the 60 Hz lines at the retail 60 Hz world
//!   tick, `skate_core::living_world::clock`), so the state is a function of the spawn record and the tick; branch decisions use the
//!   retail score with the players and the other NPCs and are kept as records a client would
//!   mirror. The NPC's position goes back into the population (culls, the 5 m rule). At the end
//!   of a line it continues on an unused line starting within 4 m, picked by the same retail
//!   score (`replay::choose_next_line`, `sub_8246C7F8`; fix 9), so it keeps riding until the
//!   population's 120 m cull removes it. A new NPC fades in over its first second; only at a dead
//!   end (no line starts near the end) does it fade out and leave once its opacity is below 0.2
//!   (`skate_core::living_world::leave_fade`; [`NpcFade`]).
//! - **Collision**: a kinematic proxy (capsule for the body, box for the board, infinite mass, the
//!   cursor's velocity) joins `physics::network::Proxies` like a mod's solid, so the player bumps
//!   into it. NPCs never react (replay tier).
//! - **Look**: the character's native roster GLB (`CustomModels::online_native_path`, the same
//!   files the customiser and online players use), or a mod's GLB from [`NpcSkaterLooks`], else
//!   the stock skater. Bound to the stock skeleton like a remote player (`AnimationStatus`).
//! - **Puppet animation** (`Update`): one stock clip per [`ReplayPhase`] ([`puppet_clip`]),
//!   evaluated with the player's evaluator; root = the recorded position and skater orientation.
//!   The stock graphs are not run (simplification until the simulated tier). The clip time is the
//!   time since the phase began; a looping clip (`_CYC` in its name) wraps by its length like the
//!   player's `ClipClock`, others hold their last frame. Each NPC carries [`NpcPuppetClip`] (the
//!   clip and time in effect). A mod may override the clip per phase id or `<phase>.<style>`
//!   (`LivingWorldSettings::skater_clips`, `sdk.world.set_tuning('living_world', {skater_clips})`);
//!   an override that does not evaluate falls back to the shipped pick. A phase change (also one a
//!   branch or line chain brings) crossfades from the previous phase's clip, which keeps playing,
//!   with the player's graph transition curve (`playback_transition::transition_weight`,
//!   Blend82B96058: smoothstep over the transition time) over [`RETAIL_BLEND_SECONDS`] (the stock
//!   graph's default transition time); mods set it per phase (`skater_blend_seconds`). The
//!   weight is a function of the cursor's phase frames, so a client derives the same pose.
//!   Render (fix 14): drawn from the cursor one tick back ([`NpcReplay::previous`]) towards the
//!   current one by the fixed-step fraction, like the player, with the recorded branches
//!   (`LineCursor::render_sample`); clip and blend times include the sub-frame; after a branch or
//!   chain the root blends onto the new line (`replay::LineSwitch`, `skater_line_chain.blend_seconds`).
//!   Fix 21: the crossfade nests like a retail transition (Blend82B96058 keeps the running
//!   transition as the outgoing tree): the cursor remembers the last `replay::PHASE_HISTORY` phases and
//!   the pose is built from every phase whose blend still runs ([`puppet_layers`]), so a phase
//!   change during a blend (half of all changes on the shipped lines) no longer restarts from one
//!   clip. A recorded trick slot (`EScorableID`) plays its stock trick animation on body and board
//!   (the board is the rig's `Skateboard_Root`): `<anim>_G` on the ground, `<anim>_A` once
//!   airborne, like `T_Trick.xml` ([`retail_trick_anim`]; mods: `skater_clips["trick.<id name>"]`).
//! - **Fade** (`Update`, [`present_fade`]): while [`NpcFade::alpha`] is below 1 every mesh under
//!   the NPC (body and board) draws with a per-NPC blended copy of its material; at 1 the shared
//!   materials come back and the copies are freed, so a solid NPC costs what it did before.
//! - **Audio**: [`NpcSkaterAudio`](crate::world_audio::NpcSkaterAudio) with a lite state from the
//!   cursor (position, velocity, air, ground trick as a grind) and the character's voice; #32's
//!   host picks the one audible NPC by retail's rule.
//! - **Events** for engine systems and the planned `sdk.living_world`: [`NpcSkaterEvent`].
//!
//! Multiplayer: nothing here decides a spawn; the entity is rebuilt from a `SpawnRecord`, the tick

use super::{LivingWorldSettings, PopulationState};
use crate::transform::Transform;
use glam::{Mat4, Quat, Vec3};
use skate_core::living_world::replay::{BranchContext, BranchRecord, CursorEvent, Decider, LineCursor, PhaseEntry, ReplayLine, ReplayPhase, ReplaySample, TrickRecord};
use skate_core::living_world::npc_tricks::{TrickContext, TrickProfile};
use skate_core::living_world::leave_fade::LeaveFade;
use skate_core::living_world::stance::{StanceEvents, StanceFlags};
use skate_core::living_world::{Decision, DespawnReason, Kind, LivingWorldId, SpawnChoice};
use std::collections::BTreeMap;
use std::sync::Arc;

/// The animation style of a character's native roster entry (skate-game `custom_models.rs`,
/// `native_animation_style`, verbatim): TU3 GetCACSettings 82590BE0..82590DBC; other Marquees use
/// Aggressive.
pub(crate) fn native_animation_style(key: &str) -> &'static str {
    match key {
        "danny_way" => "DannyWay",
        "mike_carroll" => "MikeCarroll",
        "pj_ladd" => "PJLadd",
        "jason_dill" => "JasonDill",
        "jerry_hsu" => "JerryHsu",
        "rob_dyrdek" => "RobDyrdek",
        _ => "Aggressive",
    }
}

/// Recording frames per world tick: the lines are recorded at 60 Hz ([data], `RECORDING_HZ`) and
/// the retail world tick that runs the skater manager is the 60 Hz fixed step
/// (`skate_core::living_world::clock::RETAIL_TICK_HZ`) [code], so one frame per tick.
pub(crate) const FRAMES_PER_TICK: u64 = 1;

/// Lines and per-character data of the loaded district, shared with the population.
#[derive(Clone, Default)]
pub(crate) struct NpcData {
    pub lines: Arc<BTreeMap<[u8; 16], ReplayLine>>,
    /// `characters_marquee` voice per character key (`skater_profiles.json`).
    pub voices: BTreeMap<String, u32>,
    /// Character record name per character key (`skater_profiles.json` `recipe`); a key without
    /// one is its own record name.
    pub records: BTreeMap<String, String>,
    /// The AI profiles' trick tables and each character's profile (`skater_profiles.json`).
    pub tricks: Arc<skate_data::living_world::SkaterTrickProfiles>,
}

/// The NPC skater trick choice in effect (`skate_core::living_world::npc_tricks`; retail: mode
/// profile, gate windows 300 / 50 frames).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct NpcTrickSettings {
    pub mode: skate_core::living_world::npc_tricks::TrickMode,
    pub params: skate_core::living_world::npc_tricks::TrickParams,
}

/// A mod's trick tables for a character or profile; `None` keeps the disc's table.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct NpcTrickTables {
    pub regular: Option<Vec<(i16, f32)>>,
    pub nollie: Option<Vec<(i16, f32)>>,
}

/// `a` is `b` or a sequence `<x>+b` (no allocation; b620).
fn sequence_ends_in(a: &str, b: &str) -> bool {
    a.len() > b.len() && a.ends_with(b) && a.as_bytes()[a.len() - b.len() - 1] == b'+'
}

/// The trick tables an NPC uses: the disc's table of its `aiprofile` (else `default`), each table
/// replaced by a mod's for the character key, else for the profile name.
pub(crate) fn npc_trick_profile<'a>(data: &'a skate_data::living_world::SkaterTrickProfiles, overrides: &BTreeMap<String, NpcTrickTables>, character: &str) -> Option<std::borrow::Cow<'a, TrickProfile>> {
    let base = data.for_character(character);
    if overrides.is_empty() {
        return base.map(std::borrow::Cow::Borrowed);
    }
    let profile_name = data.character_profile.get(character).map_or("default", String::as_str);
    let by_key = overrides.get(character);
    let by_profile = overrides.get(profile_name);
    let pick = |f: fn(&NpcTrickTables) -> &Option<Vec<(i16, f32)>>, disc: Option<&Vec<(i16, f32)>>| {
        by_key.and_then(|o| f(o).clone()).or_else(|| by_profile.and_then(|o| f(o).clone())).or_else(|| disc.cloned())
    };
    let regular = pick(|o| &o.regular, base.as_ref().map(|b| &b.regular));
    let nollie = pick(|o| &o.nollie, base.as_ref().map(|b| &b.nollie));
    (regular.is_some() || nollie.is_some()).then(|| std::borrow::Cow::Owned(TrickProfile { regular: regular.unwrap_or_default(), nollie: nollie.unwrap_or_default() }))
}

/// A trick id for logs: the catalog name, `none` for -1.
pub(crate) fn trick_name(trick: i16) -> String {
    usize::try_from(trick).ok().and_then(|i| skate_core::scoring::catalog::IDENTIFIERS.get(i)).map_or_else(|| if trick < 0 { "none".to_owned() } else { trick.to_string() }, |t| t.0.to_owned())
}

/// The shipped natural stance table (`skate_core::living_world::stance::RETAIL_TABLE`), parsed once.
pub(crate) fn stance_table() -> &'static [skate_core::living_world::stance::StanceRow] {
    static TABLE: std::sync::OnceLock<Vec<skate_core::living_world::stance::StanceRow>> = std::sync::OnceLock::new();
    TABLE.get_or_init(skate_core::living_world::stance::retail_table)
}

/// An NPC's natural stance at spawn (retail: once, from its character record, `82590DC0` ->
/// `Initialize82B97E38`): the record of the character key, a mod override, the table, else the
/// engine default. A function of the spawn record's character and the overrides, so a client
/// derives the host's value.
pub(crate) fn npc_stance(records: &BTreeMap<String, String>, overrides: &BTreeMap<String, skate_core::living_world::stance::NaturalStance>, character: &str) -> skate_core::living_world::stance::NaturalStance {
    let record = records.get(character).map_or(character, String::as_str);
    skate_core::living_world::stance::resolve(stance_table(), overrides, record)
}

/// One replay-tier NPC skater.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NpcSkater {
    pub id: LivingWorldId,
    pub character: String,
    pub slot: u8,
    pub seed: u64,
    pub voice: Option<u32>,
    pub spawn_tick: u64,
    pub start_line: [u8; 16],
    /// Natural stance, set once at spawn ([`npc_stance`]); regular draws the puppet mirrored.
    pub stance: skate_core::living_world::stance::NaturalStance,
}

#[derive(Clone, Debug)]
pub(crate) struct NpcReplay {
    pub cursor: LineCursor,
    /// Branch decisions so far (what a host would send).
    pub branches: Vec<BranchRecord>,
    /// Trick choices so far (what a host would send; a client applies them).
    pub tricks: Vec<TrickRecord>,
    pub last: Option<ReplaySample>,
    /// The cursor one world tick back: the render draws from it towards `cursor` by the fixed-step
    /// fraction (the player's previous-to-current interpolation), so it never guesses a branch.
    pub previous: Option<LineCursor>,
    /// The obstacle avoider's state: cursor lag and lateral offset ([`super::npc_avoid`]).
    pub avoid: super::npc_avoid::NpcAvoid,
    /// While simulated: the cursor frame cursor B is at (`skate_core::living_world::path_follow`, retail's projection),
    /// used instead of the recording's clock. Wire state a client needs with `branches` / `tricks` to follow a simulated
    /// NPC's cursor (its clock, and `LineCursor::hold_at_end`).
    pub follow_target: Option<u64>,
    /// The simulated body's branch gate and point, set by the sim each tick (`None` in the replay tier).
    pub body_gate: Option<skate_core::living_world::replay::BodyGate>,
}

/// The NPC's leave fade and its opacity now (1 = solid). Engine systems and mods read `alpha` to
/// draw the fade; the removal follows from it (`skate_core::living_world::leave_fade`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct NpcFade {
    pub fade: LeaveFade,
    pub alpha: f32,
}

impl Default for NpcFade {
    fn default() -> Self {
        Self { fade: LeaveFade::default(), alpha: 1.0 }
    }
}

/// What happened to an NPC skater (engine systems, mods, speech).
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum NpcSkaterEvent {
    Spawned { id: LivingWorldId, character: String, line: [u8; 16] },
    Despawned { id: LivingWorldId, reason: DespawnReason },
    Node { id: LivingWorldId, line: [u8; 16], node: u32, event: u8, flags: u8 },
    Branch { id: LivingWorldId, record: BranchRecord },
    /// The trick chosen at a start-trick node (`record.recorded` = the line's own).
    Trick { id: LivingWorldId, record: TrickRecord },
    LineEnd { id: LivingWorldId },
    /// A simulated NPC skater bailed; it is placed back on its line after `respawn_seconds`.
    Bail { id: LivingWorldId, respawn_seconds: f32 },
    /// The bail respawn placed it back on its line at `node`.
    Respawned { id: LivingWorldId, node: u32 },
    /// Mode 7: controller B (NavMeshController) starts walking the skater back to a line node, or hands back
    /// to the line controller there (`started` false).
    WalkBack { id: LivingWorldId, node: u32, started: bool },
    /// The obstacle avoider changed mode (target: obstacle kind and id).
    Avoid { id: LivingWorldId, mode: skate_core::living_world::avoid::AvoidMode, target: Option<(skate_core::living_world::avoid::ObstacleKind, u64)> },
}


/// The stock clip a replay NPC shows per phase ([data]: names checked against the decoded stock
/// clip list, `crate::living_world` data-gated test). Riding uses the pro's style like the
/// customiser (`native_animation_style`).
pub(crate) fn puppet_clip(phase: ReplayPhase, style: &str) -> &'static str {
    match phase {
        ReplayPhase::Rolling => match style {
            "Aggressive" => "R_IDLE_RIDE_AGGR_0_CYC",
            "Loose" => "R_IDLE_RIDE_LOOSE_0_CYC",
            _ => "R_IDLE_RIDE_N_0_CYC",
        },
        ReplayPhase::Crouched => "R_IDLE_LCOM_000",
        ReplayPhase::Air => "IA_IDLE_N_N_0_CYC",
        ReplayPhase::AirTrick => "IA_IDLE_LO_N_0_CYC",
        ReplayPhase::GroundTrick => "G_5050_FS_LOW_0_CYC",
        ReplayPhase::OffBoard => "BR_STAND_0_CYC",
    }
}

/// The clip in effect: a mod override for `<phase>.<style>`, then for `<phase>`, else the shipped
/// pick ([`puppet_clip`]).
pub(crate) fn resolve_puppet_clip<'a>(overrides: &'a BTreeMap<String, String>, phase: ReplayPhase, style: &str) -> &'a str {
    if overrides.is_empty() {
        return puppet_clip(phase, style);
    }
    overrides
        .get(&format!("{}.{style}", phase.name()))
        .or_else(|| overrides.get(phase.name()))
        .map_or_else(|| puppet_clip(phase, style), String::as_str)
}

/// Stock naming: cyclic clips carry `_CYC` (they loop); others play once and hold.
pub(crate) fn clip_loops(clip: &str) -> bool {
    clip.to_ascii_uppercase().contains("_CYC")
}

/// Sample time of `clip` after `time` s in its phase: wrapped by `length` when the clip loops.
pub(crate) fn puppet_clip_time(clip: &str, time: f32, length: f32) -> f32 {
    if clip_loops(clip) && length > 0.0 { time.rem_euclid(length) } else { time.max(0.0) }
}

/// Transition time (s) into a phase's clip: a `PlayAnimation` without a `time` attribute
/// transitions over the literal at `0x82099280` = 0.2 s ([code], read in
/// `graph_host::motion_nodes::transition`), and the stock motion graph's looping idle states the
/// puppet clips stand for enter with 0.2 s ([data] `air.xml` `B_AIR_CYC` time 0.2,
/// `offboard.xml` stand `BLENDSEC` 0.2, `T_handplantair.xml` `IA_IDLE_N_N_0_CYC` time 0.2).
pub(crate) const RETAIL_BLEND_SECONDS: f32 = 0.2;

/// Crossfade time into `to`'s clip: a mod's value for the phase id, then for `default`, else
/// [`RETAIL_BLEND_SECONDS`].
pub(crate) fn blend_seconds(overrides: &BTreeMap<String, f32>, to: ReplayPhase) -> f32 {
    overrides.get(to.name()).or_else(|| overrides.get("default")).copied().unwrap_or(RETAIL_BLEND_SECONDS)
}

/// Takeoff transition into a trick's ground clip: [data] `T_Trick.xml` / `T_Ollie.xml`
/// `Takeoff/FromAntic` and `FromManual` `PlayAnimation anim="$ANIM_NAME$_G" time="0.05"`.
pub(crate) const RETAIL_TRICK_TAKEOFF_SECONDS: f32 = 0.05;
/// Into a trick's air clip without its ground clip before it (a trick started in the air):
/// [data] `T_Trick.xml` `GrindOutAssist/LeftGround` `time="0.1"`. After the ground clip the air
/// clip follows as a sequence (`LeftGround/Default` `transType="sequence"`: no blend, 0 s).
pub(crate) const RETAIL_TRICK_AIR_SECONDS: f32 = 0.1;

/// The stock trick animation base a recorded trick slot plays (`<base>_G` on the ground,
/// `<base>_A` in the air), from the trick's `EScorableID` ([code] node ext +0x24, see
/// `ReplayLine::node_trick`; names `skate_core::scoring::catalog`, table 0x820862A8). Pairs are
/// [data] `MotionGraphIncludes/Tricks/Tricks.xml` (`TRICK_NAME` -> `ANIM_NAME`): the flips and
/// shuvits by name, kickflip / heelflip and their nollie forms use `B_<NAME>_IN`; numbered,
/// late and underflip variants use their base flip; a grab or any other air trick leaves the
/// ground with the ollie (`B_OLLIE`, retail grabs are performed out of an ollie). Ground-only
/// tricks (manuals, powerslides, reverts, grinds, slides; catalog classes 0 and 1) and
/// handplants (class 6) have none and keep the phase clip.
pub(crate) fn retail_trick_anim(trick: i16) -> Option<String> {
    let (name, class, _) = *skate_core::scoring::catalog::IDENTIFIERS.get(usize::try_from(trick).ok()?)?;
    /// `TRICK_NAME`s of `Tricks.xml` with a `T_Trick` / `T_TrickWithUnderflip` /
    /// `T_TrickWithDarkCatch` include (`ANIM_NAME` = `B_<NAME>`), each also as `N_<NAME>`.
    const FLIPS: [&str; 12] = ["popshuvit", "fspopshuvit", "varialkickflip", "varialheelflip", "hardflip", "inwardheelflip", "360popshuvit", "fs360popshuvit", "360flip", "laserflip", "360hardflip", "360inwardheelflip"];
    match class {
        3 => {
            let base = name.trim_end_matches("_underflip").trim_end_matches("_darkcatch").trim_end_matches(|c: char| c.is_ascii_digit());
            let plain = base.strip_prefix("n_").unwrap_or(base);
            Some(if matches!(plain, "kickflip" | "heelflip") {
                // `T_Kickflip.xml` includes: `ANIM_NAME="B_<NAME>_IN"`.
                format!("B_{}_IN", base.to_ascii_uppercase())
            } else if FLIPS.contains(&plain) {
                format!("B_{}", base.to_ascii_uppercase())
            } else if base.starts_with("n_") {
                "B_NOLLIE".to_owned()
            } else {
                // Late flips, dark catches out of other tricks: the takeoff is an ollie.
                "B_OLLIE".to_owned()
            })
        }
        4 if name == "nollie" => Some("B_NOLLIE".to_owned()),
        2 | 4 => Some("B_OLLIE".to_owned()),
        _ => None,
    }
}

/// The trick animation in effect for a recorded trick: a mod's `skater_clips["trick.<name>"]`
/// (the catalog's identifier, e.g. `trick.kickflip`; value = an animation base without `_G`/`_A`),
/// else [`retail_trick_anim`].
pub(crate) fn resolve_trick_anim(overrides: &BTreeMap<String, String>, trick: i16) -> Option<String> {
    let name = skate_core::scoring::catalog::IDENTIFIERS.get(usize::try_from(trick).ok()?)?.0;
    overrides.get(&format!("trick.{name}")).cloned().or_else(|| retail_trick_anim(trick))
}

/// The clip a remembered phase shows and its transition time (`older` = the phase before it):
/// a trick span with a trick animation plays `<base>_G` then `<base>_A` from the ground (0.05 s
/// in), or `<base>_A` alone when it starts in the air (0.1 s in); the air after an air trick keeps
/// the clip (one layer); everything else the phase's puppet clip with its blend time. A clip
/// that does not evaluate (`evaluates` false) falls back to the shipped phase pick; trick names
/// go through `stock` (authored tree name -> playable clip, [`stock_tree_leaf`]).
pub(crate) fn puppet_layer_clip(
    clips: &BTreeMap<String, String>,
    blends: &BTreeMap<String, f32>,
    style: &str,
    evaluates: &dyn Fn(&str) -> bool,
    stock: &dyn Fn(&str) -> Option<String>,
    scored: &dyn Fn(PhaseEntry, bool) -> Option<(String, f32)>,
    e: PhaseEntry,
    older: Option<PhaseEntry>,
) -> (String, f32) {
    let phase_clip = || {
        let clip = resolve_puppet_clip(clips, e.phase, style);
        let clip = if evaluates(clip) { clip } else { puppet_clip(e.phase, style) };
        (clip.to_owned(), blend_seconds(blends, e.phase))
    };
    let tuned = |key: &str, retail: f32| blends.get(key).copied().unwrap_or(retail);
    // Each `+` part through `stock` (authored tree name -> clip); `None` when any part is missing.
    let trick_clip = |entry: PhaseEntry, suffix: &str| {
        let base = resolve_trick_anim(clips, entry.trick)?;
        let names = if suffix == "_A" { trick_air_sequence(&base, entry.trick) } else { format!("{base}{suffix}") };
        names.split('+').map(stock).collect::<Option<Vec<_>>>().map(|v| v.join("+"))
    };
    // A grab / grind slot plays the trees of the stock graph state it scores, unless a mod names its animation.
    let scored = |entry: PhaseEntry, landed: bool| {
        let name = skate_core::scoring::catalog::IDENTIFIERS.get(usize::try_from(entry.trick).ok()?)?.0;
        if !clips.is_empty() && clips.contains_key(&format!("trick.{name}")) { None } else { scored(entry, landed) }
    };
    // The span's air phase before this ground one: the trick landed still open (no new takeoff).
    let air_before = older.filter(|o| e.phase == ReplayPhase::GroundTrick && o.phase == ReplayPhase::AirTrick && o.trick == e.trick && o.trick_since == e.trick_since);
    if let Some(c) = match e.phase {
        // Landed with the span open: the scored hold, else the air clip continues (one layer).
        ReplayPhase::GroundTrick if air_before.is_some() => scored(e, true).or_else(|| air_before.and_then(|o| scored(o, false)).map(|(c, _)| (c, 0.0))),
        ReplayPhase::GroundTrick | ReplayPhase::AirTrick => scored(e, false),
        ReplayPhase::Air => older.filter(|o| o.phase == ReplayPhase::AirTrick).and_then(|o| scored(o, false)).map(|(c, _)| (c, 0.0)),
        _ => None,
    } {
        return c;
    }
    match e.phase {
        // The takeoff: `_G`, then the air part as a sequence (`LeftGround/Default`
        // `transType="sequence"` starts it when the ground clip ends, not when the recorder's
        // airborne flag comes). [`puppet_layers`] continues this clip through the air phases.
        ReplayPhase::GroundTrick => match (trick_clip(e, "_G"), trick_clip(e, "_A")) {
            (Some(g), Some(a)) => (format!("{g}+{a}"), tuned("trick_takeoff", RETAIL_TRICK_TAKEOFF_SECONDS)),
            _ => phase_clip(),
        },
        // Without its takeoff before it (a trick started in the air): the air part, 0.1 s in.
        ReplayPhase::AirTrick => trick_clip(e, "_A").map_or_else(phase_clip, |c| (c, tuned("trick_air", RETAIL_TRICK_AIR_SECONDS))),
        // The air after an air trick keeps the trick's clip (merged into its layer).
        ReplayPhase::Air => older.filter(|o| o.phase == ReplayPhase::AirTrick).and_then(|o| trick_clip(o, "_A")).map_or_else(phase_clip, |c| (c, 0.0)),
        _ => phase_clip(),
    }
}

/// The air part of a trick animation base: `<base>_A`, and for the kickflip family (`T_Kickflip.xml`:
/// `LeftGround` plays `$ANIM_NAME$_A`, then as sequences `$ANIM_CYC_NAME$1..3` per extra flip and
/// `$ANIM_OUT_NAME$<n>`) `B_<FLIP>_IN_A`, `B_<FLIP>_CYC1..n`, `B_<FLIP>_OUT<n>` with n the
/// trick's flip count (`kickflip2` = 2; 1 without a digit). Parts are joined with `+`: the
/// puppet plays them back to back (`transType="sequence"`, no blend).
pub(crate) fn trick_air_sequence(base: &str, trick: i16) -> String {
    let Some(flip) = base.strip_suffix("_IN") else { return format!("{base}_A") };
    let name = usize::try_from(trick).ok().and_then(|t| skate_core::scoring::catalog::IDENTIFIERS.get(t)).map_or("", |e| e.0);
    let digits = name.trim_end_matches("_underflip").trim_end_matches("_darkcatch");
    let n: usize = digits.chars().last().and_then(|c| c.to_digit(10)).map_or(1, |d| d as usize).clamp(1, 4);
    let mut parts = vec![format!("{base}_A")];
    parts.extend((1..=n.min(3)).map(|i| format!("{flip}_CYC{i}")));
    parts.push(format!("{flip}_OUT{n}"));
    parts.join("+")
}

/// The clip a stock authored tree plays for an NPC: a clip is itself, a selector its default
/// child, a `TRICKHEIGHT` phase blend the child retail's AI height picks (the perform step always posts TrickHeight
/// 1.0 normalized, `sub_82469448` -> `sub_82470DD8`, research b690: the highest child value), any other phase blend
/// its first child. Blend and selection spaces: none.
pub(crate) fn stock_tree_leaf(meta: &skate_data::animation_metadata::AnimationMetadata, name: &str) -> Option<String> {
    use skate_data::animation_metadata::TreeMetadata;
    let mut name = name.to_owned();
    for _ in 0..8 {
        name = match meta.tree(&name).ok()? {
            TreeMetadata::Clip(c) => return Some(c.name.clone()),
            TreeMetadata::Selector(s) => s.default.clone(),
            TreeMetadata::PhaseBlend(p) if p.parameter.eq_ignore_ascii_case("TRICKHEIGHT") => highest_child(meta, &p.children, &p.parameter).or_else(|| p.children.first().cloned())?,
            TreeMetadata::PhaseBlend(p) => p.children.first()?.clone(),
            _ => return None,
        };
    }
    None
}

/// The phase blend child with the highest `parameter` value (`PhaseBlend` sorts its children by the value each child
/// reports, `phase_blend.rs`); `None` when a child is not a clip or carries no value.
fn highest_child(meta: &skate_data::animation_metadata::AnimationMetadata, children: &[String], parameter: &str) -> Option<String> {
    use skate_data::animation_metadata::TreeMetadata;
    let mut best: Option<(f32, &String)> = None;
    for child in children {
        let TreeMetadata::Clip(c) = meta.tree(child).ok()? else { return None };
        let value = f32::from_bits(*c.attributes.iter().find(|a| a.name.eq_ignore_ascii_case(parameter))?.payload_words.first()?);
        if best.is_none_or(|(v, _)| value > v) {
            best = Some((value, child));
        }
    }
    best.map(|(_, c)| c.clone())
}

/// [`stock_tree_leaf`] with attribute targets: a blend space, or a phase blend over a `targets` parameter, takes the
/// child nearest the target ([`nearest_child`]); everything else as [`stock_tree_leaf`].
pub(crate) fn stock_tree_leaf_at(meta: &skate_data::animation_metadata::AnimationMetadata, name: &str, targets: &[(&str, f32)]) -> Option<String> {
    use skate_data::animation_metadata::TreeMetadata;
    let target = |p: &str| targets.iter().find(|(n, _)| n.eq_ignore_ascii_case(p)).map(|t| t.1);
    let mut name = name.to_owned();
    for _ in 0..8 {
        let (parameters, children): (Vec<&str>, &[String]) = match meta.tree(&name).ok()? {
            TreeMetadata::BlendSpace(b) => (b.parameters.iter().map(String::as_str).collect(), &b.children),
            TreeMetadata::PhaseBlend(p) if target(&p.parameter).is_some() => (vec![p.parameter.as_str()], &p.children),
            _ => return stock_tree_leaf(meta, &name),
        };
        let goal: Vec<f32> = parameters.iter().map(|p| target(p).unwrap_or(0.0)).collect();
        name = nearest_child(meta, children, &parameters, &goal)?;
    }
    None
}

/// The child whose clip values of `parameters` lie nearest `goal`; when a child carries none (a sub-tree), the first
/// parameter picks by rank: the middle child for 0, the first below it, the last above.
fn nearest_child(meta: &skate_data::animation_metadata::AnimationMetadata, children: &[String], parameters: &[&str], goal: &[f32]) -> Option<String> {
    use skate_data::animation_metadata::TreeMetadata;
    let value = |child: &str, p: &str| match meta.tree(child).ok()? {
        TreeMetadata::Clip(c) => Some(f32::from_bits(*c.attributes.iter().find(|a| a.name.eq_ignore_ascii_case(p))?.payload_words.first()?)),
        _ => None,
    };
    let mut best: Option<(f32, &String)> = None;
    for child in children {
        let Some(d) = parameters.iter().zip(goal).map(|(p, g)| value(child, p).map(|v| (v - g).powi(2))).sum::<Option<f32>>() else {
            best = None;
            break;
        };
        if best.is_none_or(|(b, _)| d < b) {
            best = Some((d, child));
        }
    }
    if let Some((_, c)) = best {
        return Some(c.clone());
    }
    let g = *goal.first()?;
    let i = if g == 0.0 { children.len() / 2 } else if g < 0.0 { 0 } else { children.len().checked_sub(1)? };
    children.get(i).cloned()
}

/// Grab slots the stock graphs score as another grab: the AI posts tweaks and a grab hand (board adjust only for
/// mute / seatbelt), so e.g. nosegrab plays the BS grab tweaked down. Proven against the stock graphs by
/// `air_playback.rs` `ai_grab_schedules_through_stock_graphs`: docs/hails-additions/26c-living-world-npc-skaters.md#npc-grab-scores
pub(crate) const RETAIL_GRAB_SCORES: [(&str, &str); 33] = [
    ("gnd_bsgrab", "bsgrab"), ("gnd_dblgrab", "dblgrab"), ("gnd_fsgrab", "fsgrab"),
    ("mutegrab_down", "bsgrab_down"), ("mutegrab_left", "bsgrab_left"), ("mutegrab_up", "bsgrab_up"),
    ("nosegrab", "bsgrab_down"), ("nosegrab_left", "bsgrab_left"), ("nosegrab_right", "bsgrab_down"),
    ("rocketair", "dblgrab"),
    ("stalegrab", "fsgrab_right"), ("stalegrab_down", "fsgrab_down"), ("stalegrab_left", "fsgrab_left"), ("stalegrab_up", "fsgrab_up"),
    ("tailgrab", "bsgrab_up"), ("tailgrab_left", "bsgrab_left"), ("tailgrab_right", "bsgrab_up"),
    ("fs_onefoot_right", "fsgrab"), ("fs_backfoot", "fsgrab"), ("bs_onefoot_front", "bsgrab"), ("bs_onefoot_behind", "bsgrab"),
    ("nosegrab_onefoot_front", "bs_onefoot_front"), ("nosegrab_onefoot_behind", "bs_onefoot_behind"),
    ("tailgrab_onefoot_front", "bs_onefoot_behind"), ("tailgrab_onefoot_behind", "bs_onefoot_front"),
    ("tailgrab_airwalk", "christ_air"), ("nosegrab_airwalk", "christ_air"),
    ("crailgrab", "fsgrab_down"), ("crailgrab_left", "fsgrab_left"), ("crailgrab_right", "fsgrab_down"),
    ("seatbeltgrab", "bsgrab"), ("seatbeltgrab_left", "bsgrab"), ("seatbeltgrab_right", "bsgrab"),
];

/// Grabs the ground graph plays (`GroundGrabs/*.xml`): on the ground they keep their own score.
const GROUND_GRABS: [&str; 4] = ["gnd_bsgrab", "gnd_dblgrab", "gnd_fsgrab", "coffin"];

/// The score a grab slot reaches: on the ground a ground grab's own, else [`RETAIL_GRAB_SCORES`], else its own name.
pub(crate) fn retail_grab_score(name: &str, on_ground: bool) -> &str {
    if on_ground && GROUND_GRABS.contains(&name) {
        return name;
    }
    RETAIL_GRAB_SCORES.iter().find(|(n, _)| *n == name).map_or(name, |(_, s)| s)
}

/// Clip sequence and blend time of a grab (category 4) or grind / slide (category 5) slot in `e`'s phase, from the
/// stock motion graph's score index (`leaf`: tree + attribute targets -> playable clip):
/// docs/hails-additions/26c-living-world-npc-skaters.md#npc-grab-scores
pub(crate) fn scored_trick_clip(
    scores: &BTreeMap<String, crate::graph_host::motion::ScoreClips>,
    leaf: &dyn Fn(&str, &[(&str, f32)]) -> Option<String>,
    grind_disttocog: f32,
    manual_angle: &dyn Fn(f32) -> f32,
    e: PhaseEntry,
    landed: bool,
) -> Option<(String, f32)> {
    let (name, _, category) = *skate_core::scoring::catalog::IDENTIFIERS.get(usize::try_from(e.trick).ok()?)?;
    match (category, e.phase) {
        // An air grab still open at touchdown: the FS / BS / DBL Cyc takes the ground grab hold
        // (`T_FSBSGrab.xml` / `DBLGrab.xml` Cyc -> `OnGround...Grabbing.<X>.<X>Cyc`, the AI posting Ground<X>Grab on the
        // first ground tick, research b710); mute / stale have none: docs/hails-additions/26c-living-world-npc-skaters.md#npc-grab-touchdown
        (4, ReplayPhase::GroundTrick) if landed => {
            let air = retail_grab_score(name, false);
            let family = ["fsgrab", "bsgrab", "dblgrab"].into_iter().find(|f| air.starts_with(f))?;
            let c = scores.get(&format!("gnd_{family}"))?;
            Some((leaf(&c.cyc, &[("ANGLE", 0.0)])?, c.cyc_seconds?))
        }
        (4, ReplayPhase::GroundTrick | ReplayPhase::AirTrick) => {
            let on_ground = e.phase == ReplayPhase::GroundTrick && GROUND_GRABS.contains(&name);
            let c = scores.get(retail_grab_score(name, on_ground))?;
            // ScoringGrabs' direction windows (`select_grab_score`): the unit tweak of the score's direction.
            let (x, y) = match c.direction {
                Some("up") => (0.0, 1.0),
                Some("left") => (-1.0, 0.0),
                Some("down") => (0.0, -1.0),
                Some("right") => (1.0, 0.0),
                _ => (0.0, 0.0),
            };
            let targets = [("TWEAK_X", x), ("TWEAK_Y", y), ("ANGLE", 0.0)];
            // The Into at its state's `playBackSpeed` ([`part_speed`]).
            let into = match c.into.as_deref() {
                Some(t) => Some(with_speed(leaf(t, &targets)?, c.into_speed)),
                None => None,
            };
            let grab = into.into_iter().chain(Some(leaf(&c.cyc, &targets)?)).collect::<Vec<_>>().join("+");
            Some(match e.phase {
                // `GroundGrabs/T_Grab.xml` Into `time="0.1"`.
                ReplayPhase::GroundTrick if on_ground => (grab, 0.1),
                // An air grab's takeoff is the ollie's `_G`; the grab follows as a sequence.
                ReplayPhase::GroundTrick => (format!("{}+{grab}", leaf("B_OLLIE_G", &[])?), RETAIL_TRICK_TAKEOFF_SECONDS),
                _ => (grab, RETAIL_TRICK_AIR_SECONDS),
            })
        }
        // `T_Grind.xml` `PlayAnimation time="0.3"`; neutral twist (ours), the uncrouched rest height:
        // docs/hails-additions/26c-living-world-npc-skaters.md#npc-grind-height
        (5, ReplayPhase::GroundTrick) => Some((leaf(&scores.get(name)?.cyc, &[("TWIST", 0.0), ("DISTTOCOG", grind_disttocog)])?, 0.3)),
        // Manuals rolling forwards (`Manual/NoseManual.xml` Into 0.1 then `B_NOSE_MANUAL` 0.3 with SetManualAngle;
        // `TailManual.xml` `B_TAIL_MANUAL_LOW` 0.3 unless rolling backwards): docs/hails-additions/26c-living-world-npc-skaters.md#npc-manuals
        (8, ReplayPhase::GroundTrick) => {
            let value = skate_core::living_world::ai_signals::SignalSettings::default().manual_value;
            let targets = [("MANUAL_ANGLE", manual_angle(value)), ("DISTTOCOG", 1.0e6), ("ANGLE", 0.0)];
            match name {
                "nosemanual" => Some((format!("{}+{}", leaf("B_NOSE_MANUAL_INTO", &targets)?, leaf("B_NOSE_MANUAL", &targets)?), 0.1)),
                "tailmanual" => Some((leaf("B_TAIL_MANUAL_LOW", &targets)?, 0.3)),
                _ => None,
            }
        }
        _ => None,
    }
}

/// The stock motion graph's score index ([`crate::graph_host::motion::stock_score_clips`]), built once from the loaded
/// graph so a graph mod changes NPC grabs like the player's; empty = the shipped picks.
#[derive(Clone, Debug, Default)]
pub(crate) struct NpcScoreClips(pub BTreeMap<String, crate::graph_host::motion::ScoreClips>);

/// One clip of the puppet's nested crossfade: `weight` is how far it has blended in over
/// everything older (the oldest layer is the base, its weight is unused).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PuppetLayer {
    pub clip: String,
    /// Seconds since the clip began (its phase start, or the first of the phases it continues).
    pub time: f32,
    pub weight: f32,
    /// Cursor frame the layer began (its phase entry's `since`): the layer's identity, which
    /// keys the stance bits it was built with ([`NpcStanceTrack`]). 0 where no cursor drives it.
    pub since: u64,
    /// An air grab's `Cyc` tree, played in place of the sequence's last clip ([`grab_tree`]).
    pub tree: Option<PuppetTree>,
}

/// An air grab layer's `Cyc` tweak tree: docs/hails-additions/26c-living-world-npc-skaters.md#npc-grab-tweaks
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PuppetTree {
    pub name: String,
    /// Seconds into the tree.
    pub time: f32,
    /// `tweak_x` / `tweak_y` unmirrored (a mirrored layer negates x: `air.xml` TweakX `mirrorFilter="negate"`).
    pub tweak: [f32; 2],
}

/// Ticks [`grab_tweak`] eases over at most (the filter has long settled).
const TWEAK_TICKS: u64 = 600;

/// The `Cyc` state's tweaks `ticks` (fractional) after its entry: TweakX / TweakY from the slot `schedule` at slot tick
/// `entry_tick + m`, eased per 60 Hz tick by the state's `FilterMotionGraphIntent`s from Begin (no stored state, so a
/// scrub or a late peer gets the same value), then `TweakProject`'s clamp. From slot tick `end` (the slot is over) the
/// AG copy is gone and the filter eases out: docs/hails-additions/26c-living-world-npc-skaters.md#npc-grab-tweaks
pub(crate) fn grab_tweak(filter: &crate::graph_host::motion::TweakFilter, schedule: &[skate_core::living_world::ai_signals::IntentTuple], entry_tick: i64, ticks: f32, end: Option<i64>) -> [f32; 2] {
    let ticks = ticks.max(0.0);
    let n = (ticks.floor() as u64).min(TWEAK_TICKS);
    let mut steps = tweak_steps(filter, schedule, entry_tick, end);
    let now = steps.nth(n as usize).unwrap_or([0.0; 2]);
    let next = steps.next().unwrap_or(now);
    let a = if n == TWEAK_TICKS { 0.0 } else { ticks.fract() };
    let mut v = [now[0] + (next[0] - now[0]) * a, now[1] + (next[1] - now[1]) * a];
    let total = v[0].abs() + v[1].abs();
    if filter.project && total > 1.0 {
        v = [v[0] / total, v[1] / total];
    }
    v
}

/// The filtered (TweakX, TweakY) after each Cyc tick 0, 1, ... (Begin, then one update per tick; see [`grab_tweak`]).
fn tweak_steps<'a>(filter: &'a crate::graph_host::motion::TweakFilter, schedule: &'a [skate_core::living_world::ai_signals::IntentTuple], entry_tick: i64, end: Option<i64>) -> impl Iterator<Item = [f32; 2]> + 'a {
    use skate_core::animation::intent_filter::State;
    // `slot_signals`: the first 6 tuples, present from start to end (end < 0 = open); the AG copy defaults to 0.
    let input = move |name: &str, t: i64| {
        if end.is_some_and(|e| t >= e) {
            return None;
        }
        Some(schedule.iter().take(6).filter(|e| skate_core::input::skater_intent::name(e.intent) == Some(name) && t >= i64::from(e.start) && (e.end < 0 || t <= i64::from(e.end))).last().map_or(0.0, |e| e.value))
    };
    let (mut x, mut y) = (State::default(), State::default());
    x.begin(&filter.x);
    y.begin(&filter.y);
    (0..=TWEAK_TICKS + 1).map(move |m| {
        let t = entry_tick + m as i64;
        [x.update(&filter.x, input("TweakX", t), 1.0 / 60.0, (false, false)), y.update(&filter.y, input("TweakY", t), 1.0 / 60.0, (false, false))]
    })
}

/// The Cyc tick the Out begins: the first update after the slot's `end` whose filtered tweak is below the release
/// level ([`crate::graph_host::motion::Release`]) moves the graph on the next update (research b708).
pub(crate) fn grab_release_tick(filter: &crate::graph_host::motion::TweakFilter, schedule: &[skate_core::living_world::ai_signals::IntentTuple], entry_tick: i64, end: i64) -> Option<u64> {
    let r = filter.release?;
    let from = (end - entry_tick).max(0) as usize;
    tweak_steps(filter, schedule, entry_tick, Some(end)).enumerate().skip(from).find(|(_, [x, y])| if r.y_only { y.abs() < r.below } else { x.hypot(*y) < r.below }).map(|(m, _)| m as u64 + 1)
}

/// The tweak tree of an air grab layer the score index plays (no mod clip): the `Cyc` tree once the sequence reaches
/// its last part, with [`grab_tweak`] from the Cyc entry (the Into's end).
/// What follows an air grab layer: the Out and the air pose after it (inserted after the grab while they began before
/// the next layer), or at a touchdown with the Cyc still active the grab landing in place of the touchdown layer (with
/// the seconds it plays before that layer takes over).
#[derive(Default)]
struct GrabFollow {
    after: Vec<PuppetLayer>,
    landing: Option<(PuppetLayer, f32)>,
}

fn grab_tree(clips: &BTreeMap<String, String>, scores: &NpcScoreClips, skater: &crate::physics::SkaterRuntime, history: &[(PhaseEntry, f32)], l: &PuppetLayer, air: (&str, f32)) -> Option<(PuppetTree, GrabFollow)> {
    let i = history.iter().position(|(e, _)| e.since == l.since)?;
    let e = match history[i].0 {
        e if e.phase == ReplayPhase::Air => history.get(i + 1).map(|h| h.0).filter(|o| o.phase == ReplayPhase::AirTrick)?,
        e => e,
    };
    let (name, _, category) = *skate_core::scoring::catalog::IDENTIFIERS.get(usize::try_from(e.trick).ok()?)?;
    if category != 4 || !matches!(e.phase, ReplayPhase::GroundTrick | ReplayPhase::AirTrick) || (!clips.is_empty() && clips.contains_key(&format!("trick.{name}"))) {
        return None;
    }
    let c = scores.0.get(retail_grab_score(name, e.phase == ReplayPhase::GroundTrick && GROUND_GRABS.contains(&name)))?;
    let filter = c.tweak.as_ref()?;
    let (body, last) = l.clip.rsplit_once('+').unwrap_or(("", &l.clip));
    let evaluator = &skater.animation.evaluator;
    let prefix = if body.is_empty() { 0.0 } else { body.split('+').map(|p| part_speed(p)).map(|(p, speed)| evaluator.clip_length(p).ok().map(|l| l / speed)).sum::<Option<f32>>()? };
    let last = part_speed(last).0;
    let time = l.time - prefix;
    if time < 0.0 || !tree_holds(skater.animation.motion.animation.metadata(), &c.cyc, last, 4) {
        return None;
    }
    let schedule = skater.scoring.data.by_id(usize::try_from(e.trick).ok()?).map_or(&[][..], |d| d.intents.as_slice());
    let entry = (prefix * 60.0).ceil();
    let entry_tick = l.since as i64 + entry as i64 - e.trick_since as i64;
    // The span's end (END_TRICK: the slot and the grab intent end), from the first newer phase outside it.
    let end = history[..i].iter().rev().map(|h| h.0).find(|h| h.since > e.since && !matches!(h.phase, ReplayPhase::AirTrick | ReplayPhase::GroundTrick));
    let end_tick = end.map(|h| h.since as i64 - e.trick_since as i64);
    let tree = PuppetTree { name: c.cyc.clone(), time, tweak: grab_tweak(filter, schedule, entry_tick, l.time * 60.0 - entry, end_tick) };
    let meta = skater.animation.motion.animation.metadata();
    let weight = skate_core::animation::playback_transition::transition_weight;
    let mut follow = GrabFollow::default();
    // Released in the air: the Out at its speed once the eased tweak drops below the release level, then the air pose
    // (Out -> InAir on `WillExpire`).
    let release = end.filter(|h| h.phase == ReplayPhase::Air).and_then(|_| Some(entry + grab_release_tick(filter, schedule, entry_tick, end_tick?)? as f32));
    if let Some(start) = release {
        let out_time = (l.time * 60.0 - start) / 60.0;
        let out = c.out.as_deref().and_then(|o| stock_tree_leaf_at(meta, o, &[("MAGNITUDE", 0.0)]));
        if let (true, Some(out), Some(blend)) = (out_time >= 0.0, out, c.out_seconds) {
            let length = skater.animation.evaluator.clip_length(&out).ok().map(|n| n / c.out_speed.unwrap_or(1.0));
            follow.after.push(PuppetLayer { clip: with_speed(out, c.out_speed), time: out_time, weight: weight(out_time, blend), since: l.since + start as u64, tree: None });
            if let (Some(length), Some(expire)) = (length, c.out_expire) {
                let air_time = out_time - (length - expire).max(0.0);
                if air_time >= 0.0 {
                    let since = l.since + start as u64 + ((length - expire).max(0.0) * 60.0) as u64;
                    follow.after.push(PuppetLayer { clip: air.0.to_owned(), time: air_time, weight: weight(air_time, air.1), since, tree: None });
                }
            }
        }
    }
    // Touchdown with the Cyc still active (the span ended, no Out yet, past the Into): the grab landing; a tweak at the
    // WipeOut level never reaches touchdown on the shipped lines (survey `living_world_npc_grab_touchdowns_on_the_shipped_lines`).
    let touch = history[..i].iter().rev().find(|h| h.0.since > e.since && matches!(h.0.phase, ReplayPhase::Rolling | ReplayPhase::Crouched));
    if let (Some(&(t, seconds)), Some(g), Some(_)) = (touch, c.landing.as_ref(), end) {
        let at = t.since as f32 - l.since as f32;
        if at >= entry && release.is_none_or(|r| r > at) {
            if let Some(clip) = stock_tree_leaf_at(meta, &g.tree, &[]) {
                let length = skater.animation.evaluator.clip_length(&clip).unwrap_or(0.0);
                follow.landing = Some((PuppetLayer { clip, time: seconds, weight: weight(seconds, g.seconds), since: t.since, tree: None }, (length - g.expire).max(0.0)));
            }
        }
    }
    Some((tree, follow))
}

/// Whether `clip` is `tree` or one of its blend children (`depth` levels down).
pub(crate) fn tree_holds(meta: &skate_data::animation_metadata::AnimationMetadata, tree: &str, clip: &str, depth: u8) -> bool {
    use skate_data::animation_metadata::TreeMetadata;
    if tree.eq_ignore_ascii_case(clip) {
        return true;
    }
    let children = match meta.tree(tree) {
        Ok(TreeMetadata::BlendSpace(b)) if depth > 0 => b.children.clone(),
        Ok(TreeMetadata::PhaseBlend(p)) if depth > 0 => p.children.clone(),
        _ => return false,
    };
    children.iter().any(|c| tree_holds(meta, c, clip, depth - 1))
}

/// The puppet's layers, oldest first, from the cursor's phases (`history`: newest first, seconds
/// since each began). `resolve(entry, older)` gives a phase's clip and its transition time.
/// Consecutive phases showing the same clip are one layer (the clip keeps playing from the older
/// start: a trick's air clip runs on after its span closes). Layers stop at the first one fully
/// blended in: older ones no longer show. Pure: same history, same layers.
pub(crate) fn puppet_layers(history: &[(PhaseEntry, f32)], resolve: impl Fn(PhaseEntry, Option<PhaseEntry>) -> (String, f32)) -> Vec<PuppetLayer> {
    let mut out = Vec::new();
    puppet_layers_into(history, resolve, &mut out);
    out
}

/// [`puppet_layers`] into `out` (cleared first); each history entry is resolved once, in order.
pub(crate) fn puppet_layers_into(history: &[(PhaseEntry, f32)], resolve: impl Fn(PhaseEntry, Option<PhaseEntry>) -> (String, f32), out: &mut Vec<PuppetLayer>) {
    out.clear();
    let item = |i: usize| {
        let (e, t) = history[i];
        let (clip, seconds) = resolve(e, history.get(i + 1).map(|h| h.0));
        (clip, seconds, t, e.since)
    };
    let mut next = (!history.is_empty()).then(|| item(0));
    let mut j = 0;
    while let Some(mut current) = next.take() {
        // An older phase whose clip is this one, or a sequence ending in it (`<ground>+<air>`
        // before `<air>`), is the same playing clip: continue it from the older start.
        while j + 1 < history.len() {
            let older = item(j + 1);
            j += 1;
            if older.0 == current.0 || sequence_ends_in(&older.0, &current.0) {
                current = older;
            } else {
                next = Some(older);
                break;
            }
        }
        let (clip, seconds, time, since) = current;
        let weight = skate_core::animation::playback_transition::transition_weight(time, seconds);
        out.push(PuppetLayer { clip, time, weight, since, tree: None });
        if weight >= 1.0 {
            break;
        }
    }
    out.reverse();
}

/// An NPC's crossfade out of the previous phase's clip (the outgoing tree of a graph transition).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PuppetBlend {
    pub from: String,
    /// Time since the previous phase began: the outgoing clip keeps playing (transition matching 0).
    pub from_time: f32,
    /// Weight of the incoming clip (0 = all `from`, 1 = done).
    pub weight: f32,
}

/// [`puppet_blend`] at fractional times (s since each phase began): the render passes
/// `(frames + sub_frame) / 60`, so the weight and the outgoing clip move between ticks.
pub(crate) fn puppet_blend_at(from: &str, from_time: f32, to: &str, to_time: f32, seconds: f32) -> Option<PuppetBlend> {
    if from == to {
        return None;
    }
    let weight = skate_core::animation::playback_transition::transition_weight(to_time, seconds);
    (weight < 1.0).then(|| PuppetBlend { from: from.to_owned(), from_time, weight })
}

/// The crossfade in effect `to_frames` 60 Hz frames into the new phase, `None` once it is done or
/// when both phases show the same clip (nothing to blend).
pub(crate) fn puppet_blend(from: &str, from_frames: u64, to: &str, to_frames: u64, seconds: f32) -> Option<PuppetBlend> {
    puppet_blend_at(from, from_frames as f32 / 60.0, to, to_frames as f32 / 60.0, seconds)
}

/// The clip one NPC shows now and the time since its phase began (render side, rewritten every
/// frame from the replay; deterministic from the replay cursor).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NpcPuppetClip {
    pub phase: ReplayPhase,
    pub clip: String,
    pub time: f32,
    /// The crossfade from the previous layer's clip while it runs.
    pub blend: Option<PuppetBlend>,
    /// Every clip in the pose, oldest first ([`puppet_layers`]); the last one is `clip`.
    pub layers: Vec<PuppetLayer>,
    /// Whether the last frame posed the skeleton from it (false until the look is bound).
    pub posed: bool,
    /// Retail's fakie overlay while it shows ([`fakie_channel_layer`]): the channel tree, its time
    /// and its channel weight.
    pub fakie: Option<PuppetLayer>,
}

/// `tweak_x` / `tweak_y` (the names `TweakProject` binds, TU3 factory 82BC9510).
fn tweak_attributes(v: [f32; 2]) -> [skate_core::animation::playback_parameters::SettableAttribute; 2] {
    use skate_core::animation::{playback_parameters::SettableAttribute, skeleton_input::name::encode};
    [SettableAttribute { name: encode(b"tweak_x"), value: v[0], normalized: false, sequence_id: -1 }, SettableAttribute { name: encode(b"tweak_y"), value: v[1], normalized: false, sequence_id: -1 }]
}

/// The stock tree retail overlays while the skater rides fakie: [code] `FakieHeadChannel82BAC778`
/// starts channel `"fakie"` with `B_FAKIE_CHANNEL` (blend in / out 0.3 s, transition 0.1 s) on
/// the rising edge of the riding-fakie bit and ends it when the bit clears; the riding clip below
/// it is unchanged. A mod may replace it (`skater_clips["fakie_channel"]`).
pub(crate) const FAKIE_CHANNEL: &str = "B_FAKIE_CHANNEL";

/// The fakie channel tree in effect: a mod's `skater_clips["fakie_channel"]`, else [`FAKIE_CHANNEL`].
pub(crate) fn resolve_fakie_channel(overrides: &BTreeMap<String, String>) -> &str {
    overrides.get("fakie_channel").map_or(FAKIE_CHANNEL, String::as_str)
}

/// The `torso` value the fakie channel holds while riding: [code] `FakieHeadChannel82BAC778`
/// targets 1.0 while manualing, 0.0 while power sliding, else 0.5 (and starts at its target).
/// The replay has no manual / powerslide state, so the puppet always uses the riding value (NOT
/// RETAIL YET for recorded manuals and powerslides).
pub(crate) const FAKIE_TORSO_RIDING: f32 = 0.5;

/// The fakie channel layer of a cursor `alpha` of the next frame: `clip` = the channel tree
/// (`B_FAKIE_CHANNEL`, a stock phase blend on `torso`), its time since the bit set (the channel
/// keeps that clock while it fades out) and its weight ([`LineCursor::fakie_channel_weight`]);
/// `None` while the weight is 0.
pub(crate) fn fakie_channel_layer(cursor: &LineCursor, alpha: f32, tree: &str) -> Option<PuppetLayer> {
    let weight = cursor.fakie_channel_weight(alpha);
    if weight <= 0.0 {
        return None;
    }
    let began = if cursor.fakie { cursor.fakie_since } else { cursor.fakie_previous_since };
    let time = (cursor.frames - began.min(cursor.frames)) as f32 / 60.0 + alpha.clamp(0.0, 1.0) / 60.0;
    Some(PuppetLayer { clip: tree.to_owned(), time, weight, since: began, tree: None })
}

/// The fakie channel tree's parameters: `torso` = [`FAKIE_TORSO_RIDING`] (pass to
/// `graph_host::motion::tree_commands` with the layer's tree and time).
pub(crate) fn fakie_channel_attributes() -> [skate_core::animation::playback_parameters::SettableAttribute; 1] {
    [skate_core::animation::playback_parameters::SettableAttribute {
        name: skate_core::animation::skeleton_input::name::encode(b"torso"),
        value: FAKIE_TORSO_RIDING,
        normalized: false,
        sequence_id: -1,
    }]
}

/// The puppet's layers for a cursor (mod clips and blend times, shipped pick as fallback).
pub(crate) fn npc_puppet_layers(settings: &LivingWorldSettings, scores: &NpcScoreClips, style: &str, skater: Option<&crate::physics::SkaterRuntime>, cursor: &LineCursor, sub_frame: f32) -> Vec<PuppetLayer> {
    let mut out = Vec::new();
    npc_puppet_layers_into(settings, scores, style, skater, cursor, sub_frame, &mut Vec::new(), &mut out);
    out
}

/// skate_vita (performance, exact): the resolved clip of each remembered phase, kept per NPC ([`NpcEntity::clips`]).
pub(crate) type ClipCache = Vec<((PhaseEntry, Option<PhaseEntry>), (String, f32))>;

/// [`npc_puppet_layers`] into `out` (cleared first).
pub(crate) fn npc_puppet_layers_into(settings: &LivingWorldSettings, scores: &NpcScoreClips, style: &str, skater: Option<&crate::physics::SkaterRuntime>, cursor: &LineCursor, sub_frame: f32, cache: &mut ClipCache, out: &mut Vec<PuppetLayer>) {
    let _scope = skate_core::profile::scope("z_npc_layers");
    let evaluates = |clip: &str| skater.is_none_or(|k| k.animation.evaluator.clip_length(clip).is_ok());
    let meta = skater.map(|k| k.animation.motion.animation.metadata());
    let stock = |name: &str| match meta {
        Some(m) => stock_tree_leaf(m, name).filter(|c| evaluates(c)),
        None => Some(name.to_owned()),
    };
    let leaf = |name: &str, targets: &[(&str, f32)]| match meta {
        Some(m) => stock_tree_leaf_at(m, name, targets).filter(|c| evaluates(c)),
        None => Some(name.to_owned()),
    };
    // No skater runtime (headless): the top of the range, where retail rests.
    let grind = settings.npc_grind_disttocog.or_else(|| skater.map(|k| k.animation.motion.grind_rest_disttocog())).unwrap_or(1.0e6);
    let manual = |m: f32| skater.map_or(0.0, |k| k.animation.motion.manual_rest_angle(m));
    let scored = |e: PhaseEntry, landed: bool| if NPC_STOCK_TREES { scored_trick_clip(&scores.0, &leaf, grind, &manual, e, landed) } else { None };
    // `puppet_layer_clip` is a function of the phase entry, the one before it, the settings, the style and the stock
    // animation data, all fixed while the NPC lives, so a remembered result is the one it would compute; the Vita spent
    // most of the NPC step resolving the same few phases again every tick (string lookups in the banks and the tree
    // metadata). Entries whose phase left the history are dropped.
    let cell = std::cell::RefCell::new(std::mem::take(cache));
    let resolve = |e: PhaseEntry, older: Option<PhaseEntry>| {
        if let Some((_, hit)) = cell.borrow().iter().find(|(k, _)| *k == (e, older)) {
            return hit.clone();
        }
        let value = puppet_layer_clip(&settings.skater_clips, &settings.skater_blend_seconds, style, &evaluates, &stock, &scored, e, older);
        cell.borrow_mut().push(((e, older), value.clone()));
        value
    };
    let mut entries = cursor.phase_history().map(|(e, frames)| (e, (frames as f32 + sub_frame) / 60.0));
    let Some(first) = entries.next() else {
        puppet_layers_into(&[], &resolve, out);
        drop(resolve);
        *cache = cell.into_inner();
        return;
    };
    let mut history = [first; skate_core::living_world::replay::PHASE_HISTORY];
    let mut n = 1;
    for entry in entries {
        history[n] = entry;
        n += 1;
    }
    puppet_layers_into(&history[..n], &resolve, out);
    if let Some(k) = skater.filter(|_| NPC_STOCK_TREES) {
        // The air pose after a grab's Out: the Air phase clip at its blend.
        let air = resolve_puppet_clip(&settings.skater_clips, ReplayPhase::Air, style);
        let air = (if evaluates(air) { air } else { puppet_clip(ReplayPhase::Air, style) }, blend_seconds(&settings.skater_blend_seconds, ReplayPhase::Air));
        let mut i = 0;
        while i < out.len() {
            let (tree, follow) = grab_tree(&settings.skater_clips, scores, k, &history[..n], &out[i], air).unzip();
            out[i].tree = tree;
            let follow = follow.unwrap_or_default();
            // The Out / air pose between the grab and the next layer, when they began before it (else the grab was held).
            let next = out.get(i + 1).map(|x| x.since);
            for layer in follow.after.into_iter().filter(|r| next.is_none_or(|s| s > r.since)) {
                i += 1;
                out.insert(i, layer);
            }
            i += 1;
        }
        // A touchdown layer whose span's grab still held its Cyc: the grab landing until Land leaves for OnBoard, then
        // the touchdown layer blends back in (found from the history, so it outlives the grab layer's blend-out).
        let mut j = 0;
        while j < out.len() {
            let landed = history[..n].iter().position(|h| h.0.since == out[j].since && out[j].tree.is_none() && matches!(h.0.phase, ReplayPhase::Rolling | ReplayPhase::Crouched)).and_then(|ti| {
                let t = history[ti].0;
                let gi = (ti + 1..n).filter(|&g| history[g].0.trick_since == t.trick_since && matches!(history[g].0.phase, ReplayPhase::GroundTrick | ReplayPhase::AirTrick)).last()?;
                let (clip, _) = resolve(history[gi].0, history.get(gi + 1).map(|h| h.0));
                let grab = PuppetLayer { clip, time: history[gi].1, weight: 1.0, since: history[gi].0.since, tree: None };
                grab_tree(&settings.skater_clips, scores, k, &history[..n], &grab, air)?.1.landing.filter(|(l, _)| l.since == t.since)
            });
            if let Some((landing, plays)) = landed {
                let back = landing.time - plays;
                let seconds = history[..n].iter().find(|h| h.0.since == out[j].since).map_or(RETAIL_BLEND_SECONDS, |h| blend_seconds(&settings.skater_blend_seconds, h.0.phase));
                let weight = skate_core::animation::playback_transition::transition_weight(back.max(0.0), seconds);
                if back < 0.0 {
                    out[j] = landing;
                } else if weight < 1.0 {
                    let ground = std::mem::replace(&mut out[j], landing);
                    j += 1;
                    out.insert(j, PuppetLayer { time: back, weight, ..ground });
                } else {
                    // Fully back: the landing is gone; the touchdown layer keeps the time it came back with.
                    out[j].time = back;
                }
            }
            j += 1;
        }
    }
    drop(resolve);
    let mut kept = cell.into_inner();
    kept.retain(|((e, _), _)| history[..n].iter().any(|(h, _)| h == e));
    *cache = kept;
}

/// Layer starts an NPC remembers (its puppet shows at most a few layers at once).
const STANCE_STARTS: usize = 8;

/// An NPC skater's stance bits (retail SkaterAnim flags bits 31 / 30 and the relative stance,
/// [`StanceFlags`]): the natural stance's start bits, toggled by the stance events of the clip its
/// newest layer plays, like retail's `82593230` -> `82B98980` for every actor. Each layer keeps
/// the bits current when it began (retail bakes them into the tree's bind pose at construction).
/// Simulation state, updated on the fixed step from the cursor and the clip data only
/// ([`track_stance`]), so a client derives the host's bits; plain data for a snapshot.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NpcStanceTrack {
    /// Current bits (what a tree built now would get).
    pub flags: StanceFlags,
    /// The newest layer (clip, start frame) and its clip time at the last step.
    pub top: Option<(String, u64, f32)>,
    /// Layer start frame -> the bits it was built with, oldest first (at most [`STANCE_STARTS`]).
    pub starts: Vec<(u64, StanceFlags)>,
    /// The natural stance's start bits (layers older than every remembered start).
    pub natural: StanceFlags,
}

impl NpcStanceTrack {
    pub fn new(stance: skate_core::living_world::stance::NaturalStance) -> Self {
        let natural = StanceFlags::natural(stance);
        Self { flags: natural, top: None, starts: Vec::new(), natural }
    }

    /// The bits the layer that began at `since` was built with: its own start, else the newest
    /// start before it, else the natural bits.
    pub fn flags_for(&self, since: u64) -> StanceFlags {
        self.starts.iter().rev().find(|(s, _)| *s <= since).map_or(self.natural, |(_, f)| *f)
    }

    /// One fixed step with the newest layer `top`: a new layer (clip or start changed) records the
    /// current bits as its own and plays from its clip start; then every stance event in the
    /// clip's attributes collected over the advanced window (`attributes(clip, previous_time,
    /// time)`, names) toggles its bit ([`StanceFlags::apply`]). Returns whether a bit changed.
    pub fn step(&mut self, top: &PuppetLayer, events: &StanceEvents, attributes: impl Fn(&str, f32, f32) -> Vec<String>) -> bool {
        let fresh = !matches!(&self.top, Some((clip, since, _)) if *clip == top.clip && *since == top.since);
        let previous_time = match &self.top {
            Some((_, _, t)) if !fresh => *t,
            _ => 0.0,
        };
        if fresh {
            if self.starts.last().is_none_or(|(s, _)| *s != top.since) {
                self.starts.push((top.since, self.flags));
            }
            if self.starts.len() > STANCE_STARTS {
                self.starts.remove(0);
            }
        }
        self.top = Some((top.clip.clone(), top.since, top.time));
        let names = attributes(&top.clip, previous_time, top.time);
        let before = self.flags;
        // Names compare as the engine's attribute names (`encode`: the banks spell them in
        // upper case, `ANIMBOARDBACKWARD`), like the player's `apply_stance_events`.
        use skate_core::animation::skeleton_input::name::encode;
        self.flags.apply(events, |n| names.iter().any(|x| encode(x.as_bytes()) == encode(n.as_bytes())));
        before != self.flags
    }
}

/// The attribute names of a (`+`-joined) clip collected over `previous_time..time` s of its
/// playback ([`skate_core::living_world::stance::attribute_in_window`] per part: parts play back
/// to back, a part entered in the window plays from its start, the last part holds at its end).
/// `clip_data(part)` = the part's length (s) and its attributes (name, begin, end; normalised).
pub(crate) fn sequence_attributes<N: AsRef<str>>(clip: &str, previous_time: f32, time: f32, clip_data: impl Fn(&str) -> Option<(f32, Vec<(N, f32, f32)>)>) -> Vec<String> {
    let mut out = Vec::new();
    let mut offset = 0.0f32;
    let mut parts = clip.split('+').peekable();
    while let Some(part) = parts.next() {
        let last = parts.peek().is_none();
        let (name, speed) = part_speed(part);
        let Some((length, attributes)) = clip_data(name) else { break };
        let end = offset + length / speed;
        let reached = time >= offset;
        let left = !last && previous_time >= end;
        if reached && !left {
            let p = ((previous_time - offset) * speed).clamp(0.0, length);
            let t = ((time - offset) * speed).clamp(0.0, length);
            out.extend(attributes.iter().filter(|(_, b, e)| skate_core::living_world::stance::attribute_in_window(*b, *e, length, p, t)).map(|(n, ..)| n.as_ref().to_owned()));
        }
        if !reached {
            break;
        }
        offset = end;
    }
    out
}

/// Fixed step after [`advance`]: each NPC's stance bits from the clip its newest layer plays
/// ([`NpcStanceTrack::step`]); the clip data come from the stock banks of the skater runtime

pub(crate) const PUPPET_CLIPS: [&str; 8] =
    ["R_IDLE_RIDE_N_0_CYC", "R_IDLE_RIDE_AGGR_0_CYC", "R_IDLE_RIDE_LOOSE_0_CYC", "R_IDLE_LCOM_000", "IA_IDLE_N_N_0_CYC", "IA_IDLE_LO_N_0_CYC", "G_5050_FS_LOW_0_CYC", "BR_STAND_0_CYC"];

fn length(v: [f32; 3]) -> f32 {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}

/// Root rotation of the puppet: the recorded skater orientation (x, y, z, w, +Z forward [data]).
pub(crate) fn root_rotation(s: &ReplaySample) -> Quat {
    let [x, y, z, w] = s.skater;
    let q = Quat::from_xyzw(x, y, z, w);
    if q.is_finite() && q.length_squared() > 0.5 {
        q.normalize()
    } else {
        Quat::from_rotation_y(s.heading)
    }
}

/// Solid ids of NPC proxies: a tag in the top bits keeps them apart from mod bodies.
pub(crate) const PROXY_ID_TAG: u64 = 0x4E50_0000_0000_0000;

/// The board half of an NPC proxy carries this bit on top of the body's id.
pub(crate) const PROXY_BOARD_BIT: u64 = 1 << 47;

/// The proxy peer id of a simulated NPC's body (the actor its contacts report).
pub(crate) fn sim_peer(id: LivingWorldId) -> u64 {
    PROXY_ID_TAG | (1 << 46) | id.to_u64()
}

/// The proxy peer id of the player's body in a simulated NPC's solve.
pub(crate) const PLAYER_PEER: u64 = PROXY_ID_TAG | (1 << 45);

/// Retail collision groups: skater skeleton parts 5 (the skeleton contact pass scales a group-5
/// force by `SkaterSkeletonScalar` and raises the player's thresholds by
/// `SkaterSkaterThresholdScalar`, `sub_82BD4A30` at `0x82BD53D0`), board 4 (ignored by that
/// pass). The multiplayer peers' proxies use the same split (`physics/network.rs`).
pub(crate) const PROXY_BODY_GROUP: u32 = 5;
pub(crate) const PROXY_BOARD_GROUP: u32 = 4;

/// The kinematic collision proxies of one NPC (body capsule in the skater group, board box in
/// the board group), world space.
pub(crate) fn proxy(id: LivingWorldId, s: &ReplaySample) -> [crate::dynamics::SolidBody; 2] {
    use crate::dynamics::rapier3d::prelude::{Pose, Rotation, SharedShape, Vector};
    let [x, y, z, w] = s.skater;
    let rotation = Rotation::from_xyzw(x, y, z, w).normalize();
    let at = |local: [f32; 3]| {
        let q = Quat::from_xyzw(x, y, z, w).normalize();
        Vec3::from_array(s.position) + q * Vec3::from_array(local)
    };
    let p = |v: Vec3| Vector::new(v.x, v.y, v.z);
    // Body: a 0.25 m capsule from 0.25 to 1.55 m above the deck; board: 0.8 x 0.1 x 0.2 m.
    let body = SharedShape::capsule_y(0.65, 0.25);
    let board = SharedShape::cuboid(0.1, 0.05, 0.4);
    let com = at([0.0, 0.9, 0.0]);
    let deck = at([0.0, 0.08, 0.0]);
    let solid = |id: u64, group: u32, centre: Vec3, collider: crate::dynamics::SolidCollider| crate::dynamics::SolidBody {
        id,
        pose: Pose::from_parts(p(Vec3::from_array(s.position)), rotation),
        center_of_mass: p(centre),
        inertia_rotation: rotation,
        inverse_mass: 0.0,
        inverse_inertia: Vector::new(0.0, 0.0, 0.0),
        linvel: Vector::new(s.velocity[0], s.velocity[1], s.velocity[2]),
        angvel: Vector::new(0.0, 0.0, 0.0),
        contact_group: group,
        colliders: vec![collider],
    };
    let base = PROXY_ID_TAG | id.to_u64();
    [
        solid(base, PROXY_BODY_GROUP, com, crate::dynamics::SolidCollider { shape: body, pose: Pose::from_parts(p(com), rotation), friction: 0.5 }),
        solid(base | PROXY_BOARD_BIT, PROXY_BOARD_GROUP, deck, crate::dynamics::SolidCollider { shape: board, pose: Pose::from_parts(p(deck), rotation), friction: 0.5 }),
    ]
}

/// NPC skaters against dynamic props (DMOs), doc 26 fix 19. Retail NPC skaters are full skaters
/// (`AIController` on the player's skater physics, [code] notes `npc-skaters-re.md` section 0),
/// so their board and body hit a DMO like the player's: the prop step pushes it by the prop's own
/// tuning (`props` domain: push mass, transfer, board / body caps). Our NPC is a replay puppet, so
/// it does not slow down or bail; the prop is pushed out of its line. A mod may switch it off
/// (`sdk.world.set_tuning("living_world", {npc_skater_props = {enabled = false}})`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct NpcSkaterPropContact {
    /// Retail on.
    pub enabled: bool,
}

impl Default for NpcSkaterPropContact {
    fn default() -> Self {
        Self { enabled: true }
    }
}

/// The prop-step push volumes of one NPC: the proxy's body capsule and its board (as a capsule
/// along the deck), at the recorded velocity, in the same places as [`proxy`]. Both count as a
/// board hit (the prop's `board_push_speed` cap): the lower body-bump cap is for the player's
/// walking body, which the prop's triangles stop; a puppet is not stopped, and in retail the
/// whole skater rides into the prop at riding speed. Actor id = the proxy's solid id (stable, the
/// prop's authority owner).
pub(crate) fn prop_volumes(id: LivingWorldId, s: &ReplaySample) -> [(u64, skate_core::physics::board_world::BoardWorldVolume); 2] {
    use skate_core::math::Vector3;
    use skate_core::physics::board_step::CollisionBody;
    use skate_core::physics::board_world::BoardWorldVolume;
    use skate_core::physics::world_contact::ContactPrimitive;
    let [x, y, z, w] = s.skater;
    let q = Quat::from_xyzw(x, y, z, w);
    let q = if q.is_finite() && q.length_squared() > 0.5 { q.normalize() } else { Quat::from_rotation_y(s.heading) };
    let v = |a: Vec3| Vector3::new(a.x, a.y, a.z);
    let at = |local: [f32; 3]| v(Vec3::from_array(s.position) + q * Vec3::from_array(local));
    let material = skate_core::physics::contact::RetailContactMaterial { static_friction: 0.5, dynamic_friction: 0.5, restitution: 0.0 };
    let velocity = Vector3::new(s.velocity[0], s.velocity[1], s.velocity[2]);
    let actor = PROXY_ID_TAG | id.to_u64();
    let hit = CollisionBody::Board(skate_core::physics::board::BodyId::Deck);
    let body = BoardWorldVolume {
        collision_group: 4,
        body: hit,
        primitive: ContactPrimitive::Capsule { center: at([0.0, 0.9, 0.0]), axis: v(q * Vec3::Y), half_length: 0.65, radius: 0.25 },
        motion: skate_core::physics::board_world::VolumeMotion { linear_velocity: velocity, ..Default::default() },
        material,
    };
    let board = BoardWorldVolume {
        collision_group: 4,
        body: hit,
        // The proxy's 0.8 x 0.1 x 0.2 m board box as a capsule along the deck (0.4 m to each end).
        primitive: ContactPrimitive::Capsule { center: at([0.0, 0.08, 0.0]), axis: v(q * Vec3::Z), half_length: 0.3, radius: 0.1 },
        motion: skate_core::physics::board_world::VolumeMotion { linear_velocity: velocity, ..Default::default() },
        material,
    };
    [(actor, body), (actor, board)]
}

/// Global (model-space) bone matrices of a stock clip `time` s into its phase (wrapped when the
/// clip loops), root trajectory held at the origin (the cursor moves the root).
pub(crate) fn evaluator_pose(evaluator: &crate::animation_pose::PoseEvaluator, clip: &str, time: f32) -> Option<Vec<Mat4>> {
    puppet_blend_pose(evaluator, clip, time, None)
}

/// [`evaluator_pose`] with an optional crossfade: like a graph transition, the outgoing and the
/// incoming clip are evaluated and blended with `PoseCommand::Blend` (the player's SQT blend,
/// `pose_blend::blend_sample`), then the reference pose is added to the result.
pub(crate) fn puppet_blend_pose(evaluator: &crate::animation_pose::PoseEvaluator, clip: &str, time: f32, blend: Option<&PuppetBlend>) -> Option<Vec<Mat4>> {
    let mut layers = Vec::with_capacity(2);
    if let Some(b) = blend {
        layers.push(PuppetLayer { clip: b.from.clone(), time: b.from_time, weight: 1.0, since: 0, tree: None });
    }
    layers.push(PuppetLayer { clip: clip.to_owned(), time, weight: blend.map_or(1.0, |b| b.weight), since: 0, tree: None });
    puppet_layers_pose(evaluator, &layers)
}

/// [`puppet_blend_pose`] for nested layers (oldest first): the base clip, then each newer clip
/// blended over the result by its weight (`PoseCommand::Blend`, the player's SQT blend), like a
/// graph transition whose outgoing tree is the running transition. A layer whose clip does not
/// evaluate is skipped (an outgoing one) or fails the pose (the newest one).
pub(crate) fn puppet_layers_pose(evaluator: &crate::animation_pose::PoseEvaluator, layers: &[PuppetLayer]) -> Option<Vec<Mat4>> {
    puppet_pose_with_channel(evaluator, layers, None)
}

/// [`puppet_layers_pose`] with a channel overlay on top of the layers (the fakie channel): the
/// channel tree's pose commands blended over the layered pose with `PoseCommand::ChannelBlend`
/// (its per-bone channel weights, like `MotionChannels::evaluate`) at the channel's weight.
pub(crate) fn puppet_pose_with_channel(evaluator: &crate::animation_pose::PoseEvaluator, layers: &[PuppetLayer], channel: Option<(&[skate_core::animation::playback_tree::PoseCommand], f32)>) -> Option<Vec<Mat4>> {
    puppet_pose_in_stance(evaluator, layers, channel, false)
}

/// The puppet root's extra turn for a natural stance (`flags_root_turn` of its start bits).
pub(crate) fn stance_root_turn(stance: skate_core::living_world::stance::NaturalStance) -> Quat {
    flags_root_turn(StanceFlags::natural(stance))
}

/// The puppet root's extra turn for the stance bits of the newest layer. The regular bind pose
/// ([`bind_pose_tail`]) draws the board turned round (`BOARD_BACKWARDS`, nose along root -Z) and
/// the body mirrored; retail's physical board follows the animated board (`publish_deck_angles`
/// with `board_flipped = bit31 ^ mirror`, false for both natural stances), so a regular
/// character's frame is half a turn from its board's. The replay root is the frame the goofy
/// puppet rides nose first with (the facing rule and the fakie rule work on that board axis), so
/// a mirrored puppet turns its root half a turn: the board keeps its world heading and the left
/// foot leads. The turn follows the mirror bit alone: a trick clip that toggles only
/// `animboardbackward` (the board turned under the body) ends with the board turned in the clip,
/// and the next layer's `BOARD_BACKWARDS` keeps it turned without moving the body.
pub(crate) fn flags_root_turn(flags: StanceFlags) -> Quat {
    if flags.mirrored { Quat::from_rotation_y(std::f32::consts::PI) } else { Quat::IDENTITY }
}

/// [`stance_tail`] for a natural stance's start bits (`mirrored` = regular).
pub(crate) fn bind_pose_tail(mirrored: bool) -> Vec<skate_core::animation::playback_tree::PoseCommand> {
    stance_tail(StanceFlags { board_backward: mirrored, mirrored, switch: false })
}

/// The commands retail's bind pose appends to every tree it plays (`MotionAnimation::add_bind_pose`,
/// the player's path, from the SkaterAnim flags at the tree's construction): the reference pose
/// added, `BOARD_BACKWARDS` / `BOARD_BACKWARDS_IK` for bit 31, a mode 2 mirror for bit 30. The
/// switch bit adds nothing.
pub(crate) fn stance_tail(flags: StanceFlags) -> Vec<skate_core::animation::playback_tree::PoseCommand> {
    let mut out = Vec::new();
    stance_tail_into(flags, &mut out);
    out
}

/// [`stance_tail`] appended to `out`.
fn stance_tail_into(flags: StanceFlags, out: &mut Vec<skate_core::animation::playback_tree::PoseCommand>) {
    use skate_core::animation::playback_tree::PoseCommand;
    out.extend([PoseCommand::Pose { name: "RIG_TPOSE".into() }, PoseCommand::Add { motion_is_a: true }]);
    if flags.board_backward {
        out.extend([
            PoseCommand::Pose { name: "BOARD_BACKWARDS".into() },
            PoseCommand::Add { motion_is_a: false },
            PoseCommand::Pose { name: "BOARD_BACKWARDS_IK".into() },
            PoseCommand::Add { motion_is_a: true },
        ]);
    }
    if flags.mirrored {
        out.push(PoseCommand::Mirror { trajectory_mode: 2 });
    }
}

/// [`puppet_pose_with_channel`] in a natural stance (`mirrored` = regular): every layer with the
/// stance's start bits ([`puppet_pose_in_flags`]).
pub(crate) fn puppet_pose_in_stance(evaluator: &crate::animation_pose::PoseEvaluator, layers: &[PuppetLayer], channel: Option<(&[skate_core::animation::playback_tree::PoseCommand], f32)>, mirrored: bool) -> Option<Vec<Mat4>> {
    let flags = StanceFlags { board_backward: mirrored, mirrored, switch: false };
    puppet_pose_in_flags(evaluator, layers, &vec![flags; layers.len()], channel)
}

/// The puppet pose with each layer in the stance bits it was built with (`flags[i]` for
/// `layers[i]`; retail bakes the bits into a tree's bind pose at construction, so a transition
/// blends two trees each closed with its own tail). While every layer shares the bits, the
/// layers blend first and one tail closes the result (with no bits set: the commands every NPC
/// drew before the stance port; with the regular bits: the natural stance port); once a newer
/// layer's bits differ, every layer is closed with its own tail before it blends in. The channel
/// tree (fakie) is closed with the newest layer's bits, like the player's channel tree built with
/// the current flags.
pub(crate) fn puppet_pose_in_flags(evaluator: &crate::animation_pose::PoseEvaluator, layers: &[PuppetLayer], flags: &[StanceFlags], channel: Option<(&[skate_core::animation::playback_tree::PoseCommand], f32)>) -> Option<Vec<Mat4>> {
    let mut scratch = PoseScratch::default();
    puppet_pose_in_flags_into(evaluator, layers, flags, channel, &mut scratch).then_some(scratch.globals)
}

/// Reused buffers of [`puppet_pose_in_flags_into`].
#[derive(Default)]
pub(crate) struct PoseScratch {
    commands: Vec<skate_core::animation::playback_tree::PoseCommand>,
    /// Per layer: the pose commands of its tree ([`PuppetLayer::tree`]), played in place of its last clip; empty = none.
    pub trees: Vec<Vec<skate_core::animation::playback_tree::PoseCommand>>,
    locals: Vec<Mat4>,
    pub globals: Vec<Mat4>,
}

/// [`puppet_pose_in_flags`] into `scratch.globals`; false where it gives `None`.
pub(crate) fn puppet_pose_in_flags_into(evaluator: &crate::animation_pose::PoseEvaluator, layers: &[PuppetLayer], flags: &[StanceFlags], channel: Option<(&[skate_core::animation::playback_tree::PoseCommand], f32)>, scratch: &mut PoseScratch) -> bool {
    use skate_core::animation::playback_tree::PoseCommand;
    let PoseScratch { commands, trees, locals, globals } = scratch;
    // Pushes layer `i`'s pose: its tree in its last part, else the clip playing.
    let sample = |commands: &mut Vec<PoseCommand>, i: usize, l: &PuppetLayer| -> bool {
        let Some((clip, time)) = sequence_part(evaluator, &l.clip, l.time) else { return false };
        if let Some(tree) = trees.get(i).filter(|t| !t.is_empty() && l.clip.rsplit('+').next().map(|p| part_speed(p).0) == Some(clip)) {
            commands.extend_from_slice(tree);
            return true;
        }
        let Ok(length) = evaluator.clip_length(clip) else { return false };
        let time = puppet_clip_time(clip, time, length);
        commands.push(PoseCommand::Clip { name: clip.to_owned(), previous_time: time, time, loops: 0 });
        true
    };
    let flag = |i: usize| flags.get(i).copied().unwrap_or_default();
    let Some((top, older)) = layers.split_last() else { return false };
    let top_flags = flag(layers.len() - 1);
    let mixed = (0..layers.len()).any(|i| flag(i) != top_flags);
    commands.clear();
    commands.reserve(layers.len() * 8 + 8);
    // Per-layer tails once the bits differ (each layer closed before it blends in).
    let closed = |commands: &mut Vec<PoseCommand>, i: usize| {
        if mixed {
            stance_tail_into(flag(i), commands);
        }
    };
    for (i, l) in older.iter().enumerate() {
        let base = commands.is_empty();
        if sample(commands, i, l) {
            closed(commands, i);
            if !base {
                commands.push(PoseCommand::Blend { weight: l.weight });
            }
        }
    }
    let base = commands.is_empty();
    if !sample(commands, layers.len() - 1, top) {
        return false;
    }
    closed(commands, layers.len() - 1);
    if !base {
        commands.push(PoseCommand::Blend { weight: top.weight });
    }
    // Shared bits: one tail for the blended layers. A tail that mirrors (or turns the board)
    // closes before the channel tree, which gets its own (as the player's channel tree).
    let shaped = top_flags.board_backward || top_flags.mirrored;
    if !mixed && shaped {
        stance_tail_into(top_flags, commands);
    }
    if let Some((tree, weight)) = channel.filter(|c| !c.0.is_empty() && c.1 > 0.0) {
        commands.extend_from_slice(tree);
        if mixed || shaped {
            stance_tail_into(top_flags, commands);
        }
        commands.push(PoseCommand::ChannelBlend { weight, use_channels_from_weights: false });
    }
    if !mixed && !shaped {
        stance_tail_into(top_flags, commands);
    }
    let Ok(pose) = evaluator.evaluate(commands) else { return false };
    locals.clear();
    locals.extend(pose.iter().copied().map(skate_core::animation::output::sqt_to_matrix).map(crate::animation::native_matrix));
    let parents = &evaluator.frames.parents;
    globals.clear();
    globals.reserve(locals.len());
    for (i, local) in locals.iter().enumerate() {
        let g = match parents.get(i).copied() {
            Some(p) if p >= 0 && (p as usize) < i => globals[p as usize] * *local,
            _ => *local,
        };
        globals.push(g);
    }
    true
}

/// The part of a `+`-joined clip sequence playing `time` s in and the clip time inside it: parts play
/// back to back at their [`part_speed`], the last one holds (or loops when it is a `_CYC` clip). A plain clip is itself.
pub(crate) fn sequence_part<'a>(evaluator: &crate::animation_pose::PoseEvaluator, clip: &'a str, time: f32) -> Option<(&'a str, f32)> {
    let mut parts = clip.split('+').peekable();
    let mut t = time.max(0.0);
    while let Some(part) = parts.next() {
        let (name, speed) = part_speed(part);
        if parts.peek().is_none() {
            return Some((name, t * speed));
        }
        let len = evaluator.clip_length(name).ok()? / speed;
        if t < len {
            return Some((name, t * speed));
        }
        t -= len;
    }
    None
}

/// A sequence part's clip and playback speed: `NAME*2` plays `NAME` at twice its rate (a state's `playBackSpeed`, the
/// clip clock's length (frames - 1) / (fps x speed)); a plain name plays at 1: docs/hails-additions/26c-living-world-npc-skaters.md#npc-grab-playback-speed
pub(crate) fn part_speed(part: &str) -> (&str, f32) {
    match part.rsplit_once('*').map(|(name, s)| (name, s.parse::<f32>())) {
        Some((name, Ok(speed))) if speed.is_finite() && speed > 0.0 => (name, speed),
        _ => (part, 1.0),
    }
}

/// `clip` as a sequence part played at `speed` (`None` or 1 = the plain name).
pub(crate) fn with_speed(clip: String, speed: Option<f32>) -> String {
    match speed {
        Some(s) if s != 1.0 => format!("{clip}*{s}"),
        _ => clip,
    }
}

/// One-line NPC summary for the debug readout: count, the nearest NPC (distance, line, phase).
pub(crate) fn npc_readout(npcs: &[(LivingWorldId, String, Option<ReplaySample>)], player: Option<[f32; 3]>) -> String {
    let nearest = player.and_then(|p| {
        npcs.iter()
            .filter_map(|(id, c, s)| s.as_ref().map(|s| (id, c, s, length([s.position[0] - p[0], s.position[1] - p[1], s.position[2] - p[2]]))))
            .min_by(|a, b| a.3.total_cmp(&b.3))
    });
    match nearest {
        Some((id, c, s, d)) => format!(
            "npc skaters {} nearest #{} {c} {:.0} m line {} node {} {} {:.1} m/s heading {:.0} velocity_yaw {:.0}",
            npcs.len(),
            id.serial,
            d,
            s.line.iter().map(|b| format!("{b:02x}")).collect::<String>(),
            s.node,
            s.phase.name(),
            length(s.velocity),
            {
                let f = skate_core::living_world::replay::rotate(s.skater, [0.0, 0.0, 1.0]);
                f[0].atan2(f[2]).to_degrees()
            },
            s.velocity[0].atan2(s.velocity[2]).to_degrees()
        ),
        None => format!("npc skaters {}", npcs.len()),
    }
}

// ---- skate_vita: upstream's systems over plain data ----

/// skate_vita: one NPC entity of upstream's ECS (its `NpcSkater`, `NpcReplay`, `NpcFade` and
/// `NpcStanceTrack` components) and the puppet poses of its last two steps (render).
#[derive(Clone)]
pub(crate) struct NpcEntity {
    pub npc: NpcSkater,
    pub replay: NpcReplay,
    pub fade: NpcFade,
    pub stance: NpcStanceTrack,
    pub pose: Option<crate::presentation::Snapshot>,
    pub previous_pose: Option<crate::presentation::Snapshot>,
    /// skate_vita: the resolved clip of each remembered phase ([`npc_puppet_layers_cached`]).
    pub clips: ClipCache,
}

/// skate_vita: the NPC skaters of the loaded world (upstream's `NpcSkaterIndex`, the entities,
/// `AvoidTrack` and the event messages of the last step).
#[derive(Default)]
pub(crate) struct NpcState {
    pub index: BTreeMap<LivingWorldId, NpcEntity>,
    pub avoid: super::npc_avoid::AvoidTrack,
    pub events: Vec<NpcSkaterEvent>,
    /// skate_vita (performance, exact): each stock clip's length and attributes as the stance
    /// step reads them (`clip_data`; fixed stock data, read once per clip instead of every tick).
    pub clip_data: std::cell::RefCell<BTreeMap<String, Option<(f32, Vec<(String, f32, f32)>)>>>,
    /// The lines valid for each character (`sub_82456970`, sorted ids), cached for the loaded line count.
    pub valid: BTreeMap<String, Vec<[u8; 16]>>,
    pub valid_count: usize,
    /// The stock graph's scored trick clips (grabs, grinds, manuals), built when the world loads.
    pub scores: NpcScoreClips,
    /// Buffers the pose step reuses.
    pub scratch: NpcPoseScratch,
}

impl NpcState {
    /// The NPCs in id order (upstream sorts its queries by id).
    pub(crate) fn sorted(&mut self) -> Vec<&mut NpcEntity> {
        self.index.values_mut().collect()
    }
}

pub(crate) fn npc_lines(state: &PopulationState) -> Arc<BTreeMap<[u8; 16], ReplayLine>> {
    state.npc.lines.clone()
}

/// Spawn and despawn NPC entities from the population's records (`apply_records`).
pub(crate) fn apply_records(decisions: &[Decision], state: &PopulationState, settings: &LivingWorldSettings, npcs: &mut NpcState) {
    for d in decisions {
        let Decision::Despawn(r) = d else { continue };
        if r.id.kind != Kind::Skater {
            continue;
        }
        if npcs.index.remove(&r.id).is_some() {
            npcs.events.push(NpcSkaterEvent::Despawned { id: r.id, reason: r.reason });
        }
    }
    let lines = npc_lines(state);
    for d in decisions {
        let Decision::Spawn(s) = d else { continue };
        let SpawnChoice::Skater { line, character, slot } = &s.choice else { continue };
        if npcs.index.contains_key(&s.id) {
            continue;
        }
        let cursor = LineCursor::spawn(&*lines, *line, 0);
        let npc = NpcSkater {
            id: s.id,
            character: character.clone(),
            slot: *slot,
            seed: s.seed,
            voice: state.npc.voices.get(character).copied(),
            spawn_tick: s.tick,
            start_line: *line,
            stance: npc_stance(&state.npc.records, &settings.skater_stance, character),
        };
        let sample = cursor.sample(&*lines, 0.0);
        let stance = NpcStanceTrack::new(npc.stance);
        npcs.index.insert(
            s.id,
            NpcEntity {
                replay: NpcReplay { cursor, branches: Vec::new(), tricks: Vec::new(), last: sample, previous: None, avoid: Default::default(), follow_target: None, body_gate: None },
                fade: NpcFade { alpha: state.world.config.skaters.leave_fade.fade_in_alpha(0), ..NpcFade::default() },
                stance,
                npc,
                pose: None,
                previous_pose: None,
                clips: Vec::new(),
            },
        );
        npcs.events.push(NpcSkaterEvent::Spawned { id: s.id, character: character.clone(), line: *line });
    }
}

/// Advance every cursor to its tick, take branches, end finished lines, publish position
/// (`advance`). Returns the despawn records of the NPCs that faded out. skate_vita: no simulated
/// tier, no audio, single player.
pub(crate) fn advance(settings: &LivingWorldSettings, observers: &[skate_core::living_world::Observer], state: &mut PopulationState, npcs: &mut NpcState) -> Vec<Decision> {
    let _scope = skate_core::profile::scope("z_npc_advance");
    let lines = npc_lines(state);
    let tick = state.world.tick();
    let players: Vec<[f32; 3]> = observers.iter().map(|o| o.position).collect();
    // Others' (line, node) and lines in use, by serial order (deterministic).
    let mut order: Vec<(LivingWorldId, [u8; 16], u32)> = npcs.index.values().map(|n| (n.npc.id, n.replay.cursor.line, n.replay.cursor.node)).collect();
    order.sort_by_key(|x| x.0);
    let mut finished = Vec::new();
    let fade_cfg = state.world.config.skaters.leave_fade;
    let chain = state.world.config.skaters.line_chain;
    let mut events = std::mem::take(&mut npcs.events);
    // Branches and the line-end search take only lines valid for the character (`sub_82456970`, `fits`), cached per
    // character for the loaded line count.
    if let Some(w) = state.skaters.as_ref() {
        if npcs.valid_count != w.lines.len() {
            npcs.valid.clear();
            npcs.valid_count = w.lines.len();
        }
        for entity in npcs.index.values() {
            let character = &entity.npc.character;
            if npcs.valid.contains_key(character) {
                continue;
            }
            if let Some(c) = w.characters.iter().find(|c| c.key == *character) {
                use skate_core::living_world::SkaterWorld;
                let mut ids: Vec<[u8; 16]> = w.lines.iter().filter(|l| w.fits(l, c)).map(|l| l.id).collect();
                ids.sort_unstable();
                info!("NPC_SKATER_LINES {character} valid {} of {}", ids.len(), w.lines.len());
                npcs.valid.insert(character.clone(), ids);
            }
        }
    }
    let valid = &npcs.valid;
    for entity in npcs.index.values_mut() {
        let (npc, replay, fade) = (&entity.npc, &mut entity.replay, &mut entity.fade);
        let elapsed = tick.saturating_sub(npc.spawn_tick) * FRAMES_PER_TICK;
        // The obstacle avoider holds the cursor back (`npc_avoid`); the fade keeps the spawn clock.
        let target = replay.follow_target.unwrap_or_else(|| elapsed.saturating_sub(replay.avoid.held_frames()));
        let mut out = Vec::new();
        // The switch blend time (branch / chain root blend) is a tuning value, the same on a client.
        replay.cursor.switch_blend_seconds = chain.blend_seconds;
        // Keep the facing across switches (fix 16 rule, mod option, retail off); a tuning value.
        replay.cursor.keep_facing = chain.keep_facing;
        // Facing rule (retail riding-entry flip by default; the fix 23 per-node fold as a mod option).
        replay.cursor.facing_rule = chain.facing_rule;
        // Retail riding-fakie thresholds (data, stock graph values by default).
        replay.cursor.fakie_settings = chain.fakie;
        // The NPC's trick tables (disc profile, mod overrides), read at its trick slots.
        let trick_profile = npc_trick_profile(&state.npc.tricks, &settings.skater_trick_profiles, &npc.character);
        let valid_lines = valid.get(&npc.character).map(Vec::as_slice);
        while replay.cursor.frames < target && !replay.cursor.finished {
            if replay.cursor.frames + FRAMES_PER_TICK >= target {
                replay.previous = Some(replay.cursor.clone());
            }
            let s = replay.cursor.sample(&*lines, 0.0);
            let others: Vec<([u8; 16], u32)> = order.iter().filter(|o| o.0 != npc.id).map(|o| (o.1, o.2)).collect();
            let in_use: Vec<[u8; 16]> = others.iter().map(|o| o.0).collect();
            let (position, forward, speed) = s.as_ref().map_or(([0.0; 3], [0.0, 0.0, 1.0], 0.0), |s| (s.position, s.velocity, length(s.velocity)));
            let tricks = TrickContext { mode: settings.npc_tricks.mode, params: settings.npc_tricks.params, profile: trick_profile.as_deref(), seed: npc.seed, slot_tricks: state.npc.tricks.slot_tricks_for(&npc.character) };
            let ctx = BranchContext { position, forward, speed, players: &players, others: &others, in_use: &in_use, preferred_skill: -1, online: false, chain, tricks, body: None, valid_lines };
            let mut decider = Decider::Decide(ctx);
            replay.cursor.step(&*lines, &mut decider, &mut out);
            if let Some(o) = order.iter_mut().find(|o| o.0 == npc.id) {
                o.1 = replay.cursor.line;
                o.2 = replay.cursor.node;
            }
        }
        for ev in out {
            match ev {
                CursorEvent::Node { line, node, event, flags } => {
                    if event != 0 {
                        events.push(NpcSkaterEvent::Node { id: npc.id, line, node, event, flags });
                    }
                }
                CursorEvent::Branch(record) => {
                    replay.branches.push(record.clone());
                    events.push(NpcSkaterEvent::Branch { id: npc.id, record });
                }
                CursorEvent::Trick(record) => {
                    debug!(
                        "NPC_SKATER_TRICK #{} {} node {} frame {} recorded {} chosen {} mode {}",
                        npc.id.serial,
                        npc.character,
                        record.node,
                        record.frame,
                        trick_name(record.recorded),
                        trick_name(record.chosen),
                        settings.npc_tricks.mode.name()
                    );
                    replay.tricks.push(record.clone());
                    events.push(NpcSkaterEvent::Trick { id: npc.id, record });
                }
                CursorEvent::Finished => {
                    events.push(NpcSkaterEvent::LineEnd { id: npc.id });
                }
            }
        }
        // Spawn fade in (retail `sub_825926F8`: 0 to 1 over the first second), then the leave fade
        // (retail `sub_8246EA90` / `sub_8245A9B8`, removal below opacity 0.2): only once the
        // cursor has ended at a dead end (no line starts within the chain radius), holding the
        // last pose while fading.
        fade.fade.update(&replay.cursor, &*lines, &fade_cfg);
        let alpha = fade.fade.alpha(elapsed, &fade_cfg);
        if fade.alpha != alpha {
            fade.alpha = alpha;
        }
        if fade.fade.should_despawn(elapsed, &fade_cfg) {
            finished.push(npc.id);
        }
        let Some(s) = replay.cursor.sample(&*lines, 0.0) else { continue };
        state.world.update_position(npc.id, s.position);
        replay.last = Some(s);
    }
    // Faded out (opacity below the threshold): the NPC leaves.
    let mut despawns = Vec::new();
    for id in finished {
        if let Some(d @ Decision::Despawn(_)) = state.world.despawn(id, DespawnReason::External) {
            state.despawned += 1;
            despawns.push(d);
        }
        if npcs.index.remove(&id).is_some() {
            events.push(NpcSkaterEvent::Despawned { id, reason: DespawnReason::External });
        }
    }
    npcs.events = events;
    despawns
}

/// Add the NPC proxies to the skater solve (after the network proxies were rebuilt), and their
/// push volumes to the prop step ([`prop_volumes`]) (`push_proxies`; replay tier only).
pub(crate) fn push_proxies(npcs: &NpcState, physics: &mut crate::physics::GamePhysics, skater: &crate::physics::SkaterRuntime, settings: &LivingWorldSettings) {
    physics.actor_prop_volumes.clear();
    physics.npc_sim_proxy_mark = None;
    if npcs.index.is_empty() {
        return;
    }
    let mut proxies = std::mem::take(&mut physics.network_proxies);
    let list: Vec<_> = npcs.index.values().filter_map(|n| n.replay.last.as_ref().map(|s| (n.npc.id, s))).collect();
    for &(id, s) in &list {
        for solid in proxy(id, s) {
            proxies.append_solid(solid, physics, skater, false);
        }
    }
    physics.network_proxies = proxies;
    if settings.npc_skater_props.enabled {
        let volumes = list.iter().flat_map(|&(id, s)| prop_volumes(id, s)).collect();
        physics.actor_prop_volumes = volumes;
    }
}

/// skate_vita: upstream's `track_stance` (fixed step after [`advance`]) and `present_pose` (here on
/// the fixed step too) in one pass per NPC: the layers are built once (both upstream systems build
/// the same ones from the cursor at sub-frame 0), the stance bits step on the newest layer, then
/// the puppet pose (the root at the sample with the avoider's offset, the layers' clips posed with
/// the stance bits each was built with, the fakie channel) is kept with the previous step's for
/// the renderer.
pub(crate) fn stance_and_pose(settings: &LivingWorldSettings, skater: &crate::physics::SkaterRuntime, state: &PopulationState, npcs: &mut NpcState) {
    let NpcState { index, clip_data: clip_cache, scores, scratch, .. } = npcs;
    let _scope = skate_core::profile::scope("z_npc_pose");
    let lines = npc_lines(state);
    let events = StanceEvents::with_overrides(&settings.skater_stance_events);
    let read = |part: &str| -> Option<(f32, Vec<(String, f32, f32)>)> {
        let length = skater.animation.evaluator.clip_length(part).ok()?;
        let meta = skater.animation.motion.animation.metadata().clip(part).ok()?;
        Some((length, meta.attributes.iter().map(|a| (a.name.clone(), f32::from_bits(a.begin_bits), f32::from_bits(a.end_bits))).collect()))
    };
    let cache = &*clip_cache;
    let clip_data = |part: &str| -> Option<(f32, Vec<(String, f32, f32)>)> {
        if let Some(hit) = cache.borrow().get(part) {
            return hit.clone();
        }
        let value = read(part);
        cache.borrow_mut().insert(part.to_owned(), value.clone());
        value
    };
    let NpcPoseScratch { layers, layer_flags, channel, fakie_tree, grab_trees, pose } = scratch;
    for entity in index.values_mut() {
        let (npc, replay) = (&entity.npc, &entity.replay);
        let cursor = &replay.cursor;
        let style = native_animation_style(&npc.character);
        let usable = |phase: ReplayPhase| {
            let clip = resolve_puppet_clip(&settings.skater_clips, phase, style);
            if skater.animation.evaluator.clip_length(clip).is_err() { puppet_clip(phase, style) } else { clip }
        };
        let sample = cursor.sample(&*lines, 0.0);
        // `track_stance`: the layers at sub-frame 0; before the first remembered phase the phase's own clip.
        npc_puppet_layers_into(settings, scores, style, Some(skater), cursor, 0.0, &mut entity.clips, layers);
        if layers.is_empty() {
            let Some(s) = sample.as_ref() else { continue };
            layers.push(PuppetLayer { clip: usable(s.phase).to_owned(), time: s.phase_frames as f32 / 60.0, weight: 1.0, since: cursor.frames.saturating_sub(s.phase_frames), tree: None });
        }
        if let Some(top) = layers.last() {
            let before = entity.stance.flags;
            if entity.stance.step(top, &events, |clip, p, t| sequence_attributes(clip, p, t, &clip_data)) {
                debug!("LIVING_WORLD npc stance #{} {} clip {} t {:.2}: {:?} -> {:?} board_flipped {} fakie {}", npc.id.serial, npc.character, top.clip, top.time, before, entity.stance.flags, entity.stance.flags.board_flipped(), cursor.fakie);
            }
        }
        // `present_pose` at the step (render fraction 0: `sample.sub_frame` is 0, the same layers).
        let Some(s) = sample.or_else(|| replay.last.clone()) else { continue };
        if s.sub_frame != 0.0 {
            npc_puppet_layers_into(settings, scores, style, Some(skater), cursor, s.sub_frame, &mut entity.clips, layers);
            if layers.is_empty() {
                layers.push(PuppetLayer { clip: usable(s.phase).to_owned(), time: (s.phase_frames as f32 + s.sub_frame) / 60.0, weight: 1.0, since: cursor.frames.saturating_sub(s.phase_frames), tree: None });
            }
        }
        let translation = Vec3::from_array(s.position) + Vec3::from_array(super::npc_avoid::right_of(s.velocity)) * replay.avoid.offset;
        let natural = StanceFlags::natural(npc.stance);
        layer_flags.clear();
        layer_flags.extend(layers.iter().map(|l| entity.stance.flags_for(l.since)));
        let top_flags = layer_flags.last().copied().unwrap_or(natural);
        let rotation = root_rotation(&s) * flags_root_turn(top_flags);
        let fakie = fakie_channel_layer(cursor, 0.0, resolve_fakie_channel(&settings.skater_clips));
        // A channel tree that does not build (a mod's bad name) is left out.
        let weight = fakie.as_ref().and_then(|f| {
            crate::graph_host::motion::cached_tree_commands_into(&skater.animation.motion.animation, fakie_tree, &f.clip, &fakie_channel_attributes(), f.time, channel).ok().map(|_| f.weight)
        });
        // The grab / grind / manual trees of the layers that carry one (the stock graph's scored trees).
        pose.trees.resize_with(layers.len(), Vec::new);
        for ((l, flags), out) in layers.iter().zip(layer_flags.iter()).zip(pose.trees.iter_mut()) {
            out.clear();
            if let Some(tree) = &l.tree {
                let x = if flags.mirrored { -tree.tweak[0] } else { tree.tweak[0] };
                if crate::graph_host::motion::named_tree_commands_into(&skater.animation.motion.animation, grab_trees, &tree.name, &tweak_attributes([x, tree.tweak[1]]), tree.time, out).is_err() {
                    out.clear();
                }
            }
        }
        if !puppet_pose_in_flags_into(&skater.animation.evaluator, layers, layer_flags, weight.map(|w| (channel.as_slice(), w)), pose) {
            continue;
        }
        let bones: Vec<Mat4> = pose.globals.clone();
        let next = crate::presentation::Snapshot { root: Transform::from_translation(translation).with_rotation(rotation), bones, camera: Transform::IDENTITY, fov: 1.0 };
        entity.previous_pose = entity.pose.take().or_else(|| Some(next.clone()));
        entity.pose = Some(next);
    }
    // The built trees stay while NPCs use them; the most recently built few only (each is a few hundred KB on the
    // Vita's heap, and a tree built again costs less than the memory of every grab, grind and manual tree held).
    if grab_trees.len() > MAX_GRAB_TREES {
        grab_trees.drain(..grab_trees.len() - MAX_GRAB_TREES);
    }
}

/// TEST (11 Nov): the NPC skaters' stock graph trees (grab / grind / manual scored clips and tree layers, PR #52
/// `3d14a7bb`) on or off; off = the puppet's phase clips as before the resync.
pub(crate) const NPC_STOCK_TREES: bool = true;

/// The grab / grind / manual trees [`stance_and_pose`] keeps built.
const MAX_GRAB_TREES: usize = 12;

/// Buffers [`stance_and_pose`] reuses across NPCs and ticks (upstream's `NpcPoseScratch`).
#[derive(Default)]
pub(crate) struct NpcPoseScratch {
    layers: Vec<PuppetLayer>,
    layer_flags: Vec<StanceFlags>,
    channel: Vec<skate_core::animation::playback_tree::PoseCommand>,
    fakie_tree: Option<crate::graph_host::motion::TreeCache>,
    grab_trees: Vec<crate::graph_host::motion::TreeCache>,
    pose: PoseScratch,
}

/// An NPC skater as the renderer draws it (L1d): its pose of the previous and the current step
/// (root and the stock rig's model-space bones, the player's snapshot form) and its opacity.
#[derive(Clone)]
pub struct NpcPose {
    pub serial: u32,
    pub previous: crate::presentation::Snapshot,
    pub current: crate::presentation::Snapshot,
    pub alpha: f32,
    /// Which look the renderer dresses it in ([`look_of`] its character): the same character wears the same one.
    pub look: u32,
}

/// A character's look number (FNV-1a of its key): the renderer takes it modulo the looks it has.
pub fn look_of(character: &str) -> u32 {
    character.bytes().fold(0x811C_9DC5u32, |h, b| (h ^ u32::from(b)).wrapping_mul(0x0100_0193))
}
