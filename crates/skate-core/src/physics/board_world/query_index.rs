//! Static bounds hierarchy over authored query meshes. Results are sorted back
//! into source order before narrow-phase, preserving equal-hit/contact order.
use super::query_metadata::{Bounds, QueryMesh};
use std::ops::Range;

#[derive(Default)]
#[cfg_attr(test, derive(Debug))]
pub(super) struct QueryIndex {
    nodes: Vec<Node>,
    order: Vec<usize>,
}
#[cfg_attr(test, derive(Debug))]
struct Node {
    bounds: Bounds,
    children: Option<(usize, usize)>,
    range: Range<usize>,
}
impl QueryIndex {
    pub(super) fn new(meshes: &[QueryMesh]) -> Self {
        let mut index = Self {
            nodes: Vec::new(),
            order: (0..meshes.len()).collect(),
        };
        if !meshes.is_empty() {
            index.build(meshes, 0..meshes.len());
        }
        index
    }
    fn build(&mut self, meshes: &[QueryMesh], range: Range<usize>) -> usize {
        let bounds = Bounds::from_points(
            self.order[range.clone()]
                .iter()
                .flat_map(|&i| [meshes[i].local_bounds.min, meshes[i].local_bounds.max]),
        )
        .unwrap();
        let id = self.nodes.len();
        self.nodes.push(Node {
            bounds,
            children: None,
            range: range.clone(),
        });
        if range.len() > 8 {
            let extents = [
                bounds.max.x - bounds.min.x,
                bounds.max.y - bounds.min.y,
                bounds.max.z - bounds.min.z,
            ];
            let axis = (0..3)
                .max_by(|&a, &b| extents[a].total_cmp(&extents[b]))
                .unwrap();
            let center = |i: usize| {
                let b = meshes[i].local_bounds;
                let low = [b.min.x, b.min.y, b.min.z][axis];
                let high = [b.max.x, b.max.y, b.max.z][axis];
                low * 0.5 + high * 0.5
            };
            let mid = range.start + range.len() / 2;
            self.order[range.clone()].select_nth_unstable_by(range.len() / 2, |&a, &b| {
                center(a).total_cmp(&center(b)).then(a.cmp(&b))
            });
            let left = self.build(meshes, range.start..mid);
            let right = self.build(meshes, mid..range.end);
            self.nodes[id].children = Some((left, right));
        }
        id
    }
    /// skate_vita (performance): the meshes' bounds changed but not their number (props
    /// rebaked at new poses, parked or brought back): every node's bounds are recomputed
    /// bottom-up from the meshes under it, the tree kept. `query` returns the sorted set of
    /// meshes whose bounds overlap the box, which any tree whose nodes contain their meshes
    /// gives, so the result equals a rebuild's (test `refit_queries_equal_a_rebuild`).
    pub(super) fn refit(&mut self, meshes: &[QueryMesh]) {
        // Children are built after their parent, so a reverse pass sees them first.
        for id in (0..self.nodes.len()).rev() {
            let bounds = match self.nodes[id].children {
                Some((left, right)) => {
                    let (a, b) = (self.nodes[left].bounds, self.nodes[right].bounds);
                    Bounds::from_points([a.min, a.max, b.min, b.max])
                }
                None => Bounds::from_points(
                    self.order[self.nodes[id].range.clone()]
                        .iter()
                        .flat_map(|&i| [meshes[i].local_bounds.min, meshes[i].local_bounds.max]),
                ),
            };
            if let Some(bounds) = bounds {
                self.nodes[id].bounds = bounds;
            }
        }
    }

    pub(super) fn len(&self) -> usize {
        self.order.len()
    }

    pub(super) fn query(&self, bounds: Bounds, meshes: &[QueryMesh]) -> Vec<usize> {
        let mut result = Vec::new();
        if self.nodes.is_empty() {
            return result;
        }
        let mut stack = vec![0];
        while let Some(id) = stack.pop() {
            let node = &self.nodes[id];
            if !node.bounds.overlaps(bounds) {
                continue;
            }
            if let Some((left, right)) = node.children {
                stack.push(right);
                stack.push(left);
            } else {
                result.extend(
                    self.order[node.range.clone()]
                        .iter()
                        .copied()
                        .filter(|&i| meshes[i].local_bounds.overlaps(bounds)),
                );
            }
        }
        result.sort_unstable();
        result
    }
}

#[cfg(test)]
mod refit_tests {
    use super::*;
    use crate::math::Vector3;
    use crate::physics::board_world::query_metadata::{QueryMesh, QueryPool};
    use crate::physics::drive_frames::RetailAffineTransform;

    fn mesh(min: [f32; 3], size: f32) -> QueryMesh {
        QueryMesh {
            triangle_range: 0..0,
            local_to_world: RetailAffineTransform::IDENTITY,
            world_to_local: RetailAffineTransform::IDENTITY,
            local_bounds: Bounds::from_points([Vector3::new(min[0], min[1], min[2]), Vector3::new(min[0] + size, min[1] + size, min[2] + size)]).unwrap(),
            matching_group: -1,
            rejection_flags: 0,
            geometry: 0,
            pool: QueryPool::Ground,
        }
    }

    /// A cheap deterministic sequence in -100..100.
    fn next(seed: &mut u32) -> f32 {
        *seed ^= *seed << 13;
        *seed ^= *seed >> 17;
        *seed ^= *seed << 5;
        (*seed % 20_000) as f32 / 100.0 - 100.0
    }

    #[test]
    fn refit_queries_equal_a_rebuild() {
        let mut seed = 0x1234_5678;
        let mut meshes: Vec<QueryMesh> = (0..300).map(|_| mesh([next(&mut seed), next(&mut seed), next(&mut seed)], 2.0 + next(&mut seed).abs() / 20.0)).collect();
        let mut index = QueryIndex::new(&meshes);
        for round in 0..20 {
            // Move some meshes (props rebaked), park some far below (props streamed out).
            for _ in 0..40 {
                let i = (next(&mut seed).abs() as usize * 7) % meshes.len();
                let at = if round % 3 == 0 { [next(&mut seed), -1000.0, next(&mut seed)] } else { [next(&mut seed), next(&mut seed), next(&mut seed)] };
                meshes[i] = mesh(at, 3.0);
            }
            index.refit(&meshes);
            let fresh = QueryIndex::new(&meshes);
            for _ in 0..200 {
                let lo = [next(&mut seed), next(&mut seed), next(&mut seed)];
                let size = next(&mut seed).abs() / 4.0;
                let query = Bounds::from_points([Vector3::new(lo[0], lo[1], lo[2]), Vector3::new(lo[0] + size, lo[1] + size, lo[2] + size)]).unwrap();
                assert_eq!(index.query(query, &meshes), fresh.query(query, &meshes));
            }
        }
    }
}
