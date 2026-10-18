//! Road-side stop spots for pulling over and the lane-change gap check (retail TU3, evidence only;
//! re-implemented; `.local/research/npc/b69-traffic-manoeuvres.md` "b71 extension", main checked the passage
//! step 82C3C3C0):
//! - `sub_82E151D0` / `sub_82E15368` keep one (car, stop distance) pair per pulling-over car on a road, sorted by
//!   stop distance, far end first; only other cars' spot searches read them;
//! - `sub_82E14CF8` searches a spot up to 0.75 x the road length: the middle of the road when nothing is reserved,
//!   else the middle of the first gap (walking from the far end) longer than the car plus 5 m and beyond
//!   d + 1.5 x the car's length + speed; the gap behind the last reservation is never tried;
//! - `sub_82E14928` checks a lane-change gap: not near either road end, and one second of travel plus the lengths
//!   clear of the cars ahead and behind on the target lane.
//!
//! Multiplayer: [`Reservations`] is plain per-road data the host owns.

/// Retail numbers (code constants); a mod may override them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpotParams {
    /// The search window ends at this fraction of the road length (`0x821814A0`, 0.75).
    pub window_end: f32,
    /// Window start used when nothing is reserved (`0x820C6D98`, 0.25).
    pub window_start: f32,
    /// A gap must exceed the car's length plus this (`0x821F1790`, 5.0 m).
    pub extra_gap: f32,
    /// The spot must lie beyond d + this x the car's length + speed (`0x822249B4`, 1.5).
    pub reach_lengths: f32,
    /// Half-length factor of the gap check (`0x8209975C`, 0.5).
    pub half: f32,
}

impl Default for SpotParams {
    fn default() -> Self {
        Self { window_end: 0.75, window_start: 0.25, extra_gap: 5.0, reach_lengths: 1.5, half: 0.5 }
    }
}

/// One reserved stop: the car, its stop distance along the road and its length.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Reservation {
    pub car: u32,
    pub stop: f32,
    pub length: f32,
}

/// A road's reservations, stop distance descending (far end first).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Reservations(pub Vec<Reservation>);

impl Reservations {
    /// `sub_82E151D0`.
    pub fn reserve(&mut self, r: Reservation) {
        let at = self.0.iter().position(|x| x.stop < r.stop).unwrap_or(self.0.len());
        self.0.insert(at, r);
    }

    /// `sub_82E15368`.
    pub fn release(&mut self, car: u32) {
        self.0.retain(|r| r.car != car);
    }

    /// `sub_82E14CF8`: a stop distance for a car of `length` at distance `d` and `speed` on a road of
    /// `road_length`, or `None`.
    pub fn find_spot(&self, road_length: f32, d: f32, length: f32, speed: f32, p: &SpotParams) -> Option<f32> {
        let mut far = p.window_end * road_length;
        if self.0.is_empty() {
            return Some((p.window_start * road_length + far) * 0.5);
        }
        let reach = d + p.reach_lengths * length + speed;
        for r in &self.0 {
            let front = r.stop + 0.5 * r.length;
            if front >= far {
                continue;
            }
            let mid = (front + far) * 0.5;
            if mid > reach && far - front > length + p.extra_gap {
                return Some(mid);
            }
            far = front - r.length;
        }
        None
    }
}

/// A car on the target lane for the gap check (distance along the road, length, speed).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LaneCar {
    pub distance: f32,
    pub length: f32,
    pub speed: f32,
}

/// `sub_82E14928`: may a car of half extent `ext` at `d` moving at `speed` enter the lane holding `cars` (sorted by
/// distance)? `lane_ok` covers the lane count tests.
pub fn gap_free(road_length: f32, lane_ok: bool, d: f32, ext: f32, speed: f32, cars: &[LaneCar], p: &SpotParams) -> bool {
    if !lane_ok || d < ext || d > road_length - ext {
        return false;
    }
    let behind = cars.iter().filter(|c| c.distance <= d).next_back();
    let ahead = cars.iter().find(|c| c.distance > d);
    if ahead.is_some_and(|a| d + ext + speed > a.distance - p.half * a.length) {
        return false;
    }
    !behind.is_some_and(|b| b.distance + b.length + b.speed > d - p.half * ext)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_road_offers_its_middle() {
        let r = Reservations::default();
        assert_eq!(r.find_spot(100.0, 10.0, 4.0, 10.0, &SpotParams::default()), Some(50.0));
    }

    #[test]
    fn reservations_stay_sorted_far_first_and_release_by_car() {
        let mut r = Reservations::default();
        r.reserve(Reservation { car: 1, stop: 30.0, length: 4.0 });
        r.reserve(Reservation { car: 2, stop: 60.0, length: 4.0 });
        r.reserve(Reservation { car: 3, stop: 45.0, length: 4.0 });
        assert_eq!(r.0.iter().map(|x| x.car).collect::<Vec<_>>(), [2, 3, 1]);
        r.release(3);
        assert_eq!(r.0.iter().map(|x| x.car).collect::<Vec<_>>(), [2, 1]);
    }

    #[test]
    fn the_spot_is_the_middle_of_the_first_long_enough_gap_ahead() {
        let p = SpotParams::default();
        let mut r = Reservations::default();
        r.reserve(Reservation { car: 1, stop: 40.0, length: 4.0 });
        // far = 75, front = 42: gap 33 > 9, mid 58.5 beyond reach 10 + 6 + 10 = 26.
        assert_eq!(r.find_spot(100.0, 10.0, 4.0, 10.0, &p), Some(58.5));
        // Too far along (reach past the mid): the gap behind the reservation is never tried.
        assert_eq!(r.find_spot(100.0, 50.0, 4.0, 10.0, &p), None);
        // A reservation beyond the window is skipped.
        let mut r = Reservations::default();
        r.reserve(Reservation { car: 1, stop: 90.0, length: 4.0 });
        r.reserve(Reservation { car: 2, stop: 20.0, length: 4.0 });
        assert_eq!(r.find_spot(100.0, 0.0, 4.0, 5.0, &p), Some(48.5));
    }

    #[test]
    fn the_gap_check_needs_a_second_of_travel_clear() {
        let p = SpotParams::default();
        let cars = [LaneCar { distance: 20.0, length: 4.0, speed: 5.0 }, LaneCar { distance: 60.0, length: 4.0, speed: 5.0 }];
        assert!(gap_free(100.0, true, 40.0, 2.0, 10.0, &cars, &p));
        // Ahead too close: 40 + 2 + 10 > 50 - 2.
        assert!(!gap_free(100.0, true, 40.0, 2.0, 10.0, &[LaneCar { distance: 50.0, length: 4.0, speed: 5.0 }], &p));
        // Behind too close: 31 + 4 + 5 > 40 - 1.
        assert!(!gap_free(100.0, true, 40.0, 2.0, 10.0, &[LaneCar { distance: 31.0, length: 4.0, speed: 5.0 }], &p));
        // Exactly at the limit is still free (strict test): 30 + 4 + 5 = 40 - 1.
        assert!(gap_free(100.0, true, 40.0, 2.0, 10.0, &[LaneCar { distance: 30.0, length: 4.0, speed: 5.0 }], &p));
        // Near the road ends or no lane.
        assert!(!gap_free(100.0, true, 1.0, 2.0, 10.0, &[], &p));
        assert!(!gap_free(100.0, false, 40.0, 2.0, 10.0, &[], &p));
    }
}
