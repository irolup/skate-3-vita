//! The skater's skin palette from the simulation's presentation snapshots, as
//! skate-game's `animation::present` poses the GLB:
//! each skin joint's bone-local transform is blended between the two snapshots
//! (translation and scale lerped, rotation slerped), composed down the GLB joint
//! hierarchy, then the blended root and the inverse bind are applied.
use glam::{Mat4, Vec4};
use skate_sim::live::{Snapshot, Transform, blend};
use vita_pack::skater::Joint;

/// skate-game `render_basis`: the GLB was exported through Blender, so its bone-local
/// axes are rotated -90° about X relative to the native frames (a skin basis change,
/// not a world rotation).
pub fn render_basis() -> Mat4 {
    Mat4::from_cols(Vec4::X, -Vec4::Z, Vec4::Y, Vec4::W)
}

pub struct Binding {
    /// Stock rig bone of each skin joint (skate-game `bind`: by name, ignoring case).
    bones: Vec<usize>,
    /// Parent skin joint (always earlier in the list).
    parents: Vec<Option<usize>>,
    inverse_binds: Vec<Mat4>,
    globals: Vec<Mat4>,
}

impl Binding {
    pub fn new(joints: &[Joint], bone_names: &[String]) -> Result<Self, String> {
        let mut bones = Vec::with_capacity(joints.len());
        for joint in joints {
            bones.push(
                bone_names
                    .iter()
                    .position(|n| n.eq_ignore_ascii_case(&joint.name))
                    .ok_or_else(|| format!("skater skin bone {} is absent from the stock rig", joint.name))?,
            );
        }
        Ok(Self {
            bones,
            parents: joints.iter().map(|j| j.parent.map(usize::from)).collect(),
            inverse_binds: joints.iter().map(|j| Mat4::from_cols_array(&j.inverse_bind)).collect(),
            globals: Vec::with_capacity(joints.len()),
        })
    }

    pub fn len(&self) -> usize {
        self.bones.len()
    }

    /// Writes `root * joint global * inverse bind` per skin joint into `palette`.
    pub fn palette(&mut self, a: &Snapshot, b: &Snapshot, alpha: f32, palette: &mut Vec<Mat4>) {
        let basis = render_basis();
        let root = blend(a.root, b.root, alpha).to_matrix();
        self.globals.clear();
        palette.clear();
        for j in 0..self.bones.len() {
            let local = |s: &Snapshot| {
                let global = s.bones[self.bones[j]] * basis;
                Transform::from_matrix(match self.parents[j] {
                    Some(p) => (s.bones[self.bones[p]] * basis).inverse() * global,
                    None => global,
                })
            };
            let l = blend(local(a), local(b), alpha).to_matrix();
            let g = match self.parents[j] {
                Some(p) => self.globals[p] * l,
                None => l,
            };
            self.globals.push(g);
            palette.push(root * g * self.inverse_binds[j]);
        }
    }
}

/// The blended camera: world transform and vertical FOV (radians).
pub fn camera(a: &Snapshot, b: &Snapshot, alpha: f32) -> (Mat4, f32) {
    (
        blend(a.camera, b.camera, alpha).to_matrix(),
        a.fov + (b.fov - a.fov) * alpha,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::{Quat, Vec3};

    /// A two-bone chain: the parent turns about Y, the child keeps a constant local
    /// offset (0.2, 0.5, 0) and turns again, as rigid bones do.
    fn snapshot(turn: f32) -> Snapshot {
        let parent = Mat4::from_rotation_translation(Quat::from_rotation_y(turn), Vec3::new(0.0, 1.0, 0.0));
        let child = parent * Mat4::from_rotation_translation(Quat::from_rotation_y(turn), Vec3::new(0.2, 0.5, 0.0));
        let bones = vec![Mat4::IDENTITY /* TRAJECTORY */, parent, child];
        Snapshot {
            root: Transform::from_translation(Vec3::new(3.0, 0.0, -2.0)),
            bones,
            camera: Transform::IDENTITY,
            fov: 0.8,
        }
    }

    fn joints() -> Vec<Joint> {
        let ibm = |m: Mat4| m.to_cols_array();
        vec![
            Joint { name: "hips".into(), parent: None, inverse_bind: ibm(Mat4::from_translation(Vec3::new(0.0, -1.0, 0.0))) },
            Joint { name: "SPINE".into(), parent: Some(0), inverse_bind: ibm(Mat4::from_translation(Vec3::new(0.0, -1.5, 0.0))) },
        ]
    }

    #[test]
    fn unblended_palette_is_root_times_bone_times_basis_times_inverse_bind() {
        let names: Vec<String> = ["TRAJECTORY", "HIPS", "SPINE"].map(String::from).to_vec();
        let mut binding = Binding::new(&joints(), &names).unwrap();
        let s = snapshot(0.4);
        let mut palette = Vec::new();
        binding.palette(&s, &s, 0.3, &mut palette);
        for (j, bone) in [1usize, 2].into_iter().enumerate() {
            let expected = s.root.to_matrix() * s.bones[bone] * render_basis()
                * Mat4::from_cols_array(&joints()[j].inverse_bind);
            assert!(palette[j].abs_diff_eq(expected, 1e-5), "joint {j}: {:?} vs {expected:?}", palette[j]);
        }
    }

    #[test]
    fn blending_keeps_the_hierarchy_between_the_snapshots() {
        let names: Vec<String> = ["TRAJECTORY", "HIPS", "SPINE"].map(String::from).to_vec();
        let mut binding = Binding::new(&joints(), &names).unwrap();
        let (a, b) = (snapshot(0.0), snapshot(0.6));
        let mut palette = Vec::new();
        binding.palette(&a, &b, 0.5, &mut palette);
        // The child's world position stays at its bone length from the parent.
        let pos = |m: Mat4, j: usize| (m * Mat4::from_cols_array(&joints()[j].inverse_bind).inverse()).w_axis.truncate();
        let d = pos(palette[1], 1).distance(pos(palette[0], 0));
        let rest = Vec3::new(0.2, 0.5, 0.0).length();
        assert!((d - rest).abs() < 1e-4, "{d} vs {rest}");
        let (_, fov) = camera(&a, &b, 0.5);
        assert!((fov - 0.8).abs() < 1e-6);
    }

    #[test]
    fn unknown_joint_names_are_rejected() {
        let names = vec!["HIPS".to_string()];
        assert!(Binding::new(&joints(), &names).is_err());
    }
}

/// A ped model bound to the ped rig (upstream `present_ped_looks` / `present_ped_pose`): each
/// skin joint's rig bone by name, the rig bones the clips do not carry following their rig parent
/// with the GLB bind offset (`follower_offsets`), the inverse binds. The bone-local basis is the
/// identity, upstream's `ped_bone_basis` pick for every shipped ped model.
pub struct PedBinding {
    bones: Vec<Option<usize>>,
    followers: Vec<(usize, usize, Mat4)>,
    inverse_binds: Vec<Mat4>,
    globals: Vec<Mat4>,
}

impl PedBinding {
    pub fn new(joints: &[Joint], names: &[String], parents: &[i32], animated: &[bool]) -> Self {
        let bones: Vec<Option<usize>> = joints.iter().map(|j| names.iter().position(|n| n.eq_ignore_ascii_case(&j.name))).collect();
        let inverse_binds: Vec<Mat4> = joints.iter().map(|j| Mat4::from_cols_array(&j.inverse_bind)).collect();
        let mut bind = std::collections::BTreeMap::new();
        for (j, bone) in bones.iter().enumerate() {
            if let Some(b) = bone {
                bind.insert(*b, inverse_binds[j].inverse());
            }
        }
        let followers = (0..names.len())
            .filter(|&i| !animated.get(i).copied().unwrap_or(false))
            .filter_map(|i| {
                let p = usize::try_from(*parents.get(i)?).ok()?;
                let (bp, bi): (&Mat4, &Mat4) = (bind.get(&p)?, bind.get(&i)?);
                Some((i, p, bp.inverse() * *bi))
            })
            .collect();
        Self { bones, followers, inverse_binds, globals: Vec::new() }
    }

    /// The skin palette of a ped between its two step poses (`alpha` of the way): root (drawn
    /// feet, heading) x rig global x inverse bind per joint.
    /// The held hand prop's world frame for the pose [`Self::palette`] last blended: the blended
    /// root, the rig bone `bone`'s blended model-space frame, the prop's `local` frame in it.
    pub fn hand_world(&self, pose: &skate_sim::PedPose, alpha: f32, bone: usize, local: Mat4) -> Option<Mat4> {
        let (p0, h0, _) = &pose.previous;
        let (p1, h1, _) = &pose.current;
        let mut turn = h1 - h0;
        while turn > std::f32::consts::PI {
            turn -= std::f32::consts::TAU;
        }
        while turn < -std::f32::consts::PI {
            turn += std::f32::consts::TAU;
        }
        let root = Mat4::from_rotation_translation(glam::Quat::from_rotation_y(h0 + turn * alpha), p0.lerp(*p1, alpha));
        Some(root * *self.globals.get(bone)? * local)
    }

    pub fn palette(&mut self, pose: &skate_sim::PedPose, alpha: f32, palette: &mut Vec<Mat4>) {
        let (p0, h0, g0) = &pose.previous;
        let (p1, h1, g1) = &pose.current;
        let mut turn = h1 - h0;
        while turn > std::f32::consts::PI {
            turn -= std::f32::consts::TAU;
        }
        while turn < -std::f32::consts::PI {
            turn += std::f32::consts::TAU;
        }
        let root = Mat4::from_rotation_translation(glam::Quat::from_rotation_y(h0 + turn * alpha), p0.lerp(*p1, alpha));
        self.globals.clear();
        if g0.len() == g1.len() {
            self.globals.extend(g0.iter().zip(g1).map(|(a, b)| *a * (1.0 - alpha) + *b * alpha));
        } else {
            self.globals.extend_from_slice(g1);
        }
        for &(i, p, offset) in &self.followers {
            if p < self.globals.len() && i < self.globals.len() {
                self.globals[i] = self.globals[p] * offset;
            }
        }
        palette.clear();
        for (j, bone) in self.bones.iter().enumerate() {
            let global = bone.and_then(|b| self.globals.get(b).copied()).unwrap_or_else(|| self.inverse_binds[j].inverse());
            palette.push(root * global * self.inverse_binds[j]);
        }
    }
}
