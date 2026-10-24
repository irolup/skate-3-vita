//! skate-game's `living_world` (upstream PR #52) without Bevy: the items the copied physics names
//! (`ca647a3`), the DMO streaming, the population and the traffic cars (`618e824`:
//! [`LivingWorldState`], [`vehicles`]), the NPC skaters' replay tier ([`npc_skaters`]),
//! the peds ([`peds`]).

pub(crate) mod vehicles;

pub(crate) mod npc_avoid;
pub(crate) mod npc_skaters;

pub(crate) mod ped_body;
pub(crate) mod ped_graph;
pub(crate) mod ped_hand_props;
pub(crate) mod ped_mood;
pub(crate) mod ped_plugins;
pub(crate) mod peds;
pub(crate) mod vehicle_contacts;

pub(crate) mod npc_sim {
    use crate::physics::SkaterRuntime;

/// The wipeout bit of the skater's flags word (`flags2468` bit 18, the "Wipeout" graph
/// attribute): retail's physics step copies it to skater component `+59` (`sub_82DB6EC0`), the
/// PathController latches it into `pc+924` each tick (`sub_8246EF78`) and its rising edge starts
/// the respawn (`sub_8246EE30`).
pub(crate) const WIPEOUT_BIT: u32 = 0x0004_0000;

/// What B reads from the simulated skater (b80): `[rec+56]+161` = SkaterOffBoard (Processed +2484 0x08000000),
/// `+160` = TransitioningOnOffBoard (+2480 0x4), both written by `82DB6EC0`; "can move" (`82466ED0`) = off the board
/// and not toggling (its `[rec+72]+308 / +309` terms are not identified: taken as clear); the hand-back byte
/// `[rec+28]+59` = the wipeout bit (`82DB6EC0` copies it there).
pub(crate) fn walker_input(runtime: &SkaterRuntime) -> skate_core::living_world::controller_b::BInput {
    // The skater, not the board (left lying once off): P = the animation root ([rec+20]+416), B+112 = its forward
    // row ([rec+20]+0 [inference: forward]).
    let root = runtime.animated_skeleton.roots.animation_to_world;
    let position = [root[3][0], root[3][1], root[3][2]];
    let forward = [root[2][0], 0.0, root[2][2]];
    let p = &runtime.player_input.processed;
    let off_board = p.flags_2484 & 0x0800_0000 != 0;
    let toggling = p.flags_2480 & 0x4 != 0;
    skate_core::living_world::controller_b::BInput {
        position,
        body: forward,
        facing: forward[0].atan2(forward[2]),
        abort_flag: false,
        state_hand_back: p.flags_2468 & WIPEOUT_BIT != 0,
        online: false,
        can_move: off_board && !toggling,
        byte_161: off_board,
        byte_160: toggling,
    }
}
}

/// The census focus of one player (skate-game `living_world/mod.rs`, PR #52), verbatim.
const FOCUS_STILL_SPEED2: f32 = 1e-4;

pub(crate) fn player_focus(position: [f32; 3], velocity: [f32; 3]) -> skate_core::living_world::Observer {
    let still = velocity.iter().map(|v| v * v).sum::<f32>() <= FOCUS_STILL_SPEED2;
    skate_core::living_world::Observer { position, velocity: if still { [0.0; 3] } else { velocity } }
}

pub(crate) fn local_focus(character: Option<([f32; 3], [f32; 3])>, deck: ([f32; 3], [f32; 3])) -> skate_core::living_world::Observer {
    let (position, velocity) = character.unwrap_or(deck);
    player_focus(position, velocity)
}

/// The local player's focus as skate-game's `gather_observers` takes it: the skeleton's centre of
/// mass and its velocity, else the deck; `None` when not finite (the census then holds).
/// A prop body's frame (origin, rotation columns) as a matrix.
fn prop_frame((o, b): (skate_core::math::Vector3, skate_core::math::Basis3)) -> glam::Mat4 {
    let c = |i: usize| glam::Vec4::new(b.columns[i][0], b.columns[i][1], b.columns[i][2], 0.0);
    glam::Mat4::from_cols(c(0), c(1), c(2), glam::Vec4::new(o.x, o.y, o.z, 1.0))
}

pub(crate) fn local_observer(physics: &crate::physics::GamePhysics, skater: &crate::physics::SkaterRuntime) -> Option<skate_core::living_world::Observer> {
    use skate_core::physics::board::BodyId;
    let f = &skater.animated_skeleton.board_frames;
    let character = Some(([f.centre_of_mass[0], f.centre_of_mass[1], f.centre_of_mass[2]], [f.com_velocity[0], f.com_velocity[1], f.com_velocity[2]]));
    let deck = physics.board.bodies()[BodyId::Deck.index()].rates;
    let v = |x: skate_core::math::Vector3| [x.x, x.y, x.z];
    let observer = local_focus(character, (v(deck.position), v(deck.linear_velocity)));
    observer.position.iter().chain(&observer.velocity).all(|x| x.is_finite()).then_some(observer)
}

/// DMO streaming (skate-game `living_world/dmo_stream.rs`, PR #52): the census that wakes the
/// placed props near the player and parks the others. The Bevy system `stream_dmos` becomes
/// [`dmo_stream::stream`], called by the session after each tick with the local observer and the
/// camera (single player: one observer; no network roles).
pub(crate) mod dmo_stream {
    use skate_core::living_world::dmo::{DmoDecision, DmoStream, DmoStreamSettings, DmoView, DmoWeights};
    use skate_core::living_world::Observer;

    #[derive(Clone, Copy, Debug, PartialEq)]
    pub(crate) struct DmoStreamGameSettings {
        pub enabled: bool,
        pub respawn_authored: bool,
        pub core: DmoStreamSettings,
    }

    impl Default for DmoStreamGameSettings {
        fn default() -> Self {
            Self { enabled: std::env::var("SKATE_DMO_STREAM").ok().as_deref() != Some("0"), respawn_authored: true, core: DmoStreamSettings::default() }
        }
    }

    #[derive(Default)]
    pub(crate) struct DmoStreamState {
        stream: Option<DmoStream>,
        built: Option<(u64, usize)>,
        data: Option<(skate_core::living_world::census::CensusRange, DmoWeights)>,
        tick: u64,
    }

    /// skate_vita: read through `skate_data::vfs` (the packs).
    fn read_data(asset_root: &std::path::Path) -> Option<(skate_core::living_world::census::CensusRange, DmoWeights)> {
        let bytes = skate_data::vfs::read(&asset_root.join("private/living_world/tables.json")).ok()?;
        let root: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
        let circle = |name: &str| {
            let c = root.pointer(&format!("/classes/livingworld_census_ranges/dynamicobjects/fields/{name}"))?;
            let f = |k: &str| c.get(k).and_then(serde_json::Value::as_f64).map(|v| v as f32);
            Some(skate_core::living_world::census::CensusCircle {
                spawn_inner: f("spawn_inner")?,
                spawn_outer: f("spawn_outer")?,
                cull: f("cull")?,
                forward_offset: f("forward_offset")?,
                speed_kmh: f("speed_kmh")?,
            })
        };
        let range = skate_core::living_world::census::CensusRange { slow: circle("circle_slow")?, fast: circle("circle_fast")? };
        let w = root.pointer("/classes/livingworld/dynamicobjects/fields")?;
        let d = DmoWeights::default();
        let f = |k: &str, default: f32| w.get(k).and_then(serde_json::Value::as_f64).map_or(default, |v| v as f32);
        let weights = DmoWeights {
            in_front: f("Hash_74FE9111054663C9", d.in_front),
            keep_flag: f("Hash_883771F11981A45D", d.keep_flag),
            priority: f("Hash_16ADB1DBA4AA77D1", d.priority),
            count: f("Hash_C891B78CB013B593", d.count),
            distance: f("Hash_ABD7EA6B3B93846E", d.distance),
        };
        Some((range, weights))
    }

    /// One frame of `stream_dmos`: `generation` changes with every world load, `camera` is the
    /// camera's position and forward.
    pub(crate) fn stream(
        s: DmoStreamGameSettings,
        asset_root: &std::path::Path,
        generation: u64,
        observers: &[Observer],
        camera: Option<([f32; 3], [f32; 3])>,
        physics: &mut crate::physics::GamePhysics,
        st: &mut DmoStreamState,
    ) {
        if !s.enabled {
            if let Some(stream) = st.stream.take() {
                let pass: Vec<_> = stream.placements.iter().map(|p| (p.id, false, false)).collect();
                physics.stream_props(&pass);
                info!("DMO_STREAM off: {} props back", stream.placements.len());
            }
            st.built = None;
            return;
        }
        let Some(dynamics) = physics.prop_dynamics() else { return };
        // skate_vita (performance): the placements are built for a fill only (their count tells one);
        // upstream built and sorted them every frame.
        let fill = st.built != Some((generation, dynamics.dmo_placement_count()));
        if fill {
            let placements = dynamics.dmo_placements();
            if st.data.is_none() || st.built.is_none_or(|b| b.0 != generation) {
                st.data = read_data(asset_root);
            }
            st.stream = Some(DmoStream::new(placements, &s.core));
            st.built = Some((generation, st.stream.as_ref().map_or(0, |s| s.placements.len())));
            st.tick = 0;
        }
        st.tick += 1;
        if !fill && st.tick % skate_core::living_world::config::retail::CENSUS_ROTATION as u64 != 2 {
            return;
        }
        let mut core = s.core;
        if let Some((range, weights)) = st.data {
            core.range = range;
            core.weights = weights;
        }
        let views: Vec<(Observer, DmoView)> = observers
            .iter()
            .map(|o| {
                let (c, f) = camera.unwrap_or((o.position, [0.0, 0.0, 1.0]));
                (*o, DmoView { camera: c, forward: f, reference: o.position })
            })
            .collect();
        let held = physics.prop_carry.held();
        let layout: std::collections::BTreeSet<u32> = physics.prop_carry.layout().keys().copied().collect();
        let Some(stream) = st.stream.as_mut() else { return };
        // Kept (never the eviction front): a prop moved off its authored spawn pose. NOT RETAIL YET: retail's keep comes
        // from the object's own state (`sub_82C485C0` DMO+0x1170 bit 0x20, `sub_82C56AE8` state word DMO+0x112C == 1;
        // research b184); "moved" is our reading of it until a trace settles it. Before 2026-10-17 any prop could be the
        // front, so a prop the player had knocked was evicted and came back at its authored pose about 27 times a second
        // in DownTown (laptop session 14:36: 21,037 evictions vs 353 culls).
        // skate_vita (performance): upstream builds the moved set over every body each pass; the census asks only about
        // the live and spawning ones (`PropDynamics::is_moved`, the same test).
        let dynamics = physics.prop_dynamics();
        let decisions = stream.step_kept(&views, &core, fill, &|id| held == Some(id) || layout.contains(&id), &|id| dynamics.is_some_and(|d| d.is_moved(id)));
        if decisions.is_empty() && !fill {
            return;
        }
        let (mut spawned, mut culled, mut evicted) = (0, 0, 0);
        let mut pass = Vec::with_capacity(decisions.len());
        for d in &decisions {
            let (id, out) = match *d {
                DmoDecision::Spawn(id) => {
                    spawned += 1;
                    (id, false)
                }
                DmoDecision::Cull(id) => {
                    culled += 1;
                    (id, true)
                }
                DmoDecision::Evict(id) => {
                    evicted += 1;
                    (id, true)
                }
            };
            if !fill {
                debug!("DMO_STREAM_CHANGE {d:?}");
            }
            pass.push((id, out, s.respawn_authored));
        }
        if fill {
            let dormant: Vec<u32> = st.stream.as_ref().map_or_else(Vec::new, |s| s.placements.iter().filter(|p| !s.live.contains(&p.id) && held != Some(p.id) && !layout.contains(&p.id)).map(|p| p.id).collect());
            culled += dormant.len();
            pass.extend(dormant.into_iter().map(|id| (id, true, false)));
        }
        // One collision rebake for the pass (b114).
        physics.stream_props(&pass);
        let live = st.stream.as_ref().map_or(0, |s| s.live.len());
        info!("DMO_STREAM fill={fill} spawned={spawned} culled={culled} evicted={evicted} live={live}");
    }
}

// ---- The population (skate-game `living_world/mod.rs`, PR #52 `618e824`) ----

use skate_core::living_world::{CensusMap, Decision, Kind, LivingWorld, PopulationConfig, TickInputs};

/// Per-kind switch and density (1.0 = retail).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct KindSetting {
    pub enabled: bool,
    pub density: f32,
}

impl Default for KindSetting {
    fn default() -> Self {
        Self { enabled: true, density: 1.0 }
    }
}

/// Settings of the living world: upstream's `LivingWorldSettings` fields the Vita uses (the
/// mod overrides are not taken). Defaults =
/// retail.
#[derive(Clone, Debug, PartialEq)]
pub struct LivingWorldSettings {
    pub enabled: bool,
    pub skaters: KindSetting,
    pub pedestrians: KindSetting,
    pub vehicles: KindSetting,
    /// Desired ambient NPC skaters offline (retail 3).
    pub ambient_skaters: u32,
    /// NPC skater spawn fade in / leave fade (retail 1 s in, 1 s out, removed below 0.2).
    pub skater_fade: skate_core::living_world::leave_fade::LeaveFadeConfig,
    /// What an NPC skater does at the end of its line (retail: continue on an unused line whose
    /// start is within 4 m, `sub_8246C7F8`).
    pub skater_line_chain: skate_core::living_world::replay::ChainConfig,
    /// The NPC skaters' trick choice (retail: re-picked from the character's profile table).
    pub npc_tricks: npc_skaters::NpcTrickSettings,
    /// NPC skaters push dynamic props like the player's board and body (retail on).
    pub npc_skater_props: npc_skaters::NpcSkaterPropContact,
    /// The NPC skaters' retail obstacle avoider.
    pub npc_avoid: skate_core::living_world::avoid::AvoidSettings,
    /// Ped draw fade by camera distance and spawn fade in (retail: the model's 45 / 55 m pair, 1 s in).
    pub ped_fade: skate_core::living_world::peds::PedFadeConfig,
    /// Props as ped navigation obstacles (retail DynamicObject NavPower obstacles).
    pub ped_obstacles: skate_core::living_world::peds::ObstacleParams,
    /// Traffic cars touching peds (retail: the ped is pushed out of the car and walks on).
    pub ped_vehicle_contact: skate_core::living_world::peds::VehicleContactParams,
    /// Peds as solid bodies in the skater's contact solve (retail group 11).
    pub ped_body: ped_body::PedBodySettings,
    /// The ped behaviour runtime (doc 26 "Ped behaviour runtime"); retail values.
    pub ped_brain: PedBrainSettings,
    /// Mod overrides upstream reads (empty: no mods on the Vita); kept so the copied code reads them.
    pub skater_clips: std::collections::BTreeMap<String, String>,
    pub skater_blend_seconds: std::collections::BTreeMap<String, f32>,
    pub skater_stance: std::collections::BTreeMap<String, skate_core::living_world::stance::NaturalStance>,
    pub skater_stance_events: std::collections::BTreeMap<String, String>,
    pub skater_trick_profiles: std::collections::BTreeMap<String, npc_skaters::NpcTrickTables>,
    /// NPC draw distance (QoL, not retail; `skate_core::living_world::draw_distance`): every
    /// population range x this, caps x its square. 1.0 = retail.
    pub npc_draw_distance: f32,
    /// The player picked "None" for NPC draw distance: nothing spawns and the live ones despawn.
    pub user_npcs_off: bool,
    /// Free Play options (mode 3); `None` = career free roam (no scaling).
    pub free_play: Option<skate_core::living_world::FreePlay>,
    /// The zombie cheat (peds without cap, no traffic, no NPC skaters).
    pub zombie: bool,
    /// skate_vita: the ped pool in place of retail's 31 (`retail::PED_POOL`), the only bound left on the peds under the
    /// zombie cheat; None = retail's.
    pub ped_pool: Option<u32>,
    /// skate_vita: the called skaters (FE 62 Call Skater) at most in place of retail's 3 (`retail::SKATER_COMPANION_SLOTS`);
    /// each is one more NPC skater on the tick. None = retail's.
    pub companion_slots: Option<u32>,
    /// Session seed (0 = derive from the map name).
    pub seed: u64,
    /// Log a population summary every 5 s.
    pub debug: bool,
}

/// The ped behaviour runtime's switch and values (upstream `PedBrainSettings`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PedBrainSettings {
    pub enabled: bool,
    /// The mood system raises wants from the stock mood tables.
    pub mood: bool,
    /// Where a fleeing ped runs (retail values).
    pub flee: skate_core::living_world::peds::flee::FleeParams,
    /// RunFromHonker's goal and speeds (`826A1358`).
    pub run_from_honker: skate_core::living_world::peds::honk::RunFromHonkerParams,
    pub values: skate_core::living_world::peds::brain::BrainSettings,
    /// The world-prop plugins' values (offer scan, refusals, the seats-in-water rule); copied into the plugin props
    /// every think, so a mod's change is in effect at once.
    pub plugins: skate_core::living_world::peds::plugins::PluginSettings,
}

impl Default for PedBrainSettings {
    fn default() -> Self {
        Self { enabled: true, mood: true, flee: Default::default(), run_from_honker: Default::default(), values: Default::default(), plugins: Default::default() }
    }
}

impl Default for LivingWorldSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            skaters: KindSetting::default(),
            pedestrians: KindSetting::default(),
            vehicles: KindSetting::default(),
            ambient_skaters: skate_core::living_world::config::retail::SKATER_DESIRED,
            skater_fade: skate_core::living_world::leave_fade::LeaveFadeConfig::retail(),
            skater_line_chain: skate_core::living_world::replay::ChainConfig::retail(),
            npc_tricks: Default::default(),
            npc_skater_props: Default::default(),
            npc_avoid: Default::default(),
            ped_fade: Default::default(),
            ped_obstacles: Default::default(),
            ped_vehicle_contact: Default::default(),
            ped_body: Default::default(),
            ped_brain: Default::default(),
            skater_clips: Default::default(),
            skater_blend_seconds: Default::default(),
            skater_stance: Default::default(),
            skater_stance_events: Default::default(),
            skater_trick_profiles: Default::default(),
            npc_draw_distance: skate_core::living_world::DrawDistance::RETAIL,
            user_npcs_off: false,
            free_play: None,
            zombie: false,
            ped_pool: None,
            companion_slots: None,
            seed: 0,
            debug: false,
        }
    }
}

impl LivingWorldSettings {
    /// The draw distance in effect (sanitised: non-finite or non-positive = retail, clamped).
    pub fn draw_distance(&self) -> skate_core::living_world::DrawDistance {
        skate_core::living_world::DrawDistance::new(self.npc_draw_distance)
    }

    /// The settings menu: the player's choice, in effect at once.
    pub fn set_user_draw_distance(&mut self, multiplier: f32) {
        self.user_npcs_off = multiplier == skate_core::living_world::DrawDistance::NONE;
        self.npc_draw_distance = skate_core::living_world::DrawDistance::new(multiplier).multiplier();
    }

    /// Write the settings into the core config (code defaults and data ranges stay).
    pub fn apply(&self, config: &mut PopulationConfig) {
        config.draw_distance = self.draw_distance().multiplier();
        let on = self.enabled && !self.user_npcs_off;
        config.skaters.enabled = on && self.skaters.enabled;
        config.skaters.desired = (self.ambient_skaters as f32 * self.skaters.density.max(0.0)).round() as u32;
        config.skaters.leave_fade = self.skater_fade;
        config.skaters.line_chain = self.skater_line_chain;
        config.pedestrians.enabled = on && self.pedestrians.enabled;
        config.pedestrians.density = self.pedestrians.density;
        if let Some(pool) = self.ped_pool {
            config.pedestrians.pool = Some(pool);
        }
        if let Some(slots) = self.companion_slots {
            config.skaters.companion_slots = slots;
        }
        config.vehicles.enabled = on && self.vehicles.enabled;
        config.vehicles.density = self.vehicles.density;
    }
}

/// The population of the current world plus the loaded data.
pub(crate) struct PopulationState {
    pub world: LivingWorld,
    pub census: Option<CensusMap>,
    pub skaters: Option<skate_core::living_world::skaters::SkaterData>,
    /// Replay-tier lines and voices of the loaded district.
    pub npc: npc_skaters::NpcData,
    /// Road network and vehicle entities of the loaded world (cars are placed on lanes; none =
    /// no cars).
    pub roads: Option<skate_core::living_world::traffic::RoadNetwork>,
    pub vehicles: Option<skate_core::living_world::VehicleCatalog>,
    /// The ranges from `tables.json` (re-applied after settings changes).
    data_config: PopulationConfig,
    pub status: String,
    pub spawned: u64,
    pub despawned: u64,
    last_report: u64,
}

impl PopulationState {
    /// A new world: fresh population (seeded), new data.
    fn install(map: &str, generation: u64, settings: &LivingWorldSettings, data: LoadedData) -> Self {
        let seed = if settings.seed != 0 { settings.seed } else { skate_core::living_world::rng::derive(0x5345_4544, &[name_hash(map), generation]) };
        Self {
            world: LivingWorld::new(data.config.clone(), seed),
            census: data.census,
            skaters: data.skaters,
            npc: data.npc,
            roads: data.roads,
            vehicles: data.vehicles,
            data_config: data.config,
            status: data.status,
            spawned: 0,
            despawned: 0,
            last_report: 0,
        }
    }
}

fn name_hash(name: &str) -> u64 {
    name.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ b as u64).wrapping_mul(0x0100_0000_01b3))
}

/// Data for one world.
struct LoadedData {
    config: PopulationConfig,
    census: Option<CensusMap>,
    skaters: Option<skate_core::living_world::skaters::SkaterData>,
    npc: npc_skaters::NpcData,
    roads: Option<skate_core::living_world::traffic::RoadNetwork>,
    vehicles: Option<skate_core::living_world::VehicleCatalog>,
    status: String,
}

/// Read the export for a district (map name = district name for the retail cities), through
/// `skate_data::vfs` (the packs).
fn load_data(asset_root: &std::path::Path, district: &str) -> LoadedData {
    let dir = asset_root.join("private/living_world");
    let read = |name: &str| skate_data::vfs::read(&dir.join(name)).ok();
    let mut config = PopulationConfig::retail();
    let mut status = Vec::new();
    let tables = read("tables.json").and_then(|b| match skate_data::living_world::LivingWorldTables::parse(&b) {
        Ok(t) => Some(t),
        Err(e) => {
            status.push(e.to_string());
            None
        }
    });
    let census = tables.as_ref().and_then(|t| {
        t.apply_to(&mut config);
        let bytes = read(&format!("{district}.census.bin"))?;
        match skate_data::living_world::parse_census_grid(&bytes) {
            Ok(g) => Some(t.census_map(vec![g])),
            Err(e) => {
                status.push(e.to_string());
                None
            }
        }
    });
    let mut npc = npc_skaters::NpcData::default();
    let skaters = (|| {
        let profiles = read("skater_profiles.json")?;
        let characters = skate_data::living_world::skater_characters(&profiles, &[]).ok()?;
        npc.voices = npc_voices(&profiles);
        npc.records = npc_records(&profiles);
        npc.tricks = std::sync::Arc::new(skate_data::living_world::skater_trick_profiles(&profiles).unwrap_or_default());
        let pack = read(&format!("skater_paths/{district}.bin"))?;
        let tiles = skate_data::aipath::parse_pack(&pack).ok()?;
        let (paths, _) = skate_data::aipath::district_paths(&tiles).ok()?;
        let lines = skate_data::living_world::skater_lines(paths.iter().map(|p| &p.path));
        npc.lines = std::sync::Arc::new(paths.iter().filter(|p| p.path.id.is_ambient()).map(|p| (p.path.id.0, skate_data::living_world::replay_line(&p.path))).collect());
        Some(skate_core::living_world::skaters::SkaterData { lines, characters })
    })();
    status.insert(
        0,
        format!(
            "{district}: census {}, skater lines {}",
            census.as_ref().map_or("none".to_string(), |c| format!("{} records", c.records.len())),
            skaters.as_ref().map_or(0, |s| s.lines.len())
        ),
    );
    // Roads and vehicle entities (milestone V2). The file holds every district, but the districts
    // are separate worlds overlapping in x / z (doc 26, "Cars flying off"): only the loaded
    // district's roads join the network, or cars land on another district's road in the air.
    let roads = read("roads.bin").and_then(|b| {
        let built = skate_data::roads::RoadGraph::parse(&b).map_err(|e| e.to_string()).and_then(|g| match g.district_traffic_input(district) {
            Some(input) => skate_core::living_world::traffic::RoadNetwork::build(&input).map(Some).map_err(|e| e.to_string()),
            None => Ok(None),
        });
        built.map_err(|e| status.push(format!("roads: {e}"))).ok().flatten()
    });
    let vehicles = read("vehicles.json").and_then(|b| skate_data::living_world::vehicle_catalog(&b).map_err(|e| status.push(e.to_string())).ok());
    if let Some(first) = status.first_mut() {
        first.push_str(&format!(", roads {}, vehicle entities {}", roads.as_ref().map_or(0, |r| r.segments.len()), vehicles.as_ref().map_or(0, |v| v.entities.len())));
    }
    LoadedData { config, census, skaters, npc, roads, vehicles, status: status.join("; ") }
}

/// `characters_marquee` voice ids by character key (`skater_profiles.json` `characters.*.voice`).
pub(crate) fn npc_voices(profiles: &[u8]) -> std::collections::BTreeMap<String, u32> {
    let Ok(v) = serde_json::from_slice::<serde_json::Value>(profiles) else { return Default::default() };
    v["characters"]
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(k, c)| c["voice"].as_u64().map(|x| (k.clone(), x as u32)))
        .collect()
}

/// `characters_marquee` character record names by character key (`skater_profiles.json`
/// `characters.*.recipe`): the record the natural stance is read from.
pub(crate) fn npc_records(profiles: &[u8]) -> std::collections::BTreeMap<String, String> {
    let Ok(v) = serde_json::from_slice::<serde_json::Value>(profiles) else { return Default::default() };
    v["characters"]
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(k, c)| c["recipe"].as_str().filter(|r| !r.is_empty()).map(|r| (k.clone(), r.to_owned())))
        .collect()
}

/// Whether a periodic debug line is due at world tick `tick` (every `period` ticks since `last`).
pub(crate) fn report_due(tick: u64, last: &mut u64, period: u64) -> bool {
    if tick < *last {
        *last = 0;
    }
    if tick < *last + period {
        return false;
    }
    *last = tick;
    true
}

/// One-line population summary (debug log).
pub(crate) fn readout(state: &PopulationState) -> String {
    let w = &state.world;
    format!(
        "LIVING_WORLD tick {} skaters {} peds {} vehicles {} pool {:?} spawned {} despawned {} ({})",
        w.tick(),
        w.count(Kind::Skater),
        w.count(Kind::Pedestrian),
        w.count(Kind::Vehicle),
        w.skater_pool(),
        state.spawned,
        state.despawned,
        state.status
    )
}

/// A traffic car as the renderer draws it: its model, the fixed-step poses to interpolate
/// between, the wheel angles and the paint.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CarPose {
    pub serial: u32,
    /// Index of the car's model in `Session::car_models`.
    pub model: u16,
    /// Translation and rotation (x, y, z, w) of the two ticks the car is drawn between (upstream `drawn_pose`: the
    /// two before the car's current pose, the ticks the skater is drawn between).
    pub prev: ([f32; 3], [f32; 4]),
    pub curr: ([f32; 3], [f32; 4]),
    pub wheel_prev: f32,
    pub wheel: f32,
    pub chassis: [f32; 4],
    pub secondary: [f32; 4],
    pub paint_gain: f32,
}

/// The living world of one session (skate-game's `LivingWorldPlugin` resources): settings,
/// population, traffic and the prop obstacles the cars look ahead at. The session calls
/// [`Self::before_physics`] between the controls and the physics and [`Self::after_physics`]
/// after it, upstream's system order.
pub(crate) struct LivingWorldState {
    pub settings: LivingWorldSettings,
    pub population: PopulationState,
    pub traffic: vehicles::TrafficState,
    pub overrides: vehicles::VehicleOverrides,
    /// Prop obstacles (upstream's `PedObstacles`, filled by the peds plugin's
    /// `update_ped_obstacles`). skate_vita: updated from the props for the cars' look-ahead
    /// without the ped navmesh gate (no peds yet; a district always has its navmesh in retail).
    pub obstacles: skate_core::living_world::peds::NavObstacles,
    /// The NPC skaters (replay tier).
    pub npcs: npc_skaters::NpcState,
    /// The peds (body tier) and the car contacts logged in the last second.
    pub peds: peds::PedState,
    /// Where the world's data is (the ped data loads when peds are switched on).
    asset_root: std::path::PathBuf,
    map: String,
    ped_cars_logged: std::collections::BTreeMap<(u64, u64), u64>,
    /// The GLB paths of the models the census can spawn (`vehicles.json`), sorted: what
    /// [`CarPose::model`] indexes.
    models: Vec<String>,
    last_traffic_report: u64,
}

impl LivingWorldState {
    /// `load_for_map` + `load_traffic_data`: the data of `map`. `None` when the world has no
    /// living-world data (parks, the Course).
    pub(crate) fn load(asset_root: &std::path::Path, map: &str, settings: LivingWorldSettings) -> Option<Self> {
        let started = std::time::Instant::now();
        let data = load_data(asset_root, map);
        info!("LIVING_WORLD data {}", data.status);
        if data.census.is_none() {
            return None;
        }
        let population = PopulationState::install(map, 0, &settings, data);
        let traffic = vehicles::TrafficState::load(asset_root);
        // Memory: the ped data loads only while peds are on (`ensure_peds`).
        let peds = if settings.pedestrians.enabled { peds::PedState::load(asset_root, map, population.roads.as_ref()) } else { Default::default() };
        let mut models: Vec<String> = population
            .vehicles
            .iter()
            .flat_map(|c| c.entities.values())
            .filter_map(|e| traffic.data.models.get(&e.model).map(|m| m.glb.clone()))
            .collect();
        models.sort();
        models.dedup();
        info!("LIVING_WORLD loaded in {:.0} ms ({} car models)", started.elapsed().as_secs_f32() * 1000.0, models.len());
        Some(Self {
            settings,
            population,
            traffic,
            overrides: Default::default(),
            obstacles: skate_core::living_world::peds::NavObstacles::new(Default::default()),
            npcs: Default::default(),
            peds,
            asset_root: asset_root.to_path_buf(),
            map: map.to_owned(),
            ped_cars_logged: Default::default(),
            models,
            last_traffic_report: 0,
        })
    }

    /// Between the controls and the physics: the proxies are rebuilt (single player:
    /// `multiplayer::prepare` with no remotes), then `push_vehicle_proxies` and
    /// `push_vehicle_grab_splines`.
    pub(crate) fn before_physics(&mut self, physics: &mut crate::physics::GamePhysics, skater: &crate::physics::SkaterRuntime) {
        // Each released hand prop's frame before the step (the renderer blends it with the next).
        if let Some(dynamics) = physics.prop_dynamics() {
            for (id, _, previous) in &mut self.peds.released {
                *previous = dynamics.pose(*id).map(prop_frame);
            }
        }
        crate::physics::network::prepare(physics);
        vehicles::push_vehicle_proxies(&self.traffic, physics, skater);
        let observers: Vec<_> = local_observer(physics, skater).into_iter().collect();
        ped_body::push_ped_proxies(&self.peds, &self.settings, &observers, physics, skater);
        ped_hand_props::push_hand_prop_proxies(&self.peds, &self.settings, physics, skater);
        npc_skaters::push_proxies(&self.npcs, physics, skater, &self.settings);
        vehicles::push_vehicle_grab_splines(&self.traffic, physics);
    }

    /// After the physics: `gather_observers`, `step_population`, the obstacle update, then
    /// `apply_vehicle_records`, `apply_vehicle_hits`, `drive_traffic` and `log_traffic`.
    pub(crate) fn after_physics(&mut self, physics: &mut crate::physics::GamePhysics, skater: &mut crate::physics::SkaterRuntime, seconds: f64, camera: Option<glam::Vec3>) {
        let observers: Vec<_> = local_observer(physics, skater).into_iter().collect();
        let mut decisions = self.step_population(&observers, seconds);
        let props = physics.prop_dynamics().map(peds::prop_obstacle_inputs).unwrap_or_default();
        // The peds' chain in upstream's order (`apply_ped_records`, `release_rejected`,
        // `update_ped_obstacles`, `think_peds`, `sync_hand_props`, `apply_skater_takedowns`, `advance_peds`).
        // The ped data loads the first time peds are on.
        if self.settings.pedestrians.enabled && !self.peds.data.ready() {
            self.peds = peds::PedState::load(&self.asset_root, &self.map, self.population.roads.as_ref());
        }
        {
            let _scope = skate_core::profile::scope("z_peds");
            // The events `think_peds` read go (upstream's message cursor); the rest wait for it.
            let read = self.peds.think.cursor.min(self.peds.events.len());
            self.peds.events.drain(..read);
            self.peds.think.cursor = 0;
            peds::apply_ped_records(&decisions, physics, &mut self.peds);
            decisions.extend(peds::release_rejected(&mut self.peds, &mut self.population));
            peds::update_ped_obstacles(&self.settings, &self.peds.data, physics, &mut self.obstacles, &observers);
            // The behaviour runtime. The traffic events are the last step's (horns at peds).
            let wiping = [skater.player_input.processed.flags_2468 & npc_sim::WIPEOUT_BIT != 0];
            let cars: Vec<(u64, [f32; 3], [f32; 3])> = self.traffic.index.values().map(|(e, _)| (e.meta.id.to_u64(), e.motion.curr.translation.to_array(), (e.motion.curr.rotation * glam::Vec3::Z).to_array())).collect();
            peds::think_peds(&self.population, &mut self.peds, &self.settings, &observers, &wiping, &self.traffic.events, &cars, Some(&*physics));
            ped_hand_props::sync_hand_props(&mut self.peds, &self.settings, &observers, physics, &self.asset_root, self.population.world.tick());
            peds::apply_skater_takedowns(&mut self.peds.takedowns, skater, physics);
            let npc_actors: Vec<(u64, [f32; 3], [f32; 3])> = self.npcs.index.values().filter_map(|n| n.replay.last.as_ref().map(|s| (n.npc.id.to_u64(), s.position, s.velocity))).collect();
            peds::advance_peds(&mut self.population, &mut self.peds, self.traffic.clock.as_ref(), physics, &self.obstacles, &observers, Some(&*skater), npc_actors, &self.settings);
        }
        let ped_list: Vec<(u64, [f32; 3])> = {
            let mut l: Vec<_> = self.peds.index.values().map(|e| (e.ped.id.to_u64(), e.body.position.to_array())).collect();
            l.sort_by_key(|x| x.0);
            l
        };
        let ped_agent = self.peds.data.nav.as_deref().map(|m| (m.agent[1], m.agent[3]));
        // The NPC skaters (`apply_records`, `npc_avoid::avoid`, `advance`, `track_stance`), then
        // their puppet poses (upstream draws them in `Update`).
        {
            let _scope = skate_core::profile::scope("z_npc");
            self.npcs.events.clear();
            npc_skaters::apply_records(&decisions, &self.population, &self.settings, &mut self.npcs);
            let cars: Vec<_> = self.traffic.index.values().map(|(e, _)| (e.meta.id.to_u64(), e.motion.curr, e.meta.bounds, e.motion.velocity)).collect();
            let mut events = std::mem::take(&mut self.npcs.events);
            let (index, track) = (&mut self.npcs.index, &mut self.npcs.avoid);
            let mut list: Vec<&mut npc_skaters::NpcEntity> = index.values_mut().collect();
            npc_avoid::avoid(&self.settings, &observers, &self.population, track, seconds as f32, &ped_list, ped_agent, &cars, &props, &mut list, &mut events);
            self.npcs.events = events;
            npc_skaters::advance(&self.settings, &observers, &mut self.population, &mut self.npcs);
            npc_skaters::stance_and_pose(&self.settings, skater, &self.population, &mut self.npcs);
        }
        self.traffic.events.clear();
        vehicles::apply_vehicle_records(&decisions, &self.population, &self.overrides, &mut self.traffic);
        vehicles::apply_vehicle_hits(skater, &mut self.traffic);
        let held = (skater.player_state.current() == skate_core::player::state::PhysicalStateId::Skitching).then(|| skater.skitch_state.held_car()).flatten();
        // The cars that left are counted and removed there (upstream also writes their despawn
        // messages for the audio and mods, which the Vita does not have).
        vehicles::drive_traffic(&self.overrides, &mut self.population, &mut self.traffic, &observers, &ped_list, held, Some(&self.obstacles));
        // Cars push the peds they touch (after both stepped), then the peds' draw state.
        vehicle_contacts::ped_vehicle_contacts(&self.settings, &mut self.population, &mut self.peds, &self.traffic, &mut self.ped_cars_logged);
        peds::present_peds(&self.settings, &self.population, &mut self.peds, camera);
        for e in &self.traffic.events {
            match e {
                vehicles::TrafficEvent::Spawned { id, entity, model, .. } => debug!("TRAFFIC spawned #{} {entity} ({model})", id.serial),
                vehicles::TrafficEvent::Despawned { id, reason } => debug!("TRAFFIC despawned #{} {reason:?}", id.serial),
                _ => {}
            }
        }
        if self.settings.debug && report_due(self.population.world.tick(), &mut self.last_traffic_report, 300) {
            info!("{} / {}", readout(&self.population), vehicles::traffic_readout(&self.traffic));
        }
    }

    /// One fixed step of `step_population`: settings into the config, then the console ticks that
    /// are due.
    fn step_population(&mut self, observers: &[skate_core::living_world::Observer], seconds: f64) -> Vec<Decision> {
        let _scope = skate_core::profile::scope("z_population");
        let state = &mut self.population;
        let mut config = state.data_config.clone();
        self.settings.apply(&mut config);
        state.world.config = config;
        let inputs = TickInputs {
            observers,
            census: state.census.as_ref(),
            skater_world: state.skaters.as_ref().map(|s| s as &dyn skate_core::living_world::SkaterWorld),
            roads: state.roads.as_ref(),
            vehicles: state.vehicles.as_ref(),
            online: false,
            zombie: self.settings.zombie,
            free_play: self.settings.free_play,
            player_slots: 1,
            scripted_skaters: 0,
            ambient_skater_override: None,
            world_ready: true,
        };
        let decisions = state.world.advance(seconds, &inputs);
        for d in &decisions {
            match d {
                Decision::Spawn(_) => state.spawned += 1,
                Decision::Despawn(_) => state.despawned += 1,
            }
        }
        if self.settings.debug && report_due(state.world.tick(), &mut state.last_report, 300) {
            let focus: Vec<String> = observers.iter().map(|o| format!("[{:.1}, {:.1}, {:.1}] {:.1} m/s", o.position[0], o.position[1], o.position[2], o.velocity.iter().map(|v| v * v).sum::<f32>().sqrt())).collect();
            info!("{} focus {}", readout(state), focus.join(" / "));
        }
        decisions
    }

    /// The ped model recipes (sorted; [`peds::PedPose::model`] indexes them; the bake stores each
    /// at `private/living_world/models/<recipe>.ped`).
    pub(crate) fn ped_models(&self) -> Vec<String> {
        let mut v: Vec<String> = self.peds.data.catalog.models.values().map(|m| m.recipe.clone()).filter(|r| !r.is_empty()).collect();
        v.sort();
        v.dedup();
        v
    }

    /// The peds as the renderer draws them (those in view).
    pub(crate) fn ped_poses(&self, models: &[String], out: &mut Vec<peds::PedPose>) {
        out.clear();
        out.extend(self.peds.index.values().filter_map(|e| {
            Some(peds::PedPose {
                serial: e.ped.id.serial,
                model: models.binary_search(&e.ped.recipe).ok()? as u16,
                previous: e.previous_pose.clone()?,
                current: e.pose.clone()?,
                tint_a: e.ped.tint_a,
                tint_b: e.ped.tint_b,
                alpha: e.alpha,
                lod: e.lod,
                hand: e.held.as_ref().and_then(|h| Some((self.peds.data.hand_prop_model(&h.key)?, h.local))),
            })
        }));
    }

    /// The hand prop models (`HandPropPose::model` and `PedPose::hand` index them).
    pub(crate) fn hand_prop_models(&self) -> Vec<String> {
        self.peds.data.hand_prop_models()
    }

    /// The released hand props as the renderer draws them.
    pub(crate) fn hand_prop_poses(&self, physics: &crate::physics::GamePhysics, out: &mut Vec<peds::HandPropPose>) {
        out.clear();
        let Some(dynamics) = physics.prop_dynamics() else { return };
        out.extend(self.peds.released.iter().filter_map(|(id, key, previous)| {
            let current = prop_frame(dynamics.pose(*id)?);
            Some(peds::HandPropPose { id: *id, model: self.peds.data.hand_prop_model(key)?, previous: previous.unwrap_or(current), current })
        }));
    }

    /// The ped rig (bone names, parents, which bones the clips animate), for the renderer's
    /// binding of each ped model.
    pub(crate) fn ped_rig(&self) -> (Vec<String>, Vec<i32>, Vec<bool>) {
        let r = &self.peds.data.rig;
        (r.names.clone(), r.parents.iter().map(|&p| p as i32).collect(), r.animated.clone())
    }

    /// The NPC skaters as the renderer draws them.
    pub(crate) fn npc_poses(&self, out: &mut Vec<npc_skaters::NpcPose>) {
        out.clear();
        out.extend(self.npcs.index.values().filter_map(|n| {
            Some(npc_skaters::NpcPose { serial: n.npc.id.serial, previous: n.previous_pose.clone()?, current: n.pose.clone()?, alpha: n.fade.alpha })
        }));
    }

    /// The GLB paths of the car models (sorted; [`CarPose::model`] indexes them).
    pub(crate) fn car_models(&self) -> &[String] {
        &self.models
    }

    /// The cars as the renderer draws them.
    pub(crate) fn car_poses(&self, out: &mut Vec<CarPose>) {
        out.clear();
        out.extend(self.traffic.index.iter().map(|(&serial, (e, _))| {
            let (m, t) = (&e.motion, &e.meta);
            CarPose {
                serial,
                model: self.models.binary_search(&t.glb).map_or(u16::MAX, |i| i as u16),
                prev: (m.drawn_prev.translation.to_array(), m.drawn_prev.rotation.to_array()),
                curr: (m.prev.translation.to_array(), m.prev.rotation.to_array()),
                wheel_prev: m.wheel_drawn_prev,
                wheel: m.wheel_prev,
                chassis: t.chassis,
                secondary: t.secondary,
                paint_gain: t.paint_gain,
            }
        }));
    }
}

#[cfg(test)]
mod ped_tests {
    /// DownTown peds (body tier): they spawn on the navmesh around the idle skater, walk and
    /// get a pose when in view (desktop check).
    #[test]
    #[ignore = "requires the living-world export, stock graphs and an installed map (SKATE3_ASSET_ROOT, SKATE3_MAP=<maps/DownTown.skate>)"]
    fn downtown_peds_walk_and_pose() {
        let root = std::path::PathBuf::from(std::env::var_os("SKATE3_ASSET_ROOT").unwrap());
        let map = skate_data::skate_map::SkateMap::load(std::path::Path::new(&std::env::var_os("SKATE3_MAP").unwrap())).unwrap();
        let mut session = crate::live::Session::load_map(&root, "easy", Some(map)).unwrap();
        let started = std::time::Instant::now();
        assert!(session.start_living_world(super::LivingWorldSettings::default()));
        eprintln!("living world loaded in {:.0} ms: {}", started.elapsed().as_secs_f32() * 1000.0, session.living.as_ref().unwrap().peds.data.status);
        let (mut most, mut posed) = (0, 0);
        let mut walked = 0.0f32;
        let (mut hash, mut clip_bytes) = (0xcbf2_9ce4_8422_2325u64, 0usize);
        let started = std::time::Instant::now();
        for _ in 0..1200 {
            let before: std::collections::BTreeMap<_, _> = session.living.as_ref().unwrap().peds.index.iter().map(|(id, e)| (*id, e.body.position)).collect();
            let report = session.step(Default::default());
            assert!(report.error.is_none(), "{:?}", report.error);
            let peds = &session.living.as_ref().unwrap().peds;
            most = most.max(peds.index.len());
            posed = posed.max(peds.index.values().filter(|e| e.pose.is_some()).count());
            walked += peds.index.iter().filter_map(|(id, e)| before.get(id).map(|b| b.distance(e.body.position))).sum::<f32>();
            for e in peds.index.values() {
                let mut bits: Vec<f32> = e.body.position.to_array().to_vec();
                bits.push(e.body.heading);
                if let Some((_, _, g)) = &e.pose {
                    bits.extend(g.iter().flat_map(|m| m.to_cols_array()));
                }
                for x in bits {
                    hash = (hash ^ x.to_bits() as u64).wrapping_mul(0x0100_0000_01b3);
                }
            }
            clip_bytes = clip_bytes.max(peds.data.clips.bytes());
        }
        let living = session.living.as_ref().unwrap();
        for e in living.peds.index.values().take(6) {
            eprintln!("  #{} {} ({}) {:?} at {:?} alpha {:.2} lod {}", e.ped.id.serial, e.ped.recipe, e.ped.anim_set, e.body.player.state, e.body.position, e.alpha, e.lod);
        }
        let mut profile = skate_core::profile::take();
        profile.sort_by_key(|p| std::cmp::Reverse(p.1));
        // The brains' graph states at the end.
        if let Some(graph) = living.peds.data.graph.as_deref() {
            let mut states = std::collections::BTreeMap::<&str, usize>::new();
            for e in living.peds.index.values() {
                *states.entry(e.mind.state_name(graph)).or_default() += 1;
            }
            eprintln!("  brain states: {states:?}; conversations {}, chase groups {}, plugin props {}", living.peds.conversations.map.len(), living.peds.chase_groups.0.len(), living.peds.plugin_props.props.len());
            let held: Vec<&str> = living.peds.index.values().filter_map(|e| e.held.as_ref().map(|h| h.key.as_str())).collect();
            eprintln!("  hand props: held {held:?}, released {}, linked {}", living.peds.released.len(), living.peds.index.values().filter(|e| e.linked.is_some()).count());
        }
        for (name, total, _) in profile.iter().filter(|p| p.0.starts_with("z_ped") || p.0 == "z_think_peds" || p.0 == "z_tick" || p.0 == "z_lw_obstacles").take(8) {
            eprintln!("  {name}: {:.3} ms/tick", *total as f64 / 1e6 / 1200.0);
        }
        eprintln!("{}; most peds {most}, posed {posed}, walked {walked:.0} m in total; 1200 ticks in {:.1} s", super::readout(&living.population), started.elapsed().as_secs_f32());
        eprintln!("PED_HASH {hash:016x}; decoded clips at most {:.1} MB", clip_bytes as f64 / 1e6);
        assert!(most > 0, "peds spawn");
        assert!(walked > 1.0, "peds walk");
    }
}

#[cfg(test)]
mod npc_tests {
    /// DownTown NPC skaters (replay tier): they spawn on the district's lines around the idle
    /// skater, ride (their cursors advance) and get a puppet pose every step (desktop check).
    #[test]
    #[ignore = "requires the living-world export, stock graphs and an installed map (SKATE3_ASSET_ROOT, SKATE3_MAP=<maps/DownTown.skate>)"]
    fn downtown_npc_skaters_ride_and_pose() {
        let root = std::path::PathBuf::from(std::env::var_os("SKATE3_ASSET_ROOT").unwrap());
        let map = skate_data::skate_map::SkateMap::load(std::path::Path::new(&std::env::var_os("SKATE3_MAP").unwrap())).unwrap();
        let mut session = crate::live::Session::load_map(&root, "easy", Some(map)).unwrap();
        let started = std::time::Instant::now();
        assert!(session.start_living_world(super::LivingWorldSettings::default()));
        let living = session.living.as_ref().unwrap();
        let lines = &living.population.npc.lines;
        let nodes: usize = lines.values().map(|l| l.nodes.len()).sum();
        eprintln!("living world loaded in {:.0} ms; {} replay lines, {} nodes, {} skater lines", started.elapsed().as_secs_f32() * 1000.0, lines.len(), nodes, living.population.skaters.as_ref().map_or(0, |s| s.lines.len()));
        let (mut poses, mut most, mut posed) = (Vec::new(), 0, 0);
        let mut hash = 0xcbf2_9ce4_8422_2325u64;
        let started = std::time::Instant::now();
        for _ in 0..1200 {
            let report = session.step(Default::default());
            assert!(report.error.is_none(), "{:?}", report.error);
            session.npc_poses(&mut poses);
            most = most.max(session.living.as_ref().unwrap().npcs.index.len());
            posed = posed.max(poses.len());
            for n in session.living.as_ref().unwrap().npcs.index.values() {
                let flags = n.stance.flags;
                hash = (hash ^ (u64::from(flags.mirrored) | u64::from(flags.board_backward) << 1 | u64::from(flags.switch) << 2)).wrapping_mul(0x0100_0000_01b3);
                let matrices: Vec<glam::Mat4> = n.pose.iter().flat_map(|p| p.bones.iter().copied().chain(std::iter::once(p.root.to_matrix()))).collect();
                for m in matrices {
                    for x in m.to_cols_array() {
                        hash = (hash ^ x.to_bits() as u64).wrapping_mul(0x0100_0000_01b3);
                    }
                }
            }
        }
        let living = session.living.as_ref().unwrap();
        for n in living.npcs.index.values() {
            let s = n.replay.last.as_ref();
            eprintln!("  #{} {} line node {} frames {} phase {:?} alpha {:.2} at {:?}", n.npc.id.serial, n.npc.character, n.replay.cursor.node, n.replay.cursor.frames, s.map(|s| s.phase), n.fade.alpha, s.map(|s| s.position));
        }
        let mut profile = skate_core::profile::take();
        profile.sort_by_key(|p| std::cmp::Reverse(p.1));
        for (name, total, _) in profile.iter().filter(|p| p.0.starts_with("z_npc") || p.0 == "z_tick").take(8) {
            eprintln!("  {name}: {:.3} ms/tick", *total as f64 / 1e6 / 1200.0);
        }
        eprintln!("{}; most NPCs {most}, posed {posed}; 1200 ticks in {:.1} s", super::readout(&living.population), started.elapsed().as_secs_f32());
        eprintln!("NPC_HASH {hash:016x}");
        assert!(most > 0, "NPC skaters spawn");
        assert!(posed > 0, "NPC skaters get a pose");
    }
}

#[cfg(test)]
mod traffic_tests {
    /// A DownTown session with the living world drives traffic: cars spawn on the district's
    /// lanes around the idle skater and move (desktop check; the cost is measured on the device).
    #[test]
    #[ignore = "requires the living-world export, stock graphs and an installed map (SKATE3_ASSET_ROOT, SKATE3_MAP=<maps/DownTown.skate>)"]
    fn downtown_traffic_spawns_and_drives() {
        let root = std::path::PathBuf::from(std::env::var_os("SKATE3_ASSET_ROOT").unwrap());
        let map = skate_data::skate_map::SkateMap::load(std::path::Path::new(&std::env::var_os("SKATE3_MAP").unwrap())).unwrap();
        let mut session = crate::live::Session::load_map(&root, "easy", Some(map)).unwrap();
        let settings = super::LivingWorldSettings { debug: true, ..Default::default() };
        let started = std::time::Instant::now();
        assert!(session.start_living_world(settings), "DownTown has living-world data");
        eprintln!("living world loaded in {:.0} ms", started.elapsed().as_secs_f32() * 1000.0);
        let t = std::time::Instant::now();
        drop(super::load_data(&root, "DownTown"));
        eprintln!("  data: {:.0} ms", t.elapsed().as_secs_f32() * 1000.0);
        let t = std::time::Instant::now();
        drop(super::vehicles::TrafficState::load(&root));
        eprintln!("  traffic: {:.0} ms", t.elapsed().as_secs_f32() * 1000.0);
        let mut poses = Vec::new();
        let mut most = 0;
        let mut hash = 0xcbf2_9ce4_8422_2325u64;
        let started = std::time::Instant::now();
        for _ in 0..1200 {
            let report = session.step(Default::default());
            assert!(report.error.is_none(), "{:?}", report.error);
            session.car_poses(&mut poses);
            most = most.max(poses.len());
            for c in &poses {
                for x in c.curr.0.iter().chain(&c.curr.1).chain([&c.wheel]) {
                    hash = (hash ^ (x.to_bits() as u64 ^ (c.serial as u64) << 32)).wrapping_mul(0x0100_0000_01b3);
                }
            }
        }
        let living = session.living.as_ref().unwrap();
        let moving = living.traffic.cars.iter().filter(|c| c.speed > 0.5).count();
        eprintln!("TRAFFIC_HASH {hash:016x}");
        eprintln!("{} / {}; most cars {most}; 1200 ticks in {:.1} s", super::readout(&living.population), super::vehicles::traffic_readout(&living.traffic), started.elapsed().as_secs_f32());
        let mut profile = skate_core::profile::take();
        profile.sort_by_key(|p| std::cmp::Reverse(p.1));
        for (name, total, entries) in profile.iter().take(30) {
            eprintln!("  {name}: {:.3} ms/tick ({entries})", *total as f64 / 1e6 / 1200.0);
        }
        assert!(most > 0, "cars spawn");
        assert!(moving > 0, "cars drive");
    }
}

/// The stage spans' totals for `perf_bench` (sim-bench's subscriber), with `BENCH_SPANS=1`.
#[cfg(all(test, feature = "oracle"))]
#[path = "../../../tools/sim-bench/src/profile.rs"]
mod bench_span_profile;

/// Run with `--features stage-profile,oracle` (the skater dump is the oracle's).
#[cfg(all(test, feature = "oracle"))]
mod perf_bench {
    use super::bench_span_profile as span_profile;

    /// FNV-1a over bytes.
    fn fold(hash: &mut u64, bytes: &[u8]) {
        for b in bytes {
            *hash = (*hash ^ *b as u64).wrapping_mul(0x0100_0000_01b3);
        }
    }

    /// The ped navmesh's path search on DownTown (the chase re-plans every think): paths between pseudo-random
    /// polygon pairs of one component, the time per search and a hash of the corners (an exact change keeps it).
    #[test]
    #[ignore = "requires an installed DownTown (SKATE3_ASSET_ROOT, SKATE3_MAP=<maps/DownTown.skate>)"]
    fn downtown_path_bench() {
        let root = std::path::PathBuf::from(std::env::var_os("SKATE3_ASSET_ROOT").unwrap());
        let map = skate_data::skate_map::SkateMap::load(std::path::Path::new(&std::env::var_os("SKATE3_MAP").unwrap())).unwrap();
        let mut session = crate::live::Session::load_map(&root, "easy", Some(map)).unwrap();
        let mut settings = super::LivingWorldSettings::default();
        settings.pedestrians = super::KindSetting { enabled: true, density: 0.25 };
        assert!(session.start_living_world(settings));
        session.step(Default::default());
        let mesh = session.living.as_ref().and_then(|l| l.peds.data.nav.clone()).expect("DownTown's ped navmesh");
        let n = mesh.polys.len() as u32;
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % u64::from(n)) as u32
        };
        let point = |poly: u32| skate_core::living_world::peds::NavPoint { poly, position: mesh.polys[poly as usize].centre };
        let mut pairs = Vec::new();
        while pairs.len() < 200 {
            let (a, b) = (next(), next());
            if a != b && mesh.reachable(a, b) {
                pairs.push((point(a), point(b)));
            }
        }
        let mut hash = 0xcbf2_9ce4_8422_2325u64;
        let started = std::time::Instant::now();
        let mut found = 0;
        for (a, b) in &pairs {
            if let Some(corners) = mesh.find_path(*a, *b) {
                found += 1;
                fold(&mut hash, format!("{corners:?}").as_bytes());
            }
        }
        let per = started.elapsed().as_secs_f64() * 1e6 / pairs.len() as f64;
        eprintln!("PATH_HASH {hash:016x} polys={n} found={found}/{} {per:.1} us a search", pairs.len());
        // A chase's searches: to a polygon within 25 m.
        let mut near = Vec::new();
        while near.len() < 200 {
            let (a, b) = (next(), next());
            let (pa, pb) = (mesh.polys[a as usize].centre, mesh.polys[b as usize].centre);
            if a != b && mesh.reachable(a, b) && (pa[0] - pb[0]).hypot(pa[2] - pb[2]) < 25.0 {
                near.push((point(a), point(b)));
            }
        }
        let started = std::time::Instant::now();
        for (a, b) in &near {
            if let Some(corners) = mesh.find_path(*a, *b) {
                fold(&mut hash, format!("{corners:?}").as_bytes());
            }
        }
        let per = started.elapsed().as_secs_f64() * 1e6 / near.len() as f64;
        eprintln!("PATH_HASH_NEAR {hash:016x} {per:.1} us a search within 25 m");
    }

    /// The Vita's DownTown session on the desktop (Fast physics, Low traffic and peds, NPC skaters, the game
    /// switches): 1440 ticks on foot walking about, then 1560 on the board pushing and carving. Prints the stage
    /// profile (`--features stage-profile`) and one hash of every tick's skater dump and every drawn pose (skater,
    /// cars, NPC skaters, peds, hand props, props): an exact optimization leaves the hash unchanged.
    #[test]
    #[ignore = "requires an installed DownTown (SKATE3_ASSET_ROOT, SKATE3_MAP=<maps/DownTown.skate>)"]
    fn downtown_perf_bench() {
        let spans: Option<&'static span_profile::Profiler> = std::env::var_os("BENCH_SPANS").map(|_| &*Box::leak(Box::new(span_profile::Profiler::new())));
        if let Some(p) = spans {
            tracing::subscriber::set_global_default(p).unwrap();
        }
        crate::live::enable_game_switches();
        let root = std::path::PathBuf::from(std::env::var_os("SKATE3_ASSET_ROOT").unwrap());
        let map = skate_data::skate_map::SkateMap::load(std::path::Path::new(&std::env::var_os("SKATE3_MAP").unwrap())).unwrap();
        let mut session = crate::live::Session::load_map(&root, "easy", Some(map)).unwrap();
        session.set_solver_iterations(25);
        let mut settings = super::LivingWorldSettings::default();
        settings.vehicles = super::KindSetting { enabled: true, density: 0.2 };
        settings.pedestrians = super::KindSetting { enabled: true, density: 0.25 };
        settings.skaters.enabled = true;
        assert!(session.start_living_world(settings));
        let ped_models = session.ped_models();
        if let Some(p) = spans {
            let _ = p.take();
        }
        let ticks: u32 = std::env::var("BENCH_TICKS").ok().and_then(|v| v.parse().ok()).unwrap_or(3000);
        let _ = skate_core::profile::take();
        let (mut cars, mut npcs, mut peds, mut hands, mut props) = (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
        let mut hash = 0xcbf2_9ce4_8422_2325u64;
        let mut dump = Vec::new();
        let mut states = std::collections::BTreeMap::new();
        let mut step_time = std::time::Duration::ZERO;
        // `BENCH_INPUT_OUT=<file>`: the pads as a sim-oracle scenario (the Vita's autoplay plays it on DownTown).
        let mut input_out = std::env::var_os("BENCH_INPUT_OUT").map(|_| String::from("# difficulty=easy\n"));
        for t in 0..ticks {
            let mut pad = skate_core::input::xbox::XboxState::default();
            let stick = |deg: f32| {
                let a = deg.to_radians();
                [(a.cos() * 32767.0) as i16, (a.sin() * 32767.0) as i16]
            };
            match t {
                60 | 1440 => pad.buttons = 0x8000,
                121..1440 => pad.left = stick(90.0 + 60.0 * (t / 240) as f32),
                1500.. => {
                    if (t - 1500) % 40 < 18 {
                        pad.buttons = 0x1000;
                    }
                    pad.left = stick(90.0 + 25.0 * (((t - 1500) / 200) as f32).sin());
                }
                _ => {}
            }
            if let Some(text) = &mut input_out {
                text.push_str(&format!("{} {} {} {} {} {} {}\n", pad.buttons, pad.triggers[0], pad.triggers[1], pad.left[0], pad.left[1], pad.right[0], pad.right[1]));
            }
            let started = std::time::Instant::now();
            let report = session.step(pad);
            step_time += started.elapsed();
            assert!(report.error.is_none(), "{:?}", report.error);
            *states.entry(session.state_name()).or_insert(0u32) += 1;
            dump.clear();
            session.write_tick(&mut dump, false).unwrap();
            fold(&mut hash, &dump);
            if let Some((_, current)) = session.presentation() {
                fold(&mut hash, format!("{:?}{:?}{:?}", current.root, current.bones, current.camera).as_bytes());
            }
            session.car_poses(&mut cars);
            session.npc_poses(&mut npcs);
            session.ped_poses(&ped_models, &mut peds);
            session.hand_prop_poses(&mut hands);
            session.prop_poses(&mut props);
            fold(&mut hash, format!("{cars:?}{peds:?}{hands:?}{props:?}").as_bytes());
            for n in &npcs {
                fold(&mut hash, format!("{} {} {:?}{:?}", n.serial, n.alpha, n.current.root, n.current.bones).as_bytes());
            }
        }
        if let (Some(text), Some(path)) = (input_out, std::env::var_os("BENCH_INPUT_OUT")) {
            std::fs::write(&path, text).unwrap();
        }
        eprintln!("PERF_HASH {hash:016x}");
        eprintln!("{ticks} ticks, {:.3} ms a tick in step; states {states:?}", step_time.as_secs_f64() * 1000.0 / ticks as f64);
        eprintln!("{}", session.ped_brain_summary());
        let mut profile = skate_core::profile::take();
        profile.sort_by_key(|p| std::cmp::Reverse(p.1));
        for (name, total, entries) in profile.iter().filter(|p| !p.0.starts_with("n_") && !p.0.starts_with("q_") && !p.0.ends_with("_n")).take(45) {
            eprintln!("  {name}: {:.4} ms/tick ({entries})", *total as f64 / 1e6 / ticks as f64);
        }
        // Counters (`profile::count`): the mean per call.
        for (name, total, entries) in profile.iter().filter(|p| p.0.starts_with("n_") || p.0.starts_with("q_") || p.0.ends_with("_n")) {
            eprintln!("  {name}: {:.1} per call ({entries} calls)", *total as f64 / (*entries).max(1) as f64);
        }
        if let Some(p) = spans {
            let mut v = p.take();
            v.sort_by_key(|e| std::cmp::Reverse(e.1));
            for (name, total, entries) in v.iter().take(40) {
                eprintln!("  span {name}: {:.4} ms/tick ({entries})", total.as_secs_f64() * 1e3 / ticks as f64);
            }
        }
    }
}
