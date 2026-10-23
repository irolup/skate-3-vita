//! The Trick Guide screen's natives (doc 31 "Trick Guide"): FETrickTutorial (registrar sub_825AE7E0),
//! Game.GetMenuItems over the screen's list and the picker callbacks, HUDComponents.PlayerStance_IsRegular. The
//! navigation is `skate_core::trick_guide`; this host turns the movie's calls into it and queues the screen states
//! the C++ posts back (the facade's SetScreenState, applied by `retail_menu_stage` after the call returns).
//! skate_vita: skate-game `trick_guide_host.rs` (PR #52 `fad5aae`) in skate-ui `fe`; the stage is `menu_stage`.
use crate::fe::apt_vm::{to_integer, Value, Vm};
use skate_core::trick_guide::{Stance, TrickGuide, TrickGuideState};
use std::sync::Arc;

#[derive(Clone, Default)]
pub struct TrickGuideHost {
    pub guide: Arc<TrickGuide>,
    pub state: TrickGuideState,
    pub stance: Stance,
    /// The screen is on top of the stack (its picker callbacks are ours).
    pub open: bool,
    /// Screen states to post to the movie, in order.
    pub states: Vec<i32>,
}

impl TrickGuideHost {
    pub fn new(guide: Arc<TrickGuide>) -> Self {
        Self { state: TrickGuideState::open(&guide), guide, ..Default::default() }
    }

    /// Screen init (0x82618A40): level 0, row 0.
    pub fn reset(&mut self) {
        self.state = TrickGuideState::open(&self.guide);
        self.states.clear();
    }

    /// One native call; None = not ours.
    pub fn call(&mut self, vm: &mut Vm, object: &str, method: &str, args: &[Value]) -> Result<Option<Value>, String> {
        let index = |i: usize| args.get(i).map_or(0, to_integer).max(0) as usize;
        let guide = &*self.guide;
        let text = |s: String| Value::Text(s);
        Ok(Some(match (object, method) {
            // 825AE8A8: [level, has submenu, description id].
            ("FETrickTutorial", "GetMenuItemData") => {
                let values = vec![
                    Value::Number(self.state.level as f64),
                    Value::Bool(self.state.has_submenu(guide)),
                    text(self.state.description(guide)),
                ];
                Value::Object(vm.array(values)?)
            }
            // 825AE968: [used count, then gesture, button, connector per slot] (13 values).
            ("FETrickTutorial", "GetMenuItemGesturesButtons") => {
                let (used, slots) = self.state.gestures_buttons(guide, self.stance);
                let mut values = vec![Value::Number(used as f64)];
                values.extend(slots.into_iter().flatten().map(Value::Text));
                Value::Object(vm.array(values)?)
            }
            // 825AEBF0: always false in retail (the stance swap comes from PlayerStance_IsRegular).
            ("FETrickTutorial", "IsGoofy") => Value::Bool(false),
            ("FETrickTutorial", "GetCurrentMenuItem") => Value::Number(self.state.index as f64),
            // 826635A0: CAS byte +120 of the player's skater record.
            ("HUDComponents", "PlayerStance_IsRegular") => Value::Bool(self.stance.regular),
            ("Game", "GetMenuItems") if self.open => {
                let items = self.state.items(guide).into_iter().map(Value::Text).collect();
                Value::Object(vm.array(items)?)
            }
            // 825A5078 -> screen vt+72 = 0x8281DD70 (base): every row enabled.
            ("Game", "OnMenuGetItemEnabled") if self.open => Value::Bool(true),
            ("Game", "OnMenuHilight") if self.open => {
                let state = self.state.hilight(guide, index(1));
                self.states.push(state);
                Value::Undefined
            }
            ("Game", "OnMenuSelect") if self.open => {
                let state = self.state.select(guide, index(1));
                self.states.push(state);
                Value::Undefined
            }
            _ => return Ok(None),
        }))
    }
}

// skate_vita: the mods' Trick Guide layers (`TrickGuideMods`) are not taken (no mods on the Vita).
