//! Sidecar persistence for placed prop layouts (Phase 4).
//!
//! One versioned JSON file per map at `settings/prop-layouts/<map>.json`
//! beside the asset root, the same user-data convention as
//! `settings/gameplay.json`. Only confirmed placements are recorded; the map
//! package itself is never modified. A missing, corrupt or foreign-map file
//! leaves the authored poses untouched rather than failing the load.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const SCHEMA: u32 = 1;

/// One confirmed placement: template-origin translation plus rotation basis
/// columns, exactly what `PropDynamics::pose`/`teleport` consume.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct PropPose {
    pub id: u32,
    pub origin: [f32; 3],
    pub basis: [[f32; 3]; 3],
}

#[derive(Serialize, Deserialize)]
struct LayoutFile {
    schema: u32,
    map: String,
    props: Vec<PropPose>,
}

pub(crate) fn path(asset_root: &Path, map: &str) -> PathBuf {
    asset_root
        .parent()
        .unwrap_or(asset_root)
        .join(format!("settings/prop-layouts/{map}.json"))
}

/// Saved poses for `map`, or None when absent/invalid/foreign. Invalid
/// layouts warn and are ignored, never fail.
pub(crate) fn load(path: &Path, map: &str) -> Option<BTreeMap<u32, PropPose>> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
        Err(error) => {
            tracing::warn!("SKATE_PROP_LAYOUT: {}: {error}", path.display());
            return None;
        }
    };
    match serde_json::from_slice::<LayoutFile>(&bytes) {
        Ok(file) if file.schema == SCHEMA && file.map == map => {
            Some(file.props.into_iter().map(|p| (p.id, p)).collect())
        }
        Ok(file) => {
            tracing::warn!(
                "SKATE_PROP_LAYOUT: {}: schema {} map '{}' unsupported",
                path.display(),
                file.schema,
                file.map
            );
            None
        }
        Err(error) => {
            tracing::warn!("SKATE_PROP_LAYOUT: {}: {error}", path.display());
            None
        }
    }
}

pub(crate) fn save(path: &Path, poses: &BTreeMap<u32, PropPose>) -> Result<(), String> {
    let file = LayoutFile {
        schema: SCHEMA,
        // The map name is validated on load via the file name; keep the
        // field informational by round-tripping the file stem.
        map: path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_owned(),
        props: poses.values().copied().collect(),
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(path, serde_json::to_vec_pretty(&file).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pose(id: u32, x: f32) -> PropPose {
        PropPose {
            id,
            origin: [x, 1.0, 2.0],
            basis: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        }
    }

    #[test]
    fn layout_round_trip() {
        let dir = std::env::temp_dir().join(format!("skate-layout-{}", std::process::id()));
        let path = dir.join("demo.json");
        let poses = BTreeMap::from([(7, pose(7, 0.5)), (9, pose(9, -2.0))]);
        save(&path, &poses).unwrap();
        assert_eq!(load(&path, "demo"), Some(poses));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_corrupt_and_foreign_layouts_are_ignored() {
        let dir = std::env::temp_dir().join(format!("skate-layout-bad-{}", std::process::id()));
        let path = dir.join("demo.json");
        assert_eq!(load(&path, "demo"), None);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&path, b"{ not json").unwrap();
        assert_eq!(load(&path, "demo"), None);
        std::fs::write(&path, br#"{"schema":1,"map":"other","props":[]}"#).unwrap();
        assert_eq!(load(&path, "demo"), None);
        std::fs::write(&path, br#"{"schema":99,"map":"demo","props":[]}"#).unwrap();
        assert_eq!(load(&path, "demo"), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
