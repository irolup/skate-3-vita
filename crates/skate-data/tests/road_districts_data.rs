//! Data-gated check (doc 26 "Cars flying off, population gone") on the user's own export: the
//! road districts are separate worlds that overlap in x / z, so the DownTown world must only get
//! DownTown's roads. Every DownTown lane point over the DownTown walk mesh lies on it (within
//! 1.5 m), while the whole file holds other districts' roads metres above and below it (where
//! the first living-world build put cars "in the air"). Skips (passes with a note) when
//! `SKATE3_ASSET_ROOT` has no `roads.bin` / `navmesh.bin`.

use skate_core::living_world::peds::nav::{NavMesh, NavPoly, NavRules};
use skate_core::living_world::traffic::RoadNetwork;
use skate_data::roads::RoadGraph;
use std::path::PathBuf;

fn find(rel: &str) -> Option<PathBuf> {
    let raw = std::env::var_os("SKATE3_ASSET_ROOT")?;
    std::env::split_paths(&raw).map(|r| r.join(rel)).find(|p| p.is_file())
}

fn inside(poly: &NavPoly, x: f32, z: f32) -> bool {
    if x < poly.min[0] || x > poly.max[0] || z < poly.min[1] || z > poly.max[1] {
        return false;
    }
    let n = poly.verts.len();
    let (mut c, mut j) = (false, n - 1);
    for i in 0..n {
        let (a, b) = (poly.verts[i], poly.verts[j]);
        if (a[2] > z) != (b[2] > z) && x < (b[0] - a[0]) * (z - a[2]) / (b[2] - a[2]) + a[0] {
            c = !c;
        }
        j = i;
    }
    c
}

/// (lane points over the walk mesh, points more than 1.5 m off it, worst signed height offset).
fn offsets(net: &RoadNetwork, m: &NavMesh) -> (usize, usize, f32) {
    let (mut n, mut off, mut worst) = (0, 0, 0.0f32);
    for (si, s) in net.segments.iter().enumerate() {
        let mut d = 0.0;
        while d <= s.length {
            let p = net.lane_frame(si, 0.0, d).position;
            let best = m.polys.iter().enumerate().filter(|(_, poly)| inside(poly, p[0], p[2])).map(|(k, _)| p[1] - m.height_at(k as u32, p[0], p[2])).min_by(|a, b| a.abs().total_cmp(&b.abs()));
            if let Some(dy) = best {
                n += 1;
                off += usize::from(dy.abs() > 1.5);
                if dy.abs() > worst.abs() {
                    worst = dy;
                }
            }
            d += 4.0;
        }
    }
    (n, off, worst)
}

#[test]
fn downtown_world_gets_only_downtown_roads_and_they_lie_on_the_ground() {
    let (Some(roads), Some(nav)) = (find("private/living_world/roads.bin"), find(skate_data::ped_nav::NAVMESH)) else {
        eprintln!("skipped: set SKATE3_ASSET_ROOT to an export with living_world/roads.bin and navmesh.bin");
        return;
    };
    let g = RoadGraph::parse(&std::fs::read(roads).unwrap()).unwrap();
    let m = NavMesh::build(&skate_data::ped_nav::district(&std::fs::read(nav).unwrap(), "DownTown").unwrap().unwrap(), NavRules::default());
    let whole = RoadNetwork::build(&g.traffic_input()).unwrap();
    let downtown = RoadNetwork::build(&g.district_traffic_input("DownTown").unwrap()).unwrap();
    let d = g.districts.iter().position(|d| d.name == "DownTown").unwrap() as u32;
    assert!(downtown.segments.iter().all(|s| s.district == d));
    assert_eq!(downtown.segments.len(), g.district("DownTown").unwrap().segments.len());
    let (n, off, worst) = offsets(&downtown, &m);
    eprintln!("DownTown roads: {n} points over the walk mesh, {off} off by > 1.5 m, worst {worst:.2} m");
    assert!(n > 500, "{n}");
    assert_eq!(off, 0, "DownTown road off its ground by {worst} m");
    let (n_all, off_all, worst_all) = offsets(&whole, &m);
    eprintln!("whole file: {n_all} points, {off_all} off by > 1.5 m, worst {worst_all:.2} m");
    assert!(off_all > 0, "the export no longer overlaps districts; revisit doc 26 'Cars flying off'");
}
