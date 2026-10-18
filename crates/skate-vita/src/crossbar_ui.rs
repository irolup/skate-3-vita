//! The retail crossbar in the game: loads the linked `core_menu` screen
//! from `ui.pak` on a thread, uploads its textures one per frame and wraps
//! `skate_ui::crossbar::Crossbar` for the game loop. Any error switches it off and the
//! plain menus (`app.rs`) take over; they also stay reachable on a chord and a setting.
use std::path::Path;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::Instant;

use skate_ui::crossbar::{Crossbar, Event, MenuDef, Outcome};
use vita_render::apt::Apt;

use crate::packs::ROOT;

const SCREEN: &str = "private/ui/core_menu.linked.json";
const UI: &str = "private/ui";

/// A texture to upload: its key (the path the movie names), size and RGBA bytes.
type Texture = (String, u32, u32, Vec<u8>);

pub struct Loaded {
    crossbar: Crossbar,
    textures: Vec<Texture>,
}

pub enum MenuUi {
    /// Waiting for the packs to be mounted.
    Idle,
    /// No linked screen in ui.pak.
    Absent,
    Loading(Receiver<Result<Loaded, String>>),
    Ready { crossbar: Box<Crossbar>, pending: Vec<Texture> },
    Failed(String),
}

fn read(path: &str) -> Result<Vec<u8>, String> {
    skate_data::vfs::read(&Path::new(ROOT).join(path)).map_err(|e| format!("{path}: {e}"))
}

fn load(menus: Vec<MenuDef>) -> Result<Loaded, String> {
    let started = Instant::now();
    let json: serde_json::Value = serde_json::from_slice(&read(SCREEN)?).map_err(|e| e.to_string())?;
    let crossbar = Crossbar::load(&json, menus)?;
    let mut wanted: std::collections::BTreeMap<String, [u32; 2]> = Default::default();
    for shape in crossbar.shapes.values().flatten() {
        if let Some(t) = &shape.texture {
            wanted.insert(t.rgba.clone(), [t.width, t.height]);
        }
    }
    for font in crossbar.runtime.bindings.movie.text_assets.fonts.values() {
        wanted.insert(font.texture.clone(), font.size);
    }
    let mut textures = Vec::new();
    for (path, size) in wanted {
        let bytes = read(&format!("{UI}/{path}"))?;
        if bytes.len() != size[0] as usize * size[1] as usize * 4 {
            return Err(format!("crossbar texture {path}: wrong size"));
        }
        textures.push((path, size[0], size[1], bytes));
    }
    println!("retail menus: movie built and {} textures read in {:.0} ms", textures.len(), started.elapsed().as_secs_f32() * 1000.0);
    Ok(Loaded { crossbar, textures })
}

impl MenuUi {
    /// Starts loading once ui.pak is mounted.
    pub fn start(&mut self, menus: Vec<MenuDef>) {
        if !matches!(self, MenuUi::Idle) {
            return;
        }
        if !crate::packs::UI.load(std::sync::atomic::Ordering::Relaxed)
            || !skate_data::vfs::is_file(&Path::new(ROOT).join(SCREEN))
        {
            *self = MenuUi::Absent;
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("menu-load".into())
            .stack_size(4 * 1024 * 1024)
            .spawn(move || {
                crate::alloc::tag_thread("load");
                let _ = vita_platform::vita::pin_current_thread(2);
                let _ = tx.send(load(menus));
            })
            .expect("spawn the menu loader");
        *self = MenuUi::Loading(rx);
    }

    /// Still starting up (waiting for the packs, loading, or uploading textures); false
    /// once it is ready or settled as absent or failed.
    pub fn busy(&self) -> bool {
        match self {
            MenuUi::Idle | MenuUi::Loading(_) => true,
            MenuUi::Ready { pending, .. } => !pending.is_empty(),
            MenuUi::Absent | MenuUi::Failed(_) => false,
        }
    }

    /// Not yet decided: waiting for the packs or the load (textures may still upload).
    pub fn loading(&self) -> bool {
        matches!(self, MenuUi::Idle | MenuUi::Loading(_))
    }

    pub fn failure(&self) -> Option<&str> {
        match self {
            MenuUi::Failed(e) => Some(e),
            _ => None,
        }
    }

    fn fail(&mut self, what: &str, error: String) {
        println!("retail menus off ({what}): {error}");
        *self = MenuUi::Failed(format!("{what}: {error}"));
    }

    /// Once per frame: finishes loading and uploads a texture.
    pub fn frame(&mut self, apt: &mut Apt) {
        if let MenuUi::Loading(rx) = self {
            match rx.try_recv() {
                Ok(Ok(loaded)) => *self = MenuUi::Ready { crossbar: Box::new(loaded.crossbar), pending: loaded.textures },
                Ok(Err(e)) => self.fail("load", e),
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => self.fail("load", "the loader stopped".into()),
            }
        }
        if let MenuUi::Ready { pending, .. } = self {
            if let Some((path, w, h, bytes)) = pending.pop() {
                apt.add_texture(&path, w, h, &bytes);
            }
        }
    }

    pub fn open(&mut self, menus: Vec<MenuDef>, menu: usize, option: usize) {
        if let MenuUi::Ready { crossbar, .. } = self {
            if let Err(e) = crossbar.open(menus, menu, option) {
                self.fail("open", e);
            }
        }
    }

    pub fn refresh(&mut self, menus: Vec<MenuDef>) {
        if let MenuUi::Ready { crossbar, .. } = self {
            if let Err(e) = crossbar.refresh(menus) {
                self.fail("refresh", e);
            }
        }
    }

    pub fn event(&mut self, event: Event) -> Outcome {
        if let MenuUi::Ready { crossbar, .. } = self {
            match crossbar.event(event) {
                Ok(outcome) => return outcome,
                Err(e) => self.fail("input", e),
            }
        }
        Outcome::None
    }

    /// One movie frame (while shown).
    pub fn update(&mut self) {
        if let MenuUi::Ready { crossbar, .. } = self {
            if let Err(e) = crossbar.update() {
                self.fail("frame", e);
            }
        }
    }

    pub fn draw(&mut self, apt: &mut Apt) {
        let MenuUi::Ready { crossbar, .. } = self else { return };
        match crossbar.draw() {
            Ok(draws) => apt.draw(&draws),
            Err(e) => self.fail("draw", e),
        }
    }
}
