//! R2: the retail front end on skate-game's APT runtime (PR #52 `fad5aae`): the stage, the menu movies, the menu
//! registry and the screen hosts, without Bevy. It runs beside this crate's older runtime (the HUD, the replay editor,
//! the meters and the M9 crossbar), which moves onto it later.
pub mod apt_display;
pub mod apt_imports;
pub mod apt_movie;
pub mod apt_scene;
pub mod apt_stage;
pub mod apt_text;
pub mod apt_vm;
pub mod challenge_map_host;
pub mod crossbar_host;
pub mod extras_host;
pub mod free_skate_options_host;
pub mod fe_language;
pub mod image_renderer_host;
pub mod menu_stage;
pub mod offline_profile_host;
pub mod popup_host;
pub mod profile;
pub(crate) mod prompt_glyphs;
pub mod retail_menu_movies;
pub mod retail_menus;
pub mod replay_menu_host;
pub mod settings_host;
pub mod spawn_skater_host;
pub mod tips_loader_host;
pub mod trick_guide_host;
