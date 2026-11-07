//! The editor in the running world (docs/object-dropper-survey.md step D4): the session's [`Dropper`] owns the editor
//! while it is open and the scene always (what was moved and dropped stays in the world after the editor closes),
//! and turns the editor's events into world changes: a park's own piece is posed through its baked rows
//! (`GamePhysics::pose_park_piece`), a dropped item through the prop layer and the runtime props
//! (`GamePhysics::drop_item`). An item in hand is out of the world (its collision hidden or removed) until placed.
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use glam::{Affine3A, Vec3};

use super::{Catalog, Editor, Event, Ground, Meter, Mode, Origin, Part, Scene, Tuning};
use crate::physics::dropped::CatalogItem;
use crate::physics::park_edit::PiecePose;
use crate::physics::GamePhysics;

/// The baked catalogue's index and tuning (`vita-bake catalog`, in `catalog.pak`).
pub const CATALOG: &str = "private/object-catalog/catalog.json";

struct PhysicsGround<'a>(&'a GamePhysics);

impl Ground for PhysicsGround<'_> {
    fn below(&self, x: f32, z: f32, from: f32) -> Option<f32> {
        self.0.ground_below(x, z, from)
    }

    /// In a park, the first of the map's own surfaces at or under the surface the cursor is over: it goes through what is
    /// placed, whose bases are sunk below the ground, but not through the map (which has layers under its ground too; owner,
    /// device).
    fn floor_below(&self, x: f32, z: f32, at: f32) -> Option<f32> {
        const REACH: f32 = 100.0;
        if self.0.is_park() {
            let from = at + super::camera::UNDER;
            self.0.map_sweep([x, from, z], [x, from - REACH, z], 0.0).map(|(f, _)| from - REACH * f)
        } else {
            self.0.ground_below(x, z, at - super::camera::UNDER)
        }
    }

    fn sweep(&self, from: Vec3, to: Vec3) -> Option<(f32, Vec3)> {
        self.0.map_sweep(from.to_array(), to.to_array(), super::camera::SWEEP_RADIUS).map(|(f, n)| (f, Vec3::from(n)))
    }
}

fn to_glam(t: &[f32; 12]) -> Affine3A {
    Affine3A::from_cols(
        glam::Vec3A::new(t[0], t[1], t[2]),
        glam::Vec3A::new(t[3], t[4], t[5]),
        glam::Vec3A::new(t[6], t[7], t[8]),
        glam::Vec3A::new(t[9], t[10], t[11]),
    )
}

fn from_glam(a: &Affine3A) -> [f32; 12] {
    let m = a.matrix3;
    let t = a.translation;
    [m.x_axis.x, m.x_axis.y, m.x_axis.z, m.y_axis.x, m.y_axis.y, m.y_axis.z, m.z_axis.x, m.z_axis.y, m.z_axis.z, t.x, t.y, t.z]
}

/// The tint of the items group select has picked (NOT RETAIL YET: retail's selection highlight, `piece+0xE4`, is not
/// decoded).
const SELECTED: [f32; 3] = [0.55, 0.8, 1.0];
/// A held item that cannot be placed (Collision on, it goes into another): drawn red (retail's editor draws a piece in
/// this state red, `82866AD8` state 5; NOT RETAIL YET: its tint for a model is not traced).
const BLOCKED: [f32; 3] = [1.0, 0.25, 0.25];

/// What the renderer draws of the editor and its edits.
#[derive(Clone, Debug, PartialEq)]
pub struct DropperFrame {
    /// While the editor is open: its camera (eye, target), cursor, mode and meter.
    pub view: Option<(Vec3, Vec3)>,
    pub cursor: Vec3,
    pub mode: Mode,
    pub meter: Meter,
    /// The item in hand: template, props part, placement, colour (rgb, white when uncoloured).
    pub held: Option<(String, bool, [f32; 12], [f32; 3])>,
    /// A held group's other items, the same way.
    pub held_others: Vec<(String, bool, [f32; 12], [f32; 3])>,
    /// The park's own pieces whose layout draws are left out (moved, deleted or in hand).
    pub hidden: Vec<u32>,
    /// Items drawn from the catalogue: (scene id, template, props part, placement): moved park pieces, dropped
    /// terrain, and dropped props at their body's pose.
    pub items: Vec<(u32, String, bool, [f32; 12], [f32; 3])>,
    /// The player skater is not drawn: the park editor is open (retail hides the skater until it closes; the open
    /// world's dropper keeps it).
    pub hide_skater: bool,
    /// While the editor is open, the pieces without a model drawn as their marker (retail `82866AD8`): marker row,
    /// placement, colour (`MARKER_GREEN` highlighted or in hand, else `MARKER_BLUE`).
    pub markers: Vec<(String, [f32; 12], u8)>,
}

/// A park teleporter the player placed (retail `8287B918` / `8286D6E8`, as upstream's `layout_teleporters`): per
/// channel, the first start and end piece in the park; the entry is a box of half extents 2 (`8286CFF8`) about the
/// start's footprint middle with its axes, the exit the end's axes at its footprint middle (rows right, up, forward,
/// translation).
#[derive(Clone, Debug, PartialEq)]
pub struct ParkTeleporter {
    pub channel: u8,
    pub centre: [f32; 3],
    pub axes: [[f32; 3]; 3],
    pub half_extents: [f32; 3],
    pub exit: [[f32; 4]; 4],
}

/// The markers' colours (`82866AD8`: blue at rest, green while the cursor is on the piece).
pub const MARKER_BLUE: u8 = 0;
pub const MARKER_GREEN: u8 = 1;

pub struct Dropper {
    editor: Option<Editor>,
    /// The scene while the editor is closed (the editor holds it while open).
    scene: Option<Scene>,
    tuning: Tuning,
    catalog: Catalog,
    /// Loaded catalogue items by template.
    items: BTreeMap<String, CatalogItem>,
    /// The park's own pieces (their ids), for what to hide.
    authored: BTreeSet<u32>,
    /// The editor opened since the last update: its pad takes the held buttons first ([`Editor::prime`]).
    fresh: bool,
    /// The world changed since the last save was taken ([`Self::take_save`]).
    dirty: bool,
    /// Collision triangles by template (the Collision option's check), and the held poses last checked.
    shapes: BTreeMap<String, Vec<[Vec3; 3]>>,
    checked: Vec<(String, Vec3, glam::Quat)>,
    /// The park as its map has it (the baked layout's pieces and the spawner at the start): what a loaded layout is
    /// laid over.
    baseline: Vec<(u32, String, [f32; 12])>,
    /// Park Setup's lots and templates (`catalog.json` `park_lots`).
    pub lots: Vec<ParkLot>,
    /// Stock layouts the Challenge Map names beyond the lots' templates: {world row: layout} (`catalog.json` `park_worlds`,
    /// PCU Training - Basic).
    pub stock_worlds: BTreeMap<String, String>,
    /// The template world the park started from (a `.park` header's origin): the map's Fullpark, or the layout laid
    /// over it last.
    pub origin: u64,
}

/// A Park Setup lot (retail FE state 0x3A): its world, display name, unlock key and templates.
#[derive(Clone, Debug, PartialEq)]
pub struct ParkLot {
    pub lot: String,
    pub name: String,
    pub unlock: Option<String>,
    pub templates: Vec<ParkTemplate>,
}

/// A lot's template: its world, display name, thumbnail, stock layout (`parks/<layout>.park` in the catalogue pack),
/// origin hash and map.
#[derive(Clone, Debug, PartialEq)]
pub struct ParkTemplate {
    pub world: String,
    pub name: String,
    pub thumb: String,
    pub layout: Option<String>,
    pub origin: u64,
    pub map: String,
}

/// The catalogue pack's stock layouts (`parks/<layout>.park`).
pub const PARKS: &str = "private/object-catalog/parks";

fn park_lots(v: &serde_json::Value) -> Vec<ParkLot> {
    let text = |v: &serde_json::Value| v.as_str().unwrap_or_default().to_owned();
    v.as_array()
        .into_iter()
        .flatten()
        .map(|l| ParkLot {
            lot: text(&l["lot"]),
            name: text(&l["name"]),
            unlock: l["unlock"].as_str().map(str::to_owned),
            templates: l["templates"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|t| ParkTemplate {
                    world: text(&t["world"]),
                    name: text(&t["name"]),
                    thumb: text(&t["thumb"]),
                    layout: t["layout"].as_str().map(str::to_owned),
                    origin: t["origin"].as_str().and_then(|o| u64::from_str_radix(o, 16).ok()).unwrap_or(0),
                    map: text(&t["map"]),
                })
                .collect(),
        })
        .collect()
}

/// The park's spawner piece (`SkatePark_Spawner`) at the layout's player start (rows right, up, forward,
/// translation): its footprint's middle on the start, turned as the start (its marker's arrow, +z, along the start's
/// forward). NOT RETAIL YET: retail's spawner-to-start relation is not traced (the middle is our reading).
fn spawner(catalog: &Catalog, start: &serde_json::Value) -> Option<(u32, String, [f32; 12])> {
    let m: Vec<f32> = start.as_array()?.iter().filter_map(|r| r.as_array()).flatten().filter_map(|v| v.as_f64()).map(|v| v as f32).collect();
    (m.len() == 16).then(|| spawner_at(catalog, &std::array::from_fn(|r| std::array::from_fn(|c| m[r * 4 + c]))))?
}

/// The spawner piece for a start matrix (rows right, up, forward, translation), as [`spawner`].
fn spawner_at(catalog: &Catalog, m: &[[f32; 4]; 4]) -> Option<(u32, String, [f32; 12])> {
    let item = catalog.items.iter().find(|i| i.is_spawner())?;
    let row = |r: usize| Vec3::new(m[r][0], m[r][1], m[r][2]);
    let [lo, hi] = item.cost.bounds;
    let middle = Vec3::new((lo[0] + hi[0]) * 0.5, lo[1], (lo[2] + hi[2]) * 0.5);
    let origin = row(3) - (row(0) * middle.x + row(1) * middle.y + row(2) * middle.z);
    let (x, y, z) = (row(0), row(1), row(2));
    Some((super::scene::SPAWNER_ID, item.id.clone(), [x.x, x.y, x.z, y.x, y.y, y.z, z.x, z.y, z.z, origin.x, origin.y, origin.z]))
}

impl Dropper {
    /// The catalogue and tuning (`catalog.json`), and the scene of the world's park pieces with its spawner (the start
    /// of the map `map`, or of the park's layout).
    pub(crate) fn load(asset_root: &Path, physics: &GamePhysics, map: Option<&str>) -> Result<Self, String> {
        let path = asset_root.join(CATALOG);
        let json: serde_json::Value =
            serde_json::from_slice(&skate_data::vfs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?).map_err(|e| format!("{}: {e}", path.display()))?;
        let catalog = Catalog::from_json(&json)?;
        let tuning = Tuning::from_catalog(&json["tuning"])?;
        let layout = physics.park_layout();
        let authored = layout.iter().map(|(id, ..)| *id).collect();
        // A layout piece's template is its catalogue item (the static recipe's mod id).
        let mut pieces: Vec<_> = layout.into_iter().filter(|(_, template, _)| catalog.item(template).is_some()).collect();
        let start = [map, physics.park_layout_name()].into_iter().flatten().map(|name| &json["park_starts"][name]).find(|s| s.is_array());
        if let Some(spawner) = start.filter(|_| physics.is_park()).and_then(|start| spawner(&catalog, start)) {
            pieces.push(spawner);
        }
        let scene = Scene::from_layout(pieces.clone());
        let lots = park_lots(&json["park_lots"]);
        let stock_worlds = json["park_worlds"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|w| Some((w["world"].as_str()?.to_owned(), w["layout"].as_str()?.to_owned())))
            .collect();
        // The map's Fullpark (Phase 2 bakes it in): its template's origin.
        let fullpark = physics.park_layout_name();
        let origin = lots
            .iter()
            .flat_map(|l| &l.templates)
            .find(|t| map.is_some_and(|m| t.map.eq_ignore_ascii_case(m)) && (t.layout.as_deref() == fullpark || t.world.ends_with("_full")))
            .map_or(0, |t| t.origin);
        Ok(Self {
            editor: None,
            scene: Some(scene),
            tuning,
            catalog,
            items: BTreeMap::new(),
            authored,
            fresh: false,
            dirty: false,
            shapes: BTreeMap::new(),
            checked: Vec::new(),
            baseline: pieces,
            lots,
            stock_worlds,
            origin,
        })
    }

    pub fn is_open(&self) -> bool {
        self.editor.is_some()
    }

    /// Opens the editor with its cursor at `at` and the camera turned to `yaw`, as the park editor or the open-world
    /// dropper, its catalogue listing `listing`. The open world's camera stands where it opens and only looks around
    /// (owner, on retail), unless `free_camera` (ours, a setting) lets it follow the cursor as the park editor's does.
    pub(crate) fn open(&mut self, at: Vec3, yaw: f32, variant: super::Variant, listing: super::Listing, free_camera: bool) {
        if self.editor.is_none() {
            let scene = self.scene.take().unwrap_or_default();
            let mut editor = Editor::open(self.tuning.clone(), self.catalog.clone(), scene, at, yaw);
            editor.variant = variant;
            if variant == super::Variant::World && !free_camera {
                editor.fix_camera();
            }
            editor.set_listing(listing);
            editor.take_posts();
            self.editor = Some(editor);
            self.fresh = true;
        }
    }

    /// The colour of a `Tuning::colours` index.
    fn colour(&self, tint: u8) -> [f32; 3] {
        self.tuning.colours.get(usize::from(tint)).copied().unwrap_or([1.0; 3])
    }

    /// The open editor's limits (the measuring ones while the debug overlay is on).
    pub(crate) fn set_limits(&mut self, limits: super::Limits) {
        if let Some(editor) = self.editor.as_mut() {
            editor.limits = limits;
        }
    }

    /// The open editor (the screen reads its state).
    pub fn editor(&self) -> Option<&Editor> {
        self.editor.as_ref()
    }

    /// The screen modes the editor posted since the last call.
    pub(crate) fn take_posts(&mut self) -> Vec<u8> {
        self.editor.as_mut().map(|e| e.take_posts()).unwrap_or_default()
    }

    /// The Quick Menu's list pages (Style, Branding): the movie's highlight or select (`select`) of row `index`; the
    /// world follows a style swap.
    pub(crate) fn quick_list(&mut self, index: usize, select: bool, physics: &mut GamePhysics, asset_root: &Path) {
        let Some(editor) = self.editor.as_mut() else { return };
        let events = match (editor.quick.page, select) {
            (super::quick::Page::Style, false) => editor.style_highlight(index),
            (super::quick::Page::Style, true) => {
                editor.style_select();
                Vec::new()
            }
            (super::quick::Page::Branding, true) => {
                editor.branding_select();
                Vec::new()
            }
            _ => Vec::new(),
        };
        for e in &events {
            self.apply(e, physics, asset_root);
        }
    }

    /// The Quick Menu's screen modes posted since the last call.
    pub(crate) fn take_quick_posts(&mut self) -> Vec<u8> {
        self.editor.as_mut().map(|e| e.take_quick_posts()).unwrap_or_default()
    }

    pub(crate) fn close(&mut self, physics: &mut GamePhysics, asset_root: &Path) {
        if let Some(mut editor) = self.editor.take() {
            // Whatever is in hand goes back where it was.
            let mut events = Vec::new();
            if editor.held.is_some() {
                editor.update(&[0.0; 24], &PhysicsGround(physics));
                if let Some(held) = editor.held.take() {
                    for (id, position, rotation) in held.source.into_iter().chain(held.others.iter().filter_map(|m| m.source)) {
                        editor.scene.set_pose(id, position, rotation);
                        events.push(Event::Placed(id));
                    }
                }
            }
            self.scene = Some(std::mem::take(&mut editor.scene));
            for e in &events {
                self.apply(e, physics, asset_root);
            }
        }
    }

    fn scene(&self) -> &Scene {
        self.editor.as_ref().map_or_else(|| self.scene.as_ref().expect("the dropper's scene"), |e| &e.scene)
    }

    fn scene_mut(&mut self) -> &mut Scene {
        match self.editor.as_mut() {
            Some(e) => &mut e.scene,
            None => self.scene.get_or_insert_with(Scene::default),
        }
    }

    /// The edits to save, when the world changed since the last call.
    pub(crate) fn take_save(&mut self) -> Option<super::save::Saved> {
        std::mem::take(&mut self.dirty).then(|| super::save::Saved::of(self.scene(), &self.authored))
    }

    /// Replays a save into the world (the editor closed).
    pub(crate) fn restore(&mut self, saved: &super::save::Saved, physics: &mut GamePhysics, asset_root: &Path) {
        let catalog = self.catalog.clone();
        let (moved, deleted, dropped) = super::save::apply(saved, self.scene_mut(), |item| catalog.item(item).is_some_and(|i| i.template.is_some()));
        let events: Vec<Event> = moved.into_iter().chain(dropped).map(Event::Placed).chain(deleted.into_iter().map(Event::Removed)).collect();
        for e in &events {
            self.apply(e, physics, asset_root);
        }
        self.dirty = false;
    }

    /// Every edit undone (the park back to its layout, the drops gone): the editor's reset for all at once. An open
    /// editor stays open, back in its free camera with nothing in hand (retail `825C6638`).
    pub(crate) fn reset_all(&mut self, physics: &mut GamePhysics, _asset_root: &Path) {
        let dropped: Vec<u32> = self.scene().items.iter().filter(|p| p.origin == Origin::Dropped).map(|p| p.id).collect();
        let layout = physics.park_layout();
        let catalog = self.catalog.clone();
        let scene = Scene::from_layout(layout.into_iter().filter(|(_, template, _)| catalog.item(template).is_some()));
        match self.editor.as_mut() {
            Some(editor) => editor.reset_scene(scene),
            None => self.scene = Some(scene),
        }
        for id in dropped {
            physics.remove_dropped(id);
        }
        for id in self.authored.clone() {
            if let Err(error) = physics.pose_park_piece(id, PiecePose::Authored) {
                tracing::warn!("SKATE_DROPPER: reset {id}: {error}");
            }
        }
        self.dirty = true;
    }

    fn item(&mut self, asset_root: &Path, item: &str) -> Option<CatalogItem> {
        let entry = self.catalog.item(item)?;
        let template = entry.template.clone()?;
        if !self.items.contains_key(&template) {
            match crate::physics::dropped::load_item(asset_root, entry.part == Part::Props, &template) {
                Ok(loaded) => {
                    self.items.insert(template.clone(), loaded);
                }
                Err(error) => {
                    tracing::warn!("SKATE_DROPPER: item {item}: {error}");
                    return None;
                }
            }
        }
        self.items.get(&template).cloned()
    }

    /// One editor event in the world.
    fn apply(&mut self, event: &Event, physics: &mut GamePhysics, asset_root: &Path) {
        if matches!(event, Event::Placed(_) | Event::Removed(_) | Event::Reset(_)) {
            self.dirty = true;
        }
        // A marker (the spawner, a teleporter) has no model, collision or body.
        let id = match *event {
            Event::Taken(Some(id)) | Event::Placed(id) | Event::Removed(id) | Event::Reset(id) => Some(id),
            _ => None,
        };
        if id.and_then(|id| self.scene().get(id)).and_then(|p| self.catalog.item(&p.item)).is_some_and(|i| i.template.is_none()) {
            return;
        }
        let result = match *event {
            Event::Taken(Some(id)) => {
                if self.authored.contains(&id) {
                    physics.pose_park_piece(id, PiecePose::Hidden)
                } else {
                    physics.remove_dropped(id);
                    Ok(())
                }
            }
            Event::Placed(id) => {
                let Some(placed) = self.scene().get(id).cloned() else { return };
                match placed.origin {
                    Origin::Authored { position, rotation, scale, .. } => {
                        let authored = to_glam(&super::scene::affine(position, rotation, scale));
                        let now = to_glam(&placed.affine());
                        let delta = now * authored.inverse();
                        if delta.abs_diff_eq(Affine3A::IDENTITY, 1e-6) {
                            physics.pose_park_piece(id, PiecePose::Authored)
                        } else {
                            physics.pose_park_piece(id, PiecePose::Moved(from_glam(&delta)))
                        }
                    }
                    Origin::Dropped => {
                        physics.remove_dropped(id);
                        match self.item(asset_root, &placed.item) {
                            Some(item) if physics.drop_item(id, &item, &placed.affine()) => Ok(()),
                            _ => Err(format!("item {} not dropped", placed.item)),
                        }
                    }
                }
            }
            Event::Removed(id) => {
                if self.authored.contains(&id) {
                    physics.pose_park_piece(id, PiecePose::Hidden)
                } else {
                    physics.remove_dropped(id);
                    Ok(())
                }
            }
            Event::Reset(id) => physics.pose_park_piece(id, PiecePose::Authored),
            _ => Ok(()),
        };
        if let Err(error) = result {
            tracing::warn!("SKATE_DROPPER: {event:?}: {error}");
        }
    }

    /// One fixed update of the open editor from the pad slots; the world follows its events. Returns them (the
    /// caller closes on [`Event::Exit`]).
    pub(crate) fn update(&mut self, slots: &[f32; 24], physics: &mut GamePhysics, asset_root: &Path) -> Vec<Event> {
        let Some(editor) = self.editor.as_mut() else { return Vec::new() };
        if std::mem::take(&mut self.fresh) {
            editor.prime(slots);
        }
        let events = editor.update(slots, &PhysicsGround(physics));
        for e in &events {
            self.apply(e, physics, asset_root);
        }
        self.check_overlap(asset_root);
        events
    }

    /// The Collision option's check (`overlap`): whether the held items go into a world item, again only when they
    /// moved. Markers have no collision.
    fn check_overlap(&mut self, asset_root: &Path) {
        let Some(editor) = self.editor.as_ref() else { return };
        let mut held: Vec<(String, Vec3, glam::Quat, Vec3)> = Vec::new();
        if let (true, Some(h), Some((position, rotation))) = (editor.collision, editor.held.as_ref(), editor.held_pose()) {
            let scale = h.source.and_then(|(id, ..)| editor.scene.get(id)).map_or(Vec3::ONE, |p| p.scale);
            held.push((h.item.clone(), position, rotation, scale));
            held.extend(h.others.iter().zip(editor.member_poses()).map(|(m, (p, r))| (m.item.clone(), p, r, m.scale)));
        }
        held.retain(|(item, ..)| self.catalog.item(item).is_some_and(|i| i.template.is_some() && !i.allows_collision()));
        let key: Vec<(String, Vec3, glam::Quat)> = held.iter().map(|(i, p, r, _)| (i.clone(), *p, *r)).collect();
        if key == self.checked {
            return;
        }
        self.checked = key;
        let in_hand: BTreeSet<u32> = editor
            .held
            .as_ref()
            .map(|h| h.source.map(|s| s.0).into_iter().chain(h.others.iter().filter_map(|m| m.source.map(|s| s.0))).collect())
            .unwrap_or_default();
        let posed = |catalog: &Catalog, item: &str, position: Vec3, rotation: glam::Quat, scale: Vec3| {
            let [lo, hi] = catalog.item(item)?.cost.bounds;
            Some(super::overlap::Posed { lo: Vec3::from(lo) * scale, hi: Vec3::from(hi) * scale, position, rotation })
        };
        let others: Vec<(String, super::overlap::Posed, Vec3)> = editor
            .scene
            .items
            .iter()
            .filter(|p| !in_hand.contains(&p.id))
            .filter_map(|p| Some((p.item.clone(), posed(&self.catalog, &p.item, p.position, p.rotation, p.scale)?, p.scale)))
            .collect();
        let mut blocked = false;
        'held: for (item, position, rotation, scale) in &held {
            let Some(a) = posed(&self.catalog, item, *position, *rotation, *scale) else { continue };
            for (other, b, other_scale) in &others {
                if self.catalog.item(other).is_none_or(|i| i.template.is_none() || i.allows_collision()) || !super::overlap::boxes_overlap(&a, b) {
                    continue;
                }
                let (Some(ta), Some(tb)) = (self.shape(asset_root, item, *scale), self.shape(asset_root, other, *other_scale)) else { continue };
                if super::overlap::shapes_overlap(&a, &ta, b, &tb) {
                    blocked = true;
                    break 'held;
                }
            }
        }
        if let Some(editor) = self.editor.as_mut() {
            editor.blocked = blocked;
        }
    }

    /// An item's collision triangles in template space, scaled (loaded once per template).
    fn shape(&mut self, asset_root: &Path, item: &str, scale: Vec3) -> Option<Vec<[Vec3; 3]>> {
        let entry = self.catalog.item(item)?;
        let (template, props) = (entry.template.clone()?, entry.part == Part::Props);
        if !self.shapes.contains_key(&template) {
            let tris = match crate::physics::dropped::load_item(asset_root, props, &template) {
                Ok(loaded) => loaded.local.iter().map(|t| t.map(|v| Vec3::new(v.x, v.y, v.z))).collect(),
                Err(error) => {
                    tracing::warn!("SKATE_DROPPER: shape {item}: {error}");
                    Vec::new()
                }
            };
            self.shapes.insert(template.clone(), tris);
        }
        let tris = self.shapes.get(&template)?;
        Some(if scale == Vec3::ONE { tris.clone() } else { tris.iter().map(|t| t.map(|v| v * scale)).collect() })
    }

    /// The catalogue's choice (the screen calls it with an item id).
    pub(crate) fn choose(&mut self, item: &str) -> Vec<Event> {
        let mut events = Vec::new();
        if let Some(editor) = self.editor.as_mut() {
            editor.choose(item, &mut events);
        }
        events
    }

    pub fn catalog(&self) -> &Catalog {
        &self.catalog
    }

    /// The park as a layout file (Save Park): its pieces (the layout's left, moved or coloured, the dropped terrain and
    /// teleporters) with their tint's colour, its dropped props, the spawner as the start; `origin` (the template world
    /// it started from) and `frame`'s tail (the lot's stock layout) as retail would keep them.
    pub(crate) fn to_park(&self, origin: u64, frame: &crate::park_file::ParkFile) -> crate::park_file::ParkFile {
        use crate::park_file::{ParkFile, Piece, Prop};
        let scene = self.scene();
        let id = |item: &str| u64::from_str_radix(item, 16).ok();
        let (mut pieces, mut props) = (Vec::new(), Vec::new());
        for p in &scene.items {
            let Some(entry) = self.catalog.item(&p.item).filter(|i| !i.is_spawner()) else { continue };
            let Some(item) = id(&p.item) else { continue };
            if entry.part == Part::Props {
                props.push(Prop { position: p.position, rotation: p.rotation, item, flag: 1 });
            } else {
                pieces.push(Piece { position: p.position, rotation: p.rotation, item, tint: self.colour(p.tint) });
            }
        }
        let start = self.spawner_pose();
        ParkFile { version: crate::park_file::VERSION, origin, pieces, props, start: start.or(frame.start), tail: frame.tail.clone() }
    }

    /// The template a `.park` origin names.
    pub fn template(&self, origin: u64) -> Option<&ParkTemplate> {
        self.lots.iter().flat_map(|l| &l.templates).find(|t| t.origin == origin)
    }

    /// The park has edits not saved yet (My Parks asks before loading over them, retail `82511168`).
    pub(crate) fn unsaved(&self) -> bool {
        self.dirty
    }

    /// The park as `.park` bytes (Save Park): written in its template's stock layout (the tail, the origin); its
    /// edits count as saved from here.
    pub(crate) fn park_bytes(&mut self, asset_root: &Path) -> Vec<u8> {
        self.dirty = false;
        let frame = self
            .template(self.origin)
            .and_then(|t| t.layout.as_ref())
            .and_then(|layout| skate_data::vfs::read(&asset_root.join(PARKS).join(format!("{layout}.park"))).ok())
            .and_then(|b| crate::park_file::ParkFile::parse(&b).ok());
        let frame = frame.unwrap_or(crate::park_file::ParkFile { version: crate::park_file::VERSION, origin: self.origin, pieces: Vec::new(), props: Vec::new(), start: None, tail: Vec::new() });
        self.to_park(self.origin, &frame).to_bytes()
    }

    /// A stock layout of the catalogue pack by name (`parks/<layout>.park`).
    fn stock(asset_root: &Path, layout: &str) -> Option<crate::park_file::ParkFile> {
        let bytes = skate_data::vfs::read(&asset_root.join(PARKS).join(format!("{layout}.park"))).ok()?;
        crate::park_file::ParkFile::parse(&bytes).ok()
    }

    /// Park Setup's lots, in retail's order.
    pub fn lots(&self) -> &[ParkLot] {
        &self.lots
    }

    /// A Park Setup template (`world`, its row's key) as `.park` bytes: its stock layout, or for a bare lot (an Empty
    /// without a file, retail lays nothing) no pieces, with the start and tail of its lot's Full layout.
    pub(crate) fn template_park(&self, world: &str, asset_root: &Path) -> Result<Vec<u8>, String> {
        if let Some(layout) = self.stock_worlds.get(world) {
            return skate_data::vfs::read(&asset_root.join(PARKS).join(format!("{layout}.park"))).map_err(|e| format!("{layout}: {e}"));
        }
        let lot = self.lots.iter().find(|l| l.templates.iter().any(|t| t.world == world)).ok_or(format!("no template {world}"))?;
        let template = lot.templates.iter().find(|t| t.world == world).ok_or(format!("no template {world}"))?;
        if let Some(layout) = &template.layout {
            return skate_data::vfs::read(&asset_root.join(PARKS).join(format!("{layout}.park"))).map_err(|e| format!("{layout}: {e}"));
        }
        let full = lot.templates.iter().filter_map(|t| t.layout.as_deref()).find_map(|l| Self::stock(asset_root, l)).ok_or(format!("{}: no layout", lot.lot))?;
        Ok(crate::park_file::ParkFile { origin: template.origin, pieces: Vec::new(), props: Vec::new(), ..full }.to_bytes())
    }

    /// Loads a saved park (`.park` bytes, My Parks) over this park: the map's own props (its Fullpark's) are not dropped
    /// again. Returns the pieces kept, dropped and refused.
    pub(crate) fn load_park(&mut self, bytes: &[u8], map: Option<&str>, physics: &mut GamePhysics, asset_root: &Path) -> Result<(usize, usize, usize), String> {
        let file = crate::park_file::ParkFile::parse(bytes)?;
        let fullpark = self
            .lots
            .iter()
            .flat_map(|l| &l.templates)
            .find(|t| map.is_some_and(|m| t.map.eq_ignore_ascii_case(m)) && t.world.ends_with("_full"))
            .and_then(|t| t.layout.clone())
            .and_then(|layout| Self::stock(asset_root, &layout));
        let skip = fullpark.map(|f| f.props).unwrap_or_default();
        Ok(self.apply_park(&file, &skip, physics, asset_root))
    }

    /// Lays a layout file over the park (a template, a saved park): the map's own pieces the layout has stay (moved to the
    /// layout's pose, the nearest of their kind first), the rest of them go, the layout's other pieces and props are
    /// dropped while the budgets allow (the rest are counted in the result), the spawner goes to the start; the world
    /// follows the difference with the scene now. `skip_props`: props the map already has (its stock layout's). Closes
    /// the editor first. Returns the pieces kept, dropped and refused.
    pub(crate) fn apply_park(&mut self, file: &crate::park_file::ParkFile, skip_props: &[crate::park_file::Prop], physics: &mut GamePhysics, asset_root: &Path) -> (usize, usize, usize) {
        self.close(physics, asset_root);
        let mut target = Scene::from_layout(self.baseline.iter().filter(|(id, ..)| *id != super::scene::SPAWNER_ID).cloned());
        let item_of = |id: u64| format!("{id:016X}");
        let tint_of = |rgb: [f32; 3], colours: &[[f32; 3]]| -> u8 {
            let d = |c: &[f32; 3]| (0..3).map(|k| (c[k] - rgb[k]).powi(2)).sum::<f32>();
            colours.iter().enumerate().min_by(|a, b| d(a.1).total_cmp(&d(b.1))).map_or(0, |(i, _)| i as u8)
        };
        // The map's pieces by kind, unclaimed.
        let mut free: Vec<(u32, String, Vec3)> = target.items.iter().map(|p| (p.id, p.item.clone(), p.position)).collect();
        let mut extra = Vec::new();
        // Exact matches first, then the nearest of the kind.
        let mut left: Vec<&crate::park_file::Piece> = Vec::new();
        for piece in &file.pieces {
            let item = item_of(piece.item);
            let exact = free.iter().position(|(id, i, at)| {
                *i == item && at.distance(piece.position) < 0.01 && target.get(*id).is_some_and(|p| p.rotation.dot(piece.rotation).abs() > 0.9999)
            });
            match exact {
                Some(k) => {
                    let (id, ..) = free.swap_remove(k);
                    target.set_tint(id, tint_of(piece.tint, &self.tuning.colours));
                }
                None => left.push(piece),
            }
        }
        for piece in left {
            let item = item_of(piece.item);
            let nearest = free.iter().enumerate().filter(|(_, (_, i, _))| *i == item).min_by(|a, b| a.1 .2.distance(piece.position).total_cmp(&b.1 .2.distance(piece.position))).map(|(k, _)| k);
            match nearest {
                Some(k) => {
                    let (id, ..) = free.swap_remove(k);
                    target.set_pose(id, piece.position, piece.rotation);
                    target.set_tint(id, tint_of(piece.tint, &self.tuning.colours));
                }
                None => extra.push((item, piece.position, piece.rotation, tint_of(piece.tint, &self.tuning.colours))),
            }
        }
        let kept = target.items.len() - free.len();
        for (id, ..) in free {
            target.remove(id);
        }
        for prop in &file.props {
            let skip = skip_props.iter().any(|s| s.item == prop.item && s.position.distance(prop.position) < 0.01);
            if !skip {
                extra.push((item_of(prop.item), prop.position, prop.rotation, 0));
            }
        }
        // Dropped while the budgets allow, as the editor would.
        let limits = super::Limits::default();
        let (mut dropped, mut refused) = (0, 0);
        for (item, position, rotation, tint) in extra {
            let Some(entry) = self.catalog.item(&item) else {
                refused += 1;
                continue;
            };
            if !super::budget::allows(&limits, &target.usage(&self.catalog), &super::Usage::placed(&self.catalog, [entry])) {
                refused += 1;
                continue;
            }
            let id = target.drop_item(&item, position, rotation);
            target.set_tint(id, tint);
            dropped += 1;
        }
        let spawner = file.start.as_ref().and_then(|m| spawner_at(&self.catalog, m)).or_else(|| self.baseline.iter().find(|(id, ..)| *id == super::scene::SPAWNER_ID).cloned());
        if let Some(spawner) = spawner {
            target.items.push(Scene::from_layout([spawner]).items.remove(0));
        }
        // The world follows: what went or changed kind is removed (with the scene it was in), then what is new or moved
        // is placed.
        let old = std::mem::replace(self.scene_mut(), Scene::default());
        let gone: Vec<u32> = old.items.iter().filter(|p| target.get(p.id).is_none_or(|t| t.item != p.item)).map(|p| p.id).collect();
        *self.scene_mut() = old.clone();
        for id in &gone {
            self.apply(&Event::Removed(*id), physics, asset_root);
        }
        let placed: Vec<u32> = target.items.iter().filter(|t| old.get(t.id).is_none_or(|p| p != *t)).map(|t| t.id).collect();
        *self.scene_mut() = target;
        for id in placed {
            self.apply(&Event::Placed(id), physics, asset_root);
        }
        self.checked.clear();
        self.origin = file.origin;
        // A park just laid has no unsaved edits.
        self.dirty = false;
        (kept, dropped, refused)
    }

    /// Where the skater stands at the park's spawner (retail `825CB440` closing the park editor: the spawner piece's
    /// pose, its axes made orthonormal, `825C5710`, given to the skater, `8286DC78`): rows right, up, forward,
    /// translation. Ours stands it on the footprint's middle, the layout's start where the spawner was placed (NOT
    /// RETAIL YET: see [`spawner`]). None outside a park.
    pub(crate) fn spawner_pose(&self) -> Option<[[f32; 4]; 4]> {
        let p = self.scene().get(super::scene::SPAWNER_ID)?;
        let [lo, hi] = self.catalog.item(&p.item)?.cost.bounds;
        let middle = Vec3::new((lo[0] + hi[0]) * 0.5, lo[1], (lo[2] + hi[2]) * 0.5) * p.scale;
        let at = p.position + p.rotation * middle;
        let row = |v: Vec3, w: f32| [v.x, v.y, v.z, w];
        Some([row(p.rotation * Vec3::X, 0.0), row(p.rotation * Vec3::Y, 0.0), row(p.rotation * Vec3::Z, 0.0), row(at, 1.0)])
    }

    /// The park teleporters placed in the world (closed or open editor).
    pub(crate) fn teleporters(&self) -> Vec<ParkTeleporter> {
        let scene = self.scene();
        let first = |role: u8, channel: u8| {
            scene.items.iter().find_map(|p| {
                let item = self.catalog.item(&p.item)?;
                (item.teleport == Some((role, channel))).then_some((p, item))
            })
        };
        let middle = |p: &super::Placed, item: &super::Item| {
            let [lo, hi] = item.cost.bounds;
            p.position + p.rotation * (Vec3::new(lo[0] + hi[0], lo[1] + hi[1], lo[2] + hi[2]) * 0.5 * p.scale)
        };
        (0..4)
            .filter_map(|channel| {
                let ((start, s_item), (end, e_item)) = (first(0, channel)?, first(1, channel)?);
                let axes = [start.rotation * Vec3::X, start.rotation * Vec3::Y, start.rotation * Vec3::Z].map(|a| a.to_array());
                let at = middle(end, e_item);
                let row = |v: Vec3, w: f32| [v.x, v.y, v.z, w];
                let exit = [row(end.rotation * Vec3::X, 0.0), row(end.rotation * Vec3::Y, 0.0), row(end.rotation * Vec3::Z, 0.0), row(at, 1.0)];
                Some(ParkTeleporter { channel, centre: middle(start, s_item).to_array(), axes, half_extents: [2.0; 3], exit })
            })
            .collect()
    }

    /// What to draw now.
    pub(crate) fn frame(&self, physics: &GamePhysics) -> DropperFrame {
        let scene = self.scene();
        let template = |item: &str| self.catalog.item(item).and_then(|i| Some((i.template.clone()?, i.part == Part::Props)));
        // Items in hand (a held group's every source) are drawn from the hand, not where they were; the group being
        // selected is drawn tinted.
        let in_hand: BTreeSet<u32> = self
            .editor
            .as_ref()
            .and_then(|e| e.held.as_ref())
            .map(|h| h.source.map(|s| s.0).into_iter().chain(h.others.iter().filter_map(|m| m.source.map(|s| s.0))).collect())
            .unwrap_or_default();
        let selected: BTreeSet<u32> = self.editor.as_ref().filter(|e| e.mode == Mode::Group).map(|e| e.group.iter().copied().collect()).unwrap_or_default();
        let shade = |id: u32, tint: u8| if selected.contains(&id) { SELECTED } else { self.colour(tint) };
        // One pass over the scene (a park has hundreds of pieces and this runs every tick): the deleted pieces are
        // the authored ids it no longer holds.
        let mut hidden = Vec::new();
        let mut items = Vec::new();
        let mut present = BTreeSet::new();
        for p in &scene.items {
            let Origin::Authored { position, rotation, .. } = p.origin else { continue };
            present.insert(p.id);
            if in_hand.contains(&p.id) {
                hidden.push(p.id);
            } else if p.position != position || p.rotation != rotation || p.tint != 0 || selected.contains(&p.id) {
                // Moved, coloured or selected: out of the park's fused draws, drawn from the catalogue.
                hidden.push(p.id);
                if let Some((t, props)) = template(&p.item) {
                    items.push((p.id, t, props, p.affine(), shade(p.id, p.tint)));
                }
            }
        }
        hidden.extend(self.authored.iter().filter(|id| !present.contains(id)));
        for p in scene.items.iter().filter(|p| p.origin == Origin::Dropped && !in_hand.contains(&p.id)) {
            let Some((t, props)) = template(&p.item) else { continue };
            let placement = match physics.dropped_body(p.id).and_then(|b| physics.prop_dynamics().and_then(|d| d.pose(b))) {
                Some((o, basis)) => {
                    let c = basis.columns;
                    [c[0][0], c[0][1], c[0][2], c[1][0], c[1][1], c[1][2], c[2][0], c[2][1], c[2][2], o.x, o.y, o.z]
                }
                None => p.affine(),
            };
            items.push((p.id, t, props, placement, shade(p.id, p.tint)));
        }
        let mut held_others = Vec::new();
        if let Some((e, h)) = self.editor.as_ref().and_then(|e| Some((e, e.held.as_ref()?))) {
            for (m, (position, rotation)) in h.others.iter().zip(e.member_poses()) {
                let tint = if e.collision && e.blocked { BLOCKED } else { self.colour(m.tint) };
                held_others.push((m.template.clone(), template(&m.item).is_some_and(|t| t.1), super::scene::affine(position, rotation, m.scale), tint));
            }
        }
        // The markers: the scene's (green under the cursor or in the group being selected), then those in hand; in play
        // only the teleporters' starts (retail draws no end or spawner out of the editor).
        let marker = |item: &str| self.catalog.item(item).and_then(|i| i.marker.clone());
        let mut markers = Vec::new();
        if self.editor.is_none() {
            for p in &scene.items {
                if self.catalog.item(&p.item).is_some_and(|i| i.teleport.is_some_and(|t| t.0 == 0)) {
                    markers.extend(marker(&p.item).map(|key| (key, p.affine(), MARKER_BLUE)));
                }
            }
        }
        if let Some(e) = &self.editor {
            for p in scene.items.iter().filter(|p| !in_hand.contains(&p.id)) {
                if let Some(key) = marker(&p.item) {
                    let lit = e.highlighted() == Some(p.id) || selected.contains(&p.id);
                    markers.push((key, p.affine(), if lit { MARKER_GREEN } else { MARKER_BLUE }));
                }
            }
            if let (Some(h), Some((position, rotation))) = (&e.held, e.held_pose()) {
                if let Some(key) = marker(&h.item) {
                    markers.push((key, super::scene::affine(position, rotation, Vec3::ONE), MARKER_GREEN));
                }
                for (m, (position, rotation)) in h.others.iter().zip(e.member_poses()) {
                    if let Some(key) = marker(&m.item) {
                        markers.push((key, super::scene::affine(position, rotation, m.scale), MARKER_GREEN));
                    }
                }
            }
        }
        held_others.retain(|(t, ..)| self.catalog.by_template(t).is_some());
        let (view, cursor, mode, meter, held) = match &self.editor {
            Some(e) => {
                let held = e.held.as_ref().filter(|h| template(&h.item).is_some()).and_then(|h| {
                    let (position, rotation) = e.held_pose()?;
                    let scale = h.source.and_then(|(id, ..)| scene.get(id)).map_or(Vec3::ONE, |p| p.scale);
                    let tint = if e.collision && e.blocked { BLOCKED } else { self.colour(h.tint) };
                    Some((h.template.clone(), template(&h.item).is_some_and(|t| t.1), super::scene::affine(position, rotation, scale), tint))
                });
                (Some(e.view()), e.cursor.position, e.mode, e.meter(), held)
            }
            None => (None, Vec3::ZERO, Mode::FreeCam, Meter::default(), None),
        };
        let hide_skater = self.editor.as_ref().is_some_and(|e| e.variant == super::Variant::Park);
        DropperFrame { view, cursor, mode, meter, held, held_others, hidden, items, hide_skater, markers }
    }
}

#[cfg(test)]
mod tests {
    use crate::object_dropper::{Event, Mode};

    /// Props placed on the ground rest asleep (D6d: 46 awake placed props took the Vita's sim tick from 11 to 51 ms):
    /// forty spread over DownTown's park, then a few seconds of play; the step time is printed against the bare park.
    /// Data-gated: `SKATE3_ASSET_ROOT` and `SKATE3_CATALOG_PAK`.
    #[test]
    #[ignore = "needs the private install data and a baked catalogue"]
    fn props_placed_on_the_ground_rest_asleep() {
        let assets = std::path::PathBuf::from(std::env::var_os("SKATE3_ASSET_ROOT").expect("SKATE3_ASSET_ROOT"));
        crate::physics::dropped::mount_catalog(std::path::Path::new(&std::env::var_os("SKATE3_CATALOG_PAK").expect("SKATE3_CATALOG_PAK")), &assets);
        let mut map = skate_data::skate_map::SkateMap::load(&assets.join("../maps/DownTownSkatePark.skate")).unwrap();
        map.extensions.push(crate::park_bake::bake(&assets, "Downtown_Fullpark").unwrap().0);
        let mut session = crate::live::Session::load_map(&assets, "Normal", Some(map)).unwrap();
        let neutral = skate_core::input::xbox::XboxState::default();
        let time = |session: &mut crate::live::Session, ticks: usize| {
            let t = std::time::Instant::now();
            for _ in 0..ticks {
                session.step(neutral);
            }
            t.elapsed().as_secs_f64() * 1000.0 / ticks as f64
        };
        time(&mut session, 60);
        let bare = time(&mut session, 120);
        session.open_editor().unwrap();
        let (at, _) = session.board();
        let catalog = session.dropper_catalog().unwrap().clone();
        let prop = catalog.items.iter().find(|i| i.name.contains("WoodenPalette")).unwrap().id.clone();
        let dropper = session.dropper.as_mut().unwrap();
        for k in 0..40 {
            let editor = dropper.editor.as_mut().unwrap();
            let (x, z) = (at[0] + (k % 8) as f32 * 3.0 - 12.0, at[2] + (k / 8) as f32 * 3.0 - 6.0);
            let ground = session.physics.ground_below(x, z, at[1] + 20.0).unwrap_or(at[1]);
            editor.cursor.position = glam::Vec3::new(x, ground, z);
            editor.cursor.lift = 0.0;
            let mut events = Vec::new();
            editor.choose(&prop, &mut events);
            editor.update(&[0.0; 24], &super::PhysicsGround(&session.physics));
            let placed = {
                let pose = editor.held_pose().unwrap();
                let held = editor.held.take().unwrap();
                editor.scene.drop_item(&held.item, pose.0, pose.1)
            };
            dropper.apply(&super::Event::Placed(placed), &mut session.physics, &assets);
        }
        session.close_editor();
        let awake_at_once = session.physics.prop_dynamics().map_or(0, |d| d.awake_runtime());
        let loaded = time(&mut session, 300);
        let awake = session.physics.prop_dynamics().map_or(0, |d| d.awake_runtime());
        eprintln!("bare park {bare:.2} ms/step; with 40 placed props {loaded:.2} ms/step; awake placed props {awake_at_once} then {awake}");
        // Most start asleep (a few stand on the props' own layer or a slope, not the static ground, and fall); all but a
        // few rest within five seconds. Without the rest: 40 awake, 4 after five seconds, 1.48 against 1.25 ms/step.
        assert!(awake_at_once <= 12, "props placed on the ground start asleep: {awake_at_once} awake");
        assert!(awake <= 4, "the rest settle: {awake} awake");
    }

    /// The baked catalogue's sockets: every park piece has some, and two real street pads snap flush side by side.
    /// Data-gated: `SKATE3_ASSET_ROOT` and `SKATE3_CATALOG_PAK`.
    #[test]
    #[ignore = "needs the private install data and a baked catalogue"]
    fn real_pads_snap_flush() {
        use crate::object_dropper::snap;
        use glam::{Quat, Vec3};
        let assets = std::path::PathBuf::from(std::env::var_os("SKATE3_ASSET_ROOT").expect("SKATE3_ASSET_ROOT"));
        crate::physics::dropped::mount_catalog(std::path::Path::new(&std::env::var_os("SKATE3_CATALOG_PAK").expect("SKATE3_CATALOG_PAK")), &assets);
        let json: serde_json::Value = serde_json::from_slice(&skate_data::vfs::read(&assets.join(super::CATALOG)).unwrap()).unwrap();
        let catalog = super::Catalog::from_json(&json).unwrap();
        let terrain: Vec<_> = catalog.items.iter().filter(|i| i.part == super::Part::Terrain && i.template.is_some()).collect();
        let without = terrain.iter().filter(|i| i.sockets.is_empty()).count();
        assert_eq!(without, 0, "every park piece has sockets");
        assert_eq!(catalog.items.iter().filter(|i| i.advance_snap()).count(), 180);
        // The Colour page's 64 swatches, white first.
        let tuning = super::Tuning::from_catalog(&json["tuning"]).unwrap();
        assert_eq!((tuning.colours.len(), tuning.colours[0]), (64, [1.0; 3]));
        let pad = catalog.items.iter().find(|i| i.name == "aa_padStreet_md_Industrial").unwrap();
        let placed = snap::sockets(pad, Vec3::ZERO, Quat::IDENTITY, Vec3::ONE);
        let base = (Vec3::new(6.5, 0.1, 0.3), Quat::from_rotation_y(-0.15));
        let held = snap::sockets(pad, base.0, base.1, Vec3::ONE);
        let pair = snap::best_pair(&held, &placed, false, false).expect("a pair");
        let (position, rotation) = snap::snapped_pose(&pair, base);
        assert!((position - Vec3::new(6.0, 0.0, 0.0)).length() < 1e-3, "{position}");
        assert!(rotation.angle_between(Quat::IDENTITY) < 1e-3, "{rotation}");
    }

    /// The Collision option's check over DownTown's and MegaPark's Fullparks as the developers built them: how many of its piece pairs
    /// it calls overlapping (few: the layout is the designers' own), and how long a piece's check takes. Data-gated:
    /// `SKATE3_ASSET_ROOT` and `SKATE3_CATALOG_PAK`.
    #[test]
    #[ignore = "needs the private install data and a baked catalogue"]
    fn the_stock_park_barely_overlaps() {
        use crate::object_dropper::overlap::{boxes_overlap, shapes_overlap, Posed};
        use glam::Vec3;
        let assets = std::path::PathBuf::from(std::env::var_os("SKATE3_ASSET_ROOT").expect("SKATE3_ASSET_ROOT"));
        crate::physics::dropped::mount_catalog(std::path::Path::new(&std::env::var_os("SKATE3_CATALOG_PAK").expect("SKATE3_CATALOG_PAK")), &assets);
        let json: serde_json::Value = serde_json::from_slice(&skate_data::vfs::read(&assets.join(super::CATALOG)).unwrap()).unwrap();
        let catalog = crate::object_dropper::Catalog::from_json(&json).unwrap();
        for (park, fullpark) in [("DownTownSkatePark", "Downtown_Fullpark"), ("MegaPark", "MegaPark_Fullpark"), ("IndustrialSkatePark", "Industrial_Fullpark"), ("StartPark", "Starter_Fullpark")] {
            let mut map = skate_data::skate_map::SkateMap::load(&assets.join(format!("../maps/{park}.skate"))).unwrap();
            map.extensions.push(crate::park_bake::bake(&assets, fullpark).unwrap().0);
            let layout = crate::park_bake::decode(&map).unwrap().unwrap().layout;
            let mut shapes = std::collections::BTreeMap::new();
            let mut pieces = Vec::new();
            for (_, item, placement) in &layout {
                let Some(entry) = catalog.item(item).filter(|i| !i.allows_collision()) else { continue };
                let Some(template) = entry.template.clone() else { continue };
                let tris: &Vec<[Vec3; 3]> = shapes.entry(template.clone()).or_insert_with(|| {
                    crate::physics::dropped::load_item(&assets, entry.part == crate::object_dropper::Part::Props, &template)
                        .map(|i| i.local.iter().map(|t| t.map(|v| Vec3::new(v.x, v.y, v.z))).collect())
                        .unwrap_or_default()
                });
                let (position, rotation, scale) = crate::object_dropper::scene::split(placement);
                let [lo, hi] = entry.cost.bounds;
                pieces.push((item.clone(), Posed { lo: Vec3::from(lo) * scale, hi: Vec3::from(hi) * scale, position, rotation }, tris.iter().map(|t| t.map(|v| v * scale)).collect::<Vec<_>>()));
            }
            let collision: usize = layout
                .iter()
                .filter_map(|(_, item, _)| catalog.item(item).and_then(|i| Some((i.template.clone()?, i.part))))
                .map(|(t, part)| crate::physics::dropped::load_item(&assets, part == crate::object_dropper::Part::Props, &t).map_or(0, |i| i.local.len()))
                .sum();
            println!("{park}: {} layout pieces, {collision} collision triangles", layout.len());
            let started = std::time::Instant::now();
            let (mut boxes, mut overlaps) = (0, Vec::new());
            for i in 0..pieces.len() {
                for j in i + 1..pieces.len() {
                    let ((ia, a, ta), (ib, b, tb)) = (&pieces[i], &pieces[j]);
                    if boxes_overlap(a, b) {
                        boxes += 1;
                        if shapes_overlap(a, ta, b, tb) {
                            let name = |id: &str| catalog.item(id).map_or(id.to_owned(), |i| i.name.clone());
                            overlaps.push((name(ia), name(ib)));
                        }
                    }
                }
            }
            let per_piece = started.elapsed().as_secs_f64() * 1000.0 / pieces.len() as f64;
            println!("{park}: {} pieces, {} box pairs, {} overlapping, {per_piece:.2} ms a piece; {:?}", pieces.len(), boxes, overlaps.len(), &overlaps[..overlaps.len().min(12)]);
            assert!(overlaps.len() * 10 < pieces.len(), "{park}: the designers' layout mostly clear");
        }
    }

    /// Memory per placed terrain piece (device 31 Oct: the heap grew ~1 MB a piece until a 56 MB allocation failed):
    /// Mega ramps placed again and again on one spot of DownTown's park (Collision off), the process's resident memory
    /// printed as they go: the budgets stop them.
    /// Data-gated: `SKATE3_ASSET_ROOT` and `SKATE3_CATALOG_PAK`.
    #[test]
    #[ignore = "needs the private install data and a baked catalogue"]
    fn the_budgets_stop_stacked_mega_ramps() {
        let assets = std::path::PathBuf::from(std::env::var_os("SKATE3_ASSET_ROOT").expect("SKATE3_ASSET_ROOT"));
        crate::physics::dropped::mount_catalog(std::path::Path::new(&std::env::var_os("SKATE3_CATALOG_PAK").expect("SKATE3_CATALOG_PAK")), &assets);
        let mut map = skate_data::skate_map::SkateMap::load(&assets.join("../maps/DownTownSkatePark.skate")).unwrap();
        map.extensions.push(crate::park_bake::bake(&assets, "Downtown_Fullpark").unwrap().0);
        let mut session = crate::live::Session::load_map(&assets, "Normal", Some(map)).unwrap();
        let neutral = skate_core::input::xbox::XboxState::default();
        for _ in 0..30 {
            session.step(neutral);
        }
        session.open_editor().unwrap();
        let rss = || std::fs::read_to_string("/proc/self/statm").ok().and_then(|s| s.split(' ').nth(1)?.parse::<u64>().ok()).unwrap_or(0) * 4096 / 1024 / 1024;
        // The owner's case: Mega ramps, Collision off, placed again and again on the same spot.
        let item = session.dropper_catalog().unwrap().items.iter().find(|i| i.name == "af_MegaLaunchRampB").unwrap().id.clone();
        let template = session.dropper_catalog().unwrap().item(&item).unwrap().template.clone().unwrap();
        let shape = crate::physics::dropped::load_item(&assets, false, &template).unwrap();
        let c = &session.dropper_catalog().unwrap().item(&item).unwrap().cost;
        println!("{template}: {} collision triangles, {} rails; render {} triangles, {} vertex bytes", shape.local.len(), shape.rails.len(), c.triangles, c.vertex_bytes);
        let all = session.dropper_catalog().unwrap();
        let mut ratio: Vec<(f32, String)> = all.items.iter().filter(|i| i.part == crate::object_dropper::Part::Terrain && i.cost.triangles > 0).filter_map(|i| {
            let t = i.template.as_ref()?;
            let n = crate::physics::dropped::load_item(&assets, false, t).ok()?.local.len();
            Some((n as f32 / i.cost.triangles as f32, i.name.clone()))
        }).collect();
        ratio.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mean = ratio.iter().map(|r| r.0).sum::<f32>() / ratio.len() as f32;
        println!("collision / render triangles over {} terrain items: min {:?} median {:?} max {:?} mean {mean:.2}", ratio.len(), ratio.first(), ratio.get(ratio.len() / 2), ratio.last());
        let before = rss();
        for k in 0..360 {
            session.editor_choose(&item);
            let dropper = session.dropper.as_mut().unwrap();
            let editor = dropper.editor.as_mut().unwrap();
            editor.collision = false;
            editor.cursor.position = glam::Vec3::new(60.0, 0.0, -60.0);
            let started = std::time::Instant::now();
            for buttons in [0x1000u16, 0x1000, 0, 0] {
                session.step(skate_core::input::xbox::XboxState { buttons, ..neutral });
            }
            if k % 40 == 39 {
                let placed = session.dropper_editor().unwrap().scene.items.iter().filter(|p| p.origin == crate::object_dropper::Origin::Dropped).count();
                println!("{} tries, {placed} placed: {} MiB resident (+{}), last place {:.1} ms", k + 1, rss(), rss() - before, started.elapsed().as_secs_f64() * 1000.0);
            }
        }
        // The budgets stop it long before the Vita's heap would: about 50k placed collision triangles.
        let editor = session.dropper_editor().unwrap();
        let placed = editor.scene.items.iter().filter(|p| p.origin == crate::object_dropper::Origin::Dropped).count();
        let used = editor.scene.usage(&editor.catalog);
        println!("budget: {placed} Mega ramps, memory {:.1} of {:.1} MiB, meter {:?}", used.memory as f64 / 1048576.0, editor.limits.memory as f64 / 1048576.0, editor.meter());
        assert!((45..70).contains(&placed), "{placed} placed");
    }

    /// New Park on DownTown: the Empty template (no file: the lot bare) leaves the spawner alone at the Fullpark's
    /// start, the Full template brings every piece back; neither counts as unsaved. Data-gated: `SKATE3_ASSET_ROOT` and
    /// `SKATE3_CATALOG_PAK`.
    #[test]
    #[ignore = "needs the private install data and the baked catalogue"]
    fn park_setup_templates_lay_the_bare_lot_and_the_fullpark() {
        let assets = std::path::PathBuf::from(std::env::var_os("SKATE3_ASSET_ROOT").expect("SKATE3_ASSET_ROOT"));
        crate::physics::dropped::mount_catalog(std::path::Path::new(&std::env::var_os("SKATE3_CATALOG_PAK").expect("SKATE3_CATALOG_PAK")), &assets);
        let mut map = skate_data::skate_map::SkateMap::load(&assets.join("../maps/DownTownSkatePark.skate")).unwrap();
        map.extensions.push(crate::park_bake::bake(&assets, "Downtown_Fullpark").unwrap().0);
        let mut session = crate::live::Session::load_map(&assets, "Normal", Some(map)).unwrap();
        for _ in 0..10 {
            session.step(skate_core::input::xbox::XboxState::default());
        }
        let (kept, dropped, refused) = session.load_template("dist_downtownpark_empty").unwrap();
        let lots: Vec<_> = session.park_lots().iter().map(|l| (l.lot.clone(), l.templates.iter().map(|t| t.world.clone()).collect::<Vec<_>>())).collect();
        println!("lots {lots:?}");
        assert_eq!(lots.iter().map(|l| l.0.as_str()).collect::<Vec<_>>(), ["dist_startpark", "dist_industrialpark", "dist_downtownpark", "dist_megapark"], "retail's order (825D1AD0)");
        println!("Empty: kept {kept}, dropped {dropped}, refused {refused}");
        assert_eq!((kept, dropped, refused), (0, 0, 0));
        assert_eq!(session.dropper.as_ref().unwrap().scene().items.len(), 1, "the spawner alone");
        assert!(!session.park_unsaved());
        let (kept, dropped, refused) = session.load_template("dist_downtownpark_full").unwrap();
        println!("Full: kept {kept}, dropped {dropped}, refused {refused}");
        assert!(kept > 200 && dropped == 0 && refused == 0);
        assert!(!session.park_unsaved());
    }

    /// Layouts laid over DownTown's park: its own Fullpark changes nothing, its Empty layout takes every piece away, and
    /// an edited park saved as a layout file comes back the same over the Fullpark. Data-gated: `SKATE3_ASSET_ROOT` and
    /// `SKATE3_CATALOG_PAK`.
    #[test]
    #[ignore = "needs the private install data and a baked catalogue"]
    fn park_files_lay_over_the_baked_park_and_come_back() {
        use crate::park_file::ParkFile;
        let assets = std::path::PathBuf::from(std::env::var_os("SKATE3_ASSET_ROOT").expect("SKATE3_ASSET_ROOT"));
        crate::physics::dropped::mount_catalog(std::path::Path::new(&std::env::var_os("SKATE3_CATALOG_PAK").expect("SKATE3_CATALOG_PAK")), &assets);
        let stock = |name: &str| ParkFile::parse(&std::fs::read(assets.join(format!("private/stock/data/skatepark/{name}.park"))).unwrap()).unwrap();
        let (full, empty) = (stock("Downtown_Fullpark"), stock("Downtown_Empty"));
        let mut map = skate_data::skate_map::SkateMap::load(&assets.join("../maps/DownTownSkatePark.skate")).unwrap();
        map.extensions.push(crate::park_bake::bake(&assets, "Downtown_Fullpark").unwrap().0);
        let mut session = crate::live::Session::load_map(&assets, "Normal", Some(map)).unwrap();
        for _ in 0..10 {
            session.step(skate_core::input::xbox::XboxState::default());
        }
        session.open_editor().unwrap();
        session.close_editor();
        let poses = |session: &crate::live::Session| -> Vec<(u32, String, [i32; 3])> {
            let mut v: Vec<_> = session.dropper.as_ref().unwrap().scene().items.iter().map(|p| (p.id, p.item.clone(), (p.position * 100.0).round().as_ivec3().to_array())).collect();
            v.sort();
            v
        };
        let before = poses(&session);
        let asset_root = assets.clone();
        let (dropper, physics) = (session.dropper.as_mut().unwrap(), &mut session.physics);
        // The Fullpark over itself: all kept, nothing dropped.
        let (kept, dropped, refused) = dropper.apply_park(&full, &full.props, physics, &asset_root);
        println!("Fullpark: kept {kept}, dropped {dropped}, refused {refused}");
        assert_eq!((dropped, refused), (0, 0));
        assert_eq!(poses(&session), before);
        // Empty: only the spawner stays, at Empty's start.
        let (dropper, physics) = (session.dropper.as_mut().unwrap(), &mut session.physics);
        let (kept, ..) = dropper.apply_park(&empty, &full.props, physics, &asset_root);
        assert_eq!(kept, 0);
        let scene = session.dropper.as_ref().unwrap().scene();
        assert_eq!(scene.items.len(), 1, "the spawner alone");
        let start = empty.start.unwrap()[3];
        assert!(session.dropper.as_ref().unwrap().spawner_pose().unwrap()[3][..3].iter().zip(&start[..3]).all(|(a, b)| (a - b).abs() < 1e-3));
        // An edit: back to the Fullpark, one piece moved and a ramp dropped; saved, cleared, loaded: the same.
        let (dropper, physics) = (session.dropper.as_mut().unwrap(), &mut session.physics);
        dropper.apply_park(&full, &full.props, physics, &asset_root);
        let first = dropper.scene().items.iter().find(|p| p.id != crate::object_dropper::scene::SPAWNER_ID).unwrap().clone();
        let ramp = dropper.catalog.items.iter().find(|i| i.name == "ac_ramp_md_Downtown").unwrap().id.clone();
        dropper.scene_mut().set_pose(first.id, first.position + glam::Vec3::new(5.0, 0.0, 0.0), first.rotation);
        dropper.scene_mut().drop_item(&ramp, glam::Vec3::new(10.0, 0.0, -40.0), glam::Quat::IDENTITY);
        let edited = poses(&session);
        let saved_bytes = session.dropper.as_ref().unwrap().to_park(full.origin, &full).to_bytes();
        let saved = ParkFile::parse(&saved_bytes).unwrap();
        let (dropper, physics) = (session.dropper.as_mut().unwrap(), &mut session.physics);
        dropper.apply_park(&empty, &full.props, physics, &asset_root);
        let (kept, dropped, refused) = dropper.apply_park(&saved, &full.props, physics, &asset_root);
        println!("saved: kept {kept}, dropped {dropped}, refused {refused}");
        assert_eq!((dropped, refused), (1, 0));
        let strip = |v: Vec<(u32, String, [i32; 3])>| -> Vec<(String, [i32; 3])> {
            let mut v: Vec<_> = v.into_iter().map(|(_, i, p)| (i, p)).collect();
            v.sort();
            v
        };
        assert_eq!(strip(poses(&session)), strip(edited.clone()));
        // My Parks' load through the session: the same again.
        let (dropper, physics) = (session.dropper.as_mut().unwrap(), &mut session.physics);
        dropper.apply_park(&empty, &full.props, physics, &asset_root);
        assert_eq!(session.load_park(&saved_bytes).unwrap().1, 1);
        assert_eq!(strip(poses(&session)), strip(edited));
        // Save Park's bytes: the Fullpark's origin and tail, every piece, the spawner as the start.
        let (dropper, physics) = (session.dropper.as_mut().unwrap(), &mut session.physics);
        dropper.apply_park(&full, &full.props, physics, &asset_root);
        let file = ParkFile::parse(&session.park_file().expect("a park")).unwrap();
        assert_eq!((file.origin, file.pieces.len(), file.props.len()), (full.origin, full.pieces.len(), 0));
        assert_eq!(file.tail, full.tail);
        assert!((0..3).all(|k| (file.start.unwrap()[3][k] - full.start.unwrap()[3][k]).abs() < 1e-3));
    }

    /// Another stock layout over a park: StartPark's progression layout over its Fullpark (77 pieces shared by kind, the
    /// rest other variants, dropped), and how long laying it takes. Data-gated: `SKATE3_ASSET_ROOT` and `SKATE3_CATALOG_PAK`.
    #[test]
    #[ignore = "needs the private install data and a baked catalogue"]
    fn a_stock_layout_lays_over_another_parks_fullpark() {
        use crate::park_file::ParkFile;
        let assets = std::path::PathBuf::from(std::env::var_os("SKATE3_ASSET_ROOT").expect("SKATE3_ASSET_ROOT"));
        crate::physics::dropped::mount_catalog(std::path::Path::new(&std::env::var_os("SKATE3_CATALOG_PAK").expect("SKATE3_CATALOG_PAK")), &assets);
        let stock = |name: &str| ParkFile::parse(&std::fs::read(assets.join(format!("private/stock/data/skatepark/{name}.park"))).unwrap()).unwrap();
        let (full, progression) = (stock("Starter_Fullpark"), stock("startpark_progression"));
        let mut map = skate_data::skate_map::SkateMap::load(&assets.join("../maps/StartPark.skate")).unwrap();
        map.extensions.push(crate::park_bake::bake(&assets, "Starter_Fullpark").unwrap().0);
        let mut session = crate::live::Session::load_map(&assets, "Normal", Some(map)).unwrap();
        session.open_editor().unwrap();
        let started = std::time::Instant::now();
        let (dropper, physics) = (session.dropper.as_mut().unwrap(), &mut session.physics);
        let (kept, dropped, refused) = dropper.apply_park(&progression, &full.props, physics, &assets);
        println!("progression over the Fullpark: kept {kept}, dropped {dropped}, refused {refused} of {} pieces, {:.0} ms", progression.pieces.len(), started.elapsed().as_secs_f64() * 1000.0);
        assert_eq!(kept + dropped + refused, progression.pieces.len() + progression.props.iter().filter(|p| !full.props.iter().any(|f| f.item == p.item && f.position.distance(p.position) < 0.01)).count());
        // Every piece the Fullpark has of the progression's kinds is kept (they share 77 by kind).
        let shared: usize = {
            let mut count = std::collections::BTreeMap::<u64, (usize, usize)>::new();
            full.pieces.iter().for_each(|p| count.entry(p.item).or_default().0 += 1);
            progression.pieces.iter().for_each(|p| count.entry(p.item).or_default().1 += 1);
            count.values().map(|(a, b)| a.min(b)).sum()
        };
        assert_eq!(kept, shared);
    }

    /// The Challenge Map's PCU Training - Basic over the Fullpark a StartPark pack holds, by its world row.
    /// Data-gated: `SKATE3_ASSET_ROOT` and `SKATE3_CATALOG_PAK`.
    #[test]
    #[ignore = "needs the private install data and a baked catalogue"]
    fn the_challenge_maps_basic_row_lays_its_layout() {
        let assets = std::path::PathBuf::from(std::env::var_os("SKATE3_ASSET_ROOT").expect("SKATE3_ASSET_ROOT"));
        crate::physics::dropped::mount_catalog(std::path::Path::new(&std::env::var_os("SKATE3_CATALOG_PAK").expect("SKATE3_CATALOG_PAK")), &assets);
        let mut map = skate_data::skate_map::SkateMap::load(&assets.join("../maps/StartPark.skate")).unwrap();
        map.extensions.push(crate::park_bake::bake(&assets, "Starter_Fullpark").unwrap().0);
        let mut session = crate::live::Session::load_map(&assets, "Normal", Some(map)).unwrap();
        let (kept, dropped, refused) = session.load_template("dist_startpark_progression").unwrap();
        println!("Basic: kept {kept}, dropped {dropped}, refused {refused}");
        assert!(kept > 0 && refused == 0);
        let basic = session.dropper.as_ref().unwrap().scene().items.len();
        let (kept, _, refused) = session.load_template("dist_startpark_full").unwrap();
        println!("back to Full: kept {kept}, refused {refused}");
        assert!(refused == 0);
        assert_ne!(session.dropper.as_ref().unwrap().scene().items.len(), basic, "the two rows are different parks");
    }

    /// The park editor's camera stops at the map and ground, and passes the layout's pieces and what is placed: a line of
    /// sight straight down over a Fullpark's pieces, and through a Mega ramp placed on open ground.
    /// Data-gated: `SKATE3_ASSET_ROOT` and `SKATE3_CATALOG_PAK`.
    #[test]
    #[ignore = "needs the private install data and a baked catalogue"]
    fn the_camera_stops_at_the_map_and_passes_what_is_placed() {
        let assets = std::path::PathBuf::from(std::env::var_os("SKATE3_ASSET_ROOT").expect("SKATE3_ASSET_ROOT"));
        crate::physics::dropped::mount_catalog(std::path::Path::new(&std::env::var_os("SKATE3_CATALOG_PAK").expect("SKATE3_CATALOG_PAK")), &assets);
        let mut map = skate_data::skate_map::SkateMap::load(&assets.join("../maps/DownTownSkatePark.skate")).unwrap();
        map.extensions.push(crate::park_bake::bake(&assets, "Downtown_Fullpark").unwrap().0);
        let session = crate::live::Session::load_map(&assets, "Normal", Some(map)).unwrap();
        let (mut stood_on, mut through) = (0, 0);
        for (_, _, placement) in session.physics.park_layout() {
            let (x, z) = (placement[9], placement[11]);
            let top = placement[10] + 40.0;
            // The whole world: the highest thing under the point, a piece where one stands.
            let Some(highest) = session.physics.ground_below(x, z, top) else { continue };
            // The map only: the ground, however high the piece.
            let Some(fraction) = session.physics.map_sweep([x, top, z], [x, top - 60.0, z], 0.0).map(|h| h.0) else { continue };
            let map_ground = top - 60.0 * fraction;
            assert!(map_ground <= highest + 0.01, "the map's ground is not above the whole world's at ({x}, {z})");
            if highest > map_ground + 0.5 {
                through += 1;
            } else {
                stood_on += 1;
            }
        }
        println!("pieces passed {through}, none over the ground {stood_on}");
        assert!(through > 20, "the line of sight passes the layout's pieces ({through})");
        // A surface met from behind does not stop it: from under the ground, going up.
        let spawn = session.physics.park_layout()[0].2;
        let (x, z) = (spawn[9], spawn[11]);
        let top = spawn[10] + 40.0;
        let ground = top - 60.0 * session.physics.map_sweep([x, top, z], [x, top - 60.0, z], 0.0).map(|h| h.0).unwrap();
        assert_eq!(session.physics.map_sweep([x, ground - 1.0, z], [x, ground + 0.5, z], 0.0).map(|h| h.0), None);
    }

    /// Opened on top of something with lower ground ahead, driven off it, D-pad Down held: on all four parks' real maps the
    /// cursor reaches the ground and the camera comes down to it, all but 1% at most (owner, device: it stayed up). A 40 x 40 grid of
    /// starts.
    /// Data-gated: `SKATE3_ASSET_ROOT` and `SKATE3_CATALOG_PAK`.
    #[test]
    #[ignore = "needs the private install data and a baked catalogue"]
    fn the_camera_comes_down_off_what_it_opened_on() {
        let assets = std::path::PathBuf::from(std::env::var_os("SKATE3_ASSET_ROOT").expect("SKATE3_ASSET_ROOT"));
        crate::physics::dropped::mount_catalog(std::path::Path::new(&std::env::var_os("SKATE3_CATALOG_PAK").expect("SKATE3_CATALOG_PAK")), &assets);
        for (name, layout) in [("DownTownSkatePark", "Downtown_Fullpark"), ("StartPark", "Starter_Fullpark"), ("MegaPark", "MegaPark_Fullpark"), ("IndustrialSkatePark", "Industrial_Fullpark")] {
            let mut map = skate_data::skate_map::SkateMap::load(&assets.join(format!("../maps/{name}.skate"))).unwrap();
            map.extensions.push(crate::park_bake::bake(&assets, layout).unwrap().0);
            let mut session = crate::live::Session::load_map(&assets, "Normal", Some(map)).unwrap();
            session.open_editor().unwrap();
            session.close_editor();
            let layout: Vec<[f32; 3]> = session.physics.park_layout().iter().map(|(_, _, p)| [p[9], p[10], p[11]]).collect();
            let (lo, hi) = layout.iter().fold(([f32::MAX; 2], [f32::MIN; 2]), |(l, h), p| ([l[0].min(p[0]), l[1].min(p[2])], [h[0].max(p[0]), h[1].max(p[2])]));
            let (mut runs, mut cursor_up, mut eye_up, mut walled, mut underground) = (0, 0, 0, 0, 0);
            for ix in 0..40 {
                for iz in 0..40 {
                    let (x, z) = (lo[0] + (hi[0] - lo[0]) * ix as f32 / 39.0, lo[1] + (hi[1] - lo[1]) * iz as f32 / 39.0);
                    let Some(top) = session.physics.ground_below(x, z, 60.0) else { continue };
                    let yaw = (ix * 7 + iz * 3) as f32 * 0.9;
                    let forward = glam::Vec3::new(-yaw.sin(), 0.0, -yaw.cos());
                    let ahead = (1..8).filter_map(|k| { let p = glam::Vec3::new(x, 0.0, z) + forward * (k as f32 * 3.0); session.physics.ground_below(p.x, p.z, top + 7.0) }).fold(f32::MAX, f32::min);
                    if ahead == f32::MAX || top - ahead < 2.0 {
                        continue;
                    }
                    let (dropper, physics) = (session.dropper.as_mut().unwrap(), &mut session.physics);
                    dropper.open(glam::Vec3::new(x, top, z), yaw, crate::object_dropper::Variant::Park, Default::default(), false);
                    let mut slots = [0.0; 24];
                    slots[18] = 1.0;
                    for _ in 0..80 {
                        dropper.update(&slots, physics, &assets);
                    }
                    let mut down = [0.0; 24];
                    down[1] = 1.0;
                    for _ in 0..400 {
                        dropper.update(&down, physics, &assets);
                    }
                    for _ in 0..240 {
                        dropper.update(&[0.0; 24], physics, &assets);
                    }
                    let editor = dropper.editor().unwrap();
                    let (c, eye, want) = (editor.cursor.position, editor.view().0, editor.eye());
                    runs += 1;
                    cursor_up += usize::from(physics.ground_below(c.x, c.z, c.y + 0.5).is_some_and(|g| c.y - g > 0.5));
                    // A wall may hold it back along the ground, a little above the spot; only well above the spot is up there.
                    // Inside the ground: a surface met coming down to it from above, and none under it.
                    let probe = |from: glam::Vec3, to: glam::Vec3| physics.map_sweep(from.to_array(), to.to_array(), 0.0);
                    let under_something = probe(eye + glam::Vec3::Y * 10.0, eye).is_some();
                    underground += usize::from(under_something && probe(eye, eye - glam::Vec3::Y * 100.0).is_none());
                    let high = eye.y - want.y;
                    eye_up += usize::from(high > 5.0);
                    walled += usize::from(high > 0.5 && high <= 5.0);
                    dropper.close(physics, &assets);
                }
            }
            println!("{name}: {runs} runs, cursor left up {cursor_up}, camera left up {eye_up}, held by a wall {walled}, underground {underground}");
            assert_eq!(underground, 0, "{name}: the camera ended inside the ground");
            assert!(runs > 50, "{name}: {runs} runs");
            assert_eq!(cursor_up, 0, "{name}");
            // Left: a few on Industrial's rooftops, a wall beside the camera and a roof under it (4 of 559): through either
            // it could end inside a building.
            assert!(eye_up * 100 <= runs, "{name}: camera left up in {eye_up} of {runs}");
        }
    }

    /// Closing the park editor puts the skater at the spawner: DownTown's layout start where it stands by default.
    /// Data-gated: `SKATE3_ASSET_ROOT` and `SKATE3_CATALOG_PAK`.
    #[test]
    #[ignore = "needs the private install data and a baked catalogue"]
    fn closing_the_park_editor_puts_the_skater_at_the_spawner() {
        let assets = std::path::PathBuf::from(std::env::var_os("SKATE3_ASSET_ROOT").expect("SKATE3_ASSET_ROOT"));
        crate::physics::dropped::mount_catalog(std::path::Path::new(&std::env::var_os("SKATE3_CATALOG_PAK").expect("SKATE3_CATALOG_PAK")), &assets);
        let mut map = skate_data::skate_map::SkateMap::load(&assets.join("../maps/DownTownSkatePark.skate")).unwrap();
        map.extensions.push(crate::park_bake::bake(&assets, "Downtown_Fullpark").unwrap().0);
        let mut session = crate::live::Session::load_map(&assets, "Normal", Some(map)).unwrap();
        let neutral = skate_core::input::xbox::XboxState::default();
        for _ in 0..30 {
            session.step(neutral);
        }
        session.open_editor().unwrap();
        let pose = session.dropper.as_ref().unwrap().spawner_pose().expect("the park's spawner");
        let start = &serde_json::from_slice::<serde_json::Value>(&skate_data::vfs::read(&assets.join(super::CATALOG)).unwrap()).unwrap()["park_starts"]["DownTownSkatePark"][3];
        let start: Vec<f32> = start.as_array().unwrap().iter().map(|v| v.as_f64().unwrap() as f32).collect();
        assert!((0..3).all(|k| (pose[3][k] - start[k]).abs() < 1e-3), "{pose:?} vs {start:?}");
        // B closes it (after the editor settled): a few ticks later the skater stands there.
        for buttons in [0u16; 20].into_iter().chain([0x2000; 4]).chain([0; 10]) {
            session.step(skate_core::input::xbox::XboxState { buttons, ..neutral });
        }
        assert!(!session.editor_open());
        let (at, _) = session.board();
        println!("skater at {at:?}, spawner at {:?}", pose[3]);
        assert!(glam::Vec2::new(at[0] - pose[3][0], at[2] - pose[3][2]).length() < 1.0);
    }

    /// The editor in a session on DownTown's park as the Vita loads it: a catalogue ramp placed, one of the park's
    /// pieces moved, the editor closed with the edits kept. Data-gated: `SKATE3_ASSET_ROOT` and `SKATE3_CATALOG_PAK`.
    #[test]
    #[ignore = "needs the private install data and a baked catalogue"]
    fn the_editor_edits_a_park_in_a_session() {
        let assets = std::path::PathBuf::from(std::env::var_os("SKATE3_ASSET_ROOT").expect("SKATE3_ASSET_ROOT"));
        crate::physics::dropped::mount_catalog(std::path::Path::new(&std::env::var_os("SKATE3_CATALOG_PAK").expect("SKATE3_CATALOG_PAK")), &assets);
        let mut map = skate_data::skate_map::SkateMap::load(&assets.join("../maps/DownTownSkatePark.skate")).unwrap();
        map.extensions.push(crate::park_bake::bake(&assets, "Downtown_Fullpark").unwrap().0);
        let mut session = crate::live::Session::load_map(&assets, "Normal", Some(map)).unwrap();
        let neutral = skate_core::input::xbox::XboxState::default();
        for _ in 0..30 {
            session.step(neutral);
        }
        session.open_editor().unwrap();
        assert!(session.editor_open());
        let pad = |buttons: u16| skate_core::input::xbox::XboxState { buttons, ..neutral };
        let press = |session: &mut crate::live::Session, buttons: u16| {
            let mut events = Vec::new();
            for _ in 0..4 {
                events.extend(session.step(pad(buttons)).editor);
            }
            for _ in 0..4 {
                events.extend(session.step(neutral).editor);
            }
            events
        };
        // X opens the catalogue; choose the medium quarter pipe ramp prop; A places it.
        press(&mut session, 0x4000);
        let ramp = session.dropper_catalog().unwrap().items.iter().find(|i| i.name == "bo_DMO_Glbl_mediumQuarterPipe").unwrap().id.clone();
        assert_eq!(session.editor_choose(&ramp), vec![Event::Taken(None)]);
        press(&mut session, 0x1000);
        let frame = session.dropper_frame().unwrap();
        assert_eq!(frame.mode, Mode::FreeCam);
        let dropped = frame.items.iter().find(|i| i.2).expect("the dropped prop is drawn").0;
        assert!(session.physics.dropped_body(dropped).is_some(), "a body in the world");
        // Take the park piece under a piece's own position, push the stick for a while, place it.
        let piece = session.physics.park_layout()[0].clone();
        session.dropper.as_mut().unwrap().editor.as_mut().unwrap().cursor.position = glam::Vec3::new(piece.2[9], piece.2[10], piece.2[11]);
        let events = press(&mut session, 0x1000);
        let taken = events.iter().rev().find_map(|e| match e { Event::Taken(Some(id)) => Some(*id), _ => None }).expect("a piece taken");
        assert!(session.dropper_frame().unwrap().hidden.contains(&taken), "out of the layout draws while in hand");
        let push = skate_core::input::xbox::XboxState { left: [32767, 0], ..neutral };
        for _ in 0..30 {
            session.step(push);
        }
        press(&mut session, 0x1000);
        let frame = session.dropper_frame().unwrap();
        assert!(frame.hidden.contains(&taken) && frame.items.iter().any(|i| i.0 == taken), "drawn from the catalogue where it was put");
        // B leaves the editor; the edits stay.
        press(&mut session, 0x2000);
        assert!(!session.editor_open());
        let frame = session.dropper_frame().unwrap();
        assert!(frame.view.is_none() && frame.items.len() >= 2);
        // Saved, and replayed into a fresh load of the park: the same world (the dropped ids are new).
        let saved = session.take_dropper_save().expect("the edits to save");
        assert!(session.take_dropper_save().is_none(), "nothing new since");
        let mut map = skate_data::skate_map::SkateMap::load(&assets.join("../maps/DownTownSkatePark.skate")).unwrap();
        map.extensions.push(crate::park_bake::bake(&assets, "Downtown_Fullpark").unwrap().0);
        let mut reloaded = crate::live::Session::load_map(&assets, "Normal", Some(map)).unwrap();
        reloaded.restore_dropper(&saved).unwrap();
        let again = reloaded.dropper_frame().unwrap();
        assert_eq!(again.hidden, frame.hidden);
        let shape = |f: &super::DropperFrame| { let mut v: Vec<_> = f.items.iter().map(|i| (i.1.clone(), i.2)).collect(); v.sort(); v };
        assert_eq!(shape(&again), shape(&frame));
        assert!(reloaded.physics.dropped_body(0x6000_0000).is_some(), "the prop has a body again");
        // Reset all: the layout again, no drops, and a save of nothing.
        reloaded.reset_dropper();
        let reset = reloaded.dropper_frame().unwrap();
        assert!(reset.hidden.is_empty() && reset.items.is_empty());
        let cleared = crate::object_dropper::save::Saved::from_bytes(&reloaded.take_dropper_save().unwrap()).unwrap();
        assert!(cleared.is_empty());
    }
}
