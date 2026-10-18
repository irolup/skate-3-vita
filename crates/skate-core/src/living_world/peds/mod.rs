//! Pedestrian body (doc 26, peds milestone M2): which entity / model a census spawn becomes, the
//! ped animation player and the foot events the audio reads; navigation (milestone M3): the
//! NavPower navmesh (`nav`), retail's wander goal and avoidance (`wander`), the mod crosswalk
//! rule's walk lights (`crosswalk`). Pure and engine-independent (no
//! ECS, no I/O); `skate-data::ped_anim` fills the data types from the user's export, the game
//! (`skate-game::living_world::peds`) hosts it.
//!
//! Retail reference (TU3, addresses are evidence only):
//! - entity inside the census category: `sub_826B8B88` after the category roll; the draw is
//!   `sub_826BB058` (world RNG u32 x 2^-32, `0x822F88F4`), the index
//!   `trunc(draw x 100) % entities.len()` (`0x820ED57C` = 100) [code];
//! - model tints: `sub_827B4170`: one rand `r`, `secondary_colours[r % na]` (`tints_a`) and
//!   `chassis_colours[r % nb]` (`tints_b`) [code]; the ped shaders recolour the atlas's mask
//!   texels with them (`colorize`) [code, shader];
//! - clips: `PedestrianSkeletonPres.abin`, additive over its `PEDESTRIAN_RIG_TPOSE` pose record
//!   (the clips hold 6 of the rig's 10 parts: bones 0..=26) [data];
//! - foot plants: the clips' `LEFTTOEDOWN` / `RIGHTTOEDOWN` attributes (phase windows), the
//!   body falls `BODYFALLTYPE` (value windows) [data]; retail's audio reads them as `S+74` /
//!   `S+73` and `S+76` (`world-ped-audio.md`).
//!
//! Multiplayer: a ped's look is a pure function of its spawn record (category + seed) and the
//! catalog; its animation steps once per world tick from that seed, so the state follows from
//! the spawn record and the tick.

pub mod anim;
pub mod brain;
pub mod chase;
pub mod conversation;
pub mod perception;
pub mod takedown;
pub mod choice;
pub mod colorize;
pub mod crosswalk;
pub mod fade;
pub mod hand_prop;
pub mod flee;
pub mod honk;
pub mod mood;
pub mod nav;
pub mod plugin_motion;
pub mod plugins;
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
