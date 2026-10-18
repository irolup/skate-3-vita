//! `map_<name>.pak` (kind `Map`): one retail park.
//!
//! - **Sim**: the converted `.skate` map without its render geometry and texture
//!   pixels — everything the simulation reads (spawn, heading, environment, materials,
//!   texture names and sizes, portable collision, rails, lights, routes and every
//!   extension, i.e. the retail collision archive `RWCM` and grind metadata `WMET`),
//!   stored value for value (floats as their bits). [`Map::sim_map`] rebuilds the
//!   `SkateMap` that `skate-sim` builds its world from, so collision and rails are
//!   the retail ones, unchanged. The bake may replace an unusable authored spawn; the
//!   authored one is kept in `authored_spawn`.
//! - **Render**: material draw groups (each with its own vertex range and u16 indices),
//!   albedo and lightmap textures, the horizon colour and an exposure.
use crate::texture::Texture;
use crate::{Kind, Reader, Writer};
use skate_data::skate_map::{
    Collision, Extension, Geometry, Light, Material, Rail, Route, SkateMap, Texture as MapTexture,
};

/// v1: first map pack.
pub const VERSION: u32 = 1;
/// Map vertex: position (3 × f32), uv (2 × f32), lightmap uv (2 × f32).
pub const STRIDE: usize = 28;
pub const POSITION: usize = 0;
pub const UV: usize = 12;
pub const LIGHTMAP_UV: usize = 20;

#[derive(Clone, Debug, PartialEq)]
pub struct Draw {
    /// First vertex of this draw's range (indices are relative to it).
    pub first_vertex: u32,
    pub vertex_count: u32,
    pub first_index: u32,
    pub index_count: u32,
    pub albedo: Option<u16>,
    pub lightmap: Option<u16>,
    /// Material colour factor (RGBA).
    pub color: [f32; 4],
    /// Alpha test threshold for masked materials.
    pub alpha_cutoff: Option<f32>,
    pub bounds_min: [f32; 3],
    pub bounds_max: [f32; 3],
}

#[derive(Clone, Debug, PartialEq)]
pub struct Render {
    pub vertex_data: Vec<u8>,
    pub indices: Vec<u16>,
    pub draws: Vec<Draw>,
    pub textures: Vec<Texture>,
    /// Clear colour (linear RGB): the map horizon, environment[3..6].
    pub horizon: [f32; 3],
    pub exposure: f32,
}

#[derive(Debug, PartialEq)]
pub struct Map {
    /// `SkateMap` with empty texture pixels and render geometry.
    pub sim: SkateMap,
    pub authored_spawn: [f32; 3],
    pub authored_heading: f32,
    pub render: Render,
}

impl Map {
    /// The map the simulation is built from (the render parts empty).
    pub fn sim_map(self) -> SkateMap {
        self.sim
    }

    pub fn to_pack(&self) -> Vec<u8> {
        let mut w = Writer::default();
        write_sim(&mut w, &self.sim);
        for v in self.authored_spawn {
            w.f32(v);
        }
        w.f32(self.authored_heading);
        write_render(&mut w, &self.render);
        crate::seal(Kind::Map, VERSION, &w.bytes)
    }

    pub fn from_pack(bytes: &[u8]) -> Result<Self, String> {
        let (kind, version, payload) = crate::open(bytes)?;
        if kind != Kind::Map || version != VERSION {
            return Err(format!("expected a map pack v{VERSION}, found {kind:?} v{version}"));
        }
        let mut r = Reader::new(payload);
        let sim = read_sim(&mut r)?;
        let authored_spawn = [r.f32()?, r.f32()?, r.f32()?];
        let authored_heading = r.f32()?;
        let render = read_render(&mut r)?;
        r.done()?;
        Ok(Self { sim, authored_spawn, authored_heading, render })
    }
}

/// The movable props' render of a world: template meshes (template space) sharing
/// vertices and textures, and each template's index range in the props package (its MOBJ
/// records' `first_index` / `index_count`) with its draws.
#[derive(Debug, PartialEq)]
pub struct PropRender {
    pub render: Render,
    pub templates: Vec<PropTemplate>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PropTemplate {
    pub first_index: u32,
    pub index_count: u32,
    pub draws: [u32; 2],
}

const PROPS_VERSION: u32 = 1;

impl PropRender {
    pub fn to_pack(&self) -> Vec<u8> {
        let mut w = Writer::default();
        write_render(&mut w, &self.render);
        w.u32(self.templates.len() as u32);
        for t in &self.templates {
            w.u32(t.first_index);
            w.u32(t.index_count);
            w.u32(t.draws[0]);
            w.u32(t.draws[1]);
        }
        crate::seal(Kind::Props, PROPS_VERSION, &w.bytes)
    }

    pub fn from_pack(bytes: &[u8]) -> Result<Self, String> {
        let (kind, version, payload) = crate::open(bytes)?;
        if kind != Kind::Props || version != PROPS_VERSION {
            return Err(format!("expected a props render v{PROPS_VERSION}, found {kind:?} v{version}"));
        }
        let mut r = Reader::new(payload);
        let render = read_render(&mut r)?;
        let n = r.u32()? as usize;
        let mut templates = Vec::with_capacity(n.min(4096));
        for _ in 0..n {
            let t = PropTemplate { first_index: r.u32()?, index_count: r.u32()?, draws: [r.u32()?, r.u32()?] };
            if t.draws[0] > t.draws[1] || t.draws[1] as usize > render.draws.len() {
                return Err("props render: bad template draws".into());
            }
            templates.push(t);
        }
        r.done()?;
        Ok(Self { render, templates })
    }
}

/// The sim part of a decoded `.skate` map: render geometry and texture pixels dropped
/// (`validate_runtime` rejects maps with doors, so there are none to keep).
pub fn strip_render(map: SkateMap) -> SkateMap {
    let SkateMap { version, name, spawn, heading, environment, materials, textures, geometry, rails, doors: _, lights, routes, extensions } = map;
    SkateMap {
        version,
        name,
        spawn,
        heading,
        environment,
        materials,
        textures: textures
            .into_iter()
            .map(|t| MapTexture { rgba: Vec::new(), ..t })
            .collect(),
        geometry: Geometry { vertices: Vec::new(), indices: Vec::new(), collision: geometry.collision },
        rails,
        doors: Vec::new(),
        lights,
        routes,
        extensions,
    }
}

/// The sim part alone, in the pack's encoding (bit-exact copies; the types are not
/// `Clone`).
pub fn encode_sim(m: &SkateMap) -> Vec<u8> {
    let mut w = Writer::default();
    write_sim(&mut w, m);
    w.bytes
}
pub fn decode_sim(bytes: &[u8]) -> Result<SkateMap, String> {
    let mut r = Reader::new(bytes);
    let m = read_sim(&mut r)?;
    r.done()?;
    Ok(m)
}

fn write_f32s(w: &mut Writer, v: &[f32]) {
    w.u32(v.len() as u32);
    for &x in v {
        w.f32(x);
    }
}
fn read_f32s(r: &mut Reader) -> Result<Vec<f32>, String> {
    (0..r.u32()?).map(|_| r.f32()).collect()
}
fn write_bytes_opt(w: &mut Writer, b: &Option<Vec<u8>>) {
    match b {
        Some(b) => {
            w.u8(1);
            w.block(b);
        }
        None => w.u8(0),
    }
}
fn read_bytes_opt(r: &mut Reader) -> Result<Option<Vec<u8>>, String> {
    Ok(match r.u8()? {
        0 => None,
        _ => Some(r.block()?.to_vec()),
    })
}
fn write_rail(w: &mut Writer, rail: &Rail) {
    w.str(&rail.name);
    w.u8(u8::from(rail.closed));
    w.u32(rail.points.len() as u32);
    for p in &rail.points {
        p.iter().for_each(|&v| w.f32(v));
    }
    write_bytes_opt(w, &rail.native);
}
fn read_rail(r: &mut Reader) -> Result<Rail, String> {
    Ok(Rail {
        name: r.str()?,
        closed: r.u8()? != 0,
        points: (0..r.u32()?).map(|_| Ok([r.f32()?, r.f32()?, r.f32()?])).collect::<Result<_, String>>()?,
        native: read_bytes_opt(r)?,
    })
}

fn write_sim(w: &mut Writer, m: &SkateMap) {
    assert!(m.doors.is_empty(), "maps with doors are rejected by validate_runtime");
    assert!(m.geometry.vertices.is_empty() && m.textures.iter().all(|t| t.rgba.is_empty()), "strip_render first");
    w.u8(m.version);
    w.str(&m.name);
    m.spawn.iter().for_each(|&v| w.f32(v));
    w.f32(m.heading);
    write_f32s(w, &m.environment);
    w.u32(m.materials.len() as u32);
    for x in &m.materials {
        w.str(&x.name);
        w.u32(x.flags);
        w.f32(x.friction);
        w.f32(x.restitution);
        x.color.iter().for_each(|&v| w.f32(v));
        w.f32(x.roughness);
        w.f32(x.emissive);
        x.textures.iter().for_each(|&v| w.u32(v));
        w.f32(x.indirect_strength);
        w.u32(x.alpha_mode);
        w.f32(x.alpha_cutoff);
        w.u32(x.audio);
        w.u32(x.physics);
        w.u32(x.pattern);
        match x.depth_layer {
            Some(v) => {
                w.u8(1);
                w.u32(v);
            }
            None => w.u8(0),
        }
        write_bytes_opt(w, &x.retail_definition);
    }
    w.u32(m.textures.len() as u32);
    for t in &m.textures {
        w.str(&t.name);
        w.u32(t.width);
        w.u32(t.height);
        w.u32(t.color_space);
    }
    w.u32(m.geometry.collision.len() as u32);
    for c in &m.geometry.collision {
        c.points.iter().flatten().for_each(|&v| w.f32(v));
        w.u32(c.surface);
        w.u32(c.material);
        match c.native_edges {
            Some(e) => {
                w.u8(1);
                e.iter().for_each(|&b| w.u8(b));
            }
            None => w.u8(0),
        }
    }
    w.u32(m.rails.len() as u32);
    m.rails.iter().for_each(|rail| write_rail(w, rail));
    w.u32(m.lights.len() as u32);
    for l in &m.lights {
        w.str(&l.name);
        w.u32(l.kind);
        for v in [l.position, l.direction, l.color] {
            v.iter().for_each(|&x| w.f32(x));
        }
        for v in [l.intensity, l.range, l.radius, l.inner_cos, l.outer_cos] {
            w.f32(v);
        }
    }
    w.u32(m.routes.len() as u32);
    for route in &m.routes {
        write_rail(w, &route.rail);
        w.u32(route.skaters);
        w.f32(route.speed);
        w.f32(route.spacing);
    }
    w.u32(m.extensions.len() as u32);
    for e in &m.extensions {
        e.tag.iter().for_each(|&b| w.u8(b));
        w.u32(e.schema);
        w.block(&e.payload);
    }
}

fn read_sim(r: &mut Reader) -> Result<SkateMap, String> {
    let version = r.u8()?;
    let name = r.str()?;
    let spawn = [r.f32()?, r.f32()?, r.f32()?];
    let heading = r.f32()?;
    let environment = read_f32s(r)?;
    let materials = (0..r.u32()?)
        .map(|_| -> Result<Material, String> {
            Ok(Material {
                name: r.str()?,
                flags: r.u32()?,
                friction: r.f32()?,
                restitution: r.f32()?,
                color: [r.f32()?, r.f32()?, r.f32()?],
                roughness: r.f32()?,
                emissive: r.f32()?,
                textures: [r.u32()?, r.u32()?, r.u32()?, r.u32()?, r.u32()?],
                indirect_strength: r.f32()?,
                alpha_mode: r.u32()?,
                alpha_cutoff: r.f32()?,
                audio: r.u32()?,
                physics: r.u32()?,
                pattern: r.u32()?,
                depth_layer: match r.u8()? {
                    0 => None,
                    _ => Some(r.u32()?),
                },
                retail_definition: read_bytes_opt(r)?,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let textures = (0..r.u32()?)
        .map(|_| -> Result<MapTexture, String> {
            Ok(MapTexture { name: r.str()?, width: r.u32()?, height: r.u32()?, color_space: r.u32()?, rgba: Vec::new() })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let collision = (0..r.u32()?)
        .map(|_| -> Result<Collision, String> {
            let mut points = [[0.0; 3]; 3];
            for p in &mut points {
                *p = [r.f32()?, r.f32()?, r.f32()?];
            }
            Ok(Collision {
                points,
                surface: r.u32()?,
                material: r.u32()?,
                native_edges: match r.u8()? {
                    0 => None,
                    _ => Some([r.u8()?, r.u8()?, r.u8()?]),
                },
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let rails = (0..r.u32()?).map(|_| read_rail(r)).collect::<Result<Vec<_>, _>>()?;
    let lights = (0..r.u32()?)
        .map(|_| -> Result<Light, String> {
            Ok(Light {
                name: r.str()?,
                kind: r.u32()?,
                position: [r.f32()?, r.f32()?, r.f32()?],
                direction: [r.f32()?, r.f32()?, r.f32()?],
                color: [r.f32()?, r.f32()?, r.f32()?],
                intensity: r.f32()?,
                range: r.f32()?,
                radius: r.f32()?,
                inner_cos: r.f32()?,
                outer_cos: r.f32()?,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let routes = (0..r.u32()?)
        .map(|_| -> Result<Route, String> {
            Ok(Route { rail: read_rail(r)?, skaters: r.u32()?, speed: r.f32()?, spacing: r.f32()? })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let extensions = (0..r.u32()?)
        .map(|_| -> Result<Extension, String> {
            Ok(Extension { tag: [r.u8()?, r.u8()?, r.u8()?, r.u8()?], schema: r.u32()?, payload: r.block()?.to_vec() })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(SkateMap {
        version,
        name,
        spawn,
        heading,
        environment,
        materials,
        textures,
        geometry: Geometry { vertices: Vec::new(), indices: Vec::new(), collision },
        rails,
        doors: Vec::new(),
        lights,
        routes,
        extensions,
    })
}

fn write_render(w: &mut Writer, m: &Render) {
    w.block(&m.vertex_data);
    w.u32(m.indices.len() as u32);
    m.indices.iter().for_each(|&i| w.u16(i));
    w.u32(m.draws.len() as u32);
    for d in &m.draws {
        for v in [d.first_vertex, d.vertex_count, d.first_index, d.index_count] {
            w.u32(v);
        }
        for t in [d.albedo, d.lightmap] {
            w.i32(t.map_or(-1, i32::from));
        }
        d.color.iter().for_each(|&v| w.f32(v));
        w.f32(d.alpha_cutoff.unwrap_or(-1.0));
        d.bounds_min.iter().chain(&d.bounds_max).for_each(|&v| w.f32(v));
    }
    w.u32(m.textures.len() as u32);
    m.textures.iter().for_each(|t| t.write(w));
    m.horizon.iter().for_each(|&v| w.f32(v));
    w.f32(m.exposure);
}

fn read_render(r: &mut Reader) -> Result<Render, String> {
    let vertex_data = r.block()?.to_vec();
    let indices = (0..r.u32()?).map(|_| r.u16()).collect::<Result<Vec<_>, _>>()?;
    let draws = (0..r.u32()?)
        .map(|_| -> Result<Draw, String> {
            let (first_vertex, vertex_count, first_index, index_count) = (r.u32()?, r.u32()?, r.u32()?, r.u32()?);
            let albedo = u16::try_from(r.i32()?).ok();
            let lightmap = u16::try_from(r.i32()?).ok();
            let color = [r.f32()?, r.f32()?, r.f32()?, r.f32()?];
            let cutoff = r.f32()?;
            Ok(Draw {
                first_vertex,
                vertex_count,
                first_index,
                index_count,
                albedo,
                lightmap,
                color,
                alpha_cutoff: (cutoff >= 0.0).then_some(cutoff),
                bounds_min: [r.f32()?, r.f32()?, r.f32()?],
                bounds_max: [r.f32()?, r.f32()?, r.f32()?],
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let textures = (0..r.u32()?).map(|_| Texture::read(r)).collect::<Result<Vec<_>, _>>()?;
    let horizon = [r.f32()?, r.f32()?, r.f32()?];
    let exposure = r.f32()?;
    let render = Render { vertex_data, indices, draws, textures, horizon, exposure };
    validate_render(&render)?;
    Ok(render)
}

fn validate_render(m: &Render) -> Result<(), String> {
    if m.vertex_data.len() % STRIDE != 0 {
        return Err("map vertex block is not a whole number of vertices".into());
    }
    let vertices = m.vertex_data.len() / STRIDE;
    for d in &m.draws {
        if (d.first_vertex + d.vertex_count) as usize > vertices
            || (d.first_index + d.index_count) as usize > m.indices.len()
            || m.indices[d.first_index as usize..(d.first_index + d.index_count) as usize]
                .iter()
                .any(|&i| u32::from(i) >= d.vertex_count)
            || [d.albedo, d.lightmap].iter().flatten().any(|&t| usize::from(t) >= m.textures.len())
        {
            return Err("map draw out of range".into());
        }
    }
    m.textures.iter().try_for_each(Texture::validate)
}
