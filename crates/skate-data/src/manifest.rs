use serde::Deserialize;
use std::{
    fmt,
    path::{Component, Path, PathBuf},
};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GameAssets {
    pub version: u32,
    pub character_scene: String,
    pub initial_animation: String,
    pub action_graph: String,
    pub motion_graph: String,
}

/// skate_vita: the binary form of `game.json` (see `crate::binary`).
const BINARY_KIND: &[u8; 4] = b"GAME";

#[derive(Debug)]
pub struct AssetError(pub String);
impl fmt::Display for AssetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for AssetError {}

impl GameAssets {
    pub fn load(root: &Path) -> Result<Self, AssetError> {
        let file = root.join("private/game.json");
        let bytes = crate::vfs::read(&file).map_err(|e| AssetError(format!("Cannot read {}: {e}. Run pipeline/Prepare-Assets.ps1 or select the correct --assets directory.", file.display())))?;
        // skate_vita: the baked binary form (sim.pak) or the JSON; same checks.
        let manifest = if crate::binary::kind(&bytes).is_some() {
            Self::validate(Self::decode(&bytes).map_err(AssetError)?)?
        } else {
            Self::parse(std::str::from_utf8(&bytes).map_err(|e| AssetError(format!("Cannot read {}: {e}", file.display())))?)?
        };
        for (kind, relative) in [
            ("Character scene", manifest.character_scene.as_str()),
            ("ActionGraph", manifest.action_graph.as_str()),
            ("MotionGraph", manifest.motion_graph.as_str()),
        ] {
            let path = root.join(relative);
            if !crate::vfs::is_file(&path) {
                return Err(AssetError(format!("{kind} missing: {}", path.display())));
            }
        }
        Ok(manifest)
    }

    pub fn parse(json: &str) -> Result<Self, AssetError> {
        let manifest: Self = serde_json::from_str(json)
            .map_err(|e| AssetError(format!("Invalid game asset manifest: {e}")))?;
        Self::validate(manifest)
    }

    /// skate_vita: the binary form of a `game.json` file, for `sim.pak`.
    pub fn encode_json(json: &str) -> Result<Vec<u8>, AssetError> {
        let m: Self = serde_json::from_str(json)
            .map_err(|e| AssetError(format!("Invalid game asset manifest: {e}")))?;
        let mut w = crate::binary::Writer::new(BINARY_KIND);
        w.u32(m.version);
        for s in [&m.character_scene, &m.initial_animation, &m.action_graph, &m.motion_graph] {
            w.str(s);
        }
        Ok(w.bytes)
    }

    fn decode(bytes: &[u8]) -> Result<Self, String> {
        let mut r = crate::binary::Reader::new(bytes, BINARY_KIND)?;
        let m = Self {
            version: r.u32()?,
            character_scene: r.str()?,
            initial_animation: r.str()?,
            action_graph: r.str()?,
            motion_graph: r.str()?,
        };
        r.done()?;
        Ok(m)
    }

    /// The checks `parse` applies after deserializing (split out in skate_vita).
    fn validate(manifest: Self) -> Result<Self, AssetError> {
        if manifest.version != 1 {
            return Err(AssetError(format!(
                "Unsupported asset manifest version {}",
                manifest.version
            )));
        }
        validate_path(&manifest.character_scene, "character_scene", "glb")?;
        validate_path(&manifest.action_graph, "action_graph", "stategraph")?;
        validate_path(&manifest.motion_graph, "motion_graph", "stategraph")?;
        if manifest.initial_animation.trim().is_empty() {
            return Err(AssetError(
                "initial_animation must name a stock animation".into(),
            ));
        }
        Ok(manifest)
    }
}

fn validate_path(value: &str, field: &str, extension: &str) -> Result<(), AssetError> {
    let path = PathBuf::from(value);
    if path.as_os_str().is_empty()
        || !path.components().all(|c| matches!(c, Component::Normal(_)))
        || value.contains([':', '\\', '#'])
    {
        return Err(AssetError(format!(
            "{field} must be a relative asset path using forward slashes"
        )));
    }
    if path.extension().and_then(|x| x.to_str()) != Some(extension) {
        return Err(AssetError(format!("{field} must name a .{extension} file")));
    }
    Ok(())
}
