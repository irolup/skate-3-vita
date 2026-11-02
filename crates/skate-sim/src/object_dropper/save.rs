//! A world's Object Dropper edits as a save (docs/object-dropper-survey.md step D4e; owner: one saved layout per
//! park, with a way back to its Fullpark layout). Only what differs from the world's own layout is kept: the park
//! pieces moved (their pose) or deleted, and the items dropped (catalogue item and pose). Loading replays them through
//! the same paths as the editor's events, so the world is the one the editor left.
//!
//! The Vita keeps no file of it (owner, 2026-10-29): a park's edits last while it stays loaded until the Create
//! menu saves parks; the open world's drops go with a district change or when the game quits. This is the record
//! the Create menu's park save will build on.
use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};

use super::scene::{Origin, Scene};

pub const VERSION: u32 = 1;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Saved {
    pub version: u32,
    /// Park pieces away from their layout pose: (piece id, position, rotation quaternion x y z w).
    pub moved: Vec<(u32, [f32; 3], [f32; 4])>,
    /// Park pieces deleted.
    pub deleted: Vec<u32>,
    /// Dropped items: (catalogue item id, position, rotation).
    pub dropped: Vec<(String, [f32; 3], [f32; 4])>,
}

impl Saved {
    /// The scene's edits; `authored`: the world's own piece ids.
    pub fn of(scene: &Scene, authored: &std::collections::BTreeSet<u32>) -> Self {
        let mut saved = Saved { version: VERSION, ..Default::default() };
        for &id in authored {
            match scene.get(id) {
                None => saved.deleted.push(id),
                Some(p) => {
                    if let Origin::Authored { position, rotation, .. } = p.origin {
                        if p.position != position || p.rotation != rotation {
                            saved.moved.push((id, p.position.to_array(), p.rotation.to_array()));
                        }
                    }
                }
            }
        }
        for p in scene.items.iter().filter(|p| p.origin == Origin::Dropped) {
            saved.dropped.push((p.item.clone(), p.position.to_array(), p.rotation.to_array()));
        }
        saved
    }

    pub fn is_empty(&self) -> bool {
        self.moved.is_empty() && self.deleted.is_empty() && self.dropped.is_empty()
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(self).unwrap_or_default()
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        let saved: Saved = serde_json::from_slice(bytes).map_err(|e| format!("object dropper save: {e}"))?;
        if saved.version != VERSION {
            return Err(format!("object dropper save: version {} (this build reads {VERSION})", saved.version));
        }
        Ok(saved)
    }
}

/// What a save asks of the scene, in the editor's terms: (ids moved, ids deleted, ids dropped). Pieces the world
/// does not have and items the catalogue does not know are left out.
pub fn apply(saved: &Saved, scene: &mut Scene, known_item: impl Fn(&str) -> bool) -> (Vec<u32>, Vec<u32>, Vec<u32>) {
    let mut moved = Vec::new();
    for &(id, position, rotation) in &saved.moved {
        if scene.get(id).is_some_and(|p| matches!(p.origin, Origin::Authored { .. })) {
            scene.set_pose(id, Vec3::from_array(position), Quat::from_array(rotation).normalize());
            moved.push(id);
        }
    }
    let deleted = saved.deleted.iter().copied().filter(|&id| scene.get(id).is_some_and(|p| matches!(p.origin, Origin::Authored { .. })) && scene.remove(id)).collect();
    let dropped = saved
        .dropped
        .iter()
        .filter(|(item, ..)| known_item(item))
        .map(|(item, position, rotation)| scene.drop_item(item, Vec3::from_array(*position), Quat::from_array(*rotation).normalize()))
        .collect();
    (moved, deleted, dropped)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object_dropper::scene::affine;

    #[test]
    fn a_save_holds_the_edits_and_replays_them() {
        let layout = |id: u32, x: f32| (id, "RAMP".to_owned(), affine(Vec3::new(x, 0.0, 0.0), Quat::IDENTITY, Vec3::ONE));
        let authored: std::collections::BTreeSet<u32> = [1, 2, 3].into();
        let mut scene = Scene::from_layout([layout(1, 0.0), layout(2, 10.0), layout(3, 20.0)]);
        scene.set_pose(1, Vec3::new(1.0, 0.0, 5.0), Quat::from_rotation_y(0.5));
        scene.remove(2);
        scene.drop_item("PROP", Vec3::new(3.0, 1.0, 3.0), Quat::IDENTITY);
        let saved = Saved::of(&scene, &authored);
        assert_eq!((saved.moved.len(), saved.deleted.clone(), saved.dropped.len()), (1, vec![2], 1));
        let back = Saved::from_bytes(&saved.to_bytes()).unwrap();
        assert_eq!(back, saved);
        // Replayed on a fresh layout: the same edits; an unknown item is left out.
        let mut fresh = Scene::from_layout([layout(1, 0.0), layout(2, 10.0), layout(3, 20.0)]);
        let (moved, deleted, dropped) = apply(&back, &mut fresh, |item| item == "PROP");
        assert_eq!((moved, deleted, dropped.len()), (vec![1], vec![2], 1));
        assert_eq!(Saved::of(&fresh, &authored), saved);
        assert!(apply(&back, &mut Scene::default(), |_| false).2.is_empty());
        assert!(Saved::of(&Scene::from_layout([layout(1, 0.0)]), &[1].into()).is_empty());
    }
}
