//! The original Skate 3 UI movies (PLAN.md §14, M9; R2): the APT runtime (`fe`, skate-game's `apt_*` with the
//! Vita's optimizations) and what runs on it: the retail front end (`fe::menu_stage`), the trick display
//! (`hud_runtime`), and the linked HUD screens (`menu_runtime`: the manual meter and the trick analyzer). There is no
//! Bevy in these modules; rendering the draw batches is the caller's job (`vita-render`).
pub mod analyzer;
pub mod fe;
pub mod hud_runtime;
pub mod menu_runtime;
pub mod meter;

#[cfg(test)]
mod tests;
