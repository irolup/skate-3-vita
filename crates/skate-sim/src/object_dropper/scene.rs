//! The items the editor can touch: a park's own layout pieces (authored: they remember where the layout put them)
//! and the items dropped since (retail's spawn record vs none: a reset puts an authored piece back on its record's
//! transform and removes a dropped one, 82C4B780).
use glam::{Quat, Vec3};

use super::budget::Usage;
use super::catalog::Catalog;

/// Where an item comes from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Origin {
    /// A layout piece: its MOBJ id and the pose the layout gives it.
    Authored { piece: u32, position: Vec3, rotation: Quat, scale: Vec3 },
    Dropped,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Placed {
    /// The editor's id (an authored piece keeps its MOBJ id; dropped items count from [`DROPPED_ID_BASE`]).
    pub id: u32,
    /// The catalogue item (recipe id).
    pub item: String,
    pub position: Vec3,
    pub rotation: Quat,
    /// The layout's scale (a negative axis for a mirrored piece); one for dropped items.
    pub scale: Vec3,
    pub origin: Origin,
    /// Its colour (an index into `Tuning::colours`; 0 white, `piece+0xD0`).
    pub tint: u8,
}

impl Placed {
    /// Its template-to-world transform as a row-vector affine (`v @ basis + translation`, the MOBJ layout).
    pub fn affine(&self) -> [f32; 12] {
        affine(self.position, self.rotation, self.scale)
    }
}

/// Scale, then rotation, then translation, as a row-vector affine.
pub fn affine(position: Vec3, rotation: Quat, scale: Vec3) -> [f32; 12] {
    let axes = [rotation * Vec3::X * scale.x, rotation * Vec3::Y * scale.y, rotation * Vec3::Z * scale.z];
    [axes[0].x, axes[0].y, axes[0].z, axes[1].x, axes[1].y, axes[1].z, axes[2].x, axes[2].y, axes[2].z, position.x, position.y, position.z]
}

/// A row-vector affine split into position, rotation and scale (a mirror as one negative axis).
pub fn split(t: &[f32; 12]) -> (Vec3, Quat, Vec3) {
    let m = glam::Mat4::from_cols(
        glam::Vec4::new(t[0], t[1], t[2], 0.0),
        glam::Vec4::new(t[3], t[4], t[5], 0.0),
        glam::Vec4::new(t[6], t[7], t[8], 0.0),
        glam::Vec4::new(t[9], t[10], t[11], 1.0),
    );
    let (scale, rotation, position) = m.to_scale_rotation_translation();
    (position, rotation.normalize(), scale)
}

/// Dropped items' ids (above the layout pieces' MOBJ ids and the sim's runtime props, 0x2000_0000).
pub const DROPPED_ID_BASE: u32 = 0x6000_0000;
/// The park spawner's id (one below the dropped items': no layout piece or runtime prop has it).
pub const SPAWNER_ID: u32 = DROPPED_ID_BASE - 1;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Scene {
    pub items: Vec<Placed>,
    next: u32,
}

impl Scene {
    /// A scene of a layout's pieces: (MOBJ id, catalogue item, placement).
    pub fn from_layout(pieces: impl IntoIterator<Item = (u32, String, [f32; 12])>) -> Self {
        let items = pieces
            .into_iter()
            .map(|(piece, item, placement)| {
                let (position, rotation, scale) = split(&placement);
                Placed { id: piece, item, position, rotation, scale, origin: Origin::Authored { piece, position, rotation, scale }, tint: 0 }
            })
            .collect();
        Self { items, next: DROPPED_ID_BASE }
    }

    pub fn get(&self, id: u32) -> Option<&Placed> {
        self.items.iter().find(|p| p.id == id)
    }

    /// Everything in the world for the budgets.
    pub fn usage(&self, catalog: &Catalog) -> Usage {
        let of = |dropped: bool| self.items.iter().filter(move |p| (p.origin == Origin::Dropped) == dropped).filter_map(|p| catalog.item(&p.item));
        Usage::of(catalog, of(false)) + Usage::placed(catalog, of(true))
    }

    /// The item under `at`: the smallest whose footprint (its template bounds, turned) holds `at` on the ground
    /// plane. NOT RETAIL YET: retail highlights the item its cursor ray meets.
    pub fn pick(&self, catalog: &Catalog, at: Vec3) -> Option<u32> {
        let mut best: Option<(f32, u32)> = None;
        for p in &self.items {
            let Some(item) = catalog.item(&p.item) else { continue };
            let [lo, hi] = item.cost.bounds;
            let local = (p.rotation.inverse() * (at - p.position)) / p.scale;
            if local.x < lo[0] || local.x > hi[0] || local.z < lo[2] || local.z > hi[2] || local.y < lo[1] - 1.0 || local.y > hi[1] + 1.0 {
                continue;
            }
            let area = (hi[0] - lo[0]) * (hi[2] - lo[2]);
            if best.is_none_or(|(a, _)| area < a) {
                best = Some((area, p.id));
            }
        }
        best.map(|(_, id)| id)
    }

    /// Adds a dropped item; returns its id.
    /// Colours an item (`Tuning::colours` index).
    pub fn set_tint(&mut self, id: u32, tint: u8) {
        if let Some(p) = self.items.iter_mut().find(|p| p.id == id) {
            p.tint = tint;
        }
    }

    pub fn drop_item(&mut self, item: &str, position: Vec3, rotation: Quat) -> u32 {
        if self.next < DROPPED_ID_BASE {
            self.next = DROPPED_ID_BASE;
        }
        let id = self.next;
        self.next += 1;
        self.items.push(Placed { id, item: item.to_owned(), position, rotation, scale: Vec3::ONE, origin: Origin::Dropped, tint: 0 });
        id
    }

    pub fn set_pose(&mut self, id: u32, position: Vec3, rotation: Quat) {
        if let Some(p) = self.items.iter_mut().find(|p| p.id == id) {
            p.position = position;
            p.rotation = rotation;
        }
    }

    pub fn remove(&mut self, id: u32) -> bool {
        let before = self.items.len();
        self.items.retain(|p| p.id != id);
        self.items.len() != before
    }

    /// Retail's reset (82C4B780): an authored piece goes back on its layout pose; a dropped item is removed. Returns
    /// whether the item is still there.
    pub fn reset(&mut self, id: u32) -> bool {
        match self.get(id).map(|p| p.origin) {
            Some(Origin::Authored { position, rotation, .. }) => {
                // (The scale never changes in the editor.)
                self.set_pose(id, position, rotation);
                true
            }
            Some(Origin::Dropped) => {
                self.remove(id);
                false
            }
            None => false,
        }
    }
}
