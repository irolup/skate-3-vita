//! Runtime stock data, validation and bank storage. Stateless codec math is in core.
#![forbid(unsafe_code)]
mod manifest;
pub use manifest::{AssetError, GameAssets};
pub mod trick_guide;
pub mod challenge_map;
pub mod ice_camera;
pub mod scene_clip;
pub mod abin;
pub mod animation_banks;
pub mod animation_frames;
pub mod animation_metadata;
pub mod collections;
pub mod scoring;
// R2: the retail front-end menu tables (data side).
pub mod menu_tables;
mod scoring_fields;
pub mod attrib_hash;
pub mod input_config;
pub mod input_recording;
pub mod gesture_patterns;
pub mod physics_skeleton;
mod sha256;
pub mod state_graph;
pub mod skate_map;
pub mod retail_collision;
// skate_vita additions: loader file access and the baked binary JSON forms.
pub mod binary;
pub mod io_gate;
pub mod vfs;
// skate_vita: the movable props' package for the simulation (no texture pixels).
pub mod prop_package;
// Upstream PR #52 (the living world): AI skater lines, the living-world tables, the ped
// animation bank and navigation, the road network.
pub mod aipath;
pub mod living_world;
pub mod ped_anim;
pub mod ped_nav;
pub mod roads;
pub mod trigger_volumes;
