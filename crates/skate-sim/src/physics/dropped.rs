//! skate_vita (Object Dropper, docs/object-dropper-survey.md step D4): catalogue items dropped into the running world.
//! A terrain piece becomes a fixed instance of the prop collision layer (no body) with its rails among the fixed
//! park-piece rails; a prop becomes a runtime prop body (`spawn_runtime_prop`) with its type data and rails, pushed,
//! knocked over and grabbed like a map prop. Their data is the catalogue pack's per-item files
//! (`vita-bake catalog`): the binary collision shape, the geometry package when there is no shape, and the sidecar
//! slice (rails, type).
use std::collections::BTreeMap;

use skate_core::math::{Basis3, Vector3};

use super::park_edit::Affine;
use super::prop_dynamics::{DmoType, RuntimeProp, HELD_PARK};
use super::GamePhysics;
use crate::grind_world::props::{PosedRails, PropRail, PropRailPoses, PIECE_OWNER_TAG};

/// One catalogue item as the simulation needs it.
#[derive(Clone, Debug)]
pub(crate) struct CatalogItem {
    pub template: String,
    pub props: bool,
    /// Template-space collision triangles.
    pub local: Vec<[Vector3; 3]>,
    /// The shape's commonest surface (the prop layer takes one per instance).
    pub surface: u32,
    pub rails: Vec<PropRail>,
    pub type_data: Option<DmoType>,
}

fn dir(asset_root: &std::path::Path, props: bool) -> std::path::PathBuf {
    asset_root.join("private/object-catalog").join(if props { "props" } else { "terrain" }).join("items")
}

/// Reads one item's files (`private/object-catalog/<part>/items/<template>.*`, through `skate_data::vfs`).
pub(crate) fn load_item(asset_root: &std::path::Path, props: bool, template: &str) -> Result<CatalogItem, String> {
    let dir = dir(asset_root, props);
    let read = |ext: &str| skate_data::vfs::read(&dir.join(format!("{template}.{ext}")));
    let (local, surface) = match read("shape").ok().map(|b| crate::catalog_shapes::decode(&b)).transpose()?.and_then(|s| s.into_values().next()) {
        Some(shape) => {
            let mut counts: BTreeMap<u32, usize> = BTreeMap::new();
            for &s in &shape.surfaces {
                *counts.entry(s).or_default() += 1;
            }
            let surface = counts.into_iter().max_by_key(|&(s, n)| (n, std::cmp::Reverse(s))).map_or(0, |(s, _)| s);
            (shape.triangles.iter().map(|t| t.map(|p| Vector3::new(p[0], p[1], p[2]))).collect(), surface)
        }
        None => {
            let package = skate_data::prop_package::decode(&read("props").map_err(|e| format!("{template}.props: {e}"))?)?;
            let v = &package.geometry.vertices;
            let local = package.geometry.indices.chunks_exact(3).map(|t| [t[0], t[1], t[2]].map(|i| {
                let p = v[i as usize].position;
                Vector3::new(p[0], p[1], p[2])
            })).collect();
            (local, 0)
        }
    };
    let json_path = dir.join(format!("{template}.json"));
    let sidecar: serde_json::Value = serde_json::from_slice(&skate_data::vfs::read(&json_path).map_err(|e| format!("{template}.json: {e}"))?)
        .map_err(|e| format!("{template}.json: {e}"))?;
    let rails = match sidecar["grind_splines"].get(template) {
        Some(list) => crate::grind_world::props::parse_rails(list)?,
        None => Vec::new(),
    };
    let type_data = if props { crate::skate_world::load_types(asset_root, &json_path, template).and_then(|mut t| t.remove(template)) } else { None };
    Ok(CatalogItem { template: template.to_owned(), props, local, surface, rails, type_data })
}

/// A dropped item in the world.
#[derive(Clone, Debug)]
pub(crate) enum Dropped {
    /// A fixed instance of the prop layer, and its rails at their pose.
    Terrain { instance: usize, rails: Option<PosedRails> },
    /// A runtime prop body.
    Prop { body: u32 },
}

/// How far the static ground may be above or below a placed prop's base for it to rest there (metres).
const RESTING: f32 = 0.1;

fn columns(t: &Affine) -> [[f32; 3]; 3] {
    [[t[0], t[1], t[2]], [t[3], t[4], t[5]], [t[6], t[7], t[8]]]
}

fn origin(t: &Affine) -> Vector3 {
    Vector3::new(t[9], t[10], t[11])
}

impl GamePhysics {
    /// The cars' rails and the dropped terrain's, as the rail queries read them (`car_rails`).
    fn refresh_dropped_rails(&mut self) {
        let mut all = self.cars_only_rails.0.clone();
        all.extend(self.dropped.values().filter_map(|d| match d {
            Dropped::Terrain { rails: Some(r), .. } => Some(r.clone()),
            _ => None,
        }));
        self.car_rails = PropRailPoses(all);
    }

    /// Drops `item` at `pose` as scene item `id` (the editor's id). Returns false when the world refused it.
    pub(crate) fn drop_item(&mut self, id: u32, item: &CatalogItem, pose: &Affine) -> bool {
        if self.dropped.contains_key(&id) {
            return false;
        }
        if item.props {
            let spec = RuntimeProp {
                template: item.template.clone(),
                local: item.local.clone(),
                physics: skate_data::skate_map::ObjectPhysics::default(),
                type_data: item.type_data.clone(),
                origin: origin(pose),
                basis: Basis3 { columns: columns(pose) },
                linear_velocity: Vector3::ZERO,
                angular_velocity: Vector3::ZERO,
            };
            let Some(body) = self.spawn_runtime_prop(&spec, item.surface) else { return false };
            // Placed on the ground it rests asleep, as the map's own props load (`initially_awake` off), until something
            // touches it; placed in the air it stays awake and falls. NOT RETAIL YET (D6d device measurement: 46
            // placed props kept awake took the sim tick from 11 to 51 ms; retail's placed DMO wake state is not read).
            let base = origin(pose);
            let (start, end) = ([base.x, base.y + RESTING, base.z, 0.0], [base.x, base.y - RESTING, base.z, 0.0]);
            let on_ground = crate::camera::world_query::line(&self.world, start, end, 0.0).is_ok_and(|h| h.hit != 0);
            if let Some(dynamics) = self.prop_dynamics.as_mut().map(super::shared_mut) {
                if !item.rails.is_empty() {
                    dynamics.add_grind_rails(&item.template, item.rails.clone());
                }
                if on_ground {
                    if let Some(instance) = dynamics.teleport(body, base, Basis3 { columns: columns(pose) }) {
                        if let Some(layer) = self.prop_layer.as_mut() {
                            if let Err(error) = layer.rebake(instance, columns(pose), base) {
                                tracing::warn!("SKATE_DROP: rest {id}: {error}");
                            }
                        }
                    }
                }
            }
            self.dropped.insert(id, Dropped::Prop { body });
            return true;
        }
        if self.prop_layer.is_none() {
            match crate::skate_world::PropCollisionLayer::empty(self.settings.floor_material) {
                Ok(layer) => self.prop_layer = Some(layer),
                Err(error) => {
                    tracing::warn!("SKATE_DROP: empty layer: {error}");
                    return false;
                }
            }
        }
        let Some(layer) = self.prop_layer.as_mut() else { return false };
        let instance = match layer.add_instance(id, item.local.clone(), item.surface, columns(pose), origin(pose)) {
            Ok(instance) => instance,
            Err(error) => {
                tracing::warn!("SKATE_DROP: {}: {error}", item.template);
                return false;
            }
        };
        let rails = (!item.rails.is_empty()).then(|| PosedRails {
            id,
            tag: PIECE_OWNER_TAG,
            origin: [pose[9], pose[10], pose[11]],
            basis: columns(pose),
            scale: [1.0; 3],
            rails: item.rails.clone().into(),
        });
        self.dropped.insert(id, Dropped::Terrain { instance, rails });
        self.refresh_dropped_rails();
        tracing::info!("SKATE_DROP id={id} template={} instance={instance}", item.template);
        true
    }

    /// Moves dropped item `id` to `pose` (a prop is set down there at rest).
    pub(crate) fn move_dropped(&mut self, id: u32, pose: &Affine) -> bool {
        match self.dropped.get_mut(&id) {
            Some(Dropped::Terrain { instance, rails }) => {
                let instance = *instance;
                if let Some(r) = rails {
                    r.origin = [pose[9], pose[10], pose[11]];
                    r.basis = columns(pose);
                }
                let moved = self.prop_layer.as_mut().is_some_and(|l| l.rebake(instance, columns(pose), origin(pose)).is_ok());
                self.refresh_dropped_rails();
                moved
            }
            Some(Dropped::Prop { body }) => {
                let body = *body;
                let Some(dynamics) = self.prop_dynamics.as_mut().map(super::shared_mut) else { return false };
                let Some(instance) = dynamics.teleport(body, origin(pose), Basis3 { columns: columns(pose) }) else { return false };
                self.prop_layer.as_mut().is_some_and(|l| l.rebake(instance, columns(pose), origin(pose)).is_ok())
            }
            None => false,
        }
    }

    /// Takes dropped item `id` out of the world.
    pub(crate) fn remove_dropped(&mut self, id: u32) -> bool {
        match self.dropped.remove(&id) {
            Some(Dropped::Terrain { instance, .. }) => {
                if let Some(layer) = self.prop_layer.as_mut() {
                    if let Err(error) = layer.retire_instance(instance, HELD_PARK) {
                        tracing::warn!("SKATE_DROP: retire {id}: {error}");
                    }
                }
                self.refresh_dropped_rails();
                true
            }
            Some(Dropped::Prop { body }) => self.remove_runtime_prop(body),
            None => false,
        }
    }

    /// The ground at (x, z) at or below `from` (the static world and the prop layer: the editor's cursor stands on
    /// dropped pieces too): the highest hit of a line 60 m down.
    pub(crate) fn ground_below(&self, x: f32, z: f32, from: f32) -> Option<f32> {
        let (start, end) = ([x, from, z, 0.0], [x, from - 60.0, z, 0.0]);
        let hit = |world: &skate_core::physics::board_world::BoardWorld| {
            crate::camera::world_query::line(world, start, end, 0.0).ok().filter(|h| h.hit != 0).map(|h| from - 60.0 * h.fraction)
        };
        let stat = hit(&self.world);
        let props = self.prop_layer.as_ref().and_then(|l| hit(l.world()));
        match (stat, props) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (a, b) => a.or(b),
        }
    }

    /// The body of a dropped prop (its pose comes from the prop step), for drawing it.
    pub(crate) fn dropped_body(&self, id: u32) -> Option<u32> {
        match self.dropped.get(&id) {
            Some(Dropped::Prop { body }) => Some(*body),
            _ => None,
        }
    }
}

/// Mounts every file of a baked `catalog.pak` (its `.idx` beside it) into `skate_data::vfs` under `root` (tests).
#[cfg(test)]
pub(crate) fn mount_catalog(pak: &std::path::Path, root: &std::path::Path) {
        let index = std::fs::read(pak.with_extension("pak.idx")).unwrap();
        let pack = std::sync::Arc::new(pak.to_path_buf());
        let count = u32::from_le_bytes(index[32..36].try_into().unwrap()) as usize;
        let mut at = 36;
        for _ in 0..count {
            let len = u16::from_le_bytes(index[at..at + 2].try_into().unwrap()) as usize;
            let name = std::str::from_utf8(&index[at + 2..at + 2 + len]).unwrap().to_owned();
            at += 2 + len;
            let offset = u64::from_le_bytes(index[at..at + 8].try_into().unwrap());
            let size = u32::from_le_bytes(index[at + 8..at + 12].try_into().unwrap()) as usize;
            at += 12;
            skate_data::vfs::mount_disk(root.join(name), pack.clone(), offset, size);
        }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A quarter pipe prop and a quarter pipe terrain piece dropped into DownTown's park, moved and removed.
    /// Data-gated: `SKATE3_ASSET_ROOT` (the install's `assets`) and `SKATE3_CATALOG_PAK` (a `vita-bake catalog` pack).
    #[test]
    #[ignore = "needs the private install data and a baked catalogue"]
    fn catalogue_items_drop_move_and_leave_the_world() {
        let assets = std::path::PathBuf::from(std::env::var_os("SKATE3_ASSET_ROOT").expect("SKATE3_ASSET_ROOT"));
        mount_catalog(std::path::Path::new(&std::env::var_os("SKATE3_CATALOG_PAK").expect("SKATE3_CATALOG_PAK")), &assets);
        let mut map = skate_data::skate_map::SkateMap::load(&assets.join("../maps/DownTownSkatePark.skate")).unwrap();
        map.extensions.push(crate::park_bake::bake(&assets, "Downtown_Fullpark").unwrap().0);
        let mut physics = GamePhysics::load_with_difficulty(&assets, Some(&map), crate::difficulty::Difficulty::parse("Normal").unwrap()).unwrap();
        let at = |x: f32| [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, x, 5.0, 0.0];

        // bo_DMO_Glbl_mediumQuarterPipe: a prop with rails and type data.
        let prop = load_item(&assets, true, "2C7017070011058F").unwrap();
        assert!(!prop.local.is_empty() && prop.type_data.is_some(), "{} triangles", prop.local.len());
        assert!(physics.drop_item(0x6000_0000, &prop, &at(0.0)));
        let body = physics.dropped_body(0x6000_0000).unwrap();
        assert!(physics.prop_dynamics().unwrap().pose(body).is_some());
        // aa_quarterpipePreset_Downtown: a terrain piece with rails.
        let terrain = load_item(&assets, false, "2C70170300030297").unwrap();
        assert!(!terrain.rails.is_empty());
        assert!(physics.drop_item(0x6000_0001, &terrain, &at(20.0)));
        assert!(!physics.drop_item(0x6000_0001, &terrain, &at(20.0)), "one item per id");
        let rails = |p: &GamePhysics| p.car_rails.0.iter().filter(|r| r.tag == PIECE_OWNER_TAG).map(|r| r.origin).collect::<Vec<_>>();
        assert_eq!(rails(&physics), vec![[20.0, 5.0, 0.0]]);
        // Its collision is in the prop layer where it was dropped.
        let instance = match physics.dropped[&0x6000_0001] { Dropped::Terrain { instance, .. } => instance, _ => panic!() };
        let lowest = |p: &GamePhysics| {
            let layer = p.prop_layer.as_ref().unwrap();
            let r = layer.instances()[instance].range.clone();
            layer.world().triangles()[r].iter().map(|t| t.triangle.vertices[0].x).fold(f32::INFINITY, f32::min)
        };
        assert!(lowest(&physics) > 10.0);
        // Moved: collision and rails follow; removed: gone.
        assert!(physics.move_dropped(0x6000_0001, &at(40.0)));
        assert!(lowest(&physics) > 30.0);
        assert_eq!(rails(&physics), vec![[40.0, 5.0, 0.0]]);
        assert!(physics.remove_dropped(0x6000_0001) && physics.remove_dropped(0x6000_0000));
        assert!(rails(&physics).is_empty());
        assert!(physics.prop_dynamics().unwrap().pose(body).is_none());
    }
}
