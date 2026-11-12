//! skate_vita addition: the live simulation session for the game loop.
//!
//! `Session::step` is one fixed tick in skate-game's system order, the same body as
//! the parity oracle's tick loop (tools/sim-oracle/harness/oracle_harness.rs `run`):
//! customiser preferences (PreUpdate), input publication, `controls::sample`, then
//! `frame::advance` unless the simulation has failed. `sim-oracle run-live` replays
//! the scenario corpus through this type and must give bit-identical dumps.
//! The caller owns the clock: it runs one step per `period()` (stock `clock.rs`).
use super::{frame, GamePhysics, PlayerControls, SkaterRuntime};
use skate_core::input::xbox::XboxState;
use std::path::Path;
use std::time::Duration;

/// Render-side presentation: the copied skate-game `presentation.rs` snapshots,
/// its `blend`, and the copied Bevy `Transform` they are made of.
pub use crate::presentation::{Snapshot, blend};
pub use crate::transform::Transform;

pub struct Session {
    graphs: crate::graph_runtime::StockGraphs,
    pub(crate) physics: GamePhysics,
    skater: SkaterRuntime,
    controls: PlayerControls,
    camera: crate::camera::CameraRuntime,
    input: crate::input::ControllerInput,
    profile: serde_json::Value,
    ticks: u64,
    error: Option<String>,
    /// Presentation history, captured after every tick as skate-game's capture
    /// system does (render-only; it reads the simulation and never feeds back).
    presentation: crate::presentation::Presentation,
    /// skate_vita: the skater's audio publication (the reference's
    /// `game_audio::skate_events::observe`, run after every tick) and its memory.
    audio_cues: crate::audio_events::Cues,
    audio_seen: crate::audio_events::Seen,
    /// The world's spawn (ground anchor, heading), for `respawn`.
    spawn: ([f32; 3], f32),
    /// Manual session markers (off unless `set_session_markers(true)`: the oracle
    /// corpus never holds LB, but the dumps must not depend on this code).
    markers: Option<crate::session_marker::SessionMarker>,
    asset_root: std::path::PathBuf,
    /// Decoded stock animation keys are trimmed to this many bytes once a second;
    /// the bytes left after the last trim.
    animation_budget: usize,
    animation_bytes: usize,
    /// The placed props' streaming census (skate-game's `stream_dmos`, run after each tick).
    dmo_settings: crate::living_world::dmo_stream::DmoStreamGameSettings,
    dmo: crate::living_world::dmo_stream::DmoStreamState,
    /// Each prop's template range (MOBJ `first_index`, `index_count`), by id.
    prop_templates: std::collections::HashMap<u32, (u32, u32)>,
    /// The loaded world's name and its living world (population and traffic; off unless
    /// `start_living_world`: the oracle corpus runs without it).
    map_name: Option<String>,
    pub(crate) living: Option<crate::living_world::LivingWorldState>,
    /// The local player's career stats (off unless `start_career_stats`: the oracle corpus runs without them).
    career: Option<crate::career_stats::CareerStatsTracker>,
    /// skate_vita: the Object Dropper (made when it first opens; its scene stays with the world).
    pub(crate) dropper: Option<crate::object_dropper::world::Dropper>,
    /// The "All objects" setting (ours): the catalogue lists every item anywhere.
    dropper_all: bool,
    /// The "Free camera" setting (ours): the open world's dropper camera follows the cursor instead of standing still.
    dropper_free_camera: bool,
    /// The measuring limits (D6d; the Vita's debug overlay).
    dropper_measure: bool,
    /// The editor asked for, waiting for the skater to stop: ticks waited.
    dropper_pending: Option<u32>,
}

/// A placed prop streamed in, as the renderer draws it: its template range in the props
/// render and its pose (origin, basis columns).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PropPose {
    /// The prop body's stable id (the replay recorder's key).
    pub id: u32,
    pub first_index: u32,
    pub index_count: u32,
    pub origin: [f32; 3],
    pub basis: [[f32; 3]; 3],
    /// The instance's per-axis scale (1 for most props).
    pub scale: [f32; 3],
}

/// A shared view of the static collision world ([`Session::camera_world`]).
pub struct CameraWorld(std::sync::Arc<skate_core::physics::board_world::BoardWorld>);

impl CameraWorld {
    /// The gameplay camera's fat line (`camera::world_query::line`) from `from` to `to` with `radius`: the first hit's
    /// fraction along the segment and the hit normal.
    pub fn sweep(&self, from: glam::Vec3, to: glam::Vec3, radius: f32) -> Option<(f32, glam::Vec3)> {
        let lanes = |v: glam::Vec3| [v.x, v.y, v.z, 0.0];
        let hit = crate::camera::world_query::line(&self.0, lanes(from), lanes(to), radius).ok()?;
        (hit.hit != 0).then(|| (hit.fraction, glam::Vec3::new(hit.normal[0], hit.normal[1], hit.normal[2])))
    }
}

/// Default bound of the decoded stock animation keys (M7).
pub const ANIMATION_BUDGET: usize = 24 << 20;

pub use crate::scoring_runtime::HudFeed;

/// See `Session::manual_view`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ManualView {
    pub active: bool,
    pub value: f32,
}

/// The score module's state for a HUD (M7): what skate-game's removed `hud_input`
/// published to its scoring movie.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Score {
    /// The trick being scored (localization id, e.g. `ID_TRICK_FLIP_KICKFLIP`).
    pub trick: String,
    pub stance: [bool; 3],
    /// Points of the current sequence (with the multiplier).
    pub sequence: f32,
    /// Points of the current line, and the line timer in seconds.
    pub line: f32,
    pub line_time: f32,
    pub multiplier: f32,
    /// Banked total of the session.
    pub total: f32,
    pub clean: bool,
    pub sketchy: bool,
}

/// A trick the score module recognized.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Trick {
    /// Localization id, e.g. `ID_TRICK_FLIP_KICKFLIP`.
    pub label: String,
    pub switch: bool,
    pub fakie: bool,
    pub nollie: bool,
}

/// What a step produced, for the caller's display or logging.
#[derive(Clone, Debug, Default)]
pub struct StepReport {
    /// The trick the score module recognized this tick (`new_trick`).
    pub new_trick: Option<Trick>,
    /// The tick's simulation error; the session stays failed afterwards.
    pub error: Option<String>,
    /// Native audio ids of this tick's session marker events (`AUDIO_*`).
    pub marker_audio: Vec<u64>,
    /// The Object Dropper's events this tick (while it is open).
    pub editor: Vec<crate::object_dropper::Event>,
}

/// skate-game's `LivingWorldPlugin` switches (PR #52 `618e824`): prop grab (Move Object on
/// the authored grab records) and skitching are always on in the game (process-wide; the oracle
/// corpus and the unit tests keep them off, as upstream's tests do).
pub fn enable_game_switches() {
    crate::physics::prop_dynamics::set_prop_grab_enabled(true);
    crate::physics::set_skitch_enabled(true);
}

/// The asked-for editor's wait (retail `8270A5F0`): at most 4 s (`0x82257308`), the skater taken as stopped below
/// 0.2 m/s once the brake has had a quarter second; B held meanwhile (the skater's brake).
const DROPPER_WAIT: u32 = 240;
const DROPPER_SETTLE: u32 = 15;
const DROPPER_STOPPED: f32 = 0.2;
const DROPPER_BRAKE: u16 = 0x2000;

impl Session {
    /// The production load in skate-game's order (manifest, graphs, physics, skater,
    /// controls, camera). `difficulty` is `easy`, `normal` or `hardcore`.
    pub fn load(asset_root: &Path, difficulty: &str) -> Result<Self, String> {
        Self::load_map(asset_root, difficulty, None)
    }

    /// The same load on a retail map instead of the Course: `validate_runtime`, then
    /// the map's collision, rails and spawn, as skate-game's map startup does. The
    /// decoded map is dropped once the world is built.
    pub fn load_map(
        asset_root: &Path,
        difficulty: &str,
        map: Option<skate_data::skate_map::SkateMap>,
    ) -> Result<Self, String> {
        let difficulty = crate::difficulty::Difficulty::parse(difficulty)?;
        if let Some(map) = &map {
            crate::skate_world::validate_runtime(map)?;
        }
        let spawn = map.as_ref().map_or(([0.0, super::ground::HEIGHT, 0.0], 0.0), |m| (m.spawn, m.heading));
        let manifest = skate_data::GameAssets::load(asset_root).map_err(|e| format!("{e:?}"))?;
        let graphs = crate::graph_runtime::StockGraphs::load(asset_root, &manifest)?;
        let physics = GamePhysics::load_with_difficulty(asset_root, map.as_ref(), difficulty)?;
        // The props' templates for drawing them (only when the world has props).
        let prop_templates = match (&map, physics.prop_dynamics()) {
            (Some(map), Some(_)) => crate::skate_world::prop_template_ranges(asset_root, &map.name),
            _ => Default::default(),
        };
        let map_name = map.as_ref().map(|m| m.name.clone());
        drop(map);
        let skater = SkaterRuntime::load(asset_root, &graphs, &physics, difficulty.key())?;
        let controls = PlayerControls::load(asset_root)?;
        let camera = crate::camera::CameraRuntime::load(asset_root)?;
        let mut session = Self {
            graphs,
            physics,
            skater,
            controls,
            camera,
            input: crate::input::ControllerInput::default(),
            profile: serde_json::json!({}),
            ticks: 0,
            error: None,
            presentation: Default::default(),
            audio_cues: Default::default(),
            audio_seen: Default::default(),
            spawn,
            animation_budget: ANIMATION_BUDGET,
            animation_bytes: 0,
            dmo_settings: Default::default(),
            dmo: Default::default(),
            prop_templates,
            map_name,
            living: None,
            career: None,
            dropper: None,
            dropper_all: false,
            dropper_free_camera: false,
            dropper_measure: false,
            dropper_pending: None,
            markers: None,
            asset_root: asset_root.to_path_buf(),
        };
        // The census's first pass parks the props far from the spawn while the session
        // loads (a district's hundreds of props: 1.3 s on the Vita when it ran on the first tick),
        // then the props left streamed in get their collision.
        if session.physics.prop_dynamics().is_some() {
            let observers: Vec<_> = crate::living_world::local_observer(&session.physics, &session.skater).into_iter().collect();
            crate::living_world::dmo_stream::stream(session.dmo_settings, &session.asset_root, 0, &observers, None, &mut session.physics, &mut session.dmo);
            session.physics.prepare_props();
        }
        Ok(session)
    }

    /// Starts the living world of the loaded world (its population and traffic) with
    /// `settings`; false when the world has no living-world data (parks, the Course).
    /// Starts the career stats from these totals (the game's saved ones); they count from the next tick.
    pub fn start_career_stats(&mut self, stats: skate_core::career_stats::CareerStats) {
        self.career = Some(crate::career_stats::CareerStatsTracker::new(stats));
    }

    /// The career stats totals when they changed since the last call (the game saves them).
    pub fn changed_career_stats(&mut self) -> Option<&skate_core::career_stats::CareerStats> {
        let career = self.career.as_mut().filter(|c| c.dirty)?;
        career.dirty = false;
        Some(&career.stats)
    }

    pub fn start_living_world(&mut self, settings: crate::living_world::LivingWorldSettings) -> bool {
        self.living = self.map_name.as_deref().and_then(|map| crate::living_world::LivingWorldState::load(&self.asset_root, map, settings));
        if let Some(living) = &mut self.living {
            living.npcs.scores = crate::living_world::npc_skaters::NpcScoreClips(crate::graph_host::motion::stock_score_clips(&self.graphs.motion.source));
        }
        self.living.is_some()
    }
    /// Changes the living world's settings (the menu); in effect from the next tick.
    pub fn set_living_settings(&mut self, settings: crate::living_world::LivingWorldSettings) {
        if let Some(living) = &mut self.living {
            living.settings = settings;
        }
    }
    /// The GLB paths of the living world's car models (`CarPose::model` indexes them; the
    /// bake stores each drawable at the path with `.car` for `.glb`).
    pub fn car_models(&self) -> Vec<String> {
        self.living.as_ref().map_or_else(Vec::new, |l| l.car_models().to_vec())
    }
    /// The traffic cars as the renderer draws them (empty without a living world).
    pub fn car_poses(&self, out: &mut Vec<crate::living_world::CarPose>) {
        match &self.living {
            Some(living) => living.car_poses(out),
            None => out.clear(),
        }
    }
    /// The NPC skaters as the renderer draws them (empty without a living world).
    pub fn npc_poses(&self, out: &mut Vec<crate::living_world::npc_skaters::NpcPose>) {
        match &self.living {
            Some(living) => living.npc_poses(out),
            None => out.clear(),
        }
    }
    /// The ped model recipes (`PedPose::model` indexes them).
    pub fn ped_models(&self) -> Vec<String> {
        self.living.as_ref().map_or_else(Vec::new, |l| l.ped_models())
    }
    /// The peds in view as the renderer draws them (`models` = `ped_models()`).
    pub fn ped_poses(&self, models: &[String], out: &mut Vec<crate::living_world::peds::PedPose>) {
        match &self.living {
            Some(living) => living.ped_poses(models, out),
            None => out.clear(),
        }
    }
    /// The hand prop models (asset paths of the baked `.hprop` files).
    pub fn hand_prop_models(&self) -> Vec<String> {
        self.living.as_ref().map_or_else(Vec::new, |l| l.hand_prop_models())
    }
    /// The released hand props as the renderer draws them.
    pub fn hand_prop_poses(&self, out: &mut Vec<crate::living_world::peds::HandPropPose>) {
        match &self.living {
            Some(living) => living.hand_prop_poses(&self.physics, out),
            None => out.clear(),
        }
    }
    /// The ped rig (bone names, parents, animated flags).
    pub fn ped_rig(&self) -> Option<(Vec<String>, Vec<i32>, Vec<bool>)> {
        self.living.as_ref().map(|l| l.ped_rig())
    }
    /// The population counts (skaters, peds, cars) for diagnostics.
    /// Call a skater by character key (FE 62 Call Skater): the living world places it next (None: no living world).
    pub fn call_skater(&mut self, key: &str) -> Option<skate_core::living_world::CallOutcome> {
        self.living.as_mut().map(|l| l.population.world.call_skater(key))
    }
    /// Dismiss a called skater: its slot frees, a spawned one rides on as an ambient skater. True when it was called.
    pub fn dismiss_skater(&mut self, key: &str) -> bool {
        self.living.as_mut().is_some_and(|l| l.population.world.dismiss_skater(key))
    }
    /// The called skaters' NPC serials (the ones riding now), oldest first.
    pub fn called_serials(&self) -> Vec<u32> {
        self.living.as_ref().map_or_else(Vec::new, |l| l.population.world.companions().iter().filter_map(|c| c.serial).collect())
    }
    /// The called skaters' character keys, oldest first.
    pub fn called_skaters(&self) -> Vec<String> {
        self.living.as_ref().map_or_else(Vec::new, |l| l.population.world.companions().iter().map(|c| c.key.clone()).collect())
    }
    pub fn living_counts(&self) -> Option<[usize; 3]> {
        use skate_core::living_world::Kind;
        self.living.as_ref().map(|l| [Kind::Skater, Kind::Pedestrian, Kind::Vehicle].map(|k| l.population.world.count(k)))
    }

    /// The peds' behaviour at a glance (diagnostics): how many peds are in each AI graph state, the
    /// live conversations and chase groups. Empty without peds or their graph.
    pub fn ped_brain_summary(&self) -> String {
        let Some(l) = self.living.as_ref() else { return String::new() };
        let Some(graph) = l.peds.data.graph.as_deref() else { return String::new() };
        let mut states = std::collections::BTreeMap::<&str, usize>::new();
        for e in l.peds.index.values() {
            *states.entry(e.mind.state_name(graph)).or_default() += 1;
        }
        let states: Vec<String> = states.iter().map(|(k, n)| format!("{k}:{n}")).collect();
        format!("states [{}] conversations {} chases {}", states.join(" "), l.peds.conversations.map.len(), l.peds.chase_groups.0.len())
    }

    /// Turns the manual session markers on or off (LB held shows the prompt;
    /// LB + D-pad down places a marker, LB + D-pad up held returns to it).
    /// Opens the Object Dropper at the skater (the phone's dropper row): its catalogue and tuning come from the baked
    /// catalogue (`object_dropper::world::CATALOG`).
    pub fn open_editor(&mut self) -> Result<(), String> {
        if self.dropper.is_none() {
            self.dropper = Some(crate::object_dropper::world::Dropper::load(&self.asset_root, &self.physics, self.map_name.as_deref())?);
        }
        let (at, _) = self.board();
        let yaw = self.camera_yaw();
        // Edit Park (FE 0x35) in a skate.Park, the phone's Object Dropper (FE 0x37) in the open world.
        let variant = if self.physics.is_park() { crate::object_dropper::Variant::Park } else { crate::object_dropper::Variant::World };
        let listing = self.dropper_listing(self.dropper_all).unwrap_or_default();
        let limits = if self.dropper_measure { crate::object_dropper::Limits::measure() } else { crate::object_dropper::Limits::default() };
        if let Some(dropper) = &mut self.dropper {
            dropper.open(glam::Vec3::from_array(at), yaw, variant, listing, self.dropper_free_camera);
            dropper.set_limits(limits);
        }
        Ok(())
    }

    /// The loaded world is a skate.Park (the phone's Edit Park and Reset All Props rows), not the open world.
    pub fn in_skate_park(&self) -> bool {
        self.physics.is_park()
    }

    /// The Object Dropper's measure line (D6d, the budgets set from device measurements): what the world holds against
    /// the limits, None before the editor first opened. `used` counts the world's items; the frame and sim times are
    /// in the same log.
    pub fn dropper_usage_line(&self) -> Option<String> {
        let editor = self.dropper.as_ref()?.editor()?;
        let used = editor.scene.usage(&editor.catalog);
        let l = &editor.limits;
        let m = editor.meter();
        let catalogue_items = editor.scene.items.iter().filter(|p| p.origin == crate::object_dropper::Origin::Dropped || p.tint != 0).count();
        let awake = self.physics.prop_dynamics().map_or(0, |d| d.awake_runtime());
        Some(format!(
            "DROPPER usage: memory {:.1}/{:.1} MiB, terrain {} pieces / {} of {} triangles, props {}/{} ({awake} awake), catalogue-drawn items {}, meter terrain {:.2} props {:.2}",
            used.memory as f64 / 1048576.0,
            l.memory as f64 / 1048576.0,
            used.terrain_pieces,
            used.terrain_cost,
            l.terrain_cost,
            used.props_dropped,
            l.props_dropped,
            catalogue_items,
            m.terrain[0],
            m.props[0],
        ))
    }

    /// The measuring limits (`Limits::measure`), taken at the next opening.
    pub fn set_dropper_measure(&mut self, on: bool) {
        self.dropper_measure = on;
    }

    /// The "All objects" setting: the catalogue lists every item anywhere (taken at the next opening).
    pub fn set_dropper_all(&mut self, all: bool) {
        self.dropper_all = all;
    }

    /// The "Free camera" setting: the open world's dropper camera moves with the cursor (taken at the next opening).
    pub fn set_dropper_free_camera(&mut self, free: bool) {
        self.dropper_free_camera = free;
    }

    /// The open editor, for its screen.
    pub fn dropper_editor(&self) -> Option<&crate::object_dropper::Editor> {
        self.dropper.as_ref()?.editor()
    }

    /// The Quick Menu's Style / Branding list (the movie's `Game.OnMenuHilight` / `OnMenuSelect`): row `index`
    /// highlighted or selected.
    pub fn dropper_quick_list(&mut self, index: usize, select: bool) {
        if let Some(dropper) = &mut self.dropper {
            dropper.quick_list(index, select, &mut self.physics, &self.asset_root);
        }
    }

    /// The Quick Menu's screen modes posted since the last call (`object_dropper::quick::screen`).
    pub fn take_dropper_quick_posts(&mut self) -> Vec<u8> {
        self.dropper.as_mut().map(|d| d.take_quick_posts()).unwrap_or_default()
    }

    /// The screen modes the editor posted since the last call (`object_dropper::screen`).
    pub fn take_dropper_posts(&mut self) -> Vec<u8> {
        self.dropper.as_mut().map(|d| d.take_posts()).unwrap_or_default()
    }

    /// The phone's Object Dropper row: the editor opens once the skater has stopped (`8270A5F0`; the skater brakes
    /// meanwhile).
    pub fn request_editor(&mut self) {
        if !self.editor_open() && self.dropper_pending.is_none() {
            self.dropper_pending = Some(0);
        }
    }

    /// The editor is asked for and waits for the skater to stop.
    pub fn editor_pending(&self) -> bool {
        self.dropper_pending.is_some()
    }

    pub fn editor_open(&self) -> bool {
        self.dropper.as_ref().is_some_and(|d| d.is_open())
    }

    pub fn close_editor(&mut self) {
        if let Some(dropper) = &mut self.dropper {
            dropper.close(&mut self.physics, &self.asset_root);
        }
        self.to_spawner();
    }

    /// Closing the park editor puts the skater at the park's spawner (retail `825CB440`); the open world's dropper
    /// leaves the skater where it was.
    fn to_spawner(&mut self) {
        let Some(pose) = self.dropper.as_ref().filter(|_| self.physics.is_park()).and_then(|d| d.spawner_pose()) else { return };
        if let Err(error) = self.skater.travel_to(pose) {
            tracing::warn!("SKATE_DROPPER: to the spawner: {error}");
        }
    }

    /// The world's Object Dropper edits as a save file's bytes, when they changed since the last call (the caller
    /// writes them, off the simulation thread).
    pub fn take_dropper_save(&mut self) -> Option<Vec<u8>> {
        self.dropper.as_mut()?.take_save().map(|s| s.to_bytes())
    }

    /// Replays a world's saved edits (`take_dropper_save`'s bytes) after the world loaded.
    pub fn restore_dropper(&mut self, bytes: &[u8]) -> Result<(), String> {
        let saved = crate::object_dropper::save::Saved::from_bytes(bytes)?;
        if self.dropper.is_none() {
            self.dropper = Some(crate::object_dropper::world::Dropper::load(&self.asset_root, &self.physics, self.map_name.as_deref())?);
        }
        if let Some(dropper) = &mut self.dropper {
            dropper.restore(&saved, &mut self.physics, &self.asset_root);
        }
        Ok(())
    }

    /// Every Object Dropper edit undone: the park back to its layout, the dropped items gone (the save follows), and
    /// the world's props moved by play back on their spawn pose (retail's reset question in the open world,
    /// `825C6638` -> `82884100` on the DMO manager). An open editor stays open.
    pub fn reset_dropper(&mut self) {
        if let Some(dropper) = &mut self.dropper {
            dropper.reset_all(&mut self.physics, &self.asset_root);
        }
        let props = self.physics.reset_moved_props();
        tracing::info!("SKATE_DROPPER: reset all, {} props back", props.len());
    }

    /// The catalogue's choice: an item id of `dropper_catalog()`.
    pub fn editor_choose(&mut self, item: &str) -> Vec<crate::object_dropper::Event> {
        self.dropper.as_mut().map(|d| d.choose(item)).unwrap_or_default()
    }

    /// What the catalogue lists in this world (`Catalog::listing`): a park everything, the open world the props of
    /// the DMO banks it loads; `all` (the "All objects" setting) everything anywhere.
    pub fn dropper_listing(&self, all: bool) -> Option<Vec<(String, Vec<(String, Vec<String>)>)>> {
        let catalog = self.dropper.as_ref()?.catalog();
        Some(catalog.listing(self.physics.is_park(), self.map_name.as_deref().unwrap_or_default(), all))
    }

    pub fn dropper_catalog(&self) -> Option<&crate::object_dropper::Catalog> {
        self.dropper.as_ref().map(|d| d.catalog())
    }

    /// The skate.park being skated as a retail `.park` file's bytes (Save Park), None outside a park; the Object
    /// Dropper's catalogue loads if it has not yet.
    pub fn park_file(&mut self) -> Option<Vec<u8>> {
        if !self.physics.is_park() {
            return None;
        }
        if self.dropper.is_none() {
            match crate::object_dropper::world::Dropper::load(&self.asset_root, &self.physics, self.map_name.as_deref()) {
                Ok(d) => self.dropper = Some(d),
                Err(error) => {
                    tracing::warn!("SKATE_PARK: {error}");
                    return None;
                }
            }
        }
        let asset_root = self.asset_root.clone();
        self.dropper.as_mut().map(|d| d.park_bytes(&asset_root))
    }

    /// The skate.park has Object Dropper edits not saved yet.
    pub fn park_unsaved(&self) -> bool {
        self.dropper.as_ref().is_some_and(|d| d.unsaved())
    }

    /// Lays a saved skate.park (`.park` bytes, My Parks) over the park being skated (the editor closes first); the
    /// skater goes to its spawner. Returns the pieces kept, dropped and refused (over the budgets).
    pub fn load_park(&mut self, bytes: &[u8]) -> Result<(usize, usize, usize), String> {
        if !self.physics.is_park() {
            return Err("not in a skate.park".into());
        }
        if self.dropper.is_none() {
            self.dropper = Some(crate::object_dropper::world::Dropper::load(&self.asset_root, &self.physics, self.map_name.as_deref())?);
        }
        let Some(dropper) = self.dropper.as_mut() else { return Err("no dropper".into()) };
        let counts = dropper.load_park(bytes, self.map_name.as_deref(), &mut self.physics, &self.asset_root)?;
        self.to_spawner();
        Ok(counts)
    }

    /// Park Setup's lots and templates (none before the dropper first loaded).
    pub fn park_lots(&self) -> Vec<crate::object_dropper::world::ParkLot> {
        self.dropper.as_ref().map(|d| d.lots().to_vec()).unwrap_or_default()
    }

    /// Lays a Park Setup template (New Park: `world`, the template row's key, on this park's map) over the park being
    /// skated, as [`Self::load_park`]; it counts as saved.
    pub fn load_template(&mut self, world: &str) -> Result<(usize, usize, usize), String> {
        if self.dropper.is_none() {
            self.dropper = Some(crate::object_dropper::world::Dropper::load(&self.asset_root, &self.physics, self.map_name.as_deref())?);
        }
        let bytes = self.dropper.as_ref().ok_or("no dropper")?.template_park(world, &self.asset_root)?;
        self.load_park(&bytes)
    }

    /// The park teleporters the player placed (none before the editor first opened).
    pub fn dropper_teleporters(&self) -> Vec<crate::object_dropper::world::ParkTeleporter> {
        self.dropper.as_ref().map(|d| d.teleporters()).unwrap_or_default()
    }

    /// What the renderer draws of the editor and of the world's edits (none before the editor first opened).
    pub fn dropper_frame(&self) -> Option<crate::object_dropper::world::DropperFrame> {
        self.dropper.as_ref().map(|d| d.frame(&self.physics))
    }

    /// The camera's heading around the skater (yaw about +y; 0 looks down -z), for opening the editor behind them.
    fn camera_yaw(&self) -> f32 {
        self.presentation
            .pair()
            .map(|(_, current)| {
                // The camera looks down its -z (a Bevy transform).
                let f = current.camera.rotation * glam::Vec3::NEG_Z;
                (-f.x).atan2(-f.z)
            })
            .unwrap_or(0.0)
    }

    pub fn set_session_markers(&mut self, on: bool) -> Result<(), String> {
        self.markers = if on { Some(crate::session_marker::SessionMarker::new(&self.asset_root)?) } else { None };
        Ok(())
    }
    /// What the marker overlay shows.
    /// The manual's state for the on-screen meter (a read-only tap): `value` is the measured
    /// deck tilt as a fraction of the maximum tilt (-1 tail .. 1 nose), `active` while a
    /// manual is held.
    pub fn manual_view(&self) -> ManualView {
        let m = &self.skater.ground.manual;
        let max = self.skater.ground_settings.manual_max_tilt();
        let value = if max > 0.0 && m.measured_angle.is_finite() { (m.measured_angle / max).clamp(-1.0, 1.0) } else { 0.0 };
        ManualView { active: m.elapsed != 0.0, value }
    }
    /// Streamed collision diagnostics (see `GamePhysics::window_stats`).
    pub fn window_stats(&self) -> Option<(u32, f32, f32, usize)> {
        self.physics.window_stats()
    }
    pub fn marker_view(&self) -> crate::session_marker::MarkerView {
        self.markers.as_ref().map_or_else(Default::default, |m| m.view)
    }
    /// Gameplay resumes after a menu: nothing happens until LB is released.
    pub fn suspend_session_markers(&mut self) {
        if let Some(m) = &mut self.markers {
            m.suspend();
        }
    }

    /// Changes the decoded-animation bound (bytes; see `ANIMATION_BUDGET`).
    pub fn set_animation_budget(&mut self, bytes: usize) {
        self.animation_budget = bytes;
    }
    /// Decoded stock animation bytes after the last trim.
    pub fn animation_bytes(&self) -> usize {
        self.animation_bytes
    }

    /// One fixed tick with this pad state.
    pub fn step(&mut self, state: XboxState) -> StepReport {
        let _tick = skate_core::profile::scope("z_tick");
        // A slow tick names the phase that took the time (always on: the profiled builds never showed the 75 ms tick
        // of 11 Nov 2026). `lap` is the time since the last call, in ms.
        let mut mark = std::time::Instant::now();
        let started = mark;
        let mut laps = [0.0f32; 8];
        let mut lap = |slot: usize| {
            let now = std::time::Instant::now();
            laps[slot] = (now - mark).as_secs_f32() * 1000.0;
            mark = now;
        };
        // skate_vita (Object Dropper): an asked-for editor waits for the skater to stop (retail's flow `8270A5F0`:
        // input locked, the editor opens once the skater no longer moves, at most 4 s); meanwhile the skater holds the
        // brake (B: brake, then off the board). NOT RETAIL YET: retail's lock message (`0x7949dac1`) is not decoded.
        let state = match self.dropper_pending {
            Some(ticks) => {
                let (_, speed) = self.board();
                if (ticks >= DROPPER_SETTLE && speed < DROPPER_STOPPED) || ticks >= DROPPER_WAIT {
                    self.dropper_pending = None;
                    if let Err(error) = self.open_editor() {
                        tracing::warn!("SKATE_DROPPER: {error}");
                    }
                    XboxState::default()
                } else {
                    self.dropper_pending = Some(ticks + 1);
                    XboxState { buttons: DROPPER_BRAKE, ..XboxState::default() }
                }
            }
            None => state,
        };
        // While the editor is open it takes the pad and the skater gets a neutral one.
        let mut editor_events = Vec::new();
        let state = match self.dropper.as_mut().filter(|d| d.is_open()) {
            Some(dropper) => {
                let slots = skate_core::input::xbox::convert(&state, 0);
                editor_events = dropper.update(&slots, &mut self.physics, &self.asset_root);
                let exit = editor_events.contains(&crate::object_dropper::Event::Exit);
                if exit {
                    dropper.close(&mut self.physics, &self.asset_root);
                    self.to_spawner();
                }
                XboxState::default()
            }
            None => state,
        };
        lap(0);
        // 1. PreUpdate customiser preferences.
        crate::customiser::apply_preferences(&self.profile, &mut self.physics, &mut self.skater.animation);
        // 2. Input.
        let published = crate::input::live_input::publish(&mut self.input, state);
        // 2b. Session markers: after the input, before the controls (the reference's
        // `SimulationSet::Input` → marker `update` → `SimulationSet::Controls`).
        let mut report = StepReport { editor: editor_events, ..StepReport::default() };
        if let Some(markers) = &mut self.markers {
            report.marker_audio = markers.update(self.physics.period().as_secs_f64(), &self.input, &self.physics, &mut self.skater);
        }
        // 3. Controls: body of controls::sample.
        let mut map = published.0.actions();
        if let Err(error) =
            self.controls
                .update_for_physics(&mut map, &self.physics, &self.skater, &self.camera)
        {
            // The game panics here ("Offboard controller publication"); keep the
            // message and stop the simulation instead.
            report.error = Some(format!("Offboard controller publication: {error}"));
            self.physics.failed = true;
        }
        self.controls.publish_gestures(
            self.physics.animation_profile.physics_mode,
            self.skater.player_input.physical.state.state_16,
        );
        lap(1);
        // 4. Physics: body of the advance system (vehicles are never occupied here).
        // Streamed collision follows the deck (a no-op for whole worlds).
        if let Err(error) = self.physics.follow_window(&self.skater) {
            self.physics.failed = true;
            report.error = Some(error);
        }
        lap(2);
        // The car proxies and grab splines between the controls and the physics (skate-game
        // runs `multiplayer::prepare`, `push_vehicle_proxies` and `push_vehicle_grab_splines` there).
        if let Some(living) = &mut self.living {
            living.before_physics(&mut self.physics, &self.skater);
        }
        lap(3);
        if !self.physics.failed {
            let mut actions = published.0.actions();
            let input_available = published.0.controller_available();
            if let Err(error) = frame::advance(
                &mut self.physics,
                &mut self.skater,
                &mut self.controls,
                &self.graphs,
                &mut actions,
                input_available,
                &mut self.camera,
            ) {
                self.physics.failed = true;
                report.error = Some(error);
            }
        }
        lap(4);
        // 5. Presentation capture (skate-game runs it after the fixed tick, with the
        // period as it stands after the tick).
        crate::presentation::capture(&self.skater, &self.camera, self.physics.period(), &mut self.presentation);
        // The props near the player wake, the others are parked (skate-game runs the
        // census in Update, after the fixed tick); a world without props skips it.
        if report.error.is_none() && self.physics.prop_dynamics().is_some() {
            let observers: Vec<_> = crate::living_world::local_observer(&self.physics, &self.skater).into_iter().collect();
            let camera = self.presentation.pair().map(|(_, now)| (now.camera.translation.to_array(), (now.camera.rotation * glam::Vec3::NEG_Z).to_array()));
            let _dmo = skate_core::profile::scope("z_dmo_stream");
            crate::living_world::dmo_stream::stream(self.dmo_settings, &self.asset_root, 0, &observers, camera, &mut self.physics, &mut self.dmo);
        }
        lap(5);
        // The population and the traffic after the fixed tick (skate-game's `FixedUpdate`
        // systems after `SimulationSet::Physics`).
        if report.error.is_none() {
            if let Some(living) = &mut self.living {
                let _scope = skate_core::profile::scope("z_living_world");
                let camera = self.presentation.pair().map(|(_, now)| now.camera.translation);
                let period = self.physics.period().as_secs_f64();
                living.after_physics(&mut self.physics, &mut self.skater, period, camera);
            }
        }
        lap(6);
        // The career stats after the population (skate-game's `track` after `SimulationSet::Physics`; the ped hits are
        // the ones `advance_peds` reported this tick).
        if report.error.is_none() {
            if let Some(career) = &mut self.career {
                career.track(&self.physics, &mut self.skater, self.physics.period().as_secs_f32());
                if let Some(living) = &self.living {
                    for entity in living.player_ped_hits() {
                        career.ped_hit(entity);
                    }
                }
            }
        }
        // The audio publication after the tick (the reference's `observe` system runs after
        // Physics). It consumes the animation's latched audio events, which physics never reads.
        if report.error.is_none() {
            crate::audio_events::observe(&self.physics, &mut self.skater, &mut self.audio_cues, &mut self.audio_seen, self.physics.period().as_secs_f32());
        }
        if let Some(error) = &report.error {
            self.error.get_or_insert_with(|| error.clone());
        }
        if self.skater.scoring.new_trick {
            let [switch, fakie, _, nollie] = self.skater.scoring.trick_stance();
            report.new_trick = Some(Trick {
                label: self.skater.scoring.trick_label().to_owned(),
                switch,
                fakie,
                nollie,
            });
        }
        self.ticks += 1;
        // Bound the lazily decoded animation keys. After the tick, so nothing
        // borrowed from them is alive; a released key decodes to the same words.
        if self.ticks % 60 == 0 {
            self.animation_bytes = self.skater.animation.evaluator.frames.trim_decoded(self.animation_budget);
        }
        lap(7);
        let total = (std::time::Instant::now() - started).as_secs_f32() * 1000.0;
        if total > 40.0 {
            static SLOW: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
            if SLOW.fetch_add(1, std::sync::atomic::Ordering::Relaxed) % 20 == 0 {
                println!(
                    "SLOW_TICK {total:.1} ms (tick {}): dropper and customiser {:.1}, input and controls {:.1}, window {:.1}, before-physics {:.1}, physics {:.1}, props census {:.1}, living world {:.1}, rest and animation trim {:.1}; state {:?}",
                    self.ticks, laps[0], laps[1], laps[2], laps[3], laps[4], laps[5], laps[6], laps[7], self.state_name()
                );
            }
        }
        report
    }

    /// The skater's audio publication of the last tick and the ticks published since the last
    /// call (`skate_audio`'s native player reads it; the reference's `Cues`).
    /// The presentation's snaps so far (teleports, camera cuts): the audio host's cut signal.
    pub fn presentation_cuts(&self) -> u64 {
        self.presentation.cuts
    }

    /// The awake props' audio entries since the last call (impacts keep their strongest contact), for the audio host.
    pub fn take_prop_audio(&mut self, out: &mut Vec<crate::PropAudioSample>) {
        match self.physics.prop_dynamics_mut() {
            Some(dynamics) => dynamics.take_audio_samples(out),
            None => out.clear(),
        }
    }

    pub fn take_audio(&mut self) -> (crate::AudioRiding, u32) {
        (self.audio_cues.riding, std::mem::take(&mut self.audio_cues.steps))
    }

    /// Current physical state id (skate-core `PhysicalStateId`: 100 PhysicsGround, 101
    /// SlideGround, 2xx air, 300 WipeoutGround, 40x grinds, 5xx off board, 702
    /// Teleporting, ...).
    pub fn state_id(&self) -> u32 {
        self.skater.player_state.current() as u32
    }

    /// The score module's state for the HUD.
    pub fn score(&self) -> Score {
        let s = &self.skater.scoring;
        let (sequence, clean, sketchy) = s.hud_sequence();
        let [switch, fakie, _, nollie] = s.trick_stance();
        let drain = s.data.line_drain;
        Score {
            trick: s.trick_label().to_owned(),
            stance: [switch, fakie, nollie],
            sequence,
            line: s.session.holder.snapshot.line,
            line_time: if drain > 0.0 { s.session.line.points / drain } else { 0.0 },
            multiplier: s.session.combo.multiplier,
            total: s.session.holder.snapshot.completed_lines,
            clean,
            sketchy,
        }
    }

    /// This tick's input for the original HUD movie (see `HudFeed`).
    pub fn hud_feed(&self) -> HudFeed {
        self.skater.scoring.hud_feed()
    }

    /// Changes the difficulty live, as skate-game's menu does (`set_difficulty`): the
    /// next tick's input selects the new physics mode.
    pub fn set_difficulty(&mut self, difficulty: &str) -> Result<(), String> {
        let d = crate::difficulty::Difficulty::parse(difficulty)?;
        self.physics.set_difficulty(d);
        Ok(())
    }

    /// Constraint iterations of the board/skater solve: 50 is retail (the default), 25 the
    /// Vita's "Fast" physics (`GamePhysics::set_solver_iterations`).
    /// The profile the customiser's preferences come from (`customiser::apply_preferences`, every tick).
    pub fn set_profile(&mut self, profile: serde_json::Value) {
        self.profile = profile;
    }

    pub fn set_solver_iterations(&mut self, iterations: u32) {
        self.physics.set_solver_iterations(iterations);
    }

    /// Teleports the skater to the world's spawn, as skate-game's pause-menu teleport
    /// does (`SkaterRuntime::travel_to`, a manual teleport through the stock state).
    pub fn respawn(&mut self) -> Result<(), String> {
        // In a park the spawner, once the editor has placed it (it may have been moved).
        if let Some(pose) = self.dropper.as_ref().filter(|_| self.physics.is_park()).and_then(|d| d.spawner_pose()) {
            return self.skater.travel_to(pose);
        }
        let ([x, y, z], heading) = self.spawn;
        let (s, c) = heading.sin_cos();
        // teleports.json layout: rows right, up, forward, translation.
        self.skater.travel_to([[c, 0.0, -s, 0.0], [0.0, 1.0, 0.0, 0.0], [s, 0.0, c, 0.0], [x, y, z, 1.0]])
    }

    /// Teleports the skater to a locator (the Challenge Map's locations: `teleports.json` rows right, up, forward,
    /// translation), as `respawn` does for the spawn.
    pub fn teleport(&mut self, matrix: [[f32; 4]; 4]) -> Result<(), String> {
        self.skater.travel_to(matrix)
    }

    /// The fixed timestep for the next tick (stock `clock.rs`; 1/60 s unless the
    /// camera requested slow motion).
    pub fn period(&self) -> Duration {
        self.physics.period()
    }
    /// The poses of the props streamed in (not parked), into `out` (cleared first).
    pub fn prop_poses(&self, out: &mut Vec<PropPose>) {
        out.clear();
        let Some(dynamics) = self.physics.prop_dynamics() else { return };
        for (id, origin, basis, scale) in dynamics.streamed_poses() {
            let Some(&(first_index, index_count)) = self.prop_templates.get(&id) else { continue };
            out.push(PropPose { id, first_index, index_count, origin: [origin.x, origin.y, origin.z], basis: basis.columns, scale: [scale.x, scale.y, scale.z] });
        }
    }

    /// The static collision world for sweeps outside the step (the replay cameras while the session is paused).
    /// Drop it before the next `step`: the step changes the world in place and needs it unshared.
    pub fn camera_world(&self) -> CameraWorld {
        CameraWorld(self.physics.world_geometry())
    }

    /// The world's placed props and how many are streamed in (not parked); `None` without props.
    pub fn prop_counts(&self) -> Option<(usize, usize)> {
        let dynamics = self.physics.prop_dynamics()?;
        let placements = dynamics.dmo_placements();
        let awake = placements.iter().filter(|p| !dynamics.is_dormant(p.id)).count();
        Some((placements.len(), awake))
    }

    pub fn ticks(&self) -> u64 {
        self.ticks
    }
    /// The first simulation error, if the session has failed.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
    /// Current physical state, e.g. `PhysicsGround`.
    pub fn state_name(&self) -> String {
        format!("{:?}", self.skater.player_state.current())
    }
    /// Current motion-graph animation, e.g. `B_OLLIE_N_0`.
    pub fn animation_name(&self) -> Option<&str> {
        self.skater.animation.motion.animation.current_name.as_deref()
    }
    /// The score module's last recognized trick label.
    pub fn trick_label(&self) -> &str {
        self.skater.scoring.trick_label()
    }
    /// Deck position (m) and speed (m/s).
    pub fn board(&self) -> ([f32; 3], f32) {
        let deck = &self.physics.board.bodies()[6].rates;
        let v = deck.linear_velocity;
        (
            [deck.position.x, deck.position.y, deck.position.z],
            (v.x * v.x + v.y * v.y + v.z * v.z).sqrt(),
        )
    }

    /// The last two presentation snapshots (previous, current) of completed ticks,
    /// blended by the renderer as skate-game's `present` does. On a camera cut or a
    /// cadence change both are the current tick (no blend across the cut).
    pub fn presentation(&self) -> Option<(&Snapshot, &Snapshot)> {
        self.presentation.pair()
    }
    /// Stock rig bone names; `Snapshot::bones` is indexed the same way. The skater
    /// GLB's skin joints are matched to them by name, case-insensitively.
    pub fn bone_names(&self) -> &[String] {
        &self.skater.animation.evaluator.frames.bone_names
    }
    /// Each stock rig bone's parent (-1: a root), in [`Self::bone_names`] order.
    pub fn bone_parents(&self) -> &[i32] {
        &self.skater.animation.evaluator.frames.parents
    }

    /// Oracle dump of this session (feature `oracle`): the harness header and one
    /// record per tick, so `sim-oracle run-live` can compare against `run`.
    #[cfg(feature = "oracle")]
    pub fn write_header(&self, out: &mut dyn std::io::Write) -> std::io::Result<()> {
        super::oracle_harness::write_header(out, &self.physics, &self.skater)
    }
    #[cfg(feature = "oracle")]
    pub fn write_tick(&self, out: &mut dyn std::io::Write, failed: bool) -> std::io::Result<()> {
        super::oracle_harness::write_tick(
            out,
            (self.ticks - 1) as u32,
            failed,
            &self.physics,
            &self.skater,
            &self.camera,
        )
    }
}

/// The procedural Course's quads by surface group (0 floor, 1 platform/ramp/rails,
/// 2 half pipe, 3 climbing block), the same quads the collision is built from. Each
/// quad's triangles are (0, 2, 1) and (0, 3, 2), front faces counter-clockwise.
pub fn course_surfaces() -> [Vec<[[f32; 3]; 4]>; 4] {
    super::ground::surfaces().map(|group| {
        group
            .into_iter()
            .map(|quad| quad.map(|v| [v.x, v.y, v.z]))
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn course_has_the_authored_quads() {
        let groups = course_surfaces();
        assert_eq!(groups.iter().map(Vec::len).collect::<Vec<_>>(), [1, 100, 66, 5]);
    }

    #[test]
    #[ignore = "requires the user's converted assets (SKATE3_ASSET_ROOT)"]
    fn presentation_snapshots_follow_the_stock_rig() {
        let root = std::env::var_os("SKATE3_ASSET_ROOT").expect("Set SKATE3_ASSET_ROOT");
        let mut session = Session::load(Path::new(&root), "normal").unwrap();
        assert!(session.presentation().is_none());
        let idle = || XboxState { buttons: 0, triggers: [0, 0], left: [0, 0], right: [0, 0] };
        for _ in 0..3 {
            assert!(session.step(idle()).error.is_none());
        }
        let (previous, current) = session.presentation().expect("snapshots after a tick");
        assert_eq!(current.bones.len(), session.bone_names().len());
        assert_eq!(session.bone_names().len(), 36);
        assert_eq!(previous.bones.len(), 36);
        assert!(current.root.translation.is_finite() && current.camera.translation.is_finite());
        assert!(current.fov > 0.1 && current.fov < 2.0, "{}", current.fov);
    }

    #[test]
    #[ignore = "requires the user's converted assets (SKATE3_ASSET_ROOT)"]
    fn respawn_returns_the_skater_to_the_spawn() {
        let root = std::env::var_os("SKATE3_ASSET_ROOT").expect("Set SKATE3_ASSET_ROOT");
        let mut session = Session::load(Path::new(&root), "normal").unwrap();
        let pad = |buttons: u16| XboxState { buttons, triggers: [0, 0], left: [0, 0], right: [0, 0] };
        // Push (A) a few times and roll away.
        for tick in 0..240 {
            assert!(session.step(pad(if tick % 40 < 10 { 0x1000 } else { 0 })).error.is_none());
        }
        let (away, _) = session.board();
        assert!(away[0].abs() + away[2].abs() > 1.0, "the skater moved: {away:?}");
        session.respawn().unwrap();
        let mut teleported = false;
        for _ in 0..120 {
            assert!(session.step(pad(0)).error.is_none());
            teleported |= session.state_id() == 702;
        }
        let (back, _) = session.board();
        assert!(teleported, "went through the Teleporting state");
        assert!(back[0].abs() < 0.5 && back[2].abs() < 0.5, "back at the spawn: {back:?}");
        let score = session.score();
        assert_eq!(score.multiplier >= 1.0, true);
        session.set_difficulty("hardcore").unwrap();
        assert!(session.step(pad(0)).error.is_none());
    }

    #[test]
    #[ignore = "requires the user's converted assets (SKATE3_ASSET_ROOT)"]
    fn a_session_marker_is_placed_and_returned_to() {
        use crate::{AUDIO_DENIED, AUDIO_RETURN, AUDIO_SET};
        const LB: u16 = 0x0100;
        const DOWN: u16 = 0x0002;
        const UP: u16 = 0x0001;
        let root = std::env::var_os("SKATE3_ASSET_ROOT").expect("Set SKATE3_ASSET_ROOT");
        let mut session = Session::load(Path::new(&root), "normal").unwrap();
        session.set_session_markers(true).unwrap();
        let pad = |buttons: u16| XboxState { buttons, triggers: [0, 0], left: [0, 0], right: [0, 0] };
        let mut audio = Vec::new();
        let mut run = |session: &mut Session, buttons: u16, ticks: usize, audio: &mut Vec<u64>| {
            for _ in 0..ticks {
                let report = session.step(pad(buttons));
                assert!(report.error.is_none());
                audio.extend(report.marker_audio);
            }
        };
        // LB alone shows the prompt (after the initial block is released).
        run(&mut session, 0, 90, &mut audio);
        run(&mut session, LB, 10, &mut audio);
        assert!(session.marker_view().visible);
        assert!(session.marker_view().can_place, "the spawn accepts a marker");
        assert!(!session.marker_view().can_return, "nothing placed yet");
        // LB + D-pad down places it.
        run(&mut session, LB | DOWN, 3, &mut audio);
        run(&mut session, 0, 10, &mut audio);
        assert_eq!(audio, vec![AUDIO_SET]);
        let (placed, _) = session.board();
        // Roll away.
        for tick in 0..300 {
            run(&mut session, if tick % 40 < 10 { 0x1000 } else { 0 }, 1, &mut audio);
        }
        let (away, _) = session.board();
        assert!((away[0] - placed[0]).hypot(away[2] - placed[2]) > 1.0, "moved: {away:?} from {placed:?}");
        // LB + D-pad up held returns to it.
        run(&mut session, LB | UP, 90, &mut audio);
        assert!(audio.contains(&AUDIO_RETURN), "the return fired: {audio:x?}");
        assert!(!audio.contains(&AUDIO_DENIED));
        run(&mut session, 0, 150, &mut audio);
        let (back, _) = session.board();
        assert!((back[0] - placed[0]).hypot(back[2] - placed[2]) < 0.5, "back at the marker: {back:?} vs {placed:?}");
        // After a menu the hold is cancelled until LB is released.
        session.suspend_session_markers();
        run(&mut session, LB | UP, 30, &mut audio);
        assert!(!session.marker_view().visible);
    }
}
