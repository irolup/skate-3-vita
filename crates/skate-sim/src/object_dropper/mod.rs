//! skate_vita: retail's Object Dropper / skate.Park editor (docs/object-dropper-survey.md, step D3), Bevy-free and
//! deterministic so it can be tested on the desktop. Retail's editor is the dropper object at
//! `[0x830CFD94]+0x124` (constructor 8288B2E8); this module follows its parts: the tuning records ([`tuning`]),
//! the input, camera and cursor ([`camera`]), the held item ([`manipulate`]), the catalogue ([`catalog`]), the items
//! in the world ([`scene`]) and the placement budgets ([`budget`]).
//!
//! The skater stays where they are; the editor's camera orbits a cursor that the left stick moves over the ground,
//! the d-pad raises and lowers, and the right stick and d-pad left / right turn and zoom. A held item stands at the
//! cursor; RT turns it, RB moves it along its own axes, LT makes both smooth.
//!
//! Buttons (the screen's handlers, docs/object-dropper-survey.md "The screen"): in the free camera A takes the item
//! under the cursor (with LB held: a copy of it), X opens the catalogue, Y deletes the item under the cursor, B
//! leaves; holding an item, A places it (with LB held: places a copy and keeps one in hand), Y deletes it, L3 resets
//! it, X swaps it for a catalogue item, B puts it back. In the catalogue RT / LT change the super category, Up / Down
//! move, A opens a sub category then takes the item previewed at the cursor, B goes back; in the park editor X
//! changes the item's size. Back opens Properties in the park editor and asks to reset every object in the open
//! world. A new item, or a copy, joins only when the budgets allow it (8288EE08 / 8288EF58), else the editor refuses
//! (retail's denied sound). The screen modes the C++ posts to the movie on the way are kept for the screen
//! ([`Editor::take_posts`], [`screen`]).
pub mod browse;
pub mod budget;
pub mod camera;
pub mod catalog;
pub mod manipulate;
pub mod overlap;
pub mod quick;
pub mod save;
pub mod scene;
pub mod snap;
pub mod tuning;
pub mod world;

use glam::{Quat, Vec2, Vec3};

pub use browse::{Browse, Listing};
pub use budget::{Limits, Meter, Usage};
pub use camera::{Camera, Controls, Cursor, EditorInput, Ground, Pressed};
pub use catalog::{Catalog, Item, Part};
pub use manipulate::{Held, Member, Shift};
pub use scene::{Origin, Placed, Scene};
pub use tuning::{ModeTuning, Tuning};

/// Which screen drives the editor (docs/object-dropper-survey.md "The screen").
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Variant {
    /// FE state 0x35, "Edit Park" (flow message 0x79): terrain and props, Properties, every shift.
    #[default]
    Park,
    /// FE state 0x37, the phone's "Object Dropper" in the open world (flow message 0x7A): no Properties, no locked
    /// axis or fine tune; Back asks to reset every object.
    World,
}

/// The screen modes the C++ posts to the `ObjectDropper` movie (`SetState(n)`; n = the index in the name table
/// `0x8302C440`).
pub mod screen {
    pub const FREE_CAM: u8 = 0;
    pub const CATALOG: u8 = 1;
    pub const MANIPULATE: u8 = 2;
    pub const NEXT_CATEGORY: u8 = 4;
    pub const PREVIOUS_CATEGORY: u8 = 5;
    pub const NEXT_SUB_CATEGORY: u8 = 6;
    pub const PREVIOUS_SUB_CATEGORY: u8 = 7;
    pub const NEXT_ITEM: u8 = 8;
    pub const PREVIOUS_ITEM: u8 = 9;
    pub const MOVE_ON_LOCKED_AXIS: u8 = 13;
    pub const ROTATE_OBJECT: u8 = 14;
    pub const GROUP_SELECT: u8 = 15;
    pub const HIDE_SELECTION: u8 = 16;
    pub const SHOW_SELECTION: u8 = 17;
    pub const DELETE: u8 = 18;
    pub const REFRESH_FENG_SHUI: u8 = 19;
    pub const REFRESH_DUPLICATE: u8 = 22;
    pub const REFRESH_FINE_TUNE: u8 = 23;
    pub const REFRESH_SIZE: u8 = 24;
    pub const SHOW_LOCK_AXIS: u8 = 25;
    pub const SHOW_ROTATE: u8 = 26;
    pub const REFRESH_INFO: u8 = 27;
    pub const GOTO_SUB_CATEGORIES: u8 = 28;
    pub const GOTO_ITEMS: u8 = 29;
}

/// Updates A must be held in the free camera to start group select (200 ms, `825C84A8`).
pub const GROUP_HOLD: u32 = 12;

/// Retail's dropper modes (+0x1768 and the FE screen modes FreeCam / Catalog / Manipulate).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Flying the camera and the cursor.
    FreeCam,
    /// The catalogue is open (no move).
    Catalog,
    /// Moving a held item.
    Manipulate,
    /// The Quick Menu ("Properties", FE state 0x36) over the editor (`quick`).
    Properties,
    /// Group select (FE mode 4, `825C9818`): A held adds what the cursor passes over, Y held removes, X takes the
    /// group (LB + X a copy of it), B leaves.
    Group,
}

/// What an update did, for the world, the screen and its sounds.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// An item joined the world or moved (its scene id).
    Placed(u32),
    /// An item left the world.
    Removed(u32),
    /// An authored piece went back on its layout pose.
    Reset(u32),
    /// An item was taken in hand (its scene id when it came from the world).
    Taken(Option<u32>),
    /// A new item or a copy did not fit the budgets.
    Refused,
    OpenCatalog,
    CloseCatalog,
    /// Back in the park editor: Properties (the Quick Menu, FE state 0x36; not built yet).
    Properties,
    /// Back in the open world: the reset question (`ID_DMO_RESET_CONFIRMATION`, Yes / No; `825C6548`); Yes resets
    /// every object (`Session::reset_dropper`).
    ConfirmReset,
    /// The editor closes.
    Exit,
}

/// The editor while it is open.
#[derive(Clone, Debug, PartialEq)]
pub struct Editor {
    pub tuning: Tuning,
    pub catalog: Catalog,
    pub scene: Scene,
    pub limits: Limits,
    pub mode: Mode,
    pub camera: Camera,
    pub cursor: Cursor,
    /// Quick Menu options (the dropper's +0x17B0, +0x1774 / +0x1778): cursor speed (1 by default) and the Invert X /
    /// Y factors (1 or -1).
    pub cursor_speed: f32,
    pub invert: [f32; 2],
    /// Lock Axis (+0x1770 bits 0-2): x, y, z.
    pub locks: [bool; 3],
    pub held: Option<Held>,
    pub shift: Shift,
    /// Locked-axis steps in a row (+0x17A4).
    pub steps: u32,
    pub variant: Variant,
    /// The camera stands here and only looks around (the open world's dropper, owner on retail: placed where it opens,
    /// it turns but does not move); None: it follows the cursor (the park editor, or ours with the "free camera"
    /// setting).
    pub fixed_eye: Option<Vec3>,
    /// Where the eye got to (`82886500` sweeps its moves against the world); None until its first update, and after a
    /// jump, which is not swept (retail's flag `+0x89`, set by the camera's reset).
    eye_at: Option<Vec3>,
    /// The catalogue's cursor; its listing comes from `Catalog::listing` ([`Editor::set_listing`]).
    pub browse: Browse,
    /// Fine tune shown on (LT in Manipulate, park editor: `S+0x2CD`).
    pub fine_tune: bool,
    /// Duplicate shown on (LB held outside the catalogue: `S+0x2CC`).
    pub duplicate: bool,
    /// The Quick Menu's Snapping option (`D+0x1460`, on by default) and the held piece's snapped origin pose (drawn
    /// and placed there; the cursor keeps the stick's).
    pub snapping: bool,
    snapped: Option<(Vec3, Quat)>,
    /// The advanced modifier (LT) as last seen while moving.
    was_advanced: bool,
    /// The Quick Menu, the mode it returns to, its posts (`quick::screen`), the undo history, the clipboard (Copy:
    /// item and turn) and the Collision option (`D+0x163C`, on by default; NOT RETAIL YET: not used, retail passes
    /// it to the placement check).
    pub quick: quick::Quick,
    quick_back: Mode,
    quick_posts: Vec<u8>,
    pub history: quick::History,
    pub clipboard: Option<(String, Quat)>,
    pub collision: bool,
    /// The held item goes into another (the world's overlap check, [`overlap`]): with Collision on it is drawn red
    /// and A does not place it.
    pub blocked: bool,
    /// The scene and the hand as the Style page found them (its B reverts to them).
    style_revert: Option<(Scene, Option<Held>, Option<u32>, Vec<u32>)>,
    /// The same for the Colour page.
    colour_revert: Option<(Scene, Option<Held>)>,
    /// Group select: the selected items (`8288D840`), what A / Y held paint (`S+0x2DC`: 1 add, 2 remove), and how
    /// long A has been held in the free camera (`S+0x20C`; a short press takes, 200 ms starts group select).
    pub group: Vec<u32>,
    a_hold: u32,
    /// The item under the cursor in the free camera, as the screen last showed it (`D+0x1454`).
    highlight: Option<u32>,
    /// The catalogue's preview moved and its size is still to show (`S+0x2BC`, RefreshInfo).
    info: bool,
    posts: Vec<u8>,
    controls: Controls,
}

impl Editor {
    /// Opens the editor over `scene` with its cursor at `at` (where the skater is) and the camera behind them.
    pub fn open(tuning: Tuning, catalog: Catalog, scene: Scene, at: Vec3, yaw: f32) -> Self {
        let camera = Camera::new(&tuning, yaw);
        Self {
            tuning,
            catalog,
            scene,
            limits: Limits::default(),
            mode: Mode::FreeCam,
            camera,
            cursor: Cursor::new(at),
            cursor_speed: 1.0,
            invert: [1.0; 2],
            locks: [false; 3],
            held: None,
            shift: Shift::Move,
            steps: 0,
            variant: Variant::Park,
            fixed_eye: None,
            eye_at: None,
            browse: Browse::default(),
            fine_tune: false,
            duplicate: false,
            snapping: true,
            snapped: None,
            was_advanced: false,
            quick: quick::Quick::default(),
            quick_back: Mode::FreeCam,
            quick_posts: Vec::new(),
            history: quick::History::default(),
            clipboard: None,
            collision: true,
            blocked: false,
            style_revert: None,
            colour_revert: None,
            group: Vec::new(),
            a_hold: 0,
            highlight: None,
            info: false,
            posts: Vec::new(),
            controls: Controls::default(),
        }
    }

    /// What the catalogue lists (`Catalog::listing` for this world); the cursor starts over.
    pub fn set_listing(&mut self, listing: Listing) {
        self.browse = Browse::new(listing);
    }

    /// The screen modes posted since the last call, in order (`screen`).
    pub fn take_posts(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.posts)
    }

    fn post(&mut self, mode: u8) {
        self.posts.push(mode);
    }

    /// Takes the buttons held when the editor opens as held, not as new presses (the phone's B that opened it
    /// would otherwise close it at once): the pad polled through its held-back polls without acting.
    pub fn prime(&mut self, slots: &[f32; 24]) {
        for _ in 0..4 {
            self.controls.read(slots);
        }
    }

    /// The record of the current mode (the dropper's +0x1758).
    pub fn mode_tuning(&self) -> &ModeTuning {
        match self.mode {
            Mode::FreeCam | Mode::Group => &self.tuning.free_cam,
            Mode::Catalog | Mode::Properties => &self.tuning.default,
            Mode::Manipulate => &self.tuning.manipulate,
        }
    }

    /// The held item's pose: where it stands and its turn.
    pub fn held_pose(&self) -> Option<(Vec3, Quat)> {
        let held = self.held.as_ref()?;
        Some(self.snapped.unwrap_or((self.cursor.position - held.rotation * held.pivot, held.rotation)))
    }

    /// An item's centre in its own space: the middle of its footprint, on its base (`scale`: the placed item's).
    fn pivot(&self, item: &str, scale: Vec3) -> Vec3 {
        self.catalog.item(item).map_or(Vec3::ZERO, |i| {
            let [lo, hi] = i.cost.bounds;
            Vec3::new((lo[0] + hi[0]) * 0.5, 0.0, (lo[2] + hi[2]) * 0.5) * scale
        })
    }

    /// What a new held item adds to the world (nothing for one taken from it).
    fn adding(&self) -> Usage {
        match &self.held {
            Some(h) if h.source.is_none() => Usage::placed(&self.catalog, self.catalog.item(&h.item)),
            _ => Usage::default(),
        }
    }

    /// The Feng Shui meter now.
    pub fn meter(&self) -> Meter {
        budget::meter(&self.limits, &self.scene.usage(&self.catalog), &self.adding())
    }

    /// Whether one more `item` fits the budgets beside the world and what is in hand.
    fn fits(&self, item: &str) -> bool {
        let Some(new) = self.catalog.item(item) else { return false };
        let mut used = self.scene.usage(&self.catalog);
        let held = self.adding();
        used.memory += held.memory;
        used.props_dropped += held.props_dropped;
        used.props_live += held.props_live;
        used.terrain_cost += held.terrain_cost;
        used.terrain_pieces += held.terrain_pieces;
        budget::allows(&self.limits, &used, &Usage::placed(&self.catalog, [new]))
    }

    /// The park's spawner: moved and turned only (never deleted, copied or grouped).
    fn is_spawner(&self, item: &str) -> bool {
        self.catalog.item(item).is_some_and(Item::is_spawner)
    }

    fn is_spawner_id(&self, id: u32) -> bool {
        self.scene.get(id).is_some_and(|p| self.is_spawner(&p.item))
    }

    /// The catalogue's choice: takes a new `item` in hand at the cursor, if it has a model and fits the budgets.
    pub fn choose(&mut self, item: &str, events: &mut Vec<Event>) {
        let template = self.catalog.item(item).and_then(Item::held_as);
        self.held = None;
        match template {
            Some(template) if self.fits(item) => {
                self.history.push(&self.scene);
                let pivot = self.pivot(item, Vec3::ONE);
                self.held = Some(Held { item: item.to_owned(), template, rotation: Quat::IDENTITY, source: None, pivot, tint: 0, others: Vec::new() });
                self.enter(Mode::Manipulate);
                events.push(Event::Taken(None));
            }
            _ => {
                self.enter(Mode::FreeCam);
                events.push(Event::Refused);
            }
        }
    }

    /// A mode change (`825CC4F8`): the movie's state follows.
    fn enter(&mut self, mode: Mode) {
        self.mode = mode;
        self.snapped = None;
        self.shift = Shift::Move;
        self.steps = 0;
        self.fine_tune = false;
        self.post(match mode {
            Mode::FreeCam | Mode::Properties => screen::FREE_CAM,
            Mode::Catalog => screen::CATALOG,
            Mode::Manipulate => screen::MANIPULATE,
            Mode::Group => screen::GROUP_SELECT,
        });
    }

    /// Into the catalogue: an item from the world goes back first; the item list previews its item, the sub
    /// category list shows none.
    fn open_catalog(&mut self, events: &mut Vec<Event>) {
        if self.held.as_ref().is_some_and(|h| h.source.is_some() || h.others.iter().any(|m| m.source.is_some())) {
            if let Some(held) = self.held.take() {
                for (id, position, rotation) in held.source.into_iter().chain(held.others.iter().filter_map(|m| m.source)) {
                    self.scene.set_pose(id, position, rotation);
                    events.push(Event::Placed(id));
                }
            }
        }
        self.held = None;
        self.enter(Mode::Catalog);
        events.push(Event::OpenCatalog);
        if !self.browse.subs {
            self.preview();
        }
    }

    /// The highlighted catalogue item at the cursor (`825D1480` / `825D15C8`), kept in hand while browsing.
    fn preview(&mut self) {
        let held = self.browse.item().and_then(|id| self.catalog.item(id)).and_then(|i| Some((i.id.clone(), i.held_as()?)));
        self.held = held.map(|(item, template)| {
            let pivot = self.pivot(&item, Vec3::ONE);
            Held { item, template, rotation: Quat::IDENTITY, source: None, pivot, tint: 0, others: Vec::new() }
        });
        self.info = self.held.is_some();
    }

    /// The catalogue's keys (`825CAC18`, open world `825C6C68`).
    fn catalog_keys(&mut self, input: &EditorInput, events: &mut Vec<Event>) {
        let p = input.pressed;
        let up = input.step_elevate[0] || input.step[camera::UP];
        let down = input.step_elevate[1] || input.step[camera::DOWN];
        if p.rt || p.lt {
            if self.browse.step_super(p.rt) {
                self.held = None;
                self.post(if p.rt { screen::NEXT_CATEGORY } else { screen::PREVIOUS_CATEGORY });
            }
        } else if p.b {
            if self.browse.subs {
                self.held = None;
                self.enter(Mode::FreeCam);
                events.push(Event::CloseCatalog);
            } else {
                self.held = None;
                self.browse.subs = true;
                self.post(screen::GOTO_SUB_CATEGORIES);
            }
        } else if p.a {
            if self.browse.subs {
                if !self.browse.items().is_empty() {
                    self.browse.subs = false;
                    self.preview();
                    self.post(screen::GOTO_ITEMS);
                }
            } else if let Some(item) = self.held.as_ref().map(|h| h.item.clone()) {
                self.held = None;
                if self.fits(&item) {
                    self.preview();
                    self.enter(Mode::Manipulate);
                    events.push(Event::Taken(None));
                } else {
                    self.preview();
                    events.push(Event::Refused);
                }
            }
        } else if p.x && !self.browse.subs && self.variant == Variant::Park {
            if let Some(next) = self.browse.item().and_then(|id| self.catalog.next_size(id)) {
                self.browse.replace_item(&next);
                self.preview();
                self.post(screen::REFRESH_SIZE);
            }
        } else if p.back && !self.browse.subs && self.variant == Variant::Park {
            self.open_quick(events);
        } else if up || down {
            let moved = if self.browse.subs { self.browse.step_sub(down) } else { self.browse.step_item(down) };
            if moved {
                let mode = match (self.browse.subs, down) {
                    (true, true) => screen::NEXT_SUB_CATEGORY,
                    (true, false) => screen::PREVIOUS_SUB_CATEGORY,
                    (false, true) => screen::NEXT_ITEM,
                    (false, false) => screen::PREVIOUS_ITEM,
                };
                if !self.browse.subs {
                    self.preview();
                }
                self.post(mode);
            }
        }
    }

    /// Back outside the catalogue: Properties in the park editor, the reset question in the open world.
    fn back(&mut self, events: &mut Vec<Event>) {
        match self.variant {
            Variant::Park => self.open_quick(events),
            Variant::World => events.push(Event::ConfirmReset),
        }
    }

    /// The Quick Menu's screen modes posted since the last call (`quick::screen`).
    pub fn take_quick_posts(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.quick_posts)
    }

    /// `825C9F88` / `825CDF30`: the Quick Menu over the current mode, with or without a selection (what is held stays
    /// held), on its main list's first row; duplicate off.
    fn open_quick(&mut self, events: &mut Vec<Event>) {
        self.quick_back = self.mode;
        self.quick = quick::Quick::default();
        self.mode = Mode::Properties;
        // The first row highlighted as the menu opens (device: no highlight until Up / Down; inferred).
        self.quick_posts.push(quick::screen::ENABLE_ROW);
        self.snapped = None;
        if self.duplicate {
            self.duplicate = false;
            self.post(screen::REFRESH_DUPLICATE);
        }
        events.push(Event::Properties);
    }

    /// Back to the mode it was opened from (B on the main list, Copy).
    fn close_quick(&mut self) {
        self.mode = self.quick_back;
        self.post(match self.mode {
            Mode::Catalog => screen::CATALOG,
            Mode::Manipulate => screen::MANIPULATE,
            Mode::Group => screen::GROUP_SELECT,
            _ => screen::FREE_CAM,
        });
    }

    /// Which main list rows can be chosen (`825D02C0`, park editor): Color, Branding, Style and Options always; Copy
    /// with a selection, Paste with a clipboard, Undo / Redo by the history.
    pub fn quick_rows(&self) -> [bool; quick::ROWS] {
        let selection = self.held.is_some() || self.highlight.is_some();
        [true, true, true, true, selection, self.clipboard.is_some(), self.history.can_undo(), self.history.can_redo()]
    }

    /// The Quick Menu's keys (`825CF360`, Options `825CE7E0`).
    fn quick_keys(&mut self, input: &EditorInput, events: &mut Vec<Event>) {
        let p = input.pressed;
        let up = input.step_elevate[0] || input.step[camera::UP];
        let down = input.step_elevate[1] || input.step[camera::DOWN];
        let side = if input.step_side[0] || input.step[camera::LEFT] {
            -1.0
        } else if input.step_side[1] || input.step[camera::RIGHT] {
            1.0
        } else {
            0.0
        };
        match self.quick.page {
            quick::Page::Main => {
                if up || down {
                    self.quick.row = self.quick.step(down, &self.quick_rows());
                    self.quick_posts.push(quick::screen::ENABLE_ROW + self.quick.row as u8);
                } else if p.b {
                    self.close_quick();
                } else if p.a {
                    self.quick_choose(events);
                }
            }
            quick::Page::Options => {
                if up || down {
                    let n = quick::OPTION_ROWS;
                    self.quick.option = if down { (self.quick.option + 1) % n } else { (self.quick.option + n - 1) % n };
                    self.quick_posts.push(quick::OPTION_ENABLE[self.quick.option]);
                } else if side != 0.0 {
                    let post = match self.quick.option {
                        quick::SNAP => {
                            self.snapping = !self.snapping;
                            quick::screen::REFRESH_SNAP
                        }
                        quick::INVERT_Y => {
                            self.invert[1] = -self.invert[1];
                            quick::screen::REFRESH_INVERT_Y
                        }
                        quick::INVERT_X => {
                            self.invert[0] = -self.invert[0];
                            quick::screen::REFRESH_INVERT_X
                        }
                        quick::CURSOR_SPEED => {
                            let (lo, hi) = quick::SPEED_RANGE;
                            self.cursor_speed = ((self.cursor_speed + side * quick::SPEED_STEP) * 10.0).round() / 10.0;
                            self.cursor_speed = self.cursor_speed.clamp(lo, hi);
                            quick::screen::REFRESH_CURSOR_SPEED
                        }
                        _ => {
                            self.collision = !self.collision;
                            quick::screen::REFRESH_COLLISION
                        }
                    };
                    self.quick_posts.push(post);
                } else if p.b {
                    self.quick.page = quick::Page::Main;
                    self.quick_posts.extend([quick::screen::ACTIVATE_QUICK_MENU, quick::screen::ENABLE_ROW + self.quick.row as u8]);
                }
            }
            // B on the Style page: the selection back as it was (825CFE88).
            quick::Page::Style => {
                if p.b {
                    if let Some((scene, held, highlight, group)) = self.style_revert.take() {
                        self.restore(scene, events);
                        self.held = held;
                        self.highlight = highlight;
                        self.group = group;
                    }
                    self.quick_main();
                }
            }
            // The Colour page (825CEAF8): Left / Right a swatch, Up / Down a row of eight, previewed on the selection;
            // A keeps it on a colourisable selection, X colours every colourisable piece, B reverts.
            quick::Page::Color => {
                let n = self.tuning.colours.len().max(1);
                let step = if side != 0.0 {
                    Some(if side > 0.0 { 1 } else { n - 1 })
                } else if (up || down) && n > quick::COLOUR_ROW {
                    Some(if down { quick::COLOUR_ROW } else { n - quick::COLOUR_ROW })
                } else {
                    None
                };
                if let Some(step) = step {
                    self.quick.colour = (self.quick.colour + step) % n;
                    self.preview_tint(self.quick.colour as u8);
                    self.quick_posts.push(quick::screen::REFRESH_CURSOR);
                } else if p.a {
                    if self.selection_tint().is_some() {
                        self.colour_revert = None;
                        self.quick_main();
                    }
                } else if p.x {
                    let tint = self.quick.colour as u8;
                    let ids: Vec<u32> = self.scene.items.iter().filter(|p| self.colourisable(&p.item)).map(|p| p.id).collect();
                    for id in ids {
                        self.scene.set_tint(id, tint);
                    }
                    if let Some(h) = self.held.as_mut().filter(|h| self.catalog.item(&h.item).is_some_and(|i| i.part == Part::Terrain && i.name != quick::NO_TINT)) {
                        h.tint = tint;
                    }
                    self.colour_revert = None;
                    self.quick_main();
                } else if p.b {
                    if let Some((scene, held)) = self.colour_revert.take() {
                        self.scene = scene;
                        self.held = held;
                    }
                    self.quick_main();
                }
            }
            // NOT RETAIL YET: Branding opens and closes; the park brand is not drawn.
            quick::Page::Branding => {
                if p.b {
                    self.quick_main();
                }
            }
        }
    }

    /// A park piece retail can colour: terrain, not the one model in `terrain_no_tint_models` (props never are).
    fn colourisable(&self, item: &str) -> bool {
        self.catalog.item(item).is_some_and(|i| i.part == Part::Terrain && i.name != quick::NO_TINT && !i.is_spawner())
    }

    /// The colourisable selection's colour (held, else highlighted), None without one (`825D0D18`).
    pub fn selection_tint(&self) -> Option<u8> {
        if let Some(h) = &self.held {
            return std::iter::once((h.item.as_str(), h.tint)).chain(h.others.iter().map(|m| (m.item.as_str(), m.tint))).find(|(i, _)| self.colourisable(i)).map(|(_, t)| t);
        }
        self.selection_ids().into_iter().filter_map(|id| self.scene.get(id)).find(|p| self.colourisable(&p.item)).map(|p| p.tint)
    }

    /// The Colour page's live preview (`825CF1A0`): the held item, else the highlighted one; nothing without either.
    fn preview_tint(&mut self, tint: u8) {
        if self.selection_tint().is_none() {
            return;
        }
        let catalog = &self.catalog;
        let ok = |item: &str| catalog.item(item).is_some_and(|i| i.part == Part::Terrain && i.name != quick::NO_TINT);
        // The whole selection (825CF1A0): a held group's every item, or the items group select picked.
        if let Some(h) = self.held.as_mut() {
            if ok(&h.item) {
                h.tint = tint;
            }
            for m in h.others.iter_mut().filter(|m| ok(&m.item)) {
                m.tint = tint;
            }
            return;
        }
        let ids: Vec<u32> = self.selection_ids();
        for id in ids {
            if self.scene.get(id).is_some_and(|p| ok(&p.item)) {
                self.scene.set_tint(id, tint);
            }
        }
    }

    /// The placed items the Quick Menu acts on with nothing in hand: the group select's picks when it was opened from
    /// there, else the highlighted item.
    fn selection_ids(&self) -> Vec<u32> {
        if self.quick_back == Mode::Group && !self.group.is_empty() {
            self.group.clone()
        } else {
            self.highlight.into_iter().collect()
        }
    }

    /// The colour of a `Tuning::colours` index (white past the table).
    pub fn colour(&self, tint: u8) -> [f32; 3] {
        self.tuning.colours.get(usize::from(tint)).copied().unwrap_or([1.0; 3])
    }

    /// Back to the main list (every page's A / B: ActivateQuickMenu and the row).
    fn quick_main(&mut self) {
        self.quick.page = quick::Page::Main;
        self.quick_posts.extend([quick::screen::ACTIVATE_QUICK_MENU, quick::screen::ENABLE_ROW + self.quick.row as u8]);
    }

    /// The selection's style index in `quick::STYLES` (`GetCurrentVisualTypeIndex`; 0 without one).
    pub fn style_index(&self) -> usize {
        let item = self.held.as_ref().map(|h| h.item.clone()).or_else(|| self.selection_ids().first().and_then(|id| self.scene.get(*id)).map(|p| p.item.clone()));
        item.and_then(|i| self.catalog.item(&i)).and_then(|i| catalog::style_of(&i.name)).and_then(|s| quick::STYLES.iter().position(|t| *t == s)).unwrap_or(0)
    }

    /// The selection has a style (`825D0BA8`): what the Style page's Select needs.
    pub fn style_allowed(&self) -> bool {
        let item = self.held.as_ref().map(|h| h.item.clone()).or_else(|| self.selection_ids().first().and_then(|id| self.scene.get(*id)).map(|p| p.item.clone()));
        item.and_then(|i| self.catalog.item(&i)).is_some_and(|i| catalog::style_of(&i.name).is_some())
    }

    /// The Style page's highlight (`825D0288` -> `825D0A98`): the selection swapped for its variant in that style,
    /// live. A held item changes in hand; a placed one is replaced at its pose (a park piece leaves, the variant is
    /// dropped where it stood).
    pub fn style_highlight(&mut self, index: usize) -> Vec<Event> {
        let mut events = Vec::new();
        let Some(style) = quick::STYLES.get(index).copied().filter(|_| self.quick.page == quick::Page::Style) else { return events };
        let catalog = &self.catalog;
        let swap = |item: &mut String, template: &mut String| {
            if let Some(variant) = catalog.style_variant(item, style).filter(|v| v != item) {
                *template = catalog.item(&variant).and_then(|i| i.template.clone()).unwrap_or_default();
                *item = variant;
            }
        };
        // The whole selection: a held group's every item in hand, or each placed item picked.
        if let Some(held) = self.held.as_mut() {
            swap(&mut held.item, &mut held.template);
            for m in held.others.iter_mut() {
                swap(&mut m.item, &mut m.template);
            }
            return events;
        }
        let from_group = self.quick_back == Mode::Group && !self.group.is_empty();
        let mut kept = Vec::new();
        for id in self.selection_ids() {
            let Some(p) = self.scene.get(id).cloned() else { continue };
            let Some(variant) = self.catalog.style_variant(&p.item, style).filter(|v| *v != p.item) else {
                kept.push(id);
                continue;
            };
            self.scene.remove(p.id);
            events.push(Event::Removed(p.id));
            let new = self.scene.drop_item(&variant, p.position, p.rotation);
            self.scene.set_tint(new, p.tint);
            events.push(Event::Placed(new));
            kept.push(new);
        }
        if from_group {
            self.group = kept;
        } else {
            self.highlight = kept.first().copied();
        }
        events
    }

    /// The Style page's Select (`825D0120`): kept when the selection has a style, back to the main list.
    pub fn style_select(&mut self) {
        if self.quick.page == quick::Page::Style && self.style_allowed() {
            self.style_revert = None;
            self.quick_main();
        }
    }

    /// The Branding page's Select: back to the main list (NOT RETAIL YET: the park brand is not drawn).
    pub fn branding_select(&mut self) {
        if self.quick.page == quick::Page::Branding {
            self.quick_main();
        }
    }

    /// A on the main list.
    fn quick_choose(&mut self, events: &mut Vec<Event>) {
        let enabled = self.quick_rows();
        if !enabled[self.quick.row] {
            return;
        }
        match self.quick.row {
            // The Colour page (825CEAF8): a snapshot first when nothing is held; the swatch starts on the selection's.
            quick::COLOR => {
                if self.held.is_none() {
                    self.history.push(&self.scene);
                }
                self.colour_revert = Some((self.scene.clone(), self.held.clone()));
                self.quick.colour = usize::from(self.selection_tint().unwrap_or(0));
                self.quick.page = quick::Page::Color;
                self.quick_posts.extend([quick::screen::ACTIVATE_COLOR, quick::screen::REFRESH_CURSOR]);
            }
            quick::BRANDING => {
                self.quick.page = quick::Page::Branding;
                self.quick_posts.push(quick::screen::ACTIVATE_BRANDING);
            }
            // The Style page: the movie's list drives it (`Editor::style_highlight` / `style_select`); a snapshot
            // first when nothing is held (82516F10), what B reverts to.
            quick::STYLE => {
                if self.held.is_none() {
                    self.history.push(&self.scene);
                }
                self.style_revert = Some((self.scene.clone(), self.held.clone(), self.highlight, self.group.clone()));
                self.quick.page = quick::Page::Style;
                self.quick_posts.push(quick::screen::ACTIVATE_STYLE);
            }
            quick::OPTIONS => {
                self.quick.page = quick::Page::Options;
                self.quick_posts.extend([quick::screen::ACTIVATE_OPTIONS, quick::OPTION_ENABLE[self.quick.option]]);
            }
            // Copy (82886168): the held or highlighted item and its turn, then back.
            quick::COPY => {
                let copied = match (&self.held, self.highlight.and_then(|id| self.scene.get(id))) {
                    (Some(h), _) => Some((h.item.clone(), h.rotation)),
                    (None, Some(p)) => Some((p.item.clone(), p.rotation)),
                    _ => None,
                };
                if let Some(copied) = copied.filter(|(item, _)| !self.is_spawner(item)) {
                    self.clipboard = Some(copied);
                }
                self.close_quick();
            }
            // Paste (82886238): a copy in hand when the budgets allow it (8288EF58), else refused.
            quick::PASTE => {
                let Some((item, rotation)) = self.clipboard.clone() else { return };
                self.mode = self.quick_back;
                if self.held.as_ref().is_some_and(|h| h.source.is_some()) {
                    self.put_back(events);
                }
                self.held = None;
                let template = self.catalog.item(&item).and_then(Item::held_as);
                match template {
                    Some(template) if self.fits(&item) => {
                        self.history.push(&self.scene);
                        let pivot = self.pivot(&item, Vec3::ONE);
                        self.held = Some(Held { item, template, rotation, source: None, pivot, tint: 0, others: Vec::new() });
                        self.enter(Mode::Manipulate);
                        events.push(Event::Taken(None));
                    }
                    _ => {
                        self.enter(Mode::FreeCam);
                        events.push(Event::Refused);
                    }
                }
            }
            // Undo / Redo (82517260 / 82517528): what is held goes back, then the scene steps.
            quick::UNDO | quick::REDO => {
                if self.held.as_ref().is_some_and(|h| h.source.is_some()) {
                    self.put_back(events);
                }
                self.held = None;
                let scene = if self.quick.row == quick::UNDO { self.history.undo(&self.scene) } else { self.history.redo() };
                if let Some(scene) = scene {
                    self.restore(scene, events);
                }
                self.enter(Mode::FreeCam);
                self.post(screen::REFRESH_FENG_SHUI);
            }
            _ => {}
        }
    }

    /// A scene from the history: the world follows the differences (what went is removed, what changed or came back
    /// is placed again).
    fn restore(&mut self, scene: Scene, events: &mut Vec<Event>) {
        let old = std::mem::replace(&mut self.scene, scene);
        for p in &old.items {
            if self.scene.get(p.id).is_none() {
                events.push(Event::Removed(p.id));
            }
        }
        for p in &self.scene.items {
            if old.get(p.id) != Some(p) {
                events.push(Event::Placed(p.id));
            }
        }
        self.highlight = None;
    }

    /// The selected group's items (first, the rest) as a held group: their origins and turns in the first item's frame;
    /// the cursor on the group's middle. `copies`: new items (LB + X) instead of the world's own.
    fn hold_group(&mut self, copies: bool) -> Option<Vec<u32>> {
        let placed: Vec<Placed> = self.group.iter().filter_map(|id| self.scene.get(*id).cloned()).collect();
        let first = placed.first()?.clone();
        let template = |item: &str| self.catalog.item(item).and_then(Item::held_as);
        let centre = placed.iter().map(|p| p.position + p.rotation * self.pivot(&p.item, p.scale)).sum::<Vec3>() / placed.len() as f32;
        let inverse = first.rotation.inverse();
        let others = placed[1..]
            .iter()
            .filter_map(|p| {
                Some(Member {
                    item: p.item.clone(),
                    template: template(&p.item)?,
                    source: (!copies).then_some((p.id, p.position, p.rotation)),
                    offset: inverse * (p.position - first.position),
                    rotation: inverse * p.rotation,
                    scale: p.scale,
                    tint: p.tint,
                })
            })
            .collect();
        self.held = Some(Held {
            item: first.item.clone(),
            template: template(&first.item)?,
            rotation: first.rotation,
            source: (!copies).then_some((first.id, first.position, first.rotation)),
            pivot: inverse * (centre - first.position),
            tint: first.tint,
            others,
        });
        self.cursor.position = centre;
        self.cursor.lift = 0.0;
        Some(placed.iter().map(|p| p.id).collect())
    }

    /// Group select's X (8288D418): the group in hand.
    fn take_group(&mut self, events: &mut Vec<Event>) {
        self.history.push(&self.scene);
        let Some(ids) = self.hold_group(false) else { return };
        self.group.clear();
        self.enter(Mode::Manipulate);
        events.extend(ids.into_iter().map(|id| Event::Taken(Some(id))));
    }

    /// LB + X: a copy of the group in hand, when it all fits the budgets (8288EF58; else the refusal, 825C8E20).
    fn clone_group(&mut self, events: &mut Vec<Event>) {
        let items: Vec<String> = self.group.iter().filter_map(|id| self.scene.get(*id).map(|p| p.item.clone())).collect();
        let used = self.scene.usage(&self.catalog);
        let adding = Usage::placed(&self.catalog, items.iter().filter_map(|i| self.catalog.item(i)));
        if !budget::allows(&self.limits, &used, &adding) {
            events.push(Event::Refused);
            return;
        }
        self.history.push(&self.scene);
        if self.hold_group(true).is_none() {
            return;
        }
        self.group.clear();
        self.enter(Mode::Manipulate);
        events.push(Event::Taken(None));
    }

    /// A held group's other items' poses (origin, turn) where the group stands.
    pub fn member_poses(&self) -> Vec<(Vec3, Quat)> {
        let (Some(held), Some((origin, rotation))) = (self.held.as_ref(), self.held_pose()) else { return Vec::new() };
        held.others.iter().map(|m| (origin + rotation * m.offset, rotation * m.rotation)).collect()
    }

    /// LB + A on a world item: a new copy of it in hand, when it fits the budgets (8288EF58).
    fn clone_item(&mut self, id: u32, events: &mut Vec<Event>) {
        let Some(p) = self.scene.get(id).cloned() else { return };
        let Some(template) = self.catalog.item(&p.item).filter(|i| !i.is_spawner()).and_then(Item::held_as) else { return };
        if !self.fits(&p.item) {
            events.push(Event::Refused);
            return;
        }
        self.history.push(&self.scene);
        let pivot = self.pivot(&p.item, p.scale);
        self.cursor.position = p.position + p.rotation * pivot;
        self.held = Some(Held { item: p.item, template, rotation: p.rotation, source: None, pivot, tint: p.tint, others: Vec::new() });
        self.cursor.lift = 0.0;
        self.enter(Mode::Manipulate);
        events.push(Event::Taken(None));
    }

    /// Takes a world item in hand: the cursor goes to it.
    fn take(&mut self, id: u32, events: &mut Vec<Event>) {
        let Some(p) = self.scene.get(id).cloned() else { return };
        let Some(template) = self.catalog.item(&p.item).and_then(Item::held_as) else { return };
        // The snapshot before a pick-up (825CBB18).
        self.history.push(&self.scene);
        let pivot = self.pivot(&p.item, p.scale);
        self.cursor.position = p.position + p.rotation * pivot;
        self.held = Some(Held { item: p.item, template, rotation: p.rotation, source: Some((id, p.position, p.rotation)), pivot, tint: p.tint, others: Vec::new() });
        self.cursor.lift = 0.0;
        self.enter(Mode::Manipulate);
        events.push(Event::Taken(Some(id)));
    }

    /// A: the held item goes into the world where it is; with LB a copy stays in hand when it fits.
    fn place(&mut self, duplicate: bool, events: &mut Vec<Event>) {
        if self.collision && self.blocked {
            events.push(Event::Refused);
            return;
        }
        let Some(position) = self.held_pose().map(|(p, _)| p) else { return };
        let Some(held) = self.held.take() else { return };
        let id = match held.source {
            Some((id, ..)) => {
                self.scene.set_pose(id, position, held.rotation);
                id
            }
            None => self.scene.drop_item(&held.item, position, held.rotation),
        };
        self.scene.set_tint(id, held.tint);
        // A held group's other items, where the group stands.
        let rotation = held.rotation;
        for m in &held.others {
            let (at, turn) = (position + rotation * m.offset, rotation * m.rotation);
            let mid = match m.source {
                Some((mid, ..)) => {
                    self.scene.set_pose(mid, at, turn);
                    mid
                }
                None => self.scene.drop_item(&m.item, at, turn),
            };
            self.scene.set_tint(mid, m.tint);
            events.push(Event::Placed(mid));
        }
        events.push(Event::Placed(id));
        self.post(screen::REFRESH_FENG_SHUI);
        if duplicate && !self.is_spawner(&held.item) {
            if self.fits(&held.item) {
                let others = held.others.iter().map(|m| Member { source: None, ..m.clone() }).collect();
                self.held = Some(Held { source: None, others, ..held });
                self.enter(Mode::Manipulate);
                events.push(Event::Taken(None));
                return;
            }
            events.push(Event::Refused);
        }
        self.enter(Mode::FreeCam);
    }

    /// Y: the held item leaves the world (a new one is just dropped from the hand).
    fn delete(&mut self, events: &mut Vec<Event>) {
        if self.held.as_ref().is_some_and(|h| self.is_spawner(&h.item)) {
            return;
        }
        if let Some(held) = self.held.take() {
            for id in held.source.map(|s| s.0).into_iter().chain(held.others.iter().filter_map(|m| m.source.map(|s| s.0))) {
                self.scene.remove(id);
                events.push(Event::Removed(id));
            }
        }
        self.enter(Mode::FreeCam);
        self.post(screen::DELETE);
    }

    /// L3: retail's reset of an item: an authored piece goes back on its layout pose, a dropped one is removed.
    fn reset(&mut self, id: u32, events: &mut Vec<Event>) {
        if self.scene.reset(id) {
            events.push(Event::Reset(id));
        } else {
            events.push(Event::Removed(id));
        }
    }

    /// B while holding: a world item goes back to the pose it had, a new one is dropped from the hand.
    fn put_back(&mut self, events: &mut Vec<Event>) {
        if let Some(held) = self.held.take() {
            for (id, position, rotation) in held.source.into_iter().chain(held.others.iter().filter_map(|m| m.source)) {
                self.scene.set_pose(id, position, rotation);
                events.push(Event::Placed(id));
            }
        }
        self.enter(Mode::FreeCam);
        self.post(screen::REFRESH_FENG_SHUI);
    }

    /// One fixed update from the native pad slots (`skate_core::input::xbox::convert`): the buttons, then the camera
    /// turns and zooms and the cursor moves, or the held item turns or steps (the catalogue freezes both; the pad
    /// is still polled). Returns what happened.
    pub fn update(&mut self, slots: &[f32; 24], ground: &dyn Ground) -> Vec<Event> {
        let mut input = self.controls.read(slots);
        if self.variant == Variant::World {
            // No locked axis or fine tune in the open world (the movie keeps both disabled, `825C66D8` ignores them).
            input.lock = false;
        }
        let mut events = Vec::new();
        let p = input.pressed;
        if self.info && self.mode == Mode::Catalog {
            self.info = false;
            self.post(screen::REFRESH_INFO);
        }
        match self.mode {
            Mode::Catalog => {
                self.catalog_keys(&input, &mut events);
                return events;
            }
            Mode::Properties => {
                self.quick_keys(&input, &mut events);
                return events;
            }
            Mode::FreeCam => {
                let under = self.scene.pick(&self.catalog, self.cursor.position);
                if under != self.highlight {
                    self.highlight = under;
                    self.post(if under.is_some() { screen::SHOW_SELECTION } else { screen::HIDE_SELECTION });
                }
                // A (825CBB18 / 825C84A8): released within 200 ms it takes (with LB, copies) the item under the
                // cursor; held longer it starts group select with it.
                if self.a_hold > 0 {
                    if input.select {
                        self.a_hold += 1;
                        if self.a_hold >= GROUP_HOLD {
                            self.a_hold = 0;
                            self.group = under.into_iter().filter(|id| !self.is_spawner_id(*id)).collect();
                            self.enter(Mode::Group);
                            return events;
                        }
                    } else {
                        self.a_hold = 0;
                        match under {
                            Some(id) if input.duplicate => self.clone_item(id, &mut events),
                            Some(id) => self.take(id, &mut events),
                            None => {}
                        }
                    }
                }
                if p.a {
                    if under.is_some() {
                        self.a_hold = 1;
                    }
                } else if p.x && (under.is_none() || !input.duplicate) {
                    self.open_catalog(&mut events);
                    return events;
                } else if p.y {
                    if let Some(id) = under.filter(|id| !self.is_spawner_id(*id)) {
                        // The snapshot before a delete (825CBB18).
                        self.history.push(&self.scene);
                        self.scene.remove(id);
                        events.push(Event::Removed(id));
                        self.post(screen::REFRESH_FENG_SHUI);
                        self.post(screen::DELETE);
                    }
                } else if p.back {
                    self.back(&mut events);
                } else if p.b {
                    events.push(Event::Exit);
                    return events;
                }
            }
            // Group select (825C9818): the cursor moves as in the free camera.
            Mode::Group => {
                let under = self.scene.pick(&self.catalog, self.cursor.position);
                self.highlight = under;
                if let Some(id) = under {
                    if input.select && !self.group.contains(&id) && !self.is_spawner_id(id) {
                        self.group.push(id);
                    } else if input.delete && !input.select {
                        self.group.retain(|g| *g != id);
                    }
                }
                if p.b {
                    self.group.clear();
                    self.enter(Mode::FreeCam);
                } else if p.x && !self.group.is_empty() {
                    if input.duplicate {
                        self.clone_group(&mut events);
                    } else {
                        self.take_group(&mut events);
                    }
                } else if p.back {
                    self.back(&mut events);
                }
            }
            Mode::Manipulate => {
                if p.a {
                    self.place(input.duplicate, &mut events);
                } else if p.y {
                    self.delete(&mut events);
                } else if p.l3 {
                    if let Some(held) = self.held.take() {
                        for id in held.source.map(|s| s.0).into_iter().chain(held.others.iter().filter_map(|m| m.source.map(|s| s.0))) {
                            self.reset(id, &mut events);
                        }
                    }
                    self.enter(Mode::FreeCam);
                } else if p.x {
                    self.open_catalog(&mut events);
                    return events;
                } else if p.back {
                    self.back(&mut events);
                } else if p.b {
                    self.put_back(&mut events);
                }
            }
        }
        if input.duplicate != self.duplicate {
            self.duplicate = input.duplicate;
            self.post(screen::REFRESH_DUPLICATE);
        }
        if self.mode == Mode::Manipulate && self.variant == Variant::Park && input.advanced != self.fine_tune {
            self.fine_tune = input.advanced;
            self.post(screen::REFRESH_FINE_TUNE);
        }
        let mode = self.mode_tuning().clone();
        let before = self.cursor.position;
        // The camera turns where it stands: a turn moves the cursor to where it now looks (the screen's middle). A fixed
        // camera does not zoom either.
        let (eye, turn) = (self.eye(), (self.camera.yaw, self.camera.pitch));
        let mut turning = input;
        if self.fixed_eye.is_some() {
            turning.zoom = [false; 2];
        }
        self.camera.update(&self.tuning, &mode, &turning, self.invert);
        if self.fixed_eye.is_none() && (self.camera.yaw, self.camera.pitch) != turn {
            let ahead = self.camera.ground_forward() * self.camera.reach(&self.tuning);
            self.cursor.position = Vec3::new(eye.x + ahead.x, self.cursor.position.y, eye.z + ahead.z);
        }
        let shift = if self.mode == Mode::Manipulate && self.held.is_some() { self.shift.next(&input) } else { Shift::Move };
        if shift != self.shift {
            self.steps = 0;
            // Entering Rotate or Lock Axis makes the snapped pose the real one (`8288AB50`).
            if shift != Shift::Move {
                self.commit_snap(ground);
            }
            let modes: &[u8] = match shift {
                Shift::Rotate => &[screen::SHOW_ROTATE, screen::ROTATE_OBJECT],
                Shift::LockedAxis => &[screen::SHOW_LOCK_AXIS, screen::MOVE_ON_LOCKED_AXIS],
                Shift::Move => &[screen::MANIPULATE],
            };
            self.posts.extend_from_slice(modes);
        }
        self.shift = shift;
        match (shift, self.held.as_mut()) {
            (Shift::Rotate, Some(held)) => manipulate::rotate(held, &mode, &self.camera, &input),
            (Shift::LockedAxis, Some(held)) => {
                self.cursor.position += manipulate::locked_move(held, &self.tuning, &mode, &self.camera, &input, self.cursor_speed, &mut self.steps);
            }
            // A fixed camera: the left stick moves nothing (the d-pad still lifts); the cursor goes where it looks.
            _ if self.fixed_eye.is_some() => {
                let still = EditorInput { left: [0.0; 4], ..input };
                self.cursor.update(&self.tuning, &mode, &self.camera, &still, self.cursor_speed, self.locks, ground);
            }
            _ => {
                let rest = self.cursor.update(&self.tuning, &mode, &self.camera, &input, self.cursor_speed, self.locks, ground);
                // With an item in hand the camera keeps its distance (retail's held camera orbits at the zoom).
                if self.held.is_none() && rest != 0.0 {
                    // Straight up or down (owner, device: along its line to the cursor it also went forward): the eye keeps
                    // its place over the ground, and the cursor, where its look meets the ground, comes along under it.
                    let reach = self.camera.reach(&self.tuning);
                    self.camera.move_height(rest);
                    self.cursor.position += self.camera.ground_forward() * (self.camera.reach(&self.tuning) - reach);
                }
            }
        }
        if let Some(eye) = self.fixed_eye {
            self.aim(eye, ground);
        }
        // Snapping (`8288A5E0` / `82889F58`): solved on the updates the held piece moves (move or locked axis); LT
        // pressed while moving commits the snapped pose first. Still, the last snap stays.
        let stick: f32 = input.left.iter().map(|v| v.abs()).sum::<f32>() + input.elevate.iter().filter(|e| **e).count() as f32;
        let moved = (self.cursor.position - before).length_squared() > 1e-8;
        let moving = self.mode == Mode::Manipulate && shift != Shift::Rotate && (stick > 0.0001 || moved);
        if moving {
            if input.advanced && !self.was_advanced {
                self.commit_snap(ground);
            }
            self.was_advanced = input.advanced;
            self.snapped = self.solve_snap(input.advanced, ground);
        }
        self.sweep_eye(ground);
        events
    }

    /// Moves the eye to where the camera wants it, as retail's `82886500`: a clear move goes, whatever stands between the
    /// eye and the cursor. A stopped one with an item in hand comes in along the line to the cursor (retail's camera mode
    /// `**(+0x1670)`, the cursor's held item); with none it slides along the surface. If neither is clear the eye stays.
    ///
    /// NOT RETAIL: a floor or a roof in the way, or anything in the way of a move mostly up or down, does not hold the eye:
    /// it goes to the spot it wants, or as near as the cursor sees along the line to it (owner, device: above a roof, with
    /// the cursor down on the ground past it, the camera never came back down). Walls still stop a move along the ground.
    fn sweep_eye(&mut self, ground: &dyn Ground) {
        if self.fixed_eye.is_some() {
            return;
        }
        let want = Self::out_of_the_ground(self.camera.eye(&self.tuning, self.cursor.position), ground);
        // From clear of the surface the cursor stands on: a line starting against it counts as stopped at once.
        let cursor = self.cursor.position + Vec3::Y * (camera::SWEEP_RADIUS + 0.1);
        let seen = ground.sweep(cursor, want);
        let Some(at) = self.eye_at else {
            // Where it starts is not a move: it starts on this side of whatever stands between the cursor and that spot (a
            // skater by a wall; NOT RETAIL: retail starts from the game camera's position, ours from the cursor).
            self.eye_at = Some(seen.map_or(want, |(f, _)| cursor.lerp(want, f)));
            return;
        };
        if (want - at).length_squared() < 1e-8 {
            return;
        }
        let Some((_, normal)) = ground.sweep(at, want) else {
            self.eye_at = Some(want);
            return;
        };
        let to = want - at;
        let going_up_or_down = to.y.abs() > Vec2::new(to.x, to.z).length();
        if normal.y.abs() > camera::FLOOR_NORMAL || going_up_or_down {
            self.eye_at = Some(seen.map_or(want, |(f, _)| cursor.lerp(want, f)));
            return;
        }
        let instead = if self.held.is_some() {
            seen.map(|(f, _)| cursor.lerp(want, f))
        } else {
            Some(want - normal * (want - at).dot(normal))
        };
        // Held sideways by a wall, it still goes straight up or down to the height it wants, where that is clear.
        let level = Vec3::new(at.x, want.y, at.z);
        if let Some(instead) = instead.into_iter().chain([level]).find(|i| ground.sweep(at, *i).is_none()) {
            self.eye_at = Some(instead);
        }
    }

    /// The spot the eye wants, on top of the ground if it is inside it: it stands above the cursor's ground, and the ground
    /// under the eye can be higher (owner, device: lowered beside a slope, the camera went a little underground). Inside the
    /// ground nothing faces it from below, and the surface is what faces it from above.
    fn out_of_the_ground(want: Vec3, ground: &dyn Ground) -> Vec3 {
        // A little only: a surface higher than this over a spot with no floor is a ledge, off the map's ground.
        const LOOK: f32 = 3.0;
        if ground.sweep(want, want - Vec3::Y * 100.0).is_some() {
            return want;
        }
        let above = want + Vec3::Y * LOOK;
        ground.sweep(above, want).map_or(want, |(f, _)| above.lerp(want, f))
    }

    /// The held piece's snapped origin pose (`snap`), None when nothing qualifies, snapping is off, or the held item
    /// is not a park piece (props never snap).
    fn solve_snap(&self, advanced: bool, ground: &dyn Ground) -> Option<(Vec3, Quat)> {
        // NOT RETAIL YET: a held group does not snap (retail snaps the union of its members' sockets).
        let held = self.held.as_ref().filter(|h| self.snapping && h.others.is_empty())?;
        let item = self.catalog.item(&held.item).filter(|i| i.part == Part::Terrain && !i.sockets.is_empty())?;
        let base = (self.cursor.position - held.rotation * held.pivot, held.rotation);
        let scale = held.source.and_then(|(id, ..)| self.scene.get(id)).map_or(Vec3::ONE, |p| p.scale);
        let from = snap::sockets(item, base.0, base.1, scale);
        let (lo, hi) = snap::bounds(&from)?;
        let (lo, hi) = (lo - Vec3::splat(snap::STEP), hi + Vec3::splat(snap::STEP));
        // The neighbours: the park pieces whose box meets the held piece's box grown by 0.75 m (`8287D118`).
        let mut near = snap::Sockets::default();
        for p in &self.scene.items {
            if held.source.is_some_and(|(id, ..)| id == p.id) {
                continue;
            }
            let Some(other) = self.catalog.item(&p.item).filter(|i| i.part == Part::Terrain && !i.sockets.is_empty()) else { continue };
            let [a, b] = other.cost.bounds;
            let corners = (0..8).map(|k| p.position + p.rotation * (Vec3::new([a[0], b[0]][k & 1], [a[1], b[1]][(k >> 1) & 1], [a[2], b[2]][k >> 2]) * p.scale));
            let (plo, phi) = corners.fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(l, h), c| (l.min(c), h.max(c)));
            if plo.cmple(hi).all() && phi.cmpge(lo).all() {
                let s = snap::sockets(other, p.position, p.rotation, p.scale);
                near.points.extend(s.points);
                near.faces.extend(s.faces);
            }
        }
        let held_advanced = item.advance_snap();
        let pair = snap::best_pair(&from, &near, advanced, held_advanced).or_else(|| {
            let world = snap::ground_targets(&from, |p| ground.below(p.x, p.z, p.y));
            snap::best_pair(&from, &world, advanced, held_advanced)
        })?;
        Some(snap::snapped_pose(&pair, base))
    }

    /// The snapped pose becomes the held piece's own: its turn, and the cursor on its middle (`8288AB50`).
    fn commit_snap(&mut self, ground: &dyn Ground) {
        let (Some((position, rotation)), Some(held)) = (self.snapped.take(), self.held.as_mut()) else { return };
        held.rotation = rotation;
        self.cursor.position = position + rotation * held.pivot;
        let floor = ground.below(self.cursor.position.x, self.cursor.position.z, self.cursor.position.y + 0.01).unwrap_or(self.cursor.position.y);
        self.cursor.lift = (self.cursor.position.y - floor).max(0.0);
    }

    /// Reset all (`825C6638`): the world's own scene back, nothing in hand, the free camera.
    pub fn reset_scene(&mut self, scene: Scene) {
        self.history.push(&self.scene);
        self.scene = scene;
        self.held = None;
        self.highlight = None;
        self.enter(Mode::FreeCam);
        self.post(screen::REFRESH_FENG_SHUI);
    }

    /// Where the camera is and a point it looks at.
    pub fn view(&self) -> (Vec3, Vec3) {
        let eye = self.fixed_eye.or(self.eye_at).unwrap_or_else(|| self.eye());
        (eye, eye + self.camera.look())
    }

    fn eye(&self) -> Vec3 {
        self.fixed_eye.unwrap_or_else(|| self.camera.eye(&self.tuning, self.cursor.position))
    }

    /// Stands the camera where it is now: from here it only looks around (the open world's dropper).
    pub fn fix_camera(&mut self) {
        self.fixed_eye = Some(self.eye());
    }

    /// The cursor where the fixed camera at `eye` looks: the look down to the ground under it (lifted by the d-pad).
    fn aim(&mut self, eye: Vec3, ground: &dyn Ground) {
        let forward = self.camera.ground_forward();
        let lift = self.cursor.lift;
        let mut floor = self.cursor.position.y - lift;
        let mut at = self.cursor.position;
        // The ground where the look lands moves where it lands: a few rounds settle it.
        for _ in 0..4 {
            at = eye + forward * self.camera.reach_from(&self.tuning, eye.y - (floor + lift));
            if self.tuning.follow_ground {
                if let Some(h) = ground.below(at.x, at.z, eye.y) {
                    floor = h;
                }
            }
        }
        self.cursor.position = Vec3::new(at.x, floor + lift, at.z);
    }

    /// The world item under the cursor (free camera, group select).
    pub fn highlighted(&self) -> Option<u32> {
        self.highlight
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tuning::Curve;

    struct Flat(f32);
    impl Ground for Flat {
        fn below(&self, _x: f32, _z: f32, from: f32) -> Option<f32> {
            (self.0 <= from).then_some(self.0)
        }
    }

    /// Retail's `free_cam` and camera values (the disc's records, as `data_tuning_matches_the_disc` checks).
    fn tuning() -> Tuning {
        let x = vec![0.0, 0.125, 0.25, 0.375, 0.5, 0.625, 0.75, 1.0];
        // `default`'s curve ramps with the zoom; `free_cam` and `object_spawner` override it flat.
        let ramp = Curve { x: x.clone(), y: vec![0.2, 0.3625, 0.525, 0.6875, 0.85, 1.0125, 1.175, 1.5] };
        let default = ModeTuning { move_scale: 0.5, elevate_scale: 0.15, speed_curve: ramp, pitch_rate: 0.03, yaw_rate: 0.04, zoom_rate: 0.7, rotate_rate: 0.04, along_camera: false };
        let flat = Curve { x, y: vec![1.0; 8] };
        let free_cam = ModeTuning { move_scale: 0.3, elevate_scale: 0.1, speed_curve: flat.clone(), pitch_rate: 0.02, zoom_rate: 1.0, along_camera: true, ..default.clone() };
        let object_spawner = ModeTuning { move_scale: 0.35, elevate_scale: 0.1, speed_curve: flat, ..default.clone() };
        let d = 0.017_453_292;
        Tuning {
            default: default.clone(),
            free_cam,
            manipulate: default,
            object_spawner,
            smoothing: 0.7,
            orbit_pitch: [5.0 * d, 80.0 * d],
            free_pitch: [-80.0 * d, 60.0 * d],
            start_pitch: -20.0 * d,
            start_distance: 20.0,
            distance: [5.0, 40.0],
            follow_ground: true,
            ground_step: 7.0,
            colours: vec![[1.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        }
    }

    /// One prop (2 x 2 m) and one terrain ramp (4 x 4 m, 100 triangles).
    fn catalog() -> Catalog {
        let item = |id: &str, part: Part, half: f32, triangles: u32| Item {
            id: id.into(),
            name: id.into(),
            part,
            sub: String::new(),
            template: Some(format!("T_{id}")),
            teleporter: false,
            marker: None,
            teleport: None,
            caps: Default::default(),
            sockets: Vec::new(),
            cost: catalog::Cost { triangles, draws: 1, vertex_bytes: 100, textures: vec![0], bounds: [[-half, 0.0, -half], [half, 1.0, half]], collision: triangles },
        };
        Catalog::new(
            Vec::new(),
            vec![item("PROP", Part::Props, 1.0, 10), item("RAMP", Part::Terrain, 2.0, 100)],
            [(Part::Props, vec![1000]), (Part::Terrain, vec![2000])].into(),
        )
    }

    fn slots(f: impl Fn(&mut [f32; 24])) -> [f32; 24] {
        let mut s = [0.0; 24];
        f(&mut s);
        s
    }

    /// Lets the pad settle (retail keeps a change back for three polls).
    fn settle(editor: &mut Editor) {
        for _ in 0..4 {
            editor.update(&[0.0; 24], &Flat(editor.cursor.position.y - editor.cursor.lift));
        }
    }

    #[test]
    fn a_wall_stops_the_cameras_move_and_it_goes_on_when_clear() {
        /// A wall across x = 0 facing +x, for moves that cross it from its front.
        struct Wall;
        impl Ground for Wall {
            fn below(&self, _: f32, _: f32, _: f32) -> Option<f32> {
                Some(0.0)
            }
            fn sweep(&self, from: Vec3, to: Vec3) -> Option<(f32, Vec3)> {
                (from.x > 0.0 && to.x <= 0.0).then(|| (from.x / (from.x - to.x), Vec3::X))
            }
        }
        let mut editor = Editor::open(tuning(), catalog(), Scene::default(), Vec3::new(20.0, 0.0, 0.0), std::f32::consts::FRAC_PI_2);
        editor.update(&[0.0; 24], &Wall);
        let start = editor.view().0;
        assert!(start.x > 0.0, "starts in front of the wall: {start}");
        // The cursor walks through the wall: the eye slides along it, never through.
        for step in 1..200 {
            editor.cursor.position = Vec3::new(20.0 - step as f32 * 0.5, 0.0, step as f32 * 0.1);
            editor.update(&[0.0; 24], &Wall);
            assert!(editor.view().0.x > 0.0, "the eye stays in front at step {step}");
        }
        let slid = editor.view().0;
        assert!(slid.z > start.z + 1.0, "it slid along the wall: {start} to {slid}");
        // The cursor comes back: a clear move goes, the eye is back where the camera wants it.
        editor.cursor.position = Vec3::new(30.0, 0.0, 0.0);
        editor.update(&[0.0; 24], &Wall);
        assert!((editor.view().0 - editor.eye()).length() < 1e-4);
    }

    /// Opened with the camera's spot behind a wall: it starts on the cursor's side.
    #[test]
    fn the_camera_starts_on_the_cursors_side_of_a_wall() {
        struct Wall;
        impl Ground for Wall {
            fn below(&self, _: f32, _: f32, _: f32) -> Option<f32> {
                Some(0.0)
            }
            fn sweep(&self, from: Vec3, to: Vec3) -> Option<(f32, Vec3)> {
                (from.x > 0.0 && to.x <= 0.0).then(|| (from.x / (from.x - to.x), Vec3::X))
            }
        }
        let mut editor = Editor::open(tuning(), catalog(), Scene::default(), Vec3::new(1.0, 0.0, 0.0), -std::f32::consts::FRAC_PI_2);
        assert!(editor.eye().x < 0.0, "the spot it wants is behind the wall: {}", editor.eye());
        editor.update(&[0.0; 24], &Wall);
        assert!(editor.view().0.x >= 0.0, "it starts in front: {}", editor.view().0);
    }

    /// Off the edge of something taller than a step: the cursor comes down to the ground below, and the camera with it.
    #[test]
    fn the_cursor_comes_down_off_something_tall() {
        struct Tower;
        impl Ground for Tower {
            // A tower 30 m high over x < 10, the ground at 0 beyond it.
            fn below(&self, x: f32, _: f32, from: f32) -> Option<f32> {
                Some(if x < 10.0 && from >= 30.0 { 30.0 } else { 0.0 })
            }
        }
        let mut editor = Editor::open(tuning(), catalog(), Scene::default(), Vec3::new(9.5, 30.0, 0.0), std::f32::consts::FRAC_PI_2);
        editor.update(&[0.0; 24], &Tower);
        let high = editor.view().0.y;
        assert!((editor.cursor.position.y - 30.0).abs() < 1e-3, "on top");
        // Off the far side.
        editor.cursor.position.x = 10.5;
        editor.update(&[0.0; 24], &Tower);
        assert!(editor.cursor.position.y.abs() < 1e-3, "on the ground: {}", editor.cursor.position);
        assert!(editor.view().0.y < high - 20.0, "the camera came down with it");
    }

    /// Above a roof with the cursor on the ground past its edge: the camera comes down to where it wants to be. A wall in
    /// the same place holds it (the wall test above).
    #[test]
    fn a_roof_does_not_hold_the_camera_when_the_cursor_sees_the_spot() {
        /// A roof at y 10 and a floor at y 0, met from above.
        struct Roof(std::cell::Cell<f32>);
        impl Ground for Roof {
            fn below(&self, _: f32, _: f32, _: f32) -> Option<f32> {
                Some(self.0.get())
            }
            fn sweep(&self, from: Vec3, to: Vec3) -> Option<(f32, Vec3)> {
                [10.0, 0.0].into_iter().find(|h| from.y > *h && to.y <= *h).map(|h| ((from.y - h) / (from.y - to.y), Vec3::Y))
            }
        }
        let roof = Roof(std::cell::Cell::new(20.0));
        let mut editor = Editor::open(tuning(), catalog(), Scene::default(), Vec3::new(0.0, 20.0, 0.0), 0.0);
        editor.update(&[0.0; 24], &roof);
        assert!(editor.view().0.y > 10.0);
        roof.0.set(0.0);
        editor.update(&[0.0; 24], &roof);
        assert!(editor.cursor.position.y.abs() < 1e-5);
        assert!((editor.view().0 - editor.eye()).length() < 1e-3, "down with the cursor: {} wants {}", editor.view().0, editor.eye());
    }

    /// A clear move goes even with a surface between the eye and the cursor (`82886500` does not pull the eye in for it):
    /// the camera comes back down from high up with the cursor, the owner's case on the device.
    #[test]
    fn a_clear_move_goes_with_a_wall_before_the_cursor() {
        /// The ground at the height it holds.
        struct Roof(std::cell::Cell<f32>);
        impl Ground for Roof {
            fn below(&self, _: f32, _: f32, _: f32) -> Option<f32> {
                Some(self.0.get())
            }
            // A roof at y 10, met from below by a line from the cursor to the eye; moves above it are clear.
            fn sweep(&self, from: Vec3, to: Vec3) -> Option<(f32, Vec3)> {
                (from.y < 10.0 && to.y >= 10.0).then(|| ((10.0 - from.y) / (to.y - from.y), Vec3::NEG_Y))
            }
        }
        let roof = Roof(std::cell::Cell::new(20.0));
        let mut editor = Editor::open(tuning(), catalog(), Scene::default(), Vec3::new(0.0, 20.0, 0.0), 0.0);
        editor.update(&[0.0; 24], &roof);
        let high = editor.view().0;
        assert!(high.y > 10.0);
        // The cursor comes down; the eye's move, all of it above the roof, is stopped by nothing.
        roof.0.set(12.0);
        editor.update(&[0.0; 24], &roof);
        assert!((editor.cursor.position.y - 12.0).abs() < 1e-5);
        assert!((editor.view().0.y - (high.y - 8.0)).abs() < 1e-3, "the camera came down with the cursor: {} from {}", editor.view().0, high);
    }

    #[test]
    fn stick_moves_the_cursor_on_the_ground_relative_to_the_camera() {
        let mut editor = Editor::open(tuning(), catalog(), Scene::default(), Vec3::new(0.0, 2.0, 0.0), 0.0);
        let forward = slots(|s| s[18] = 1.0);
        editor.update(&forward, &Flat(2.0));
        // Yaw 0 looks down -z; full stick moves `free_cam`'s 0.3 m (its curve is flat).
        let p = editor.cursor.position;
        assert!((p.z + 0.3).abs() < 1e-6, "{p}");
        assert!(p.x.abs() < 1e-6 && (p.y - 2.0).abs() < 1e-6);
        // A half push moves an eighth as far (the axis is cubed).
        let mut half = Editor::open(tuning(), catalog(), Scene::default(), Vec3::ZERO, 0.0);
        half.update(&slots(|s| s[18] = 0.5), &Flat(0.0));
        assert!((half.cursor.position.z / p.z - 0.125).abs() < 1e-3);
    }

    /// A 1.5 m box top over the ground at 0 (everything under it).
    struct BoxTop;
    impl Ground for BoxTop {
        fn below(&self, _x: f32, _z: f32, from: f32) -> Option<f32> {
            Some(if from >= 1.5 { 1.5 } else { 0.0 })
        }
    }

    /// On the ground with the camera above it (owner, device: the camera could not come down): D-pad Down brings the camera
    /// low over the cursor, which stays on the ground; D-pad Up takes it back, and only then lifts the cursor.
    #[test]
    fn down_on_the_ground_lowers_the_camera_and_up_brings_it_back() {
        let mut editor = Editor::open(tuning(), catalog(), Scene::default(), Vec3::ZERO, 0.0);
        settle(&mut editor);
        let high = editor.view().0;
        let start = high.y;
        assert!(start > 2.0, "{start}");
        let down = slots(|s| s[1] = 1.0);
        for _ in 0..600 {
            editor.update(&down, &Flat(0.0));
            let eye = editor.view().0;
            assert!((eye.x - high.x).abs() < 1e-3 && (eye.z - high.z).abs() < 1e-3, "straight down: {eye} from {high}");
        }
        assert!(editor.cursor.position.y.abs() < 1e-5, "the cursor stays on the ground: {}", editor.cursor.position);
        let low = editor.view().0.y;
        assert!((low - 0.8).abs() < 0.01, "down to just over the ground: {low}");
        let (eye, target) = editor.view();
        assert!((editor.cursor.position - eye).normalize().dot((target - eye).normalize()) > 0.999, "the cursor stays in the middle of the screen");
        let up = slots(|s| s[0] = 1.0);
        for _ in 0..600 {
            editor.update(&up, &Flat(0.0));
            if editor.camera.lowered_from == 0.0 {
                break;
            }
            assert_eq!(editor.cursor.lift, 0.0, "the camera goes back first");
        }
        assert!(editor.camera.lowered_from == 0.0, "it went back");
        assert!((editor.camera.height() - start).abs() < 1e-3, "to where it was: {} from {start}", editor.camera.height());
        assert!((editor.view().0 - high).length() < 1e-2, "straight back up: {} from {high}", editor.view().0);
        for _ in 0..30 {
            editor.update(&up, &Flat(0.0));
        }
        assert!(editor.cursor.lift > 0.0, "then the cursor lifts");
    }

    /// Pressed down through a piece, then off its edge onto lower ground: the cursor is on the ground, not as far under it as
    /// it was under the piece (owner, device: a little underground).
    #[test]
    fn off_a_piece_with_down_pressed_the_cursor_stays_out_of_the_ground() {
        struct Piece;
        impl Ground for Piece {
            // A piece 6 m high over x < 10, the map's ground at 0.
            fn below(&self, x: f32, _: f32, from: f32) -> Option<f32> {
                Some(if x < 10.0 && from >= 6.0 { 6.0 } else { 0.0 })
            }
            fn floor_below(&self, _: f32, _: f32, _: f32) -> Option<f32> {
                Some(0.0)
            }
        }
        let mut editor = Editor::open(tuning(), catalog(), Scene::default(), Vec3::new(9.0, 6.0, 0.0), 0.0);
        settle(&mut editor);
        let down = slots(|s| s[1] = 1.0);
        for _ in 0..600 {
            editor.update(&down, &Piece);
        }
        assert!(editor.cursor.position.y.abs() < 1e-4, "down through the piece to the ground: {}", editor.cursor.position);
        assert!(editor.cursor.lift < -5.0, "{}", editor.cursor.lift);
        editor.cursor.position.x = 11.0;
        for _ in 0..3 {
            editor.update(&[0.0; 24], &Piece);
        }
        assert!(editor.cursor.position.y.abs() < 1e-4, "still on the ground: {}", editor.cursor.position);
    }

    /// Lowered where the ground under the eye is higher than under the cursor: it stops on top of it.
    #[test]
    fn the_lowered_camera_stays_out_of_higher_ground() {
        /// The cursor's ground at 0; a surface at y 2, facing up, that the eye's spot is under.
        struct Rise;
        impl Ground for Rise {
            fn below(&self, _: f32, _: f32, _: f32) -> Option<f32> {
                Some(0.0)
            }
            fn sweep(&self, from: Vec3, to: Vec3) -> Option<(f32, Vec3)> {
                (from.y > 2.0 && to.y <= 2.0).then(|| ((from.y - 2.0) / (from.y - to.y), Vec3::Y))
            }
        }
        let mut editor = Editor::open(tuning(), catalog(), Scene::default(), Vec3::ZERO, 0.0);
        settle(&mut editor);
        let down = slots(|s| s[1] = 1.0);
        for _ in 0..600 {
            editor.update(&down, &Rise);
        }
        assert!(editor.camera.height() < 1.0, "lowered: {}", editor.camera.height());
        assert!(editor.view().0.y >= 2.0 - 1e-3, "on top of it: {}", editor.view().0);
    }

    /// With an item in hand the camera keeps its distance.
    #[test]
    fn down_on_the_ground_with_an_item_leaves_the_camera() {
        let mut editor = Editor::open(tuning(), catalog(), Scene::default(), Vec3::ZERO, 0.0);
        settle(&mut editor);
        editor.held = Some(Held { item: String::new(), template: String::new(), rotation: Quat::IDENTITY, source: None, pivot: Vec3::ZERO, tint: 0, others: Vec::new() });
        editor.mode = Mode::Manipulate;
        let height = editor.camera.height();
        let down = slots(|s| s[1] = 1.0);
        for _ in 0..120 {
            editor.update(&down, &Flat(0.0));
        }
        assert!((editor.camera.height() - height).abs() < 1e-5);
    }

    /// Pieces stacked over the map's ground: D-pad Down goes through them to the ground, and no further.
    #[test]
    fn down_goes_through_pieces_to_the_map_and_no_further() {
        struct Stack;
        impl Ground for Stack {
            fn below(&self, _: f32, _: f32, from: f32) -> Option<f32> {
                [20.0, 10.0, 0.0, -4.0].into_iter().find(|h| from >= *h)
            }
            fn floor_below(&self, _: f32, _: f32, _: f32) -> Option<f32> {
                Some(0.0)
            }
        }
        let mut editor = Editor::open(tuning(), catalog(), Scene::default(), Vec3::new(0.0, 20.0, 0.0), 0.0);
        settle(&mut editor);
        let down = slots(|s| s[1] = 1.0);
        for _ in 0..600 {
            editor.update(&down, &Stack);
        }
        assert!(editor.cursor.position.y.abs() < 1e-3, "{}", editor.cursor.position);
    }

    #[test]
    fn down_takes_the_cursor_under_a_higher_surface_to_the_next() {
        let mut editor = Editor::open(tuning(), catalog(), Scene::default(), Vec3::new(0.0, 1.5, 0.0), 0.0);
        settle(&mut editor);
        // On the box top, D-pad Down held: the cursor goes on down past the top, to the ground and no further.
        let down = slots(|s| s[1] = 1.0);
        for _ in 0..120 {
            editor.update(&down, &BoxTop);
        }
        assert!(editor.cursor.position.y.abs() < 1e-4, "{}", editor.cursor.position);
    }

    #[test]
    fn the_cursor_follows_the_ground_and_the_dpad_lifts_it() {
        let mut editor = Editor::open(tuning(), catalog(), Scene::default(), Vec3::ZERO, 0.0);
        editor.update(&slots(|s| s[16] = 1.0), &Flat(1.5));
        assert!((editor.cursor.position.y - 1.5).abs() < 1e-6, "a step up within 7 m is followed");
        settle(&mut editor);
        let up = slots(|s| s[0] = 1.0);
        let mut heights = Vec::new();
        // The press shows on the pad after its three held-back polls.
        for _ in 0..3 {
            editor.update(&up, &Flat(1.5));
        }
        for _ in 0..4 {
            editor.update(&up, &Flat(1.5));
            heights.push(editor.cursor.lift);
        }
        // The ramp grows while the d-pad is held: each step is larger than the last.
        assert!(heights.windows(3).all(|w| w[2] - w[1] > w[1] - w[0]), "{heights:?}");
        // Released, the ramp rests again and the lift stays.
        let lift = editor.cursor.lift;
        for _ in 0..4 {
            editor.update(&[0.0; 24], &Flat(1.5));
        }
        assert!(editor.cursor.lift >= lift);
        assert_eq!(editor.cursor.ramp, 0.3);
        let lift = editor.cursor.lift;
        // A drop of more than 7 m is followed too, the lift kept (owner, device: the cursor stayed up off a tall edge).
        editor.update(&slots(|s| s[16] = 1.0), &Flat(-10.0));
        assert!((editor.cursor.position.y - (-10.0 + lift)).abs() < 1e-5);
    }

    /// The open world's camera (owner, on retail): it stands where it opens and looks around; the left stick and the
    /// zoom move nothing; the cursor stays at the middle of the screen, on the ground where the camera looks.
    #[test]
    fn the_open_worlds_camera_stands_still_and_looks_around() {
        let mut editor = Editor::open(tuning(), catalog(), Scene::default(), Vec3::ZERO, 0.0);
        editor.variant = Variant::World;
        editor.fix_camera();
        let (eye, _) = editor.view();
        let centred = |editor: &Editor| {
            let (eye, target) = editor.view();
            (editor.cursor.position - eye).normalize().dot((target - eye).normalize()) > 0.9999
        };
        editor.update(&slots(|s| s[16] = 1.0), &Flat(0.0));
        editor.update(&slots(|s| s[2] = 1.0), &Flat(0.0));
        assert!((editor.view().0 - eye).length() < 1e-5, "the left stick and the zoom do not move it");
        assert!(editor.cursor.position.length() < 1e-3, "the cursor stays where the camera looks");
        assert_eq!(editor.camera.target_distance, editor.tuning.start_distance);
        editor.update(&slots(|s| s[20] = 1.0), &Flat(0.0));
        editor.update(&slots(|s| s[23] = 1.0), &Flat(0.0));
        assert!((editor.view().0 - eye).length() < 1e-5, "it turns where it stands");
        assert!(centred(&editor) && editor.cursor.position.y.abs() < 1e-5, "the cursor is on the ground at the middle of the screen");
    }

    #[test]
    fn camera_turns_zooms_and_clamps_as_the_records_say() {
        let mut editor = Editor::open(tuning(), catalog(), Scene::default(), Vec3::ZERO, 0.0);
        let start = editor.camera;
        assert!((start.pitch + 20.0_f32.to_radians()).abs() < 1e-5, "it starts looking 20 degrees down");
        let (eye, _) = editor.view();
        editor.update(&slots(|s| s[20] = 1.0), &Flat(0.0));
        assert!((editor.camera.yaw + 0.04).abs() < 1e-6, "right turns right");
        assert!((editor.view().0 - eye).length() < 1e-4, "it turns where it stands (the cursor swings about it)");
        let (eye, target) = editor.view();
        let to_cursor = (editor.cursor.position - eye).normalize();
        assert!(to_cursor.dot((target - eye).normalize()) > 0.9999, "the cursor stays at the middle of the screen");
        editor.invert = [-1.0, 1.0];
        editor.update(&slots(|s| s[20] = 1.0), &Flat(0.0));
        assert!(editor.camera.yaw.abs() < 1e-6, "Invert X turns back");
        for _ in 0..1000 {
            editor.update(&slots(|s| s[22] = 1.0), &Flat(0.0));
        }
        assert!((editor.camera.pitch - 60.0_f32.to_radians()).abs() < 1e-5, "it looks up to 60 degrees");
        assert!((editor.view().0 - eye).length() < 1e-3, "looking up does not move it");
        // Zoom out (d-pad left) to the 40 m limit; the camera eases there.
        for _ in 0..100 {
            editor.update(&slots(|s| s[2] = 1.0), &Flat(0.0));
        }
        assert_eq!(editor.camera.target_distance, 40.0);
        assert!((editor.camera.distance - 40.0).abs() < 1e-3);
        // Free camera: the same 0.3 m at full zoom. Moving an item (`manipulate`, `default`'s curve): 1.5 x 0.5 m.
        let before = editor.cursor.position;
        editor.update(&slots(|s| s[16] = 1.0), &Flat(0.0));
        assert!(((editor.cursor.position - before).length() - 0.3).abs() < 1e-4);
        editor.mode = Mode::Manipulate;
        for _ in 0..100 {
            editor.update(&slots(|s| s[2] = 1.0), &Flat(0.0));
        }
        let before = editor.cursor.position;
        editor.update(&slots(|s| s[16] = 1.0), &Flat(0.0));
        assert!(((editor.cursor.position - before).length() - 0.75).abs() < 1e-4);
        // The catalogue freezes the camera and the cursor.
        editor.mode = Mode::Catalog;
        let frozen = (editor.camera, editor.cursor, editor.held.clone());
        editor.update(&slots(|s| s[16] = 1.0), &Flat(0.0));
        assert_eq!((editor.camera, editor.cursor, editor.held.clone()), frozen);
    }

    #[test]
    fn axis_locks_hold_their_axis() {
        let mut editor = Editor::open(tuning(), catalog(), Scene::default(), Vec3::ZERO, 0.0);
        editor.locks = [true, false, false];
        editor.update(&slots(|s| { s[16] = 1.0; s[18] = 1.0; }), &Flat(0.0));
        assert_eq!(editor.cursor.position.x, 0.0);
        assert!(editor.cursor.position.z < 0.0);
    }

    #[test]
    fn rt_turns_the_held_item_in_quarter_steps_and_lt_makes_it_smooth() {
        let mut editor = Editor::open(tuning(), catalog(), Scene::default(), Vec3::ZERO, 0.0);
        editor.choose("PROP", &mut Vec::new());
        settle(&mut editor);
        // RT and the stick right: the press shows after three polls, then one quarter turn on the first repeat.
        let turn = slots(|s| { s[11] = 1.0; s[16] = 1.0; });
        for _ in 0..4 {
            editor.update(&turn, &Flat(0.0));
        }
        assert_eq!(editor.shift, Shift::Rotate);
        let (_, r) = editor.held_pose().unwrap();
        let (axis, angle) = r.to_axis_angle();
        assert!((angle - std::f32::consts::FRAC_PI_2).abs() < 1e-5 && (axis - Vec3::Y).length() < 1e-5, "{axis} {angle}");
        // Held on, the next quarter comes after 24 polls, then every 12; the item did not move.
        for _ in 0..24 {
            editor.update(&turn, &Flat(0.0));
        }
        assert!((editor.held_pose().unwrap().1.to_axis_angle().1 - std::f32::consts::PI).abs() < 1e-4);
        assert_eq!(editor.cursor.position, Vec3::ZERO);
        // With LT: 0.04 rad an update at full stick.
        let smooth = slots(|s| { s[10] = 1.0; s[11] = 1.0; s[16] = 1.0; });
        for _ in 0..3 {
            editor.update(&smooth, &Flat(0.0));
        }
        let before = editor.held_pose().unwrap().1;
        editor.update(&smooth, &Flat(0.0));
        let step = (editor.held_pose().unwrap().1 * before.inverse()).to_axis_angle().1;
        assert!((step - 0.04).abs() < 1e-4, "{step}");
        // Letting go of RT returns to the plain move.
        for _ in 0..4 {
            editor.update(&[0.0; 24], &Flat(0.0));
        }
        assert_eq!(editor.shift, Shift::Move);
    }

    #[test]
    fn rb_steps_the_held_item_along_its_own_axes() {
        let mut editor = Editor::open(tuning(), catalog(), Scene::default(), Vec3::ZERO, 0.0);
        editor.choose("PROP", &mut Vec::new());
        // Turned a quarter: its own x is the world's -z.
        editor.held.as_mut().unwrap().rotation = Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);
        settle(&mut editor);
        let push = slots(|s| { s[9] = 1.0; s[16] = 1.0; });
        let mut positions = Vec::new();
        for _ in 0..(4 + 24 + 12 + 12 + 12) {
            editor.update(&push, &Flat(0.0));
            positions.push(editor.cursor.position);
        }
        assert_eq!(editor.shift, Shift::LockedAxis);
        let p = editor.cursor.position;
        // Five steps: 0.75 three times, then 2.25 twice, all along the item's x (world -z).
        assert!(p.x.abs() < 1e-5 && (p.z + (3.0 * 0.75 + 2.0 * 2.25)).abs() < 1e-4, "{p}");
        // Released and pushed again, the steps start short again.
        for _ in 0..4 {
            editor.update(&slots(|s| s[9] = 1.0), &Flat(0.0));
        }
        assert_eq!(editor.steps, 0);
    }

    /// Presses a button: held through the pad's held-back polls, then let go.
    fn press(editor: &mut Editor, slot: usize, extra: &[usize]) -> Vec<Event> {
        let mut events = Vec::new();
        let s = slots(|s| { s[slot] = 1.0; for &e in extra { s[e] = 1.0; } });
        for _ in 0..4 {
            events.extend(editor.update(&s, &Flat(0.0)));
        }
        let hold = slots(|s| for &e in extra { s[e] = 1.0; });
        for _ in 0..4 {
            events.extend(editor.update(&hold, &Flat(0.0)));
        }
        events
    }

    const A: usize = 12;
    const B: usize = 13;
    const X: usize = 14;
    const Y: usize = 15;
    const L3: usize = 6;
    const LB: usize = 8;

    #[test]
    fn the_button_held_when_it_opens_is_not_a_press() {
        let mut editor = Editor::open(tuning(), catalog(), Scene::default(), Vec3::ZERO, 0.0);
        let b_held = slots(|s| s[B] = 1.0);
        editor.prime(&b_held);
        for _ in 0..30 {
            assert!(editor.update(&b_held, &Flat(0.0)).is_empty(), "B kept down from the phone does not leave");
        }
        // Let go and pressed again: it leaves.
        settle(&mut editor);
        assert_eq!(press(&mut editor, B, &[]), vec![Event::Exit]);
    }

    #[test]
    fn place_take_delete_and_put_back() {
        let mut editor = Editor::open(tuning(), catalog(), Scene::default(), Vec3::ZERO, 0.0);
        settle(&mut editor);
        assert_eq!(press(&mut editor, X, &[]), vec![Event::OpenCatalog]);
        let mut events = Vec::new();
        editor.choose("RAMP", &mut events);
        assert_eq!((events, editor.mode), (vec![Event::Taken(None)], Mode::Manipulate));
        // The meter shows what the ramp would add.
        assert!(editor.meter().terrain[1] > 0.0);
        let placed = press(&mut editor, A, &[]);
        let id = match placed[..] { [Event::Placed(id)] => id, _ => panic!("{placed:?}") };
        assert_eq!((editor.mode, editor.scene.items.len()), (Mode::FreeCam, 1));
        assert!(editor.meter().terrain[0] > 0.0 && editor.meter().terrain[1] == 0.0);
        // A over it takes it; moved and put back with B, it returns.
        assert_eq!(press(&mut editor, A, &[]), vec![Event::Taken(Some(id))]);
        for _ in 0..10 {
            editor.update(&slots(|s| s[16] = 1.0), &Flat(0.0));
        }
        press(&mut editor, B, &[]);
        assert_eq!(editor.scene.get(id).unwrap().position, Vec3::ZERO);
        // Taken again (the cursor back over it) and deleted with Y.
        editor.cursor.position = Vec3::ZERO;
        assert_eq!(press(&mut editor, A, &[]), vec![Event::Taken(Some(id))]);
        assert_eq!(press(&mut editor, Y, &[]), vec![Event::Removed(id)]);
        assert!(editor.scene.items.is_empty());
        // B in the free camera leaves the editor.
        assert_eq!(press(&mut editor, B, &[]), vec![Event::Exit]);
    }

    #[test]
    fn lb_places_a_copy_until_the_budget_refuses() {
        let mut editor = Editor::open(tuning(), catalog(), Scene::default(), Vec3::ZERO, 0.0);
        editor.limits = Limits { props_dropped: 3, props_live: 3, ..Limits::default() };
        settle(&mut editor);
        editor.choose("PROP", &mut Vec::new());
        let mut placed = 0;
        let mut refused = false;
        for _ in 0..5 {
            let events = press(&mut editor, A, &[LB]);
            placed += events.iter().filter(|e| matches!(e, Event::Placed(_))).count();
            refused |= events.contains(&Event::Refused);
            if editor.held.is_none() {
                break;
            }
        }
        // Three props fit: the third placement finds no room for a fourth copy.
        assert_eq!((placed, refused, editor.scene.items.len(), editor.mode), (3, true, 3, Mode::FreeCam));
        // The catalogue refuses a fourth too.
        let mut events = Vec::new();
        editor.choose("PROP", &mut events);
        assert_eq!(events, vec![Event::Refused]);
    }

    #[test]
    fn l3_resets_an_authored_piece_and_removes_a_dropped_item() {
        let rotation = Quat::from_rotation_y(0.5);
        let scene = Scene::from_layout([(7, "RAMP".to_owned(), scene::affine(Vec3::new(10.0, 0.0, 0.0), rotation, Vec3::ONE))]);
        let mut editor = Editor::open(tuning(), catalog(), scene, Vec3::new(10.0, 0.0, 0.0), 0.0);
        settle(&mut editor);
        // Take the layout piece, move it, place it: it remembers its layout pose.
        press(&mut editor, A, &[]);
        for _ in 0..10 {
            editor.update(&slots(|s| s[16] = 1.0), &Flat(0.0));
        }
        press(&mut editor, A, &[]);
        let moved = editor.scene.get(7).unwrap().position;
        assert!(moved.x > 10.0);
        // Taken again, L3 puts it back on the layout pose (the free camera has no reset: `825CBB18`).
        assert!(press(&mut editor, L3, &[]).is_empty());
        assert_eq!(press(&mut editor, A, &[]), vec![Event::Taken(Some(7))]);
        assert_eq!(press(&mut editor, L3, &[]), vec![Event::Reset(7)]);
        assert_eq!(editor.scene.get(7).unwrap().position, Vec3::new(10.0, 0.0, 0.0));
        // A dropped item resets out of the world.
        editor.choose("PROP", &mut Vec::new());
        let id = match press(&mut editor, A, &[])[..] { [Event::Placed(id)] => id, ref e => panic!("{e:?}") };
        assert_eq!(press(&mut editor, A, &[]), vec![Event::Taken(Some(id))]);
        assert_eq!(press(&mut editor, L3, &[]), vec![Event::Removed(id)]);
        assert!(editor.scene.get(id).is_none());
    }

    #[test]
    fn the_catalogue_walks_previews_and_takes_as_the_screen_does() {
        const DOWN: usize = 1;
        const BACK: usize = 5;
        const RT: usize = 11;
        let mut editor = Editor::open(tuning(), catalog(), Scene::default(), Vec3::ZERO, 0.0);
        let listing = vec![("terrain".to_owned(), vec![("ramps".to_owned(), vec!["RAMP".to_owned()])]), ("props".to_owned(), vec![("misc".to_owned(), vec!["PROP".to_owned()])])];
        editor.set_listing(listing);
        settle(&mut editor);
        editor.take_posts();
        // X: the catalogue on its sub category list, nothing previewed.
        assert_eq!(press(&mut editor, X, &[]), vec![Event::OpenCatalog]);
        assert_eq!((editor.mode, editor.held.is_none()), (Mode::Catalog, true));
        // RT: the props; A opens the sub category and previews its item at the cursor.
        press(&mut editor, RT, &[]);
        press(&mut editor, A, &[]);
        assert_eq!(editor.held.as_ref().map(|h| (h.item.as_str(), h.source)), Some(("PROP", None)));
        // Down on one item does not move; the size shows once the preview is in.
        press(&mut editor, DOWN, &[]);
        assert_eq!(
            editor.take_posts(),
            vec![screen::CATALOG, screen::NEXT_CATEGORY, screen::GOTO_ITEMS, screen::REFRESH_INFO]
        );
        // B: back to the sub categories (the preview goes), B again: out.
        press(&mut editor, B, &[]);
        assert!(editor.held.is_none() && editor.browse.subs);
        assert_eq!(press(&mut editor, B, &[]), vec![Event::CloseCatalog]);
        assert_eq!(editor.take_posts(), vec![screen::GOTO_SUB_CATEGORIES, screen::FREE_CAM]);
        // Back again into the item list: A takes the previewed item in hand.
        press(&mut editor, X, &[]);
        press(&mut editor, A, &[]);
        assert_eq!(press(&mut editor, A, &[]), vec![Event::Taken(None)]);
        assert_eq!(editor.mode, Mode::Manipulate);
        assert_eq!(editor.take_posts(), vec![screen::CATALOG, screen::GOTO_ITEMS, screen::REFRESH_INFO, screen::MANIPULATE]);
        let id = match press(&mut editor, A, &[])[..] { [Event::Placed(id)] => id, ref e => panic!("{e:?}") };
        assert_eq!(editor.take_posts()[..2], [screen::REFRESH_FENG_SHUI, screen::FREE_CAM]);
        settle(&mut editor);
        editor.take_posts();
        // The free camera (the placed item highlighted): Y deletes the item under the cursor; Back is Properties in the
        // park editor.
        assert!(editor.take_posts().is_empty());
        assert_eq!(press(&mut editor, Y, &[]), vec![Event::Removed(id)]);
        assert_eq!(editor.take_posts(), vec![screen::REFRESH_FENG_SHUI, screen::DELETE, screen::HIDE_SELECTION]);
        assert_eq!(press(&mut editor, BACK, &[]), vec![Event::Properties]);
        assert_eq!(editor.mode, Mode::Properties);
        press(&mut editor, B, &[]);
        assert_eq!(editor.mode, Mode::FreeCam);
        // The open world asks to reset instead, and has no locked axis.
        editor.variant = Variant::World;
        assert_eq!(press(&mut editor, BACK, &[]), vec![Event::ConfirmReset]);
        editor.choose("PROP", &mut Vec::new());
        editor.take_posts();
        for _ in 0..6 {
            editor.update(&slots(|s| s[9] = 1.0), &Flat(0.0));
        }
        assert!(editor.shift == Shift::Move && editor.take_posts().is_empty());
    }

    #[test]
    fn a_held_item_turns_and_moves_about_its_middle() {
        // A 4 x 2 m piece whose model origin is its corner.
        let mut catalog = catalog();
        let mut corner = catalog.items[1].clone();
        corner.id = "CORNER".into();
        corner.cost.bounds = [[0.0, 0.0, 0.0], [4.0, 1.0, 2.0]];
        catalog = Catalog::new(Vec::new(), vec![catalog.items[0].clone(), catalog.items[1].clone(), corner], catalog.texture_bytes.clone());
        let scene = Scene::from_layout([(9, "CORNER".to_owned(), scene::affine(Vec3::new(10.0, 0.0, 0.0), Quat::IDENTITY, Vec3::ONE))]);
        let mut editor = Editor::open(tuning(), catalog, scene, Vec3::new(12.0, 0.0, 1.0), 0.0);
        settle(&mut editor);
        assert_eq!(press(&mut editor, A, &[]), vec![Event::Taken(Some(9))]);
        // The cursor goes to the piece's middle, not its corner.
        assert_eq!(editor.cursor.position, Vec3::new(12.0, 0.0, 1.0));
        // A half turn about the middle: the corner moves to the far side, the middle stays.
        editor.held.as_mut().unwrap().rotation = Quat::from_rotation_y(std::f32::consts::PI);
        let (origin, _) = editor.held_pose().unwrap();
        assert!((origin - Vec3::new(14.0, 0.0, 2.0)).length() < 1e-4, "{origin}");
        press(&mut editor, A, &[]);
        assert!((editor.scene.get(9).unwrap().position - Vec3::new(14.0, 0.0, 2.0)).length() < 1e-4);
    }

    #[test]
    fn the_spawner_moves_but_is_never_deleted_copied_or_grouped() {
        // The spawner: no model, a 6 m footprint from its corner, drawn as its marker.
        let mut catalog = catalog();
        let mut spawner = catalog.items[1].clone();
        (spawner.id, spawner.sub, spawner.template, spawner.marker) = ("SPAWN".into(), catalog::SPAWNER_SUB.into(), None, Some("MARK".into()));
        spawner.cost.bounds = [[0.0, 0.0, -6.0], [6.0, 3.0, 0.0]];
        catalog = Catalog::new(Vec::new(), vec![catalog.items[0].clone(), catalog.items[1].clone(), spawner], catalog.texture_bytes.clone());
        let id = scene::SPAWNER_ID;
        let scene = Scene::from_layout([(id, "SPAWN".to_owned(), scene::affine(Vec3::ZERO, Quat::IDENTITY, Vec3::ONE))]);
        let mut editor = Editor::open(tuning(), catalog, scene, Vec3::new(3.0, 0.0, -3.0), 0.0);
        settle(&mut editor);
        assert_eq!(editor.highlighted(), Some(id));
        // Y: still there. LB + A: no copy. A held: no group.
        assert!(press(&mut editor, Y, &[]).is_empty());
        assert!(press(&mut editor, A, &[LB]).is_empty());
        for _ in 0..GROUP_HOLD + 2 {
            editor.update(&slots(|s| s[A] = 1.0), &Flat(0.0));
        }
        assert!(editor.group.is_empty());
        press(&mut editor, B, &[]);
        // A: in hand (as its marker); Y keeps it; moved and placed.
        settle(&mut editor);
        assert_eq!(press(&mut editor, A, &[]), vec![Event::Taken(Some(id))]);
        assert_eq!(editor.held.as_ref().map(|h| h.template.as_str()), Some("MARK"));
        press(&mut editor, Y, &[]);
        assert!(editor.held.is_some());
        editor.cursor.position = Vec3::new(13.0, 0.0, -3.0);
        assert_eq!(press(&mut editor, A, &[]), vec![Event::Placed(id)]);
        assert!((editor.scene.get(id).unwrap().position - Vec3::new(10.0, 0.0, 0.0)).length() < 1e-4);
    }

    #[test]
    fn a_blocked_item_is_not_placed_unless_collision_is_off() {
        let mut editor = Editor::open(tuning(), catalog(), Scene::default(), Vec3::ZERO, 0.0);
        settle(&mut editor);
        editor.choose("RAMP", &mut Vec::new());
        // The world's check found it inside another item: A is refused, it stays in hand.
        editor.blocked = true;
        assert_eq!(press(&mut editor, A, &[]), vec![Event::Refused]);
        assert!(editor.held.is_some());
        // Collision off: it goes in.
        editor.collision = false;
        assert!(matches!(press(&mut editor, A, &[])[..], [Event::Placed(_)]));
    }

    #[test]
    fn rotation_steps_stay_square_whatever_the_camera_angle() {
        // The camera at 30 degrees: tilts and turns in any order leave every item axis on a world axis.
        let editor = Editor::open(tuning(), catalog(), Scene::default(), Vec3::ZERO, 0.5236);
        let mode = editor.mode_tuning().clone();
        let mut held = Held { item: "PROP".into(), template: String::new(), rotation: Quat::IDENTITY, source: None, pivot: Vec3::ZERO, tint: 0, others: Vec::new() };
        // Right (turn), Down (tilt), Left, Up, Right, Down: stick and step on the same direction.
        for (k, &dir) in [1, 3, 0, 2, 1, 3].iter().enumerate() {
            let mut input = EditorInput::default();
            input.left[dir] = 1.0;
            input.step[dir] = true;
            manipulate::rotate(&mut held, &mode, &editor.camera, &input);
            for axis in [Vec3::X, Vec3::Y, Vec3::Z] {
                let a = (held.rotation * axis).abs();
                assert!((a.max_element() - 1.0).abs() < 1e-4, "{a} after step {k}");
            }
            if k == 1 {
                // The tilt really tilted: the item's up now lies flat.
                assert!((held.rotation * Vec3::Y).y.abs() < 1e-4);
            }
        }
    }

    #[test]
    fn the_quick_menu_sets_options_copies_pastes_and_undoes() {
        const BACK: usize = 5;
        const DOWN: usize = 1;
        const RIGHT: usize = 3;
        let mut editor = Editor::open(tuning(), catalog(), Scene::default(), Vec3::ZERO, 0.0);
        settle(&mut editor);
        // Opens with nothing selected: Copy, Paste, Undo and Redo are off.
        assert_eq!(press(&mut editor, BACK, &[]), vec![Event::Properties]);
        assert_eq!(editor.quick_rows(), [true, true, true, true, false, false, false, false]);
        // Down three times: Options; A, then Right on Snapping and on Cursor speed (two rows down in the C++ order).
        for _ in 0..3 {
            press(&mut editor, DOWN, &[]);
        }
        press(&mut editor, A, &[]);
        assert_eq!(editor.quick.page, quick::Page::Options);
        press(&mut editor, RIGHT, &[]);
        assert!(!editor.snapping);
        for _ in 0..3 {
            press(&mut editor, DOWN, &[]);
        }
        press(&mut editor, RIGHT, &[]);
        assert!((editor.cursor_speed - 1.1).abs() < 1e-6);
        press(&mut editor, B, &[]);
        press(&mut editor, B, &[]);
        assert_eq!(editor.mode, Mode::FreeCam);
        // A prop placed, highlighted, copied, pasted: a copy in hand, placed.
        editor.choose("PROP", &mut Vec::new());
        press(&mut editor, A, &[]);
        settle(&mut editor);
        press(&mut editor, BACK, &[]);
        editor.quick.row = quick::COPY;
        press(&mut editor, A, &[]);
        assert_eq!(editor.clipboard.as_ref().map(|c| c.0.as_str()), Some("PROP"));
        press(&mut editor, BACK, &[]);
        editor.quick.row = quick::PASTE;
        assert_eq!(press(&mut editor, A, &[]), vec![Event::Taken(None)]);
        editor.cursor.position = Vec3::new(5.0, 0.0, 0.0);
        press(&mut editor, A, &[]);
        assert_eq!(editor.scene.items.len(), 2);
        // Undo: the paste goes; again: the first prop goes; Redo brings it back.
        press(&mut editor, BACK, &[]);
        editor.quick.row = quick::UNDO;
        let events = press(&mut editor, A, &[]);
        assert_eq!((editor.scene.items.len(), events.iter().filter(|e| matches!(e, Event::Removed(_))).count()), (1, 1));
        press(&mut editor, BACK, &[]);
        editor.quick.row = quick::UNDO;
        press(&mut editor, A, &[]);
        assert!(editor.scene.items.is_empty());
        press(&mut editor, BACK, &[]);
        editor.quick.row = quick::REDO;
        let events = press(&mut editor, A, &[]);
        assert_eq!((editor.scene.items.len(), events.iter().filter(|e| matches!(e, Event::Placed(_))).count()), (1, 1));
    }

    #[test]
    fn the_style_page_swaps_the_selection_live_and_b_reverts() {
        const BACK: usize = 5;
        let base = catalog();
        let style = |id: &str, name: &str| {
            let mut i = base.items[1].clone();
            i.id = id.into();
            i.name = name.into();
            i.sub = "street".into();
            i.template = Some(format!("T_{id}"));
            i
        };
        let catalog = Catalog::new(
            Vec::new(),
            vec![base.items[0].clone(), style("DT", "aa_pad_md_Downtown"), style("IN", "aa_pad_md_Industrial"), style("UN", "aa_pad_md_University")],
            base.texture_bytes.clone(),
        );
        let scene = Scene::from_layout([(4, "DT".to_owned(), scene::affine(Vec3::ZERO, Quat::IDENTITY, Vec3::ONE))]);
        let mut editor = Editor::open(tuning(), catalog, scene, Vec3::ZERO, 0.0);
        settle(&mut editor);
        settle(&mut editor);
        assert!(editor.highlight.is_some(), "the piece under the cursor");
        press(&mut editor, BACK, &[]);
        assert!(editor.style_allowed());
        assert_eq!(editor.style_index(), 2, "downtown is the third style row");
        editor.quick.row = quick::STYLE;
        press(&mut editor, A, &[]);
        assert_eq!(editor.quick.page, quick::Page::Style);
        // Highlight Industrial: the piece is replaced by its Industrial variant where it stood.
        let events = editor.style_highlight(1);
        assert!(matches!(events[..], [Event::Removed(4), Event::Placed(_)]), "{events:?}");
        assert_eq!(editor.highlight.and_then(|id| editor.scene.get(id)).map(|p| p.item.as_str()), Some("IN"));
        // B: back as it was.
        let events = press(&mut editor, B, &[]);
        assert_eq!(editor.scene.items.iter().map(|p| p.item.as_str()).collect::<Vec<_>>(), vec!["DT"]);
        assert!(events.contains(&Event::Placed(4)), "{events:?}");
        assert_eq!(editor.quick.page, quick::Page::Main);
        // Again, University kept with Select.
        press(&mut editor, A, &[]);
        editor.style_highlight(0);
        editor.style_select();
        assert_eq!((editor.quick.page, editor.scene.items[0].item.as_str()), (quick::Page::Main, "UN"));
        // Undo brings the Downtown piece back.
        editor.quick.row = quick::UNDO;
        press(&mut editor, A, &[]);
        assert_eq!(editor.scene.items.iter().map(|p| p.item.as_str()).collect::<Vec<_>>(), vec!["DT"]);
    }

    #[test]
    fn the_colour_page_previews_keeps_reverts_and_colours_all() {
        const BACK: usize = 5;
        const RIGHT: usize = 3;
        let scene = Scene::from_layout([
            (4, "RAMP".to_owned(), scene::affine(Vec3::ZERO, Quat::IDENTITY, Vec3::ONE)),
            (5, "RAMP".to_owned(), scene::affine(Vec3::new(20.0, 0.0, 0.0), Quat::IDENTITY, Vec3::ONE)),
        ]);
        let mut editor = Editor::open(tuning(), catalog(), scene, Vec3::ZERO, 0.0);
        settle(&mut editor);
        settle(&mut editor);
        assert_eq!(editor.highlight, Some(4));
        press(&mut editor, BACK, &[]);
        editor.quick.row = quick::COLOR;
        press(&mut editor, A, &[]);
        assert_eq!((editor.quick.page, editor.quick.colour), (quick::Page::Color, 0));
        // Right: the next swatch, previewed on the highlighted ramp; B: back to white.
        press(&mut editor, RIGHT, &[]);
        assert_eq!(editor.scene.get(4).unwrap().tint, 1);
        press(&mut editor, B, &[]);
        assert_eq!((editor.quick.page, editor.scene.get(4).unwrap().tint), (quick::Page::Main, 0));
        // Again, two to the right and A: kept.
        press(&mut editor, A, &[]);
        press(&mut editor, RIGHT, &[]);
        press(&mut editor, RIGHT, &[]);
        press(&mut editor, A, &[]);
        assert_eq!((editor.scene.get(4).unwrap().tint, editor.scene.get(5).unwrap().tint), (2, 0));
        // X: every colourisable piece takes the swatch.
        press(&mut editor, A, &[]);
        press(&mut editor, RIGHT, &[]);
        press(&mut editor, X, &[]);
        assert_eq!((editor.scene.get(4).unwrap().tint, editor.scene.get(5).unwrap().tint), (0, 0), "wrapped to white");
        // A prop is never coloured.
        press(&mut editor, B, &[]);
        editor.choose("PROP", &mut Vec::new());
        assert!(editor.selection_tint().is_none());
    }

    #[test]
    fn group_select_picks_several_and_moves_them_as_one() {
        let scene = Scene::from_layout([
            (4, "RAMP".to_owned(), scene::affine(Vec3::ZERO, Quat::IDENTITY, Vec3::ONE)),
            (5, "RAMP".to_owned(), scene::affine(Vec3::new(10.0, 0.0, 0.0), Quat::IDENTITY, Vec3::ONE)),
            (6, "RAMP".to_owned(), scene::affine(Vec3::new(30.0, 0.0, 0.0), Quat::IDENTITY, Vec3::ONE)),
        ]);
        let mut editor = Editor::open(tuning(), catalog(), scene, Vec3::ZERO, 0.0);
        settle(&mut editor);
        // A held over the first ramp past 200 ms: group select with it.
        let a = slots(|s| s[A] = 1.0);
        for _ in 0..(GROUP_HOLD + 4) {
            editor.update(&a, &Flat(0.0));
        }
        assert_eq!((editor.mode, editor.group.clone()), (Mode::Group, vec![4]));
        // Still holding A, the cursor over the second: added; the third is never passed over.
        editor.cursor.position = Vec3::new(10.0, 0.0, 0.0);
        editor.update(&a, &Flat(0.0));
        assert_eq!(editor.group, vec![4, 5]);
        settle(&mut editor);
        // X: the group in hand; moved 3 m along x and placed: both moved by the same.
        let events = press(&mut editor, X, &[]);
        assert_eq!(events.iter().filter(|e| matches!(e, Event::Taken(Some(_)))).count(), 2, "{events:?}");
        assert_eq!((editor.mode, editor.held.as_ref().map(|h| h.others.len())), (Mode::Manipulate, Some(1)));
        editor.cursor.position += Vec3::new(3.0, 0.0, 0.0);
        let events = press(&mut editor, A, &[]);
        assert_eq!(events.iter().filter(|e| matches!(e, Event::Placed(_))).count(), 2, "{events:?}");
        assert!((editor.scene.get(4).unwrap().position - Vec3::new(3.0, 0.0, 0.0)).length() < 1e-4);
        assert!((editor.scene.get(5).unwrap().position - Vec3::new(13.0, 0.0, 0.0)).length() < 1e-4);
        assert_eq!(editor.scene.get(6).unwrap().position, Vec3::new(30.0, 0.0, 0.0));
        // Properties from group select colours the whole pick; a held group takes it as one too.
        const BACK: usize = 5;
        const RIGHT: usize = 3;
        editor.cursor.position = Vec3::new(3.0, 0.0, 0.0);
        settle(&mut editor);
        for _ in 0..(GROUP_HOLD + 4) {
            editor.update(&a, &Flat(0.0));
        }
        editor.cursor.position = Vec3::new(13.0, 0.0, 0.0);
        editor.update(&a, &Flat(0.0));
        settle(&mut editor);
        assert_eq!(editor.group, vec![4, 5]);
        press(&mut editor, BACK, &[]);
        editor.quick.row = quick::COLOR;
        press(&mut editor, A, &[]);
        press(&mut editor, RIGHT, &[]);
        press(&mut editor, A, &[]);
        assert_eq!((editor.scene.get(4).unwrap().tint, editor.scene.get(5).unwrap().tint), (1, 1));
        press(&mut editor, B, &[]);
        assert_eq!(editor.mode, Mode::Group, "B on the main list goes back to group select");
        press(&mut editor, B, &[]);
        // A short A on an item still takes it alone.
        editor.cursor.position = Vec3::new(30.0, 0.0, 0.0);
        settle(&mut editor);
        assert_eq!(press(&mut editor, A, &[]), vec![Event::Taken(Some(6))]);
    }

    /// The disc's records give the values the tests use. Data-gated (`SKATE3_ASSET_ROOT`).
    #[test]
    #[ignore = "needs the private install data"]
    fn data_tuning_matches_the_disc() {
        let assets = std::path::PathBuf::from(std::env::var_os("SKATE3_ASSET_ROOT").expect("SKATE3_ASSET_ROOT"));
        let catalog: serde_json::Value = serde_json::from_slice(&std::fs::read(assets.join("private/object-catalog/catalog.json")).unwrap()).unwrap();
        let c = Catalog::from_json(&catalog).unwrap();
        assert_eq!(c.items.len(), 661);
        let order: Vec<(&str, usize)> = c.categories.iter().map(|c| (c.key.as_str(), c.subs.len())).collect();
        assert_eq!(order, [("terrain", 7), ("miscterrain", 6), ("props", 7)]);
        assert_eq!(c.categories[2].subs[0], "dmo_barriers");
        // (The export carries no costs; `vita-bake catalog` adds them to the Vita's copy.)
        let t = Tuning::from_catalog(&catalog["tuning"]).unwrap();
        // `manipulate` inherits all of `default`; `object_spawner` overrides three values, `free_cam` seven. The tint
        // table: 64 swatches, white first.
        assert_eq!((t.colours.len(), t.colours[0]), (64, [1.0; 3]));
        assert_eq!(Tuning { colours: tuning().colours, ..t }, tuning());
    }
}
