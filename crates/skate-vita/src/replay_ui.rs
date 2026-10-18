//! The retail replay editor screen in the game: loads `replay_editor2` from `ui.pak` on a
//! thread when the replay is first opened, uploads its textures one per frame and drives it from
//! the replay's playback state. Any error switches it off; the plain status line takes over.
use std::path::Path;
use std::sync::mpsc::{Receiver, TryRecvError};

use skate_ui::replay_editor::{ReplayEditor, ReplayHelp, state};
use vita_render::apt::Apt;

use crate::packs::ROOT;
use crate::session_replay::{Mode, Over, Playback, Recorder};

const SCREEN: &str = "private/ui/replay_editor2.linked.json";
const UI: &str = "private/ui";

type Texture = (String, u32, u32, Vec<u8>);

pub struct Loaded {
    editor: ReplayEditor,
    textures: Vec<Texture>,
}

fn read(path: &str) -> Result<Vec<u8>, String> {
    skate_data::vfs::read(&Path::new(ROOT).join(path)).map_err(|e| format!("{path}: {e}"))
}

fn load() -> Result<Loaded, String> {
    let started = std::time::Instant::now();
    let json: serde_json::Value = serde_json::from_slice(&read(SCREEN)?).map_err(|e| e.to_string())?;
    let mut editor = ReplayEditor::load(&json)?;
    drop(json);
    let mut wanted: std::collections::BTreeMap<String, [u32; 2]> = Default::default();
    for shape in editor.shapes.values().flatten() {
        if let Some(t) = &shape.texture {
            wanted.insert(t.rgba.clone(), [t.width, t.height]);
        }
    }
    for font in editor.runtime.bindings.movie.text_assets.fonts.values() {
        wanted.insert(font.texture.clone(), font.size);
    }
    let mut textures = Vec::new();
    for (path, size) in wanted {
        let bytes = read(&format!("{UI}/{path}"))?;
        if bytes.len() != size[0] as usize * size[1] as usize * 4 {
            return Err(format!("replay editor texture {path}: wrong size"));
        }
        textures.push((path, size[0], size[1], bytes));
    }
    editor.intro()?;
    println!("replay editor: movie built and {} textures read in {:.0} ms", textures.len(), started.elapsed().as_secs_f32() * 1000.0);
    Ok(Loaded { editor, textures })
}

/// What the editor was last told, to send only changes.
#[derive(Default)]
struct Sent {
    cursor: f32,
    start: f32,
    end: f32,
    mode: Option<Mode>,
    paused: Option<bool>,
    hidden: Option<bool>,
    ready: bool,
    selection: usize,
    speed: u8,
    shake: u8,
    zoom: String,
    over: Option<Over>,
}

pub enum ReplayUi {
    Idle,
    Loading(Receiver<Result<Loaded, String>>),
    Ready { editor: Box<ReplayEditor>, pending: Vec<Texture>, sent: Sent },
    Failed(String),
}

impl ReplayUi {
    /// Starts loading (once) if ui.pak has the screen.
    pub fn start(&mut self) {
        if !matches!(self, ReplayUi::Idle) {
            return;
        }
        if !skate_data::vfs::is_file(&Path::new(ROOT).join(SCREEN)) {
            *self = ReplayUi::Failed("no replay editor screen in ui.pak".into());
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("replay-ui-load".into())
            .stack_size(4 * 1024 * 1024)
            .spawn(move || {
                crate::alloc::tag_thread("load");
                let _ = vita_platform::vita::pin_current_thread(2);
                let _ = tx.send(load());
            })
            .expect("spawn the replay editor loader");
        *self = ReplayUi::Loading(rx);
    }

    fn fail(&mut self, what: &str, error: String) {
        println!("replay editor off ({what}): {error}");
        *self = ReplayUi::Failed(format!("{what}: {error}"));
    }

    /// Whether the editor has finished loading (the help loads next).
    pub fn loaded(&self) -> bool {
        matches!(self, ReplayUi::Ready { .. })
    }

    /// Ready to draw (loaded, textures uploaded).
    pub fn ready(&self) -> bool {
        matches!(self, ReplayUi::Ready { pending, .. } if pending.is_empty())
    }

    /// Every frame, replay or not: finishes loading and uploads a texture. True while it did work.
    pub fn poll(&mut self, apt: &mut Apt) -> bool {
        if let ReplayUi::Loading(rx) = self {
            match rx.try_recv() {
                Ok(Ok(l)) => *self = ReplayUi::Ready { editor: Box::new(l.editor), pending: l.textures, sent: Sent::default() },
                Ok(Err(e)) => self.fail("load", e),
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => self.fail("load", "the loader stopped".into()),
            }
        }
        if let ReplayUi::Ready { pending, .. } = self {
            if let Some((path, w, h, bytes)) = pending.pop() {
                if !apt.has_texture(&path) {
                    apt.add_texture(&path, w, h, &bytes);
                }
                return true;
            }
        }
        false
    }

    /// While the replay plays: sends the changes of the playback state and runs a movie frame.
    pub fn frame(&mut self, apt: &mut Apt, rec: &Recorder, pb: &Playback) {
        if self.poll(apt) {
            return;
        }
        let ReplayUi::Ready { editor, sent, .. } = self else { return };
        let result = (|| {
            let [cursor, start, end] = pb.fractions(rec);
            let m = editor.model();
            (m.cursor, m.start, m.end) = (cursor, start, end);
            m.length = format!("Estimated Video Length: {:.1} s", pb.clip_seconds(rec));
            m.instant = sent.ready && pb.mode == Mode::Scrub;
            let fx = pb.edit_fx();
            (m.camera_index, m.speed, m.shake) = (pb.selection as i32, i32::from(fx.speed_setting), i32::from(fx.shake));
            m.zoom = pb.zoom_text();
            if !sent.ready {
                // The screen's intro ends in `InstantDone`, which switches it to the timeline editor.
                if !editor.model().ready {
                    return editor.update();
                }
                sent.ready = true;
            }
            if sent.mode != Some(pb.mode) {
                sent.mode = Some(pb.mode);
                editor.set_state(pb.screen_state())?;
                // The mode resets the timeline's markers and playhead.
                (sent.cursor, sent.start, sent.end) = (-1.0, -1.0, -1.0);
                sent.paused = None;
                (sent.selection, sent.speed, sent.shake) = (pb.selection, pb.edit_fx().speed_setting, pb.edit_fx().shake);
                sent.zoom = pb.zoom_text();
            }
            if pb.mode == Mode::Main && sent.over != Some(pb.over()) {
                // Over a clip marker the screen dims "Edit clip" and the place-marker icon; over a
                // camera key it lights the remove icon.
                sent.over = Some(pb.over());
                editor.set_state(match pb.over() {
                    Over::Marker => state::OVER_CLIP_MARKER,
                    Over::Key => state::OVER_KEY_MARKER,
                    Over::Clip => state::OVER_CLIP,
                })?;
            }
            if pb.mode != Mode::Main {
                sent.over = None;
            }
            if pb.mode == Mode::Effects {
                if sent.selection != pb.selection {
                    sent.selection = pb.selection;
                    editor.set_state(state::REFRESH_EFFECTS)?;
                }
                if sent.speed != pb.edit_fx().speed_setting || sent.shake != pb.edit_fx().shake {
                    (sent.speed, sent.shake) = (pb.edit_fx().speed_setting, pb.edit_fx().shake);
                    editor.set_state(state::UPDATE_VALUES)?;
                }
            }
            if matches!(pb.mode, Mode::Camera(..)) && sent.zoom != pb.zoom_text() {
                sent.zoom = pb.zoom_text();
                editor.set_state(state::UPDATE_ZOOM)?;
            }
            if (sent.cursor - cursor).abs() > 1e-5 {
                sent.cursor = cursor;
                editor.set_state(state::MOVE_PLAYHEAD)?;
            }
            if (sent.start - start).abs() > 1e-5 {
                sent.start = start;
                editor.set_state(state::MOVE_START)?;
            }
            if (sent.end - end).abs() > 1e-5 {
                sent.end = end;
                editor.set_state(state::MOVE_END)?;
            }
            if sent.paused != Some(pb.paused) {
                sent.paused = Some(pb.paused);
                editor.set_state(if pb.paused { state::STOP_PLAYBACK } else { state::START_PLAYBACK })?;
            }
            if sent.hidden != Some(pb.hidden) {
                sent.hidden = Some(pb.hidden);
                editor.set_state(if pb.hidden { state::HIDE_EDITOR } else { state::SHOW_EDITOR })?;
            }
            // Scrub shows no In and Out points; the timeline editor does; the other screens decide.
            let shown = match pb.mode {
                Mode::Scrub => Some(false),
                Mode::Main => Some(true),
                _ => None,
            };
            editor.place_range_markers(start, end, shown)?;
            editor.update()
        })();
        if let Err(e) = result {
            self.fail("frame", e);
        }
    }

    /// Draws the screen; the controller glyphs it leaves to the host come back as
    /// (retail button name, stage x, y, width, height).
    pub fn draw(&mut self, apt: &mut Apt) -> Vec<(String, [f32; 4])> {
        let ReplayUi::Ready { editor, pending, .. } = self else { return Vec::new() };
        if !pending.is_empty() {
            return Vec::new();
        }
        match editor.draw() {
            Ok(draws) => {
                apt.draw(&draws);
                editor.glyphs()
            }
            Err(e) => {
                self.fail("draw", e);
                Vec::new()
            }
        }
    }

    pub fn failure(&self) -> Option<&str> {
        match self {
            ReplayUi::Failed(e) => Some(e),
            _ => None,
        }
    }
}

const HELP_SCREEN: &str = "private/ui/replay_help.linked.json";

struct HelpLoaded {
    help: ReplayHelp,
    textures: Vec<Texture>,
}

fn load_help() -> Result<HelpLoaded, String> {
    let started = std::time::Instant::now();
    let json: serde_json::Value = serde_json::from_slice(&read(HELP_SCREEN)?).map_err(|e| e.to_string())?;
    let mut help = ReplayHelp::load(&json)?;
    drop(json);
    let mut wanted: std::collections::BTreeMap<String, [u32; 2]> = Default::default();
    for shape in help.shapes.values().flatten() {
        if let Some(t) = &shape.texture {
            wanted.insert(t.rgba.clone(), [t.width, t.height]);
        }
    }
    for font in help.runtime.bindings.movie.text_assets.fonts.values() {
        wanted.insert(font.texture.clone(), font.size);
    }
    let mut textures = Vec::new();
    for (path, size) in wanted {
        let bytes = read(&format!("{UI}/{path}"))?;
        if bytes.len() != size[0] as usize * size[1] as usize * 4 {
            return Err(format!("replay help texture {path}: wrong size"));
        }
        textures.push((path, size[0], size[1], bytes));
    }
    for _ in 0..30 {
        help.update()?;
    }
    println!("replay help: movie built and {} textures read in {:.0} ms", textures.len(), started.elapsed().as_secs_f32() * 1000.0);
    Ok(HelpLoaded { help, textures })
}

/// The retail help pages over the editor (Start): loaded after the editor, shown for the editor's mode.
pub enum HelpUi {
    Idle,
    Loading(Receiver<Result<HelpLoaded, String>>),
    Ready { help: Box<ReplayHelp>, pending: Vec<Texture>, shown: bool },
    Failed(String),
}

impl HelpUi {
    pub fn start(&mut self) {
        if !matches!(self, HelpUi::Idle) {
            return;
        }
        if !skate_data::vfs::is_file(&Path::new(ROOT).join(HELP_SCREEN)) {
            *self = HelpUi::Failed("no replay help screen in ui.pak".into());
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("replay-help-load".into())
            .stack_size(4 * 1024 * 1024)
            .spawn(move || {
                crate::alloc::tag_thread("load");
                let _ = vita_platform::vita::pin_current_thread(2);
                let _ = tx.send(load_help());
            })
            .expect("spawn the replay help loader");
        *self = HelpUi::Loading(rx);
    }

    pub fn poll(&mut self, apt: &mut Apt) -> bool {
        if let HelpUi::Loading(rx) = self {
            match rx.try_recv() {
                Ok(Ok(l)) => *self = HelpUi::Ready { help: Box::new(l.help), pending: l.textures, shown: false },
                Ok(Err(e)) => {
                    println!("replay help off: {e}");
                    *self = HelpUi::Failed(e);
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => *self = HelpUi::Failed("the loader stopped".into()),
            }
        }
        if let HelpUi::Ready { pending, .. } = self {
            if let Some((path, w, h, bytes)) = pending.pop() {
                if !apt.has_texture(&path) {
                    apt.add_texture(&path, w, h, &bytes);
                }
                return true;
            }
        }
        false
    }

    /// Whether the help can be opened now.
    pub fn available(&self) -> bool {
        matches!(self, HelpUi::Ready { pending, .. } if pending.is_empty())
    }

    /// Opens the pages of an editor mode (`Playback::help_mode`).
    pub fn open(&mut self, number: i32) {
        let HelpUi::Ready { help, shown, .. } = self else { return };
        if let Err(e) = help.open(number) {
            println!("replay help off: {e}");
            *self = HelpUi::Failed(e);
            return;
        }
        *shown = true;
    }

    pub fn next_page(&mut self) {
        if let HelpUi::Ready { help, .. } = self {
            if let Err(e) = help.next_page() {
                println!("replay help off: {e}");
                *self = HelpUi::Failed(e);
            }
        }
    }

    pub fn close(&mut self) {
        if let HelpUi::Ready { shown, .. } = self {
            *shown = false;
        }
    }

    pub fn update(&mut self) {
        if let HelpUi::Ready { help, shown: true, .. } = self {
            if let Err(e) = help.update() {
                println!("replay help off: {e}");
                *self = HelpUi::Failed(e);
            }
        }
    }

    pub fn draw(&mut self, apt: &mut Apt) -> Vec<(String, [f32; 4])> {
        let HelpUi::Ready { help, shown: true, .. } = self else { return Vec::new() };
        match help.draw() {
            Ok(draws) => {
                apt.draw(&draws);
                help.glyphs()
            }
            Err(e) => {
                println!("replay help off: {e}");
                *self = HelpUi::Failed(e);
                Vec::new()
            }
        }
    }
}

const POPUP_SCREEN: &str = "private/ui/sk8popup.linked.json";

/// The retail list popup the replay editor's Finished menu uses (Upload Video, Save Replay, Exit):
/// loaded after the help, shown while the menu is open.
pub enum PopupUi {
    Idle,
    Loading(Receiver<Result<(skate_ui::popup::Popup, Vec<Texture>), String>>),
    Ready { popup: Box<skate_ui::popup::Popup>, pending: Vec<Texture> },
    Failed(String),
}

fn load_popup() -> Result<(skate_ui::popup::Popup, Vec<Texture>), String> {
    let json: serde_json::Value = serde_json::from_slice(&read(POPUP_SCREEN)?).map_err(|e| e.to_string())?;
    let popup = skate_ui::popup::Popup::load(&json)?;
    drop(json);
    let mut wanted: std::collections::BTreeMap<String, [u32; 2]> = Default::default();
    for shape in popup.shapes.values().flatten() {
        if let Some(t) = &shape.texture {
            wanted.insert(t.rgba.clone(), [t.width, t.height]);
        }
    }
    for font in popup.runtime.bindings.movie.text_assets.fonts.values() {
        wanted.insert(font.texture.clone(), font.size);
    }
    let mut textures = Vec::new();
    for (path, size) in wanted {
        let bytes = read(&format!("{UI}/{path}"))?;
        if bytes.len() != size[0] as usize * size[1] as usize * 4 {
            return Err(format!("popup texture {path}: wrong size"));
        }
        textures.push((path, size[0], size[1], bytes));
    }
    Ok((popup, textures))
}

impl PopupUi {
    pub fn start(&mut self) {
        if !matches!(self, PopupUi::Idle) {
            return;
        }
        if !skate_data::vfs::is_file(&Path::new(ROOT).join(POPUP_SCREEN)) {
            *self = PopupUi::Failed("no popup screen in ui.pak".into());
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("popup-load".into())
            .stack_size(4 * 1024 * 1024)
            .spawn(move || {
                crate::alloc::tag_thread("load");
                let _ = vita_platform::vita::pin_current_thread(2);
                let _ = tx.send(load_popup());
            })
            .expect("spawn the popup loader");
        *self = PopupUi::Loading(rx);
    }

    fn fail(&mut self, error: String) {
        println!("replay popup off: {error}");
        *self = PopupUi::Failed(error);
    }

    pub fn poll(&mut self, apt: &mut Apt) {
        if let PopupUi::Loading(rx) = self {
            match rx.try_recv() {
                Ok(Ok((popup, textures))) => *self = PopupUi::Ready { popup: Box::new(popup), pending: textures },
                Ok(Err(e)) => self.fail(e),
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => self.fail("the loader stopped".into()),
            }
        }
        if let PopupUi::Ready { pending, .. } = self {
            if let Some((path, w, h, bytes)) = pending.pop() {
                if !apt.has_texture(&path) {
                    apt.add_texture(&path, w, h, &bytes);
                }
            }
        }
    }

    pub fn available(&self) -> bool {
        matches!(self, PopupUi::Ready { pending, .. } if pending.is_empty())
    }

    /// Follows the Finished menu: open with `options` (the highlighted one is `selected`) or closed.
    pub fn frame(&mut self, menu: Option<usize>, options: &[&str]) {
        let PopupUi::Ready { popup, pending } = self else { return };
        if !pending.is_empty() {
            return;
        }
        let result = (|| {
            match menu {
                Some(selected) => {
                    if !popup.shown() {
                        popup.open(options.iter().map(|o| (*o).to_owned()).collect(), "")?;
                    }
                    if popup.selected() != selected {
                        popup.select(selected)?;
                    }
                    popup.update()
                }
                None => {
                    popup.close();
                    Ok(())
                }
            }
        })();
        if let Err(e) = result {
            self.fail(e);
        }
    }

    pub fn draw(&mut self, apt: &mut Apt) -> Vec<(String, [f32; 4])> {
        let PopupUi::Ready { popup, .. } = self else { return Vec::new() };
        if !popup.shown() {
            return Vec::new();
        }
        match popup.draw() {
            Ok(draws) => {
                apt.draw(&draws);
                popup.glyphs()
            }
            Err(e) => {
                self.fail(e);
                Vec::new()
            }
        }
    }
}
