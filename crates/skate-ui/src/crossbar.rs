//! The retail crossbar (skate_vita, PLAN.md §14, M9): Skate 3's own pause/main menu movie
//! (`main/core_menu`, linked with its control bundles) driven by our menu model. The
//! movie is a view: it asks the host for names, icons and indices (`coremenu.*`) and the
//! host tells it what changed (`SetState`, `UpdateHelperText`, `UpdateInput`). The
//! protocol is in docs/milestones/M9-progress.md §5.
use crate::apt_scene::{self, Draw, Shapes};
use crate::apt_vm::{Value, Vm};
use crate::menu_runtime::{MenuRuntime, Natives};

/// What choosing an option does; the game maps it to its own state changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Skate,
    Resume,
    Respawn,
    MainMenu,
    Quit,
    Park,
    /// A campaign district (the sub-option picks which); its collision and render stream in.
    District,
    Difficulty,
    /// The Physics setting: Fast (25 solver iterations) or Retail (50).
    Physics,
    /// L1c: the traffic density (Off, Low, Medium, Retail).
    Traffic,
    /// L1d: the ambient NPC skaters (Off, On).
    AiSkaters,
    /// L1e: the pedestrian density (Off, Low, Medium, Retail).
    Pedestrians,
    Controls,
    Calibration,
    RearTouch,
    DpadBumpers,
    FrontTouch,
    Volume,
    DebugHud,
    /// The audio setting: the procedural sounds or the retail audio (experimental).
    Audio,
    /// Learn > On Screen Help: the sub-options are the two toggles (Trick Analyzer, Manual Meter).
    OnScreenHelp,
    MenuStyle,
    /// The replay editor (pause menu): the sub-option picks the instant replay or a saved replay.
    Replay,
}

#[derive(Clone, Debug)]
pub struct OptionDef {
    pub name: String,
    /// A frame label of the crossbar's option icon (`freeskate`, `skatepark`, ...).
    pub icon: &'static str,
    pub help: String,
    pub action: Action,
    /// Sub-options (a picker); empty for a plain action.
    pub values: Vec<String>,
    pub current: usize,
}

#[derive(Clone, Debug)]
pub struct MenuDef {
    pub name: &'static str,
    /// A frame label of the category icon (`singleplayer`, `learn`, `options`, ...).
    pub icon: &'static str,
    pub options: Vec<OptionDef>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    Left,
    Right,
    Up,
    Down,
    Accept,
    Back,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    None,
    /// An option was chosen (with the chosen value index for a picker).
    Chosen(Action, Option<usize>),
    /// Back at the top level.
    Close,
}

// Host constants: key codes and the crossbar's native states (the script only compares).
pub(crate) const APT_UP: f64 = 1.0;
pub(crate) const APT_DOWN: f64 = 2.0;
pub(crate) const APT_PREVIOUS: f64 = 3.0;
const STATE_REFRESH_MENU: f64 = 10.0;
const STATE_REFRESH_OPTION: f64 = 11.0;
const STATE_INITIALISE_MENU: f64 = 12.0;
const STATE_INITIALISE_SUB: f64 = 13.0;

/// The host side the movie talks to: the menu model and the cursor.
pub struct Host {
    pub menus: Vec<MenuDef>,
    menu: usize,
    /// The remembered option of each menu.
    option: Vec<usize>,
    sub: usize,
    /// 1 = option list, 2 = sub-option picker (the script's `*_MENULEVEL`).
    level: u32,
    title: String,
}

impl Host {
    fn current(&self) -> Option<&OptionDef> {
        self.menus.get(self.menu)?.options.get(self.option[self.menu])
    }
}

impl Natives for Host {
    fn names(&self) -> Vec<String> {
        ["coremenu", "FELanguage", "Game", "ScreenManager"].iter().map(|s| (*s).to_owned()).collect()
    }
    fn constants(&self) -> Vec<(String, Value)> {
        [
            ("AptUp", APT_UP),
            ("AptDown", APT_DOWN),
            ("AptPrevious", APT_PREVIOUS),
            ("FECrossBarScreenMode_RefreshMenu", STATE_REFRESH_MENU),
            ("FECrossBarScreenMode_RefreshOption", STATE_REFRESH_OPTION),
            ("FECrossBarScreenMode_InitialiseMenu", STATE_INITIALISE_MENU),
            ("FECrossBarScreenMode_InitialiseSubOption", STATE_INITIALISE_SUB),
            ("FECrossBarScreenMode_InitSubOptionOnlineTeam", 14.0),
            ("FECrossBarScreenMode_RefreshDLCBanner", 15.0),
        ]
        .iter()
        .map(|(k, v)| ((*k).to_owned(), Value::Number(*v)))
        .collect()
    }
    fn call(&mut self, _vm: &mut Vm, native: &str, method: &str, args: &[Value]) -> Result<Value, String> {
        let n = |i: usize| args.get(i).map_or(0, |v| v.number().max(0.0) as usize);
        let menu = |i: usize| self.menus.get(i);
        let option = |m: usize, o: usize| self.menus.get(m).and_then(|m| m.options.get(o));
        Ok(match (native, method) {
            ("coremenu", "GetIsDemo" | "IsDLCEnabled") => Value::Bool(false),
            ("coremenu", "GetNumMenus") => Value::Number(self.menus.len() as f64),
            ("coremenu", "GetMenuName") => Value::Text(menu(n(0)).map_or(String::new(), |m| m.name.to_owned())),
            ("coremenu", "GetMenuIconName") => Value::Text(menu(n(0)).map_or("singleplayer", |m| m.icon).to_owned()),
            ("coremenu", "GetNumOptions") => Value::Number(menu(n(0)).map_or(0, |m| m.options.len()) as f64),
            ("coremenu", "GetOptionName") => Value::Text(option(n(0), n(1)).map_or(String::new(), |o| o.name.clone())),
            ("coremenu", "GetOptionIconName") => Value::Text(option(n(0), n(1)).map_or("freeskate", |o| o.icon).to_owned()),
            ("coremenu", "IsItemEnabled") | ("Game", "OnMenuGetItemEnabled") => Value::Bool(true),
            ("coremenu", "GetOptionHelperDescription") => Value::Text(self.current().map_or(String::new(), |o| o.help.clone())),
            ("coremenu", "GetNumSubOptions") => Value::Number(option(n(0), n(1)).map_or(0, |o| o.values.len()) as f64),
            ("coremenu", "GetMaxSubOptions") => Value::Number(self.menus.iter().flat_map(|m| &m.options).map(|o| o.values.len()).max().unwrap_or(0) as f64),
            ("coremenu", "GetSubOptionName") => Value::Text(option(n(0), n(1)).and_then(|o| o.values.get(n(2))).cloned().unwrap_or_default()),
            ("coremenu", "GetCurrentMenuIndex") => Value::Number(self.menu as f64),
            ("coremenu", "GetCurrentOptionIndex") => Value::Number(self.option.get(self.menu).copied().unwrap_or(0) as f64),
            ("coremenu", "GetCurrentSubOptionIndex") => Value::Number(self.sub as f64),
            ("coremenu", "GetCurrentMenuLevel") => Value::Number(f64::from(self.level)),
            ("coremenu", "GetCurrentModeTitle") => Value::Text(self.title.clone()),
            // Not used: the career panel and the DLC banner are hidden.
            _ => Value::Undefined,
        })
    }
}

pub struct Crossbar {
    pub runtime: MenuRuntime<Host>,
    pub shapes: Shapes,
}

impl Crossbar {
    /// Builds the movie from the linked screen (`private/ui/core_menu.linked.json`).
    /// Heavy (the JSON is 2 MB): call from a thread.
    pub fn load(json: &serde_json::Value, menus: Vec<MenuDef>) -> Result<Self, String> {
        let shapes: Shapes = serde_json::from_value(json["shapes"].clone()).map_err(|e| e.to_string())?;
        let host = Host { option: vec![0; menus.len()], menus, menu: 0, sub: 0, level: 1, title: String::new() };
        let mut runtime = MenuRuntime::load(json, host)?;
        // The career panel is not ours; the DLC banner hides itself.
        runtime.hide(&["mProgressionPanel", "mDLC"])?;
        Ok(Self { runtime, shapes })
    }

    fn host(&mut self) -> &mut Host {
        &mut self.runtime.bindings.natives
    }

    fn frames(&mut self, n: usize) -> Result<(), String> {
        for _ in 0..n {
            self.runtime.update()?;
        }
        Ok(())
    }

    /// Shows the menu from the top: the cursor goes to `menu`/`option`.
    pub fn open(&mut self, menus: Vec<MenuDef>, menu: usize, option: usize) -> Result<(), String> {
        let host = self.host();
        host.option = vec![0; menus.len()];
        host.menus = menus;
        host.menu = menu.min(host.menus.len().saturating_sub(1));
        host.option[host.menu] = option.min(host.menus[host.menu].options.len().saturating_sub(1));
        host.sub = 0;
        host.level = 1;
        self.runtime.call_screen("InitializeMenu", vec![])?;
        self.runtime.call_screen("UpdateHelperText", vec![]).map(|_| ())
    }

    /// New contents (a value changed): same cursor, option list refreshed.
    pub fn refresh(&mut self, menus: Vec<MenuDef>) -> Result<(), String> {
        let host = self.host();
        host.menus = menus;
        host.option.resize(host.menus.len(), 0);
        let menu = host.menu.min(host.menus.len().saturating_sub(1));
        host.menu = menu;
        self.set_state(STATE_REFRESH_MENU)?;
        self.runtime.call_screen("UpdateHelperText", vec![]).map(|_| ())
    }

    fn set_state(&mut self, state: f64) -> Result<(), String> {
        self.runtime.call_screen("SetState", vec![Value::Number(state)]).map(|_| ())
    }

    /// (menu, option) of the cursor.
    pub fn cursor(&self) -> (usize, usize) {
        let host = &self.runtime.bindings.natives;
        (host.menu, host.option.get(host.menu).copied().unwrap_or(0))
    }

    /// Whether the sub-option picker is open.
    pub fn in_picker(&self) -> bool {
        self.runtime.bindings.natives.level == 2
    }

    pub fn event(&mut self, event: Event) -> Result<Outcome, String> {
        if self.in_picker() {
            return self.picker_event(event);
        }
        let host = self.host();
        let (menus, menu) = (host.menus.len(), host.menu);
        let options = host.menus.get(menu).map_or(0, |m| m.options.len());
        match event {
            Event::Left | Event::Right if menus > 1 => {
                host.menu = if event == Event::Left { (menu + menus - 1) % menus } else { (menu + 1) % menus };
                self.set_state(STATE_REFRESH_MENU)?;
                self.runtime.call_screen("UpdateHelperText", vec![])?;
            }
            Event::Up | Event::Down if options > 1 => {
                let at = host.option[menu];
                host.option[menu] = if event == Event::Up { (at + options - 1) % options } else { (at + 1) % options };
                self.set_state(STATE_REFRESH_OPTION)?;
                self.runtime.call_screen("UpdateHelperText", vec![])?;
            }
            Event::Accept => {
                let Some(option) = host.current().cloned() else { return Ok(Outcome::None) };
                if option.values.is_empty() {
                    return Ok(Outcome::Chosen(option.action, None));
                }
                host.sub = option.current.min(option.values.len() - 1);
                host.level = 2;
                self.set_state(STATE_INITIALISE_SUB)?;
                // The press that opened the picker is not a picker press (SKIP_BUTTONPRESS).
                let controller = self.runtime.controller.ok_or("no controller")?;
                self.runtime.vm.set(controller, "mNumButtonPress", Value::Number(1.0))?;
                self.runtime.call_screen("UpdateHelperText", vec![])?;
            }
            Event::Back => return Ok(Outcome::Close),
            _ => {}
        }
        Ok(Outcome::None)
    }

    fn picker_event(&mut self, event: Event) -> Result<Outcome, String> {
        let host = self.host();
        let count = host.current().map_or(0, |o| o.values.len());
        match event {
            Event::Up | Event::Down if count > 0 => {
                host.sub = if event == Event::Up { (host.sub + count - 1) % count } else { (host.sub + 1) % count };
                let key = if event == Event::Up { APT_UP } else { APT_DOWN };
                self.runtime.call_screen("UpdateInput", vec![Value::Number(key), Value::Number(0.0)])?;
            }
            Event::Accept => {
                let (action, value) = {
                    let host = self.host();
                    (host.current().map(|o| o.action), host.sub)
                };
                self.close_picker()?;
                if let Some(action) = action {
                    return Ok(Outcome::Chosen(action, Some(value)));
                }
            }
            Event::Back => self.close_picker()?,
            _ => {}
        }
        Ok(Outcome::None)
    }

    fn close_picker(&mut self) -> Result<(), String> {
        self.host().level = 1;
        self.runtime.call_screen("UpdateInput", vec![Value::Number(APT_PREVIOUS), Value::Number(0.0)])?;
        Ok(())
    }

    /// One movie frame.
    pub fn update(&mut self) -> Result<(), String> {
        self.frames(1)
    }

    /// The draw batches of the current frame.
    pub fn draw(&self) -> Result<Vec<Draw>, String> {
        apt_scene::draw(&self.runtime.bindings.movie, &self.runtime.vm, &self.shapes)
    }
}
