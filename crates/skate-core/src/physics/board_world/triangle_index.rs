//! skate_vita: a bounds hierarchy over each query mesh's
//! triangle bounds, so a line query tests the triangles near it instead of every
//! triangle of every overlapping mesh (24,000 bounds tests per tick while walking).
//!
//! Exactness: a node's bounds are the union of its triangles' bounds, and triangle
//! bounds never contain NaN (a triangle with non-finite vertices gets infinite
//! bounds). `Bounds::overlaps` is "no strict separation on any axis", so a node that
//! does not overlap the query box has no triangle that does; a NaN in the query box
//! never prunes. Leaves apply the same per-triangle `overlaps` test as the linear
//! scan, and each mesh's result is sorted, so the output equals the scan's.
//!
//! skate_vita: a tree holds indices local to its mesh (the mesh's first
//! triangle is added at query time), so a streamed window reuses the trees of the clusters it
//! shares with the previous window ([`MeshTree`]). Local indices order as the absolute ones do,
//! so the trees and the results are the same.
use super::query_metadata::{Bounds, QueryMesh};
use crate::math::Vector3;
use std::ops::Range;
use std::sync::Arc;

const LEAF: usize = 8;

#[derive(Default)]
pub(super) struct TriangleIndex {
    trees: Vec<Arc<Tree>>,
    /// Each mesh's first triangle.
    starts: Vec<usize>,
}

/// One query mesh's hierarchy over its triangles' bounds, built once and shared by every world
/// holding the mesh (`BoardWorld::with_query_metadata_prebuilt`).
#[derive(Clone)]
pub struct MeshTree(Arc<Tree>);

impl MeshTree {
    /// The hierarchy of a mesh whose triangles have these bounds, in order.
    pub fn build(triangle_bounds: &[Bounds]) -> Self {
        Self(Arc::new(Tree::new(triangle_bounds)))
    }

    /// The triangles it indexes.
    pub fn len(&self) -> usize {
        self.0.order.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.order.is_empty()
    }
}

struct Tree {
    nodes: Vec<Node>,
    order: Vec<u32>,
}

struct Node {
    bounds: Bounds,
    children: Option<(u32, u32)>,
    range: Range<u32>,
}

impl TriangleIndex {
    pub(super) fn new(meshes: &[QueryMesh], triangle_bounds: &[Bounds]) -> Self {
        Self {
            trees: meshes
                .iter()
                .map(|mesh| Arc::new(Tree::new(&triangle_bounds[mesh.triangle_range.clone()])))
                .collect(),
            starts: meshes.iter().map(|mesh| mesh.triangle_range.start).collect(),
        }
    }

    /// skate_vita: the index of meshes whose hierarchies were built already (one per mesh, in
    /// order, each over that mesh's triangles).
    pub(super) fn from_trees(meshes: &[QueryMesh], trees: Vec<MeshTree>) -> Self {
        Self {
            trees: trees.into_iter().map(|t| t.0).collect(),
            starts: meshes.iter().map(|mesh| mesh.triangle_range.start).collect(),
        }
    }

    /// skate_vita: rebuilds the hierarchies of the meshes at `meshes` indices (their
    /// triangles moved: a prop rebaked at a new pose) and appends any mesh added since (props
    /// created mid-game); the other meshes' hierarchies are kept.
    pub(super) fn rebuild(&mut self, meshes: &[QueryMesh], triangle_bounds: &[Bounds], changed: impl IntoIterator<Item = usize>) {
        for i in changed {
            if let Some(tree) = self.trees.get_mut(i) {
                *tree = Arc::new(Tree::new(&triangle_bounds[meshes[i].triangle_range.clone()]));
                self.starts[i] = meshes[i].triangle_range.start;
            }
        }
        for mesh in &meshes[self.trees.len()..] {
            self.trees.push(Arc::new(Tree::new(&triangle_bounds[mesh.triangle_range.clone()])));
            self.starts.push(mesh.triangle_range.start);
        }
    }

    /// Appends, in ascending order, every triangle of `mesh` whose bounds overlap
    /// `bounds`: the same indices as filtering `mesh.triangle_range` linearly.
    pub(super) fn query(
        &self,
        mesh: usize,
        bounds: Bounds,
        triangle_bounds: &[Bounds],
        out: &mut Vec<usize>,
    ) {
        let from = out.len();
        let (tree, start) = (&self.trees[mesh], self.starts[mesh]);
        tree.query(bounds, &triangle_bounds[start..start + tree.order.len()], start, out);
        out[from..].sort_unstable();
    }
}

impl Tree {
    /// Over one mesh's triangle bounds; the order holds indices into them.
    fn new(triangle_bounds: &[Bounds]) -> Self {
        let mut tree = Self {
            nodes: Vec::new(),
            order: (0..triangle_bounds.len() as u32).collect(),
        };
        if !tree.order.is_empty() {
            tree.build(0..tree.order.len(), triangle_bounds);
        }
        tree
    }

    fn build(&mut self, range: Range<usize>, triangle_bounds: &[Bounds]) -> u32 {
        let bounds = union(self.order[range.clone()].iter().map(|&i| triangle_bounds[i as usize]));
        let id = self.nodes.len() as u32;
        self.nodes.push(Node {
            bounds,
            children: None,
            range: range.start as u32..range.end as u32,
        });
        if range.len() > LEAF {
            let extents = [
                bounds.max.x - bounds.min.x,
                bounds.max.y - bounds.min.y,
                bounds.max.z - bounds.min.z,
            ];
            let axis = (0..3)
                .max_by(|&a, &b| extents[a].total_cmp(&extents[b]))
                .unwrap();
            let center = |i: u32| {
                let b = triangle_bounds[i as usize];
                let (low, high) = match axis {
                    0 => (b.min.x, b.max.x),
                    1 => (b.min.y, b.max.y),
                    _ => (b.min.z, b.max.z),
                };
                low * 0.5 + high * 0.5
            };
            let mid = range.start + range.len() / 2;
            self.order[range.clone()].select_nth_unstable_by(range.len() / 2, |&a, &b| {
                center(a).total_cmp(&center(b)).then(a.cmp(&b))
            });
            let left = self.build(range.start..mid, triangle_bounds);
            let right = self.build(mid..range.end, triangle_bounds);
            self.nodes[id as usize].children = Some((left, right));
        }
        id
    }

    /// The mesh's triangles (`triangle_bounds`, the mesh's) overlapping `bounds`, as world
    /// indices (`start` added).
    fn query(&self, bounds: Bounds, triangle_bounds: &[Bounds], start: usize, out: &mut Vec<usize>) {
        if self.nodes.is_empty() {
            return;
        }
        // Depth is at most log2(triangles / LEAF) + 1, far below 64.
        let mut stack = [0u32; 64];
        let mut depth = 1;
        while depth > 0 {
            depth -= 1;
            let node = &self.nodes[stack[depth] as usize];
            if !node.bounds.overlaps(bounds) {
                continue;
            }
            match node.children {
                Some((left, right)) => {
                    stack[depth] = right;
                    stack[depth + 1] = left;
                    depth += 2;
                }
                None => out.extend(
                    self.order[node.range.start as usize..node.range.end as usize]
                        .iter()
                        .map(|&i| i as usize)
                        .filter(|&i| triangle_bounds[i].overlaps(bounds))
                        .map(|i| i + start),
                ),
            }
        }
    }
}

/// Componentwise min/max; inputs never contain NaN (see the module comment).
fn union(mut bounds: impl Iterator<Item = Bounds>) -> Bounds {
    let first = bounds.next().unwrap();
    bounds.fold(first, |u, b| Bounds {
        min: Vector3::new(min(u.min.x, b.min.x), min(u.min.y, b.min.y), min(u.min.z, b.min.z)),
        max: Vector3::new(max(u.max.x, b.max.x), max(u.max.y, b.max.y), max(u.max.z, b.max.z)),
    })
}
fn min(a: f32, b: f32) -> f32 {
    if b < a { b } else { a }
}
fn max(a: f32, b: f32) -> f32 {
    if b > a { b } else { a }
}
