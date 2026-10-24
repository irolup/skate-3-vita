//! The called skaters' trails (skate-game `visual_indicators.rs` `companion_trails`, PR #52 `36018e19`; research b566 /
//! b567): one trail per called skater (retail keeps 4), fed every 0.05 s with its board bone (`Skateboard_Root` +
//! 0.25 m, `skate_core::indicator_trail`), drawn as retail's B-spline with the teleporter icons' passes and the
//! `trail_follow` style (`teleporter_icons::Icons::trail_mesh`); hidden behind the camera, past the style's fade and
//! below 4 points.
//!
//! Ours: the board bone from the NPC's published step poses (the stock rig's order, relative to the root), interpolated; the
//! mesh goes to the GPU only when a point was added (`vita_render::scene::Scene::set_trail`).
use crate::teleporter_icons::Icons;
use glam::{Mat4, Vec3};
use skate_core::indicator_trail::{Trail, TrailRules};
use vita_render::scene::{Scene, TrailInstance, TRAIL_SLOTS};

#[derive(Default)]
pub struct CalledTrails {
    /// By GPU slot: the called skater's NPC serial and its trail.
    slots: [Option<(u32, Trail)>; TRAIL_SLOTS],
    rules: TrailRules,
}

impl CalledTrails {
    /// One frame: the called skaters (NPC serials), their published poses at `alpha`, `dt` seconds of play (0 while
    /// paused), the rig's bone names and the camera (world matrix, forward -z). Changed meshes are uploaded to `scene`;
    /// the trails to draw go to `out`.
    #[allow(clippy::too_many_arguments)]
    pub fn update(&mut self, icons: &Icons, called: &[u32], poses: &[skate_sim::NpcPose], alpha: f32, dt: f32, rig: &[String], camera: &Mat4, scene: &mut Scene, out: &mut Vec<TrailInstance>) {
        out.clear();
        for (i, slot) in self.slots.iter_mut().enumerate() {
            if slot.as_ref().is_some_and(|(serial, _)| !called.contains(serial)) {
                *slot = None;
                scene.set_trail(i, &[], &[]);
            }
        }
        for &serial in called {
            if self.slots.iter().flatten().any(|(s, _)| *s == serial) {
                continue;
            }
            if let Some(free) = self.slots.iter_mut().find(|s| s.is_none()) {
                let bone = rig.iter().position(|n| n.eq_ignore_ascii_case(&self.rules.bone));
                println!("called skater #{serial}: trail on bone {bone:?} of {}", rig.len());
                *free = Some((serial, Trail::default()));
            }
        }
        let bone = rig.iter().position(|n| n.eq_ignore_ascii_case(&self.rules.bone));
        let eye = camera.w_axis.truncate();
        let forward = -camera.z_axis.truncate().normalize_or_zero();
        for (i, slot) in self.slots.iter_mut().enumerate() {
            let Some((serial, trail)) = slot else { continue };
            let Some(pose) = poses.iter().find(|p| p.serial == *serial) else { continue };
            // The bones are relative to the root (`skin::Binding::palette`: root x bone).
            let at = |s: &skate_sim::live::Snapshot| {
                let local = bone.and_then(|b| s.bones.get(b)).map_or(Vec3::ZERO, |m| m.w_axis.truncate());
                s.root.to_matrix().transform_point3(local)
            };
            let anchor: Vec3 = at(&pose.previous).lerp(at(&pose.current), alpha);
            if trail.step(dt, anchor.to_array(), &self.rules) {
                let (vertices, indices) = icons.trail_mesh(trail);
                scene.set_trail(i, &vertices, &indices);
            }
            if let Some((front, background)) = icons.trail_factors(trail, (anchor - eye).dot(forward)) {
                out.push(TrailInstance { slot: i, front, background });
            }
        }
    }

    /// A new world: every trail starts over.
    pub fn clear(&mut self, scene: &mut Scene) {
        for (i, slot) in self.slots.iter_mut().enumerate() {
            if slot.take().is_some() {
                scene.set_trail(i, &[], &[]);
            }
        }
    }
}
