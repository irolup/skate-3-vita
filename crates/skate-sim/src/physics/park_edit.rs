//! skate_vita (Object Dropper, docs/object-dropper-survey.md step D4): a park's own layout pieces moved, hidden and
//! put back in the running world. A baked park (crate::park_bake) gives each fused piece's rows a range in the static
//! world and the railed pieces' template rails and placements; an edit poses the piece's rows again in place
//! (`BoardWorld::replace_triangles`: same count, the edge flags and cosines kept, normals and edges rebuilt from the
//! moved vertices) and its rails (`StaticProvider::repose_piece`). Hidden pieces go to [`HELD_PARK`], far below the
//! world, as a held prop's triangles do.
//!
//! NOT RETAIL: retail rebuilds a layout's static set when it changes (the fuser runs again, b430 / b431); ours keeps
//! the fused rows as the layout made them, so a moved piece keeps the seam edges it had beside its old neighbours.
use std::collections::BTreeMap;
use std::ops::Range;

use skate_core::physics::board_world::WorldTriangle;
use skate_core::math::Vector3;

use super::prop_dynamics::HELD_PARK;
use super::GamePhysics;
use crate::grind_world::props::PropRail;

/// A row-vector affine (`v @ basis + translation`): the three basis rows, then the translation (the MOBJ layout).
pub(crate) type Affine = [f32; 12];

pub(crate) const IDENTITY: Affine = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0];

/// `p` through `t`.
fn apply(t: &Affine, p: [f32; 3]) -> [f32; 3] {
    [
        p[0] * t[0] + p[1] * t[3] + p[2] * t[6] + t[9],
        p[0] * t[1] + p[1] * t[4] + p[2] * t[7] + t[10],
        p[0] * t[2] + p[1] * t[5] + p[2] * t[8] + t[11],
    ]
}

/// `first`, then `then` (row vectors: `p @ first @ then`).
pub(crate) fn compose(first: &Affine, then: &Affine) -> Affine {
    let mut out = [0.0; 12];
    for r in 0..3 {
        for c in 0..3 {
            out[r * 3 + c] = (0..3).map(|k| first[r * 3 + k] * then[k * 3 + c]).sum();
        }
    }
    let t = apply(then, [first[9], first[10], first[11]]);
    out[9..12].copy_from_slice(&t);
    out
}

/// The park's pieces as the editor can touch them.
#[derive(Default)]
pub(crate) struct ParkPieces {
    /// Each fused piece's rows in the static world.
    ranges: BTreeMap<u32, Range<usize>>,
    /// The static world's rows before this one are the park's own map; the layout's, then placed terrain, follow.
    map_rows: usize,
    /// The rows of the pieces edited so far, as the layout made them (a reset puts them back bit for bit).
    originals: BTreeMap<u32, Vec<WorldTriangle>>,
    /// Railed pieces: their template's rails and the layout's placement.
    rails: BTreeMap<u32, (Vec<PropRail>, Affine)>,
    /// Every piece: its template (catalogue item) and the layout's placement, in layout order.
    pub(crate) layout: Vec<(u32, String, Affine)>,
    /// The layout's name (the map's mark).
    pub(crate) name: Option<String>,
}

impl ParkPieces {
    /// From a baked layout whose rows are the last `baked.len()` of a static world of `world_len` triangles.
    pub(crate) fn from_baked(baked: &crate::park_bake::Baked<'_>, world_len: usize, name: Option<String>) -> Self {
        let offset = world_len - baked.len();
        let ranges = baked.ids.iter().zip(&baked.ranges).map(|(&id, r)| (id, r.start + offset..r.end + offset)).collect();
        let rails = baked
            .placements
            .iter()
            .filter_map(|(id, template, placement)| Some((*id, (baked.templates.get(template)?.clone(), *placement))))
            .collect();
        Self { ranges, map_rows: offset, originals: BTreeMap::new(), rails, layout: baked.layout.clone(), name }
    }

    pub(crate) fn ids(&self) -> impl Iterator<Item = u32> + '_ {
        self.ranges.keys().copied().chain(self.rails.keys().copied().filter(|id| !self.ranges.contains_key(id)))
    }
}

/// Where a piece goes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum PiecePose {
    /// Back where the layout put it (its rows bit for bit).
    Authored,
    /// Moved: the layout's placement followed by this affine.
    Moved(Affine),
    /// Out of the world (deleted in the editor).
    Hidden,
}

impl GamePhysics {
    /// Every layout piece: id, template (its catalogue item), the layout's placement (none outside a baked park).
    pub(crate) fn park_layout(&self) -> Vec<(u32, String, Affine)> {
        self.park.as_ref().map(|p| p.layout.clone()).unwrap_or_default()
    }

    /// The baked park's layout name (`Starter_Fullpark`, ...), None outside a park.
    pub(crate) fn park_layout_name(&self) -> Option<&str> {
        self.park.as_ref().and_then(|p| p.name.as_deref())
    }

    /// A baked skate.Park (its pieces can be edited) rather than the open world.
    pub(crate) fn is_park(&self) -> bool {
        self.park.is_some()
    }

    /// Where a move of the Object Dropper's camera first meets the park's own map and ground (the items placed in it let
    /// it through): the fraction of the move and the surface's normal; None when clear, or outside a baked park.
    pub(crate) fn map_sweep(&self, from: [f32; 3], to: [f32; 3], radius: f32) -> Option<(f32, [f32; 3])> {
        crate::camera::world_query::line_in_rows(&self.world, from, to, self.park.as_ref()?.map_rows, radius)
    }

    /// The layout pieces the running world can move (none outside a baked park).
    pub(crate) fn park_piece_ids(&self) -> Vec<u32> {
        self.park.as_ref().map(|p| p.ids().collect()).unwrap_or_default()
    }

    /// Poses one layout piece's collision and rails. The world must not be shared (between ticks, as prop streaming
    /// changes it); the rails are changed only while no NPC worker shares them.
    pub(crate) fn pose_park_piece(&mut self, id: u32, pose: PiecePose) -> Result<(), String> {
        let Some(park) = self.park.as_mut() else { return Err("no park layout in this world".into()) };
        let delta = match pose {
            PiecePose::Authored => IDENTITY,
            PiecePose::Moved(delta) => delta,
            PiecePose::Hidden => [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, HELD_PARK.x, HELD_PARK.y, HELD_PARK.z],
        };
        if let Some(range) = park.ranges.get(&id).cloned() {
            let original = park.originals.entry(id).or_insert_with(|| self.world.triangles()[range.clone()].to_vec()).clone();
            let rows: Vec<WorldTriangle> = if pose == PiecePose::Authored {
                original
            } else {
                original
                    .iter()
                    .map(|t| {
                        let [a, b, c] = t.triangle.vertices.map(|v| {
                            let p = apply(&delta, [v.x, v.y, v.z]);
                            Vector3::new(p[0], p[1], p[2])
                        });
                        let f = &t.triangle.feature;
                        WorldTriangle::from_vertices([a, b, c], t.material, t.tag, f.flags, f.edge_cosines, t.triangle.fatness).unwrap_or(*t)
                    })
                    .collect()
            };
            self.world.replace_triangles(range, &rows).map_err(str::to_owned)?;
        }
        if let Some((rails, placement)) = park.rails.get(&id) {
            let segments = crate::park_layout::pose_rails(id, &compose(placement, &delta), rails);
            match std::sync::Arc::get_mut(&mut self.grind_world) {
                Some(provider) => {
                    provider.repose_piece(id, &segments)?;
                }
                None => tracing::warn!("SKATE_PARK_EDIT: rails of piece {id} shared by a worker; not moved"),
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// DownTown's park as the Vita loads it (its map with the baked layout): a railed piece moved, hidden and put
    /// back. Data-gated (`SKATE3_ASSET_ROOT`, the install's `assets`).
    #[test]
    #[ignore = "needs the private install data"]
    fn a_park_piece_moves_hides_and_comes_back() {
        let assets = std::path::PathBuf::from(std::env::var_os("SKATE3_ASSET_ROOT").expect("SKATE3_ASSET_ROOT"));
        let mut map = skate_data::skate_map::SkateMap::load(&assets.join("../maps/DownTownSkatePark.skate")).unwrap();
        let (extension, _) = crate::park_bake::bake(&assets, "Downtown_Fullpark").unwrap();
        map.extensions.push(extension);
        let mut physics = GamePhysics::load_with_difficulty(&assets, Some(&map), crate::difficulty::Difficulty::parse("Normal").unwrap()).unwrap();
        let park = physics.park.as_ref().expect("a baked park");
        let id = *park.rails.keys().find(|id| park.ranges.contains_key(id)).expect("a railed piece with collision");
        let range = park.ranges[&id].clone();
        let before: Vec<String> = physics.world.triangles()[range.clone()].iter().map(|t| format!("{t:?}")).collect();
        let rails_before = physics.grind_world.primitives().to_vec();
        // 10 m along x.
        let shift = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 10.0, 0.0, 0.0];
        physics.pose_park_piece(id, PiecePose::Moved(shift)).unwrap();
        let moved = &physics.world.triangles()[range.clone()];
        let original = &physics.park.as_ref().unwrap().originals[&id];
        assert!(moved.iter().zip(original).all(|(m, o)| (m.triangle.vertices[0].x - o.triangle.vertices[0].x - 10.0).abs() < 1e-3));
        let rails_moved = physics.grind_world.primitives().to_vec();
        let changed: Vec<usize> = (0..rails_before.len()).filter(|&i| format!("{:?}", rails_before[i]) != format!("{:?}", rails_moved[i])).collect();
        assert!(!changed.is_empty(), "its rails moved");
        assert!(changed.iter().all(|&i| (rails_moved[i].start[0] - rails_before[i].start[0] - 10.0).abs() < 1e-3));
        // Hidden: far below the world.
        physics.pose_park_piece(id, PiecePose::Hidden).unwrap();
        assert!(physics.world.triangles()[range.clone()].iter().all(|t| t.triangle.vertices[0].y < -9000.0));
        // Back: bit for bit, rails too.
        physics.pose_park_piece(id, PiecePose::Authored).unwrap();
        let after: Vec<String> = physics.world.triangles()[range].iter().map(|t| format!("{t:?}")).collect();
        assert!(before == after, "the layout's rows again");
        assert!(physics.grind_world.primitives().iter().zip(&rails_before).all(|(a, b)| format!("{a:?}") == format!("{b:?}")));
        assert!(physics.park_piece_ids().len() >= 230);
    }

    #[test]
    fn compose_applies_first_then_second() {
        let turn: Affine = [0.0, 0.0, -1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0]; // +90 degrees about y
        let shift: Affine = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 5.0, 0.0, 0.0];
        let both = compose(&turn, &shift);
        let p = apply(&both, [1.0, 0.0, 0.0]);
        let q = apply(&shift, apply(&turn, [1.0, 0.0, 0.0]));
        assert!((0..3).all(|i| (p[i] - q[i]).abs() < 1e-6), "{p:?} {q:?}");
    }
}
