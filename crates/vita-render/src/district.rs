//! A campaign district on the GPU: 64 m tiles with three levels of detail
//! that come and go while the skater moves, and the textures they share, evicted by use.
//! The renderer only holds what it is given: `skate-vita`'s streamer decides what to load and drop.
use core::ffi::c_void;
use std::collections::HashMap;

use glam::{Vec3, Vec4};
use vita_pack::district::{GREY_LIGHTMAP, NO_TEXTURE, TileMesh};
use vita_pack::map::{LIGHTMAP_UV, POSITION, STRIDE, UV};
use vita_pack::texture::Texture;

use crate::scene::{ParkProgram, attributes, buffer, frustum, upload_texture, visible};
use crate::vgl::*;

/// A tile is drawn at level 0 up to this distance (m), then level 1, then level 2.
pub const LOD_RANGES: [f32; 2] = [90.0, 160.0];
/// Nothing farther than this is drawn (the fog ends before it).
pub const RADIUS: f32 = 250.0;
/// A draw whose bounds subtend less than this (radius over distance; about 6 pixels across at
/// 960 x 544) is not submitted: a dense street has hundreds of small pieces per tile and the
/// frame is bound by the number of draw calls, not by triangles.
pub const SMALL: f32 = 0.010;
/// Fog start and end (m).
pub const FOG: [f32; 2] = [150.0, 235.0];

struct Draw {
    first_index: u32,
    index_count: u32,
    albedo: u32,
    lightmap: u32,
    tint: [f32; 4],
    cutoff: Option<f32>,
    min: Vec3,
    max: Vec3,
}

struct MeshGpu {
    vbo: GLuint,
    ibo: GLuint,
    draws: Vec<Draw>,
    bytes: usize,
}

struct TextureGpu {
    id: GLuint,
    bytes: usize,
    used: u64,
}

pub struct DistrictGpu {
    pub tile_size: f32,
    horizon: [f32; 3],
    scale: f32,
    tiles: HashMap<(i32, i32), [Option<Vec<MeshGpu>>; 3]>,
    /// Levels being uploaded one mesh at a time: drawn once finished.
    building: HashMap<((i32, i32), usize), Vec<MeshGpu>>,
    textures: HashMap<u32, TextureGpu>,
    /// Texture of far levels that have no lightmap: a mid grey.
    grey: GLuint,
    frame: u64,
    pub mesh_bytes: usize,
    pub texture_bytes: usize,
}

impl DistrictGpu {
    pub fn new(tile_size: f32, horizon: [f32; 3], scale: f32) -> Self {
        let grey = upload_texture(&Texture { width: 1, height: 1, format: vita_pack::texture::Format::Rgba8, mips: vec![vec![150, 150, 150, 255]] });
        Self { tile_size, horizon, scale, tiles: HashMap::new(), building: HashMap::new(), textures: HashMap::new(), grey, frame: 0, mesh_bytes: 0, texture_bytes: 0 }
    }

    /// The exposure scale the tiles are drawn with (the props in the district use it too).
    pub fn scale(&self) -> f32 {
        self.scale
    }

    pub fn horizon(&self) -> [f32; 3] {
        self.horizon
    }

    pub fn has_tile(&self, tile: (i32, i32), lod: usize) -> bool {
        self.tiles.get(&tile).is_some_and(|l| l[lod].is_some())
    }

    pub fn has_texture(&self, id: u32) -> bool {
        self.textures.contains_key(&id)
    }

    /// The (tile, level) pairs on the GPU.
    pub fn resident(&self) -> Vec<((i32, i32), usize)> {
        self.tiles.iter().flat_map(|(k, l)| (0..3).filter(|&i| l[i].is_some()).map(move |i| (*k, i))).collect()
    }

    pub fn texture_ids(&self) -> Vec<u32> {
        self.textures.keys().copied().collect()
    }

    /// Uploads one level of a tile (replacing a resident one) in one go.
    pub fn add_tile(&mut self, tile: (i32, i32), lod: usize, meshes: &[TileMesh]) {
        self.begin_tile(tile, lod);
        for m in meshes {
            self.add_mesh(tile, lod, m);
        }
        self.finish_tile(tile, lod);
    }

    /// Starts a level uploaded mesh by mesh (a frame's budget is spent between meshes).
    pub fn begin_tile(&mut self, tile: (i32, i32), lod: usize) {
        self.drop_tile(tile, lod);
        self.building.insert((tile, lod), Vec::new());
    }

    pub fn add_mesh(&mut self, tile: (i32, i32), lod: usize, m: &TileMesh) {
        let Some(gpu) = self.building.get_mut(&(tile, lod)) else { return };
        {
            let index_bytes: Vec<u8> = m.indices.iter().flat_map(|i| i.to_le_bytes()).collect();
            let (vbo, ibo) = unsafe {
                let vbo = buffer(GL_ARRAY_BUFFER, &m.vertex_data);
                let ibo = buffer(GL_ELEMENT_ARRAY_BUFFER, &index_bytes);
                glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, 0);
                (vbo, ibo)
            };
            let bytes = m.vertex_data.len() + index_bytes.len();
            self.mesh_bytes += bytes;
            gpu.push(MeshGpu {
                vbo,
                ibo,
                bytes,
                draws: m
                    .draws
                    .iter()
                    .map(|d| Draw {
                        first_index: d.first_index,
                        index_count: d.index_count,
                        albedo: d.albedo,
                        lightmap: d.lightmap,
                        tint: d.color,
                        cutoff: d.alpha_cutoff,
                        min: Vec3::from_array(d.bounds_min),
                        max: Vec3::from_array(d.bounds_max),
                    })
                    .collect(),
            });
        }
    }

    /// The level is complete: it is drawn from now on.
    pub fn finish_tile(&mut self, tile: (i32, i32), lod: usize) {
        if let Some(gpu) = self.building.remove(&(tile, lod)) {
            self.tiles.entry(tile).or_default()[lod] = Some(gpu);
        }
    }

    pub fn drop_tile(&mut self, tile: (i32, i32), lod: usize) {
        let Some(levels) = self.tiles.get_mut(&tile) else { return };
        if let Some(meshes) = levels[lod].take() {
            for m in meshes {
                unsafe {
                    glDeleteBuffers(1, &m.vbo);
                    glDeleteBuffers(1, &m.ibo);
                }
                self.mesh_bytes -= m.bytes;
            }
        }
        if levels.iter().all(Option::is_none) {
            self.tiles.remove(&tile);
        }
    }

    pub fn add_texture(&mut self, id: u32, t: &Texture) {
        if self.textures.contains_key(&id) {
            return;
        }
        let bytes = t.bytes();
        self.texture_bytes += bytes;
        self.textures.insert(id, TextureGpu { id: upload_texture(t), bytes, used: self.frame });
    }

    /// Frees the least recently used textures (not used in the last 30 frames) until `budget` bytes remain.
    pub fn trim_textures(&mut self, budget: usize) {
        if self.texture_bytes <= budget {
            return;
        }
        let mut order: Vec<(u64, u32)> = self.textures.iter().filter(|(_, t)| t.used + 30 < self.frame).map(|(id, t)| (t.used, *id)).collect();
        order.sort_unstable();
        for (_, id) in order {
            if self.texture_bytes <= budget {
                break;
            }
            if let Some(t) = self.textures.remove(&id) {
                unsafe { glDeleteTextures(1, &t.id) };
                self.texture_bytes -= t.bytes;
            }
        }
    }

    /// The level a tile is drawn at: the one its distance asks for, else the nearest resident one.
    fn level(&self, tile: (i32, i32), distance: f32) -> Option<usize> {
        let levels = self.tiles.get(&tile)?;
        let wanted: usize = if distance < LOD_RANGES[0] { 0 } else if distance < LOD_RANGES[1] { 1 } else { 2 };
        [wanted, wanted.saturating_sub(1), 0, 1, 2].into_iter().filter(|&l| l <= 2).find(|&l| levels[l].is_some())
    }

    /// Distance (m, horizontal) from `eye` to the tile's square.
    pub fn distance(&self, tile: (i32, i32), eye: Vec3) -> f32 {
        let s = self.tile_size;
        let (x0, z0) = (tile.0 as f32 * s, tile.1 as f32 * s);
        let dx = (x0 - eye.x).max(eye.x - (x0 + s)).max(0.0);
        let dz = (z0 - eye.z).max(eye.z - (z0 + s)).max(0.0);
        (dx * dx + dz * dz).sqrt()
    }

    /// Draws every tile within `RADIUS`: opaque draws of all tiles, then the alpha-tested ones.
    /// Returns (submitted, culled).
    pub fn draw(&mut self, opaque: &ParkProgram, masked: &ParkProgram, white: GLuint, view_proj: &glam::Mat4, eye: Vec3) -> (u32, u32) {
        self.frame += 1;
        let planes: [Vec4; 6] = frustum(view_proj);
        let vp = view_proj.to_cols_array();
        let mut tiles: Vec<((i32, i32), f32)> = self.tiles.keys().map(|&t| (t, self.distance(t, eye))).filter(|(_, d)| *d < RADIUS).collect();
        tiles.sort_by(|a, b| a.1.total_cmp(&b.1));
        let (mut submitted, mut culled) = (0, 0);
        unsafe {
            glDisable(GL_CULL_FACE);
            for pass_masked in [false, true] {
                let p = if pass_masked { masked } else { opaque };
                glUseProgram(p.program.id);
                glUniformMatrix4fv(p.viewproj, 1, GL_FALSE, vp.as_ptr());
                glUniform3f(p.eye, eye.x, eye.y, eye.z);
                glUniform2f(p.fog, FOG[0], FOG[1]);
                glUniform3f(p.horizon, self.horizon[0], self.horizon[1], self.horizon[2]);
                glUniform1f(p.scale, self.scale);
                // State is set only when it changes (the baked draws are ordered by texture).
                let mut last_albedo = GLuint::MAX;
                let mut last_lightmap = GLuint::MAX;
                let mut last_tint = [f32::NAN; 4];
                let mut last_cutoff = f32::NAN;
                for &(tile, distance) in &tiles {
                    let Some(lod) = self.level(tile, distance) else { continue };
                    let frame = self.frame;
                    let grey = self.grey;
                    let Some(meshes) = self.tiles.get(&tile).and_then(|l| l[lod].as_ref()) else { continue };
                    for mesh in meshes {
                        let mut bound = false;
                        for d in &mesh.draws {
                            if d.cutoff.is_some() != pass_masked {
                                continue;
                            }
                            if !visible(&planes, d.min, d.max) {
                                culled += 1;
                                continue;
                            }
                            let centre = (d.min + d.max) * 0.5;
                            if (d.max - d.min).length() * 0.5 < (centre - eye).length() * SMALL {
                                culled += 1;
                                continue;
                            }
                            let albedo = if d.albedo == NO_TEXTURE { Some(white) } else { self.textures.get_mut(&d.albedo).map(|t| { t.used = frame; t.id }) };
                            let lightmap = if d.lightmap == NO_TEXTURE { Some(white) } else if d.lightmap == GREY_LIGHTMAP { Some(grey) } else { self.textures.get_mut(&d.lightmap).map(|t| { t.used = frame; t.id }) };
                            // A texture still on its way: the draw waits (no white flash).
                            let (Some(albedo), Some(lightmap)) = (albedo, lightmap) else { continue };
                            if !bound {
                                glBindBuffer(GL_ARRAY_BUFFER, mesh.vbo);
                                glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, mesh.ibo);
                                attributes(&[
                                    (3, GL_FLOAT, GL_FALSE, POSITION),
                                    (2, GL_FLOAT, GL_FALSE, UV),
                                    (2, GL_FLOAT, GL_FALSE, LIGHTMAP_UV),
                                ], STRIDE);
                                bound = true;
                            }
                            if let (Some(location), Some(cutoff)) = (p.cutoff, d.cutoff) {
                                if cutoff != last_cutoff {
                                    glUniform1f(location, cutoff);
                                    last_cutoff = cutoff;
                                }
                            }
                            if d.tint != last_tint {
                                glUniform4f(p.tint, d.tint[0], d.tint[1], d.tint[2], d.tint[3]);
                                last_tint = d.tint;
                            }
                            if albedo != last_albedo {
                                glActiveTexture(GL_TEXTURE0);
                                glBindTexture(GL_TEXTURE_2D, albedo);
                                last_albedo = albedo;
                            }
                            if lightmap != last_lightmap {
                                glActiveTexture(GL_TEXTURE1);
                                glBindTexture(GL_TEXTURE_2D, lightmap);
                                last_lightmap = lightmap;
                            }
                            glDrawElements(GL_TRIANGLES, d.index_count as GLsizei, GL_UNSIGNED_SHORT, (d.first_index as usize * 2) as *const c_void);
                            submitted += 1;
                        }
                    }
                }
            }
            glActiveTexture(GL_TEXTURE0);
            glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, 0);
        }
        (submitted, culled)
    }

    /// Frees everything.
    pub fn free(&mut self) {
        for (tile, _) in self.tiles.keys().map(|k| (*k, ())).collect::<Vec<_>>() {
            for lod in 0..3 {
                self.drop_tile(tile, lod);
            }
        }
        for (_, meshes) in self.building.drain() {
            for m in meshes {
                unsafe {
                    glDeleteBuffers(1, &m.vbo);
                    glDeleteBuffers(1, &m.ibo);
                }
            }
        }
        self.mesh_bytes = 0;
        let ids: Vec<GLuint> = self.textures.values().map(|t| t.id).collect();
        unsafe {
            glDeleteTextures(ids.len() as GLsizei, ids.as_ptr());
            glDeleteTextures(1, &self.grey);
        }
        self.textures.clear();
        self.texture_bytes = 0;
    }
}
