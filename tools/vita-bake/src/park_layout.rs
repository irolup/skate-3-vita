//! Phase 2: the skate.Park layout a park map loads with, baked into its map pack. The owner's choice (2026-10-24):
//! every park places its Fullpark layout (`worlds.json` row `dist_<park>_full`). The pieces are posed into the world
//! once and drawn with the park (merged by material and by area: [`CELL`]), and the sim reads them fused
//! (`skate_sim::park_bake`).
//!
//! Upstream draws each piece as its own entity with its tint and the layout's projected lightmap
//! (`retail_world.wgsl`, research b439 / b440 / b444): the tint is folded into the piece's material colour here, and
//! the projected lightmap's coordinates go into the vertices' lightmap channel with the layout's lightmap texture,
//! which the park shader then reads like the stream's own lightmap.
use std::path::Path;

use skate_data::skate_map::{Material, SkateMap, Vertex};

/// The pieces' draws are split on a grid of this size (metres, x / z) so that they cull with the camera.
pub const CELL: f32 = 32.0;

/// The Fullpark layout `worlds.json` places on the park `map` (its `dist_*_full` row), if any.
pub fn fullpark(assets: &Path, map: &str) -> Option<String> {
    let bytes = std::fs::read(assets.join("private/park-layouts/worlds.json")).ok()?;
    let index: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    index["worlds"].as_object()?.iter().find_map(|(row, world)| {
        let stream = world["stream"].as_str()?;
        let stream = stream.get(..5).filter(|p| p.eq_ignore_ascii_case("DIST_")).map_or(stream, |_| &stream[5..]);
        (row.ends_with("_full") && stream.eq_ignore_ascii_case(map)).then(|| world["layout"].as_str().map(str::to_owned)).flatten()
    })
}

/// The layout's pieces as one render-only map in world space: each piece's triangles posed by its placement, its
/// tint in the material colour, the projected lightmap in the lightmap channel. `sun`: the park's sun direction.
pub fn posed(assets: &Path, layout: &str, sun: [f32; 3]) -> Result<SkateMap, String> {
    let directory = assets.join("private/park-layouts");
    let data = std::fs::read(directory.join(format!("{layout}.skate"))).map_err(|e| format!("{layout}.skate: {e}"))?;
    let mut package = if skate_data::prop_package::is_package(&data) { skate_data::prop_package::decode(&data)? } else { SkateMap::parse_render_only(&data)? };
    let mut objects = Vec::new();
    for extension in package.extensions.iter().filter(|e| e.tag == *b"MOBJ") {
        objects.extend(skate_data::skate_map::parse_static_objects(&package, extension)?);
    }
    let sidecar: serde_json::Value = serde_json::from_slice(&std::fs::read(directory.join(format!("{layout}.json"))).map_err(|e| e.to_string())?)
        .map_err(|e| format!("{layout}.json: {e}"))?;
    // Each piece's tint by its MOBJ id (`instance_id` low 32 bits, as setup writes the id).
    let mut tints = std::collections::HashMap::new();
    for instance in sidecar["instances"].as_array().into_iter().flatten() {
        let Some(id) = instance["instance_id"].as_str().and_then(|s| u64::from_str_radix(s, 16).ok()) else { continue };
        let tint: Vec<f32> = instance["tint"].as_array().into_iter().flatten().filter_map(|v| v.as_f64().map(|v| v as f32)).collect();
        if let Ok(tint) = <[f32; 3]>::try_from(tint) {
            tints.insert((id & 0xFFFF_FFFF) as u32, tint);
        }
    }
    // The projected lightmap: its bounds (x0, x1, z0, z1) and its texture in the package.
    let lightmap = sidecar["projected_lightmap"]["texture_id"].as_str().and_then(|name| package.textures.iter().position(|t| t.name == name)).map(|i| i as u32 + 1);
    let bounds: Vec<f32> = sidecar["projected_lightmap"]["bounds"].as_array().into_iter().flatten().filter_map(|v| v.as_f64().map(|v| v as f32)).collect();
    let bounds = <[f32; 4]>::try_from(bounds).ok().filter(|b| b[1] != b[0] && b[3] != b[2] && lightmap.is_some());
    let project = |w: [f32; 3]| -> [f32; 2] {
        let Some([x0, x1, z0, z1]) = bounds else { return [0.0; 2] };
        let (dx, dz) = (x1 - x0, z1 - z0);
        let ly = if sun[1].abs() < 1e-4 { 1e-4 } else { sun[1] };
        let u = (w[2] - z0) / dz + w[1] * sun[2] / (ly * dz.abs());
        let v = (w[0] - x0) / dx - w[1] * sun[0] / (ly * dx.abs());
        [u, 1.0 - v]
    };
    // A material per (material, tint): the tint multiplies the colour; the projected lightmap in slot 1.
    let mut materials: Vec<Material> = Vec::new();
    let mut variants: std::collections::HashMap<(u32, [u32; 3]), u32> = std::collections::HashMap::new();
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    for object in &objects {
        let t = &object.transform;
        let pose = |p: [f32; 3]| [
            p[0] * t[0] + p[1] * t[3] + p[2] * t[6] + t[9],
            p[0] * t[1] + p[1] * t[4] + p[2] * t[7] + t[10],
            p[0] * t[2] + p[1] * t[5] + p[2] * t[8] + t[11],
        ];
        let mirrored = t[0] * (t[4] * t[8] - t[5] * t[7]) - t[1] * (t[3] * t[8] - t[5] * t[6]) + t[2] * (t[3] * t[7] - t[4] * t[6]) < 0.0;
        let tint = tints.get(&object.id).copied().unwrap_or([1.0; 3]);
        let range = object.first_index as usize..(object.first_index + object.index_count) as usize;
        // The piece's vertices are shared within it (a template vertex posed once per piece).
        let mut placed: std::collections::HashMap<u32, u32> = std::collections::HashMap::new();
        for tri in package.geometry.indices[range].chunks_exact(3) {
            let order = if mirrored { [tri[0], tri[2], tri[1]] } else { [tri[0], tri[1], tri[2]] };
            for i in order {
                let source = &package.geometry.vertices[i as usize];
                let key = (source.material, tint.map(f32::to_bits));
                let material = *variants.entry(key).or_insert_with(|| {
                    let mut m = package.materials[source.material as usize - 1].clone();
                    m.color = std::array::from_fn(|c| m.color[c] * tint[c]);
                    if let Some(lightmap) = lightmap {
                        m.textures[1] = lightmap;
                    }
                    materials.push(m);
                    materials.len() as u32
                });
                let index = *placed.entry(i).or_insert_with(|| {
                    let position = pose(source.position);
                    vertices.push(Vertex { position, lightmap_uv: project(position), material, ..source.clone() });
                    vertices.len() as u32 - 1
                });
                indices.push(index);
            }
        }
    }
    package.materials = materials;
    package.geometry = skate_data::skate_map::Geometry { vertices, indices, collision: Vec::new() };
    package.extensions.clear();
    Ok(package)
}

/// The cell of a triangle (its first vertex) on the [`CELL`] grid.
pub fn cell(position: [f32; 3]) -> (i32, i32) {
    ((position[0] / CELL).floor() as i32, (position[2] / CELL).floor() as i32)
}
