# Provenance

These are the reference repositories this project copies from or follows. Pinned commits are the ones this project is built against; `scripts/setup-references.sh` checks them out.

| Reference | URL | Commit |
|---|---|---|
| skate-3-rust-engine | https://github.com/SK8-ENGINE/skate-3-rust-engine | `8df32c51`, a commit of pull request #52 (copied at `cb79689`, resynced 2026-10-08 to `b3c9679`, 2026-10-13 to `2e16697`, 2026-10-17 to `618e824`, 2026-10-19 to `fad5aae`, 2026-10-21 to `b110bab`, 2026-10-23 to `74c124a`, 2026-10-24 to `36018e19`, then `0947dea5`, and 2026-10-26 to `8df32c51`: see **Resync to b3c9679**, **Resync to 2e16697**, **L1 resync to PR #52 head `618e824`**, **L1 resync to PR #52 head `fad5aae`**, **Resync to PR #52 head `b110bab`**, **Resync to PR #52 head `74c124a`** and **Resync to PR #52 head `36018e19`** (steps 6 and 7: `0947dea5`) and **Resync to PR #52 head `8df32c51`**) |
| cargo-vita | https://github.com/vita-rust/cargo-vita | `c6bbf22` |
| vitasdk-sys | https://github.com/vita-rust/vitasdk-sys | `cee132f` |
| examples | https://github.com/vita-rust/examples | `5ef9e08` |
| book | https://github.com/vita-rust/book | `fec0a66` |
| vita-newlib-shims | https://github.com/vita-rust/vita-newlib-shims | `013e28c` |
| docker | https://github.com/vita-rust/docker | `c585981` |
| vitaGL | https://github.com/Rinnegatamante/vitaGL | `fdf4184` for the sources in `../../vitaGL`. The library linked is the newer build installed in `$VITASDK` (`libvitaGL.a` of 2025-08-28: no `vglEnd`, adds `vglShaderGxpBinary`); FFI is declared against `$VITASDK/arm-vita-eabi/include/vitaGL.h`. |

## Copied files

Add a row for every file or folder copied into this repo: the source path, the destination, the reference commit, and any edits made (PLAN.md §5.2 and §5.3 list the allowed ones).

| Source | Destination | Commit | Edits |
|---|---|---|---|
| `skate-3-rust-engine/crates/skate-core/` (whole crate: `src/`, `tests/`, `Cargo.toml`) | `crates/skate-core/` | `cb79689` | Only the PLAN.md §5.3 edit: every `.mul_add(` → `.madd(` (1,416 calls in 279 files), plus a `use crate::madd::Madd as _;` (`use skate_core::madd::Madd as _;` in `tests/`) after each file's leading `//!`/`#![…]` block (left out in 3 files that get it through `use super::*`). New `src/madd.rs` and `pub mod madd;` in `lib.rs`. `Cargo.toml`: a `no-fma` feature and a `[package.metadata.vita]` block. M3: see **M3 performance edits**. |
| `skate-3-rust-engine/crates/skate-data/` (`src/`, `tests/`, `Cargo.toml`, 5 of 9 `examples/`) | `crates/skate-data/` | `cb79689` | Byte-identical at M1/M2; M3 performance edits listed under **M3 performance edits**; M6 file-access edits under **M6 data edits**. Not copied: `examples/{apt_data,apt_movie_data,hud_data,scoring_flow_data}.rs`, which `#[path]`-include APT/HUD sources from skate-game (the HUD is cut). `tests/fixtures/retail-collision.rwcmset` (843 B, a synthetic upstream fixture) is present locally but **not committed**: `*.rwcmset` is a blocked pattern, so `tests/retail_collision.rs` only builds where the reference is next to the repo; copy the fixture from there. |
| `skate-3-rust-engine/crates/skate-mods/src/schema.rs` lines 136–190 (`TrainerTuning`, its `Default` and `valid()`) | `crates/skate-trainer-stub/src/lib.rs` | `cb79689` | Removed the serde derives and `#[serde(...)]` attribute; nothing else changed. Added one test. |
| `skate-3-rust-engine/crates/skate-game/src/`: `physics/` (incl. `climbing/`), `graph_host/`, `camera/`, `skater_animation/`, `animation_pose/`, `grind_world/`, `physics.rs`, `graph_runtime.rs`, `skater_animation.rs`, `animation_pose.rs`, `grind_world.rs`, `difficulty.rs`, `camera.rs`, `presentation.rs`, `scoring_runtime.rs`, `input.rs` + 6 `input/` files, and the headless `tests/` | `crates/skate-sim/src/` (same relative paths) | `cb79689` | 391 of 427 files byte-identical. See **skate-sim edits** below. |
| `skate-3-rust-engine/crates/skate-game/src/`: `apt_display.rs`, `apt_movie.rs`, `apt_scene.rs`, `apt_text.rs`, `apt_vm.rs`, `hud_runtime.rs` (the original HUD movie runtime; M9) | `crates/skate-ui/src/` (same names) | `cb79689` | **Byte-identical** (`cmp`); none of them uses Bevy. `scoring_hud.rs` (the Bevy renderer and lifecycle) is not copied: `vita-render` draws the batches and `skate-vita` drives the movie. `crates/skate-ui/src/tests.rs` is skate-data's `examples/hud_data.rs` (the reference's data-only HUD audit: 1,800 frames, `cb79689`) turned into an `#[ignore]` test, with the data path read from `SKATE3_ASSET_ROOT` |
| `skate-3-rust-engine/crates/skate-game/src/session_marker/`: `state.rs`, `noise.rs` (byte-identical, with their tests), `validation.rs` (only the Bevy import and `#[derive(Resource)]` removed), `mod.rs` (the update logic; M9) | `crates/skate-sim/src/session_marker/` | `cb79689` | `mod.rs` is the reference's update without the Bevy plugin/system/resource wrappers; no replay or vehicle checks (neither exists here); no map generation (a session is one map); the UI clock is the tick period instead of `Time<Real>` (identical at 60 Hz, pauses with the simulation); the three native audio ids are returned, not written as messages; `Noise` is exposed through `DissolveNoise`; `Session::step` runs it at the reference's place (after the input, before the controls) only when `set_session_markers(true)` (the oracle corpus never holds LB and its dumps stay 20/20 either way). `hud.rs` and `effect.rs` (Bevy rendering) are not copied: the game draws the overlay through `vita-render` |
| `skate-3-rust-engine/tools/prepare_hud.py` (`cb79689`; `prepare()` and its use of the vendored `tools/vendor/skate3_ui` extractor, MIT) | `tools/prepare_ui_screen.py` | `cb79689` | The logic of `prepare()` with the bundle and the output name as arguments instead of constants (and unresolved shapes recorded instead of raising); it imports `prepare_hud.font_mapping` and the vendored extractor from the reference checkout without copying them, read-only (`sys.dont_write_bytecode`). Output is the user's private data (`../game_data/ui-screens/`), never committed |
| `bevy_transform` 0.18.1 `src/components/transform.rs` (MIT OR Apache-2.0): `Transform` fields, `IDENTITY`, `Default`, `from_matrix`, `from_translation`, `with_rotation`, `to_matrix` | `crates/skate-sim/src/transform.rs` | crates.io 0.18.1 | Removed ECS/reflect derives and doc links. |

Dependencies (not copied): `texpresso` 2.0.2 (MIT, DXT encoder in vita-bake); `font8x8` 0.3.1 (MIT; Daniel Hepper's public-domain 8×8 font) draws the input lab's text in `crates/skate-vita/src/gfx.rs`.

Ported (re-implemented from reference logic, M5):

| Reference | Here | Notes |
|---|---|---|
| `skate-game/src/animation.rs` `present` + `render_basis` (`cb79689`) | `crates/skate-vita/src/skin.rs` | The same bone-local blend (`(bones[parent]·B)⁻¹ · bones[bone]·B`, lerp/slerp/lerp via the copied `presentation::blend`) and composition, producing GPU skin matrices `root · global · inverse bind` instead of Bevy joint transforms |
| `skate-game/src/world.rs` `spawn_test_world` colours and light, `presentation.rs` camera conventions | `crates/vita-render/src/{course,math}.rs` | Course group colours, the (4, 7, 4) sun, the clear colour; the camera looks along local −Z with the stock vertical FOV |
| `skate-game/src/retail_world.wgsl` + `retail_tone.wgsl` (lightmapped retail look: `lm² · albedo² · kd · exposure`, then the retail tone curve), `skate_world.rs` `render_groups` (triangles by their first vertex's 1-based material) and the horizon clear colour (environment[3..6]) | `shaders/park*.cg`, `tools/vita-bake/src/map.rs` | One fixed exposure instead of auto-exposure; no normal maps, detail, decals, specular or environment maps; water families dropped (PLAN §7) |
| `skate-game` skin binding (`animation.rs` `bind`: joints matched to the stock rig by name, non-joint ancestors must be identity) | `tools/vita-bake/src/main.rs`, `crates/skate-vita/src/skin.rs` | Checked at bake time (identity ancestors, joint parents) and at load (names) |

Followed but not copied: `crates/skate-vita/build.rs` takes its link-library list from the vitaGL samples' Makefile (`vitaGL/samples/*/Makefile`, `fdf4184`), as PLAN.md §6.2 says. `crates/skate-vita/src/vgl.rs` declares a few prototypes from the installed `vitaGL.h`.

## skate-sim edits

All edits are the mechanical ones PLAN.md §5.2 allows. None changes a recovered routine, constant, iteration order or the `frame.rs` stage order. Desktop check: all 245 skate-sim tests give the same result as in the reference skate-game, both by default and with `SKATE3_ASSET_ROOT --include-ignored` (docs/milestones/M2.md).

| Edit | Files |
|---|---|
| `bevy::log::{info,warn,debug,error,info_span}` → `tracing::…`. `bevy::log` *is* a re-export of the tracing macros, and the call sites use tracing's structured-field syntax, which the `log` crate cannot parse. Release builds compile out info/debug (`release_max_level_warn`). | 16 files under `physics/`, `camera/`, `graph_host/action.rs`, `skate_world.rs` |
| `#[derive(Resource)]` and `use bevy::prelude::Resource` removed | `graph_runtime.rs`, `input/controllers.rs`, `physics/skater.rs`, `physics/controls.rs`, `camera/runtime.rs`, `physics.rs`, `input.rs` |
| `bevy::prelude::*` / `bevy::math::*` → the same glam 0.30.10 types (`bevy_math` 0.18.1 re-exports them) and the copied `Transform`; the prelude's `default()` → `Default::default()` | `physics.rs`, `physics/climbing/{mod,clip,contacts,ledge}.rs`, `animation_pose/authored_clips.rs`, `presentation.rs`, `skate_world.rs` |
| Bevy systems → plain functions: `physics::advance` (`Res`/`ResMut` → references; returns the error instead of sending `AppExit`; the caller applies `period()` as the next fixed timestep; vehicle check removed) and `controls::sample` (same body) | `physics.rs`, `physics/controls.rs` |
| New `physics::fixed_tick` = `controls::sample` then `advance`, the chained `SimulationSet::{Controls, Physics}` order | `physics.rs` |
| Bevy plugins and render-side systems removed: `PhysicsPlugin`, `physics::present`, `InputPlugin` + `poll_controllers`/`publish_actions`, `GrindGeometryPlugin`, `CameraPlugin` + `spawn`/`present`/`set_world_environment` (and its Bevy test), `PresentationPlugin` | `physics.rs`, `input.rs`, `grind_world.rs`, `camera.rs`, `presentation.rs` |
| `presentation.rs`: `capture` system → function with the fixed timestep passed in; replay recording/playback removed (replay deferred, §9) | `presentation.rs` |
| `skate_mods::TrainerTuning` → `skate_trainer_stub::TrainerTuning` | `physics.rs`, `physics/frame.rs`, `physics/ground_runtime/settings.rs` |
| Multiplayer: `physics/network.rs` keeps only `Proxies` (always empty), `bounds` and `xyz`, so `solve.rs`/`skeleton_feedback.rs` stay unchanged; `skate_net` snapshot code, `Schema`, `append`, `append_vehicle` not copied | `physics/network.rs` |
| Crash-report breadcrumb `crate::crash_context::requested(…)` removed (it only wrote a ring buffer that nothing in the simulation reads) | `physics/player_state/transition.rs` |
| APT HUD glue `ScoringRuntime::hud_input` and its `apt_vm`/`hud_runtime` imports removed (the APT HUD is cut) | `scoring_runtime.rs` |
| PLAN.md §5.3: every `.mul_add(` → `.madd(` (144 calls in 36 files) with `use skate_core::madd::Madd as _;`; `no-fma` feature forwarded to skate-core | 36 files |
| Partial copies of Bevy-coupled files: `skate_world.rs` = lines 27–356 (collision only); `input/platform.rs` = the `DevicePacket`/`DeviceError` types only; `animation.rs` = `native_matrix` only; `customiser.rs` = `scalar` + `apply_preferences` only | those files |
| New: `lib.rs` (crate root mirroring skate-game's module tree), `transform.rs` | |
| M9 HUD feed (no simulation change): `ScoringRuntime::hud_feed()` and the plain-data `HudFeed`, skate-game's removed `hud_input` field for field (same casts and divisions) plus the three pulse flags `new_trick`, `modified_trick`, `close_tricks` that `scoring_hud.rs` passed to `Runtime::update`; `Session::hud_feed()`. skate-sim does not depend on the HUD runtime: the caller builds `hud_runtime::Input` (`trick_metrics`: the trick name and four `false`, as `hud_input` did) | `scoring_runtime.rs`, `physics/live.rs` |
| M7 game API (no simulation change): `Session::{score, state_id}` (read-only HUD and sound-cue state), `Session::set_difficulty` (the stock `GamePhysics::set_difficulty`, as skate-game's menu), `Session::respawn` (the stock `SkaterRuntime::travel_to` to the world's spawn, as skate-game's pause-menu teleport); `ScoringRuntime::hud_sequence()` (read-only: sequence score, clean, sketchy, what the removed `hud_input` published) | `physics/live.rs`, `scoring_runtime.rs` |
| M6: `Session::load_map` (the same load on a retail map: `validate_runtime`, then `load_with_difficulty(Some(map))`, as skate-game's map startup) | `physics/live.rs` |
| M5 render API (read-only): `Session` calls the copied `presentation::capture` after each tick (as skate-game's capture system), exposes the snapshot pair, the stock bone names and `course_surfaces()` (the `ground::surfaces()` quads), and re-exports `Snapshot`, `blend` and `Transform`; run-live still 20/20 | `physics/live.rs` |
| M4 game-loop API (new files, no simulation change): `physics/live.rs` (`live::Session`: load + one tick in the oracle harness's order; `sim-oracle run-live` gives bit-identical dumps, 20/20), `input/live_input.rs` (the harness's slot-0 publication), `ScoringRuntime::trick_label()` (read-only; what the removed `hud_input` published) | `physics.rs`, `physics/live.rs`, `input.rs`, `input/live_input.rs`, `lib.rs`, `scoring_runtime.rs` |
| skate_vita hooks (feature `oracle`, off by default): `#[path]` modules for the shared oracle harness in `physics.rs` and `input.rs`, and its re-export in `lib.rs` | `physics.rs`, `input.rs`, `lib.rs` |

## skate-ui edits (M9)

`skate-ui` is the reference's HUD runtime, byte-identical until the front-end screens needed more. These edits extend it for the linked screens (`tools/link_ui_screen.py`, `docs/milestones/M9-progress.md` §3); none changes what the trick display does (`cargo test -p skate-ui -- --include-ignored` runs the reference's 1,800-frame audit on the owned data: unchanged).

| Edit | File |
|---|---|
| Opcodes `0xa5` (push member `operand` of the popped object), `0xa6` (pop a value into the variable `operand`, as `0x1d`) and `0x26` (Trace, ignored), used by the front-end init and construct blocks | `apt_vm.rs` |
| `Control::clip_actions` / `ClipAction` (decoded placement clip actions), `Placement::clip_actions`, `CREATION_EVENTS` (Load 0x1, Initialize 0x200, Construct 0x40000); the "clip actions are not implemented" error now only fires for undecoded ones | `apt_display.rs` |
| `Movie::class_for` (`Object.registerClass`: character → class), the `CONSTRUCTOR` pending entry queued by `create` for a registered clip (it takes the class's prototype), the creation clip events queued before the new clip's own frame actions (Flash's order: construct event, constructor, frame scripts), `create` and `remove` made `pub(crate)`, clips attached at `ATTACH_DEPTH` and above kept across a parent's seek | `apt_movie.rs` |
| `Shape::texture` optional (untextured solid fills draw with the texture key `WHITE`), `Vertex::uv` defaults to 0 | `apt_scene.rs` |
| Inventory fixes: constructors `TextFormat` and `Object` build plain objects; opcodes `0x42` InitArray, `0x43` InitObject, `0x4a` ToNumber, `0x4b` ToString, `0x44` TypeOf, `0x30` RandomNumber (always 0); `Pop` (`0x17`) on an empty stack is a no-op; `gotoAndPlay`/`gotoAndStop` with an unknown label do nothing; a font with no native layout is skipped and text in it draws nothing; a shadow font without the heavy foreground in the same movie draws the shadow alone; underflow errors name the block and opcode | `apt_vm.rs`, `apt_movie.rs`, `apt_text.rs`, `apt_scene.rs` |
| `_xscale` / `_yscale` set by a script scale (or, negative, mirror) a clip's placement axes (the trick analyzer mirrors its gesture in switch); unset, the placement's own scale stays | `apt_scene.rs` |
| Resync b3c9679: `trick_names.rs` is skate-game `scoring_hud.rs`'s `localize_trick` and `humanize_trick_id` (unchanged, pure functions), and `hud_runtime` passes `GetCurrentTrickName` and the first `GetCurrentTrickMetrics` value localized as a literal (`#…`), as upstream's `hud_runtime` does: the scorer now composes names from several localization ids | `trick_names.rs`, `hud_runtime.rs` |
| New files: `apt_attach.rs` (`attachMovie`, `removeMovieClip`), `menu_runtime.rs` (the runtime for linked screens: natives, `registerClass`, constructors, drains) | `skate-ui/src/` |
| P1 (replay editor): setting an array element past its end grows `length` (Flash's behaviour; the menu picker fills its items by index) | `apt_vm.rs` |
| P1: `Math.ceil`; `setTextFormat` keeps the format's colour, font and size on the field (`textColor`, `textFont`, `textSize`), and the scene and the width measure use them (`TextAssets::names`, `field_font`) | `menu_runtime.rs`, `apt_text.rs`, `apt_movie.rs`, `apt_scene.rs` |
| P1: `textHeight` on text fields; word-wrapped fields break lines at their width and at `^`; a NaN `_x`/`_y` (a layout from a clip's missing `_width`) draws at the origin instead of failing; helpers `clip_size`, `size_buttons` (measured `_width` of the button menus' icons), `text_fields`, `world_position`, `dim` | `apt_movie.rs`, `apt_scene.rs` |
| P1 new files: `replay_editor.rs` (the `FEReplayMenu` host of `replay_editor2` and the `replay_help` host), `popup.rs` (the `Popup` host of `sk8popup`) | `skate-ui/src/` |

## M3 performance edits

PLAN.md §5.4 allows performance changes that the oracle verifies. After each of these, `scripts/verify-parity.sh` gave 20/20 bit-identical dumps against the reference (mul_add build) and against the no-fma baseline, and the Vita dumps matched desktop no-fma. None changes a recovered routine's arithmetic, its operation order or the `frame.rs` stage order.

| Edit | Files |
|---|---|
| `madd`: `soft-fma` (correctly rounded `fmaf_exact`, fast path + `#[cold]` slow path), `fma-f64`, `madd-audit`/`madd-count` diagnostics | `skate-core/src/madd.rs`, `Cargo.toml` |
| Stage profile (feature `stage-profile`, zero-sized when off): `profile::Scope` in skate-core, tracing spans in skate-sim, `profile_take()`/`arithmetic()` | `skate-core/src/profile.rs`, `physics/board_step.rs`; skate-sim `lib.rs`, `physics/{frame,solve}.rs`, `skater_animation.rs` |
| Solver kernels `#[inline(always)]`; `packed::pass` generic over the kernel closure instead of `&dyn Fn` | `skate-core/src/physics/solver/{contact,contact_data,drive,joint,joint_data,packed}.rs` |
| World queries: skip a triangle whose bounds miss the union of the query volumes' bounds, only when authored query metadata is present (the per-volume test that follows rejected the same triangles) | `skate-core/src/physics/board_world.rs` |
| Action graph: operations borrowed from a shared `Arc<[ActionInstance]>` instead of deep-cloned on begin/update/end/hook | `skate-sim/src/graph_host/action.rs` |
| `Collections`: an index (id → row, keeping the first match as the linear search did) and per-path memoised `load()` behind an `Arc` | `skate-data/src/collections.rs` |
| Narrowphase stack traffic: `best_separating_direction` projects each candidate into one local interval per shape right before comparing it (was: two zero-filled 16-slot interval arrays, 1.5 KB per call); `maximum` fills the caller's zeroed feature record instead of returning 576 bytes by value. Same projections, same selection order | `skate-core/src/physics/world_contact/{projection,primitive_query,primitive_pair}.rs` |
| `BoardWorld::line_candidates`: with query metadata and valid bounds, each candidate mesh's triangles come from a per-mesh bounds hierarchy over the triangle bounds (new `board_world/triangle_index.rs`, built in `with_query_metadata`) instead of a linear filter of the whole mesh range; same indices, same order (union-bounds pruning, sorted per mesh; equivalence test in `board_world/tests.rs`). `candidate_ranges` split into `candidate_meshes` + mapping, same code | `skate-core/src/physics/board_world{,/broadphase,/triangle_index,/tests}.rs` |
| Pack once: new `PackedPrimitive` and `primitive_triangle_world_contacts_packed` / `primitive_pair_contacts_packed` (the old entries wrap them). `query_primitives` packs each volume once per query and each triangle on its first overlapping volume; the skater self-contact assembly packs each volume once per tick. `query_primitives` also takes its union-filtered triangles from the triangle hierarchy (same indices and order) | `skate-core/src/physics/{board_world.rs,world_contact/{mod,primitive_query,primitive_pair}.rs}`; skate-sim `physics/solve/assembly_contacts.rs` |
| Early reject in the generic SAT: `best_separating_direction_unless_above` (new, beside the unchanged `best_separating_direction`) compares the same candidates in the same order, face normals first, and returns None once the running best exceeds the caller's own gate threshold (later candidates only raise it; a NaN best never stops early). Both narrowphase entries form that threshold before the search (pure arithmetic, same expression). `separating_axis_candidates` split into `normal_candidates` + `edge_candidates` (same code). Equivalence test over 40,000 random pairs and thresholds | `skate-core/src/physics/world_contact/{axes,projection,primitive_query,primitive_pair}.rs` |
| Diagnostics, off by default: feature `kernel-bench` (narrowphase input capture + phase timing, `world_contact/kernel_bench.rs`, two `#[cfg]` capture calls); `primitive_query::contact_points` and `primitive_pair::separating_direction` made `pub(super)` for it | `skate-core/src/physics/world_contact/{mod,primitive_query,primitive_pair,kernel_bench}.rs`, `Cargo.toml` |
| Animation keys decoded on first use: `VbrStream` (the owned header of `VbrDecoder`, which now wraps it), `decode_frame_into`; the IDCT weights computed once per (frame, channel type) instead of per value, same operations; `ClipFrames::frames` is a `Frames` (decoded, or stock keys decoded on first use, each 8-frame block unpacked once); readers use `Frames::get`/`to_vec`. A corrupt key now errors when first sampled instead of at the clip's first use. | `skate-core/src/animation/vbr/{mod,transform}.rs`; `skate-data/src/animation_frames{,/native}.rs`; skate-sim `animation_pose.rs`, `animation_pose/authored_clips.rs`, `tests/animation_pose.rs` |

## M7 memory edit

The M3 lazy keys were never freed: playing on the Vita grew the heap by ~30 MB in four minutes, and all 131,642 stock keys would not fit in the 224 MB heap. Checked like the M3 edits: `verify-parity.sh` 20/20 + 20/20; `sim-oracle run-live` (which trims) 20/20 against the no-fma baseline with the default bound, and with `ORACLE_ANIM_BUDGET=0` (every idle clip released each second, then re-decoded) 20/20 no-fma and 20/20 against the reference (mul_add); the asset tests give the same results as before (231 passed, the same 17 failing).

| Edit | Files |
|---|---|
| A stock clip's decoded keys and unpacked blocks live in one cache behind a `Mutex` and are handed out as `Arc<[SampleWords]>`; `Frames::get` returns a `Key` (derefs to `&[SampleWords]`). `Frames::{decoded_bytes, release}`, `AnimationFrames::trim_decoded(budget)`: releases the keys of the least recently used clips not requested in the last two epochs while above the budget, down to three quarters of it. A failed key decode is no longer cached (it fails again identically). Callers: one `&` (`validate_samples(&clip.frames.get(i)?, …)`) and `zip(second)` → `zip(second.iter())` in `sample_clip`, same values in the same order. `live::Session` trims to `ANIMATION_BUDGET` (24 MiB) after every 60th tick | `skate-data/src/animation_frames{,/native}.rs`; skate-sim `animation_pose.rs`, `physics/live.rs`; sim-oracle `ORACLE_ANIM_BUDGET` |

## M6 data edits

For the baked packs (PLAN.md §3 "std::fs behind a trait", §7 `sim.pak`). No simulation code changes. With nothing mounted the loaders behave exactly as before (verify-parity 20/20 + 20/20); the corpus loaded only from `sim.pak` + `anim.pak` (strict, no disk) is 20/20 identical to the no-fma baseline (`sim-oracle run-live --packs`).

| Edit | Files |
|---|---|
| New `vfs` module (mounted in-memory files, else `std::fs`; strict mode) and `binary` module (magic-tagged binary records) | `skate-data/src/{vfs,binary,lib}.rs` |
| The loaders' file reads go through `vfs` (`read`, `read_to_string`, `is_file`) | skate-data `collections.rs`, `physics_skeleton.rs`, `manifest.rs`, `state_graph.rs`, `gesture_patterns.rs`, `input_config.rs`, `abin/bank.rs`, `skate_map.rs`; skate-sim `camera/runtime.rs`, `physics/climbing/clip.rs`, `animation_pose/authored_clips.rs` |
| The three JSON loaders also accept the binary form of their file (same values: strings as-is, numbers as their parsed integers), then run the same validation; `encode_json` writes it (desktop bake). `GameAssets::parse` split into deserialize + `validate` | `collections.rs`, `physics_skeleton.rs`, `manifest.rs` |

Not code edits, but they change timing: Vita builds use `-C llvm-args=-pre-RA-sched=list-ilp` (docs/toolchain.md), and Vita binaries link `tools/vita-libc/aeabi_mem.c`, which routes the AEABI memory builtins to SceLibc (docs/toolchain.md). The Vita dumps stay bit-identical to desktop no-fma.

Tests not copied (outside the sim): everything under skate-game `src/tests/` not listed above (`map_transition`, …) and the tests of uncopied modules (renderer, HUD, multiplayer, replay, customiser UI).

Synthetic fixtures: `maps/format-demo.skate` (737 B, made by the reference's `tools/make_skate_demo.py`) and `crates/skate-data/tests/fixtures/retail-collision.rwcmset` are not retail data, but their extensions are blocked, so they stay gitignored. `scripts/setup-fixtures.sh` copies them from the reference.

## M10 streamed collision (skate-sim, skate-core)

| Edit | Files |
|---|---|
| New `collision_window.rs`: cluster set decoded as `skate_world::retail_collision_world` does (lines 137-212 copied, triangles kept per query mesh), windows built from the meshes near the deck and the skater in archive order | `skate-sim/src/collision_window.rs` |
| `decode_native_edges` made `pub(crate)` | `skate-sim/src/skate_world.rs` |
| `BoardWorld::broadphase_limits` / `with_broadphase_limits` (read and carry the whole world's query paddings; additive, no change to any query) | `skate-core/src/physics/board_world.rs` |
| `GamePhysics::window`, `follow_window` (called once per tick before `frame::advance`), respawn candidates validated against a world built around them; all inactive unless `collision_window::configure` turned windows on | `skate-sim/src/physics.rs`, `physics/live.rs`, `physics/respawn.rs`, `tools/sim-oracle/harness/oracle_harness.rs` |

## Resync to b3c9679 (2026-10-08)

Upstream moved from `cb79689` to `b3c9679` (158 commits), with fixes the port lacked (the darkslide exit, state 202 `PhysicsAirSecondary`, crashed the session). Every copied file was merged three ways: ours (the `cb79689` copy plus the edits recorded above) against base `cb79689` and upstream `b3c9679`, with the PLAN.md §5.3 `madd` transform applied to base and upstream first (it reproduces 1,122 of the 1,202 copied files exactly; the other 80 are the recorded edits). 104 files took upstream's version unchanged, 13 merged cleanly, 36 new upstream files were copied (with the `madd` transform), 2 deleted files were removed, and 22 conflicts were resolved by hand as listed here. The edits above still apply; the commit column of the copy rows now reads `b3c9679` for the merged files.

| Edit | Files |
|---|---|
| Conflict: upstream's water exclusion (collision groups 7 and 16 skip water triangles) and its `break 'triangles` on a full contact buffer, merged into the M3 query loop (per-mesh index, pack once); the flush and the returned contacts are unchanged | `skate-core/src/physics/board_world.rs` |
| Conflict: upstream's own lookup index and `Field::array` / `FieldArray`; the M3 index is kept, `override_profile` is ported onto it (copy on write, index rebuilt), the binary pack form encodes `array` (kind `COL2`; a `COLL` sim.pak must be rebaked), upstream's index tests run against our index | `skate-data/src/collections.rs` |
| Conflicts kept as both: profile scope + upstream `solved_reactions`, `vfs`/`binary` + upstream modules, M3 test + upstream water tests; `input_config.rs` keeps `vfs` (no `fs` import) | `board_step.rs`, `skate-data/src/lib.rs`, `board_world/tests.rs`, `input_config.rs` |
| Upstream removed trainer tuning (`GamePhysics::trainer`, `GroundSettings::tuned`) and the vehicle ejection from the tick: taken | `physics.rs`, `physics/frame.rs`, `physics/ground_runtime/settings.rs` |
| `bevy::log` → `tracing`, `Resource` derives removed (as at `cb79689`) in the merged code | `scoring_runtime.rs`, `physics/input_teleport.rs`, `physics/offboard/contact_toolkit/world.rs`, `input/controllers.rs`, `camera/runtime.rs` |
| `physics::advance` stays the plain function; upstream's `board_surface` (read-only, for audio) added; its `dev_trace` checkpoints (debug builds only) come with `frame.rs` | `physics.rs` |
| Mod and host hooks left out of the solve (`apply_parts`, `solid_contacts::append`, `modding::player_physics::{joints, filter_joints, filter_drives, filter_possession, before_solve, after_solve, joint_loads}`, `capture_dynamics_reactions`): with no mods each is the identity or only observes; `solve.rs` and `network.rs` keep the `cb79689` copy | `physics/solve.rs`, `physics/network.rs` |
| New stand-ins for host crates: `dynamics.rs` (`SolidBody`, an uninhabited type: no Rapier solids can exist), `physics/solid_contacts.rs` (`constrain_ground`: upstream returns the target unchanged with no solids), `physics/offboard/mod_solid_ground.rs` (empty vehicle edge cache, no vehicle probed or found); `Proxies::{solids, groups}` (always empty); `skate_dynamics::SolidBody` → `crate::dynamics::SolidBody` | those files, `physics/offboard/{air_selector,ground_geometry,ground_sync}.rs`, `physics/network.rs` |
| `SkaterRuntime::{mod_part_overrides, mod_contact_frame, mod_joint_overrides}` removed (written, never read without mods) | `physics/skater.rs` |
| `custom_difficulty::load_collections` returns the stock collections without the Custom overlay (no Custom difficulty on the Vita; the overlay only adds `physics_mode/test`, which the stock difficulties never read, and would copy the shared table per load) | `custom_difficulty.rs` |
| `camera/angle.rs` (retail Camera Angle setting through `skate-mods`) not copied: `CameraRuntime::camera_type` is upstream's default, High (graph type 1). `skate_mods::presentation::CameraShotTuning` → the copy in `skate-trainer-stub` (serde removed); nothing sets one | `camera/runtime.rs`, `camera/shot_data.rs`, `skate-trainer-stub/src/lib.rs` |
| `presentation::capture` uses upstream's `presentation_frame()` (through the new skate-core `ManualCam`) and counts `cuts`; replay recording still left out | `presentation.rs` |
| Session marker: upstream's facing (`physics::facing_from_visual`) replaces the velocity projection; the `modifier` field for the menu-open sound; the Bevy plugin and message writers stay out | `session_marker/mod.rs` |
| `DevicePacket::kind` (always `None`) and a stub `ControllerKind`; `input/controller_kind.rs` (SDL) not copied, `ControllerInput` keeps the `cb79689` copy | `input/platform.rs`, `input/live_input.rs`, `input/controllers.rs` |
| skate-ui: upstream deleted `Draw::size`, `Vm::construct` (used here by vita-render and the menu runtime) and made `hud_runtime` localize through the Bevy `scoring_hud`: all three files keep the `cb79689` copy | `skate-ui/src/{apt_scene,apt_vm,hud_runtime}.rs` |
| Not copied: skate-data `src/xex/` (Xbox 360 executable decryption: it embeds the retail XEX key), `src/ocean_pca.rs` and `examples/xex_unpack.rs` (its only users, the ocean renderer's wave table read from the executable); never needed by the simulation |
| Not copied (Bevy, Rapier, SDL or upstream documentation): `tests/audio_state_capture.rs`, `input/tests/action_docs.rs`, `input/tests/controller_kind.rs`, `physics/camera_angle_tests.rs`, the Rapier platform test in `physics/foot_ik_queries.rs`, and the two HUD tests in `scoring_runtime/tests.rs` (`composed_names_localize_every_component`, `landed_multiplier_reaches_hud_across_repeated_tricks`) | |
| Physics setting (game API, no change to a solve): `GamePhysics::set_solver_iterations` / `Session::set_solver_iterations` set `settings.step.iterations`, which every board/skater solve reads. Upstream's 50 (retail, `settings::SIMULATION_ITERATIONS`) stays the default; the Vita's "Fast" physics uses 25 (the `cb79689` value, `physics/default.RWMaxIterations`). `ORACLE_ITERATIONS=N` applies it in the harness and `run-live`, and `reference.sh` sets the reference's constant to N, so both modes are checked against upstream | `physics.rs`, `physics/live.rs`, `tools/sim-oracle/{harness/oracle_harness.rs,src/main.rs,reference.sh}` |
| Trick stance (upstream changed the scorer's stance to `[switch, fakie, regular, nollie]`, the nollie flag from the descriptor, 825E51A0): `live::Score::stance` reads slot 3 | `physics/live.rs` |
| Oracle harness: the grind words are now the active grind name, the raw animation chromosome, word 136 and the distance (upstream removed `mod_grind` and `active_family`); `skate_vita_only!` wraps the window follow (reference.sh defines it empty for the reference build) | `tools/sim-oracle/harness/oracle_harness.rs`, `physics.rs`, `tools/sim-oracle/reference.sh` |

Tests after the resync (desktop): skate-core 640 passed, 2 failed (the same two fail in upstream's own build); with `SKATE3_ASSET_ROOT`, skate-sim 276 passed and 16 failed, the same set as before the resync minus `production_factory_routes_three_handlers_and_all_five_conditions_to_grind_owner`, which upstream fixed; skate-data fails `private_archive_decodes_all_clusters` as before.

## Retail audio (branch retail-audio, stage S0, 2026-10-09)

| Source | Destination | Commit | Edits |
|---|---|---|---|
| `skate-3-rust-engine/crates/skate-audio/` (whole crate: `src/`, `tests/`, `examples/`, `Cargo.toml`) | `crates/skate-audio/` | `b3c9679` | PLAN.md §5.3 as for skate-core: every `.mul_add(` → `.madd(` with `use skate_audio_fma::madd::Madd as _;` after the leading doc block; `no-fma` feature forwarded. Default build: all 253 runnable tests pass; no-fma: one golden-vector test (`grain::board::positions_match_the_golden_vectors`) differs by 1–2 ulps, the fused rounding it pins |
| `skate-3-rust-engine/crates/skate-audio-fma/` (whole crate) | `crates/skate-audio-fma/` | `b3c9679` | `.mul_add(` → `.madd(` in `body.rs`; new `src/madd.rs` (`Madd`: `mul_add`, or `a*b+c` with the `no-fma` feature) and `pub mod madd;` |
| Diagnostic stage profiler (feature `profile`, off by default; empty scopes otherwise): `src/prof.rs`, scopes in `runtime.rs`, `mixer.rs` (voice render), `bus/mod.rs` | `crates/skate-audio/src/` | | Timing only, no computation changed |
| Vita performance (retail-audio option A, measured with `scripts/vita-audio-bench.py`): `skate_audio_fma::biquad4` runs four channels of the biquad in NEON lanes (`neon.rs`, only with `no-fma` on ARM; unfused `vmla` = the same `a*b+c`, Vita output still bit-identical to the desktop); `dsp::biquad::kernel_many` feeds it the channels that take the plain kernel (HPF/LPF and PeakingIir2 `process`); `EqBuses::render` skips an idle bus (silent input and a state unchanged by the last silent block, or only the bias residue under 1e-12), dropping that residue from SFX Master (test `idle_buses_drop_only_the_bias_residue`: under 1e-11); `Buses` tracks touched / dirty EQ inputs so untouched ones are neither read nor cleared | `skate-audio-fma/src/{lib,neon}.rs`, `skate-audio/src/{dsp/biquad.rs,dsp/peaking.rs,bus/eqchain.rs,bus/mod.rs}` | | |
| `crates/skate-audio/tests/fma_paths.rs` `scene`, `drive`, `reverb01` | `tools/audio-bench/src/main.rs` | `b3c9679` | Voice count as a parameter, no poison variants |
| `skate-3-rust-engine/tools/asset_pipeline/audio_export.py` `BANKS` (`b3c9679`) | `tools/vita-bake/src/audio.rs` `BANKS` | `b3c9679` | The board set's bank names, without extensions. The exporter itself is not copied: the user runs the reference's `convert` (with the vgmstream r2117 Linux CLI) and `vita-bake audio` packs its output |
| Sample storage (retail-audio S1) | `crates/skate-audio/src/adpcm.rs`, `mixer.rs` | | `Pcm::channels` is `Vec<adpcm::Samples>` (`F32` as the reference, or `Adpcm`: 4-bit IMA in 64-frame blocks that decode alone, to `f32` as the reference converts PCM16); the voice and grain resamplers read ADPCM through a block reader, the `f32` path is the reference's code; every `Pcm { channels: … }` literal in tests, examples and the bench maps its planes into `Samples::F32` |
| `skate-3-rust-engine/crates/skate-game/src/game_audio/skate_events.rs` (whole file) | `crates/skate-sim/src/audio_events.rs` | `b3c9679` | The `observe` Bevy system is a plain function (physics, skater, cues, memory, `dt` instead of `Time::delta_secs`); `bevy::prelude` → `glam::Vec3` and `tracing::info`; no `Resource` derive; the state-log block (diagnostic file) and the bindings only it used removed; `Riding` public (re-exported as `AudioRiding`). `Session::step` runs it after the presentation capture when the tick succeeded, `Session::take_audio` hands the publication out; `run-live` stays 20/20 bit-identical to the no-fma baseline |
| `skate-3-rust-engine/crates/skate-game/src/game_audio/library.rs` (whole file) | `crates/game-audio/src/library.rs` | `b3c9679` | No Bevy: `load` is `load_with` without content overlays (the manifest as it is), the overlay types and tests, the Bevy clip playback (`Clip`, `clip`, `ambience`, `sample`, `release`) and its test removed; files read through `skate_data::vfs` (sound.pak mounted); `wav_pcm` also reads `.ima` (`skate_audio::adpcm`); the world sources' tuning stays raw JSON (`crate::world_sources`), mod-file paths go through a stand-in that never matches (`crate::skate_mods`) |
| `skate-3-rust-engine/crates/skate-game/src/game_audio/player_audio.rs` (whole file) | `crates/game-audio/src/player_audio.rs` | `b3c9679` | `bevy::log` → `tracing`; `skate_events::Riding` → `skate_sim::AudioRiding`; the two mod-only tests (`event_tags_fire_on_real_posts`, `rules_mute_and_layer_the_players_pop_grind_and_landing`) removed |
| `…/game_audio/grain_bed.rs` (whole file) | `crates/game-audio/src/grain_bed.rs` | `b3c9679` | The Bevy system `update` removed (its body is `Native::grain_bed_update`); `bevy::prelude` → `glam` / `tracing`; `timing::lock` (a plain lock with its profiler off) → `lock` |
| `…/game_audio/native.rs`: the host clock, `Pass`, the constants, `Native`, `PendingPass`, `WorldInstances`, `start*`, `ensure_bank`, the player bank loading, `bank_id` / `bank_loaded` / `loaded_banks`, `redeliver`, `release`, `hold_flag_inputs`, `azimuth` | `crates/game-audio/src/native.rs` | `b3c9679` | No prefetch worker, front end or resident mod banks (fields and calls removed); `HostClock::pass` takes the step count; `azimuth` takes the camera's position and forward (`Listener`); the Bevy systems `mixmap_frame`, `mixmap_tick`, `grain_bed::update`, `reverb_frame` are methods called in the reference's order by `Native::frame` (resources as `FrameInput`; reverb01 and no reverb zones: map audio regions not ported); `Native::render` = `Runtime::fill_stereo` (the Bevy stream) |
| `…/game_audio/mod_audio.rs`: `EventKind`, `Source`, `EventRow`, `EventBuf`, `MAX_EVENTS`, `record`, `Observed` and its `SpliceHost` impl, `intern`, `player_slot` | `crates/game-audio/src/mod_audio.rs` | `b3c9679` | Unchanged; `mod_rules::RuleSet` is an empty stand-in (no mods: never made, mutes nothing) |
| Vita host integration (retail-audio S1 step 4b) | `crates/skate-audio/src/adpcm.rs`, `mixer.rs`; `crates/game-audio/src/library.rs`, `grain_bed.rs`, `native.rs`; `crates/skate-data/src/vfs.rs` | | `Adpcm` keeps `(bytes, start)` so an `.ima` shares the mounted pack (`pcm_from_ima_shared`, `vfs::read_shared` / `mount_range` / `unmount`); `Library::read_pcm` / `shared_pcm` for the bank and grain WAVs (was read, copied and decoded). `Mixer::render`'s loop / fade path reads ADPCM through the block reader (`Voice::sample_read`; it decoded a 64-frame block per output sample). `Native::renderer` (`Renderer`: `fill_stereo` from the audio thread), `set_max_voices`, `voices`. All bit-identical (sim-oracle `audio-render` WAVs byte-equal) |
| Vita audio performance | `crates/skate-audio/src/dsp/reverb.rs`, `bus/env.rs` | | `ReverbModel1::process` runs one comb at a time over the block when every comb delay is at least the block (same sums in the same order; test `the_blocked_combs_match_the_per_sample_loop`). An env side faded to weight 0 whose wet outputs stayed under 1e-12 for 1 s stops rendering (LFOs keep running) until its weight rises |
| Streamed collision memory (retail-audio S1 step 4b) | `crates/skate-core/src/physics/board_world.rs`, `crates/skate-sim/src/collision_window.rs`, `physics.rs`, `physics/respawn.rs` | | `BoardWorld::new_in` / `with_query_metadata_in` (bounds buffer reused) and `into_buffers` (`WorldBuffers`); collision windows take their triangle, bounds and surface buffers from a pool (`reserve_buffers` at boot on the Vita, `recycle` from the window free thread, a finished build of an ended session, `Drop for Window` / `GamePhysics` / respawn `Scene`). Same contents: `scripts/verify-windows.sh` 20/20 with `bg:80,20` (University, DownTown), `bg:80,12` (University), `bg:60,8` (DownTown) |
| Packs on the card (retail-audio S1 step 4c) | `crates/skate-data/src/vfs.rs`, `crates/vita-pack/src/lib.rs` | | vfs entries are a range of a buffer in memory or a range of a pack file on the card (`mount_disk`, read on every access); `vita_pack::verify_file` hashes a pack in 1 MiB pieces. The Vita mounts sim.pak, anim.pak and ui.pak this way (38 MB less resident; index cached in `<pack>.idx`) and leaves the grains' unused speed bands out of sound.pak (6.7 MB; renders byte-equal without them) |
| Animation banks loaded once (retail-audio S1 step 4c) | `crates/skate-data/src/animation_banks.rs` | | `AnimationBanks::load` memoised per asset root, as `Collections::load` (M3): every session asked for OnBoard.abin (10.8 MB, two blocks per load) and OffBoard.abin again, which a fragmented Vita heap stopped having. verify-parity 20/20 (fma vs reference, no-fma vs baseline) |

| Background checks yield to the loaders (retail-audio S1 boot fix) | `crates/skate-data/src/io_gate.rs`, `vfs.rs`, `crates/vita-pack/src/lib.rs`, `district.rs` | | New `io_gate`: a loader's card read (vfs disk entries, the `sound.pak` mount) holds a `Foreground` guard; `verify_file` of the packs and of the districts calls `wait_background` before each piece (at most 1 s of waiting per piece). No data changes |

## Resync to 2e16697 (R1, 2026-10-13)

Upstream `main` moved from `b3c9679` to `2e16697` (11 commits: the grinds' per-volume world-query box and rail bails, the merged PRs #57-#60 with the retail menus 1, see-through fences). The port's physics, data and audio code was merged three ways as at `b3c9679` (ours / base / upstream, the `madd` transform applied to base and upstream first, `tools/sim-oracle/reference.sh` rebuilt at the new commit). Only what the Vita needs was taken; the rest is PC-side (the Bevy host and its menus) or belongs to later milestones.

| Taken | Files | Edits |
|---|---|---|
| The world query's per-volume box (`82777E70` / `8277BC58`): the swept, scaled box every triangle is tested against (also without query metadata), `VolumeMotion` in `BoardWorldVolume` (replaces `linear_velocity`), the capsule and box bounds in the recovered operation order (`82AD97A0`, `82AD9558`) | `skate-core/src/physics/board_world.rs`, `board_world/broadphase.rs`, `board_world/tests.rs`, `physics/tests/board_world.rs` | `.mul_add(` → `.madd(` (+ `use crate::madd::Madd as _;`); merged clean with the M3 per-mesh index and union filter (the union now includes the swept box, as upstream's cluster preselection does) |
| `linear_velocity: …` → `motion: VolumeMotion::of(…)` in the four volume constructors | `skate-sim/src/physics/colliders.rs`, `skeleton_colliders.rs`, `solve/assembly_contacts_tests.rs` (upstream `network.rs` has no counterpart: the network is a stand-in) | mechanical |

| Not taken (and why) | Upstream files |
|---|---|
| Retail menus: the menu tables, values, movies and the Settings, pause, Trick Guide screens come with R2 / M12 | `skate-core/src/menus.rs`, `menus_tests.rs`, `menu_values.rs`, `skate-data/src/menu_tables.rs`, `tests/menu_tables.rs`, `skate-game/src/retail_menus.rs`, `retail_menu_movies.rs`, `graphics_menu.rs`, `tools/prepare_menu_movies.py`, `tools/asset_pipeline/menu_tables.py` |
| The PC APT runtime and its menu support (our `skate-ui` has its own; the in-game HUD is unchanged) | `skate-game/src/apt_*.rs`, `hud_runtime.rs`, `scoring_hud.rs` (a refactor: `localize_trick` moves to `apt_text`; ours is `skate-ui/src/trick_names.rs`) |
| Audio option volumes into the MixMap Master inputs (`RetailVolumes`; the defaults write the same 32767 as before, so the board sounds are unchanged); to be taken with the Game Settings screen (R2) or A2 | `skate-game/src/game_audio/mod.rs`, `native.rs`, `emitters.rs` |
| PC host, mod VM, overlays and tools: frame-time counter, mods, scripting SDK, the app and the window | `skate-game/src/app.rs`, `main.rs`, `frame_timing/`, `modding/`, `skate-mods/`, `sdk/` |
| Rendering: see-through fences and grates (`environment.transparent`); our renderer and bake are separate (an optional visual follow-up: the material flag in `vita-bake`, alpha in `vita-render`) | `skate-game/src/retail_render.rs`, `retail_world.wgsl`, `tools/asset_pipeline/retail_material.py`, `asset_exports.py`, `versions.py` |
| A headless diagnostic of the PCU Library rail bails (needs the PC host's camera and assets) | `skate-game/src/tests/grind_bail.rs` |
| Docs and examples | `docs/hails-additions/`, `skate-data/examples/` |
| `skate-3-rust-engine/crates/skate-game/src/game_audio/frontend.rs` (whole file; unchanged between `b3c9679` and `2e16697`) | `crates/game-audio/src/frontend.rs` | `2e16697` | No Bevy: `bevy::log` → `tracing`; the Bevy system `frame` is `Native::frontend_frame` (`native.rs`: this frame's requests, then on a pass the object once, volume from the MixMap's Master output 0); the tests are not copied (they need the install; `lib.rs` `the_marker_sounds_start_retails_splice_sounds` checks the same Splice sounds, 235 / 237 / 236, against the baked pack) |
| Session-marker sounds (retail-audio A2, part 1) | `crates/game-audio/src/frontend_marker.rs`, `native.rs`, `crates/skate-sim/src/session_marker/mod.rs` | | `frontend_marker`: `skate_sim`'s audio ids → the `fe` records of `ui_audio::SessionMarkerSounds`' defaults (`cellphone_activate`, `_place_marker`, `_marker_error`, `_goto_marker`); `FrameInput` gains `frontend` (record keys) and `teleport` (the Go To Marker hold, written to `player.teleport_effect`: `Class_Treatment`'s crackle); `Native::has_frontend`. skate-sim: `AUDIO_OPEN` (LB pressed, the reference's `SessionMarkerAction::Opened`). The Vita keeps `sk8_menu` resident (1.2 MB) |
| `skate-3-rust-engine/crates/skate-game/src/game_audio/ambience.rs` (`update`, `mixmap_output`, `crossfade_ready`; `2e16697`) | `crates/game-audio/src/ambience.rs` | `2e16697` | No Bevy: `AmbienceHost` + `Native::ambience_update` / `ambience_output` (called around the MixMap tick as upstream's systems are ordered). The native phase machine (`skate_audio::world::ambience::Controller`), the zone lookup (`Library::region_key`) and the MixMap's AMBIENCE controls are upstream's. The beds are the runtime's direct voices (bank `1 << 22`) over streamed `.ima` files instead of Bevy voices; `RETAIL_SCALE` is taken out as upstream does while the native player runs. **Not ported yet:** the crossfade bank's transition sounds (the phase machine still reports `start_crossfade` / `stop_crossfade`), the per-map fallback bed, the mod events. `FrameInput.map` (name and load number) |
| Streamed samples (retail-audio A2) | `crates/skate-audio/src/stream.rs`, `adpcm.rs`; `crates/skate-data/src/vfs.rs`; `crates/game-audio/src/library.rs`; `tools/vita-bake/src/audio.rs`; `crates/skate-vita/src/packs.rs`, `retail_audio.rs` | | New `Samples::Stream`: one ADPCM channel of a file on the card, a window of three 36 KiB chunks kept filled by a prefetch thread (silence on an underrun), the first chunks read on the opening thread; the decoder is the same (`decode_block_bytes`), test `a_stream_decodes_like_the_in_memory_samples`. `vfs::disk_location`, `Library::bed_pcm` (streams from the pack when mounted from the card, else reads it), `vita-bake ambience` (`ambience.pak`, 137.5 MiB, `private/audio/ambience/<bed>.ima`), `packs::mount_on_card`, the prefetch thread on core 0 at priority 170 (`skate_audio::stream::set_thread_init`) |
| `skate-3-rust-engine/crates/skate-game/src/game_audio/emitters.rs` (the sound-emitter half: `reach`, `falloff`, `Emitter`, `Node`, `prefetch_near`, `update`; unchanged between `b3c9679` and `2e16697`) and `native.rs` (`unload_map_banks`, `has_bank`, `post_emitter`, `claim_emitter_state`, `release_emitter_state`, `set_emitter_position`, `emitter_payload`, `write_position`, `azimuth`) | `crates/game-audio/src/emitters.rs` | `2e16697` | No Bevy; the five MixMap emitter states, the `c_emitter` posts redelivered every frame, the record test and the falloff are upstream's. **Not ported:** the reverb zones (`eVolumeType` 5), published emitters (`WorldEmitter`), the mod rules and events. The `.ems` list per map is a table (`ems_files`; upstream reads it from the stock collections). The azimuth uses the listener's forward vector (no camera roll). **Vita additions:** a loaded bank no started emitter plays and the listener is `EVICT` (90 m) away from is unloaded again (`Native::unload_bank`); a node starts only when its bank is already read (`Prefetch::ready`), never a card read on the host thread |
| `skate-3-rust-engine/crates/skate-game/src/game_audio/native/prefetch.rs` (whole file minus the tests) | `crates/game-audio/src/prefetch.rs` | `2e16697` | `crate::library` paths; the worker runs the stream thread's init (core 0, priority 170: `skate_audio::stream::run_thread_init`); `ready()` (non-blocking). What it hides is the card's latency, not decoding |
| World banks and voices (retail-audio A2) | `tools/vita-bake/src/audio.rs`, `crates/skate-audio/src/mixer.rs`, `crates/skate-vita/src/retail_audio.rs` | | `vita-bake audio` also writes `world.pak` (the 95 banks the maps' sound emitters play, 81 MiB: samples and bank programs; read by proximity from the card) and keeps their manifest entries in `sound.pak`; `Mixer::world_reserve` (0 = the reference): voices of the world group open only while fewer than `max_voices - world_reserve` play (8 on the Vita) |
| `skate-3-rust-engine/crates/skate-game/src/game_audio/crossfade_groups.rs` (whole file; `pub(crate)`) and `ambience.rs` (`start_crossfade`, `stop_crossfade`, the crossfade half of `mixmap_output`) | `crates/game-audio/src/crossfade_groups.rs`, `ambience.rs` | `2e16697` | The zone crossfades' four looping directional voices per zone pair, from the layout table (the reference checks it against the banks' programs; ours does not rebuild it from the program), as direct voices over the crossfade bank's samples (mixer bank `(1 << 22) + 1`). `Main_Ambience_Crossfade_*` go to `world.pak` (`vita-bake audio`) |
| `skate-3-rust-engine/crates/skate-game/src/game_audio/emitters.rs` (`reverb_zones`, `zone_records`, `zone_enabled`, `zones_at`) and `native.rs` (`reverb_frame`) | `crates/game-audio/src/emitters.rs` (`ZoneHost`), `native.rs` (`reverb_frame`) | `2e16697` | No Bevy; published `ReverbZoneVolume` zones not ported. `reverb_frame` takes the preset of the map's `audio_reverb` region under the skater (`Library::region_key`; reverb01 where none) and the zones the listener is inside (the `.ems` files' `eVolumeType` 5 records), as upstream |
| Pack index shipped by the bake (retail-audio A2) | `crates/vita-pack/src/files.rs`, `tools/vita-bake/src/audio.rs`, `crates/skate-vita/src/packs.rs`, `scripts/vita-run.py` | | `vita_pack::files::{index_of, encode_index, decode_index}`: `vita-bake audio` / `ambience` write `<pack>.pak.idx` (payload hash, then name, offset, length per file) next to the pack, `vita-run --upload-packs` uploads it, the Vita reads it instead of one pass over the pack (11 s of card time for a 137 MB pack); the format is the one the Vita already cached |
| LiveArea art (2026-10-14) | `static/sce_sys/icon0.png`, `pic0.png`, `livearea/contents/bg.png`, `startup.png` | | The project author's own images (`images_app_vita/`), converted to 8-bit palette PNGs with alpha kept as tRNS (the installer rejects RGB PNGs). Replaces the placeholders `tools/make-livearea-art.py` generated; that script is removed. No Skate 3 asset. |

## L1a: upstream PR #52 (living world), simulation part

Taken from the PR branch (`pull/52/head` at `ca647a3`, based on upstream `main` `2e16697`) although it is not merged upstream (the user's decision, PLAN.md §20). Method as for R1: a three-way merge per file (ours / base `2e16697` / PR head) with the `madd` transform applied to both sides first.

| Part | What | Edits |
|---|---|---|
| skate-core | 34 modified files merged without conflict; 84 new files (`living_world/*`: population, peds, traffic, NPC skater lines and AI, DMO streaming; `riding/skitching/*`, `player/offboard/move_object*`, `player/ghost.rs`, `riding/grounded/state/board_path*`) | `madd` in `board_path.rs`; `board_world.rs`: the M3 per-mesh triangle index is rebuilt where upstream rebuilds its query index (`append_triangles`, `replace_triangles_batch`: props created and moved) |
| skate-sim (skate-game's physics) | 52 files merged clean; conflicts resolved in `physics.rs` (our collision window kept beside upstream's prop layer, prop dynamics and prop carry; the Bevy plugin, systems and `present` left out), `frame.rs`, `solve.rs` (prop-layer contacts before our stage span), `skater.rs` (`Takedown`), `respawn.rs`, `camera.rs` (Bevy camera left out), `skate_world.rs` (render left out, `PropCollisionLayer` taken), `air_trajectory/grind.rs`; new: `prop_dynamics.rs`, `prop_carry.rs`, `prop_layout.rs`, `skitch_state.rs`, `ground_runtime/skitch.rs`, `board_path.rs`, `grind_world/props.rs`, `graph_host/motion_skitching.rs` and their tests | Bevy log macros → `tracing`, Bevy derives and systems (`apply_prop_tuning`, `sync_prop_transforms`, `apply_carry_settings`, `apply_respawn_settings`, `emit_respawns`) removed; `prop_carry_hud.rs` and `manual_landing_log.rs` (PC HUD and log) not taken; `solid.id` → `solid.id()` on the stand-in `dynamics::SolidBody`; stand-ins `frame_timing::hitch` (no clock), `trace_all::on` (false), `modding::player_physics::apply_parts` (no mods: no overrides), and `living_world.rs` with the PR's `vehicles::{CarGrabSpline, CAR_GRAB_TAG, parse_grab_splines}`, `npc_skaters::{PROXY_ID_TAG, prop_volumes}`, `peds::prop_obstacle_inputs`, `npc_sim::{WIPEOUT_BIT, walker_input}` copied verbatim; the Bevy-world tests of `skate_world.rs` and `skater_context_tests.rs` (Rapier solids) are not compiled |
| skate-data | `animation_metadata.rs`, `skate_map.rs` merged; new `aipath.rs`, `living_world.rs`, `ped_anim.rs`, `ped_nav.rs`, `roads.rs` and their tests | `lib.rs`: the new modules beside ours (the PR's `xex`, `ocean_pca`, `menu_tables` are main's, left out as in R1) |

Checks: the PR's reference (`reference.sh` with `ORACLE_REFERENCE` at the PR head) gives the same 20 dumps as `2e16697` on the Course (the living world adds, it changes no default physics); `scripts/verify-parity.sh` 20/20 and 20/20; skate-core 1011 of 1012 tests (the known `a_moving_group_8_body…` failure, also on main), skate-sim 277 of 278 (`embedded_static_rwcm_hits_distinct_actor_query_ids`, also on main).

## L1b: movable props (PR #52's props on the Vita)

Upstream renders the props in Bevy (`skate_world` render, `sync_prop_transforms`); ours is new code, and four upstream behaviours are changed (marked `skate_vita (L1b)` in the source).

| Part | What | Origin |
|---|---|---|
| skate-data `prop_package.rs` | `<map>.props` (SKPR): the `.skate` prop package without texture pixels (materials, texture metadata, positions, indices, extensions) | ours |
| skate-sim `skate_world.rs` | prop loaders read through `skate_data::vfs` and prefer `<map>.props`; `prop_template_ranges` (id → template draw range for the renderer) | upstream loaders, vfs ours |
| skate-sim `living_world::dmo_stream` | the DMO streaming census (spawn within 90 m, cull at 100 m, pool 49, every 4 ticks) driven from `Session::step` after the tick | port of PR #52's Bevy system, logic verbatim |
| skate-sim `live.rs` | `Session::prop_poses` / `prop_counts`, `PropDynamics::render_pose` (with the instance scale) | ours |
| skate-core `board_world` | `QueryIndex::refit` (exact: node bounds recomputed bottom-up, results sorted), incremental `TriangleIndex::rebuild` (changed meshes only), `triangles_overlapping` (a triangle range against bounds through the per-mesh hierarchy); the upstream `fingerprint` test compares query results instead of the index structure | ours (M3 index) |
| vita-bake `props`, `prop-stats`; vita-pack `PropRender`, `Kind::Props` | `props.pak`: per world the `.props` package, the template render (`.render`), a slim sidecar `.json` and a slim `living_world/tables.json`; `VITA_BAKE_MOVABLE_PROPS=1` drops the static copies of movable props from park and district renders | ours |
| vita-render `scene.rs`, `shaders/prop.vert.cg` | template meshes drawn per instance with a model matrix, frustum-culled, the world's light and fog; textures uploaded when the props load | ours |
| skate-vita | `props.pak` mounted, the props render handed to the main thread at world load, poses published every tick; feature `stage-profile` prints the prop stages (`STAGES` line) | ours |

Changed upstream behaviour (all props-only: the Course has no props, parity unaffected):

- **Skater push against the prop's triangles** (`PropDynamics::triangle_contacts`), not its contact box: the box of a quarter pipe is a 2 m wall at its lip and a rail's a slab under the bar, so riding up a ramp or beside a rail shoved the prop away. A contact whose face is turned up (normal y > cos 45°) carries the skater and does not push (riding a quarter pipe kept it awake). Test `legacy_push_keeps_bench_awake_with_parked_deck` now expects no push (the deck sits clear of the bench's faces).
- **Props seated on the floor** (`PropDynamics::seat`): the parks place props 2.5 cm above the floor and they sleep where placed, so a ramp's lip was a step of a wheel's radius. Once per stream-in an upright sleeping prop is lowered onto the floor below its box bottom (gap up to 6 cm); the seated pose becomes the spawn pose. Test `a_prop_placed_above_the_floor_is_seated`.
- **Base-plane edges welded to the floor** (`skate_world::floor_base_edges`): the edges on a prop's lowest plane take the floor as their neighbour (no convex bit, the floor normal's cosine) instead of the prop's own bottom face, whose knife-edge held wheels back at a ramp's lip. Test `a_ramp_lip_on_the_floor_does_not_hold_a_wheel_back`.
- **Foot IK sees the props** (`foot_ik_queries::query`): the props' world after the static one. Walking on a prop still uses the static ground otherwise (biped ground query, contact scene); feet sink a little on props.

**Memory and districts (the districts' props: DownTown 786, University 479).** Upstream keeps every prop's collision in the layer (~150,000 triangles in a district: +34 MB steady, +53 MB peak on the Vita, DownTown ran out of memory). Ours:

- `skate_world::PropTemplate`: a prop type's triangles (scale folded), surface tags and edge features (welded within the template; base-plane edges on the floor) once per template range, scale and handedness; instances share them.
- Collision slots: a map prop has triangles only while streamed in. `PropCollisionLayer::rebake_many` takes `LayerPose` (`None` streams out: the triangles parked at `HELD_PARK`, the slot freed for the template); an instance without a slot takes a free one or new triangles (`BoardWorld::append_triangles_batch`, one index rebuild); a rebake to the pose already baked is skipped. Maps load unbaked (`prop_layer_unbaked`); bodiless instances are baked where placed; `build_prop_layer` (tests) bakes every instance. `PropDynamics::prepare_streamed` (each step and at load) gives streamed-in bodies their slot and seat in one batch. Test `streamed_props_share_their_templates_collision_slots`.
- The census's first pass runs at session load (a district's first tick took 1.3 s).
- Result on the Vita: DownTown heap 131-149 MB in use, peak 151 MB (before L1: 137 / 153); University's prop triangles 149k → 1.6k.

**Performance (DownTown, exact: census decisions, skater dumps and final prop poses bit-identical on DownTown and BlackBoxPark scenarios).**

- skate-core `living_world::dmo::DmoStream::step`: scores once per pass and lazily, the live objects in a `BTreeSet` ordered by (`total_cmp` key, id) for the eviction front, the touching groups' members precomputed (upstream rescanned every placement per spawn attempt and per group member): census 7.7-9.2 → 0.5-1.4 ms/tick.
- `living_world::dmo_stream`: the placements built for a fill only (`dmo_placement_count`); `Session::prop_poses` walks the streamed bodies (`streamed_poses`) instead of building and sorting the placements every tick.
- `GamePhysics::stream_props`: a pass's collision takes each instance's last state (the census evicts and respawns the same props with a full pool: six bakes per pass for no change).
- `PropDynamics::row_corrections` / `contact_corrections`: the static triangles whose bounds reach the box (`near_triangles`, padded by the world's broadphase limits), in upstream's order, instead of every triangle of the candidate meshes (~12 ms per awake prop per tick); no solve when no body is awake.
- Push loop: the volumes' bounds once per step and their union as a reject.
- Prop rail queries (`RailSource for PropDynamics`): the bodies with rails in id order (`rail_bodies`), tested in place with the same distance reject, instead of building and sorting every prop's posed rails per query; `rail_poses` uses the same list. Test `prop_rail_queries_match_the_posed_list`.

Measured (Vita, `stage-profile`): BlackBoxPark riding the quarter pipe 15.8-18.3 ms/tick before (prop step 4-7 ms) → 12.5-14.8 ms (prop step 0.9-2.5 ms); DownTown at the spawn: without props 14.4-17.5 ms/tick (A/B, same build, `props.pak` renamed), with props 15.4-17.0 ms in normal play (the prop paths 2-3 ms), more when the district's collision window rebuild (1.8-3 s at ~50k triangles, not prop work) stalls the tick. `scripts/verify-parity.sh` 20/20 and 20/20.

`skate-vita` feature `stage-profile`: `STAGES` (skate-core `profile` scopes, `z_*`) and `SPANS` (the sim's tracing spans; `span_profile.rs` copied from `tools/sim-bench/src/profile.rs`) every 600 ticks.

## L1: district collision windows (M10 streamed collision, made to keep up in DownTown)

Ours (M10 code). At DownTown's spawn (windows of ~50,000 triangles, rebuilt every 8 m, `packs::WINDOW` 60 m / 8 m) a rebuild took 1.8-3 s on the Vita and the tick waited for it 6-15 times a run, with or without props. Phases on the device (`WINDOW_PHASES`, now printed with `stage-profile` too): card reads 0.2-4.7 s, decoding 0.1-0.7 s, index 0.4-1.4 s.

| Part | What |
|---|---|
| skate-core `board_world/triangle_index.rs` | trees hold indices local to their mesh (the mesh's first triangle added at query time); `MeshTree`, a shareable mesh hierarchy; local indices order as the absolute ones, so the trees and results are the same |
| skate-core `board_world.rs` | `triangle_bounds_of` (the per-triangle bounds every world computes), `with_query_metadata_prebuilt`: a world from given triangle bounds and mesh trees (the partition checks kept) |
| skate-sim `collision_window.rs` | a decoded cluster's runs keep their triangle bounds and `MeshTree`; a window copies them instead of recomputing them for every triangle |
| vita-pack `district.rs` | `DistrictFile::reopen` (a second handle sharing the index); `read_range` is a foreground read (`skate_data::io_gate`) |
| skate-vita `packs.rs` | the collision reads on their own handle (they queued behind the tile streaming on one lock); the pack and district verify threads below the window builder (190 < 170; the 257 MB blake3 check starved it) |

Exactness: DownTown with streamed windows (60 m / 8 m, 11 rebuilds) dumps bit-identical to the previous builder; parity 20/20 and 20/20. Device (DownTown spawn, props on): 165 rebuilds, mean read 228 + decode 185 + index 101 ms (later in the session 100-500 ms a rebuild), 7 waits all in the first minutes, ticks 12.5-14.4 ms (16-27 before); the district verify now takes 264 s in the background.

## L1 resync to PR #52 head `618e824` (2026-10-17)

The PR moved 86 commits after L1a's `ca647a3` (living world, traffic, retail prop and car collision, and PR #62's menus folded into the branch). Method as L1a (`git merge-file` per copied file, ours / `ca647a3` / `618e824`, the `madd` transform on both upstream sides): 78 files merged cleanly, 11 resolved by hand, 7 test files merged, 5 new files taken.

| Part | What | Edits |
|---|---|---|
| skate-core | living world (peds, NPC tricks and reactions, traffic follow), grab scene, skitching, input, animation; new `animation/raw.rs`, `input/skater_intent.rs`, `living_world/npc_reactions.rs`; `board_world.rs`: upstream's `QueryScratch` (a query's output and retention buffer outside the world, `query_primitives_in`) | our M3 query body (triangle candidates from the per-mesh trees, volumes packed once, profile counters) kept inside upstream's `query_primitives_in`; the PR #62 menu modules (`crossbar`, `fe_*`, `menus`, `menu_values`, `trick_guide`, `video_calibration`, `camera_track`, `rumble`) left for R2 |
| skate-data | collections (`FieldArray::items`), living world, ped animation, scoring; tests | `collections.rs`: the binary form is `COL3` (it carries `items`; the scoring tables read them; `sim.pak` rebaked); `ice_camera`, `scene_clip`, `trick_guide`, menu tables left for R2 |
| skate-sim | 46 physics / camera / grind / input files; new `physics/world_handle.rs` (the static world behind an `Arc` with its own query scratch, for simulated NPC skaters on other threads), `camera/water.rs` | `GamePhysics.world` is a `WorldHandle` beside our collision `window`; `PropCollisionLayer` keeps our templates and slots and takes upstream's sharing (`WorldHandle`, `Arc` instance and template lists, `share`, `world_handle_mut`) and the templates' retail collision shapes (`PropShape`, `load_dmo_collision` through `vfs`, `build_prop_layer_with`; a shape's native edge codes, no base-plane edits); `network.rs`: `Proxies` takes `actors`, `dynamics_before`, `dynamics_deltas`, `ProxyMark`, `mark`, `truncate`, `capture_dynamics_reactions` (`append`, `append_solid`: multiplayer / Rapier, not copied); `solve.rs`: `prop_query`, our spans (upstream's PC hitch phases and mod hooks left out as in L1a); `frame.rs`: the NPC solve phase (`frame_timing` stand-in `PHASE_SOLVE_NPC`); `capture_context_body` (`skate_net`) not copied; Bevy logging → `tracing` |
| vita-bake | `props`: the sidecars keep `collision` | `props.pak` from the new setup export (`prepared-618`) |

Reference: built from `618e824` (`reference.sh`; the scratch copy's `splash.rs`, a Win32 splash that links user32 / gdi32 / kernel32 even in test builds, replaced by no-ops to link on Linux): the 20 Course dumps equal the previous reference's; the port (mul_add) 20/20 against it, no-fma 20/20 against its baseline. Tests: skate-core 1053 passed (the old known failure fixed upstream), skate-data all, skate-sim 285 of 286 (`embedded_static_rwcm_hits_distinct_actor_query_ids`, as before). Setup export: the PR's `tools/prepare_assets.py` at `618e824` on the user's disc (`prepared-618`; menus and ocean need the XEX tools, not run): retail collision shapes for the props (BlackBoxPark 12/12, DownTown 714/786) and the cars (`vehicles.json` `collision`).

## L1c: traffic (PR #52 `618e824`)

| Part | What | Edits |
|---|---|---|
| skate-sim `dynamics.rs` | the solid boundary of `crates/skate-dynamics/src/solid.rs`: `SolidCollider`, `SolidBody`, `SolidContact`, `SolidBody::contact`, `valid_contact` (verbatim) | `parry3d` 0.30.2 directly (the parry rapier 0.35.3 re-exports) instead of `rapier3d`; a `rapier3d` module gives the copied code its upstream paths (`rapier3d::{prelude, parry}`); the Rapier world (mod bodies, definitions, deformation) not copied. Replaces the uninhabited `SolidBody` stand-in |
| skate-sim `physics/solid_contacts.rs`, `physics/offboard/mod_solid_ground.rs` | upstream's files (parry contacts of the native volumes against a solid; the walking constraint; vehicle edges, probe and classification) replace the empty stand-ins | `skate_dynamics::` → `crate::dynamics::`, Bevy math → glam, the madd transform; the walking-constraint tests build their box solid directly (no Rapier world) |
| skate-sim `physics/network.rs`, `solve.rs`, `skeleton_feedback.rs` | `Proxies::append_solid` (verbatim); the solve's two `solid_contacts::append` calls; `solid.id` | — |
| skate-sim `physics/traffic_car_tests.rs` | the traffic-car tests of `physics/skater_context_tests.rs` and their rig | `skate_dynamics::` → `crate::dynamics::`; the rest of that module needs Bevy, `skate_net` or the peds and stays out |

| skate-sim `living_world/vehicles.rs` | skate-game `living_world/vehicles.rs`: the car data (`parse_vehicle_data`, `CarCollision`, grab splines, grind rails), `car_from_record`, `apply_vehicle_records`, `drive_traffic`, `look_ahead`, `proxy_with`, `apply_vehicle_hits`, `push_vehicle_proxies`, `push_vehicle_grab_splines`, `car_rails`, `grab_object_at`, `traffic_readout` (verbatim) | Bevy removed: a car entity is a `CarEntity` (its `TrafficCar` and `CarMotion`) in `TrafficState::index`, systems are functions, messages a `Vec`; reads through `skate_data::vfs`; not taken: the look (Bevy scene, retail car material), the engine / horn / alarm audio, the mod horn patches (`skate_mods`), peds and simulated NPC skaters in the look-ahead and holders (L1d / L1e) |
| skate-sim `living_world.rs` | skate-game `living_world/mod.rs`: `KindSetting`, the settings' `apply`, `PopulationState`, `load_data`, `step_population`, `report_due`, `readout`; `LivingWorldState` runs them in upstream's system order (`before_physics`: `multiplayer::prepare` with no peers, the car proxies and grab splines; `after_physics`: observers, population, the prop obstacles, the car records, hits, drive) | `LivingWorldSettings` keeps the fields the Vita uses (no mod overrides); skaters and peds off by default until L1d / L1e; `load_data` without the NPC skater data (L1d); the prop obstacles (`PedObstacles`, filled by upstream's peds plugin only with the ped navmesh loaded) updated from the props for the cars without that gate |
| skate-sim `physics/live.rs` | `Session::start_living_world` (opt-in: the oracle corpus runs without it), `set_living_settings`, `car_poses`, `living_counts`; the living world's two calls in `step` | ours |
| skate-sim `physics/network.rs` | `prepare`: the single-player body of skate-game's `multiplayer::prepare` | — |
| skate-sim `physics/traffic_car_tests.rs` | `a_skater_latches_onto_a_traffic_car_rides_and_lets_go` | the log subscriber line dropped (no subscriber crate) |
| skate-vita, vita-render (cars on the Vita) | `living.pak` mounted after `props.pak`; the living world started after each load with the app's settings, the car models handed to the main thread; the cars published with each tick's presentation pair and drawn between their two poses (upstream `present_car_pose`); the skinned car draw (the skater's skinning shader; the wheel joints spun about x) with the retail paint rule (`car.frag.cg`, upstream's documented `vehicle_defaultPS` rule) and the glass blended without depth write (`car_glass.frag.cg`); `enable_game_switches` (upstream `LivingWorldPlugin`: prop grab and skitching always on in the game) | ours; the retail car lighting, normal maps and environment cube not drawn (the skater's lighting) |
| skate-vita `settings.rs`, `app.rs`, skate-ui `crossbar.rs` | the Traffic setting (Off, Low 0.2, Medium 0.4, Retail 1.0; saved, version 6), in the crossbar's OPTIONS menu and the plain Settings screen, applied live (`Session::set_living_settings`); upstream's `KindSetting` density, retail's Free Play traffic scale semantics (the census area's cap x density) | ours; default Medium (the Vita's tick budget); in OPTIONS and the plain Settings screen (the SKATE menu movie shows nine options) |
| vita-bake `living.rs`, `main.rs` | `living.pak`: slim `tables.json` and `vehicles.json`, `roads.bin`, the districts' census grids, the spawnable car models as skinned models (`skinned_model`, split out of `bake_skater`; the glass blended) | ours |

| skate-sim `dynamics.rs` (performance, exact) | `SolidBody::contact`: one contact query with the prediction widened by the step's motion bound (`skin + travel`) instead of upstream's at `skin`; within `skin` it is the same contact, beyond `skin + travel` the sweep cannot hit within the step and is skipped | Vita: the car contacts went from 10 ms to 1.6 ms a tick near traffic; both car tests' per-tick deck hashes unchanged (`fold` in `traffic_car_tests.rs`) |
| skate-sim `dynamics.rs` (Vita deviation, the user's choice 2026-10-17) | `SolidBody::contact`'s sweep is parry's linear `cast_shapes` at both bodies' linear velocities instead of `cast_shapes_nonlinear` (which also turns them at their angular velocities) | 3 vs 46 us a sweep on the desktop, about 1.5 ms each on the Vita, 10-15 of them in a tick brushing a car at speed; only a not-yet-touching volume's predicted hit differs (by its turn within the step); touching contacts unchanged. Car knock test: wipeout still at tick 6 |
| skate-core `living_world/traffic/follow.rs` (performance, exact) | `step`: the occupancy lists and snapshot map, built at the top of every car's turn, are built on first use from the same state (the turn works on a copy of its car, written back at its end) | the Vita spent about 1 ms a tick allocating them for 15 cars; DownTown 20 s traffic: per-tick car pose hash identical to the original `step`; traffic tests 56/56 |
| vita-bake `main.rs` | an opaque material's texture is baked with alpha 255 | the cars' body atlas keeps a mask in alpha, which DXT1 made transparent black |

Check: with no solids the dumps are unchanged (parity 20/20 and 20/20); `a_traffic_car_knocks_the_skater_down_above_the_contact_limit` on DownTown: first wipeout at tick 6, upstream's recorded value; the skitch test: latched at tick 3, 99 ticks skitching. `downtown_traffic_spawns_and_drives` (20 s idle at the DownTown spawn): 23 cars spawned, 10 left, 13 live, 12 moving at 6.2 m/s on average.

## L1d: NPC skaters, replay tier (PR #52 `618e824`)

| Part | What | Edits |
|---|---|---|
| skate-sim `living_world/npc_skaters.rs` | skate-game `living_world/npc_skaters.rs`: NPC data, stance, trick profile, the puppet clip and layer rules (`puppet_clip` .. `sequence_attributes`), `NpcStanceTrack`, `root_rotation`, the proxies (`proxy`, `prop_volumes`, `NpcSkaterPropContact`), the pose (`puppet_pose_in_flags`, `stance_tail`, `sequence_part`, ...), `npc_readout` (verbatim); `apply_records`, `advance`, `track_stance`, `push_proxies` | Bevy removed: an NPC entity is an `NpcEntity` in `NpcState::index`, systems are functions, events a `Vec`; `present_pose` runs on the fixed step from the cursor (upstream: every render frame from the cursor one tick back plus the render fraction) and the renderer blends the last two steps' poses like the player's; not taken: the roster look (the stock skater is drawn, upstream's fallback without the roster), the blended fade materials (the Vita's renderer fades), audio, the simulated tier (`npc_sim`), the client mirror, the backwards log; `native_animation_style` from `custom_models.rs` (verbatim) |
| skate-sim `living_world/npc_avoid.rs` | skate-game `living_world/npc_avoid.rs`: the NPC obstacle avoider (verbatim per-NPC body) | cars from the traffic state, props from their obstacle boxes, no peds yet (L1e), single player |
| skate-sim `living_world.rs` | upstream `load_data`'s skater part (`skater_profiles.json`, `skater_paths/<District>.bin`, `npc_voices`, `npc_records`), the skater settings (fade, line chain, tricks, avoider, prop contact; the mod override maps kept empty), the skater world in the census; the NPC steps in `after_physics` (`apply_records`, `avoid`, `advance`, `track_stance`, the pose) and their proxies in `before_physics` | the inline `npc_skaters` stand-in module replaced by the file |
| skate-vita, vita-render, skate-ui | the NPC poses published with each tick, a palette each from the player's binding, the skater mesh drawn per NPC (culled as a 2 m sphere; fading NPCs blended, `skin_fade.frag.cg`); the AI Skaters setting (On / Off, settings v7, OPTIONS / plain Settings); the plain menu's rows close up to stay above the hint line | ours |
| skate-sim `npc_skaters.rs` (performance, exact) | `stance_and_pose`: upstream's `track_stance` and `present_pose` share one build of the layers (both build them from the cursor at sub-frame 0 here); `npc_puppet_layers_cached`: each phase's resolved clip remembered per NPC (`puppet_layer_clip` is a function of fixed inputs); the stance step's clip lengths and attributes read once per clip | NPC step 0.046 → 0.028 ms a tick on the desktop (Vita: 2.9 ms before); DownTown 20 s NPC pose and stance hash identical |
| vita-bake `living.rs` | `living.pak` carries `skater_profiles.json` and the districts' `skater_paths` | — |

Check: `downtown_npc_skaters_ride_and_pose` (20 s idle at the DownTown spawn): 3 NPC skaters riding (retail's desired count), all posed; 760 lines, 47,988 nodes, loaded in 126 ms; NPC cost on the desktop 0.045 ms a tick. Parity 20/20 and 20/20.

## L1e: pedestrians, stage 1 (body tier; PR #52 `618e824`)

| Part | What | Edits |
|---|---|---|
| skate-sim `living_world/peds.rs` | skate-game `living_world/peds.rs`: `PedData` (load, navmesh, playable set), `PedLooks`, `Pedestrian`, `PedNavSettings`, the obstacle inputs, `PedBody`, `PedEvent`, the skater / ped contact rules (`skater_hit`, `ped_hit`), `ground`, `PedMind` (verbatim), the pose helpers (`reference_globals`, `ped_bone_basis`, `ped_globals`, `ped_draw_alpha`, `lod_for`), `advance_peds`' step loop (verbatim); `apply_ped_records`, `release_rejected`, `update_ped_obstacles` | Bevy removed: a ped entity is a `PedEntity` in `PedState::index`; data through `skate_data::vfs`; `present_ped_pose` runs on the fixed step for the peds in view (opacity above 0) and the renderer blends the last two steps; not taken in stage 1: the behaviour runtime (`think_peds`; `PedMind` stays at its default, upstream's wandering ped), hand props (two stand-ins with the default values where `advance_peds` reads them), audio, the still / floating / blocked diagnostics |
| skate-sim `living_world/ped_body.rs`, `vehicle_contacts.rs` | peds as solids in the skater solve (`part_proxy`, `bone_world`, `push_ped_proxies`); cars pushing peds (`car_box`, `resolve`, `ped_vehicle_contacts`) | over the living world's ped and traffic state; the body uses the present step's pose when the ped has one |
| skate-sim `living_world.rs`, `vehicles.rs`, `npc_avoid.rs` | the ped steps in upstream's order (records, rejected, obstacles, advance; car contacts after peds and cars; ped bodies before the solve); the peds in the cars' look-ahead and the NPC avoider (verbatim blocks); ped settings (fade, obstacles, car contacts, body) | the prop obstacles now follow `update_ped_obstacles` (navmesh gate), shared with the cars as upstream |
| vita-bake `living.rs`, `main.rs` | `living.pak`: the ped tables, `navmesh.bin`, `PedestrianSkeletonPres.abin`, every ped model (`skinned_model_nodes`: LOD0 and LOD1 in one model, `LOD1:` / `TINT:` material names, textures shared); glTF `extras` read for the retail material type | ours |
| skate-vita, vita-render, skate-ui | ped models uploaded, bound to the ped rig per model (`PedBinding`: joints by name, the followers, identity basis), drawn per LOD with the retail tint rule and the fade (`ped.frag.cg`); the skin palette at 40 joints (peds have 39); the Pedestrians setting (Off / Low / Medium / Retail, default Low; settings v8) in a WORLD crossbar menu with Traffic and AI Skaters | ours |

Memory (the Vita ran out of heap with stage 1 as first ported: upstream decodes every ped clip at load, 38 MB, and keeps the navmesh records beside the mesh): `ClipCache` decodes a clip with the same `PedBank::clip` the first time a ped needs it and drops the clips not asked for in 2 s once 8 MB are held (they decode again to the same frames); the navmesh records are not kept (only a mod's rule change reads them); the ped data loads only while peds are on. DownTown 20 s at retail density: per-tick ped position, heading and pose hash identical to the decode-everything version, decoded clips at most 7.9 MB; ped data 14 MB and navmesh 11.7 MB on the desktop (43.5 and 17.1 before). The Vita's `HEAP` line also counts the small blocks now (`small=`). Device (DownTown, Medium traffic, 3 NPC skaters, Low peds): the heap reached about 260 MB of 304 (fragmented) during the first minute's streaming and a 1 MB block failed; ticks 26 ms. The user's decision (2026-10-17): peds opt-in, Pedestrians Off by default on the Vita (settings v9 resets v8's Low); the peds' classes in their own `ped_tables.json` (every living-world loader parses its tables whole); clip budget 6 MB (ped hash unchanged).

Check: `downtown_peds_walk_and_pose` (20 s idle at the DownTown spawn, retail density): 15 peds on the navmesh, 11 in view and posed, 364 m walked; desktop cost 0.07 ms a tick. Parity 20/20 and 20/20.

## L1e: pedestrians, stage 2a (behaviour runtime; PR #52 `618e824`)

| Part | What | Edits |
|---|---|---|
| skate-sim `living_world/ped_graph.rs`, `ped_mood.rs` | skate-game files verbatim: the ped AI graph on the shared graph runtime (`PedGraph`, operations parsed into `PedOp`); the mood, chase, takedown, sight, sit, starting hand prop, conversation and reaction-set tables | none |
| skate-sim `living_world/ped_plugins.rs` | skate-game file: plugin classes, descriptors, `plugin_odds`, the placed waypoint groups, the map's hotpoint props | reads through `skate_data::vfs`; the peds' classes from `ped_tables.json` (fallback `tables.json`) |
| skate-sim `living_world/ped_hand_props.rs` | the hand prop records and idle-action usages (`HandPropRecord`, `HandPropData`) verbatim | glam instead of Bevy's prelude, `vfs` reads; `sync_hand_props` (ours, stage 2a): every request takes upstream's "no model" path (the ped holds nothing); the held object, its release as a prop body and the proxies come with stage 2b |
| skate-sim `living_world/peds.rs` | `think_peds` verbatim (perception, mood posts and reactions, the AI graph and the plugin graphs on the brain, plugin props and offers, conversations, chase groups and intercept, flee, RunFromHonker, takedowns, speech state), `PedPluginProps`, `PedConversations`, `PedChaseGroups`, `SkaterTakedownRequest`, `plugin_controller_for`, `apply_skater_takedowns`; `PedData`'s behaviour fields and the behaviour part of `load_ped_data` | Bevy removed: the query is the ped index (id order), messages are `PedState` vectors (the ped events read through a cursor, as upstream's message reader), `Local`s are `ThinkLocals`; no speech or tazer sound (no ped audio on the Vita), no still-ped reports, no client role |
| skate-sim `living_world/vehicles.rs` | `TrafficEvent::HonkedAt` sent where upstream sends it (after the car's pose) | the horn sound itself is not ours |
| skate-sim `living_world.rs` | the chain in upstream's order: records, rejected, obstacles, `think_peds`, `sync_hand_props`, `apply_skater_takedowns`, `advance_peds`; `PedBrainSettings` (retail defaults) | the skater's wiping-out bit from its processed input (upstream `gather_observers`) |
| vita-bake `living.rs` | `ped_tables.json` with the behaviour classes; the compiled ped graphs and plugin descriptors (`.stategraph`) under their stock paths; `waypoints.json`, `hand_props.json` | ours |

Check: `downtown_peds_walk_and_pose` (20 s idle at the DownTown spawn): 15 peds, at the end 8 in plugins (seats, bins, conversations), 4 wandering, 2 starting a conversation, 1 greeting; 2 conversations; deterministic (PED_HASH 507ae464bfaea8c2 twice); `think_peds` 0.15 ms a tick on the desktop. TRAFFIC_HASH unchanged.

## L1 resync to PR #52 head `fad5aae` (2026-10-19)

The PR moved 80 commits after `618e824` (living world fixes, the Challenge Map, the Steam launcher, Better Water). Method as before (`git merge-file` per copied file, ours / `618e824` / `fad5aae`, the `madd` transform on both upstream sides): 68 files merged cleanly, the rest resolved by hand; the five living-world files ported without Bevy (`peds.rs`, `ped_body.rs`, `ped_hand_props.rs`, `vehicles.rs`, `npc_skaters.rs`) were merged the same way and their Bevy-side blocks ported by hand.

| Part | What | Edits |
|---|---|---|
| skate-core | living world: peds (brain, mood store evicting by priority, chase and the new `peds/chase_alert.rs`, conversations that passers-by join through the offer scan, plugins with the seats-in-water rule, nav, wander), NPC skaters (`ai_record`, `ai_signals`, `controller_b`, `npc_tricks`), population, replay; `dmo.rs` (`step_kept`: a kept prop scores like a keep placement), `board_world.rs` (`set_rejection_flags`), skeleton body collision feedback and update, grab scene, move object | `dmo.rs`: upstream's live score inside our memoized eviction queue (a placement's live and candidate scores cached apart, compared as upstream's `front` and `best`) |
| skate-sim physics | on foot: props are walker step geometry (`BipedWorld`: map and prop world), the prop collision types (heavy 12 / light 14 / held 13) the walker filters, ped part contacts on foot; grab slots, carry stick sides and turn ease, the skitch grab, the bail clamp on props, kept props near the player, `moved_ids`; the solve's ped part velocities after the solve (`Proxies::ped_after`, `SkaterRuntime::{ped_hits, ped_after}`) | `skate_world.rs`: `set_collision_type` over our template slots (an instance that takes a slot or new triangles has its type written again, `forget_collision_type`; appended slots already have meshes of their own, upstream's `portable_world` splits); `physics.rs`: `biped_world` beside our collision window; `grab_view` (a PC gizmo viewer) a stand-in with `logging()` off; Bevy log macros → `tracing` |
| skate-sim living world | `think_peds` (the seat push gate as a step, seats in water, conversation offers for founders and passers-by, the plugin graph's End on leaving, a held sit avoid, the approach route dropped with the move, tazer clips, refusals for conversation areas, a lost prop's waypoint released, the map-change reset); `advance_peds` (tazer clips; the local skater's ped reaction from the solve's contact with the ped body, the torso's speed after the solve); `ped_body.rs` (retail part material, no friction; capsule inertia, the parts turn in the solve; proxy ids from the ped serial, `proxy_serial`; the bodies stay through a reaction; `reaction_part_speed`); `ped_hand_props.rs` (the record rotation as Rz Ry Rx, the attach locator from `hand_props.json` `attach`, the tazer's clip states); `ped_mood.rs` / `ped_plugins.rs` / `ped_hand_props.rs` read one parsed tables document; `vehicles.rs` (a car drawn one tick behind its current pose, `drawn_prev`); the census keeps moved props (`step_kept`, `moved_ids`); `PedBrainSettings::plugins` | our loader reads the tables once through `skate_data::vfs` (`PedData::read_tables`, `ped_tables.json` first) and passes the parsed document to every reader; `CarPose` publishes the drawn pair; the map-change resets of `sync_hand_props` and `PopulationState::install` (`continue_serials_from`) are not needed (the Vita builds a new living world for each world it loads) |

| Not taken (and why) | Upstream files |
|---|---|
| The Challenge Map and the retail menu changes (R2) | `skate-core/src/challenge_map.rs`, `menus.rs`, `skate-data/src/challenge_map.rs`, `menu_tables.rs`, `skate-game/src/challenge_map*.rs`, `retail_menu*.rs`, `crossbar_host.rs`, `tools/prepare_menu_movies.py` |
| The HUD's central message (ZAP!, Tazed!, BAM!) and the chase alerts over peds: presentation over the PC HUD; the chase messages' choice is in skate-core (`chase_message`) for later | `skate-game/src/hud_central.rs`, `living_world/ped_alerts.rs`, `peds.rs` `post_chase_messages`, `tools/prepare_chase_arrows.py` |
| Loading behind the PC loading screen (the Vita loads the living world with the world) | `living_world/preload.rs`, `mod.rs` early loads, `peds.rs` `early_load_ped_data` |
| NPC skater speech and the skater-bump lines (no NPC audio on the Vita), the ambience read-ahead at map load | `skate-audio/src/player/contacts.rs`, `state.rs`, `world/skater_speech.rs`, `world/skaters.rs`, `skate-game/src/game_audio/*` |
| The simulated NPC tier (not ported), NPC log lines and the roster look | `living_world/npc_sim.rs`, `npc_skaters.rs` |
| PC host: launcher, Steam launcher, setup, SDK docs, mods, Better Water, XInput rumble, trace-all switches and timing diagnostics, the grab visualizer, the APT runtime (`skate-ui` keeps its own) | `app.rs`, `main.rs`, `setup.rs`, `tools/steam-launcher/`, `sdk/`, `skate-mods/`, `modding/`, `water_*.rs`, `input/platform.rs`, `trace_all.rs`, `system_timing.rs`, `physics/grab_view.rs`, `living_world/opt_ins.rs`, `apt_*.rs` |

Reference: built from `fad5aae` (`reference.sh`; in the scratch copy `splash.rs` and `setup.rs` `local_stamp`, which link Windows libraries even in test builds, replaced by no-ops): 19 of the 20 Course dumps equal `618e824`'s (`04_kickflip` differs from tick 1701 upstream); the port (mul_add) 20/20 against it, no-fma 20/20 against its baseline; Fast physics (25 iterations) 20/20 against the reference and its baseline. Tests: skate-core 1074 passed, skate-sim 307 of 308 (`embedded_static_rwcm_hits_distinct_actor_query_ids`, as before). DownTown (20 s idle at the spawn): traffic 13 cars, 12 moving at 6.2 m/s; the skitch test latched at tick 3, 99 ticks skitching; the car knock's first wipeout at tick 6 (both unchanged); 3 NPC skaters posed; 15 peds (4 in plugins, 6 starting conversations, 2 conversations), deterministic (PED_HASH 3c87b0e441ed4c93 twice).

**Performance after the resync (exact).** On the Vita, DownTown's tick went from 16-18 ms to 26-44 ms after this resync. Most of it came from upstream's grab-slot fix (every streamed-in prop is in the grab scene now; before it, ids of 2^24 and up were left out, 2 of DownTown's 786 props): the grab scene was rebuilt from every prop twice a tick, each spline's geometry allocated again and every record checked again (3-17 ms on the Vita).

| Change | Files | Check |
|---|---|---|
| A body's grab-scene splines built once (`set_grab_splines`; the keys, scaled points and approach depend only on the grab slot, the axis scale and the splines) | skate-sim `physics/prop_dynamics.rs` (`grab_scene_splines`) | carry, skitch and car-knock tests unchanged |
| `set_props` / `set_cars` replace their provider's objects in place and check only the new ones, with the same checks in the same order; a spline the registry already held under the same object id (the same `Arc` geometry, descriptor, `word_272` and assembly identities) only needs its frame and object vector finite | skate-sim `physics/offboard/grab_scene.rs` (`replace`, `check`) | as above |
| The collision-type pass runs only when a type can have changed (the held prop, the bodies or their tuning: `collision_type_epoch`; a slot taken or an instance added: `collision_types_stale`); upstream walked every body every tick | skate-sim `physics.rs`, `prop_dynamics.rs`, `skate_world.rs` | DownTown ped, NPC, traffic, skitch and knock hashes unchanged |
| The census asks `PropDynamics::is_moved` for the ids it scores instead of building the moved set over every body each pass | skate-sim `living_world.rs` | as above |

Vita (DownTown, Low traffic and peds, NPC skaters, on foot and on board): grab scene 0.3-0.9 ms a tick (was 2.6-17), tick 19-24 ms (before the last two rows; they took 0.6 and 0.27 ms). The rest of the difference to before the resync is retail behaviour taken with it: ped body parts that turn in the solve (+1 ms), on-foot queries that see the props (+0.5-1 ms).

Known upstream issue (`fad5aae`): with prop grab on (the game's mode), carrying newspaper box 2198011218 by its authored grab records sinks it below the floor, in upstream's engine too (`downtown_dragged_props_rest_on_the_floor` with `set_prop_grab_enabled(true)`); the test runs with the switch off, as upstream's.

## DownTown performance (exact, 2026-10-20)

Every change here keeps every simulated and drawn value: `living_world::perf_bench::downtown_perf_bench` (3000 ticks of DownTown as the Vita plays it: Fast physics, Low traffic and peds, NPC skaters, on foot and on the board; `--features stage-profile,oracle`) hashes every tick's skater dump and every drawn pose (skater, cars, NPC skaters, peds, hand props, props) and gives `1c5bcc5f282bfa7d` before and after each one; the ped, traffic, NPC, skitch and car-knock hashes are unchanged and parity is 20/20 and 20/20. Most of the cost was memory traffic the Vita's caches do not hide: loops over all 786 prop bodies (large records) for the ~50 streamed in, and allocations per query.

| Change | Files |
|---|---|
| `PropDynamics::live`: the streamed-in bodies' indices (ascending), kept by `set_dormant`, `spawn_body`, `remove_body`; the step (stream-in, upright, push, awake list), the contact pass, the grab objects, the streamed poses, the nearest body and the obstacle boxes walk it instead of skipping dormant bodies one by one; `live_rail_bodies` the same for the rail queries | skate-sim `physics/prop_dynamics.rs` |
| `dmo_placement_count` is a count kept at load (the census asked every tick and counted every body) | skate-sim `physics/prop_dynamics.rs` |
| `think_peds`: the ped seats follow their props only when a prop's pose can have changed (`PropDynamics::pose_epoch`, bumped by every pose write: integration, teleport, placement, seating, tuning, bodies added or removed) or a prop is settling (the pass wrote back the same seats for every one of DownTown's 887 plugin props each tick); the streamed-out test per id (`is_dormant`) instead of a set of every dormant body | skate-sim `living_world/peds.rs`, `physics/prop_dynamics.rs` |
| The grab scene's props are rebuilt only when their inputs changed (`PropDynamics::grab_signature`: each grabbable body's grab slot, frame, velocities, position and inertia as bits, and the splines' generation) | skate-sim `physics.rs`, `physics/prop_dynamics.rs` |
| World queries: `QueryIndex::query_into` (the traversal stack an array, the result into a caller's list), the candidate meshes from a per-thread scratch list (`with_candidate_meshes`, a nested query gets its own) and `candidate_mesh_indices_extend` for the walker's ground queries; the same indices in the same order | skate-core `physics/board_world.rs`, `board_world/{broadphase,query_index}.rs`; skate-sim `physics/offboard/ground_query/world.rs` |
| Solid contacts: the skater's volume shapes built on the stack (the same parry shapes `SharedShape` builds) and every shape's bounds once per call (`ShapeSphere`, `SolidBody::contact_near`; `contact` is `contact_near` over the bounds it computed) instead of per pair | skate-sim `dynamics.rs`, `physics/solid_contacts.rs` |

Tools: the bench above (it writes its pads as a scenario with `BENCH_INPUT_OUT`, and prints the stage spans with `BENCH_SPANS=1` through sim-bench's subscriber); `scripts/vita-render-bench.py --park DownTown --features stage-profile` plays such a scenario on the Vita with the living world and the saved settings (`autoplay.rs`: districts start the living world; their dump is not kept) and prints the stage totals.

## R2a: retail menus 2, data side (PR #52 `fad5aae`)

| Part | What | Edits |
|---|---|---|
| skate-core `crossbar.rs`, `fe_blur.rs`, `fe_fade.rs`, `fe_input.rs`, `fe_popup.rs`, `fe_stack.rs`, `menu_values.rs`, `menus.rs`, `menus_tests.rs`, `video_calibration.rs` | the retail front end's pure parts: the menu registry and its value rules, the crossbar model, the screen stack, pad input, popups, fades, the pause blur's parameters, Video Calibration | the `madd` transform (`menu_values.rs`); `rumble.rs` (no rumble on the Vita), `trick_guide.rs` and `camera_track.rs` (the Trick Guide, a later step) not taken |
| skate-data `menu_tables.rs` | the menu tables' records, their JSON form and `to_menu_data` | the data side only: `extract`, `json_from_xex` and the table locators are not copied (they unpack the executable with upstream's `xex` module, which holds the retail key; this repo holds no part of it); a data test reads the user's extracted tables |
| `tools/menu-tables` (ours) | writes `menu-tables.json` from the user's own `default.xex` with the reference checkout's `skate_data::menu_tables::json_from_xex`, built from that checkout (its own workspace, a path dependency on `../skate-3-rust-engine`, the lock at `fad5aae`): upstream setup's `skate3rust --extract-menu-tables` | — |
| vita-bake `ui.rs` | `ui.pak` carries the menu movie set (`private/menu-movies`, without the Challenge Map's world map images) and `private/menu-tables.json` | ours |

User's disc: 47 pause items, 35 settings rows, 13 settings screens, 8 mode menus, 70 FE screens, 8 Trick Guide categories; the movie set is 35 movies (`ui.pak` 18.7 -> 31.6 MB).

## R2b: retail menus 2, the front end on the Vita (PR #52 `fad5aae`)

| Part | What | Edits |
|---|---|---|
| skate-ui `fe/` (`apt_display`, `apt_imports`, `apt_movie`, `apt_scene`, `apt_stage`, `apt_text`, `apt_vm`, `fe_language`, `crossbar_host`, `settings_host`, `popup_host`, `image_renderer_host`) | upstream skate-game's APT runtime and screen hosts, beside this crate's older runtime | `crate::X` -> `crate::fe::X`; no Bevy; mods (`PopupMods`) not taken; stand-ins for the Trick Guide, Challenge Map and prompt-glyph hosts. Memory (exact): one shared `SharedText` (language and fonts, no fonts JSON tree kept), `Font.glyphs` shared; a movie's characters, actions and display states shared by its clones (`Arc`, copy on write at the link / createTextField / setTextFormat writes), display states kept every 8 frames and the frames between replayed. Speed (exact, frame-by-frame draw hash unchanged): object field names interned (`Name`), `Vm::set` in place; clips in a vector by id (`Instances`); `seek` keeps the last frame's list per clip (`cursor`), updates the children map and placements in place, and a settled clip's frame without controls only writes its children's names again (`settled`, cleared by attachMovie / createTextField / removeMovieClip); the enterFrame pass walks the clips once (`born`); the unresolved-import set computed once; `create` reads the character without copying it; function bodies, functions and their constant pools shared, register banks reused. `fe::profile` (feature `fe-profile`): phase timers |
| skate-ui `fe/menu_stage.rs`, `retail_menus.rs`, `retail_menu_movies.rs` | `retail_menu_stage.rs` `setup` / `update` / the draw list without Bevy: the pause menu, Game Settings, popups, Video Calibration | the Trick Guide and the Challenge Map not taken yet; `PAUSE_ROOTS` (the lean set: `read_lean`); FE sound names from `private/menu-sounds.json` (or the manifest's `frontend` field only); no keyboard, no env switches; a stage error stops the front end (the game drops it) |
| skate-vita `fe_menus.rs`, `app.rs` | device test: loads after the other UI loaders, Triangle on the pause menu opens it (waits with a note while it loads); the world behind the pause menu is captured once and drawn as an image (`vita_render::read_rgba`, `Apt::draw_screen`) | ours |
| vita-bake `ui.rs` | `private/menu-sounds.json` (the fe sound names) in `ui.pak` | ours |
| `tools/prepare_menu_screens.py` | the reference's `prepare_menu_movies` with the Vita's extra roots (HUD meters, replay screens) | ours |

DownTown on the Vita, new pause menu open: update 30 -> 4.1 ms, the pause screen 38-45 -> 58-60 fps; the front end's memory on the desktop 49 -> 17.6 MB.

## R2c: the retail front end is the Vita's main and pause menu

| Part | What | Edits |
|---|---|---|
| skate-core `crossbar.rs` | A on a sub-option row reports `SelectSubOption { tab, row, sub }` | ours (NOT RETAIL YET: retail's sub-option select cases, sub_8261BC30 by kind, are not decoded) |
| skate-ui `fe/menu_stage.rs` | `boot_with` (the host edits the menu data, sub-option lists and settings screens before the registry), `Frame::sub_options`, `close`, `set_sub_options`, `closable` (a menu Back and Start do not close) | ours |
| skate-vita `vita_menus.rs` (ours) | the Vita's layout of retail's free skate menu (`FreePlay`): Respawn, Parks / Districts / Training (sub-option lists: the column holds six rows), Main menu, Quit, Replay Editor (instant and saved replays), On Screen Help, Controls, Stick calibration, Game Settings; the career, online, Challenge Map, Trick Guide, music and extras items leave it. A `VitaMain` mode of the same crossbar is the main menu. Game Settings: retail's Difficulty and effects volume drive the Vita's; three screens for the Vita's own settings (controls, world, system); retail rows without a Vita equivalent stay greyed | ours |
| skate-vita `app.rs`, `fe_menus.rs` | the front end opens on the main and pause screens (the plain menus without it); the loading screen at startup (pic0.png and a turning circle) until the world and every UI loader are ready, then straight into the game; the same screen while a world loads; the world behind the main menu is frozen too | ours; the M9 crossbar (`crossbar_ui.rs`, `handle_crossbar`, `crossbar_menus`) removed |

## R2d: the Trick Guide (list) and the menus' button prompts

| Part | What | Edits |
|---|---|---|
| skate-core `trick_guide.rs` | the Trick Guide screen's model: three-level list, picker callbacks, slots, stance remap | verbatim |
| skate-data `trick_guide.rs` | the guide from the skater collections and the menu tables | `Interned` field data (`as_str`); ours: `to_json` / `from_json` (vita-bake writes `private/trick-guide.json` into ui.pak; the Vita does not load the collections a second time) |
| skate-ui `fe/trick_guide_host.rs` | FETrickTutorial and the picker natives | `crate::fe`; the mods' layers (`TrickGuideMods`) not taken |
| skate-ui `fe/menu_stage.rs` | upstream's Trick Guide flow (FE 14: open on black, fade-in count, Back up / close on black, queued screen states); `PAUSE_ROOTS` adds `tricks/trickguide`; button prompts are reported (`prompts`) instead of dropped | the 3D demo scene not yet (R2d step 2); the stance is the default (regular) |
| skate-vita `fe_menus.rs`, `app.rs`, `vita_menus.rs` | Learn > Trick Guide in both menus; the prompts drawn as the Vita's controller glyphs (the replay screens' icons and plates) | ours |
| vita-bake `ui.rs` | `private/trick-guide.json`; the M9 `core_menu` screen is no longer packed | ours |

## R2d step 2: the Trick Guide's 3D demo

| Part | What | Edits |
|---|---|---|
| skate-core `camera_track.rs`, skate-data `ice_camera.rs`, `scene_clip.rs` | the ViewScene camera tracks (`ice_000.bin`) and the one-clip scene banks (`trickguide/<name>1.abin`) | verbatim |
| skate-vita `trick_demo.rs` | skate-game `trick_guide_scene.rs` without Bevy: the highlighted entry's clip after the 60 ms settle, the clip's bones as a simulation snapshot (the skater's binding skins it), the camera from the track fitted to the clip (`place`, dutch roll, lens) or the side view | the set stays at the origin and the skater is posed in its space (upstream parents the set to the player root: the same picture); the set's sky, sun and fog not drawn; a clip that fails is not read again while highlighted |
| vita-render `scene.rs` | a demo slot (`load_demo`, `draw_demo`: the set and the skater only); the park's buffer setup shared (`park_gpu`, `delete_park`) | ours |
| vita-bake `map.rs` (`trickguide`) | `trickguide.pak` and its `.idx`: the set as a map pack, the camera tracks, the 312 clips (29 MB, read from the card one at a time) | ours |
| skate-vita `app.rs`, skate-ui `menu_stage.rs` | the demo replaces the frozen world while the guide is open (`trick_guide_demo`); the pack is mounted during the loading screen | ours |

## R2e: the Challenge Map

| Part | What | Edits |
|---|---|---|
| skate-core `challenge_map.rs` | the map screen's model (tree, tabs, back, picker, markers) | verbatim |
| skate-data `challenge_map.rs` | the tree, pictures and markers from the collections | `Interned` field data; ours: `Baked` (`bake`: skate-game `retail_menus::challenge_map` with the teleports of `teleports.json`; `to_json` / `from_json`: vita-bake writes `private/challenge-map.json` into ui.pak) |
| skate-ui `fe/challenge_map_host.rs` | the map screen's natives | `crate::fe` |
| skate-ui `fe/menu_stage.rs` | upstream's FE 17 flow (open on the `progression` root, B / tab keys, close at the root, the marker quads, a picked location through the registry: `teleport:<key>` closes the menus and is reported); `register_teleports` (only the locations whose world the host has), `teleport` | `PAUSE_ROOTS` adds `map/challengemap` |
| skate-sim `live.rs` | `Session::teleport` (`travel_to` a locator) | ours |
| skate-vita `app.rs`, `lab.rs`, `fe_menus.rs`, `vita_menus.rs` | Single Player > Challenge Map in both menus; a location teleports here or loads its world first (the request waits for the load); named images (world maps, marker icons) upload on first draw and go when the menus close | ours |
| vita-bake `ui.rs` | `private/challenge-map.json`; the world map images (14 MB) stay in ui.pak on the card | ours |

## Resync to PR #52 head `b110bab`, step 1: simulation and living world (2026-10-21)

The PR moved 36 commits after `fad5aae` (ped fixes, ped body A, skate.Park layouts, the Challenge Map's unlocks and skate.Park folder, Instant Replay, DLC). Method as before (`git merge-file` per copied file, ours / `fad5aae` / `b110bab`, the `madd` transform on both upstream sides): 40 files merged cleanly, 17 resolved by hand (upstream's Bevy systems kept out; their logic ported into our Bevy-free functions). The front-end runtime and Instant Replay are step 2.

| Part | What | Edits |
|---|---|---|
| skate-core | living world: the stored contact reaction during a takedown attempt (b326), knockdown re-hits (b372), recovery intervals, obstacle snap-out, chase reachability (`reach_failed`, `apply_reach_report`), wander re-path to the same target, NPC flip latch, `controller_b` start snap radius; new `living_world/ai_push.rs` (used by upstream's simulated NPC tier only); wipeout side-on mid-trick factor (b448); grab-scene registry without the O(n²) id scan (b334) | `ai_push.rs` with `madd` |
| skate-data | `challenge_map.rs` (the World field filter, `LocationUnlocks`, the skate.Park folder, leaves sorted by label text, b447-b458), `ped_anim.rs` (recovery intervals, ground restart times), `skate_map.rs` (declared-size check) | `bake` takes `LocationUnlocks` and sorts by the `label_key` / `name` of `teleports.json`; the DLC rows (`Collections::merge_dlc`) not taken |
| skate-sim physics | ped body A in the prop step (`ped_prop_volumes`, `actor_push_mass`), the trajectory selector reset after placement (b310), `Proxies::clone_from` reusing buffers (b333), the wipeout reasons' ped force, Better Object Mover (generated grab splines; off) | `prop_dynamics.rs`: our grab-scene spline cache and epochs kept, rebuilt when Better Object Mover generates splines (no Vita setting for it yet, upstream's toggle is a PC overlay system) |
| skate-sim park layouts | new `park_layout.rs`: a world row's skate.Park layout (`attach`, `layout_of`), its pieces' posed collision appended to the static world and their rails to the rail provider (`with_pieces`); `skate_world.rs` `posed_collision`, `prop_files`, the sidecar loaders by path | Bevy-free: no `ParkPiece` entities or draw inputs (`load_draws`); files through `skate_data::vfs`, a layout's package may be a `prop_package`; `posed_collision` returns the ranges only (our prop layer is built by `prop_layer_unbaked`); not with a collision window (streamed district). Nothing on the Vita attaches a layout yet and vita-bake does not bake `private/park-layouts`, so the parks stay empty for now |
| skate-sim living world | `peds.rs` (the deferred reaction played in `advance_peds`, contacts during an attempt stored, knocked-down re-hits, a plugin / taunt clip interrupted by a hit (b371), walking out of a prop footprint, slides applied however short (b311 / b331), stuck peds re-path, the render ground probe at 1.6 m with a ceiling-limited retry (b359), seat carry (b219), conversation areas kept until emptied (b348), mood log limiter, intercept destination and the reach check every 10 updates); `ped_body.rs` (body A, b342) | ported into our functions; `ped_body.rs` body A from `peds.index`, sorted by id as upstream |

| Not taken (and why) | Upstream files |
|---|---|
| Instant Replay and its recorder hooks (`replay::retire`, `keep_model`, `world_live`): step 2, compared first with our replay editor | `skate-core/src/replay_buffer*`, `replay_edit*`, `skate-game/src/replay/*`, `replay_menu_host.rs` |
| DLC packs (rows, maps, unlock files) | `collections.rs` `merge_dlc`, `tools/asset_pipeline/dlc.py`, `stfs.py` |
| Extras / cheats (the zombie cheat), offline profile, text entry and viewer, licenses screen | `cheats.rs`, `extras.rs`, `offline_profile.rs`, `text_entry.rs`, `text_viewer.rs`, `*_host.rs` |
| The teleporter prompt (D-pad Up) | `teleporters.rs`, `controllers.rs` `dpad_up_pressed` |
| The simulated NPC tier (not ported) and its logs | `npc_sim.rs`, `npc_skaters.rs` `log_backwards` |
| Ped diagnostics (`PED_BLOCKED` blocker names, `PED_FLOATING`), the despawn distance log | `peds.rs` |

Reference: built from `b110bab` (`reference.sh`; in the scratch copy the `#[link(name = "kernel32" / "user32" / "gdi32")]` attributes removed and the 16 Win32 functions the test binary then lacks defined as no-op stubs, `win32_stubs.rs`): its 20 Course dumps are bit-identical to `fad5aae`'s, so `oracle/ref` is unchanged; the port (mul_add) 20/20, no-fma 20/20 against its baseline. Tests: skate-core 1144 passed, skate-data passed, skate-sim 316 of 317 (`embedded_static_rwcm_hits_distinct_actor_query_ids`, as before). DownTown perf bench: `PERF_HASH 53abc53d9efa40c7` (was `1c5bcc5f282bfa7d`: the ped behaviour changed upstream), the same timings. Vita: DownTown peds checked by the user.

## Resync to PR #52 head `b110bab`, step 2a: the world in instant replays, front-end runtime fixes (2026-10-22)

| Part | What | Edits |
|---|---|---|
| skate-core `replay_buffer.rs` (+ tests) | the instant replay recording of every drawn actor: one key track per actor id, local joint rotations as i16 quaternions, a key only when the quantised pose changed, retail's 5400-frame window, a byte budget | verbatim |
| skate-vita `world_replay.rs` | upstream `replay/world.rs` without Bevy: the sim thread records the skater, NPC skaters, peds, traffic cars (the wheel angle as the one joint's rotation), streamed-in props and released hand props after each tick (outside the shared lock); a replay draws them from the recording at its time, at full opacity and LOD 0 | ours: the byte budget is 24 MB (upstream 128 MB); bones arrive as model-space matrices, so each joint's local is its parent's (affine) inverse times its own; what the poses do not carry (models, tints, paint, mesh ranges) is each actor's latest; a held hand prop shows the latest one held (NOT RETAIL YET); a replay loaded from the card has no world |
| skate-vita `session_replay.rs`, `app.rs`, `lab.rs`; skate-sim `live.rs` | P1 frames carry their session tick (`world_time`); `Session::bone_parents`; `PropPose::id` | ours |
| skate-ui `fe/apt_movie.rs` | registerClass: the constructor runs after the clip's children are placed (b452: a child's construct value no longer overwrites what the parent's constructor set, e.g. a button's type) | verbatim logic; the menu draw hash (`pause_stage_draw_hash`) is now `4e61a07dab07242d` |
| skate-ui `fe/apt_vm.rs` | `UPDATE_BUDGET` 2,000,000 instructions (was 100,000) | verbatim |

Vita (DownTown, traffic, peds and AI skaters): the full 90 s window in 19.5 MB with 28-57 actors (a busier minute, 60 actors: ~0.3 MB/s, about 75 s in 24 MB); recording 0.8-1.4 ms a tick on the sim thread; playback checked by the user. Tests: skate-core `replay_buffer` 7, skate-vita 26 (`world_replay` locals compose back to the globals), skate-ui 38.

**Step 2a, part 1 (2026-10-22).** Released hand props are recorded by their prop body id (`HandPropPose::id`; the DownTown perf hash, which prints the hand prop poses, is now `61cf3defaaa72803`, the simulation unchanged); the looks (models, tints, the held hand prop) are a history of changes, so a replay shows what a ped held at its time; the bones' inverses are affine. The replay cameras take retail's 40 m leash (`replay_camera::leash`, `82682CB0`, b381 / b449); retail's swept-sphere camera collision (`82682D80`) is not ported (a district streams its collision around the live skater, not the replay's). The living world needs no freeze for the replay (upstream's `world_live`): the simulation is paused while a replay is open. Vita DownTown (about 60 actors): recording 1.05-1.44 ms a tick, worst tick 1.9 ms, 18.8 MB for 60 s; checked by the user.

## Resync to PR #52 head `b110bab`, step 2b: the retail replay screen on the menu stage (2026-10-22)

| Part | What | Edits |
|---|---|---|
| skate-ui `fe/replay_menu_host.rs` | the instant replay screen's natives (FE 12 `replay2/replay_editor2`, `FEReplayMenu.*`) from a per-frame feed, the screen states posted on its edges (play / stop, panel, the editor modes b386, effects rows and values, in / out heads in 30 fps frames, the cursor), the length text, the help's page turns; the timeline strip (`CustomTimeline`: bands per camera, key quads, b403) | without the Create pack's screen (`replay_editor3`, channels, tints); ours from the P1 editor (device-tuned on the same movie): the over-marker states 1006 / 1007 / 1009, the zoom text and its refresh (11), the help pages per camera mode, a camera edit returning to the effects menu |
| skate-ui `fe/menu_stage.rs`, `fe/apt_scene.rs`, `fe/crossbar_host.rs`, `fe/prompt_glyphs.rs`, `fe/apt_stage.rs` | FE 12 opened with the replay, shown after its IntroComplete, fed and closed with it; FE 13 (help) over it; the `CustomTimeline` placeholder and its expansion; `FEReplayMenu` natives; the replay prompt names (`_hud`, `_UpDown`, `_Arrows`) | `PAUSE_ROOTS` adds the two replay movies; `Stage::draw_levels` (ours): while the help is shown only its prompts go to the host (the Vita's glyphs are drawn over every movie) |
| skate-vita `session_replay.rs` (`screen_feed`), `fe_menus.rs` (`replay`), `app.rs` | the replay's feed from P1's playback; the retail screen takes the replay when the set has it (P1's own screens otherwise); the help by page turns; P1's Finished popup stays for now | ours |

The DownTown chase: upstream's intercept (b306) sets a destination every think, so a chasing ped plans a path every tick. `NavMesh::find_path` (skate-core `peds/nav.rs`) allocated and filled three arrays the size of the mesh per search (DownTown: 36,443 polygons); they are now kept per thread and stamped with the search number (an unstamped polygon reads as the fresh values): the same search and path (`downtown_path_bench`: `PATH_HASH` unchanged; a search within 25 m 22.4 → 8.1 µs on the desktop), the DownTown perf hash unchanged. Vita (DownTown, chases): peds 1.4-1.8 ms a tick; checked by the user. The replay runs at 36-38 fps (P1's screens: about 30).

## Resync to PR #52 head `b110bab`, step 2c: the replay's Finished menu on the menu stage, menu fixes (2026-10-22)

| Part | What | Edits |
|---|---|---|
| skate-ui `fe/menu_stage.rs` | the game's popups (`push_popup`, `remove_popups`, `popup_shown`, choices in `Frame::popup_choices`), as upstream's engine popup channel for the replay's Finished menu; while a popup is shown only its prompts go to the host; `track_pads` (the press tracking follows the pads without keys, so the press that raised a popup is not taken by it: upstream's fix of 2026-10-20 for the same instant close) | ours: the popup movie's B answers -1 (retail's cancel constant is 255), both read as a cancel |
| skate-ui `fe/crossbar_host.rs` | the crossbar's sub-option column (a picker) asks `Game.OnMenuGetItemEnabled` per row and greyed every row nobody answered: no highlight in Parks, Districts, the Replay Editor list or On-screen help since R2c. The crossbar answers enabled while the column is open | NOT RETAIL YET: the crossbar's own handler is not decoded (every row enabled, as the popup host's) |
| skate-vita `fe_menus.rs`, `app.rs` | the Finished menu (Upload Video, Save Replay, Exit; B closes) as the stage's popup with the pad; the retail replay screen closes in the frame the replay ends (it stayed drawn over the pause menu after a saved replay); the old Vita pause menu draws only when the retail menus are off or failed (it showed over the retail one during the hand-over and in the frozen world) | ours |

Tests: `replay_tests` (the Finished popup: Down + A picks Save, B cancels, only its prompts; Exit then the pause menu opens again), `column_tests::a_columns_highlight_moves_with_down` (fails without the crossbar answer); the pause menu draw hash unchanged (`4e61a07dab07242d`). Checked on the Vita by the user.

## Resync to PR #52 head `b110bab`, step 2d: the editor's speed and shake rules (2026-10-22)

| Part | What | Edits |
|---|---|---|
| skate-vita `session_replay.rs` | retail's effects rules (upstream doc 37 "M2 step 8", research b398, b461-b472): the speed in percent, 10..100 in steps of 10 (P1 had 25 % to 400 %); Left / Right and A step live values loaded when the menu opens; B commits them into the clip under the playhead unless a clip marker is under it, and A leaving a camera edit commits too (`82507438`); the encoded length counts each clip's frames inside the In / Out points at its speed (`8250B590`) | ours (user decision 2026-10-22): with no marker yet the commit lands on the whole replay (retail loses a commit on a lane without keys, b465, medium-high confidence); P1's saved speeds 1..5 load as 30 / 50 / 100 % |
| skate-ui `fe/crossbar_host.rs` | `Game.IsDemo` answers false: the effects menu highlights its advanced cameras only when it is false (`UpdateEffectsSelection`); upstream leaves it unanswered too | ours |

Not taken: the look lanes (brightness, contrast, saturation, tint; upstream "M3"): they belong to the Create pack's editor (`replay_editor3`), not the base game's. Tests: `effects_land_on_commit_and_the_length_counts_slow_motion`, `the_full_game_is_not_the_demo`. Checked on the Vita by the user.

## Resync to PR #52 head `b110bab`, step 2e: the replay cameras stay out of walls (2026-10-22)

| Part | What | Edits |
|---|---|---|
| skate-vita `replay_camera.rs` | upstream `replay/camera_collision.rs` (research b449): `constrain` (the leash, a swept 0.13 m sphere from the last accepted position with a slide along the hit plane, a line of sight from the player; a cut pulls the camera in front of an occluder; a hidden player turns a non-Free camera back) and its tests | without Bevy; the sweep is a closure over the session's world |
| skate-sim `live.rs`, `physics.rs`, `physics/world_handle.rs` | `Session::camera_world`: the static collision world's shared geometry with the gameplay camera's fat line (`CameraWorld::sweep`) | ours |
| skate-vita `lab.rs`, `app.rs`, `session_replay.rs` | the paused sim thread lends the world to the replay (`Shared::camera_world`) and takes it back before any tick (a tick changes it in place, `Arc::get_mut`); the app constrains the shown camera each frame (the clip's, the edited one, the instant orbit; not the recorded camera of play, which collided when played); a seek, a new camera or a jump over 1 m is a cut | ours: the line of sight starts 1 m above the skater's root; in a district only the loaded window collides (user decision 2026-10-22: no streaming for the replay) |

Vita: the replay at 38 fps as before; checked by the user.

## Resync to PR #52 head `b110bab`, step 3: one APT runtime (2026-10-22)

The M9 copy of the APT runtime (skate-ui `apt_attach`, `apt_display`, `apt_movie`, `apt_scene`, `apt_text`, `apt_vm`, `trick_names`) is gone; everything runs on `skate_ui::fe` (skate-game's `apt_*` with the Vita's optimizations).

| Part | What | Edits |
|---|---|---|
| skate-ui `hud_runtime.rs` | the trick display's runtime: skate-game `hud_runtime.rs` at `b110bab` (on the same runtime as the front end; AS2 scale / rotation writes, `next_action`) | `crate::fe` paths |
| skate-ui `menu_runtime.rs` (ours, M9) | the linked HUD screens (the manual meter, the trick analyzer) on `fe`: `registerClass` through the movie's linkage table and `classes`, `attachMovie` through `Movie::attach` with the new subtree's constructors inside the call (as `apt_stage`), the queue through `next_action` | ours |
| skate-ui `fe/apt_display.rs` | the M9 linked screens' clip event fields (`key`, `offset`) read as `key_code`, `actions_offset` | serde aliases |
| removed | P1's old replay screens (skate-ui `replay_editor`, `popup`; skate-vita `replay_ui`), the M9 crossbar (`crossbar`), sim-oracle `ui-crossbar` / `ui-popup` / `ui-replay` / `ui-replay-help`; vita-bake packs only the meters' linked screens; README step 4 links only them | — |

Checks against the old runtime (sim-oracle built from the previous commit): `hud-trace` over the 20 scenarios gives the same text changes in the same order (only the VM's object ids differ by one); `ui-meter` and `ui-analyzer` render pixel-identical PNGs; the pause menu draw hash unchanged (`4e61a07dab07242d`). Vita: the HUD builds in 1.26 s as before; checked by the user.

## Resync to PR #52 head `b110bab`, step 4a: the teleporter prompt (2026-10-22)

Menu set v11 (`tools/prepare_menu_screens.py --reference` at `b110bab`: the action overlay, Extras, the Skate Profile, the terms screen) and the disc's teleporters.

| Part | What | Edits |
|---|---|---|
| skate-vita `teleporters.rs` | skate-game `teleporters.rs` at `b110bab` (research b420 / b421 / b423): the loaded map's teleporters armed, the skater's query cylinder against each volume, D-pad Up asks `ID_PAUSEMENU_CONFIRM` (No / Yes) or lists the exits, Yes moves the skater (another map loads first) | the cylinder stands on the root with fixed head / hips heights (1.63 m / 0.98 m, NOT RETAIL YET while crouching or falling); skate.Park layout teleporters not armed (no layouts on the Vita) |
| skate-ui `menu_stage.rs`, `crossbar_host.rs` | the action overlay (`hud2/actionoverlay2`, SignUpHUD) on the popup layer: state 2 (Teleport) with the title from `SignUpPoster.GetTeleportTitle` / `HUDComponents.SignUp_GetChallengeTitle`, state 0 when hidden | ours: prompts off the 1280×720 stage are not reported (the idle prompt sits at y = 864) |
| vita-bake `ui.rs` | `private/teleporters.json` in ui.pak with every volume inline from `maps/<map>.triggers` | 12 of 16 resolve; the DownTown / Industrial skate park pairs have no volume in the install's triggers (same gap upstream), left out |
| skate-ui `replay_menu_host.rs`, skate-vita `session_replay.rs` | fixes: the movie places In / Out from `GetPlayheadPosition` (its `GetCursorOffset` ignores its argument), so during MOVE_START / MOVE_END the playhead answers the head's fraction, and both are placed when the screen opens; the timeline band ends at the Out point; the LB drag cannot cross the heads (the Vita panicked on the playhead clamp) | ours |

Vita: checked by the user (teleporters, the In / Out markers and band, no crash with the heads together).

## Resync to PR #52 head `b110bab`, step 4b: Extras, cheat codes, the licenses viewer (2026-10-23)

| Part | What | Edits |
|---|---|---|
| skate-core `cheats.rs`, `extras.rs`, `text_entry.rs`, `text_viewer.rs` | upstream at `b110bab` (doc 31, research b304 / b305 / b320 / b322) | verbatim |
| skate-ui `fe/extras_host.rs` | skate-game `extras_host.rs`: the Extras natives and the text viewer's `tos` object | no Bevy messages (the stage's frame carries the cheats); no on-screen keyboard state |
| skate-ui `fe/menu_stage.rs` | upstream `retail_menu_stage` Extras (FE 18), text entry (FE 46) and text viewer (FE 3): input, open / close, loaded and intro states, row actions, the cheat matcher | ours: FE 46 asks the game for text (`Frame::text_entry`, `MenuStage::text_entered`): the Vita's system keyboard, as retail opened the Xbox one (user 2026-10-23); the dialogs are standard popups (upstream's small popups show no option and take only B on a cancelable popup: they never closed); the Licenses row is enabled (upstream greys it as WIP) and a line break is its own word (the movie ends a line only on a "\n" word) |
| skate-ui `fe/apt_vm.rs` | ours: `eval("a.b")` resolves the first name on the current target (the clip whose frame actions run) when the scope chain has none, then the members (nucleus_tos finds its line fields so; upstream's VM leaves them undefined) | NOT RETAIL (mechanism) |
| tools `gen_licenses.py`, skate-ui `fe/licenses.txt` | upstream's generator for skate-vita on the Vita target, plus the linked C libraries | ours |
| vita-platform `ime.rs`, vita-render `swap(dialog)` | ours: the IME common dialog, drawn by vitaGL's swap while it runs | ours |
| skate-vita `settings.rs`, skate-sim `living_world.rs` | the zombie cheat (the sim's existing zombie path): session-only flag, peds on whatever the setting, `ped_pool` 8 instead of retail's 31 (user's choice: a Vita cap; the sim is at its budget with them) | NOT RETAIL: the cap; the yellow colour matrix (`colour_matrix_zombiemode`) is not drawn yet |

Checks: Extras test on menu set v11 (cheat on / off, the warning, Licenses lines); pause draw hash `d6aff5817a2f6680` (was `516c36b60e3d81e2`: only the Extras item now enabled). Vita: checked by the user (zombie code, Licenses, HUD and pause menu unchanged).

## Resync to PR #52 head `b110bab`, step 4c: the zombie colour grade (2026-10-23)

| Part | What | Edits |
|---|---|---|
| skate-vita `colour_grade.rs` | skate-game `colour_matrix.rs` at `b110bab` (research b100): the stock `post_fx` record `colour_matrix_zombiemode` from the collections in sim.pak, built with `skate_core::colour_matrix::build` | loaded the first time the cheat is on |
| vita-render `scene.rs` `Grade`, the world shaders | ours: retail applies the grade in its tone pass (b101: near / far rows weighted linearly in view depth between the record's distances); here every world shader applies it last (opaque draws the same; alpha blends of an affine map commute), the weight per vertex from `gl_Position.w`, the sky's clear colour through the far band on the CPU; the HUD and menus stay ungraded | user's choice 2026-10-23 (world shaders, not a post pass) |

Vita: checked by the user (the zombie look on and off, the picture and frame rate unchanged without it).

## Resync to PR #52 head `b110bab`, step 4d: the Skate Profile, the retail pause rows (2026-10-23)

| Part | What | Edits |
|---|---|---|
| skate-core `offline_profile.rs` | upstream at `b110bab` (doc 31, research b308) | verbatim |
| skate-ui `fe/offline_profile_host.rs` | skate-game `offline_profile_host.rs`: the Progression and Stats objects | upstream's `ProfileMods` layers not taken (no mods): the pages stay empty until a career system fills them |
| skate-ui `fe/menu_stage.rs` | upstream `retail_menu_stage` FE 61: input (tab keys, scroll left to the movie, Back), open / close, loaded / intro, the queued states | the movie in `PAUSE_ROOTS` |
| skate-vita `vita_menus.rs` | ours (user 2026-10-23): the pause SinglePlayer tab lists retail's rows (Challenge Map, Resume Career, Free Play, Call Skater greyed, Skate Profile); Respawn, Parks, Districts, Training and Main menu leave the pause menu (the Challenge Map reaches every world, the session marker stands in for Respawn; the start-up menu keeps its lists); Quit ends Options | ours |

Checks: Skate Profile test on menu set v11 (opens on tab 0, the tab key steps, Back closes); pause draw hash `67632849c692e697`. Vita: checked by the user.

## Resync to PR #52 head `74c124a`, step 1: simulation and living world (2026-10-23)

The PR moved 39 commits after `b110bab` (much of it upstream's comment compaction; skate.Park fusing, the teleporter icon, replay editor and menu changes are later steps or later milestones).

| Part | What | Edits |
|---|---|---|
| skate-core `living_world/{ai_controller, avoid, path_follow, population, peds/wander}` and tests, `physics/{mod, contact_reduction, teleport_ground_fit}` | upstream at `74c124a` (our copies were still `b110bab`'s) | verbatim |
| skate-sim `physics/prop_dynamics.rs` | retail's ground contact reduction (`82779EB0`: at most four coplanar world contacts per prop in the row solver); a skater contact on the prop's mesh gets no overlap nudge (riding up a ramp prop does not shove it) | ours already tested the prop's mesh (skate_vita); upstream's props-only world (skate.Park pieces) and `SKATE_PROP_WATCH` not taken |
| skate-sim `living_world/{npc_avoid, peds, npc_skaters}.rs` | the NPC avoider's right side as retail; a ped's approach route (`set_approach_route`); `sequence_attributes` generic over the name type | ped stall diagnostics not taken (tracing prints nothing on the Vita) |

Reference: built from `74c124a` with `reference.sh` (it links on Linux now, no stubs): its 20 Course dumps are bit-identical to `oracle/ref`; the port 20/20, no-fma 20/20. Tests: skate-core 1185 passed, skate-sim as before (`embedded_static_rwcm_hits_distinct_actor_query_ids`). DownTown perf bench: `PERF_HASH 77fc1129ae1e3070` (was `61cf3defaaa72803`), 0.78 ms a tick as before; path bench unchanged. Vita: DownTown 59 fps, sim 19.8 ms (as before); checked by the user.

## Resync to PR #52 head `74c124a`, step 2: the replay key edit rates (2026-10-23)

| Part | What | Edits |
|---|---|---|
| skate-vita `replay_camera.rs` `edit` | retail's key edit rates (upstream `replay/camera_key.rs` `CameraRules` at `74c124a`, b402 / b549): orbit stick x 2.5 degrees capped at 3 a frame, pitch to 85 degrees; distance and free moves the stick clamped to 0.2 m; free turn stick x 5 capped at 1.6 degrees, level moves along the heading, eased height (0.015 / 0.2 / x0.6, within 10 m); look height 0.01 m within 1 m of the aim, the Tripod lift moving the look point only; lens 0.5 mm within 24-75 mm | ours: the stick dead zone, LB x3, the Follow / Tripod distance ranges (40 m leash) |

Not taken from the other replay commits after `b110bab`: the film pack's Finished menu (the Create pack screen is not taken), the Game key / B / Save to PC rules (ours already behave so, no PC export), the Tripod key orbit (ours already orbits). Vita: checked by the user.

## Resync to PR #52 head `74c124a`, step 3: teleports and the teleporter icons (2026-10-23)

| Part | What | Edits |
|---|---|---|
| skate-sim `physics/{skater, player_input/mod, player_input/services, teleport_state, player_state, input_phase, climbing/mod}.rs` | upstream at `74c124a` (`git merge-file` ours / `b110bab` / `74c124a`, all clean): one move per teleport through the actor reply, refused while one is in flight (`teleport_pending`); the arrival fitted onto the ground, props and cars (`teleport_ground_fit`, retail `82598428`); an arrival always enters state 100 (on-foot landings through the ordinary selection) | the cars' fit-up is a parry ray cast on each car collider (upstream sweeps through the mod bridge; no mods on the Vita); `TELEPORT_ARRIVAL` printed; upstream's props-plate test case dropped (skate.Park pieces) |
| skate-sim `session_marker/mod.rs` | the marker return as one move, refused while a teleport is in flight | — |
| skate-vita `teleporter_icons.rs`, `teleporters.rs` | skate-game `visual_indicators.rs`: retail's challenge visual indicator over each armed teleporter (b532 to b539): the ribbon from the `teleport` spline and `smallarrow` style, anchor + Offset, billboard about its up, the fade window on view depth (x10 end on park maps), the nearest 22 within 400 m of the player | one ribbon (the disc's teleporters all name `teleport`) |
| vita-render `scene.rs` `load_icons` / `IconInstance`, shaders `icon.{vert,frag}.cg` | retail's two passes (b535 / b536): background alpha blended (rgb x I.y, alpha x sqrt(I.y x fade)), front added (rgb x I.x x fade); depth test, no depth write, both faces; drawn last | in display space as retail's shaders (no linear conversion) |
| vita-bake `ui.rs` | `private/visual-indicators.json` slimmed to the rows the teleporters name, with the `default` colour's two textures; teleporters keep their `indicator` | ours |

Data: upstream setup's `teleporters.py` and `visual_indicators.py` at `74c124a` (they write `indicator` per teleporter and `visual-indicators.json` with its textures); without them the bake leaves the icons out. Not taken: exits that load a skate.Park world (no layouts on the Vita). Checks: parity 20/20 + 20/20, DownTown perf hash unchanged. Vita: icons, Challenge Map and teleporter arrivals (fitted), 57.6 fps; checked by the user.

## Resync to PR #52 head `74c124a`, step 4: the Challenge Map rows (2026-10-23)

| Part | What | Edits |
|---|---|---|
| skate-core `challenge_map.rs`, `menus.rs`, `menus_tests.rs` | upstream at `74c124a`: a folder row's location count, retail's tab rules (`progression_active` / `progression_completed` hidden in free skate), the debug listings behind `DEBUG_MENUS` | verbatim |
| skate-data `challenge_map.rs` (merged), `menu_tables.rs` | `debug_listings`; folders pointing at a `map_category` take its label | ours: `Baked::debug` carries the debug listings into ui.pak (older bakes read as none) |
| skate-ui `fe/challenge_map_host.rs`, `fe/menu_stage.rs` | a folder's label with its count, "Downtown (14)"; the stage's `map_debug` | — |
| skate-vita `vita_menus.rs` tests | the parks pick moves to the main menu (the pause menu reaches the worlds through the Challenge Map) | ours |

Not taken: menu movie set versions 12 (every DLC pack's text) and 13 (the Credits screen, still greyed upstream): the set stays at 11; the cheat entry's popup style (upstream's on-screen keyboard; ours is the system keyboard). Vita: checked by the user.

## Resync to PR #52 head `36018e19`, step 1: simulation and living world (2026-10-24)

The PR moved 6 commits after `74c124a`: Spawn / Call Skater, Free Play options and the HUD minimap (`06f439e0`, later steps), the map switch's ambience read-ahead (`c871c2f0`), camera slow motion (`351f0366`), the NPC body pool (`18bbb37d`), PC setup and docs (`93f11780`, `36018e19`, not taken).

| Part | What | Edits |
|---|---|---|
| skate-core `living_world/{config, mod, population, skaters}.rs` and tests | upstream at `36018e19` (our copies were `74c124a`'s, comments aside): called skaters (`call_skater` / `dismiss_skater`, `Companion`, 3 slots, the 4 m inner radius while one waits, research b544 / b563) | verbatim; the Vita's slot count is set with Call Skater (step 3) |
| skate-sim `physics/{frame, clock}.rs`, `tests/physics_startup.rs` | the camera graph's slow-motion requests are dropped, not applied (retail does not slow down on a long air) | upstream's `CAMERA_SLOW_MOTION` log lines not taken |

Not taken: the NPC body pool fix (`18bbb37d`: we have no simulated NPC tier, `npc_sim` holds only its helpers); the map switch's ambience read-ahead (`c871c2f0`: our zone beds stream from the card, no bed file is read whole on the game thread); the frame-time overlay move. Reference: built from `36018e19`, its 20 Course dumps bit-identical to `oracle/ref`; the port 20/20, no-fma 20/20. Tests: skate-core 1188 passed, skate-sim as before (`embedded_static_rwcm_hits_distinct_actor_query_ids`; the ignored startup test still fails at the standing kickturn, as on `main`). DownTown perf bench: `PERF_HASH 77fc1129ae1e3070` (unchanged: no skater is called), 0.85 to 0.90 ms a tick against 0.91 for `main` on the same machine the same day.
Vita: DownTown 60 fps median, sim 20.9 ms median (21.1 at `74c124a`); the slow frames in the prop-heavy block are render-side (CPU 30 to 70 ms at up to 921 draws, sim 15 to 25 ms), as before; checked by the user.

## Resync to PR #52 head `36018e19`, step 2: the teleporter Yes (2026-10-24)

Nothing to port: upstream's replay popup handler took every engine popup answer (`06f439e0`); our `fe_menus.rs` already copies only the replay's own answers out of `popup_choices` (`REPLAY_POPUP`) and the teleporter reads its `CONFIRM` answers from the same frame.

## Resync to PR #52 head `36018e19`, step 3a: Call Skater and the Free Play options (2026-10-24)

| Part | What | Edits |
|---|---|---|
| skate-core `spawn_skater.rs`, `free_skate_options.rs` | upstream at `36018e19` (`06f439e0`): FE 62 SpawnSkater's rows, picks and keys (b543 / b544), FE 34's Traffic / Pedestrians / A.I. Skaters rows (b548 / b564) | verbatim |
| skate-ui `fe/spawn_skater_host.rs`, `fe/free_skate_options_host.rs`, `fe/crossbar_host.rs`, `fe/menu_stage.rs`, `fe/retail_menus.rs` | skate-game `spawn_skater_host.rs`, `free_skate_options_host.rs`, `retail_menu_stage.rs`: the `SkaterSpawner` / `FreeSkateOption` natives, the pause items' handlers and FE 62's mode, FE 62 / FE 34 input, open / close, first states, the limit popup | ours: Call Skater lists the skater profiles' pros with retail's attribute 0xCABC362B1B05CA19 (upstream: living world pool membership), labelled with their `name_id`; Free Play lists only You (no player character swap on the Vita, owner 2026-10-24); the limit popup is a standard popup with its option, and with a cap other than retail's 3 its text is ours (`limit_text`: retail's says "three skaters"); the actions and values are frame output, not Bevy messages; the Vita's cap of 1 caller (`set_callable`) |
| skate-sim `living_world.rs`, `physics/live.rs` | `apply_called_skaters` (skate-game `living_world/mod.rs`): calls and dismisses to the population's companions | ours: `companion_slots` (the Vita's 1, owner 2026-10-24) and the session's `call_skater` / `dismiss_skater` / `called_skaters` |
| skate-vita `settings.rs`, `fe_menus.rs`, `lab.rs`, `app.rs` | — | ours: FE 34 opens on the Traffic / Pedestrians levels' densities and its values go back to the nearest level (a tie to the lower); the calls go to the sim thread |
| skate-ui `fe/retail_menu_movies.rs` `VERSION`, `fe/menu_stage.rs` `PAUSE_ROOTS` | menu set version 14 (upstream `VERSION`): spawnskater and freeskate_options in the lean set | (step 5 brought forward: the screens need the set) |

Data: the menu set re-exported with `tools/prepare_menu_screens.py --reference <36018e19>` (version 14, 46 movies; the version 11 set kept as `menu-movies.v11`); ui.pak rebaked (old ui.pak.pre-36018). Not taken: the greenscreen (no Replay Mode), the preview of the highlighted skater, the called skater's trail (step 3b). Checks: skate-ui test (Call Skater calls, the limit popup at 1, X dismisses; Free Play's X opens FE 34, Right steps Traffic, B writes back); `FE_HASH 3bc34026d85cc590` (was `67632849c692e697`: the version 14 set, and the two rows enabled). The skate-ui test build was broken since `74c124a` step 4 (`map_debug` missing in a test `MenuData`): fixed. Vita: Call Skater calls, the limit popup, X dismisses, Free Play's options; DownTown 60 fps median, sim 22.2 ms median with a called skater part of the run; checked by the user (who asked for the limit text to match the cap).

## Resync to PR #52 head `36018e19`, step 3b: the called skater's trail (2026-10-24)

| Part | What | Edits |
|---|---|---|
| skate-core `indicator_trail.rs` | upstream at `36018e19` (`06f439e0`): the trail's sampling (every 0.05 s, 0.25 m spacing, 71 points, cleared on a jump) and retail's B-spline (b566 / b567) | verbatim |
| skate-vita `called_trails.rs`, `teleporter_icons.rs` | skate-game `visual_indicators.rs` `companion_trails` / `strips`: one trail per called skater (4 at most) on its `Skateboard_Root` + 0.25 m, the `trail_follow` style's strips, fade and passes | ours: the bone from the NPC's published step poses (relative to the root, interpolated); the mesh uploaded only when a point is added; no trail in a replay |
| vita-render `scene.rs` `set_trail` / `TrailInstance` | — | ours: trail slots drawn in world space with the icons' two passes |
| vita-bake `ui.rs` | the trail style row (`trail_follow`, exported as `Hash_8330C6CBB7402726`) kept in the slim `visual-indicators.json` | — |

Data: ui.pak rebaked (ui.pak.pre-36018 is the pack before step 3a). Checks: skate-vita trail mesh / fade test; skate-core trail tests. Vita: the trail follows the called skater's board (bone 25 of 36), DownTown 60 fps median; checked by the user.

## Resync to PR #52 head `36018e19`, step 4: the HUD minimap (2026-10-24)

| Part | What | Edits |
|---|---|---|
| skate-core `minimap.rs` | upstream at `36018e19` (`06f439e0`): when the minimap shows (82665278), the smoothed heading (82665388), the icon placement and fade (8268CB40) and the north pointer's turn (b568 to b571) | verbatim |
| skate-ui `minimap.rs` | skate-game `hud_minimap.rs` and its part of `scoring_hud.rs`: the frame movie's Show and needle clip, the heading at the console's 30 HUD updates a second, the called skaters' `mini_skater` icons and the north pointer quad over the frame | ours: the movie as a linked screen on `menu_runtime` (as the manual meter); the heading tracker without Bevy |
| skate-vita `retail_hud.rs` `Minimap`, `app.rs`, `settings.rs`, `vita_menus.rs` | — | ours: loaded with the HUD meters, the icon textures from the menu set's `texture/north_pointer` / `texture/mini_skater`; the setting (on by default, settings version 10) bound to retail's Minimap row (`ID_GAMESETTINGS_MINIMAP_TOGGLE`) |
| vita-bake `ui.rs` | `hudminimap.linked.json` among the linked screens (its strings trimmed) | — |

Data: `hud2/hudminimap` prepared and linked with `tools/prepare_ui_screen.py` / `tools/link_ui_screen.py` into `ui-screens-fad` (upstream exports it with `prepare_hud.py` at `36018e19`; the install's HUD files are unchanged); ui.pak rebaked. Checks: skate-ui minimap tests (the heading, the needle, the linked screen shows 7 draws and hides); skate-vita settings round trip (version 9 files load with the minimap on). Vita: the minimap shows top right, the pointer turns, the called skater's icon moves, the Game Settings row hides and shows it; DownTown 60 fps median, sim 18.1 ms median; checked by the user.

## Resync to PR #52 head `36018e19`, step 5: menu set version 14 (2026-10-24)

The export, `VERSION` 14, the two `PAUSE_ROOTS` and the ui.pak rebake came in with step 3a (the screens need them). Heap on the Vita (device logs of steps 1 and 4): the menu stage loads 42 movies instead of 40, +1,335 KiB for the stage (20,702 KiB, was 19,367 KiB), textures unchanged (336, 7,819 KiB); the run's peak heap 165.0 MB (167.0 MB in step 1's run). Not taken: version 12 (every DLC pack's text) and 13 (the Credits screen) are in the set but unused.

## Resync to PR #52 head `36018e19`, step 6: on to `0947dea5` (2026-10-24)

The PR moved 5 commits during this resync (`4bfa4d7f`, `81ae7419`, `3dd2ee2d`, `05529b4b`, `0947dea5`); the pin moves to `0947dea5` (reference.sh, setup-references.sh).

| Part | What | Edits |
|---|---|---|
| skate-data `challenge_map.rs` | `map_icons` / `map_icon`: the fe_map_icons records (texture, max distance; b565 / b571) | `git merge-file` ours / `36018e19` / `0947dea5`, clean; the texture is our interned string |
| skate-data `collections.rs` | `FieldArray::text_pairs` (the loading screen's hints) | merged clean; sim.pak's binary form does not carry them (`vita-bake` reads the JSON) |

Not taken: the minimap's groups in add order and its hide messages (one group on the Vita, no mods), the render-side hitch split, the map read-ahead and bank prefetch on the map switch, the map upload in slices (desktop Bevy), FE 62's swap fade (no player swap on the Vita), the setup export speed passes, the docs and skills. Data: the install's `tips_loader/default` hints given their `text_pairs` by upstream's `vlt.convert` at `0947dea5` (a driver patches only that array; the old file kept as `skater-collections.json.pre-tips`). Checks: reference built from `0947dea5`, its 20 Course dumps bit-identical to `oracle/ref`; the port 20/20, no-fma 20/20; skate-data `the_retail_map_icons`.

## Resync to PR #52 head `36018e19`, step 7: the retail loading screen (2026-10-24)

| Part | What | Edits |
|---|---|---|
| skate-core `tips_loader.rs` | upstream at `0947dea5` (`81ae7419`): FE 52's panel pick, line pick, panel cycle (9 s, 0.5 s outro) and the load sequence (black, push, drop the black, black, close, 4 black updates, fade in; b581) | verbatim |
| skate-data `tips_loader.rs` | the `tips_loader/default` record's Coach Frank lines and hints | serialisable (ui.pak's `private/tips-loader.json`) |
| skate-ui `fe/tips_loader_host.rs`, `fe/menu_stage.rs`, `fe/crossbar_host.rs`, `fe/retail_menus.rs` | skate-game `tips_loader_host.rs`, `retail_menu_stage.rs`: the `TipsLoader` natives, FE 52 open / close, the sequence on fader bit 2, a panel each period, no keys while a load runs | ours: the game says when it loads (`set_loading`); `load_covered` for the black under it; tiploader in `PAUSE_ROOTS` |
| skate-vita `fe_menus.rs`, `app.rs` | — | ours: a load that starts while the menus are ready runs the screen (the first world's keeps the picture and the circle: the menus load after it); black under the screen; no HUD over it; play resumes once it is gone |
| vita-bake `ui.rs` | `private/tips-loader.json` from the collections | — |

Data: the menu set re-exported at `0947dea5` (tiploader, still version 14; the `36018e19` set kept as `menu-movies.v14-36018`); ui.pak rebaked. Checks: skate-ui `the_loading_screen_runs_around_a_load` (FE 52 opens, the black drops, 74 draws, a panel cycle, closes and fades back in); skate-data `the_retail_tips` (20 lines, 31 hints); `FE_HASH 3bc34026d85cc590` (unchanged). Vita: DownTown -> University through FE 52 (black, push, up, black, close, fade in), University loaded in 15.2 s (13.1 to 14.5 s in earlier runs); the menu stage +1,143 KiB (21,845 KiB), its textures 8,484 KiB (were 7,819), the run's peak heap 164.8 MB; checked by the user.

## Phase 2, step 2: the skate.Park fuser (2026-10-24)

| Part | What | Edits |
|---|---|---|
| skate-sim `skate_world.rs` | upstream at `0947dea5`: retail's fuser for the pieces (`append_pieces`, `split_unfused`, `props_only_world`, the welded and fused edge features, the plane and mesh-floor passes, `fused_vertex_ranks` with `eastl_quick_sort`, the edge bytes and vertex bits, `portable_parts_fused`), `FuseTag`, `PropShape::fuse`, `load_collision_shapes` reading the fuse tags | taken whole where ours still equalled `b110bab`; `load_collision_shapes` reads through `skate_data::vfs`; `posed_collision` returns each range with its object's id (not upstream's `PropCollisionInstance`) and the fuse tags; upstream's 12 data-free fuser tests in `fuser_tests` (the old tests module stays off) |
| skate-sim `park_layout.rs` | `Pieces::{fuse, unfused, tags}`, `FuseRules` / `FusePlane` and the sidecar's fuse rules, the `IgnoreFuser` cap | `git merge-file` ours / `b110bab` / `0947dea5`; sidecars read through `skate_data::vfs` |
| skate-data `skate_map.rs` | `Collision` is `Clone` (as upstream) | — |

Not taken yet: the props-only world in prop dynamics (`set_props_only_world`; the Fullparks' only IgnoreFuser pieces are 8 bushes in Downtown). Checks: the fuser tests (12); parity 20/20 + 20/20; DownTown `PERF_HASH 77fc1129ae1e3070` (unchanged), 0.78 ms a tick; the survey test fuses the Fullparks to 44,346 / 83,765 / 148,739 / 58,184 triangles in 89 / 209 / 345 / 151 ms on the desktop (so the Vita reads them fused from the bake).

## Phase 2, steps 3 and 4: the Fullpark layouts baked into the park packs (2026-10-24)

| Part | What | Edits |
|---|---|---|
| skate-sim `park_bake.rs` (new) | — | ours: a layout baked as the park map's `PKFU` extension: the fused rows (`skate_world::FusedTriangle`) with one range per piece, and each railed piece's id, template and placement with the templates' splines; decoded at load into the same triangles and posed rails (test `a_baked_layout_builds_the_fused_world`: bit-identical triangles, the same rails) |
| skate-sim `skate_world.rs` | — | ours: `portable_parts_fused` split into `fuse_rows` and `parts_from_rows`, `fused_piece_rows` (what `append_pieces` appends), `LayoutPieces` (loaded or baked) in `collision_world_with` / `retail_collision_world`, `PKFU` accepted by `validate_runtime` |
| skate-sim `physics.rs` | — | ours: a map with a baked layout gets its fused rows in the static world and its rails in the grind world |
| vita-bake `park_layout.rs` (new), `map.rs` | upstream draws each piece with its tint and the layout's projected lightmap (`retail_world.wgsl`, b439 / b440 / b444) | ours: every park places its Fullpark layout (owner); the pieces posed into the park's render (vertices shared within a piece), each tint folded into its material colour, the projected lightmap's coordinates (sun from the park's sky) in the lightmap channel with the layout's lightmap texture; the pieces' draws split on a 32 m grid; the sim map takes the `PKFU` extension before the spawn check |
| skate-data `skate_map.rs` | `Material` is `Clone` | — |

Packs (`vita-bake maps`): MegaPark 13.4 MB (279 draws, 59k triangles, 74k vertices, 149k fused triangles), StartPark 6.5 MB, IndustrialSkatePark 10.9 MB, DownTownSkatePark 11.9 MB; the old packs kept as `map_<park>.pak.pre-phase2`. Checks: parity 20/20 + 20/20 (the Course has no layout); skate-sim as before; every park's spawn holds with its pieces. Heap: MegaPark from University ran out of memory building its world (a 21 MB block: 161k triangles of 132 bytes); the baked rows are read in place from the extension and appended straight into the world (`skate_world::append_rows`, no 19.6 MB copy, no 9 MB row copy). Not yet: the pieces drawn with upstream's dynamic-object lighting (ours: the park shader, lightmap squared), the props-only world (Downtown's 8 bushes), the teleporter exits into skate.Park worlds (step 5).

## Phase 2, step 5: the skate.Park folder's parks on the Challenge Map (2026-10-24)

| Part | What | Edits |
|---|---|---|
| skate-data `challenge_map.rs` `bake` | upstream `retail_menus::challenge_map` (b447): each park world row enters its world through the first destination that names it (`teleports.json` `world`) | ours: those teleports baked under the world row's id |
| skate-ui `fe/menu_stage.rs` `register_teleports` | upstream `register_teleports` over (kind, id) pairs | ours: World rows as well as locations |

The skate.Park tab's five rows (Stadium, PCU Training Full and Basic, Team HQ, Downtown Full) were greyed since `74c124a` (left out with no layouts on the Vita). Every park loads its Fullpark layout (owner), so PCU Training - Basic loads the Fullpark too. ui.pak rebaked (74 teleports). Vita (all four parks through the Challenge Map, the Stadium also from University's teleporter): loads 4.3 to 10.2 s; 60 fps median everywhere; draws median 238 in the Stadium (the bowl stays in view, CPU 11 ms), 46 to 97 elsewhere; heap peak 160.5 MB; checked by the user. Owner (2026-10-24): retail does not offer these parks from the start; they stay open on the Vita until the career (Phase 5) brings their unlocks.

## Resync to PR #52 head `8df32c51`, step 1: simulation and living world (2026-10-26)

The PR moved 47 commits after `0947dea5`; about half are PC-only (setup speed, build stamps, the fixed-step executor, the PC overlay, custom models, mods, docs and skills) and are not taken.

| Part | What | Edits |
|---|---|---|
| skate-sim `camera/graph.rs`, `physics/{frame, clock}.rs`, `tests/physics_startup.rs` | upstream `8bd84c24`: retail's subject gate on the SlowMotionController's Update (no subject, no slower request, no time passes), the rate requests applied again | the test `without_its_subject_the_controller_neither_posts_nor_advances`; our step 1 of `36018e19` (requests dropped) undone |
| skate-core `living_world/{air_steer, ground_steer}.rs`, `living_world/peds/{anim, avoid, brain, mod, plugins, tests}.rs` | upstream at `8df32c51` (ours were `0947dea5`'s, comments aside): NPC air steer and lane rejoin (used by the simulated NPC tier, not ported: no effect on the Vita), seated peds' SkaterAvoidance (`avoid.rs`), the bench water rule at 0.8 m | verbatim |
| skate-sim `living_world/peds.rs` | upstream `46e6266b`: a seated ped flinches or jumps up when a skater comes at it (closest approach within 10 m / 0.75 s / 3 m, the escape point on the navmesh) | the patch applied to ours; the observers are our slice |
| skate-sim `physics/prop_dynamics.rs` | upstream `1db9eb41`: skater mesh contacts as rows in the prop solve (retail b617; the volume a body with its velocity and mass, the board side 8 kg), a closing touch wakes a resting prop, the impulse push only with `skater_rows` off | ours always tests the prop's mesh (no box fallback), so `npc_skater_bench_push_is_deterministic` checks the impulse with the rows off and the rows' determinism; `PROP_PUSHED` diagnostics not taken |

Not taken: NPC Skater Physics and Ped Speech on by default (the simulated tier is not ported; speech waits for Phase 4), the ped material (render), the custom model swap fixes (no player swap on the Vita). Checks: reference built from `8df32c51`, its 20 Course dumps bit-identical to `oracle/ref`; the port 20/20, no-fma 20/20; skate-core 1211 passed, skate-sim as before; DownTown `PERF_HASH bd5e031e28835b47` (was `77fc1129ae1e3070`: the prop rows and the seated peds), 0.76 to 0.79 ms a tick (as before).

## Resync to PR #52 head `8df32c51`, steps 2 to 4: replay cameras, career stats, loading screen, menus (2026-10-26)

| Part | What | Edits |
|---|---|---|
| skate-vita `replay_camera.rs` `constrain` | upstream `e1ef4c59` (`replay/camera_collision.rs`): the camera sweep is one-sided, a surface stops a move only into its front face (a cut from the skater's feet is not pulled down, a camera under the ground can come back up) | the filter wraps our sweep; two tests |
| skate-core `career_stats.rs`, `hall_of_meat.rs` | upstream at `8df32c51` (`86c59e5c`, `a34b0bfe`, `2fde61a8`, `2b427cbf`): the 65-row career table, the push latch, the Hall of Meat bail record, injuries and body tweak poses | verbatim; `pub mod` lines in `lib.rs` |
| skate-core `scoring/catalog.rs`, `fe_fade.rs`, `tips_loader.rs` | upstream at `8df32c51`: the categories the stats read; the FE fade at 60 ticks a second (`8817f3a9`); every open clears "shown before" and the release comes 2 updates after the close (`1e377aad`) | verbatim (ours were `0947dea5`'s, comments aside) |
| skate-sim `scoring_runtime.rs` | upstream's `StatEvent` rows (trick ends with their time and distance, body flips) | three-way merge, clean |
| skate-sim `physics/settings.rs` | upstream's Hall of Meat tuning and BodyTweak poses from the collections | three-way merge; `bevy::log::warn!` → `tracing::warn!` |
| skate-sim `career_stats.rs` (new) | upstream `career_stats_host.rs` `track`, `hits` and `event`: time played, distance on and off the board, skitching, pushes, wipeouts and bail distance, the Hall of Meat rows, ped hits by entity key | Bevy-free and without the file: `Session::start_career_stats` / `changed_career_stats`, run after the living world in `Session::step` (off for the oracle); the ped hits are this step's `PedEvent::Hit` by the player (`LivingWorldState::player_ped_hits`); a map change is a new session, so no transition check |
| skate-data `menu_tables.rs` | upstream's `StatRow` and `career_stats` (serde default) | the struct and field only: the extractor stays in the reference checkout (`tools/menu-tables` builds against it) |
| skate-ui `fe/tips_loader_host.rs`, `fe/menu_stage.rs` | upstream `57ed60e9`, `8817f3a9`: the Stats panel rows (`panel_rows` as `stat_rows`), GetStats shuffled on every call, no SD screen edge offset, the autosave layer's corner wheel while the loading screen is up | the totals and the Units setting come from the game (`MenuStage::set_career_stats`); no mods' lines, holds or timing (`LoadingMods`) |
| skate-ui `fe/menu_stage.rs` `show_popup` | upstream `d64617ce`, `4548d316`: the popup shown log line and the small-popup-with-options warning | log lines in the stage log; our cheat and limit popups were already standard popups |
| skate-vita `career.rs` (new), `lab.rs`, `app.rs`, `fe_menus.rs` | upstream's `settings/career-stats.json` format at `ux0:data/SKATEVITA/career-stats.json` | the main thread loads it at boot and writes it every 30 s when changed and on quit; the sim thread copies the session's totals once a second |
| skate-vita `settings.rs`, `vita_menus.rs` | upstream's Units row enabled (removed from `NO_EFFECT_YET`) | settings version 11 adds the units byte (older files read metric); the row uses retail's Metric / Imperial rule |

Not taken: the loading screen for PC overlay loads, mods' `sdk.loading`, the ped material (render), the replay editor's Follow eye frame and keyed-clip camera pick (our editor's keys differ). Checks: parity 20/20 and no-fma 20/20; DownTown `PERF_HASH bd5e031e28835b47` (unchanged: no stats in the bench); `menu-tables.json` written by the reference's `8df32c51` extractor (65 rows, 47 enabled, as upstream's test) and ui.pak rebaked; the loading screen stage test shows Stats rows on the b110 menu set.

## Optimization phase before Phase 5 (2026-10-26)

Ours, measured with the new streaming benchmark (`skate-vita` feature `stream-bench`: the camera flies a loop through each district at 12 and 24 m/s, each leg without and with the change).

| Part | What |
|---|---|
| skate-vita `district_stream.rs` | tiles are wanted and ranked by the nearer of the camera and where it will be in 1.5 s (at most 40 m ahead): far tiles missing in 9-31% of frames before, 0-0.9% after; queued uploads go with a restarted district |
| skate-sim `collision_window.rs` | a background window is built around where the deck will be in 1.5 s (at most 14 m ahead; the desktop oracle's windows are unchanged) |
| skate-vita `packs.rs` | a pack is hashed again only when it changed (`verified.txt`: path, length, time): 67 s of card reads at every start, and each district's 30-100 s at its loads, beside the streaming |
| skate-vita `alloc.rs` | a thread keeps at most 64 KiB of free blocks per size class (classes up to 512 bytes keep their 128) |
| skate-vita `district_stream.rs`, vita-pack `district.rs` | lean reads: a tile's middle and far levels (side by side in the pack) come in one read and stay while it is near, the near level goes beyond 160 m, levels on their way are not read again, and a tile's textures are read in runs (neighbours within 32 KB, at most 512 KB a read). The card's small reads were slow while the tiles, the collision windows and the audio shared it (holes in front of the camera in Industrial); the streaming benchmark's loader reads at 24 m/s in DownTown: 1088 before, 374 after |
| skate-vita `district_stream.rs`, `retail_audio.rs` | the tile loader runs on core 0 or 2 just below the main thread (pinned to core 2 it starved beside the retail audio render); the retail audio plays at most 16 voices at once (owner; 20 took 5.2 ms of each 5.33 ms block on Industrial's roads) |
| vita-render `district.rs`, skate-vita `district_stream.rs` | beyond 160 m a tile's draws of at most 64 triangles are not submitted (owner; about a fifth of DownTown's draws in its dense streets); the streamer plans (wanted tiles, drops, the loader's view) every 4th frame and drops at most about 1 ms of tile levels a frame. Dense DownTown: main thread 15.8 -> 13.7 ms a frame, stutters 5.5 -> 2.5 a second |
| skate-vita `app.rs`, vita-render `scene.rs` | diagnostics: `MAIN` (the main thread's frame split each second) and `SCENE` (the scene draw's passes every 10 s) log lines |
| skate-vita `app.rs` | diagnostics: the RENDER line counts frames shown more than 20 ms after the last and gives the slowest frame's CPU time and the streamer's part of it |
| skate-vita `lab.rs` | the replay recordings are cleared before the next world loads (a district's 7.4 MB index found no free piece with the last world's replay keys still spread through the heap) |
| skate-data `io_gate.rs`, `vfs.rs`, skate-audio `stream.rs` | diagnostics: card reads counted by source (`CARD` log line every 10 s with the district stream line) |
| skate-core `replay_buffer.rs` | edit to the copied file: a key is allocated at its exact size (the moved translations and scales were not counted, so most skeleton keys doubled), and a key's overhead is counted as 64 bytes. The Vita's instant replay held about twice its 24 MB budget; DownTown ran out of heap in the benchmark |
