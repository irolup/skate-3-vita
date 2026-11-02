//! skate_vita: Park Setup (skate.Park > New Park, retail FE state 0x3A, movie `skatepark/parksetup`; docs/skatepark-survey.md
//! step S5): two rows, the lot and its template, a thumbnail and a bar. The movie takes no keys: the C++ handler
//! (`825D28C0`) moves the row (Up / Down) and the row's choice (Left / Right; a lot change resets the template), then
//! refreshes the movie (state 0, `825D21E0`); A takes an unlocked lot's template, B goes back. The movie asks the
//! `SkatePark` natives (`ParkSetup_*`, `825B45E8` .. `825B46B8`) for the names, the thumbnail and the active row.
//!
//! NOT RETAIL YET: the key sounds are not played; a locked lot's DLC / career dialogs (FE 0xBB / 0xBA) are not shown (the
//! Vita's lots are all open).
use crate::fe::apt_vm::{to_integer, Value, Vm};

/// The movie's only state: refresh everything from the natives.
pub const STATE_REFRESH: i32 = 0;
/// Retail's thumbnail path (`825B4638`, "fe\source\images\skatepark\%s").
const THUMBNAILS: &str = "fe\\source\\images\\skatepark\\";

/// A lot as the screen shows it: its display name, whether it can be chosen, its templates (display name, thumbnail).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ParkSetupLot {
    pub name: String,
    pub unlocked: bool,
    pub templates: Vec<(String, String)>,
}

/// The player took a lot's template (indices into the lots given).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParkSetupChoice {
    pub lot: usize,
    pub template: usize,
}

/// The keys the screen's C++ handler takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParkSetupKey {
    Up,
    Down,
    Left,
    Right,
    Select,
    Back,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ParkSetupHost {
    pub open: bool,
    pub shown: bool,
    pub lots: Vec<ParkSetupLot>,
    /// The chosen lot and template, and the active row (0 lot, 1 template; `S+0x54`, `+0x58`, `+0x5C`).
    pub lot: usize,
    pub template: usize,
    pub bar: usize,
    /// States to post, in order.
    pub states: Vec<i32>,
    pub choices: Vec<ParkSetupChoice>,
    /// B closed it.
    pub closed: bool,
}

impl ParkSetupHost {
    /// Opened on the first lot and template, the lot row active (OnEnter `825D1BF8`).
    pub fn open(lots: Vec<ParkSetupLot>) -> Self {
        Self { open: true, lots, states: vec![STATE_REFRESH], ..Default::default() }
    }

    fn templates(&self) -> usize {
        self.lots.get(self.lot).map_or(0, |l| l.templates.len())
    }

    /// One key of the C++ handler; false when not its own.
    pub fn key(&mut self, key: ParkSetupKey) -> bool {
        let wrap = |i: usize, step: isize, n: usize| if n == 0 { 0 } else { (i as isize + step).rem_euclid(n as isize) as usize };
        match key {
            ParkSetupKey::Up | ParkSetupKey::Down => {
                self.bar = wrap(self.bar, if key == ParkSetupKey::Up { -1 } else { 1 }, 2);
            }
            ParkSetupKey::Left | ParkSetupKey::Right => {
                let step = if key == ParkSetupKey::Left { -1 } else { 1 };
                if self.bar == 0 {
                    self.lot = wrap(self.lot, step, self.lots.len());
                    self.template = 0;
                } else {
                    self.template = wrap(self.template, step, self.templates());
                }
            }
            ParkSetupKey::Select => {
                if self.lots.get(self.lot).is_some_and(|l| l.unlocked) && self.template < self.templates() {
                    self.choices.push(ParkSetupChoice { lot: self.lot, template: self.template });
                }
                return true;
            }
            ParkSetupKey::Back => {
                self.closed = true;
                return true;
            }
        }
        self.states.push(STATE_REFRESH);
        true
    }

    /// One native call; None = not ours.
    pub fn call(&mut self, _vm: &mut Vm, object: &str, method: &str, args: &[Value]) -> Result<Option<Value>, String> {
        if !self.open || object != "SkatePark" {
            return Ok(None);
        }
        let lot = self.lots.get(self.lot);
        let template = lot.and_then(|l| l.templates.get(self.template));
        Ok(Some(match method {
            // 825D2D90: the lot's (0) or the template's (1) display name.
            "ParkSetup_GetSelectedName" => {
                let name = if args.first().map_or(0, to_integer) == 0 { lot.map(|l| l.name.clone()) } else { template.map(|t| t.0.clone()) };
                Value::Text(name.unwrap_or_default())
            }
            "ParkSetup_GetSelectedThumbnail" => Value::Text(template.map_or(String::new(), |t| format!("{THUMBNAILS}{}", t.1))),
            "ParkSetup_GetSelectedBar" => Value::Number(self.bar as f64),
            "ParkSetup_IsSelectedParkUnlocked" => Value::Bool(lot.is_some_and(|l| l.unlocked)),
            _ => return Ok(None),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lots() -> Vec<ParkSetupLot> {
        let lot = |name: &str| ParkSetupLot {
            name: name.into(),
            unlocked: true,
            templates: vec![("ID_FULL".into(), format!("{name}_full")), ("ID_EMPTY".into(), format!("{name}_empty"))],
        };
        vec![lot("starter"), lot("industrial"), lot("downtown")]
    }

    #[test]
    fn the_rows_and_choices_move_and_wrap_as_retail() {
        let mut p = ParkSetupHost::open(lots());
        p.key(ParkSetupKey::Down);
        p.key(ParkSetupKey::Right);
        assert_eq!((p.bar, p.lot, p.template), (1, 0, 1), "Down to the template row, Right the next template");
        p.key(ParkSetupKey::Right);
        assert_eq!(p.template, 0, "it wraps");
        p.key(ParkSetupKey::Right);
        p.key(ParkSetupKey::Up);
        p.key(ParkSetupKey::Left);
        assert_eq!((p.bar, p.lot, p.template), (0, 2, 0), "Left on the lot row wraps to the last lot and resets the template");
        p.key(ParkSetupKey::Select);
        assert_eq!(p.choices, vec![ParkSetupChoice { lot: 2, template: 0 }]);
        p.lots[2].unlocked = false;
        p.key(ParkSetupKey::Select);
        assert_eq!(p.choices.len(), 1, "a locked lot is not taken");
        assert!(p.states.iter().all(|s| *s == STATE_REFRESH) && p.states.len() == 7, "the opening refresh, then one a move");
        p.key(ParkSetupKey::Back);
        assert!(p.closed);
    }
}
