//! Off-board game adapters over the shared physical and scene owners.
//!
//! These modules are the source-backed implementation units for the off-board
//! owner. The coordinator calls them through the selected physical state.
#![allow(dead_code)]
pub(crate) mod air_feet;

/// The world the on-foot (biped) queries see: the map, and the DMO prop world as query pool bit 2 (`82764AB0`,
/// world+0x5580, "DMOPool"; research b218 / b221: retail's walker probes use pool mask 7, so a heavy prop is ordinary
/// step geometry). Light and held props are left out by their meshes' collision type flags (the walker's mask 0x6000,
/// `PropDynamics::for_each_collision_type`). Derefs to the map, so map-only code reads it unchanged.
#[derive(Clone, Copy)]
pub(crate) struct BipedWorld<'a> {
    pub map: &'a skate_core::physics::board_world::BoardWorld,
    pub props: Option<&'a skate_core::physics::board_world::BoardWorld>,
}

impl<'a> From<&'a skate_core::physics::board_world::BoardWorld> for BipedWorld<'a> {
    fn from(map: &'a skate_core::physics::board_world::BoardWorld) -> Self {
        Self { map, props: None }
    }
}

impl std::ops::Deref for BipedWorld<'_> {
    type Target = skate_core::physics::board_world::BoardWorld;
    fn deref(&self) -> &Self::Target {
        self.map
    }
}
pub(crate) mod air_selector;
pub(crate) mod board_manager;
pub(crate) mod board_possession;
pub(crate) mod contact_toolkit;
pub(crate) mod grab_scene;
pub(crate) mod ground_geometry;
pub(crate) mod ground_query;
pub(crate) mod ground_sync;
pub(crate) mod mod_solid_ground;
pub(crate) mod landing_deck;
pub(crate) mod pose_adjust;
pub(crate) mod post_physics;
pub(crate) mod settings;
pub(crate) mod skeleton_air;
pub(crate) mod skeleton_ground;

pub(crate) mod contact_queries;
