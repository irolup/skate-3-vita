//! The game's own trick display: the original HUD movie from
//! `ui.pak`, driven by the simulation's per-tick scoring input exactly as skate-game's
//! `scoring_hud.rs` does, and drawn through `vita_render::apt`.
//!
//! Loading: a thread reads and parses the movie and its textures (the JSON is 1.6 MB);
//! the main thread builds the movie's VM, uploads the textures one per frame and plays
//! the queued ticks. Any error switches the HUD off (the basic HUD takes over).
use std::path::Path;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::Instant;

use skate_sim::live::HudFeed;
use skate_ui::fe::apt_scene::{self, Shapes};
use skate_ui::fe::apt_vm::Value;
use skate_ui::hud_runtime::{Input, Runtime};
use vita_render::apt::Apt;

use crate::packs::ROOT;

const HUD: &str = "private/hud";

/// A texture to upload: its key (the path the movie names), size and RGBA bytes.
type Texture = (String, u32, u32, Vec<u8>);

pub struct Loaded {
    /// The movie's JSON as read (kept as text, parsed again for each build; the
    /// parsed tree took about 10 MB of small blocks for the whole run).
    source: Vec<u8>,
    shapes: Shapes,
    textures: Vec<Texture>,
}

pub enum Hud {
    /// Waiting for the packs to be mounted (the simulation thread does it at startup).
    Idle,
    /// No ui.pak.
    Absent,
    Loading(Receiver<Result<Loaded, String>>),
    Ready(Box<Ready>),
    Failed(String),
}

pub struct Ready {
    source: Arc<[u8]>,
    shapes: Shapes,
    /// None while the movie is being (re)built on its thread: nothing is drawn or played.
    runtime: Option<Runtime>,
    /// The session generation (`Shared::hud_generation`) the movie was built for.
    generation: u64,
    rebuilding: Option<(u64, Receiver<Result<Runtime, String>>)>,
    /// The first movie has been built (`booting` is over).
    built_once: bool,
    /// Textures still to upload (one per frame).
    pending: Vec<Texture>,
}

/// The movie's input from one tick of the simulation (skate-game's `hud_input`:
/// `trick_metrics` is the trick name and four `false`, no context tricks).
fn input(f: &HudFeed) -> Input {
    Input {
        sequence_score: f.sequence_score,
        line_score: f.line_score,
        sequence_timer: f.sequence_timer,
        line_time: f.line_time,
        line_capacity: f.line_capacity,
        multiplier: f.multiplier,
        clean: f.clean,
        sketchy: f.sketchy,
        stance: f.stance,
        trick_name: f.trick_name.clone(),
        trick_metrics: [Value::Text(f.trick_name.clone()), Value::Bool(false), Value::Bool(false), Value::Bool(false), Value::Bool(false)],
        context_tricks: Vec::new(),
    }
}

fn read(path: &str) -> Result<Vec<u8>, String> {
    skate_data::vfs::read(&Path::new(ROOT).join(HUD).join(path)).map_err(|e| format!("{path}: {e}"))
}

fn load() -> Result<Loaded, String> {
    let started = Instant::now();
    let source = read("runtime/trickdisplay.json")?;
    let json: serde_json::Value = serde_json::from_slice(&source).map_err(|e| e.to_string())?;
    let shapes: Shapes = serde_json::from_value(json["shapes"].clone()).map_err(|e| e.to_string())?;
    // The textures the movie draws: its shapes' atlases and the fonts' (the fonts come
    // from a throwaway movie, as the reference reads them from the loaded one).
    let probe = Runtime::load(&json, input(&HudFeed::default()))?;
    let mut wanted: std::collections::BTreeMap<String, [u32; 2]> = Default::default();
    for shape in shapes.values().flatten() {
        if let Some(t) = &shape.texture {
            wanted.insert(t.rgba.clone(), [t.width, t.height]);
        }
    }
    for font in probe.bindings.movie.text_assets.fonts.values() {
        wanted.insert(font.texture.clone(), font.size);
    }
    drop((probe, json));
    let mut textures = Vec::new();
    for (path, size) in wanted {
        let bytes = read(&path)?;
        if bytes.len() != size[0] as usize * size[1] as usize * 4 {
            return Err(format!("HUD texture {path}: wrong size"));
        }
        textures.push((path, size[0], size[1], bytes));
    }
    println!("retail HUD: movie and {} textures read in {:.0} ms", textures.len(), started.elapsed().as_secs_f32() * 1000.0);
    Ok(Loaded { source, shapes, textures })
}

impl Hud {
    /// Starts loading when ui.pak is mounted; call once the packs are mounted.
    pub fn start(&mut self) {
        if !matches!(self, Hud::Idle) {
            return;
        }
        if !crate::packs::UI.load(std::sync::atomic::Ordering::Relaxed) {
            *self = Hud::Absent;
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("hud-load".into())
            .stack_size(4 * 1024 * 1024)
            .spawn(move || {
                crate::alloc::tag_thread("load");
                let _ = vita_platform::vita::pin_current_thread(2);
                let _ = tx.send(load());
            })
            .expect("spawn the HUD loader");
        *self = Hud::Loading(rx);
    }

    /// Still starting up for the first time: the load, the first movie build or the texture
    /// uploads have not finished (false once ready, absent or failed). Playing is held
    /// until then: the loaders contend with the simulation for the heap and the cores.
    pub fn booting(&self) -> bool {
        match self {
            Hud::Idle | Hud::Loading(_) => true,
            Hud::Ready(r) => !r.pending.is_empty() || !r.built_once,
            Hud::Absent | Hud::Failed(_) => false,
        }
    }

    /// The movie is being built; play waits for it (`app.rs` holds the simulation).
    pub fn building(&self) -> bool {
        matches!(self, Hud::Ready(r) if r.rebuilding.is_some() || r.runtime.is_none())
    }

    /// Whether the retail HUD has taken over from the basic one (it may be rebuilding).
    pub fn active(&self) -> bool {
        matches!(self, Hud::Ready(_))
    }

    pub fn failure(&self) -> Option<&str> {
        match self {
            Hud::Failed(e) => Some(e),
            _ => None,
        }
    }

    /// Once per frame: finishes loading, uploads a texture, (re)builds the movie when the
    /// session reloaded (on a thread: building it takes ~300 ms), then plays the ticks
    /// queued by the simulation (`play` is false off the playing screen: the queue is
    /// dropped, as the reference pauses the movie with the game).
    pub fn frame(&mut self, apt: &mut Apt, shared: &mut crate::lab::Shared, play: bool) {
        if let Hud::Loading(rx) = self {
            match rx.try_recv() {
                Ok(Ok(loaded)) => {
                    *self = Hud::Ready(Box::new(Ready {
                        source: Arc::from(loaded.source),
                        shapes: loaded.shapes,
                        runtime: None,
                        generation: u64::MAX,
                        rebuilding: None,
                        built_once: false,
                        pending: loaded.textures,
                    }));
                }
                Ok(Err(e)) => *self = Hud::Failed(e),
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => *self = Hud::Failed("the HUD loader stopped".into()),
            }
        }
        let Hud::Ready(ready) = self else { return };
        if let Some((path, w, h, bytes)) = ready.pending.pop() {
            apt.add_texture(&path, w, h, &bytes);
        }
        // A finished build replaces the movie; a reloaded session (first load, new
        // world, recovery) starts a new one.
        if let Some((generation, rx)) = &ready.rebuilding {
            match rx.try_recv() {
                Ok(Ok(runtime)) => {
                    ready.generation = *generation;
                    ready.runtime = Some(runtime);
                    ready.rebuilding = None;
                    ready.built_once = true;
                }
                Ok(Err(e)) => {
                    *self = Hud::Failed(e);
                    return;
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {
                    *self = Hud::Failed("the HUD builder stopped".into());
                    return;
                }
            }
        }
        // A session's movie is built once the session has loaded, with the simulation held
        // (`building`): building it is a burst of allocations that, run beside the park
        // load or the first seconds of play, slows both many times over (measured on the
        // device: the park load took 45 s instead of 8 s).
        let target = shared.hud_generation;
        if ready.rebuilding.is_none() && !shared.loading && (ready.runtime.is_none() || ready.generation != target) {
            ready.runtime = None;
            let (source, feed, generation) = (ready.source.clone(), input(&shared.hud_latest), target);
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::Builder::new()
                .name("hud-build".into())
                .stack_size(4 * 1024 * 1024)
                .spawn(move || {
                    crate::alloc::tag_thread("load");
                    let _ = vita_platform::vita::pin_current_thread(2);
                    let started = Instant::now();
                    let result = serde_json::from_slice::<serde_json::Value>(&source).map_err(|e| e.to_string()).and_then(|json| Runtime::load(&json, feed));
                    println!("retail HUD: movie built in {:.0} ms", started.elapsed().as_secs_f32() * 1000.0);
                    let _ = tx.send(result);
                })
                .expect("spawn the HUD builder");
            ready.rebuilding = Some((generation, rx));
        }
        let Some(runtime) = &mut ready.runtime else { return };
        if !play {
            shared.hud_feeds.clear();
            return;
        }
        let mut result = Ok(());
        while let Some(feed) = shared.hud_feeds.pop_front() {
            result = runtime.update(input(&feed), feed.new_trick, feed.modified_trick, feed.close_tricks);
            if result.is_err() {
                break;
            }
        }
        if let Err(e) = result {
            println!("retail HUD stopped: {e}");
            *self = Hud::Failed(e);
        }
    }

    /// Draws the movie (when built and its textures are all in).
    pub fn draw(&mut self, apt: &mut Apt) {
        let Hud::Ready(ready) = self else { return };
        let Some(runtime) = &ready.runtime else { return };
        if !ready.pending.is_empty() {
            return;
        }
        match apt_scene::draw(&runtime.bindings.movie, &runtime.vm, &ready.shapes) {
            Ok(draws) => apt.draw(&draws),
            Err(e) => {
                println!("retail HUD geometry: {e}");
                *self = Hud::Failed(e);
            }
        }
    }
}

const MARKER: &str = "private/session-marker";

#[derive(serde::Deserialize)]
struct MarkerManifest {
    version: u32,
    canvas: [f32; 2],
    textures: Vec<MarkerTexture>,
    meshes: Vec<MarkerMesh>,
}
#[derive(serde::Deserialize)]
struct MarkerTexture {
    file: String,
    width: u32,
    height: u32,
}
#[derive(serde::Deserialize)]
struct MarkerMesh {
    texture: Option<usize>,
    color: [f32; 4],
    vertices: Vec<MarkerVertex>,
}
#[derive(serde::Deserialize)]
struct MarkerVertex {
    position: [f32; 2],
    uv: [f32; 2],
}

/// The session marker's overlay: the original prompt panel (static
/// triangles from the owned data, drawn while LB is held) and the noise dissolve that
/// plays while a return is held.
pub struct Marker {
    draws: Vec<apt_scene::Draw>,
    noise: skate_sim::DissolveNoise,
    dissolve: vita_render::apt::Dissolve,
    clock: Instant,
    ui_time: f64,
}

impl Marker {
    pub fn load(apt: &mut Apt) -> Result<Self, String> {
        let root = Path::new(ROOT).join(MARKER);
        let bytes = skate_data::vfs::read(&root.join("hud.json")).map_err(|e| format!("session marker hud.json: {e}"))?;
        let hud: MarkerManifest = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        if hud.version != 1 || hud.canvas != [1280., 720.] {
            return Err("unsupported session-marker HUD manifest".into());
        }
        let mut keys = Vec::new();
        for t in &hud.textures {
            let data = skate_data::vfs::read(&root.join(&t.file)).map_err(|e| format!("{}: {e}", t.file))?;
            if data.len() != t.width as usize * t.height as usize * 4 {
                return Err(format!("session marker texture {}: wrong size", t.file));
            }
            let key = format!("{MARKER}/{}", t.file);
            apt.add_texture(&key, t.width, t.height, &data);
            keys.push((key, [t.width, t.height]));
        }
        apt.add_texture("white", 1, 1, &[255; 4]);
        let linear = |c: f32| c.max(0.0).powf(2.2);
        let draws = hud
            .meshes
            .iter()
            .map(|m| {
                let texture = m.texture.and_then(|i| keys.get(i)).map_or_else(|| "white".to_owned(), |(k, _)| k.clone());
                apt_scene::Draw {
                    texture,
                    fill: apt_scene::Fill::TextureClamped,
                    vertices: m.vertices.iter().map(|v| apt_scene::Vertex { position: v.position, uv: v.uv }).collect(),
                    // The reference's ColorMaterial colour is sRGB; the shader works in linear light.
                    multiply: [linear(m.color[0]), linear(m.color[1]), linear(m.color[2]), m.color[3]],
                    add: [0.0; 4],
                }
            })
            .collect();
        let dissolve = vita_render::apt::Dissolve::new(&skate_sim::DissolveNoise::texture());
        println!("session marker overlay: {} meshes, {} textures", hud.meshes.len(), hud.textures.len());
        Ok(Self { draws, noise: skate_sim::DissolveNoise::default(), dissolve, clock: Instant::now(), ui_time: 0.0 })
    }

    /// The noise stream runs on the UI clock, 1/60 s a step, whether shown or not.
    pub fn advance(&mut self) {
        self.ui_time += self.clock.elapsed().as_secs_f64().min(0.25);
        self.clock = Instant::now();
        while self.ui_time >= 1.0 / 60.0 {
            self.ui_time -= 1.0 / 60.0;
            self.noise.advance();
        }
    }

    /// The dissolve (under the trick display) while a return is held.
    pub fn draw_dissolve(&mut self, view: &skate_sim::MarkerView) {
        if view.progress > 0.0 {
            let (scroll, first, second) = self.noise.params();
            self.dissolve.draw(scroll, first, second, view.progress);
        }
    }

    /// The prompt panel while LB is held.
    pub fn draw_panel(&self, apt: &mut Apt, view: &skate_sim::MarkerView) {
        if view.visible {
            apt.draw(&self.draws);
        }
    }
}

const METER: &str = "private/ui/manualmeter.linked.json";
const ANALYZER: &str = "private/ui/trickanalyser.linked.json";

fn read_screen(path: &str) -> Result<serde_json::Value, String> {
    let bytes = skate_data::vfs::read(&Path::new(ROOT).join(path)).map_err(|e| format!("{path}: {e}"))?;
    serde_json::from_slice(&bytes).map_err(|e| e.to_string())
}

/// Uploads the textures a linked screen's shapes use, from `private/ui/`.
fn upload_textures(shapes: &Shapes, apt: &mut Apt) -> Result<(), String> {
    let mut wanted: std::collections::BTreeMap<String, [u32; 2]> = Default::default();
    for shape in shapes.values().flatten() {
        if let Some(t) = &shape.texture {
            wanted.insert(t.rgba.clone(), [t.width, t.height]);
        }
    }
    for (path, size) in wanted {
        let bytes = skate_data::vfs::read(&Path::new(ROOT).join("private/ui").join(&path)).map_err(|e| format!("{path}: {e}"))?;
        if bytes.len() != size[0] as usize * size[1] as usize * 4 {
            return Err(format!("texture {path}: wrong size"));
        }
        apt.add_texture(&path, size[0], size[1], &bytes);
    }
    Ok(())
}

pub fn screen_present(name: &str) -> bool {
    skate_data::vfs::is_file(&Path::new(ROOT).join(name))
}

/// The retail manual meter (`hud2/manualmeter`): small enough to build on the main thread.
/// The arrow and the sweet spots follow the sim's deck tilt (`Session::manual_view`).
pub struct Meter {
    meter: skate_ui::meter::ManualMeter,
}

impl Meter {
    pub const FILE: &'static str = METER;

    pub fn load(apt: &mut Apt) -> Result<Self, String> {
        let meter = skate_ui::meter::ManualMeter::load(&read_screen(METER)?)?;
        upload_textures(&meter.shapes, apt)?;
        Ok(Self { meter })
    }

    /// One displayed frame while playing; an error switches the meter off (returns false).
    pub fn frame(&mut self, view: &skate_sim::ManualView) -> bool {
        self.meter.update(view.active, view.value).is_ok()
    }

    pub fn draw(&self, apt: &mut Apt) {
        if let Ok(draws) = self.meter.draw() {
            apt.draw(&draws);
        }
    }
}

/// Frames the gesture of the last trick stays on screen once the trick display is idle.
const ANALYZER_HOLD: u32 = 180;

/// The retail trick analyzer (`hud2/trickanalyser`): the stick gesture of the trick being
/// scored, held for a few seconds after it ends.
pub struct Analyzer {
    analyzer: skate_ui::analyzer::TrickAnalyzer,
    idle: u32,
}

impl Analyzer {
    pub const FILE: &'static str = ANALYZER;

    pub fn load(apt: &mut Apt) -> Result<Self, String> {
        let analyzer = skate_ui::analyzer::TrickAnalyzer::load(&read_screen(ANALYZER)?)?;
        upload_textures(&analyzer.shapes, apt)?;
        Ok(Self { analyzer, idle: 0 })
    }

    /// One displayed frame while playing; false means a script error (switch it off).
    pub fn frame(&mut self, score: &skate_sim::live::Score) -> bool {
        // The clips show the stick motion: `nollie` (b3c9679, TU3 825E51A0) is set when the
        // trick's descriptor is a nollie variant, the motion up then down, in any stance.
        // Switch mirrors the clip (see skate_ui::analyzer): the gesture then always shows
        // what the player's thumb did.
        let [switch, _, nollie] = score.stance;
        let result = if score.trick.is_empty() {
            self.idle += 1;
            if self.idle == ANALYZER_HOLD { self.analyzer.clear() } else { Ok(()) }.and_then(|()| self.analyzer.update("", false, false))
        } else {
            self.idle = 0;
            let before = self.analyzer.shown().to_owned();
            let r = self.analyzer.update(&score.trick, nollie, switch);
            if self.analyzer.shown() != before {
                println!("ANALYZER trick {} stance switch/fakie/nollie {:?} gesture {}", score.trick, score.stance, self.analyzer.shown());
            }
            r
        };
        result.is_ok()
    }

    pub fn draw(&self, apt: &mut Apt) {
        if let Ok(draws) = self.analyzer.draw() {
            apt.draw(&draws);
        }
    }
}
