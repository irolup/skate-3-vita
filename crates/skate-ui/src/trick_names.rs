//! Trick name localization, from skate-game `scoring_hud.rs` (`b3c9679`):
//! `localize_trick` and `humanize_trick_id`, unchanged. Since b3c9679 the scorer composes
//! a trick's name from several localization ids (`ID_TRICK_FLIP_KICKFLIP 360`), so each
//! part is localized before the HUD movie receives the literal.

/// 825E51A0 localizes each authored component before composing a literal.
pub fn localize_trick(label: &str, assets: Option<&crate::apt_text::TextAssets>) -> String {
    if let Some(literal) = label.strip_prefix('#') {
        return literal.to_owned();
    }
    label
        .split_whitespace()
        .map(|part| {
            let text = assets
                .map(|a| a.localize(part))
                .unwrap_or_else(|| part.to_owned());
            if text.starts_with("ID_") {
                humanize_trick_id(&text)
            } else {
                text
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn humanize_trick_id(id: &str) -> String {
    let rest = id
        .strip_prefix("ID_TRICK_")
        .or_else(|| id.strip_prefix("ID_"))
        .unwrap_or(id);
    rest.split('_')
        .filter(|word| !word.is_empty())
        .map(|word| {
            let lower = word.to_ascii_lowercase();
            let mut chars = lower.chars();
            match chars.next() {
                None => String::new(),
                Some(first) => first.to_ascii_uppercase().to_string() + chars.as_str(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}
