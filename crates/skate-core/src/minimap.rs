//! The HUD minimap's rules (retail component `82665278` / `82665388`, research b568): when it shows and the smoothed
//! heading the map turns with. Details: docs/hails-additions/31-retail-menus.md#hud-minimap

/// When the minimap shows (82665278): never with the HUD off (+0x77); game modes 1 and 6 force it on, 2 off; any
/// other mode (Free Play is 3) follows the Game Settings Minimap row (settings +158, default on).
pub fn visible(hud_on: bool, game_mode: i32, setting: bool) -> bool {
    if !hud_on {
        return false;
    }
    match game_mode {
        1 | 6 => true,
        2 => false,
        _ => setting,
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct HeadingRules {
    /// Above this speed the travel direction steers the heading, below it the skater's facing (1.0).
    pub min_speed: f32,
    /// Weight of the old heading per update (0.85; the new direction gets 1 - it).
    pub keep: f32,
}

impl Default for HeadingRules {
    fn default() -> Self {
        Self { min_speed: 1.0, keep: 0.85 }
    }
}

/// The map's up direction on the ground plane (component +0x50), heading-up from the travel direction (82665388).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Heading(pub [f32; 2]);

impl Default for Heading {
    fn default() -> Self {
        Self([0.0, 1.0])
    }
}

fn normalize(v: [f32; 2]) -> Option<[f32; 2]> {
    let l = (v[0] * v[0] + v[1] * v[1]).sqrt();
    (l > 1e-6).then(|| [v[0] / l, v[1] / l])
}

impl Heading {
    /// One update (82665388, b571): moving faster than the minimum, ease towards the travel direction and normalise;
    /// slower, ease towards the camera's forward (x, z) without normalising.
    pub fn step(&mut self, velocity: [f32; 2], camera: [f32; 2], rules: &HeadingRules) {
        let speed = (velocity[0] * velocity[0] + velocity[1] * velocity[1]).sqrt();
        let mix = |a: [f32; 2], b: [f32; 2]| [rules.keep * a[0] + (1.0 - rules.keep) * b[0], rules.keep * a[1] + (1.0 - rules.keep) * b[1]];
        if speed > rules.min_speed {
            let d = [velocity[0] / speed, velocity[1] / speed];
            if let Some(n) = normalize(mix(self.0, d)) {
                self.0 = n;
            }
        } else {
            self.0 = mix(self.0, camera);
        }
    }

    /// The needle's own easing towards the heading (82665848: 0.85 old + 0.15 heading, 0x822250F8 / 0x820994B4).
    pub fn follow(&mut self, target: &Heading, rules: &HeadingRules) {
        self.0 = [rules.keep * self.0[0] + (1.0 - rules.keep) * target.0[0], rules.keep * self.0[1] + (1.0 - rules.keep) * target.0[1]];
    }

    /// The heading's angle from +z towards +x, radians.
    pub fn yaw(&self) -> f32 {
        self.0[0].atan2(self.0[1])
    }
}

/// World north on the ground plane (x, z): -z, the reference 82665848 measures the heading against ((0, 0, -1) from
/// 0x82165A10 and 0x8216DEE0, b571); our importers keep retail's axes.
pub const NORTH: [f32; 2] = [0.0, -1.0];

/// Placement and fade of the icons (8268CB40, b570).
#[derive(Clone, Debug, PartialEq)]
pub struct IconRules {
    /// The map matrix the draw receives (82665388 r5, traced in the recomp 2026-10-23, constant all run): pixels per map
    /// unit (= metre) on x and y, and the disc centre on the 1280 x 720 HUD.
    pub scale: [f32; 2],
    pub centre: [f32; 2],
    /// Icons farther out are held on this radius, map units (every far icon quad's centre sat at 71.28 in the trace;
    /// 0x821E956C = 71.281).
    pub clamp: f32,
    /// Icon size per unit of its scale (0x821735AC = 32).
    pub size: f32,
    /// Fade: full up to 71.281 m (0x821E956C), then 1 - (d - 60) / 290 (0x821FF080, 0x822F959C), gone past the max
    /// distance (default 1000, 0x82256FE8).
    pub fade_full: f32,
    pub fade_start: f32,
    pub fade_span: f32,
    pub max_distance: f32,
}

impl Default for IconRules {
    fn default() -> Self {
        Self { scale: [0.6291, 0.6293], centre: [1128.0, 110.0], clamp: 71.281, size: 32.0, fade_full: 71.281, fade_start: 60.0, fade_span: 290.0, max_distance: 1000.0 }
    }
}

/// One minimap icon (MiniMapManager item, b570): data a group owner or a mod fills.
#[derive(Clone, Debug, PartialEq)]
pub struct Icon {
    /// Stable id (group + key).
    pub id: String,
    /// Texture key (`mini_skater` ..).
    pub texture: String,
    /// World position on the ground plane (x, z).
    pub position: [f32; 2],
    /// Own turn, radians (item +0x20).
    pub angle: f32,
    /// Size factor (item +0x24, default 1).
    pub scale: f32,
    /// Fades with distance (item +0x30).
    pub fade: bool,
    /// The row's max distance (fe_map_icons 0x2CF25B33B439DDCD) when above 0.
    pub max_distance: f32,
    /// RGBA tint (item +0x40).
    pub colour: [f32; 4],
}

/// The distance fade (8268CB40).
pub fn alpha(d: f32, max_distance: f32, rules: &IconRules) -> f32 {
    let max = if max_distance > 0.0 { max_distance } else { rules.max_distance };
    if d > max {
        0.0
    } else if d <= rules.fade_full {
        1.0
    } else {
        (1.0 - (d - rules.fade_start) / rules.fade_span).clamp(0.0, 1.0)
    }
}

/// A ground vector (x, z) on the heading-up map as screen pixels (x right, y down), before the scale.
pub fn to_map(v: [f32; 2], heading: &Heading) -> [f32; 2] {
    let [fx, fz] = heading.0;
    // Right of the heading on the ground (forward x up).
    [-v[0] * fz + v[1] * fx, -(v[0] * fx + v[1] * fz)]
}

/// The needle's turn, degrees clockwise from up: where north lands on the heading-up map.
pub fn needle_degrees(heading: &Heading) -> f32 {
    let [x, y] = to_map(NORTH, heading);
    x.atan2(-y).to_degrees()
}

/// The icon's four corners on the HUD (top-left, top-right, bottom-right, bottom-left) and its alpha, or None when
/// faded out.
pub fn place(icon: &Icon, player: [f32; 2], heading: &Heading, rules: &IconRules) -> Option<([[f32; 2]; 4], f32)> {
    let d = [icon.position[0] - player[0], icon.position[1] - player[1]];
    let a = if icon.fade { alpha((d[0] * d[0] + d[1] * d[1]).sqrt(), icon.max_distance, rules) } else { 1.0 };
    if a <= 0.0 {
        return None;
    }
    let mut at = to_map(d, heading);
    let r = (at[0] * at[0] + at[1] * at[1]).sqrt();
    if r > rules.clamp {
        at = [at[0] * rules.clamp / r, at[1] * rules.clamp / r];
    }
    let half = rules.size * icon.scale / 2.0;
    // The icon turns with the map plus its own angle; map units, then the matrix's scale and centre.
    let turn = icon.angle + needle_degrees(heading).to_radians();
    let (s, c) = turn.sin_cos();
    let corner = |x: f32, y: f32| [rules.centre[0] + (at[0] + x * c - y * s) * rules.scale[0], rules.centre[1] + (at[1] + x * s + y * c) * rules.scale[1]];
    Some(([corner(-half, -half), corner(half, -half), corner(half, half), corner(-half, half)], a))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visibility_follows_hud_mode_and_setting() {
        assert!(!visible(false, 3, true));
        assert!(visible(true, 3, true));
        assert!(!visible(true, 3, false));
        assert!(visible(true, 1, false) && visible(true, 6, false));
        assert!(!visible(true, 2, true));
    }

    #[test]
    fn icons_place_heading_up_clamp_to_the_rim_and_fade() {
        let rules = IconRules::default();
        let north = Heading(NORTH);
        let icon = |x: f32, z: f32| Icon { id: "t".into(), texture: "mini_skater".into(), position: [x, z], angle: 0.0, scale: 1.0, fade: true, max_distance: 0.0, colour: [1.0; 4] };
        // 20 m ahead: 20 x 0.6293 px straight up from the centre, 32 units = 20.1 px wide.
        let (c, a) = place(&icon(0.0, -20.0), [0.0, 0.0], &north, &rules).unwrap();
        let mid = [(c[0][0] + c[2][0]) / 2.0, (c[0][1] + c[2][1]) / 2.0];
        assert!((mid[0] - 1128.0).abs() < 1e-3 && (mid[1] - (110.0 - 20.0 * 0.6293)).abs() < 1e-3);
        assert!((c[1][0] - c[0][0] - 32.0 * 0.6291).abs() < 1e-3);
        assert_eq!(a, 1.0);
        // 200 m to the right: held on the rim, faded.
        let (c, a) = place(&icon(200.0, 0.0), [0.0, 0.0], &north, &rules).unwrap();
        assert!(c[0][0] > 1127.0, "east is right when heading north");
        let mid = [(c[0][0] + c[2][0]) / 2.0, (c[0][1] + c[2][1]) / 2.0];
        assert!(((mid[0] - 1128.0).abs() - 71.281 * 0.6291).abs() < 1e-3);
        assert!((a - (1.0 - 140.0 / 290.0)).abs() < 1e-4);
        assert!(place(&icon(0.0, -1500.0), [0.0, 0.0], &north, &rules).is_none());
        // Heading north (-z): the needle points up; heading +z (south): down; heading +x (east): north on the left.
        assert!(needle_degrees(&north).abs() < 1e-3);
        assert!((needle_degrees(&Heading([0.0, 1.0])).abs() - 180.0).abs() < 1e-3);
        assert!((needle_degrees(&Heading([1.0, 0.0])) + 90.0).abs() < 1e-3);
    }

    #[test]
    fn heading_eases_towards_travel_and_snaps_to_facing_when_slow() {
        let rules = HeadingRules::default();
        let mut h = Heading::default();
        h.step([5.0, 0.0], [0.0, 1.0], &rules);
        let first = h.0;
        assert!(first[0] > 0.0 && first[0] < 0.2 && first[1] > 0.9);
        for _ in 0..200 {
            h.step([5.0, 0.0], [0.0, 1.0], &rules);
        }
        assert!((h.0[0] - 1.0).abs() < 1e-3);
        // Slow: eases towards the camera's forward, not normalised.
        h.step([0.1, 0.0], [0.0, -1.0], &rules);
        assert!((h.0[0] - 0.85).abs() < 1e-3 && (h.0[1] + 0.15).abs() < 1e-3);
        let mut needle = Heading([0.0, -1.0]);
        needle.follow(&Heading([1.0, 0.0]), &rules);
        assert!((needle.0[0] - 0.15).abs() < 1e-6 && (needle.0[1] + 0.85).abs() < 1e-6);
    }
}
