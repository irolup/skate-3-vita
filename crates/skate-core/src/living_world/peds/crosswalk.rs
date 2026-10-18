//! Walk lights for the mod crosswalk rule ([`super::wander::CrosswalkRule::WalkSignal`]): which
//! signalled junction arm a ped is about to cross and the walk light the shared
//! [`SignalClock`] shows for it.
//!
//! Retail never consults this for ambient peds (the road branch of the ped graph is
//! unreachable, `wander.rs`). The mapping is ours: a crossing over the arm at node end `e` runs
//! parallel to the traffic of the neighbouring ends, so it uses the walk light of end
//! `(e + 1) % 4` (the walk list of a controller is green inside its car green, `signals.rs`).
//! A mod can supply its own [`WalkSignals`] instead.

use crate::living_world::Vec3;
use crate::living_world::traffic::graph::{ENDS, RoadNetwork};
use crate::living_world::traffic::signals::{Light, SignalClock};

use super::nav::dist_xz;
use super::wander::WalkSignals;

/// The end points of every signalled junction arm: (junction, end, point on the road middle).
pub fn signalled_arms(net: &RoadNetwork) -> Vec<(usize, u8, Vec3)> {
    let mut out = Vec::new();
    for (j, junction) in net.junctions.iter().enumerate() {
        if !junction.signalled {
            continue;
        }
        for e in 0..ENDS {
            let approach = junction.approaches[e].as_ref().and_then(|a| a.segment).and_then(|s| net.segments[s].pieces.last()).map(|p| p.centre.end);
            let exit = junction.exits[e].as_ref().and_then(|a| a.segment).and_then(|s| net.segments[s].pieces.first()).map(|p| p.centre.start);
            if let Some(p) = approach.or(exit) {
                out.push((j, e as u8, p));
            }
        }
    }
    out
}

/// Walk lights from the road network and the shared signal clock.
pub struct RoadWalkSignals<'a> {
    pub arms: &'a [(usize, u8, Vec3)],
    pub clock: &'a SignalClock,
    /// A crossing belongs to an arm whose end point is within this distance (ours: 20 m, about a
    /// junction's half width plus the pavement).
    pub radius: f32,
}

impl WalkSignals for RoadWalkSignals<'_> {
    fn walk_light(&self, at: Vec3) -> Option<Light> {
        let arm = self
            .arms
            .iter()
            .map(|a| (dist_xz(a.2, at), a))
            .filter(|(d, _)| *d <= self.radius)
            .min_by(|a, b| a.0.total_cmp(&b.0).then_with(|| (a.1.0, a.1.1).cmp(&(b.1.0, b.1.1))))?
            .1;
        Some(self.clock.walk_for_end((arm.1 + 1) % ENDS as u8).light)
    }
}
