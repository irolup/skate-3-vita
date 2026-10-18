//! Data-gated checks of the road graph (`roads.bin` v2) on the user's own export. Skips (passes with
//! a note) when no data is configured. `SKATE3_ASSET_ROOT`: converted assets (one root, or several
//! joined like PATH); reads `private/living_world/roads.bin`.
//!
//! The expected counts are the shipped Skate 3 road networks as decoded for milestone V0
//! (`docs/hails-additions/living-world/vehicles-data.md`); they are asserted, not embedded in the engine.

use skate_data::roads::RoadGraph;
use std::path::PathBuf;

fn graph() -> Option<RoadGraph> {
    let raw = std::env::var_os("SKATE3_ASSET_ROOT")?;
    let path: PathBuf = std::env::split_paths(&raw)
        .flat_map(|root| ["private/living_world/roads.bin", "living_world/roads.bin"].map(|p| root.join(p)))
        .find(|p| p.is_file())?;
    Some(RoadGraph::parse(&std::fs::read(path).unwrap()).unwrap())
}

fn dist(a: [f32; 3], b: [f32; 3]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

fn in_quad(p: [f32; 3], quad: &[[f32; 3]; 4], margin: f32) -> bool {
    let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
    for c in quad {
        lo = [lo[0].min(c[0]), lo[1].min(c[2])];
        hi = [hi[0].max(c[0]), hi[1].max(c[2])];
    }
    p[0] >= lo[0] - margin && p[0] <= hi[0] + margin && p[2] >= lo[1] - margin && p[2] <= hi[1] + margin
}

#[test]
fn districts_hold_the_shipped_counts() {
    let Some(g) = graph() else {
        eprintln!("skipped: set SKATE3_ASSET_ROOT to an export with living_world/roads.bin");
        return;
    };
    let counts: Vec<(String, usize, usize, usize)> = g
        .districts
        .iter()
        .map(|d| {
            let connectors = g.junctions[d.junctions.clone()].iter().map(|j| j.connectors.len()).sum();
            (d.name.clone(), d.segments.len(), d.junctions.len(), connectors)
        })
        .collect();
    assert_eq!(
        counts,
        vec![("DownTown".into(), 46, 19, 88), ("Industrial".into(), 26, 10, 46), ("University".into(), 4, 4, 4)]
    );
    assert_eq!(g.pieces.len(), 3831);
    let incomplete: Vec<String> = g.segments.iter().filter(|s| !s.pieces_complete()).map(|s| format!("{:016X}", s.id)).collect();
    assert!(incomplete.is_empty(), "segments without full lane geometry: {incomplete:?}");
    for s in &g.segments {
        let kmh = s.speed_limit * 3.6;
        assert!([51.0f32, 50.0, 30.0].iter().any(|v| (kmh - v).abs() < 0.01), "{:016X}: {kmh} km/h", s.id);
        assert!((1..=2).contains(&s.lanes), "{:016X}: {} lanes", s.id, s.lanes);
    }
}

#[test]
fn pieces_are_continuous_and_end_at_their_junction() {
    let Some(g) = graph() else {
        eprintln!("skipped: set SKATE3_ASSET_ROOT to an export with living_world/roads.bin");
        return;
    };
    for s in g.segments.iter().filter(|s| s.pieces_complete()) {
        let pieces = g.segment_pieces(s);
        for (k, w) in pieces.windows(2).enumerate() {
            assert!(dist(w[0].centre.end, w[1].centre.start) < 1e-2, "{:016X} piece {k}", s.id);
            assert!(w[1].distance > w[0].distance);
        }
        let last = pieces.last().unwrap();
        assert!((last.distance - s.length).abs() < 0.05, "{:016X}: {} vs {}", s.id, last.distance, s.length);
        // Pieces run towards to_node: the last one ends on the destination junction's outer box.
        if let Some(j) = g.junction(s.to_node) {
            assert!(in_quad(last.centre.end, &j.outer, 0.6), "{:016X} does not end at {:016X}", s.id, j.node);
        }
        for p in pieces {
            assert!((p.centre.arc[15] - p.centre.length).abs() < 1e-3);
        }
    }
}

#[test]
fn connectors_match_their_lane_lists_and_segments() {
    let Some(g) = graph() else {
        eprintln!("skipped: set SKATE3_ASSET_ROOT to an export with living_world/roads.bin");
        return;
    };
    let mut exits = 0;
    for j in &g.junctions {
        for c in g.junction_connectors(j) {
            assert!(g.lane_connectors(j, c.from_end as usize, c.from_lane as usize).contains(&c.index), "{:016X} #{}", j.node, c.index);
            assert!(g.lane_connectors(j, 4 + c.to_end as usize, c.to_lane as usize).contains(&c.index), "{:016X} #{}", j.node, c.index);
            assert!(in_quad(c.curve.start, &j.outer, 0.6) && in_quad(c.curve.end, &j.outer, 0.6), "{:016X} #{}", j.node, c.index);
            assert!(dist(c.curve.point(1.0), c.curve.end) < 1e-3);
            assert!((c.curve.arc[15] - c.curve.length).abs() < 1e-3);
            if g.connector_exit(c).is_some() {
                exits += 1;
            }
        }
    }
    assert_eq!(exits, g.connectors.len(), "every connector leads onto a segment");
    // Every lane of a segment that ends at a junction has somewhere to go.
    for s in g.segments.iter().filter(|s| g.junction(s.to_node).is_some()) {
        for lane in 0..s.lanes as usize {
            assert!(!g.next_connectors(s, lane).is_empty(), "{:016X} lane {lane}", s.id);
        }
    }
}
