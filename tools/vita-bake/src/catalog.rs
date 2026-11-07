//! `catalog.pak`: the Object Dropper catalogue (docs/object-dropper-survey.md step D2), from the install's
//! `private/object-catalog/` (`tools/prepare_object_catalog.py`). The Vita reads one item at a time, when the editor
//! shows or places it, so each template is cut into its own files:
//!
//! - `<part>/items/<template>.render` (`vita_pack::catalog::ItemRender`): its draws, textures by index in the part's
//!   table `<part>/textures/<n>.tex` (each texture baked once for the whole part, as the props render bakes them);
//! - `<part>/items/<template>.props` (`skate_data::prop_package`): its geometry and materials, what the simulation
//!   builds its collision from when the sidecar has no shape;
//! - `<part>/items/<template>.shape` (`skate_sim::catalog_shapes`): its retail collision shape in binary (as JSON
//!   text the shapes were 61 of the pack's 78 MB);
//! - `<part>/items/<template>.json`: the rest of its slice of the part's sidecar (`grind_splines`, and for props
//!   `types` and `grab_splines`), what `skate_sim` reads for park pieces and props;
//! - `catalog.json`: the tree, the items (caps, snap sockets, template, bounds and costs: triangles, collision triangles, draws, vertex and texture
//!   bytes, for the budgets) and the editor's tuning records; the pieces without a model (the park spawner, the
//!   teleporters) with their footprint and the marker they are drawn as, the markers (`markers`: spline, ribbon style;
//!   `marker_colours`: the colour rows' textures) and the stock parks' player starts (`park_starts`).
//! - `markers/<texture>.png`: the marker colours' pass textures (the visual indicators' PNGs).
//! - `parks/<layout>.park`: the Park Setup templates' stock layouts (`park_lots`, the lots and their templates).
//!
//! `<part>` is `terrain` (static pieces) or `props` (dynamic props).
use std::collections::BTreeMap;
use std::path::Path;

use skate_data::skate_map::{Geometry, SkateMap, Vertex};
use vita_pack::catalog::{texture_to_pack, ItemRender};
use vita_pack::files::File;
use vita_pack::map::STRIDE;

const DIR: &str = "private/object-catalog";
const PARTS: [&str; 2] = ["terrain", "props"];
const SIDECAR_KEYS: [&str; 3] = ["grind_splines", "types", "grab_splines"];
/// The visual indicators export (splines, styles, colours) the markers come from.
const INDICATORS: &str = "private/visual-indicators.json";
/// The marker colours retail's editor draws with (`82866AD8`): blue at rest, green highlighted, red (a state not
/// traced yet).
const MARKER_COLOURS: [&str; 3] = ["blue", "green", "red"];

/// What the budgets need of one template.
struct Cost {
    triangles: u32,
    draws: u32,
    vertex_bytes: u32,
    textures: Vec<u16>,
    bounds: [[f32; 3]; 2],
    /// Its collision triangles (its shape's, else its render mesh's, as the game builds it).
    collision: u32,
}

pub fn bake(install: &Path, out: &Path) -> Result<(), String> {
    let source = install.join("assets").join(DIR);
    let read_json = |path: &Path| -> Result<serde_json::Value, String> {
        serde_json::from_slice(&std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?).map_err(|e| format!("{}: {e}", path.display()))
    };
    let catalog = read_json(&source.join("catalog.json"))?;
    let mut files = Vec::new();
    let mut costs: BTreeMap<String, Cost> = BTreeMap::new();
    let mut parts = serde_json::Map::new();
    for part in PARTS {
        let map = SkateMap::parse_render_only(&std::fs::read(source.join(format!("{part}.skate"))).map_err(|e| format!("{part}.skate: {e}"))?)?;
        let sidecar = read_json(&source.join(format!("{part}.json")))?;
        let mut objects = Vec::new();
        for extension in map.extensions.iter().filter(|e| e.tag == *b"MOBJ") {
            objects.extend(skate_data::skate_map::parse_static_objects(&map, extension)?);
        }
        // One object per template (the export places each item once; a template two items share is cut once).
        let mut templates: Vec<(String, u32, u32)> = objects.iter().map(|o| (template_of(&o.name).to_owned(), o.first_index, o.index_count)).collect();
        templates.sort();
        templates.dedup_by(|a, b| a.0 == b.0);
        let ranges: Vec<(u32, u32)> = templates.iter().map(|t| (t.1, t.2)).collect();
        let (render, draw_ranges) = crate::map::build_prop_render(&map, &ranges)?;
        for (n, texture) in render.textures.iter().enumerate() {
            files.push(File { path: format!("{DIR}/{part}/textures/{n}.tex"), bytes: texture_to_pack(texture) });
        }
        let texture_bytes: Vec<usize> = render.textures.iter().map(vita_pack::texture::Texture::bytes).collect();
        for ((template, first, count), draws) in templates.iter().zip(&draw_ranges) {
            let item = cut(&render, draws.clone()).map_err(|e| format!("{part} {template}: {e}"))?;
            let bytes = item.to_pack();
            if ItemRender::from_pack(&bytes, render.textures.len())? != item {
                return Err(format!("{part} {template}: item render does not read back"));
            }
            let mut textures: Vec<u16> = item.draws.iter().flat_map(|d| [d.albedo, d.lightmap]).flatten().collect();
            textures.sort_unstable();
            textures.dedup();
            let (sub, bounds) = sub_map(&map, *first, *count, template)?;
            let collision = sidecar["collision"][template.as_str()]["triangles"].as_array().map_or(count / 3, |t| t.len() as u32);
            costs.insert(template.clone(), Cost {
                triangles: count / 3,
                draws: item.draws.len() as u32,
                vertex_bytes: item.vertex_data.len() as u32,
                textures,
                bounds,
                collision,
            });
            files.push(File { path: format!("{DIR}/{part}/items/{template}.render"), bytes });
            let package = skate_data::prop_package::encode(&sub);
            let back = skate_data::prop_package::decode(&package)?;
            if back.geometry.indices != sub.geometry.indices || back.materials != sub.materials {
                return Err(format!("{part} {template}: item package does not read back"));
            }
            files.push(File { path: format!("{DIR}/{part}/items/{template}.props"), bytes: package });
            let slice: serde_json::Map<String, serde_json::Value> = SIDECAR_KEYS
                .iter()
                .filter_map(|k| Some(((*k).to_owned(), serde_json::json!({ template.clone(): sidecar[*k].get(template.as_str())?.clone() }))))
                .collect();
            files.push(File { path: format!("{DIR}/{part}/items/{template}.json"), bytes: serde_json::to_vec(&slice).map_err(|e| e.to_string())? });
            if let Some(shape) = sidecar["collision"].get(template.as_str()) {
                let one = serde_json::json!({ "collision": { template.clone(): shape.clone() } });
                files.push(File { path: format!("{DIR}/{part}/items/{template}.shape"), bytes: skate_sim::catalog_shapes::encode(&one, template) });
            }
        }
        println!(
            "  {part}: {} templates, {} draws, {} triangles, {} textures ({} KiB), vertices {} KiB",
            templates.len(),
            render.draws.len(),
            render.indices.len() / 3,
            render.textures.len(),
            texture_bytes.iter().sum::<usize>() / 1024,
            render.vertex_data.len() / 1024
        );
        parts.insert(part.to_owned(), serde_json::json!({ "textures": render.textures.len(), "texture_bytes": texture_bytes }));
    }

    // The slim catalogue: items with their costs, the caps that are set, the tree and the tuning as exported.
    let mut items = Vec::new();
    for item in catalog["items"].as_array().ok_or("catalog.json: no items")? {
        let mut out = serde_json::Map::new();
        for key in ["id", "name", "kind", "sub", "template", "teleporter"] {
            out.insert(key.to_owned(), item[key].clone());
        }
        if let Some(indicator) = item["indicator"].as_str() {
            out.insert("indicator".into(), indicator.into());
        }
        if item["teleport"].is_array() {
            out.insert("teleport".into(), item["teleport"].clone());
        }
        let caps: serde_json::Map<String, serde_json::Value> =
            item["caps"].as_object().into_iter().flatten().filter(|(_, v)| v.as_str().is_some_and(|s| !s.is_empty())).map(|(k, v)| (k.clone(), v.clone())).collect();
        out.insert("caps".into(), caps.into());
        // Snap sockets: [group, x axis, y axis, z axis, position] (the 4x4 row-vector frame without its last column).
        let sockets: Vec<serde_json::Value> = item["sockets"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|s| {
                let group = s[0].as_u64()?;
                let m: Vec<f64> = s[1].as_array()?.iter().filter_map(|v| v.as_f64()).collect();
                (m.len() == 16).then(|| {
                    let frame: Vec<f64> = (0..4).flat_map(|r| (0..3).map(move |c| (r, c))).map(|(r, c)| (m[r * 4 + c] * 1e4).round() / 1e4).collect();
                    serde_json::json!([group, frame])
                })
            })
            .collect();
        if !sockets.is_empty() {
            out.insert("sockets".into(), sockets.into());
        }
        if let Some(cost) = item["template"].as_str().and_then(|t| costs.get(t)) {
            out.insert("cost".into(), serde_json::json!({
                "triangles": cost.triangles, "draws": cost.draws, "vertex_bytes": cost.vertex_bytes, "textures": cost.textures,
                "bounds": cost.bounds, "collision": cost.collision,
            }));
        } else if item["bounds"].is_array() {
            // A piece without a model: its footprint only (nothing to draw or load).
            out.insert("cost".into(), serde_json::json!({ "triangles": 0, "draws": 0, "vertex_bytes": 0, "textures": [], "bounds": item["bounds"] }));
        }
        items.push(serde_json::Value::Object(out));
    }
    let (markers, marker_colours) = markers(install, &items, &mut files)?;
    // The park maps' starts by map name too (the Vita's baked park map does not name its layout): each park's
    // Fullpark layout, as the map bake picks it.
    let mut park_starts = catalog["park_starts"].as_object().cloned().unwrap_or_default();
    for map in ["StartPark", "MegaPark", "IndustrialSkatePark", "DownTownSkatePark"] {
        if let Some(start) = crate::park_layout::fullpark(&install.join("assets"), map).and_then(|layout| park_starts.get(&layout).cloned()) {
            park_starts.insert(map.to_owned(), start);
        }
    }
    let park_worlds = park_worlds(install, &catalog)?;
    let slim = serde_json::json!({
        "version": 1,
        "parts": parts,
        "super_categories": catalog["super_categories"],
        "sub_categories": catalog["sub_categories"],
        "items": items,
        "tuning": catalog["tuning"],
        "markers": markers,
        "marker_colours": marker_colours,
        "park_starts": park_starts,
        "park_lots": catalog["park_lots"],
        "park_worlds": park_worlds,
    });
    // The templates' stock layouts (Park Setup, and the frame a saved park is written in).
    let stock = install.join("assets").join("private/stock/data/skatepark");
    let mut parks = 0;
    for layout in catalog["park_lots"].as_array().into_iter().flatten().flat_map(|l| l["templates"].as_array().into_iter().flatten()).filter_map(|t| t["layout"].as_str()) {
        let path = format!("{DIR}/parks/{layout}.park");
        if files.iter().any(|f| f.path == path) {
            continue;
        }
        let bytes = std::fs::read(stock.join(format!("{layout}.park"))).map_err(|e| format!("{layout}.park: {e}"))?;
        files.push(File { path, bytes });
        parks += 1;
    }
    println!("  park templates: {parks} stock layouts");
    for layout in park_worlds.iter().filter_map(|w| w["layout"].as_str()) {
        let path = format!("{DIR}/parks/{layout}.park");
        if !files.iter().any(|f| f.path == path) {
            let bytes = std::fs::read(stock.join(format!("{layout}.park"))).map_err(|e| format!("{layout}.park: {e}"))?;
            files.push(File { path, bytes });
        }
    }
    files.push(File { path: format!("{DIR}/catalog.json"), bytes: serde_json::to_vec(&slim).map_err(|e| e.to_string())? });

    std::fs::create_dir_all(out).map_err(|e| e.to_string())?;
    let pack = vita_pack::files::to_pack(&files);
    std::fs::write(out.join("catalog.pak"), &pack).map_err(|e| e.to_string())?;
    let index = vita_pack::files::index_of(&pack)?;
    std::fs::write(out.join("catalog.pak.idx"), vita_pack::files::encode_index(&pack[24..56], &index)).map_err(|e| e.to_string())?;
    println!("{}: {} items, {} files, {} KiB", out.join("catalog.pak").display(), items.len(), files.len(), pack.len() / 1024);
    Ok(())
}

/// The markers the model-less items name: per indicator row its spline (power-basis segments) and ribbon style; and
/// the marker colours' textures, added to `files`. Empty without the visual indicators export.
/// The Challenge Map's skate.Park rows whose stock layout no lot's template lays (PCU Training - Basic).
fn park_worlds(install: &Path, catalog: &serde_json::Value) -> Result<Vec<serde_json::Value>, String> {
    let private = install.join("assets").join("private");
    let read_json = |path: &Path| -> Result<serde_json::Value, String> {
        serde_json::from_slice(&std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?).map_err(|e| format!("{}: {e}", path.display()))
    };
    let (worlds, teleports) = match (read_json(&private.join("park-layouts/worlds.json")), read_json(&private.join("teleports.json"))) {
        (Ok(worlds), Ok(teleports)) => (worlds, teleports),
        // An older install: the catalogue bakes without them, and those Challenge Map rows only respawn.
        (Err(e), _) | (_, Err(e)) => {
            println!("  (no challenge map layouts: {e})");
            return Ok(Vec::new());
        }
    };
    let templates: Vec<&str> = catalog["park_lots"].as_array().into_iter().flatten().flat_map(|l| l["templates"].as_array().into_iter().flatten()).filter_map(|t| t["layout"].as_str()).collect();
    let mut rows = Vec::new();
    for (world, row) in worlds["worlds"].as_object().into_iter().flatten() {
        let layout = row["layout"].as_str().unwrap_or_default();
        let listed = teleports["destinations"].as_array().into_iter().flatten().any(|d| d["world"].as_str() == Some(world));
        if listed && !templates.contains(&layout) {
            rows.push(serde_json::json!({ "world": world, "layout": layout }));
        }
    }
    println!("  challenge map's stock layouts: {rows:?}");
    Ok(rows)
}

fn markers(install: &Path, items: &[serde_json::Value], files: &mut Vec<File>) -> Result<(serde_json::Value, serde_json::Value), String> {
    let assets = install.join("assets");
    let Ok(bytes) = std::fs::read(assets.join(INDICATORS)) else {
        println!("  (no visual-indicators.json: the park spawner and teleporters are not drawn)");
        return Ok((serde_json::json!({}), serde_json::json!({})));
    };
    let full: serde_json::Value = serde_json::from_slice(&bytes).map_err(|e| format!("{INDICATORS}: {e}"))?;
    let mut markers = serde_json::Map::new();
    for item in items {
        let Some(key) = item["indicator"].as_str() else { continue };
        let row = &full["indicators"][key];
        let (Some(spline), style) = (row["spline"].as_str(), row["style"].as_str().unwrap_or("default")) else { continue };
        let style = if full["styles"][style].is_object() { &full["styles"][style] } else { &full["styles"]["default"] };
        // The shape's middle on the footprint's (on the ground plane): the door splines stand about their own origin,
        // the spawner's box already spans the footprint. NOT RETAIL YET: retail places the indicator at a point of the
        // piece (`82866AD8`, piece +0xB0) not traced; the footprint's middle is where the cursor holds the piece.
        let num = |v: &serde_json::Value| v.as_f64().unwrap_or(0.0);
        let (bounds, shape) = (&item["cost"]["bounds"], &full["splines"][spline]);
        let middle = |k: usize| (num(&bounds[0][k]) + num(&bounds[1][k])) * 0.5 - (num(&shape["min"][k]) + num(&shape["max"][k])) * 0.5;
        let offset = [middle(0), 0.0, middle(2)];
        markers.insert(key.to_owned(), serde_json::json!({ "spline": shape, "style": style, "offset": offset }));
    }
    let folder = full["textures"].as_str().unwrap_or("visual-indicators");
    let mut colours = serde_json::Map::new();
    for name in MARKER_COLOURS {
        let mut paths = serde_json::Map::new();
        for pass in ["front", "background"] {
            let Some(texture) = full["colours"][name][pass].as_str() else { continue };
            let bytes = std::fs::read(assets.join(format!("private/{folder}/{texture}.png"))).map_err(|e| format!("marker texture {texture}: {e}"))?;
            let path = format!("{DIR}/markers/{texture}.png");
            if !files.iter().any(|f| f.path == path) {
                files.push(File { path: path.clone(), bytes });
            }
            paths.insert(pass.to_owned(), path.into());
        }
        colours.insert(name.to_owned(), paths.into());
    }
    println!("  markers: {} rows, {} colours", markers.len(), colours.len());
    Ok((markers.into(), colours.into()))
}

/// The template id an export placement's name starts with (`<template>/<item name>`).
fn template_of(name: &str) -> &str {
    name.split('/').next().unwrap_or_default()
}

/// The draws `range` of the part's render with only the vertex and index data they use, rebased. The builder appends
/// one template after another (then puts its alpha-tested draws last), so a template's data is one run of vertices
/// and one run of indices, whatever the order of its draws.
fn cut(render: &vita_pack::map::Render, range: std::ops::Range<usize>) -> Result<ItemRender, String> {
    let draws = &render.draws[range];
    if draws.is_empty() {
        return Ok(ItemRender { vertex_data: Vec::new(), indices: Vec::new(), draws: Vec::new() });
    }
    let v0 = draws.iter().map(|d| d.first_vertex).min().unwrap_or(0);
    let i0 = draws.iter().map(|d| d.first_index).min().unwrap_or(0);
    let v1 = draws.iter().map(|d| d.first_vertex + d.vertex_count).max().unwrap_or(0);
    let i1 = draws.iter().map(|d| d.first_index + d.index_count).max().unwrap_or(0);
    let (vertices, indices): (u32, u32) = (draws.iter().map(|d| d.vertex_count).sum(), draws.iter().map(|d| d.index_count).sum());
    if v1 - v0 != vertices || i1 - i0 != indices {
        return Err("template draws are not one run".into());
    }
    Ok(ItemRender {
        vertex_data: render.vertex_data[v0 as usize * STRIDE..v1 as usize * STRIDE].to_vec(),
        indices: render.indices[i0 as usize..i1 as usize].to_vec(),
        draws: draws.iter().map(|d| vita_pack::map::Draw { first_vertex: d.first_vertex - v0, first_index: d.first_index - i0, ..d.clone() }).collect(),
    })
}

/// One template of the part as its own package (vertices it uses, remapped; its materials without texture references)
/// and its bounds.
fn sub_map(map: &SkateMap, first: u32, count: u32, template: &str) -> Result<(SkateMap, [[f32; 3]; 2]), String> {
    let mut vertices: Vec<Vertex> = Vec::new();
    let mut materials = Vec::new();
    let mut vertex_of = BTreeMap::new();
    let mut material_of = BTreeMap::new();
    let mut bounds = [[f32::INFINITY; 3], [f32::NEG_INFINITY; 3]];
    let indices = map.geometry.indices.get(first as usize..(first + count) as usize).ok_or_else(|| format!("{template}: index range out of the package"))?;
    let mut out_indices = Vec::with_capacity(indices.len());
    for &index in indices {
        let next = vertices.len() as u32;
        let local = *vertex_of.entry(index).or_insert(next);
        if local == next {
            let mut vertex = map.geometry.vertices.get(index as usize).ok_or_else(|| format!("{template}: vertex out of the package"))?.clone();
            let next_material = materials.len() as u32 + 1;
            let material = *material_of.entry(vertex.material).or_insert(next_material);
            if material == next_material {
                let mut m = map.materials.get(vertex.material as usize - 1).ok_or_else(|| format!("{template}: material out of the package"))?.clone();
                m.textures = [0; 5];
                materials.push(m);
            }
            vertex.material = material;
            for i in 0..3 {
                bounds[0][i] = bounds[0][i].min(vertex.position[i]);
                bounds[1][i] = bounds[1][i].max(vertex.position[i]);
            }
            vertices.push(vertex);
        }
        out_indices.push(local);
    }
    let sub = SkateMap {
        version: map.version,
        name: template.to_owned(),
        spawn: [0.0; 3],
        heading: 0.0,
        environment: map.environment.clone(),
        materials,
        textures: Vec::new(),
        geometry: Geometry { vertices, indices: out_indices, collision: Vec::new() },
        rails: Vec::new(),
        doors: Vec::new(),
        lights: Vec::new(),
        routes: Vec::new(),
        extensions: Vec::new(),
    };
    Ok((sub, bounds))
}
