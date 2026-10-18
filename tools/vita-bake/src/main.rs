//! vita-bake: converted private assets → skate vita packs. The output is
//! retail-derived: write it to `../game_data/SKATEVITA/`, outside the
//! repository, and upload it to `ux0:data/SKATEVITA/`; never commit it.
//!
//! usage: vita-bake all INSTALL OUT_DIR     every pack below (INSTALL is
//!                                          game_data/prepared/installations/<id>)
//!        vita-bake skater GLB OUT_DIR      skater.pak: the skinned skater and board
//!        vita-bake sim ASSETS OUT_DIR      sim.pak + anim.pak: the simulation data
//!        vita-bake audio EXPORT OUT_DIR    sound.pak: the board sounds (EXPORT: the reference audio exporter's audio/ folder)
//!        vita-bake ambience EXPORT OUT_DIR ambience.pak: the 22 zone beds (137 MB, streamed from the card)
//!        vita-bake ui ASSETS OUT_DIR [SCREENS]   ui.pak: the original HUD movie, the session marker panel and
//!                                          (with SCREENS, tools/prepare_ui_screen.py's folder) the linked screens
//!        vita-bake district-stats INSTALL [NAME…]   what whole districts cost
//!        vita-bake maps INSTALL OUT_DIR [NAME…]   map_<name>.pak for the 7 parks (or NAMEs)
mod audio;
mod district;
mod living;
mod map;
mod sim;
mod textures;
mod tiles;
mod ui;

use std::path::Path;
use std::process::ExitCode;

#[global_allocator]
static ALLOC: district::Counting = district::Counting;

use vita_pack::skater::{AlphaMode, Format, Joint, Material, Primitive, Skater, Texture, Vertex};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["all", install, out] => {
            let (install, out) = (Path::new(install), Path::new(out));
            bake_skater(&install.join("assets/private/skater.glb"), out)
                .and_then(|()| sim::bake(&install.join("assets"), out))
                .and_then(|()| ui::bake(&install.join("assets"), out, Some(&install.join("../../../ui-screens"))))
                .and_then(|()| map::bake(install, out, &map::PARKS.map(String::from)))
        }
        ["skater", glb, out] => bake_skater(Path::new(glb), Path::new(out)),
        ["sim", assets, out] => sim::bake(Path::new(assets), Path::new(out)),
        ["audio", export, out] => audio::bake(Path::new(export), Path::new(out)),
        ["ambience", export, out] => audio::bake_ambience(Path::new(export), Path::new(out)),
        ["ui", assets, out] => ui::bake(Path::new(assets), Path::new(out), None),
        ["ui", assets, out, screens] => ui::bake(Path::new(assets), Path::new(out), Some(Path::new(screens))),
        ["maps", install, out, names @ ..] => {
            let names: Vec<String> = if names.is_empty() {
                map::PARKS.iter().map(|s| s.to_string()).collect()
            } else {
                names.iter().map(|s| s.to_string()).collect()
            };
            map::bake(Path::new(install), Path::new(out), &names)
        }
        ["district-sim", install, out, names @ ..] => {
            let names: Vec<String> = if names.is_empty() {
                map::DISTRICTS.iter().map(|s| s.to_string()).collect()
            } else {
                names.iter().map(|s| s.to_string()).collect()
            };
            map::bake_district_sim(Path::new(install), Path::new(out), &names)
        }
        ["district", install, out, names @ ..] => {
            let names: Vec<String> = if names.is_empty() {
                map::DISTRICTS.iter().map(|s| s.to_string()).collect()
            } else {
                names.iter().map(|s| s.to_string()).collect()
            };
            map::bake_district(Path::new(install), Path::new(out), &names)
        }
        ["district-tiles", install, name] => {
            let install = Path::new(install);
            skate_data::skate_map::SkateMap::load(&install.join("maps").join(format!("{name}.skate")))
                .and_then(|map| {
                    let props_path = install.join("assets/private/native-props").join(format!("{name}.skate"));
                    let props = if props_path.is_file() {
                        Some(skate_data::skate_map::SkateMap::parse_render_only(&std::fs::read(&props_path).map_err(|e| e.to_string())?)?)
                    } else {
                        None
                    };
                    let started = std::time::Instant::now();
                    let baked = tiles::bake(&map, props.as_ref())?;
                    tiles::report(name, &baked);
                    println!("  baked in {:.0} s", started.elapsed().as_secs_f32());
                    Ok(())
                })
        }
        ["verify", dir] => verify_dir(Path::new(dir)),
        ["window-heap", assets, pack] => district::window_heap(Path::new(assets), Path::new(pack)),
        ["prop-stats", install, names @ ..] => map::prop_stats(Path::new(install), names),
        ["props", install, out] => map::bake_props(Path::new(install), Path::new(out)),
        ["living", install, out] => living::bake(Path::new(install), Path::new(out)),
        ["district-stats", install, names @ ..] => {
            let names: Vec<String> = if names.is_empty() {
                ["Industrial", "University", "DownTown"].map(String::from).to_vec()
            } else {
                names.iter().map(|s| s.to_string()).collect()
            };
            district::stats(Path::new(install), &names)
        }
        _ => Err("usage: vita-bake all INSTALL OUT_DIR | skater GLB OUT_DIR | sim ASSETS OUT_DIR | ui ASSETS OUT_DIR [SCREENS] | maps INSTALL OUT_DIR [NAME…] | district-stats INSTALL [NAME…] | district-sim INSTALL OUT_DIR [NAME…] | district INSTALL OUT_DIR [NAME…]".into()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("vita-bake: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Opens every pack of a folder as the game does and checks its hash: what a copy to another card
/// must still satisfy.
fn verify_dir(dir: &Path) -> Result<(), String> {
    let mut packs: Vec<_> = std::fs::read_dir(dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "pak"))
        .collect();
    packs.sort();
    let mut bad = 0;
    for path in &packs {
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let result = if name.starts_with("district_") {
            vita_pack::district::open(path).map(|(d, _)| format!("district, {} clusters, {} tiles, {} textures", d.clusters.len(), d.tiles.len(), d.textures.len()))
                .and_then(|info| vita_pack::district::verify_file(path).map(|()| info))
        } else {
            std::fs::read(path).map_err(|e| e.to_string()).and_then(|bytes| {
                let (kind, version, _) = vita_pack::open(&bytes)?;
                Ok(format!("{kind:?} v{version}"))
            })
        };
        match result {
            Ok(info) => println!("ok   {name} ({:.1} MiB): {info}", std::fs::metadata(path).map(|m| m.len()).unwrap_or(0) as f64 / 1048576.0),
            Err(e) => {
                bad += 1;
                println!("BAD  {name}: {e}");
            }
        }
    }
    if bad > 0 { Err(format!("{bad} of {} packs are damaged or of another version", packs.len())) } else { Ok(()) }
}

fn bake_skater(glb: &Path, out: &Path) -> Result<(), String> {
    let skater = skinned_model(glb, &|_| false)?;
    let pack = skater.to_pack();
    std::fs::create_dir_all(out).map_err(|e| e.to_string())?;
    let path = out.join("skater.pak");
    std::fs::write(&path, &pack).map_err(|e| format!("{}: {e}", path.display()))?;
    let texture_bytes: usize = skater.textures.iter().flat_map(|t| &t.mips).map(Vec::len).sum();
    println!(
        "{}: {} joints, {} vertices, {} triangles, {} primitives, {} materials, {} textures ({} KiB), pack {} KiB",
        path.display(),
        skater.joints.len(),
        skater.vertex_count(),
        skater.indices.len() / 3,
        skater.primitives.len(),
        skater.materials.len(),
        skater.textures.len(),
        texture_bytes / 1024,
        pack.len() / 1024
    );
    for p in &skater.primitives {
        let m = &skater.materials[usize::from(p.material)];
        let t = m.texture.map(|t| &skater.textures[usize::from(t)]);
        println!(
            "  {:<12} {:>5} triangles  {:?}{}",
            m.name,
            p.index_count / 3,
            m.alpha_mode,
            t.map(|t| format!("  texture {}x{}", t.width, t.height)).unwrap_or_default()
        );
    }
    Ok(())
}

/// A skinned GLB (the skater, the traffic cars) as a `vita_pack::skater::Skater`: the skin's
/// joints parents first, every primitive of the skinned mesh in one vertex buffer, the base colour
/// textures baked. `blend` marks the materials drawn alpha-blended whatever the GLB says (the cars'
/// glass: retail blends it, the export writes it opaque).
pub(crate) fn skinned_model(glb: &Path, blend: &dyn Fn(&gltf::Material) -> bool) -> Result<Skater, String> {
    let first = std::cell::Cell::new(true);
    // The first skinned mesh node (LOD 0); the material keeps its GLB name.
    skinned_model_nodes(
        glb,
        &|n| (n.mesh().is_some() && n.skin().is_some() && first.replace(false)).then_some(0),
        &|m, _| (m.name().unwrap_or("").to_owned(), blend(m)),
    )
}

/// [`skinned_model`] over the skinned mesh nodes `pick` accepts (its value: the LOD the node's
/// primitives belong to, in node order), each material once per LOD with the name and blend flag
/// `material` gives (the peds' LOD0 and LOD1 in one model, the LOD and the ped tint marked in
/// the material names; the textures are shared). A GLB without a skin is a rigid model: one joint
/// (`root`, identity bind) that every vertex follows (the ped hand props).
pub(crate) fn skinned_model_nodes(glb: &Path, pick: &dyn Fn(&gltf::Node) -> Option<u8>, material: &dyn Fn(&gltf::Material, u8) -> (String, bool)) -> Result<Skater, String> {
    let (doc, buffers, images) = gltf::import(glb).map_err(|e| format!("{}: {e}", glb.display()))?;
    let skin = doc.skins().next();
    let rigid = skin.is_none();
    let (joints, order): (Vec<Joint>, Vec<usize>) = match skin {
        None => {
            let identity: [f32; 16] = std::array::from_fn(|i| f32::from(u8::from(i % 5 == 0)));
            (vec![Joint { name: "root".into(), parent: None, inverse_bind: identity }], Vec::new())
        }
        Some(skin) => {
    let joint_nodes: Vec<gltf::Node> = skin.joints().collect();
    let joint_of = |node: usize| joint_nodes.iter().position(|j| j.index() == node);

    // Scene-graph parents, then each joint's nearest ancestor joint. A non-joint
    // ancestor must be identity (skate-game `bind` rejects other conventions).
    let mut parent_node = vec![None; doc.nodes().len()];
    for node in doc.nodes() {
        for child in node.children() {
            parent_node[child.index()] = Some(node.index());
        }
    }
    let node_by_index: Vec<gltf::Node> = doc.nodes().collect();
    let mut joint_parent = Vec::with_capacity(joint_nodes.len());
    for joint in &joint_nodes {
        let mut at = parent_node[joint.index()];
        let mut found = None;
        while let Some(p) = at {
            if let Some(j) = joint_of(p) {
                found = Some(j);
                break;
            }
            let m = node_by_index[p].transform().matrix();
            let identity = (0..4).all(|c| (0..4).all(|r| (m[c][r] - f32::from(u8::from(c == r))).abs() < 1e-5));
            if !identity {
                return Err(format!(
                    "skin ancestor {:?} of {:?} is not identity",
                    node_by_index[p].name(),
                    joint.name()
                ));
            }
            at = parent_node[p];
        }
        joint_parent.push(found);
    }
    // Parents before children, keeping the GLB order otherwise.
    let mut order: Vec<usize> = Vec::with_capacity(joint_nodes.len());
    while order.len() < joint_nodes.len() {
        let before = order.len();
        for j in 0..joint_nodes.len() {
            if !order.contains(&j) && joint_parent[j].is_none_or(|p| order.contains(&p)) {
                order.push(j);
            }
        }
        if order.len() == before {
            return Err("the skin's joint hierarchy has a cycle".into());
        }
    }
    let new_index = |old: usize| order.iter().position(|&o| o == old).unwrap();
    let inverse_binds: Vec<[[f32; 4]; 4]> = skin
        .reader(|b| Some(&buffers[b.index()]))
        .read_inverse_bind_matrices()
        .ok_or("the skin has no inverse bind matrices")?
        .collect();
    let joints: Vec<Joint> = order
        .iter()
        .map(|&old| Joint {
            name: joint_nodes[old].name().unwrap_or("").to_owned(),
            parent: joint_parent[old].map(|p| new_index(p) as u16),
            inverse_bind: inverse_binds[old].concat().try_into().unwrap(),
        })
        .collect();
            (joints, order)
        }
    };
    let new_index = |old: usize| if rigid { 0 } else { order.iter().position(|&o| o == old).unwrap() };

    // The skinned meshes: every primitive into one vertex buffer.
    let nodes: Vec<(gltf::Mesh, u8)> = doc.nodes().filter_map(|n| Some((n.mesh()?, pick(&n)?))).collect();
    if nodes.is_empty() {
        return Err("no skinned mesh node".into());
    }
    let mut vertex_data = Vec::new();
    let mut indices: Vec<u16> = Vec::new();
    let mut primitives = Vec::new();
    let mut materials: Vec<Material> = Vec::new();
    let mut material_of: Vec<((Option<usize>, u8), u16)> = Vec::new();
    let mut texture_of_image: Vec<(usize, u16)> = Vec::new();
    let mut textures: Vec<Texture> = Vec::new();
    for (primitive, lod) in nodes.iter().flat_map(|(mesh, lod)| mesh.primitives().map(move |p| (p, *lod))) {
        if primitive.mode() != gltf::mesh::Mode::Triangles {
            return Err("only triangle primitives are supported".into());
        }
        let r = primitive.reader(|b| Some(&buffers[b.index()]));
        let positions: Vec<[f32; 3]> = r.read_positions().ok_or("primitive without positions")?.collect();
        let normals: Vec<[f32; 3]> = r.read_normals().ok_or("primitive without normals")?.collect();
        let uvs: Vec<[f32; 2]> = r.read_tex_coords(0).ok_or("primitive without uvs")?.into_f32().collect();
        let skin_joints: Vec<[u16; 4]> = match r.read_joints(0) {
            Some(j) => j.into_u16().collect(),
            None if rigid => vec![[0; 4]; positions.len()],
            None => return Err("primitive without joints".into()),
        };
        let weights: Vec<[f32; 4]> = match r.read_weights(0) {
            Some(w) => w.into_f32().collect(),
            None if rigid => vec![[1.0, 0.0, 0.0, 0.0]; positions.len()],
            None => return Err("primitive without weights".into()),
        };
        let base = vertex_data.len() / vita_pack::skater::STRIDE;
        for i in 0..positions.len() {
            let joints = skin_joints[i].map(|j| new_index(usize::from(j)) as u8);
            Vertex {
                position: positions[i],
                normal: normals[i].map(|v| (v.clamp(-1.0, 1.0) * 127.0).round() as i8),
                uv: uvs[i],
                joints,
                weights: quantize(weights[i]),
            }
            .encode(&mut vertex_data);
        }
        let first = indices.len();
        for i in r.read_indices().ok_or("primitive without indices")?.into_u32() {
            let index = base + i as usize;
            indices.push(u16::try_from(index).map_err(|_| "more than 65536 vertices")?);
        }
        let gltf_material = primitive.material();
        let key = (gltf_material.index(), lod);
        let material = match material_of.iter().find(|(k, _)| *k == key) {
            Some(&(_, m)) => m,
            None => {
                let pbr = gltf_material.pbr_metallic_roughness();
                let (name, blended) = material(&gltf_material, lod);
                let texture = match pbr.base_color_texture() {
                    Some(info) => {
                        let image = info.texture().source().index();
                        Some(match texture_of_image.iter().find(|(i, _)| *i == image) {
                            Some(&(_, t)) => t,
                            None => {
                                let alpha = blended || gltf_material.alpha_mode() != gltf::material::AlphaMode::Opaque;
                                textures.push(texture(&images[image], alpha)?);
                                let t = (textures.len() - 1) as u16;
                                texture_of_image.push((image, t));
                                t
                            }
                        })
                    }
                    None => None,
                };
                materials.push(Material {
                    name,
                    base_color: pbr.base_color_factor(),
                    texture,
                    alpha_mode: match gltf_material.alpha_mode() {
                        _ if blended => AlphaMode::Blend,
                        gltf::material::AlphaMode::Opaque => AlphaMode::Opaque,
                        gltf::material::AlphaMode::Mask => AlphaMode::Mask,
                        gltf::material::AlphaMode::Blend => AlphaMode::Blend,
                    },
                    alpha_cutoff: gltf_material.alpha_cutoff().unwrap_or(0.5),
                    double_sided: gltf_material.double_sided(),
                });
                let m = (materials.len() - 1) as u16;
                material_of.push((key, m));
                m
            }
        };
        primitives.push(Primitive {
            first_index: first as u32,
            index_count: (indices.len() - first) as u32,
            material,
        });
    }
    let skater = Skater {
        joints,
        vertex_data,
        indices,
        primitives,
        materials,
        textures,
    };
    skater.validate()?;
    Ok(skater)
}

/// Weights to u8 summing to exactly 255 (largest remainders get the rounding).
fn quantize(w: [f32; 4]) -> [u8; 4] {
    let sum: f32 = w.iter().map(|v| v.max(0.0)).sum();
    if sum <= 0.0 {
        return [255, 0, 0, 0];
    }
    let scaled = w.map(|v| v.max(0.0) / sum * 255.0);
    let mut q = scaled.map(|v| v.floor() as u8);
    let mut left = 255 - q.iter().map(|&v| u32::from(v)).sum::<u32>();
    let mut by_remainder = [0, 1, 2, 3];
    by_remainder.sort_by(|&a, &b| (scaled[b] - scaled[b].floor()).total_cmp(&(scaled[a] - scaled[a].floor())));
    for &i in by_remainder.iter().cycle() {
        if left == 0 {
            break;
        }
        q[i] += 1;
        left -= 1;
    }
    q
}

/// A GLB image as a skater texture: RGBA8 pixels, then the shared baker (DXT5 when the
/// material uses alpha, else DXT1).
fn texture(data: &gltf::image::Data, alpha: bool) -> Result<Texture, String> {
    use gltf::image::Format as Source;
    let rgba: Vec<u8> = match data.format {
        // An opaque material's alpha is not coverage (the cars' body atlas keeps a mask there): DXT1
        // would make every texel below 128 transparent black.
        Source::R8G8B8A8 if !alpha => data.pixels.chunks(4).flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
        Source::R8G8B8A8 => data.pixels.clone(),
        Source::R8G8B8 => data.pixels.chunks(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
        f => return Err(format!("unsupported texture format {f:?}")),
    };
    textures::bake(&rgba, data.width, data.height, if alpha { Format::Dxt5 } else { Format::Dxt1 })
}

#[cfg(test)]
mod tests {
    use super::quantize;

    #[test]
    fn weights_sum_to_255() {
        for w in [[0.5, 0.5, 0.0, 0.0], [1.0, 0.0, 0.0, 0.0], [0.333, 0.333, 0.334, 0.0], [0.1, 0.2, 0.3, 0.4], [0.0; 4]] {
            let q = quantize(w);
            assert_eq!(q.iter().map(|&v| u32::from(v)).sum::<u32>(), 255, "{w:?} -> {q:?}");
        }
        assert_eq!(quantize([0.75, 0.25, 0.0, 0.0]), [191, 64, 0, 0]);
    }
}
