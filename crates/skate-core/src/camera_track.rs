//! ICE camera tracks (the cutscene Scene system's cameras, e.g. the Trick Guide demo's 21 ViewScene tracks in
//! scene.big `data/scene/ice_000.bin`; parsed by `skate_data::ice_camera`). A track keys the camera elements of
//! key group 0: EYE and LOOK (scene space, Z-up), DUTCH (roll), TANGENT_EYE / TANGENT_LOOK and LENS_LENGTH (mm).
//!
//! Sampling ports the TU3 segment setup sub_828F0BB0 and the tangent sub_828F0990: each element is a cubic Hermite
//! per segment, `((a*u + b)*u + c)*u + d` with u the segment fraction, a = m0 + m1 - 2D, b = 3D - m1 - 2*m0, c = m0,
//! d = p0, D = p1 - p0. The tangent at a segment end is the non-uniform Catmull-Rom tangent in the segment's own
//! parameter, `D*dt_n/(dt+dt_n) + D_n*(dt/dt_n)*dt/(dt+dt_n)` with the neighbour segment n (previous for m0, next
//! for m1; none at the track ends: D), scaled by the element's TANGENT value at that key. Each element names its
//! spline switch and tangent element (descriptor +60 / +64, read live in the recomp): EYE, DUTCH and LENS_LENGTH use
//! CUBIC_EYE (87) / TANGENT_EYE (7), LOOK uses CUBIC_LOOK (88) / TANGENT_LOOK (8). A switch is a per-segment value;
//! when it is 0 the tangents are D (linear).
//! Each segment also carries the eye and look space (SPACE_EYE / SPACE_LOOK, 89 / 90) and the followed object
//! (AVATAR_EYE / AVATAR_LOOK, 91 / 92), applied by the scene player (sub_828F2C38 -> sub_828F36B0).
//! Data-driven: tracks come from the disc; a mod can replace or add tracks by index, segment settings included.

/// sub_828F1E80: the film half-width (mm, 0x822F91F4) the lens is measured against.
pub const FILM_HALF_WIDTH_MM: f32 = 15.96;
/// LENS_LENGTH range (element 9): clamped before the angle.
pub const LENS_MIN_MM: f32 = 10.0;
pub const LENS_MAX_MM: f32 = 500.0;
/// LENS_LENGTH default (no key).
pub const LENS_DEFAULT_MM: f32 = 24.0;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CameraKey {
    /// Seconds from the track start.
    pub time: f32,
    pub eye: [f32; 3],
    pub look: [f32; 3],
    pub dutch: f32,
    pub tangent_eye: f32,
    pub tangent_look: f32,
    pub lens_mm: f32,
}

/// Where a camera point lives (SPACE_* with AVATAR_*; sub_828F36B0 modes).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CameraSpace {
    /// 0 the player locator with its orientation, 2 plus an object's position (no rotation), 3 an object's full
    /// matrix; other values have no case (the point stays in scene space).
    pub mode: u8,
    /// The followed object, 1-based (sub_82673560 takes `avatar - 1`): 5 = the scene's first character at height
    /// 0, 7 and up = scene character `avatar - 7` at full height.
    pub avatar: u8,
}

/// Group 0's per-segment values (elements 87..92, one value per segment).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CameraSegment {
    pub cubic_eye: bool,
    pub cubic_look: bool,
    pub eye_space: CameraSpace,
    pub look_space: CameraSpace,
}

impl Default for CameraSegment {
    /// The element defaults (sub_82903840): CUBIC 1, SPACE 0, AVATAR 0.
    fn default() -> Self {
        Self { cubic_eye: true, cubic_look: true, eye_space: CameraSpace::default(), look_space: CameraSpace::default() }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CameraTrack {
    /// Track length in seconds (record +12); retail fits it to the playing clip ("Fixing Camera Group length").
    pub length: f32,
    pub keys: Vec<CameraKey>,
    /// One per segment (keys - 1); missing entries take the defaults.
    pub segments: Vec<CameraSegment>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CameraSample {
    pub eye: [f32; 3],
    pub look: [f32; 3],
    pub dutch: f32,
    pub lens_mm: f32,
    pub eye_space: CameraSpace,
    pub look_space: CameraSpace,
}

impl CameraSample {
    /// sub_828F1E80: `2*atan(15.96 / lens)`, lens clamped to [10, 500] mm.
    pub fn fov_radians(&self) -> f32 {
        2.0 * (FILM_HALF_WIDTH_MM / self.lens_mm.clamp(LENS_MIN_MM, LENS_MAX_MM)).atan()
    }
}

/// One element's Hermite over segment `s` of `keys` (value by key, tangent scale by key); `cubic` off = linear.
fn hermite(times: &[f32], value: impl Fn(usize) -> f32, scale: impl Fn(usize) -> f32, s: usize, u: f32,
    cubic: bool) -> f32 {
    let (p0, p1) = (value(s), value(s + 1));
    let d = p1 - p0;
    if !cubic {
        return p0 + d * u;
    }
    let dt = times[s + 1] - times[s];
    // sub_828F0990: neighbour segment n, its delta and duration.
    let tangent = |n: Option<usize>| -> f32 {
        let Some(n) = n.filter(|n| n + 1 < times.len()) else { return d };
        let (dn, dtn) = (value(n + 1) - value(n), times[n + 1] - times[n]);
        let (mut own, mut other) = (1.0, 0.0);
        if dt + dtn > 0.0 {
            own = dtn / (dt + dtn);
            other = 1.0 - own;
            if dtn > 0.0 {
                other *= dt / dtn;
            }
        }
        dn * other + d * own
    };
    let m0 = tangent(s.checked_sub(1)) * scale(s);
    let m1 = tangent(Some(s + 1)) * scale(s + 1);
    let (a, b, c) = (m0 + m1 - 2.0 * d, 3.0 * d - m1 - 2.0 * m0, m0);
    ((a * u + b) * u + c) * u + p0
}

impl CameraTrack {
    /// The camera at `time` seconds into the track (clamped to the keys).
    pub fn sample(&self, time: f32) -> Option<CameraSample> {
        let keys = &self.keys;
        let first = keys.first()?;
        let segment = |s: usize| self.segments.get(s).copied().unwrap_or_default();
        if keys.len() == 1 || time <= first.time {
            let g = segment(0);
            return Some(CameraSample { eye: first.eye, look: first.look, dutch: first.dutch, lens_mm: first.lens_mm,
                eye_space: g.eye_space, look_space: g.look_space });
        }
        let s = keys.windows(2).position(|w| time < w[1].time).unwrap_or(keys.len() - 2);
        let g = segment(s);
        let span = keys[s + 1].time - keys[s].time;
        let u = if span > 0.0 { ((time - keys[s].time) / span).clamp(0.0, 1.0) } else { 1.0 };
        let times: Vec<f32> = keys.iter().map(|k| k.time).collect();
        let axis = |pick: fn(&CameraKey) -> [f32; 3], tangent: fn(&CameraKey) -> f32, cubic: bool| {
            [0, 1, 2].map(|i| hermite(&times, |k| pick(&keys[k])[i], |k| tangent(&keys[k]), s, u, cubic))
        };
        let eye_tangent = |k: usize| keys[k].tangent_eye;
        Some(CameraSample {
            eye: axis(|k| k.eye, |k| k.tangent_eye, g.cubic_eye),
            look: axis(|k| k.look, |k| k.tangent_look, g.cubic_look),
            dutch: hermite(&times, |k| keys[k].dutch, eye_tangent, s, u, g.cubic_eye),
            lens_mm: hermite(&times, |k| keys[k].lens_mm, eye_tangent, s, u, g.cubic_eye),
            eye_space: g.eye_space,
            look_space: g.look_space,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(time: f32, x: f32, tangent: f32) -> CameraKey {
        CameraKey { time, eye: [x, 0.0, 0.0], look: [x, 1.0, 0.0], tangent_eye: tangent, tangent_look: tangent,
            lens_mm: LENS_DEFAULT_MM, ..Default::default() }
    }

    #[test]
    fn keys_are_hit_and_two_keys_interpolate_linearly() {
        let track = CameraTrack { length: 2.0, keys: vec![key(0.0, 0.0, 1.0), key(2.0, 4.0, 1.0)], ..Default::default() };
        assert_eq!(track.sample(0.0).unwrap().eye[0], 0.0);
        assert!((track.sample(2.0).unwrap().eye[0] - 4.0).abs() < 1e-5);
        // Both tangents are D with no neighbours: the Hermite is the straight line.
        assert!((track.sample(0.5).unwrap().eye[0] - 1.0).abs() < 1e-5);
    }

    #[test]
    fn zero_tangent_eases_in_and_out() {
        let mut track =
            CameraTrack { length: 1.0, keys: vec![key(0.0, 0.0, 0.0), key(1.0, 1.0, 0.0)], ..Default::default() };
        let v = track.sample(0.25).unwrap().eye[0];
        assert!((v - 0.15625).abs() < 1e-5, "{v}");
        // CUBIC_EYE 0 on the segment: linear, the TANGENT value no longer applies.
        track.segments = vec![CameraSegment { cubic_eye: false, ..Default::default() }];
        assert!((track.sample(0.25).unwrap().eye[0] - 0.25).abs() < 1e-5);
    }

    #[test]
    fn dutch_and_lens_follow_the_eye_spline() {
        // DUTCH and LENS_LENGTH share CUBIC_EYE / TANGENT_EYE: a zero TANGENT_EYE eases them like the eye.
        let mut a = key(0.0, 0.0, 0.0);
        let mut b = key(1.0, 1.0, 0.0);
        (a.dutch, b.dutch, a.lens_mm, b.lens_mm) = (0.0, 0.1, 20.0, 40.0);
        let track = CameraTrack { length: 1.0, keys: vec![a, b], ..Default::default() };
        let s = track.sample(0.25).unwrap();
        assert!((s.dutch - 0.015625).abs() < 1e-6, "{}", s.dutch);
        assert!((s.lens_mm - 23.125).abs() < 1e-4, "{}", s.lens_mm);
    }

    #[test]
    fn interior_key_takes_the_catmull_rom_tangent() {
        // Evenly spaced collinear keys: the spline stays on the line.
        let track = CameraTrack { length: 2.0, keys: vec![key(0.0, 0.0, 1.0), key(1.0, 1.0, 1.0), key(2.0, 2.0, 1.0)],
            ..Default::default() };
        for t in [0.25, 0.5, 1.3, 1.9] {
            assert!((track.sample(t).unwrap().eye[0] - t).abs() < 1e-5, "{t}");
        }
    }

    #[test]
    fn lens_gives_the_retail_angle() {
        let s = CameraSample { eye: [0.0; 3], look: [0.0; 3], dutch: 0.0, lens_mm: LENS_DEFAULT_MM,
            eye_space: CameraSpace::default(), look_space: CameraSpace::default() };
        assert!((s.fov_radians().to_degrees() - 67.24).abs() < 0.05);
    }
}
