//! Ped animation player for the 50-bone ped rig (`PedestrianSkeletonPres.abin`).
//! What it ports and the simplifications: docs/hails-additions/26f-living-world-peds.md#ped-animation-player

use crate::animation::output::Sqt;
use crate::animation::{pose_add, pose_blend, pose_mirror, pose_sample, pose_trajectory};
use crate::living_world::rng::{Rng, derive};
use std::collections::BTreeMap;

/// Identity SQT (absent channels).
pub const IDENTITY: Sqt = Sqt { scale: [1.0, 1.0, 1.0, 1.0], rotation: [0.0, 0.0, 0.0, 1.0], translation: [0.0, 0.0, 0.0, 1.0] };

/// The animation rig (from the bank's hierarchy record and reference pose).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PedRig {
    pub names: Vec<String>,
    pub parents: Vec<i32>,
    pub mirrors: Vec<i32>,
    /// `PEDESTRIAN_RIG_TPOSE` locals (identity where the pose record has no part).
    pub reference: Vec<Sqt>,
    /// Bones the clips carry (the bank's first parts).
    pub animated: Vec<bool>,
}

/// One frame: a local SQT per rig bone.
pub type PedFrame = Vec<Sqt>;

/// A clip attribute channel window (begin / end as phase 0..1 of the clip length).
#[derive(Clone, Debug, PartialEq)]
pub struct ClipWindow {
    pub channel: String,
    pub begin: f32,
    pub end: f32,
    pub value: f32,
}

/// A decoded ped clip.
#[derive(Clone, Debug, PartialEq)]
pub struct PedClip {
    pub name: String,
    pub fps: f32,
    pub frames: Vec<PedFrame>,
    pub looping: bool,
    pub loop_rotation: [f32; 4],
    pub loop_translation: [f32; 3],
    pub windows: Vec<ClipWindow>,
    /// A channel clip's per-bone channel weights (the clip parts' weight table): how much of each bone the clip takes
    /// over when it plays on the ped's channel (`ACSChannelBlend`: coefficient = channel weight x bone weight).
    /// `None` for plain clips.
    pub channel_weights: Option<Vec<f32>>,
}

impl PedClip {
    pub fn length(&self) -> f32 {
        if self.frames.len() < 2 || self.fps <= 0.0 { 0.0 } else { (self.frames.len() - 1) as f32 / self.fps }
    }

    fn sample(&self, time: f32) -> Option<PedFrame> {
        let t = time.clamp(0.0, self.length());
        let s = pose_sample::select_frames(t, self.fps, self.frames.len(), true, 0.0).ok()?;
        Some(self.frames[s.first].iter().zip(&self.frames[s.second]).map(|(a, b)| pose_sample::sample_key(*a, *b, s)).collect())
    }

    fn trajectory(&self, time: f32) -> Option<Sqt> {
        let t = time.clamp(0.0, self.length());
        let s = pose_sample::select_frames(t, self.fps, self.frames.len(), true, 0.0).ok()?;
        Some(pose_sample::sample_key(*self.frames[s.first].first()?, *self.frames[s.second].first()?, s))
    }

    /// The value of a channel at a clip time (0 when no window is active).
    pub fn channel(&self, channel: &str, time: f32) -> f32 {
        let len = self.length();
        let phase = if len > 0.0 { (time / len).clamp(0.0, 1.0) } else { 0.0 };
        self.windows.iter().filter(|w| w.channel.eq_ignore_ascii_case(channel) && phase >= w.begin && phase <= w.end).map(|w| w.value).next().unwrap_or(0.0)
    }
}

/// Clip lookup for the player.
pub trait PedClips {
    fn clip(&self, name: &str) -> Option<&PedClip>;
}

impl PedClips for BTreeMap<String, PedClip> {
    fn clip(&self, name: &str) -> Option<&PedClip> {
        self.get(name)
    }
}

/// One remap candidate: a clip and its `tAnimAttributes` branch windows (start, end s, tag).
#[derive(Clone, Debug, PartialEq)]
pub struct RemapClip {
    pub clip: String,
    pub windows: Vec<(f32, f32, i32)>,
}

/// An animation set (`livingworld_entity_animation` record) resolved: logical name -> candidates.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PedAnimSet {
    pub entries: BTreeMap<String, Vec<RemapClip>>,
    /// The set's collision values (knock-down speeds, whether it allows knock-downs, ground time).
    pub collision: super::skater_contact::CollisionRules,
    /// The locomotion speed rows, ascending (empty in an install from before setup named them).
    pub speed_rows: Vec<SpeedRow>,
}

/// One locomotion speed row of a set (poster `sub_82E29D98`, research b682) [code + data]: from `threshold` m/s up
/// the ped is in `state` with the `tier` cycle. `turn_from`: the previous row states a turn may start from.
#[derive(Clone, Debug, PartialEq)]
pub struct SpeedRow {
    pub threshold: f32,
    pub state: String,
    pub tier: Tier,
    pub turn_from: Vec<String>,
}

/// The cycle tier intent of a speed row (`WalkCyc.Brisk`, `ChaseRunCyc.Slow` / `.Fast`) [data].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tier {
    #[default]
    Default,
    Brisk,
    Slow,
    Fast,
}

impl Tier {
    pub fn from_intent(name: Option<&str>) -> Self {
        match name {
            Some("Brisk") => Tier::Brisk,
            Some("Slow") => Tier::Slow,
            Some("Fast") => Tier::Fast,
            _ => Tier::Default,
        }
    }
}

/// The poster adds `add` to a speed above `above` before the pick (`0x8231A844` 1.0 / `0x820C6D98` 0.25, b682).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpeedBump {
    pub above: f32,
    pub add: f32,
}

impl Default for SpeedBump {
    fn default() -> Self {
        Self { above: 1.0, add: 0.25 }
    }
}

/// A moving ped's intent and tier. With a brain speed the speed rows decide: a ChaseRun row runs, a Walk row walks,
/// each with its tier, a Shuffle row shuffles (an Idle row keeps the nav's walk). Without a speed a fleeing /
/// chasing ped runs the set's untiered run row, else its first run row (OURS: the flee speed is not decoded; a set
/// whose only run row is Slow, the female one, runs Slow). Rows empty (an old install): walk, or run when fleeing.
pub fn moving_intent(nav: Intent, running: bool, speed: Option<f32>, rows: &[SpeedRow], bump: SpeedBump) -> (Intent, Tier) {
    if nav != Intent::Walk {
        return (nav, Tier::Default);
    }
    let run = |tier| (Intent::Run, tier);
    match speed.and_then(|s| speed_row(rows, s, bump)) {
        Some(r) if r.state == "ChaseRun" => run(r.tier),
        Some(r) if r.state == "Walk" => (Intent::Walk, r.tier),
        Some(r) if r.state == "Shuffle" => (Intent::Shuffle, Tier::Default),
        Some(_) => (Intent::Walk, Tier::Default),
        None if running => {
            let runs = || rows.iter().filter(|r| r.state == "ChaseRun");
            run(runs().find(|r| r.tier == Tier::Default).or_else(|| runs().next()).map_or(Tier::Default, |r| r.tier))
        }
        None => (Intent::Walk, Tier::Default),
    }
}

/// A strafe step direction (`sub_82E2AFC0`'s Move* intents).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StrafeMove {
    Forwards,
    Backwards,
    Side { degrees: u16, left: bool },
}

/// What the strafe flag posts this update: a turn toward the target, else a step along the move direction.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StrafeInput {
    /// (45 / 90, left).
    pub turn: Option<(u16, bool)>,
    pub step: Option<StrafeMove>,
}

/// The strafe poster's edges (`sub_82E2AFC0`: 25 / 70 / 115 / 160 degrees, inclusive; the turn bucket `sub_82E2A500`
/// with mask 0xCC keeps 45 and 90 only; b686) [code].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StrafeRules {
    pub edges: [f32; 4],
    /// The step aims at the first path node farther than this, m (`sub_82E2A6B0`: 3D squared distance > 0.01; b695).
    pub node_reach: f32,
}

impl Default for StrafeRules {
    fn default() -> Self {
        Self { edges: [25.0, 70.0, 115.0, 160.0], node_reach: 0.1 }
    }
}

/// Signed degrees from `heading` (yaw, `atan2(x, z)`) to the flat direction `dir`; positive = left (ours; retail's
/// positive is right).
fn signed_degrees(heading: f32, dir: [f32; 2]) -> f32 {
    let mut e = dir[0].atan2(dir[1]) - heading;
    while e > std::f32::consts::PI {
        e -= std::f32::consts::TAU;
    }
    while e < -std::f32::consts::PI {
        e += std::f32::consts::TAU;
    }
    e.to_degrees()
}

/// `sub_82E2ADE0` + `sub_82E2A500` (mask 0xCC), then `sub_82E2AFC0`: a turn toward `target` (3D, ped to target) from 25
/// degrees off the facing (90 from 70 up); with no turn and `speed > 0`, the step along `step_dir` (none: Forwards).
pub fn strafe_input(heading: f32, target: [f32; 3], step_dir: Option<[f32; 2]>, speed: f32, rules: &StrafeRules) -> StrafeInput {
    let [e1, e2, e3, e4] = rules.edges;
    // `sub_82E2AB18` normalises the target in 3D: the size is acos of the 3D dot with the flat facing, the side from the
    // flat cross (b695).
    let length = (target[0] * target[0] + target[1] * target[1] + target[2] * target[2]).sqrt();
    let turn = (length > 1e-4 && target[0] * target[0] + target[2] * target[2] > 1e-8).then(|| {
        let side = if signed_degrees(heading, [target[0], target[2]]) > 0.0 { 1.0 } else { -1.0 };
        let dot = (target[0] * heading.sin() + target[2] * heading.cos()) / length;
        side * dot.clamp(-1.0, 1.0).acos().to_degrees()
    }).and_then(|d| {
        let left = d > 0.0;
        if d.abs() >= e2 {
            Some((90, left))
        } else if d.abs() >= e1 {
            Some((45, left))
        } else {
            None
        }
    });
    let step = (turn.is_none() && speed > 0.0).then(|| {
        let d = step_dir.filter(|v| v[0] * v[0] + v[1] * v[1] > 1e-8).map_or(0.0, |v| signed_degrees(heading, v));
        let left = d > 0.0;
        match d.abs() {
            a if a >= e4 => StrafeMove::Backwards,
            a if a >= e3 => StrafeMove::Side { degrees: 135, left },
            a if a >= e2 => StrafeMove::Side { degrees: 90, left },
            a if a >= e1 => StrafeMove::Side { degrees: 45, left },
            _ => StrafeMove::Forwards,
        }
    });
    StrafeInput { turn, step }
}

/// The tier a cycle of `state` uses: Brisk is a walk cycle, Slow / Fast are run cycles.
fn cycle_tier(state: Locomotion, tier: Tier) -> Tier {
    match (state, tier) {
        (Locomotion::Run, Tier::Slow | Tier::Fast) | (Locomotion::Walk, Tier::Brisk) => tier,
        _ => Tier::Default,
    }
}

/// The row a speed picks: the highest threshold at or below the bumped speed.
pub fn speed_row(rows: &[SpeedRow], speed: f32, bump: SpeedBump) -> Option<&SpeedRow> {
    let s = if speed > bump.above { speed + bump.add } else { speed };
    rows.iter().filter(|r| r.threshold <= s).max_by(|a, b| a.threshold.total_cmp(&b.threshold))
}

/// The logical names M2 plays (`MotionGraph_Pedestrian.xml` includes) [data].
pub mod names {
    pub const IDLE: &str = "IdleBasicCyc";
    pub const WALK: &str = "FwdWalkCyc";
    pub const START: &str = "Stand2Walk";
    pub const STOP: &str = "Walk2Stand";
    pub const TURN_180: &str = "StandTurnR180";
    /// The turn clip of a bucket (`motiongraph_idle.xml` IdleDefault / `motiongraph_walk.xml` Walk turn branches, the
    /// right-hand clips mirrored for the left; b156 M1 / M2) [data].
    pub fn stand_turn(degrees: u16) -> &'static str {
        match degrees {
            45 => "StandTurnR45",
            90 => "StandTurnR90",
            135 => "StandTurnR135",
            _ => TURN_180,
        }
    }
    pub fn walk_turn(degrees: u16) -> &'static str {
        match degrees {
            45 => "WalkTurnR45",
            90 => "WalkTurnR90",
            135 => "WalkTurnR135",
            _ => "WalkTurnR180",
        }
    }
    /// The chase / flee run cycle (stock `*_CHASE_RUN_N_0_CYC`, logical name hash
    /// `7746273E01422734`).
    pub const RUN: &str = "FwdChaseRunCyc";
    pub const BRISK_WALK: &str = "FwdBriskWalkCyc";
    pub const SLOW_RUN: &str = "FwdChaseRunSlowCyc";
    /// The run's own start and stop (`motiongraph_chaserun.xml` ChaseIntoDefault, `motiongraph_idle.xml`
    /// ChaseRunToIdle; research b684) [data].
    pub const RUN_START: &str = "Stand2ChaseRun";
    pub const RUN_STOP: &str = "ChaseRun2Stand";
    /// The Shuffle state (`motiongraph_shuffle.xml`, the speed row from 1.0; research b686) [data].
    pub const SHUFFLE_START: &str = "Stand2Shuffle";
    pub const SHUFFLE: &str = "FwdShuffleCyc";
    pub const SHUFFLE_STOP: &str = "Shuffle2Stand";
    pub fn shuffle_turn(degrees: u16) -> &'static str {
        match degrees {
            45 => "ShuffleTurnR45",
            90 => "ShuffleTurnR90",
            135 => "ShuffleTurnR135",
            _ => "ShuffleTurnR180",
        }
    }
    /// The Strafe state (`motiongraph_strafe.xml`, template `strafemotion.xml`; BlockChasee; research b686) [data].
    pub const STRAFE_INTO: &str = "Stand2Strafe";
    pub const CHASE_TO_STRAFE: &str = "Chase2Strafe";
    pub const STRAFE_IDLE: &str = "StrafeIdle";
    pub const STRAFE_TO_RUN: &str = "Strafe2ChaseRun";
    /// Strafe turns: 45 and 90 only (the poster's mask 0xCC).
    pub fn strafe_turn(degrees: u16) -> &'static str {
        if degrees <= 45 {
            "StrafeTurnR45"
        } else {
            "StrafeTurnR90"
        }
    }
    /// A strafe step's clip; a left step mirrors the right one.
    pub fn strafe_move(m: super::StrafeMove) -> (&'static str, bool) {
        match m {
            super::StrafeMove::Forwards => ("StrafeForwards", false),
            super::StrafeMove::Backwards => ("StrafeBackwards", false),
            super::StrafeMove::Side { degrees, left } => (if degrees <= 45 { "StrafeR45" } else if degrees <= 90 { "StrafeR90" } else { "StrafeR135" }, left),
        }
    }
    /// The Tired state (`motiongraph_tired.xml`, RestFromChase; b684) [data].
    pub const TIRED_INTO: &str = "ChaseTiredInto";
    pub const TIRED_CYCLE: &str = "ChaseTiredCyc";
    pub const TIRED_OUT: &str = "ChaseTiredOut";
    /// A turn while running (`blendedturn` on `ChaseRun.ChaseRunCyc`, right-hand clips mirrored for the left).
    pub fn run_turn(degrees: u16) -> &'static str {
        match degrees {
            45 => "ChaseRunTurnR45",
            90 => "ChaseRunTurnR90",
            135 => "ChaseRunTurnR135",
            _ => "ChaseRunTurnR180",
        }
    }
    pub const FAST_RUN: &str = "FwdChaseRunFastCyc";
    /// Every logical name the loader resolves (more for later milestones; a mod may add).
    pub const ALL: &[&str] = &[
        "IdleBasicCyc", "FwdWalkCyc", "FwdBriskWalkCyc", "Stand2Walk", "Walk2Stand", "StandTurnR45", "StandTurnR90", "StandTurnR135",
        "StandTurnR180", "StandTurnR180Out", "WalkTurnR45", "WalkTurnR90", "WalkTurnR135", "WalkTurnR180", "WalkTurnR180Out", "FwdShuffleCyc",
        "Stand2Shuffle", "Shuffle2Stand", "FwdChaseRunCyc", "FwdChaseRunSlowCyc", "FwdChaseRunFastCyc",
        "Stand2ChaseRun", "ChaseRun2Stand", "ChaseRunTurnR45", "ChaseRunTurnR90", "ChaseRunTurnR135", "ChaseRunTurnR180",
        "ChaseTiredInto", "ChaseTiredCyc", "ChaseTiredOut", "ShuffleTurnR45", "ShuffleTurnR90", "ShuffleTurnR135", "ShuffleTurnR180",
        "Stand2Strafe", "Chase2Strafe", "StrafeIdle", "Strafe2ChaseRun", "StrafeTurnR45", "StrafeTurnR90", "StrafeForwards",
        "StrafeBackwards", "StrafeR45", "StrafeR90", "StrafeR135",
        // Collision reactions (`super::skater_contact::REACTION_ANIMS`).
        "CollisionBackStanding", "CollisionFwdStanding", "CollisionLeftStanding",
        "WipeoutBackFall", "WipeoutBackGroundCyc", "WipeoutBackGetUp",
        "WipeoutFwdFall", "WipeoutFwdGroundCyc", "WipeoutFwdGetUp",
        "WipeoutLeftFall", "WipeoutLeftGroundCyc", "WipeoutLeftGetUp",
        "WipeoutBackCrouch", "WipeoutFwdCrouch", "WipeoutLeftCrouch",
        // motiongraph_taunt.
        "Taunt",
        // The plugin states (`super::plugin_motion`): sit, ATM, vending machine, water fountain, newspaper box.
        "Stand2Sit", "SitIdleCyc", "Sit2Stand", "SitAvoidFwd", "SitAvoidLeft", "SitAvoidBack", "SitAvoidSmallFront",
        "SitAvoidSmallLeft", "SitAvoidSmallBack", "AvoidJumpForward", "AvoidJumpLeft", "AvoidJumpBack", "AvoidStepForward", "AvoidStepLeft", "AvoidStepBack", "ATMInsertCard", "ATMMakeSelection", "ATMCollectMoney", "ATMCollectCard",
        "VendInsert", "VendSelect", "VendCollect", "WaterFountainInto", "WaterFountainCyc", "WaterFountainOut", "NewspaperCollect",
        // The light hand prop throw (`super::hand_prop::light_throw_clip`).
        "HandPropThrowLightForward", "HandPropThrowLightL45", "HandPropThrowLightL90", "HandPropThrowLightR180",
        "HandPropThrowLightR90", "HandPropThrowLightR45",
        // The attack throw (`super::hand_prop::attack_throw_clip`).
        "HandPropAttackThrow", "HandPropAttackThrowLeft", "HandPropAttackThrowRight",
        // The hand prop carry poses on the ped channel (`livingworld_handprops` `Hash_FC1D2C4E5CCA6AED`, b99).
        "CarrySmallRHChannel", "CarryBigRHChannel", "CarryPaperRHChannel", "CarryWineRHChannel",
        // The idle hand prop actions on the channel (`82E3F1D8` / `82E3F470`, strings `0x82064AA0..0x82064B3C`).
        "HandPropEat", "HandPropDrink", "HandPropReadInto", "HandPropReadCyc", "HandPropReadOut", "HandPropPhoneAnswer",
        "HandPropPhoneTalk", "HandPropPhoneHangup", "HandPropLitter",
        // The tazer on the channel (DrawTazer / `82E3ED50` / EndTaze, b248).
        "TazerInto", "TazerCyc", "TazerOut",
        // The greet / warn gestures on the channel (ChannelGreet / ChannelWarnWantTarget Begin, b677).
        "GreetChannel", "WarnChannel",
    ];
}

/// Blend times and exits from the motion graph [data].
pub mod timing {
    pub const IDLE_BLEND: f32 = 0.1;
    pub const IDLE_CYCLE_SWAP: f32 = 0.5;
    pub const START_BLEND: f32 = 0.25;
    pub const START_EXIT: f32 = 0.03;
    pub const WALK_BLEND: f32 = 0.1;
    pub const TRANSITION_BLEND: f32 = 0.15;
    pub const TRANSITION_EXIT: f32 = 0.1;
    pub const TIRED_BLEND: f32 = 0.1;
    pub const TIRED_EXIT: f32 = 0.01;
}

/// The locomotion state (subset of `Motion.Locomotion` / `Idle`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Locomotion {
    Idle,
    Start,
    Walk,
    /// Running (flee / chase): the run cycle; it starts through `Stand2ChaseRun` (`RunStart`), turns with the run
    /// turn clips, stops through `ChaseRun2Stand` and goes back to walking through `Stand2Walk`.
    Run,
    RunStart,
    RunTurnRight,
    RunTurnLeft,
    /// Shuffling (a slow approach, e.g. onto a plugin point): `Stand2Shuffle` in, the shuffle cycle, its turns.
    ShuffleStart,
    Shuffle,
    ShuffleTurnRight,
    ShuffleTurnLeft,
    /// Strafing (BlockChasee): in, standing, a step, a turn (45 / 90).
    StrafeInto,
    StrafeIdle,
    StrafeMove,
    StrafeTurnRight,
    StrafeTurnLeft,
    /// Tired after a chase (RestFromChase): into from the run cycle, the tired cycle until rested, out.
    TiredInto,
    TiredCycle,
    TiredOut,
    Stop,
    TurnRight,
    TurnLeft,
    /// A walk turn (`blendedturn` in Walk: the turn clip, then back to the walk cycle).
    WalkTurnRight,
    WalkTurnLeft,
    /// A collision reaction (`Collision` state of the motion graph): no locomotion until it ends.
    Reaction,
}

impl Locomotion {
    pub fn name(self) -> &'static str {
        match self {
            Locomotion::Idle => "idle",
            Locomotion::Start => "start",
            Locomotion::Walk => "walk",
            Locomotion::Run => "run",
            Locomotion::RunStart => "run_start",
            Locomotion::RunTurnRight => "run_turn_right",
            Locomotion::RunTurnLeft => "run_turn_left",
            Locomotion::ShuffleStart => "shuffle_start",
            Locomotion::Shuffle => "shuffle",
            Locomotion::ShuffleTurnRight => "shuffle_turn_right",
            Locomotion::ShuffleTurnLeft => "shuffle_turn_left",
            Locomotion::StrafeInto => "strafe_into",
            Locomotion::StrafeIdle => "strafe_idle",
            Locomotion::StrafeMove => "strafe_move",
            Locomotion::StrafeTurnRight => "strafe_turn_right",
            Locomotion::StrafeTurnLeft => "strafe_turn_left",
            Locomotion::TiredInto => "tired_into",
            Locomotion::TiredCycle => "tired",
            Locomotion::TiredOut => "tired_out",
            Locomotion::Stop => "stop",
            Locomotion::TurnRight => "turn_right",
            Locomotion::TurnLeft => "turn_left",
            Locomotion::WalkTurnRight => "walk_turn_right",
            Locomotion::WalkTurnLeft => "walk_turn_left",
            Locomotion::Reaction => "reaction",
        }
    }
}

/// What the ped wants (later: the AI graph's intents).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Intent {
    Idle,
    Walk,
    /// The Shuffle speed row.
    Shuffle,
    Run,
    TurnRight,
    TurnLeft,
}

#[derive(Clone, Debug, PartialEq)]
struct Layer {
    clip: String,
    windows: Vec<(f32, f32, i32)>,
    time: f32,
    previous_time: f32,
    wrapped: bool,
    mirror: bool,
}

/// Root motion of one step, in the ped's frame before the step (x right, z forward, metres;
/// yaw radians about +y, counter-clockwise seen from above).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RootMotion {
    pub translation: [f32; 3],
    pub yaw: f32,
}

/// Per-step outputs: root motion and the channels the audio reads.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StepOut {
    pub root: RootMotion,
    /// `[left, right]` toe down.
    pub feet_down: [bool; 2],
    pub body_fall: f32,
    pub entered: Option<Locomotion>,
}

/// The player: current layer, a fading previous layer, the locomotion state and its RNG.
#[derive(Clone, Debug, PartialEq)]
pub struct PedAnimPlayer {
    pub state: Locomotion,
    current: Layer,
    previous: Option<Layer>,
    blend_elapsed: f32,
    blend_time: f32,
    rng: Rng,
    pub intent: Intent,
    /// The bucket of a turn intent (45 / 90 / 135 / 180; [`super::wander::NavOutput::turn_degrees`]).
    pub turn_degrees: u16,
    /// The speed row's tier; the walk / run cycle follows it at the next branch window (blend 0.1).
    pub tier: Tier,
    /// The tier of the cycle playing now.
    cycle_tier: Tier,
    /// The brain rests from a chase (RestFromChase's Tired packet; its end posts StopTired).
    pub resting: bool,
    /// The strafe flag's intents ([`strafe_input`]); `None` = no strafe.
    pub strafe: Option<StrafeInput>,
    /// The strafe step playing now.
    strafe_playing: Option<StrafeMove>,
    plays: u32,
    reaction: Option<ReactionRun>,
    channel: Option<ChannelRun>,
}

/// The ped's one channel slot ("PedChannel", the skeleton controller at `skeleton+17936`; requests `82E32878`, state
/// machine `82E3A5A0`, research b99) [code]: a clip played over locomotion on the bones its channel weights pick
/// (carry poses, the tazer, greet / warn gestures). A request replaces what plays; a stop fades it out.
#[derive(Clone, Debug, PartialEq)]
struct ChannelRun {
    clip: String,
    time: f32,
    /// Fade 0..1 and the seconds to full / to nothing.
    weight: f32,
    blend_in: f32,
    blend_out: f32,
    stopping: bool,
}

/// A running collision reaction: its steps, the current one and the ground time left.
#[derive(Clone, Debug, PartialEq)]
struct ReactionRun {
    steps: Vec<super::skater_contact::ReactionStep>,
    index: usize,
    ground_left: f32,
    /// The ground time a knock-down re-arms to (the set's `ground_seconds`).
    ground_seconds: f32,
    /// Retail's PedestrianColliding knock-down state (`sub_8269DEE0` states 1 / 2, b372); `None` for a stumble or
    /// a plugin sequence.
    knockdown: Option<Knockdown>,
}

/// A knock-down's Colliding state: the stored direction (`s+0x18`), the timer expiries (`s+0x14`, copied to the
/// contact gate `c+4`), a contact waiting for the expiry (`c+8`) and whether `Recover` was posted (state 2).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Knockdown {
    direction: super::skater_contact::ReactionDirection,
    expiries: u8,
    pending: bool,
    recovering: bool,
}

/// What a contact did to a knocked-down ped ([`PedAnimPlayer::knockdown_contact`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum KnockdownContact {
    /// Not knocked down, or contacts are blocked (too many expiries).
    Ignored,
    /// Down: the contact waits for the ground timer's expiry, which then re-arms it (`sub_8269E058`).
    Pending,
    /// Getting up: a re-hit from this posture and direction (`sub_8269E170`).
    ReHit(super::skater_contact::Posture, super::skater_contact::ReactionDirection),
}

/// Seed label of the animation sub-RNG.
pub const ANIM_LABEL: u64 = 0x5045_4441_4E49_4D00;

fn pick(set: &PedAnimSet, name: &str, rng: &mut Rng) -> Option<RemapClip> {
    let list = set.entries.get(name)?;
    if list.is_empty() {
        return None;
    }
    Some(list[rng.modulo(list.len() as u32) as usize].clone())
}

fn quat_yaw(q: [f32; 4]) -> f32 {
    // Rotation about +y only is (0, sin(a/2), 0, cos(a/2)); project the delta onto it.
    2.0 * q[1].atan2(q[3])
}

impl PedAnimPlayer {
    /// A new player in the idle state (`None` when the set has no idle clip).
    pub fn new(set: &PedAnimSet, seed: u64) -> Option<Self> {
        let mut rng = Rng::new(derive(seed, &[ANIM_LABEL]));
        let idle = pick(set, names::IDLE, &mut rng)?;
        Some(Self {
            state: Locomotion::Idle,
            current: Layer { clip: idle.clip, windows: idle.windows, time: 0.0, previous_time: 0.0, wrapped: false, mirror: false },
            previous: None,
            blend_elapsed: 0.0,
            blend_time: 0.0,
            rng,
            intent: Intent::Idle,
            turn_degrees: 180,
            tier: Tier::Default,
            cycle_tier: Tier::Default,
            resting: false,
            strafe: None,
            strafe_playing: None,
            plays: 0,
            reaction: None,
            channel: None,
        })
    }

    pub fn current_clip(&self) -> &str {
        &self.current.clip
    }

    pub fn current_time(&self) -> f32 {
        self.current.time
    }

    pub fn mirrored(&self) -> bool {
        self.current.mirror
    }

    /// The incoming layer's blend weight (1 = no blend in progress).
    pub fn blend_weight(&self) -> f32 {
        if self.previous.is_none() || self.blend_time <= 0.0 { 1.0 } else { (self.blend_elapsed / self.blend_time).clamp(0.0, 1.0) }
    }

    /// Strafe flag cleared: back to locomotion without a stand clip; a run plays `Strafe2ChaseRun`.
    fn leave_strafe(&mut self, set: &PedAnimSet) -> bool {
        self.strafe_playing = None;
        let moved = match self.intent {
            Intent::Walk => self.play_cycle(set, timing::IDLE_BLEND, Locomotion::Walk),
            Intent::Shuffle => self.play(set, names::SHUFFLE, timing::IDLE_BLEND, false, Locomotion::Shuffle),
            Intent::Run => self.play(set, names::STRAFE_TO_RUN, timing::START_BLEND, false, Locomotion::RunStart) || self.play_cycle(set, timing::IDLE_BLEND, Locomotion::Run),
            _ => false,
        };
        moved || self.play(set, names::IDLE, timing::IDLE_BLEND, false, Locomotion::Idle)
    }

    /// TiredComplete: back to locomotion with LastState Tired: Idle and Walk start their cycles directly (their into
    /// lists have no Tired), a run plays the plain `Stand2ChaseRun`.
    fn leave_tired(&mut self, set: &PedAnimSet) {
        let ok = match self.intent {
            Intent::Walk => self.play_cycle(set, timing::IDLE_BLEND, Locomotion::Walk),
            Intent::Run => self.play(set, names::RUN_START, timing::START_BLEND, false, Locomotion::RunStart) || self.play_cycle(set, timing::IDLE_BLEND, Locomotion::Run),
            _ => false,
        };
        if !ok {
            self.play(set, names::IDLE, timing::IDLE_BLEND, false, Locomotion::Idle);
        }
    }

    /// The walk or run cycle of the speed row's tier (`WalkCyc` / `ChaseRunCyc`, blend 0.1 [data]); a set without
    /// the tier's clip plays the default cycle.
    fn play_cycle(&mut self, set: &PedAnimSet, blend: f32, state: Locomotion) -> bool {
        let tier = cycle_tier(state, self.tier);
        let (tiered, base) = match (state, tier) {
            (Locomotion::Run, Tier::Slow) => (Some(names::SLOW_RUN), names::RUN),
            (Locomotion::Run, Tier::Fast) => (Some(names::FAST_RUN), names::RUN),
            (Locomotion::Run, _) => (None, names::RUN),
            (_, Tier::Brisk) => (Some(names::BRISK_WALK), names::WALK),
            _ => (None, names::WALK),
        };
        let played = tiered.is_some_and(|n| self.play(set, n, blend, false, state)) || self.play(set, base, blend, false, state);
        if played {
            self.cycle_tier = tier;
        }
        played
    }

    fn play(&mut self, set: &PedAnimSet, name: &str, blend: f32, mirror: bool, state: Locomotion) -> bool {
        let Some(next) = pick(set, name, &mut self.rng) else { return false };
        let old = std::mem::replace(&mut self.current, Layer { clip: next.clip, windows: next.windows, time: 0.0, previous_time: 0.0, wrapped: false, mirror });
        self.previous = Some(old);
        self.blend_elapsed = 0.0;
        self.blend_time = blend;
        self.state = state;
        self.plays += 1;
        true
    }

    /// Start a collision reaction ([`super::skater_contact::reaction_steps`]); `false` when the set
    /// has none of its first animation or a reaction is already running.
    pub fn react(&mut self, set: &PedAnimSet, steps: Vec<super::skater_contact::ReactionStep>, ground_seconds: f32) -> bool {
        if self.reaction.is_some() || steps.is_empty() {
            return false;
        }
        let first = steps[0];
        if !self.play(set, first.anim, first.blend, first.mirror, Locomotion::Reaction) {
            return false;
        }
        self.reaction = Some(ReactionRun { steps, index: 0, ground_left: ground_seconds, ground_seconds, knockdown: None });
        true
    }

    /// Mark the reaction just started as a knock-down from `direction` (`sub_8269DA00` kind 0): its ground timer,
    /// contacts and re-hits follow retail's Colliding state machine from here on.
    pub fn begin_knockdown(&mut self, direction: super::skater_contact::ReactionDirection) {
        if let Some(r) = self.reaction.as_mut() {
            r.knockdown = Some(Knockdown { direction, expiries: 0, pending: false, recovering: false });
        }
    }

    /// Whether a knock-down is running (the ped is down or getting up).
    pub fn knocked_down(&self) -> bool {
        self.reaction.as_ref().is_some_and(|r| r.knockdown.is_some())
    }

    /// A contact on a knocked-down ped (82E38FB8; consumed by 8269E058 / 8269E170): docs/hails-additions/26f-living-world-peds.md#knockdown-contact
    pub fn knockdown_contact(&mut self, set: &PedAnimSet, direction: super::skater_contact::ReactionDirection) -> KnockdownContact {
        use super::skater_contact::{rehit_steps, Posture};
        let rules = set.collision;
        let getup_time = self.current.time;
        let Some(r) = self.reaction.as_mut() else { return KnockdownContact::Ignored };
        let Some(k) = r.knockdown.as_mut() else { return KnockdownContact::Ignored };
        if k.expiries > rules.block_after {
            return KnockdownContact::Ignored;
        }
        if !k.recovering {
            k.pending = true;
            return KnockdownContact::Pending;
        }
        let posture = rules.posture(getup_time, k.direction);
        let dir = if posture == Posture::LyingDown { k.direction } else { direction };
        let mut next = *k;
        if posture != Posture::LyingDown {
            next.direction = direction;
            next.expiries = 0;
        }
        next.recovering = false;
        next.pending = false;
        let (steps, start) = rehit_steps(posture, dir, &rules);
        let first = steps[0];
        if !self.play(set, first.anim, first.blend, first.mirror, Locomotion::Reaction) {
            return KnockdownContact::Ignored;
        }
        self.current.time = start;
        self.current.previous_time = start;
        let seconds = rules.ground_seconds;
        self.reaction = Some(ReactionRun { steps, index: 0, ground_left: seconds, ground_seconds: seconds, knockdown: Some(next) });
        KnockdownContact::ReHit(posture, dir)
    }

    /// Replace the running reaction or plugin sequence with `steps` (a motion-graph transition out of it, e.g. the
    /// Avoid graph's SitJump from `Locomotion.Sit.SittingCyc`); the old one stays when the new one cannot start.
    pub fn interrupt(&mut self, set: &PedAnimSet, steps: Vec<super::skater_contact::ReactionStep>, ground_seconds: f32) -> bool {
        let running = self.reaction.take();
        if self.react(set, steps, ground_seconds) {
            return true;
        }
        self.reaction = running;
        false
    }

    /// Release a held cycle step (a plugin state's `HasIntent <next stage>` transition): it ends now.
    pub fn release_hold(&mut self) {
        if let Some(r) = self.reaction.as_mut() {
            if r.steps[r.index].cycle {
                r.ground_left = 0.0;
            }
        }
    }

    /// The running reaction's steps from the current one on (a clip that returns to it plays these after).
    pub fn remaining_steps(&self) -> Vec<super::skater_contact::ReactionStep> {
        self.reaction.as_ref().map(|r| r.steps[r.index..].to_vec()).unwrap_or_default()
    }

    /// Whether a reaction (or plugin sequence) is running.
    pub fn reacting(&self) -> bool {
        self.reaction.is_some()
    }

    /// The running reaction's current logical animation, if any.
    pub fn reaction_anim(&self) -> Option<&'static str> {
        self.reaction.as_ref().map(|r| r.steps[r.index].anim)
    }

    /// The current clip is in a branch window (`InTurnBranchWindow`; a clip without windows always is).
    pub fn in_branch_window(&self) -> bool {
        self.current.windows.is_empty() || self.current.windows.iter().any(|&(s, e, tag)| tag != 0 && self.current.time >= s && self.current.time <= e)
    }

    /// Play the logical animation `name` of `set` on the channel (ped vfunc +240 `82E3A418`: the name through the
    /// anim set, then a request with blend in / out), replacing what plays. False when the set has no such clip.
    pub fn channel_request(&mut self, set: &PedAnimSet, name: &str, blend_in: f32, blend_out: f32) -> bool {
        let Some(entry) = pick(set, name, &mut self.rng) else { return false };
        let weight = self.channel.as_ref().filter(|c| !c.stopping).map_or(0.0, |c| c.weight);
        self.channel = Some(ChannelRun { clip: entry.clip, time: 0.0, weight, blend_in, blend_out, stopping: false });
        true
    }

    /// Stop the channel with a fade (`state 5`, fade `blend_out`); no-op when idle.
    pub fn channel_stop(&mut self, fade: f32) {
        if let Some(c) = self.channel.as_mut() {
            c.stopping = true;
            c.blend_out = fade;
        }
    }

    /// Stop the channel with its clip's own blend-out (the Collision state machine's `StopChannel` behaviour,
    /// `motiongraph_collision.xml`; NOT RETAIL YET: the behaviour's fade is in code not read).
    pub fn channel_stop_own(&mut self) {
        if let Some(c) = self.channel.as_mut() {
            c.stopping = true;
        }
    }

    /// The slot is free (`skeleton+140 == 7`).
    pub fn channel_idle(&self) -> bool {
        self.channel.is_none()
    }

    /// Seconds left of the channel's clip (0 when idle or the clip is unknown): the anim's +0x24 query that the
    /// hand prop actions compare against 0.2 s (`82E3F470`) [inferred: time left].
    pub fn channel_remaining(&self, clips: &dyn PedClips) -> f32 {
        self.channel.as_ref().and_then(|c| clips.clip(&c.clip).map(|clip| (clip.length() - c.time).max(0.0))).unwrap_or(0.0)
    }

    /// The clip on the channel, if any.
    pub fn channel_clip(&self) -> Option<&str> {
        self.channel.as_ref().map(|c| c.clip.as_str())
    }

    fn step_channel(&mut self, dt: f32, clips: &dyn PedClips) {
        let Some(c) = self.channel.as_mut() else { return };
        let Some(clip) = clips.clip(&c.clip) else {
            self.channel = None;
            return;
        };
        let len = clip.length();
        c.time += dt;
        if clip.looping && len > 0.0 {
            c.time %= len;
        } else if c.time >= len {
            // A one-shot clip ends: the slot fades out.
            c.time = len;
            c.stopping = true;
        }
        let rate = |s: f32| if s > 0.0 { dt / s } else { 1.0 };
        if c.stopping {
            c.weight -= rate(c.blend_out);
            if c.weight <= 0.0 {
                self.channel = None;
            }
        } else {
            c.weight = (c.weight + rate(c.blend_in)).min(1.0);
        }
    }

    /// Advance by `dt` seconds (the host steps once per population world tick, dt = 1/60).
    pub fn step(&mut self, dt: f32, set: &PedAnimSet, clips: &dyn PedClips) -> StepOut {
        self.step_channel(dt, clips);
        let mut out = StepOut::default();
        // Clock: advance both layers (looping clips wrap with the loop transform).
        let mut root = [RootMotion::default(), RootMotion::default()];
        for (i, layer) in [Some(&mut self.current), self.previous.as_mut()].into_iter().enumerate() {
            let Some(layer) = layer else { continue };
            let Some(clip) = clips.clip(&layer.clip) else { continue };
            let len = clip.length();
            layer.previous_time = layer.time;
            layer.time += dt;
            layer.wrapped = false;
            if clip.looping && len > 0.0 && layer.time > len {
                layer.time -= len;
                layer.wrapped = true;
            } else if layer.time > len {
                layer.time = len;
            }
            root[i] = root_motion(clip, layer.previous_time, layer.time, layer.wrapped, layer.mirror);
        }
        let w = if self.previous.is_some() {
            self.blend_elapsed += dt;
            self.blend_weight()
        } else {
            1.0
        };
        out.root = RootMotion {
            translation: std::array::from_fn(|k| root[1].translation[k] + (root[0].translation[k] - root[1].translation[k]) * w),
            yaw: root[1].yaw + (root[0].yaw - root[1].yaw) * w,
        };
        if self.previous.is_none() {
            out.root = root[0];
        }
        if w >= 1.0 {
            self.previous = None;
        }
        // Channels from the incoming layer (mirrored = feet swapped).
        if let Some(clip) = clips.clip(&self.current.clip) {
            let l = clip.channel("LEFTTOEDOWN", self.current.time) != 0.0;
            let r = clip.channel("RIGHTTOEDOWN", self.current.time) != 0.0;
            out.feet_down = if self.current.mirror { [r, l] } else { [l, r] };
            out.body_fall = clip.channel("BODYFALLTYPE", self.current.time);
        }
        // Transitions.
        let remaining = clips.clip(&self.current.clip).map_or(0.0, |c| c.length() - self.current.time);
        let before = self.plays;
        // Tired from Idle / Walk at once (NotFromRun), from the run cycle at its branch window (FromRun; the graph's
        // `TiredBranchInfo` is not decoded, ours: the run cycle's windows).
        let tired = self.resting
            && match self.state {
                Locomotion::Idle | Locomotion::Walk => self.play(set, names::TIRED_CYCLE, timing::TIRED_BLEND, false, Locomotion::TiredCycle),
                Locomotion::Run => self.in_branch_window() && self.play(set, names::TIRED_INTO, timing::TIRED_BLEND, false, Locomotion::TiredInto),
                _ => false,
            };
        // Strafe in (`StrafeInto` from Walk / Shuffle / Idle, `StrafeFromChase` from the run; at the branch window) and out
        // (no Strafe2Stand: IdleInto / WalkInto / ShuffleInto do not list Strafe, the cycles start directly).
        let strafing = matches!(self.state, Locomotion::StrafeIdle | Locomotion::StrafeMove | Locomotion::StrafeTurnRight | Locomotion::StrafeTurnLeft);
        let strafe = !tired
            && match (self.strafe.is_some(), self.state) {
                (true, Locomotion::Idle) => self.play(set, names::STRAFE_INTO, timing::START_BLEND, false, Locomotion::StrafeInto),
                (true, Locomotion::Walk | Locomotion::Shuffle) => self.in_branch_window() && self.play(set, names::STRAFE_INTO, timing::START_BLEND, false, Locomotion::StrafeInto),
                (true, Locomotion::Run) => self.in_branch_window() && self.play(set, names::CHASE_TO_STRAFE, timing::START_BLEND, false, Locomotion::StrafeInto),
                (false, Locomotion::StrafeIdle) => self.leave_strafe(set),
                (false, Locomotion::StrafeMove) => self.in_branch_window() && self.leave_strafe(set),
                (false, _) if strafing => remaining <= timing::TRANSITION_EXIT && self.leave_strafe(set),
                _ => false,
            };
        match self.state {
            _ if tired || strafe => {}
            Locomotion::StrafeInto => {
                if remaining <= timing::START_EXIT {
                    self.strafe_playing = None;
                    self.play(set, names::STRAFE_IDLE, timing::IDLE_BLEND, false, Locomotion::StrafeIdle);
                }
            }
            Locomotion::StrafeIdle | Locomotion::StrafeMove => {
                let input = self.strafe.unwrap_or_default();
                if self.state == Locomotion::StrafeIdle || self.in_branch_window() {
                    if let Some((degrees, left)) = input.turn {
                        let state = if left { Locomotion::StrafeTurnLeft } else { Locomotion::StrafeTurnRight };
                        self.strafe_playing = None;
                        self.play(set, names::strafe_turn(degrees), timing::TRANSITION_BLEND, left, state);
                    } else if input.step != self.strafe_playing {
                        self.strafe_playing = input.step;
                        match input.step {
                            Some(m) => {
                                let (clip, mirror) = names::strafe_move(m);
                                self.play(set, clip, timing::TRANSITION_BLEND, mirror, Locomotion::StrafeMove);
                            }
                            None => {
                                self.play(set, names::STRAFE_IDLE, timing::IDLE_BLEND, false, Locomotion::StrafeIdle);
                            }
                        }
                    }
                }
            }
            Locomotion::StrafeTurnRight | Locomotion::StrafeTurnLeft => {
                if remaining <= timing::TRANSITION_EXIT {
                    self.play(set, names::STRAFE_IDLE, timing::IDLE_BLEND, false, Locomotion::StrafeIdle);
                }
            }
            Locomotion::TiredInto => {
                if remaining <= timing::TIRED_EXIT {
                    self.play(set, names::TIRED_CYCLE, timing::TIRED_BLEND, false, Locomotion::TiredCycle);
                }
            }
            // StopTired, at the end of a loop (`WillExpire 0.01 always`).
            Locomotion::TiredCycle => {
                if !self.resting && (self.current.wrapped || remaining <= timing::TIRED_EXIT) && !self.play(set, names::TIRED_OUT, timing::TIRED_BLEND, false, Locomotion::TiredOut) {
                    self.leave_tired(set);
                }
            }
            Locomotion::TiredOut => {
                if remaining <= timing::TIRED_EXIT {
                    self.leave_tired(set);
                }
            }
            Locomotion::Idle => match self.intent {
                Intent::Walk => {
                    self.play(set, names::START, timing::START_BLEND, false, Locomotion::Start);
                }
                Intent::Run => {
                    if !self.play(set, names::RUN_START, timing::START_BLEND, false, Locomotion::RunStart) && !self.play_cycle(set, timing::START_BLEND, Locomotion::Run) {
                        self.play(set, names::START, timing::START_BLEND, false, Locomotion::Start);
                    }
                }
                Intent::TurnRight => {
                    self.play(set, names::stand_turn(self.turn_degrees), timing::TRANSITION_BLEND, false, Locomotion::TurnRight);
                }
                Intent::TurnLeft => {
                    self.play(set, names::stand_turn(self.turn_degrees), timing::TRANSITION_BLEND, true, Locomotion::TurnLeft);
                }
                Intent::Shuffle => {
                    if !self.play(set, names::SHUFFLE_START, timing::START_BLEND, false, Locomotion::ShuffleStart) {
                        self.play(set, names::START, timing::START_BLEND, false, Locomotion::Start);
                    }
                }
                Intent::Idle => {
                    if self.current.wrapped && self.rng.unit() < timing::IDLE_CYCLE_SWAP {
                        self.play(set, names::IDLE, timing::IDLE_BLEND, false, Locomotion::Idle);
                    }
                }
            },
            Locomotion::Start => {
                if remaining <= timing::START_EXIT && !(self.intent == Intent::Run && self.play_cycle(set, timing::WALK_BLEND, Locomotion::Run)) {
                    self.play_cycle(set, timing::WALK_BLEND, Locomotion::Walk);
                }
            }
            Locomotion::Walk => {
                // `blendedturn`: wait for the branch window, then the walk turn clip (blend 0.15) [data]. A set
                // without the clip keeps walking.
                let turn = match self.intent {
                    Intent::TurnRight => Some((false, Locomotion::WalkTurnRight)),
                    Intent::TurnLeft => Some((true, Locomotion::WalkTurnLeft)),
                    _ => None,
                };
                if let Some((mirror, state)) = turn {
                    if self.in_branch_window() {
                        self.play(set, names::walk_turn(self.turn_degrees), timing::TRANSITION_BLEND, mirror, state);
                    }
                } else if self.intent == Intent::Shuffle && self.in_branch_window() {
                    self.play(set, names::SHUFFLE_START, timing::START_BLEND, false, Locomotion::ShuffleStart);
                } else if self.intent == Intent::Run && self.in_branch_window() {
                    // Walk is in ChaseRunInto's list: the run start plays from the walk too.
                    if !self.play(set, names::RUN_START, timing::START_BLEND, false, Locomotion::RunStart) {
                        self.play_cycle(set, timing::WALK_BLEND, Locomotion::Run);
                    }
                } else if self.intent != Intent::Walk && self.intent != Intent::Run && self.in_branch_window() {
                    self.play(set, names::STOP, timing::TRANSITION_BLEND, false, Locomotion::Stop);
                } else if self.intent == Intent::Walk && cycle_tier(Locomotion::Walk, self.tier) != self.cycle_tier && self.in_branch_window() {
                    self.play_cycle(set, timing::WALK_BLEND, Locomotion::Walk);
                }
            }
            Locomotion::Run => {
                let turn = match self.intent {
                    Intent::TurnRight => Some((false, Locomotion::RunTurnRight)),
                    Intent::TurnLeft => Some((true, Locomotion::RunTurnLeft)),
                    _ => None,
                };
                if !self.in_branch_window() {
                } else if let Some((mirror, state)) = turn {
                    // A set without the run turn stops, as before.
                    if !self.play(set, names::run_turn(self.turn_degrees), timing::TRANSITION_BLEND, mirror, state) {
                        self.play(set, names::STOP, timing::TRANSITION_BLEND, false, Locomotion::Stop);
                    }
                } else if self.intent == Intent::Walk {
                    // WalkInto from ChaseRun: WalkIntoDefault's Stand2Walk.
                    if !self.play(set, names::START, timing::START_BLEND, false, Locomotion::Start) {
                        self.play_cycle(set, timing::WALK_BLEND, Locomotion::Walk);
                    }
                } else if self.intent == Intent::Shuffle {
                    self.play(set, names::SHUFFLE_START, timing::START_BLEND, false, Locomotion::ShuffleStart);
                } else if self.intent == Intent::Idle {
                    if !self.play(set, names::RUN_STOP, timing::TRANSITION_BLEND, false, Locomotion::Stop) {
                        self.play(set, names::STOP, timing::TRANSITION_BLEND, false, Locomotion::Stop);
                    }
                } else if cycle_tier(Locomotion::Run, self.tier) != self.cycle_tier {
                    self.play_cycle(set, timing::WALK_BLEND, Locomotion::Run);
                }
            }
            // `motiongraph_shuffle.xml`: the shuffle start and turns exit at 0.1 into the shuffle cycle (blend 0.1).
            Locomotion::ShuffleStart | Locomotion::ShuffleTurnRight | Locomotion::ShuffleTurnLeft => {
                if remaining <= timing::TRANSITION_EXIT {
                    self.play(set, names::SHUFFLE, timing::IDLE_BLEND, false, Locomotion::Shuffle);
                }
            }
            Locomotion::Shuffle => {
                if self.in_branch_window() {
                    match self.intent {
                        Intent::TurnRight => {
                            self.play(set, names::shuffle_turn(self.turn_degrees), timing::TRANSITION_BLEND, false, Locomotion::ShuffleTurnRight);
                        }
                        Intent::TurnLeft => {
                            self.play(set, names::shuffle_turn(self.turn_degrees), timing::TRANSITION_BLEND, true, Locomotion::ShuffleTurnLeft);
                        }
                        // IdleInto's ShuffleToIdle; WalkInto / ChaseRunInto play their plain starts.
                        Intent::Idle => {
                            if !self.play(set, names::SHUFFLE_STOP, timing::TRANSITION_BLEND, false, Locomotion::Stop) {
                                self.play(set, names::STOP, timing::TRANSITION_BLEND, false, Locomotion::Stop);
                            }
                        }
                        Intent::Walk => {
                            self.play(set, names::START, timing::START_BLEND, false, Locomotion::Start);
                        }
                        Intent::Run => {
                            if !self.play(set, names::RUN_START, timing::START_BLEND, false, Locomotion::RunStart) {
                                self.play_cycle(set, timing::WALK_BLEND, Locomotion::Run);
                            }
                        }
                        Intent::Shuffle => {}
                    }
                }
            }
            // The run start and run turns exit at `WillExpire 0.1` into the run cycle.
            Locomotion::RunStart | Locomotion::RunTurnRight | Locomotion::RunTurnLeft => {
                if remaining <= timing::TRANSITION_EXIT {
                    self.play_cycle(set, timing::WALK_BLEND, Locomotion::Run);
                }
            }
            Locomotion::Stop | Locomotion::TurnRight | Locomotion::TurnLeft => {
                if remaining <= timing::TRANSITION_EXIT {
                    self.play(set, names::IDLE, timing::IDLE_BLEND, false, Locomotion::Idle);
                }
            }
            // `blendedturn` exit (`WillExpire 0.1`, then TurnComplete): back to the walk cycle (its blend is ours).
            Locomotion::WalkTurnRight | Locomotion::WalkTurnLeft => {
                if remaining <= timing::TRANSITION_EXIT {
                    self.play_cycle(set, timing::WALK_BLEND, Locomotion::Walk);
                }
            }
            Locomotion::Reaction => {
                // Ground cycle until Recover; ground time counts from the reaction Begin (b372): docs/hails-additions/26f-living-world-peds.md#ground-time
                let next = match self.reaction.as_mut() {
                    Some(r) => {
                        r.ground_left -= dt;
                        // A knock-down's expiry (`sub_8269E058`): a waiting contact re-arms the timer (stay down,
                        // no clip change); otherwise `Recover` is posted and the ground cycle gives way to the get-up.
                        // Either way the expiry counts toward the contact block.
                        if let Some(k) = r.knockdown.as_mut().filter(|k| !k.recovering && r.ground_left <= 0.0) {
                            k.expiries = k.expiries.saturating_add(1);
                            if std::mem::take(&mut k.pending) {
                                r.ground_left = r.ground_seconds;
                            } else {
                                k.recovering = true;
                            }
                        }
                        match (r.steps[r.index].cycle, r.knockdown) {
                            (true, Some(k)) => k.recovering,
                            (true, None) => r.ground_left <= 0.0,
                            (false, _) => remaining <= 0.01,
                        }
                    }
                    None => true,
                };
                if next {
                    let following = self.reaction.as_mut().and_then(|r| {
                        r.index += 1;
                        r.steps.get(r.index).copied()
                    });
                    match following {
                        Some(step) => {
                            if !self.play(set, step.anim, step.blend, step.mirror, Locomotion::Reaction) {
                                self.reaction = None;
                                self.play(set, names::IDLE, timing::IDLE_BLEND, false, Locomotion::Idle);
                            }
                        }
                        None => {
                            self.reaction = None;
                            self.play(set, names::IDLE, timing::IDLE_BLEND, false, Locomotion::Idle);
                        }
                    }
                }
            }
        }
        if self.plays != before {
            out.entered = Some(self.state);
        }
        out
    }

    /// Local pose at the player time plus `ahead`; each layer made a full pose before mirroring: docs/hails-additions/26f-living-world-peds.md#ped-pose-and-mirror
    pub fn pose(&self, rig: &PedRig, clips: &dyn PedClips, ahead: f32) -> Option<PedFrame> {
        let sample = |layer: &Layer| -> Option<PedFrame> {
            let clip = clips.clip(&layer.clip)?;
            let mut t = layer.time + ahead;
            if clip.looping && clip.length() > 0.0 {
                t %= clip.length();
            }
            let mut f = clip.sample(t)?;
            if f.len() != rig.parents.len() {
                return None;
            }
            add_reference(rig, &mut f);
            if layer.mirror {
                pose_mirror::mirror(&mut f, &rig.parents, &rig.mirrors, 1).ok()?;
            }
            Some(f)
        };
        let mut pose = sample(&self.current)?;
        if let Some(prev) = &self.previous {
            if let Some(p) = sample(prev) {
                let w = self.blend_weight();
                for (a, b) in pose.iter_mut().zip(p) {
                    *a = pose_blend::blend_sample(b, *a, w);
                }
            }
        }
        // The channel over it (`ACSChannelBlend`): per bone, coefficient = fade x the clip's channel weight.
        if let Some((c, clip)) = self.channel.as_ref().and_then(|c| Some((c, clips.clip(&c.clip)?))) {
            if let (Some(weights), Some(mut ch)) = (clip.channel_weights.as_ref(), clip.sample(c.time)) {
                if ch.len() == pose.len() {
                    add_reference(rig, &mut ch);
                    for ((a, mut b), w) in pose.iter_mut().zip(ch).zip(weights) {
                        b.translation[3] = *w;
                        let blended = pose_blend::channel_blend_sample(*a, b, c.weight, false);
                        *a = Sqt { translation: [blended.translation[0], blended.translation[1], blended.translation[2], a.translation[3]], ..blended };
                    }
                }
            }
        }
        if let Some(root) = pose.first_mut() {
            *root = IDENTITY;
        }
        Some(pose)
    }
}

/// Clip delta -> full local pose: `AddSQT(delta, reference)` on the animated bones, the
/// reference on the rest.
fn add_reference(rig: &PedRig, frame: &mut [Sqt]) {
    for (i, s) in frame.iter_mut().enumerate() {
        let reference = rig.reference.get(i).copied().unwrap_or(IDENTITY);
        *s = if rig.animated.get(i).copied().unwrap_or(false) { pose_add::add(*s, reference, true) } else { reference };
    }
}

/// Trajectory delta between two clip times (`pose_trajectory::delta`), as root motion.
pub fn root_motion(clip: &PedClip, previous: f32, time: f32, wrapped: bool, mirror: bool) -> RootMotion {
    let (Some(a), Some(b)) = (clip.trajectory(time), clip.trajectory(previous)) else { return RootMotion::default() };
    let d = pose_trajectory::delta(a, b, wrapped.then_some(pose_trajectory::LoopTransform { rotation: clip.loop_rotation, translation: clip.loop_translation }));
    let mut m = RootMotion { translation: [d.translation[0], d.translation[1], d.translation[2]], yaw: quat_yaw(d.rotation) };
    if mirror {
        m.translation[0] = -m.translation[0];
        m.yaw = -m.yaw;
    }
    m
}

/// Global (model-space) bone matrices from a local pose (`sqt_to_matrix` + parent chain).
pub struct PedEvaluator;

impl PedEvaluator {
    pub fn globals(rig: &PedRig, locals: &[Sqt]) -> Vec<crate::animation::output::NativeMatrix> {
        let mut out: Vec<crate::animation::output::NativeMatrix> = Vec::with_capacity(locals.len());
        for (i, l) in locals.iter().enumerate() {
            let m = crate::animation::output::sqt_to_matrix(*l);
            let g = match rig.parents.get(i).copied() {
                Some(p) if p >= 0 && (p as usize) < i => mul(&out[p as usize], &m),
                _ => m,
            };
            out.push(g);
        }
        out
    }
}

/// Row-vector convention of `NativeMatrix` (translation in row 3): child global = local x parent.
fn mul(parent: &crate::animation::output::NativeMatrix, local: &crate::animation::output::NativeMatrix) -> crate::animation::output::NativeMatrix {
    let mut r = [[0.0f32; 4]; 4];
    for (i, row) in r.iter_mut().enumerate() {
        for (j, v) in row.iter_mut().enumerate() {
            *v = (0..4).map(|k| local[i][k] * parent[k][j]).sum();
        }
    }
    r
}

/// The no-navigation placeholder until M3 (documented in doc 26): idle, start, walk a short
/// straight path, stop, idle, turn round (180, alternating sides), repeat. Durations per ped
/// from its seed (idle 3..7 s, walk 4..8 s) so a crowd does not move in step.
#[derive(Clone, Debug, PartialEq)]
pub struct TestPath {
    pub idle: f32,
    pub walk: f32,
    elapsed: f32,
    phase: u8,
    turns: u32,
}

impl TestPath {
    pub fn new(seed: u64) -> Self {
        let mut rng = Rng::new(derive(seed, &[ANIM_LABEL, 1]));
        Self { idle: 3.0 + 4.0 * rng.unit(), walk: 4.0 + 4.0 * rng.unit(), elapsed: 0.0, phase: 0, turns: 0 }
    }

    /// The intent for the next step, given the player's state.
    pub fn intent(&mut self, dt: f32, state: Locomotion) -> Intent {
        self.elapsed += dt;
        match self.phase {
            0 if self.elapsed >= self.idle && state == Locomotion::Idle => {
                self.phase = 1;
                self.elapsed = 0.0;
                Intent::Walk
            }
            0 => Intent::Idle,
            1 if self.elapsed >= self.walk => {
                self.phase = 2;
                self.elapsed = 0.0;
                Intent::Idle
            }
            1 => Intent::Walk,
            2 if state == Locomotion::Idle && self.elapsed >= 1.0 => {
                self.phase = 3;
                self.elapsed = 0.0;
                self.turns += 1;
                if self.turns % 2 == 1 { Intent::TurnRight } else { Intent::TurnLeft }
            }
            2 => Intent::Idle,
            _ => {
                if matches!(state, Locomotion::TurnLeft | Locomotion::TurnRight) {
                    Intent::Idle
                } else {
                    if state == Locomotion::Idle && self.elapsed > 0.2 {
                        self.phase = 0;
                        self.elapsed = 0.0;
                    }
                    Intent::Idle
                }
            }
        }
    }
}
