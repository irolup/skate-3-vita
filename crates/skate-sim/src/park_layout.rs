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

/// The layout a world row places on `map` (the stream's own name, `MegaPark`), if any.
pub(crate) fn layout_for<'a>(worlds: &'a BTreeMap<String, WorldLayout>, world: Option<&str>, map: &str) -> Option<&'a str> {
    let found = worlds.get(world?)?;
    found.stream.strip_prefix("DIST_").unwrap_or(&found.stream).eq_ignore_ascii_case(map).then_some(found.layout.as_str())
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
    let (collision, ranges) = crate::skate_world::posed_collision(&package, &objects, &shapes);
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
        objects.len(), shapes.len(), ranges.len(), collision.len(), rails.len()
    );
    Some(Pieces { collision, ranges, materials: package.materials, rails })
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
}
