//! The Object Dropper on the Vita (docs/object-dropper-survey.md step D4): the phone's dropper row opens the
//! session's editor (`Session::open_editor`), the renderer draws its frame (`Session::dropper_frame`): the editor's
//! camera, the park without the pieces it moved or deleted (the map pack's piece spans), and the catalogue items
//! (`catalog.pak`, each item's render read the first time it is drawn).
//!
//! The retail screen (the `ObjectDropper` movie, step D5) runs on the menu stage from [`screen_feed`] and the
//! editor's posted screen modes (`Session::take_dropper_posts`).
//!
//! Nothing is saved (owner, 2026-10-29): the edits live in the session, so they stay while the world stays loaded
//! (teleports inside it included) and go with a world change or when the game quits. A park is saved later through
//! the Create menu.
use std::collections::HashMap;

use glam::{Mat4, Vec3, Vec4};
#[cfg(target_os = "vita")]
use skate_sim::object_dropper::world::DropperFrame;
#[cfg(target_os = "vita")]
use vita_render::scene::{CatalogInstance, MarkerInstance, Scene};

#[cfg(target_os = "vita")]
const DIR: &str = "private/object-catalog";
/// Xbox buttons (`XboxState::buttons`).
pub(crate) const LB: u16 = 0x0100;
pub(crate) const B: u16 = 0x2000;
pub(crate) const Y: u16 = 0x8000;

/// The fade's far end scale on a park map (the visual indicators' `park_fade_scale`, the teleporter icons').
#[cfg(target_os = "vita")]
const PARK_FADE_SCALE: f32 = 10.0;

/// The popup callback of the open world's reset question.
pub(crate) const RESET_POPUP: &str = "object_dropper.reset";

/// What the `ObjectDropper` movie shows of the open editor (sim thread, each tick).
pub(crate) fn screen_feed(editor: &skate_sim::object_dropper::Editor) -> skate_ui::fe::object_dropper_host::ObjectDropperFeed {
    use skate_sim::object_dropper::{Part, Variant};
    let b = &editor.browse;
    let catalog = &editor.catalog;
    let name = |id: &str| catalog.item(id).map_or_else(|| id.to_owned(), |i| i.name.clone());
    let current = b.item().and_then(|id| catalog.item(id));
    let held = editor.held.as_ref().and_then(|h| catalog.item(&h.item));
    let meter = editor.meter();
    skate_ui::fe::object_dropper_host::ObjectDropperFeed {
        world: editor.variant == Variant::World,
        supers: b.listing.iter().map(|(k, _)| k.clone()).collect(),
        subs: b.sub_categories().iter().map(|(k, ids)| (k.clone(), ids.len())).collect(),
        items: b.items().iter().map(|id| name(id)).collect(),
        super_index: b.super_index,
        sub_index: b.sub_index,
        item_index: b.item_index,
        selecting_sub: b.subs,
        item: held.map(|i| {
            let [lo, hi] = i.cost.bounds;
            (i.name.clone(), std::array::from_fn(|k| hi[k] - lo[k]))
        }),
        terrain: current.or(held).is_some_and(|i| i.part == Part::Terrain),
        fine_tune: editor.fine_tune,
        duplicate: editor.duplicate,
        terrain_bar: meter.terrain,
        props_bar: meter.props,
        warning: meter.warning,
        quick: (editor.mode == skate_sim::object_dropper::Mode::Properties).then(|| skate_ui::fe::object_dropper_host::QuickFeed {
            rows: editor.quick_rows(),
            snap: editor.snapping,
            collision: editor.collision,
            invert_x: editor.invert[0] < 0.0,
            invert_y: editor.invert[1] < 0.0,
            cursor_speed: editor.cursor_speed,
            page: match editor.quick.page {
                skate_sim::object_dropper::quick::Page::Main => 0,
                skate_sim::object_dropper::quick::Page::Options => 1,
                skate_sim::object_dropper::quick::Page::Color => 2,
                skate_sim::object_dropper::quick::Page::Branding => 3,
                skate_sim::object_dropper::quick::Page::Style => 4,
            },
            style_index: editor.style_index(),
            style_allowed: editor.style_allowed(),
            colours: editor.tuning.colours.clone(),
            colour_index: editor.quick.colour,
            colour_allowed: editor.selection_tint().is_some(),
        }),
        colour: editor.colour(editor.selection_tint().unwrap_or(0)),
    }
}

#[cfg(target_os = "vita")]
fn model(t: &[f32; 12]) -> Mat4 {
    Mat4::from_cols(Vec4::new(t[0], t[1], t[2], 0.0), Vec4::new(t[3], t[4], t[5], 0.0), Vec4::new(t[6], t[7], t[8], 0.0), Vec4::new(t[9], t[10], t[11], 1.0))
}

/// The catalogue renders the scene holds (main thread).
#[cfg(target_os = "vita")]
#[derive(Default)]
pub(crate) struct CatalogDraws {
    keys: HashMap<String, u32>,
    /// Each part's shared texture count (`catalog.json` `parts`).
    textures: Option<[usize; 2]>,
    failed: std::collections::HashSet<String>,
    /// The markers (`catalog.json` `markers`): mesh index and pass intensities by marker row, loaded at the first
    /// frame that has one; None before.
    markers: Option<HashMap<String, (usize, [f32; 2], [f32; 4])>>,
}

#[cfg(target_os = "vita")]
impl CatalogDraws {
    fn read(rel: &str) -> Option<Vec<u8>> {
        skate_data::vfs::read(&std::path::Path::new(crate::packs::ROOT).join(rel)).ok()
    }

    fn texture_counts(&mut self) -> [usize; 2] {
        *self.textures.get_or_insert_with(|| {
            let counts = Self::read(&format!("{DIR}/catalog.json")).and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok()).map(|v| {
                [v["parts"]["terrain"]["textures"].as_u64().unwrap_or(0) as usize, v["parts"]["props"]["textures"].as_u64().unwrap_or(0) as usize]
            });
            counts.unwrap_or([0, 0])
        })
    }

    /// The scene key of `template`, its render loaded the first time.
    fn key(&mut self, scene: &mut Scene, template: &str, props: bool) -> Option<u32> {
        if let Some(&k) = self.keys.get(template) {
            return Some(k);
        }
        if self.failed.contains(template) {
            return None;
        }
        let part = if props { "props" } else { "terrain" };
        let count = self.texture_counts()[usize::from(props)];
        let item = Self::read(&format!("{DIR}/{part}/items/{template}.render")).ok_or_else(|| "no render".to_owned()).and_then(|b| vita_pack::catalog::ItemRender::from_pack(&b, count));
        let item = match item {
            Ok(item) => item,
            Err(error) => {
                println!("catalogue item {template}: {error}");
                self.failed.insert(template.to_owned());
                return None;
            }
        };
        let key = self.keys.len() as u32;
        scene.load_catalog_item(key, props, &item, &mut |n| {
            Self::read(&format!("{DIR}/{part}/textures/{n}.tex")).and_then(|b| vita_pack::catalog::texture_from_pack(&b).ok())
        });
        self.keys.insert(template.to_owned(), key);
        Some(key)
    }

    /// The frame's items and held item as scene instances.
    pub(crate) fn instances(&mut self, scene: &mut Scene, frame: &DropperFrame, out: &mut Vec<CatalogInstance>) {
        out.clear();
        for (_, template, props, placement, tint) in &frame.items {
            if let Some(key) = self.key(scene, template, *props) {
                out.push(CatalogInstance { key, model: model(placement), tint: *tint, outline: false });
            }
        }
        // The item in hand (and a held group) outlined, as retail (owner: only once taken).
        for (template, props, placement, tint) in frame.held.iter().chain(&frame.held_others) {
            if let Some(key) = self.key(scene, template, *props) {
                out.push(CatalogInstance { key, model: model(placement), tint: *tint, outline: true });
            }
        }
    }

    /// The frame's markers as scene instances (the meshes and colours loaded the first time). In the editor at full
    /// intensity (NOT RETAIL YET: retail's editor passes the indicator a style override not traced, `82866AD8`); in
    /// play (the teleporters' starts) with the row style's fade on view depth, its far end x10 on a park map as the
    /// teleporter icons': hidden up close, as the skater walks in. `camera`: the play camera (world matrix, -z ahead).
    pub(crate) fn markers(&mut self, scene: &mut Scene, frame: &DropperFrame, camera: &Mat4, out: &mut Vec<MarkerInstance>) {
        out.clear();
        if frame.markers.is_empty() {
            return;
        }
        let markers = self.markers.get_or_insert_with(|| load_markers(scene));
        let (eye, forward) = (camera.w_axis.truncate(), -camera.z_axis.truncate().normalize_or_zero());
        for (key, placement, colour) in &frame.markers {
            let Some(&(mesh, [front, back], mut fade)) = markers.get(key) else { continue };
            let model = model(placement);
            let alpha = if frame.view.is_some() {
                1.0
            } else {
                fade[3] *= PARK_FADE_SCALE;
                let depth = (model.w_axis.truncate() - eye).dot(forward);
                if depth <= 0.0 { 0.0 } else { crate::teleporter_icons::fade_alpha(fade, depth) }
            };
            if alpha > 0.0 {
                out.push(MarkerInstance { mesh, colour: usize::from(*colour), model, front: front * alpha, background: [back, (back * alpha).sqrt()] });
            }
        }
    }

    /// A new world: the scene's catalogue items are freed.
    pub(crate) fn clear(&mut self, scene: &mut Scene) {
        self.keys.clear();
        self.failed.clear();
        scene.unload_catalog();
    }
}

/// The catalogue's markers into the scene: one ribbon mesh per marker row, the colours in `MARKER_*` order (blue,
/// green, red); returns each row's mesh index and intensities.
#[cfg(target_os = "vita")]
fn load_markers(scene: &mut Scene) -> HashMap<String, (usize, [f32; 2], [f32; 4])> {
    let read = |rel: &str| skate_data::vfs::read(&std::path::Path::new(crate::packs::ROOT).join(rel)).ok();
    let Some(catalog) = read(&format!("{DIR}/catalog.json")).and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok()) else { return HashMap::new() };
    let (mut rows, mut meshes) = (HashMap::new(), Vec::new());
    for (key, row) in catalog["markers"].as_object().into_iter().flatten() {
        let offset: [f32; 3] = std::array::from_fn(|k| row["offset"][k].as_f64().unwrap_or(0.0) as f32);
        if let Some((vertices, indices, intensity, fade)) = crate::teleporter_icons::marker_mesh(&row["spline"], &row["style"], offset) {
            rows.insert(key.clone(), (meshes.len(), intensity, fade));
            meshes.push((vertices, indices));
        }
    }
    let texture = |path: &serde_json::Value| -> Option<vita_pack::texture::Texture> {
        let (w, h, rgba) = crate::teleporter_icons::decode_png(&read(path.as_str()?)?).ok()?;
        Some(vita_pack::texture::Texture { width: w as u16, height: h as u16, format: vita_pack::texture::Format::Rgba8, mips: vec![rgba] })
    };
    let colours: Vec<_> = ["blue", "green", "red"]
        .iter()
        .filter_map(|c| Some((texture(&catalog["marker_colours"][c]["front"])?, texture(&catalog["marker_colours"][c]["background"])?)))
        .collect();
    println!("object dropper markers: {} meshes, {} colours", meshes.len(), colours.len());
    scene.load_markers(&meshes, &colours);
    rows
}

/// The park spans of the frame's hidden pieces, sorted for the park draw.
pub(crate) fn hidden_spans(pieces: &HashMap<u32, Vec<[u32; 3]>>, hidden: &[u32], out: &mut Vec<[u32; 3]>) {
    out.clear();
    for id in hidden {
        if let Some(spans) = pieces.get(id) {
            out.extend_from_slice(spans);
        }
    }
    out.sort_unstable();
}

/// The editor's camera as a camera matrix (eye, looking at the target).
pub(crate) fn camera((eye, target): (Vec3, Vec3)) -> Mat4 {
    Mat4::look_at_rh(eye, target, Vec3::Y).inverse()
}
