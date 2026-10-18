//! Common grind Post using CURRENT collision observations and request owner.
use super::super::wipeout::Observations;
use tracing::info;
use skate_core::{physics::grind_forces::post, player::wipeout::Requests};

/// Request index names of the grind Post (`post::check`): 0 body contact, 1 pose displacement,
/// 2 board closing velocity, 3 deck angle error.
const NAMES: [&str; 4] = ["body_contact", "pose_displacement", "board_closing_velocity", "deck_angle"];

pub(crate) fn check(
    state: &mut Requests,
    settings: post::Settings,
    observations: &Observations<'_>,
) -> Result<(), String> {
    let frame = observations.frame()?;
    let before = state.reasons;
    post::check(state, settings, &frame);
    // GRIND_WIPEOUT: which Post check asked for the wipeout and the values it compared, so a grind
    // that bails at once (prop rails, 2026-10-15) shows its cause in the session log.
    let fired: Vec<&str> = (0..NAMES.len()).filter(|&i| state.reasons[i] && !before[i]).map(|i| NAMES[i]).collect();
    if !fired.is_empty() {
        let e = frame.pose_error;
        info!(
            "GRIND_WIPEOUT fired={fired:?} pose_error={:.3} (max {:.3}) board_contact={} closing_velocity=[{:.2}, {:.2}, {:.2}] (xz max {:.2}) max_body_contact={:.1} max_arm_contact={:.1} max_deck_error={:.3}",
            (e[0] * e[0] + e[1] * e[1] + e[2] * e[2]).sqrt(),
            settings.max_displacement_208,
            frame.board_contact,
            frame.closing_velocity[0],
            frame.closing_velocity[1],
            frame.closing_velocity[2],
            settings.xz_acceleration_204,
            settings.max_body_contact_168,
            settings.max_arm_contact_164,
            settings.max_angular_deck_error_212,
        );
    }
    Ok(())
}
