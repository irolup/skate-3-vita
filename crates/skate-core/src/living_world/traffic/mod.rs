//! Living-world traffic, milestone V1: the road graph, the lane cursor, the traffic signals and
//! the junction entry query (doc `docs/hails-additions/26h-living-world-traffic.md`, design
//! `docs/hails-additions/living-world/vehicles-design.md`).
//!
//! Pure and engine-independent like the population core: no I/O, no ECS, no rendering. The data
//! layer turns the user's exported `roads.bin` into a [`RoadInput`]
//! (`skate-data::roads::RoadGraph::traffic_input`); a mod can build one for its own map.
//!
//! Retail reference (TU3, addresses are evidence only; behaviour re-implemented, not copied):
//! - road network object (global `0x830854C0`, vtable `0x8230C6A8`): `+8` vehicle by id, `+16`
//!   junction by node, `+24` lane run by id, `+32` segment by id, `+36` / `+40` signal controller
//!   by index (`sub_8269B2F0`: `this + 24 + 320 i`);
//! - FollowingLane `sub_82C376E8`: connector choice, junction query, lane leader;
//! - junction query `sub_82E11E90` (+ `sub_82E11C78`, `sub_82E11980`), leader `sub_82E14EE0`;
//! - signals `sub_826B1540` (build), `sub_82E156D8` (phases), `sub_82E158D8` (tick),
//!   `sub_826B2C18` (manager), `sub_826B3C88` (green wave).
//!
//! Multiplayer seams (no networking): everything is a pure function of the roads, the signal
//! clock's tick count and the cars' snapshots; ids are retail ids; iteration is sorted; the only
//! randomness (the unused random connector branch) takes an explicit [`Rng`](super::rng::Rng).
//! A client can rebuild the lights from the host's tick count ([`SignalClock::ticks`]).
//!
//! Moddability: durations are data ([`SignalTimings`] from the `trafficlights` record), the
//! connector choice is a parameter ([`ConnectorChoice`] plus the load function), the lights can
//! be held for a car ([`SignalClock::request_green`], retail's priority vehicle), and the graph
//! is plain input records a mod can write ([`RoadInput`]).

pub mod cursor;
pub mod follow;
pub mod graph;
pub mod horn;
pub mod junction;
pub mod manoeuvre;
pub mod passage;
pub mod obstacles;
pub mod signals;
pub mod skater_scan;
pub mod spots;
pub mod spawn;

pub use cursor::{choose_connector, Advance, ConnectorChoice, LaneCursor, Place};
pub use follow::{Car, FollowEvent, FollowParams};
pub use graph::{
    ConnectorId, ConnectorInput, Curve, EndInput, Frame, JunctionId, JunctionInput, LaneId, PieceInput, RoadInput, RoadNetwork, SegmentId,
    SegmentInput, TrafficError, Turn,
};
pub use junction::{junction_entry, query_due, Entry, EntryInfo, EntryQuery, Occupancy, VehicleKey, VehicleSnapshot, Vehicles};
pub use signals::{priority_end, Controller, Light, Phase, SignalChange, SignalClock, SignalTimings};
pub use spawn::{LaneCar, LaneSpot, PlacementRules};

#[cfg(test)]
mod tests;
