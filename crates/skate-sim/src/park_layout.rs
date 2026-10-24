//! skate.Park layouts: the pieces a park world places on its stream (setup `tools/asset_pipeline/park_layout.py`,
//! research b424 / b425 / b430 / b431, doc 38 "skate.Park layouts").
//!
//! A world row (`dist_megapark_full`) names a `.park` layout; setup writes each layout as a render-only MOBJ package
//! `private/park-layouts/<layout>.skate`, its sidecar `<layout>.json` (instances, per-template collision and rails,
//! the movable pieces) and `worlds.json` {world row: {layout, stream}}. Retail keeps every static piece as its own
//! object: it collides through a per-piece static set of the physics world (b431 section 2) and grinds on its
//! template's splines posed once by the piece matrix (82868860 -> 82C1F098, b431 section 1), and both are rebuilt when
//! a layout is applied. Ours: the map load carries the chosen layout (an in-memory [`EXTENSION_TAG`] extension on the
//! loaded map, never written to a file), physics appends the posed piece triangles to the static world as one query
//! range per piece and the posed rails to the static rail provider as one asset per piece, and the renderer spawns the
//! pieces as `ParkPiece` entities. A layout change is a map load (NOT RETAIL: retail re-applies a layout in place;
//! the outcome, the new pieces colliding and grinding, is the same).
//!
//! skate_vita: the physics side only (Bevy-free): the pieces' collision and rails; files through `skate_data::vfs`
//! (a layout's package may be a `skate_data::prop_package`). No `ParkPiece` entities or draw inputs (`load_draws`):
//! the Vita renderer does not draw the pieces yet.
//!
//! Moddable: a mod ships its own `<layout>.skate` / `.json` and `worlds.json` row (the same shapes); the world row
//! choice is part of the serialisable map request ([`crate::map_library::Entry::world`]).
use tracing::{info, warn};
use serde::Deserialize;
use skate_data::skate_map::{Extension, SkateMap};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The in-memory extension a loaded map carries when a park layout was chosen for it (payload: the layout name).
pub(crate) const EXTENSION_TAG: [u8; 4] = *b"PARK";

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub(crate) struct WorldLayout {
    pub layout: String,
    pub stream: String,
}

#[derive(Deserialize)]
struct Index {
    version: u32,
    worlds: BTreeMap<String, WorldLayout>,
}

pub(crate) fn directory(assets: &Path) -> PathBuf {
    assets.join("private").join("park-layouts")
}

/// {world row key: layout} from setup's `worlds.json`; empty when setup wrote none (older install).
pub(crate) fn worlds(assets: &Path) -> BTreeMap<String, WorldLayout> {
    let path = directory(assets).join("worlds.json");
    let Ok(bytes) = skate_data::vfs::read(&path) else { return BTreeMap::new() };
    match serde_json::from_slice::<Index>(&bytes) {
        Ok(index) if index.version == 1 => index.worlds,
        Ok(index) => {
            warn!("SKATE_PARK_LAYOUT: {} version {} not supported", path.display(), index.version);
            BTreeMap::new()
        }
        Err(error) => {
            warn!("SKATE_PARK_LAYOUT: {}: {error}", path.display());
            BTreeMap::new()
        }
    }
}

/// The map a district stream names: `DIST_MegaPark` -> `MegaPark`; the prefix in any case (Zen's is `Dist_Zen`).
pub(crate) fn stream_map(stream: &str) -> &str {
    stream.get(..5).filter(|p| p.eq_ignore_ascii_case("DIST_")).map_or(stream, |_| &stream[5..])
}

/// The layout a world row places on `map` (the stream's own name, `MegaPark`), if any.
pub(crate) fn layout_for<'a>(worlds: &'a BTreeMap<String, WorldLayout>, world: Option<&str>, map: &str) -> Option<&'a str> {
    let found = worlds.get(world?)?;
    stream_map(&found.stream).eq_ignore_ascii_case(map).then_some(found.layout.as_str())
}

/// Mark a freshly loaded map with the layout of `world` (nothing when the world places none on it).
pub(crate) fn attach(map: &mut SkateMap, assets: &Path, world: Option<&str>) {
    map.extensions.retain(|e| e.tag != EXTENSION_TAG);
    let worlds = worlds(assets);
    if let Some(layout) = layout_for(&worlds, world, &map.name) {
        info!("SKATE_PARK_LAYOUT: world {} places {layout} on {}", world.unwrap_or_default(), map.name);
        map.extensions.push(Extension { tag: EXTENSION_TAG, schema: 1, payload: layout.as_bytes().to_vec() });
    }
}

/// The layout a loaded map carries.
pub(crate) fn layout_of(map: &SkateMap) -> Option<&str> {
    map.extensions.iter().find(|e| e.tag == EXTENSION_TAG && e.schema == 1).and_then(|e| std::str::from_utf8(&e.payload).ok())
}

/// One layout's pieces: the render package and its MOBJ placements (template id first in each name, as for props).
pub(crate) fn load_package(assets: &Path, map: &SkateMap) -> Option<(SkateMap, Vec<skate_data::skate_map::StaticObject>)> {
    let layout = layout_of(map)?;
    let path = directory(assets).join(format!("{layout}.skate"));
    let loaded = skate_data::vfs::read(&path).map_err(|e| e.to_string()).and_then(|data| {
        if skate_data::prop_package::is_package(&data) {
            skate_data::prop_package::decode(&data)
        } else {
            SkateMap::parse_render_only(&data)
        }
    });
    let package = match loaded {
        Ok(package) => package,
        Err(error) => {
            warn!("SKATE_PARK_LAYOUT: {}: {error}", path.display());
            return None;
        }
    };
    let mut objects = Vec::new();
    for extension in package.extensions.iter().filter(|e| e.tag == *b"MOBJ") {
        match skate_data::skate_map::parse_static_objects(&package, extension) {
            Ok(parsed) => objects.extend(parsed),
            Err(error) => {
                warn!("SKATE_PARK_LAYOUT: {}: {error}", path.display());
                return None;
            }
        }
    }
    (!objects.is_empty()).then_some((package, objects))
}

/// The posed static pieces of a loaded map's layout for physics: world-space collision triangles with one range per
/// piece (b431: one registration per piece), the package's materials, and the posed grind segments.
pub(crate) struct Pieces {
    pub collision: Vec<skate_data::skate_map::Collision>,
    pub ranges: Vec<std::ops::Range<usize>>,
    pub materials: Vec<skate_data::skate_map::Material>,
    pub rails: Vec<PieceRails>,
    pub fuse: FuseRules,
    /// Per range: the piece stays out of the fused set (retail piece flag 2 from the recipe's `IgnoreFuser` cap,
    /// 828662B8 record +0x10; research b482). Data: the sidecar instance's `caps.IgnoreFuser`.
    pub unfused: Vec<bool>,
    /// Per collision triangle.
    pub tags: Vec<crate::skate_world::FuseTag>,
}

/// A recipe cap counts as set as retail reads its byte (neither 0 nor 0xFF): `True` (or a number other than 0 / 255);
/// `False` and an empty value do not.
fn cap_set(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::String(s) => s.eq_ignore_ascii_case("true") || s.parse::<u8>().is_ok_and(|b| b != 0 && b != 0xFF),
        serde_json::Value::Bool(b) => *b,
        serde_json::Value::Number(n) => n.as_u64().is_some_and(|b| b != 0 && b != 0xFF),
        _ => false,
    }
}

/// The MOBJ ids (`instance_id` low 32 bits) of the sidecar's instances whose `IgnoreFuser` cap is set.
fn ignore_fuser_ids(sidecar: &Path) -> std::collections::HashSet<u32> {
    let Some(value) = skate_data::vfs::read(sidecar).ok().and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok()) else {
        return Default::default();
    };
    value["instances"].as_array().into_iter().flatten()
        .filter(|instance| cap_set(&instance["caps"]["IgnoreFuser"]))
        .filter_map(|instance| instance["instance_id"].as_str().and_then(|s| u64::from_str_radix(s, 16).ok()))
        .map(|id| (id & 0xFFFF_FFFF) as u32)
        .collect()
}

/// Retail's fuse builder default (b478), used when a layout sidecar names no `fuse_weld`.
pub(crate) const FUSE_WELD: f32 = 0.01;

/// docs/hails-additions/39-skatepark-layouts.md#fused-edge-bytes-and-vertex-bits
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct FuseRules {
    pub weld: f32,
    pub concave_tolerance: f32,
    pub coplanar: f32,
    pub in_plane: f32,
    pub below_plane: f32,
    pub wedge_slack: f32,
    pub quad_first_half_only: bool,
    pub ground_planes: [Option<FusePlane>; 6],
    pub plane_distance: f32,
    pub plane_normal: f32,
    pub plane_score_limit: f32,
    pub mesh_floor_pass: bool,
    pub mesh_floor_gap: f32,
    pub mesh_floor_up: [f32; 3],
}

/// A ground plane of the fuser's plane pass; `min == max == 0` = unbounded.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct FusePlane {
    pub normal: [f32; 3],
    pub d: f32,
    pub min: [f32; 3],
    pub max: [f32; 3],
}

/// The fuser job's plane tolerances (job +4 / +8 / +12, read in the recomp at 82874248).
pub(crate) const PLANE_DISTANCE: f32 = 0.05;
pub(crate) const PLANE_NORMAL: f32 = 0.01;
pub(crate) const PLANE_SCORE_LIMIT: f32 = 0.1;
/// job +0x10 (82099920).
pub(crate) const MESH_FLOOR_GAP: f32 = 0.005;

/// 827765C0: the plane a park gets when it names none.
pub(crate) const DEFAULT_GROUND_PLANE: FusePlane = FusePlane { normal: [0.0, 1.0, 0.0], d: 0.0, min: [0.0; 3], max: [0.0; 3] };

impl Default for FuseRules {
    fn default() -> Self {
        Self {
            weld: FUSE_WELD, concave_tolerance: -0.2, coplanar: 0.01, in_plane: 0.05, below_plane: 0.15, wedge_slack: 0.05,
            quad_first_half_only: true,
            ground_planes: [Some(DEFAULT_GROUND_PLANE), None, None, None, None, None],
            plane_distance: PLANE_DISTANCE,
            plane_normal: PLANE_NORMAL,
            plane_score_limit: PLANE_SCORE_LIMIT,
            mesh_floor_pass: true,
            mesh_floor_gap: MESH_FLOOR_GAP,
            mesh_floor_up: [0.0, 1.0, 0.0],
        }
    }
}

/// The sidecar's `fuse_weld` and `fuse_rules` object, else the defaults.
fn fuse_rules(sidecar: &Path) -> FuseRules {
    let mut rules = FuseRules::default();
    let Some(value) = skate_data::vfs::read(sidecar).ok().and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok()) else {
        return rules;
    };
    let number = |v: &serde_json::Value| v.as_f64().map(|x| x as f32).filter(|x| x.is_finite());
    if let Some(weld) = number(&value["fuse_weld"]).filter(|w| *w > 0.0) {
        rules.weld = weld;
    }
    let r = &value["fuse_rules"];
    for (key, field) in [
        ("concave_tolerance", &mut rules.concave_tolerance),
        ("coplanar", &mut rules.coplanar),
        ("in_plane", &mut rules.in_plane),
        ("below_plane", &mut rules.below_plane),
        ("wedge_slack", &mut rules.wedge_slack),
        ("plane_distance", &mut rules.plane_distance),
        ("plane_normal", &mut rules.plane_normal),
        ("plane_score_limit", &mut rules.plane_score_limit),
        ("mesh_floor_gap", &mut rules.mesh_floor_gap),
    ] {
        if let Some(x) = number(&r[key]) {
            *field = x;
        }
    }
    if let Some(b) = r["quad_first_half_only"].as_bool() {
        rules.quad_first_half_only = b;
    }
    if let Some(b) = r["mesh_floor_pass"].as_bool() {
        rules.mesh_floor_pass = b;
    }
    if let Some(list) = r["ground_planes"].as_array() {
        let vec3 = |v: &serde_json::Value, or: [f32; 3]| {
            v.as_array().filter(|a| a.len() == 3).and_then(|a| {
                Some([number(&a[0])?, number(&a[1])?, number(&a[2])?])
            }).unwrap_or(or)
        };
        rules.ground_planes = [None; 6];
        for (slot, p) in rules.ground_planes.iter_mut().zip(list) {
            *slot = Some(FusePlane {
                normal: vec3(&p["normal"], DEFAULT_GROUND_PLANE.normal),
                d: number(&p["d"]).unwrap_or(0.0),
                min: vec3(&p["min"], [0.0; 3]),
                max: vec3(&p["max"], [0.0; 3]),
            });
        }
    }
    rules
}

/// One piece's rails, posed once by its matrix (82C1F098).
pub(crate) struct PieceRails {
    pub piece: u32,
    pub segments: Vec<(skate_core::physics::grind_contact::Primitive, crate::grind_world::PrimitiveMetadata, [[f32; 3]; 2])>,
}

pub(crate) fn load_pieces(assets: &Path, map: &SkateMap) -> Option<Pieces> {
    let (package, objects) = load_package(assets, map)?;
    let layout = layout_of(map)?;
    let sidecar = directory(assets).join(format!("{layout}.json"));
    let shapes = crate::skate_world::load_collision_shapes(&sidecar);
    let (collision, instances, tags) = crate::skate_world::posed_collision(&package, &objects, &shapes);
    let rails = match crate::skate_world::load_grind_rails(&sidecar) {
        Ok(Some(rails)) => objects.iter().filter_map(|object| {
            let template = object.name.split('/').next().unwrap_or_default();
            let list = rails.get(template)?;
            Some(PieceRails { piece: object.id, segments: pose_rails(object.id, &object.transform, list) })
        }).collect(),
        Ok(None) => Vec::new(),
        Err(error) => {
            warn!("SKATE_PARK_LAYOUT: rails of {layout} not loaded: {error}");
            Vec::new()
        }
    };
    info!(
        "SKATE_PARK_LAYOUT: {layout} pieces={} shapes={} collision_pieces={} triangles={} railed_pieces={}",
        objects.len(), shapes.len(), instances.len(), collision.len(), rails.len()
    );
    let fuse = fuse_rules(&sidecar);
    let ignore = ignore_fuser_ids(&sidecar);
    let unfused = instances.iter().map(|(id, _)| ignore.contains(id)).collect();
    Some(Pieces { collision, ranges: instances.into_iter().map(|(_, range)| range).collect(), materials: package.materials, rails, fuse, unfused, tags })
}

/// A piece's template rails through its row-vector affine (v @ basis + translation): chord endpoints and the
/// segment bounds' eight corners, the near-vertical bit from the posed chord as 82C1F098 / 82C1E568 compute it.
pub(crate) fn pose_rails(
    piece: u32,
    t: &[f32; 12],
    rails: &[crate::grind_world::props::PropRail],
) -> Vec<(skate_core::physics::grind_contact::Primitive, crate::grind_world::PrimitiveMetadata, [[f32; 3]; 2])> {
    let place = |p: [f32; 3]| -> [f32; 3] {
        [
            p[0] * t[0] + p[1] * t[3] + p[2] * t[6] + t[9],
            p[0] * t[1] + p[1] * t[4] + p[2] * t[7] + t[10],
            p[0] * t[2] + p[1] * t[5] + p[2] * t[8] + t[11],
        ]
    };
    let mut out = Vec::new();
    for (index, rail) in rails.iter().enumerate() {
        for (segment_index, segment) in rail.segments.iter().enumerate() {
            let mut lo = [f32::INFINITY; 3];
            let mut hi = [f32::NEG_INFINITY; 3];
            for corner in 0..8 {
                let p = place(std::array::from_fn(|i| if corner >> i & 1 == 0 { segment.min[i] } else { segment.max[i] }));
                for i in 0..3 {
                    lo[i] = lo[i].min(p[i]);
                    hi[i] = hi[i].max(p[i]);
                }
            }
            let [start, end] = [segment.start, segment.end].map(|p| {
                let w = place([p[0], p[1], p[2]]);
                [w[0], w[1], w[2], p[3]]
            });
            let tolerance = f32::from_bits(0x3780_0000);
            let vertical = (end[0] - start[0]).abs() <= tolerance && (end[2] - start[2]).abs() <= tolerance;
            let owner = crate::grind_world::props::piece_owner(piece, index);
            out.push((
                skate_core::physics::grind_contact::Primitive { start, end, owner },
                crate::grind_world::PrimitiveMetadata {
                    spline_guids: rail.guids,
                    segment_index: segment_index as u32,
                    flags: if vertical { 0x8000_0000 } else { 0 },
                },
                [lo, hi],
            ));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn index() -> BTreeMap<String, WorldLayout> {
        [("dist_megapark_full".to_string(), WorldLayout { layout: "MegaPark_Fullpark".into(), stream: "DIST_MegaPark".into() })].into()
    }

    #[test]
    fn a_world_places_its_layout_only_on_its_own_stream() {
        let worlds = index();
        assert_eq!(layout_for(&worlds, Some("dist_megapark_full"), "MegaPark"), Some("MegaPark_Fullpark"));
        assert_eq!(layout_for(&worlds, Some("dist_megapark_full"), "DownTown"), None);
        assert_eq!(layout_for(&worlds, Some("dist_megapark"), "MegaPark"), None, "the Empty row places nothing");
        assert_eq!(layout_for(&worlds, None, "MegaPark"), None);
    }

    /// Zen Plaza's stream is `Dist_Zen` (research b539): its Full row places ZenPlaza_Full on the map `Zen`.
    #[test]
    fn the_stream_prefix_matches_in_any_case() {
        let worlds: BTreeMap<String, WorldLayout> =
            [("dist_zenplazapark_full".to_string(), WorldLayout { layout: "ZenPlaza_Full".into(), stream: "Dist_Zen".into() })].into();
        assert_eq!(layout_for(&worlds, Some("dist_zenplazapark_full"), "Zen"), Some("ZenPlaza_Full"));
        assert_eq!((stream_map("DIST_MegaPark"), stream_map("Dist_Zen"), stream_map("DLC_DW_MegaCompund")), ("MegaPark", "Zen", "DLC_DW_MegaCompund"));
    }

    #[test]
    fn piece_rails_are_posed_by_the_piece_matrix() {
        let rail = crate::grind_world::props::PropRail {
            guids: [0x11, 0x22],
            segments: vec![crate::grind_world::props::PropSegment { start: [0., 0.5, 0., 1.], end: [1., 0.5, 0., 1.], min: [0., 0.4, -0.1], max: [1., 0.6, 0.1] }],
        };
        // A quarter turn about +Y (row-vector: +X -> -Z), moved to (10, 0, 5).
        let t = [0., 0., -1., 0., 1., 0., 1., 0., 0., 10., 0., 5.];
        let posed = pose_rails(7, &t, &[rail]);
        let (primitive, metadata, bounds) = &posed[0];
        assert_eq!(primitive.start, [10., 0.5, 5., 1.]);
        assert_eq!(primitive.end, [10., 0.5, 4., 1.]);
        assert_eq!(primitive.owner, crate::grind_world::props::piece_owner(7, 0));
        assert_eq!((metadata.spline_guids, metadata.segment_index, metadata.flags), ([0x11, 0x22], 0, 0));
        let expected = [[9.9, 0.4, 4.], [10.1, 0.6, 5.]];
        assert!((0..2).all(|k| (0..3).all(|i| (bounds[k][i] - expected[k][i]).abs() < 1e-5)), "{bounds:?}");
    }

    #[test]
    fn the_layout_rides_on_the_loaded_map_and_is_replaced_on_reattach() {
        let mut map = SkateMap::parse(include_bytes!("../../../maps/format-demo.skate")).unwrap();
        map.extensions.push(Extension { tag: EXTENSION_TAG, schema: 1, payload: b"Old".to_vec() });
        assert_eq!(layout_of(&map), Some("Old"));
        attach(&mut map, Path::new("no-such-assets"), Some("dist_megapark_full"));
        assert_eq!(layout_of(&map), None, "no index: nothing placed, the stale mark removed");
    }

    /// Phase 2 survey (skate_vita): per layout, the pieces, their templates, render and collision triangles, the
    /// textures (count, RGBA8 bytes, largest) and the rails, from the install (`SKATE3_ASSET_ROOT`).
    #[test]
    #[ignore = "needs the private install data"]
    fn park_layout_survey() {
        let assets = std::path::PathBuf::from(std::env::var_os("SKATE3_ASSET_ROOT").expect("SKATE3_ASSET_ROOT"));
        let mut layouts: Vec<String> = worlds(&assets).values().map(|w| w.layout.clone()).collect();
        layouts.sort();
        layouts.dedup();
        for layout in layouts {
            let mut map = SkateMap::parse_render_only(&[]).unwrap_or_else(|_| empty_map());
            map.extensions.push(Extension { tag: EXTENSION_TAG, schema: 1, payload: layout.as_bytes().to_vec() });
            let Some((package, objects)) = load_package(&assets, &map) else {
                println!("SURVEY {layout}: no package");
                continue;
            };
            let templates: std::collections::BTreeSet<&str> = objects.iter().map(|o| o.name.split('/').next().unwrap_or_default()).collect();
            let ranges: std::collections::BTreeSet<(u32, u32)> = objects.iter().map(|o| (o.first_index, o.index_count)).collect();
            let drawn: u64 = objects.iter().map(|o| o.index_count as u64 / 3).sum();
            let stored = package.geometry.indices.len() / 3;
            let texture_bytes: usize = package.textures.iter().map(|t| t.rgba.len()).sum();
            let largest = package.textures.iter().map(|t| (t.width, t.height)).max_by_key(|(w, h)| w * h).unwrap_or_default();
            let started = std::time::Instant::now();
            let pieces = load_pieces(&assets, &map);
            let load_ms = started.elapsed().as_secs_f32() * 1000.0;
            let (collision, rails) = pieces.as_ref().map_or((0, 0), |p| (p.collision.len(), p.rails.iter().map(|r| r.segments.len()).sum::<usize>()));
            // The fuser over the pieces alone (an empty map: the portable path, then the pieces appended).
            let material = skate_core::physics::contact::RetailContactMaterial { static_friction: 0., dynamic_friction: 0., restitution: 1. };
            let started = std::time::Instant::now();
            let fused = pieces.as_ref().map(|p| crate::skate_world::collision_world_with(&empty_map(), material, Some(crate::skate_world::LayoutPieces::Loaded(p))).map(|w| w.triangles().len()));
            let fuse_ms = started.elapsed().as_secs_f32() * 1000.0;
            println!("SURVEY {layout}: load_pieces {load_ms:.0} ms, fused triangles {fused:?} in {fuse_ms:.0} ms");
            let lightmaps: std::collections::BTreeSet<String> = package.materials.iter().filter(|m| m.textures[1] != 0)
                .map(|m| package.textures[m.textures[1] as usize - 1].name.clone()).collect();
            let unlit = package.materials.iter().filter(|m| m.textures[1] == 0).count();
            println!("SURVEY {layout}: textures named like a lightmap {:?}", package.textures.iter().filter(|t| t.name.to_lowercase().contains("light")).map(|t| (t.name.clone(), t.width, t.height)).collect::<Vec<_>>());
            println!("SURVEY {layout}: lightmap textures {lightmaps:?}, materials without one {unlit}, alpha modes {:?}",
                package.materials.iter().map(|m| m.alpha_mode).collect::<std::collections::BTreeSet<_>>());
            println!(
                "SURVEY {layout}: pieces={} templates={} index_ranges={} drawn_tris={drawn} stored_tris={stored} vertices={} materials={} textures={} rgba_kib={} largest={}x{} collision_tris={collision} rail_segments={rails} props_package={}",
                objects.len(), templates.len(), ranges.len(), package.geometry.vertices.len(), package.materials.len(), package.textures.len(),
                texture_bytes / 1024, largest.0, largest.1,
                skate_data::vfs::is_file(&directory(&assets).join(format!("{layout}.props.skate"))),
            );
        }
    }

    fn empty_map() -> SkateMap {
        SkateMap {
            version: 0,
            name: String::new(),
            spawn: [0.0; 3],
            heading: 0.0,
            environment: Vec::new(),
            materials: Vec::new(),
            textures: Vec::new(),
            geometry: skate_data::skate_map::Geometry { vertices: Vec::new(), indices: Vec::new(), collision: Vec::new() },
            rails: Vec::new(),
            doors: Vec::new(),
            lights: Vec::new(),
            routes: Vec::new(),
            extensions: Vec::new(),
        }
    }
}
