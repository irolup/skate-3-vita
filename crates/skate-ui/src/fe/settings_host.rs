//! The Game Settings screen's natives (options.apt, template 90, FE screen 45; doc 31 "Game Settings screen"):
//! `Game.GameSettings_*` and the `Game.OnMenu*` calls, answered from the menu registry like CoreMenu, so mods'
//! rows, labels, rules and values show up. Research `.local/research/settings-host-map.md`.
//!
//! The movie only knows row indices; retail indexes its raw table by the screen index (+68) and row (+72). Ours
//! indexes a snapshot of the visible row ids taken at GetNumOptions (the start of CreateOptions) and kept until
//! the next rebuild, so SetState(50) refreshes the same rows.
use crate::fe::apt_vm::{to_integer, Value};
use skate_core::menu_values::Display;
use skate_core::menus::{EntryRef, MenuContext, MenuRegistry, Widget};

/// Game Settings screen state (screen +68 / +72 and the row snapshot).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SettingsState {
    /// The settings screen is the top screen: `Game.OnMenu*` calls are its.
    pub open: bool,
    /// Settings screen index (+68; 0 / 1 = the main list).
    pub screen: usize,
    /// Highlighted row (+72, OnMenuHilight).
    pub row: usize,
    /// Setting ids of the rows the movie built.
    pub rows: Vec<String>,
    /// Rows the movie selected this frame (OnMenuSelect), for the host to dispatch.
    pub selected: Vec<usize>,
}

impl SettingsState {
    pub fn row_id(&self, row: usize) -> Option<&str> {
        self.rows.get(row).map(String::as_str)
    }
}

pub struct GameSettings<'a> {
    pub registry: &'a MenuRegistry,
    pub context: &'a MenuContext,
    pub titles: &'a [String],
    pub state: &'a mut SettingsState,
}

fn widget_name(widget: Widget) -> &'static str {
    match widget {
        Widget::Option => "option",
        Widget::Slider => "slider",
        Widget::Selector => "selector",
    }
}

impl GameSettings<'_> {
    /// Answer a `Game` member when it is one of the settings natives (`None` = not ours).
    pub fn call(&mut self, method: &str, args: &[Value]) -> Option<Value> {
        let index = |i: usize| args.get(i).map(|v| to_integer(v).max(0) as usize).unwrap_or(0);
        let setting = |s: &Self, row: usize| s.state.row_id(row).and_then(|id| s.registry.data().setting(id).cloned());
        let display = |s: &Self, row: usize| s.state.row_id(row).and_then(|id| s.registry.display(&EntryRef::setting(id)));
        Some(match method {
            // 825A6F70: the screen's rows (retail stops at a hidden row; the registry skips hidden rows).
            "GameSettings_GetNumOptions" => {
                self.state.rows = self.registry.screen_rows(self.state.screen, self.context).into_iter().map(|r| r.id).collect();
                Value::Number(self.state.rows.len() as f64)
            }
            "GameSettings_GetOptionType" => {
                Value::Text(setting(self, index(0)).map_or("", |s| widget_name(s.widget)).into())
            }
            "GameSettings_GetOptionTitle" => Value::Text(setting(self, index(0)).map(|s| s.label).unwrap_or_default()),
            // 825A6DA8: the title table (extracted at setup) by screen index.
            "GameSettings_GetScreenTitle" => Value::Text(self.titles.get(self.state.screen).cloned().unwrap_or_default()),
            // 825A6D58: slider bars.
            "GameSettings_GetIntegerValue" => match display(self, index(0)) {
                Some(Display::Bars(n)) => Value::Number(n as f64),
                _ => Value::Number(0.0),
            },
            // 825A6E38: the value's label string id; "#ERROR!!" out of range.
            "GameSettings_GetStringValue" => match display(self, index(0)) {
                Some(Display::Label(label)) => Value::Text(label),
                _ => Value::Text("#ERROR!!".into()),
            },
            // 825A6E88: only the Difficulty value row has a description (by difficulty index, the same names as
            // its value labels); every other row "#".
            "GameSettings_GetValueDescription" => {
                let row = index(0);
                match (self.state.row_id(row), display(self, row)) {
                    (Some("ID_GAMESETTINGS_DIFFICULTY_OPTIONS"), Some(Display::Label(label))) => Value::Text(
                        label.replacen("ID_GAMESETTINGS_DIFFICULTY_", "ID_GAMESETTINGS_DIFFICULTY_DESCRIPTION_", 1),
                    ),
                    _ => Value::Text("#".into()),
                }
            }
            "OnMenuGetItemEnabled" if self.state.open => {
                let row = index(1);
                let rows = self.registry.screen_rows(self.state.screen, self.context);
                let enabled = |id: &str| rows.iter().any(|r| r.id == id && r.enabled);
                // NOT RETAIL (guard): options.apt's NextValueCalc recurses forever when no row is enabled; retail
                // never has such a screen, mods can. Then the first row counts as enabled.
                let none = !self.state.rows.iter().any(|id| enabled(id));
                Value::Bool(self.state.row_id(row).is_some_and(enabled) || (none && row == 0))
            }
            // 8260D710: the row Left / Right act on.
            "OnMenuHilight" if self.state.open => {
                self.state.row = index(1);
                Value::Undefined
            }
            // 8260D290 runs in the host (it changes the registry and the screen).
            "OnMenuSelect" if self.state.open => {
                self.state.selected.push(index(1));
                Value::Undefined
            }
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use skate_core::menus::{Handler, MenuData, Owner, Setting};

    fn registry() -> MenuRegistry {
        let setting = |id: &str, widget| Setting { id: id.into(), label: format!("{id}_LABEL"), widget, link: None };
        let mut screens = vec![Vec::new(); 3];
        screens[1] = vec!["A".into(), "B".into(), "ID_GAMESETTINGS_DIFFICULTY_OPTIONS".into()];
        let mut r = MenuRegistry::new(MenuData {
            settings: vec![
                setting("A", Widget::Option),
                setting("B", Widget::Slider),
                setting("ID_GAMESETTINGS_DIFFICULTY_OPTIONS", Widget::Selector),
            ],
            screens,
            ..Default::default()
        });
        r.register_handler(
            EntryRef::setting("A"),
            Handler { owner: Owner::Engine("t".into()), select: true, highlight: false, callback: None },
        );
        r
    }

    #[test]
    fn answers_options_apt_from_the_registry() {
        let registry = registry();
        let context = MenuContext::new("FreePlay");
        let titles = vec!["ID_GAMESETTINGS_TITLE".to_owned(); 2];
        let mut state = SettingsState { open: true, screen: 1, ..Default::default() };
        let mut g = GameSettings { registry: &registry, context: &context, titles: &titles, state: &mut state };
        let n = |v: f64| Value::Number(v);
        assert_eq!(g.call("GameSettings_GetNumOptions", &[]), Some(n(3.0)));
        assert_eq!(g.call("GameSettings_GetOptionType", &[n(1.0)]), Some(Value::Text("slider".into())));
        assert_eq!(g.call("GameSettings_GetOptionTitle", &[n(0.0)]), Some(Value::Text("A_LABEL".into())));
        assert_eq!(g.call("GameSettings_GetScreenTitle", &[]), Some(Value::Text("ID_GAMESETTINGS_TITLE".into())));
        assert_eq!(g.call("GameSettings_GetStringValue", &[n(1.0)]), Some(Value::Text("#ERROR!!".into())));
        assert_eq!(g.call("GameSettings_GetValueDescription", &[n(0.0)]), Some(Value::Text("#".into())));
        assert_eq!(g.call("OnMenuGetItemEnabled", &[n(0.0), n(0.0)]), Some(Value::Bool(true)));
        assert_eq!(g.call("OnMenuGetItemEnabled", &[n(0.0), n(1.0)]), Some(Value::Bool(false)), "no handler");
        g.call("OnMenuHilight", &[n(0.0), n(2.0), Value::Text(String::new())]);
        g.call("OnMenuSelect", &[n(0.0), n(0.0), Value::Text(String::new())]);
        assert_eq!(g.call("GetSomethingElse", &[]), None);
        assert_eq!((state.row, state.selected.as_slice()), (2, &[0][..]));
        // Not the top screen: the OnMenu calls are not the settings screen's.
        state.open = false;
        let mut g = GameSettings { registry: &registry, context: &context, titles: &titles, state: &mut state };
        assert_eq!(g.call("OnMenuSelect", &[n(0.0), n(1.0)]), None);
    }
}
