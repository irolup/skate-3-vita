//! Normal gameplay camera rendering endpoint. Simulation owns the cadence of
//! CameraRuntime::advance after its complete physical/animation publication.
//!
//! skate-game `camera.rs` (`cb79689`) keeps only its submodules and re-exports. The
//! Bevy rendering endpoint (`CameraPlugin`, `spawn`, `present`, `GameplayCamera`,
//! `VehicleCameraBlend`, `set_world_environment` and its test) is not copied.
//! `present` also called `CameraRuntime::set_aspect_ratio` from the window size;
//! the game loop does that now.
mod settings;
mod collision;
mod world_query;
mod shot_data;
mod stock_names;
mod shake_data;
mod trajectory;
mod subject;
mod graph_subject;
mod graph_conditions;
mod graph;
mod runtime;
mod water;
mod publication;
pub(crate) use publication::{
    snapshot as publish_camera_subject, CameraPublicationInputs, CameraStateOutput,
    CameraAnimationOutput, CameraAirOutput, CameraOffboardOutput, CameraGrindOutput,
    CameraEventsOutput, CameraPreferences,
};
pub(crate) use graph_subject::CameraGraphEnvironment;
pub(crate) use runtime::CameraRuntime;
