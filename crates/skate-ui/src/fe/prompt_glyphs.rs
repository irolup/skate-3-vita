//! skate_vita: the part of skate-game `prompt_glyphs.rs` (PR #52 `b110bab`) the movie draw reads: which retail
//! button names have a prompt (`apt_scene` draws a `CustomButton` of a known name as the texture `prompt/<name>`).
//! The glyphs themselves (upstream renders PC pad glyphs) are the Vita's own button icons.

/// The retail button name in `_test` -> face position (South, East, West, North), Xbox 360 naming (verbatim).
pub(crate) fn face_index(retail: &str) -> Option<usize> {
    Some(match retail {
        "A" => 0,
        "B" => 1,
        "X" => 2,
        "Y" => 3,
        _ => return None,
    })
}

/// A stick or d-pad direction suffix (verbatim `direction`, the direction itself not needed here).
fn direction(suffix: &str) -> Option<()> {
    match suffix {
        "" | "_Up" | "_Down" | "_L" | "_Left" | "_R" | "_Right" => Some(()),
        // The replay editor's camera prompts: up / down (height) and the arrows (lens); shown as the plain D-pad.
        "_UpDown" | "_Arrows" => Some(()),
        _ => None,
    }
}

/// A retail button name that has a prompt: upstream's `resolve` with the Xbox 360 naming is `Some`.
pub(crate) fn known(retail: &str) -> bool {
    // `_hud` names the HUD-style art of the same button (the replay screen's Hide prompt asks for `L3_Click_hud`,
    // `EnableShowHide`); the prompt is the same.
    let retail = retail.strip_suffix("_hud").unwrap_or(retail);
    if face_index(retail).is_some() {
        return true;
    }
    for prefix in ["Stick_Left", "Stick_Right", "DPad"] {
        if let Some(rest) = retail.strip_prefix(prefix) {
            return direction(rest).is_some();
        }
    }
    matches!(retail, "L_Bumper" | "R_Bumper" | "L_Trigger" | "R_Trigger" | "L3_Click" | "R3_Click" | "Start" | "Back")
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_replay_screens_names_have_prompts() {
        for name in ["A", "Back", "DPad_Up", "Stick_Right_L", "L3_Click_hud", "DPad_UpDown_hud", "DPad_Arrows_hud", "R_Trigger"] {
            assert!(super::known(name), "{name}");
        }
        assert!(!super::known("Unknown"));
    }
}
