//! The retail manual meter (`hud2/manualmeter`, skate_vita M9) driven by the game: the
//! screen's own script moves the arrow (`Update(value)`, -100..100) and lights the nose
//! or tail sweet spot (`SetState`: 1 off, 2 nose rollover, 3 nose on, 4 tail rollover,
//! 5 tail on, 6 hide, 7 show). The reference has no feed for it: how the deck tilt maps
//! to the arrow and the states below is ours (documented in PLAN.md §14 and
//! `docs/milestones/M9-progress.md`).
use crate::fe::apt_scene::{self, Draw, Shapes};
use crate::fe::apt_vm::{Value, Vm};
use crate::menu_runtime::{MenuRuntime, Natives};

pub struct Host;
impl Natives for Host {
    fn names(&self) -> Vec<String> {
        vec!["ScreenManager".into()]
    }
    fn call(&mut self, _: &mut Vm, _: &str, _: &str, _: &[Value]) -> Result<Value, String> {
        Ok(Value::Undefined)
    }
}

/// Fraction of the maximum tilt at which the sweet spot starts to glow, and at which it
/// is lit.
const ROLLOVER: f32 = 0.55;
const ON: f32 = 0.85;

pub struct ManualMeter {
    pub runtime: MenuRuntime<Host>,
    pub shapes: Shapes,
    shown: bool,
    state: u8,
}

impl ManualMeter {
    pub fn load(json: &serde_json::Value) -> Result<Self, String> {
        let shapes: Shapes = serde_json::from_value(json["shapes"].clone()).map_err(|e| e.to_string())?;
        let mut runtime = MenuRuntime::load(json, Host)?;
        runtime.call_screen("SetState", vec![Value::Number(6.0)])?;
        Ok(Self { runtime, shapes, shown: false, state: 1 })
    }

    /// `active`: a manual is held; `value`: the tilt as a fraction of the maximum (-1 tail
    /// .. 1 nose). Call once per displayed frame.
    pub fn update(&mut self, active: bool, value: f32) -> Result<(), String> {
        if active != self.shown {
            self.shown = active;
            self.runtime.call_screen("SetState", vec![Value::Number(if active { 7.0 } else { 6.0 })])?;
            if !active {
                self.state = 1;
            }
        }
        if active {
            let state = match (value.abs() >= ON, value.abs() >= ROLLOVER, value > 0.0) {
                (true, _, true) => 3,
                (false, true, true) => 2,
                (true, _, false) => 5,
                (false, true, false) => 4,
                _ => 1,
            };
            if state != self.state {
                self.state = state;
                self.runtime.call_screen("SetState", vec![Value::Number(state as f64)])?;
            }
            self.runtime.call_screen("Update", vec![Value::Number((-value * 100.0) as f64)])?;
        }
        self.runtime.update()
    }

    pub fn draw(&self) -> Result<Vec<Draw>, String> {
        apt_scene::draw(&self.runtime.bindings.movie, &self.runtime.vm, &self.shapes)
    }
}
