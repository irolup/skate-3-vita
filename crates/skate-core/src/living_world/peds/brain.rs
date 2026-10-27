//! A pedestrian's behaviour brain: the host the stock ped AI graph runs on (wants, timers, flags, outputs).
//! Details: docs/hails-additions/26g-living-world-ped-interactions.md#ped-brain

use super::super::Vec3;
use crate::graph::activation::ConditionHost;
use crate::graph::controller::{BehaviorId, Frame, Host, HookId};
use std::collections::BTreeMap;

/// Retail values the operations use (data-driven; per category later).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BrainSettings {
    /// Wander's speed suggestion, m/s (`8269F410`, `0x82060C50` 2.0).
    pub wander_speed: f32,
    /// How long a ped warns before it releases the want, s (`8269F7B0`, timer 36, `0x82063AB8` 3.5).
    pub warn_seconds: f32,
    /// KnowAboutWantTarget's value (`826A5A70`, `82E42868(brain, target, 30.0)`; meaning open).
    pub know_about_seconds: f32,
    /// How long a passed mood result keeps each raised want's target known (`sub_82E41B98` ->
    /// `82E42868(brain, target, 30.0)`, `0x820D4924`): the entry that counts finished reactions (`7FF7`) lives this long.
    pub mood_want_memory_seconds: f32,
    /// WatchWantTargetWithoutInterruptingLocomotion keeps walking while the target is within this
    /// angle of the facing, radians (`0x821DBCF4`, 55 deg).
    pub watch_cone: f32,
    /// StandAndWatchSkater's first watch point, metres ahead (`0x82257308`, 4.0).
    pub watch_ahead: f32,
    /// ChannelWarnWantTarget's speech value (`8269F7B0` sends 53, or 54 when the ped's `+2000`
    /// component answers non-zero; that query is open, so 53).
    pub warn_speech: i32,
    /// The chase manager's "chases allowed" switch (`G+6332` bit 0x10: on at init `826B41B0`,
    /// game modes turn it off / on through `826B5A98` / `826B5AC0`); CanNewChaseStart reads it.
    pub chases_enabled: bool,
    /// ChaseeCloseEnoughToSatisfyAggressiveChase (`826AC7B0`): 2 m (`0x82257308` 4.0 squared) [code].
    pub aggressive_capture_distance: f32,
    /// CannotReachChaseTarget's radius (`82E31D58`: flat distance to the chasee, 25.0 squared at `0x8209994C`) [code].
    pub unreachable_radius: f32,
    /// How often the mover checks that its destination is reachable, seconds: every 10 mover updates
    /// (`82E2CB08`, counter mover `+0xB0`), at the console's 30 fps cadence 10 / 30 s [code; cadence: ours].
    pub reach_check_seconds: f32,
    /// DrawTazer's draw time (`826A8258`, timer 37, `0x82099478` 0.133 s) and TazeWantTarget's
    /// delay before the hit (`826A8398`, timer 39, `0x820D06C0` 0.3 s); speeches 66 / 67.
    pub tazer_draw_seconds: f32,
    pub tazer_hit_seconds: f32,
    pub taze_speech: i32,
    pub end_taze_speech: i32,
    /// The tazer clips' channel blends (ped vfn248 `82E3A3F0` -> vfn240(name, 0.2, 0.2)) and the time left on the
    /// draw / holster clip at which the tazer counts as drawn / put away (`82E3ED50`, `0x82099280` 0.2 s) [code, b248].
    pub tazer_clip_blend: f32,
    pub tazer_clip_end: f32,
    /// ChannelGreetWantTarget: the greet time (timer 36, `0x82063AB8` 3.5 s), speeches 56 / 63
    /// (`8269FC30`).
    pub greet_seconds: f32,
    /// A conversation turn, s (ConversationSpeak timer 30, 3.0).
    pub conversation_turn_seconds: f32,
    /// A conversation's gather timer, s (`D+1136`, 30.0; `conversation.rs`).
    pub conversation_gather_seconds: f32,
    pub greet_speech: i32,
    pub return_greet_speech: i32,
    /// StartChase Update's and NewChasee Begin's speech values (`826A3780` 55, `826A37F0` 15).
    pub start_chase_speech: i32,
    pub new_chasee_speech: i32,
    /// Hand prop release values (`peds/hand_prop.rs`).
    pub hand_prop: super::hand_prop::HandPropSettings,
    /// ZombieFollow's distances and speeds (`826A9250`; retail hard-codes them).
    pub zombie_follow: ZombieFollowSettings,
    /// The reaction a failed takedown's trip stores (`826A5068`: Knockdown `830218C0[0]`, FromBack `830218B0[1]`; b326).
    pub trip_kind: super::skater_contact::ReactionKind,
    pub trip_direction: super::skater_contact::ReactionDirection,
    /// SkaterAvoidance's closest-approach test and Jump / Step limits (`super::avoid`).
    pub avoid: super::avoid::AvoidRules,
}

/// ZombieFollow (`826A91A8` / `826A9250` / `826A9518`) [code]: beyond `follow_distance` the goal is the player; inside
/// it, once the goal is reached, a random point `ring_min..ring_max` around the player (`82E17508`); the speed is
/// `sprint_speed` beyond `sprint_distance`, else `walk_speed`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ZombieFollowSettings {
    /// `0x82099250` 8.0 m.
    pub follow_distance: f32,
    /// `0x820BD16C` 15.0 m.
    pub sprint_distance: f32,
    /// The ring around the player: `0x8231A844` 1.0 and `0x82099250` 8.0 m.
    pub ring_min: f32,
    pub ring_max: f32,
    /// `0x82099250` 8.0 and `0x82063B08` 3.0 m/s.
    pub sprint_speed: f32,
    pub walk_speed: f32,
    /// "Goal reached" = the mover's vfunc +124 (`82E2BC70`): 3D distance within the mover's arrival radius `+0x0C`,
    /// which every writer found sets to `0x82060C50` 2.0 m (`82E35E80`, mover modes 2 / 3 / 4 / 6) [code].
    pub arrive_distance: f32,
}

impl Default for ZombieFollowSettings {
    fn default() -> Self {
        Self { follow_distance: 8.0, sprint_distance: 15.0, ring_min: 1.0, ring_max: 8.0, sprint_speed: 8.0, walk_speed: 3.0, arrive_distance: 2.0 }
    }
}

impl Default for BrainSettings {
    fn default() -> Self {
        Self { wander_speed: 2.0, warn_seconds: 3.5, know_about_seconds: 30.0, mood_want_memory_seconds: 30.0, watch_cone: 0.959_931, watch_ahead: 4.0, warn_speech: 53, tazer_draw_seconds: 0.133, tazer_hit_seconds: 0.3, taze_speech: 66, end_taze_speech: 67, tazer_clip_blend: 0.2, tazer_clip_end: 0.2, greet_seconds: 3.5, conversation_turn_seconds: 3.0, conversation_gather_seconds: 30.0, greet_speech: 56, return_greet_speech: 63, chases_enabled: true, aggressive_capture_distance: 2.0, unreachable_radius: 5.0, reach_check_seconds: 10.0 / 30.0, start_chase_speech: 55, new_chasee_speech: 15, hand_prop: Default::default(), zombie_follow: Default::default(), trip_kind: super::skater_contact::ReactionKind::Knockdown, trip_direction: super::skater_contact::ReactionDirection::FromBack, avoid: super::avoid::AvoidRules::RETAIL }
    }
}

/// The ped's nav modifiers (ChangeNavModifierSetting).
pub mod nav_modifier {
    pub const CHASE_GLUE: u8 = 0;
    pub const PEDESTRIAN: u8 = 1;
    pub const SKATER_AVOIDANCE: u8 = 2;
    pub const VEHICLE_AVOIDANCE: u8 = 3;
}

/// Timer keys (`82E40940` map keys).
pub mod timers {
    /// ChannelWarnWantTarget's warn time (`InterestTimer`).
    pub const WARN: i32 = 36;
    /// SetSitTimer's timer (`SitTimer`).
    pub const SIT: i32 = 24;
    /// The graph's timer names by index (`sub_82E42B08` compares in this order).
    pub const NAMES: [&str; 47] = [
        "NextWarnTimer",
        "InvestigateTimer",
        "ChaseSkaterOutViewTimer",
        "ChaseExhaustionTimer",
        "RestTimer",
        "OutOfViewTimer",
        "LookAtTimer",
        "AlertTimer",
        "TimedRangeRandom",
        "SpectatorBoredomTimer",
        "SpectatorCheerTimer",
        "AmbientBehaviourTimer",
        "CellPhoneTimer",
        "GatherTimer",
        "WanderWaitTimer",
        "PatrolZoneMaintainInterestTimer",
        "AttemptTakeDownTimeout",
        "TakeDownSuccessTimeout",
        "ReturnToPatrolZone",
        "ProRecTimer",
        "TargetConverse",
        "StartChase",
        "ForceWarnTimer",
        "WaitAfterCheer",
        "SitTimer",
        "PresenceCheckTimer",
        "TrespassCheck",
        "PatrolTimer",
        "IdleTimer",
        "TimeUntilICanBePrimaryChaserTimer",
        "ConversationSpeakingTimer",
        "HandPropActionTimer",
        "HandPropUsageTimer",
        "HandPropUsageTimer2",
        "ThrowHandPropTimer",
        "ThrowHandPropReactionTimer",
        "InterestTimer",
        "TazerIntoTime",
        "TazerWaitTime",
        "TazerPlayerDropTime",
        "TazerCycTime",
        "WanderRoadCheck",
        "TargetUnreachableTimer",
        "WanderTargetTime",
        "CollisionVolumeUnlock",
        "PluginTimeout",
        "ScatterTime",
    ];
    /// `sub_82E42B08`: a timer name's index; an unknown name gets 47.
    pub fn index(name: &str) -> i32 {
        NAMES.iter().position(|n| *n == name).map_or(47, |i| i as i32)
    }
    /// The brain holds at most this many running timers (`82E40940` drops a new one when full).
    pub const CAPACITY: usize = 14;
}

/// Motion intents posted to the ped's motion graph (`82E...` intent ids; names open).
pub mod motion {
    /// Wander (`8269F380`).
    pub const WANDER: u8 = 6;
    /// Flee (`826A3530`).
    pub const FLEE: u8 = 4;
    /// InterceptChasee (`826A3B10`; name open).
    pub const INTERCEPT: u8 = 3;
    /// RunFromHonker (`826A1330`).
    pub const RUN_FROM_HONKER: u8 = 5;
    /// ZombieFollow Begin (`826A91A8`: `ped.vfn192(0)`).
    pub const ZOMBIE_FOLLOW: u8 = 0;
}

/// One graph operation, parsed from its name and attributes.
#[derive(Clone, Debug, PartialEq)]
pub enum PedOp {
    // Conditions.
    /// `826ACDE8`: the want is set and flagged "needs addressing".
    HasWantToAddress { want: String },
    /// `8269E758` / `8269E790`: the collision component's "needs to begin" / "is colliding".
    NeedsToBeginColliding,
    IsColliding,
    /// The Colliding state's behaviour. Begin `8269DA00`: posts the collision mood event (`sub_8269D8A8`, the
    /// collision broadcast: `collision` for the hit ped, `nearbycollision` for bystanders) and plays the contact
    /// reaction; End `8269E610`: clears the component's needs-to-begin / colliding bytes (+8, +9, +10).
    PedestrianColliding,
    /// `826AD790`: brain `+3281` bit 0x80.
    ShouldScatter,
    /// `826ABF98`: a honker is set.
    IsBeingHonkedAt,
    /// `826A1330` Begin: motion intent 5; `826A1358` Update: the host runs the ped sideways of the honking
    /// car's line (`peds/honk.rs`). The `timeout` attribute is never read in retail.
    RunFromHonker,
    /// `826AD7B0`: the zombie game mode.
    IsZombieMode,
    /// `826A99C0`: the ped has a plugin component.
    HasPlugin,
    /// `826AD1A8`: the ped holds or has requested a hand prop (b25, b87).
    HasHandProp,
    /// `826AD270`: HasHandProp and the held record's key is `handprop` (the attribute's hash vs the record key,
    /// b87).
    IsHoldingSpecificHandProp { handprop: String },
    /// `8269D170`: HasHandProp and the held record's IsDisposable (record +60, b90).
    HasDisposableHandProp,
    /// `8269D268`: no hand prop, or the held record's CanSitWithHandProp (record +61, b90).
    CanSitWithHandProp,
    /// `826AD330`: HasHandProp and the held record's CanAttackThrowHandProp (record +62, b25 / b90).
    CanAttackThrowHandProp,
    /// `826AD558`: distance to the want's target compared with the threshold (squared by the
    /// factory `826D0038`).
    DistanceToWantTarget { want: String, greater: Option<f32>, less: Option<f32> },
    // Behaviours.
    /// `8269F380` Begin: clear the honker, motion intent 6; `8269F410` Update: speed 2.0.
    Wander,
    /// `826A2FB8` Begin: ped `+5936` bit 0x80, motion intent 6 (a second virtual call with 1,
    /// meaning open); no update. Off-road wander on the navmesh.
    NoRoadWander,
    /// `8269F6A8` Begin: suggest the velocity; `8269F770` End: clear it.
    SuggestVelocity { linear: f32 },
    /// `826A5A70` Begin: remember the want's target.
    KnowAboutWantTarget { want: String },
    /// `826A6010` Begin / `826A60C8` End: unset the want.
    UnsetWant { want: String },
    UnsetWantOnEnd { want: String },
    /// `8269F7B0` Begin: warn timer 3.5 s; `8269F980` Update: unset the want when it ran out and
    /// `deactivateWant` (op `+28`) is set.
    ChannelWarnWantTarget { want: String, deactivate: bool },
    /// `8269FAC0` Begin (ctor `8269F9F0`; `want`, `deactivateWant` default true, `returnGreet`
    /// default false): greet timer, post `greeted` into the target's mood store (not for a return
    /// greet); `8269FC30` Update: speech 56 / 63, unset the want when the timer ran out.
    ChannelGreetWantTarget { want: String, deactivate: bool, return_greet: bool },
    /// `8269FDF0` Update (`speed`, default 2.0): head for the want's target at `speed`.
    ApproachWantTarget { want: String, speed: f32 },
    /// `8269FD70` Begin / `8269FDC0` End: push / pop the want's target on the look-at stack.
    AlertToWantTarget { want: String },
    /// `826A3530` Begin: motion intent 4, remember the flee target; `826A3760` End: forget it.
    /// Update `826A35F8` not decoded (the body steers away from the target).
    Flee,
    /// Begin: save the speed suggestion, look at the want target; Update: face it (flat), speed
    /// 0.0 (`0x82165A10`); End: restore the speed, stop facing.
    StopAndFaceWantTarget { want: String },
    /// As StopAndFace, but while the target is inside the watch cone the ped keeps walking at its
    /// saved speed; once outside it latches into stop-and-face until End.
    WatchWantTarget { want: String },
    /// Update: face the skater (steering goal 2 m towards it, mode 2 [inferred: face]).
    TurnToFaceSkater,
    /// Begin: watch point 4 m ahead; Update: speed 0, face the watch point; when the direction to
    /// it and to the skater differ (dot < `maxAngle`, 0.4) it jumps to the skater's position +
    /// velocity x `predictTime` (2.0 s).
    StandAndWatchSkater { max_angle: f32, predict_time: f32 },
    /// `826A9E60`: dot(forward, flat direction to the skater) > `FOVAngle` (a cosine).
    /// `826A9E60`: the flat direction to the skater within `FOVAngle` (a half angle, radians: `dot >
    /// cos(fov)`, `sub_82473930`) and, when `Distance` is not 0, nearer than it (flat).
    IsFacingSkater { fov: f32, distance: f32 },
    /// `826A3508` Begin: store `speechvalue` on the ped (`ped+2468` through ped vfunc +204,
    /// `82E22798`); the audio side speaks when the value changes. `speechevent` is a label only.
    SendSpeech { value: i32 },
    /// `826A2810` Begin: start timer `timerName` with `length` s (factory `826C9E88`, default 0.0).
    SetSimpleTimer { timer: i32, length: f32 },
    /// `826ACFF0`: the timer is at or below 0 (a timer that is not running reads 0).
    SimpleTimerExpired { timer: i32 },
    /// `826AC470`: the chaser component has a chasee (`brain+3204`).
    IsChasing,
    /// `826AC668`: the chasee is farther than the chase record's escape distance.
    ChaseeEscaped,
    /// `826AAC08`: the chase manager allows new chases.
    CanNewChaseStart,
    /// `826A3780` Update: speech value 55 (StartChase picks ResumeChase / NewChasee in the graph).
    StartChase,
    /// `826A3B10` Begin: motion intent 3; `826A3BF0` Update: the game steers to the intercept
    /// point (`chase::intercept`) at the run speed.
    InterceptChasee,
    /// `826ACAC8`: the angrychase want's target can take one more chaser.
    CanChaseeAddNewChaser,
    /// `826ACBD8`: this ped is entry 0 of its chasee's group.
    IsPrimaryChaser,
    /// `826ACC80`: only the player's actor has a protector interface; what it holds is open, so
    /// false.
    ChaseeHasProtector,
    /// `826ACB70` [code, b106]: the chasee's chase group exists and its vfn +88 (wiping out) holds.
    ChaseeIsWipedOut,
    /// `826AD640` [code, b108]: brain `+3280` bit 0x08, written only by the intercept goal provider's result
    /// (`82E31D58`): set when its move ends or fails within `unreachable_radius` (5 m flat, `0x8209994C` 25.0) of the
    /// chasee, cleared beyond.
    CannotReachChaseTarget,
    /// `826A8810` Begin [code, b108]: timer `TargetUnreachableTimer` = the chase record's `unreachable_timer` (5 s for
    /// peds; 0.0 without a record).
    SetTargetUnreachableTimer,
    /// `826AD660`: a per-ped byte whose writer is open (b20 / b108); false until found.
    CanTazeUnreachableTargets,
    /// `826AC7B0` [code, b106]: squared 3D distance chaser to chasee under `aggressive_capture_distance`^2
    /// (`0x82257308` 4.0, so 2 m, strict).
    ChaseeCloseEnoughToSatisfyAggressiveChase,
    /// `826ACD30`: false in retail (the group owner is never null).
    ChasersAreScared,
    /// `826AAC20`: the group's end reason, else this chaser's own, is set.
    IsChaserEndingChase,
    /// `826AAC80`: the chase component's end reason (vfunc +84) equals `reason` (op+28). Picks EndChase's branch
    /// (aggressivecapture / lostinterest / default).
    IsEndChaseReason { reason: i32 },
    /// `826A8A60`: the want's target handle (`brain+412+12*want`) becomes the entity whose contacts the ped ignores
    /// (`ped+5680`, read by the contact classifier `sub_82E38400`); EndChase's aggressivecapture branch.
    IgnoreCollisionsWithWantTarget { want: String },
    /// `826AD690`: this chaser's takedowns reached the record's give-up count.
    ChaserShouldGiveUpDueToTakedowns,
    /// `826A37F0` Begin: speech 15, takedowns 0, join the angrychase target's group.
    NewChasee,
    /// `826A5980` Begin (`timeout`): hand the primary role to entry 1 and hold timer 29.
    GiveUpBeingPrimaryChaser { timeout: f32 },
    /// `826A52A0` Begin: this chaser's end reason.
    ChaserEndChase { reason: i32 },
    /// `826A5318` Begin: the group's end reason (every chaser ends).
    ChaserGroupEndChase { reason: i32 },
    /// `826A53F0` Begin: leave the group, forget the chasee, stop steering.
    EndChase,
    /// `826A6210` Begin: copy the original want's slot to the related want and flag it (`82E42A58`
    /// with 1); with `deactivateOriginalWant` (default true) clear the original's flag.
    ActivateRelatedWant { original: String, related: String, deactivate_original: bool },
    /// `826A75D8` Begin: save the ped's nav modifier and set it; `826A7658` End: restore it.
    /// Modifiers (factory `826A74C8`): 0 ChaseGlue, 1 Pedestrian, 2 SkaterAvoidance, 3 VehicleAvoidance.
    ChangeNavModifierSetting { modifier: u8, set_to: bool },
    /// 826A77A8 Begin (factory 826CB3B8): send the chased player the chase state (message 0xEFFA9715): docs/hails-additions/26g-living-world-ped-interactions.md#send-chase-state
    SendChaseStateMessage { state: u8 },
    /// `826A4838` Update: the takedown target (`brain+2180`) is the chasee.
    ChaserEvaluateWhoToTakeDown,
    /// `826A4948` Update: pick the takedown that fits now (`82E3C000`, `takedown::choose`).
    TakeDownTargetablePredictions,
    /// `826AC988`: a takedown target is set.
    HasTakeDownTargetable,
    /// `826ACA08`: the target exists and a takedown was picked (the target's own veto byte
    /// `T.vfn24` is open: never vetoes).
    CanAttemptTakeDownTargetable,
    /// `HasMonitoredIntent`: only `ActiveTakedown` is ported (the attempt's takedown intent).
    HasMonitoredIntent { intent: String },
    /// `826A49D8` Begin: start the takedown intent, clear the result; `826A4BE8` Update: a
    /// contact with the target while it plays succeeds (result 2); `826A4DB8` End.
    AttemptTakeDownTargetable,
    /// `826ABBC0` / `826ABC18`: the result is 2 / 3 (`brain+3224`).
    TakeDownAttemptSuccessful,
    TakeDownAttemptFailed,
    /// `826A4E50` Begin: knock the target down (`T.vfn20(chaser position)`), speech 65 for the
    /// player, else 19.
    TakeDownTargetableSuccess,
    /// `826A5068` Begin: the failed attempt. The ped trips (stores Knockdown / FromBack) when the target refused, or when
    /// the chosen takedown entry may trip and a 1..=100 roll is at most `trip_chance` x 100 (`tripChance`, behaviour
    /// +0x1C, default `8208EA6C` 0.35); otherwise a reaction stored during the attempt stays stored (b326).
    TakeDownTargetableFailure { trip_chance: f32 },
    /// `826A5230` End: forget the takedown target (inferred from the name).
    TakedownTargetableClearOnEnd,
    /// `826AC898`: exhausted, or resting and not rested yet (chaser vfn148 / 164 / 152).
    NeedToRest,
    /// `8269F4D8` Begin: exhaustion 0, resting, timer 4 = the record's rest time; `8269F608` End:
    /// not resting.
    RestFromChase,
    /// `826A58B8` Begin: exhaustion 0 (chaser vfn172).
    ClearExhaustionTimer,
    /// `826A5918` Begin: timer 1 = the record's investigate time; `826A5968` End: timer 1 = 0;
    /// `826AACE0`: timer 1 at or below 0.
    StartInvestigateTimer,
    EndInvestigateTimer,
    InvestigateTimeExceeded,
    /// `826AC4B0`: the chasee's perception entry is visible or just told (`flags & 0xC0`).
    CanSeeChasee,
    /// `826A5868` Update: know the chasee for 30 s (`82E42868`).
    KnowAboutChasee,
    /// `826A56C8` Begin (`timeout`): block mood events about the chasee for at least that long.
    SuppressMoodAboutChasee { timeout: f32 },
    /// `826A6028` Begin (`want`, `timeout`): an active want is unset and its target suppressed.
    UnsetWantAndSuppressMoodForATime { want: String, timeout: f32 },
    /// `826A5790` Begin: forget every mood record and the perception entry about the chasee.
    MoodResetAboutChasee,
    /// `826A4500` Update: walk to the chasee's last known position; `826A47C8` End: stop.
    LostChasee,
    /// `826A3A28` Update: the alternate steering target follows the chasee.
    SetAltTargetToChaseePosition,
    /// `826A4038` Begin (shared with Block): steering like the intercept; `826A43B0` Update: run
    /// to the formation point at the run speed, exhaustion += dt.
    PursueChasee,
    /// `826A4038` Begin; `826A40E8` Update: at the block point (within `attargetdist`) stand facing
    /// the chasee and recover exhaustion, else walk there at `slowspeed`; `826A4338` End.
    BlockChasee { at_target: f32, slow_speed: f32 },
    /// `826A3FD8` Update: the block rule (`chase::should_block`) for this tick.
    UpdateBlockPrediction,
    /// `826AD130`: this tick's block flag.
    ChaserShouldBlock,
    /// `826AA1B8` (factory `826CFBC0` / ctor `826AA0C0`): the want's target is within `distance`
    /// (flat; 0 = any) and inside the `angle` cone (half of it each side, degrees).
    IsFacingWantTarget { want: String, angle: f32, distance: f32 },
    /// `826A8258` Begin: draw the tazer (timer 37), forget the last hit and line of sight;
    /// `826A8318` Update: the tazer is in the hand when timer 37 ran out (the `tazr` prop is not
    /// attached yet).
    DrawTazer { want: String },
    /// `826AD2F8`: drawn (draw requested and the draw finished).
    HasDrawnTazer,
    /// `826AD620`: the line of sight to the taze target (host-computed).
    HasLineOfSightToTazeTarget,
    /// `826A8398` Begin: speech 66, timer 39; `826A8420` Update: once timer 39 ran out, hit the
    /// want's target once (the target's `vfn20`: the same knock-down as a takedown).
    TazeWantTarget { want: String },
    /// `826A8610` Begin / `826A86D8` End: the ped holds a live tazer (`ped+2492`).
    RegisterTazer { want: String },
    UnregisterTazerOnEnd,
    /// `826A85A8` Begin: put the tazer away, speech 67.
    EndTaze { want: String },
    /// `8269F248` Update (main graph): run the plugin's own graph (the host runs it while
    /// `in_plugin`); `ExitPlugin` (`8269F1B8`) leaves the plugin.
    Plugin,
    ExitPlugin,
    /// `826A6CD8` Begin: spawn a conversation 3 m ahead with the startconversation want's target
    /// (none within 50 m of another; host-applied).
    SpawnConversationArea,
    /// `826A6408` / `826A6428`: in a conversation (`brain+3278` bit 0x04); `826ACE20` reads it.
    PedestrianInConversation,
    PedestrianIsInConversation,
    /// Waypoint ops (`moveontowaypoint.xml`, 826A62E0): lock, walk to, face, release a plugin waypoint.
    LockClosestWaypoint,
    /// `LockFirstWaypoint` (`826A0580`): with no waypoint locked, lock the plugin's first waypoint if it is free.
    LockFirstWaypoint,
    HasWaypointLocked,
    DistanceFromWaypointXZ { less_equal: Option<f32>, greater: Option<f32>, y_tolerance: f32 },
    TargetWaypoint { speed: f32, slide_distance: f32, slide_speed: f32 },
    LockToCurrentPosition,
    /// `DisableNavigation` (`826A3170` Update clears ped `+5936` bit 0x80, the navmesh navigation `NoRoadWander`
    /// sets; `826A3188` End sets it): no navigation while active (the marquee graph's AI state).
    DisableNavigation,
    TurnToFaceWaypointOrientation,
    IsFacingWaypointOrientation { fov: f32 },
    UnlockWaypoint,
    /// Monitored intents (`intentName`, `numberOfStages`): created, stepped, present.
    CreateSimpleMonitoredIntent { intent: String, stages: u8, names: Vec<String> },
    /// `DisableHeavyCollision` (`826A30D0` / `826A30E8`): `brain+3277` bit 0x20 while active.
    DisableHeavyCollision,
    /// `DisableCollisionSliding` (`826A3140` / `826A3158`): ped `+5936` bit 0x40 (collision sliding) off while active.
    DisableCollisionSliding,
    /// `DisableAllMoods` (`826A5A20` / `826A5A50`): `brain+3278` bit 0x08 on while active; End restores the value
    /// saved at Begin.
    DisableAllMoods,
    /// `OverridePluginCollision` / `OverridePluginAvoid` (`8269AC98` / `8269ACE8`, `8269AD38` / `8269AD88`):
    /// `brain+3200` / `+3201` = 1 while active, 0 at End; read by `IsPluginCollisionOverride` (`8269ADD8`) and
    /// `IsPluginAvoidOverride` (`8269AE38`).
    OverridePluginCollision,
    OverridePluginAvoid,
    IsPluginCollisionOverride,
    IsPluginAvoidOverride,
    /// `IsOnRoad` (graph condition, vtable slot 12 = the `li r3,0` stub `8274CA90`): always false.
    IsOnRoad,
    /// `SimpleRandom` (`826AB540`, factory `826CCEF0`: `percentage` x 0.01): true when the chance >= a uniform roll in
    /// [0, 1); rolls on every evaluation.
    SimpleRandom { chance: f32 },
    /// `IsAtWaypoint` (`826AB880`, factory `826CDEE8` default `0x82063B08` 3.0): the locked waypoint within `radius`
    /// horizontally; false without one.
    IsAtWaypoint { radius: f32 },
    /// `InSkaterRadius` (`826AAAF8`): the skater within `radius` (3D).
    InSkaterRadius { radius: f32 },
    /// `OwnPluginObject` (`826A7498` / `826A74B0`): `brain+3277` bit 0x01 while active.
    OwnPluginObject,
    /// `IgnoreStandingCollisions` (`826A3108` / `826A3120`): `brain+3277` bit 0x10 while active.
    IgnoreStandingCollisions,
    /// `DisableCollisionsWithBehaviourSource` (`826A7378` / `826A7410`): `brain+3277` bit 0x01 and the plugin prop as an
    /// object the ped's collision ignores (`82E3DB68`, `ped+5916`) while active. The host exempts that prop from the ped's
    /// obstacle step (`Obstacles::resolve_step_ignoring`; body id = the plugin prop id's low 32 bits).
    DisableCollisionsWithBehaviourSource,
    /// `SetExplicitTurnDirectionToWaypointOrientation` (`826A2988` / `826A29F0`): while active the locomotion turns to
    /// the locked waypoint's orientation (`ped+5888` +2080, flag +2100 bit 0x20); nothing without a waypoint.
    SetExplicitTurnDirectionToWaypointOrientation,
    /// `SetSitTimer` Begin `826A2898`: SitTimer (24) = min + rand x 2^-32 x (max - min), the ped type's sit times.
    SetSitTimer,
    /// `GoingToStandBackUp` `826AD1F0` -> `8269A588`: d100 roll (`rand() % 100 + 1`) at most the ped type's
    /// stand-up chance x 100 (`0x820ED57C` 100.0); rolls on every evaluation.
    GoingToStandBackUp,
    IncrementMonitoredPacketStage { intent: String },
    /// Conversation ops: in position (vf44), am I the speaker (vf12), speak for the turn time
    /// then pass the turn (vf48), face the speaker, complete (vf36).
    ConversationSignalInPosition,
    ConversationThisParticipantIsSpeaker,
    ConversationSpeak,
    ConversationListenToSpeaker,
    ConversationIsComplete,
    /// Taunt victim (826A70E0 Begin / 826A72F0 End) and the mood gate flags (brain+3278): docs/hails-additions/26g-living-world-ped-interactions.md#takedown-taunt-victim
    TakedownTauntVictim,
    /// `826A7998` Begin: the light throw (`82E3E648` attack 0) at the plugin object's hotpoint 0 (b87 §3, b90 §1);
    /// the release follows in the per-ped step ([`PedBrain::update_hand_prop_release`]).
    ThrowHandPropAtTrashBin,
    /// `826A7A58` Begin: the aimed attack throw at the want's target (`82E3E960` -> `82E3E648` attack 1, b25 §5,
    /// b92 / b93). `826A7AA8` Update: once the prop is released and timer 35 has run out, the want is unset.
    ThrowHandPropAtWantTarget { want: String },
    /// `826A8730` Begin: `82E3EBE0(ped, 0, zero)`, the held prop is released in place (b25 §6).
    DropHandProp,
    /// ZombieFollow (`826A91A8` / `826A9250` / `826A9518`, b95): follow the player, mill around inside the ring.
    ZombieFollow,
    /// OverrideAnimData (factory `826CA578`): the ped plays another entity type's animation set while the state runs
    /// (`overrideEntityName`; stock: `zombie`). Clearing it on End is [inferred] (no vtable found).
    OverrideAnimData { entity: String },
    /// `826A7900` Begin / `826A7918` End: `brain+3279` bit 0x10 for the behaviour's life (b25 §3).
    DisallowHandPropActions,
    SetWaitingToReactFlagOnBegin,
    ClearWaitingToReactFlagOnBegin,
    SetIsReactingToMoodEventFlagOnBegin,
    UnsetIsReactingToMoodEventFlagOnEnd,
    /// Marker behaviours with no effect of their own (`AllowPedestrianJumping`: all no-op slots).
    Marker { name: String },
    /// Not ported yet.
    Pending { name: String },
}

impl PedOp {
    /// Parse an operation (text / float attribute readers from the graph).
    pub fn parse(name: &str, text: &dyn Fn(&str) -> Option<String>, float: &dyn Fn(&str) -> Option<f32>) -> PedOp {
        let want = || text("want").unwrap_or_default().to_ascii_lowercase();
        let flag = |k: &str, default: bool| text(k).map_or(default, |v| matches!(v.to_ascii_lowercase().as_str(), "true" | "1"));
        let timer = || timers::index(&text("timerName").unwrap_or_else(|| "TimedRangeRandom".into()));
        match name {
            "HasSpecificWantThatNeedsToBeAddressed" => PedOp::HasWantToAddress { want: want() },
            "NeedsToBeginColliding" => PedOp::NeedsToBeginColliding,
            "IsPedestrianColliding" => PedOp::IsColliding,
            "PedestrianColliding" => PedOp::PedestrianColliding,
            "ShouldScatter" => PedOp::ShouldScatter,
            "IsBeingHonkedAt" => PedOp::IsBeingHonkedAt,
            "RunFromHonker" => PedOp::RunFromHonker,
            "IsZombieMode" => PedOp::IsZombieMode,
            "HasPlugin" => PedOp::HasPlugin,
            "HasHandProp" => PedOp::HasHandProp,
            "IsHoldingSpecificHandProp" => PedOp::IsHoldingSpecificHandProp { handprop: text("handprop").unwrap_or_default().to_ascii_lowercase() },
            "DistanceToWantTarget" => PedOp::DistanceToWantTarget { want: want(), greater: float("greater"), less: float("less") },
            "Wander" => PedOp::Wander,
            "NoRoadWander" => PedOp::NoRoadWander,
            "SuggestVelocity" => PedOp::SuggestVelocity { linear: float("linear").unwrap_or(0.0) },
            "KnowAboutWantTarget" => PedOp::KnowAboutWantTarget { want: want() },
            "UnsetWant" => PedOp::UnsetWant { want: want() },
            "UnsetWantOnEnd" => PedOp::UnsetWantOnEnd { want: want() },
            "ChannelWarnWantTarget" => PedOp::ChannelWarnWantTarget { want: want(), deactivate: flag("deactivateWant", true) },
            // The stock graph also writes `unsetWant`, which the constructor never reads.
            "ChannelGreetWantTarget" => PedOp::ChannelGreetWantTarget { want: want(), deactivate: flag("deactivateWant", true), return_greet: flag("returnGreet", false) },
            "ApproachWantTarget" => PedOp::ApproachWantTarget { want: want(), speed: float("speed").unwrap_or(2.0) },
            "AlertToWantTarget" => PedOp::AlertToWantTarget { want: want() },
            "Flee" => PedOp::Flee,
            "StopAndFaceWantTarget" => PedOp::StopAndFaceWantTarget { want: want() },
            "WatchWantTargetWithoutInterruptingLocomotion" => PedOp::WatchWantTarget { want: want() },
            "TurnToFaceSkater" => PedOp::TurnToFaceSkater,
            "StandAndWatchSkater" => PedOp::StandAndWatchSkater { max_angle: float("maxAngle").unwrap_or(0.4), predict_time: float("predictTime").unwrap_or(2.0) },
            "IsFacingSkater" => PedOp::IsFacingSkater { fov: float("FOVAngle").unwrap_or(0.8), distance: float("Distance").unwrap_or(0.0) },
            // The factory reads the float and truncates it (`fctiwz`), default 0.0.
            // Both factories read `timerName` with the default "TimedRangeRandom" (`0x82307E4C`).
            "SetSimpleTimer" => PedOp::SetSimpleTimer { timer: timer(), length: float("length").unwrap_or(0.0) },
            "SimpleTimerExpired" => PedOp::SimpleTimerExpired { timer: timer() },
            "IsChasing" => PedOp::IsChasing,
            "ChaseeEscaped" => PedOp::ChaseeEscaped,
            "CanNewChaseStart" => PedOp::CanNewChaseStart,
            "StartChase" => PedOp::StartChase,
            "InterceptChasee" => PedOp::InterceptChasee,
            "CanChaseeAddNewChaser" => PedOp::CanChaseeAddNewChaser,
            "IsPrimaryChaser" => PedOp::IsPrimaryChaser,
            "ChaseeHasProtector" => PedOp::ChaseeHasProtector,
            "ChaseeIsWipedOut" => PedOp::ChaseeIsWipedOut,
            "CannotReachChaseTarget" => PedOp::CannotReachChaseTarget,
            "SetTargetUnreachableTimer" => PedOp::SetTargetUnreachableTimer,
            "CanTazeUnreachableTargets" => PedOp::CanTazeUnreachableTargets,
            "ChaseeCloseEnoughToSatisfyAggressiveChase" => PedOp::ChaseeCloseEnoughToSatisfyAggressiveChase,
            "ChasersAreScared" => PedOp::ChasersAreScared,
            "IsChaserEndingChase" => PedOp::IsChaserEndingChase,
            "IsEndChaseReason" => PedOp::IsEndChaseReason { reason: super::chase::reason::parse(&text("reason").unwrap_or_default().to_ascii_lowercase()) },
            "IgnoreCollisionsWithWantTarget" => PedOp::IgnoreCollisionsWithWantTarget { want: want() },
            "ChaserShouldGiveUpDueToTakedowns" => PedOp::ChaserShouldGiveUpDueToTakedowns,
            "NewChasee" => PedOp::NewChasee,
            "GiveUpBeingPrimaryChaser" => PedOp::GiveUpBeingPrimaryChaser { timeout: float("timeout").unwrap_or(0.0) },
            "ChaserEndChase" => PedOp::ChaserEndChase { reason: super::chase::reason::parse(&text("reason").unwrap_or_default().to_ascii_lowercase()) },
            "ChaserGroupEndChase" => PedOp::ChaserGroupEndChase { reason: super::chase::reason::parse(&text("reason").unwrap_or_default().to_ascii_lowercase()) },
            "EndChase" => PedOp::EndChase,
            "ActivateRelatedWant" => PedOp::ActivateRelatedWant {
                original: text("originalWant").unwrap_or_default().to_ascii_lowercase(),
                related: text("relatedWant").unwrap_or_default().to_ascii_lowercase(),
                deactivate_original: text("deactivateOriginalWant").map_or(true, |v| !matches!(v.to_ascii_lowercase().as_str(), "false" | "0")),
            },
            "ChangeNavModifierSetting" => PedOp::ChangeNavModifierSetting {
                modifier: match text("modifier").unwrap_or_default().as_str() {
                    "Pedestrian" => nav_modifier::PEDESTRIAN,
                    "SkaterAvoidance" => nav_modifier::SKATER_AVOIDANCE,
                    "VehicleAvoidance" => nav_modifier::VEHICLE_AVOIDANCE,
                    _ => nav_modifier::CHASE_GLUE,
                },
                set_to: text("set_to").is_some_and(|v| matches!(v.to_ascii_lowercase().as_str(), "true" | "1")),
            },
            "SendChaseStateMessage" => PedOp::SendChaseStateMessage {
                state: match text("stateName").unwrap_or_default().as_str() {
                    "warn" => 0,
                    "chase" => 1,
                    "tired" => 2,
                    "giveup" => 3,
                    _ => 4,
                },
            },
            "ChaserEvaluateWhoToTakeDown" => PedOp::ChaserEvaluateWhoToTakeDown,
            "TakeDownTargetablePredictions" => PedOp::TakeDownTargetablePredictions,
            "HasTakeDownTargetable" => PedOp::HasTakeDownTargetable,
            "CanAttemptTakeDownTargetable" => PedOp::CanAttemptTakeDownTargetable,
            "HasMonitoredIntent" => PedOp::HasMonitoredIntent { intent: text("intentName").unwrap_or_default() },
            "AttemptTakeDownTargetable" => PedOp::AttemptTakeDownTargetable,
            "TakeDownAttemptSuccessful" => PedOp::TakeDownAttemptSuccessful,
            "TakeDownAttemptFailed" => PedOp::TakeDownAttemptFailed,
            "TakeDownTargetableSuccess" => PedOp::TakeDownTargetableSuccess,
            "TakeDownTargetableFailure" => PedOp::TakeDownTargetableFailure { trip_chance: float("tripchance").unwrap_or(0.35) },
            "TakedownTargetableClearOnEnd" => PedOp::TakedownTargetableClearOnEnd,
            "NeedToRest" => PedOp::NeedToRest,
            "RestFromChase" => PedOp::RestFromChase,
            "ClearExhaustionTimer" => PedOp::ClearExhaustionTimer,
            "StartInvestigateTimer" => PedOp::StartInvestigateTimer,
            "EndInvestigateTimer" => PedOp::EndInvestigateTimer,
            "InvestigateTimeExceeded" => PedOp::InvestigateTimeExceeded,
            "CanSeeChasee" => PedOp::CanSeeChasee,
            "KnowAboutChasee" => PedOp::KnowAboutChasee,
            "SuppressMoodAboutChasee" => PedOp::SuppressMoodAboutChasee { timeout: float("timeout").unwrap_or(0.0) },
            "UnsetWantAndSuppressMoodForATime" => PedOp::UnsetWantAndSuppressMoodForATime { want: want(), timeout: float("timeout").unwrap_or(0.0) },
            "MoodResetAboutChasee" => PedOp::MoodResetAboutChasee,
            "LostChasee" => PedOp::LostChasee,
            "SetAltTargetToChaseePosition" => PedOp::SetAltTargetToChaseePosition,
            "PursueChasee" => PedOp::PursueChasee,
            "BlockChasee" => PedOp::BlockChasee { at_target: float("attargetdist").unwrap_or(0.0), slow_speed: float("slowspeed").unwrap_or(0.0) },
            "UpdateBlockPrediction" => PedOp::UpdateBlockPrediction,
            "ChaserShouldBlock" => PedOp::ChaserShouldBlock,
            "IsFacingWantTarget" => PedOp::IsFacingWantTarget { want: want(), angle: float("angle").unwrap_or(0.0), distance: float("distance").unwrap_or(0.0) },
            "DrawTazer" => PedOp::DrawTazer { want: want() },
            "HasDrawnTazer" => PedOp::HasDrawnTazer,
            "HasLineOfSightToTazeTarget" => PedOp::HasLineOfSightToTazeTarget,
            "TazeWantTarget" => PedOp::TazeWantTarget { want: want() },
            "RegisterTazer" => PedOp::RegisterTazer { want: want() },
            "UnregisterTazerOnEnd" => PedOp::UnregisterTazerOnEnd,
            "EndTaze" => PedOp::EndTaze { want: want() },
            "TakedownTauntVictim" => PedOp::TakedownTauntVictim,
            "SetWaitingToReactFlagOnBegin" => PedOp::SetWaitingToReactFlagOnBegin,
            "ClearWaitingToReactFlagOnBegin" => PedOp::ClearWaitingToReactFlagOnBegin,
            "SetIsReactingToMoodEventFlagOnBegin" => PedOp::SetIsReactingToMoodEventFlagOnBegin,
            "UnsetIsReactingToMoodEventFlagOnEnd" => PedOp::UnsetIsReactingToMoodEventFlagOnEnd,
            "SendSpeechEvent" => PedOp::SendSpeech { value: float("speechvalue").unwrap_or(0.0) as i32 },
            "Plugin" => PedOp::Plugin,
            "ExitPlugin" => PedOp::ExitPlugin,
            "SpawnConversationArea" => PedOp::SpawnConversationArea,
            "PedestrianInConversation" => PedOp::PedestrianInConversation,
            "PedestrianIsInConversation" => PedOp::PedestrianIsInConversation,
            "LockClosestWaypoint" => PedOp::LockClosestWaypoint,
            "LockFirstWaypoint" => PedOp::LockFirstWaypoint,
            "HasWaypointLocked" => PedOp::HasWaypointLocked,
            "DistanceFromWaypointXZ" => PedOp::DistanceFromWaypointXZ { less_equal: float("lessEqual"), greater: float("greater"), y_tolerance: float("yTolerance").unwrap_or(f32::MAX) },
            "TargetWaypoint" => PedOp::TargetWaypoint { speed: float("speed").unwrap_or(1.0), slide_distance: float("slideDistance").unwrap_or(0.0), slide_speed: float("slideSpeed").unwrap_or(0.0) },
            "LockToCurrentPosition" => PedOp::LockToCurrentPosition,
            "DisableNavigation" => PedOp::DisableNavigation,
            "TurnToFaceWaypointOrientation" => PedOp::TurnToFaceWaypointOrientation,
            "IsFacingWaypointOrientation" => PedOp::IsFacingWaypointOrientation { fov: float("FOVAngle").unwrap_or(0.3) },
            "UnlockWaypoint" => PedOp::UnlockWaypoint,
            "CreateSimpleMonitoredIntent" => {
                let intent = text("intentName").unwrap_or_default();
                let stages = float("numberOfStages").unwrap_or(1.0).max(1.0) as u8;
                // Stage 1 is the packet name, stage n its `stage<n>Name` (b84, `826A2600`).
                let names = (1..=stages).map(|n| if n == 1 { intent.clone() } else { text(&format!("stage{n}Name")).unwrap_or_default() }).collect();
                PedOp::CreateSimpleMonitoredIntent { intent, stages, names }
            }
            "IncrementMonitoredPacketStage" => PedOp::IncrementMonitoredPacketStage { intent: text("intentName").unwrap_or_default() },
            "SetSitTimer" => PedOp::SetSitTimer,
            "OwnPluginObject" => PedOp::OwnPluginObject,
            "IsOnRoad" => PedOp::IsOnRoad,
            "DisableHeavyCollision" => PedOp::DisableHeavyCollision,
            "DisableCollisionSliding" => PedOp::DisableCollisionSliding,
            "DisableAllMoods" => PedOp::DisableAllMoods,
            "OverridePluginCollision" => PedOp::OverridePluginCollision,
            "OverridePluginAvoid" => PedOp::OverridePluginAvoid,
            "IsPluginCollisionOverride" => PedOp::IsPluginCollisionOverride,
            "IsPluginAvoidOverride" => PedOp::IsPluginAvoidOverride,
            "SimpleRandom" => PedOp::SimpleRandom { chance: float("percentage").unwrap_or(0.0) * 0.01 },
            "IsAtWaypoint" => PedOp::IsAtWaypoint { radius: float("radius").unwrap_or(3.0) },
            "InSkaterRadius" => PedOp::InSkaterRadius { radius: float("radius").unwrap_or(0.0) },
            "IgnoreStandingCollisions" => PedOp::IgnoreStandingCollisions,
            "DisableCollisionsWithBehaviourSource" => PedOp::DisableCollisionsWithBehaviourSource,
            "SetExplicitTurnDirectionToWaypointOrientation" => PedOp::SetExplicitTurnDirectionToWaypointOrientation,
            "GoingToStandBackUp" => PedOp::GoingToStandBackUp,
            "ConversationSignalInPosition" => PedOp::ConversationSignalInPosition,
            "ConversationThisParticipantIsSpeaker" => PedOp::ConversationThisParticipantIsSpeaker,
            "ConversationSpeak" => PedOp::ConversationSpeak,
            "ConversationListenToSpeaker" => PedOp::ConversationListenToSpeaker,
            "ConversationIsComplete" => PedOp::ConversationIsComplete,
            "ThrowHandPropAtTrashBin" => PedOp::ThrowHandPropAtTrashBin,
            "ThrowHandPropAtWantTarget" => PedOp::ThrowHandPropAtWantTarget { want: want() },
            "DropHandProp" => PedOp::DropHandProp,
            "ZombieFollow" => PedOp::ZombieFollow,
            "OverrideAnimData" => PedOp::OverrideAnimData { entity: text("overrideEntityName").unwrap_or_default().to_ascii_lowercase() },
            "DisallowHandPropActions" => PedOp::DisallowHandPropActions,
            "HasDisposableHandProp" => PedOp::HasDisposableHandProp,
            "CanSitWithHandProp" => PedOp::CanSitWithHandProp,
            "CanAttackThrowHandProp" => PedOp::CanAttackThrowHandProp,
            "KnowAboutChasers" | "AllowPedestrianJumping" | "IgnoreTakedownTargetNavRigVolume" => PedOp::Marker { name: name.to_string() },
            _ => PedOp::Pending { name: name.to_string() },
        }
    }

    pub fn is_pending(&self) -> bool {
        matches!(self, PedOp::Pending { .. })
    }
}

/// One want slot (`brain+412+want*12`: target handle, flags bit 0x80 "needs addressing", bit 0x40 "forget the
/// target on unset").
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Want {
    pub target: u64,
    pub needs_addressing: bool,
    /// Flag 0x40, from the mood result's `CE65` (set on the chase, tazeperp and runfromchaser results): unsetting the
    /// want erases the mood records and the perception entry about the target (`sub_82E42A58` -> `sub_82E40FF0`,
    /// research b28 / b18). Without it a chase's collision record outlived the chase and restarted it at once.
    pub forget_on_unset: bool,
}

/// The ped type's sit values (`ped+5696` attribute collection; `livingworld_entities`): sit time min / max (s,
/// `1190326371F1A684` 30.0 / `69F67C678B2673C9` 60.0) and the stand-up chance (`B040D387ABA6E24D` 0.5).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SitValues {
    pub min_seconds: f32,
    pub max_seconds: f32,
    pub stand_up_chance: f32,
}

impl Default for SitValues {
    fn default() -> Self {
        Self { min_seconds: 30.0, max_seconds: 60.0, stand_up_chance: 0.5 }
    }
}

/// A monitored packet (`brain+2188` map, lookup `82E40F10`; b84): one intent per stage, the current stage (`+452`,
/// from 1), active (`+456`: `HasMonitoredIntent` reads it; the motion side clears it when the packet's last motion
/// step completes).
#[derive(Clone, Debug, PartialEq)]
pub struct Packet {
    pub stages: Vec<String>,
    pub stage: u8,
    pub active: bool,
}

impl Packet {
    pub fn new(stages: Vec<String>) -> Self {
        Self { stages, stage: 1, active: true }
    }

    /// The intent of the current stage (`None` past the last one).
    pub fn current(&self) -> Option<&str> {
        self.stages.get(usize::from(self.stage).checked_sub(1)?).map(String::as_str)
    }
}

/// A ped's hand prop [code, b87].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HandProp {
    pub key: Option<String>,
    pub requested: bool,
    pub holding: bool,
    /// `brain+3278` bit 0x01: a throw started or the prop was released and is still linked to the ped (cleared by
    /// the unlink `82E3FAE0`).
    pub linked: bool,
    /// The started throw (`3279` bit 0x40 aimed, target `+3152`, speed `+3252`).
    pub throw: Option<super::hand_prop::HandPropThrow>,
    /// The held record's bools (`livingworld_handprops`, host-set with the request): IsDisposable (+60),
    /// CanSitWithHandProp (+61), CanAttackThrowHandProp (+62) [b90].
    pub disposable: bool,
    pub can_sit: bool,
    pub can_attack_throw: bool,
    /// The idle actions (`82E3F818` / `82E3F1D8` / `82E3F470`): the usage list (host-set at the attach), the picked
    /// action (`+0xCBC`), has actions (`+0xCCF` 0x20), running (0x08), the first and second stage done (0x04 / 0x02),
    /// and a litter drop waiting for timer 34 (`+0xCCE` 0x01).
    pub actions: Vec<super::hand_prop::HandPropAction>,
    pub action: i32,
    pub has_actions: bool,
    pub acting: bool,
    pub stage_one: bool,
    pub stage_two: bool,
    pub litter_pending: bool,
}

impl HandProp {
    /// HasHandProp (`826AD1A8`): held, or requested and not created yet.
    pub fn has(&self) -> bool {
        self.holding || self.requested
    }

    /// SpawnInteractionBasedHandProp's request (`82E3DDA0`): the record key and the requested bit; the host
    /// creates the object later and then sets `holding`.
    pub fn request(&mut self, key: &str) {
        self.key = Some(key.to_string());
        self.requested = true;
    }

    /// The plugin prop's hand prop: a weighted pick from its list (`livingworld_props` field `E15E856F2CA9B96B`,
    /// {handprop, probability}) with `roll` in [0, 1). NOT RETAIL YET: that the plugin object's vfunc +52 picks
    /// by these weights is inferred (b87); a one-entry list always gives its entry.
    pub fn pick(list: &[(String, f32)], roll: f32) -> Option<&str> {
        let total: f32 = list.iter().map(|(_, p)| p.max(0.0)).sum();
        if total <= 0.0 {
            return None;
        }
        let mut left = roll.clamp(0.0, 1.0) * total;
        for (key, p) in list {
            left -= p.max(0.0);
            if left < 0.0 {
                return Some(key);
            }
        }
        list.iter().rev().find(|(_, p)| *p > 0.0).map(|(k, _)| k.as_str())
    }

    /// Ped starting prop (ctor 82E33198, 8269A588): chance and pick rolls: docs/hails-additions/26g-living-world-ped-interactions.md#starting-hand-prop
    pub fn starting_pick(chance: f32, list: &[(String, f32)], chance_roll: u32, pick_roll: u32) -> Option<&str> {
        if chance * 100.0 < chance_roll as f32 {
            return None;
        }
        let mut total = 0.0;
        list.iter().find(|(_, p)| {
            total += p;
            pick_roll as f32 <= total * 100.0
        })
        .map(|(k, _)| k.as_str())
    }

    /// The object is gone (thrown, dropped, despawned).
    pub fn clear(&mut self) {
        *self = HandProp::default();
    }
}

/// The tazer's logical clips (entity animation table: `SG_TAZR_STND2TAZR_F_0_N`, `SG_TAZR_TAZR_F_0_CYC`,
/// `SG_TAZR_TAZR2STND_F_0_N`) and its hand prop record (`livingworld_handprops` "tazr", tasergun_lw) [data, b248].
/// Mods change them through those tables.
pub const TAZER_INTO: &str = "TazerInto";
pub const TAZER_CYC: &str = "TazerCyc";
pub const TAZER_OUT: &str = "TazerOut";
pub const TAZER_PROP: &str = "tazr";

impl PedBrain {
    /// Tazer step of the ped update (82E3ED50, b153 / b248): docs/hails-additions/26g-living-world-ped-interactions.md#tazer-clip
    pub fn update_tazer_clip(&mut self, s: &BrainSettings, played: Option<(&str, bool)>, left: f32) {
        let at_end = |clip: &str| played.is_some_and(|(p, ok)| p == clip && (!ok || left <= s.tazer_clip_end));
        match self.tazer_state {
            Some(1) if at_end(TAZER_INTO) => {
                self.tazer_state = Some(2);
                self.chase_requests.push(ChaseRequest::TazerClip { clip: TAZER_CYC });
            }
            Some(3) if at_end(TAZER_OUT) => {
                self.tazer_state = Some(0);
                self.tazer_drawn_requested = false;
                if self.hand_prop.key.as_deref() == Some(TAZER_PROP) {
                    self.hand_prop.clear();
                }
            }
            _ => {}
        }
    }
}

/// What a ped's brain knows and decided (host-owned, serialisable plain data).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PedBrain {
    /// The ped's own rolls (host-seeded; `None` = seed 0) and its type's sit values.
    pub rng: Option<crate::living_world::Rng>,
    pub sit: SitValues,
    pub wants: BTreeMap<String, Want>,
    pub timers: BTreeMap<i32, f32>,
    pub scatter: bool,
    pub honker: Option<u64>,
    /// The collision component's +8 ("needs to begin colliding", NeedsToBeginColliding): set by a contact that
    /// made the ped react (`sub_82E38FB8`), cleared when the Colliding state ends.
    pub begin_colliding: bool,
    /// The component's +10 (IsPedestrianColliding): true while the contact reaction plays (host side).
    pub colliding: bool,
    /// PedestrianColliding began this tick: the host posts the collision mood broadcast (`sub_8269D8A8`).
    pub collision_began: bool,
    pub zombie: bool,
    /// ZombieFollow's current goal (host steers the body to it).
    pub zombie_goal: Option<Vec3>,
    /// OverrideAnimData's entity type: the host plays that type's animation set.
    pub anim_override: Option<String>,
    pub has_plugin: bool,
    /// Outputs the body follows.
    pub motion_intent: Option<u8>,
    pub speed_suggestion: Option<f32>,
    pub flee_from: Option<u64>,
    /// The plugin (conversation id) the ped belongs to, whether the main graph runs it, in a
    /// conversation, the locked waypoint, monitored intents (name -> (stage, stages)), the
    /// speaker's turn passed.
    pub plugin: Option<u64>,
    /// `brain+3278` bits 0x20 / 0x10 (see [`PedOp::SetWaitingToReactFlagOnBegin`]).
    pub waiting_to_react: bool,
    pub reacting_to_mood: bool,
    pub in_plugin: bool,
    pub in_conversation: bool,
    pub waypoint: Option<Vec3>,
    /// The locked waypoint's orientation (a world prop's waypoint facing; host-set with the lock).
    pub waypoint_facing: Option<Vec3>,
    /// `brain+3277` bits 0x01 (OwnPluginObject) and 0x10 (IgnoreStandingCollisions).
    pub own_plugin_object: bool,
    pub ignore_standing_collisions: bool,
    /// `ped+5916`: the plugin prop is ignored by the ped's collision (DisableCollisionsWithBehaviourSource).
    pub ignore_source_collision: bool,
    /// `brain+3277` bit 0x20, ped `+5936` bit 0x40 cleared, `brain+3278` bit 0x08 (with each DisableAllMoods
    /// behaviour's saved value), `brain+3200` / `+3201`.
    pub heavy_collision_disabled: bool,
    pub collision_sliding_disabled: bool,
    /// DisableNavigation: ped `+5936` bit 0x80 cleared (no navigation).
    pub navigation_disabled: bool,
    pub moods_disabled: bool,
    pub moods_saved: BTreeMap<usize, bool>,
    pub plugin_collision_override: bool,
    pub plugin_avoid_override: bool,
    /// The explicit turn direction the locomotion turns to (`ped+5888` +2080 with +2100 bit 0x20).
    pub explicit_turn: Option<Vec3>,
    pub monitored: BTreeMap<String, Packet>,
    pub turn_passed: bool,
    /// The hand prop (`livingworld_handprops` key at `ped+5760`), requested (`brain+3279` bit 0x01, set by
    /// `82E3DDA0`) and held (`brain+3278` bit 0x02, set when `82E3EC60` attaches the created object) [code, b87].
    pub hand_prop: HandProp,
    /// `brain+3279` bit 0x10 (DisallowHandPropActions).
    pub hand_prop_actions_disallowed: bool,
    /// The plugin object's hotpoint 0 (host-set with the plugin lock): ThrowHandPropAtTrashBin's target.
    pub plugin_target: Option<Vec3>,
    /// What the ped knows about and sees (`brain+624`; KnowAboutWantTarget's `82E42868`).
    pub perceptions: super::perception::Perceptions,
    /// LostChasee: the chasee's last known position the ped walks to; SetAltTargetToChaseePosition:
    /// the chasee's live position (alternate steering target; not used by our nav yet).
    pub search_point: Option<Vec3>,
    /// ApproachWantTarget's goal and speed this tick (cleared before each tick) and the look-at
    /// stack (`ped+5900`, at most 3).
    pub approach: Option<(Vec3, f32)>,
    /// TargetWaypoint's slide: within this distance the ped slides onto the point at this speed.
    pub approach_slide: Option<(f32, f32)>,
    /// LockToCurrentPosition: the body holds where it stood when the op began (an origin-locked
    /// trajectory: root motion does not move it) until the op ends.
    pub position_locked: bool,
    pub locked_at: Option<Vec3>,
    /// ConversationListenToSpeaker: the speaker the ped looks at (no head tracking consumer yet).
    pub listen_to: Option<u64>,
    pub look_at: Vec<u64>,
    /// The tazer: draw state (`brain+3264`: 1 drawing, 2 drawn, 3 putting away, 0 put away; b153), draw requested
    /// (`+3280` 0x40), hit delivered (0x20), line of sight (0x10, host-written), the want it aims at
    /// (`+3268`) and whether it is registered (`ped+2492`).
    pub tazer_state: Option<u8>,
    pub tazer_drawn_requested: bool,
    /// The hand was empty when the draw began, so the "tazr" prop is wanted (`826A8258`), and it was requested.
    pub tazer_hand_was_empty: bool,
    pub tazer_prop_requested: bool,
    pub tazer_hit: bool,
    pub tazer_line_of_sight: bool,
    pub tazer_want: Option<String>,
    pub tazer_registered: bool,
    /// Which chase steering runs (all use motion intent 3) and the secondary chaser's block flag
    /// and point (`C+8`, `C+16`).
    pub chase_steer: Option<ChaseSteer>,
    pub should_block: bool,
    pub block_point: Option<Vec3>,
    pub alt_target: Option<Vec3>,
    /// The point the body turns to face (`ped+5888` face direction, bit 0x20 at `+2100`).
    pub face: Option<Vec3>,
    /// The speed suggestion saved by a face / watch Begin, restored by its End.
    pub saved_speed: Option<Option<f32>>,
    /// WatchWantTarget latched into stop-and-face; StandAndWatchSkater's watch point.
    pub watch_latched: bool,
    pub watch_point: Option<Vec3>,
    /// The ped's speech value (`ped+2468`); `None` = the constructor's 68 (`82E33198`), which no
    /// line uses. The game sends it to the audio side when it changes.
    pub speech: Option<i32>,
    /// The variant and row value a conversation turn stores with the speech (`ped+2472` /
    /// `+2476`, `82E3DAD8`); no known audio meaning yet, carried with the speech event.
    pub speech_topic: Option<(u8, i32)>,
    /// The entity whose contacts the ped ignores (`ped+5680`: set by a successful takedown and by
    /// IgnoreCollisionsWithWantTarget, kept until the next one; read by the contact classifier `sub_82E38400`). No
    /// knock-down reaction and no collision mood about it, so a takedown's own bumps do not restart the chase.
    pub ignore_collisions_with: Option<u64>,
    /// The chasee's id (`brain+3204`, the chaser component's handle).
    pub chasee: Option<u64>,
    /// CannotReachChaseTarget's bit (brain `+3280` 0x08), written by the host's intercept movement (`82E31D58`).
    pub target_unreachable: bool,
    /// This chaser's end reason (`brain+2176`).
    pub end_reason: Option<i32>,
    /// Takedowns in this chase (`brain+3248`).
    pub takedowns: u32,
    /// Group changes for the host to apply after the tick, in order.
    pub chase_requests: Vec<ChaseRequest>,
    /// The takedown target (`brain+2180`), the picked takedown, the attempt's result (2 success,
    /// 3 failure; `brain+3224`), whether the takedown intent plays and whether the ped touched
    /// its target during it (the contact record).
    pub takedown_target: Option<u64>,
    pub takedown_choice: Option<super::takedown::TakedownChoice>,
    pub takedown_result: Option<u8>,
    pub takedown_active: bool,
    pub takedown_contact: bool,
    /// The chosen takedown entry may trip on failure (`826A49D8` copies entry byte +0x26 bit 0 to brain +0xCCC bit 0).
    pub takedown_trips: bool,
    /// A contact reaction stored until the next `PedestrianColliding` (the collision component's +8 with
    /// `ped+0x9C0` / `+0x9C4`): a contact during a takedown attempt, or the failed takedown's trip (b326).
    pub pending_reaction: Option<super::skater_contact::StoredReaction>,
    /// The stored reaction moved here when `PedestrianColliding` begins; the host plays it that tick.
    pub reaction_due: Option<super::skater_contact::StoredReaction>,
    /// Seconds spent chasing (`brain+3220`; the Intercept and Pursue updates add dt) and the
    /// resting flag (`brain+3277` bit 0x02).
    pub chase_exhaustion: f32,
    pub chase_resting: bool,
    /// Nav modifiers the graph set (`None` / absent = the ped's own setting).
    pub nav_modifiers: BTreeMap<u8, bool>,
    /// What each ChangeNavModifierSetting behaviour saved at Begin, by behaviour id.
    pub nav_saved: BTreeMap<usize, Option<bool>>,
}

impl PedBrain {
    /// `sub_82E3BF70`: the mood producer does nothing for a busy ped. Retail ORs the colliding flag (`brain+3277`
    /// bit 0x04, `82E3BF60`, b316 / b393) and `*(ped+96)` (the plugin instance, b318) with the two flags. Without the
    /// colliding term ours ran the producer one tick into Colliding (laptop session 2026-10-20, #281 / #126).
    pub fn busy(&self) -> bool {
        self.waiting_to_react || self.reacting_to_mood || self.colliding || self.plugin.is_some()
    }
    /// Leaving a plugin in any way releases its waypoint: retail's slot `ped+0x1718` belongs to the plugin, and
    /// LockFirstWaypoint (`826A0580`) only locks an empty one, so a kept prop waypoint would steer the next
    /// conversation (research b186).
    pub fn release_plugin_waypoint(&mut self) {
        self.waypoint = None;
        self.waypoint_facing = None;
    }
    /// A nav modifier: the graph's setting, else on (the ped's own default; per-type values open).
    pub fn nav_modifier(&self, modifier: u8) -> bool {
        self.nav_modifiers.get(&modifier).copied().unwrap_or(true)
    }
}

/// The chase steering a ped follows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ChaseSteer {
    Intercept,
    /// Run to the formation point.
    Pursue,
    /// Walk to the block point at `slow_speed`, stand there.
    Block { at_target: f32, slow_speed: f32 },
}

/// A change to a chase group the brain asks the host for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ChaseRequest {
    /// A hand prop throw started: the host plays `clip` (blend [`super::hand_prop::HandPropSettings::clip_blend`]).
    HandPropClip { clip: &'static str },
    /// A tazer clip on the upper-body channel (`TAZER_INTO` / `TAZER_CYC` / `TAZER_OUT`, ped vfn248, blends 0.2).
    TazerClip { clip: &'static str },
    /// The held hand prop leaves the hand with `velocity` (m/s): the host turns it into a physics prop.
    HandPropReleased { velocity: Vec3 },
    Join { chasee: u64 },
    /// SendChaseStateMessage: the host posts it when `target` is a player.
    StateMessage { target: u64, state: u8 },
    Leave { chasee: u64 },
    GiveUpPrimary { chasee: u64 },
    GroupEnd { chasee: u64, reason: i32 },
    /// TakeDownTargetableSuccess: knock `target` down (the host speaks 65 / 19).
    Takedown { target: u64 },
    /// TakeDownTargetableFailure.
    TakedownFailed { target: u64 },
    /// ChannelGreetWantTarget: post `greeted` (instigator = this ped) into `target`'s mood store.
    Greeted { target: u64 },
    /// SpawnConversationArea: a conversation with `target`.
    SpawnConversation { target: u64 },
    /// Plugin / conversation changes for the host.
    LockWaypoint { at: Vec3 },
    UnlockWaypoint,
    ExitPlugin,
    SignalInPosition,
    PassTurn,
    /// ConversationSpeak Begin: the host gives every listener the speech value 41.
    Spoke,
    /// TazeWantTarget's hit: knock `target` down (the takedown's skater path).
    Taze { target: u64 },
    /// RegisterTazer: the tazer is live (audio burst).
    TazerOn,
    /// TakedownTauntVictim: speak (65 player / 19 other) and play the "Taunt" clip; the host removes the
    /// "SGIntent" monitored intent when the clip ends.
    Taunt { target: u64 },
    /// MoodResetAboutChasee: the host forgets the mood records about `target`.
    MoodReset { target: u64 },
}

/// Timer 1 InvestigateTimer and timer 4 RestTimer.
pub const INVESTIGATE_TIMER: i32 = 1;
pub const REST_TIMER: i32 = 4;

/// What the plugin graph reads of the ped's conversation.
#[derive(Clone, Copy, Debug)]
pub struct ConversationInfo<'a> {
    pub complete: bool,
    pub speaker: Option<u64>,
    pub center: Vec3,
    pub free_waypoints: &'a [Vec3],
    /// This turn's speech for the speaker (`82E3DAD8`).
    pub speech: Option<super::conversation::TurnSpeech>,
}

/// ConversationSpeak's turn timer (timer 30 ConversationSpeakingTimer).
pub const SPEAK_TIMER: i32 = 30;

/// The time a former primary chaser waits (`TimeUntilICanBePrimaryChaserTimer`).
pub const PRIMARY_TIMER: i32 = 29;

/// What the brain reads of chases: its own id, the ped type's chase record (`ped+5712`) and the
/// chase groups by chasee id.
#[derive(Clone, Copy, Default)]
pub struct ChaseView<'a> {
    pub me: u64,
    pub record: Option<&'a super::chase::ChaseRecord>,
    pub groups: Option<&'a dyn Fn(u64) -> Option<super::chase::ChaseGroupInfo>>,
    /// The ped's conversation: complete, the speaker, its centre and its free waypoints.
    pub conversation: Option<ConversationInfo<'a>>,
    /// A secondary chaser's block rule and formation point against its chasee (host-computed).
    pub block: Option<&'a dyn Fn(u64) -> Option<(bool, Vec3)>>,
    /// The takedown that fits now against a target (`82E3C000`).
    pub takedowns: Option<&'a dyn Fn(u64) -> Option<super::takedown::TakedownChoice>>,
    /// Target velocities by id and the ped's own velocity, m/s (the attack throw's prediction, `82E3E960`).
    pub velocity: Option<&'a dyn Fn(u64) -> Option<Vec3>>,
    pub own_velocity: Vec3,
    /// Whether a chasee is wiping out (the chase group's vfn +88: the skater's phys-out byte 59, `825948B0`; a ped's
    /// brain `+3277` bit 0x04) [code, b106].
    pub wiping: Option<&'a dyn Fn(u64) -> bool>,
}

impl ChaseView<'_> {
    fn group(&self, chasee: u64) -> Option<super::chase::ChaseGroupInfo> {
        self.groups.and_then(|g| g(chasee))
    }
}

impl PedBrain {
    /// Raise a want (the mood system's job; a mod or test may call it).
    pub fn set_want(&mut self, want: &str, target: u64) {
        self.raise_want(want, target, false);
    }
    /// A want from the mood producer, with the result's flag 0x40 ([`Want::forget_on_unset`]).
    pub fn raise_want(&mut self, want: &str, target: u64, forget_on_unset: bool) {
        self.wants.insert(want.to_ascii_lowercase(), Want { target, needs_addressing: true, forget_on_unset });
    }
    /// A passed mood result's want (`sub_82E41B98`): raised, and its target known for at least `memory` seconds
    /// ([`BrainSettings::mood_want_memory_seconds`], `82E42868`), so the entry that counts finished reactions exists.
    pub fn raise_mood_want(&mut self, want: &str, target: u64, forget_on_unset: bool, memory: f32) {
        self.raise_want(want, target, forget_on_unset);
        self.perceptions.know(target, memory);
    }
    /// sub_82E42A58(brain, want, 0): count a finished reaction, or forget the target (0x40 wants): docs/hails-additions/26g-living-world-ped-interactions.md#unset-want
    pub fn unset_want(&mut self, want: &str) {
        let Some(w) = self.wants.remove(want) else { return };
        if w.forget_on_unset {
            if w.needs_addressing {
                self.perceptions.forget(w.target);
                self.chase_requests.push(ChaseRequest::MoodReset { target: w.target });
            }
        } else if w.needs_addressing {
            if let Some(e) = self.perceptions.get_mut(w.target) {
                e.reacted += 1;
            }
        }
    }
    /// `sub_82E40940`: a length at or below 0 stops the timer; a new timer is dropped when
    /// [`timers::CAPACITY`] are running.
    pub fn rng(&mut self) -> &mut crate::living_world::Rng {
        self.rng.get_or_insert_with(|| crate::living_world::Rng::new(0))
    }

    /// The motion side's end of a packet whose motion state is not ported (`motion_handles` false): taken as done once
    /// the packet reaches its last stage, so it goes inactive. NOT RETAIL: retail's motion graph clears `+456` when the
    /// state's last clip completes (`MajorIntentComplete`, b84).
    pub fn settle_packets(&mut self, motion_handles: &dyn Fn(&str) -> bool) {
        for (name, p) in self.monitored.iter_mut() {
            if !motion_handles(name) && p.stages.len() > 1 && usize::from(p.stage) >= p.stages.len() {
                p.active = false;
            }
        }
    }

    /// `GoingToStandBackUp`'s roll.
    pub fn roll_stand_up(&mut self) -> bool {
        self.roll_chance(self.sit.stand_up_chance)
    }

    /// `8269A588`: true when `chance` x 100 is at least a 1..=100 roll (one draw of the brain's RNG).
    pub fn roll_chance(&mut self, chance: f32) -> bool {
        let k = self.rng().modulo(100) + 1;
        chance * 100.0 >= k as f32
    }

    pub fn set_timer(&mut self, timer: i32, seconds: f32) {
        if seconds <= 0.0 {
            self.timers.remove(&timer);
        } else if self.timers.contains_key(&timer) || self.timers.len() < timers::CAPACITY {
            self.timers.insert(timer, seconds);
        }
    }
    /// `sub_82E40A80`: the time left, 0 when the timer is not running.
    pub fn timer(&self, timer: i32) -> f32 {
        self.timers.get(&timer).copied().unwrap_or(0.0)
    }
    /// Timers run down with the graph's tick, clamped at 0 (`82E40940` clamps).
    pub fn tick_timers(&mut self, dt: f32) {
        for t in self.timers.values_mut() {
            *t = (*t - dt).max(0.0);
        }
    }
}

/// One tick's view of a ped for the graph: the program's operations by behaviour / condition id,
/// the brain, and where targets are.
pub struct BrainHost<'a> {
    pub behaviors: &'a [PedOp],
    pub conditions: &'a [PedOp],
    pub brain: &'a mut PedBrain,
    pub settings: &'a BrainSettings,
    pub position: Vec3,
    /// The ped's heading (radians about up, 0 = +z) and the skater it watches (position,
    /// velocity).
    pub heading: f32,
    pub skater: Option<(Vec3, Vec3)>,
    pub target_position: &'a dyn Fn(u64) -> Option<Vec3>,
    pub chase: ChaseView<'a>,
}

impl BrainHost<'_> {
    fn want_distance_sq(&self, want: &str) -> Option<f32> {
        let w = self.brain.wants.get(want)?;
        let p = (self.target_position)(w.target)?;
        let d = [p[0] - self.position[0], p[1] - self.position[1], p[2] - self.position[2]];
        Some(d[0] * d[0] + d[1] * d[1] + d[2] * d[2])
    }

    fn want_target(&self, want: &str) -> Option<Vec3> {
        self.brain.wants.get(want).and_then(|w| (self.target_position)(w.target))
    }

    /// Unit flat direction from the ped to `p`.
    fn flat_dir(&self, p: Vec3) -> Option<[f32; 2]> {
        let d = [p[0] - self.position[0], p[2] - self.position[2]];
        let l = (d[0] * d[0] + d[1] * d[1]).sqrt();
        (l > 1e-4).then(|| [d[0] / l, d[1] / l])
    }

    fn forward(&self) -> [f32; 2] {
        [self.heading.sin(), self.heading.cos()]
    }

    fn evaluate(&self, op: &PedOp) -> bool {
        let b = &*self.brain;
        match op {
            PedOp::IsOnRoad | PedOp::SimpleRandom { .. } => false,
            PedOp::IsPluginCollisionOverride => b.plugin_collision_override,
            PedOp::IsPluginAvoidOverride => b.plugin_avoid_override,
            PedOp::IsAtWaypoint { radius } => self.brain.waypoint.is_some_and(|w| {
                let (dx, dz) = (w[0] - self.position[0], w[2] - self.position[2]);
                (dx * dx + dz * dz).sqrt() < *radius
            }),
            PedOp::InSkaterRadius { radius } => self.skater.is_some_and(|(p, _)| {
                let d = [p[0] - self.position[0], p[1] - self.position[1], p[2] - self.position[2]];
                (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt() < *radius
            }),
            PedOp::HasWantToAddress { want } => b.wants.get(want).is_some_and(|w| w.needs_addressing),
            PedOp::NeedsToBeginColliding => b.begin_colliding,
            PedOp::IsColliding => b.colliding,
            PedOp::ShouldScatter => b.scatter,
            PedOp::IsBeingHonkedAt => b.honker.is_some(),
            PedOp::IsZombieMode => b.zombie,
            PedOp::HasPlugin => b.has_plugin,
            PedOp::HasHandProp => b.hand_prop.has(),
            PedOp::IsHoldingSpecificHandProp { handprop } => b.hand_prop.has() && b.hand_prop.key.as_deref().is_some_and(|k| k.eq_ignore_ascii_case(handprop)),
            PedOp::HasDisposableHandProp => b.hand_prop.has() && b.hand_prop.disposable,
            PedOp::CanSitWithHandProp => !b.hand_prop.has() || b.hand_prop.can_sit,
            PedOp::CanAttackThrowHandProp => b.hand_prop.has() && b.hand_prop.can_attack_throw,
            PedOp::SimpleTimerExpired { timer } => b.timer(*timer) <= 0.0,
            PedOp::IsChasing => b.chasee.is_some(),
            // A target without a group yet takes chasers (the group is empty).
            PedOp::CanChaseeAddNewChaser => b.wants.get("angrychase").is_some_and(|w| self.chase.group(w.target).is_none_or(|g| g.can_add)),
            PedOp::IsPrimaryChaser => b.chasee.and_then(|c| self.chase.group(c)).is_some_and(|g| g.primary == Some(self.chase.me)),
            PedOp::ChaseeHasProtector | PedOp::ChasersAreScared => false,
            // `82E3D988`: no group -> not ending; an ending group's reason wins.
            PedOp::IsChaserEndingChase => b.chasee.and_then(|c| self.chase.group(c)).is_some_and(|g| g.end_reason.is_some() || b.end_reason.is_some()),
            // The ped's own end reason, else its chase group's (a group end reaches every member).
            // `826AAC80` -> GetEndReason `82E3D988`: no group answers -1; an ending group's reason wins over the ped's own.
            PedOp::IsEndChaseReason { reason } => b.chasee.and_then(|c| self.chase.group(c)).and_then(|g| g.end_reason.or(b.end_reason)) == Some(*reason),
            // `82E3D7B0` (exhausted: accumulator past the limit; no record or field: never),
            // `82E3D838` (rested: not exhausted and timer 4 out).
            PedOp::NeedToRest => {
                let exhausted = self.chase.record.and_then(|r| r.exhaustion_limit()).is_some_and(|l| b.chase_exhaustion > l);
                exhausted || (b.chase_resting && !(!exhausted && b.timer(REST_TIMER) <= 0.0))
            }
            PedOp::InvestigateTimeExceeded => b.timer(INVESTIGATE_TIMER) <= 0.0,
            PedOp::ChaserShouldBlock => b.should_block,
            PedOp::PedestrianIsInConversation => b.in_conversation,
            PedOp::HasWaypointLocked => b.waypoint.is_some(),
            PedOp::DistanceFromWaypointXZ { less_equal, greater, y_tolerance } => b.waypoint.is_some_and(|w| {
                if (w[1] - self.position[1]).abs() > *y_tolerance {
                    return false;
                }
                let d = ((w[0] - self.position[0]).powi(2) + (w[2] - self.position[2]).powi(2)).sqrt();
                less_equal.is_none_or(|l| d <= l) && greater.is_none_or(|g| d > g)
            }),
            PedOp::IsFacingWaypointOrientation { fov } => self.chase.conversation.and_then(|c| self.flat_dir(c.center)).is_some_and(|d| {
                let f = self.forward();
                (f[0] * d[0] + f[1] * d[1]).clamp(-1.0, 1.0).acos() <= *fov
            }),
            PedOp::ConversationThisParticipantIsSpeaker => self.chase.conversation.is_some_and(|c| c.speaker == Some(self.chase.me)),
            PedOp::ConversationIsComplete => self.chase.conversation.is_none_or(|c| c.complete),
            PedOp::IsFacingWantTarget { want, angle, distance } => self.want_target(want).is_some_and(|p| {
                let d = [p[0] - self.position[0], p[2] - self.position[2]];
                if *distance != 0.0 && d[0] * d[0] + d[1] * d[1] > distance * distance {
                    return false;
                }
                // The heading difference in [0, 2 pi): inside half the cone either side.
                let half = (angle * 0.5).to_radians();
                let diff = (d[0].atan2(d[1]) - self.heading).rem_euclid(std::f32::consts::TAU);
                diff < half || diff > std::f32::consts::TAU - half
            }),
            PedOp::HasDrawnTazer => b.tazer_drawn_requested && b.tazer_state == Some(2),
            PedOp::HasLineOfSightToTazeTarget => b.tazer_line_of_sight,
            PedOp::CanSeeChasee => b.chasee.and_then(|c| b.perceptions.get(c)).is_some_and(|e| e.visible || e.told),
            PedOp::HasTakeDownTargetable => b.takedown_target.is_some(),
            // `826ACA08` also asks the target's vfn +24 = `825948B0` (wiping out) [code, b106].
            PedOp::CanAttemptTakeDownTargetable => {
                b.takedown_target.is_some_and(|t| (self.target_position)(t).is_some() && !self.chase.wiping.is_some_and(|w| w(t))) && b.takedown_choice.is_some()
            }
            PedOp::CannotReachChaseTarget => b.target_unreachable,
            PedOp::CanTazeUnreachableTargets => false,
            PedOp::ChaseeIsWipedOut => b.chasee.is_some_and(|c| self.chase.group(c).is_some() && self.chase.wiping.is_some_and(|w| w(c))),
            PedOp::ChaseeCloseEnoughToSatisfyAggressiveChase => b.chasee.and_then(|c| (self.target_position)(c)).is_some_and(|q| {
                let d = [q[0] - self.position[0], q[1] - self.position[1], q[2] - self.position[2]];
                let limit = self.settings.aggressive_capture_distance;
                d[0] * d[0] + d[1] * d[1] + d[2] * d[2] < limit * limit
            }),
            PedOp::HasMonitoredIntent { intent } => (intent == "ActiveTakedown" && b.takedown_active) || b.monitored.get(intent).is_some_and(|p| p.active),
            PedOp::TakeDownAttemptSuccessful => b.takedown_result == Some(2),
            PedOp::TakeDownAttemptFailed => b.takedown_result == Some(3),
            PedOp::ChaserShouldGiveUpDueToTakedowns => self.chase.record.and_then(|r| r.give_up_after_takedowns()).is_some_and(|n| b.takedowns >= n),
            PedOp::CanNewChaseStart => self.settings.chases_enabled,
            // No record: the escape distance reads 0.0, so any distance escapes.
            PedOp::ChaseeEscaped => match b.chasee.and_then(|c| (self.target_position)(c)) {
                Some(q) => super::chase::escaped(self.position, q, self.chase.record.unwrap_or(&super::chase::ChaseRecord::default())),
                None => false,
            },
            PedOp::IsFacingSkater { fov, distance } => self.skater.is_some_and(|(p, _)| {
                let near = *distance == 0.0 || {
                    let (dx, dz) = (p[0] - self.position[0], p[2] - self.position[2]);
                    *distance > (dx * dx + dz * dz).sqrt()
                };
                near && self.flat_dir(p).is_some_and(|d| {
                    let f = self.forward();
                    f[0] * d[0] + f[1] * d[1] > fov.cos()
                })
            }),
            PedOp::DistanceToWantTarget { want, greater, less } => match self.want_distance_sq(want) {
                Some(d2) => greater.is_none_or(|g| d2 > g * g) && less.is_none_or(|l| d2 < l * l),
                None => false,
            },
            _ => false,
        }
    }
}

impl ConditionHost for BrainHost<'_> {
    fn condition_activation(&mut self, condition: usize, _frame: &Frame) -> u32 {
        match self.conditions.get(condition) {
            Some(PedOp::GoingToStandBackUp) => return u32::from(self.brain.roll_stand_up()),
            Some(PedOp::SimpleRandom { chance }) => return u32::from(*chance >= self.brain.rng().unit()),
            _ => {}
        }
        u32::from(self.conditions.get(condition).is_some_and(|op| self.evaluate(op)))
    }
}

impl Host for BrainHost<'_> {
    fn context(&self) -> [u32; 6] {
        [0; 6]
    }
    fn allocate(&mut self, behavior: BehaviorId, _frame: &Frame) -> u32 {
        behavior as u32 + 1
    }
    fn begin(&mut self, behavior: BehaviorId, _context: [u32; 6], _frame: &Frame) {
        let Some(op) = self.behaviors.get(behavior) else { return };
        let s = *self.settings;
        let b = &mut *self.brain;
        match op {
            PedOp::PedestrianColliding => {
                b.collision_began = true;
                // A reaction stored earlier plays now (`8269DA00` posts it at Begin); the host takes it this tick.
                b.reaction_due = b.pending_reaction.take();
            }
            PedOp::Wander => {
                b.honker = None;
                b.motion_intent = Some(motion::WANDER);
            }
            PedOp::NoRoadWander => b.motion_intent = Some(motion::WANDER),
            PedOp::ThrowHandPropAtTrashBin => {
                if let Some(target) = b.plugin_target {
                    if let Some(clip) = b.start_light_throw(&s.hand_prop, self.heading, self.position, target) {
                        b.chase_requests.push(ChaseRequest::HandPropClip { clip });
                    }
                }
            }
            PedOp::ThrowHandPropAtWantTarget { want } => {
                let target = b.wants.get(want).map(|w| w.target);
                let motion = target.and_then(|t| Some(((self.target_position)(t)?, self.chase.velocity.and_then(|v| v(t)).unwrap_or([0.0; 3]))));
                if let Some((at, velocity)) = motion {
                    let aim = super::hand_prop::AttackAim { position: self.position, velocity: self.chase.own_velocity, target: at, target_velocity: velocity };
                    if let Some(clip) = b.start_attack_throw(&s.hand_prop, self.heading, aim) {
                        b.chase_requests.push(ChaseRequest::HandPropClip { clip });
                    }
                }
            }
            PedOp::ZombieFollow => {
                b.motion_intent = Some(motion::ZOMBIE_FOLLOW);
                b.zombie_goal = self.skater.map(|(p, _)| p);
            }
            PedOp::OverrideAnimData { entity } => b.anim_override = Some(entity.clone()),
            PedOp::DropHandProp => {
                if let Some(velocity) = b.drop_hand_prop() {
                    b.chase_requests.push(ChaseRequest::HandPropReleased { velocity });
                }
            }
            PedOp::DisallowHandPropActions => b.hand_prop_actions_disallowed = true,
            PedOp::SuggestVelocity { linear } => b.speed_suggestion = Some(*linear),
            PedOp::KnowAboutWantTarget { want } => {
                if let Some(w) = b.wants.get(want) {
                    b.perceptions.know(w.target, s.know_about_seconds);
                }
            }
            PedOp::UnsetWant { want } => b.unset_want(want),
            PedOp::ChannelGreetWantTarget { want, deactivate, return_greet } => match b.wants.get(want).map(|w| w.target) {
                Some(target) if (self.target_position)(target).is_some() => {
                    b.set_timer(timers::WARN, s.greet_seconds);
                    if !return_greet {
                        b.chase_requests.push(ChaseRequest::Greeted { target });
                    }
                }
                _ => {
                    if *deactivate {
                        b.unset_want(want);
                    }
                }
            },
            PedOp::AlertToWantTarget { want } => {
                if let Some(t) = b.wants.get(want).map(|w| w.target) {
                    if b.look_at.len() < 3 {
                        b.look_at.push(t);
                    }
                }
            }
            PedOp::ChannelWarnWantTarget { .. } => {
                b.set_timer(timers::WARN, s.warn_seconds);
                b.speech = Some(s.warn_speech);
            }
            // `826A77A8`: no ped speech event in zombie mode (b95).
            PedOp::SendSpeech { value } if !b.zombie => b.speech = Some(*value),
            PedOp::SetSimpleTimer { timer, length } => b.set_timer(*timer, *length),
            PedOp::SetTargetUnreachableTimer => {
                let seconds = self.chase.record.map_or(0.0, |r| r.unreachable_timer());
                b.set_timer(timers::index("TargetUnreachableTimer"), seconds);
            }
            PedOp::InterceptChasee => {
                b.motion_intent = Some(motion::INTERCEPT);
                b.chase_steer = Some(ChaseSteer::Intercept);
            }
            PedOp::PursueChasee => {
                b.motion_intent = Some(motion::INTERCEPT);
                b.chase_steer = Some(ChaseSteer::Pursue);
            }
            PedOp::BlockChasee { at_target, slow_speed } => {
                b.motion_intent = Some(motion::INTERCEPT);
                b.chase_steer = Some(ChaseSteer::Block { at_target: *at_target, slow_speed: *slow_speed });
            }
            PedOp::RestFromChase => {
                b.chase_exhaustion = 0.0;
                b.chase_resting = true;
                b.set_timer(REST_TIMER, self.chase.record.map_or(0.0, |r| r.rest_time()));
            }
            PedOp::ClearExhaustionTimer => b.chase_exhaustion = 0.0,
            PedOp::SetWaitingToReactFlagOnBegin => b.waiting_to_react = true,
            PedOp::ClearWaitingToReactFlagOnBegin => b.waiting_to_react = false,
            PedOp::SetIsReactingToMoodEventFlagOnBegin => b.reacting_to_mood = true,
            PedOp::Plugin => b.in_plugin = true,
            PedOp::ExitPlugin => b.chase_requests.push(ChaseRequest::ExitPlugin),
            PedOp::SpawnConversationArea => {
                if let Some(target) = b.wants.get("startconversation").map(|w| w.target) {
                    b.chase_requests.push(ChaseRequest::SpawnConversation { target });
                }
            }
            PedOp::PedestrianInConversation => b.in_conversation = true,
            PedOp::UnlockWaypoint => {
                if b.waypoint.take().is_some() {
                    b.chase_requests.push(ChaseRequest::UnlockWaypoint);
                }
            }
            PedOp::LockToCurrentPosition => {
                b.speed_suggestion = Some(0.0);
                b.approach = None;
                b.position_locked = true;
                b.locked_at = Some(self.position);
            }
            PedOp::OwnPluginObject => b.own_plugin_object = true,
            PedOp::DisableCollisionsWithBehaviourSource => {
                b.own_plugin_object = true;
                b.ignore_source_collision = true;
            }
            PedOp::DisableHeavyCollision => b.heavy_collision_disabled = true,
            PedOp::DisableCollisionSliding => b.collision_sliding_disabled = true,
            PedOp::DisableAllMoods => {
                b.moods_saved.insert(behavior, b.moods_disabled);
                b.moods_disabled = true;
            }
            PedOp::OverridePluginCollision => b.plugin_collision_override = true,
            PedOp::OverridePluginAvoid => b.plugin_avoid_override = true,
            PedOp::IgnoreStandingCollisions => b.ignore_standing_collisions = true,
            PedOp::SetExplicitTurnDirectionToWaypointOrientation => {
                if b.waypoint.is_some() {
                    b.explicit_turn = b.waypoint_facing;
                }
            }
            PedOp::SetSitTimer => {
                let v = b.sit;
                let t = v.min_seconds + b.rng().unit() * (v.max_seconds - v.min_seconds);
                b.set_timer(timers::SIT, t);
            }
            PedOp::CreateSimpleMonitoredIntent { intent, names, .. } => {
                b.monitored.insert(intent.clone(), Packet::new(names.clone()));
            }
            // `826A27D0`: only the stage counter moves; the motion side ends the packet.
            PedOp::IncrementMonitoredPacketStage { intent } => {
                if let Some(p) = b.monitored.get_mut(intent) {
                    p.stage = p.stage.saturating_add(1);
                }
            }
            PedOp::ConversationSignalInPosition => b.chase_requests.push(ChaseRequest::SignalInPosition),
            // `826A66B8` -> `82E3DAD8`: the speaker's line, listeners 41 (host).
            PedOp::ConversationSpeak => {
                b.set_timer(SPEAK_TIMER, s.conversation_turn_seconds);
                b.turn_passed = false;
                if let Some(t) = self.chase.conversation.and_then(|c| c.speech) {
                    b.speech = Some(t.value);
                    b.speech_topic = Some((t.variant, t.list_value));
                    b.chase_requests.push(ChaseRequest::Spoke);
                }
            }
            // `826A8258`: the draw clip on the upper-body channel (ped vfn248 -> vfn240, blends 0.2), and whether the
            // hand was empty (only then the "tazr" prop comes once timer 37 runs out).
            PedOp::DrawTazer { want } => {
                b.tazer_state = Some(1);
                b.set_timer(37, s.tazer_draw_seconds);
                b.tazer_drawn_requested = true;
                b.tazer_want = Some(want.clone());
                b.tazer_hit = false;
                b.tazer_line_of_sight = false;
                b.tazer_hand_was_empty = !b.hand_prop.has();
                b.tazer_prop_requested = false;
                b.chase_requests.push(ChaseRequest::TazerClip { clip: TAZER_INTO });
            }
            // `826A8398`: the speech and the hit timer start only when the want's target resolves.
            PedOp::TazeWantTarget { want } => {
                if b.wants.get(want).is_some_and(|w| (self.target_position)(w.target).is_some()) {
                    b.speech = Some(s.taze_speech);
                    b.set_timer(39, s.tazer_hit_seconds);
                }
            }
            PedOp::RegisterTazer { .. } => {
                b.tazer_registered = true;
                b.chase_requests.push(ChaseRequest::TazerOn);
            }
            // `826A85A8`: the holster clip and state 3; the draw request, state 0 and the prop's removal wait for the
            // clip's end (`82E3ED50`, [`PedBrain::update_tazer_clip`]; b153 M2).
            PedOp::EndTaze { .. } => {
                b.tazer_state = Some(3);
                b.speech = Some(s.end_taze_speech);
                b.chase_requests.push(ChaseRequest::TazerClip { clip: TAZER_OUT });
            }
            PedOp::SuppressMoodAboutChasee { timeout } => {
                if let Some(c) = b.chasee {
                    b.perceptions.suppress(c, *timeout);
                }
            }
            PedOp::UnsetWantAndSuppressMoodForATime { want, timeout } => {
                if let Some(w) = b.wants.get(want).copied().filter(|w| w.needs_addressing) {
                    b.unset_want(want);
                    b.perceptions.suppress(w.target, *timeout);
                }
            }
            PedOp::MoodResetAboutChasee => {
                if let Some(c) = b.chasee {
                    b.perceptions.forget(c);
                    b.chase_requests.push(ChaseRequest::MoodReset { target: c });
                }
            }
            PedOp::StartInvestigateTimer => b.set_timer(INVESTIGATE_TIMER, self.chase.record.map_or(0.0, |r| r.investigate_time())),
            PedOp::LockFirstWaypoint => {
                if self.brain.waypoint.is_none() {
                    if let Some(w) = self.chase.conversation.and_then(|c| c.free_waypoints.first().copied()) {
                        self.brain.waypoint = Some(w);
                        self.brain.chase_requests.push(ChaseRequest::LockWaypoint { at: w });
                    }
                }
                return;
            }
            PedOp::LockClosestWaypoint => {
                // `82E1CBC0`: the nearest free waypoint, locked now (the host records the lock).
                let at = self.position;
                let d2 = |p: &Vec3| (p[0] - at[0]).powi(2) + (p[1] - at[1]).powi(2) + (p[2] - at[2]).powi(2);
                let w = self.chase.conversation.and_then(|c| c.free_waypoints.iter().copied().min_by(|a, b| d2(a).total_cmp(&d2(b))));
                if let Some(w) = w {
                    self.brain.waypoint = Some(w);
                    self.brain.chase_requests.push(ChaseRequest::LockWaypoint { at: w });
                }
                return;
            }
            PedOp::AttemptTakeDownTargetable => {
                b.takedown_active = true;
                b.takedown_result = None;
                b.takedown_contact = false;
                // `826A49D8` clears the begin-colliding byte (+8): a reaction stored before the attempt is dropped. It
                // keeps the chosen entry's "may trip" flag for a failure.
                b.begin_colliding = false;
                b.pending_reaction = None;
                b.takedown_trips = b.takedown_choice.is_some_and(|c| c.trips);
            }
            // Ours: the takedown intent ends with the decision (no takedown clips play yet).
            PedOp::TakeDownTargetableSuccess => {
                b.takedown_active = false;
                // `826A4E50` clears the controller's begin-colliding byte (+8): the touch that decided the takedown
                // starts no Colliding reaction (b317).
                b.begin_colliding = false;
                if let Some(target) = b.takedown_target {
                    // `826A4E50` also stores the target in `ped+5680`: its contacts are ignored from now on.
                    b.ignore_collisions_with = Some(target);
                    b.chase_requests.push(ChaseRequest::Takedown { target });
                }
            }
            PedOp::IgnoreCollisionsWithWantTarget { want } => {
                if let Some(w) = b.wants.get(want) {
                    b.ignore_collisions_with = Some(w.target);
                }
            }
            PedOp::TakeDownTargetableFailure { trip_chance } => {
                // `826A5068`: refusal first, the roll only when not refused (one RNG draw, only with the entry flag).
                // Every retail takedown target accepts: target slot 0x10 is `8281DD70` (`li r3,1`) in all seven
                // vtables carrying the interface (b340, 26g), so the refusal branch never runs.
                let refused = false;
                if refused || (b.takedown_trips && b.roll_chance(*trip_chance)) {
                    b.pending_reaction = Some(super::skater_contact::StoredReaction { kind: s.trip_kind, direction: s.trip_direction, closing: 0.0, toucher: None });
                    b.begin_colliding = true;
                }
                // Otherwise a reaction stored during the attempt stays stored and plays at the next Colliding.
                b.takedown_active = false;
                if let Some(target) = b.takedown_target {
                    b.chase_requests.push(ChaseRequest::TakedownFailed { target });
                }
            }
            PedOp::ActivateRelatedWant { original, related, deactivate_original } => {
                // An empty original slot copies nothing we model (inferred: no want raised).
                if let Some(w) = b.wants.get(original).copied() {
                    // The related want takes the original's target and flags (inferred: the same slot record).
                    b.wants.insert(related.clone(), Want { target: w.target, needs_addressing: true, forget_on_unset: w.forget_on_unset });
                    if *deactivate_original {
                        if let Some(o) = b.wants.get_mut(original) {
                            o.needs_addressing = false;
                        }
                    }
                }
            }
            PedOp::ChangeNavModifierSetting { modifier, set_to } => {
                let prior = b.nav_modifiers.insert(*modifier, *set_to);
                b.nav_saved.insert(behavior, prior);
            }
            PedOp::NewChasee => {
                b.speech = Some(s.new_chasee_speech);
                b.takedowns = 0;
                // The new target is the angrychase want's (as CanChaseeAddNewChaser reads it).
                if let Some(target) = b.wants.get("angrychase").map(|w| w.target) {
                    // Retail removes the ped from the NEW target's group before joining it (as read).
                    if b.chasee.is_some() {
                        b.chase_requests.push(ChaseRequest::Leave { chasee: target });
                    }
                    b.chasee = Some(target);
                    b.end_reason = None;
                    // Ours: a new chase starts reachable (retail leaves the bit to the next report, b108).
                    b.target_unreachable = false;
                    b.chase_requests.push(ChaseRequest::Join { chasee: target });
                }
            }
            PedOp::GiveUpBeingPrimaryChaser { timeout } => {
                if let Some(c) = b.chasee {
                    let me = self.chase.me;
                    if self.chase.group(c).is_some_and(|g| g.count >= 2 && g.primary == Some(me)) {
                        let held = b.timer(PRIMARY_TIMER).max(*timeout);
                        b.set_timer(PRIMARY_TIMER, held);
                        b.chase_requests.push(ChaseRequest::GiveUpPrimary { chasee: c });
                    }
                }
            }
            PedOp::ChaserEndChase { reason } => b.end_reason = Some(*reason),
            PedOp::SendChaseStateMessage { state } if !b.zombie => {
                let target = b.chasee.or_else(|| (*state == 0).then(|| b.wants.get("warn").filter(|w| w.needs_addressing).map(|w| w.target)).flatten());
                if let Some(target) = target {
                    b.chase_requests.push(ChaseRequest::StateMessage { target, state: *state });
                }
            }
            PedOp::ChaserGroupEndChase { reason } => {
                if let Some(c) = b.chasee {
                    b.chase_requests.push(ChaseRequest::GroupEnd { chasee: c, reason: *reason });
                }
            }
            PedOp::EndChase => {
                b.chase_steer = None;
                // SetChasee(none) `82E3CFF8`: the end reason resets (`brain+2176` = -1), and clearing a set chasee unsets
                // want slot 1 (`82E42A58(brain, 1, 0)`; slot 1 is angrychase, the brain's debug list `0x820646BC`).
                b.end_reason = None;
                if let Some(c) = b.chasee.take() {
                    b.chase_requests.push(ChaseRequest::Leave { chasee: c });
                    b.unset_want("angrychase");
                }
                // Ours: no stale CannotReachChaseTarget into the next chase (retail leaves the bit, b108).
                b.target_unreachable = false;
                // Steering speed 0 (`vfn176 -> vfn168(0.0)`): the intercept stops.
                if b.motion_intent == Some(motion::INTERCEPT) {
                    b.motion_intent = None;
                }
            }
            PedOp::RunFromHonker => b.motion_intent = Some(motion::RUN_FROM_HONKER),
            PedOp::TakedownTauntVictim => {
                if let Some(target) = b.wants.get("taunt").map(|w| w.target) {
                    b.face = (self.target_position)(target);
                    b.monitored.insert("SGIntent".to_string(), Packet::new(vec!["SGIntent".to_string()]));
                    b.chase_requests.push(ChaseRequest::Taunt { target });
                }
            }
            PedOp::Flee => {
                b.motion_intent = Some(motion::FLEE);
                b.flee_from = b.wants.get("flee").map(|w| w.target);
            }
            PedOp::StopAndFaceWantTarget { .. } | PedOp::WatchWantTarget { .. } => {
                b.saved_speed = Some(b.speed_suggestion);
                b.watch_latched = false;
            }
            PedOp::StandAndWatchSkater { .. } => {
                let f = [self.heading.sin(), self.heading.cos()];
                self.brain.watch_point = Some([self.position[0] + f[0] * s.watch_ahead, self.position[1], self.position[2] + f[1] * s.watch_ahead]);
            }
            _ => {}
        }
    }
    fn update(&mut self, behavior: BehaviorId, _context: [u32; 6], _frame: &Frame) {
        let Some(op) = self.behaviors.get(behavior) else { return };
        let s = *self.settings;
        let b = &mut *self.brain;
        match op {
            PedOp::Wander => b.speed_suggestion = Some(s.wander_speed),
            PedOp::DisableNavigation => b.navigation_disabled = true,
            PedOp::ZombieFollow => {
                if let Some((player, _)) = self.skater {
                    let z = s.zombie_follow;
                    let flat = |a: Vec3, c: Vec3| ((a[0] - c[0]).powi(2) + (a[2] - c[2]).powi(2)).sqrt();
                    let d = flat(self.position, player);
                    let reached = b.zombie_goal.is_none_or(|g| (0..3).map(|i| (self.position[i] - g[i]).powi(2)).sum::<f32>() <= z.arrive_distance * z.arrive_distance);
                    if d > z.follow_distance {
                        b.zombie_goal = Some(player);
                    } else if reached {
                        b.zombie_goal = Some(super::hand_prop::jitter(b.rng(), player, z.ring_min, z.ring_max));
                    }
                    b.speed_suggestion = Some(if d > z.sprint_distance { z.sprint_speed } else { z.walk_speed });
                }
            }
            PedOp::ThrowHandPropAtWantTarget { want } => {
                if !b.hand_prop.holding && b.timer(super::hand_prop::THROW_REACTION_TIMER) <= 0.0 {
                    b.unset_want(want);
                }
            }
            PedOp::StartChase => b.speech = Some(s.start_chase_speech),
            PedOp::ChaserEvaluateWhoToTakeDown => b.takedown_target = b.chasee,
            PedOp::TargetWaypoint { speed, slide_distance, slide_speed } => {
                if let Some(w) = b.waypoint {
                    b.approach = Some((w, *speed));
                    b.approach_slide = (*slide_distance > 0.0).then_some((*slide_distance, *slide_speed));
                    b.speed_suggestion = Some(*speed);
                }
            }
            // `826A0DC8`: also the locomotion's explicit turn to the waypoint's orientation (`+2080`, `+2100` bit 0x20).
            PedOp::TurnToFaceWaypointOrientation => {
                b.face = self.chase.conversation.map(|c| c.center);
                if b.waypoint.is_some() && b.waypoint_facing.is_some() {
                    b.explicit_turn = b.waypoint_facing;
                }
            }
            PedOp::ConversationSpeak => {
                if !b.turn_passed && b.timer(SPEAK_TIMER) <= 0.0 {
                    b.turn_passed = true;
                    b.chase_requests.push(ChaseRequest::PassTurn);
                }
            }
            // Ours: a head look-at (no body turn: the body keeps the waypoint orientation, which
            // the TurnToFace state above it checks every tick).
            PedOp::ConversationListenToSpeaker => {
                self.brain.listen_to = self.chase.conversation.and_then(|c| c.speaker);
            }
            // `826A8258` update: once timer 37 runs out the "tazr" prop is requested (`82E3DDA0`) if the hand was empty
            // at the draw. "Drawn" (state 2) comes from the draw clip, [`PedBrain::update_tazer_clip`].
            PedOp::DrawTazer { .. } => {
                if b.tazer_state == Some(1) && b.timer(37) <= 0.0 && b.tazer_hand_was_empty && !b.tazer_prop_requested {
                    b.tazer_prop_requested = true;
                    b.hand_prop.request(TAZER_PROP);
                }
            }
            // `826A8420`: once timer 39 is out, the resolved target is hit once (`brain+3280` bit 0x20); from then on,
            // every update while the target's chase group has chasers ends it with aggressivecapture (group vfn24(1)).
            PedOp::TazeWantTarget { want } => {
                if b.timer(39) <= 0.0 {
                    if let Some(target) = b.wants.get(want).map(|w| w.target).filter(|t| (self.target_position)(*t).is_some()) {
                        if !b.tazer_hit {
                            b.tazer_hit = true;
                            b.chase_requests.push(ChaseRequest::Taze { target });
                        }
                        let capture = super::chase::reason::AGGRESSIVE_CAPTURE;
                        if self.chase.group(target).is_some_and(|g| g.count > 0 && g.end_reason != Some(capture)) {
                            b.chase_requests.push(ChaseRequest::GroupEnd { chasee: target, reason: capture });
                        }
                    }
                }
            }
            PedOp::KnowAboutChasee => {
                if let Some(c) = b.chasee {
                    b.perceptions.know(c, s.know_about_seconds);
                }
            }
            PedOp::LostChasee => b.search_point = b.chasee.and_then(|c| b.perceptions.get(c)).map(|e| e.position),
            PedOp::SetAltTargetToChaseePosition => {
                let c = self.brain.chasee;
                self.brain.alt_target = c.and_then(|c| (self.target_position)(c));
            }
            // `826A3BF0` adds the tick to the exhaustion (chaser vfn156).
            PedOp::InterceptChasee | PedOp::PursueChasee => b.chase_exhaustion += _frame.dt,
            PedOp::UpdateBlockPrediction => {
                let c = self.brain.chasee;
                let r = c.and_then(|c| self.chase.block.and_then(|f| f(c)));
                self.brain.should_block = r.is_some_and(|r| r.0);
                self.brain.block_point = r.filter(|r| r.0).map(|r| r.1);
            }
            PedOp::BlockChasee { at_target, slow_speed } => {
                let chasee = self.brain.chasee.and_then(|c| (self.target_position)(c));
                let at_point = self.brain.block_point.is_some_and(|p| {
                    let d = [p[0] - self.position[0], p[1] - self.position[1], p[2] - self.position[2]];
                    d[0] * d[0] + d[1] * d[1] + d[2] * d[2] < at_target * at_target
                });
                let b = &mut *self.brain;
                if at_point {
                    b.face = chasee;
                    b.speed_suggestion = Some(0.0);
                    b.chase_exhaustion = (b.chase_exhaustion - _frame.dt).max(0.0);
                } else {
                    b.face = None;
                    b.speed_suggestion = Some(*slow_speed);
                }
            }
            PedOp::TakeDownTargetablePredictions => {
                let target = self.brain.takedown_target;
                self.brain.takedown_choice = target.and_then(|t| self.chase.takedowns.and_then(|f| f(t)));
            }
            PedOp::AttemptTakeDownTargetable => {
                // `826A4BE8`: a contact by anyone but the target blocks the decision (the 3 s timer then fails it).
                // Ours looks at any stored contact; retail at the last toucher (`brain+0xC70`) [approximation].
                if b.takedown_active && b.takedown_result.is_none() && b.takedown_contact && b.pending_reaction.is_none() {
                    b.takedown_result = Some(2);
                }
            }
            PedOp::ChannelGreetWantTarget { want, deactivate, return_greet } => {
                b.speech = Some(if *return_greet { s.return_greet_speech } else { s.greet_speech });
                if *deactivate && b.timer(timers::WARN) <= 0.0 {
                    b.unset_want(want);
                }
            }
            PedOp::ApproachWantTarget { want, speed } => {
                let t = self.want_target(want);
                self.brain.approach = t.map(|p| (p, *speed));
                if t.is_some() {
                    self.brain.speed_suggestion = Some(*speed);
                }
            }
            PedOp::ChannelWarnWantTarget { want, deactivate: true } => {
                if b.timers.get(&timers::WARN).is_none_or(|t| *t <= 0.0) {
                    b.unset_want(want);
                }
            }
            PedOp::StopAndFaceWantTarget { want } => {
                let t = self.want_target(want);
                self.brain.face = t;
                self.brain.speed_suggestion = Some(0.0);
            }
            PedOp::WatchWantTarget { want } => {
                let t = self.want_target(want);
                let inside = t.and_then(|p| self.flat_dir(p)).is_some_and(|d| {
                    let f = self.forward();
                    (f[0] * d[0] + f[1] * d[1]).clamp(-1.0, 1.0).acos() <= s.watch_cone
                });
                if !inside {
                    self.brain.watch_latched = true;
                }
                if self.brain.watch_latched {
                    self.brain.face = t;
                    self.brain.speed_suggestion = Some(0.0);
                } else {
                    self.brain.speed_suggestion = self.brain.saved_speed.flatten();
                }
            }
            PedOp::TurnToFaceSkater => {
                self.brain.face = self.skater.map(|(p, _)| p);
            }
            PedOp::StandAndWatchSkater { max_angle, predict_time } => {
                self.brain.speed_suggestion = Some(0.0);
                if let (Some((sp, sv)), Some(w)) = (self.skater, self.brain.watch_point) {
                    if let (Some(dw), Some(ds)) = (self.flat_dir(w), self.flat_dir(sp)) {
                        if dw[0] * ds[0] + dw[1] * ds[1] < *max_angle {
                            self.brain.watch_point = Some([sp[0] + sv[0] * predict_time, sp[1] + sv[1] * predict_time, sp[2] + sv[2] * predict_time]);
                        }
                    }
                }
                self.brain.face = self.brain.watch_point;
            }
            _ => {}
        }
    }
    fn end(&mut self, behavior: BehaviorId, _context: [u32; 6], _frame: &Frame) {
        let Some(op) = self.behaviors.get(behavior) else { return };
        let b = &mut *self.brain;
        match op {
            PedOp::PedestrianColliding => {
                b.begin_colliding = false;
                b.colliding = false;
            }
            PedOp::OwnPluginObject => b.own_plugin_object = false,
            PedOp::ZombieFollow => b.zombie_goal = None,
            PedOp::OverrideAnimData { .. } => b.anim_override = None,
            PedOp::DisallowHandPropActions => b.hand_prop_actions_disallowed = false,
            PedOp::DisableCollisionsWithBehaviourSource => {
                b.own_plugin_object = false;
                b.ignore_source_collision = false;
            }
            // `826A2770`: the Create behaviour's End removes its packet.
            PedOp::CreateSimpleMonitoredIntent { intent, .. } => {
                b.monitored.remove(intent);
            }
            PedOp::DisableHeavyCollision => b.heavy_collision_disabled = false,
            PedOp::DisableCollisionSliding => b.collision_sliding_disabled = false,
            PedOp::DisableNavigation => b.navigation_disabled = false,
            PedOp::DisableAllMoods => b.moods_disabled = b.moods_saved.remove(&behavior).unwrap_or(false),
            PedOp::OverridePluginCollision => b.plugin_collision_override = false,
            PedOp::OverridePluginAvoid => b.plugin_avoid_override = false,
            PedOp::IgnoreStandingCollisions => b.ignore_standing_collisions = false,
            PedOp::SetExplicitTurnDirectionToWaypointOrientation => b.explicit_turn = None,
            PedOp::SuggestVelocity { .. } => b.speed_suggestion = None,
            PedOp::UnsetWantOnEnd { want } => b.unset_want(want),
            PedOp::Flee => b.flee_from = None,
            PedOp::TakedownTauntVictim => {
                b.face = None;
                b.monitored.remove("SGIntent");
                b.unset_want("taunt");
            }
            PedOp::StopAndFaceWantTarget { .. } | PedOp::WatchWantTarget { .. } => {
                if let Some(saved) = b.saved_speed.take() {
                    b.speed_suggestion = saved;
                }
                b.face = None;
                b.watch_latched = false;
            }
            PedOp::TurnToFaceSkater => b.face = None,
            PedOp::AttemptTakeDownTargetable => b.takedown_active = false,
            PedOp::RestFromChase => b.chase_resting = false,
            PedOp::LostChasee => b.search_point = None,
            PedOp::UnsetIsReactingToMoodEventFlagOnEnd => b.reacting_to_mood = false,
            // Plugin End (8269F310 -> 82E0C9E8) releases the plugin, so the ped goes on to Wander (b318).
            PedOp::Plugin => {
                b.in_plugin = false;
                b.chase_requests.push(ChaseRequest::ExitPlugin);
            }
            PedOp::PedestrianInConversation => b.in_conversation = false,
            // `826A0F50` clears the explicit turn flag.
            PedOp::TurnToFaceWaypointOrientation => {
                b.face = None;
                b.explicit_turn = None;
            }
            PedOp::ConversationListenToSpeaker => b.listen_to = None,
            PedOp::AlertToWantTarget { .. } => {
                b.look_at.pop();
            }
            PedOp::UnregisterTazerOnEnd => b.tazer_registered = false,
            PedOp::LockToCurrentPosition => {
                b.position_locked = false;
                b.locked_at = None;
                b.speed_suggestion = None;
            }
            PedOp::BlockChasee { .. } => {
                b.face = None;
                b.speed_suggestion = None;
            }
            PedOp::SetAltTargetToChaseePosition => b.alt_target = None,
            PedOp::EndInvestigateTimer => b.set_timer(INVESTIGATE_TIMER, 0.0),
            PedOp::TakedownTargetableClearOnEnd => {
                b.takedown_target = None;
                b.takedown_choice = None;
            }
            PedOp::ChangeNavModifierSetting { modifier, .. } => match b.nav_saved.remove(&behavior) {
                Some(Some(v)) => {
                    b.nav_modifiers.insert(*modifier, v);
                }
                _ => {
                    b.nav_modifiers.remove(modifier);
                }
            },
            PedOp::StandAndWatchSkater { .. } => {
                b.face = None;
                b.watch_point = None;
                b.speed_suggestion = None;
            }
            _ => {}
        }
    }
    fn hook(&mut self, _hook: HookId, _frame: &Frame) {}
    fn release(&mut self, _instance: u32) {}
}

#[cfg(test)]
#[path = "brain_tests.rs"]
mod tests;
