//! The original Skate 3 HUD movie runtime (PLAN.md §14, M9): the APT action-stream
//! interpreter, the movie hierarchy and timelines, the text layout and the scene
//! traversal that produces draw batches (textured triangles with a multiply/add colour
//! transform). Copied unchanged from skate-game; there is no Bevy in these modules.
//! Rendering the batches is the caller's job (`vita-render`).
pub mod analyzer;
pub mod apt_attach;
pub mod apt_display;
pub mod apt_movie;
pub mod apt_scene;
pub mod apt_text;
pub mod apt_vm;
pub mod crossbar;
pub mod hud_runtime;
pub mod menu_runtime;
pub mod meter;
pub mod popup;
pub mod replay_editor;
pub mod trick_names;

#[cfg(test)]
mod tests;
