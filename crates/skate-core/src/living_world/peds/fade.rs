//! Ped draw fade: retail never shows a ped pop at the census cull because the ped is already
//! drawn fully transparent by then.
//!
//! Retail reference (TU3, addresses are evidence only):
//! - `sub_827C1188` (per living-world render instance, every frame) [code]: the model record's
//!   pair `Hash_73B6874C7B46C7C6` (near, far) (peds 45 / 55 m [data]) against the distance from
//!   the camera: opacity `1 - clamp((d - near) / (far - near), 0, 1)` beyond `near`, 1 inside.
//!   The pair `Hash_9FCFDBEA56BA4733` (65 / 75) replaces it only when its third float is larger
//!   (`sub_827C1870` lerp); both are 0 for every ped model [data], so 45 / 55 applies.
//! - The same function keeps a spawn fade in at `+576`: `+= 1/30` per console frame
//!   (`0x8232BA4C`), clamped to 1, so about 1 s at the console's 30 fps; the drawn opacity is the
//!   smaller of the two, and below 1 the instance draws blended (`+580`).
//! - The census cull (`sub_826BA8B0`, 70 m slow / 90 m fast around the focused skater) is a pure
//!   3-D distance test with no camera or visibility rule; only security guards hand the
//!   "beyond" flag to their `ISecurityGuard` component (vfunc +4). So the fade above is what hides
//!   the cull from the player.
//!
//! Moddable: the distance pair comes from the model record (a mod model sets its own), the
//! defaults and the fade in time are plain data in [`PedFadeConfig`].

/// Fade defaults (retail values; mods and settings may override them).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PedFadeConfig {
    /// Camera distance pair (near, far) used when a model record has none [data, peds 45 / 55].
    pub distance: [f32; 2],
    /// Spawn fade in (s): 1/30 per console frame at ~30 fps = 1.0 s [code `0x8232BA4C`].
    pub fade_in_seconds: f32,
    /// Off switch for mods that want the pop (retail has a global gate byte, normally on).
    pub enabled: bool,
}

impl Default for PedFadeConfig {
    fn default() -> Self {
        Self { distance: [45.0, 55.0], fade_in_seconds: 1.0, enabled: true }
    }
}

/// Opacity from the camera distance: 1 up to `near`, linear to 0 at `far` (`sub_827C1188`).
/// A pair with `far <= near` never fades (retail divides only when `far - near > 0`, else the
/// fade term stays 0).
pub fn distance_alpha(distance: f32, pair: [f32; 2]) -> f32 {
    let [near, far] = pair;
    let span = far - near;
    if !(distance > near) || !(span > 0.0) {
        return 1.0;
    }
    1.0 - ((distance - near) / span).clamp(0.0, 1.0)
}

/// Drawn opacity of a ped: the smaller of the distance fade and the spawn fade in.
/// `since_spawn` is in seconds of world time (deterministic from the spawn tick).
pub fn draw_alpha(cfg: &PedFadeConfig, pair: Option<[f32; 2]>, distance: f32, since_spawn: f32) -> f32 {
    if !cfg.enabled {
        return 1.0;
    }
    let by_distance = distance_alpha(distance, pair.unwrap_or(cfg.distance));
    let fade_in = if cfg.fade_in_seconds > 0.0 { (since_spawn / cfg.fade_in_seconds).clamp(0.0, 1.0) } else { 1.0 };
    by_distance.min(fade_in)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ped_fade_follows_the_model_pair_like_sub_827c1188() {
        let cfg = PedFadeConfig::default();
        let a = |d: f32| draw_alpha(&cfg, Some([45.0, 55.0]), d, 10.0);
        assert_eq!(a(10.0), 1.0);
        assert_eq!(a(45.0), 1.0);
        assert!((a(50.0) - 0.5).abs() < 1e-6);
        assert_eq!(a(55.0), 0.0);
        assert_eq!(a(200.0), 0.0);
        // A record without a pair uses the configured default (retail 45 / 55).
        assert!((draw_alpha(&cfg, None, 50.0, 10.0) - 0.5).abs() < 1e-6);
        // Broken pair (far <= near): retail never fades.
        assert_eq!(distance_alpha(80.0, [60.0, 60.0]), 1.0);
    }

    #[test]
    fn peds_are_invisible_before_the_census_cull_can_remove_them() {
        // Exported `livingworld_census_ranges` pedestrians [data]: slow circle cull 70 m with no
        // forward offset, fast circle cull 90 m with 20 m offset. The behind-the-player edge of
        // either circle is 70 m from the skater; the camera sits near the skater, so a ped is
        // fully faded long before the cull (the old drawing kept it solid to 70 m and popped it).
        let cfg = PedFadeConfig::default();
        for cull_edge in [70.0f32, 90.0 - 20.0] {
            for camera_slack in [0.0f32, 5.0, 10.0] {
                let seen_at = cull_edge - camera_slack;
                assert_eq!(draw_alpha(&cfg, Some([45.0, 55.0]), seen_at, 10.0), 0.0, "cull edge {cull_edge} m, camera {camera_slack} m nearer");
            }
        }
    }

    #[test]
    fn ped_spawn_fades_in_over_a_second_and_mods_can_change_or_disable_it() {
        let cfg = PedFadeConfig::default();
        assert_eq!(draw_alpha(&cfg, Some([45.0, 55.0]), 10.0, 0.0), 0.0);
        assert!((draw_alpha(&cfg, Some([45.0, 55.0]), 10.0, 0.5) - 0.5).abs() < 1e-6);
        assert_eq!(draw_alpha(&cfg, Some([45.0, 55.0]), 10.0, 1.0), 1.0);
        // The smaller of fade in and distance fade.
        assert!((draw_alpha(&cfg, Some([45.0, 55.0]), 52.5, 0.5) - 0.25).abs() < 1e-6);
        let off = PedFadeConfig { enabled: false, ..cfg };
        assert_eq!(draw_alpha(&off, Some([45.0, 55.0]), 80.0, 0.0), 1.0);
        let far = PedFadeConfig { distance: [100.0, 120.0], fade_in_seconds: 0.0, ..cfg };
        assert_eq!(draw_alpha(&far, None, 80.0, 0.0), 1.0);
    }
}
