//! Pedestrian body and navigation core: entity / model choice, animation, foot events, navmesh, wander. Pure.
//! Retail reference (826B8B88, 826BB058, 827B4170): docs/hails-additions/26f-living-world-peds.md#ped-core

pub mod anim;
pub mod brain;
pub mod chase;
pub mod chase_alert;
pub mod conversation;
pub mod perception;
pub mod avoid;
pub mod takedown;
pub mod choice;
pub mod colorize;
pub mod crosswalk;
pub mod fade;
pub mod hand_prop;
pub mod flee;
pub mod object_flee;
pub mod honk;
pub mod mood;
pub mod nav;
pub mod plugin_motion;
pub mod plugins;
pub mod scatter;
pub mod obstacles;
pub mod skater_contact;
pub mod vehicle_contact;
pub mod wander;

pub use anim::{Locomotion, PedAnimPlayer, PedAnimSet, PedClip, PedEvaluator, PedFrame, PedRig};
pub use choice::{PedCatalog, PedEntity, PedLook, PedModel, PedOverrides};
pub use fade::{draw_alpha, PedFadeConfig};
pub use nav::{NavMesh, NavMeshInput, NavPoint, NavPolyInput, NavRules};
pub use obstacles::{Footprint, NavObstacles, ObstacleInput, ObstacleParams};
pub use vehicle_contact::{CarBox, PedCylinder, VehicleContact, VehicleContactParams, VehicleContactReaction};
pub use wander::{CrosswalkRule, Fan, NavOutput, NavWait, Neighbour, PedNav, PedRoute, WalkSignals, WanderParams};

/// Map one rig's bone names onto another's by name (case-insensitive). Returns, per `target`
/// bone, the index in `source` (or `None`), plus the source bones no target uses.
pub fn match_bones(target: &[String], source: &[String]) -> (Vec<Option<usize>>, Vec<String>) {
    let map: Vec<Option<usize>> = target.iter().map(|t| source.iter().position(|s| s.eq_ignore_ascii_case(t))).collect();
    let unused = source.iter().enumerate().filter(|(i, _)| !map.contains(&Some(*i))).map(|(_, s)| s.clone()).collect();
    (map, unused)
}

#[cfg(test)]
mod nav_tests;
#[cfg(test)]
mod obstacle_tests;
#[cfg(test)]
mod tests;
