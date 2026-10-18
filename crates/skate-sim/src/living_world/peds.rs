//! Pedestrians, body tier: skate-game `living_world/peds.rs` (upstream PR #52 `618e824`) without
//! Bevy (looks, animation, navigation, obstacles, contacts, draw fade, behaviour, hand props).
//!
//! skate_vita: the functions are copied verbatim; the Bevy plumbing becomes plain data. A ped
//! entity (`Pedestrian`, `PedBody`, `PedMind`) is a [`PedEntity`] in [`PedState::index`]; the
//! systems are functions the living world calls in upstream's order. Data is read through
//! `skate_data::vfs`. The ped's pose is computed on the fixed step (upstream: every render frame)
//! for the peds in view, with its draw opacity and LOD from the camera. The behaviour runtime
//! (`think_peds`: the ped AI graph, mood, chase, conversations, plugins, takedowns) and the held hand
//! props ([`super::ped_hand_props`]) are taken too. Not taken: the audio (footsteps, speech), the
//! diagnostics logs (still, floating, blocked peds).
//!
//! Upstream's module docs follow.
//!
//! Pedestrians, body tier (doc 26, peds milestone M2): turns the population's pedestrian spawn
//! / despawn records into visible, animated peds with footstep audio.
//!
//! - **Look** ([`Pedestrian`]): the entity inside the census category and its model from the
//!   spawn record's seed (`skate_core::living_world::peds::choice`, retail `sub_826B8B88` /
//!   `sub_826BB058`), the model's tint pair (`sub_827B4170`), painted onto the body's mask
//!   texels by [`present_ped_tints`] (retail's ped shader rule, `peds::colorize`). GLB `private/living_world/models/
//!   <recipe>.glb` (both LODs, parts, textures), or a mod's GLB from [`PedLooks`].
//! - **Animation** ([`PedBody`]): the ped animation player on the clips of
//!   `PedestrianSkeletonPres.abin`, stepped once per population world tick (1/60 s, `clock::RETAIL_TICK_HZ`) so a ped's state is a
//!   function of its spawn record and the population tick. Root motion moves the ped; it is
//!   snapped to the ground below (a line query, like the audio's ground material probe).
//!   The intent comes from navigation (M3, below); on a map without a navmesh a ped follows
//!   [`TestPath`] (idle, walk a few metres, stop, turn round).
//! - **Navigation** (milestone M3): the district's NavPower navmesh (`private/living_world/
//!   navmesh.bin`, [`PedData::nav`]) and retail's `NoRoadWander` goal
//!   (`skate_core::living_world::peds::wander`: probe fans 40 m / 10 m, A* + funnel corners,
//!   re-target on arrival), ped-to-ped avoidance and separation, every step kept on walkable
//!   polygons. Retail ambient peds never use crosswalks (the road branch of `Pedestrian.xml` is
//!   unreachable in TU3); [`PedNavSettings::crosswalk`] = `WalkSignal` is a mod option that waits
//!   for the walk light of the shared traffic signal clock.
//! - **Dynamic obstacles** (fix 11, retail DynamicObject NavPower obstacles, see
//!   `skate_core::living_world::peds::obstacles`): [`PedObstacles`] holds every prop (the
//!   physics' prop bodies at their current pose, so a prop the player moved counts where it lies)
//!   and every mod body. At rest they are cut out of the walkable area (re-cut after moving more
//!   than a quarter of the smallest half extent, no cut while moving faster than 0.4 m/s or
//!   carried); targets inside a cut do not fit, paths bend round cuts, and a body is never stepped
//!   into ([`NavObstacles::resolve_step`]). Rules: [`LivingWorldSettings::ped_obstacles`]
//!   (`sdk.world.set_tuning("living_world", {ped_obstacles = {...}})`).
//! - **Skinning**: the GLB's 39 joints bind to the 50-bone rig by name (all match, data test);
//!   bones the clips do not carry (fingers, face) follow their rig parent with the GLB's bind
//!   offset. The skin matrix is `bone global x inverse(GLB bind)` with NO extra bone-local
//!   basis: the ped GLBs keep the retail model's bind frames, which are the rig's reference
//!   frames (data test `ped_glb_bind_frames_are_the_rig_reference_frames`), so the reference pose
//!   skins to the bind mesh. The skater's `render_basis` (its GLBs bake a matching basis into the
//!   bind) twisted every ped bone 90 degrees about its own axis (torso -90, legs +90 at the
//!   hips): the pinched waist / warped peds of fix10. [`ped_bone_basis`] picks the basis per
//!   model from its bind frames, so a mod GLB written the skater way still works.
//! - **Draw fade** (retail `sub_827C1188`, `skate_core::living_world::peds::fade`): opaque up to
//!   the model's first distance pair (45 m [data]) from the camera, gone at 55 m, plus a 1 s
//!   spawn fade in; the opacity goes into [`NpcFade::alpha`](super::npc_skaters::NpcFade) and is
//!   drawn by the shared NPC fade; at 0 the ped's scene is hidden. This is what keeps the census
//!   cull (70 m, a pure distance test) out of sight, as in retail.
//! - **LOD** (placeholder, the LOD pick is not decoded): `LOD0` within 45 m, `LOD1` beyond 55 m,
//!   hysteresis between; with the fade above `LOD1` only shows while fading.
//! - **Audio**: [`PedAudio`](crate::world_audio::PedAudio) with the model's voice, the clip's
//!   `LEFTTOEDOWN` / `RIGHTTOEDOWN` windows as `feet_down` and `BODYFALLTYPE` as `body_fall`, so
//!   #32's ped footsteps and body falls play.
//! - **Events** for engine systems and the planned `sdk.living_world`: [`PedEvent`].
//!
//! Moddability: [`PedLooks`] (category entity lists, entity -> model / animation set, recipe ->
//! GLB path) is the one place overrides go; restoring `PedLooks::default()` undoes a mod (new

use super::{LivingWorldSettings, PopulationState};
use glam::{Mat4, Quat, Vec3};
use skate_core::living_world::peds::anim::{PedClip, PedClips, TestPath};
use skate_core::living_world::peds::wander::{NoSignals, constrain_move, crosswalk_ok, separation_blocker};
use skate_core::living_world::peds::{CrosswalkRule, Locomotion, NavMesh, NavObstacles, NavRules, Neighbour, ObstacleInput, PedAnimPlayer, PedCatalog, PedEvaluator, PedNav, PedOverrides, PedRig, WalkSignals, WanderParams};
use skate_core::living_world::{Decision, DespawnReason, Kind, LivingWorldId, SpawnChoice};
use std::collections::BTreeMap;
use std::sync::Arc;

/// Seconds per population tick: the ped player steps once per world tick (retail's 60 Hz world
/// step, `clock::RETAIL_TICK_HZ`), so a ped's state is a function of its spawn record and the tick.
pub(crate) fn tick_seconds(hz: f64) -> f32 {
    (1.0 / hz.max(1.0)) as f32
}

/// Loaded ped data for the current world.
#[derive(Default)]
pub(crate) struct PedData {
    pub catalog: Arc<PedCatalog>,
    pub anim_sets: Arc<BTreeMap<String, skate_core::living_world::peds::PedAnimSet>>,
    pub rig: Arc<PedRig>,
    /// skate_vita (memory): the clips decoded on first use ([`ClipCache`]; upstream decoded every
    /// clip at load: 38 MB).
    pub clips: ClipCache,
    pub status: String,
    /// The district's navmesh (M3); `None` on maps without one (peds use [`TestPath`]).
    pub nav: Option<Arc<NavMesh>>,
    /// The navmesh records as loaded (rebuilt with new [`NavRules`] when a mod changes them).
    pub nav_input: Option<Arc<skate_core::living_world::peds::NavMeshInput>>,
    /// Signalled junction arms of the loaded roads (mod crosswalk rule).
    pub arms: Arc<Vec<(usize, u8, [f32; 3])>>,
    /// The stock ped AI graph on the shared graph runtime (behaviour runtime; `None` when the
    /// install has none).
    pub graph: Option<Arc<super::ped_graph::PedGraph>>,
    /// The stock mood tables and each entity type's reaction set (`ped_mood`).
    pub mood: Option<Arc<skate_core::living_world::peds::mood::MoodTables>>,
    pub reaction_sets: Arc<BTreeMap<String, String>>,
    /// Each entity type's chase record and the chase manager's `global` record (`ped_mood::chase_records`).
    pub chase: Arc<BTreeMap<String, skate_core::living_world::peds::chase::ChaseRecord>>,
    pub chase_global: Option<Arc<skate_core::living_world::peds::chase::ChaseRecord>>,
    /// The `zombie` chase record (zombie cheat: every chase group and every ped's own record, `ped_mood::ZOMBIE_KEY`).
    pub chase_zombie: Option<Arc<skate_core::living_world::peds::chase::ChaseRecord>>,
    /// The conversation plugin graph (`plugin/conversation.stategraph`) and the conversation tables.
    pub conversation_graph: Option<Arc<super::ped_graph::PedGraph>>,
    /// The world-prop plugin graphs by prop class (each descriptor's `file`, compiled).
    pub plugin_graphs: BTreeMap<String, Arc<super::ped_graph::PedGraph>>,
    pub conversations: Arc<super::ped_mood::ConversationTables>,
    /// Ped plugins on world props: classes, descriptors, `plugin_odds`, placed props (`ped_plugins`).
    pub plugins: Arc<super::ped_plugins::PluginData>,
    /// Hand props: records and models (`ped_hand_props`).
    pub hand_props: Arc<super::ped_hand_props::HandPropData>,
    /// Each entity type's starting hand prop chance and list (`ped_mood::starting_hand_props`).
    pub starting_props: Arc<BTreeMap<String, (f32, Vec<(String, f32)>)>>,
    /// The sit plugin's values by entity type (`ped_mood::sit_values`).
    pub sit: Arc<BTreeMap<String, skate_core::living_world::peds::brain::SitValues>>,
    /// Each entity type's vision test ranges (`ped_mood::sight`).
    pub sight: Arc<BTreeMap<String, skate_core::living_world::peds::perception::Sight>>,
    /// Each entity type's perception scales (near, far, fov) for the mood post's vision test.
    pub sight_scales: Arc<BTreeMap<String, [f32; 3]>>,
    /// Each entity type's takedown table (`ped_mood::takedown_tables`).
    pub takedowns: Arc<BTreeMap<String, skate_core::living_world::peds::takedown::TakedownTable>>,
    /// The `zombie` takedown table (zombie cheat, every ped).
    pub takedown_zombie: Option<Arc<skate_core::living_world::peds::takedown::TakedownTable>>,
    loaded_for: Option<(String, u64)>,
}


impl PedData {
    /// The behaviour part of upstream's `load_ped_data` (the graphs, the mood, chase, takedown,
    /// sight, conversation and sit tables, the plugins and the hand prop records), verbatim but for the reads
    /// (`skate_data::vfs`; the peds' classes in `ped_tables.json`) and the map generation (one load per world).
    pub(crate) fn load_behaviour(&mut self, asset_root: &std::path::Path, map: &str) {
        let loaded = self;
        match super::ped_graph::PedGraph::load(asset_root, super::ped_graph::AI_GRAPH) {
            Ok(graph) => {
                let pending = graph.pending();
                info!(
                    "PED_GRAPH loaded: {} behaviours, {} conditions; {} operation names not ported yet: {}",
                    graph.behaviors.len(),
                    graph.conditions.len(),
                    pending.len(),
                    pending.keys().cloned().collect::<Vec<_>>().join(", ")
                );
                loaded.graph = Some(Arc::new(graph));
                match super::ped_graph::PedGraph::load(asset_root, super::ped_graph::CONVERSATION_GRAPH) {
                    Ok(g) => {
                        info!("PED_GRAPH conversation plugin: {} behaviours, {} conditions; not ported yet: {:?}", g.behaviors.len(), g.conditions.len(), g.pending().keys().collect::<Vec<_>>());
                        loaded.conversation_graph = Some(Arc::new(g));
                    }
                    Err(error) => warn!("PED_GRAPH conversation plugin not loaded (no conversations): {error}"),
                }
            }
            Err(error) => warn!("PED_GRAPH not loaded (peds keep wandering without the behaviour graph): {error}"),
        }
        let dir = asset_root.join("private/living_world");
        let tables = skate_data::vfs::read(&dir.join("ped_tables.json")).or_else(|_| skate_data::vfs::read(&dir.join("tables.json"))).map_err(|e| e.to_string());
        match tables.and_then(|b| Ok((super::ped_mood::parse(&b)?, super::ped_mood::reaction_sets(&b), super::ped_mood::chase_records(&b), super::ped_mood::takedown_tables(&b), super::ped_mood::sight(&b), super::ped_mood::conversation_tables(&b), super::ped_mood::sit_values(&b), super::ped_mood::starting_hand_props(&b)))) {
            Ok((tables, sets, (chase, global, zombie), (takedowns, takedown_zombie), (sight, sight_scales), conversations, sit, starting_props)) => {
                info!(
                    "PED_MOOD tables: {} categories, {} results, {} reaction sets, {} entity types, {} chase records (global {}, zombie {}, zombie takedowns {})",
                    tables.categories.len(),
                    tables.results.len(),
                    tables.reactions.len(),
                    sets.len(),
                    chase.len(),
                    global.is_some(),
                    zombie.is_some(),
                    takedown_zombie.is_some()
                );
                loaded.mood = Some(Arc::new(tables));
                loaded.reaction_sets = Arc::new(sets);
                loaded.chase = Arc::new(chase);
                loaded.chase_global = global.map(Arc::new);
                loaded.chase_zombie = zombie.map(Arc::new);
                loaded.takedown_zombie = takedown_zombie.map(Arc::new);
                loaded.takedowns = Arc::new(takedowns);
                loaded.sight = Arc::new(sight);
                loaded.sight_scales = Arc::new(sight_scales);
                loaded.conversations = Arc::new(conversations);
                loaded.sit = Arc::new(sit);
                loaded.starting_props = Arc::new(starting_props);
            }
            Err(error) => warn!("PED_MOOD tables not loaded (no mood reactions): {error}"),
        }
        match super::ped_plugins::PluginData::load(asset_root) {
            Ok(mut p) => {
                // The map's DMO hotpoint props (benches, bins, newspaper boxes) beside the placed waypoint groups.
                let seats = super::ped_plugins::map_props(asset_root, map);
                info!("PED_PLUGINS {}: {} hotpoint props ({} seats)", map, seats.len(), seats.iter().filter(|s| s.class == "waypoint_sit").count());
                p.placed.entry(map.to_owned()).or_default().extend(seats);
                info!(
                    "PED_PLUGINS {} prop classes ({} with descriptors), {} ped types with plugin_odds, placed props {:?}",
                    p.classes.len(),
                    p.classes.values().filter(|c| c.descriptor.is_some()).count(),
                    p.odds.len(),
                    p.placed.iter().map(|(k, v)| (k.clone(), v.len())).collect::<Vec<_>>()
                );
                // Each class's plugin graph (`plugin/<name>.xml` compiled to `.stategraph`).
                for (class, c) in &p.classes {
                    let Some(d) = c.descriptor.as_ref().filter(|d| !d.graph.is_empty() && class != "waypoint_conversation") else { continue };
                    let relative = format!("private/stock/data/{}", d.graph.replace('\\', "/").trim_end_matches(".xml").to_string() + ".stategraph");
                    match super::ped_graph::PedGraph::load(asset_root, &relative) {
                        Ok(g) => {
                            info!("PED_GRAPH plugin {class}: {} behaviours, {} conditions; not ported yet: {:?}", g.behaviors.len(), g.conditions.len(), g.pending().keys().collect::<Vec<_>>());
                            loaded.plugin_graphs.insert(class.clone(), Arc::new(g));
                        }
                        Err(error) => warn!("PED_GRAPH plugin {class} not loaded: {error}"),
                    }
                }
                loaded.plugins = Arc::new(p);
            }
            Err(error) => warn!("PED_PLUGINS not loaded (no plugin props): {error}"),
        }
        match super::ped_hand_props::HandPropData::load(asset_root) {
            Ok(h) => {
                info!("PED_HAND_PROPS {} records, {} with models", h.props.len(), h.props.values().filter(|p| p.glb.is_some()).count());
                loaded.hand_props = Arc::new(h);
            }
            Err(error) => warn!("PED_HAND_PROPS not loaded (no hand props): {error}"),
        }
        loaded.loaded_for = Some((map.to_owned(), 0));
    }

    /// The animation set to play: the entity's, or `default` when the set's idle clips are not
    /// in the bank (the `granny` set names `GRAN_WNDR_*` clips that no shipped bank holds [data]).
    pub(crate) fn playable_set(&self, name: &str) -> Option<(&String, &skate_core::living_world::peds::PedAnimSet)> {
        let playable = |set: &skate_core::living_world::peds::PedAnimSet| {
            set.entries.get(skate_core::living_world::peds::anim::names::IDLE).is_some_and(|l| !l.is_empty() && l.iter().all(|c| self.clips.contains_key(&c.clip)))
        };
        self.anim_sets.get_key_value(name).filter(|(_, s)| playable(s)).or_else(|| self.anim_sets.get_key_value("default").filter(|(_, s)| playable(s)))
    }

    pub(crate) fn ready(&self) -> bool {
        !self.rig.names.is_empty() && !self.catalog.categories.is_empty()
    }

    /// Read the district's navmesh (M3) from `navmesh.bin` (none: peds use the test path).
    pub(crate) fn load_nav(&mut self, asset_root: &std::path::Path, district: &str, rules: &NavRules) {
        let input = skate_data::vfs::read(&asset_root.join(skate_data::ped_nav::NAVMESH))
            .map_err(|e| e.to_string())
            .and_then(|b| skate_data::ped_nav::district(&b, district));
        match input {
            Ok(Some(input)) => {
                let mesh = NavMesh::build(&input, rules.clone());
                self.status.push_str(&format!(", navmesh {} polygons", mesh.polys.len()));
                self.nav = Some(Arc::new(mesh));
                // skate_vita (memory): the records are not kept (upstream keeps them to rebuild the
                // mesh when a mod changes the nav rules; no mods on the Vita).
                drop(input);
            }
            Ok(None) => self.status.push_str(", no navmesh for this map"),
            Err(e) => self.status.push_str(&format!(", navmesh: {e}")),
        }
    }

    /// Read the tables and the bank, decode every clip an animation set names.
    pub(crate) fn load(asset_root: &std::path::Path) -> Self {
        // skate_vita: through `skate_data::vfs` (the packs).
        // skate_vita (memory): the packs carry the peds' classes in their own `ped_tables.json`
        // (an install keeps them in `tables.json`).
        let dir = asset_root.join("private/living_world");
        let tables = skate_data::vfs::read(&dir.join("ped_tables.json"))
            .or_else(|_| skate_data::vfs::read(&dir.join("tables.json")))
            .map_err(|e| e.to_string())
            .and_then(|b| skate_data::ped_anim::PedTables::parse(&b));
        let bank = skate_data::vfs::read(&asset_root.join(skate_data::ped_anim::PED_BANK)).map_err(|e| e.to_string()).and_then(skate_data::ped_anim::PedBank::parse);
        match (tables, bank) {
            (Ok(t), Ok(bank)) => {
                // skate_vita (memory): the clips the sets name are checked against the bank here
                // (present, valid frame count) and decoded on first use ([`ClipCache`]).
                let mut known = std::collections::BTreeSet::new();
                let mut failed = 0;
                for set in t.anim_sets.values() {
                    for c in set.entries.values().flatten() {
                        if !known.contains(&c.clip) {
                            if ClipCache::decodable(&bank, &c.clip) {
                                known.insert(c.clip.clone());
                            } else {
                                failed += 1;
                            }
                        }
                    }
                }
                let status = format!("peds: {} categories, {} sets, {} clips ({} missing, {} unresolved remaps)", t.catalog.categories.len(), t.anim_sets.len(), known.len(), failed, t.unresolved);
                let rig = Arc::new(bank.rig.clone());
                Self { catalog: Arc::new(t.catalog), anim_sets: Arc::new(t.anim_sets), rig, clips: ClipCache::new(bank, known), status, ..Self::default() }
            }
            (t, b) => Self { status: format!("peds: no body data ({})", [t.err(), b.err()].into_iter().flatten().collect::<Vec<_>>().join("; ")), ..Self::default() },
        }
    }
}

/// Mod / engine look overrides. Default = retail.
#[derive(Default, Clone, PartialEq)]
pub(crate) struct PedLooks {
    pub overrides: PedOverrides,
    /// recipe -> GLB asset path (with the ped bone names).
    pub glb: BTreeMap<String, String>,
}

/// One ped's identity (from its spawn record).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Pedestrian {
    pub id: LivingWorldId,
    pub census: String,
    pub category: String,
    pub entity: String,
    pub model: String,
    pub recipe: String,
    pub anim_set: String,
    pub seed: u64,
    pub spawn_tick: u64,
    pub tint_a: [f32; 4],
    pub tint_b: [f32; 4],
}

/// Navigation settings (mod-facing; default = retail): wander parameters, which navmesh areas
/// peds may use and what they cost, and the crosswalk rule (retail `Off`).
#[derive(Default, Clone, Debug, PartialEq)]
pub(crate) struct PedNavSettings {
    pub wander: WanderParams,
    pub rules: NavRules,
    pub crosswalk: CrosswalkRule,
}

/// The ped ground probe's retry reach after a miss (`82EA5F48`: 4.0 at `0x82257308`) [code, b107].
pub(crate) const GROUND_RETRY_FACTOR: f32 = 4.0;

/// Dynamic objects as ped navigation obstacles (fix 11): props and mod bodies by stable id.
#[derive(Default, Clone, Debug, PartialEq)]
pub(crate) struct PedObstacles(pub NavObstacles);

/// Obstacle id of a mod body (props keep their prop id; mod bodies live above 2^40).
pub(crate) const MOD_BODY_OBSTACLE_BASE: u64 = 1 << 40;

/// The obstacle list of this tick: prop boxes (current pose, held flag) and mod bodies (world
/// AABBs, the attached one held). Held is not the retail obstacle-off gate (that word is never set by
/// Move Object), so a held prop stays an obstacle (`ObstacleParams::held_is_obstacle`, retail on).
pub(crate) fn obstacle_inputs(physics: Option<&crate::physics::GamePhysics>, mod_solids: &[(u64, [f32; 3], [f32; 3], [f32; 3], bool)]) -> Vec<ObstacleInput> {
    let mut out = physics.and_then(|p| p.prop_dynamics()).map(prop_obstacle_inputs).unwrap_or_default();
    for (id, min, max, v, attached) in mod_solids {
        let center = [(min[0] + max[0]) * 0.5, (min[1] + max[1]) * 0.5, (min[2] + max[2]) * 0.5];
        let half = [(max[0] - min[0]) * 0.5, (max[1] - min[1]) * 0.5, (max[2] - min[2]) * 0.5];
        if !half.iter().all(|h| h.is_finite()) {
            continue;
        }
        out.push(ObstacleInput { id: MOD_BODY_OBSTACLE_BASE | id, center, axes: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]], half_extents: half, velocity: *v, inactive: false, held: *attached });
    }
    out
}

/// The prop boxes of this tick as obstacle inputs (current pose and velocity, held flag).
pub(crate) fn prop_obstacle_inputs(d: &crate::physics::prop_dynamics::PropDynamics) -> Vec<ObstacleInput> {
    d.obstacle_boxes()
        .into_iter()
        .map(|(id, c, basis, h, v, held)| ObstacleInput { id: id as u64, center: [c.x, c.y, c.z], axes: basis.columns, half_extents: [h.x, h.y, h.z], velocity: [v.x, v.y, v.z], inactive: false, held })
        .collect()
}

/// The simulated body: animation player, navigation, position and heading.
#[derive(Clone, Debug)]
pub(crate) struct PedBody {
    pub player: PedAnimPlayer,
    /// Placeholder intent source on maps without a navmesh.
    pub path: TestPath,
    /// Navigation state (M3); a mod route goes in `nav.route`.
    pub nav: PedNav,
    /// Seconds the body's steps have been refused (walls, other peds, the crosswalk rule).
    pub blocked: f32,
    pub position: Vec3,
    /// The last console tick's movement over its length, m/s (the attack throw's look-ahead, `82E3E960`).
    pub velocity: Vec3,
    pub heading: f32,
    /// Console ticks stepped since the spawn.
    pub ticks: u64,
    pub feet_down: [bool; 2],
    pub body_fall: f32,
    /// The taunt clip (motiongraph_taunt): requested by TakedownTauntVictim, playing, finished (the brain then
    /// drops its "SGIntent").
    pub taunt: TauntClip,
    pub plugin_motion: PluginMotionRun,
}

/// Where a ped's taunt clip is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum TauntClip {
    #[default]
    None,
    Requested,
    Playing,
    Done,
}

/// A plugin state's run on the motion side (`skate_core::living_world::peds::plugin_motion`): the packet it plays,
/// its progress (the taunt's handshake) and a pending release of its held cycle.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct PluginMotionRun {
    pub packet: Option<String>,
    pub state: TauntClip,
    pub release: bool,
    /// The packet ended while its clips still play (the AI left the sit because the seat moved away). Retail's sit
    /// state is non-interruptable (`motiongraph_sit.xml`): Stand2Sit plays out, then SittingCyc leaves on
    /// "not HasIntent Sit" to Sit2Stand, so the held cycle is released as soon as it is reached.
    pub orphaned: bool,
    /// The step that spawns the hand prop reached its branch window (`SpawnInteractionBasedHandProp`); the brain side
    /// takes it once (`think_peds`) and `hand_prop_taken` keeps it from firing again in the same run.
    pub hand_prop_due: bool,
    pub hand_prop_taken: bool,
    /// A hand prop throw clip the brain asked for (`82E3E648` plays it on the ped, blend 0.2) and its blend.
    pub throw_clip: Option<(&'static str, f32)>,
}

/// A ped counts as still while it stays within this distance of where it stopped, m.
const STILL_RADIUS: f32 = 0.1;
/// Ticks still before the first report and between reports (5 s on the 60 Hz world clock).
const STILL_REPORT_TICKS: u64 = 300;
/// Full still-ped reports per tick; due peds beyond this wait for a later tick, so many stuck
/// peds do not all format their report in the same frame.
const STILL_REPORTS_PER_TICK: u32 = 1;

/// What happened to a ped (engine systems, mods).
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum PedEvent {
    Spawned { id: LivingWorldId, entity: String, recipe: String },
    /// The look could not be resolved (no entities / model): the population slot is released.
    Rejected { id: LivingWorldId, category: String },
    Despawned { id: LivingWorldId, reason: DespawnReason },
    State { id: LivingWorldId, state: Locomotion },
    /// The mood system picked a reaction (`skate_core::living_world::peds::mood`); `wants` were
    /// raised when `passed`.
    Mood { id: LivingWorldId, result: String, passed: bool, wants: Vec<String> },
    /// A skater knocked the ped down or made it stumble (`skate_core::living_world::peds::skater_contact`).
    Hit {
        id: LivingWorldId,
        kind: skate_core::living_world::peds::skater_contact::ReactionKind,
        direction: skate_core::living_world::peds::skater_contact::ReactionDirection,
        /// The skater's speed into the ped, m/s (our stand-in for the ped body's speed).
        closing: f32,
        /// Who touched the ped (the brain's contact record `+3184`; players: `PLAYER_TARGET_BASE + n`).
        toucher: u64,
    },
    /// The ped's Colliding state began (PedestrianColliding Begin `8269DA00`): the collision mood broadcast
    /// (`sub_8269D8A8`), read by every ped's think on the next step.
    Colliding { id: LivingWorldId },
    /// A chase group changed (`kind`: join, join_refused, leave, primary, group_end); `chasee` is
    /// the chased id (players: `PLAYER_TARGET_BASE + n`).
    Chase { id: LivingWorldId, chasee: u64, kind: &'static str, reason: Option<i32> },
    /// A ped's tazer hit `target` (the takedown's knock-down).
    Taze { id: LivingWorldId, target: u64 },
    /// A ped's takedown attempt ended (`success`: the target is knocked down).
    Takedown { id: LivingWorldId, target: u64, success: bool },
    /// The ped's AI graph changed its speech value (`SendSpeechEvent` / the warn); `state` is the
    /// graph state that sent it.
    /// `topic`: a conversation turn's variant and row value (`ped+2472` / `+2476`).
    Speech { id: LivingWorldId, value: i32, topic: Option<(u8, i32)>, state: String },
    /// The ped took a hand prop (`livingworld_handprops` key) at a plugin prop (SpawnInteractionBasedHandProp).
    HandProp { id: LivingWorldId, key: String },
}

/// Skater against ped contact (doc 26, "Skater hits peds"). NOT RETAIL YET: the skater is a
/// vertical cylinder at the observer (the board) of this radius and 2 m tall, and the ped body's
/// speed after the contact (retail: the Havok solve) is the skater's speed into the ped.
pub(crate) const SKATER_CONTACT_RADIUS: f32 = 0.35;
/// `82E3DBE0`'s gate on the owned object's vector, |v|^2 > 2.5e-5 (`0x822F8BE0`) [code].
pub(crate) const SEAT_PUSH_SPEED_SQ: f32 = 2.5e-5;
/// `82C539A8`'s seat probe tolerance, m (the navmesh point within this of the seat in x and z).
pub(crate) const SEAT_PROBE_TOLERANCE: f32 = 0.1;
/// The sit plugin's held cycle (`plugin_motion` Sit) and the Avoid graph's forward jump off a seat
/// (`motiongraph_avoid.xml` JumpForward, `PlayRemappedAnimation SitAvoidFwd`) [data].
pub(crate) const SIT_CYCLE: &str = "SitIdleCyc";
pub(crate) const SIT_AVOID_CLIP: &str = "SitAvoidFwd";
/// `motiongraph_avoid.xml` SitJump: `PedestrianSpeech` PedestrianAvoidJump [data].
pub(crate) const SIT_AVOID_SPEECH: i32 = 4;

/// The animation set's collision rules for one contact: DisableHeavyCollision (`brain+3277` bit 0x20) turns the
/// knock-down off and leaves the standing stumble (`sub_82E38FB8` clears its knock-down test when the bit is set).
pub(crate) fn contact_rules(
    set: &skate_core::living_world::peds::skater_contact::CollisionRules,
    heavy_collision_disabled: bool,
) -> skate_core::living_world::peds::skater_contact::CollisionRules {
    skate_core::living_world::peds::skater_contact::CollisionRules { can_knock_down: set.can_knock_down && !heavy_collision_disabled, ..*set }
}

/// The retail reaction to a skater at `skater` moving at `velocity` touching the ped at `ped`
/// facing `heading`: `None` when not touching, not moving into it, or no reaction.
pub(crate) fn skater_hit(
    ped: Vec3,
    heading: f32,
    ped_radius: f32,
    skater: [f32; 3],
    velocity: [f32; 3],
    rules: &skate_core::living_world::peds::skater_contact::CollisionRules,
    ignore_standing: bool,
) -> Option<(skate_core::living_world::peds::skater_contact::ReactionKind, skate_core::living_world::peds::skater_contact::ReactionDirection, f32)> {
    use skate_core::living_world::peds::skater_contact::{reaction_direction, skater_reaction};
    let d = [ped.x - skater[0], ped.z - skater[2]];
    let dist = (d[0] * d[0] + d[1] * d[1]).sqrt();
    if dist >= ped_radius + SKATER_CONTACT_RADIUS || (ped.y - skater[1]).abs() > 2.0 || dist <= 1e-4 {
        return None;
    }
    let n = [d[0] / dist, d[1] / dist];
    let closing = velocity[0] * n[0] + velocity[2] * n[1];
    if closing <= 0.0 {
        return None;
    }
    // IgnoreStandingCollisions (`brain+3277` bit 0x10, e.g. a seated ped): only a knock-down reacts.
    let kind = skater_reaction([closing, 0.0], [0.0, 0.0], rules, ignore_standing)?;
    Some((kind, reaction_direction(n, [heading.sin(), heading.cos()]), closing))
}

/// A ped touching the ped at `ped` (retail `sub_82E38FB8` kind 5 with a ped toucher): `toucher` stood there and moved
/// at `velocity` relative to the ped. NOT RETAIL YET: the ped body speed after the contact (retail: the Havok solve) is
/// the closing speed along the line between the two.
#[allow(clippy::too_many_arguments)]
pub(crate) fn ped_hit(
    ped: Vec3,
    heading: f32,
    toucher: [f32; 3],
    velocity: [f32; 3],
    rules: &skate_core::living_world::peds::skater_contact::CollisionRules,
    ignore_standing: bool,
    zombie: bool,
) -> Option<(skate_core::living_world::peds::skater_contact::ReactionKind, skate_core::living_world::peds::skater_contact::ReactionDirection, f32)> {
    use skate_core::living_world::peds::skater_contact::{ped_reaction, reaction_direction};
    let d = [ped.x - toucher[0], ped.z - toucher[2]];
    let dist = (d[0] * d[0] + d[1] * d[1]).sqrt();
    if dist <= 1e-4 {
        return None;
    }
    let n = [d[0] / dist, d[1] / dist];
    let closing = velocity[0] * n[0] + velocity[2] * n[1];
    if closing <= 0.0 {
        return None;
    }
    let kind = ped_reaction([closing, 0.0], [0.0, 0.0], rules, ignore_standing, zombie, false)?;
    Some((kind, reaction_direction(n, [heading.sin(), heading.cos()]), closing))
}

/// Two peds' bodies touched: a step refused by another ped (`separation_blocker`). Each ped takes the other as its
/// toucher on its next step.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PedContact {
    touched: LivingWorldId,
    toucher: LivingWorldId,
    toucher_at: [f32; 3],
    velocity: [f32; 3],
}

impl PedClips for PedData {
    fn clip(&self, name: &str) -> Option<&PedClip> {
        self.clips.clip(name)
    }
}

/// skate_vita (memory, exact): the ped clips decoded on first use and kept within a byte budget.
/// Upstream decodes every clip the animation sets name at load (`PedBank::clip`: all frames, 48 bytes
/// a bone sample): 38 MB for the 152 clips, which the Vita's heap cannot hold beside a district. A
/// clip is decoded by the same `PedBank::clip` the first time a ped needs it; clips not used for a
/// while are dropped when the cache passes its budget ([`ClipCache::trim`], between ticks) and decode
/// again to the same frames when needed, so a ped plays exactly what upstream's decoded map holds.
pub(crate) struct ClipCache {
    bank: Option<skate_data::ped_anim::PedBank>,
    /// The clips the sets name that the bank holds (checked at load).
    known: std::collections::BTreeSet<String>,
    /// Decoded clips (boxed, owned through raw pointers: a reference handed out stays valid while
    /// other clips are inserted) and the tick each was last asked for. Only `trim` and `drop`
    /// (`&mut self`) remove entries.
    decoded: std::cell::UnsafeCell<BTreeMap<String, (*mut PedClip, usize)>>,
    used: std::cell::RefCell<BTreeMap<String, u64>>,
    bytes: std::cell::Cell<usize>,
    tick: std::cell::Cell<u64>,
}

impl Default for ClipCache {
    fn default() -> Self {
        Self { bank: None, known: Default::default(), decoded: Default::default(), used: Default::default(), bytes: Default::default(), tick: Default::default() }
    }
}

// SAFETY: the cache owns its clips (the raw pointers come from `Box::into_raw` and are freed once);
// it is used by one thread at a time (the session's living world).
unsafe impl Send for ClipCache {}

impl ClipCache {
    pub(crate) fn new(bank: skate_data::ped_anim::PedBank, known: std::collections::BTreeSet<String>) -> Self {
        Self { bank: Some(bank), known, decoded: Default::default(), used: Default::default(), bytes: Default::default(), tick: Default::default() }
    }

    /// Whether `PedBank::clip` would accept the clip's record (present, a whole frame count); the
    /// decode itself is not run.
    pub(crate) fn decodable(bank: &skate_data::ped_anim::PedBank, name: &str) -> bool {
        bank.bank.clip(name).is_some_and(|(_, clip)| {
            let frames = f32::from_bits(clip.frame_count_bits);
            frames.is_finite() && frames >= 1.0 && frames.fract() == 0.0
        })
    }

    pub(crate) fn contains_key(&self, name: &str) -> bool {
        self.known.contains(name)
    }

    pub(crate) fn len(&self) -> usize {
        self.known.len()
    }

    /// Bytes of decoded frames held now.
    pub(crate) fn bytes(&self) -> usize {
        self.bytes.get()
    }

    /// The clip, decoded now if it is not held.
    pub(crate) fn clip(&self, name: &str) -> Option<&PedClip> {
        self.used.borrow_mut().insert(name.to_owned(), self.tick.get());
        // SAFETY: the map is only changed here (insertion through `&self`, no reference into the map
        // is held across it) and in `trim` / `drop` (`&mut self`); the clips live in their own boxes,
        // which an insertion does not move, so a returned reference stays valid until `&mut self`.
        let map = unsafe { &mut *self.decoded.get() };
        if let Some((clip, _)) = map.get(name) {
            return Some(unsafe { &**clip });
        }
        if !self.known.contains(name) {
            return None;
        }
        let clip = self.bank.as_ref()?.clip(name).ok()?;
        let size = clip.frames.iter().map(|f| f.len() * std::mem::size_of::<skate_core::animation::output::Sqt>() + 32).sum::<usize>() + 256;
        let raw = Box::into_raw(Box::new(clip));
        map.insert(name.to_owned(), (raw, size));
        self.bytes.set(self.bytes.get() + size);
        Some(unsafe { &*raw })
    }

    /// Between ticks: the tick the next asks belong to, and when more than `budget` bytes are held,
    /// the clips asked for longest ago are dropped (never one asked for in the last `keep` ticks).
    pub(crate) fn trim(&mut self, tick: u64, budget: usize, keep: u64) {
        self.tick.set(tick);
        if self.bytes.get() <= budget {
            return;
        }
        let used = self.used.get_mut();
        let map = self.decoded.get_mut();
        let mut order: Vec<(u64, String)> = map.keys().map(|k| (used.get(k).copied().unwrap_or(0), k.clone())).collect();
        order.sort();
        for (last, name) in order {
            if self.bytes.get() <= budget || last + keep >= tick {
                break;
            }
            if let Some((raw, size)) = map.remove(&name) {
                // SAFETY: `&mut self`: no reference handed out by `clip` is alive.
                drop(unsafe { Box::from_raw(raw) });
                self.bytes.set(self.bytes.get() - size);
                used.remove(&name);
            }
        }
    }
}

impl Drop for ClipCache {
    fn drop(&mut self) {
        for (_, (raw, _)) in std::mem::take(self.decoded.get_mut()) {
            // SAFETY: each pointer came from `Box::into_raw` and is freed once.
            drop(unsafe { Box::from_raw(raw) });
        }
    }
}

/// Ground height near a point: the first hit of a line from `up` above to `down` below, `None`
/// when nothing is hit. The first hit from the top wins, so `up` must not reach overhead geometry.
pub(crate) fn ground(physics: Option<&crate::physics::GamePhysics>, at: Vec3, up: f32, down: f32) -> Option<f32> {
    use skate_core::math::Vector3;
    let p = physics?;
    match p.world().query_thin_line(Vector3::new(at.x, at.y + up, at.z), Vector3::new(at.x, at.y - down, at.z)) {
        Ok(Some(hit)) => Some(hit.geometry.position.y),
        _ => None,
    }
}

/// A stable ordering key for a ped (avoidance priority: lower first).
pub(crate) fn id_order(id: LivingWorldId) -> u64 {
    id.serial as u64
}

fn render_basis() -> Mat4 {
    Mat4::from_cols(glam::Vec4::X, -glam::Vec4::Z, glam::Vec4::Y, glam::Vec4::W)
}

/// Model-space globals of the rig's reference pose (`PEDESTRIAN_RIG_TPOSE`, root at the origin
/// like [`PedAnimPlayer::pose`](skate_core::living_world::peds::PedAnimPlayer::pose)).
pub(crate) fn reference_globals(rig: &PedRig) -> Vec<Mat4> {
    let mut locals = rig.reference.clone();
    if let Some(root) = locals.first_mut() {
        *root = skate_core::living_world::peds::anim::IDENTITY;
    }
    PedEvaluator::globals(rig, &locals).into_iter().map(crate::animation::native_matrix).collect()
}

/// The bone-local basis between a ped GLB's joint frames and the rig's: the candidate (identity,
/// the retail ped GLBs; or the skater GLB convention `render_basis`) whose
/// `reference x basis` is closest in rotation to the GLB bind over the matched bones. Identity
/// for every shipped ped model (data test); a pure function of the model, so deterministic.
pub(crate) fn ped_bone_basis(reference: &[Mat4], glb_bind: &BTreeMap<usize, Mat4>) -> Mat4 {
    let angle = |a: Mat4, b: Mat4| {
        let (qa, qb) = (Quat::from_mat4(&a).normalize(), Quat::from_mat4(&b).normalize());
        2.0 * qa.dot(qb).abs().min(1.0).acos()
    };
    let score = |basis: Mat4| -> f32 { glb_bind.iter().filter_map(|(&i, b)| Some(angle(*reference.get(i)? * basis, *b))).sum() };
    [Mat4::IDENTITY, render_basis()].into_iter().map(|b| (score(b), b)).fold((f32::INFINITY, Mat4::IDENTITY), |best, x| if x.0 < best.0 { x } else { best }).1
}

/// Joint globals for [`AnimationStatus::pose_transforms`](crate::animation::AnimationStatus)
/// (which right-multiplies the skater's `render_basis`): the rig globals in the GLB's joint
/// frames, so the skin matrix is `global x basis x inverse(GLB bind)`.
pub(crate) fn ped_joint_globals(globals: &[Mat4], basis: Mat4) -> Vec<Mat4> {
    let cancel = render_basis().inverse() * basis;
    globals.iter().map(|g| *g * cancel).collect()
}

/// Bones the clips do not carry follow their rig parent with the bind offset between them.
pub(crate) fn follower_offsets(rig: &PedRig, bind: &BTreeMap<usize, Mat4>) -> Vec<(usize, usize, Mat4)> {
    (0..rig.names.len())
        .filter(|&i| !rig.animated.get(i).copied().unwrap_or(false))
        .filter_map(|i| {
            let p = usize::try_from(*rig.parents.get(i)?).ok()?;
            let (bp, bi) = (bind.get(&p)?, bind.get(&i)?);
            Some((i, p, bp.inverse() * *bi))
        })
        .collect()
}

/// Model-space bone matrices for a ped pose (column convention, like `puppet_pose`).
pub(crate) fn ped_globals(rig: &PedRig, body: &PedBody, clips: &dyn PedClips, ahead: f32, followers: &[(usize, usize, Mat4)]) -> Option<Vec<Mat4>> {
    let locals = body.player.pose(rig, clips, ahead)?;
    let mut g: Vec<Mat4> = PedEvaluator::globals(rig, &locals).into_iter().map(crate::animation::native_matrix).collect();
    for &(i, p, offset) in followers {
        if p < g.len() && i < g.len() {
            g[i] = g[p] * offset;
        }
    }
    Some(g)
}

/// Drawn ped opacity with the NPC draw distance (QoL, not retail): the fade pair (model pair or
/// the configured default) x the multiplier, so the fade keeps ending before the scaled census
/// cull. At retail (1x) the pair is used as is. The LOD distances stay retail (far peds keep the
/// cheaper LOD).
pub(crate) fn ped_draw_alpha(settings: &LivingWorldSettings, pair: Option<[f32; 2]>, distance: f32, since_spawn: f32) -> f32 {
    let dd = settings.draw_distance();
    if dd.is_retail() {
        return skate_core::living_world::peds::draw_alpha(&settings.ped_fade, pair, distance, since_spawn);
    }
    let scale = |p: [f32; 2]| [dd.distance(p[0]), dd.distance(p[1])];
    let cfg = skate_core::living_world::peds::PedFadeConfig { distance: scale(settings.ped_fade.distance), ..settings.ped_fade };
    skate_core::living_world::peds::draw_alpha(&cfg, pair.map(scale), distance, since_spawn)
}

/// LOD placeholder: LOD0 within the model's first distance (45 m), LOD1 beyond its second
/// (55 m), hysteresis between [data pair `Hash_73B6874C7B46C7C6`, meaning unconfirmed].
pub(crate) fn lod_for(distance: f32, current: u8, near: [f32; 2]) -> u8 {
    if distance < near[0] {
        0
    } else if distance > near[1] {
        1
    } else {
        current
    }
}

/// A ped's behaviour brain and its controller on the stock ped AI graph (behaviour runtime,
/// doc 26). Host-owned plain data.
#[derive(Default)]
pub(crate) struct PedMind {
    pub brain: skate_core::living_world::peds::brain::PedBrain,
    /// The idle hand prop action's channel cue and its change count (`ped_hand_props::sync_hand_props` sets them,
    /// `advance_peds` plays a new cue once).
    pub action_cue: Option<skate_core::living_world::peds::hand_prop::ActionCue>,
    pub action_seq: u32,
    pub mood: skate_core::living_world::peds::mood::MoodStore,
    /// The current flee leg's goal (`skate_core::living_world::peds::flee`).
    pub flee_goal: Option<[f32; 3]>,
    /// The current intercept goal (`skate_core::living_world::peds::chase::intercept`).
    pub chase_goal: Option<[f32; 3]>,
    /// Where the body was at the last think (the ped's speed for the takedown window).
    last_at: Option<[f32; 3]>,
    presence_timer: f32,
    /// Time to the next zombie broadcast (`mood::BROADCAST_PERIOD`).
    zombie_timer: f32,
    rng: Option<skate_core::living_world::Rng>,
    controller: Option<skate_core::graph::controller::Controller>,
    /// The plugin graph's controller while the main graph runs the Plugin state.
    plugin_controller: Option<skate_core::graph::controller::Controller>,
    plugin_state: Option<usize>,
    /// Which plugin graph `plugin_controller` was made for (the prop class, or the conversation graph).
    plugin_graph: Option<String>,
    /// The seat's prop is moving under the seated owner this tick: the body jumps off forward (`82E3DBE0`).
    sit_avoid: bool,
    /// The speech value last sent to the audio side (`PedestrianSpeech` requests a line only when
    /// the value changes; a fresh ped holds the constructor's 68).
    spoken: Option<i32>,
    /// Population ticks run so far.
    ticks: u64,
    state: Option<usize>,
    /// The world prop the ped took (index in `PedPluginProps`, prop id, waypoint) and the props it refused (`82E40BD0`).
    prop: Option<(usize, u64, usize)>,
    refusals: skate_core::living_world::peds::plugins::RefusalMemory,
    /// A release the brain posted (DropHandProp) for `sync_hand_props`: the velocity, m/s.
    pub hand_prop_release: Option<[f32; 3]>,
    /// Log repeat limiter for PED_BRAIN / PED_MOOD lines (a refused startconversation re-raises its want every few
    /// ticks, retail by b28 / b29, and filled the log with 27k lines in 4 minutes).
    log_repeats: LogRepeats,
    /// Where the ped stopped and since when, for the still-ped report (`log_still_ped`).
    still: StillWatch,
}

impl PedMind {
    /// skate_vita (tests): the name of the AI graph state the brain is in.
    pub(crate) fn state_name<'a>(&self, graph: &'a super::ped_graph::PedGraph) -> &'a str {
        self.state.and_then(|s| graph.graph.binding.states.get(s)).map_or("none", |s| s.name.as_str())
    }
}

/// A ped that has not moved: where, since which tick, the next tick a report is due, and the
/// last full report (a repeat that matches it logs a short line instead).
#[derive(Default)]
struct StillWatch {
    anchor: Option<([f32; 3], u64)>,
    due: u64,
    last: String,
}

/// Per-ped log repeat limiter: the same line key logs at most once per `LOG_REPEAT_TICKS`; the next line of that
/// key carries how many were held back.
#[derive(Default)]
struct LogRepeats(BTreeMap<u64, (u64, u32)>);

/// 5 s of the 60 Hz world tick.
const LOG_REPEAT_TICKS: u64 = 300;

impl LogRepeats {
    /// `Some(held back count)` when the line should be logged now, `None` when it is held back.
    fn allow(&mut self, key: u64, tick: u64) -> Option<u32> {
        match self.0.get_mut(&key) {
            Some((last, held)) if tick < *last + LOG_REPEAT_TICKS => {
                *held += 1;
                None
            }
            Some((last, held)) => {
                *last = tick;
                Some(std::mem::take(held))
            }
            None => {
                self.0.insert(key, (tick, 0));
                Some(0)
            }
        }
    }
}

/// Obstacle / want target ids of the players (above every living-world id).
pub(crate) const PLAYER_TARGET_BASE: u64 = u64::MAX - 64;


// ---- skate_vita: upstream's systems over plain data ----

/// skate_vita: one ped entity of upstream's ECS (its `Pedestrian`, `PedBody` and `PedMind`
/// components), where it is drawn (the render ground under the navigation position) and the render
/// state of its last two steps.
pub(crate) struct PedEntity {
    pub ped: Pedestrian,
    pub body: PedBody,
    pub mind: PedMind,
    /// The drawn feet (upstream's `Transform` translation) and its spawn fade / draw fade opacity.
    pub shown: Vec3,
    pub alpha: f32,
    pub lod: u8,
    /// The rig's model-space bones of the last two steps with the drawn root (position, heading),
    /// for the peds in view (render).
    pub pose: Option<(Vec3, f32, Vec<Mat4>)>,
    pub previous_pose: Option<(Vec3, f32, Vec<Mat4>)>,
    /// The held hand prop (upstream `HeldHandProp`) and the released one still linked to the ped
    /// (`ReleasedHandProp`: its prop body).
    pub held: Option<super::ped_hand_props::HeldHandProp>,
    pub linked: Option<u32>,
}

/// skate_vita: the peds of the loaded world (upstream's `PedIndex`, the entities, `PedRejected`,
/// the ped contacts of the last step and the played hand prop cues).
#[derive(Default)]
pub(crate) struct PedState {
    pub data: PedData,
    pub looks: PedLooks,
    pub nav: PedNavSettings,
    pub index: BTreeMap<LivingWorldId, PedEntity>,
    pub rejected: Vec<LivingWorldId>,
    pub contacts: Vec<PedContact>,
    pub played_cues: BTreeMap<LivingWorldId, u32>,
    pub events: Vec<PedEvent>,
    /// The behaviour runtime's shared state (upstream resources) and `think_peds`' locals.
    pub chase_groups: PedChaseGroups,
    pub conversations: PedConversations,
    pub plugin_props: PedPluginProps,
    pub think: ThinkLocals,
    pub takedowns: Vec<SkaterTakedownRequest>,
    /// Live released hand props in creation order (upstream `ReleasedHandProps`): body id, key and
    /// the body's frame at the last present step (render).
    pub released: Vec<(u32, String, Option<Mat4>)>,
    /// Hand prop collision triangles by model path, read on first release.
    pub hand_triangles: BTreeMap<String, Option<Arc<Vec<[Vec3; 3]>>>>,
}

impl PedState {
    /// `load_ped_data` for a new world (the bodies and the navmesh).
    pub(crate) fn load(asset_root: &std::path::Path, map: &str, roads: Option<&skate_core::living_world::traffic::RoadNetwork>) -> Self {
        let nav = PedNavSettings::default();
        let mut data = PedData::load(asset_root);
        data.load_nav(asset_root, map, &nav.rules);
        if let Some(roads) = roads {
            data.arms = Arc::new(skate_core::living_world::peds::crosswalk::signalled_arms(roads));
        }
        data.load_behaviour(asset_root, map);
        info!("LIVING_WORLD {}", data.status);
        Self { data, nav, ..Default::default() }
    }
}

/// Spawn and despawn ped entities from the population's records (`apply_ped_records`).
pub(crate) fn apply_ped_records(decisions: &[Decision], physics: &crate::physics::GamePhysics, peds: &mut PedState) {
    for d in decisions {
        let Decision::Despawn(r) = d else { continue };
        if r.id.kind != Kind::Pedestrian {
            continue;
        }
        if peds.index.remove(&r.id).is_some() {
            peds.events.push(PedEvent::Despawned { id: r.id, reason: r.reason });
        }
    }
    let data = &peds.data;
    for d in decisions {
        let Decision::Spawn(s) = d else { continue };
        if s.id.kind != Kind::Pedestrian || peds.index.contains_key(&s.id) {
            continue;
        }
        let SpawnChoice::Census { record, category } = &s.choice else { continue };
        if !data.ready() {
            continue; // no body data: the population still runs (audio-less, invisible)
        }
        let look = data.catalog.choose(category, s.seed, &peds.looks.overrides);
        let (Some(mut look), Some(set)) = (look.clone(), look.as_ref().and_then(|l| data.playable_set(&l.anim_set))) else {
            peds.events.push(PedEvent::Rejected { id: s.id, category: category.clone() });
            peds.rejected.push(s.id);
            continue;
        };
        look.anim_set = set.0.clone();
        let Some(player) = PedAnimPlayer::new(set.1, s.seed) else { continue };
        let mut at = Vec3::from_array(s.position);
        let mut nav = PedNav::default();
        // On the navmesh at the record's own height (a ground probe first could land on a wall
        // top above the record and put the ped on that layer, fix 17); maps without a navmesh
        // take the ground below.
        match data.nav.as_ref().and_then(|m| m.locate(at.to_array())) {
            Some(p) => {
                at = Vec3::from_array(p.position);
                nav.poly = Some(p.poly);
            }
            // The census ring point carries the observer's height, not the floor's: release the
            // spawn like an unresolved look; the census tries another point next pass.
            None if data.nav.is_some() => {
                peds.events.push(PedEvent::Rejected { id: s.id, category: category.clone() });
                peds.rejected.push(s.id);
                continue;
            }
            None => {
                if let Some(y) = ground(Some(physics), at, 3.0, 3.0) {
                    at.y = y;
                }
            }
        }
        let ped = Pedestrian {
            id: s.id,
            census: record.clone(),
            category: category.clone(),
            entity: look.entity.clone(),
            model: look.model.clone(),
            recipe: look.recipe.clone(),
            anim_set: look.anim_set.clone(),
            seed: s.seed,
            spawn_tick: s.tick,
            tint_a: look.tint_a,
            tint_b: look.tint_b,
        };
        let body = PedBody { player, path: TestPath::new(s.seed), nav, blocked: 0.0, position: at, velocity: Vec3::ZERO, heading: s.heading, ticks: 0, feet_down: [false; 2], body_fall: 0.0, taunt: TauntClip::None, plugin_motion: PluginMotionRun::default() };
        // Spawn fade in starts at 0 (retail `+576`); the present step raises it.
        peds.index.insert(s.id, PedEntity { ped, body, mind: PedMind::default(), shown: at, alpha: 0.0, lod: 0, pose: None, previous_pose: None, held: None, linked: None });
        peds.events.push(PedEvent::Spawned { id: s.id, entity: look.entity, recipe: look.recipe });
    }
}

/// Give the population slot of a rejected spawn back (`release_rejected`). Returns the despawns.
pub(crate) fn release_rejected(peds: &mut PedState, state: &mut PopulationState) -> Vec<Decision> {
    let mut out = Vec::new();
    for id in peds.rejected.drain(..) {
        if let Some(d @ Decision::Despawn(_)) = state.world.despawn(id, DespawnReason::External) {
            state.despawned += 1;
            out.push(d);
        }
    }
    out
}

/// Retail's per-object obstacle update, once per population tick before the peds step
/// (`update_ped_obstacles`; no mod bodies, no diagnostics trace).
pub(crate) fn update_ped_obstacles(settings: &LivingWorldSettings, data: &PedData, physics: &crate::physics::GamePhysics, obstacles: &mut NavObstacles, observers: &[skate_core::living_world::Observer]) {
    let _scope = skate_core::profile::scope("z_lw_obstacles");
    obstacles.set_params(settings.ped_obstacles.clone());
    if data.nav.is_none() {
        if !obstacles.states.is_empty() {
            obstacles.update(&[]);
        }
        return;
    }
    let mut inputs = obstacle_inputs(Some(physics), &[]);
    // Every player skater (b135: always NavPower's moving avoider for peds), by player index.
    if settings.ped_obstacles.skaters {
        let h = settings.ped_obstacles.skater_half_extents;
        for (n, o) in observers.iter().enumerate() {
            let center = [o.position[0], o.position[1] + h[1], o.position[2]];
            inputs.push(ObstacleInput { id: skate_core::living_world::peds::obstacles::SKATER_OBSTACLE_BASE + n as u64, center, axes: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]], half_extents: h, velocity: o.velocity, inactive: false, held: false });
        }
    }
    obstacles.update(&inputs);
}

/// Step every ped to the population tick (`advance_peds`): navigation, animation, root motion
/// kept on the navmesh and apart from other peds, ground, position. Peds step in id order against
/// a shared position list. skate_vita: no audio, no diagnostics logs.
#[allow(clippy::too_many_arguments)]
pub(crate) fn advance_peds(
    state: &mut PopulationState,
    peds: &mut PedState,
    clock: Option<&skate_core::living_world::traffic::SignalClock>,
    physics: &crate::physics::GamePhysics,
    obstacles: &NavObstacles,
    observers: &[skate_core::living_world::Observer],
    npc_actors: Vec<(u64, [f32; 3], [f32; 3])>,
    settings: &LivingWorldSettings,
) {
    let _scope = skate_core::profile::scope("z_ped_advance");
    let tick = state.world.tick();
    let dt = tick_seconds(state.world.clock().hz);
    let (data, nav_settings) = (&peds.data, &peds.nav);
    let mut list: Vec<&mut PedEntity> = peds.index.values_mut().collect();
    list.sort_by_key(|e| e.ped.id);
    let mut neighbours: Vec<Neighbour> = list.iter().map(|e| Neighbour { order: id_order(e.ped.id), position: e.body.position.to_array() }).collect();
    let ids: Vec<LivingWorldId> = list.iter().map(|e| e.ped.id).collect();
    let pending = std::mem::take(&mut peds.contacts);
    let ped_contacts = &mut peds.contacts;
    let played_cues = &mut peds.played_cues;
    let events = &mut peds.events;
    // Skater touchers (`IActor` owners, contact kind 5 with the skater thresholds): the players, then the NPC skaters
    // in id order (NPC skaters knock peds over in retail).
    let mut actors: Vec<(u64, [f32; 3], [f32; 3])> = observers.iter().enumerate().map(|(i, o)| (PLAYER_TARGET_BASE + i as u64, o.position, o.velocity)).collect();
    let mut npc_actors = npc_actors;
    npc_actors.sort_by_key(|a| a.0);
    actors.extend(npc_actors);
    let road_signals = clock.map(|clock| skate_core::living_world::peds::crosswalk::RoadWalkSignals { arms: &data.arms, clock, radius: 20.0 });
    let signals: &dyn WalkSignals = match &road_signals {
        Some(s) => s,
        None => &NoSignals,
    };
    for (k, entity) in list.into_iter().enumerate() {
        let (ped, body, mind) = (&entity.ped, &mut entity.body, Some(&entity.mind));
        // OverrideAnimData (zombie mode: the `zombie` set) wins over the ped's own set while its state runs.
        let anim_set = mind.and_then(|m| m.brain.anim_override.as_deref()).filter(|n| data.anim_sets.contains_key(*n)).unwrap_or(&ped.anim_set);
        let Some(set) = data.anim_sets.get(anim_set).or_else(|| data.anim_sets.get("default")) else { continue };
        let target = tick.saturating_sub(ped.spawn_tick);
        let me = id_order(ped.id);
        while body.ticks < target {
            let mut turn = 0.0;
            let step_from = body.position;
            // The taunt clip (motiongraph_taunt `PlayTaunt`: the remapped "Taunt" once, blend 0.1, then
            // `MajorIntentComplete`); a set without the clip completes at once.
            match body.taunt {
                TauntClip::Requested if body.player.state != Locomotion::Reaction => {
                    let step = skate_core::living_world::peds::skater_contact::ReactionStep { anim: "Taunt", mirror: false, blend: 0.1, cycle: false };
                    body.taunt = if body.player.react(set, vec![step], 0.0) { TauntClip::Playing } else { TauntClip::Done };
                }
                TauntClip::Playing if body.player.state != Locomotion::Reaction => body.taunt = TauntClip::Done,
                _ => {}
            }
            // A hand prop throw clip (`82E3E648`, ped vfunc +240 = the channel request `82E3A418`): a channel clip (its
            // weights pick the upper body, b99), played once over locomotion; it replaces the carry pose, which comes back
            // once the slot is free and the prop is still held.
            if let Some((clip, blend)) = body.plugin_motion.throw_clip.take() {
                if !body.player.channel_request(set, clip, blend, blend) {
                    info!("PED_HAND_PROP ped=#{} no clip {clip} tick={tick}", ped.id.serial);
                }
            }
            // A plugin state's clips (sit, ATM, vending machine, water fountain, newspaper box).
            let run = &mut body.plugin_motion;
            // The Avoid graph's SitJump out of the sitting cycle (`think_peds` raises `sit_avoid`).
            if mind.is_some_and(|m| m.sit_avoid) && body.player.reaction_anim() == Some(SIT_CYCLE) {
                let steps = vec![skate_core::living_world::peds::skater_contact::ReactionStep { anim: SIT_AVOID_CLIP, mirror: false, blend: 0.1, cycle: false }];
                if body.player.interrupt(set, steps, 0.0) {
                    run.state = TauntClip::Done;
                }
            }
            match (run.state, run.packet.as_deref().and_then(skate_core::living_world::peds::plugin_motion::motion_for)) {
                (TauntClip::Requested, Some(m)) if !body.player.reacting() => {
                    use skate_core::living_world::peds::plugin_motion::BLEND;
                    let steps = m.steps.iter().map(|st| skate_core::living_world::peds::skater_contact::ReactionStep { anim: st.anim, mirror: false, blend: BLEND, cycle: st.hold_until.is_some() }).collect();
                    run.state = if body.player.react(set, steps, f32::INFINITY) { TauntClip::Playing } else { TauntClip::Done };
                }
                (TauntClip::Playing, m) => {
                    if std::mem::take(&mut run.release) || run.orphaned {
                        body.player.release_hold();
                    }
                    // VendHandProp / CollectHandProp: the collect clip's branch window (`InTurnBranchWindow`).
                    let collect = m.and_then(|m| m.steps.iter().find(|st| st.hand_prop)).map(|st| st.anim);
                    if !run.hand_prop_taken && collect.is_some() && body.player.reaction_anim() == collect && body.player.in_branch_window() {
                        run.hand_prop_due = true;
                        run.hand_prop_taken = true;
                    }
                    if !body.player.reacting() {
                        run.state = TauntClip::Done;
                    }
                }
                _ => {}
            }
            // A skater running into the ped (retail `sub_82E38FB8` kind 5). Retail skips the contact only while the ped is
            // already in a collision reaction (controller byte +9), not during any clip: a ped sitting on a bench or
            // using a prop (its plugin clip plays as a reaction) can be knocked over.
            let clip_reaction = body.taunt == TauntClip::Playing || body.plugin_motion.state == TauntClip::Playing;
            let in_collision = mind.is_some_and(|m| m.brain.colliding) || (body.player.state == Locomotion::Reaction && !clip_reaction);
            if !in_collision {
                let radius = data.nav.as_deref().map_or(super::vehicle_contacts::FALLBACK_PED_RADIUS, |m| m.agent[1]);
                // Contacts from the entity the ped ignores (`ped+5680`, its takedown target) are dropped (`sub_82E38400`).
                let ignored = mind.and_then(|m| m.brain.ignore_collisions_with);
                let rules = contact_rules(&set.collision, mind.is_some_and(|m| m.brain.heavy_collision_disabled));
                let ignore_standing = mind.is_some_and(|m| m.brain.ignore_standing_collisions);
                let skater = actors.iter()
                    .filter(|(id, ..)| ignored != Some(*id))
                    .find_map(|&(id, at, v)| skater_hit(body.position, body.heading, radius, at, v, &rules, ignore_standing).map(|h| (h, id)));
                let touched = || pending.iter()
                    .filter(|c| c.touched == ped.id && ignored != Some(c.toucher.to_u64()))
                    .find_map(|c| ped_hit(body.position, body.heading, c.toucher_at, c.velocity, &rules, ignore_standing, settings.zombie).map(|h| (h, c.toucher.to_u64())));
                if let Some(((kind, direction, closing), toucher)) = skater.or_else(touched) {
                    let steps = skate_core::living_world::peds::skater_contact::reaction_steps(kind, direction);
                    if body.player.react(set, steps, set.collision.ground_seconds) {
                        info!("PED_SKATER_CONTACT ped=#{} by={toucher} kind={} direction={} closing={closing:.2} at=[{:.2}, {:.2}, {:.2}] tick={tick}", ped.id.serial, kind.name(), direction.name(), body.position.x, body.position.y, body.position.z);
                        events.push(PedEvent::Hit { id: ped.id, kind, direction, closing, toucher });
                    }
                }
            }
            // The behaviour graph stops the ped and faces a point (StopAndFaceWantTarget,
            // watching, StandAndWatchSkater: speed suggestion 0.0 and the face point): stand and
            // turn towards it at the nav's turn rate (ours until the motion graph's turn
            // branches run), instead of the wander step.
            let face = mind.filter(|m| m.brain.speed_suggestion == Some(0.0)).and_then(|m| m.brain.face);
            // LockToCurrentPosition: stand where the ped is (turning only for a face point).
            // DisableNavigation (ped `+5936` bit 0x80 clear): no navigation, the ped stands the same way.
            let locked = mind.is_some_and(|m| m.brain.position_locked || m.brain.navigation_disabled);
            // TargetWaypoint's slide: within the slide distance the think step moves the ped onto the point; the body
            // stands (no wander target while the route is off).
            let sliding = mind.is_some_and(|m| {
                m.brain.approach.zip(m.brain.approach_slide).is_some_and(|((p, _), (slide, _))| (p[0] - body.position.x).hypot(p[2] - body.position.z) <= slide)
            });
            match data.nav.as_deref() {
                _ if body.player.state == Locomotion::Reaction => body.player.intent = skate_core::living_world::peds::anim::Intent::Idle,
                _ if (locked || sliding) && face.is_none() => {
                    body.player.intent = skate_core::living_world::peds::anim::Intent::Idle;
                    // The explicit turn direction (`+2100` bit 0x20): a ped standing on its waypoint keeps turning to
                    // the waypoint's orientation.
                    if let Some(dir) = mind.and_then(|m| m.brain.explicit_turn).filter(|d| d[0] * d[0] + d[2] * d[2] > 1e-6) {
                        let mut error = dir[0].atan2(dir[2]) - body.heading;
                        while error > std::f32::consts::PI {
                            error -= std::f32::consts::TAU;
                        }
                        while error < -std::f32::consts::PI {
                            error += std::f32::consts::TAU;
                        }
                        let step = nav_settings.wander.turn_rate * dt;
                        turn = error.clamp(-step, step);
                    }
                }
                _ if face.is_some() => {
                    let p = face.unwrap_or_default();
                    let d = [p[0] - body.position.x, p[2] - body.position.z];
                    body.player.intent = skate_core::living_world::peds::anim::Intent::Idle;
                    if d[0] * d[0] + d[1] * d[1] > 1e-6 {
                        let desired = d[0].atan2(d[1]);
                        let mut error = desired - body.heading;
                        while error > std::f32::consts::PI {
                            error -= std::f32::consts::TAU;
                        }
                        while error < -std::f32::consts::PI {
                            error += std::f32::consts::TAU;
                        }
                        let step = nav_settings.wander.turn_rate * dt;
                        turn = error.clamp(-step, step);
                    }
                }
                Some(mesh) => {
                    // The graph's Pedestrian nav modifier off (a chase): no avoiding other peds.
                    let avoid_peds = mind.is_none_or(|m| m.brain.nav_modifier(skate_core::living_world::peds::brain::nav_modifier::PEDESTRIAN));
                    let neighbours_now: &[_] = if avoid_peds { &neighbours } else { &[] };
                    let out = body.nav.step_avoiding(mesh, &nav_settings.wander, nav_settings.crosswalk, signals, me, body.position.to_array(), body.heading, body.player.state, neighbours_now, Some(obstacles), dt);
                    // Fleeing: run (the chase run cycle) where the nav walks.
                    let fleeing = mind.is_some_and(|m| m.flee_goal.is_some() || m.chase_goal.is_some());
                    body.player.intent = if fleeing && out.intent == skate_core::living_world::peds::anim::Intent::Walk { skate_core::living_world::peds::anim::Intent::Run } else { out.intent };
                    turn = out.turn;
                    if out.turn_degrees != 0 {
                        use skate_core::living_world::peds::anim::{names, Intent};
                        let walking = body.player.state == Locomotion::Walk;
                        let clip = if walking { names::walk_turn(out.turn_degrees) } else { names::stand_turn(out.turn_degrees) };
                        if set.entries.contains_key(clip) {
                            body.player.turn_degrees = out.turn_degrees;
                        } else {
                            // Ours: a set without the bucket's clip (a mod set, a test set) pivots / steers instead.
                            body.player.intent = if walking { Intent::Walk } else { Intent::Idle };
                            turn = out.steer;
                        }
                    }
                }
                None => body.player.intent = body.path.intent(dt, body.player.state),
            }
            // The carry pose (`82E3F048`): while the hand prop is held and the channel is free, its record's carry
            // animation is requested again (blend 0.2); a released or dropped prop stops it with a 0.2 s fade (unlink
            // `82E3FAE0`). Retail requests it only while the prop is linked; ours: while held.
            if let Some(h) = mind.map(|m| &m.brain.hand_prop) {
                // skate_vita: the carry clip is not taken from the hand prop record here (`carry` stays `None`).
                let carry: Option<&str> = h.key.as_ref().and(None);
                // The idle action's cue (`82E3F470`): a new cue plays once; the talk / read cycles are asked for every
                // frame in retail, so ours asks again when their clip ends early.
                use skate_core::living_world::peds::hand_prop::ActionCue;
                if let Some((cue, seq)) = mind.and_then(|m| m.action_cue.map(|c| (c, m.action_seq))) {
                    // skate_vita: the brain settings' default (upstream's `ped_brain`).
                    let blend = skate_core::living_world::peds::brain::BrainSettings::default().hand_prop.actions.clip_blend;
                    let new = played_cues.get(&ped.id) != Some(&seq);
                    match cue {
                        ActionCue::Clip(name) if new || (body.player.channel_idle() && matches!(name, "HandPropPhoneTalk" | "HandPropReadCyc")) => {
                            body.player.channel_request(set, name, blend, blend);
                        }
                        ActionCue::Carry if new => {
                            if let Some(name) = carry {
                                body.player.channel_request(set, name, blend, blend);
                            }
                        }
                        _ => {}
                    }
                    played_cues.insert(ped.id, seq);
                }
                match (h.holding, carry) {
                    (true, Some(name)) if body.player.channel_idle() => {
                        body.player.channel_request(set, name, 0.2, 0.2);
                    }
                    (false, _) if body.player.channel_clip().is_some_and(|c| c.starts_with("NPC_CARRY")) => body.player.channel_stop(0.2),
                    _ => {}
                }
            }
            let out = body.player.step(dt, set, &*data);
            body.heading += turn;
            let rotation = Quat::from_rotation_y(body.heading);
            let to = body.position + rotation * Vec3::from_array(out.root.translation);
            match data.nav.as_deref() {
                Some(mesh) => {
                    // Over linked polygons only: across tile seams, never onto an unconnected
                    // layer such as a wall top (fix 17).
                    let from = body.position.to_array();
                    let (next, mut poly, on_mesh) = constrain_move(mesh, body.nav.poly, from, to.to_array());
                    let moving = (to - body.position).length_squared() > 1e-10;
                    // Never into a prop or mod body: slide along its face or stay (fix 11). A ped in a plugin whose
                    // `DisableCollisionsWithBehaviourSource` is active ignores the plugin's own prop (its body id is the
                    // plugin prop id's low 32 bits), so it can reach a seat inside its bench.
                    let ignore = skate_core::living_world::peds::obstacles::Ignore {
                        id: mind.filter(|m| m.brain.ignore_source_collision).and_then(|m| m.prop).map(|(_, id, _)| u64::from(id as u32)),
                        // SkaterAvoidance off (ChangeNavModifierSetting index 2): the skaters are not obstacles.
                        skaters: mind.is_some_and(|m| !m.brain.nav_modifier(skate_core::living_world::peds::brain::nav_modifier::SKATER_AVOIDANCE)),
                    };
                    let (next, clear) = match obstacles.resolve_step_ignoring(from, next, mesh.agent[1], ignore) {
                        Some(n) if n == next => (n, true),
                        Some(n) => {
                            let (m, k, ok) = constrain_move(mesh, body.nav.poly, from, n);
                            poly = k;
                            (m, ok)
                        }
                        None => (next, false),
                    };
                    // A step that slid to (almost) nothing against an edge counts as refused, so
                    // a ped walking into a boundary re-plans instead of walking in place.
                    let wanted = (to - body.position).with_y(0.0).length();
                    let progressed = !moving || Vec3::from_array(next).with_y(0.0).distance(body.position.with_y(0.0)) >= 0.25 * wanted;
                    let blocker = separation_blocker(body.position.to_array(), next, me, &neighbours, mesh.agent[1]);
                    let ok = on_mesh && clear && progressed && blocker.is_none() && crosswalk_ok(mesh, nav_settings.crosswalk, signals, body.position.to_array(), next);
                    // Refused by another ped: the two bodies touch; both react to the other on their next step.
                    if let Some(j) = blocker.filter(|_| moving) {
                        let v = ((to - body.position) / dt).with_y(0.0).to_array();
                        ped_contacts.push(PedContact { touched: ids[j], toucher: ped.id, toucher_at: from, velocity: v });
                        ped_contacts.push(PedContact { touched: ped.id, toucher: ids[j], toucher_at: neighbours[j].position, velocity: [-v[0], 0.0, -v[2]] });
                    }
                    // skate_vita: the moved-prop diagnostics (`PED_BLOCKED`) are not taken.
                    if ok {
                        body.position = Vec3::from_array(next);
                        body.nav.poly = poly;
                        body.blocked = 0.0;
                    } else if moving {
                        body.blocked += dt;
                        if body.blocked > nav_settings.wander.yield_patience {
                            // Stuck against a wall / ped / red light: pick another way (short fan).
                            body.blocked = 0.0;
                            body.nav.skip_long = true;
                            body.nav.corners.clear();
                        }
                    }
                    neighbours[k].position = body.position.to_array();
                }
                None => body.position = to,
            }
            body.heading += out.root.yaw;
            body.feet_down = out.feet_down;
            body.body_fall = out.body_fall;
            body.velocity = (body.position - step_from) / dt;
            body.ticks += 1;
            if let Some(s) = out.entered {
                events.push(PedEvent::State { id: ped.id, state: s });
            }
        }

        // The render ground: a line query round the navmesh height. It never feeds back into the
        // navigation position (fix 17). Upward it only searches the NavPower step height; downward
        // one agent height, then 4x after a miss (retail's mover probe, `82EA5F48`).
        let mut shown = body.position;
        let (up, down) = data.nav.as_deref().map_or((3.0, 3.0), |m| (m.agent[2].max(0.05), m.agent[3].max(0.5)));
        if let Some(y) = ground(Some(physics), body.position, up, down).or_else(|| ground(Some(physics), body.position, up, down * GROUND_RETRY_FACTOR)) {
            shown.y = y;
        }
        entity.shown = shown;
        state.world.update_position(ped.id, body.position.to_array());
    }
}

/// skate_vita: upstream's `present_ped_pose` on the fixed step: each ped's draw opacity (model fade
/// pair x the NPC draw distance, spawn fade in) and LOD from the camera, and the rig pose of the
/// peds in view (opacity above 0) kept with the previous step's for the renderer.
pub(crate) fn present_peds(settings: &LivingWorldSettings, state: &PopulationState, peds: &mut PedState, camera: Option<Vec3>) {
    let _scope = skate_core::profile::scope("z_ped_pose");
    let hz = state.world.clock().hz;
    // skate_vita (memory): the clip cache's budget, checked between ticks (the next tick's asks).
    peds.data.clips.trim(state.world.tick() + 1, CLIP_BUDGET, CLIP_KEEP_TICKS);
    let data = &peds.data;
    for entity in peds.index.values_mut() {
        let (ped, body) = (&entity.ped, &entity.body);
        if let Some(cam) = camera {
            let pair = data.catalog.models.get(&ped.model).and_then(|m| m.lod_near);
            let since = state.world.tick().saturating_sub(ped.spawn_tick) as f32 / hz.max(1.0) as f32;
            entity.alpha = ped_draw_alpha(settings, pair, cam.distance(body.position), since);
            let near = pair.unwrap_or([45.0, 55.0]);
            entity.lod = lod_for(cam.distance(body.position), entity.lod, near);
        }
        if entity.alpha <= 0.0 {
            entity.pose = None;
            entity.previous_pose = None;
            continue;
        }
        let Some(globals) = ped_globals(&data.rig, body, data, 0.0, &[]) else { continue };
        let next = (entity.shown, body.heading, globals);
        entity.previous_pose = entity.pose.take().or_else(|| Some(next.clone()));
        entity.pose = Some(next);
    }
}

/// skate_vita (memory): bytes of decoded ped clips kept, and ticks a clip asked for stays.
pub(crate) const CLIP_BUDGET: usize = 6 << 20;
pub(crate) const CLIP_KEEP_TICKS: u64 = 120;

/// A ped as the renderer draws it: its model, the rig's model-space bones of the previous
/// and the current step with the drawn root (position, heading), the tint pair, the opacity and the LOD.
#[derive(Clone, Debug)]
pub struct PedPose {
    pub serial: u32,
    /// Index of the ped's model in `Session::ped_models`.
    pub model: u16,
    pub previous: (Vec3, f32, Vec<Mat4>),
    pub current: (Vec3, f32, Vec<Mat4>),
    pub tint_a: [f32; 4],
    pub tint_b: [f32; 4],
    pub alpha: f32,
    pub lod: u8,
    /// The held hand prop (its model's index in `Session::hand_prop_models`, its frame in the
    /// `RIGHTHANDPROP` bone's frame).
    pub hand: Option<(u16, Mat4)>,
}

/// A released hand prop as the renderer draws it (its body's frame before and after the step).
#[derive(Clone, Debug, PartialEq)]
pub struct HandPropPose {
    /// Index of its model in `Session::hand_prop_models`.
    pub model: u16,
    pub previous: Mat4,
    pub current: Mat4,
}

impl PedData {
    /// The hand prop models the renderer loads (`hand_props/<key>.hprop` beside each record's GLB), in key order.
    pub(crate) fn hand_prop_models(&self) -> Vec<String> {
        self.hand_props.props.values().filter_map(|r| r.glb.as_ref()).map(|g| g.strip_suffix(".glb").unwrap_or(g).to_owned() + ".hprop").collect()
    }

    /// A hand prop's index in [`Self::hand_prop_models`].
    pub(crate) fn hand_prop_model(&self, key: &str) -> Option<u16> {
        self.hand_props.props.iter().filter(|(_, r)| r.glb.is_some()).position(|(k, _)| k == key).map(|i| i as u16)
    }
}

#[cfg(test)]
mod memory_tests {
    fn rss() -> usize {
        let statm = std::fs::read_to_string("/proc/self/statm").unwrap();
        statm.split_whitespace().nth(1).unwrap().parse::<usize>().unwrap() * 4096
    }

    /// Measurement: what the ped data costs in memory (process resident size around each load).
    #[test]
    #[ignore = "requires the living-world export (SKATE3_ASSET_ROOT)"]
    fn ped_data_memory() {
        let root = std::path::PathBuf::from(std::env::var_os("SKATE3_ASSET_ROOT").unwrap());
        let r0 = rss();
        let mut data = super::PedData::load(&root);
        let r1 = rss();
        data.load_nav(&root, "DownTown", &Default::default());
        let r2 = rss();
        let clips: usize = data.clips.len();
        if let Some(nav) = &data.nav {
            let verts: usize = nav.polys.iter().map(|p| p.verts.len()).sum();
            let bytes: usize = nav.polys.iter().map(|p| p.verts.len() * 12 + p.neighbours.len() * 8 + 48 + std::mem::size_of::<skate_core::living_world::peds::nav::NavPoly>()).sum();
            eprintln!("navmesh: {} polygons, {verts} vertices, about {:.1} MB in polygons (links, grid and the kept input on top)", nav.polys.len(), bytes as f64 / 1e6);
        }
        eprintln!("ped bank + tables + {clips} clips: {:.1} MB; DownTown navmesh ({}): {:.1} MB", (r1 - r0) as f64 / 1e6, data.status, (r2 - r1) as f64 / 1e6);
    }
}

// ---- The behaviour runtime (upstream `think_peds` and its resources) ----

/// skate_vita: upstream `think_peds`' `Local`s and its ped event cursor.
#[derive(Default)]
pub(crate) struct ThinkLocals {
    /// `greeted` events for other peds' mood stores (posted when the target thinks next).
    greeted: Vec<(u64, u64, [f32; 3])>,
    /// Conversation listeners that get the speech value 41 when they think next (`82E3DAD8`).
    listening: Vec<u64>,
    touchers: BTreeMap<u64, u64>,
    /// [`PedState::events`] before this index were read (the chain drops them before the next tick).
    pub cursor: usize,
}


/// The world props offering ped plugins on the current map (host-owned; `skate_core::living_world::peds::plugins`).
#[derive(Default)]
pub(crate) struct PedPluginProps {
    pub key: Option<(String, u64)>,
    pub props: Vec<skate_core::living_world::peds::plugins::PluginProp>,
    pub settings: skate_core::living_world::peds::plugins::PluginSettings,
    /// World ticks since the last offer scan (`826C0058` every 11th tick) and the last world tick seen.
    pub scan_ticks: u32,
    pub last_tick: Option<u64>,
    pub rng: Option<skate_core::living_world::Rng>,
}

/// The live conversations by id (host-owned plain data, `skate_core::living_world::peds::conversation`).
#[derive(Default, Debug)]
pub(crate) struct PedConversations {
    pub map: BTreeMap<u64, skate_core::living_world::peds::conversation::Conversation>,
    pub next_id: u64,
    rng: Option<skate_core::living_world::Rng>,
}

/// The chase groups by chasee id (host-owned; retail keeps one on every chasee: the player's
/// actor and each ped).
#[derive(Default, Debug)]
pub(crate) struct PedChaseGroups(pub BTreeMap<u64, skate_core::living_world::peds::chase::ChaseGroup>);

/// A ped took player `player` down from `chaser` (retail `T.vfn20(chaser position)`, `82592390`):
/// the skater's takedown latch. Host-owned; a mod can write it too.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SkaterTakedownRequest {
    pub player: usize,
    pub chaser: [f32; 3],
}

/// Set the skater's takedown latch (direction from the chaser to the deck). skate_vita: over the requests
/// [`think_peds`] wrote this tick.
pub(crate) fn apply_skater_takedowns(requests: &mut Vec<SkaterTakedownRequest>, skater: &mut crate::physics::SkaterRuntime, physics: &crate::physics::GamePhysics) {
    for r in requests.drain(..) {
        // Local play: player 0 is the simulated skater.
        if r.player != 0 {
            continue;
        }
        let deck = physics.board.bodies()[skate_core::physics::board::BodyId::Deck.index()].rates.position;
        skater.takedown = Some(crate::physics::Takedown::from_positions([deck.x, deck.y, deck.z], r.chaser));
        info!("SKATER_TAKEDOWN from=[{:.1}, {:.1}, {:.1}]", r.chaser[0], r.chaser[1], r.chaser[2]);
    }
}

/// `PedMind::plugin_graph` of the conversation graph (prop graphs use their class name).
const CONVERSATION_GRAPH: &str = "conversation";

/// The plugin controller for graph `key` with `states` states: kept while the graph is the same, made fresh when there
/// is none or the ped switched graphs (a controller only fits the graph it was sized for). Returns the old graph's key
/// on a switch.
pub(crate) fn plugin_controller_for(
    slot: &mut Option<skate_core::graph::controller::Controller>,
    graph: &mut Option<String>,
    key: &str,
    states: usize,
) -> Option<String> {
    let switched = graph.as_deref().is_some_and(|g| g != key);
    if switched || slot.is_none() {
        *slot = Some(skate_core::graph::controller::Controller::new(states));
    }
    let old = graph.replace(key.to_owned());
    if switched { old } else { None }
}

/// Run every ped's AI graph once per population tick (before the bodies step), in id order.
/// Wants come from the mood system or a mod; without them a ped wanders as before. Logs `PED_BRAIN` on every
/// state change.
///
/// skate_vita: upstream's `think_peds` verbatim over plain data: the ped query is the ped index (id
/// order), the messages are [`PedState`] vectors (the ped events read through [`ThinkLocals::cursor`]), the `Local`s
/// are [`ThinkLocals`]; no speech or tazer sound (the Vita has no ped audio), no still-ped reports; the client role
/// does not exist (single player).
#[allow(clippy::too_many_arguments)]
pub(crate) fn think_peds(
    state: &PopulationState,
    peds: &mut PedState,
    settings: &LivingWorldSettings,
    observers: &[skate_core::living_world::Observer],
    wiping: &[bool],
    traffic_events: &[super::vehicles::TrafficEvent],
    cars: &[(u64, [f32; 3], [f32; 3])],
    physics: Option<&crate::physics::GamePhysics>,
) {
    let PedState { data, index, events, chase_groups, conversations, plugin_props, think, takedowns, .. } = peds;
    let data = &*data;
    let (greeted, listening, touchers) = (&mut think.greeted, &mut think.listening, &mut think.touchers);
    let _scope = skate_core::profile::scope("z_think_peds");
    // Streamed-out props (DMO streaming): their plugins are not offered and a ped using one lets go. A world prop's
    // plugin id carries its DMO body id in the low 32 bits (`ped_plugins::map_props`).
    let dormant = physics.as_ref().and_then(|p| p.prop_dynamics()).map(|d| d.dormant_ids()).unwrap_or_default();
    let streamed_out = |plugin_id: u64| dormant.contains(&(plugin_id as u32));
    // Peds whose seat's prop moved this tick (grabbed, pushed, knocked): `82E3DBE0` fires the seated owner's avoid.
    let mut seat_pushed: Vec<u64> = Vec::new();
    // Horn kind 2 at a ped (`sub_82E3C3D0`: the honker id into brain `+3232`; the last car wins).
    let honked: BTreeMap<u64, u64> = traffic_events.iter().filter_map(|e| match e { super::vehicles::TrafficEvent::HonkedAt { id, ped } => Some((*ped, id.to_u64())), _ => None }).collect();
    let Some(graph) = data.graph.as_deref() else {
        think.cursor = events.len();
        return;
    };
    // The honking cars' pose for RunFromHonker (`826A1358` looks the car up every frame): position and forward.
    let car_pose: BTreeMap<u64, ([f32; 3], [f32; 3])> = cars.iter().map(|&(id, at, forward)| (id, (at, forward))).collect();
    // Collisions from the skater contact of the last steps (`PedEvent::Hit`): retail posts
    // `collision` to every receiver (broadcast radius 0 = unlimited); a ped that was not the one
    // hit fails its "ped itself" prerequisite and records `nearbycollision` instead
    // (`sub_82E41060`). The instigator is the ped's last toucher (the contact record `+3184`), kept until its
    // Colliding state begins.
    // A reacting contact sets the hit ped's collision component (`sub_82E38FB8`: needs-to-begin); the mood broadcast
    // waits for its Colliding state to begin (`PedEvent::Colliding`), so a ped whose state does not allow Colliding
    // (a non-interruptible chase, a plugin with the collision override) posts nothing yet.
    let (mut hits, mut collisions) = (Vec::<u64>::new(), Vec::<(u64, Option<u64>)>::new());
    let read_to = events.len();
    for e in &events[think.cursor.min(read_to)..read_to] {
        match e {
            PedEvent::Hit { id, toucher, .. } => {
                hits.push(id.to_u64());
                touchers.insert(id.to_u64(), *toucher);
            }
            PedEvent::Colliding { id } => collisions.push((id.to_u64(), touchers.remove(&id.to_u64()))),
            _ => {}
        }
    }
    think.cursor = read_to;
    if !settings.ped_brain.enabled {
        return;
    }
    let program = &graph.graph.runtime.program;
    let dt = tick_seconds(state.world.clock().hz);
    let tick = state.world.tick();
    let players: Vec<[f32; 3]> = observers.iter().map(|o| o.position).collect();
    // skate_vita: the ped index is in id order (upstream sorts the query).
    let mut list: Vec<(LivingWorldId, &Pedestrian, &mut PedBody, &mut PedMind)> = index
        .iter_mut()
        .map(|(id, e)| {
            let PedEntity { ped, body, mind, .. } = e;
            (*id, &*ped, body, mind)
        })
        .collect();
    // Entity type and position of every id this tick (players are the `skater` type).
    let entities: BTreeMap<u64, (String, [f32; 3])> = list
        .iter()
        .map(|(_, p, b, _)| (p.id.to_u64(), (p.entity.clone(), b.position.to_array())))
        .chain(players.iter().enumerate().map(|(i, p)| (PLAYER_TARGET_BASE + i as u64, ("skater".to_string(), *p))))
        .collect();
    // A hit ped that is gone no longer waits for its Colliding state.
    touchers.retain(|id, _| entities.contains_key(id));
    // Velocity of every id this tick (the attack throw's look-ahead).
    let velocities: BTreeMap<u64, [f32; 3]> = list
        .iter()
        .map(|(_, p, b, _)| (p.id.to_u64(), b.velocity.to_array()))
        .chain(observers.iter().enumerate().map(|(i, o)| (PLAYER_TARGET_BASE + i as u64, o.velocity)))
        .collect();
    let velocity_of = |id: u64| velocities.get(&id).copied();
    // World-prop plugins: the map's props, then the offer scan every 11th world tick (`826C0058` -> `826BFE18`).
    {
        let pp = &mut *plugin_props;
        let key = data.loaded_for.clone();
        if pp.key != key {
            pp.props = key.as_ref().and_then(|k| data.plugins.placed.get(&k.0)).cloned().unwrap_or_default();
            pp.key = key;
            pp.scan_ticks = 0;
            info!("PED_PLUGINS props on this map: {}", pp.props.len());
        }
        // Hotpoint seats ride on their object (a prop's plugin id carries its DMO body id in the low 32 bits): every
        // think they follow the body from its authored pose, so a moved bench takes its seats along.
        if let (Some(authored), Some(dynamics)) = (pp.key.as_ref().and_then(|k| data.plugins.placed.get(&k.0)), physics.as_ref().and_then(|p| p.prop_dynamics())) {
            let pose = |(o, b): (skate_core::math::Vector3, skate_core::math::Basis3)| ([o.x, o.y, o.z], b.columns);
            for (prop, original) in pp.props.iter_mut().zip(authored) {
                let body = prop.id as u32;
                let (Some(spawn), Some(now)) = (dynamics.spawn_pose(body), dynamics.pose(body)) else { continue };
                let (spawn, now) = (pose(spawn), pose(now));
                for (w, a) in prop.waypoints.iter_mut().zip(&original.waypoints) {
                    let (position, facing) = super::ped_plugins::follow_prop(a.position, a.facing, spawn, now);
                    // A seat whose prop moves (grabbed, pushed, knocked): retail keeps the ped's plugin and seat
                    // (research b102). The owned object reporting a move to its seated ped (`82E3DBE0`: |v|^2 > 2.5e-5,
                    // `0x822F8BE0`) makes it jump off forward; the graph then walks it to the seat's new place and sits
                    // it again (the locked waypoint is re-read every evaluation, `826AB9C0`). The seat's own speed stands
                    // in for the object's vector (open: velocity or step).
                    let step = (0..3).map(|k| (position[k] - w.position[k]).powi(2)).sum::<f32>().sqrt();
                    let speed = step / dt.max(1e-4);
                    if let Some(occupant) = w.occupant.filter(|_| speed * speed > SEAT_PUSH_SPEED_SQ) {
                        info!("PED_PLUGIN_SEAT_MOVED prop={:016X} {} occupant={occupant} speed={speed:.2} from=[{:.2}, {:.2}, {:.2}] to=[{:.2}, {:.2}, {:.2}] tick={tick}", prop.id, prop.class, w.position[0], w.position[1], w.position[2], position[0], position[1], position[2]);
                        seat_pushed.push(occupant);
                    }
                    (w.position, w.facing) = (position, facing);
                }
                // The DMO's plugin check (`82C56DB0`, b136 / b137): moved more than its type's max distance, or turned
                // past its per-axis tolerance -> off (seated peds evicted); else each seat must stand on pavement navmesh.
                // NOT RETAIL YET: when retail runs it (DMO vtable +264's caller) is open, likely on a move / settle
                // message; ours every think, only for a prop away from its authored pose.
                if let Some((tolerance, max_distance)) = dynamics.plugin_check_limits(body) {
                    use skate_core::living_world::peds::plugins::{check_prop, PropCheck, ROTATION_EPSILON};
                    let moved = (0..3).any(|k| (now.0[k] - spawn.0[k]).abs() > 1e-3)
                        || (0..3).any(|c| (0..3).any(|r| (now.1[c][r] - spawn.1[c][r]).abs() > ROTATION_EPSILON));
                    // The per-seat pavement probe (`82C539A8`) is not applied yet: our probe fails 10-23% of untouched
                    // authored seats (test `living_world_authored_plugin_seats_pass_the_pavement_probe`), so it is not
                    // retail's yet; a moved, upright prop keeps its seats as before.
                    let off = moved && check_prop(spawn, now, tolerance, max_distance) == PropCheck::DisableAll;
                    if off != prop.disabled {
                        prop.disabled = off;
                        info!("PED_PLUGIN_PROP {} {:016X} {} tick={tick}", prop.class, prop.id, if off { "disabled (moved / tipped), occupants evicted" } else { "enabled again" });
                    }
                }
            }
        }
        let seed = state.world.seed();
        let rng = pp.rng.get_or_insert_with(|| skate_core::living_world::Rng::new(skate_core::living_world::rng::derive(seed, &[0x504c_5547])));
        let elapsed = pp.last_tick.map_or(1, |t| tick.saturating_sub(t)) as u32;
        pp.last_tick = Some(tick);
        pp.scan_ticks += elapsed;
        // Waypoints held by peds that are gone or no longer on the prop are released.
        let holding: Vec<(u64, u64)> = list.iter().filter_map(|(_, p, _, m)| m.prop.map(|(_, id, _)| (p.id.to_u64(), id))).collect();
        for prop in &mut pp.props {
            for w in &mut prop.waypoints {
                if let Some(o) = w.occupant.filter(|o| !holding.contains(&(*o, prop.id))) {
                    info!("PED_PLUGIN_PROP {} {:016X} released: ped {o} gone tick={tick}", prop.class, prop.id);
                    w.occupant = None;
                }
            }
        }
        while pp.scan_ticks >= pp.settings.scan_period.max(1) {
            pp.scan_ticks -= pp.settings.scan_period.max(1);
            let mut attaches = Vec::new();
            {
                // The descriptors' transfer expressions (`UseWorldProp`: e.g. usetrashbin needs a disposable hand prop).
                let views: std::collections::HashMap<u64, skate_core::living_world::peds::plugins::TransferView> =
                    list.iter().map(|(_, p, _, m)| (p.id.to_u64(), skate_core::living_world::peds::plugins::TransferView::of(&m.brain))).collect();
                let offer_settings = pp.settings;
                let mut qualifies = |prop: &skate_core::living_world::peds::plugins::PluginProp, ped: &skate_core::living_world::peds::plugins::OfferPed| {
                    let view = views.get(&ped.id).copied().unwrap_or_default();
                    let transfer = data.plugins.classes.get(&prop.class).and_then(|c| c.descriptor.as_ref()).and_then(|d| d.transfer.as_ref());
                    transfer.is_none_or(|t| t.evaluate(&mut |name, _| view.condition(name, prop, ped.position, &offer_settings)))
                };
                let mut offer_peds: Vec<skate_core::living_world::peds::plugins::OfferPed> = list
                    .iter_mut()
                    .map(|(_, p, b, m)| {
                        let m = &mut **m;
                        skate_core::living_world::peds::plugins::OfferPed {
                            id: p.id.to_u64(),
                            position: b.position.to_array(),
                            has_plugin: m.brain.has_plugin || m.prop.is_some(),
                            odds: data.plugins.odds.get(&p.entity).map(Vec::as_slice).unwrap_or(&[]),
                            memory: &mut m.refusals,
                        }
                    })
                    .collect();
                for (index, prop) in pp.props.iter_mut().enumerate() {
                    if !data.plugin_graphs.contains_key(&prop.class) || streamed_out(prop.id) || prop.disabled {
                        continue;
                    }
                    let class = data.plugins.classes.get(&prop.class);
                    // [inference] the offer radius: the class field E2101F2B17A0E6B5 (10 sit / 5 bin / 15 ATM).
                    let radius = class.and_then(|c| c.numbers.get("Hash_E2101F2B17A0E6B5").copied()).unwrap_or(10.0);
                    let max = class.and_then(|c| c.descriptor.as_ref()).map_or(1, |d| d.max_participants);
                    for a in skate_core::living_world::peds::plugins::offer(prop, radius, max, &mut offer_peds, &mut qualifies, &pp.settings, rng) {
                        attaches.push((index, a));
                    }
                }
            }
            for (index, a) in attaches {
                if let Some((_, p, _, m)) = list.iter_mut().find(|(_, p, ..)| p.id.to_u64() == a.ped) {
                    m.prop = Some((index, a.prop, a.waypoint));
                    info!("PED_PLUGIN_PROP ped=#{} takes {} {:016X} waypoint {} tick={tick}", p.id.serial, pp.props[index].class, a.prop, a.waypoint);
                }
            }
        }
    }
    let entity = |id: u64| entities.get(&id).cloned();
    let target = |id: u64| entities.get(&id).map(|e| e.1);
    let player_list: Vec<(u64, [f32; 3])> = players.iter().enumerate().map(|(i, p)| (PLAYER_TARGET_BASE + i as u64, *p)).collect();
    let world_seed = state.world.seed();
    // Live peds for the presence scan (`sub_82E3CA20`: every spawned, not torn-down ped counts).
    let ped_list: Vec<(u64, [f32; 3])> = list.iter().map(|(_, p, b, _)| (p.id.to_u64(), b.position.to_array())).collect();
    let greeted_now = std::mem::take(&mut *greeted);
    // Who is busy this tick (check 8 of the mood results; players: not busy, unverified).
    let busy_now: BTreeMap<u64, bool> = list.iter().map(|(_, p, _, m)| (p.id.to_u64(), m.brain.busy())).collect();
    let busy = |id: u64| busy_now.get(&id).copied().unwrap_or(false);
    // Chasers that left the world leave their groups (the ped's destruction; retail removes it
    // with the chaser component).
    for g in chase_groups.0.values_mut() {
        g.chasers.retain(|c| entities.contains_key(c));
    }
    chase_groups.0.retain(|chasee, g| !g.chasers.is_empty() && entities.contains_key(chasee));
    // Conversations: members that left the world leave (an unfinished conversation aborts); an
    // empty one is gone (every area is spawned: `82E1BD08` removes it).
    for c in conversations.map.values_mut() {
        let gone: Vec<u64> = c.members.iter().map(|m| m.ped).filter(|p| !entities.contains_key(p)).collect();
        for p in gone {
            c.leave(p);
        }
    }
    conversations.map.retain(|_, c| !c.members.is_empty());
    listening.retain(|p| entities.contains_key(p));
    let conversations = &mut *conversations;
    let conv_rng_seed = world_seed;
    let conv_rng = conversations.rng.get_or_insert_with(|| skate_core::living_world::Rng::new(skate_core::living_world::rng::derive(conv_rng_seed, &[0x434f_4e56])));
    let mut conv_rand = || conv_rng.unit();
    let conv_params = skate_core::living_world::peds::conversation::ConversationParams {
        turn_seconds: settings.ped_brain.values.conversation_turn_seconds,
        gather_seconds: settings.ped_brain.values.conversation_gather_seconds,
        ..Default::default()
    };
    // The gather timer (`82E1EB10`): a conversation starts with whoever is there when it runs out.
    for c in conversations.map.values_mut() {
        if c.tick(dt, &data.conversations.rows, &mut conv_rand) {
            info!("PED_CONVERSATION id={} gather_timeout state={} members={} tick={tick}", c.id, c.state, c.members.len());
        }
    }
    // How many chasers a chasee takes: every chase group (skater and ped alike) reads `global`, or `zombie` under the
    // zombie cheat (group init `82D967B0`, b105).
    let group_record = if settings.zombie { data.chase_zombie.as_deref() } else { data.chase_global.as_deref() };
    let max_chasers = |_chasee: u64| skate_core::living_world::peds::chase::ChaseRecord::max_chasers(group_record);
    for (_entity_id, ped, body, mind) in list {
        let mind = &mut *mind;
        let body = &mut *body;
        let me = ped.id.to_u64();
        let at = body.position.to_array();
        // Perception (`82E418E8`): memory, age and suppression run, the vision test from the eye
        // (ours: 1.6 m above the feet; retail's eye point is set elsewhere) along the heading; the
        // line of sight is our navmesh line (retail: a physics ray).
        {
            let steps = tick.saturating_sub(ped.spawn_tick).saturating_sub(mind.ticks) as f32;
            if steps > 0.0 && !mind.brain.perceptions.entries.is_empty() {
                use skate_core::living_world::peds::perception;
                let sight = data.sight.get(&ped.entity).copied();
                let eye = [at[0], at[1] + 1.6, at[2]];
                let forward = [body.heading.sin(), 0.0, body.heading.cos()];
                let here = data.nav.as_deref().and_then(|m| m.locate(at).map(|h| (m, h)));
                let los = |_: [f32; 3], t: [f32; 3]| here.is_none_or(|(m, h)| m.clear_line(h, t));
                let look = |t: u64| {
                    let p = target(t)?;
                    let v = observers.get(t.wrapping_sub(PLAYER_TARGET_BASE) as usize).map_or([0.0; 3], |o| o.velocity);
                    Some((p, v, sight.is_some_and(|s| perception::sees(at, eye, forward, p, &s, &los))))
                };
                mind.brain.perceptions.tick(dt * steps, &look);
            }
        }
        // Mood: tick the records, post presence and collisions, produce wants.
        if let (Some(tables), true) = (data.mood.as_deref(), settings.ped_brain.mood) {
            use skate_core::living_world::peds::mood::{self, MoodContext, MoodEvent};
            let set = data.reaction_sets.get(&ped.entity).cloned().unwrap_or_else(|| "default".into());
            let magnitude = |c: &str| tables.categories.get(c).map_or(0.0, |c| c.magnitude);
            let lifetime = |c: &str| tables.categories.get(c).map_or(30.0, |c| c.lifetime);
            // The post's vision test (`sub_82E41060` -> `82E27240`, b5 / b18): the event position against the
            // category's near / far ranges times the type's scales, the same eye, cone and line as the perception
            // pass. A type without a perception record keeps every post (retail's record default is unread).
            let sight_scales = data.sight_scales.get(&ped.entity).copied();
            let post_eye = [at[0], at[1] + 1.6, at[2]];
            let post_forward = [body.heading.sin(), 0.0, body.heading.cos()];
            let post_here = data.nav.as_deref().and_then(|m| m.locate(at).map(|h| (m, h)));
            let post_los = |_: [f32; 3], t: [f32; 3]| post_here.is_none_or(|(m, h)| m.clear_line(h, t));
            let seen = |e: &MoodEvent| match (sight_scales, tables.categories.get(&e.category)) {
                (Some(s), Some(c)) => skate_core::living_world::peds::perception::sees(
                    at,
                    post_eye,
                    post_forward,
                    e.position,
                    &skate_core::living_world::peds::perception::Sight::new(c.near, c.far, s[0], s[1], s[2]),
                    &post_los,
                ),
                _ => true,
            };
            let steps = tick.saturating_sub(ped.spawn_tick).saturating_sub(mind.ticks) as f32;
            if steps > 0.0 {
                mind.mood.tick(dt * steps, &lifetime);
                mind.presence_timer -= dt * steps;
                if mind.presence_timer <= 0.0 {
                    mind.presence_timer = magnitude(mood::category::PRESENCE).max(dt);
                    // `sub_82E41060` drops a post whose prerequisites fail (presence: within 12 m in
                    // the stock sets).
                    let post_ctx = MoodContext { ped: me, ped_type: &set, position: at, entity: &entity, zombie: settings.zombie, busy: &busy, own_plugin_object: mind.brain.own_plugin_object, chasing: mind.brain.chasee.is_some() };
                    for e in mood::presence(me, at, &ped_list, &player_list) {
                        // A suppressed entity raises no mood (perception `+64`).
                        if !e.instigator.is_some_and(|i| mind.brain.perceptions.suppressed(i)) && tables.accepts(&set, &e, &post_ctx) && seen(&e) {
                            mind.mood.post(e, magnitude(mood::category::PRESENCE));
                        }
                    }
                }
                // The zombie broadcast (`sub_82E24B60`): every console frame while the cheat is on.
                if settings.zombie {
                    mind.zombie_timer -= dt * steps;
                    let mut posts = 0;
                    while mind.zombie_timer <= 0.0 && posts < 8 {
                        mind.zombie_timer += mood::BROADCAST_PERIOD;
                        posts += 1;
                    }
                    if posts > 0 {
                        let post_ctx = MoodContext { ped: me, ped_type: &set, position: at, entity: &entity, zombie: true, busy: &busy, own_plugin_object: mind.brain.own_plugin_object, chasing: mind.brain.chasee.is_some() };
                        for e in mood::zombie_mode(&player_list) {
                            let suppressed = e.instigator.is_some_and(|i| mind.brain.perceptions.suppressed(i));
                            let in_sight = seen(&e);
                                let accepted = !suppressed && in_sight && tables.accepts(&set, &e, &post_ctx);
                            if let Some(held) = mind.log_repeats.allow(0x5a4f_4d42 ^ u64::from(accepted), tick) {
                                info!(
                                    "PED_ZOMBIE_POST ped=#{} {set} accepted={accepted} seen={in_sight} suppressed={suppressed} posts={posts} busy={} tick={tick}{}",
                                    ped.id.serial,
                                    mind.brain.busy(),
                                    if held > 0 { format!(" (+{held} held back)") } else { String::new() }
                                );
                            }
                            if accepted {
                                for _ in 0..posts {
                                    mind.mood.post(e.clone(), magnitude(mood::category::ZOMBIE_MODE));
                                }
                            }
                        }
                    }
                } else {
                    mind.zombie_timer = 0.0;
                }
            }
            if let Some(&car) = honked.get(&me) {
                mind.brain.honker = Some(car);
            }
            if let Some(i) = listening.iter().position(|&p| p == me) {
                listening.swap_remove(i);
                mind.brain.speech = Some(skate_core::living_world::peds::conversation::LISTENER_SPEECH);
            }
            // Every post runs the ped's prerequisites (`sub_82E41060`).
            let post_ctx = MoodContext { ped: me, ped_type: &set, position: at, entity: &entity, zombie: settings.zombie, busy: &busy, own_plugin_object: mind.brain.own_plugin_object, chasing: mind.brain.chasee.is_some() };
            // ChannelGreetWantTarget's `greeted` (magnitude 1.0, `8269FAC0`).
            for &(_, instigator, position) in greeted_now.iter().filter(|g| g.0 == me) {
                let e = MoodEvent { category: mood::category::GREETED.into(), instigator: Some(instigator), second: Some(me), position };
                if !mind.brain.perceptions.suppressed(instigator) && tables.accepts(&set, &e, &post_ctx) && seen(&e) {
                    mind.mood.post(e, 1.0);
                }
            }
            if hits.contains(&me) {
                mind.brain.begin_colliding = true;
                mind.brain.colliding = true;
            }
            // IsPedestrianColliding holds while the contact reaction plays.
            if mind.brain.colliding && !body.player.reacting() {
                mind.brain.colliding = false;
            }
            for &(hit, toucher) in &collisions {
                let instigator = toucher.or_else(|| player_list.first().map(|p| p.0));
                if instigator.is_some_and(|i| mind.brain.perceptions.suppressed(i) || mind.brain.ignore_collisions_with == Some(i)) {
                    continue;
                }
                // Event origin (`sub_8269D8A8`: the contact's entity position): the hit ped's, so bystanders run the
                // vision test against it [inferred: which entity +188 is was not read].
                let origin = entity(hit).map_or(at, |(_, p)| p);
                // `sub_82E41060`: the contact is posted as `collision`; when the prerequisites refuse it and the hit ped
                // is another ped, it is posted again as `nearbycollision` (stock sets refuse a bystander's `collision`
                // with kind 6; the security sets have no such item, so their guards react to `collision`).
                let mut e = MoodEvent { category: mood::category::COLLISION.into(), instigator, second: Some(hit), position: origin };
                if !tables.accepts(&set, &e, &post_ctx) {
                    if hit == me {
                        continue;
                    }
                    e.category = mood::category::NEARBY_COLLISION.into();
                    if !tables.accepts(&set, &e, &post_ctx) {
                        continue;
                    }
                }
                if seen(&e) {
                    let m = magnitude(&e.category);
                    mind.mood.post(e, m);
                }
            }
            // The brain's own rolls (sit time, stand-up) and its type's sit values, once per ped.
            if mind.brain.rng.is_none() {
                mind.brain.rng = Some(skate_core::living_world::Rng::new(skate_core::living_world::rng::derive(world_seed, &[0x4252_4e52, me])));
                mind.brain.sit = entity(me).and_then(|(name, _)| data.sit.get(name.as_str()).copied()).unwrap_or_default();
                // The starting hand prop (ped constructor `82E33198`): the type's chance, then its weighted list.
                let start = entity(me).and_then(|(name, _)| data.starting_props.get(name.as_str()));
                if let (Some((chance, list)), true) = (start, settings.ped_brain.values.hand_prop.starting_props) {
                    let r = mind.brain.rng.as_mut().expect("seeded above");
                    let (a, b) = (r.modulo(100) + 1, r.modulo(100) + 1);
                    if let Some(key) = skate_core::living_world::peds::brain::HandProp::starting_pick(*chance, list, a, b).map(str::to_string) {
                        mind.brain.hand_prop.request(&key);
                        if let Some(r) = data.hand_props.props.get(&key) {
                            let h = &mut mind.brain.hand_prop;
                            (h.disposable, h.can_sit, h.can_attack_throw) = (r.disposable, r.can_sit, r.can_attack_throw);
                        }
                        info!("PED_HAND_PROP ped=#{} requested {key} at spawn tick={tick}", ped.id.serial);
                    }
                }
            }
            let rng = mind.rng.get_or_insert_with(|| skate_core::living_world::Rng::new(skate_core::living_world::rng::derive(world_seed, &[0x4d4f_4f44, me])));
            let ctx = MoodContext { ped: me, ped_type: &set, position: at, entity: &entity, zombie: settings.zombie, busy: &busy, own_plugin_object: mind.brain.own_plugin_object, chasing: mind.brain.chasee.is_some() };
            let brain = &mind.brain;
            // `7FF7`: the reactions about the instigator the ped has finished (its perception entry `+68`, `82E42A58`).
            let outstanding = |id: u64| brain.perceptions.get(id).map_or(0, |e| e.reacted);
            let pending = |w: &str| brain.wants.get(w).is_some_and(|x| x.needs_addressing);
            // `sub_82E41B98` does nothing for a busy ped (`sub_82E3BF70`); a pass sets WaitingToReact.
            // `072D`: the hand-prop bit (`brain+3278` 0x02, set while a hand prop is attached, `82E3EC60` / `82E3EBE0`).
            let produced = if brain.busy() { None } else { tables.produce(&mut mind.mood, &ctx, brain.hand_prop.holding, &outstanding, &pending, rng) };
            if let Some(reaction) = produced {
                let key = reaction.result.bytes().chain([0]).chain(reaction.category.bytes()).fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100_0000_01b3)) & !(1 << 63);
                if let Some(held) = mind.log_repeats.allow(key, tick) {
                    info!(
                        "PED_MOOD ped=#{} {} result={} category={} roll={:?} passed={} wants={:?} tick={tick}{}",
                        ped.id.serial,
                        set,
                        reaction.result,
                        reaction.category,
                        reaction.rolled,
                        reaction.passed,
                        reaction.wants.iter().map(|w| w.want.as_str()).collect::<Vec<_>>(),
                        if held > 0 { format!(" (+{held} held back)") } else { String::new() }
                    );
                }
                if reaction.passed {
                    mind.brain.waiting_to_react = true;
                }
                for w in &reaction.wants {
                    mind.brain.raise_mood_want(&w.want, w.target, w.flag_40, settings.ped_brain.values.mood_want_memory_seconds);
                }
                events.push(PedEvent::Mood { id: ped.id, result: reaction.result.clone(), passed: reaction.passed, wants: reaction.wants.iter().map(|w| w.want.clone()).collect() });
            }
        }
        let due = tick.saturating_sub(ped.spawn_tick);
        let controller = mind.controller.get_or_insert_with(|| skate_core::graph::controller::Controller::new(program.topology.states.len()));
        // The ped's own chase record: its type's, or `zombie` under the zombie cheat (`82E33FAC`, b105).
        let record = if settings.zombie { data.chase_zombie.as_deref() } else { data.chase.get(&ped.entity) };
        let ped_speed = mind.last_at.map_or(0.0, |l| ((at[0] - l[0]).powi(2) + (at[2] - l[2]).powi(2)).sqrt() / (dt * tick.saturating_sub(ped.spawn_tick).saturating_sub(mind.ticks).max(1) as f32));
        mind.last_at = Some(at);
        // The takedown contact (`82E38FB8` records a touch of the takedown target and counts it,
        // `brain+3248`): our skater cylinder against the ped while the takedown plays.
        if mind.brain.takedown_active && !mind.brain.takedown_contact {
            let ped_radius = data.nav.as_deref().map_or(super::vehicle_contacts::FALLBACK_PED_RADIUS, |m| m.agent[1]);
            if let Some(p) = mind.brain.takedown_target.filter(|t| *t >= PLAYER_TARGET_BASE).and_then(|t| target(t)) {
                let d = ((p[0] - at[0]).powi(2) + (p[2] - at[2]).powi(2)).sqrt();
                if d < ped_radius + SKATER_CONTACT_RADIUS && (p[1] - at[1]).abs() <= 2.0 {
                    mind.brain.takedown_contact = true;
                    mind.brain.takedowns += 1;
                }
            }
        }
        // HasLineOfSightToTazeTarget (`+3280` bit 0x10, retail's batched ray pass `82E23D68`):
        // ours is the navmesh line to the tazer want's target while the tazer is out.
        if mind.brain.tazer_drawn_requested {
            let goal = mind.brain.tazer_want.as_ref().and_then(|w| mind.brain.wants.get(w)).and_then(|w| target(w.target));
            mind.brain.tazer_line_of_sight = match (goal, data.nav.as_deref()) {
                (Some(g), Some(mesh)) => mesh.locate(at).is_some_and(|h| mesh.clear_line(h, g)),
                (Some(_), None) => true,
                _ => false,
            };
        }
        let takedown_table = if settings.zombie { data.takedown_zombie.as_deref() } else { data.takedowns.get(&ped.entity) };
        let forward = [body.heading.sin(), body.heading.cos()];
        let choose = |t: u64| {
            let (table, p) = (takedown_table?, target(t)?);
            let v = observers.get(t.wrapping_sub(PLAYER_TARGET_BASE) as usize).map_or([0.0; 3], |o| o.velocity);
            skate_core::living_world::peds::takedown::choose(table, at, forward, ped_speed, p, v)
        };
        // LockToCurrentPosition: the body stays on the locked point (root motion does not move it).
        if let Some(p) = mind.brain.locked_at {
            body.position.x = p[0];
            body.position.z = p[2];
        }
        while mind.ticks < due {
            mind.ticks += 1;
            mind.brain.tick_timers(dt);
            // The taunt clip ended (`MajorIntentComplete`): the intent goes, the graph leaves DoTaunt.
            if body.taunt == TauntClip::Done {
                body.taunt = TauntClip::None;
                mind.brain.monitored.remove("SGIntent");
            }
            // ApproachWantTarget sets its goal each tick it runs.
            mind.brain.approach = None;
            mind.brain.approach_slide = None;
            // HasPlugin (`826A99C0`): the ped is a participant of a live conversation.
            if mind.brain.plugin.is_none_or(|id| !conversations.map.get(&id).is_some_and(|c| c.members.iter().any(|m| m.ped == me))) {
                mind.brain.plugin = conversations.map.values().find(|c| c.members.iter().any(|m| m.ped == me)).map(|c| c.id);
            }
            mind.refusals.tick(dt, &plugin_props.settings);
            // The Plugin behaviour's Update (`8269F248` -> `82E40BD0`, b104) re-stores the 30 s refusal for its prop every
            // tick it runs, so a ped that leaves a prop is not offered it again for 30 s.
            if mind.brain.in_plugin {
                if let Some(prop) = mind.prop.and_then(|(i, ..)| plugin_props.props.get(i)) {
                    mind.refusals.remember(prop.id, &plugin_props.settings);
                }
            }
            // `82E3DBE0`: the seated owner (OwnPluginObject, `brain+3277` bit 0x01) of a moving object jumps off forward:
            // intent Avoid / Jump / Forward -> motiongraph_avoid SitJump -> JumpForward `SitAvoidFwd` (blend 0.1), speech
            // PedestrianAvoidJump (4). The gate `[ped+5712]+1416 == 0` is not decoded (taken as open).
            // The body plays it (`advance_peds`) while the flag is up.
            let avoid = seat_pushed.contains(&me) && mind.brain.own_plugin_object && body.player.reaction_anim() == Some(SIT_CYCLE);
            if avoid && !mind.sit_avoid {
                mind.brain.speech = Some(SIT_AVOID_SPEECH);
                info!("PED_SIT_AVOID ped=#{} jumps off forward (its seat moved) tick={tick}", ped.id.serial);
            }
            mind.sit_avoid = avoid;
            // HasPlugin: a conversation member, or a ped that took a world prop.
            let prop_wp = mind.prop.and_then(|(i, id, w)| plugin_props.props.get(i).filter(|p| p.id == id && !streamed_out(id) && !p.disabled).and_then(|p| p.waypoints.get(w)).copied());
            if mind.prop.is_some() && prop_wp.is_none() {
                mind.prop = None;
            }
            // The locked waypoint is read live (`826AB9C0` re-reads its position on every evaluation): a seat that moved
            // with its prop is where the ped walks and sits.
            if let (Some(w), true) = (prop_wp, mind.brain.waypoint.is_some()) {
                mind.brain.waypoint = Some(w.position);
            }
            mind.brain.has_plugin = mind.brain.plugin.is_some() || mind.prop.is_some();
            // ThrowHandPropAtTrashBin's target, the plugin object's hotpoint 0 (`826A7998`, b87 §3): our hotpoint props
            // are one prop per hotpoint and every stock bin has only hotpoint 0, so the taken prop's own point.
            mind.brain.plugin_target = mind.prop.and_then(|(i, id, _)| plugin_props.props.get(i).filter(|p| p.id == id)).and_then(|p| p.waypoints.first()).map(|w| w.position);
            let conv = mind.brain.plugin.and_then(|id| conversations.map.get(&id));
            let free: Vec<[f32; 3]> = match (conv, prop_wp) {
                (Some(c), _) => c.waypoints.iter().filter(|w| w.1.is_none()).map(|w| w.0).collect(),
                (None, Some(w)) => vec![w.position],
                _ => Vec::new(),
            };
            // A world prop's view: its waypoint (reserved at the offer) and a facing point along its orientation.
            let conversation = match (conv, prop_wp) {
                (Some(c), _) => Some(skate_core::living_world::peds::brain::ConversationInfo { complete: c.is_complete(), speaker: c.speaker(), center: c.center, free_waypoints: &free, speech: c.turn_speech() }),
                (None, Some(w)) => {
                    mind.brain.waypoint_facing = Some(w.facing);
                    let center = [w.position[0] + w.facing[0] * 10.0, w.position[1], w.position[2] + w.facing[2] * 10.0];
                    Some(skate_core::living_world::peds::brain::ConversationInfo { complete: false, speaker: None, center, free_waypoints: &free, speech: None })
                }
                _ => None,
            };
            let groups_now = &chase_groups.0;
            let groups = |chasee: u64| groups_now.get(&chasee).map(|g| g.info(max_chasers(chasee)));
            // UpdateBlockPrediction (`82D99540`): the chasee radius `G+1648` has no known writer: 0.0.
            let block = |chasee: u64| {
                let (r, q, g) = (record?, target(chasee)?, groups_now.get(&chasee)?);
                let v = observers.get(chasee.wrapping_sub(PLAYER_TARGET_BASE) as usize).map_or([0.0; 3], |o| o.velocity);
                Some((skate_core::living_world::peds::chase::should_block(at, q, v, 0.0, r), g.formation_point(me, q)))
            };
            mind.brain.zombie = settings.zombie;
            // ChaseeIsWipedOut / the takedown gate: a player chasee's wiping-out bit; a ped chasee's flag (brain `+3277`
            // bit 0x04) is not modelled yet (false).
            let wiping_of = |id: u64| id >= PLAYER_TARGET_BASE && wiping.get((id - PLAYER_TARGET_BASE) as usize).copied().unwrap_or(false);
            let mut host = skate_core::living_world::peds::brain::BrainHost {
                behaviors: &graph.behaviors,
                conditions: &graph.conditions,
                brain: &mut mind.brain,
                settings: &settings.ped_brain.values,
                position: body.position.to_array(),
                heading: body.heading,
                skater: observers.first().map(|o| (o.position, o.velocity)),
                target_position: &target,
                chase: skate_core::living_world::peds::brain::ChaseView { me, record, groups: Some(&groups), takedowns: Some(&choose), block: Some(&block), conversation, velocity: Some(&velocity_of), own_velocity: body.velocity.to_array(), wiping: Some(&wiping_of) },
            };
            controller.update(program, dt, &mut host);
            if std::mem::take(&mut mind.brain.collision_began) {
                info!("PED_COLLIDING ped=#{} begins: collision broadcast tick={tick}", ped.id.serial);
                events.push(PedEvent::Colliding { id: ped.id });
            }
            // The Plugin state runs the plugin's own graph on the same brain (`8269F248`).
            let plugin_graph = match mind.prop {
                Some((i, ..)) => plugin_props.props.get(i).and_then(|p| Some((p.class.clone(), data.plugin_graphs.get(&p.class)?))).map(|(k, g)| (k, &**g)),
                None => data.conversation_graph.as_deref().map(|g| (CONVERSATION_GRAPH.to_owned(), g)),
            };
            match (mind.brain.in_plugin, plugin_graph) {
                (true, Some((key, pg))) => {
                    let states = pg.graph.runtime.program.topology.states.len();
                    if let Some(old) = plugin_controller_for(&mut mind.plugin_controller, &mut mind.plugin_graph, &key, states) {
                        // The prop the ped used streamed out or was dropped while it stayed in the Plugin state: the
                        // other graph starts fresh (its controller was sized for the old one; a session crashed, 2026-10-12).
                        info!("PED_PLUGIN ped=#{} graph {old} -> {key} tick={tick}", ped.id.serial);
                        mind.plugin_state = None;
                    }
                    let pc = mind.plugin_controller.as_mut().expect("plugin controller just made");
                    let mut host = skate_core::living_world::peds::brain::BrainHost {
                        behaviors: &pg.behaviors,
                        conditions: &pg.conditions,
                        brain: &mut mind.brain,
                        settings: &settings.ped_brain.values,
                        position: body.position.to_array(),
                        heading: body.heading,
                        skater: observers.first().map(|o| (o.position, o.velocity)),
                        target_position: &target,
                        chase: skate_core::living_world::peds::brain::ChaseView { me, record, groups: Some(&groups), takedowns: Some(&choose), block: Some(&block), conversation, velocity: Some(&velocity_of), own_velocity: body.velocity.to_array(), wiping: Some(&wiping_of) },
                    };
                    pc.update(&pg.graph.runtime.program, dt, &mut host);
                    if mind.prop.is_some() && pc.frame.current == mind.plugin_state && tick % 150 == 0 {
                        // Progress of a ped on a world prop (logs must diagnose a stall).
                        let name = |s: Option<usize>| s.and_then(|s| pg.graph.binding.states.get(s)).map_or("none", |s| s.name.as_str());
                        let dist = mind.brain.waypoint.map(|w| ((w[0] - body.position.x).powi(2) + (w[2] - body.position.z).powi(2)).sqrt());
                        // The plugin's give-up timer (`PluginTimeout`, 60 s from EnterInteraction) and how many brain timers
                        // run: a ped stuck past 60 s must show why the timer did not expire (2026-10-14).
                        let timeout = mind.brain.timers.get(&skate_core::living_world::peds::brain::timers::index("PluginTimeout")).copied();
                        info!("PED_PLUGIN_WAIT ped=#{} in {} dist={:?} at=[{:.2}, {:.2}, {:.2}] approach={:?} slide={:?} route={} plugin_timeout={timeout:?} timers={} tick={tick}", ped.id.serial, name(pc.frame.current), dist.map(|d| (d * 1000.0).round() / 1000.0), body.position.x, body.position.y, body.position.z, mind.brain.approach.map(|a| a.1), mind.brain.approach_slide, body.nav.route.is_some(), mind.brain.timers.len());
                    }
                    if pc.frame.current != mind.plugin_state {
                        let name = |s: Option<usize>| s.and_then(|s| pg.graph.binding.states.get(s)).map_or("none", |s| s.name.as_str());
                        let dist = mind.brain.waypoint.map(|w| ((w[0] - body.position.x).powi(2) + (w[2] - body.position.z).powi(2)).sqrt());
                        info!("PED_PLUGIN ped=#{} {} -> {} waypoint={:?} dist={:?} at=[{:.2}, {:.2}, {:.2}] tick={tick}", ped.id.serial, name(mind.plugin_state), name(pc.frame.current), mind.brain.waypoint.map(|w| [w[0].round(), w[2].round()]), dist.map(|d| (d * 1000.0).round() / 1000.0), body.position.x, body.position.y, body.position.z);
                        mind.plugin_state = pc.frame.current;
                    }
                }
                _ => {
                    mind.plugin_controller = None;
                    mind.plugin_state = None;
                    mind.plugin_graph = None;
                }
            }
            // The motion side of the monitored packets (`82E29540`): a plugin state plays the front packet, releases its
            // held cycle on the next stage intent and ends the packet when its clips are done (`IntentStageComplete`
            // decrement, `MajorIntentComplete`); packets whose motion state is not ported end at their last stage.
            {
                use skate_core::living_world::peds::plugin_motion::motion_for;
                let run = &mut body.plugin_motion;
                match run.packet.clone() {
                    None => {
                        if let Some(name) = mind.brain.monitored.iter().find(|(n, p)| p.active && motion_for(n).is_some()).map(|(n, _)| n.clone()) {
                            info!("PED_PLUGIN_MOTION ped=#{} {name} start tick={tick}", ped.id.serial);
                            *run = PluginMotionRun { packet: Some(name), state: TauntClip::Requested, ..Default::default() };
                        }
                    }
                    Some(name) => match mind.brain.monitored.get_mut(&name).filter(|_| !run.orphaned) {
                        // The packet ended mid-clip: its clips play out (the cycle released) before the run ends.
                        None if run.state == TauntClip::Playing => {
                            if !run.orphaned {
                                info!("PED_PLUGIN_MOTION ped=#{} {name} ended mid-clip, standing up tick={tick}", ped.id.serial);
                            }
                            run.orphaned = true;
                        }
                        None => *run = PluginMotionRun::default(),
                        Some(p) => {
                            let m = motion_for(&name).expect("only plugin states start a run");
                            if run.state == TauntClip::Done {
                                if m.decrement && p.stage > 0 {
                                    p.stage -= 1;
                                }
                                p.active = false;
                                info!("PED_PLUGIN_MOTION ped=#{} {name} complete tick={tick}", ped.id.serial);
                                *run = PluginMotionRun::default();
                            } else if m.steps.iter().any(|st| st.hold_until.is_some() && st.hold_until == p.current()) {
                                run.release = true;
                            }
                        }
                    },
                }
            }
            // SpawnInteractionBasedHandProp (`826AE950` -> `82E3DDA0`): the plugin prop's hand prop is requested
            // (`brain+3279` 0x01) with its record's bools; `ped_hand_props::sync_hand_props` creates the object.
            if std::mem::take(&mut body.plugin_motion.hand_prop_due) {
                let class = mind.prop.and_then(|(i, _, _)| plugin_props.props.get(i)).map(|p| p.class.clone());
                let list = class.as_ref().and_then(|c| data.plugins.classes.get(c)).map(|c| c.hand_props.clone()).unwrap_or_default();
                let roll = mind.brain.rng.as_mut().map_or(0.0, |r| r.unit());
                match skate_core::living_world::peds::brain::HandProp::pick(&list, roll) {
                    Some(key) => {
                        mind.brain.hand_prop.request(key);
                        if let Some(r) = data.hand_props.props.get(key) {
                            let h = &mut mind.brain.hand_prop;
                            (h.disposable, h.can_sit, h.can_attack_throw) = (r.disposable, r.can_sit, r.can_attack_throw);
                        }
                        info!("PED_HAND_PROP ped=#{} requested {key} from {} tick={tick}", ped.id.serial, class.as_deref().unwrap_or("?"));
                        events.push(PedEvent::HandProp { id: ped.id, key: key.to_string() });
                    }
                    None => info!("PED_HAND_PROP ped=#{} none offered by {} tick={tick}", ped.id.serial, class.as_deref().unwrap_or("?")),
                }
            }
            mind.brain.settle_packets(&|name| name == "SGIntent" || skate_core::living_world::peds::plugin_motion::motion_for(name).is_some());
            // Group changes, in the order the graph asked for them.
            for request in std::mem::take(&mut mind.brain.chase_requests) {
                use skate_core::living_world::peds::brain::ChaseRequest;
                let (chasee, kind, reason) = match request {
                    ChaseRequest::Join { chasee } => {
                        // The formation offset: chaser - chasee at the join (`82D96C38`).
                        let offset = target(chasee).map_or([0.0; 3], |q| [at[0] - q[0], at[1] - q[1], at[2] - q[2]]);
                        let joined = chase_groups.0.entry(chasee).or_default().add(me, offset, max_chasers(chasee));
                        if !joined && mind.brain.chasee == Some(chasee) {
                            mind.brain.chasee = None;
                        }
                        (chasee, if joined { "join" } else { "join_refused" }, None)
                    }
                    ChaseRequest::Leave { chasee } => {
                        if let Some(g) = chase_groups.0.get_mut(&chasee) {
                            g.remove(me);
                        }
                        (chasee, "leave", None)
                    }
                    ChaseRequest::GiveUpPrimary { chasee } => {
                        chase_groups.0.get_mut(&chasee).map(|g| g.give_up_primary(me));
                        (chasee, "primary", None)
                    }
                    // Only a player gets the marker message (`vfn92` "is a player").
                    ChaseRequest::StateMessage { target, state } => {
                        if target >= PLAYER_TARGET_BASE {
                            let kind = ["state_warn", "state_chase", "state_tired", "state_giveup", "state_other"][usize::from(state.min(4))];
                            info!("PED_CHASE ped=#{} {kind} chasee={target} tick={tick}", ped.id.serial);
                            events.push(PedEvent::Chase { id: ped.id, chasee: target, kind, reason: None });
                        }
                        continue;
                    }
                    ChaseRequest::Takedown { target } | ChaseRequest::TakedownFailed { target } => {
                        let success = matches!(request, ChaseRequest::Takedown { .. });
                        let player = target >= PLAYER_TARGET_BASE;
                        if success {
                            // `826A4E50`: speech 65 when the player was taken down, else 19.
                            mind.brain.speech = Some(if player { 65 } else { 19 });
                            if player {
                                takedowns.push(SkaterTakedownRequest { player: (target - PLAYER_TARGET_BASE) as usize, chaser: at });
                            }
                        }
                        info!("PED_TAKEDOWN ped=#{} target={target} success={success} entry={:?} takedowns={} tick={tick}", ped.id.serial, mind.brain.takedown_choice.map(|c| c.entry), mind.brain.takedowns);
                        events.push(PedEvent::Takedown { id: ped.id, target, success });
                        continue;
                    }
                    // `826A8420`: `T.vfn20(ped position)`, the takedown's latch on the skater.
                    ChaseRequest::Taze { target } => {
                        if target >= PLAYER_TARGET_BASE {
                            takedowns.push(SkaterTakedownRequest { player: (target - PLAYER_TARGET_BASE) as usize, chaser: at });
                        }
                        info!("PED_TAZE ped=#{} target={target} tick={tick}", ped.id.serial);
                        events.push(PedEvent::Taze { id: ped.id, target });
                        continue;
                    }
                    // The tazer's burst sound for the state graph's TazerCycTime (2.0 s).
                    ChaseRequest::TazerOn => {
                        // skate_vita: the tazer's sound is not the Vita's.
                        continue;
                    }
                    ChaseRequest::Greeted { target } => {
                        greeted.push((target, me, at));
                        info!("PED_GREET ped=#{} target={target} tick={tick}", ped.id.serial);
                        continue;
                    }
                    // SpawnConversationArea (`826A6CD8`): none within 50 m of another; 3 m ahead (no check there);
                    // the starter and its target join. The waypoint generator (`82E1D620`, b134) drops the centre
                    // with a +-3 m world ray (`82E166C0`; a miss = no waypoints) and keeps the waypoints only when
                    // all land on low-cost navmesh (no roads): an area without waypoints dissolves.
                    ChaseRequest::SpawnConversation { target: partner } => {
                        let near = conversations.map.values().any(|c| (c.center[0] - at[0]).powi(2) + (c.center[2] - at[2]).powi(2) < conv_params.exclusion * conv_params.exclusion);
                        if !near && target(partner).is_some() {
                            let ahead = [at[0] + body.heading.sin() * conv_params.ahead, at[1], at[2] + body.heading.cos() * conv_params.ahead];
                            // Without a physics world (tests) the centre is used as it is.
                            let center = match physics {
                                Some(p) => ground(Some(p), Vec3::from_array(ahead), conv_params.drop_height, conv_params.drop_height).map(|y| [ahead[0], y, ahead[2]]),
                                None => Some(ahead),
                            };
                            let probe = |w: [f32; 3]| data.nav.as_deref().and_then(|n| n.locate_any(w).map(|p| (p.position, n.area(p.poly))));
                            // The group's category by probability, its rows the candidates.
                            let cats = data.conversations.by_entity.get(&ped.entity).cloned().unwrap_or_default();
                            let total: f32 = cats.iter().map(|c| c.0).sum();
                            let mut pick = conv_rand() * total;
                            let candidates = cats.iter().find(|c| {
                                pick -= c.0;
                                pick < 0.0
                            }).or(cats.last()).map(|c| c.1.clone()).unwrap_or_default();
                            conversations.next_id += 1;
                            let id = conversations.next_id;
                            let mut c = skate_core::living_world::peds::conversation::Conversation::new(id, center, candidates, &conv_params, data.nav.is_some().then_some(&probe as skate_core::living_world::peds::conversation::Probe), &mut conv_rand);
                            c.join(me);
                            c.join(partner);
                            info!("PED_CONVERSATION id={id} start ped=#{} with={partner} center=[{:.1}, {:.1}, {:.1}] dropped={} waypoints={} tick={tick}", ped.id.serial, c.center[0], c.center[1], c.center[2], center.is_some(), c.waypoints.len());
                            conversations.map.insert(id, c);
                            mind.brain.plugin = Some(id);
                        }
                        continue;
                    }
                    ChaseRequest::LockWaypoint { at: w } => {
                        if let Some(c) = mind.brain.plugin.and_then(|id| conversations.map.get_mut(&id)) {
                            c.lock_closest_waypoint(me, w);
                        }
                        continue;
                    }
                    ChaseRequest::UnlockWaypoint => {
                        if let Some(c) = mind.brain.plugin.and_then(|id| conversations.map.get_mut(&id)) {
                            c.unlock_waypoint(me);
                        }
                        continue;
                    }
                    ChaseRequest::ExitPlugin if mind.prop.is_some() => {
                        if let Some((i, id, _)) = mind.prop.take() {
                            if let Some(p) = plugin_props.props.get_mut(i).filter(|p| p.id == id) {
                                p.release(me);
                                info!("PED_PLUGIN_PROP ped=#{} leaves {} {:016X} tick={tick}", ped.id.serial, p.class, id);
                            }
                        }
                        mind.brain.has_plugin = false;
                        mind.brain.waypoint = None;
                        mind.brain.waypoint_facing = None;
                        mind.plugin_controller = None;
                        mind.plugin_state = None;
                        mind.plugin_graph = None;
                        continue;
                    }
                    ChaseRequest::ExitPlugin => {
                        if let Some(id) = mind.brain.plugin.take() {
                            use skate_core::living_world::peds::conversation::Left;
                            let left = conversations.map.get_mut(&id).map(|c| (c.is_complete(), c.leave(me)));
                            let how = match left {
                                Some((false, Left::Remaining)) => "abort",
                                Some((_, Left::Empty)) => "empty",
                                _ => "leave",
                            };
                            if matches!(left, Some((_, Left::Empty))) {
                                conversations.map.remove(&id);
                            }
                            info!("PED_CONVERSATION id={id} leave ped=#{} result={how} tick={tick}", ped.id.serial);
                        }
                        mind.brain.has_plugin = false;
                        mind.brain.waypoint = None;
                        continue;
                    }
                    ChaseRequest::SignalInPosition => {
                        if let Some(c) = mind.brain.plugin.and_then(|id| conversations.map.get_mut(&id)) {
                            let rows = &data.conversations.rows;
                            let before = c.state;
                            c.signal_in_position(me, rows, &mut conv_rand);
                            if before < 2 && c.state == 2 {
                                info!("PED_CONVERSATION id={} begins row={:?} value={:?} tick={tick}", c.id, c.row.and_then(|r| rows.get(r)).map(|r| r.name.as_str()), c.value);
                            }
                        }
                        continue;
                    }
                    ChaseRequest::PassTurn => {
                        if let Some(c) = mind.brain.plugin.and_then(|id| conversations.map.get_mut(&id)) {
                            c.pass_turn(&mut conv_rand);
                            info!("PED_CONVERSATION id={} turn state={} speaker={:?} tick={tick}", c.id, c.state, c.speaker());
                        }
                        continue;
                    }
                    ChaseRequest::Spoke => {
                        if let Some(c) = mind.brain.plugin.and_then(|id| conversations.map.get(&id)) {
                            listening.extend(c.listeners());
                        }
                        continue;
                    }
                    // TakedownTauntVictim (`826A70E0`): speech 65 when the victim is the player, else 19.
                    ChaseRequest::Taunt { target } => {
                        mind.brain.speech = Some(if target >= PLAYER_TARGET_BASE { 65 } else { 19 });
                        body.taunt = TauntClip::Requested;
                        info!("PED_TAUNT ped=#{} target={target} tick={tick}", ped.id.serial);
                        continue;
                    }
                    ChaseRequest::HandPropClip { clip } => {
                        body.plugin_motion.throw_clip = Some((clip, settings.ped_brain.values.hand_prop.clip_blend));
                        info!("PED_HAND_PROP ped=#{} throw clip {clip} target={:?} tick={tick}", ped.id.serial, mind.brain.hand_prop.throw.map(|t| t.target));
                        continue;
                    }
                    ChaseRequest::HandPropReleased { velocity } => {
                        mind.hand_prop_release = Some(velocity);
                        continue;
                    }
                    ChaseRequest::MoodReset { target } => {
                        mind.mood.forget(target);
                        continue;
                    }
                    ChaseRequest::GroupEnd { chasee, reason } => {
                        if let Some(g) = chase_groups.0.get_mut(&chasee) {
                            g.end_reason = Some(reason);
                        }
                        (chasee, "group_end", Some(reason))
                    }
                };
                info!("PED_CHASE ped=#{} {kind} chasee={chasee} reason={reason:?} group={:?} tick={tick}", ped.id.serial, chase_groups.0.get(&chasee).map(|g| &g.chasers));
                events.push(PedEvent::Chase { id: ped.id, chasee, kind, reason });
            }
        }
        // Intercept: each tick a new goal from the solver (`826A3BF0`): the chasee must be
        // reachable (`sub_82C465A8`, our navmesh line) and the goal on the mesh (`sub_82C460E0`).
        // A locked ped drops its route (the plugin's waypoint walk is over).
        if mind.brain.position_locked && body.nav.route.is_some() {
            body.nav.set_route(None);
        }
        use skate_core::living_world::peds::brain::ChaseSteer;
        let one_point = |p: [f32; 3]| Some(skate_core::living_world::peds::PedRoute { points: vec![p], looped: false });
        match (mind.brain.motion_intent, mind.brain.chasee, data.nav.as_deref()) {
            // LostChasee: walk to the last known position (a one-point route).
            _ if mind.brain.search_point.is_some() => {
                let p = mind.brain.search_point.unwrap_or_default();
                if mind.chase_goal.take().is_some() || body.nav.route.as_ref().is_none_or(|r| r.points.first() != Some(&p)) {
                    body.nav.set_route(one_point(p));
                }
            }
            // ZombieFollow (`826A9250`): walk / run to the brain's goal (the player or a point of the ring round them).
            (Some(skate_core::living_world::peds::brain::motion::ZOMBIE_FOLLOW), ..) if mind.brain.zombie_goal.is_some() => {
                let p = mind.brain.zombie_goal.unwrap_or_default();
                mind.chase_goal = None;
                if body.nav.route.as_ref().is_none_or(|r| r.points.first() != Some(&p)) {
                    body.nav.set_route(one_point(p));
                }
            }
            // ApproachWantTarget (`8269FDF0`): walk to the want's target.
            _ if mind.brain.approach.is_some() => {
                let p = mind.brain.approach.map(|a| a.0).unwrap_or_default();
                mind.chase_goal = None;
                let d = [p[0] - at[0], p[2] - at[2]];
                let dist = (d[0] * d[0] + d[1] * d[1]).sqrt();
                match mind.brain.approach_slide {
                    // TargetWaypoint's slide (`slideDistance` / `slideSpeed`): the last stretch
                    // moves the body straight onto the point (ours: no slide clip).
                    Some((slide, speed)) if dist <= slide => {
                        body.nav.set_route(None);
                        if dist > 1e-4 {
                            let step = (speed * dt).min(dist);
                            body.position.x += d[0] / dist * step;
                            body.position.z += d[1] / dist * step;
                        }
                    }
                    _ => body.nav.set_route(one_point(p)),
                }
            }
            // PursueChasee (`826A43B0`): run to the formation point.
            (Some(skate_core::living_world::peds::brain::motion::INTERCEPT), Some(chasee), Some(_)) if mind.brain.chase_steer == Some(ChaseSteer::Pursue) => {
                if let (Some(q), Some(g)) = (target(chasee), chase_groups.0.get(&chasee)) {
                    let p = g.formation_point(me, q);
                    body.nav.set_route(one_point(p));
                    mind.chase_goal = Some(p);
                }
            }
            // BlockChasee (`826A40E8`): walk to the block point; at it the brain stands and faces.
            (Some(skate_core::living_world::peds::brain::motion::INTERCEPT), Some(_), Some(_)) if matches!(mind.brain.chase_steer, Some(ChaseSteer::Block { .. })) => {
                mind.chase_goal = None;
                match mind.brain.block_point.filter(|_| mind.brain.speed_suggestion != Some(0.0)) {
                    Some(p) => body.nav.set_route(one_point(p)),
                    None => body.nav.set_route(None),
                }
            }
            (Some(skate_core::living_world::peds::brain::motion::INTERCEPT), Some(chasee), Some(mesh)) => {
                use skate_core::living_world::peds::chase;
                // CannotReachChaseTarget (`82E31D58`, b108): the intercept move's result sets the bit within 5 m of the
                // chasee and clears it beyond. Ours: a chasee off the navmesh (in water, on a roof) is a failed move
                // [inferred mapping of retail's result codes 0 / 1 / 3]; back on the navmesh clears it.
                if let Some(q) = target(chasee) {
                    let off_mesh = mesh.locate(q).is_none();
                    let near = (q[0] - at[0]).hypot(q[2] - at[2]) <= settings.ped_brain.values.unreachable_radius;
                    let now = off_mesh && near;
                    if now != mind.brain.target_unreachable {
                        info!("PED_UNREACHABLE ped=#{} chasee={chasee} {} off_mesh={off_mesh} near={near} tick={tick}", ped.id.serial, if now { "cannot reach" } else { "reachable again" });
                    }
                    if off_mesh {
                        mind.brain.target_unreachable = near;
                    } else {
                        mind.brain.target_unreachable = false;
                    }
                }
                if let (Some(q), Some(here)) = (target(chasee), mesh.locate(at)) {
                    let velocity = observers.get(chasee.wrapping_sub(PLAYER_TARGET_BASE) as usize).map_or([0.0; 3], |o| o.velocity);
                    let speed = record.and_then(|r| r.run_speed()).unwrap_or(0.0);
                    // No record: the max lead time reads 0.0, so no goal.
                    let max_lead = record.map_or(0.0, |r| r.max_lead_time());
                    let goal = mesh.clear_line(here, q).then(|| chase::intercept(at, q, velocity, speed, max_lead, chase::predict_angle(data.chase_global.as_deref()))).flatten().filter(|i| mesh.locate(i.point).is_some());
                    if let Some(i) = goal {
                        if mind.chase_goal.is_none() {
                            info!("PED_INTERCEPT ped=#{} chasee={chasee} goal=[{:.1}, {:.1}, {:.1}] leads={} t={:.2} tick={tick}", ped.id.serial, i.point[0], i.point[1], i.point[2], i.leads, i.time);
                        }
                        body.nav.set_route(Some(skate_core::living_world::peds::PedRoute { points: vec![i.point], looped: false }));
                        mind.chase_goal = Some(i.point);
                    }
                }
            }
            _ => {
                if mind.chase_goal.take().is_some() {
                    body.nav.set_route(None);
                }
            }
        }
        // Flee movement: 15 m legs away from the threat, a new one within 2 m of the goal; the
        // leg goes to the nav as a one-point route (cleared when the flee ends).
        let flee = settings.ped_brain.flee;
        match (mind.brain.motion_intent, mind.brain.flee_from, data.nav.as_deref()) {
            (Some(skate_core::living_world::peds::brain::motion::FLEE), Some(threat), Some(mesh)) => {
                if mind.flee_goal.is_none_or(|g| skate_core::living_world::peds::flee::arrived(at, g, &flee)) {
                    if let Some(threat_at) = target(threat) {
                        let velocity = observers.get(threat.wrapping_sub(PLAYER_TARGET_BASE) as usize).map_or([0.0; 3], |o| o.velocity);
                        let mut goal = skate_core::living_world::peds::flee::goal(at, threat_at, velocity, &flee);
                        // The navmesh cast (`sub_82C465A8`): a blocked leg ends at the last clear
                        // point along it (`sub_82C46208`'s rewrite, inferred).
                        if let Some(here) = mesh.locate(at) {
                            if !mesh.clear_line(here, goal) {
                                let (mut lo, mut hi) = (0.0f32, 1.0f32);
                                for _ in 0..8 {
                                    let mid = (lo + hi) * 0.5;
                                    let p = [at[0] + (goal[0] - at[0]) * mid, at[1], at[2] + (goal[2] - at[2]) * mid];
                                    if mesh.clear_line(here, p) {
                                        lo = mid;
                                    } else {
                                        hi = mid;
                                    }
                                }
                                goal = [at[0] + (goal[0] - at[0]) * lo, at[1], at[2] + (goal[2] - at[2]) * lo];
                            }
                        }
                        body.nav.set_route(Some(skate_core::living_world::peds::PedRoute { points: vec![goal], looped: false }));
                        mind.flee_goal = Some(goal);
                        info!("PED_FLEE ped=#{} from={} goal=[{:.1}, {:.1}, {:.1}] tick={tick}", ped.id.serial, threat, goal[0], goal[1], goal[2]);
                    }
                }
            }
            // RunFromHonker (`826A1358`): every frame while the car exists, a goal 10 m sideways of its line; a
            // car that is gone leaves the last goal (retail keeps it).
            (Some(skate_core::living_world::peds::brain::motion::RUN_FROM_HONKER), _, _) => {
                if let Some((car_at, dir)) = mind.brain.honker.and_then(|c| car_pose.get(&c)) {
                    if let Some((goal, speed)) = skate_core::living_world::peds::honk::run_goal(at, *car_at, *dir, &settings.ped_brain.run_from_honker) {
                        if mind.flee_goal.is_none() {
                            info!("PED_HONKED ped=#{} car={:?} goal=[{:.1}, {:.1}, {:.1}] tick={tick}", ped.id.serial, mind.brain.honker, goal[0], goal[1], goal[2]);
                        }
                        body.nav.set_route(Some(skate_core::living_world::peds::PedRoute { points: vec![goal], looped: false }));
                        mind.flee_goal = Some(goal);
                        mind.brain.speed_suggestion = Some(speed);
                    }
                }
            }
            _ => {
                if mind.flee_goal.take().is_some() {
                    body.nav.set_route(None);
                }
            }
        }
        // Speech: the brain stores the value on the ped (`ped+2468`); PedestrianSpeech speaks when
        // it changes, so only a change goes to the audio side (a state re-sending its value stays
        // silent, as in retail).
        if mind.brain.speech != mind.spoken {
            mind.spoken = mind.brain.speech;
            // A conversation turn's variant and row value go with its line only.
            let topic = mind.brain.speech_topic.take();
            if let Some(value) = mind.brain.speech {
                let state = controller.frame.current.and_then(|s| graph.graph.binding.states.get(s)).map_or("none", |s| s.name.as_str());
                info!("PED_SPEECH ped=#{} value={value} topic={topic:?} state={state} tick={tick}", ped.id.serial);
                events.push(PedEvent::Speech { id: ped.id, value, topic, state: state.to_string() });
            }
        }
        if controller.frame.current != mind.state {
            let name = |s: Option<usize>| s.and_then(|s| graph.graph.binding.states.get(s)).map_or("none", |s| s.name.as_str());
            let key = (1 << 63) | (mind.state.map_or(0, |s| s as u64 + 1) << 24) | controller.frame.current.map_or(0, |s| s as u64 + 1);
            if let Some(held) = mind.log_repeats.allow(key, tick) {
                let held = if held > 0 { format!(" (+{held} held back)") } else { String::new() };
                info!("PED_BRAIN ped=#{} {} -> {} intent={:?} wants={:?} tick={tick}{held}", ped.id.serial, name(mind.state), name(controller.frame.current), mind.brain.motion_intent, mind.brain.wants.keys().collect::<Vec<_>>());
            }
            // Why a new chase was refused (NewChasee/CancelChase: not CanNewChaseStart or not CanChaseeAddNewChaser).
            if name(controller.frame.current) == "CancelChase" {
                let chasee = mind.brain.wants.get("angrychase").map(|w| w.target);
                let group = chasee.and_then(|c| chase_groups.0.get(&c).map(|g| g.info(max_chasers(c))));
                if let Some(held) = mind.log_repeats.allow(0x4341_4e43, tick) {
                    info!(
                        "PED_CHASE_CANCEL ped=#{} chasee={chasee:?} chases_enabled={} group={:?} max_chasers={:?} tick={tick}{}",
                        ped.id.serial,
                        settings.ped_brain.values.chases_enabled,
                        group,
                        chasee.map(&max_chasers),
                        if held > 0 { format!(" (+{held} held back)") } else { String::new() }
                    );
                }
            }
            mind.state = controller.frame.current;
        }
    }
}
