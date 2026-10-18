//! The session marker's sounds: `skate_sim`'s audio ids → the `fe` records retail asks for there
//! (the reference's `ui_audio::SessionMarkerSounds` defaults: opened `cellphone_activate`, placed
//! `cellphone_place_marker`, refused `cellphone_marker_error`, returned `cellphone_goto_marker`).
use skate_audio::frontend::{key, marker};

/// The front-end record key for a session-marker audio id (`Session::step`'s `marker_audio`).
pub fn marker_record(id: u64) -> Option<u64> {
    Some(key(match id {
        skate_sim::AUDIO_OPEN => marker::ACTIVATE,
        skate_sim::AUDIO_SET => marker::PLACE,
        skate_sim::AUDIO_DENIED => marker::ERROR,
        skate_sim::AUDIO_RETURN => marker::GOTO,
        _ => return None,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_marker_id_has_its_retail_record() {
        assert_eq!(marker_record(skate_sim::AUDIO_OPEN), Some(key("cellphone_activate")));
        assert_eq!(marker_record(skate_sim::AUDIO_SET), Some(key("cellphone_place_marker")));
        assert_eq!(marker_record(skate_sim::AUDIO_DENIED), Some(key("cellphone_marker_error")));
        assert_eq!(marker_record(skate_sim::AUDIO_RETURN), Some(key("cellphone_goto_marker")));
        assert_eq!(marker_record(0), None);
    }
}
