//! Manual session markers (skate_vita): skate-game's `session_marker/mod.rs` without
//! Bevy: a plain struct updated from `live::Session::step` at the reference's place in
//! the frame (after the input is published, before the controls). Automatic bail
//! recovery owns a different checkpoint. Edits against the reference: the Bevy
//! resource/plugin/system wrappers, the replay and vehicle checks (neither exists here)
//! and the map generation (a session is one map; a new map is a new session) are gone;
//! the UI clock (`Time<Real>` in the reference) is the tick period, so it pauses with
//! the simulation; the three native audio ids are returned instead of written as
//! messages. `state.rs` and `noise.rs` are the reference's files unchanged;
//! `validation.rs` only lost its Bevy imports.
mod noise;
mod state;
mod validation;

use crate::input::ControllerInput;
use crate::physics::{GamePhysics, SkaterRuntime};
use glam::Vec3;
use skate_core::physics::{board::BodyId, skeleton_animation_record::AnimationPartTransform};

use noise::Noise;
use validation::Validation;

/// The native noise generator of the return dissolve (noise.rs, unchanged), for the
/// renderer: a 64 x 64 RGBA noise texture, advanced every 1/60 s of UI time while
/// hidden or not, and the scroll and phase weights the shader reads.
pub struct DissolveNoise(Noise);
impl Default for DissolveNoise {
    fn default() -> Self {
        let mut noise = Noise::default();
        // The reference draws the texture once, first, from the same generator.
        let _ = noise.texture();
        Self(noise)
    }
}
impl DissolveNoise {
    /// The 64 x 64 RGBA8 texture: the generator's first output (call before `advance`).
    pub fn texture() -> Vec<u8> {
        Noise::default().texture()
    }
    pub fn advance(&mut self) {
        self.0.advance();
    }
    /// (scroll, first weights, second weights) for the shader.
    pub fn params(&self) -> ([f32; 4], [f32; 4], [f32; 4]) {
        (self.0.scroll, Noise::weights(self.0.phases[0]), Noise::weights(self.0.phases[1]))
    }
}

#[derive(Clone, Copy, Debug)]
struct Marker {
    transform: AnimationPartTransform,
    on_board: bool,
    foot_forward: bool,
}

/// Native GlobalFEPlaySound event ids the reference's audio owners consume.
pub const AUDIO_SET: u64 = 0x0d6c_88a3_b91c_828f;
pub const AUDIO_DENIED: u64 = 0x66b3_afe3_b602_918c;
pub const AUDIO_RETURN: u64 = 0x7f13_5f9f_d28f_7f21;
/// LB pressed: the marker menu opens (retail's cellphone activate sound; skate_vita addition, an audio id
/// the reference sends as `SessionMarkerAction::Opened`).
pub const AUDIO_OPEN: u64 = 0x0d6c_88a3_b91c_8290;

/// What the overlay needs.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MarkerView {
    /// LB is held: the prompt panel shows.
    pub visible: bool,
    pub can_place: bool,
    pub can_return: bool,
    /// The return hold, 0..1 (drives the noise dissolve).
    pub progress: f32,
}

pub(crate) struct SessionMarker {
    validation: Validation,
    marker: Option<Marker>,
    hold: state::Hold,
    pub view: MarkerView,
    blocked_until_release: bool,
    ui_time: f64,
    last_batch: u64,
    /// The modifier (LB) as last seen: its press opens the marker menu (retail's cellphone
    /// activate sound, `ui_audio::SessionMarkerAction::Opened`).
    modifier: bool,
}

impl SessionMarker {
    /// A fresh marker state: blocked until LB is released, as after a map change. The
    /// placement checks read the stock collections (`Validation::load`).
    pub fn new(asset_root: &std::path::Path) -> Result<Self, String> {
        Ok(Self {
            validation: Validation::load(asset_root)?,
            marker: None,
            hold: state::Hold::default(),
            view: MarkerView::default(),
            blocked_until_release: true,
            ui_time: 0.,
            last_batch: 0,
            modifier: false,
        })
    }

    /// `suspend` of the reference when gameplay is not active (a menu): the hold is
    /// cancelled and nothing happens until LB is released.
    pub fn suspend(&mut self) {
        self.hold.cancel();
        self.view.visible = false;
        self.view.progress = 0.;
        self.blocked_until_release = true;
        self.ui_time = 0.;
    }

    /// One fixed tick of `period` seconds. Returns the native audio ids to play.
    pub fn update(
        &mut self,
        period: f64,
        input: &ControllerInput,
        physics: &GamePhysics,
        skater: &mut SkaterRuntime,
    ) -> Vec<u64> {
        let mut audio = Vec::new();
        // PlayerUI runs on the UI clock, independently of physics substeps.
        if !self.blocked_until_release {
            self.ui_time += period;
        }
        let (modifier, set, held) = input.session_marker_actions();
        let pressed = modifier && !self.modifier;
        self.modifier = modifier;
        if self.blocked_until_release {
            if !modifier {
                self.blocked_until_release = false;
            }
            return audio;
        }
        // The cellphone UI (sub_826682B0): LB opens the menu with its activate sound.
        if pressed {
            audio.push(AUDIO_OPEN);
        }
        let p = &skater.player_input.physical;
        let processed = &skater.player_input.processed;
        let on_board = p.state.category_12 != 500;
        let deck = physics.board.part_transforms()[BodyId::Deck.index()];
        let mut transform = skater.animated_skeleton.roots.animation_to_world;
        if on_board {
            for i in 0..3 {
                transform[i][..3].copy_from_slice(&deck.basis.columns[i]);
            }
            transform[3] = [
                deck.translation.x,
                deck.translation.y + 0.2,
                deck.translation.z,
                0.,
            ];
        }
        transform = crate::physics::facing_from_visual(
            transform,
            processed.flags_2468,
            processed.flags_2476,
            on_board,
        );
        let state = p.state.state_16;
        let state_allowed = (p.state.category_12 == 100
            && p.collision.wheel_count_0 >= 2
            && state != 104
            && deck.basis.columns[1][1] > 0.71)
            || (p.state.category_12 == 500 && state == 500);
        self.view.can_place = modifier
            && state_allowed
            && p.surface_default_mode != 8
            && self.validation.check(physics.world(), transform[3]);
        self.view.can_return = self.marker.is_some() && !matches!(state, 104 | 502);
        self.view.visible = modifier;
        // A retained Pad publication must not turn one .pressed into repeated sets.
        if set && self.last_batch != input.consumed_batches {
            if self.view.can_place {
                self.marker = Some(Marker {
                    transform,
                    on_board,
                    foot_forward: skater.animation.foot_forward(),
                });
                audio.push(AUDIO_SET);
            } else {
                audio.push(AUDIO_DENIED);
            }
        }
        self.last_batch = input.consumed_batches;
        let ready = p.state.flag_69 == 0
            && state != 702
            && !(physics.board_wiping_out && p.skeleton.teleport_pending_604 != 0)
            && !skater.teleport_pending();
        let distance = self.marker.map_or(0., |m| {
            // 82DB6EC0 -> 82BE1AE8 publishes animation-to-world at output+368;
            // UpdateSessionMarker reads its translation at output+416.
            Vec3::from_slice(&skater.animated_skeleton.roots.animation_to_world[3][..3])
                .distance(Vec3::from_slice(&m.transform[3][..3]))
        });
        let usable = self.view.can_return;
        while self.ui_time >= 1. / 60. {
            self.ui_time -= 1. / 60.;
            let step = self.hold.update(held, usable, distance, ready);
            self.view.progress = step.progress;
            if step.relocate {
                if let Some(target) = self.marker {
                    // One move through the actor reply, as retail's marker return (b473; upstream `74c124a`); refused
                    // while one is in flight.
                    if skater.teleport_pending() {
                        tracing::warn!("Session marker return rejected: a teleport is in flight");
                        self.hold.cancel();
                    } else {
                        skater.animation.restore_foot_forward(target.foot_forward);
                        skater.teleport_state.request_manual(target.transform, target.on_board);
                        audio.push(AUDIO_RETURN);
                    }
                }
            }
        }
        audio
    }
}
