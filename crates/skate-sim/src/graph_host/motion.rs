#[path = "motion_toggle_execute.rs"]
mod toggle_execution;
#[path = "motion_fakie_hold.rs"]
mod fakie_hold;
#[path = "motion_air_execute.rs"]
mod air_execution;
#[path = "motion_execute.rs"]
mod execution;
#[path = "motion_execute_leaf.rs"]
mod leaf_execution;
#[path = "motion_score_execute.rs"]
mod score_execution;
#[path = "motion_filter_execute.rs"]
mod filter_execution;
#[path = "motion_sliding_execute.rs"]
mod slide_execution;
#[path = "motion_wipeout_execute.rs"]
mod wipeout_execution;
#[path = "motion_offboard/body_tweak.rs"]
pub(crate) mod body_tweak;
#[path = "motion_offboard/match_air_time.rs"]
pub(crate) mod match_air_time;
// Persistent host called by the production stock MotionGraph controller.
pub use super::motion_animation::MotionAnimation;
use super::{
    motion_nodes::{MotionFactory, MotionOperation},
    pushing::{PushContext, PushInstance},
    pushing_settings::PushingSettings,
};
use crate::graph_runtime::{LoadedGraph, OperationRemap};
use skate_core::graph::intents::IntentMap;
use skate_core::{
    animation::{
        playback::{PlayAnimationInstance, PlaybackContext},
        playback_parameters::{AttributeSink, ParameterInputs, SettableAttribute},
        skeleton_input::name::encode,
    },
    graph::{
        activation::ConditionHost,
        conditions::ConditionInputs,
        controller::{BehaviorId, Frame, HookId, Host},
    },
    input::set_turning,
    riding::push_behaviors::{PushFootFrame, PushState},
};
use skate_data::collections::Collections;
use std::collections::BTreeMap;
use super::outputs::{ActionControls, GraphCapabilityReport, GraphDiagnostics, GraphEffects, MotionGraphInput, TurningOutput};

#[derive(Clone, Copy, Debug)]
pub struct MotionPhysical {
    pub turning: set_turning::Physical,
    pub stance: (bool, bool),
    pub forward_speed: f32,
    pub time_since_teleport: f32,
    pub is_switch: bool,
    pub foot_frame: Option<PushFootFrame>,
}
#[path = "motion_instances.rs"]
mod instances;
use instances::Instance;
pub struct MotionHost {
    pub animation: MotionAnimation,
    pub playback_context: PlaybackContext,
    pub condition_inputs: ConditionInputs,
    pub gameplay_conditions: Option<super::motion_gameplay_conditions::GameplayConditions>,
    pub manual_exit: Option<bool>,
    /// Completed Skeleton PhysOut536/540, in native yaw-then-pitch order.
    pub deck_yaw_pitch: Option<[f32; 2]>,
    pub riding_conditions: Option<super::motion_riding_conditions::RidingConditionInputs>,
    pub condition_random: super::motion_riding_conditions::MotionRandom,
    pub native_physical: Option<super::motion_native::Physical>,
    pub prelanding_physical: Option<super::motion_spin::PrelandingPhysical>,
    pub air_leg_physical: Option<skate_core::animation::air_leg_extension::Physical>,
    pub gesture_physical: Option<super::motion_native::GesturePhysical>,
    pub gesture_publication: Option<super::motion_character_gesture::GesturePublication>,
    pub toggle_board_physical: Option<super::motion_toggle_board::Physical>,
    pub runout_physical: Option<super::motion_runout::Observation>,
    pub grind_physical: Option<super::motion_grind::Physical>,
    pub grind_conditions: Option<super::motion_grind::conditions::Physical>,
    pub(super) grind_settings: super::motion_grind::Settings,
    pub shove_physical: Option<super::motion_shove::ShovePhysical>,
    pub hand_services: super::motion_hand_services::HandServices,
    pub slide_latch: set_turning::SlideLatch,
    ///SpecificCA4 bit26, getter8258F900/setter8258F910; reset clears it.
    pub is_power_sliding: bool,
    pub score_packet: super::motion_native::ScorePacket,
    pub(super) trick_requests: super::motion_tricks::Requests,
    pub(super) trick_height_settings: (bool, bool),
    pub action_controls: ActionControls,
    pub(crate) action_intents: IntentMap,
    /// Typed turning values transported from the ActionGraph boundary.
    pub turning_output: TurningOutput,
    pub graph_effects: GraphEffects,
    pub time_tags: Option<BTreeMap<String, f32>>,
    pub physical: Option<MotionPhysical>,
    /// Completed Biped/OffBoard publications. These remain absent until the
    /// corresponding physical owner is integrated; graph leaves must not
    /// invent replacement values.
    pub offboard_cadence_phase: Option<f32>,
    pub offboard_locomotion_state: Option<u32>,
    pub ground_slope_type: Option<u32>,
    pub biped_ground_thin: Option<bool>,
    pub animation_phase: f32,
    pub moving_objects: super::motion_stock_gameplay::MovingObjectRegistry,
    pub push_state: Option<PushState>,
    pub applying_body_tilt: bool,
    pub crouching_physical: Option<skate_core::animation::crouching::Physical>,
    pub body_tilt_physical: Option<skate_core::animation::body_tilt::Physical>,
    pub hold_fakie: bool,
    automatic_fakie_conditions: Vec<bool>,
    pub fakie_physical: Option<skate_core::animation::riding_fakie::Physical>,
    /// PhysOutAnimation158/157, used directly by IsRidingGoofy82BA5AA8.
    pub physical_stance: Option<(bool, bool)>,
    pub flags: super::motion_landing::Flags,
    hippy_jump: super::motion_hippy_jump::Settings,
    finger_flip: super::motion_finger_flip::Settings,
    /// `anim_skitching/default` for the skitch nodes.
    pub(crate) skitching: super::motion_skitching::Settings,
    pub landing_physical: Option<super::motion_landing::Physical>,
    pub wipeout_physical: Option<super::motion_wipeout::Physical>,
    pub wipeout_controls: super::motion_wipeout::Controls,
    wipeout_settings: super::motion_wipeout::Settings,
    ///Native PhysOutGround pumping acceleration268.
    pub pumping_acceleration: Option<f32>,
    /// Completed PhysOutAnimation112 acceleration before bump conditioning.
    pub bump_acceleration: Option<[f32; 4]>,
    bump_settings: super::motion_bump::Settings,
    pub allow_pumping: bool,
    pub riding: super::motion_riding::RidingState,
    pub errors: GraphDiagnostics,
    pub(super) state_parents: Vec<Option<usize>>,
    operations: Vec<MotionOperation>,
    instances: Vec<Instance>,
    remap: OperationRemap,
    pushing: PushingSettings,
    turning: set_turning::Settings,
    crouching: skate_core::animation::crouching::Settings,
    body_tilt: skate_core::animation::body_tilt::Settings,
    pumping: skate_core::animation::pumping_channel::Settings,
    sliding: super::motion_sliding::Settings,
    pub(super) spin: super::motion_spin::Settings,
    air_leg: skate_core::animation::air_leg_extension::Settings,
    kickturn: skate_core::animation::kickturn::Settings,
    manual: skate_core::animation::manual::Settings,
    next_instance: u32,
}
/// The authored `UpdateRidingFakie` settings of the stock motion graph (every node, in graph
/// order): the retail fakie rule's thresholds, also used by the NPC replay puppet.
pub(crate) fn stock_riding_fakie_settings(graph: &LoadedGraph) -> Result<Vec<skate_core::animation::riding_fakie::Settings>, String> {
    Ok(graph
        .binding
        .instantiate_operations(&graph.source, &mut MotionFactory)
        .map_err(|e| e.to_string())?
        .operations
        .into_iter()
        .filter_map(|o| match o {
            MotionOperation::UpdateRidingFakie(s) => Some(s),
            _ => None,
        })
        .collect())
}

/// The trees the stock motion graph plays for one score name: docs/hails-additions/26c-living-world-npc-skaters.md#score-clips
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct ScoreClips {
    pub into: Option<String>,
    /// The `Into` / `Out` state's `playBackSpeed` (`None` = 1).
    pub into_speed: Option<f32>,
    pub out_speed: Option<f32>,
    /// A ground grab's `<X>Cyc` blend time (`PlayAnimation time`): the hold an air grab lands into.
    pub cyc_seconds: Option<f32>,
    /// The `Out` state's blend time (`PlayAnimation time`).
    pub out_seconds: Option<f32>,
    /// The Out leaves for InAir this long before its clip ends (`WillExpire InTime`).
    pub out_expire: Option<f32>,
    /// The grab landing a Cyc still active at touchdown plays (`Land.GrabLanding.<X>`).
    pub landing: Option<GrabLanding>,
    pub cyc: String,
    pub out: Option<String>,
    /// The `ScoringGrabs` direction attribute that names it (`up` / `left` / `down` / `right`); `None` = the base name.
    pub direction: Option<&'static str>,
    /// The `Cyc` state's tweak easing, when it has one (air grabs).
    pub tweak: Option<TweakFilter>,
}

/// An air grab `Cyc`'s `FilterMotionGraphIntent` TweakX / TweakY and whether `TweakProject` (abs(x) + abs(y) <= 1)
/// or a raw `SetBlend` writes `tweak_x` / `tweak_y`: docs/hails-additions/26c-living-world-npc-skaters.md#npc-grab-tweaks
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TweakFilter {
    pub x: skate_core::animation::intent_filter::Settings,
    pub y: skate_core::animation::intent_filter::Settings,
    pub project: bool,
    /// Cyc -> Out: the filtered tweak below this (`HasTweak less`, the x / y magnitude; or `HasFilteredMotionGraphIntent`
    /// TweakY `abs less`, `y_only`).
    pub release: Option<Release>,
}

/// `Land.GrabLanding.<X>`: its tree, blend time and how long before its end `Land` leaves for OnBoard (`WillExpire`).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct GrabLanding {
    pub tree: String,
    pub seconds: f32,
    pub expire: f32,
}

/// The tweak level an air grab's Cyc leaves for its Out at (research b708).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Release {
    pub below: f32,
    pub y_only: bool,
}

/// Every `ScoringGrabs` / `ScoringTrick` name of a motion graph (lower case) -> the trees of the state that scores it
/// (first in graph order wins, a transition state last), so a graph mod changes NPC grabs like the player's: docs/hails-additions/26c-living-world-npc-skaters.md#score-clips
pub(crate) fn stock_score_clips(graph: &skate_data::state_graph::StateGraph) -> BTreeMap<String, ScoreClips> {
    let e = &graph.elements;
    let mut parent = vec![usize::MAX; e.len()];
    for (i, el) in e.iter().enumerate() {
        for &c in &el.children {
            parent[c] = i;
        }
    }
    let attr = |i: usize, n: &str| e[i].attributes.iter().find(|a| a.name.eq_ignore_ascii_case(n)).map(|a| a.text.as_str());
    let is_state = |i: usize| e[i].tag.eq_ignore_ascii_case("state");
    let is_behaviour = |i: usize, n: &str| e[i].tag.eq_ignore_ascii_case("behaviour") && attr(i, "name").is_some_and(|v| v.eq_ignore_ascii_case(n));
    let ends = |i: usize, suffix: &str| attr(i, "name").is_some_and(|n| n.len() >= suffix.len() && n[n.len() - suffix.len()..].eq_ignore_ascii_case(suffix));
    // A state's own PlayAnimation, else the first one of its sub-states (a cycle split by speed).
    let anim = |s: usize| -> Option<String> {
        let direct = e[s].children.iter().find(|&&c| is_behaviour(c, "PlayAnimation")).and_then(|&c| attr(c, "anim"));
        direct.or_else(|| (s + 1..e.len()).take_while(|&i| is_inside(&parent, s, i)).find(|&i| is_behaviour(i, "PlayAnimation")).and_then(|i| attr(i, "anim"))).map(str::to_owned)
    };
    let cycle = |i: usize| ends(i, "Cyc") || ends(i, "Cycle");
    let phase = |p: usize, suffix: &str| e[p].children.iter().copied().find(|&c| is_state(c) && ends(c, suffix)).and_then(anim);
    let cyc_seconds = |p: usize| {
        let s = e[p].children.iter().copied().find(|&c| is_state(c) && cycle(c))?;
        let play = e[s].children.iter().copied().find(|&c| is_behaviour(c, "PlayAnimation"))?;
        attr(play, "time")?.trim().parse::<f32>().ok()
    };
    let speed = |p: usize, suffix: &str| {
        let s = e[p].children.iter().copied().find(|&c| is_state(c) && ends(c, suffix))?;
        let play = e[s].children.iter().copied().find(|&c| is_behaviour(c, "PlayAnimation"))?;
        attr(play, "playBackSpeed")?.trim().parse::<f32>().ok().filter(|v| v.is_finite() && *v > 0.0 && *v != 1.0)
    };
    let cyc_phase = |p: usize| e[p].children.iter().copied().find(|&c| is_state(c) && cycle(c)).and_then(anim);
    let filter = |s: usize, intent: &str| {
        let c = e[s].children.iter().copied().find(|&c| is_behaviour(c, "FilterMotionGraphIntent") && attr(c, "filteredIntent").is_some_and(|v| v.eq_ignore_ascii_case(intent)))?;
        Some(super::motion_intent_filter::Operation::parse(&skate_data::state_graph::attributes::Attributes::new(&e[c].attributes)).settings)
    };
    let release = |s: usize| {
        let t = e[s].children.iter().copied().find(|&c| e[c].tag.eq_ignore_ascii_case("transition") && attr(c, "target").is_some_and(|v| v.eq_ignore_ascii_case("Out")))?;
        let is_condition = |i: usize, n: &str| e[i].tag.eq_ignore_ascii_case("condition") && attr(i, "name").is_some_and(|v| v.eq_ignore_ascii_case(n));
        (t + 1..e.len()).take_while(|&i| is_inside(&parent, t, i)).find_map(|i| {
            let y_only = if is_condition(i, "HasTweak") {
                false
            } else if is_condition(i, "HasFilteredMotionGraphIntent") && attr(i, "intent").is_some_and(|v| v.eq_ignore_ascii_case("TweakY")) {
                true
            } else {
                return None;
            };
            Some(Release { below: attr(i, "less")?.trim().parse().ok()?, y_only })
        })
    };
    let tweak = |s: usize| Some(TweakFilter { x: filter(s, "TweakX")?, y: filter(s, "TweakY")?, project: e[s].children.iter().any(|&c| is_behaviour(c, "TweakProject")), release: release(s) });
    // A state's transition to `target` (last path segment) that fires on `WillExpire`: its InTime.
    let expire = |s: usize, target: &str| {
        let t = e[s].children.iter().copied().find(|&c| e[c].tag.eq_ignore_ascii_case("transition") && attr(c, "target").is_some_and(|v| v.rsplit('.').next().is_some_and(|l| l.eq_ignore_ascii_case(target))))?;
        (t + 1..e.len()).take_while(|&i| is_inside(&parent, t, i)).find(|&i| e[i].tag.eq_ignore_ascii_case("condition") && attr(i, "name").is_some_and(|v| v.eq_ignore_ascii_case("WillExpire"))).and_then(|i| attr(i, "InTime")?.trim().parse::<f32>().ok())
    };
    let out_expire = |p: usize| expire(e[p].children.iter().copied().find(|&c| is_state(c) && ends(c, "Out"))?, "InAir");
    let name_is = |i: usize, n: &str| attr(i, "name").is_some_and(|v| v.eq_ignore_ascii_case(n));
    // The Cyc's `...GrabLanding.<X>` target: the state X under a `GrabLanding` state, and its parent's (`Land`) OnBoard exit.
    let landing = |s: usize| {
        let target = e[s].children.iter().copied().filter(|&c| e[c].tag.eq_ignore_ascii_case("transition")).filter_map(|c| attr(c, "target")).find(|t| t.split('.').rev().nth(1).is_some_and(|g| g.eq_ignore_ascii_case("GrabLanding")))?;
        let x = target.rsplit('.').next()?;
        let st = (0..e.len()).find(|&i| is_state(i) && name_is(i, x) && parent[i] != usize::MAX && is_state(parent[i]) && name_is(parent[i], "GrabLanding"))?;
        let play = e[st].children.iter().copied().find(|&c| is_behaviour(c, "PlayAnimation"))?;
        let land = parent[parent[st]];
        Some(GrabLanding { tree: attr(play, "anim")?.to_owned(), seconds: attr(play, "time")?.trim().parse().ok()?, expire: (land != usize::MAX).then(|| expire(land, "OnBoard")).flatten()? })
    };
    let out_seconds = |p: usize| {
        let s = e[p].children.iter().copied().find(|&c| is_state(c) && ends(c, "Out"))?;
        let play = e[s].children.iter().copied().find(|&c| is_behaviour(c, "PlayAnimation"))?;
        attr(play, "time")?.trim().parse::<f32>().ok()
    };
    let mut out = BTreeMap::new();
    let mut weak = std::collections::BTreeSet::new();
    for i in 0..e.len() {
        let names: Vec<(&str, Option<&'static str>)> = if is_behaviour(i, "ScoringGrabs") {
            [("grabName", None), ("up", Some("up")), ("left", Some("left")), ("down", Some("down")), ("right", Some("right"))]
                .into_iter()
                .filter_map(|(a, d)| attr(i, a).map(|n| (n, d)))
                .collect()
        } else if is_behaviour(i, "ScoringTrick") {
            attr(i, "trick").map(|n| (n, None)).into_iter().collect()
        } else {
            continue;
        };
        let s = parent[i];
        if s == usize::MAX {
            continue;
        }
        // Grinds, slides: the scoring state's own tree; air grabs: that state is the `Cyc` between `Into` and `Out`;
        // ground grabs: no own tree, it holds `<X>Into` / `<X>Cyc` / `<X>Out`.
        let own = e[s].children.iter().find(|&&c| is_behaviour(c, "PlayAnimation")).and_then(|&c| attr(c, "anim")).map(str::to_owned);
        let clips = match own {
            Some(cyc) if cycle(s) && parent[s] != usize::MAX => Some(ScoreClips { into: phase(parent[s], "Into"), into_speed: speed(parent[s], "Into"), cyc, out: phase(parent[s], "Out"), out_speed: speed(parent[s], "Out"), cyc_seconds: None, out_seconds: out_seconds(parent[s]), out_expire: out_expire(parent[s]), landing: landing(s), direction: None, tweak: tweak(s) }),
            Some(cyc) => Some(ScoreClips { cyc, ..Default::default() }),
            None if e[s].children.iter().any(|&c| is_state(c) && cycle(c)) => cyc_phase(s).map(|cyc| ScoreClips { into: phase(s, "Into"), into_speed: speed(s, "Into"), cyc, out: phase(s, "Out"), out_speed: speed(s, "Out"), cyc_seconds: cyc_seconds(s), out_seconds: out_seconds(s), out_expire: None, landing: None, direction: None, tweak: None }),
            None => anim(s).map(|cyc| ScoreClips { cyc, ..Default::default() }),
        };
        let Some(clips) = clips else { continue };
        // A grind's grab `Into` also scores the grind: a transition state names a score only when no other does.
        let transition = ends(s, "Into") || ends(s, "Out");
        for (name, direction) in names {
            let key = name.to_ascii_lowercase();
            let free = !out.contains_key(&key);
            if transition && free {
                weak.insert(key.clone());
            }
            if free || (!transition && weak.remove(&key)) {
                out.insert(key, ScoreClips { direction, ..clips.clone() });
            }
        }
    }
    out
}

/// Whether element `i` lies under `root` (walks `i`'s parents).
fn is_inside(parent: &[usize], root: usize, mut i: usize) -> bool {
    while i != usize::MAX {
        if i == root {
            return true;
        }
        i = parent[i];
    }
    false
}

/// A motion animation over stock metadata alone (no graph), for building stock trees outside the
/// player's host (tests, the NPC puppet's channel).
pub(crate) fn metadata_animation(metadata: skate_data::animation_metadata::AnimationMetadata) -> super::motion_animation::MotionAnimation {
    super::motion_animation::MotionAnimation::from_metadata(metadata)
}

/// The pose commands of the stock tree `name` (built like a channel's tree,
/// `MotionAnimation::build_tree`) with `attributes` set (e.g. the fakie channel's `torso`) at
/// `time` s (wrapped by the tree's length: channel trees loop).
pub(crate) fn tree_commands(
    animation: &super::motion_animation::MotionAnimation,
    name: &str,
    attributes: &[SettableAttribute],
    time: f32,
) -> Result<Vec<skate_core::animation::playback_tree::PoseCommand>, String> {
    let mut out = Vec::new();
    tree_commands_into(animation, name, attributes, time, &mut out)?;
    Ok(out)
}

/// [`tree_commands`] into `out` (cleared first).
pub(crate) fn tree_commands_into(
    animation: &super::motion_animation::MotionAnimation,
    name: &str,
    attributes: &[SettableAttribute],
    time: f32,
    out: &mut Vec<skate_core::animation::playback_tree::PoseCommand>,
) -> Result<(), String> {
    out.clear();
    let mut tree = animation.build_tree(name)?;
    tree.set_attributes(attributes)?;
    let length = tree.length();
    tree.set_time(if length > 0.0 { time.max(0.0).rem_euclid(length) } else { 0.0 });
    tree.evaluate(skate_core::animation::playback_tree::Evaluation { cull_threshold: 0.0, update_history: false }, true, out)?;
    Ok(())
}

/// A built stock tree kept for [`cached_tree_commands_into`] with the inputs it was built and set with.
pub(crate) struct TreeCache {
    name: String,
    construction: Vec<(skate_core::animation::output::attributes::AttributeName, skate_core::animation::output::attributes::AttributeName)>,
    attributes: Vec<SettableAttribute>,
    tree: skate_core::animation::playback_tree::PlaybackTree,
}

/// [`tree_commands_into`] on a reused tree: same output while the name, construction values and
/// attributes match (docs/hails-additions/34-physics-performance.md "Fakie channel tree cache").
/// Drop the cache when `animation` is replaced.
pub(crate) fn cached_tree_commands_into(
    animation: &super::motion_animation::MotionAnimation,
    cache: &mut Option<TreeCache>,
    name: &str,
    attributes: &[SettableAttribute],
    time: f32,
    out: &mut Vec<skate_core::animation::playback_tree::PoseCommand>,
) -> Result<(), String> {
    out.clear();
    let hit = cache.as_ref().is_some_and(|c| c.name == name && c.construction == animation.construction_values && c.attributes == attributes);
    if !hit {
        *cache = None;
        let mut tree = animation.build_tree(name)?;
        tree.set_attributes(attributes)?;
        *cache = Some(TreeCache { name: name.to_owned(), construction: animation.construction_values.clone(), attributes: attributes.to_vec(), tree });
    }
    let c = cache.as_mut().expect("cache filled");
    let result = (|| {
        if hit {
            c.tree.set_attributes(attributes)?;
        }
        let length = c.tree.length();
        c.tree.set_time(if length > 0.0 { time.max(0.0).rem_euclid(length) } else { 0.0 });
        c.tree.evaluate(skate_core::animation::playback_tree::Evaluation { cull_threshold: 0.0, update_history: false }, true, out).map(|_| ())
    })();
    if result.is_err() {
        *cache = None;
    }
    result
}

/// [`tree_commands_into`] on one reused tree per name (attributes set every call, for values that move each frame
/// like an NPC grab's tweaks). Clear `caches` when `animation` is replaced.
pub(crate) fn named_tree_commands_into(
    animation: &super::motion_animation::MotionAnimation,
    caches: &mut Vec<TreeCache>,
    name: &str,
    attributes: &[SettableAttribute],
    time: f32,
    out: &mut Vec<skate_core::animation::playback_tree::PoseCommand>,
) -> Result<(), String> {
    out.clear();
    caches.retain(|c| c.construction == animation.construction_values);
    let i = match caches.iter().position(|c| c.name == name) {
        Some(i) => i,
        None => {
            let tree = animation.build_tree(name)?;
            caches.push(TreeCache { name: name.to_owned(), construction: animation.construction_values.clone(), attributes: Vec::new(), tree });
            caches.len() - 1
        }
    };
    let c = &mut caches[i];
    let result = (|| {
        c.tree.set_attributes(attributes)?;
        let length = c.tree.length();
        c.tree.set_time(if length > 0.0 { time.max(0.0).rem_euclid(length) } else { 0.0 });
        c.tree.evaluate(skate_core::animation::playback_tree::Evaluation { cull_threshold: 0.0, update_history: false }, true, out).map(|_| ())
    })();
    if result.is_err() {
        caches.remove(i);
    }
    result
}

impl MotionHost {
    pub(super) fn end_gesture_channels(&mut self) {
        // EndGesture nodes share the graph-wide gesture channel owner. The
        // active CharacterGesture instance is found by its operation binding.
        for instance in &mut self.instances {
            if let Instance::CharacterGesture(state) = instance {
                state.end(&mut self.animation);
            }
        }
        self.gesture_publication = None;
    }

    /// The grind's rest board-to-CoG height with no crouch (`ControlGrindCrouch`): `max_grind_disttocog`.
    pub(crate) fn grind_rest_disttocog(&self) -> f32 {
        self.grind_settings.height[1]
    }

    /// `SetManualAngle`'s target for a held `Manual` intent (`anim_motion/manual` balance graph, signed).
    pub(crate) fn manual_rest_angle(&self, manual: f32) -> f32 {
        let t = self.manual.balance.evaluate(manual.abs());
        if manual >= 0.0 { t } else { -t }
    }

    pub fn from_graph(
        graph: &LoadedGraph,
        data: &Collections,
        mut metadata: skate_data::animation_metadata::AnimationMetadata,
        playback_context: PlaybackContext,
    ) -> Result<Self, String> {
        let mut operations = graph
            .binding
            .instantiate_operations(&graph.source, &mut MotionFactory)
            .map_err(|e| e.to_string())?
            .operations;
        super::motion_conditions::bind_states(graph, &mut operations);
        let instances = graph
            .runtime
            .operations
            .behaviors
            .iter()
            .map(|&id| Instance::new(&operations[id]))
            .collect();
        let mut capabilities = GraphCapabilityReport {
            supported_operations: operations
                .iter()
                .filter(|operation| !matches!(operation, MotionOperation::Unsupported { .. }))
                .count(),
            unsupported_operations: operations
                .iter()
                .filter_map(|operation| match operation {
                    MotionOperation::Unsupported { kind, name } => {
                        Some(format!("{:?} `{name}`", kind))
                    }
                    _ => None,
                })
                .collect(),
            ..GraphCapabilityReport::default()
        };
        for &operation in &graph.runtime.operations.conditions {
            if let Some(MotionOperation::Unsupported { kind, name }) = operations.get(operation) {
                capabilities
                    .unsupported_conditions
                    .push(format!("{kind:?} `{name}`"));
            }
        }
        for &operation in &graph.runtime.operations.hooks {
            if let Some(MotionOperation::Unsupported { kind, name }) = operations.get(operation) {
                capabilities.unsupported_hooks.push(format!("{kind:?} `{name}`"));
            }
        }
        // Preserve lazy diagnostics for optional unsupported graph branches.
        //8258F488 and final reset825953B0 explicitly zero the complete push
        //state; reset clears manualing and body-tilt flag bits too.
        let zero = skate_core::riding::push_animation::PushBlendParameters {
            hstr_vel_b: 0.0,
            lstr_vel_b: 0.0,
            vel_e: 0.0,
        };
        let push_state = Some(PushState {
            out_factor: 0.0,
            current_push_dv: 0.0,
            current: zero,
            target: zero,
            continue_push: false,
        });
        let pushing = PushingSettings::load(data, &mut metadata)?;
        Ok(Self {
            animation: MotionAnimation::from_metadata(metadata),
            playback_context,
            condition_inputs: ConditionInputs::default(),
            gameplay_conditions: None,
            manual_exit: None,
            deck_yaw_pitch: None,
            riding_conditions: None,
            condition_random: super::motion_riding_conditions::MotionRandom::new(),
            native_physical: None,
            prelanding_physical: None,
            air_leg_physical: None,
            gesture_physical: None,
            gesture_publication: None,
            toggle_board_physical: None,
            runout_physical: None,
            grind_physical: None,
            grind_conditions: None,
            grind_settings: super::motion_grind::Settings::load(data)?,
            shove_physical: None,
            hand_services: Default::default(),
            slide_latch: set_turning::SlideLatch::default(),
            is_power_sliding: false,
            score_packet: super::motion_native::ScorePacket::default(),
            trick_requests: Default::default(),
            trick_height_settings: (true, true),
            action_controls: ActionControls::default(),
            action_intents: IntentMap::new(),
            turning_output: TurningOutput::default(),
            graph_effects: GraphEffects::default(),
            time_tags: None,
            physical: None,
            offboard_cadence_phase: None,
            offboard_locomotion_state: None,
            ground_slope_type: None,
            biped_ground_thin: None,
            animation_phase: 0.0,
            moving_objects: Default::default(),
            push_state,
            applying_body_tilt: false,
            crouching_physical: None,
            body_tilt_physical: None,
            hold_fakie: false,
            automatic_fakie_conditions: fakie_hold::bind(&graph.binding),
            fakie_physical: None,
            physical_stance: None,
            flags: Default::default(),
            landing_physical: None,
            wipeout_physical: None,
            wipeout_controls: Default::default(),
            wipeout_settings: super::motion_wipeout::Settings::load(data)?,
            pumping_acceleration: None,
            bump_acceleration: None,
            bump_settings: super::motion_bump::Settings::load(data)?,
            //8258F488 seeds bit24, and reset825953B0 preserves that bit.
            allow_pumping: true,
            riding: super::motion_riding::RidingState::new(),
            errors: GraphDiagnostics::default(),
            state_parents: graph.binding.states.iter().map(|s| s.parent).collect(),
            operations,
            instances,
            remap: graph.runtime.operations.clone(),
            pushing,
            turning: super::turning_settings::load(data)?,
            crouching: super::crouching_settings::load(data)?,
            body_tilt: super::motion_riding::body_tilt_settings(data)?,
            pumping: super::pumping_settings::load(data)?,
            sliding: super::motion_sliding::Settings::load(data)?,
            spin: super::motion_spin::Settings::load(data)?,
            air_leg: super::motion_air_leg::load(data)?,
            kickturn: super::motion_kickturn::load_settings(data)?,
            manual: super::motion_manual::settings(data)?,
            hippy_jump: super::motion_hippy_jump::Settings::load(data)?,
            finger_flip: super::motion_finger_flip::Settings::load(data)?,
            skitching: super::motion_skitching::Settings::load(data),
            next_instance: 1,
        })
    }

    pub fn accept_action_graph(&mut self, input: MotionGraphInput) {
        let MotionGraphInput { tick, action } = input;
        debug_assert_eq!(tick, action.tick);
        self.errors.clear();
        self.turning_output = action.turning;
        self.graph_effects = action.effects.clone();
        self.action_controls = action.controls.clone();
        // Clone before handing the full output to MotionAnimation so the two
        // consumers observe one immutable packet.
        self.action_intents = action.controls.authored_values();
        self.animation.accept_action_graph(action);
    }
    fn run(&mut self, id: BehaviorId, frame: &Frame, phase: u8) {
        if let Err(error) = self.execute(id, frame, phase) {
            self.errors
                .push(format!("MotionGraph behavior {id}: {error}"));
        }
    }
}
impl ConditionHost for MotionHost {
    fn condition_activation(&mut self, condition: usize, frame: &Frame) -> u32 {
        // Gate only the authored automatic switch transitions. Other fakie
        // conditions (tricks, pushes, dismounts) still see the real stance.
        if self.remap.conditions.get(condition).is_some_and(|&id|
            fakie_hold::blocked(self.hold_fakie, &self.automatic_fakie_conditions, id)) {
            return 0;
        }
        let result = self
            .remap
            .conditions
            .get(condition)
            .and_then(|&id| self.operations.get(id))
            .ok_or_else(|| "Unbound MotionGraph condition".to_owned())
            .and_then(|op| match op {
                MotionOperation::Condition(condition) => condition.evaluate(self, frame),
                other => Err(format!("Unsupported MotionGraph condition {other:?}")),
            });
        match result {
            Ok(value) => u32::from(value),
            Err(error) => {
                self.errors.push(error);
                0
            }
        }
    }
}
impl Host for MotionHost {
    fn context(&self) -> [u32; 6] {
        [0; 6]
    }
    fn allocate(&mut self, behavior: BehaviorId, _frame: &Frame) -> u32 {
        if let Some(&id) = self.remap.behaviors.get(behavior) {
            self.instances[behavior] = Instance::new(&self.operations[id]);
        }
        let handle = self.next_instance;
        self.next_instance = self.next_instance.wrapping_add(1).max(1);
        handle
    }
    fn begin(&mut self, id: BehaviorId, _context: [u32; 6], frame: &Frame) {
        self.run(id, frame, 0);
    }
    fn update(&mut self, id: BehaviorId, _context: [u32; 6], frame: &Frame) {
        self.run(id, frame, 1);
    }
    fn end(&mut self, id: BehaviorId, _context: [u32; 6], frame: &Frame) {
        self.run(id, frame, 2);
    }
    fn hook(&mut self, hook: HookId, _frame: &Frame) {
        if let Some(MotionOperation::Hook(operation)) = self
            .remap
            .hooks
            .get(hook)
            .and_then(|&id| self.operations.get(id))
            .cloned()
        {
            match operation {

                super::motion_hooks::MotionHook::GrabSlide { right } => {
                    self.slide_latch.grab(right)
                }
                super::motion_hooks::MotionHook::Override(settings) => {
                    self.playback_context.transition_override = Some(settings)
                }
                super::motion_hooks::MotionHook::MongoPushToAntic { animation } => {
                    //82BBBBB8 passes the authored name for both channel/tree.
                    let settings = skate_core::animation::channel_playback::ChannelSettings {
                        priority: 0,
                        keep_alive: false,
                        mirrored: false,
                        speed: 1.0,
                        blend_in: f32::from_bits(0x3dcccccd),
                        hold_during_blend_in: false,
                        blend_out: f32::from_bits(0x3d8f5c29),
                        hold_during_blend_out: false,
                        use_attributes: false,
                    };
                    if let Err(error) = self.animation.new_channel(&animation, &animation, settings)
                    {
                        self.errors.push(error);
                    }
                }
            }
            return;
        }
        self.errors.push(format!(
            "Unsupported MotionGraph hook {:?}",
            self.remap
                .hooks
                .get(hook)
                .and_then(|&id| self.operations.get(id))
        ));
    }
    fn release(&mut self, _instance: u32) {}
}

#[cfg(test)]
#[path = "tests/motion_stock.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/motion_slide.rs"]
mod slide_tests;

#[cfg(test)]
#[path = "tests/motion_jump_into.rs"]
mod jump_into_tests;

#[cfg(test)]
#[path = "tests/motion_deck_angles.rs"]
mod deck_angles_tests;

#[cfg(test)]
#[path = "tests/motion_finger_flip.rs"]
mod finger_flip_tests;

#[cfg(test)]
#[path = "tests/motion_board_adjust.rs"]
mod board_adjust_tests;
