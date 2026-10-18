//! Water camera. Retail footage (PS3 version in RPCS3: Aletown canal and the
//! University fountain channel) shows the same shot whenever the skater bails
//! into water: the camera rises high above and behind the skater, looks down
//! about 60 degrees and slowly drifts toward overhead, with a dark rounded
//! vignette. This blends the normal camera into that shot during a water bail
//! (the physics water state, not mere proximity: water surfaces reach under
//! nearby walkways) and back out afterwards; the weight also drives the
//! vignette (`retail_exposure`, tone pass). Separately, a camera under a water
//! surface is lifted just above it. Distances and timings are project choices
//! matched by eye (docs/hails-additions/09-water.md).
use skate_core::{camera::CameraFrame, math::{Basis3, Vector3}, physics::board_world::BoardWorld};

/// Camera height above the water surface.
const HEIGHT: f32 = 3.0;
/// Horizontal distance from the skater: 60 degrees down at entry, drifting to
/// about 73 degrees over `DRIFT` seconds.
const DISTANCE_START: f32 = 1.75;
const DISTANCE_END: f32 = 0.9;
const DRIFT: f32 = 6.0;
/// Blend in and out, seconds.
const EASE_IN: f32 = 0.25;
const EASE_OUT: f32 = 0.5;
/// Minimum height above the surface for a camera that is under water.
const SURFACE_MARGIN: f32 = 0.3;
const MAX_DEPTH: f32 = 6.0;
/// Aim point above the skater root.
const AIM_HEIGHT: f32 = 0.2;
/// Vignette strength at full weight (tone pass).
const VIGNETTE: f32 = 0.85;

#[derive(Default)]
pub(crate) struct WaterView {
    /// Blend toward the water shot, 0..1.
    weight: f32,
    /// Seconds since the water bail started.
    time_in: f32,
    /// Horizontal direction from the skater to the camera, fixed at entry.
    direction: Option<[f32; 2]>,
    /// Water surface of the bail, kept while blending out.
    surface: f32,
    /// Seconds to the next `WATER_CAMERA` log line while in the shot.
    log_timer: f32,
    /// In a water bail last frame (for the in / out log line).
    in_water: bool,
}

impl WaterView {
    /// Vignette amount for the tone pass.
    pub(crate) fn vignette(&self) -> f32 {
        self.weight * VIGNETTE
    }

    /// `water`: the water surface height while the skater is in a water bail.
    pub(crate) fn adjust(
        &mut self,
        mut frame: CameraFrame,
        world: &BoardWorld,
        subject: [f32; 3],
        water: Option<f32>,
        dt: f32,
    ) -> CameraFrame {
        let root = Vector3::new(subject[0], subject[1], subject[2]);
        if water.is_some() != self.in_water {
            self.in_water = water.is_some();
            tracing::info!(
                "WATER_CAMERA {} surface={:?} skater_y={:.2} camera_y={:.2}",
                if water.is_some() { "water bail: shot in" } else { "out of water: shot out" },
                water,
                root.y,
                frame.position[1]
            );
        }
        if let Some(surface) = water {
            self.surface = surface;
            self.time_in += dt.max(0.0);
        }
        let (target, time) = if water.is_some() { (1.0, EASE_IN) } else { (0.0, EASE_OUT) };
        let blend = if dt > 0.0 { 1.0 - (-dt / time).exp() } else { 1.0 };
        self.weight += (target - self.weight) * blend;
        if self.weight < 1e-3 && water.is_none() {
            self.weight = 0.0;
            self.time_in = 0.0;
            self.direction = None;
            return lift_out_of_water(frame, world);
        }

        let eye = [frame.position[0], frame.position[1], frame.position[2]];
        let direction = *self.direction.get_or_insert_with(|| {
            let away = [eye[0] - root.x, eye[2] - root.z];
            let length = away[0].hypot(away[1]);
            if length > 1e-3 {
                [away[0] / length, away[1] / length]
            } else {
                // Camera straight above: back away against its view direction.
                let at = frame.basis.columns[2];
                let length = at[0].hypot(at[2]).max(1e-6);
                [-at[0] / length, -at[2] / length]
            }
        });
        let drift = smoothstep((self.time_in / DRIFT).min(1.0));
        let distance = DISTANCE_START + (DISTANCE_END - DISTANCE_START) * drift;
        let shot = [
            root.x + direction[0] * distance,
            self.surface + HEIGHT,
            root.z + direction[1] * distance,
        ];
        let w = self.weight;
        let position: [f32; 3] = core::array::from_fn(|i| eye[i] + (shot[i] - eye[i]) * w);
        frame.position[..3].copy_from_slice(&position);

        let aim = normalize([
            root.x - position[0],
            root.y + AIM_HEIGHT - position[1],
            root.z - position[2],
        ]);
        let at = frame.basis.columns[2];
        let forward = normalize(core::array::from_fn(|i| at[i] + (aim[i] - at[i]) * w));
        // Columns are [right, up, at] with right = up x at (presentation.rs).
        // Looking nearly straight down, use the shot's horizontal direction.
        let hint = if forward[1].abs() > 0.98 { [-direction[0], 0.0, -direction[1]] } else { [0.0, 1.0, 0.0] };
        let right = normalize(cross(hint, forward));
        if right.iter().all(|v| v.is_finite()) && right != [0.0; 3] {
            let up = cross(forward, right);
            frame.basis = Basis3 { columns: [right, up, forward] };
        }
        let frame = lift_out_of_water(frame, world);
        // About once a second in the shot: where the camera is against the water (logs must diagnose).
        self.log_timer -= dt.max(0.0);
        if water.is_some() && self.log_timer <= 0.0 {
            self.log_timer = 1.0;
            tracing::info!(
                "WATER_CAMERA shot weight={:.2} surface={:.2} skater_y={:.2} camera_y={:.2} above_surface={:.2}",
                self.weight,
                self.surface,
                root.y,
                frame.position[1],
                frame.position[1] - self.surface
            );
        }
        frame
    }
}

/// A camera under a water surface is lifted just above it.
fn lift_out_of_water(mut frame: CameraFrame, world: &BoardWorld) -> CameraFrame {
    let eye = Vector3::new(frame.position[0], frame.position[1], frame.position[2]);
    if let Some(surface) = world.water_surface_at(eye, 0.0, MAX_DEPTH) {
        frame.position[1] = frame.position[1].max(surface + SURFACE_MARGIN);
    }
    frame
}

fn smoothstep(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let length = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if length > 1e-6 { v.map(|x| x / length) } else { [0.0; 3] }
}

#[cfg(test)]
mod tests {
    use super::*;
    use skate_core::physics::{board_world::WorldTriangle, contact::RetailContactMaterial};

    /// A 40 m water triangle (surface type 12) at `height`.
    fn water(height: f32) -> BoardWorld {
        let material = RetailContactMaterial { static_friction: 0.7, dynamic_friction: 0.4, restitution: 0.2 };
        let v = |x: f32, z: f32| Vector3::new(x, height, z);
        let face = WorldTriangle::from_vertices([v(-20., -20.), v(-20., 20.), v(20., -20.)], material, 0x637, 0xe0, [1.; 3], 0.).expect("valid triangle");
        BoardWorld::new(vec![face])
    }

    fn frame(position: [f32; 3]) -> CameraFrame {
        let basis = Basis3 { columns: [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]] };
        CameraFrame {
            basis,
            position: [position[0], position[1], position[2], 0.],
            previous_basis: basis,
            previous_position: [0.; 4],
            linear_velocity: [0.; 4],
            angular_velocity: [0.; 4],
            shake_translation: [0.; 4],
            discontinuity: false,
            field_of_view_degrees: 60.,
            opacity: 1.,
            blur: 1.,
        }
    }

    /// A water bail: the camera never ends under the surface and settles `HEIGHT` above it, looking down at the
    /// skater, with the vignette at full weight; after the bail it blends back out (session 2026-10-13: the camera
    /// went under the water and stayed zoomed in on the skater).
    #[test]
    fn a_water_bail_keeps_the_camera_above_the_surface_and_blends_out() {
        let world = water(9.0);
        let mut view = WaterView::default();
        // The gameplay camera dragged under water next to the sinking skater.
        let under = frame([-1.5, 8.0, -1.5]);
        let skater = [-1.0, 8.2, -1.0];
        let mut out = under;
        for _ in 0..60 {
            out = view.adjust(under, &world, skater, Some(9.0), 1. / 30.);
            assert!(out.position[1] >= 9.0 + SURFACE_MARGIN - 1e-4, "camera under water: {:?}", out.position);
        }
        assert!((out.position[1] - (9.0 + HEIGHT)).abs() < 0.05, "settles {HEIGHT} m above: {:?}", out.position);
        assert!(out.basis.columns[2][1] < -0.5, "looks down at the skater");
        assert!((view.vignette() - VIGNETTE).abs() < 0.01);
        // Out of the bail: back to the gameplay frame (only lifted out of the water) and no vignette.
        for _ in 0..150 {
            out = view.adjust(under, &world, skater, None, 1. / 30.);
        }
        assert_eq!(view.vignette(), 0.);
        assert!((out.position[1] - (9.0 + SURFACE_MARGIN)).abs() < 1e-4);
    }

    /// Without a water bail a camera on dry ground is untouched.
    #[test]
    fn a_dry_camera_is_left_alone() {
        let world = water(9.0);
        let mut view = WaterView::default();
        let dry = frame([50.0, 2.0, 50.0]);
        assert_eq!(view.adjust(dry, &world, [50.0, 1.0, 51.0], None, 1. / 30.), dry);
    }
}
