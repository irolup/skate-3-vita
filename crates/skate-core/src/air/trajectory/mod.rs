//! TU3 trajectory selection82D67848/82D682E8/82D68800.
//! Host world storage and batch ownership are independent of console layout.
mod launch;
pub mod grind_surface;
pub mod grind;
mod math;
mod prediction;
mod query;
mod scoring;
mod selector;
mod types;
pub use prediction::{Prediction, QueryRequest, QueryResult, Trajectory};
pub use query::{SurfaceHit, query_trajectory};
pub use selector::{Selection, TrajectorySelector, WorldWithoutGrindEdges};
pub use types::{LaunchInfo, RecordedArc, SelectorInput, SelectorSettings};
