//! The ped scatter alarm (`sub_826C1348`, b654): docs/hails-additions/26g-living-world-ped-interactions.md#scatter-alarm

/// One world step of the alarm timer: off in zombie mode (timer untouched); a raised alarm (the local player's D-pad
/// Down gesture) sets it to `length` (assigned, not added), then it runs down by `seconds`. Scatter while it is above 0.
pub fn step(timer: &mut f32, raised: bool, zombie: bool, seconds: f32, length: f32) -> bool {
    if zombie {
        return false;
    }
    if raised {
        *timer = length;
    }
    *timer = (*timer - seconds).max(0.0);
    *timer > 0.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_alarm_holds_while_raised_runs_180_s_and_is_off_in_zombie_mode() {
        let mut t = 0.0;
        assert!(!step(&mut t, false, false, 1.0 / 60.0, 180.0), "never raised");
        for _ in 0..30 {
            assert!(step(&mut t, true, false, 1.0 / 60.0, 180.0));
        }
        assert!((t - (180.0 - 1.0 / 60.0)).abs() < 1e-4, "raised steps assign, not add: {t}");
        let mut on = 0;
        while step(&mut t, false, false, 1.0 / 60.0, 180.0) {
            on += 1;
        }
        assert!((10_790..=10_800).contains(&on), "about 180 s at 60 steps a second: {on}");
        let mut z = 50.0;
        assert!(!step(&mut z, true, true, 1.0 / 60.0, 180.0));
        assert_eq!(z, 50.0, "zombie mode leaves the timer alone");
    }
}
