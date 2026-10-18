//! The part of Bevy's `Transform` that the simulation uses, copied from
//! `bevy_transform` 0.18.1 (`src/components/transform.rs`, MIT OR Apache-2.0) with the
//! ECS/reflection derives and doc links removed. The methods are unchanged and use the
//! same glam version as bevy_math, so results are identical.
use glam::{Mat4, Quat, Vec3};

#[derive(Debug, PartialEq, Clone, Copy)]
pub struct Transform {
    /// Position of the entity.
    pub translation: Vec3,
    /// Rotation of the entity.
    pub rotation: Quat,
    /// Scale of the entity.
    pub scale: Vec3,
}

impl Transform {
    /// An identity [`Transform`] with no translation, rotation, and a scale of 1 on all axes.
    pub const IDENTITY: Self = Transform {
        translation: Vec3::ZERO,
        rotation: Quat::IDENTITY,
        scale: Vec3::ONE,
    };

    /// Extracts the translation, rotation, and scale from `matrix`. It must be a 3d affine
    /// transformation matrix.
    #[inline]
    pub fn from_matrix(world_from_local: Mat4) -> Self {
        let (scale, rotation, translation) = world_from_local.to_scale_rotation_translation();

        Transform {
            translation,
            rotation,
            scale,
        }
    }

    /// Creates a new [`Transform`], with `translation`. Rotation will be 0 and scale 1 on
    /// all axes.
    #[inline]
    pub const fn from_translation(translation: Vec3) -> Self {
        Transform {
            translation,
            ..Self::IDENTITY
        }
    }

    #[inline]
    #[must_use]
    pub const fn with_rotation(mut self, rotation: Quat) -> Self {
        self.rotation = rotation;
        self
    }

    /// Computes the 3d affine transformation matrix from this transform's translation,
    /// rotation, and scale.
    #[inline]
    pub fn to_matrix(&self) -> Mat4 {
        Mat4::from_scale_rotation_translation(self.scale, self.rotation, self.translation)
    }
}

impl Default for Transform {
    fn default() -> Self {
        Self::IDENTITY
    }
}
