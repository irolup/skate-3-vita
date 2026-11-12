//! Props in water, Better Water only (not retail: a retail prop falls through water to the floor, no lift or drag).
//! A prop sinks or floats by its own density (mass over box volume) against the water's, with linear and angular
//! water drag:
//! docs/hails-additions/09-water.md#props-in-water
//! skate_vita: the logic only. The Vita has no Better Water setting, so the props' water stays off (, retail);
//! the Bevy systems that feed it the player's water look and fade the meshes are not ported.
use skate_core::math::Vector3;

/// Water values (`WaterLook` prop_water fields).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PropWater {
    /// kg/m3.
    pub density: f32,
    /// Share of the submerged part's velocity removed per second.
    pub drag: f32,
    pub spin_drag: f32,
    /// Deep water (deeper than the prop is tall) carries every prop: its density counts as at most this (kg/m3).
    /// Shallow water leaves the prop's own density: heavy ones sink, light ones float near the top.
    pub deep_float_density: f32,
    /// Wave height (m, crest to mean) and period (s) of the surface props bob on in deep water; 0 height = flat.
    pub bob_height: f32,
    pub bob_period: f32,
    /// Material density (kg/m3) from which a prop always sinks, deep water or not.
    pub heavy_density: f32,
    /// How far under the surface (m) a sinking prop's top has gone when it has faded out, in water too deep to see
    /// the bottom; 0 = no fade.
    pub fade_depth: f32,
    /// Below this speed a prop denser than the water that touched the floor last step gets no water forces (m/s).
    pub settle_speed: f32,
}

/// Opacity of a prop whose top is at `top` under a surface at `surface`: 1 at the surface, 0 at `fade_depth` under.
pub(crate) fn fade(water: &PropWater, surface: f32, top: f32) -> f32 {
    if water.fade_depth <= 0.0 {
        return 1.0;
    }
    (1.0 - (surface - top) / water.fade_depth).clamp(0.0, 1.0)
}

/// The material density (kg/m3) of a prop type by its template name, when no `water_density` tuning sets it. Our
/// table (retail has no densities; its masses are pushing masses): bags, cones, wood and cardboard float, bins,
/// dumpsters and benches sink in shallow water, solid street furniture sinks fast.
pub(crate) fn density_for_template(template: &str) -> f32 {
    let name = template.to_ascii_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| name.contains(w));
    if has(&["bag", "paper", "cardboard"]) {
        300.0
    } else if has(&["cone", "pallet", "crate", "wood", "box", "plank", "barrier"]) {
        600.0
    } else if has(&["dumpster"]) {
        2400.0
    } else if has(&["bin", "trash", "garbage", "barrel", "can", "bench", "chair", "table", "cart"]) {
        1400.0
    } else if has(&["hydrant", "newspaper", "vending", "mailbox", "rack", "sign", "bollard", "planter", "pot"]) {
        2500.0
    } else {
        1200.0
    }
}

/// Height of the bobbing surface over the flat water at `x, z` and time `t` (s): two crossing swells, so props a few
/// metres apart do not move together.
pub(crate) fn bob(water: &PropWater, x: f32, z: f32, t: f32) -> f32 {
    if water.bob_height <= 0.0 || water.bob_period <= 0.0 {
        return 0.0;
    }
    let w = std::f32::consts::TAU / water.bob_period;
    water.bob_height * (0.65 * (w * t + 0.35 * x + 0.21 * z).sin() + 0.35 * (1.6 * w * t - 0.18 * x + 0.43 * z).sin())
}

/// How far above the surface a body still counts (m) and the deepest surface looked for under it.
pub(crate) const SURFACE_ABOVE: f32 = 0.0;
pub(crate) const SURFACE_DEPTH: f32 = 50.0;

/// Downward speed (m/s) below which a prop entering water makes no splash.
pub(crate) const SPLASH_SPEED: f32 = 1.5;

/// One eighth of the box: its centre (world) and how much of it is under water (0..1).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Sample {
    pub point: Vector3,
    pub submerged: f32,
}

/// The octant centres of a box (centre, basis columns, half extents) and their submerged shares under `surface`
/// (a height per point, `None` = no water there). `height` is the box's world vertical half extent: each octant
/// spans that much vertically.
pub(crate) fn samples(center: Vector3, columns: [[f32; 3]; 3], half: Vector3, height: f32, surface: impl Fn(Vector3) -> Option<f32>) -> [Sample; 8] {
    let mut out = [Sample { point: center, submerged: 0.0 }; 8];
    let span = height.max(1e-3);
    for (i, slot) in out.iter_mut().enumerate() {
        let s = [if i & 1 == 0 { -0.5 } else { 0.5 }, if i & 2 == 0 { -0.5 } else { 0.5 }, if i & 4 == 0 { -0.5 } else { 0.5 }];
        let local = [s[0] * half.x, s[1] * half.y, s[2] * half.z];
        let p = Vector3::new(
            center.x + columns[0][0] * local[0] + columns[1][0] * local[1] + columns[2][0] * local[2],
            center.y + columns[0][1] * local[0] + columns[1][1] * local[1] + columns[2][1] * local[2],
            center.z + columns[0][2] * local[0] + columns[1][2] * local[1] + columns[2][2] * local[2],
        );
        let submerged = surface(p).map_or(0.0, |h| ((h - p.y) / span + 0.5).clamp(0.0, 1.0));
        *slot = Sample { point: p, submerged };
    }
    out
}

/// The impulse on one sample over `dt`: buoyancy of its share of the box volume (up, against `gravity`) and drag at
/// the sample's velocity, scaled by the water it displaces (a light float settles as fast as a heavy one sinks
/// smoothly), at most stopping the sample's share of the mass within the step.
pub(crate) fn sample_impulse(water: &PropWater, sample: Sample, volume: f32, mass: f32, velocity: Vector3, gravity: Vector3, dt: f32) -> Vector3 {
    let s = sample.submerged;
    if s <= 0.0 {
        return Vector3::ZERO;
    }
    let displaced = water.density * volume / 8.0 * s;
    let lift = displaced * dt;
    let share = mass / 8.0;
    let drag = share * (water.drag * displaced / share.max(1e-6) * dt).min(1.0);
    Vector3::new(
        -gravity.x * lift - drag * velocity.x,
        -gravity.y * lift - drag * velocity.y,
        -gravity.z * lift - drag * velocity.z,
    )
}

/// Angular velocity after the spin drag of the submerged share over `dt` (implicit, never reverses).
pub(crate) fn spin_damped(water: &PropWater, angular: Vector3, submerged: f32, dt: f32) -> Vector3 {
    let k = 1.0 / (1.0 + water.spin_drag * submerged * dt);
    Vector3::new(angular.x * k, angular.y * k, angular.z * k)
}

#[cfg(test)]
mod tests {
    use super::*;

    const UP: [[f32; 3]; 3] = [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]];
    const G: Vector3 = Vector3::new(0., -9.81, 0.);
    const WATER: PropWater = PropWater { density: 1000., drag: 1.5, spin_drag: 1.5, deep_float_density: 700., bob_height: 0., bob_period: 3., heavy_density: 2000., fade_depth: 1.5, settle_speed: 0.2 };

    /// A 1 m cube of `density` dropped into still water at y = 0, stepped at 60 Hz with gravity and the water impulses.
    fn settle(density: f32) -> f32 {
        let half = Vector3::new(0.5, 0.5, 0.5);
        let (volume, mass, dt) = (1.0, density, 1.0 / 60.0);
        let (mut y, mut v) = (2.0f32, 0.0f32);
        for _ in 0..60 * 30 {
            let s = samples(Vector3::new(0., y, 0.), UP, half, 0.5, |p| (p.y < 0.5).then_some(0.0));
            let j: f32 = s.iter().map(|s| sample_impulse(&WATER, *s, volume, mass, Vector3::new(0., v, 0.), G, dt).y).sum();
            v += G.y * dt + j / mass;
            y += v * dt;
            if y < -20. {
                break;
            }
        }
        y
    }

    #[test]
    fn denser_than_water_sinks_lighter_floats_at_its_density_share() {
        assert!(settle(2400.) < -10., "concrete sinks");
        // Archimedes: a floating cube sinks to (density / water) of its height; the centre sits 0.5 - share above 0.
        for (density, share) in [(500f32, 0.5f32), (250., 0.25), (800., 0.8)] {
            let y = settle(density);
            assert!((y - (0.5 - share)).abs() < 0.05, "{density} kg/m3 floats at {y:.3}, expected {:.3}", 0.5 - share);
        }
    }

    #[test]
    fn out_of_water_gets_nothing_and_drag_opposes_motion() {
        let dry = samples(Vector3::new(0., 5., 0.), UP, Vector3::new(0.5, 0.5, 0.5), 0.5, |_| None);
        assert!(dry.iter().all(|s| s.submerged == 0.0));
        let wet = Sample { point: Vector3::ZERO, submerged: 1.0 };
        let j = sample_impulse(&WATER, wet, 1.0, 800.0, Vector3::new(2., 0., 0.), G, 0.1);
        assert!(j.x < 0.0 && j.y > 0.0, "drag against the motion, lift up: {j:?}");
        // A very light body is stopped at most, never pushed backwards.
        let k = sample_impulse(&PropWater { drag: 20., spin_drag: 0., ..WATER }, wet, 1.0, 1.0, Vector3::new(2., 0., 0.), Vector3::ZERO, 0.1);
        assert!((k.x + 2.0 / 8.0).abs() < 1e-5, "{k:?}");
        let w = spin_damped(&WATER, Vector3::new(4., 0., 0.), 1.0, 1.0 / 60.0);
        assert!(w.x > 0.0 && w.x < 4.0);
    }

    #[test]
    fn densities_by_name_and_a_bounded_moving_bob() {
        assert!(density_for_template("GarbageBag_DMO_1007") < 1000.0, "bags float");
        assert!(density_for_template("DMO_DT_DumpsterLargeBlue_1005") > 1000.0, "dumpsters sink in shallow water");
        assert!(density_for_template("anything_else") > 1000.0);
        let water = PropWater { bob_height: 0.06, ..WATER };
        let heights: Vec<f32> = (0..300).map(|i| bob(&water, 3.0, -2.0, i as f32 / 60.0)).collect();
        assert!(heights.iter().all(|h| h.abs() <= 0.0601));
        assert!(heights.iter().cloned().fold(f32::MIN, f32::max) - heights.iter().cloned().fold(f32::MAX, f32::min) > 0.05, "it moves");
        assert_eq!(bob(&WATER, 3.0, -2.0, 1.0), 0.0, "zero height = flat");
    }

    #[test]
    fn a_tilted_half_submerged_box_gets_more_lift_on_its_low_side() {
        let c = std::f32::consts::FRAC_1_SQRT_2;
        let tilted = [[c, c, 0.], [-c, c, 0.], [0., 0., 1.]];
        let s = samples(Vector3::ZERO, tilted, Vector3::new(1.0, 0.25, 0.5), 0.88, |_| Some(0.0));
        let low = s.iter().filter(|s| s.point.x < 0.0).map(|s| s.submerged).sum::<f32>();
        let high = s.iter().filter(|s| s.point.x > 0.0).map(|s| s.submerged).sum::<f32>();
        assert!(low > high, "the end under the surface lifts more: {low} vs {high}");
    }
}
