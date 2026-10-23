//! The popup movies' C++ side (doc 31 "Popups"; research `.local/research/menus-m6/fe-popup.md`): the AS `Popup`
//! natives (registrar `sub_825A9F40`) answer from the shown request, and while a popup is up it is the focus screen
//! for `Game.OnMenuSelect` / `OnMenuGetItemEnabled` / `OnMenuHilight` (`sub_82D09F28`), so a choice reaches the popup
//! list and not the screen below (the list itself is `skate_core::fe_popup`, run by retail_menu_stage).

use crate::fe::apt_vm::Value;
use skate_core::fe_popup::PopupRequest;

// skate_vita: the mod popups (`PopupMods`, `mod_callback`) are not taken (no mods on the Vita).

#[derive(Clone, Debug, Default)]
pub struct PopupHost {
    /// The shown popup (a copy of the list's last entry, like host 29's +60 buffer), None without one.
    pub shown: Option<PopupRequest>,
    /// Choices the movie made this frame (host vt+64 runs in the stage after the movie update).
    pub choices: Vec<i32>,
    /// +49: input taken until the next show (OnMenuSelect clears it).
    pub input_ready: bool,
}

impl PopupHost {
    /// Natives for `Popup.*`, and the focus-screen `Game` callbacks while a popup is shown.
    pub fn call(&mut self, object: &str, method: &str, args: &[Value]) -> Option<Value> {
        let shown = self.shown.as_ref()?;
        let index = |i: usize| args.get(i).map_or(0, crate::fe::apt_vm::to_integer);
        Some(match (object, method) {
            ("Popup", "GetTitle") => Value::Text(shown.title.clone()),
            ("Popup", "GetDescription") => Value::Text(shown.description.clone()),
            ("Popup", "GetNumOptions") => Value::Number(shown.options.len() as f64),
            // 0x825AA170 has no bounds check; past the count we answer "".
            ("Popup", "GetOption") => Value::Text(shown.options.get(index(0).max(0) as usize).cloned().unwrap_or_default()),
            ("Popup", "IsPopupCancelable") => Value::Bool(shown.cancelable),
            ("Popup", "GetDefaultSelection") => Value::Number(f64::from(shown.default_selection)),
            // Host vt+72 0x8281DD70: every item enabled.
            ("Game", "OnMenuGetItemEnabled") => Value::Bool(true),
            ("Game", "OnMenuHilight") => Value::Undefined,
            // 0x825A4F00: input blocked until the next show, then host vt+64 with the item index (255 = cancel).
            ("Game", "OnMenuSelect") => {
                self.input_ready = false;
                self.choices.push(index(1));
                Value::Undefined
            }
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn natives_answer_from_the_shown_popup_and_choices_are_taken() {
        let mut host = PopupHost::default();
        assert!(host.call("Popup", "GetTitle", &[]).is_none(), "no popup, no answer");
        assert!(host.call("Game", "OnMenuSelect", &[]).is_none(), "the screen below gets it");
        host.shown = Some(PopupRequest {
            description: "D".into(),
            options: vec!["ID_COMMON_YES".into(), "ID_COMMON_NO".into()],
            ..Default::default()
        });
        host.input_ready = true;
        assert_eq!(host.call("Popup", "GetNumOptions", &[]), Some(Value::Number(2.0)));
        assert_eq!(host.call("Popup", "GetOption", &[Value::Number(1.0)]), Some(Value::Text("ID_COMMON_NO".into())));
        assert_eq!(host.call("Popup", "IsPopupCancelable", &[]), Some(Value::Bool(false)));
        assert_eq!(host.call("Game", "OnMenuGetItemEnabled", &[Value::Number(0.0), Value::Number(1.0)]), Some(Value::Bool(true)));
        host.call("Game", "OnMenuSelect", &[Value::Number(0.0), Value::Number(1.0), Value::Text("ID_COMMON_NO".into())]);
        assert_eq!(host.choices, vec![1]);
        assert!(!host.input_ready);
    }
}
