//! FE 67's natives (the `TeamManagement` object, registrar 825C3498, and the `Game.OnMenu*` calls) while it is on top;
//! docs/customization-survey.md. The screen's logic is `skate_core::team`: this answers the movie.
use crate::fe::apt_vm::{to_integer, Value, Vm};
use skate_core::team::Team;

#[derive(Clone, Default)]
pub struct TeamHost {
    pub screen: Option<Team>,
    pub shown: bool,
    /// The movie's asks for the team's data (`TeamManagement` reaches the movie only as a listed native object).
    pub asked: usize,
    /// B closed it: its close ends it. A screen opened over it (the customisation) closes it too, and the stack
    /// opens it again after: then it keeps its team.
    pub leaving: bool,
}

impl TeamHost {
    pub fn call(&mut self, vm: &mut Vm, object: &str, method: &str, args: &[Value]) -> Option<Result<Value, String>> {
        let t = self.screen.as_mut()?;
        let int = |i: usize| to_integer(args.get(i).unwrap_or(&Value::Undefined));
        let text = |s: &str| Value::Text(s.to_owned());
        let number = |n: usize| Value::Number(n as f64);
        let value = match (object, method) {
            // [members, team name, logo, a photo per member] (825C36B0). The logo and the photos are asset names the
            // image renderer loads; none are rendered here (an empty name draws nothing).
            ("TeamManagement", "GetTeamData") => {
                self.asked += 1;
                let t = self.screen.as_ref()?;
                let (members, name) = t.team_data();
                let mut items = vec![number(members), text(name), text("")];
                items.extend((0..members).map(|_| text("")));
                return Some(vm.array(items).map(Value::Object));
            }
            ("TeamManagement", "GetSelectedTeammateData") => {
                let (selected, name) = t.selected_data();
                return Some(vm.array(vec![number(selected), text(name)]).map(Value::Object));
            }
            ("TeamManagement", "GetEditSkaterOptions") => {
                let options = t.options();
                let mut items = vec![number(options.len())];
                items.extend(options.iter().map(|o| text(o)));
                return Some(vm.array(items).map(Value::Object));
            }
            ("TeamManagement", "GetNumTeamLogos") => number(0),
            ("TeamManagement", "GetLogoAssetID") => text(""),
            ("TeamManagement", "SelectLogo") => Value::Undefined,
            ("TeamManagement", "Unlock_GetTeammateToEdit") => number(t.selected),
            // 825C3D08: 10 after a new career's logo, else 0.
            ("TeamManagement", "GetNewTeamState") => number(0),
            ("TeamManagement", "GetIsUnlockScreenInTransition") => Value::Bool(false),
            ("Game", "OnMenuHilight") => {
                t.highlight(int(0), int(1));
                Value::Undefined
            }
            ("Game", "OnMenuSelect") => {
                t.select(int(1));
                Value::Undefined
            }
            ("Game", "OnMenuGetItemEnabled") => Value::Bool(true),
            _ => return None,
        };
        Some(Ok(value))
    }
}
