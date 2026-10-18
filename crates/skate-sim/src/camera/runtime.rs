//! Complete normal camera owner: subject publications, stock graph, CameraMan,
//! swept world queries and retained query history. The simulation calls advance
//! once per completed physical/animation publication; rendering only presents it.
use std::path::Path;
use skate_core::{camera::{CameraFrame, CameraMan, CompassSettings, ManualCam, ManualCamSettings,
    ManagerSettings, MovingObstacleProvider, ShakeSamples, SimulationRateRequest, SlowMotionSettings},
    physics::board_world::BoardWorld, point_graph::PointGraph};
use skate_data::collections::Collections;
use super::{collision::CameraCollision, graph::CameraGraph, graph_subject::CameraGraphEnvironment,
    settings, shake_data, shot_data::StockShots, subject::{CameraSubjectSnapshot, SubjectPublisher},
    trajectory::{CameraTrajectory, TrajectoryResult}};


pub(crate) struct CameraRuntime {
    manager: CameraMan,
    subject: SubjectPublisher,
    graph: CameraGraph,
    shots: StockShots,
    settings: ManagerSettings,
    compass_settings: CompassSettings,
    shakes: [ShakeSamples; 2],
    trajectories: [TrajectoryResult; 3],
    pub frame: Option<CameraFrame>,
    /// Immutable physical subject publication consumed by the camera graph.
    /// Rendering may inspect this snapshot for diagnostics without rebuilding
    /// subject fields from mutable skater state.
    pub latest_subject: Option<CameraSubjectSnapshot>,
    /// Native message order, including End followed by Begin in one graph tick.
    /// The simulation schedule drains these after advance.
    pub simulation_rate_requests: Vec<SimulationRateRequest>,
    pub manual_cam: ManualCam,
    pub manual_cam_settings: ManualCamSettings,
    /// Game Settings > Control > offboard X / Y axis (settings +193 / +192): retail's camera config +24 / +25,
    /// which the offboard camera (82DF69C0) applies by negating the look stick's x / y. Synced by
    /// `sync_look_invert`.
    pub invert_look: [bool; 2],
    /// Keeps the camera above water while the skater is in it.
    water: super::water::WaterView,
    /// Stock camera graph `IsCameraTypeActive` value: 0 Low, 1 High (camera/angle.rs).
    camera_type: u32,
    /// `CameraAngleSettings` generation whose shot tunings are applied (0 = none yet).
    tuning_generation: u64,
    /// Reload the current shot tree once, after the shot tunings changed.
    reselect_shot: bool,
}

#[cfg(test)]
#[path = "tests/runtime.rs"]
mod tests;
impl CameraRuntime {
    pub fn load(root: &Path) -> Result<Self, String> {
        let data = Collections::load(root)?;
        let values = data.words::<32>("slowmotion_controller", "default", "timescale")?
            .map(f32::from_bits);
        let slow_motion = SlowMotionSettings {
            timescale: PointGraph {
                x: core::array::from_fn(|i| values[i]),
                y: core::array::from_fn(|i| values[i + 16]),
            },
            fps_at_scale_one: data.float("slowmotion_controller", "default", "fps_at_scale_one")?,
        };
        let graph = CameraGraph::load(&root.join(
            "private/stock/data/script/camera/Default_cameragraph.stategraph"), slow_motion)?;
        let samples = |name: &str| -> Result<ShakeSamples, String> {
            let path = root.join("private/stock/data/camera").join(name);
            shake_data::parse(&skate_data::vfs::read_to_string(&path)
                .map_err(|e| format!("{}: {e}", path.display()))?)
        };
        Ok(Self { manager: CameraMan::new(), subject: SubjectPublisher::new(), graph,
            shots: StockShots::from_collections(&data)?, settings: settings::manager_settings(&data)?,
            compass_settings: settings::compass_settings(&data)?, shakes: [samples("1.shk")?, samples("2.shk")?],
            trajectories: core::array::from_fn(|_| TrajectoryResult::new()), frame: None,
            latest_subject: None,
            simulation_rate_requests: Vec::new(),
            manual_cam: ManualCam::default(),
            manual_cam_settings: settings::manual_cam_settings(&data)?,
            invert_look: [false; 2],
            water: Default::default(),
            // skate_vita: the reference's default Camera Angle, High (graph type 1, camera/angle.rs).
            camera_type: 1,
            tuning_generation: 0, reselect_shot: false })
    }

    /// Ignores a degenerate ratio rather than storing it.
    ///
    /// A minimized window reports 0x0, so the caller's `width / height` is
    /// `0.0 / 0.0` — NaN. Storing that makes the manager derive a NaN field of
    /// view, which fails the non-finite frame check on every subsequent frame
    /// even after the window is restored. Keeping the last good ratio is
    /// correct: nothing is visible while minimized.
    /// Water-shot vignette amount for the tone pass (0 when not in water).
    pub fn water_vignette(&self) -> f32 {
        self.water.vignette()
    }

    pub fn set_aspect_ratio(&mut self, value: f32) {
        if value.is_finite() && value > 0.0 {
            self.manager.state.aspect_ratio = value;
        }
    }
    pub fn selected_shot(&self) -> &str { &self.manager.shots.current().name }

    pub fn camera_type(&self) -> u32 { self.camera_type }
    /// The graph re-evaluates `IsCameraTypeActive` on its next update and switches branch
    /// through its own shot transitions.
    pub fn set_camera_type(&mut self, value: u32) { self.camera_type = value; }

    pub fn has_shot(&self, name: &str) -> bool { self.shots.contains(name) }
    pub fn tuning_generation(&self) -> u64 { self.tuning_generation }

    /// Replaces the mod shot tunings. The current shot tree is re-selected on the next
    /// advance, so a tuned shot that is in use changes at once (through the normal shot
    /// transition) instead of waiting for the graph's next choice.
    pub fn set_shot_tunings<'a>(&mut self, generation: u64,
        tunings: impl Iterator<Item = (&'a str, &'a skate_trainer_stub::presentation::CameraShotTuning)>) {
        self.shots.set_tunings(tunings);
        self.tuning_generation = generation;
        self.reselect_shot = true;
    }

    /// `GetMatrix` for presentation capture when manual cam may be active.
    pub fn presentation_frame(&self) -> Option<CameraFrame> {
        self.frame.map(|frame| self.manual_cam.get_matrix(&frame))
    }

    pub fn advance(&mut self, dt: f32, snapshot: CameraSubjectSnapshot,
        world: &BoardWorld, query_gravity: [f32; 4], environment: &CameraGraphEnvironment,
        moving: &mut impl MovingObstacleProvider,
        // Water surface while the skater is in a water bail (camera/water.rs).
        water: Option<f32>) -> Result<CameraFrame, String> {
        if let Some(previous) = self.latest_subject.as_ref()
            && snapshot.tick <= previous.tick
        {
            return Err(format!(
                "Camera received non-monotonic subject tick: previous={}, current={}",
                previous.tick, snapshot.tick,
            ));
        }
        self.latest_subject = Some(snapshot);
        let mut subject = self.subject.publish(snapshot, &self.manager, self.compass_settings)?;
        self.manager.prepare(&subject, self.settings);
        let requests = self.graph.update(dt, &mut self.manager, &subject,
            snapshot.graph, environment, &self.shots)?;
        self.simulation_rate_requests.extend(requests);
        if std::mem::take(&mut self.reselect_shot) {
            let current = self.manager.shots.current().name.clone();
            if !current.is_empty() {
                self.manager.set_shot(&current, true, &subject, &self.shots)?;
            }
        }
        let [a, b, c] = &mut self.trajectories;
        let mut trajectories = [a, b, c].map(|result| CameraTrajectory {
            world, gravity: query_gravity, result,
        });
        let mut collision = CameraCollision::new(world);
        let frame = self.manager.update(dt, &mut subject, self.settings,
            [&self.shakes[0], &self.shakes[1]], &mut trajectories, moving, &mut collision)?;
        if let Some(error) = collision.error { return Err(error); }
        if let Some(error) = self.trajectories.iter().find_map(|v| v.error.as_ref()) {
            return Err(error.clone());
        }
        if !frame.position.iter().all(|v| v.is_finite())
            || !frame.basis.columns.iter().flatten().all(|v| v.is_finite())
            || !frame.field_of_view_degrees.is_finite() {
            return Err(format!(
                "Normal gameplay camera produced a non-finite frame: frame={frame:?}; lens_length={:?}; aspect_ratio={:?}; subject_transform={:?}; skeleton_root={:?}; ground_normal={:?}; launch_position={:?}; landing_position={:?}",
                self.manager.shots.interpolated.lens_length,
                self.manager.state.aspect_ratio,
                subject.rig.transform,
                subject.rig.skeleton_root,
                subject.ground_normal,
                subject.launch_position,
                subject.landing_position,
            ));
        }
        // The recovered CameraMan82DFEE80 result, then the water shot (camera/water.rs).
        let root = subject.rig.skeleton_root[3];
        let frame = self.water.adjust(frame, world, [root[0], root[1], root[2]], water, dt);
        self.frame = Some(frame);
        Ok(frame)
    }
}
