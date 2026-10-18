//! Contacts for every board primitive against the host triangle world.
//! Rust owns geometry storage and traversal. Recovered query8277B720,
//! triangle fixup82AD3130, material combine82763078 and contact retention
//! determine the physical result, in world-triangle then moving-volume order.
mod broadphase;
mod query_index;
mod triangle_index;
pub mod query_metadata;
use crate::math::Vector3;
use broadphase::{conservative_bounds, primitive_bounds};
pub use broadphase::{VOLUME_QUERY_SCALE, VOLUME_QUERY_STEP, VolumeMotion, volume_query_bounds};
pub use triangle_index::MeshTree;

/// A triangle's broadphase bounds as every world computes them (skate_vita: shared by the
/// streamed windows' cached clusters): its vertices' box, infinite for non-finite vertices.
pub fn triangle_bounds_of(triangle: &WorldTriangle) -> Bounds {
    Bounds::from_points(triangle.triangle.vertices).unwrap_or(Bounds {
        min: Vector3::new(f32::NEG_INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY),
        max: Vector3::new(f32::INFINITY, f32::INFINITY, f32::INFINITY),
    })
}
use query_metadata::{Bounds, QueryMetadata};

use super::{
    board::BodyId,
    board_runtime::BoardRuntime,
    board_step::{BoardCollision, CollisionBody},
    collision::{Sphere, Triangle, WorldContactSettings},
    contact::{RetailContactInput, RetailContactMaterial, combine_contact_materials},
    triangle_query::{TriangleLineHit, triangle_segment},
    world_contact::{
        ContactBuffer, ContactPrimitive, ContactRecord, PackedPrimitive,
        primitive_triangle_world_contacts_packed,
        triangle_from_volume,
    },
};

/// Retail surface type 12 (`(tag >> 7) & 31`): water.
pub const fn is_water_tag(tag: u32) -> bool {
    tag & 0xF80 == 0x600
}

/// Water with geometry within this depth under the surface (at a given point)
/// is shallow: solid there, like retail's University fountain channel, where
/// the bailed skater lies on the water. Deeper water is not solid: bodies sink
/// in and float (Aletown canal). Project-chosen depth
/// (docs/hails-additions/09-water.md).
pub const FLOAT_DEPTH: f32 = 0.5;

#[derive(Clone, Copy, Debug)]
pub struct WorldTriangle {
    pub triangle: Triangle,
    pub material: RetailContactMaterial,
    pub tag: u32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WorldLineHit {
    pub geometry: TriangleLineHit,
    pub tag: u32,
}

impl WorldTriangle {
    /// Host geometry preparation, with explicit adjacency and sidedness.
    /// Uses the native Volume constructor and its authored winding. Invalid
    /// host geometry is rejected without substituting a repaired triangle.
    pub fn from_vertices(
        vertices: [Vector3; 3],
        material: RetailContactMaterial,
        tag: u32,
        flags: u32,
        edge_cosines: [f32; 3],
        fatness: f32,
    ) -> Option<Self> {
        if vertices
            .iter()
            .any(|v| !v.x.is_finite() || !v.y.is_finite() || !v.z.is_finite())
        {
            return None;
        }
        let triangle = triangle_from_volume(vertices, fatness, edge_cosines, flags);
        let normal = triangle.feature.normal;
        if triangle
            .edge_lengths
            .iter()
            .any(|&v| !v.is_finite() || v <= 0.0)
            || !normal.x.is_finite()
            || !normal.y.is_finite()
            || !normal.z.is_finite()
            || normal == Vector3::ZERO
        {
            return None;
        }
        Some(Self {
            triangle,
            material,
            tag,
        })
    }
}

/// Caller-resolved TempContactBuffer settings. Retention changes solver rows,
/// so its thresholds/capacity remain explicit instead of using tuned defaults.
#[derive(Clone, Copy, Debug)]
pub struct ContactRetentionSettings {
    pub capacity: u32,
    pub duplicate_distance_squared: f32,
    pub deferred_reduction: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct WheelWorldSettings {
    /// Sphere shared by wheel definitions in82C0AA78.
    pub radius: f32,
    pub material: RetailContactMaterial,
    pub query: WorldContactSettings,
    pub retention: ContactRetentionSettings,
}

/// One enabled moving primitive, with its owning solver body and material.
#[derive(Clone, Copy, Debug)]
pub struct BoardWorldVolume {
    /// Native serialized VolumeData+212 (Part+92), not Volume+84.
    pub collision_group: u32,
    pub body: CollisionBody,
    pub primitive: ContactPrimitive,
    /// Owning body's rates; 82777E70 sweeps the query box over them.
    pub motion: VolumeMotion,
    pub material: RetailContactMaterial,
}

/// Host-supplied moving geometry. The core remains independent of Rapier and
/// queries the same solid shapes as the host's rigid-body contact bridge.
#[derive(Clone, Copy, Debug)]
pub struct ExternalLineHit {
    pub hit: WorldLineHit,
    pub surface: u16,
    pub geometry_id: u32,
    pub frame: [[f32; 4]; 4],
}
pub trait ExternalQueries: Send + Sync {
    fn line(&self, start: Vector3, end: Vector3, radius: f32) -> Option<ExternalLineHit>;
    fn nearby(&self, center: Vector3, radius: f32) -> Vec<[Vector3; 3]>;
}

/// World geometry remains in supplied order. Each query reuses output storage;
/// the returned contacts are valid until the next mutable call.
pub struct BoardWorld {
    triangles: Vec<WorldTriangle>,
    external: Option<std::sync::Arc<dyn ExternalQueries>>,
    triangle_bounds: Vec<Bounds>,
    query_metadata: Option<QueryMetadata>,
    query_index: query_index::QueryIndex,
    triangle_index: triangle_index::TriangleIndex,
    maximum_fatness: f32,
    maximum_triangle_margin: f32,
    /// Indices of water triangles, for `water_surface_at`.
    water: Vec<usize>,
    /// The query's own scratch (taken out for the query, so `query_primitives` keeps its `&mut self` API).
    scratch: Option<Box<QueryScratch>>,
}

/// The per-query output and retention buffer. A caller that shares one world between threads keeps one of these each
/// and calls [`BoardWorld::query_primitives_in`] (the world itself is only read by a query).
#[derive(Clone, Debug)]
pub struct QueryScratch {
    contacts: Vec<BoardCollision>,
    buffer: ContactBuffer,
}

/// skate_vita: a retired world's largest buffers, emptied, for the next
/// streamed window to fill (no fresh 10 MB allocations in a fragmented Vita heap).
#[derive(Default)]
pub struct WorldBuffers {
    pub triangles: Vec<WorldTriangle>,
    pub triangle_bounds: Vec<Bounds>,
    pub packed_surfaces: Vec<u16>,
}

impl Default for QueryScratch {
    fn default() -> Self {
        Self {
            contacts: Vec::new(),
            buffer: ContactBuffer {
                count: 0,
                flushed: 0,
                capacity: 0,
                dropped: 0,
                distance_squared_threshold: 0.0,
                allow_flush: 1,
                deferred_reduction: 0,
                full: 0,
                records: [[0; 64]; 50],
            },
        }
    }
}

impl QueryScratch {
    pub fn scratch_contacts(&self) -> &[BoardCollision] {
        &self.contacts
    }

    pub fn scratch_dropped_contacts(&self) -> u32 {
        self.buffer.dropped
    }
}

impl BoardWorld {
    pub fn new(triangles: Vec<WorldTriangle>) -> Self {
        Self::new_in(triangles, Vec::new())
    }

    /// skate_vita: [`Self::new`] writing the triangle bounds into `triangle_bounds` (emptied;
    /// its storage reused). The same values.
    pub fn new_in(triangles: Vec<WorldTriangle>, mut triangle_bounds: Vec<Bounds>) -> Self {
        triangle_bounds.clear();
        triangle_bounds.extend(triangles.iter().map(triangle_bounds_of));
        Self::with_bounds(triangles, triangle_bounds)
    }

    /// skate_vita: [`Self::new_in`] with each triangle's [`triangle_bounds_of`] given, in order.
    fn with_bounds(triangles: Vec<WorldTriangle>, triangle_bounds: Vec<Bounds>) -> Self {
        let maximum_fatness = triangles
            .iter()
            .map(|t| {
                if t.triangle.fatness.is_finite() && t.triangle.fatness >= 0. {
                    t.triangle.fatness
                } else {
                    f32::INFINITY
                }
            })
            .fold(0., f32::max);
        // The current thin-triangle leaf accepts barycentric weights down to
        // -epsilon. Two negative weights can extend an axis by 2*epsilon*span.
        let maximum_triangle_margin = triangle_bounds
            .iter()
            .map(|b| {
                let span = (b.max.x - b.min.x)
                    .max(b.max.y - b.min.y)
                    .max(b.max.z - b.min.z);
                span * (2. * broadphase::THIN_MARGIN)
            })
            .fold(0., f32::max);
        let water = (0..triangles.len())
            .filter(|&i| is_water_tag(triangles[i].tag))
            .collect();
        Self {
            water,
            triangles,
            external: None,
            triangle_bounds,
            query_metadata: None,
            query_index: query_index::QueryIndex::default(),
            triangle_index: triangle_index::TriangleIndex::default(),
            maximum_fatness,
            maximum_triangle_margin,
            scratch: Some(Box::default()),
        }
    }

    /// Whether the water at `point` on its surface is shallow: some geometry
    /// lies within `FLOAT_DEPTH` under it (a channel, a ledge, a bank).
    pub fn water_shallow_at(&self, point: Vector3) -> bool {
        let start = Vector3::new(point.x, point.y - 0.001, point.z);
        let end = Vector3::new(point.x, point.y - FLOAT_DEPTH, point.z);
        !matches!(self.query_thin_line(start, end), Ok(None))
    }

    pub fn with_query_metadata(
        triangles: Vec<WorldTriangle>,
        metadata: QueryMetadata,
    ) -> Result<Self, &'static str> {
        Self::with_query_metadata_in(triangles, metadata, Vec::new())
    }

    /// skate_vita: [`Self::with_query_metadata`] reusing a bounds buffer ([`Self::new_in`]).
    /// skate_vita: [`Self::with_query_metadata_in`] for meshes whose triangle
    /// bounds and hierarchies were computed when their triangles were (a streamed window reuses
    /// those of the clusters it shares with the previous window: computing them was most of a
    /// window build on the Vita). `triangle_bounds` holds each triangle's [`triangle_bounds_of`] in
    /// order and `trees` one [`MeshTree`] per mesh over that mesh's triangles. The meshes must
    /// partition the triangles as there; their bounds hold their triangles by construction, so
    /// that check is not repeated. The same world.
    pub fn with_query_metadata_prebuilt(
        triangles: Vec<WorldTriangle>,
        metadata: QueryMetadata,
        triangle_bounds: Vec<Bounds>,
        trees: Vec<MeshTree>,
    ) -> Result<Self, &'static str> {
        if metadata.packed_surfaces.len() != triangles.len() || triangle_bounds.len() != triangles.len() {
            return Err("Query surface and bounds counts must equal canonical triangle count");
        }
        if trees.len() != metadata.meshes.len() {
            return Err("One mesh hierarchy per query mesh");
        }
        let mut next = 0;
        for (mesh, tree) in metadata.meshes.iter().zip(&trees) {
            let r = &mesh.triangle_range;
            if r.start != next || r.end <= next || r.end > triangles.len() || tree.len() != r.len() {
                return Err("Query meshes must partition canonical triangles in supplied order");
            }
            next = r.end;
        }
        let mut world = Self::with_bounds(triangles, triangle_bounds);
        world.query_index = query_index::QueryIndex::new(&metadata.meshes);
        world.triangle_index = triangle_index::TriangleIndex::from_trees(&metadata.meshes, trees);
        world.query_metadata = Some(metadata);
        Ok(world)
    }

    pub fn with_query_metadata_in(
        triangles: Vec<WorldTriangle>,
        metadata: QueryMetadata,
        triangle_bounds: Vec<Bounds>,
    ) -> Result<Self, &'static str> {
        metadata.validate(&triangles)?;
        let mut world = Self::new_in(triangles, triangle_bounds);
        world.query_index = query_index::QueryIndex::new(&metadata.meshes);
        world.triangle_index =
            triangle_index::TriangleIndex::new(&metadata.meshes, &world.triangle_bounds);
        world.query_metadata = Some(metadata);
        Ok(world)
    }

    /// skate_vita: the world's largest buffers, emptied, for the next one ([`WorldBuffers`]).
    pub fn into_buffers(self) -> WorldBuffers {
        let mut b = WorldBuffers {
            triangles: self.triangles,
            triangle_bounds: self.triangle_bounds,
            packed_surfaces: self.query_metadata.map(|m| m.packed_surfaces).unwrap_or_default(),
        };
        b.triangles.clear();
        b.triangle_bounds.clear();
        b.packed_surfaces.clear();
        b
    }

    /// The two world-wide broadphase paddings (the largest triangle fatness and the
    /// largest thin-triangle margin). skate_vita: a streamed window holds a part of
    /// the world and must pad its queries exactly as the whole world does.
    pub fn broadphase_limits(&self) -> (f32, f32) {
        (self.maximum_fatness, self.maximum_triangle_margin)
    }

    /// Replaces the paddings by those of the whole world (see `broadphase_limits`).
    pub fn with_broadphase_limits(mut self, limits: (f32, f32)) -> Self {
        (self.maximum_fatness, self.maximum_triangle_margin) = limits;
        self
    }

    pub fn query_metadata(&self) -> Result<&QueryMetadata, &'static str> {
        self.query_metadata
            .as_ref()
            .ok_or("Canonical world has no authored query metadata")
    }

    pub fn set_external_queries(&mut self, queries: Option<std::sync::Arc<dyn ExternalQueries>>) {
        self.external = queries;
    }
    pub fn external_line(&self, start: Vector3, end: Vector3, radius: f32) -> Option<ExternalLineHit> {
        self.external.as_ref()?.line(start, end, radius)
    }
    pub fn external_nearby(&self, center: Vector3, radius: f32) -> Vec<[Vector3; 3]> {
        self.external.as_ref().map_or_else(Vec::new, |q| q.nearby(center, radius))
    }

    pub fn triangles(&self) -> &[WorldTriangle] {
        &self.triangles
    }

    /// Board wheel segments use zero query radius; rounded world triangles
    /// still select the native swept branch through their own fatness.
    pub fn query_thin_line(
        &self,
        start: Vector3,
        end: Vector3,
    ) -> Result<Option<WorldLineHit>, &'static str> {
        self.query_swept_line(start, end, 0.0)
    }

    /// Ordered nearest-hit traversal for wheel and camera queries. The host
    /// owns traversal; each candidate uses the complete TU3 triangle dispatcher.
    pub fn query_swept_line(
        &self,
        start: Vector3,
        end: Vector3,
        radius: f32,
    ) -> Result<Option<WorldLineHit>, &'static str> {
        if !radius.is_finite() || radius < 0.0 {
            return Err("line query radius must be finite and nonnegative");
        }
        let direction = Vector3::new(end.x - start.x, end.y - start.y, end.z - start.z);
        let mut nearest: Option<WorldLineHit> = self.external_line(start, end, radius).map(|h| h.hit);
        for (_, entry) in self.line_candidates(start, end, radius) {
            let mut geometry = TriangleLineHit {
                position: Vector3::ZERO,
                normal: Vector3::ZERO,
                fraction: 0.0,
                volume_parameter: [0.0; 3],
            };
            if triangle_segment(
                &mut geometry,
                start,
                direction,
                entry.triangle.vertices,
                radius,
                entry.triangle.fatness,
            ) {
                if nearest
                    .as_ref()
                    .is_none_or(|hit| geometry.fraction < hit.geometry.fraction)
                {
                    nearest = Some(WorldLineHit {
                        geometry,
                        tag: entry.tag,
                    });
                }
            }
        }
        Ok(nearest)
    }

    /// Sphere-only convenience for callers querying wheels. The game uses
    /// query_primitives with all enabled deck/truck/wheel children.
    pub fn query(
        &mut self,
        board: &BoardRuntime,
        settings: WheelWorldSettings,
    ) -> &[BoardCollision] {
        let volumes = Self::wheel_volumes(board, settings);
        self.query_primitives(&volumes, settings.query, settings.retention)
    }

    /// The wheel spheres [`Self::query`] asks about (shared with callers that keep their own scratch).
    pub fn wheel_volumes(board: &BoardRuntime, settings: WheelWorldSettings) -> Vec<BoardWorldVolume> {
        let poses = board.part_transforms();
        let volumes: Vec<BoardWorldVolume> = BodyId::ORDER[..4]
            .iter()
            .filter_map(|&id| {
                let body = &board.bodies()[id.index()];
                (body.state_flags != 1).then_some(BoardWorldVolume {
                    collision_group: board.collision_group(),
                    body: CollisionBody::Board(id),
                    primitive: ContactPrimitive::Sphere(Sphere {
                        center: poses[id.index()].translation,
                        radius: settings.radius,
                    }),
                    motion: VolumeMotion::of(&body.rates),
                    material: settings.material,
                })
            })
            .collect();
        volumes
    }

    /// World-space shapes come from the live part poses and authored children.
    /// Cluster bounds include shape radii and the maximum predictive padding.
    /// Candidate pairs retain world-triangle then moving-volume order.
    pub fn query_primitives(
        &mut self,
        volumes: &[BoardWorldVolume],
        query: WorldContactSettings,
        retention: ContactRetentionSettings,
    ) -> &[BoardCollision] {
        let mut scratch = self.scratch.take().expect("BoardWorld query scratch is always present between queries");
        self.query_primitives_in(&mut scratch, volumes, query, retention);
        self.scratch = Some(scratch);
        &self.scratch.as_ref().expect("restored above").contacts
    }

    /// [`Self::query_primitives`] with the caller's scratch: the world is only read, so several threads can query one
    /// shared world, each with its own [`QueryScratch`].
    pub fn query_primitives_in<'s>(
        &self,
        scratch: &'s mut QueryScratch,
        volumes: &[BoardWorldVolume],
        query: WorldContactSettings,
        retention: ContactRetentionSettings,
    ) -> &'s [BoardCollision] {
        let QueryScratch { contacts: self_contacts, buffer: self_buffer } = scratch;
        self_contacts.clear();
        self_buffer.count = 0;
        self_buffer.flushed = 0;
        self_buffer.dropped = 0;
        self_buffer.full = 0;
        self_buffer.capacity = retention.capacity;
        self_buffer.distance_squared_threshold = retention.duplicate_distance_squared;
        self_buffer.deferred_reduction = u8::from(retention.deferred_reduction);
        // No volumes, no pairs: the union bounds below would be None and
        // candidate_ranges(None) means "every triangle", so a hidden board
        // (all volumes disabled, state 3) walked the whole map each tick
        // for an empty result (fix15). The result and buffer state are the
        // same as the full walk's: nothing allocated, nothing flushed.
        if volumes.is_empty() {
            return &*self_contacts;
        }
        let padding =
            if query.volume_padding.is_finite() && query.maximum_separating_distance.is_finite() {
                query.volume_padding.max(0.) + query.maximum_separating_distance.max(0.)
            } else {
                f32::INFINITY
            };
        // Cluster preselection only: a conservative superset of every pair the
        // per-volume test below can keep.
        let preselect_bounds: Vec<_> = volumes
            .iter()
            .map(|v| {
                primitive_bounds(v.primitive)
                    .map(|b| conservative_bounds(b, padding + self.maximum_fatness))
            })
            .collect();
        // 82777E70: the swept, scaled box 8277BC58 tests every triangle against.
        let volume_bounds: Vec<_> = volumes
            .iter()
            .map(|v| {
                volume_query_bounds(v.primitive, v.motion, VOLUME_QUERY_STEP, VOLUME_QUERY_SCALE)
            })
            .collect();
        let bounds: Option<Vec<_>> = preselect_bounds
            .iter()
            .zip(&volume_bounds)
            .map(|(p, v)| match (p, v) {
                (Some(p), Some(v)) => Some(Bounds {
                    min: Vector3::new(p.min.x.min(v.min.x), p.min.y.min(v.min.y), p.min.z.min(v.min.z)),
                    max: Vector3::new(p.max.x.max(v.max.x), p.max.y.max(v.max.y), p.max.z.max(v.max.z)),
                }),
                _ => None,
            })
            .collect();
        let bounds = bounds
            .and_then(|b| Bounds::from_points(b.iter().flat_map(|b| [b.min, b.max])))
            .map(|b| b.expanded(padding));
        // skate_vita: with metadata and a valid union, the triangles that pass
        // the union test below come from the per-mesh hierarchy (triangle_index.rs):
        // the same indices in the same order as scanning the candidate ranges.
        let union_hits = match (&self.query_metadata, bounds) {
            (Some(_), Some(union)) => self.candidate_meshes(bounds).map(|meshes| {
                let mut out = Vec::new();
                for mesh in meshes {
                    self.triangle_index.query(mesh, union, &self.triangle_bounds, &mut out);
                }
                out
            }),
            _ => None,
        };
        let ranges = if union_hits.is_some() {
            Vec::new()
        } else {
            self.candidate_ranges(bounds)
        };
        // skate_vita: profile counters (crate::profile); no computation changes.
        // q_triangles: triangles visited by the loop below.
        crate::profile::count("q_volumes", volumes.len() as u64);
        crate::profile::count(
            "q_triangles",
            union_hits.as_ref().map_or(0, |h| h.len() as u64)
                + ranges.iter().map(|r| r.len() as u64).sum::<u64>(),
        );
        // Counted locally and reported once per query, so the counter itself (a lock
        // in profile builds) does not distort the timing.
        let mut narrowphase_calls = 0u64;
        // skate_vita: each volume packed once per query and each triangle once,
        // on its first overlapping volume, instead of both on every pair (`pack` is
        // pure; same records).
        let packed_volumes: Vec<PackedPrimitive> =
            volumes.iter().map(|v| PackedPrimitive::new(v.primitive)).collect();
        let output = &mut *self_contacts;
        let mut publish = |records: &[ContactRecord]| {
            output.extend(records.iter().map(collision_from_record));
        };
        'triangles: for index in union_hits.into_iter().flatten().chain(ranges.into_iter().flatten()) {
            // skate_vita: skip a triangle that misses the union
            // of all volume bounds. `bounds` is Some only when every volume has bounds; it
            // contains each of them, so such a triangle fails every per-volume test below
            // and produces nothing either way. Same results, one test instead of one per
            // volume.
            if self.query_metadata.is_some()
                && bounds.is_some_and(|union| !self.triangle_bounds[index].overlaps(union))
            {
                continue;
            }
            let entry = &self.triangles[index];
            let mut packed_triangle = None;
            for ((volume, volume_bounds), packed_volume) in
                volumes.iter().zip(&volume_bounds).zip(&packed_volumes)
            {
                // GroundPipeline8277BC58 checks the group/surface exclusion
                // table before running the geometric dispatcher. Its stock
                // initializer82767D60 sets only (7,12) and (16,12).
                if matches!(volume.collision_group, 7 | 16) && is_water_tag(entry.tag) {
                    continue;
                }
                // 8277BC58 BE94..BF38: unpadded triangle vertex box against the
                // volume box, for every triangle. A non-finite box keeps the pair.
                if volume_bounds.is_some_and(|b| !self.triangle_bounds[index].overlaps(b)) {
                    continue;
                }
                narrowphase_calls += 1;
                let packed_triangle = packed_triangle.get_or_insert_with(|| {
                    PackedPrimitive::new(ContactPrimitive::Triangle(entry.triangle))
                });
                let Some(manifold) = primitive_triangle_world_contacts_packed(
                    packed_volume,
                    packed_triangle,
                    entry.triangle,
                    volume.motion.linear_velocity,
                    query,
                ) else {
                    continue;
                };
                let material = combine_contact_materials(volume.material, entry.material);
                for pair in &manifold.points[..manifold.count] {
                    let contact = RetailContactInput {
                        position_on_a: pair.a,
                        position_on_b: pair.b,
                        normal: manifold.normal,
                        restitution: material.restitution,
                        static_friction: material.static_friction,
                        dynamic_friction: material.dynamic_friction,
                        tag: entry.tag,
                    };
                    let Some(slot) = self_buffer.allocate(&mut publish) else {
                        self_buffer.flush(&mut publish);
                        break 'triangles;
                    };
                    self_buffer.records[slot] = retention_record(volume.body, contact);
                    //8277C23C removes the most recent record on rejection.
                    if self_buffer.last_is_duplicate() {
                        self_buffer.count -= 1;
                    }
                }
            }
        }
        if self_buffer.full == 0 {
            self_buffer.flush(&mut publish);
        }
        crate::profile::count("q_narrowphase", narrowphase_calls);
        &*self_contacts
    }

    pub fn contacts(&self) -> &[BoardCollision] {
        &self.scratch.as_ref().expect("BoardWorld query scratch").contacts
    }

    /// `water_surface_at`, but only where the water is at least `min_depth`
    /// deep: no solid (or other) geometry within `min_depth` under the surface
    /// at the point's XZ. Shallow water over a floor is not something to float in.
    pub fn deep_water_surface_at(
        &self,
        point: Vector3,
        above: f32,
        max_depth: f32,
        min_depth: f32,
    ) -> Option<f32> {
        let surface = self.water_surface_at(point, above, max_depth)?;
        let start = Vector3::new(point.x, surface - 0.001, point.z);
        let end = Vector3::new(point.x, surface - min_depth, point.z);
        matches!(self.query_thin_line(start, end), Ok(None)).then_some(surface)
    }

    /// Height of the water surface over `point`, when the point lies at most
    /// `max_depth` below it (or less than `above` over it). Water surfaces are
    /// the type-12 triangles, interpolated at the point's XZ; the nearest
    /// surface at or above the point wins.
    pub fn water_surface_at(&self, point: Vector3, above: f32, max_depth: f32) -> Option<f32> {
        let mut best: Option<f32> = None;
        for &index in &self.water {
            let [a, b, c] = self.triangles[index].triangle.vertices;
            let det = (b.x - a.x) * (c.z - a.z) - (c.x - a.x) * (b.z - a.z);
            if det.abs() < 1e-8 {
                continue;
            }
            let u = ((point.x - a.x) * (c.z - a.z) - (c.x - a.x) * (point.z - a.z)) / det;
            let v = ((b.x - a.x) * (point.z - a.z) - (point.x - a.x) * (b.z - a.z)) / det;
            if u < 0. || v < 0. || u + v > 1. {
                continue;
            }
            let height = a.y + u * (b.y - a.y) + v * (c.y - a.y);
            let depth = height - point.y;
            if depth > -above && depth <= max_depth && best.is_none_or(|h| height < h) {
                best = Some(height);
            }
        }
        best
    }

    /// Replace a contiguous run of triangles in place (same count, same order).
    /// Used by prop instances rebaked at a new rigid pose: adjacency flags and
    /// edge cosines are rigid-motion invariant, so callers reuse them. Per-triangle
    /// and per-mesh bounds plus the broadphase index are refreshed.
    pub fn replace_triangles(
        &mut self,
        range: std::ops::Range<usize>,
        triangles: &[WorldTriangle],
    ) -> Result<(), &'static str> {
        self.replace_triangles_batch(&[(range, triangles)])
    }

    /// Several [`Self::replace_triangles`] edits applied in order (a later edit of the same range wins), with the
    /// overlapped mesh bounds and the query index rebuilt once at the end. The index depends only on the final
    /// mesh bounds, so the result is identical to one call per edit (test
    /// `batched_replacements_match_one_call_per_edit`); a prop stream pass rebuilt the whole index once per prop.
    /// Every edit is checked first: an error changes nothing.
    pub fn replace_triangles_batch(
        &mut self,
        edits: &[(std::ops::Range<usize>, &[WorldTriangle])],
    ) -> Result<(), &'static str> {
        let mut bounds = Vec::with_capacity(edits.iter().map(|e| e.1.len()).sum());
        for (range, triangles) in edits {
            if range.len() != triangles.len() || range.end > self.triangles.len() {
                return Err("replacement triangles must match the existing range");
            }
            for entry in triangles.iter() {
                bounds.push(Bounds::from_points(entry.triangle.vertices).ok_or("invalid triangle bounds")?);
            }
        }
        let mut bounds = bounds.into_iter();
        for (range, triangles) in edits {
            self.triangles[range.clone()].copy_from_slice(triangles);
            for i in range.clone() {
                self.triangle_bounds[i] = bounds.next().expect("one bounds per triangle");
            }
        }
        if let Some(metadata) = &mut self.query_metadata {
            // skate_vita (index): the meshes whose triangles moved, for their hierarchies.
            let mut changed = Vec::new();
            for (i, mesh) in metadata.meshes.iter_mut().enumerate() {
                let r = &mesh.triangle_range;
                if edits.iter().any(|(range, _)| r.start < range.end && range.start < r.end) {
                    mesh.local_bounds = Bounds::from_points(
                        self.triangles[r.clone()].iter().flat_map(|t| t.triangle.vertices),
                    )
                    .ok_or("invalid query mesh bounds")?;
                    changed.push(i);
                }
            }
            if !edits.is_empty() {
                // skate_vita (performance): the same meshes with new bounds: the index is
                // refitted, which queries the same as the rebuild (`QueryIndex::refit`).
                if self.query_index.len() == metadata.meshes.len() {
                    self.query_index.refit(&metadata.meshes);
                } else {
                    self.query_index = query_index::QueryIndex::new(&metadata.meshes);
                }
                // skate_vita (index): the per-mesh triangle hierarchies are over the
                // triangle bounds: those of the moved meshes are rebuilt.
                self.triangle_index.rebuild(&metadata.meshes, &self.triangle_bounds, changed);
            }
        }
        Ok(())
    }

    /// Append triangles after the existing ones (a prop body created mid-game).
    /// Existing triangle indices, order and query meshes are unchanged; the new
    /// range gets its own query meshes (64-triangle chunks, the portable world's
    /// grouping) so its bounds never widen an existing chunk. `packed_surfaces`
    /// holds one authored surface code per triangle (required when the world has
    /// query metadata). Returns the new range.
    pub fn append_triangles(
        &mut self,
        triangles: &[WorldTriangle],
        packed_surfaces: &[u16],
    ) -> Result<std::ops::Range<usize>, &'static str> {
        Ok(self.append_triangles_batch(&[(triangles, packed_surfaces)])?.remove(0))
    }

    /// skate_vita: several [`Self::append_triangles`] runs in order, each with its own query
    /// meshes, and the query index rebuilt once at the end (props streamed in by one census pass).
    /// Every run is checked first: an error changes nothing. Returns the new ranges in order.
    pub fn append_triangles_batch(
        &mut self,
        parts: &[(&[WorldTriangle], &[u16])],
    ) -> Result<Vec<std::ops::Range<usize>>, &'static str> {
        let mut bounds = Vec::with_capacity(parts.iter().map(|p| p.0.len()).sum());
        for (triangles, packed_surfaces) in parts {
            for entry in triangles.iter() {
                bounds.push(Bounds::from_points(entry.triangle.vertices).ok_or("invalid triangle bounds")?);
            }
            if self.query_metadata.is_some() && packed_surfaces.len() != triangles.len() {
                return Err("appended triangles need one packed surface each");
            }
        }
        let mut bounds = bounds.into_iter();
        let mut ranges = Vec::with_capacity(parts.len());
        for (triangles, packed_surfaces) in parts {
            let start = self.triangles.len();
            self.triangles.extend_from_slice(triangles);
            for (i, entry) in triangles.iter().enumerate() {
                let fatness = if entry.triangle.fatness.is_finite() && entry.triangle.fatness >= 0. {
                    entry.triangle.fatness
                } else {
                    f32::INFINITY
                };
                self.maximum_fatness = self.maximum_fatness.max(fatness);
                let b = bounds.next().expect("one bounds per triangle");
                let span = (b.max.x - b.min.x).max(b.max.y - b.min.y).max(b.max.z - b.min.z);
                self.maximum_triangle_margin = self.maximum_triangle_margin.max(span * (2. * broadphase::THIN_MARGIN));
                if is_water_tag(entry.tag) {
                    self.water.push(start + i);
                }
                self.triangle_bounds.push(b);
            }
            let end = self.triangles.len();
            if let Some(metadata) = &mut self.query_metadata {
                metadata.packed_surfaces.extend_from_slice(packed_surfaces);
                for chunk in (start..end).step_by(64) {
                    let chunk_end = (chunk + 64).min(end);
                    metadata.meshes.push(query_metadata::QueryMesh {
                        triangle_range: chunk..chunk_end,
                        local_to_world: crate::physics::drive_frames::RetailAffineTransform::IDENTITY,
                        world_to_local: crate::physics::drive_frames::RetailAffineTransform::IDENTITY,
                        local_bounds: Bounds::from_points(
                            self.triangles[chunk..chunk_end].iter().flat_map(|t| t.triangle.vertices),
                        )
                        .ok_or("invalid query mesh bounds")?,
                        matching_group: -1,
                        rejection_flags: 0,
                        geometry: 0,
                        pool: query_metadata::QueryPool::Ground,
                    });
                }
            }
            ranges.push(start..end);
        }
        if let Some(metadata) = &mut self.query_metadata {
            self.query_index = query_index::QueryIndex::new(&metadata.meshes);
            // skate_vita (index): the new meshes get their triangle hierarchies.
            self.triangle_index.rebuild(&metadata.meshes, &self.triangle_bounds, std::iter::empty());
        }
        Ok(ranges)
    }

    pub fn dropped_contacts(&self) -> u32 {
        self.scratch.as_ref().expect("BoardWorld query scratch").buffer.dropped
    }
}

/// Retention reads only contact header geometry, body IDs and material/tag.
/// BoardStep builds the actual body workspaces after this frame's force queue
/// has been applied, preventing stale acceleration copies in solver contacts.
fn retention_record(id: CollisionBody, contact: RetailContactInput) -> ContactRecord {
    let mut record = [0; 64];
    for (offset, v) in [
        (0, contact.position_on_a),
        (4, contact.position_on_b),
        (8, contact.normal),
    ] {
        record[offset..offset + 3].copy_from_slice(&[v.x, v.y, v.z].map(f32::to_bits));
    }
    record[3] = id.contact_id();
    record[7] = u32::MAX;
    record[11] = contact.restitution.to_bits();
    record[15] = contact.static_friction.to_bits();
    record[19] = contact.dynamic_friction.to_bits();
    record[23] = contact.tag;
    record
}

fn collision_from_record(record: &ContactRecord) -> BoardCollision {
    let f = |i| f32::from_bits(record[i]);
    let vector = |i| Vector3::new(f(i), f(i + 1), f(i + 2));
    BoardCollision {
        body_a: CollisionBody::from_contact_id(record[3]),
        body_b: CollisionBody::StaticWorld,
        contact: RetailContactInput {
            position_on_a: vector(0),
            position_on_b: vector(4),
            normal: vector(8),
            restitution: f(11),
            static_friction: f(15),
            dynamic_friction: f(19),
            tag: record[23],
        },
    }
}

#[cfg(test)]
#[path = "tests/board_world.rs"]
mod tests;

#[cfg(test)]
#[path = "board_world/tests.rs"]
mod broadphase_tests;
