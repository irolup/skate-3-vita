//! Ephemeral adapter over the canonical BoardWorld metadata. Geometry is borrowed.
use super::{Bounds, Frame, IndexedEdgeBody, Mesh, PrimaryEdges, Scene, Segment};
use skate_core::player::offboard::ground_query::{
    EdgeSearch, GroundQueryPacket, GroundQueryScene, LineHit,
};
use skate_core::{
    math::Vector3,
    physics::{
        board_world::{BoardWorld, query_metadata::QueryPool},
        drive_frames::RetailAffineTransform,
    },
    player::offboard::ground_query::Edge,
};

/// Static mesh candidates use the same ZIP hierarchy as the other world queries.
/// The existing ground leaf still owns transforms, exact bounds gates and hits.
pub struct WorldScene<'a> {
    world: &'a BoardWorld,
    /// The prop world as pool bit 2 (`offboard::BipedWorld`).
    props: Option<&'a BoardWorld>,
    edges: Scene<'a>,
}
impl GroundQueryScene for WorldScene<'_> {
    type Error = &'static str;
    fn edge_candidates(&mut self, search: &EdgeSearch) -> Result<Vec<Edge>, Self::Error> {
        self.edges.edge_candidates(search)
    }
    fn query_lines(
        &mut self,
        packet: &GroundQueryPacket,
    ) -> Result<[Option<LineHit>; 7], Self::Error> {
        let metadata = self.world.query_metadata()?;
        let triangles = self.world.triangles();
        let mut candidates = Vec::new();
        for line in &packet.lines {
            if !line.radius.is_finite() || line.radius < 0. {
                return Err("Invalid Biped line radius");
            }
            // skate_vita (performance): appended in place (the same indices).
            self.world.candidate_mesh_indices_extend(self.world.line_candidate_bounds(line.start, line.end, line.radius), &mut candidates)?;
        }
        candidates.sort_unstable();
        candidates.dedup();
        let mut pools: [Vec<Mesh<'_>>; 3] = std::array::from_fn(|_| Vec::new());
        for index in candidates {
            let source = &metadata.meshes[index];
            let pool = match source.pool {
                QueryPool::Ground => 0,
                QueryPool::Island => 1,
                QueryPool::Conditional => 2,
            };
            pools[pool].push(Mesh {
                local_to_world: frame(source.local_to_world),
                world_to_local: frame(source.world_to_local),
                local_bounds: Bounds {
                    min: source.local_bounds.min,
                    max: source.local_bounds.max,
                },
                matching_group: source.matching_group,
                triangles: &triangles[source.triangle_range.clone()],
                surfaces: &metadata.packed_surfaces[source.triangle_range.clone()],
            });
        }
        // Pool bit 2: the prop world's meshes. Light and held props are left out by their collision type with the
        // walker's mask (NOT RETAIL YET for this query: the ground query's own mask is not read; the contact probes'
        // is 0x6000).
        if let Some(props) = self.props {
            let prop_metadata = props.query_metadata()?;
            let prop_triangles = props.triangles();
            let mut prop_candidates = Vec::new();
            for line in &packet.lines {
                props.candidate_mesh_indices_extend(props.line_candidate_bounds(line.start, line.end, line.radius), &mut prop_candidates)?;
            }
            prop_candidates.sort_unstable();
            prop_candidates.dedup();
            for index in prop_candidates {
                let source = &prop_metadata.meshes[index];
                if source.rejection_flags & skate_core::player::offboard::air_selector::MESH_REJECT_MASK != 0 {
                    continue;
                }
                let part = source.triangle_range.clone();
                pools[1].push(Mesh {
                    local_to_world: frame(source.local_to_world),
                    world_to_local: frame(source.world_to_local),
                    local_bounds: Bounds { min: source.local_bounds.min, max: source.local_bounds.max },
                    matching_group: source.matching_group,
                    triangles: &prop_triangles[part.clone()],
                    surfaces: &prop_metadata.packed_surfaces[part],
                });
            }
        }
        super::lines::query_pools(
            [
                &pools[0],
                &pools[1],
                if metadata.island_flags == 3 {
                    &pools[2]
                } else {
                    &[]
                },
            ],
            packet,
        )
    }
}

/// Caller supplies actual dynamic-provider views from the same live scene owner.
/// Empty providers mean the authored scene actually has none, never a failed lookup.
pub fn with_world_scene<'w, T>(
    world: impl Into<crate::physics::offboard::BipedWorld<'w>>,
    primary_edges: PrimaryEdges<'_>,
    indexed_edges: &[IndexedEdgeBody<'_>],
    operation: impl FnOnce(&mut WorldScene<'_>) -> Result<T, &'static str>,
) -> Result<T, &'static str> {
    let world = world.into();
    let metadata = world.map.query_metadata()?;
    let edges: Vec<_> = metadata
        .static_edges
        .iter()
        .map(|s| Segment {
            edge: Edge {
                start: s.start,
                end: s.end,
            },
            local_bounds: Bounds {
                min: s.local_bounds.min,
                max: s.local_bounds.max,
            },
        })
        .collect();
    let edges = Scene {
        ground_pool: &[],
        island_pool: &[],
        conditional_pool: &[],
        island_flags: metadata.island_flags,
        static_edges: &edges,
        primary_edges,
        indexed_edges,
    };
    let mut scene = WorldScene { world: world.map, props: world.props, edges };
    operation(&mut scene)
}
fn frame(f: RetailAffineTransform) -> Frame {
    let v = |a: [f32; 3]| Vector3::new(a[0], a[1], a[2]);
    Frame {
        right: v(f.basis.columns[0]),
        up: v(f.basis.columns[1]),
        forward: v(f.basis.columns[2]),
        position: f.translation,
    }
}
