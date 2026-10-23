//! Engine-independent recovered calculations. No rendering, file I/O, or ECS.
//! The game schedules these systems; full gameplay parity remains under review.
//! Arithmetic uses independent Rust implementations. Original-hardware numerical
//! agreement is unverified. See docs/correction for current source coverage,
//! confirmed integration gaps and the evidence still required.
#![forbid(unsafe_code)]
pub mod trick_guide;
pub mod challenge_map;
pub mod camera_track;
pub mod air;
pub mod animation;
pub mod camera;
pub mod colour_matrix;
pub mod graph;
pub mod input;
pub mod madd;
pub mod profile;
pub mod math;
pub mod physics;
pub mod player;
pub mod point_graph;
pub mod replay_buffer;
pub mod riding;
pub mod scoring;
pub mod trigonometry;
pub mod living_world;
pub mod triggers;
pub mod crossbar;
pub mod fe_blur;
pub mod fe_fade;
pub mod fe_input;
pub mod fe_popup;
pub mod fe_stack;
pub mod menu_values;
pub mod menus;
pub mod video_calibration;
pub mod cheats;
pub mod extras;
pub mod offline_profile;
pub mod text_entry;
pub mod text_viewer;
