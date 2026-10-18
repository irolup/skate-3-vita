//! Adapter for authored .skate world geometry. Does not replace controllers.
//!
//! Collision part of skate-game `skate_world.rs` (`cb79689`), lines 27–356
//! (`validate_runtime`, `collision_world` and their helpers), unchanged. The render
//! adapter (meshes, textures, shadows) is not copied. `bevy::prelude::Vec3` is
//! `glam::Vec3`, the same type bevy re-exports.
use glam::Vec3;
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
    if let Some(archive) = retail_archive(map)? {
        return retail_collision_world(archive, material);
    }
    portable_world(&map.geometry.collision, &map.materials, material)
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

/// Welded vertices and reconstructed edge features (flags, cosines) of a
/// triangle soup: the 1 mm welding and edge pairing shared by the portable map
/// world and prop instances created mid-game. `reconstruct` = false keeps the
/// default features (fully native maps decode theirs per triangle).
fn welded_edge_features(
    triangles: &[[[f32; 3]; 3]],
    reconstruct: bool,
) -> Result<(Vec<Vec3>, Vec<[usize; 3]>, Vec<u32>, Vec<[f32; 3]>), String> {
    // Match the reference RW mesh compiler's 1 mm vertex welding and reversed
    // edge pairing. Triangle diagonals are adjacency, never authored ledges.
    let mut welded = HashMap::<[i64; 3], usize>::new();
    let mut positions = Vec::<Vec3>::new();
    let mut vertices = Vec::new();
    let mut normals = Vec::new();
    for points in triangles {
        let ids = points.map(|p| {
            let inverse = 1.0 / f64::from(0.001_f32);
            let key = p.map(|v| (f64::from(v) * inverse).round() as i64);
            *welded.entry(key).or_insert_with(|| {
                let id = positions.len();
                positions.push(Vec3::from_array(p));
                id
            })
        });
        let [a, b, c] = points.map(Vec3::from_array);
        let normal = (b - a)
            .cross(c - a)
            .try_normalize()
            .ok_or("Invalid SKATE collision triangle normal")?;
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

/// Portable triangle world: 1 mm vertex welding, reconstructed adjacency and
/// contiguous-range broadphase metadata. Shared by the playable map and by
/// static prop instances, which supply already-placed world-space triangles.
fn portable_world(
    collision: &[skate_data::skate_map::Collision],
    materials: &[skate_data::skate_map::Material],
    material: RetailContactMaterial,
) -> Result<BoardWorld, String> {
    let points: Vec<_> = collision.iter().map(|t| t.points).collect();
    let (positions, vertices, mut flags, mut cosines) =
        welded_edge_features(&points, collision.iter().any(|t| t.native_edges.is_none()))?;
    let mut triangles = Vec::with_capacity(vertices.len());
    let mut packed_surfaces = Vec::with_capacity(vertices.len());
    for (i, source) in collision.iter().enumerate() {
        if let Some(edges) = source.native_edges {
            (flags[i], cosines[i]) = decode_native_edges(edges)?;
        }
        let m = &materials[source.material as usize - 1];
        // Exact EncodeRwSurfaceId mapping from the reference native adapter.
        packed_surfaces.push((m.audio | (m.physics << 7) | (m.pattern << 12)) as u16);
        let points = vertices[i].map(|id| {
            let p = positions[id];
            Vector3::new(p.x, p.y, p.z)
        });
        // Keep the supplied game's original static-world contact combine values.
        // The native map bridge supplies packed surfaces, not a guessed split of
        // the package's single friction scalar into static/dynamic coefficients.
        triangles.push(
            WorldTriangle::from_vertices(
                points,
                material,
                source.surface,
                flags[i],
                cosines[i],
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
    for start in (0..triangles.len()).step_by(64) {
        let end = (start + 64).min(triangles.len());
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
    let metadata = QueryMetadata {
        packed_surfaces,
        meshes,
        static_edges: vec![],
        island_flags: 0,
    };
    BoardWorld::with_query_metadata(triangles, metadata).map_err(str::to_owned)
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
            } else {
                appends.push((instance, triangles));
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

/// Static collision for spawned DMO prop instances. No authored DMO collision
/// mesh is recovered, so each instance reuses its template's render triangles,
/// baked into world space with the instance transform at load. Reflections
/// flip winding to keep outward normals; degenerate render triangles are
/// skipped rather than rejecting the whole layer.
/// One DMO template's retail physics collision (`native-props/<map>.json` `collision`, setup
/// `dynamic_props.template_collision`): the template's ClusteredMesh triangles in model space, with their RW edge codes
/// and surface ids (research dmo-physics-collision.md). Keyed by template id, so a mod can supply or replace one.
#[derive(Clone, Debug, Default)]
pub(crate) struct PropShape {
    pub triangles: Vec<[[f32; 3]; 3]>,
    pub edges: Vec<Option<[u8; 3]>>,
    pub surfaces: Vec<u32>,
}

/// The retail collision shapes per DMO template id; empty when setup exported none (older install).
pub(crate) fn load_dmo_collision(asset_root: &std::path::Path, map_name: &str) -> std::collections::BTreeMap<String, PropShape> {
    let path = asset_root.join("private").join("native-props").join(format!("{map_name}.json"));
    // skate_vita: through `skate_data::vfs` (the packs).
    let Some(sidecar) = skate_data::vfs::read(&path).ok().and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok()) else { return Default::default() };
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
        out.insert(key.clone(), PropShape { triangles, edges, surfaces });
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
    let path = asset_root.join("private").join("native-props").join(format!("{map_name}.json"));
    let sidecar: serde_json::Value = serde_json::from_slice(&skate_data::vfs::read(&path).ok()?).ok()?;
    let map = sidecar.get("grab_splines")?.as_object()?;
    let splines: std::collections::BTreeMap<_, _> = map.iter().map(|(k, v)| (k.clone(), crate::living_world::vehicles::parse_grab_splines(v))).filter(|(_, v): &(String, Vec<_>)| !v.is_empty()).collect();
    (!splines.is_empty()).then_some(splines)
}

/// The grind rails per DMO template id (`native-props/<map>.json` `grind_splines`, template space; retail setup
/// 82C535C0, doc 26 "Grinding on props"); `None` when the export has none. A malformed list is an error.
pub(crate) fn load_dmo_grind_rails(asset_root: &std::path::Path, map_name: &str) -> Result<Option<std::collections::BTreeMap<String, Vec<crate::grind_world::props::PropRail>>>, String> {
    let path = asset_root.join("private").join("native-props").join(format!("{map_name}.json"));
    let Ok(bytes) = skate_data::vfs::read(&path) else { return Ok(None) };
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
    use crate::physics::prop_dynamics::{dmo_type_blocks, DmoType, DMO_TYPE_CLASS};
    let path = asset_root.join("private").join("native-props").join(format!("{map_name}.json"));
    let sidecar: serde_json::Value = serde_json::from_slice(&skate_data::vfs::read(&path).ok()?).ok()?;
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
    let dir = asset_root.join("private").join("native-props");
    let package = dir.join(format!("{map_name}.props"));
    let path = if skate_data::vfs::is_file(&package) { package } else { dir.join(format!("{map_name}.skate")) };
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
    // skate_vita: files through `skate_data::vfs` (the packs); on the Vita the package is
    // `<map>.props` (`skate_data::prop_package`: the `.skate` without its texture pixels).
    let package = asset_root.join("private").join("native-props").join(format!("{map_name}.props"));
    let path = if skate_data::vfs::is_file(&package) {
        package
    } else {
        asset_root.join("private").join("native-props").join(format!("{map_name}.skate"))
    };
    if !skate_data::vfs::is_file(&path) {
        return None;
    }
    let loaded = skate_data::vfs::read(&path).map_err(|e| e.to_string()).and_then(|data| {
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
    let shapes = load_dmo_collision(asset_root, map_name);
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
            if let Some(types) = load_dmo_types(asset_root, map_name) {
                let typed = dynamics.set_type_data(&types);
                info!("SKATE_PROP_TYPES: {map_name} types={} props_with_type_data={typed}/{}", types.len(), objects.len());
            }
            if let Some(splines) = load_dmo_grab_splines(asset_root, map_name) {
                let with = dynamics.set_grab_splines(&splines);
                info!("SKATE_PROP_GRAB: {map_name} templates={} props_with_grab_splines={with}/{}", splines.len(), objects.len());
            }
            match load_dmo_grind_rails(asset_root, map_name) {
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
