//! Data-gated checks of the AIPATH decoder on the user's own disc data. Skips
//! (passes with a note) when no data is configured:
//! - `SKATE3_ASSET_ROOT`: converted assets; reads
//!   `private/living_world/skater_paths/*.bin` (setup group `livingworld`).
//! - `SKATE3_AIPATH_BLOBS`: a folder of raw AIPATHDATA objects
//!   (`*_00eb0014_*.bin`, e.g. from the recomp-research tool `sim_dump.py`).
use skate_data::aipath::{self, AiPath, AiPathId, distance};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

/// Totals of the stock disc [data]: path copies over all city tiles, unique
/// paths (retail's path manager keys by id), node copies.
const COPIES: usize = 3_891;
const UNIQUE: usize = 1_691;
const NODE_COPIES: usize = 389_676;

/// (source name, paths in it) for every configured source.
fn sources() -> Option<Vec<(String, Vec<AiPath>)>> {
    if let Some(root) = std::env::var_os("SKATE3_ASSET_ROOT") {
        let root = PathBuf::from(root);
        let folder = [
            "private/living_world/skater_paths",
            "living_world/skater_paths",
        ]
        .iter()
        .map(|p| root.join(p))
        .find(|p| p.is_dir());
        if let Some(folder) = folder {
            let mut out = Vec::new();
            for entry in std::fs::read_dir(&folder).unwrap() {
                let path = entry.unwrap().path();
                if path.extension().and_then(|e| e.to_str()) != Some("bin") {
                    continue;
                }
                let bytes = std::fs::read(&path).unwrap();
                for tile in aipath::parse_pack(&bytes).unwrap() {
                    let name = format!(
                        "{}:{}",
                        path.file_stem().unwrap().to_string_lossy(),
                        tile.name
                    );
                    out.push((name, aipath::parse(tile.blob).unwrap()));
                }
            }
            return Some(out);
        }
        eprintln!("SKATE3_ASSET_ROOT has no living_world/skater_paths; trying SKATE3_AIPATH_BLOBS");
    }
    let folder = PathBuf::from(std::env::var_os("SKATE3_AIPATH_BLOBS")?);
    let mut out = Vec::new();
    for entry in std::fs::read_dir(folder).ok()? {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if name.contains("_00eb0014_") && name.ends_with(".bin") {
            out.push((name, aipath::parse(&std::fs::read(&path).unwrap()).unwrap()));
        }
    }
    Some(out)
}

#[test]
fn every_recorded_line_decodes_with_the_known_totals() {
    let Some(sources) = sources() else {
        eprintln!(
            "skipped: set SKATE3_ASSET_ROOT or SKATE3_AIPATH_BLOBS to check the disc's AIPATH data"
        );
        return;
    };
    assert!(
        !sources.is_empty(),
        "configured source holds no AIPATH data"
    );
    let all: Vec<&AiPath> = sources.iter().flat_map(|(_, p)| p).collect();
    assert_eq!(all.len(), COPIES);
    assert_eq!(
        all.iter().map(|p| p.nodes.len()).sum::<usize>(),
        NODE_COPIES
    );

    // Copies of one id are identical; city tags only (no park district lines).
    let mut unique: HashMap<AiPathId, &AiPath> = HashMap::new();
    for path in &all {
        assert!(path.id.is_ambient());
        assert!(
            matches!(&path.tag(), b"dwtn" | b"univ" | b"indu"),
            "tag {}",
            path.id.tag_str()
        );
        if let Some(first) = unique.insert(path.id, path) {
            assert_eq!(first, *path, "copies of {} differ", path.id);
        }
    }
    assert_eq!(unique.len(), UNIQUE);

    let mut ratios = Vec::new();
    for path in unique.values() {
        assert!(!path.nodes.is_empty());
        assert_eq!(path.extra, [0, 0, 0]);
        for node in &path.nodes {
            for a in 0..3 {
                assert!(
                    node.position[a] >= path.bbox_min[a] - 1e-3
                        && node.position[a] <= path.bbox_max[a] + 1e-3
                );
            }
            assert!(matches!(node.event, 0 | 1 | 2 | 4));
            if let Some(e) = node.extended {
                assert!(e < path.extended.len());
            }
        }
        for w in path.nodes.windows(2) {
            let step = w[1].direction;
            let size = (step[0] * step[0] + step[1] * step[1] + step[2] * step[2]).sqrt();
            if size > 1e-4 && w[1].frames_since_last_node > 0 {
                ratios.push(
                    distance(w[0].position, w[1].position)
                        / f32::from(w[1].frames_since_last_node)
                        / size,
                );
            }
        }
        // The line network closes: every branch names a known path and node.
        for group in &path.branch_groups {
            assert!((group.node as usize) < path.nodes.len());
            for branch in &group.branches {
                let target = unique
                    .get(&branch.target)
                    .unwrap_or_else(|| panic!("unknown branch target {}", branch.target));
                assert!((branch.target_node as usize) < target.nodes.len());
                assert!((0.0..=1.0).contains(&branch.weight));
            }
        }
    }
    // Recorded at 60 Hz: distance / frames = |direction| (ratio 1 within 5 %).
    ratios.sort_by(f32::total_cmp);
    let median = ratios[ratios.len() / 2];
    assert!((median - 1.0).abs() < 0.05, "median ratio {median}");

    let tags: HashSet<[u8; 4]> = unique.values().map(|p| p.tag()).collect();
    assert_eq!(tags.len(), 3);
}

#[test]
fn packs_keep_tile_copies_and_district_dedupe_matches() {
    let Some(root) = std::env::var_os("SKATE3_ASSET_ROOT") else {
        eprintln!("skipped: set SKATE3_ASSET_ROOT to check the exported skater path packs");
        return;
    };
    let folder = PathBuf::from(root).join("private/living_world/skater_paths");
    if !folder.is_dir() {
        eprintln!("skipped: {} not exported", folder.display());
        return;
    }
    let mut unique = 0;
    for entry in std::fs::read_dir(&folder).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("bin") {
            continue;
        }
        let bytes = std::fs::read(&path).unwrap();
        let tiles = aipath::parse_pack(&bytes).unwrap();
        assert!(
            tiles.iter().all(|t| t.grid().is_some()),
            "{}",
            path.display()
        );
        let (paths, conflicts) = aipath::district_paths(&tiles).unwrap();
        assert_eq!(conflicts, 0);
        unique += paths.len();
    }
    assert_eq!(unique, UNIQUE);
}
