//! The Extras screen's natives and the text viewer's `tos` object (skate-game `extras_host.rs`, PR #52 `b110bab`; doc
//! 31 "Extras and cheat codes", research b320 / b304 / b305 / b322): Game.Extras_GetNumOptions / _GetOptionTitle /
//! _MenuTitle and the menu picker's Game.OnMenuGetItemEnabled / OnMenuHilight / OnMenuSelect while FE 18 is on top.
//! Rows and states are `skate_core::extras`; the cheat matcher `skate_core::cheats`; the text entry's rules
//! `skate_core::text_entry`. The stage applies what this host queues (screen states, row actions) after the movie call
//! returns.
//!
//! skate_vita: the text entry is the Vita's system keyboard (the host's IME dialog), as retail opened the Xbox one, so
//! upstream's on-screen keyboard state (`TextEntryView`) is not taken; the cheat and unlock events are the stage's
//! frame output instead of Bevy messages.
use crate::fe::apt_vm::{to_integer, ObjectKind, Value, Vm};
use skate_core::extras::{Extras, ExtrasAction, ExtrasRules};

#[derive(Clone, Default)]
pub struct ExtrasHost {
    pub screen: Extras,
    pub rules: ExtrasRules,
    /// FE 18 is on top (its picker callbacks are ours).
    pub open: bool,
    /// Screen states to post to the movie, in order.
    pub states: Vec<i32>,
    /// Row actions the player picked, in order.
    pub selected: Vec<ExtrasAction>,
}

impl ExtrasHost {
    pub fn call(&mut self, method: &str, args: &[Value]) -> Option<Value> {
        if !self.open {
            return None;
        }
        let arg = |i: usize| args.get(i).map_or(0, to_integer);
        Some(match method {
            // 825A6C80: the main rows (the Skate Reel list has no videos yet).
            "Extras_GetNumOptions" => Value::Number(self.rows() as f64),
            "Extras_GetOptionTitle" => {
                let i = arg(0);
                let label = (i >= 0).then(|| self.rules.rows.get(i as usize)).flatten().filter(|_| self.main()).map(|r| r.label.clone());
                Value::Text(label.unwrap_or_default())
            }
            "Extras_MenuTitle" => Value::Text(self.screen.title(&self.rules).to_owned()),
            "OnMenuGetItemEnabled" => {
                let i = arg(1);
                Value::Bool(i >= 0 && self.main() && self.screen.enabled(&self.rules, i as usize))
            }
            // 0x8274CA90-style no-op (+68 base).
            "OnMenuHilight" => Value::Undefined,
            "OnMenuSelect" => {
                let i = arg(1);
                if i >= 0 && self.main() {
                    if let Some(action) = self.screen.select(&self.rules, i as usize) {
                        if action == ExtrasAction::SkateReel {
                            self.states.push(self.rules.list_state);
                        }
                        self.selected.push(action);
                    }
                }
                Value::Undefined
            }
            _ => return None,
        })
    }

    fn main(&self) -> bool {
        self.screen.mode == skate_core::extras::ExtrasMode::Main
    }

    fn rows(&self) -> usize {
        if self.main() { self.rules.rows.len() } else { 0 }
    }
}

/// The text viewer's `tos` object (registrar 0x825AA260, research b322) while FE 3 shows a text.
#[derive(Clone, Default)]
pub struct TosHost {
    pub viewer: Option<skate_core::text_viewer::TextViewer>,
}

impl TosHost {
    pub fn call(&self, vm: &mut Vm, method: &str, args: &[Value]) -> Result<Option<Value>, String> {
        let Some(v) = &self.viewer else { return Ok(None) };
        let arg = |i: usize| args.get(i).map_or(0, to_integer).max(0) as usize;
        Ok(Some(match method {
            "GetIsReady" => Value::Bool(true),
            "GetHeaderText" => Value::Text(v.title.clone()),
            "GetTOSstate" => Value::Number(v.state as f64),
            "GetNumWords" => Value::Number(v.words.len() as f64),
            // 0x825AA3D0: an object { numWords, word0 .. word(n-1) }.
            "GetWordRange" => {
                let words = v.range(arg(0), arg(1));
                let o = vm.object(ObjectKind::Plain);
                vm.set(o, "numWords", Value::Number(words.len() as f64))?;
                for (i, w) in words.iter().enumerate() {
                    vm.set(o, format!("word{i}"), Value::Text(w.clone()))?;
                }
                Value::Object(o)
            }
            // 0x825AA578: the screen's state +0x10 = 0 (the stage closes FE 3 on Back itself).
            "SetTOSclosed" => Value::Undefined,
            _ => return Ok(None),
        }))
    }
}

/// The Licenses text (ours; `tools/gen_licenses.py` writes it from `cargo metadata`).
pub const LICENSES: &str = include_str!("licenses.txt");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_answer_only_while_open_and_select_queues_the_action() {
        let mut host = ExtrasHost::default();
        assert_eq!(host.call("Extras_GetNumOptions", &[]), None, "closed: not ours");
        host.open = true;
        assert_eq!(host.call("Extras_GetNumOptions", &[]), Some(Value::Number(7.0)));
        assert_eq!(host.call("Extras_GetOptionTitle", &[Value::Number(0.0)]), Some(Value::Text("ID_EXTRA_CHEAT_CODE".into())));
        assert_eq!(host.call("Extras_MenuTitle", &[]), Some(Value::Text("ID_MAIN_MENU_EXTRAS".into())));
        assert_eq!(host.call("OnMenuGetItemEnabled", &[Value::Number(0.0), Value::Number(0.0)]), Some(Value::Bool(true)));
        assert_eq!(host.call("OnMenuGetItemEnabled", &[Value::Number(0.0), Value::Number(4.0)]), Some(Value::Bool(false)), "credits greyed");
        host.call("OnMenuSelect", &[Value::Number(0.0), Value::Number(0.0), Value::Text("x".into())]);
        host.call("OnMenuSelect", &[Value::Number(0.0), Value::Number(4.0), Value::Text("x".into())]);
        assert_eq!(host.selected, vec![ExtrasAction::CheatEntry], "a greyed row selects nothing");
    }
}
