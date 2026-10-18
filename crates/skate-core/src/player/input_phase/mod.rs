//! Skate 3 TU3 `PhysicalPlayerHiLOD::Input` (`0x82DB4048`).
//!
//! The phase is split into direct field publication and an ordered runtime.
//! Separate native subsystems remain mandatory service calls; none have a
//! fallback implementation here.

mod publication;
mod special_surface;
pub use special_surface::{BoundaryContact, publish_special_surface};
mod motion_math;
mod runtime;
mod types;
mod air_output;
mod pose_output;
mod requests;
mod grind_input;
mod grind_output;

pub use runtime::{InputContinuation, InputPhaseError, InputPhaseServices, process_input, start_input, finish_input};
pub use types::*;
pub use air_output::AirOutputFields;
pub use pose_output::{AnimationOutputFields, ScoringOutputFields, SkeletonOutputFields};
pub use requests::*;
pub use grind_input::GrindInvestigationFields;
pub use grind_output::GrindOutputFields;

#[cfg(test)]
mod tests;
