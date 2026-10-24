//! Adapter for authored .skate world geometry. Does not replace controllers.
//!
//! Collision part of skate-game `skate_world.rs` (`cb79689`), lines 27–356
//! (`validate_runtime`, `collision_world` and their helpers), unchanged. The render
//! adapter (meshes, textures, shadows) is not copied. `bevy::prelude::Vec3` is
//! `glam::Vec3`, the same type bevy re-exports.
use glam::{Vec2, Vec3};
use tracing::info_span;
use skate_core::{
    math::Vector3,
    physics::{
        board_world::{
            BoardWorld, WorldTriangle,
            query_metadata::{Bounds, QueryMesh, QueryMetadata, QueryPool},
        },
        collision::TriangleFeature,
        contact::RetailContactMaterial,
        drive_frames::RetailAffineTransform,
    },
};
use skate_data::skate_map::SkateMap;
use std::collections::HashMap;

pub(crate) fn validate_runtime(map: &SkateMap) -> Result<(), String> {
    let _span = info_span!("validate_map").entered();
    let archive = retail_archive(map)?;
    // skate_vita: a district pack streams its collision (collision_window.rs).
    if map.geometry.collision.is_empty() && archive.is_none() && !crate::collision_window::district_installed() {
        return Err("SKATE map has no collision geometry".into());
    }
    for triangle in map.geometry.collision.iter().filter(|_| archive.is_none()) {
        if let Some(edges) = triangle.native_edges {
            decode_native_edges(edges)?;
        }
    }
    if !map.doors.is_empty() {
        return Err(format!(
            "Map '{}' contains {} hinged doors. This imported game has no door body/controller adapter yet; refusing to drop their geometry or turn them into static walls.",
            map.name,
            map.doors.len()
        ));
    }
    for extension in &map.extensions {
        let tag = String::from_utf8_lossy(&extension.tag);
        if extension.tag == *b"RWCM" {
            continue;
        }
        if extension.tag == *b"MOBJ" {
            skate_data::skate_map::validate_static_objects(map, extension)?;
            continue;
        }
        if extension.tag == crate::park_layout::EXTENSION_TAG {
            // The in-memory skate.Park layout mark (park_layout::attach), never in a package file.
            continue;
        }
        if extension.tag == crate::park_bake::EXTENSION_TAG && extension.schema == 1 {
            // skate_vita: a park pack's baked layout (park_bake.rs), appended to the static world and the rails.
            continue;
        }
        if extension.tag == *b"SKYB" && extension.schema == 1 {
            eprintln!(
                "SKATE LIMITATION: SKYB retail sky retained; using the map horizon until its shader adapter is available."
            );
            continue;
        }
        if extension.tag != *b"WMET" && extension.tag != *b"WCFG" && extension.tag != *b"BMAT" {
            return Err(format!(
                "SKATE extension {tag} schema {} is decoded but its runtime adapter is not implemented. Refusing to silently omit potentially required world geometry.",
                extension.schema
            ));
        }
        eprintln!(
            "SKATE LIMITATION: {tag} extension retained; its runtime behavior is not connected."
        );
    }
    if map.materials.iter().any(|m| m.retail_definition.is_some()) {
        eprintln!(
            "SKATE_RENDER: retail world material adapter enabled; unsupported families retain portable PBR rendering."
        );
    }
    if map.textures.iter().any(|t| t.width == 0) {
        return Err(
            "SKATE contains external texture placeholders; supply a package with embedded textures"
                .into(),
        );
    }
    if !map.routes.is_empty() {
        eprintln!(
            "SKATE LIMITATION: {} NPC routes parsed; supplied game has no NPC controller.",
            map.routes.len()
        );
    }
    if map.lights.iter().any(|l| l.kind == 2) {
        eprintln!(
            "SKATE LIMITATION: area-light records retained; Bevy adapter currently renders point and spot lights only."
        );
    }
    eprintln!(
        "SKATE LIMITATION: native frame-lighting/day-night controller is not connected; using package lighting and retail district sky where available."
    );
    eprintln!(
        "SKATE_MAP_LOADED name={:?} version={} render_triangles={} collision_triangles={} textures={} spawn={:?}",
        map.name,
        map.version,
        map.geometry.indices.len() / 3,
        map.geometry.collision.len(),
        map.textures.len(),
        map.spawn
    );
    Ok(())
}

/// TU3 ClusteredMesh::GetUnitVolumes (82AC8A68): fdivs then fsubs,
/// using the pi-squared word at 822F88D0. This is not acos/angle decoding.
/// Bit 7 denotes an unmatched compiler edge and is not a triangle flag.
pub(crate) fn decode_native_edges(edges: [u8; 3]) -> Result<(u32, [f32; 3]), String> {
    let mut flags = 1 | TriangleFeature::ONE_SIDED | TriangleFeature::USE_EDGE_COSINES;
    let mut cosines = [0.; 3];
    for (i, code) in edges.into_iter().enumerate() {
        let exponent = code & 0x1f;
        // The native signed 32-bit shift becomes negative at 28 and zero
        // above it. Reject those malformed codes instead of producing NaNs.
        if exponent >= 28 {
            return Err(format!(
                "Invalid SKATE native edge angle code {exponent} at corner {i}"
            ));
        }
        cosines[i] = 1.0 - f32::from_bits(0x411d_e9e7) / ((8_u32 << exponent) as f32);
        flags |= u32::from(code & 0x20) << i;
        flags |= u32::from(code & 0x40) << (i + 3);
    }
    Ok((flags, cosines))
}

fn retail_archive(map: &SkateMap) -> Result<Option<&[u8]>, String> {
    let mut archives = map.extensions.iter().filter(|e| e.tag == *b"RWCM");
    let Some(archive) = archives.next() else {
        return Ok(None);
    };
    if archive.schema != 1 || archives.next().is_some() {
        return Err("SKATE requires one RWCM extension with schema 1".into());
    }
    Ok(Some(&archive.payload))
}

fn retail_collision_world(
    archive: &[u8],
    material: RetailContactMaterial,
    pieces: Option<LayoutPieces>,
) -> Result<BoardWorld, String> {
    let mut triangles = Vec::new();
    let mut packed_surfaces = Vec::new();
    let mut meshes = Vec::new();
    let count = skate_data::retail_collision::visit_clusters(archive, |_, cluster| {
        // Preserve cluster order and partition further only for group filters.
        let mut cursor = 0;
        while cursor < cluster.len() {
            let group = cluster[cursor].group;
            let start = triangles.len();
            while cursor < cluster.len() && cluster[cursor].group == group {
                let source = cluster[cursor];
                let (flags, cosines) = match source.edges {
                    Some(edges) => {
                        let (mut flags, cosines) = decode_native_edges(edges)?;
                        flags &= !TriangleFeature::ONE_SIDED;
                        if source.one_sided {
                            flags |= TriangleFeature::ONE_SIDED;
                        }
                        (flags, cosines)
                    }
                    // TriangleVolume ctor82AC7770 retains these defaults when
                    // the unit has no edge data; the mesh sidedness is not read.
                    None => (0x1e1, [-1.; 3]),
                };
                triangles.push(
                    WorldTriangle::from_vertices(
                        source.points.map(|p| Vector3::new(p[0], p[1], p[2])),
                        material,
                        u32::from(source.surface),
                        flags,
                        cosines,
                        0.,
                    )
                    .ok_or("Invalid RWCM collision triangle")?,
                );
                packed_surfaces.push(source.surface);
                cursor += 1;
            }
            let range = start..triangles.len();
            let bounds = Bounds::from_points(
                triangles[range.clone()]
                    .iter()
                    .flat_map(|t| t.triangle.vertices),
            )
            .ok_or("Invalid RWCM cluster bounds")?;
            meshes.push(QueryMesh {
                geometry: 0, rejection_flags: 0,
                triangle_range: range,
                local_to_world: RetailAffineTransform::IDENTITY,
                world_to_local: RetailAffineTransform::IDENTITY,
                local_bounds: bounds,
                matching_group: i32::from(group),
                pool: QueryPool::Ground,
            });
        }
        Ok(())
    })?;
    eprintln!(
        "SKATE_RWCM_READY triangles={count} query_clusters={} source=embedded",
        meshes.len()
    );
    if let Some(pieces) = pieces {
        pieces.append(&mut triangles, &mut packed_surfaces, &mut meshes, material)?;
    }
    BoardWorld::with_query_metadata(
        triangles,
        QueryMetadata {
            packed_surfaces,
            meshes,
            static_edges: vec![],
            island_flags: 0,
        },
    )
    .map_err(str::to_owned)
}

pub(crate) fn collision_world(
    map: &SkateMap,
    material: RetailContactMaterial,
) -> Result<BoardWorld, String> {
    collision_world_with(map, material, None)
}

/// [`collision_world`] plus a skate.Park layout's placed pieces (crate::park_layout::Pieces): their posed triangles
/// after the map's own, one query range per piece (retail registers each piece in the static set, b431).
/// skate_vita: or a baked layout's fused rows ([`LayoutPieces::Baked`]).
pub(crate) fn collision_world_with(
    map: &SkateMap,
    material: RetailContactMaterial,
    pieces: Option<LayoutPieces>,
) -> Result<BoardWorld, String> {
    let pieces = pieces.filter(|p| !p.is_empty());
    if let Some(archive) = retail_archive(map)? {
        return retail_collision_world(archive, material, pieces);
    }
    let Some(pieces) = pieces else {
        return portable_world(&map.geometry.collision, &map.materials, material, None);
    };
    let (mut triangles, mut packed_surfaces, mut meshes) = portable_parts(&map.geometry.collision, &map.materials, material, None, false)?;
    pieces.append(&mut triangles, &mut packed_surfaces, &mut meshes, material)?;
    BoardWorld::with_query_metadata(triangles, QueryMetadata { packed_surfaces, meshes, static_edges: vec![], island_flags: 0 })
        .map_err(str::to_owned)
}

fn append_pieces(
    triangles: &mut Vec<WorldTriangle>,
    packed_surfaces: &mut Vec<u16>,
    meshes: &mut Vec<QueryMesh>,
    pieces: &crate::park_layout::Pieces,
    material: RetailContactMaterial,
) -> Result<(), String> {
    // 828728B8 / 827765C0: docs/hails-additions/39-skatepark-layouts.md#code-reference-skate_worldrs
    let (fused, unfused) = split_unfused(pieces);
    let (more, surfaces, piece_meshes) =
        portable_parts_fused(&fused.0, &pieces.materials, material, Some(&fused.1), true, Some((&pieces.fuse, &fused.2)))?;
    info!("SKATE_PARK_FUSE weld={} m triangles={} kept={} unfused_pieces={} rules={:?} (collapsed and back-to-back faces dropped, seams paired, edge bytes as retail's fuser)",
        pieces.fuse.weld, fused.0.len(), more.len(), unfused.1.len(), pieces.fuse);
    let offset = triangles.len();
    triangles.extend(more);
    packed_surfaces.extend(surfaces);
    meshes.extend(piece_meshes.into_iter().map(|mut mesh| {
        mesh.triangle_range = mesh.triangle_range.start + offset..mesh.triangle_range.end + offset;
        mesh
    }));
    Ok(())
}
/// skate_vita: what [`append_pieces`] appends, as rows with one range per piece (for the bake, crate::park_bake).
pub(crate) fn fused_piece_rows(pieces: &crate::park_layout::Pieces) -> Result<(Vec<FusedTriangle>, Vec<std::ops::Range<usize>>), String> {
    let (fused, _) = split_unfused(pieces);
    let (rows, splits) = fuse_rows(&fused.0, &pieces.materials, Some(&fused.1), true, Some((&pieces.fuse, &fused.2)))?;
    Ok((rows, splits.unwrap_or_default()))
}

/// skate_vita: [`append_pieces`] from a baked layout's rows (no fuse at load).
fn append_baked(
    triangles: &mut Vec<WorldTriangle>,
    packed_surfaces: &mut Vec<u16>,
    meshes: &mut Vec<QueryMesh>,
    baked: &crate::park_bake::Baked<'_>,
    material: RetailContactMaterial,
) -> Result<(), String> {
    let piece_meshes = append_rows(baked.rows(), Some(&baked.ranges), material, triangles, packed_surfaces)?;
    info!("SKATE_PARK_BAKED triangles={} pieces={}", baked.len(), baked.ranges.len());
    meshes.extend(piece_meshes);
    Ok(())
}

/// The pieces a static world takes beside the map's own: a layout's loaded pieces (fused at load, upstream) or a baked
/// layout's rows (skate_vita, crate::park_bake).
#[derive(Clone, Copy)]
pub(crate) enum LayoutPieces<'a> {
    Loaded(&'a crate::park_layout::Pieces),
    Baked(&'a crate::park_bake::Baked<'a>),
}

impl LayoutPieces<'_> {
    fn append(self, triangles: &mut Vec<WorldTriangle>, packed_surfaces: &mut Vec<u16>, meshes: &mut Vec<QueryMesh>, material: RetailContactMaterial) -> Result<(), String> {
        match self {
            Self::Loaded(pieces) => append_pieces(triangles, packed_surfaces, meshes, pieces, material),
            Self::Baked(baked) => append_baked(triangles, packed_surfaces, meshes, baked, material),
        }
    }

    fn is_empty(self) -> bool {
        match self {
            Self::Loaded(pieces) => pieces.collision.is_empty(),
            Self::Baked(baked) => baked.len() == 0,
        }
    }
}

/// The layout's IgnoreFuser pieces as a world of their own, which only props collide with:
/// docs/hails-additions/39-skatepark-layouts.md#props-and-ignorefuser-pieces
pub(crate) fn props_only_world(
    pieces: &crate::park_layout::Pieces,
    material: RetailContactMaterial,
) -> Result<Option<BoardWorld>, String> {
    let (_, unfused) = split_unfused(pieces);
    if unfused.0.is_empty() {
        return Ok(None);
    }
    let (triangles, packed_surfaces, meshes) = portable_parts(&unfused.0, &pieces.materials, material, Some(&unfused.1), true)?;
    info!("SKATE_PARK_PROPS_ONLY pieces={} triangles={}", unfused.1.len(), triangles.len());
    BoardWorld::with_query_metadata(triangles, QueryMetadata { packed_surfaces, meshes, static_edges: vec![], island_flags: 0 })
        .map(Some)
        .map_err(str::to_owned)
}
type PieceSet =(Vec<skate_data::skate_map::Collision>, Vec<std::ops::Range<usize>>, Vec<FuseTag>);
/// The layout's pieces split into the fused set and the IgnoreFuser pieces, each with its own triangles and one range
/// per piece (layout order kept, so both are deterministic).
fn split_unfused(pieces: &crate::park_layout::Pieces) -> (PieceSet, PieceSet) {
    let (mut fused, mut unfused): (PieceSet, PieceSet) = Default::default();
    for (index, range) in pieces.ranges.iter().enumerate() {
        let set = if pieces.unfused.get(index).copied().unwrap_or(false) { &mut unfused } else { &mut fused };
        let start = set.0.len();
        set.0.extend_from_slice(&pieces.collision[range.clone()]);
        set.1.push(start..set.0.len());
        set.2.extend(range.clone().map(|i| pieces.tags.get(i).copied().unwrap_or_default()));
    }
    (fused, unfused)
}

/// skate_vita: a prop stands on the floor with its base plane (its template's lowest points, where
/// [`crate::physics::prop_dynamics`] seats it). Welded alone, the faces meeting on that plane
/// form knife-edges (a ramp's slope and its bottom face at the lip): convex edges whose contacts
/// point back at a wheel rolling up, so a ramp's lip slowed the board. The floor is the neighbour
/// of every edge on the base plane instead (as if the prop were welded into the ground): the
/// floor normal's cosine and no convex bit, like a static ground-to-ramp or ground-to-wall join.
fn floor_base_edges(points: &[[[f32; 3]; 3]], range: std::ops::Range<usize>, flags: &mut [u32], cosines: &mut [[f32; 3]]) {
    let Some(base) = points[range.clone()].iter().flatten().map(|p| p[1]).reduce(f32::min) else { return };
    for i in range {
        let [a, b, c] = points[i].map(Vec3::from_array);
        let Some(normal) = (b - a).cross(c - a).try_normalize() else { continue };
        for edge in 0..3 {
            let (p, q) = (points[i][edge], points[i][(edge + 1) % 3]);
            if p[1] - base < 0.001 && q[1] - base < 0.001 {
                flags[i] &= !(0x20 << edge);
                cosines[i][edge] = normal.y;
            }
        }
    }
}

/// Welded vertices and reconstructed edge features: docs/hails-additions/39-skatepark-layouts.md#code-reference-skate_worldrs
fn welded_edge_features(
    triangles: &[[[f32; 3]; 3]],
    reconstruct: bool,
) -> Result<(Vec<Vec3>, Vec<[usize; 3]>, Vec<u32>, Vec<[f32; 3]>), String> {
    welded_edge_features_within(triangles, reconstruct, None)
}
/// [`welded_edge_features`] with `merge` = a distance weld (a vertex joins the first earlier vertex within `merge`
/// metres, in triangle order, so the result is deterministic): the fused park pieces (b478). A triangle whose corners
/// merge together has repeated vertex ids; its edges pair with nothing and the caller drops it.
fn welded_edge_features_within(
    triangles: &[[[f32; 3]; 3]],
    reconstruct: bool,
    merge: Option<f32>,
) -> Result<(Vec<Vec3>, Vec<[usize; 3]>, Vec<u32>, Vec<[f32; 3]>), String> {
    // Match the reference RW mesh compiler's 1 mm vertex welding and reversed
    // edge pairing. Triangle diagonals are adjacency, never authored ledges.
    let mut welded = HashMap::<[i64; 3], usize>::new();
    let mut cells = HashMap::<[i64; 3], Vec<usize>>::new();
    let mut positions = Vec::<Vec3>::new();
    let mut vertices = Vec::new();
    let mut normals = Vec::new();
    for points in triangles {
        let ids = points.map(|p| {
            if let Some(merge) = merge {
                let cell = p.map(|v| (f64::from(v) / f64::from(merge)).floor() as i64);
                let point = Vec3::from_array(p);
                let near = (-1..=1).flat_map(|x| (-1..=1).flat_map(move |y| (-1..=1).map(move |z| [x, y, z])))
                    .filter_map(|d| cells.get(&[cell[0] + d[0], cell[1] + d[1], cell[2] + d[2]]))
                    .flatten()
                    .copied()
                    .filter(|&id| positions[id].distance(point) <= merge)
                    .min();
                return near.unwrap_or_else(|| {
                    let id = positions.len();
                    positions.push(point);
                    cells.entry(cell).or_default().push(id);
                    id
                });
            }
            let inverse = 1.0 / f64::from(0.001_f32);
            let key = p.map(|v| (f64::from(v) * inverse).round() as i64);
            *welded.entry(key).or_insert_with(|| {
                let id = positions.len();
                positions.push(Vec3::from_array(p));
                id
            })
        });
        let [a, b, c] = points.map(Vec3::from_array);
        // 82AC2010: the fused path drops zero-area triangles after the merge instead of failing the set.
        let normal = match ((b - a).cross(c - a).try_normalize(), merge) {
            (Some(n), _) => n,
            (None, Some(_)) => Vec3::ZERO,
            (None, None) => return Err("Invalid SKATE collision triangle normal".into()),
        };
        vertices.push(ids);
        normals.push(normal);
    }
    let mut cosines = vec![[1.; 3]; vertices.len()];
    let mut flags =
        vec![TriangleFeature::ONE_SIDED | TriangleFeature::USE_EDGE_COSINES | 0xe0; vertices.len()];
    // Fully native maps need no reconstructed adjacency. Mixed maps still
    // include every face when finding neighbors for their authored geometry.
    if reconstruct {
        let mut open = HashMap::<(usize, usize), (usize, usize)>::new();
        for (i, ids) in vertices.iter().enumerate() {
            for edge in 0..3 {
                let (a, b) = (ids[edge], ids[(edge + 1) % 3]);
                // A collapsed edge of a fused triangle (dropped by the caller); the 1 mm path keeps its old pairing.
                if a == b && merge.is_some() {
                    continue;
                }
                if let Some((other, oe)) = open.remove(&(b, a)) {
                    let cosine = normals[i].dot(normals[other]).clamp(-1., 1.);
                    let orientation =
                        (positions[b] - positions[a]).dot(normals[i].cross(normals[other]));
                    // ExtendedEdgeCosine / MakeEdgeCode in rw_collision_mesh.cpp:
                    // orientation >= -1e-6 is convex; flat edges have no convex bit.
                    for (ti, e) in [(i, edge), (other, oe)] {
                        cosines[ti][e] = cosine;
                        if orientation <= -1.0e-6 || cosine >= 1. {
                            flags[ti] &= !(0x20 << e);
                        }
                    }
                } else {
                    open.entry((a, b)).or_insert((i, edge));
                }
            }
        }
        let mut adjacent = vec![Vec::new(); positions.len()];
        for (i, ids) in vertices.iter().enumerate() {
            for &v in ids {
                adjacent[v].push(i);
            }
        }
        for (v, faces) in adjacent.iter().enumerate() {
            let reference = normals[faces[0]];
            if faces
                .iter()
                .all(|&i| (reference.dot(normals[i]) - 1.).abs() <= 0.01)
            {
                for &i in faces {
                    for corner in 0..3 {
                        if vertices[i][corner] == v {
                            flags[i] |= 0x200 << corner;
                        }
                    }
                }
            }
        }
    }
    Ok((positions, vertices, flags, cosines))
}
/// Retail's fused-mesh edge score (`82AC00A0`): d = n1.n2; a convex or flat edge (edge . (n1 x n2) > -1e-6) scores
/// max(d, -1) in [-1, 1], a concave one min(2 - d, 3) in [1, 3]. Faces folded flat onto each other score 3 (concave)
/// or -1 (a convex knife edge).
fn fused_edge_score(n1: Vec3, n2: Vec3, edge: Vec3) -> f32 {
    // 82AC00A0's fmadds / fmsubs order, so a coplanar pair can reach exactly 1.0.
    let d = n2.z.mul_add(n1.z, n2.x.mul_add(n1.x, n2.y * n1.y));
    let cx = n1.y.mul_add(n2.z, -(n2.y * n1.z));
    let cy = n2.x.mul_add(n1.z, -(n1.x * n2.z));
    let cz = n1.x.mul_add(n2.y, -(n2.x * n1.y));
    let orientation = edge.x.mul_add(cx, edge.z.mul_add(cz, edge.y * cy));
    if orientation > -1.0e-6 { d.max(-1.0) } else { (2.0 - d).min(3.0) }
}
/// Retail's back-to-back cull thresholds (`828755B8`: 2.99 `822F90E8`, -0.99 `822F8E60`).
const FUSED_FOLD_ABOVE: f32 = 2.99;
const FUSED_FOLD_BELOW: f32 = -0.99;
/// 82876188: docs/hails-additions/39-skatepark-layouts.md#code-reference-skate_worldrs. `pieces[i]` = triangle i's
/// piece; also returns `dropped[i]`.
#[allow(clippy::type_complexity)]
fn fused_edge_features(
    triangles: &[[[f32; 3]; 3]],
    rules: &crate::park_layout::FuseRules,
    pieces: &[usize],
    tags: &[FuseTag],
) -> Result<(Vec<Vec3>, Vec<[usize; 3]>, Vec<u32>, Vec<[f32; 3]>, Vec<bool>), String> {
    let (positions, vertices, mut flags, mut cosines) = welded_edge_features_within(triangles, false, Some(rules.weld))?;
    let n = vertices.len();
    // 82872D70: one builder triangle per unit, docs/hails-additions/39-skatepark-layouts.md#fused-vertex-order
    let excluded: Vec<bool> = (0..n).map(|i| rules.quad_first_half_only && tags.get(i).is_some_and(|t| t.second_half)).collect();
    let normals: Vec<Option<Vec3>> = vertices
        .iter()
        .zip(&excluded)
        .map(|(v, &excluded)| {
            let [a, b, c] = v.map(|id| positions[id]);
            (!excluded && v[0] != v[1] && v[1] != v[2] && v[0] != v[2]).then(|| fused_normal(a, b, c)).flatten()
        })
        .collect();
    let mut dropped: Vec<bool> = normals.iter().map(Option::is_none).collect();
    let mut users = vec![Vec::new(); positions.len()];
    let ranked = fused_vertex_ranks(&vertices, positions.len(), pieces, tags);
    let order: Vec<(u32, u32)> = match &ranked {
        // 82AC21D8 -> 82AC4A40.
        Some(rank) => {
            let mut table: Vec<(u32, u32)> = (0..n)
                .filter(|&i| !excluded[i])
                .flat_map(|i| vertices[i].map(|id| (rank[id], i as u32)))
                .collect();
            eastl_quick_sort(&mut table);
            table
        }
        None => vertices.iter().enumerate().flat_map(|(i, v)| v.map(|id| (id as u32, i as u32))).collect(),
    };
    let vertex_of_rank: Vec<usize> = match &ranked {
        Some(rank) => {
            let mut back = vec![0; positions.len()];
            for (id, &r) in rank.iter().enumerate() {
                back[r as usize] = id;
            }
            back
        }
        None => (0..positions.len()).collect(),
    };
    for (r, i) in order {
        let (id, i) = (vertex_of_rank[r as usize], i as usize);
        if !dropped[i] && users[id].last() != Some(&i) {
            users[id].push(i);
        }
    }
    let normal = |i: usize| normals[i].unwrap_or(Vec3::ZERO);
    // 82875438: the live triangle of `t`'s piece across `t`'s edge (from, to), and its third vertex.
    let partner = |dropped: &[bool], t: usize, from: usize, to: usize| {
        users[from].iter().copied().filter(|&u| u != t && !dropped[u] && pieces[u] == pieces[t]).find_map(|u| {
            (0..3).find(|&j| vertices[u][j] == from && vertices[u][(j + 2) % 3] == to).map(|j| (u, vertices[u][(j + 1) % 3]))
        })
    };
    // 828755B8: back-to-back faces of different pieces.
    for t in 0..n {
        if dropped[t] {
            continue;
        }
        for e in 0..3 {
            let (a, b, c) = (vertices[t][e], vertices[t][(e + 1) % 3], vertices[t][(e + 2) % 3]);
            for &u in &users[a] {
                if u <= t || dropped[u] || pieces[u] == pieces[t] {
                    continue;
                }
                for k in 0..3 {
                    if vertices[u][k] != a || vertices[u][(k + 2) % 3] != b {
                        continue;
                    }
                    let score = fused_edge_score(normal(t), normal(u), positions[b] - positions[a]);
                    if !(score > FUSED_FOLD_ABOVE || score < FUSED_FOLD_BELOW) {
                        continue;
                    }
                    let x = vertices[u][(k + 1) % 3];
                    if c == x {
                        dropped[t] = true;
                        dropped[u] = true;
                    } else if let (Some((t2, t2_third)), Some((u2, u2_third))) =
                        (partner(&dropped, t, b, c), partner(&dropped, u, a, x))
                    {
                        if t2_third == x && u2_third == c {
                            for i in [t, t2, u, u2] {
                                dropped[i] = true;
                            }
                        }
                    }
                }
            }
        }
    }
    // 82875B20 / 82875CA8: best-score pairing; unpaired edges keep score 1.
    let mut neighbour = vec![[usize::MAX; 3]; n];
    let mut best = vec![[1.0f32; 3]; n];
    let unlink = |neighbour: &mut Vec<[usize; 3]>, best: &mut Vec<[f32; 3]>, x: usize, from: usize| {
        if let Some(j) = (0..3).find(|&j| neighbour[x][j] == from) {
            neighbour[x][j] = usize::MAX;
            best[x][j] = 1.0;
        }
    };
    for t in 0..n {
        if dropped[t] {
            continue;
        }
        for e in 0..3 {
            let (a, b) = (vertices[t][e], vertices[t][(e + 1) % 3]);
            for ui in 0..users[a].len() {
                let u = users[a][ui];
                if u >= t || dropped[u] {
                    continue;
                }
                let Some(ue) = (0..3).find(|&k| vertices[u][k] == b && vertices[u][(k + 1) % 3] == a) else { continue };
                let score = fused_edge_score(normal(t), normal(u), positions[b] - positions[a]);
                let (tn, un) = (neighbour[t][e], neighbour[u][ue]);
                let replace = match (tn == usize::MAX, un == usize::MAX) {
                    (true, true) => true,
                    (true, false) => best[u][ue] < score,
                    (false, true) => best[t][e] < score,
                    (false, false) => tn != u && un != t && best[t][e] < score && best[u][ue] < score,
                };
                if !replace {
                    continue;
                }
                if tn != usize::MAX {
                    unlink(&mut neighbour, &mut best, tn, t);
                }
                if un != usize::MAX {
                    unlink(&mut neighbour, &mut best, un, u);
                }
                neighbour[t][e] = u;
                neighbour[u][ue] = t;
                best[t][e] = score;
                best[u][ue] = score;
            }
        }
    }
    // 82874248 / 82874070: docs/hails-additions/39-skatepark-layouts.md#ground-plane-pass
    for plane in rules.ground_planes.iter().flatten() {
        let np = Vec3::from_array(plane.normal);
        let (lo, hi) = (Vec3::from_array(plane.min), Vec3::from_array(plane.max));
        let tiny = f32::from_bits(0x3400_0000);
        let boxed = lo.abs().max_element() > tiny || hi.abs().max_element() > tiny;
        for t in (0..n).filter(|&t| !dropped[t]) {
            let p = vertices[t].map(|id| positions[id]);
            if boxed && !p.iter().any(|q| q.cmpge(lo).all() && q.cmple(hi).all()) {
                continue;
            }
            if (-normal(t) - np).abs().max_element() > rules.plane_normal || (np.dot(p[1]) - plane.d).abs() > rules.plane_distance {
                continue;
            }
            for e in 0..3 {
                let u = neighbour[t][e];
                if u == usize::MAX || best[t][e] >= rules.plane_score_limit {
                    continue;
                }
                let s = fused_edge_score(np, normal(u), p[(e + 1) % 3] - p[e]);
                if let Some(j) = (0..3).find(|&j| neighbour[u][j] == t) {
                    best[u][j] = s;
                }
                best[t][e] += s;
            }
        }
    }
    // 82873EA0 / 82890E20: docs/hails-additions/39-skatepark-layouts.md#mesh-floor-pass
    if rules.mesh_floor_pass {
        let up = Vec3::from_array(rules.mesh_floor_up);
        let mut floors: Vec<(f32, usize)> = (0..n)
            .filter(|&g| !dropped[g] && (1.0 - normal(g).dot(up)).abs() <= rules.plane_normal)
            .map(|g| (positions[vertices[g][0]].dot(up), g))
            .collect();
        floors.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        for t in (0..n).filter(|&t| !dropped[t]) {
            let p = vertices[t].map(|id| positions[id]);
            if (-1.0 - normal(t).dot(up)).abs() > rules.plane_normal || p[0].dot(up).abs() <= rules.plane_distance {
                continue;
            }
            for e in 0..3 {
                let u = neighbour[t][e];
                if u == usize::MAX || best[t][e] >= -rules.plane_score_limit {
                    continue;
                }
                let (a, b) = (p[e], p[(e + 1) % 3]);
                let h = a.dot(up);
                let start = floors.partition_point(|f| f.0 < h - rules.mesh_floor_gap);
                let on_floor = floors[start..].iter().take_while(|f| f.0 <= h + rules.mesh_floor_gap).any(|&(_, g)| {
                    pieces[g] != pieces[t] && xz_segment_meets_triangle(vertices[g].map(|id| positions[id]), a, b)
                });
                if on_floor {
                    let s = fused_edge_score(up, normal(u), b - a);
                    if let Some(j) = (0..3).find(|&j| neighbour[u][j] == t) {
                        best[u][j] = s;
                    }
                    best[t][e] += s;
                }
            }
        }
    }
    // 82AC2438: docs/hails-additions/39-skatepark-layouts.md#fused-edge-bytes-and-vertex-bits
    let mut edges = vec![[0u8; 3]; n];
    for t in (0..n).filter(|&t| !dropped[t]) {
        for e in 0..3 {
            edges[t][e] = fused_edge_byte(best[t][e], neighbour[t][e] != usize::MAX, rules.concave_tolerance);
        }
    }
    // 82875200: docs/hails-additions/39-skatepark-layouts.md#fused-edge-bytes-and-vertex-bits
    for (v, faces) in users.iter().enumerate() {
        let faces: Vec<usize> = faces.iter().copied().filter(|&i| !dropped[i]).collect();
        let Some((&first, rest)) = faces.split_first() else { continue };
        let coplanar = rest.iter().all(|&i| (normal(first).dot(normal(i)) - 1.).abs() <= rules.coplanar);
        if coplanar || fused_vertex_not_convex(&positions, &vertices, normal(first), v, first, rest, rules) {
            for &i in &faces {
                for corner in 0..3 {
                    if vertices[i][corner] == v {
                        edges[i][corner] |= 0x40;
                    }
                }
            }
        }
    }
    for t in (0..n).filter(|&t| !dropped[t]) {
        let (decoded, decoded_cosines) = decode_native_edges(edges[t])?;
        flags[t] = decoded & !1;
        cosines[t] = decoded_cosines;
    }
    Ok((positions, vertices, flags, cosines, dropped))
}
/// 82873270 / 828731A8: the segment a-b and the triangle overlap in xz (four separating axes; touching counts).
fn xz_segment_meets_triangle(tri: [Vec3; 3], a: Vec3, b: Vec3) -> bool {
    let flat = |v: Vec3| Vec2::new(v.x, v.z);
    let (t, s) = (tri.map(flat), [flat(a), flat(b)]);
    let axes = [t[1] - t[0], t[0] - t[2], t[2] - t[1], s[1] - s[0]];
    axes.iter().filter_map(|d| d.perp().try_normalize()).all(|axis| {
        let span = |points: &[Vec2]| points.iter().fold((f32::MAX, f32::MIN), |(lo, hi), p| (lo.min(p.dot(axis)), hi.max(p.dot(axis))));
        let ((tl, th), (sl, sh)) = (span(&t), span(&s));
        (sl - th).max(tl - sh) <= 0.0
    })
}
/// 82875CA8: a builder triangle's normal, in retail's fmsubs / fmadds order (b506).
fn fused_normal(a: Vec3, b: Vec3, c: Vec3) -> Option<Vec3> {
    let (e1, e2) = (b - a, c - a);
    let x = e1.y.mul_add(e2.z, -(e1.z * e2.y));
    let y = e1.z.mul_add(e2.x, -(e2.z * e1.x));
    let z = e2.y.mul_add(e1.x, -(e1.y * e2.x));
    let length = x.mul_add(x, z.mul_add(z, y * y)).sqrt();
    if !(length > 0.0) || !length.is_finite() {
        return None;
    }
    let inverse = 1.0 / length;
    Some(Vec3::new(x * inverse, y * inverse, z * inverse))
}
/// 82AC10E8: a merged vertex's rank is that of its smallest raw key (piece, mesh, cluster, index); `None` without
/// keys for every triangle. docs/hails-additions/39-skatepark-layouts.md#fused-vertex-order
fn fused_vertex_ranks(vertices: &[[usize; 3]], count: usize, pieces: &[usize], tags: &[FuseTag]) -> Option<Vec<u32>> {
    if tags.len() != vertices.len() {
        return None;
    }
    let mut smallest = vec![u64::MAX; count];
    for ((v, tag), &piece) in vertices.iter().zip(tags).zip(pieces) {
        let keys = tag.keys?;
        for (&id, key) in v.iter().zip(keys) {
            smallest[id] = smallest[id].min((piece as u64) << 40 | key);
        }
    }
    let mut ids: Vec<usize> = (0..count).collect();
    ids.sort_by_key(|&id| (smallest[id], id));
    let mut rank = vec![0; count];
    for (r, id) in ids.into_iter().enumerate() {
        rank[id] = r as u32;
    }
    Some(rank)
}
/// EASTL quick_sort on the first field, as 82AC4A40 / 82AC52D0 / 82AC5418 / 82AC6660 run it (unstable).
fn eastl_quick_sort(v: &mut [(u32, u32)]) {
    const LIMIT: usize = 28;
    fn insertion(v: &mut [(u32, u32)]) {
        for i in 1..v.len() {
            let temp = v[i];
            let mut j = i;
            while j > 0 && temp.0 < v[j - 1].0 {
                v[j] = v[j - 1];
                j -= 1;
            }
            v[j] = temp;
        }
    }
    fn adjust_heap(v: &mut [(u32, u32)], top: usize, size: usize, mut position: usize, value: (u32, u32)) {
        let mut child = 2 * position + 2;
        while child < size {
            if v[child].0 < v[child - 1].0 {
                child -= 1;
            }
            v[position] = v[child];
            position = child;
            child = 2 * child + 2;
        }
        if child == size {
            v[position] = v[child - 1];
            position = child - 1;
        }
        while position > top {
            let parent = (position - 1) >> 1;
            if value.0 <= v[parent].0 {
                break;
            }
            v[position] = v[parent];
            position = parent;
        }
        v[position] = value;
    }
    fn heap_sort(v: &mut [(u32, u32)]) {
        let n = v.len();
        if n >= 2 {
            let mut parent = ((n - 2) >> 1) + 1;
            while parent != 0 {
                parent -= 1;
                let value = v[parent];
                adjust_heap(v, parent, n, parent, value);
            }
        }
        let mut last = n;
        while last > 1 {
            let value = v[last - 1];
            v[last - 1] = v[0];
            adjust_heap(v, 0, last - 1, 0, value);
            last -= 1;
        }
    }
    fn introsort(v: &mut [(u32, u32)], mut depth: i32) {
        let mut last = v.len();
        while last > LIMIT && depth > 0 {
            let (a, b, c) = (v[0].0, v[last / 2].0, v[last - 1].0);
            let pivot = if a < b {
                if b < c { b } else if a < c { c } else { a }
            } else if a < c {
                a
            } else if b < c {
                c
            } else {
                b
            };
            let (mut first, mut end) = (0, last);
            loop {
                while v[first].0 < pivot {
                    first += 1;
                }
                end -= 1;
                while pivot < v[end].0 {
                    end -= 1;
                }
                if end <= first {
                    break;
                }
                v.swap(first, end);
                first += 1;
            }
            depth -= 1;
            introsort(&mut v[first..last], depth);
            last = first;
        }
        if depth == 0 {
            heap_sort(&mut v[..last]);
        }
    }
    let n = v.len();
    if n == 0 {
        return;
    }
    let bits = usize::BITS - n.leading_zeros();
    introsort(v, 2 * bits as i32 - 2);
    if n > LIMIT {
        insertion(&mut v[..LIMIT]);
        for i in LIMIT..n {
            let temp = v[i];
            let mut j = i;
            while temp.0 < v[j - 1].0 {
                v[j] = v[j - 1];
                j -= 1;
            }
            v[j] = temp;
        }
    } else {
        insertion(v);
    }
}
/// 82AC0140: docs/hails-additions/39-skatepark-layouts.md#fused-edge-bytes-and-vertex-bits
fn fused_angle_code(score: f32) -> u8 {
    let x = if score <= 1.0 { score } else { 2.0 - score };
    let theta = x.clamp(-1.0, 1.0).acos().max(f32::from_bits(0x388A_697B)) * std::f32::consts::FRAC_1_PI;
    let code = (-2.0 * theta.log2()) as i32;
    code.clamp(0, 26) as u8
}
/// 82AC2390: docs/hails-additions/39-skatepark-layouts.md#fused-edge-bytes-and-vertex-bits
fn fused_edge_byte(score: f32, paired: bool, concave_tolerance: f32) -> u8 {
    let mut code = fused_angle_code(score);
    if score < 1.0 {
        code |= 0x20;
    }
    if score > 2.0 + concave_tolerance {
        code |= 0x1A;
    }
    if !paired {
        code |= 0x9A;
    }
    code
}
/// 82874DC8 / 82874CD0: docs/hails-additions/39-skatepark-layouts.md#fused-edge-bytes-and-vertex-bits
fn fused_vertex_not_convex(
    positions: &[Vec3],
    vertices: &[[usize; 3]],
    n_ref: Vec3,
    v: usize,
    first: usize,
    rest: &[usize],
    rules: &crate::park_layout::FuseRules,
) -> bool {
    let at = positions[v];
    let others = |t: usize| {
        let k = (0..3).find(|&k| vertices[t][k] == v).unwrap_or(0);
        [vertices[t][(k + 1) % 3], vertices[t][(k + 2) % 3]]
    };
    let [a, b] = others(first);
    let (mut u1, mut u2) = ((at - positions[a]).normalize_or_zero(), (at - positions[b]).normalize_or_zero());
    for &t in rest {
        for p in others(t) {
            let w = (at - positions[p]).normalize_or_zero();
            let s = w.dot(n_ref);
            if s.abs() > rules.in_plane {
                if s < rules.below_plane {
                    return true;
                }
                continue;
            }
            let (c, q) = (u1.dot(u2), -w);
            if (u1 + u2).dot(q) >= 0.0 && q.dot(u1) >= c - rules.wedge_slack && q.dot(u2) >= c - rules.wedge_slack {
                return true;
            }
            let (w1, w2) = (u1.dot(w), u2.dot(w));
            if w2 > w1 && c > w1 {
                u2 = w;
            } else if c > w2 {
                u1 = w;
            }
        }
    }
    false
}

/// Portable triangle world: 1 mm vertex welding, reconstructed adjacency and
/// contiguous-range broadphase metadata. Shared by the playable map and by
/// static prop instances, which supply already-placed world-space triangles.
fn portable_world(
    collision: &[skate_data::skate_map::Collision],
    materials: &[skate_data::skate_map::Material],
    material: RetailContactMaterial,
    // Ranges the query meshes must not cross (one prop instance each, so each has its own collision type); `None`
    // chunks the whole list.
    splits: Option<&[std::ops::Range<usize>]>,
) -> Result<BoardWorld, String> {
    let (triangles, packed_surfaces, meshes) = portable_parts(collision, materials, material, splits, false)?;
    let metadata = QueryMetadata {
        packed_surfaces,
        meshes,
        static_edges: vec![],
        island_flags: 0,
    };
    BoardWorld::with_query_metadata(triangles, metadata).map_err(str::to_owned)
}

/// [`portable_world`]'s triangles, packed surfaces and query meshes. `native_surfaces`: a triangle with native edge
/// codes packs its own surface id (as an RWCM cluster unit does) instead of its package material's; the park pieces
/// use it, the props keep their established packing.
fn portable_parts(
    collision: &[skate_data::skate_map::Collision],
    materials: &[skate_data::skate_map::Material],
    material: RetailContactMaterial,
    splits: Option<&[std::ops::Range<usize>]>,
    native_surfaces: bool,
) -> Result<(Vec<WorldTriangle>, Vec<u16>, Vec<QueryMesh>), String> {
    portable_parts_fused(collision, materials, material, splits, native_surfaces, None)
}
/// [`portable_parts`] with an optional fuse (82872D70): docs/hails-additions/39-skatepark-layouts.md#code-reference-skate_worldrs
fn portable_parts_fused(
    collision: &[skate_data::skate_map::Collision],
    materials: &[skate_data::skate_map::Material],
    material: RetailContactMaterial,
    splits: Option<&[std::ops::Range<usize>]>,
    native_surfaces: bool,
    fuse: Option<(&crate::park_layout::FuseRules, &[FuseTag])>,
) -> Result<(Vec<WorldTriangle>, Vec<u16>, Vec<QueryMesh>), String> {
    let (rows, fused_splits) = fuse_rows(collision, materials, splits, native_surfaces, fuse)?;
    parts_from_rows(&rows, fused_splits.as_deref().or(splits), material)
}

/// The first half of [`portable_parts_fused`] (skate_vita: split out for the baked layouts): the kept triangles as
/// rows, and with a fuse the split ranges over the kept ones.
pub(crate) fn fuse_rows(
    collision: &[skate_data::skate_map::Collision],
    materials: &[skate_data::skate_map::Material],
    splits: Option<&[std::ops::Range<usize>]>,
    native_surfaces: bool,
    fuse: Option<(&crate::park_layout::FuseRules, &[FuseTag])>,
) -> Result<(Vec<FusedTriangle>, Option<Vec<std::ops::Range<usize>>>), String> {
    let points: Vec<_> = collision.iter().map(|t| t.points).collect();
    let (positions, vertices, mut flags, mut cosines, kept) = if let Some((rules, tags)) = fuse {
        // The piece of each triangle: one split range per piece (append_pieces), else one piece.
        let mut pieces = vec![0; points.len()];
        for (piece, range) in splits.unwrap_or(&[]).iter().enumerate() {
            pieces[range.clone()].fill(piece);
        }
        let (positions, vertices, flags, cosines, dropped) = fused_edge_features(&points, rules, &pieces, tags)?;
        (positions, vertices, flags, cosines, dropped.into_iter().map(|d| !d).collect::<Vec<bool>>())
    } else {
        let reconstruct = collision.iter().any(|t| t.native_edges.is_none());
        let (positions, vertices, flags, cosines) = welded_edge_features_within(&points, reconstruct, None)?;
        let kept = vec![true; vertices.len()];
        (positions, vertices, flags, cosines, kept)
    };
    let fused_splits: Option<Vec<std::ops::Range<usize>>> = (fuse.is_some()).then(|| {
        let before: Vec<usize> = std::iter::once(0).chain(kept.iter().scan(0, |n, &k| { *n += usize::from(k); Some(*n) })).collect();
        splits.map_or_else(|| vec![0..before[kept.len()]], |s| s.iter().map(|r| before[r.start]..before[r.end]).filter(|r| !r.is_empty()).collect())
    });
    let mut rows = Vec::with_capacity(vertices.len());
    for (i, source) in collision.iter().enumerate() {
        if !kept[i] {
            continue;
        }
        if let (Some(edges), None) = (source.native_edges, fuse) {
            (flags[i], cosines[i]) = decode_native_edges(edges)?;
        }
        let packed = if native_surfaces && source.native_edges.is_some() {
            source.surface as u16
        } else {
            let m = &materials[source.material as usize - 1];
            // Exact EncodeRwSurfaceId mapping from the reference native adapter.
            (m.audio | (m.physics << 7) | (m.pattern << 12)) as u16
        };
        let points = vertices[i].map(|id| positions[id].to_array());
        rows.push(FusedTriangle { points, surface: source.surface, packed, flags: flags[i], cosines: cosines[i] });
    }
    Ok((rows, fused_splits))
}

/// skate_vita: a static triangle as [`portable_parts_fused`] decides it, before it becomes a `WorldTriangle`: what a
/// baked skate.Park layout stores (crate::park_bake), so the Vita builds the fused pieces without running the fuser.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct FusedTriangle {
    pub points: [[f32; 3]; 3],
    pub surface: u32,
    pub packed: u16,
    pub flags: u32,
    pub cosines: [f32; 3],
}

/// The second half of [`portable_parts_fused`] (skate_vita: split out for the baked layouts): the triangles, their
/// packed surfaces and the query meshes over `splits` (64 triangles at most each).
pub(crate) fn parts_from_rows(
    rows: &[FusedTriangle],
    splits: Option<&[std::ops::Range<usize>]>,
    material: RetailContactMaterial,
) -> Result<(Vec<WorldTriangle>, Vec<u16>, Vec<QueryMesh>), String> {
    let (mut triangles, mut packed_surfaces) = (Vec::new(), Vec::new());
    let meshes = append_rows(rows.iter().copied(), splits, material, &mut triangles, &mut packed_surfaces)?;
    Ok((triangles, packed_surfaces, meshes))
}

/// [`parts_from_rows`] appending to a world's triangles and packed surfaces in place (skate_vita: a baked layout's
/// 149,000 MegaPark triangles are 19.6 MB, built once instead of built and copied); the meshes' ranges are the
/// appended triangles' places.
pub(crate) fn append_rows(
    rows: impl ExactSizeIterator<Item = FusedTriangle>,
    splits: Option<&[std::ops::Range<usize>]>,
    material: RetailContactMaterial,
    triangles: &mut Vec<WorldTriangle>,
    packed_surfaces: &mut Vec<u16>,
) -> Result<Vec<QueryMesh>, String> {
    let offset = triangles.len();
    triangles.reserve_exact(rows.len());
    packed_surfaces.reserve_exact(rows.len());
    for row in rows {
        packed_surfaces.push(row.packed);
        // Keep the supplied game's original static-world contact combine values.
        // The native map bridge supplies packed surfaces, not a guessed split of
        // the package's single friction scalar into static/dynamic coefficients.
        triangles.push(
            WorldTriangle::from_vertices(
                row.points.map(|p| Vector3::new(p[0], p[1], p[2])),
                material,
                row.surface,
                row.flags,
                row.cosines,
                0.,
            )
            .ok_or("Invalid SKATE collision volume")?,
        );
    }
    // Portable maps have no native cluster hierarchy. Bound contiguous ranges
    // once at load time so the existing BVH can reject distant geometry. Keep
    // triangle order and mesh identity/filter values: contact tie-breaking,
    // packed surfaces and adjacency must not change with this acceleration.
    let mut meshes = Vec::new();
    let whole = [0..triangles.len() - offset];
    for (start, end) in splits.unwrap_or(&whole).iter().flat_map(|r| (r.start..r.end).step_by(64).map(move |s| (s, (s + 64).min(r.end)))) {
        let (start, end) = (start + offset, end + offset);
        let bounds = Bounds::from_points(triangles[start..end].iter().flat_map(|t| t.triangle.vertices))
            .ok_or("SKATE collision bounds empty")?;
        meshes.push(QueryMesh {
            geometry: 0, rejection_flags: 0,
            triangle_range: start..end,
            local_to_world: RetailAffineTransform::IDENTITY,
            world_to_local: RetailAffineTransform::IDENTITY,
            local_bounds: bounds,
            matching_group: -1,
            pool: QueryPool::Ground,
        });
    }
    Ok(meshes)
}

/// One prop type's collision (skate_vita): the winding-fixed template-space triangles with
/// the per-axis scale folded in, their packed surface tags and their edge features (welded within
/// the template, base-plane edges on the floor: [`floor_base_edges`]), shared by every instance
/// with the same template range, scale and handedness. Rigid-motion invariance of the features
/// lets every bake reuse them. Upstream kept the triangles per instance and read the features back
/// from the world: a district's ~150,000 prop triangles did not fit the Vita.
#[derive(Clone)]
struct PropTemplate {
    local: std::sync::Arc<[[Vector3; 3]]>,
    tags: Vec<u32>,
    flags: Vec<u32>,
    cosines: Vec<[f32; 3]>,
}

impl PropTemplate {
    /// From a template's retail collision shape ([`PropShape`]): the native edge codes decoded
    /// per triangle (`portable_world`'s rule: a triangle without codes takes the welded features
    /// when any lacks them, the default ones otherwise); no base-plane edits (retail data).
    fn retail(local: Vec<[Vector3; 3]>, tags: Vec<u32>, edges: Vec<Option<[u8; 3]>>) -> Result<Self, String> {
        let points: Vec<[[f32; 3]; 3]> = local.iter().map(|t| t.map(|p| [p.x, p.y, p.z])).collect();
        let (_, _, mut flags, mut cosines) = welded_edge_features(&points, edges.iter().any(Option::is_none))?;
        for (i, e) in edges.iter().enumerate() {
            if let Some(e) = e {
                (flags[i], cosines[i]) = decode_native_edges(*e)?;
            }
        }
        Ok(Self { local: local.into(), tags, flags, cosines })
    }

    fn new(local: Vec<[Vector3; 3]>, tags: Vec<u32>) -> Result<Self, String> {
        let points: Vec<[[f32; 3]; 3]> = local.iter().map(|t| t.map(|p| [p.x, p.y, p.z])).collect();
        let (_, _, mut flags, mut cosines) = welded_edge_features(&points, true)?;
        floor_base_edges(&points, 0..points.len(), &mut flags, &mut cosines);
        Ok(Self { local: local.into(), tags, flags, cosines })
    }
}

/// One prop instance's share of the prop collision world: its world triangle
/// range plus the winding-fixed template-space source triangles. Rigid-motion
/// invariance of adjacency flags and edge cosines lets `rebake` skip welding.
/// skate_vita: the range is a slot, empty while the instance has none (a map prop streamed
/// out gives its slot back); the triangles and features are its template's.
#[derive(Clone)]
pub(crate) struct PropCollisionInstance {
    pub id: u32,
    /// Index of the originating MOBJ record; `usize::MAX` for an instance
    /// added mid-game ([`PropCollisionLayer::add_instance`]).
    pub object: usize,
    pub range: std::ops::Range<usize>,
    template: usize,
    local: std::sync::Arc<[[Vector3; 3]]>,
    /// The pose it was placed at (rotation basis columns, template origin).
    placement: ([[f32; 3]; 3], Vector3),
    /// The pose its slot's triangles were baked at (`None`: no slot, or parked).
    baked: LayerPose,
}

impl PropCollisionInstance {
    /// Winding-fixed template-space triangles with per-axis scale folded in.
    pub fn local_points(&self) -> &[[Vector3; 3]] {
        &self.local
    }
}

/// A pose for [`PropCollisionLayer::rebake_many`]: rotation basis columns and the template
/// origin; `None` (skate_vita) streams the instance out.
pub(crate) type LayerPose = Option<([[f32; 3]; 3], Vector3)>;

/// Static collision for spawned DMO prop instances, kept out of the map
/// collision world so dynamic instances can be rebaked independently.
pub(crate) struct PropCollisionLayer {
    /// Shared with NPC skater workers ([`Self::share`]); changed only while no worker holds it.
    world: crate::physics::WorldHandle,
    /// Shared with NPC skater workers ([`Self::share`]): copying the template triangles per NPC per tick cost ~0.5 ms.
    instances: std::sync::Arc<Vec<PropCollisionInstance>>,
    /// skate_vita: the prop types' collision, shared by their instances (and NPC workers).
    templates: std::sync::Arc<Vec<PropTemplate>>,
    /// skate_vita: free slots per template (the parked ranges of map props streamed out),
    /// taken by the template's next instance streamed in.
    free: Vec<Vec<std::ops::Range<usize>>>,
    contact_material: RetailContactMaterial,
    /// Instances added mid-game and retired since; their parked triangle
    /// ranges are reused by the next instance of the same size.
    retired: Vec<usize>,
    /// The collision type last written to each instance's meshes (`u32::MAX` = none yet), so an unchanged type
    /// costs no mesh scan.
    collision_types: Vec<u32>,
    /// skate_vita (performance): an instance took a slot or new triangles, or one was added or retired, since the
    /// last collision-type pass (`GamePhysics::step_props` runs one only then or when a type can have changed).
    collision_types_stale: bool,
}

impl PropCollisionLayer {
    /// A copy for an NPC skater worker: the same (shared) prop world with its own query scratch, and the instance list.
    pub(crate) fn share(&self) -> Self {
        Self {
            world: self.world.share(),
            instances: std::sync::Arc::clone(&self.instances),
            templates: std::sync::Arc::clone(&self.templates),
            free: self.free.clone(),
            contact_material: self.contact_material,
            retired: self.retired.clone(),
            collision_types: self.collision_types.clone(),
            collision_types_stale: self.collision_types_stale,
        }
    }

    /// The world handle for queries (its own scratch).
    pub(crate) fn world_handle_mut(&mut self) -> &mut crate::physics::WorldHandle {
        &mut self.world
    }

    pub fn world(&self) -> &BoardWorld {
        &*self.world
    }
    pub fn world_mut(&mut self) -> &mut BoardWorld {
        &mut *self.world
    }
    /// Gives an instance's query meshes the retail collision type's rejection flag (`1 << type`; DMOPool rebuild
    /// `827778C0`). Returns whether anything changed.
    pub fn set_collision_type(&mut self, instance: usize, collision_type: u32) -> bool {
        let Some(range) = self.instances.get(instance).map(|i| i.range.clone()) else { return false };
        if self.collision_types.len() < self.instances.len() {
            self.collision_types.resize(self.instances.len(), u32::MAX);
        }
        if self.collision_types[instance] == collision_type {
            return false;
        }
        self.collision_types[instance] = collision_type;
        let flags = 1u32.checked_shl(collision_type).unwrap_or(0);
        self.world_mut().set_rejection_flags(range, flags)
    }

    /// skate_vita: an instance that takes a slot (or new triangles) has meshes whose flags are not its own yet:
    /// the next [`Self::set_collision_type`] writes them.
    fn forget_collision_type(&mut self, instance: usize) {
        if let Some(t) = self.collision_types.get_mut(instance) {
            *t = u32::MAX;
        }
        self.collision_types_stale = true;
    }

    /// skate_vita: see `collision_types_stale` (the field).
    pub(crate) fn collision_types_stale(&self) -> bool {
        self.collision_types_stale
    }

    pub(crate) fn clear_collision_types_stale(&mut self) {
        self.collision_types_stale = false;
    }

    pub fn instances(&self) -> &[PropCollisionInstance] {
        &self.instances
    }

    /// skate_vita: whether `instance` has triangles in the world (a slot).
    pub fn has_slot(&self, instance: usize) -> bool {
        self.instances.get(instance).is_some_and(|i| !i.range.is_empty())
    }

    /// skate_vita: the packed surface code (tag) of `instance`'s first triangle.
    pub fn surface(&self, instance: usize) -> Option<u32> {
        self.templates.get(self.instances.get(instance)?.template)?.tags.first().copied()
    }

    /// Re-bake one instance at a new rigid pose (rotation basis columns plus
    /// translation of the template origin). Scale, if any, is baked into the
    /// local triangles at load and is not reapplied here.
    /// skate_vita: an instance without a slot takes one ([`Self::rebake_many`]).
    pub fn rebake(
        &mut self,
        instance: usize,
        basis: [[f32; 3]; 3],
        translation: Vector3,
    ) -> Result<(), String> {
        match self.rebake_many(&[(instance, Some((basis, translation)))]).into_iter().next() {
            Some((_, error)) => Err(error),
            None => Ok(()),
        }
    }

    /// [`Self::rebake`] for several instances in order with one query index rebuild
    /// (`BoardWorld::replace_triangles_batch`: identical result). An instance whose triangles
    /// do not bake is skipped and reported, as its own `rebake` call failed before.
    /// skate_vita: `None` streams an instance out: its triangles are parked at
    /// [`crate::physics::prop_dynamics::HELD_PARK`] and a map instance's slot is freed for its
    /// template (a runtime instance keeps its parked slot). An instance without a slot takes a
    /// free one of its template, else new triangles (appended together, one index rebuild).
    pub fn rebake_many(&mut self, poses: &[(usize, LayerPose)]) -> Vec<(usize, String)> {
        let park = ([[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]], crate::physics::prop_dynamics::HELD_PARK);
        let mut errors = Vec::new();
        let mut edits: Vec<(std::ops::Range<usize>, Vec<WorldTriangle>)> = Vec::new();
        let mut appends: Vec<(usize, Vec<WorldTriangle>)> = Vec::new();
        for &(instance, pose) in poses {
            let Some(entry) = self.instances.get(instance) else {
                errors.push((instance, "Unknown prop instance".to_owned()));
                continue;
            };
            if pose.is_none() && entry.range.is_empty() {
                continue;
            }
            // Its triangles are already at this pose.
            if pose.is_some() && !entry.range.is_empty() && entry.baked == pose {
                continue;
            }
            let (basis, translation) = pose.unwrap_or(park);
            let triangles = match self.bake(instance, basis, translation) {
                Ok(triangles) => triangles,
                Err(error) => {
                    errors.push((instance, error));
                    continue;
                }
            };
            let entry = &mut std::sync::Arc::make_mut(&mut self.instances)[instance];
            entry.baked = pose;
            if !entry.range.is_empty() {
                edits.push((entry.range.clone(), triangles));
                if pose.is_none() && entry.object != usize::MAX {
                    self.free[entry.template].push(std::mem::replace(&mut entry.range, 0..0));
                }
            } else if let Some(range) = self.free[entry.template].pop() {
                entry.range = range.clone();
                edits.push((range, triangles));
                self.forget_collision_type(instance);
            } else {
                appends.push((instance, triangles));
                self.forget_collision_type(instance);
            }
        }
        let refs: Vec<_> = edits.iter().map(|(range, t)| (range.clone(), t.as_slice())).collect();
        if let Err(error) = self.world.replace_triangles_batch(&refs) {
            errors.extend(poses.iter().map(|p| (p.0, error.to_owned())));
        }
        if !appends.is_empty() {
            let surfaces: Vec<Vec<u16>> = appends.iter().map(|(_, t)| t.iter().map(|t| t.tag as u16).collect()).collect();
            let parts: Vec<_> = appends.iter().zip(&surfaces).map(|((_, t), s)| (t.as_slice(), s.as_slice())).collect();
            match self.world.append_triangles_batch(&parts) {
                Ok(ranges) => {
                    let instances = std::sync::Arc::make_mut(&mut self.instances);
                    for ((instance, _), range) in appends.iter().zip(ranges) {
                        instances[*instance].range = range;
                    }
                }
                Err(error) => errors.extend(appends.iter().map(|(instance, _)| (*instance, error.to_owned()))),
            }
        }
        errors
    }

    /// skate_vita: every instance in `instances` baked where it was placed (props without a
    /// dynamic body: static collision that never streams).
    pub fn bake_placed(&mut self, instances: impl IntoIterator<Item = usize>) -> Vec<(usize, String)> {
        let poses: Vec<(usize, LayerPose)> =
            instances.into_iter().filter_map(|i| Some((i, Some(self.instances.get(i)?.placement)))).collect();
        self.rebake_many(&poses)
    }

    /// One instance's triangles at a rigid pose (tag, flags and edge cosines kept).
    fn bake(&self, instance: usize, basis: [[f32; 3]; 3], translation: Vector3) -> Result<Vec<WorldTriangle>, String> {
        let template = &self.templates[self.instances[instance].template];
        let transform = |p: Vector3| {
            Vector3::new(
                p.x * basis[0][0] + p.y * basis[1][0] + p.z * basis[2][0] + translation.x,
                p.x * basis[0][1] + p.y * basis[1][1] + p.z * basis[2][1] + translation.y,
                p.x * basis[0][2] + p.y * basis[1][2] + p.z * basis[2][2] + translation.z,
            )
        };
        let mut triangles = Vec::with_capacity(template.local.len());
        for (i, &local) in template.local.iter().enumerate() {
            triangles.push(
                WorldTriangle::from_vertices(
                    local.map(transform),
                    self.contact_material,
                    template.tags[i],
                    template.flags[i],
                    template.cosines[i],
                    0.,
                )
                .ok_or("Rebaked prop collision triangle is invalid")?,
            );
        }
        Ok(triangles)
    }

    /// A layer with no instances yet (a map without placed props, which can
    /// still get props created mid-game).
    pub fn empty(material: RetailContactMaterial) -> Result<Self, String> {
        let metadata = QueryMetadata {
            packed_surfaces: vec![],
            meshes: vec![],
            static_edges: vec![],
            island_flags: 0,
        };
        Ok(Self {
            world: BoardWorld::with_query_metadata(vec![], metadata).map_err(str::to_owned)?.into(),
            instances: std::sync::Arc::new(Vec::new()),
            templates: std::sync::Arc::new(Vec::new()),
            free: Vec::new(),
            contact_material: material,
            retired: Vec::new(),
            collision_types: Vec::new(),
            collision_types_stale: true,
        })
    }

    /// Add one instance mid-game (a released hand prop, a mod's prop): `local`
    /// are template-space triangles with scale folded in, baked at `basis` /
    /// `translation` like [`Self::rebake`]; `surface` is the packed surface code
    /// (tag) of every triangle. Edge features come from the instance's own
    /// triangles (load-time instances also pair edges with touching neighbours).
    /// A retired slot with the same triangle count is reused, so repeated
    /// spawns of one template do not grow the world. Returns the instance index.
    /// skate_vita: a runtime instance has a template of its own, replaced on reuse.
    pub fn add_instance(
        &mut self,
        id: u32,
        local: Vec<[Vector3; 3]>,
        surface: u32,
        basis: [[f32; 3]; 3],
        translation: Vector3,
    ) -> Result<usize, String> {
        let local: Vec<[Vector3; 3]> = local
            .into_iter()
            .filter(|t| {
                let [a, b, c] = t.map(|p| Vec3::new(p.x, p.y, p.z));
                (b - a).cross(c - a).length_squared() > 0.
            })
            .collect();
        if local.is_empty() {
            return Err("Prop instance has no valid triangles".into());
        }
        let tags = vec![surface; local.len()];
        let template = PropTemplate::new(local, tags)?;
        let reuse = self
            .retired
            .iter()
            .position(|&slot| self.instances[slot].range.len() == template.local.len());
        let index = match reuse {
            Some(position) => {
                let slot = self.retired.remove(position);
                let entry = &mut std::sync::Arc::make_mut(&mut self.instances)[slot];
                entry.local = template.local.clone();
                entry.baked = None;
                let index = entry.template;
                std::sync::Arc::make_mut(&mut self.templates)[index] = template;
                slot
            }
            None => {
                let local = template.local.clone();
                std::sync::Arc::make_mut(&mut self.templates).push(template);
                self.free.push(Vec::new());
                std::sync::Arc::make_mut(&mut self.instances).push(PropCollisionInstance {
                    id,
                    object: usize::MAX,
                    range: 0..0,
                    template: self.templates.len() - 1,
                    local,
                    placement: (basis, translation),
                    baked: None,
                });
                self.instances.len() - 1
            }
        };
        let entry = &mut std::sync::Arc::make_mut(&mut self.instances)[index];
        entry.id = id;
        entry.placement = (basis, translation);
        self.rebake(index, basis, translation)?;
        self.collision_types_stale = true;
        Ok(index)
    }

    /// Retire an instance added with [`Self::add_instance`] (the released prop is
    /// removed): its triangles are parked far below the world and the slot is
    /// kept for the next instance of the same size.
    pub fn retire_instance(&mut self, instance: usize, park: Vector3) -> Result<(), String> {
        if instance >= self.instances.len() || self.retired.contains(&instance) {
            return Err("Unknown or already retired prop instance".into());
        }
        if self.instances[instance].object != usize::MAX {
            return Err("Map prop instances are not retired".into());
        }
        self.rebake(instance, [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]], park)?;
        self.retired.push(instance);
        Ok(())
    }
}

/// One DMO template's retail collision: docs/hails-additions/39-skatepark-layouts.md#code-reference-skate_worldrs
#[derive(Clone, Debug, Default)]
pub(crate) struct PropShape {
    pub triangles: Vec<[[f32; 3]; 3]>,
    pub edges: Vec<Option<[u8; 3]>>,
    pub surfaces: Vec<u32>,
    /// Empty when the sidecar predates them.
    pub fuse: Vec<FuseTag>,
}
/// docs/hails-additions/39-skatepark-layouts.md#fused-vertex-order
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct FuseTag {
    /// Raw vertex key per corner within the template: mesh << 32 | cluster << 8 | index.
    pub keys: Option<[u64; 3]>,
    pub second_half: bool,
}

/// The retail collision shapes per DMO template id; empty when setup exported none (older install).
pub(crate) fn load_dmo_collision(asset_root: &std::path::Path, map_name: &str) -> std::collections::BTreeMap<String, PropShape> {
    load_collision_shapes(&asset_root.join("private").join("native-props").join(format!("{map_name}.json")))
}

/// The `collision` map of one placement sidecar (`dynamic_props.write_placements`: district props, park layouts).
pub(crate) fn load_collision_shapes(path: &std::path::Path) -> std::collections::BTreeMap<String, PropShape> {
    let Some(sidecar) = skate_data::vfs::read(path).ok().and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok()) else { return Default::default() };
    let Some(map) = sidecar.get("collision").and_then(|v| v.as_object()) else { return Default::default() };
    let mut out = std::collections::BTreeMap::new();
    for (key, v) in map {
        let point = |p: &serde_json::Value| -> Option<[f32; 3]> {
            let a = p.as_array()?;
            Some([a.first()?.as_f64()? as f32, a.get(1)?.as_f64()? as f32, a.get(2)?.as_f64()? as f32])
        };
        let triangles: Option<Vec<[[f32; 3]; 3]>> = v["triangles"].as_array().map(|ts| {
            ts.iter().filter_map(|t| { let t = t.as_array()?; Some([point(t.first()?)?, point(t.get(1)?)?, point(t.get(2)?)?]) }).collect()
        });
        let Some(triangles) = triangles.filter(|t| !t.is_empty()) else { continue };
        let edges: Vec<Option<[u8; 3]>> = v["edges"].as_array().map(|es| es.iter().map(|e| {
            let a = e.as_array()?;
            Some([a.first()?.as_u64()? as u8, a.get(1)?.as_u64()? as u8, a.get(2)?.as_u64()? as u8])
        }).collect()).unwrap_or_default();
        let surfaces: Vec<u32> = v["surfaces"].as_array().map(|ss| ss.iter().map(|x| x.as_u64().unwrap_or(0) as u32).collect()).unwrap_or_default();
        if edges.len() != triangles.len() || surfaces.len() != triangles.len() {
            warn!("SKATE_PROP_COLLISION: {}: template {key}: triangle, edge and surface counts differ; render mesh kept", path.display());
            continue;
        }
        let tag = |unit: &serde_json::Value, keys: &serde_json::Value| -> Option<FuseTag> {
            let corner = |k: &serde_json::Value| -> Option<u64> {
                let k = k.as_array()?;
                Some(k.first()?.as_u64()? << 32 | k.get(1)?.as_u64()? << 8 | k.get(2)?.as_u64()?)
            };
            let keys = keys.as_array()?;
            Some(FuseTag {
                keys: Some([corner(keys.first()?)?, corner(keys.get(1)?)?, corner(keys.get(2)?)?]),
                second_half: unit.get(0)?.as_u64()? == 2 && unit.get(1)?.as_u64()? == 1,
            })
        };
        let fuse: Vec<FuseTag> = match (v["units"].as_array(), v["vertex_keys"].as_array()) {
            (Some(u), Some(k)) if u.len() == triangles.len() && k.len() == triangles.len() => {
                u.iter().zip(k).map(|(u, k)| tag(u, k)).collect::<Option<_>>().unwrap_or_default()
            }
            _ => Vec::new(),
        };
        out.insert(key.clone(), PropShape { triangles, edges, surfaces, fuse });
    }
    out
}

pub(crate) fn build_prop_layer(
    map: &SkateMap,
    objects: &[skate_data::skate_map::StaticObject],
    material: RetailContactMaterial,
) -> Result<Option<PropCollisionLayer>, String> {
    build_prop_layer_with(map, objects, material, &Default::default())
}

/// [`build_prop_layer`] with retail collision shapes per template: a prop whose template has one collides with that
/// mesh and its native edge codes (retail's DMO physics collision); others with their render triangles (the welded
/// render mesh left seams between a bench's seat pieces as catching edges, 2026-10-15: 10 of 12 grind bails on one).
pub(crate) fn build_prop_layer_with(
    map: &SkateMap,
    objects: &[skate_data::skate_map::StaticObject],
    material: RetailContactMaterial,
    shapes: &std::collections::BTreeMap<String, PropShape>,
) -> Result<Option<PropCollisionLayer>, String> {
    let Some(mut layer) = prop_layer_unbaked(map, objects, material, shapes)? else { return Ok(None) };
    let errors = layer.bake_placed(0..layer.instances.len());
    if let Some((_, error)) = errors.into_iter().next() {
        return Err(error);
    }
    Ok(Some(layer))
}

/// Every placed object's collision in world space (its template's retail shape when it has one, else its render
/// triangles) with one contiguous range per object: the props' layer and the skate.Park pieces' share of the static
/// world (crate::park_layout), and each triangle's fuse tag (the template shape's, corners in the posed order).
/// skate_vita: each range with its object's id, not upstream's `PropCollisionInstance` (the props build their layer
/// with [prop_layer_unbaked]); the local triangles are not kept.
pub(crate) fn posed_collision(
    map: &SkateMap,
    objects: &[skate_data::skate_map::StaticObject],
    shapes: &std::collections::BTreeMap<String, PropShape>,
) -> (Vec<skate_data::skate_map::Collision>, Vec<(u32, std::ops::Range<usize>)>, Vec<FuseTag>) {
    let mut tags = Vec::new();
    let mut collision = Vec::new();
    let mut mismatched = std::collections::BTreeSet::new();
    let mut instances = Vec::new();
    for object in objects {
        let t = &object.transform;
        // Row-vector affine (v @ basis + translation), as in spawn_instances.
        let transform = |p: [f32; 3]| -> [f32; 3] {
            [
                p[0] * t[0] + p[1] * t[3] + p[2] * t[6] + t[9],
                p[0] * t[1] + p[1] * t[4] + p[2] * t[7] + t[10],
                p[0] * t[2] + p[1] * t[5] + p[2] * t[8] + t[11],
            ]
        };
        let determinant = t[0] * (t[4] * t[8] - t[5] * t[7])
            - t[1] * (t[3] * t[8] - t[5] * t[6])
            + t[2] * (t[3] * t[7] - t[4] * t[6]);
        let range =
            object.first_index as usize..(object.first_index + object.index_count) as usize;
        let start = collision.len();
        let template = object.name.split('/').next().unwrap_or_default();
        // Both meshes should be in the template's model space: a retail shape that does not line up with the render
        // mesh (the garbage bags sit 0.38 m lower: anchored elsewhere, not decoded) keeps the render mesh, logged once.
        let shape = shapes.get(template).filter(|shape| {
            let bounds = |points: &mut dyn Iterator<Item = [f32; 3]>| points.fold(([f32::MAX; 3], [f32::MIN; 3]), |(lo, hi), p| {
                (std::array::from_fn(|i| lo[i].min(p[i])), std::array::from_fn(|i| hi[i].max(p[i])))
            });
            let (rl, rh) = bounds(&mut map.geometry.indices[range.clone()].iter().map(|&i| map.geometry.vertices[i as usize].position));
            let (sl, sh) = bounds(&mut shape.triangles.iter().flatten().copied());
            let gap = (0..3).map(|i| (rl[i] - sl[i]).abs().max((rh[i] - sh[i]).abs())).fold(0.0f32, f32::max);
            if gap > 0.25 && mismatched.insert(template.to_owned()) {
                warn!("SKATE_PROP_COLLISION: template {template} retail shape bounds {sl:?}..{sh:?} differ from the render mesh {rl:?}..{rh:?} by {gap:.2} m; render mesh kept");
            }
            gap <= 0.25
        });
        if let Some(shape) = shape {
            for (i, ((tri, edges), surface)) in shape.triangles.iter().zip(&shape.edges).zip(&shape.surfaces).enumerate() {
                let mut points = *tri;
                let mut edges = *edges;
                let mut tag = shape.fuse.get(i).copied().unwrap_or_default();
                if determinant < 0. {
                    points.swap(1, 2);
                    // Edge i runs from vertex i to i + 1: swapping b and c reverses the loop, so edge 0 (a-b) becomes
                    // a-c (old edge 2) and edge 2 becomes b-a (old edge 0).
                    edges = edges.map(|e| [e[2], e[1], e[0]]);
                    tag.keys = tag.keys.map(|k| [k[0], k[2], k[1]]);
                }
                let [a, b, c] = points.map(Vec3::from_array);
                if (b - a).cross(c - a).length_squared() <= 0. {
                    continue;
                }
                collision.push(skate_data::skate_map::Collision {
                    points: points.map(transform),
                    surface: *surface,
                    material: 1,
                    native_edges: edges,
                });
                tags.resize(collision.len() - 1, FuseTag::default());
                tags.push(tag);
            }
            if collision.len() > start {
                instances.push((object.id, start..collision.len()));
            }
            continue;
        }
        for tri in map.geometry.indices[range].chunks_exact(3) {
            let mut points =
                [tri[0], tri[1], tri[2]].map(|i| map.geometry.vertices[i as usize].position);
            if determinant < 0. {
                points.swap(1, 2);
            }
            let [a, b, c] = points.map(Vec3::from_array);
            let cross = (b - a).cross(c - a);
            if cross.length_squared() <= 0. {
                continue;
            }
            let source = &map.materials
                [map.geometry.vertices[tri[0] as usize].material as usize - 1];
            // Wheels read the packed surface nibble from the triangle tag; use
            // the same EncodeRwSurfaceId mapping as static map collision.
            let surface = source.audio | (source.physics << 7) | (source.pattern << 12);
            collision.push(skate_data::skate_map::Collision {
                points: points.map(transform),
                surface,
                material: map.geometry.vertices[tri[0] as usize].material,
                native_edges: None,
            });
        }
        if collision.len() > start {
            instances.push((object.id, start..collision.len()));
        }
    }
    tags.resize(collision.len(), FuseTag::default());
    (collision, instances, tags)
}

/// skate_vita: [`build_prop_layer`] without triangles: the templates and instances, each
/// instance taking its slot when it is first baked (streamed in). Instances are placed with the
/// pose their body starts at (`PropDynamics::build_body`: the transform's rows normalised, the
/// scale folded into the template triangles).
pub(crate) fn prop_layer_unbaked(
    map: &SkateMap,
    objects: &[skate_data::skate_map::StaticObject],
    material: RetailContactMaterial,
    shapes: &std::collections::BTreeMap<String, PropShape>,
) -> Result<Option<PropCollisionLayer>, String> {
    let mut layer = PropCollisionLayer::empty(material)?;
    // Template key: the MOBJ triangle range, the per-axis scale and the handedness.
    let mut keys: HashMap<(u32, u32, [u32; 3], bool), usize> = HashMap::new();
    let mut mismatched = std::collections::BTreeSet::new();
    for (object_index, object) in objects.iter().enumerate() {
        let t = &object.transform;
        let determinant = t[0] * (t[4] * t[8] - t[5] * t[7])
            - t[1] * (t[3] * t[8] - t[5] * t[6])
            + t[2] * (t[3] * t[7] - t[4] * t[6]);
        // Row-vector rows are the world images of the local axes. Their lengths
        // are the per-axis scale; rebake applies rotation only, so local source
        // triangles carry the scale and stay exact for diagonal-scale placements.
        let scale = [
            (t[0] * t[0] + t[1] * t[1] + t[2] * t[2]).sqrt(),
            (t[3] * t[3] + t[4] * t[4] + t[5] * t[5]).sqrt(),
            (t[6] * t[6] + t[7] * t[7] + t[8] * t[8]).sqrt(),
        ];
        let key = (object.first_index, object.index_count, scale.map(f32::to_bits), determinant < 0.);
        let template = match keys.get(&key) {
            Some(&template) => template,
            None => {
                let range = object.first_index as usize..(object.first_index + object.index_count) as usize;
                let mut local = Vec::new();
                let mut tags = Vec::new();
                // Upstream (618e824): the template's retail collision shape when setup exported one that lines up
                // with the render mesh (the garbage bags' sits 0.38 m lower: anchored elsewhere, not decoded).
                let name = object.name.split('/').next().unwrap_or_default();
                let shape = shapes.get(name).filter(|shape| {
                    let bounds = |points: &mut dyn Iterator<Item = [f32; 3]>| points.fold(([f32::MAX; 3], [f32::MIN; 3]), |(lo, hi), p| {
                        (std::array::from_fn(|i| lo[i].min(p[i])), std::array::from_fn(|i| hi[i].max(p[i])))
                    });
                    let (rl, rh) = bounds(&mut map.geometry.indices[range.clone()].iter().map(|&i| map.geometry.vertices[i as usize].position));
                    let (sl, sh) = bounds(&mut shape.triangles.iter().flatten().copied());
                    let gap = (0..3).map(|i| (rl[i] - sl[i]).abs().max((rh[i] - sh[i]).abs())).fold(0.0f32, f32::max);
                    if gap > 0.25 && mismatched.insert(name.to_owned()) {
                        warn!("SKATE_PROP_COLLISION: template {name} retail shape bounds {sl:?}..{sh:?} differ from the render mesh {rl:?}..{rh:?} by {gap:.2} m; render mesh kept");
                    }
                    gap <= 0.25
                });
                if let Some(shape) = shape {
                    let mut edges = Vec::new();
                    for ((tri, e), surface) in shape.triangles.iter().zip(&shape.edges).zip(&shape.surfaces) {
                        let mut points = *tri;
                        let mut e = *e;
                        if determinant < 0. {
                            points.swap(1, 2);
                            // Edge i runs from vertex i to i + 1: swapping b and c reverses the loop, so edge 0 (a-b)
                            // becomes a-c (old edge 2) and edge 2 becomes b-a (old edge 0).
                            e = e.map(|e| [e[2], e[1], e[0]]);
                        }
                        let [a, b, c] = points.map(Vec3::from_array);
                        if (b - a).cross(c - a).length_squared() <= 0. {
                            continue;
                        }
                        local.push(points.map(|p| Vector3::new(p[0] * scale[0], p[1] * scale[1], p[2] * scale[2])));
                        tags.push(*surface);
                        edges.push(e);
                    }
                    let template = if local.is_empty() {
                        usize::MAX
                    } else {
                        std::sync::Arc::make_mut(&mut layer.templates).push(PropTemplate::retail(local, tags, edges)?);
                        layer.free.push(Vec::new());
                        layer.templates.len() - 1
                    };
                    keys.insert(key, template);
                    template
                } else {
                for tri in map.geometry.indices[range].chunks_exact(3) {
                    let mut points = [tri[0], tri[1], tri[2]].map(|i| map.geometry.vertices[i as usize].position);
                    if determinant < 0. {
                        points.swap(1, 2);
                    }
                    let [a, b, c] = points.map(Vec3::from_array);
                    if (b - a).cross(c - a).length_squared() <= 0. {
                        continue;
                    }
                    local.push(points.map(|p| Vector3::new(p[0] * scale[0], p[1] * scale[1], p[2] * scale[2])));
                    let source = &map.materials[map.geometry.vertices[tri[0] as usize].material as usize - 1];
                    // Wheels read the packed surface nibble from the triangle tag; use
                    // the same EncodeRwSurfaceId mapping as static map collision.
                    tags.push(source.audio | (source.physics << 7) | (source.pattern << 12));
                }
                // A template whose scaled triangles all degenerate has no collision.
                let template = if local.is_empty() {
                    usize::MAX
                } else {
                    std::sync::Arc::make_mut(&mut layer.templates).push(PropTemplate::new(local, tags)?);
                    layer.free.push(Vec::new());
                    layer.templates.len() - 1
                };
                keys.insert(key, template);
                template
                }
            }
        };
        if template == usize::MAX {
            continue;
        }
        let basis = [0, 1, 2].map(|row| [0, 1, 2].map(|c| t[row * 3 + c] / scale[row]));
        std::sync::Arc::make_mut(&mut layer.instances).push(PropCollisionInstance {
            id: object.id,
            object: object_index,
            range: 0..0,
            template,
            local: layer.templates[template].local.clone(),
            placement: (basis, Vector3::new(t[9], t[10], t[11])),
            baked: None,
        });
    }
    if layer.instances.is_empty() {
        return Ok(None);
    }
    Ok(Some(layer))
}

/// Retail per-type DMO data for the district's props: setup writes the map
/// from template id to the type's vault record (`types` in
/// `private/native-props/<map>.json`, from the template's EB000D +120); the
/// values come from the installation's stock vault
/// (`livingworld_dynamicobject_characteristics`,
/// [`crate::physics::prop_dynamics::dmo_type_blocks`]). `None` when the
/// sidecar has no type map (setup older than the type data): the props keep
/// their authored material (NOT RETAIL YET there; re-run setup group maps).
/// The authored grab splines per DMO template id (`native-props/<map>.json` `grab_splines`, doc 26i "Move Object
/// step 1"); `None` when the export has none.
pub(crate) fn load_dmo_grab_splines(asset_root: &std::path::Path, map_name: &str) -> Option<std::collections::BTreeMap<String, Vec<crate::living_world::vehicles::CarGrabSpline>>> {
    load_grab_splines(&asset_root.join("private").join("native-props").join(format!("{map_name}.json")))
}

/// The grab_splines map of one placement sidecar.
pub(crate) fn load_grab_splines(path: &std::path::Path) -> Option<std::collections::BTreeMap<String, Vec<crate::living_world::vehicles::CarGrabSpline>>> {
    let sidecar: serde_json::Value = serde_json::from_slice(&skate_data::vfs::read(path).ok()?).ok()?;
    let map = sidecar.get("grab_splines")?.as_object()?;
    let splines: std::collections::BTreeMap<_, _> = map.iter().map(|(k, v)| (k.clone(), crate::living_world::vehicles::parse_grab_splines(v))).filter(|(_, v): &(String, Vec<_>)| !v.is_empty()).collect();
    (!splines.is_empty()).then_some(splines)
}

/// The grind rails per DMO template id (`native-props/<map>.json` `grind_splines`, template space; retail setup
/// 82C535C0, doc 26 "Grinding on props"); `None` when the export has none. A malformed list is an error.
pub(crate) fn load_dmo_grind_rails(asset_root: &std::path::Path, map_name: &str) -> Result<Option<std::collections::BTreeMap<String, Vec<crate::grind_world::props::PropRail>>>, String> {
    load_grind_rails(&asset_root.join("private").join("native-props").join(format!("{map_name}.json")))
}

/// The `grind_splines` map of one placement sidecar, per template id (template space).
pub(crate) fn load_grind_rails(path: &std::path::Path) -> Result<Option<std::collections::BTreeMap<String, Vec<crate::grind_world::props::PropRail>>>, String> {
    let Ok(bytes) = skate_data::vfs::read(path) else { return Ok(None) };
    let sidecar: serde_json::Value = serde_json::from_slice(&bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    let Some(map) = sidecar.get("grind_splines").and_then(|v| v.as_object()) else { return Ok(None) };
    let rails = map.iter().map(|(k, v)| Ok((k.clone(), crate::grind_world::props::parse_rails(v).map_err(|e| format!("{}: {k}: {e}", path.display()))?)))
        .collect::<Result<std::collections::BTreeMap<_, _>, String>>()?;
    Ok((!rails.is_empty()).then_some(rails))
}

pub(crate) fn load_dmo_types(
    asset_root: &std::path::Path,
    map_name: &str,
) -> Option<std::collections::BTreeMap<String, crate::physics::prop_dynamics::DmoType>> {
    load_types(asset_root, &asset_root.join("private").join("native-props").join(format!("{map_name}.json")), map_name)
}

/// The 	ypes map of one placement sidecar, resolved against the installation's stock vault.
pub(crate) fn load_types(
    asset_root: &std::path::Path,
    path: &std::path::Path,
    map_name: &str,
) -> Option<std::collections::BTreeMap<String, crate::physics::prop_dynamics::DmoType>> {
    use crate::physics::prop_dynamics::{dmo_type_blocks, DmoType, DMO_TYPE_CLASS};
    let sidecar: serde_json::Value = serde_json::from_slice(&skate_data::vfs::read(path).ok()?).ok()?;
    let map = sidecar.get("types")?.as_object()?;
    if map.is_empty() {
        return None;
    }
    let collections = match skate_data::collections::Collections::load(asset_root) {
        Ok(c) => c,
        Err(error) => {
            warn!("SKATE_PROP_TYPES: {map_name}: {error}");
            return None;
        }
    };
    let names: std::collections::HashMap<String, &str> = collections
        .entries()
        .iter()
        .filter(|c| c.class_name == DMO_TYPE_CLASS)
        .map(|c| (skate_data::attrib_hash::numeric_name(&c.key), c.key.as_str()))
        .collect();
    let mut types = std::collections::BTreeMap::new();
    for (template, record) in map {
        let Some(record) = record.as_str() else { continue };
        let id = skate_data::attrib_hash::numeric_name(record);
        let key = names.get(&id).map_or(record, |name| *name);
        match dmo_type_blocks(&collections, key) {
            Ok(blocks) => {
                let priority = crate::physics::prop_dynamics::dmo_type_priority(&collections, key).unwrap_or_else(|error| {
                    warn!("SKATE_PROP_TYPES: {map_name} template {template}: priority: {error}");
                    Some(100)
                });
                types.insert(template.clone(), DmoType { key: key.to_owned(), blocks, priority });
            }
            Err(error) => warn!("SKATE_PROP_TYPES: {map_name} template {template}: {error}"),
        }
    }
    Some(types)
}

/// Load the district's prop package and build its collision layer plus the
/// dynamic bodies for every instance. The package is a presentation
/// supplement: missing or invalid files leave props uncollidable rather than
/// failing the map, matching the render path.
/// skate_vita: each placed prop's template range (`first_index`, `index_count` of its MOBJ
/// record), by id: what the renderer draws for it. From the same package the props load from.
pub(crate) fn prop_template_ranges(asset_root: &std::path::Path, map_name: &str) -> std::collections::HashMap<u32, (u32, u32)> {
    let path = props_package(&asset_root.join("private").join("native-props"), map_name);
    let Ok(data) = skate_data::vfs::read(&path) else { return Default::default() };
    let map = if skate_data::prop_package::is_package(&data) { skate_data::prop_package::decode(&data) } else { SkateMap::parse_render_only(&data) };
    let Ok(map) = map else { return Default::default() };
    map.extensions
        .iter()
        .filter(|e| e.tag == *b"MOBJ")
        .filter_map(|e| skate_data::skate_map::parse_static_objects(&map, e).ok())
        .flatten()
        .map(|o| (o.id, (o.first_index, o.index_count)))
        .collect()
}

pub(crate) fn load_prop_layer(
    asset_root: &std::path::Path,
    map_name: &str,
    material: RetailContactMaterial,
    simulation: skate_core::physics::rigid_body::RetailSimulationStep,
) -> Option<(PropCollisionLayer, crate::physics::prop_dynamics::PropDynamics)> {
    let directory = asset_root.join("private").join("native-props");
    load_prop_layer_at(asset_root, &props_package(&directory, map_name), &directory.join(format!("{map_name}.json")), map_name, material, simulation)
}

/// skate_vita: the package `<name>.props` (`skate_data::prop_package`: the `.skate` without its texture pixels) when
/// baked, else `<name>.skate`.
fn props_package(directory: &std::path::Path, name: &str) -> std::path::PathBuf {
    let package = directory.join(format!("{name}.props"));
    if skate_data::vfs::is_file(&package) { package } else { directory.join(format!("{name}.skate")) }
}

/// The prop package and sidecar a loaded map uses: its skate.Park layout's movable pieces
/// (`park-layouts/<layout>.props.skate`, research b435: ordinary DMOs) when it carries a layout that has them, else the
/// district's own (`native-props/<map>.skate`). No park stream has district props (2026-10-21), so the layout's
/// package stands in for them; a stream with both would need the two merged.
pub(crate) fn prop_files(asset_root: &std::path::Path, map: &SkateMap) -> (std::path::PathBuf, std::path::PathBuf, String) {
    if let Some(layout) = crate::park_layout::layout_of(map) {
        let directory = crate::park_layout::directory(asset_root);
        let skate = props_package(&directory, &format!("{layout}.props"));
        if skate_data::vfs::is_file(&skate) {
            return (skate, directory.join(format!("{layout}.props.json")), layout.to_owned());
        }
    }
    let directory = asset_root.join("private").join("native-props");
    (props_package(&directory, &map.name), directory.join(format!("{}.json", map.name)), map.name.clone())
}

/// [load_prop_layer] for a loaded map ([prop_files]).
pub(crate) fn load_map_prop_layer(
    asset_root: &std::path::Path,
    map: &SkateMap,
    material: RetailContactMaterial,
    simulation: skate_core::physics::rigid_body::RetailSimulationStep,
) -> Option<(PropCollisionLayer, crate::physics::prop_dynamics::PropDynamics)> {
    let (skate, json, name) = prop_files(asset_root, map);
    load_prop_layer_at(asset_root, &skate, &json, &name, material, simulation)
}

fn load_prop_layer_at(
    asset_root: &std::path::Path,
    path: &std::path::Path,
    sidecar: &std::path::Path,
    map_name: &str,
    material: RetailContactMaterial,
    simulation: skate_core::physics::rigid_body::RetailSimulationStep,
) -> Option<(PropCollisionLayer, crate::physics::prop_dynamics::PropDynamics)> {
    if !skate_data::vfs::is_file(path) {
        return None;
    }
    let loaded = skate_data::vfs::read(path).map_err(|e| e.to_string()).and_then(|data| {
        if skate_data::prop_package::is_package(&data) {
            skate_data::prop_package::decode(&data)
        } else {
            SkateMap::parse_render_only(&data)
        }
    });
    let map = match loaded {
        Ok(map) => map,
        Err(error) => {
            warn!("SKATE_PROP_COLLISION: {}: {error}", path.display());
            return None;
        }
    };
    let mut objects = Vec::new();
    for extension in map.extensions.iter().filter(|e| e.tag == *b"MOBJ") {
        match skate_data::skate_map::parse_static_objects(&map, extension) {
            Ok(parsed) => objects.extend(parsed),
            Err(error) => {
                warn!("SKATE_PROP_COLLISION: {}: {error}", path.display());
                return None;
            }
        }
    }
    let shapes = load_collision_shapes(sidecar);
    let retail_shaped = objects.iter().filter(|o| shapes.contains_key(o.name.split('/').next().unwrap_or_default())).count();
    info!("SKATE_PROP_COLLISION: {map_name} retail collision shapes {} templates, {retail_shaped}/{} props", shapes.len(), objects.len());
    // skate_vita: no triangles yet; the props take collision slots as they stream in.
    match prop_layer_unbaked(&map, &objects, material, &shapes) {
        Ok(Some(mut layer)) => {
            info!(
                "SKATE_PROP_COLLISION: {map_name} instances={} templates={}",
                objects.len(),
                layer.templates.len()
            );
            let mut dynamics = crate::physics::prop_dynamics::PropDynamics::new(
                &objects,
                layer.instances(),
                simulation,
            );
            // Instances without a body never stream: static collision from the start.
            let bodiless: Vec<usize> =
                (0..layer.instances().len()).filter(|&i| dynamics.instance_of(layer.instances()[i].id) != Some(i)).collect();
            for (instance, error) in layer.bake_placed(bodiless) {
                warn!("SKATE_PROP_COLLISION: {map_name} instance {instance}: {error}");
            }
            if let Some(types) = load_types(asset_root, sidecar, map_name) {
                let typed = dynamics.set_type_data(&types);
                info!("SKATE_PROP_TYPES: {map_name} types={} props_with_type_data={typed}/{}", types.len(), objects.len());
            }
            if let Some(splines) = load_grab_splines(sidecar) {
                let with = dynamics.set_grab_splines(&splines);
                info!("SKATE_PROP_GRAB: {map_name} templates={} props_with_grab_splines={with}/{}", splines.len(), objects.len());
            }
            match load_grind_rails(sidecar) {
                Ok(Some(rails)) => {
                    let templates = rails.len();
                    let with = dynamics.set_grind_rails(rails);
                    info!("SKATE_PROP_RAILS: {map_name} templates={templates} props_with_rails={with}/{}", objects.len());
                }
                Ok(None) => {}
                Err(error) => warn!("SKATE_PROP_RAILS: {map_name} not loaded: {error}"),
            }
            Some((layer, dynamics))
        }
        Ok(None) => None,
        Err(error) => {
            warn!("SKATE_PROP_COLLISION: {}: {error}", path.display());
            None
        }
    }
}

// skate_vita: upstream's tests here build the PC renderer's Bevy world (map_render, retail_render);
// they do not compile without it. The prop collision they also cover is tested in prop_dynamics.
#[cfg(any())]
mod tests {
    use super::*;

    /// Every prop type in a set-up map's type map resolves to retail type
    /// data from the stock vault, and the values are the retail record's
    /// (the default record: restitution 0.5, free pair 0.8 / 0.6).
    /// `SKATE3_ASSET_ROOT` = set-up assets (setup group maps run with the
    /// type map), `SKATE3_PROP_MAP` = map name (default DownTown).
    #[test]
    #[ignore = "Requires private installed assets"]
    fn installed_map_props_resolve_retail_type_data() {
        let root = std::path::PathBuf::from(std::env::var("SKATE3_ASSET_ROOT").expect("SKATE3_ASSET_ROOT"));
        let map = std::env::var("SKATE3_PROP_MAP").unwrap_or_else(|_| "DownTown".into());
        let sidecar: serde_json::Value = serde_json::from_slice(
            &std::fs::read(root.join("private/native-props").join(format!("{map}.json"))).unwrap(),
        )
        .unwrap();
        let expected = sidecar["types"].as_object().expect("type map (re-run setup group maps)").len();
        let types = load_dmo_types(&root, &map).expect("type data");
        assert!(expected > 0);
        assert_eq!(types.len(), expected, "every template's record resolved");
        for (template, t) in &types {
            let b = t.blocks;
            for v in [b.restitution.unwrap(), b.free.unwrap()[0], b.free.unwrap()[1]] {
                assert!((0.0..=1.0).contains(&v), "{template} {} {v}", t.key);
            }
            assert_eq!(b.upright_pair, b.record_272);
            for v in [b.linear_drag.unwrap(), b.angular_drag.unwrap()] {
                assert!((0.0..=1.0).contains(&v), "{template} {} drag {v}", t.key);
            }
            // Body data of 82C4E568: mass +304, caps +292 / +296, inertia box +16 / +32.
            assert!(b.mass.unwrap() > 0.0, "{template} {} mass", t.key);
            assert!(b.maximum_linear_velocity.unwrap() > 0.0 && b.maximum_angular_velocity.unwrap() > 0.0);
            assert!(b.inertia_scale.is_some() && b.inertia_offset.is_some());
        }
        let collections = skate_data::collections::Collections::load(&root).unwrap();
        let default = crate::physics::prop_dynamics::dmo_type_blocks(&collections, "default").unwrap();
        assert_eq!((default.restitution, default.free, default.upright_pair), (Some(0.5), Some([0.8, 0.6]), Some(false)));
        assert_eq!((default.linear_drag, default.angular_drag), (Some(0.0), Some(0.0)));
        assert_eq!((default.mass, default.maximum_linear_velocity, default.maximum_angular_velocity), (Some(100.0), Some(100.0), Some(100.0)));
        assert_eq!((default.inertia_scale, default.inertia_offset), (Some([1.2, 1.2, 1.2]), Some([0.0; 3])));
        for (template, t) in &types {
            let b = t.blocks;
            eprintln!("{template} {} mass={:?} caps={:?}/{:?} box={:?}+{:?}", t.key, b.mass, b.maximum_linear_velocity, b.maximum_angular_velocity, b.inertia_scale, b.inertia_offset);
        }
        let mut keys: Vec<_> = types.values().map(|t| t.key.as_str()).collect();
        keys.dedup();
        eprintln!("{map}: {} templates, types {:?}", types.len(), keys);
    }

    /// Measures the static draw budget on a real installed map. This is the
    /// check that the whole architecture exists to pass, so it reports the class
    /// mix rather than only the total: if the budget is ever missed, the mix says
    /// whether to subdivide less or to stop splitting on a material property.
    ///
    /// `SKATE_BUDGET_MAP` names the `.skate` file; `SKATE_TRANSITION_TEST_ASSETS`
    /// gives the asset root.
    #[test]
    #[ignore = "Requires private installed assets; CPU only, no GPU or window"]
    fn static_draw_budget_holds_on_an_installed_map() {
        let root = std::path::PathBuf::from(
            std::env::var("SKATE_TRANSITION_TEST_ASSETS")
                .expect("SKATE_TRANSITION_TEST_ASSETS"),
        );
        let path = std::path::PathBuf::from(
            std::env::var("SKATE_BUDGET_MAP").expect("SKATE_BUDGET_MAP"),
        );
        let map = skate_data::skate_map::SkateMap::load(&path).unwrap();
        let mut world = World::new();
        world.init_resource::<Assets<Mesh>>();
        world.init_resource::<Assets<Image>>();
        world.init_resource::<Assets<StandardMaterial>>();
        world.init_resource::<Assets<crate::retail_render::WorldMaterial>>();
        world.init_resource::<Assets<bevy::render::storage::ShaderStorageBuffer>>();
        let mut scene = crate::map_render::PreparedScene::new(&world);
        let started = std::time::Instant::now();
        scene.prepare(Some(&map), &root);
        let elapsed = started.elapsed();
        let stats = scene.stats;
        eprintln!(
            "SKATE_BUDGET map={} draws={} triangles={} slabs={} materials={} prepare_ms={}",
            path.file_stem().unwrap_or_default().to_string_lossy(),
            stats.draws,
            stats.triangles,
            stats.slabs,
            map.materials.len(),
            elapsed.as_millis()
        );
        assert!(
            stats.draws < 300,
            "static draw budget is under 300; got {} draws",
            stats.draws
        );
        assert!(stats.triangles > 0, "map produced no geometry");
    }

    fn triangle(centroid: Vec3) -> Triangle {
        Triangle { indices: [0, 1, 2], centroid, slab: 0, class: RenderClass::Opaque }
    }

    #[test]
    fn partition_respects_the_leaf_budget() {
        let triangles: Vec<Triangle> = (0..LEAF_TRIANGLE_BUDGET * 3 + 7)
            .map(|i| triangle(Vec3::new(i as f32, 0., 0.)))
            .collect();
        let mut members: Vec<usize> = (0..triangles.len()).collect();
        let leaves = partition(&mut members, &triangles);
        assert!(leaves.len() >= 4, "expected several leaves, got {}", leaves.len());
        for leaf in &leaves {
            assert!(leaf.len() <= LEAF_TRIANGLE_BUDGET, "leaf of {} triangles", leaf.len());
        }
    }

    #[test]
    fn partition_preserves_every_triangle_exactly_once() {
        let triangles: Vec<Triangle> = (0..5_000)
            .map(|i| triangle(Vec3::new((i % 71) as f32, (i % 13) as f32, i as f32)))
            .collect();
        let mut members: Vec<usize> = (0..triangles.len()).collect();
        let mut seen: Vec<usize> = partition(&mut members, &triangles)
            .into_iter()
            .flatten()
            .collect();
        seen.sort_unstable();
        assert_eq!(seen, (0..triangles.len()).collect::<Vec<_>>());
    }

    #[test]
    fn a_single_leaf_is_not_split() {
        let triangles: Vec<Triangle> = (0..64).map(|i| triangle(Vec3::splat(i as f32))).collect();
        let mut members: Vec<usize> = (0..triangles.len()).collect();
        assert_eq!(partition(&mut members, &triangles).len(), 1);
    }

    fn material() -> RetailContactMaterial {
        RetailContactMaterial {
            static_friction: 0.,
            dynamic_friction: 0.,
            restitution: 1.,
        }
    }

    /// A 4x1x1 slab template (x ±2, y 0..1, z ±0.5) with two placed instances:
    /// one translated, one rotated 90° about Y and translated.
    fn prop_fixture() -> (SkateMap, Vec<skate_data::skate_map::StaticObject>) {
        let corners = [
            [-2., 0., -0.5], [2., 0., -0.5], [2., 0., 0.5], [-2., 0., 0.5],
            [-2., 1., -0.5], [2., 1., -0.5], [2., 1., 0.5], [-2., 1., 0.5],
        ];
        let faces = [
            [4, 7, 6], [4, 6, 5], // +Y top
            [0, 1, 2], [0, 2, 3], // -Y bottom
            [1, 5, 6], [1, 6, 2], // +X
            [0, 7, 4], [0, 3, 7], // -X
            [3, 2, 6], [3, 6, 7], // +Z
            [0, 5, 1], [0, 4, 5], // -Z
        ];
        let vertex = |position| skate_data::skate_map::Vertex {
            position,
            normal: [0., 1., 0.],
            uv: [0.; 2],
            lightmap_uv: [0.; 2],
            material: 1,
            decal_uv: None,
            tangent_frame: None,
        };
        let map = SkateMap {
            version: 14,
            name: "props".into(),
            spawn: [0.; 3],
            heading: 0.,
            environment: vec![0.; 45],
            materials: vec![skate_data::skate_map::Material {
                name: "prop".into(),
                flags: 0,
                friction: 0.5,
                restitution: 0.1,
                color: [1.; 3],
                roughness: 0.5,
                emissive: 0.,
                textures: [0; 5],
                indirect_strength: 0.,
                alpha_mode: 0,
                alpha_cutoff: 0.5,
                audio: 3,
                physics: 1,
                pattern: 0,
                depth_layer: None,
                retail_definition: None,
            }],
            textures: vec![],
            geometry: skate_data::skate_map::Geometry {
                vertices: corners.into_iter().map(vertex).collect(),
                indices: faces.into_iter().flatten().collect(),
                collision: vec![],
            },
            rails: vec![],
            doors: vec![],
            lights: vec![],
            routes: vec![],
            extensions: vec![],
        };
        let object = |id, transform| skate_data::skate_map::StaticObject {
            id,
            name: format!("template/prop{id}"),
            transform,
            first_index: 0,
            index_count: 36,
            first_collision: 0,
            collision_count: 0,
            rails: vec![],
            physics: Default::default(),
        };
        let objects = vec![
            object(7, [1., 0., 0., 0., 1., 0., 0., 0., 1., 10., 5., 0.]),
            // 90° about Y (row-vector): x' = -z, z' = x; then z -= 10.
            object(8, [0., 0., 1., 0., 1., 0., -1., 0., 0., 0., 0., -10.]),
        ];
        (map, objects)
    }

    #[test]
    fn prop_instances_collide_as_placed_static_triangles() {
        let (map, objects) = prop_fixture();
        let layer = build_prop_layer(&map, &objects, material()).unwrap().unwrap();
        let mut world = layer.world;
        assert_eq!(world.triangles().len(), 24);
        // Translated instance: top face at y=6 between x 8..12.
        let hit = world
            .query_thin_line(Vector3::new(10., 10., 0.), Vector3::new(10., 0., 0.))
            .unwrap()
            .unwrap();
        assert!((hit.geometry.position.y - 6.).abs() < 1e-4);
        assert!(hit.geometry.normal.y > 0.99);
        // The packed surface tag follows the EncodeRwSurfaceId mapping.
        assert_eq!(hit.tag, 3 | (1 << 7));
        // Rotated instance: slab now spans z -12..-8, x ±0.5.
        let hit = world
            .query_thin_line(Vector3::new(0., 5., -10.), Vector3::new(0., -1., -10.))
            .unwrap()
            .unwrap();
        assert!((hit.geometry.position.y - 1.).abs() < 1e-4);
        // Outside the rotated footprint: unrotated, this line would hit.
        assert!(world
            .query_thin_line(Vector3::new(1.5, 5., -10.), Vector3::new(1.5, -1., -10.))
            .unwrap()
            .is_none());
        // A wheel sphere resting on the translated instance reports a contact.
        let (query, retention) = crate::physics::ground::query_settings();
        let volumes = [skate_core::physics::board_world::BoardWorldVolume {
            collision_group: 4,
            body: skate_core::physics::board_step::CollisionBody::Board(
                skate_core::physics::board::BodyId::Deck,
            ),
            primitive: skate_core::physics::world_contact::ContactPrimitive::Sphere(
                skate_core::physics::collision::Sphere {
                    center: Vector3::new(10., 6.05, 0.),
                    radius: 0.1,
                },
            ),
            motion: skate_core::physics::board_world::VolumeMotion { linear_velocity: Vector3::ZERO, ..Default::default() },
            material: material(),
        }];
        let contacts = world.query_primitives(&volumes, query, retention);
        assert!(!contacts.is_empty());
        assert!(contacts.iter().all(|c| c.contact.normal.y > 0.9));
    }

    #[test]
    fn prop_instances_spawn_one_placed_root_per_record_sharing_template_meshes() {
        use crate::map_render::{MapEntity, StagedAssets};
        let (map, objects) = prop_fixture();
        let mut world = World::new();
        world.init_resource::<Assets<Mesh>>();
        world.init_resource::<Assets<Image>>();
        world.init_resource::<Assets<WorldMaterial>>();
        world.init_resource::<Assets<bevy::render::storage::ShaderStorageBuffer>>();
        let mut meshes = StagedAssets::<Mesh>::new(&world);
        let mut images = StagedAssets::<Image>::new(&world);
        let mut materials = StagedAssets::<WorldMaterial>::new(&world);
        let mut buffers = StagedAssets::<bevy::render::storage::ShaderStorageBuffer>::new(&world);
        let mut commands = SceneCommands::default();
        let spawned = spawn_instances(
            &map,
            &objects,
            &crate::retail_render::MaterialTuning::default(),
            &crate::retail_sky::SkyEnvironment::default(),
            &mut commands,
            &mut meshes,
            &mut materials,
            &mut images,
            &mut buffers,
            None,
        );
        assert_eq!(spawned, 2);
        meshes.publish(&mut world);
        images.publish(&mut world);
        materials.publish(&mut world);
        buffers.publish(&mut world);
        commands.apply(&mut world);
        // Both records place the same range: one template mesh, built once.
        assert_eq!(world.resource::<Assets<Mesh>>().len(), 1);
        let mut roots: Vec<(u32, Transform, Vec<Entity>)> = world
            .query::<(&PropInstance, &Transform, &Children)>()
            .iter(&world)
            .map(|(prop, transform, children)| (prop.id, *transform, children.to_vec()))
            .collect();
        roots.sort_by_key(|(id, ..)| *id);
        assert_eq!(roots.iter().map(|(id, ..)| *id).collect::<Vec<_>>(), [7, 8]);
        // Only the roots carry MapEntity; children go with their parent.
        assert_eq!(world.query_filtered::<Entity, With<MapEntity>>().iter(&world).count(), 2);
        assert!((roots[0].1.translation - Vec3::new(10., 5., 0.)).length() < 1e-5);
        // Row-vector basis row 0 is (0, 0, 1): template +X maps to world +Z.
        let tip = roots[1].1.transform_point(Vec3::X);
        assert!((tip - Vec3::new(0., 0., -9.)).length() < 1e-5, "{tip}");
        let mesh = |entity: Entity| world.get::<Mesh3d>(entity).unwrap().0.id();
        assert_eq!(roots[0].2.len(), 1);
        assert_eq!(mesh(roots[0].2[0]), mesh(roots[1].2[0]));
    }
}

/// skate_vita: upstream's skate.Park fuser tests (upstream 0947dea5), compiled on their own (the module above builds the
/// PC renderer's world).
#[cfg(test)]
mod fuser_tests {
    use super::*;

    fn material() -> RetailContactMaterial {
        RetailContactMaterial {
            static_friction: 0.,
            dynamic_friction: 0.,
            restitution: 1.,
        }
    }

    /// Two flat 1 m plates (two triangles each, normal up) meeting at x = 1 with piece B moved by `offset`.
    fn plates(offset: [f32; 3]) -> Vec<[[f32; 3]; 3]> {
        let a = |x: f32, z: f32| [x, 0.0, z];
        let b = |x: f32, z: f32| [x + offset[0], offset[1], z + offset[2]];
        vec![
            [a(0., 0.), a(0., 1.), a(1., 0.)],
            [a(1., 0.), a(0., 1.), a(1., 1.)],
            [b(1., 0.), b(1., 1.), b(2., 0.)],
            [b(2., 0.), b(1., 1.), b(2., 1.)],
        ]
    }

    /// b477 / b478: flush park pieces (laptop session 2026-10-21: 1 to 2 mm apart) fuse, so the seam edges pair and
    /// lose their convex bit; the 1 mm map path leaves them open; a real 5 cm gap stays open when fused.
    #[test]
    fn fused_park_pieces_pair_their_flush_seams() {
        let seam = |features: &(Vec<Vec3>, Vec<[usize; 3]>, Vec<u32>, Vec<[f32; 3]>)| {
            let (_, _, flags, cosines) = features;
            // Triangle 1 edge 2 (x = 1, z 1 -> 0) and triangle 2 edge 0 (x = 1, z 0 -> 1).
            [(1, 2), (2, 0)].map(|(t, e)| (flags[t] & (0x20 << e) != 0, cosines[t][e]))
        };
        let fuse = |points: &[[[f32; 3]; 3]]| {
            let (p, v, f, c, _) = fused_edge_features(points, &crate::park_layout::FuseRules::default(), &[0, 0, 1, 1], &[]).unwrap();
            (p, v, f, c)
        };
        let flush = plates([0.0015, 0.001, 0.0]);
        let fused = fuse(&flush);
        // Normals come from the merged corners (retail), so plate B's 1.5 mm pull tilts it by a hair.
        assert!(seam(&fused).iter().all(|&(convex, cosine)| !convex && (cosine - 1.0).abs() < 1e-5), "paired: flat, not convex");
        let mapped = welded_edge_features_within(&flush, true, None).unwrap();
        assert!(seam(&mapped).iter().all(|&(convex, _)| convex), "the 1 mm grid keeps them open");
        // A real gap stays unpaired: retail writes such an edge unmatched with cosine 1, not convex (82AC2390, b487).
        let gap = fuse(&plates([0.05, 0.0, 0.0]));
        assert!(seam(&gap).iter().all(|&(convex, cosine)| !convex && cosine == 1.0), "unpaired: not convex");
        // Deterministic: the same input gives the same features.
        assert_eq!(fused, fuse(&flush));
    }

    /// An IgnoreFuser piece (retail flag 2, b482 / b483) stays out of the fused set, and so out of the skater's world
    /// (the skater's ground query reads only the fused GroundPool object, b484).
    #[test]
    fn ignore_fuser_pieces_stay_out_of_the_fused_set() {
        let flush = plates([0.0015, 0.001, 0.0]);
        let collision: Vec<_> = flush.iter().map(|&points| skate_data::skate_map::Collision {
            // Template pieces carry native codes; 0x60 is a retail open border (convex, b477).
            points, surface: 0, material: 1, native_edges: Some([0x60; 3]),
        }).collect();
        let pieces = |unfused: Vec<bool>| crate::park_layout::Pieces {
            collision: collision.clone(), ranges: vec![0..2, 2..4], materials: vec![], rails: vec![],
            fuse: crate::park_layout::FuseRules::default(), unfused, tags: vec![],
        };
        let flagged = pieces(vec![false, true]);
        let ((fused, fused_ranges, _), (unfused, unfused_ranges, _)) = split_unfused(&flagged);
        assert_eq!((fused.len(), fused_ranges, unfused.len(), unfused_ranges), (2, vec![0..2], 2, vec![0..2]));
        assert_eq!(fused[..], collision[..2]);
        assert_eq!(unfused[..], collision[2..]);
        let none = pieces(vec![false, false]);
        let ((all, ranges, _), (rest, _, _)) = split_unfused(&none);
        assert_eq!((all.len(), ranges, rest.len()), (4, vec![0..2, 2..4], 0));
        // Fewer flags than ranges (an older sidecar): the rest fuse.
        let ((all, _, _), _) = split_unfused(&pieces(vec![]));
        assert_eq!(all.len(), 4);
        // Only the fused set reaches the skater's world, one query range per piece.
        let build = |p: &crate::park_layout::Pieces| {
            let (mut triangles, mut surfaces, mut meshes) = (vec![], vec![], vec![]);
            append_pieces(&mut triangles, &mut surfaces, &mut meshes, p, material()).unwrap();
            (triangles.len(), meshes.iter().map(|m| m.triangle_range.clone()).collect::<Vec<_>>())
        };
        assert_eq!(build(&flagged), (2, vec![0..2]));
        assert_eq!(build(&none), (4, vec![0..2, 2..4]));
        assert_eq!(build(&flagged), build(&flagged), "deterministic");
        // The unfused pieces form the props-only world.
        let props = props_only_world(&flagged, material()).unwrap().unwrap();
        assert_eq!(props.triangles().len(), 2);
        assert!(props_only_world(&none, material()).unwrap().is_none());
    }

    /// Two raised pads side by side (828755B8): docs/hails-additions/39-skatepark-layouts.md#code-reference-skate_worldrs
    #[test]
    fn fused_pads_drop_their_back_to_back_walls_and_pair_the_top_seam() {
        let tri = |a: [f32; 3], b: [f32; 3], c: [f32; 3], out: Vec3| {
            let n = (Vec3::from_array(b) - Vec3::from_array(a)).cross(Vec3::from_array(c) - Vec3::from_array(a));
            if n.dot(out) < 0. { [a, c, b] } else { [a, b, c] }
        };
        // A closed box from x0 to x1 (y 0..0.75, z 0..1); `cross` splits the x = x1 face on the other diagonal.
        let pad = |x0: f32, x1: f32, cross: bool| {
            let (y, h) = (0.0, 0.75);
            let centre = Vec3::new((x0 + x1) / 2., h / 2., 0.5);
            let mut out = Vec::new();
            let mut face = |q: [[f32; 3]; 4], other: bool| {
                let n = q.iter().map(|p| Vec3::from_array(*p)).sum::<Vec3>() / 4. - centre;
                if other {
                    out.push(tri(q[1], q[2], q[3], n));
                    out.push(tri(q[1], q[3], q[0], n));
                } else {
                    out.push(tri(q[0], q[1], q[2], n));
                    out.push(tri(q[0], q[2], q[3], n));
                }
            };
            face([[x0, h, 0.], [x1, h, 0.], [x1, h, 1.], [x0, h, 1.]], false);
            face([[x0, y, 0.], [x1, y, 0.], [x1, y, 1.], [x0, y, 1.]], false);
            face([[x0, y, 0.], [x1, y, 0.], [x1, h, 0.], [x0, h, 0.]], false);
            face([[x0, y, 1.], [x1, y, 1.], [x1, h, 1.], [x0, h, 1.]], false);
            face([[x0, y, 0.], [x0, h, 0.], [x0, h, 1.], [x0, y, 1.]], false);
            face([[x1, y, 0.], [x1, h, 0.], [x1, h, 1.], [x1, y, 1.]], cross);
            out
        };
        // Pad B is built mirrored (x0 = 2, x1 = 1) so its x = 1 face is the `cross` one.
        for cross in [false, true] {
            let mut points = pad(0., 1., false);
            points.extend(pad(2., 1., cross));
            let pieces: Vec<usize> = (0..points.len()).map(|i| i / 12).collect();
            let (positions, vertices, flags, cosines, dropped) =
                fused_edge_features(&points, &crate::park_layout::FuseRules::default(), &pieces, &[]).unwrap();
            let walls: Vec<usize> = (0..points.len())
                .filter(|&i| points[i].iter().all(|p| (p[0] - 1.).abs() < 1e-6))
                .collect();
            assert_eq!(walls.len(), 4);
            assert_eq!((0..points.len()).filter(|&i| dropped[i]).collect::<Vec<_>>(), walls, "cross {cross}: the walls go");
            // Every top edge on the seam (both ends at x = 1, y = 0.75) is paired flat: no convex bit, cosine 1.
            let mut seam_edges = 0;
            for t in (0..points.len()).filter(|&i| !dropped[i]) {
                for e in 0..3 {
                    let [a, b] = [vertices[t][e], vertices[t][(e + 1) % 3]].map(|v| positions[v]);
                    if [a, b].iter().all(|p| (p.x - 1.).abs() < 1e-6 && (p.y - 0.75).abs() < 1e-6) {
                        seam_edges += 1;
                        assert_eq!((flags[t] & (0x20 << e), cosines[t][e]), (0, 1.0), "cross {cross}: triangle {t} edge {e}");
                    }
                }
            }
            assert!(seam_edges >= 2, "both tops have a seam edge");
            // The same pads inside ONE piece keep their walls (retail culls only between different pieces).
            let (_, _, _, _, one) = fused_edge_features(&points, &crate::park_layout::FuseRules::default(), &vec![0; points.len()], &[]).unwrap();
            assert!(one.iter().all(|d| !d));
        }
    }

    /// b478: the fused set ignores the pieces' own edge codes (keeps their surfaces), drops triangles the merge
    /// collapses (retail 82AC2010) and keeps each piece's range over the kept triangles.
    #[test]
    fn fused_pieces_drop_collapsed_triangles_and_keep_their_ranges() {
        let mut points = plates([0.0015, 0.0, 0.0]);
        // A 4 mm sliver on piece A's far edge collapses under the 1 cm merge.
        points.insert(2, [[0.0, 0.0, 0.0], [0.004, 0.0, 0.0], [0.0, 0.0, 0.004]]);
        let collision: Vec<_> = points
            .iter()
            .map(|&points| skate_data::skate_map::Collision { points, surface: 7, material: 1, native_edges: Some([0x60; 3]) })
            .collect();
        let ranges = [0..3, 3..5];
        let (triangles, surfaces, meshes) =
            portable_parts_fused(&collision, &[], material(), Some(&ranges), true, Some((&crate::park_layout::FuseRules::default(), &[]))).unwrap();
        assert_eq!(triangles.len(), 4, "the sliver is dropped");
        assert_eq!(surfaces, [7; 4], "the pieces keep their surfaces");
        assert_eq!(meshes.iter().map(|m| m.triangle_range.clone()).collect::<Vec<_>>(), [0..2, 2..4]);
        // Without fusing, the same pieces keep every triangle and their native codes.
        let (kept, _, _) = portable_parts(&collision[..2], &[], material(), None, true).unwrap();
        assert_eq!(kept.len(), 2);
    }

    #[test]
    fn fused_edge_bytes_follow_retail() {
        let tol = crate::park_layout::FuseRules::default().concave_tolerance;
        let decode = |byte: u8| decode_native_edges([byte, 0x1A, 0x1A]).unwrap();
        assert_eq!(fused_edge_byte(1.0, false, tol), 0x9A);
        let (flags, cosines) = decode(0x9A);
        assert_eq!((flags & 0x20, cosines[0]), (0, 1.0));
        assert_eq!(fused_edge_byte(0.0, true, tol), 0x22);
        let (flags, cosines) = decode(0x22);
        assert_eq!(flags & 0x20, 0x20);
        assert!((cosines[0] - 0.6916).abs() < 1e-4, "{}", cosines[0]);
        assert_eq!(fused_edge_byte(2.17, true, tol), 0x1B);
        let (flags, cosines) = decode(0x1B);
        assert_eq!((flags & 0x20, cosines[0]), (0, 1.0));
        assert_eq!(fused_angle_code(0.97), 7);
    }

    #[test]
    fn eastl_quick_sort_orders_by_key_and_keeps_every_entry() {
        for n in [0usize, 1, 2, 28, 29, 64, 300] {
            let mut v: Vec<(u32, u32)> = (0..n as u32).map(|i| ((i * 7919 + 13) % 17, i)).collect();
            let mut want = v.clone();
            eastl_quick_sort(&mut v);
            assert!(v.windows(2).all(|w| w[0].0 <= w[1].0), "n={n}");
            want.sort();
            let mut got = v.clone();
            got.sort();
            assert_eq!(got, want, "n={n}");
        }
        let mut few = vec![(2, 0), (1, 1), (2, 2), (1, 3)];
        eastl_quick_sort(&mut few);
        assert_eq!(few, [(1, 1), (1, 3), (2, 0), (2, 2)], "short runs are an insertion sort (stable)");
    }

    /// b505 (Stadium - Full toe, recomp WHLC): a knife toe on the ground plane is not convex in retail.
    #[test]
    fn a_knife_toe_on_the_ground_plane_is_not_convex() {
        let triangles = vec![
            [[0.0, 0.0, 0.75], [0.0, 0.0, 0.0], [-0.505, 0.027, 0.75]],
            [[0.0, 0.0, 0.75], [-0.75, 0.0, 0.75], [0.0, 0.0, 0.0]],
        ];
        let mut rules = crate::park_layout::FuseRules::default();
        let (_, _, flags, cosines, _) = fused_edge_features(&triangles, &rules, &[0, 0], &[]).unwrap();
        assert_eq!((cosines[0][0], flags[0] & 0x20), (0.99939764, 0), "toe edge: code 11, not convex");
        assert_eq!((cosines[1][2], flags[1] & 0x80), (0.6915748, 0x80), "underside toe edge: code 2, convex");
        rules.ground_planes = [None; 6];
        let (_, _, flags, cosines, _) = fused_edge_features(&triangles, &rules, &[0, 0], &[]).unwrap();
        assert_eq!((cosines[0][0], flags[0] & 0x20), (-0.23370063, 0x20), "without planes: the convex knife");
    }

    /// b507: a knife-toe ramp standing on another piece's deck above the ground takes its toe from the deck.
    #[test]
    fn a_knife_toe_on_a_raised_deck_is_not_convex() {
        let deck = [[[-2.0, 1.0, -2.0], [-2.0, 1.0, 2.0], [2.0, 1.0, -2.0]], [[2.0, 1.0, -2.0], [-2.0, 1.0, 2.0], [2.0, 1.0, 2.0]]];
        let ramp = |y: f32| [
            [[0.0, y, 0.75], [0.0, y, 0.0], [-0.505, y + 0.027, 0.75]],
            [[0.0, y, 0.75], [-0.75, y, 0.75], [0.0, y, 0.0]],
        ];
        let run = |y: f32, rules: &crate::park_layout::FuseRules, pieces: [usize; 4]| {
            let triangles: Vec<_> = deck.iter().chain(ramp(y).iter()).copied().collect();
            fused_edge_features(&triangles, rules, &pieces, &[]).unwrap()
        };
        let mut rules = crate::park_layout::FuseRules::default();
        let (_, _, flags, cosines, _) = run(1.002, &rules, [0, 0, 1, 1]);
        assert_eq!((cosines[2][0], flags[2] & 0x20), (0.99939764, 0), "toe edge on the deck: code 11, not convex");
        let (_, _, flags, _, _) = run(1.01, &rules, [0, 0, 1, 1]);
        assert_eq!(flags[2] & 0x20, 0x20, "more than 0.005 above the deck: the knife stays");
        let (_, _, flags, _, _) = run(1.002, &rules, [0, 0, 0, 0]);
        assert_eq!(flags[2] & 0x20, 0x20, "the same piece's deck does not count");
        rules.mesh_floor_pass = false;
        let (_, _, flags, _, _) = run(1.002, &rules, [0, 0, 1, 1]);
        assert_eq!(flags[2] & 0x20, 0x20, "pass off: the knife stays");
    }

    #[test]
    fn fused_vertex_ranks_follow_the_smallest_raw_key() {
        let vertices = [[0, 1, 2], [2, 1, 3]];
        let tag = |k: [u64; 3]| FuseTag { keys: Some(k), second_half: false };
        let rank = fused_vertex_ranks(&vertices, 4, &[0, 1], &[tag([5, 4, 9]), tag([1, 3, 0])]).unwrap();
        assert_eq!(rank, [1, 0, 2, 3], "keys 5, 4, 9 (piece 0 beats piece 1), 1 << 40");
        assert!(fused_vertex_ranks(&vertices, 4, &[0, 1], &[tag([5, 4, 9]), FuseTag::default()]).is_none());
    }

    #[test]
    fn fused_set_leaves_out_a_quads_second_half() {
        let triangles = vec![
            [[0.0, 0.0, 0.0], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]],
            [[1.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
        ];
        let tags = [
            FuseTag { keys: Some([0, 1, 2]), second_half: false },
            FuseTag { keys: Some([3, 2, 1]), second_half: true },
        ];
        let mut rules = crate::park_layout::FuseRules::default();
        let (_, _, _, _, dropped) = fused_edge_features(&triangles, &rules, &[0, 0], &tags).unwrap();
        assert_eq!(dropped, [false, true]);
        rules.quad_first_half_only = false;
        let (_, _, _, _, dropped) = fused_edge_features(&triangles, &rules, &[0, 0], &tags).unwrap();
        assert_eq!(dropped, [false, false], "a mod can keep both halves");
    }

    #[test]
    fn fused_set_drops_a_zero_area_triangle() {
        let triangles = vec![
            [[0.0, 0.0, 0.0], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]],
            [[0.0, 0.0, 0.0], [0.5, 0.0, 0.0], [1.0, 0.0, 0.0]],
        ];
        let rules = crate::park_layout::FuseRules::default();
        let (_, _, _, _, dropped) = fused_edge_features(&triangles, &rules, &[0, 1], &[]).unwrap();
        assert_eq!(dropped, [false, true]);
    }

    #[test]
    fn fused_vertex_bits_follow_retail() {
        let grid = |height: fn(f32, f32) -> f32| {
            let p = |x: f32, z: f32| [x, height(x, z), z];
            let mut triangles = Vec::new();
            for x in [-1.0, 0.0] {
                for z in [-1.0, 0.0] {
                    triangles.push([p(x, z), p(x, z + 1.), p(x + 1., z)]);
                    triangles.push([p(x + 1., z), p(x, z + 1.), p(x + 1., z + 1.)]);
                }
            }
            triangles
        };
        let centre_disabled = |triangles: Vec<[[f32; 3]; 3]>| {
            let rules = crate::park_layout::FuseRules::default();
            let (positions, vertices, flags, _, dropped) = fused_edge_features(&triangles, &rules, &vec![0; triangles.len()], &[]).unwrap();
            let centre = positions.iter().position(|p| p.x == 0.0 && p.z == 0.0).unwrap();
            let at: Vec<bool> = (0..vertices.len())
                .filter(|&t| !dropped[t])
                .filter_map(|t| (0..3).find(|&k| vertices[t][k] == centre).map(|k| flags[t] & (0x200 << k) != 0))
                .collect();
            assert!(!at.is_empty() && at.iter().all(|&d| d == at[0]));
            at[0]
        };
        assert!(centre_disabled(grid(|_, _| 0.0)), "flat");
        assert!(centre_disabled(grid(|x, _| x.abs())), "valley");
        assert!(!centre_disabled(grid(|x, z| 1.0 - x.abs().max(z.abs()))), "pyramid apex");
    }
}
