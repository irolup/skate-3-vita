//! FE 34 FreeSkateOption's `FreeSkateOption` natives (registrar ~825AD570, b564) while it is on top (skate-game
//! `free_skate_options_host.rs`, PR #52 `36018e19`; doc 31 "Free Play options (FE 34)").
use crate::fe::apt_vm::Value;
use skate_core::free_skate_options::{FreeSkateOptions, FreeSkateRules};

#[derive(Clone, Default)]
pub struct FreeSkateOptionsHost {
    pub screen: FreeSkateOptions,
    pub rules: FreeSkateRules,
    /// FE 34 is on top.
    pub open: bool,
    /// Screen states to post to the movie, in order.
    pub states: Vec<i32>,
}

impl FreeSkateOptionsHost {
    pub fn call(&self, object: &str, method: &str) -> Option<Value> {
        if !self.open || object != "FreeSkateOption" {
            return None;
        }
        let v = self.screen.values;
        Some(match method {
            "GetSelection" => Value::Number(self.screen.row as f64),
            "GetTrafficSetting" => Value::Number(FreeSkateOptions::tenths(v.traffic) as f64),
            "GetPedestrianSetting" => Value::Number(FreeSkateOptions::tenths(v.pedestrians) as f64),
            "GetAISkatersSetting" => Value::Bool(v.ai_skaters),
            _ => return None,
        })
    }
}
