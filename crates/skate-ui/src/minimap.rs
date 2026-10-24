//! The HUD minimap (skate-game `hud_minimap.rs` and its part of `scoring_hud.rs`, PR #52 `36018e19`; research b568 to
//! b571): the frame movie (`hud2/hudminimap`, template 53 MiniMap: the round backdrop, glass and north needle clip),
//! shown and hidden through its screen's Show like retail's component (82665278); the heading-up map's heading from
//! the player's motion (82665388) stepped at the console's HUD rate; the called skaters' icons and the north pointer
//! drawn over it (8268CB40, 82665848) with the menu set's minimap icon textures.
//!
//! skate_vita: the movie is a linked screen on `menu_runtime` (as the manual meter); the texture keys of the icons are
//! [`TEXTURES`] with the [`TEXTURE_PREFIX`], uploaded by the host from the menu set.
use crate::fe::apt_scene::{self, Draw, Fill, Shapes, Vertex};
use crate::fe::apt_vm::{Value, Vm};
use crate::menu_runtime::{MenuRuntime, Natives};
use skate_core::minimap::{Heading, HeadingRules, Icon, IconRules};

pub struct Host;
impl Natives for Host {
    fn names(&self) -> Vec<String> {
        vec!["ScreenManager".into()]
    }
    // The screen registers with the front end's screen manager; the HUD has none to answer.
    fn call(&mut self, _: &mut Vm, _: &str, _: &str, _: &[Value]) -> Result<Value, String> {
        Ok(Value::Undefined)
    }
}

/// The menu set's minimap icon images (`texture/<name>`) the minimap draws, keyed [`TEXTURE_PREFIX`]`<name>`.
pub const TEXTURES: [&str; 2] = ["north_pointer", "mini_skater"];
pub const TEXTURE_PREFIX: &str = "minimap:";
/// The north pointer quad in map units (+-16 x +-100, 0x8232F9C0..0x8232F9FF; b571), scaled like the icons.
const NEEDLE_UNITS: [f32; 2] = [32.0, 200.0];
/// Retail's HUD update runs at the console's 30 a second; the smoothing is per update, so it is stepped at that rate.
const HUD_STEP: f32 = 1.0 / 30.0;

/// The map's heading stepped from the player's motion (upstream `MinimapHeading` / `track_heading`).
#[derive(Default)]
pub struct Tracker {
    pub heading: Heading,
    /// The needle's own eased heading (82665848).
    pub needle: Heading,
    /// The player's ground position (x, z), the map's centre.
    pub player: [f32; 2],
    last: Option<[f32; 3]>,
    carry: f32,
    since: f32,
}

impl Tracker {
    /// `dt` seconds with the player at `position` and the camera looking along `forward` (world).
    pub fn step(&mut self, dt: f32, position: [f32; 3], forward: [f32; 3]) {
        self.player = [position[0], position[2]];
        let rules = HeadingRules::default();
        self.carry += dt;
        self.since += dt;
        if self.carry < HUD_STEP {
            return;
        }
        let velocity = match self.last {
            Some(l) if self.since > 0.0 => [(position[0] - l[0]) / self.since, (position[2] - l[2]) / self.since],
            _ => [0.0; 2],
        };
        self.last = Some(position);
        self.since = 0.0;
        while self.carry >= HUD_STEP {
            self.carry -= HUD_STEP;
            // Slow: the camera's forward (82665388, b571).
            self.heading.step(velocity, [forward[0], forward[2]], &rules);
            let heading = self.heading;
            self.needle.follow(&heading, &rules);
        }
    }

    /// A teleport or a new world: no velocity from the jump.
    pub fn reset(&mut self) {
        self.last = None;
        self.carry = 0.0;
        self.since = 0.0;
    }
}

pub struct MiniMap {
    pub runtime: MenuRuntime<Host>,
    pub shapes: Shapes,
    shown: bool,
}

impl MiniMap {
    pub fn load(json: &serde_json::Value) -> Result<Self, String> {
        let shapes: Shapes = serde_json::from_value(json["shapes"].clone()).map_err(|e| e.to_string())?;
        let runtime = MenuRuntime::load(json, Host)?;
        Ok(Self { runtime, shapes, shown: false })
    }

    /// One displayed frame: shown or not (`skate_core::minimap::visible`), the needle's heading; the movie's timelines
    /// advance.
    pub fn update(&mut self, shown: bool, tracker: &Tracker) -> Result<(), String> {
        if shown != self.shown {
            self.shown = shown;
            self.runtime.call_screen("Show", vec![Value::Bool(shown)])?;
        }
        self.set_north(skate_core::minimap::needle_degrees(&tracker.needle))?;
        self.runtime.update()
    }

    pub fn shown(&self) -> bool {
        self.shown
    }

    /// Turn the north needle clip (`mScreen.mNorth`): retail's pointer (82665848) turns with the smoothed heading.
    fn set_north(&mut self, degrees: f32) -> Result<(), String> {
        let Some(controller) = self.runtime.controller else { return Ok(()) };
        let vm = &mut self.runtime.vm;
        let Value::Object(screen) = vm.get(controller, "mScreen") else { return Ok(()) };
        let Value::Object(north) = vm.get(screen, "mNorth") else { return Ok(()) };
        if vm.get(north, "_rotation") == Value::Number(degrees as f64) {
            return Ok(());
        }
        vm.set(north, "_rotation", Value::Number(degrees as f64))?;
        self.runtime.bindings.movie.transform_changed(vm, north, "_rotation")
    }

    /// The frame, then the icons in order (826656F8 -> 8268CB40), then the north pointer on top (82665848); nothing
    /// while hidden.
    pub fn draw(&self, tracker: &Tracker, icons: &[Icon]) -> Result<Vec<Draw>, String> {
        if !self.shown {
            return Ok(Vec::new());
        }
        let mut draws = apt_scene::draw(&self.runtime.bindings.movie, &self.runtime.vm, &self.shapes).map_err(|e| e.to_string())?;
        let rules = IconRules::default();
        for icon in icons {
            let Some((c, a)) = skate_core::minimap::place(icon, tracker.player, &tracker.heading, &rules) else { continue };
            let [r, g, b, alpha] = icon.colour;
            draws.push(quad(format!("{TEXTURE_PREFIX}{}", icon.texture), c, [r, g, b, alpha * a]));
        }
        draws.push(needle(skate_core::minimap::needle_degrees(&tracker.needle)));
        Ok(draws)
    }
}

/// A textured quad over four HUD corners (top-left, top-right, bottom-right, bottom-left).
fn quad(texture: String, c: [[f32; 2]; 4], multiply: [f32; 4]) -> Draw {
    let v = |i: usize, u: f32, w: f32| Vertex { position: c[i], uv: [u, w] };
    Draw { texture, vertices: vec![v(0, 0.0, 0.0), v(1, 1.0, 0.0), v(2, 1.0, 1.0), v(0, 0.0, 0.0), v(2, 1.0, 1.0), v(3, 0.0, 1.0)], multiply, add: [0.0; 4], fill: Fill::TextureClamped }
}

/// The north pointer turned `degrees` clockwise: map units turned, then the map matrix's scale and centre (traced).
fn needle(degrees: f32) -> Draw {
    let rules = IconRules::default();
    let (s, c) = degrees.to_radians().sin_cos();
    let [hw, hh] = [NEEDLE_UNITS[0] / 2.0, NEEDLE_UNITS[1] / 2.0];
    let corner = |x: f32, y: f32| [rules.centre[0] + (x * c - y * s) * rules.scale[0], rules.centre[1] + (x * s + y * c) * rules.scale[1]];
    quad(format!("{TEXTURE_PREFIX}north_pointer"), [corner(-hw, -hh), corner(hw, -hh), corner(hw, hh), corner(-hw, hh)], [1.0; 4])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Moving along +x for a second turns the heading towards +x; standing still eases towards the camera.
    #[test]
    fn the_heading_follows_the_travel_direction() {
        let mut t = Tracker::default();
        for i in 0..=60 {
            t.step(1.0 / 60.0, [i as f32 * 0.2, 0.0, 0.0], [0.0, 0.0, 1.0]);
        }
        assert!(t.heading.0[0] > 0.9, "{:?}", t.heading);
        assert_eq!(t.player, [12.0, 0.0]);
        let x = t.heading.0[0];
        for _ in 0..30 {
            t.step(1.0 / 60.0, [12.0, 0.0, 0.0], [0.0, 0.0, 1.0]);
        }
        assert!(t.heading.0[0] < x && t.heading.0[1] > 0.0);
    }

    /// The linked screen (`SKATE3_MINIMAP_SCREEN=<ui-screens>/runtime/hudminimap.linked.json`): it builds, Show draws
    /// the frame with the icons and the needle on top, hide draws nothing. Skipped without the file.
    #[test]
    fn the_minimap_screen_shows_and_hides() {
        let Some(bytes) = std::env::var_os("SKATE3_MINIMAP_SCREEN").and_then(|p| std::fs::read(p).ok()) else {
            eprintln!("skipped: SKATE3_MINIMAP_SCREEN not set");
            return;
        };
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let mut m = MiniMap::load(&json).unwrap();
        let mut t = Tracker::default();
        t.step(0.1, [0.0; 3], [0.0, 0.0, -1.0]);
        let icon = Icon { id: "a".into(), texture: "mini_skater".into(), position: [10.0, 0.0], angle: 0.0, scale: 1.0, fade: true, max_distance: 0.0, colour: [1.0; 4] };
        for _ in 0..30 {
            m.update(true, &t).unwrap();
        }
        let draws = m.draw(&t, std::slice::from_ref(&icon)).unwrap();
        eprintln!("minimap draws {}: {:?}", draws.len(), draws.iter().map(|d| d.texture.as_str()).collect::<Vec<_>>());
        assert!(draws.len() >= 3, "the frame, the icon and the needle");
        assert_eq!(draws.last().unwrap().texture, "minimap:north_pointer");
        m.update(false, &t).unwrap();
        assert!(m.draw(&t, &[icon]).unwrap().is_empty());
    }

    /// The pointer quad is centred on the map's centre and points up with no turn.
    #[test]
    fn the_needle_stands_on_the_map_centre() {
        let d = needle(0.0);
        let rules = IconRules::default();
        let (top, bottom) = (d.vertices[0].position, d.vertices[2].position);
        assert!(((top[0] + bottom[0]) / 2.0 - rules.centre[0]).abs() < 1e-3 && ((top[1] + bottom[1]) / 2.0 - rules.centre[1]).abs() < 1e-3);
        assert!(top[1] < bottom[1]);
    }
}
