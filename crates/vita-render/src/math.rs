//! Camera and lighting conventions (the stock camera's output,
//! FOV unchanged).
use glam::{Mat4, Vec3};

pub const WIDTH: i32 = 960;
pub const HEIGHT: i32 = 544;
pub const ASPECT: f32 = WIDTH as f32 / HEIGHT as f32;
/// skate-game uses Bevy's default near plane; the far plane only needs to cover the
/// Course (100 m floor) and, later, a cropped park (about 120 m far clip).
pub const NEAR: f32 = 0.1;
pub const FAR: f32 = 200.0;

/// Vertical-FOV perspective (Bevy's camera projection uses a vertical FOV, which is
/// what the stock camera's `field_of_view_degrees` is).
pub fn projection(fov_y: f32) -> Mat4 {
    Mat4::perspective_rh_gl(fov_y, ASPECT, NEAR, FAR)
}

/// The same with another far plane (a district draws to 250 m).
pub fn projection_far(fov_y: f32, far: f32) -> Mat4 {
    Mat4::perspective_rh_gl(fov_y, ASPECT, NEAR, far)
}

/// View matrix from the camera's world transform (looking along its local -Z, Y up).
pub fn view(camera_world: Mat4) -> Mat4 {
    camera_world.inverse()
}

/// Unit vector towards the sun: skate-game's test world lights the Course from
/// (4, 7, 4) towards the origin.
pub fn light_direction() -> Vec3 {
    Vec3::new(4.0, 7.0, 4.0).normalize()
}

/// Ambient and diffuse weights of the single directional light.
pub const AMBIENT: f32 = 0.42;
pub const DIFFUSE: f32 = 0.58;

/// The background colour (skate-game's clear colour), also the fog colour.
pub const SKY: [f32; 3] = [0.065, 0.08, 0.10];

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec4;

    #[test]
    fn a_point_in_front_of_the_camera_projects_inside_the_view() {
        // Camera at (0, 2, 10) looking down -Z (identity rotation).
        let camera = Mat4::from_translation(Vec3::new(0.0, 2.0, 10.0));
        let clip = projection(1.0) * view(camera) * Vec4::new(0.0, 2.0, 0.0, 1.0);
        let ndc = clip.truncate() / clip.w;
        assert!(ndc.x.abs() < 1e-5 && ndc.y.abs() < 1e-5 && ndc.z > -1.0 && ndc.z < 1.0);
        let behind = projection(1.0) * view(camera) * Vec4::new(0.0, 2.0, 20.0, 1.0);
        assert!(behind.w < 0.0);
    }
}
